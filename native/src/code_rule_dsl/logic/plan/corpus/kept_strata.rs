//! The cross-file part's strata kept across processes (decision 0077 §4), for a run with no session: what a session
//! keeps in memory (`retain`), a run from the command line keeps in the tape cache.
//!
//! A run after an edit reads every file's exports from the memo, but its cross-file part derived everything again:
//! 23 s of the Python pack's 51 s after a one-line edit. Most strata read a relation the edit changed (`node`, the
//! edited file's exports), so a cut made before the run (0077 §2: a stratum runs when an input it reads changed, or a
//! relation of a stratum that runs) runs 815 of the pack's 1,556 strata, 21.7 of 24.4 s. But most strata that run
//! derive the rows they had: an appended line changes no import, no class, no call of another file. So the cut is
//! made as the run goes, stratum by stratum (`plan::Hooks::strata`), with each stratum's rows' digest: a stratum
//! runs when a relation it reads has rows other than the last run's (an input's digest, or a relation of a stratum
//! that ran and came out different), and a stratum that comes out the same leaves its readers clean (early cutoff).
//! On the pack that is 351 strata run, 16.6 s.
//!
//! Kept as the store's kept facts (`content_store::kept_facts`, decisions 0104, 0456), in the corpus root's cross-file
//! fact space: each derived relation under its identity (`plan::identities`: its rules, and what they read, down to
//! the inputs), with its digest, its rows and the digest each relation its stratum read had then. A stratum is clean
//! when its relations are kept under their identities and each read has the digest it had. So a relation two
//! programs share is one entry, whichever ran last, and a rule edit makes only the relations it reaches new. A clean
//! relation's rows are read back only when a stratum that runs reads it, or the run's end keeps it (an output): on
//! the pack, 68 k rows of 695 relations, against 3.0 M rows derived. A run writes the relations whose rows changed.

use rerail_blocks::vector::Vector;
use crate::code_rule_dsl::logic::native::component::LIVE_BATCH;
use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::logic::plan::KeptStrata;
use super::retain::kept_read::Bound;
use crate::content_store::kept_facts::{KeptPut, KeptWhat};
use crate::content_store::{ContentStore, KeptRel, KeptStored};
use sha2::{Digest as _, Sha256};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

/// Rows (values) buffered before a write.
const FLUSH_VALUES: usize = 1 << 22;

/// The cross-file fact space of the corpus at `root`, of the phase that runs (decision 0083: each phase its own
/// namespaced fact space). Phase 1 and the analysis read `doc` over the same root with different rows (phase 1's
/// with the libraries' documents): in one space, each overwrote the other's `doc` frame and the next run found no
/// stratum current.
pub(in crate::code_rule_dsl) fn space(root: &Path) -> [u8; 32] {
    match super::phases::current() {
        Some(phase) => part_space(root, &format!("cross/{phase}")),
        None => part_space(root, "cross"),
    }
}

/// The fact space of a part of the run over `root` (`cross`, `cone`: the cone of requests answered in the fixpoint).
pub(in crate::code_rule_dsl) fn part_space(root: &Path, part: &str) -> [u8; 32] {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut h = Sha256::new();
    h.update(format!("rerail.space/v1\0{part}\0").as_bytes());
    h.update(root.to_string_lossy().as_bytes());
    h.finalize().into()
}

/// `root`'s cross part's space (`part_space(root, "cross")`).
pub(in crate::code_rule_dsl) fn cross_space(root: &Path) -> [u8; 32] {
    part_space(root, "cross")
}

/// What a space keeps of `rels` at partition `part`: each entry's digest and reads.
fn stored_of(bound: Option<&Arc<Bound>>, rels: &[KeptRel], part: i64) -> HashMap<KeptRel, KeptStored> {
    let Some(b) = bound else { return HashMap::new() };
    rels.iter().filter_map(|r| b.entry(&r.name, &r.ident, part).map(|e| (r.clone(), KeptStored { digest: e.digest(), reads: e.reads() }))).collect()
}

/// An entry of `n` rows, flat, of `types`' columns.
fn flat_put(rel: &KeptRel, digest: u64, reads: Vec<(String, u64)>, types: Vec<u8>, rows: Vector<i64>, n: usize) -> KeptPut {
    KeptPut { rel: rel.name.clone(), ident: rel.ident, part: 0, digest, reads, what: KeptWhat::Flat { widths: rerail_blocks::vector![8; types.len()], rows, n, levels: None, given: false }, types }
}

pub(in crate::code_rule_dsl) struct Kept<'s> {
    store: &'s ContentStore,
    space: [u8; 32],
    /// Each relation of the program, as its frame names it (`program`).
    rels: HashMap<String, KeptRel>,
    /// What is stored of those relations.
    was: HashMap<String, KeptStored>,
    /// The space's kept blocks as the run read them (decision 0401 F3): a kept relation is bound from them.
    bound: Option<Arc<Bound>>,
    /// The inputs bound before the run, and those bound as their first reader runs (`loaded`, by the caller).
    inputs: HashMap<String, Arc<dyn Source>>,
    pub(in crate::code_rule_dsl) loaded: Rc<RefCell<HashMap<String, Arc<dyn Source>>>>,
    digests: HashMap<String, u64>,
    /// Each input's column types (`retain::types_of`): its digest is of its content (`Cells`, decision 0212). And each
    /// relation's as its frame keeps them (`typed`, by the columns' domains), as `load` reads them back: without, the
    /// engine's declared types, a kind's or a field's `sym` column typed `s`, which `load` refuses as kept under other
    /// types, and a forked process's writer maps as atoms.
    types: HashMap<String, Vec<u8>>,
    puts: Vec<KeptPut>,
    values: usize,
    pub(in crate::code_rule_dsl) written_rows: usize,
    /// The strata the run evaluated, of all (`KeptStrata::counted`).
    pub(in crate::code_rule_dsl) strata: Option<(usize, usize)>,
    /// The kept relations the run bound (`rows`): those a stratum that ran read, and the outputs.
    pub(in crate::code_rule_dsl) bound_rels: Vec<String>,
}

impl<'s> Kept<'s> {
    pub(in crate::code_rule_dsl) fn new(store: &'s ContentStore, space: [u8; 32], inputs: &[(String, Arc<dyn Source>)], types: HashMap<String, Vec<u8>>) -> Self {
        Kept {
            store,
            space,
            rels: HashMap::new(),
            was: HashMap::new(),
            bound: None,
            inputs: inputs.iter().map(|(r, s)| (r.clone(), Arc::clone(s))).collect(),
            loaded: Default::default(),
            digests: HashMap::new(),
            types,
            puts: Vec::new(),
            values: 0,
            written_rows: 0,
            strata: None,
            bound_rels: Vec::new(),
        }
    }

    /// Kept under `prog`'s frame types (`retain::types_of`), as `load` reads them: a kind or field column is no atom.
    /// An input's types given to `new` stand.
    pub(in crate::code_rule_dsl) fn typed(mut self, prog: &crate::code_rule_dsl::logic::plan::body::Placed, domains: &super::lower::Domains) -> Self {
        let Ok(fields) = prog.fields() else { return self };
        for (rel, _) in fields.decls_by_name() {
            self.types.entry(rel.to_owned()).or_insert_with(|| super::retain::types_of(prog, domains, rel));
        }
        self
    }

    fn flush(&mut self) {
        let puts = std::mem::take(&mut self.puts);
        self.values = 0;
        // Nothing put or forgotten: nothing written (decision 0227).
        if !puts.is_empty() {
            let _span = crate::code_rule_dsl::logic::profile::span(|| "keep the strata's entries, queued".into());
            self.store.queue_kept(self.space, puts);
        }
    }

    /// Writes what is left.
    pub(in crate::code_rule_dsl) fn finish(mut self) {
        if self.rels.is_empty() {
            return;
        }
        self.flush();
    }
}

impl KeptStrata for Kept<'_> {
    fn program(&mut self, identities: &HashMap<String, [u8; 32]>) {
        self.rels = identities.iter().map(|(name, ident)| (name.clone(), KeptRel { name: name.clone(), ident: *ident })).collect();
        let _span = crate::code_rule_dsl::logic::profile::span(|| "the strata kept, from the store".into());
        let rels: Vec<KeptRel> = self.rels.values().cloned().collect();
        // One commit of the space's blocks, read before the first wave: what the cut reads and the rows bound agree.
        self.bound = Bound::of(self.store, &self.space).ok().flatten();
        self.was = stored_of(self.bound.as_ref(), &rels, 0).into_iter().map(|(r, s)| (r.name, s)).collect();
    }

    fn input(&mut self, rel: &str) -> Option<u64> {
        if let Some(d) = self.digests.get(rel) {
            return Some(*d);
        }
        let source = self.inputs.get(rel).cloned().or_else(|| self.loaded.borrow().get(rel).cloned())?;
        let atoms = self.store.atoms_handle();
        let width = source.columns().len();
        let cells = self.types.get(rel).filter(|t| t.len() == width).map_or(crate::code_rule_dsl::logic::plan::Cells::plain(width), |t| crate::code_rule_dsl::logic::plan::Cells::new(&atoms, t));
        let d = super::retain::digest(source.as_ref(), &cells);
        self.digests.insert(rel.to_owned(), d);
        Some(d)
    }

    fn was(&self, rel: &str) -> Option<u64> {
        self.was.get(rel).map(|w| w.digest)
    }

    fn read_was(&self, rel: &str, read: &str) -> Option<u64> {
        self.was.get(rel)?.reads.iter().find(|(r, _)| r == read).map(|(_, d)| *d)
    }

    fn has_rows(&self, rel: &str) -> bool {
        self.was.contains_key(rel)
    }

    fn rows(&mut self, rel: &str, types: &[u8]) -> Option<(Vec<u8>, Arc<crate::code_rule_dsl::logic::native::component::InRel>)> {
        let name = self.rels.get(rel)?;
        let entry = self.bound.as_ref()?.entry(&name.name, &name.ident, 0).filter(|e| self.was.get(rel).is_some_and(|w| w.digest == e.digest()))?;
        let held = entry.rel_as(types.to_vec(), &self.store.atoms_handle())?;
        self.bound_rels.push(rel.to_owned());
        Some((entry.types(), held))
    }

    fn counted(&mut self, evaluated: usize, of: usize) {
        self.strata = Some((evaluated, of));
    }

    fn ran(&mut self, rel: &str, got: Option<(u64, Vec<u8>, Vector<i64>, usize, Vec<(String, u64)>)>) {
        let Some(frame) = self.rels.get(rel).cloned() else { return };
        match got {
            Some((digest, types, rows, n, reads)) => {
                if self.was.get(rel).is_some_and(|w| w.digest == digest && w.reads == reads) {
                    return;
                }
                let types = self.types.get(rel).filter(|t| t.len() == types.len()).cloned().unwrap_or(types);
                self.values += rows.len();
                self.written_rows += n;
                self.puts.push(flat_put(&frame, digest, reads, types, rows, n));
                if self.values >= FLUSH_VALUES {
                    self.flush();
                }
            }
            None => {
                if self.was.contains_key(rel) {
                    self.puts.push(KeptPut { rel: frame.name, ident: frame.ident, part: 0, digest: 0, reads: Vec::new(), types: Vec::new(), what: KeptWhat::Gone });
                }
            }
        }
    }
}

#[cfg(test)]
pub mod tests;

/// The cross-file part a new process maintains as a session does (t32 4.1): what `retain::after` keeps, kept in
/// the store's frames under each relation's identity, and read back as the next process's `Retained`. Its strata
/// whose reads changed are maintained from those rows (`maintain::rewrite`), not derived again.
///
/// A relation's frame records the digest of each input its rows were derived from (the inputs its stratum reads, and
/// theirs, down to the program's inputs). Frames are shared by identity, so a relation another program kept may be
/// of other inputs than the inputs' frames hold now: it is read back only when every input it records has the
/// digest the input's frame holds. An input's frame is its program's (`retain::key`), not shared. A program output's rows are kept as any relation's, and its lines decoded from
/// them when its stratum is clean (`plan::decode_kept`); but `finding`, which SQL decodes, runs again.
pub(in crate::code_rule_dsl) struct Part {
    space: [u8; 32],
    /// The program it is of (`retain::key`): the next run's `Retained` is of it.
    key: [u8; 32],
    /// Each relation and input of the program, as its frame names it, and what the store holds of it.
    frames: HashMap<String, (KeptRel, Option<KeptStored>)>,
    /// The space's kept blocks as the run read them (decision 0401 F3).
    bound: Option<Arc<Bound>>,
    /// Each derived relation's inputs (`cone`).
    cones: HashMap<String, std::collections::BTreeSet<String>>,
    /// Each input's and relation's column types as its frame keeps them, from its declaration: `s` a symbol, which
    /// the store's writer maps to the store's atoms (decision 0170), else `i`.
    types: HashMap<String, Vec<u8>>,
}

/// Whether a new process maintains its cross-file part from the store (`RERAIL_KEPT_PART=off`: the per-stratum
/// cut of `Kept` instead, to compare).
pub(in crate::code_rule_dsl) fn part_wanted() -> bool {
    std::env::var("RERAIL_KEPT_PART").map_or(true, |v| v != "off")
}

/// Each derived relation of `prog` and the inputs (of `inputs`) it is derived from, through the strata it reads.
fn cones(prog: &crate::code_rule_dsl::logic::plan::body::Placed, sccs: &crate::code_rule_dsl::logic::front::Components, inputs: &std::collections::HashSet<&str>) -> super::Result<HashMap<String, std::collections::BTreeSet<String>>> {
    let mut out: HashMap<String, std::collections::BTreeSet<String>> = HashMap::new();
    // What each stratum reads: its rules' body atoms, its natives' reads, and the tree a `nearest` climbs.
    let reads = super::retain::Reads::of(prog)?;
    for scc in sccs {
        let mut cone = std::collections::BTreeSet::new();
        for r in reads.of_scc(scc) {
            if scc.contains(&r) {
                continue;
            }
            if inputs.contains(r.as_str()) {
                cone.insert(r);
            } else if let Some(c) = out.get(&r) {
                cone.extend(c.iter().cloned());
            }
        }
        for rel in scc {
            out.insert(rel.to_owned(), cone.to_owned());
        }
    }
    Ok(out)
}

/// What the store keeps of program `prog` (of `key` and `binds`, `retain::keys`) at `root`, as a `Retained`; `inputs` the run's,
/// whose columns an input's kept rows are read under; `sccs` its strata, computed once by the caller.
#[allow(clippy::too_many_arguments)]
pub(in crate::code_rule_dsl) fn load(
    store: &ContentStore,
    root: &Path,
    key: [u8; 32],
    binds: [u8; 32],
    prog: &crate::code_rule_dsl::logic::plan::body::Placed,
    domains: &super::lower::Domains,
    sccs: &crate::code_rule_dsl::logic::front::Components,
    inputs: &[(String, Arc<dyn Source>)],
    now: &HashMap<String, u64>,
) -> super::Result<(Option<super::retain::Retained>, Part)> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "the cross-file part kept, from the store".into());
    // The fact space of the corpus at `root` that a maintained cross-file part is kept in.
    let space = part_space(root, "cross-maintained");
    let input_names: std::collections::HashSet<&str> = inputs.iter().map(|(r, _)| r.as_str()).collect();
    let cones = cones(prog, sccs, &input_names)?;
    // The program's version, for its identities and its inputs' leaves.
    let leaves = crate::code_rule_dsl::logic::plan::Leaves::of_placed(prog.clone());
    let mut frames: HashMap<String, (KeptRel, Option<KeptStored>)> = leaves.identities_by_heads(sccs)?
        .into_iter()
        .filter(|(name, _)| name != "finding" && !input_names.contains(name.as_str()))
        .map(|(name, ident)| (name.clone(), (KeptRel { name, ident }, None)))
        .collect();
    // An input's rows are its program's: programs over the same root bind inputs of the same name to other rows
    // (P3's cross part and its loading cone, a phase), so its frame is under the program's key too: the key of
    // the part's program alone (`binds`, `retain::keys`), which an edit of a file-local rule leaves.
    for (rel, _) in inputs {
        let leaf = leaves.identity(rel).ok_or_else(|| super::Error::Plan("the program has no image: its leaves are read from its version".into()))?;
        let mut h = Sha256::new();
        h.update(b"rerail.kept-input/v1\0");
        h.update(binds);
        h.update(leaf);
        frames.insert(rel.clone(), (KeptRel { name: rel.clone(), ident: h.finalize().into() }, None));
    }
    let asked: Vec<KeptRel> = frames.values().map(|(f, _)| f.clone()).collect();
    let bound = Bound::of(store, &space).map_err(|e| super::Error::Plan(e.to_string()))?;
    for (f, s) in stored_of(bound.as_ref(), &asked, 0) {
        if let Some(e) = frames.get_mut(&f.name) {
            e.1 = Some(s);
        }
    }
    let types = frames.keys().map(|rel| (rel.clone(), super::retain::types_of(prog, domains, rel))).collect();
    load_from_snapshot(store, key, inputs, now, Part { space, key, frames, bound, cones, types })
}

fn load_from_snapshot(
    store: &ContentStore, key: [u8; 32], inputs: &[(String, Arc<dyn Source>)],
    now: &HashMap<String, u64>, mut part: Part,
) -> super::Result<(Option<super::retain::Retained>, Part)> {
    let kept = restore(store, key, inputs, now, &part);
    if kept.is_none() {
        // Publication after the snapshot, a missing payload, or bad rows: derive again and republish.
        // Keeping the old metadata here could make `keep` skip a required replacement write.
        for (_, was) in part.frames.values_mut() { *was = None; }
    }
    Ok((kept, part))
}

fn restore(
    store: &ContentStore, key: [u8; 32], inputs: &[(String, Arc<dyn Source>)],
    now: &HashMap<String, u64>, part: &Part,
) -> Option<super::retain::Retained> {
    if part.frames.values().all(|(_, s)| s.is_none()) { return None; }
    let bound = part.bound.as_ref()?;
    let atoms = store.atoms_handle();
    let input_names: std::collections::HashSet<&str> = inputs.iter().map(|(r, _)| r.as_str()).collect();
    // Each kept input bound where it lies (decision 0401 F3), named as the run's source of it.
    let mut kept_inputs = Vec::new();
    for (rel, source) in inputs {
        let Some((f, Some(was))) = part.frames.get(rel) else { continue };
        let entry = bound.entry(&f.name, &f.ident, 0)?;
        let names = source.columns();
        // Kept under other column types (untyped, before decision 0170): its symbols may be another numbering's.
        if names.len() != entry.width() || part.types.get(rel).is_none_or(|t| *t != entry.types()) { return None; }
        let held = entry.rel(&atoms)?;
        kept_inputs.push((rel.clone(), was.digest, held.source(names)));
    }
    // Inputs with no old entry are known by their current digest; no old rows are asserted for them.
    let mut input_digest: HashMap<&str, u64> = kept_inputs.iter().map(|(r, d, _)| (r.as_str(), *d)).collect();
    let mut unkept = Vec::new();
    for (rel, _) in inputs {
        if !input_digest.contains_key(rel.as_str()) && let Some(d) = now.get(rel) {
            input_digest.insert(rel.as_str(), *d);
            unkept.push((rel.clone(), *d));
        }
    }
    let mut rels = Vec::new();
    let mut stale = 0usize;
    for (name, (f, was)) in &part.frames {
        let Some(was) = was.as_ref().filter(|_| !input_names.contains(name.as_str())) else { continue };
        let current = part.cones.get(name).into_iter().flatten().all(|i|
            input_digest.get(i.as_str()).is_some_and(|d| was.reads.iter().any(|(r, x)| r == i && x == d)));
        if !current {
            stale += 1;
            continue;
        }
        let entry = bound.entry(&f.name, &f.ident, 0)?;
        // Kept under other column types (untyped, before decision 0170): its symbols may be another numbering's.
        if part.types.get(&f.name).is_none_or(|t| *t != entry.types()) { return None; }
        rels.push((f.name.clone(), entry.width(), entry.rel(&atoms)?));
    }
    crate::code_rule_dsl::logic::profile::decision("cross-file part kept across processes (t32 4.1)", || {
        format!("{} relations and {} inputs bound where they lie, {} rows; {stale} kept of other inputs, not read", rels.len(), kept_inputs.len(), rels.iter().map(|(_, _, r)| r.rows()).sum::<usize>())
    });
    Some(super::retain::Retained::of_rows(key, kept_inputs, unkept, rels))
}

/// Writes what `next` keeps whose rows or inputs differ from what the store holds, behind the run, and marks every
/// relation of the program used.
pub(in crate::code_rule_dsl) fn keep(store: &ContentStore, part: Part, next: &super::retain::Retained) {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "keep the cross-file part's entries, queued".into());
    let (inputs, rels) = next.parts();
    let digests: HashMap<&str, u64> = inputs.iter().map(|(r, d, _)| (*r, *d)).collect();
    let mut puts = Vec::new();
    let mut written = 0usize;
    for (rel, digest, source) in inputs {
        let Some((f, was)) = part.frames.get(rel) else { continue };
        if was.as_ref().is_some_and(|w| w.digest == digest) {
            continue;
        }
        let w = source.columns().len();
        // Its symbols typed, so the writer maps them; an input of no declared width is not kept.
        let Some(types) = part.types.get(rel).filter(|t| t.len() == w).cloned() else { continue };
        let n = source.len();
        // copy: not block data: an input's rows gathered for its entry's copy (a source the run did not hold as a
        // relation of a block), a batch at a time.
        let mut rows: Vector<i64> = rerail_blocks::vector::Vector::with_capacity(n * w);
        if crate::code_rule_dsl::logic::native::component::each_held_batch(source.as_ref(), w, &mut |b: &[i64]| rows.extend_from_slice(b)).is_none() {
            let all: Vector<usize> = (0..w).collect();
            let mut row = rerail_blocks::vector![0i64; w];
            for i in 0..n {
                source.row_at(i, &all, &mut row);
                rows.extend_from_slice(&row);
            }
        }
        written += n;
        puts.push(flat_put(f, digest, Vec::new(), types, rows, n));
    }
    for (rel, width, held) in rels {
        let Some((f, was)) = part.frames.get(rel) else { continue };
        // The inputs it was derived from, each with its digest: one not digested this run leaves it unkept.
        let Some(reads) = part.cones.get(rel).and_then(|c| c.iter().map(|i| Some((i.clone(), *digests.get(i.as_str())?))).collect::<Option<Vec<(String, u64)>>>()) else { continue };
        let n = held.rows();
        // Its symbols typed as declared (the engine holds them as integers), so the writer maps them; its digest of
        // its content (decision 0212), read where its rows lie (`retain::held_digest`).
        let Some(types) = part.types.get(rel).filter(|t| t.len() == width).cloned() else { continue };
        let Some(digest) = super::retain::held_digest(&crate::code_rule_dsl::logic::plan::Cells::new(&store.atoms_handle(), &types), held) else { continue };
        if was.as_ref().is_some_and(|w| w.digest == digest && w.reads == reads) {
            continue;
        }
        written += n;
        // Its rows lent whole, or a relation of no block of its rows as its live rows (`retain::kept_put`).
        puts.push(super::retain::kept_put(f, digest, reads, types, held));
    }
    crate::code_rule_dsl::logic::profile::decision("cross-file part kept across processes (t32 4.1)", || format!("{} entries, {written} rows, written to the store", puts.len()));
    // Nothing changed: nothing written (decision 0211).
    if !puts.is_empty() {
        store.queue_kept(part.space, puts);
    }
}
