//! The daemon's state: compiled rules, the memo (content store), contents and paths interned, the
//! membership model (base, cluster bases, worktree deltas; `membership.rs`), the per-file results
//! of every (path, content) pair some tag holds, and the cross-file side's worker.
//!
//! Per-worktree state is its delta from its parent level and its epoch; findings are not stored per
//! worktree: a worktree's per-file findings are its members' pair results, and a membership change
//! answers with the changed paths' old and new pair results (introduced / removed): at once for
//! the pairs resident or served by the memo, then as the workers finish the clauses evaluated
//! (partial frames, PROTOCOL.md § Streaming), the remainder in the reply.
use super::crossfile::{self, PathChange, Snapshot, Worker};
use super::git::Odb;
use super::membership::{self, BASE, Cid, Members, Model, Resident, Sym, W};
use super::memory;
use super::perfile::{self, Progress};
use crate::eval::{Failure, Rule};
use crate::memo::Memo;
use rerail_engine::content_store::{HashKey, SourceId};
use rustc_hash::FxHashMap;
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// The pins the daemon holds on the sources of the contents some worktree holds.
const PIN_OWNER: &str = "rerail-facts-daemon";
const NO_COLUMN: u32 = u32::MAX;
/// A file-level failure (not UTF-8, not parsed): no clause.
const NO_CLAUSE: u32 = u32::MAX;

struct ContentInfo {
    source: SourceId,
    blake3: Option<[u8; 32]>,
}

/// One finding of a per-file clause, compact: the clause's ordinal in the policy set, its path
/// (the pair's own, by construction of per-file clauses), line, column and code.
#[derive(Clone, Copy)]
struct Found {
    ord: u32,
    path: Sym,
    line: u32,
    column: u32,
    code: u32,
}

/// A held (path, content)'s per-file results.
struct Pair {
    found: Box<[Found]>,
    failures: Box<[(u32, Box<str>)]>,
}

pub struct Worktree {
    tag: W,
    /// Paths where the worktree differs from its parent level: its content, or `None` (absent).
    delta: BTreeMap<Sym, Option<Cid>>,
    epoch: u64,
}

#[derive(Default, Clone, Copy)]
pub struct Work {
    pub evaluated_clauses: u64,
    pub memoised_clauses: u64,
    pub evaluated_pairs: u64,
    pub blobs_read: u64,
}

impl Work {
    fn add(&mut self, o: &Work) {
        self.evaluated_clauses += o.evaluated_clauses;
        self.memoised_clauses += o.memoised_clauses;
        self.evaluated_pairs += o.evaluated_pairs;
        self.blobs_read += o.blobs_read;
    }
    fn json(&self) -> Json {
        json!({"evaluated_clauses": self.evaluated_clauses, "memoised_clauses": self.memoised_clauses, "evaluated_pairs": self.evaluated_pairs, "blobs_read": self.blobs_read})
    }
}

/// A membership change's answer: the reply's part of the delta (what no partial frame carried).
pub struct Change {
    pub epoch: u64,
    pub introduced: Vec<Json>,
    pub removed: Vec<Json>,
    pub failures_introduced: Vec<Json>,
    pub failures_removed: Vec<Json>,
    pub changed_paths: usize,
    pub work: Work,
    /// Partial frames sent before the reply.
    pub frames: u32,
    pub shape: perfile::Shape,
}

impl Change {
    fn none(epoch: u64) -> Change {
        Change { epoch, introduced: Vec::new(), removed: Vec::new(), failures_introduced: Vec::new(), failures_removed: Vec::new(), changed_paths: 0, work: Work::default(), frames: 0, shape: perfile::Shape::default() }
    }

    pub fn json(&self, worktree: &str) -> Json {
        json!({
            "op": "findings", "worktree": worktree, "epoch": self.epoch, "cross": false, "partial": false,
            "introduced": self.introduced, "removed": self.removed,
            "failures_introduced": self.failures_introduced, "failures_removed": self.failures_removed,
            "changed_paths": self.changed_paths, "work": self.work.json(), "frames": self.frames,
            "tasks": self.shape.tasks, "workers": self.shape.workers, "scratch_peak": self.shape.scratch_peak,
        })
    }
}

/// Findings and failures introduced and removed, not yet sent.
#[derive(Default)]
struct Buffered {
    introduced: Vec<Json>,
    removed: Vec<Json>,
    failures_introduced: Vec<Json>,
    failures_removed: Vec<Json>,
}

impl Buffered {
    fn is_empty(&self) -> bool {
        self.introduced.is_empty() && self.removed.is_empty() && self.failures_introduced.is_empty() && self.failures_removed.is_empty()
    }

    /// A partial `findings` frame of `worktree`'s request (the connection adds `seq` and `ms`).
    fn frame(&mut self, worktree: &str, epoch: u64) -> Json {
        let b = std::mem::take(self);
        json!({
            "op": "findings", "worktree": worktree, "epoch": epoch, "cross": false, "partial": true,
            "introduced": b.introduced, "removed": b.removed,
            "failures_introduced": b.failures_introduced, "failures_removed": b.failures_removed,
        })
    }
}

/// A changed path while its request runs: the old content's per-file results by clause ordinal
/// (`NO_CLAUSE`: file-level failures), each removed when the new content's clause is sent, and the
/// new content's ordinals already sent. `whole`: answered at once (deleted, or the new pair was
/// resident).
struct Changing {
    new: Option<Cid>,
    old: BTreeMap<u32, (Vec<Json>, Vec<Json>)>,
    sent: HashSet<u32>,
    whole: bool,
}

pub struct Engine {
    rules: Arc<[Rule]>,
    refused: Vec<Failure>,
    memo: Arc<Memo>,
    per_file: Vec<Vec<bool>>,
    /// Per global clause ordinal, (rule, clause).
    clause_of: Vec<(usize, usize)>,
    first: Vec<u32>,
    /// The evaluation workers (`--threads`), persistent for the daemon's life.
    pool: rayon::ThreadPool,
    paths: Vec<Arc<str>>,
    path_ids: HashMap<Arc<str>, Sym>,
    contents: Vec<ContentInfo>,
    by_source: HashMap<SourceId, Cid>,
    by_blake3: HashMap<[u8; 32], Cid>,
    by_git: HashMap<[u8; 20], Cid>,
    codes: Vec<Arc<str>>,
    code_ids: HashMap<Arc<str>, u32>,
    pairs: FxHashMap<(Sym, Cid), Pair>,
    content_held: Vec<u32>,
    resident: Resident,
    model: Model,
    worktrees: BTreeMap<String, Worktree>,
    next_tag: W,
    odbs: HashMap<PathBuf, Odb>,
    cross: Option<Worker>,
    pub total: Work,
    /// The largest worker scratch of any request (`perfile::Shape::scratch_peak`).
    pub scratch_peak: u64,
    /// Purges jemalloc's freed pages after each evaluating request, on its own thread.
    pub purger: memory::Purger,
}

impl Engine {
    pub fn new(rules: Arc<[Rule]>, refused: Vec<Failure>, memo: Arc<Memo>, threads: usize, cross: Option<Worker>) -> Engine {
        let per_file = crossfile::per_file_clauses(&rules);
        let mut first = Vec::new();
        let mut clause_of = Vec::new();
        for (ri, r) in rules.iter().enumerate() {
            first.push(clause_of.len() as u32);
            clause_of.extend((0..r.clauses.len()).map(|ci| (ri, ci)));
        }
        Engine {
            rules,
            refused,
            memo,
            per_file,
            clause_of,
            first,
            pool: rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("facts-eval-{i}")).build().expect("evaluation pool"),
            paths: Vec::new(),
            path_ids: HashMap::new(),
            contents: Vec::new(),
            by_source: HashMap::new(),
            by_blake3: HashMap::new(),
            by_git: HashMap::new(),
            codes: Vec::new(),
            code_ids: HashMap::new(),
            pairs: FxHashMap::default(),
            content_held: Vec::new(),
            resident: Resident::default(),
            model: Model::default(),
            worktrees: BTreeMap::new(),
            next_tag: 1,
            odbs: HashMap::new(),
            cross,
            total: Work::default(),
            scratch_peak: 0,
            purger: memory::Purger::spawn(),
        }
    }

    pub fn hello(&self) -> Json {
        let clauses = self.clause_of.len();
        let per_file: Vec<Json> = self.clause_of.iter().filter(|&&(ri, ci)| self.per_file[ri][ci]).map(|&(ri, ci)| json!([self.rules[ri].check, self.rules[ri].clauses[ci].id])).collect();
        json!({
            "op": "hello", "protocol": 2,
            "rules": self.rules.len(), "refused": self.refused.iter().map(|f| json!({"check": f.check, "message": f.message})).collect::<Vec<_>>(),
            "clauses": clauses, "cross_file_clauses": clauses - per_file.len(), "per_file_clauses": per_file,
            "cross": self.cross.as_ref().map(|c| c.name), "store": self.memo.path, "threads": self.pool.current_num_threads(),
        })
    }

    fn path_id(&mut self, p: &str) -> Sym {
        if let Some(&id) = self.path_ids.get(p) {
            return id;
        }
        let a: Arc<str> = Arc::from(p);
        let id = self.paths.len() as Sym;
        self.paths.push(a.clone());
        self.path_ids.insert(a, id);
        id
    }

    fn code_id(&mut self, c: &str) -> u32 {
        if let Some(&id) = self.code_ids.get(c) {
            return id;
        }
        let a: Arc<str> = Arc::from(c);
        let id = self.codes.len() as u32;
        self.codes.push(a.clone());
        self.code_ids.insert(a, id);
        id
    }

    fn intern_source(&mut self, source: SourceId, blake3: Option<[u8; 32]>, git: Option<[u8; 20]>) -> Cid {
        let cid = match self.by_source.get(&source) {
            Some(&c) => c,
            None => {
                let c = self.contents.len() as Cid;
                self.contents.push(ContentInfo { source, blake3: None });
                self.content_held.push(0);
                self.by_source.insert(source, c);
                c
            }
        };
        if let Some(h) = blake3 {
            self.contents[cid as usize].blake3 = Some(h);
            self.by_blake3.insert(h, cid);
        }
        if let Some(g) = git {
            self.by_git.insert(g, cid);
        }
        cid
    }

    /// The content whose blake3 hash is `hash`, if the store holds it.
    pub fn content_by_hash(&mut self, hash: [u8; 32]) -> Option<Cid> {
        if let Some(&c) = self.by_blake3.get(&hash) {
            return Some(c);
        }
        let source = self.memo.store.facts_source(hash).unwrap_or_else(|e| panic!("store: {e}"))?;
        Some(self.intern_source(source, Some(hash), None))
    }

    /// The content whose Git blob id is `oid`, if the store holds it.
    pub fn content_by_git(&mut self, oid: [u8; 20]) -> Option<Cid> {
        if let Some(&c) = self.by_git.get(&oid) {
            return Some(c);
        }
        let source = self.memo.store.source_by_alias(HashKey::git_sha1(oid)).unwrap_or_else(|e| panic!("store: {e}"))?;
        Some(self.intern_source(source, None, Some(oid)))
    }

    /// Stores `bytes` (checked against the Git blob id `oid`) and returns its content.
    pub fn put_content_git(&mut self, oid: [u8; 20], bytes: &[u8]) -> Result<Cid, String> {
        let id = gix::objs::compute_hash(gix::hash::Kind::Sha1, gix::object::Kind::Blob, bytes).map_err(|e| e.to_string())?;
        if id.as_bytes() != oid {
            return Err("the bytes do not hash to the edit's Git blob id".into());
        }
        let hash = *blake3::hash(bytes).as_bytes();
        let source = self.memo.store.put_facts_source(bytes, hash, &[HashKey::git_sha1(oid)]).map_err(|e| e.to_string())?;
        Ok(self.intern_source(source, Some(hash), Some(oid)))
    }

    /// Stores `bytes` (checked against `hash`) and returns its content.
    pub fn put_content(&mut self, hash: [u8; 32], bytes: &[u8]) -> Result<Cid, String> {
        if *blake3::hash(bytes).as_bytes() != hash {
            return Err("the bytes do not hash to the edit's hash".into());
        }
        let source = self.memo.store.put_facts_source(bytes, hash, &[]).map_err(|e| e.to_string())?;
        Ok(self.intern_source(source, Some(hash), None))
    }

    fn below(&self, tag: W) -> &Members {
        self.model.below(tag)
    }

    fn member(&self, w: &Worktree, p: Sym) -> Option<Cid> {
        match w.delta.get(&p) {
            Some(v) => *v,
            None => self.below(w.tag).get(&p).copied(),
        }
    }

    /// The worktree's members (its parent level overlaid with its delta), in path-id order.
    fn effective(&self, w: &Worktree) -> Members {
        let mut m = self.below(w.tag).clone();
        for (p, v) in &w.delta {
            match v {
                Some(c) => {
                    m.insert(*p, *c);
                }
                None => {
                    m.remove(p);
                }
            }
        }
        m
    }

    fn worktree_records(&self, w: &Worktree) -> Vec<((Sym, Cid), i32)> {
        let below = self.below(w.tag);
        let mut v = Vec::new();
        for (p, c) in &w.delta {
            if let Some(c) = c {
                v.push(((*p, *c), 1));
            }
            if let Some(c0) = below.get(p) {
                v.push(((*p, *c0), -1));
            }
        }
        v
    }

    fn update(&mut self, tag: W, records: &[((Sym, Cid), i32)], sign: i32, acquired: &mut Vec<(Sym, Cid)>, released: &mut Vec<(Sym, Cid)>) {
        for ((p, c), d) in records {
            self.resident.update(tag, *p, *c, d * sign, acquired, released);
        }
    }

    /// Settles a batch of record updates: pairs held at the end and not resident are evaluated (or
    /// read from the memo), pairs no longer held are dropped, and contents whose held-pair count
    /// crossed zero are pinned or unpinned. A pair both acquired and released in the batch (moved
    /// between tags) is neither. `fresh`: bytes read for this change (new blobs, an edit's body).
    /// `on` sees the evaluation's progress (`perfile::Progress`, results indexed into the pairs
    /// evaluated) with the engine as it was before the evaluation.
    fn settle(&mut self, acquired: &[(Sym, Cid)], released: &[(Sym, Cid)], fresh: &HashMap<Cid, Arc<Vec<u8>>>, work: &mut Work, on: &mut dyn FnMut(&Engine, &[(Sym, Cid)], Progress)) -> perfile::Shape {
        let mut before: HashMap<Cid, u32> = HashMap::new();
        for &(_, c) in acquired {
            before.entry(c).or_insert(self.content_held[c as usize]);
            self.content_held[c as usize] += 1;
        }
        for &(_, c) in released {
            before.entry(c).or_insert(self.content_held[c as usize]);
            self.content_held[c as usize] -= 1;
        }
        let (mut pins, mut unpins) = (Vec::new(), Vec::new());
        for (c, b) in before {
            match (b, self.content_held[c as usize]) {
                (0, n) if n > 0 => pins.push(self.contents[c as usize].source),
                (b, 0) if b > 0 => unpins.push(self.contents[c as usize].source),
                _ => {}
            }
        }
        if !pins.is_empty() {
            let lease = crate::daemon::now() + 30 * 24 * 3600;
            self.memo.store.pin_facts_sources(&pins, PIN_OWNER, lease).unwrap_or_else(|e| panic!("store: {e}"));
        }
        if !unpins.is_empty() {
            self.memo.store.unpin_facts_sources(&unpins, PIN_OWNER).unwrap_or_else(|e| panic!("store: {e}"));
        }
        for k in released {
            if !self.resident.is_held(k.0, k.1) {
                self.pairs.remove(k);
            }
        }
        let mut todo: Vec<(Sym, Cid)> = acquired.iter().copied().filter(|k| self.resident.is_held(k.0, k.1) && !self.pairs.contains_key(k)).collect();
        todo.sort_unstable();
        todo.dedup();
        if todo.is_empty() {
            return perfile::Shape::default();
        }
        let jobs: Vec<perfile::Job> = todo.iter().map(|&(p, c)| perfile::Job { path: self.paths[p as usize].clone(), source: self.contents[c as usize].source, bytes: fresh.get(&c).cloned() }).collect();
        let this = &*self;
        let (results, shape) = perfile::evaluate(&this.rules, &this.memo, &jobs, &this.pool, &mut |p| on(this, &todo, p));
        for (&(p, c), ev) in todo.iter().zip(results) {
            work.evaluated_clauses += u64::from(ev.evaluated);
            work.memoised_clauses += u64::from(ev.memoised);
            work.evaluated_pairs += u64::from(ev.evaluated > 0);
            let pair = self.pair_of(p, ev);
            self.pairs.insert((p, c), pair);
        }
        self.scratch_peak = self.scratch_peak.max(shape.scratch_peak);
        shape
    }

    fn pair_of(&mut self, p: Sym, ev: perfile::Evaluated) -> Pair {
        let mut found = Vec::new();
        let mut failures = Vec::new();
        for r in ev.reported {
            let (ri, ci) = r.clause;
            if !self.per_file[ri][ci] {
                continue;
            }
            let ord = self.first[ri] + ci as u32;
            for f in r.findings {
                let path = if *f.path == *self.paths[p as usize] { p } else { self.path_id(&f.path) };
                let code = self.code_id(&f.code);
                found.push(Found { ord, path, line: f.line, column: f.column.unwrap_or(NO_COLUMN), code });
            }
            failures.extend(r.failures.into_iter().map(|f| (ord, f.message.into_boxed_str())));
        }
        failures.extend(ev.file_failures.into_iter().map(|f| (NO_CLAUSE, f.message.into_boxed_str())));
        Pair { found: found.into(), failures: failures.into() }
    }

    fn finding_json(&self, f: &Found) -> Json {
        let (ri, ci) = self.clause_of[f.ord as usize];
        let column = if f.column == NO_COLUMN { Json::Null } else { json!(f.column) };
        json!([self.rules[ri].check, self.rules[ri].clauses[ci].id, &*self.paths[f.path as usize], f.line, &*self.codes[f.code as usize], column])
    }

    fn failure_json(&self, p: Sym, ord: u32, message: &str) -> Json {
        let (check, clause) = if ord == NO_CLAUSE {
            ("", "")
        } else {
            let (ri, ci) = self.clause_of[ord as usize];
            (self.rules[ri].check.as_str(), self.rules[ri].clauses[ci].id.as_str())
        };
        json!([check, clause, &*self.paths[p as usize], message])
    }

    fn pair_json(&self, p: Sym, c: Cid, findings: &mut Vec<Json>, failures: &mut Vec<Json>) {
        let pair = self.pairs.get(&(p, c)).expect("a held pair is resident");
        findings.extend(pair.found.iter().map(|f| self.finding_json(f)));
        failures.extend(pair.failures.iter().map(|(o, m)| self.failure_json(p, *o, m)));
    }

    /// Replaces `name`'s membership by `m` (creating the worktree if new) and answers with the
    /// per-file findings of the changed paths. When the change evaluates some clause, `send` gets
    /// partial frames first (PROTOCOL.md § Streaming): the changed paths whose new pair is resident
    /// or served by the memo, then each batch of clauses the workers finish, each clause's old
    /// results removed in the frame that introduces its new ones; the returned change holds the
    /// rest. Without evaluation every result is in the returned change.
    pub fn set_membership(&mut self, name: &str, m: Members, fresh: &HashMap<Cid, Arc<Vec<u8>>>, mut work: Work, send: &mut dyn FnMut(Json)) -> Change {
        let (old, existed) = match self.worktrees.get(name) {
            Some(w) => (self.effective(w), true),
            None => (Members::new(), false),
        };
        let mut changed: Vec<(Sym, Option<Cid>, Option<Cid>)> = Vec::new();
        for (p, c) in &m {
            let o = old.get(p).copied();
            if o != Some(*c) {
                changed.push((*p, o, Some(*c)));
            }
        }
        for (p, c) in &old {
            if !m.contains_key(p) {
                changed.push((*p, Some(*c), None));
            }
        }
        // The old pairs' results by clause (they may be released below), and the paths answered
        // at once: deleted, or their new pair resident.
        let mut buf = Buffered::default();
        let mut changing: HashMap<Sym, Changing> = HashMap::with_capacity(changed.len());
        for &(p, o, n) in &changed {
            let mut ch = Changing { new: n, old: BTreeMap::new(), sent: HashSet::new(), whole: false };
            if let Some(o) = o {
                let pair = self.pairs.get(&(p, o)).expect("a held pair is resident");
                for f in pair.found.iter() {
                    ch.old.entry(f.ord).or_default().0.push(self.finding_json(f));
                }
                for (ord, msg) in pair.failures.iter() {
                    ch.old.entry(*ord).or_default().1.push(self.failure_json(p, *ord, msg));
                }
            }
            if n.is_none_or(|n| self.pairs.contains_key(&(p, n))) {
                ch.whole = true;
                for (_, (fs, xs)) in std::mem::take(&mut ch.old) {
                    buf.removed.extend(fs);
                    buf.failures_removed.extend(xs);
                }
                if let Some(n) = n {
                    self.pair_json(p, n, &mut buf.introduced, &mut buf.failures_introduced);
                }
            }
            changing.insert(p, ch);
        }
        let (mut acquired, mut released) = (Vec::new(), Vec::new());
        let tag = if existed {
            let w = &self.worktrees[name];
            let (tag, recs) = (w.tag, self.worktree_records(w));
            self.update(tag, &recs, -1, &mut acquired, &mut released);
            tag
        } else {
            let tag = self.next_tag;
            self.next_tag += 1;
            if self.worktrees.is_empty() && self.model.base.is_empty() && self.model.clusters.is_empty() {
                // The first worktree is the base until the next rebalance.
                self.model.base = m.clone();
                let recs: Vec<((Sym, Cid), i32)> = m.iter().map(|(p, c)| ((*p, *c), 1)).collect();
                self.update(BASE, &recs, 1, &mut acquired, &mut released);
                self.model.parent.insert(tag, BASE);
            } else {
                self.model.attach(tag, &m, &|_, _| 1);
            }
            tag
        };
        let below = self.below(tag);
        let mut delta = BTreeMap::new();
        for ((p, c), d) in membership::delta(&m, below) {
            if d > 0 {
                delta.insert(p, Some(c));
            } else {
                delta.entry(p).or_insert(None);
            }
        }
        let epoch = self.worktrees.get(name).map_or(0, |w| w.epoch) + 1;
        let w = Worktree { tag, delta, epoch };
        let recs = self.worktree_records(&w);
        self.update(tag, &recs, 1, &mut acquired, &mut released);
        self.worktrees.insert(name.to_owned(), w);
        let mut frames = 0u32;
        let shape = self.settle(&acquired, &released, fresh, &mut work, &mut |eng, todo, progress| {
            let (results, flush) = match progress {
                Progress::Served { results, evaluating } => (results, evaluating),
                Progress::Evaluated(results) => (results, true),
            };
            for (j, r) in results {
                let (p, c) = todo[*j];
                let Some(ch) = changing.get_mut(&p) else { continue };
                let (ri, ci) = r.clause;
                if ch.whole || ch.new != Some(c) || !eng.per_file[ri][ci] {
                    continue;
                }
                let ord = eng.first[ri] + ci as u32;
                ch.sent.insert(ord);
                buf.introduced.extend(r.findings.iter().map(|f| json!([f.check, f.clause, f.path, f.line, f.code, f.column])));
                buf.failures_introduced.extend(r.failures.iter().map(|f| json!([f.check, f.clause, &*eng.paths[p as usize], f.message])));
                if let Some((fs, xs)) = ch.old.remove(&ord) {
                    buf.removed.extend(fs);
                    buf.failures_removed.extend(xs);
                }
            }
            if flush && !buf.is_empty() {
                send(buf.frame(name, epoch));
                frames += 1;
            }
        });
        // The rest: what was not sent of each changed path (with no evaluation, everything).
        for (p, ch) in &mut changing {
            if ch.whole {
                continue;
            }
            for (_, (fs, xs)) in std::mem::take(&mut ch.old) {
                buf.removed.extend(fs);
                buf.failures_removed.extend(xs);
            }
            let pair = self.pairs.get(&(*p, ch.new.expect("not whole: present"))).expect("a held pair is resident");
            buf.introduced.extend(pair.found.iter().filter(|f| !ch.sent.contains(&f.ord)).map(|f| self.finding_json(f)));
            buf.failures_introduced.extend(pair.failures.iter().filter(|(o, _)| !ch.sent.contains(o)).map(|(o, m)| self.failure_json(*p, *o, m)));
        }
        if let Some(cross) = &self.cross
            && !changed.is_empty()
        {
            let changes: Vec<PathChange> = changed
                .iter()
                .map(|(p, o, n)| PathChange { path: self.paths[*p as usize].clone(), old: o.map(|c| self.contents[c as usize].source), new: n.map(|c| self.contents[c as usize].source) })
                .collect();
            cross.apply(name, epoch, self.snapshot(&self.worktrees[name]), changes);
        }
        self.total.add(&work);
        let Buffered { introduced, removed, failures_introduced, failures_removed } = buf;
        Change { epoch, introduced, removed, failures_introduced, failures_removed, changed_paths: changed.len(), work, frames, shape }
    }

    /// A worktree's members for the cross-file worker.
    fn snapshot(&self, w: &Worktree) -> Snapshot {
        Snapshot(self.effective(w).into_iter().map(|(p, c)| (self.paths[p as usize].clone(), self.contents[c as usize].source)).collect())
    }

    /// One path's new content (`None`: deleted). An unchanged content is no work and no change.
    pub fn edit(&mut self, name: &str, path: &str, content: Option<Cid>, fresh: &HashMap<Cid, Arc<Vec<u8>>>, send: &mut dyn FnMut(Json)) -> Result<Change, String> {
        let p = self.path_id(path);
        let w = self.worktrees.get(name).ok_or_else(|| format!("unknown worktree {name}"))?;
        if self.member(w, p) == content {
            return Ok(Change::none(w.epoch));
        }
        let mut m = self.effective(w);
        match content {
            Some(c) => {
                m.insert(p, c);
            }
            None => {
                m.remove(&p);
            }
        }
        Ok(self.set_membership(name, m, fresh, Work::default(), send))
    }

    /// `name`'s membership from `commit`'s tree in the Git object database at `git_dir`: blobs the
    /// store knows by their Git id are not read.
    pub fn sync_git(&mut self, name: &str, git_dir: &Path, commit: &str, send: &mut dyn FnMut(Json)) -> Result<Change, String> {
        if !self.odbs.contains_key(git_dir) {
            let odb = Odb::open(git_dir)?;
            self.odbs.insert(git_dir.to_path_buf(), odb);
        }
        let files = self.odbs[git_dir].tree_files(commit)?;
        self.sync_files(name, git_dir, files.into_iter().map(|f| (f.path, f.oid)).collect(), send)
    }

    /// `name`'s membership from an index snapshot: (path, Git blob id) entries, blobs read from
    /// the object database at `git_dir` when the store does not know them.
    pub fn sync_files(&mut self, name: &str, git_dir: &Path, files: Vec<(String, [u8; 20])>, send: &mut dyn FnMut(Json)) -> Result<Change, String> {
        if !self.odbs.contains_key(git_dir) {
            let odb = Odb::open(git_dir)?;
            self.odbs.insert(git_dir.to_path_buf(), odb);
        }
        let mut m = Members::new();
        let mut fresh: HashMap<Cid, Arc<Vec<u8>>> = HashMap::new();
        let mut work = Work::default();
        for (path, oid) in files {
            let p = self.path_id(&path);
            let c = match self.by_git.get(&oid) {
                Some(&c) => c,
                None => match self.memo.store.source_by_alias(HashKey::git_sha1(oid)).map_err(|e| e.to_string())? {
                    Some(source) => self.intern_source(source, None, Some(oid)),
                    None => {
                        let bytes = self.odbs[git_dir].blob(&oid)?;
                        work.blobs_read += 1;
                        let hash = *blake3::hash(&bytes).as_bytes();
                        let source = self.memo.store.put_facts_source(&bytes, hash, &[HashKey::git_sha1(oid)]).map_err(|e| e.to_string())?;
                        let c = self.intern_source(source, Some(hash), Some(oid));
                        fresh.insert(c, Arc::new(bytes));
                        c
                    }
                },
            };
            m.insert(p, c);
        }
        Ok(self.set_membership(name, m, &fresh, work, send))
    }

    /// Recomputes the membership model (plurality base and cluster bases over every worktree, in
    /// name order) and every worktree's delta; no findings change.
    pub fn rebalance(&mut self) -> Json {
        let t0 = Instant::now();
        let names: Vec<String> = self.worktrees.keys().cloned().collect();
        let effective: Vec<(W, Members)> = names.iter().map(|n| (self.worktrees[n].tag, self.effective(&self.worktrees[n]))).collect();
        let old_records = self.all_records();
        let order: Vec<(W, &Members)> = effective.iter().map(|(w, m)| (*w, m)).collect();
        let model = membership::model(&order, &|_, _| 1);
        let (cost, flat_cost, clusters) = (model.cost, model.flat_cost, model.clusters.len());
        self.model = model;
        for (name, (_, m)) in names.iter().zip(&effective) {
            let tag = self.worktrees[name].tag;
            let mut delta = BTreeMap::new();
            for ((p, c), d) in membership::delta(m, self.below(tag)) {
                if d > 0 {
                    delta.insert(p, Some(c));
                } else {
                    delta.entry(p).or_insert(None);
                }
            }
            self.worktrees.get_mut(name).unwrap().delta = delta;
        }
        let new_records = self.all_records();
        let (mut acquired, mut released) = (Vec::new(), Vec::new());
        for (tag, recs) in &new_records {
            self.update(*tag, recs, 1, &mut acquired, &mut released);
        }
        for (tag, recs) in &old_records {
            self.update(*tag, recs, -1, &mut acquired, &mut released);
        }
        let mut work = Work::default();
        self.settle(&acquired, &released, &HashMap::new(), &mut work, &mut |_, _, _| {});
        self.total.add(&work);
        json!({"op": "rebalanced", "worktrees": names.len(), "clusters": clusters, "cost": cost, "flat_cost": flat_cost, "ms": t0.elapsed().as_secs_f64() * 1e3, "work": work.json(), "state": self.state_json()})
    }

    fn all_records(&self) -> Vec<(W, Vec<((Sym, Cid), i32)>)> {
        let mut v = self.model.level_records();
        for w in self.worktrees.values() {
            v.push((w.tag, self.worktree_records(w)));
        }
        v
    }

    pub fn close(&mut self, name: &str) -> Result<Json, String> {
        let w = self.worktrees.remove(name).ok_or_else(|| format!("unknown worktree {name}"))?;
        let recs = self.worktree_records(&w);
        let (mut acquired, mut released) = (Vec::new(), Vec::new());
        self.update(w.tag, &recs, -1, &mut acquired, &mut released);
        let mut work = Work::default();
        self.settle(&acquired, &released, &HashMap::new(), &mut work, &mut |_, _, _| {});
        self.total.add(&work);
        self.model.parent.remove(&w.tag);
        if let Some(c) = &self.cross {
            c.forget(name);
        }
        Ok(json!({"op": "closed", "worktree": name, "released_pairs": released.len()}))
    }

    /// Every per-file finding and failure of `name` (for checks against another engine).
    pub fn findings(&self, name: &str) -> Result<Json, String> {
        let w = self.worktrees.get(name).ok_or_else(|| format!("unknown worktree {name}"))?;
        let (mut findings, mut failures) = (Vec::new(), Vec::new());
        for (p, c) in self.effective(w) {
            self.pair_json(p, c, &mut findings, &mut failures);
        }
        failures.extend(self.refused.iter().map(|f| json!([f.check, f.clause, f.path, f.message])));
        Ok(json!({"op": "all_findings", "worktree": name, "epoch": w.epoch, "findings": findings, "failures": failures}))
    }

    pub fn state_json(&self) -> Json {
        let deltas: BTreeMap<&str, usize> = self.worktrees.iter().map(|(n, w)| (n.as_str(), w.delta.len())).collect();
        let found: usize = self.pairs.values().map(|p| p.found.len()).sum();
        json!({
            "worktrees": self.worktrees.len(),
            "base_paths": self.model.base.len(),
            "clusters": self.model.clusters.iter().map(|(t, (ws, m))| json!({"tag": t, "worktrees": ws.len(), "paths": m.len()})).collect::<Vec<_>>(),
            "worktree_delta_paths": deltas,
            "held_pairs": self.resident.held(),
            "tag_records": self.resident.records(),
            "resident_pairs": self.pairs.len(),
            "resident_findings": found,
            "contents": self.contents.len(),
            "held_contents": self.content_held.iter().filter(|h| **h > 0).count(),
            "paths": self.paths.len(),
            "cross_held_findings": self.cross.as_ref().map_or(0, |c| c.held_findings()),
            "cross_runs": self.cross.as_ref().map_or(0, |c| c.runs()),
            "scratch_peak": self.scratch_peak,
            "purges": self.purger.json(),
        })
    }

    /// Tears the engine down one component at a time and measures what each held: the live
    /// Rust heap (jemalloc `stats.allocated`) and the C heap (the system malloc zones: SQLite's
    /// page caches, statements and schema) that dropping it returned. What stays live after the
    /// last component is the process's own (statics such as the regex cache, thread-locals,
    /// connections' buffers).
    pub fn attribute(self) -> Json {
        let start = memory::heaps();
        let mut last = start;
        let mut rows = Vec::new();
        let mut step = |component: &str, items: Json| {
            let now = memory::heaps();
            rows.push(json!({"component": component, "items": items, "rust_heap": last.0 as i64 - now.0 as i64, "c_heap": last.1 as i64 - now.1 as i64}));
            last = now;
        };
        let Engine { rules, refused, memo, per_file, clause_of, first, paths, path_ids, contents, by_source, by_blake3, by_git, codes, code_ids, pairs, content_held, resident, model, worktrees, odbs, cross, .. } = self;
        let n = cross.as_ref().map_or(0, |c| c.held_findings());
        drop(cross);
        step("cross-file worker", json!({"held_findings": n}));
        let n = (pairs.len(), pairs.values().map(|p| p.found.len()).sum::<usize>());
        drop(pairs);
        step("per-file results (held pairs)", json!({"pairs": n.0, "findings": n.1}));
        let n = (resident.held(), resident.records(), worktrees.len(), model.base.len());
        drop((resident, model, worktrees));
        step("membership (tag records, base and cluster bases, deltas)", json!({"held_pairs": n.0, "tag_records": n.1, "worktrees": n.2, "base_paths": n.3}));
        let n = (paths.len(), contents.len(), codes.len());
        drop((paths, path_ids, contents, by_source, by_blake3, by_git, codes, code_ids, content_held));
        step("interner (paths, contents, codes)", json!({"paths": n.0, "contents": n.1, "codes": n.2}));
        let n = odbs.len();
        drop(odbs);
        step("Git object databases", json!({"repositories": n}));
        let store = Arc::try_unwrap(memo).ok().expect("the engine holds the last memo").into_store();
        step("memo index (plan set, clause ordinals)", Json::Null);
        drop(store);
        step("content store (read caches, plan sets, this thread's SQLite connection)", Json::Null);
        let n = rules.iter().map(|r| r.clauses.len()).sum::<usize>();
        drop((rules, refused, per_file, clause_of, first));
        step("compiled rules", json!({"clauses": n}));
        let end = memory::heaps();
        json!({"start": {"rust_heap": start.0, "c_heap": start.1}, "components": rows, "remainder": {"rust_heap": end.0, "c_heap": end.1}, "allocators": memory::allocators()})
    }

    /// The store's size on disk (database and WAL).
    pub fn store_bytes(&self) -> u64 {
        ["", "-wal"].iter().filter_map(|s| std::fs::metadata(format!("{}{s}", self.memo.path.display())).ok()).map(|m| m.len()).sum()
    }
}
