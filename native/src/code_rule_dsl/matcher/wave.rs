//! Native operation cohorts over (program, document, preorder) lanes. No SQL, captures,
//! thread-local matcher context, or scalar AstMatcher callbacks enter these kernels.
use super::{AstMatcher, StopBy, factor::Key, relation::WalkFilter};
use crate::code_rule_dsl::tape::TapeDoc;
use rustc_hash::FxHashMap;
use std::{collections::BTreeMap, sync::Arc};

const NONE: u32 = u32::MAX;
const CACHE_ENTRIES: usize = 32_768;
pub(crate) const TILE: usize = 2048;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(C)]
pub(crate) struct Lane {
    pub program: u32,
    pub document: u32,
    pub node: u32,
    /// Bit zero preserves the caller's three-valued walk admission policy.
    pub flags: u32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Answer {
    pub matched: bool,
    pub steps: usize,
    pub levels: u32,
    pub incomplete: bool,
}
impl Answer {
    fn add(&mut self, other: Self) {
        self.steps = self.steps.saturating_add(other.steps);
        self.levels = self.levels.max(other.levels);
        self.incomplete |= other.incomplete;
    }
}
#[derive(Default, Clone, serde::Serialize)]
pub(crate) struct Metrics {
    pub submissions: u64,
    pub lanes: u64,
    pub unique_lanes: u64,
    pub kernel_dispatches: u64,
    pub kernel_lanes: u64,
    pub family_dispatches: [u64; 8],
    pub family_lanes: [u64; 8],
    pub max_memo_entries: usize,
    pub max_text_entries: usize,
    pub walk_advances: u64,
    pub closure_nodes: u64,
    pub closure_hits: u64,
    pub posting_builds: u64,
    pub posting_nodes: u64,
    pub posting_seeks: u64,
    pub subtree_materializations: u64,
    pub subtree_nodes: u64,
    pub subtree_hits: u64,
    pub utility_hits: u64,
    pub text_hits: u64,
    pub text_evaluations: u64,
    pub literal_evaluations: u64,
    pub max_kernel_width: usize,
    pub multi_document_submissions: u64,
    pub multi_program_submissions: u64,
    pub max_documents: usize,
    pub max_programs: usize,
}
#[derive(Clone, Copy)]
enum Stop {
    Neighbor,
    End,
    Rule(u32),
}
#[derive(Clone)]
struct Walk {
    axis: u8,
    child: u32,
    stop: Stop,
    filter: Option<Arc<WalkFilter>>,
    count: Option<(usize, Option<usize>)>,
}
enum Text {
    Literal {
        bytes: Box<[u8]>,
        start: bool,
        end: bool,
    },
    Regex(regex::Regex),
}
impl Text {
    fn lower(regex: &regex::Regex) -> Self {
        use regex_syntax::hir::{HirKind, Look};
        if let Ok(hir) = regex_syntax::Parser::new().parse(regex.as_str()) {
            let parts = match hir.kind() {
                HirKind::Concat(v) => v.as_slice(),
                _ => std::slice::from_ref(&hir),
            };
            let (mut start, mut end, mut literal) = (false, false, None);
            let mut valid = true;
            for (i, part) in parts.iter().enumerate() {
                match part.kind() {
                    HirKind::Look(Look::Start) if i == 0 => start = true,
                    HirKind::Look(Look::End) if i + 1 == parts.len() => end = true,
                    HirKind::Literal(l) if literal.is_none() => literal = Some(l.0.clone()),
                    _ => valid = false,
                }
            }
            if valid && let Some(bytes) = literal {
                return Self::Literal { bytes, start, end };
            }
        }
        Self::Regex(regex.clone())
    }
    fn holds(&self, text: &str) -> bool {
        match self {
            Self::Literal { bytes, start, end } => match (*start, *end) {
                (true, true) => text.as_bytes() == bytes.as_ref(),
                (true, false) => text.as_bytes().starts_with(bytes),
                (false, true) => text.as_bytes().ends_with(bytes),
                (false, false) => memchr::memmem::find(text.as_bytes(), bytes).is_some(),
            },
            Self::Regex(r) => r.is_match(text),
        }
    }
}
enum Op {
    Never,
    Kind(Vec<u16>),
    Text(Text),
    All(Vec<u32>),
    Any(Vec<u32>),
    Not(u32),
    Call(u32),
    Walk(Walk),
}
impl Op {
    fn family(&self) -> usize {
        match self {
            Self::Never => 0,
            Self::Kind(_) => 1,
            Self::Text(_) => 2,
            Self::All(_) => 3,
            Self::Any(_) => 4,
            Self::Not(_) => 5,
            Self::Call(_) => 6,
            Self::Walk(_) => 7,
        }
    }
}
#[derive(Default)]
pub(crate) struct Programs {
    ops: Vec<Op>,
    keys: BTreeMap<Key, u32>,
    guards: BTreeMap<Arc<[u8]>, u32>,
}
impl Programs {
    pub fn lower(&mut self, matcher: &AstMatcher) -> Option<u32> {
        if let AstMatcher::All(v) | AstMatcher::Any(v) = matcher
            && v.len() == 1
        {
            return self.lower(&v[0]);
        }
        let key = super::factor::key(matcher)?;
        if let Some(id) = self.keys.get(&key) {
            return Some(*id);
        }
        let id = self.ops.len() as u32;
        self.keys.insert(key, id);
        self.ops.push(Op::Never); // Recursive utilities reference this reserved program.
        let op = match matcher {
            AstMatcher::Never => Op::Never,
            AstMatcher::Kind(_) => {
                let Key::Kinds(k) = super::factor::key(matcher)? else {
                    unreachable!()
                };
                Op::Kind(k)
            }
            AstMatcher::Regex(r) => Op::Text(Text::lower(r)),
            AstMatcher::All(v) => Op::All(v.iter().map(|m| self.lower(m)).collect::<Option<_>>()?),
            AstMatcher::Any(v) => Op::Any(v.iter().map(|m| self.lower(m)).collect::<Option<_>>()?),
            AstMatcher::Not(m) => Op::Not(self.lower(m)?),
            AstMatcher::Util(u) => Op::Call(u.with_target(|m| self.lower(m))??),
            AstMatcher::Count(c) if c.binding.is_none() => {
                let mut w = self.walk(&c.relation)?;
                w.count = Some((c.min, c.max));
                w.filter = None;
                Op::Walk(w)
            }
            _ => Op::Walk(self.walk(matcher)?),
        };
        self.ops[id as usize] = op;
        Some(id)
    }
    /// A false answer proves that native execution stops before any opaque operation.
    /// A true answer only admits the original program; it never publishes a finding.
    /// In particular we cannot negate a partial guard or move it past an opaque operation.
    pub fn admission(&mut self, matcher: &AstMatcher) -> Option<u32> {
        self.admission_at(matcher, 0)
    }
    fn admission_at(&mut self, matcher: &AstMatcher, depth: usize) -> Option<u32> {
        if depth > 64 {
            return None;
        }
        if let Some(program) = self.lower(matcher) {
            return Some(program);
        }
        let op = match matcher {
            AstMatcher::All(terms) => {
                let mut children = vec![];
                for term in terms {
                    if let Some(program) = self.admission_at(term, depth + 1) {
                        children.push(program);
                    }
                    if super::factor::key(term).is_none() {
                        break;
                    }
                }
                if children.is_empty() {
                    return None;
                }
                if children.len() == 1 {
                    return Some(children[0]);
                }
                Op::All(children)
            }
            AstMatcher::Any(terms) => {
                // One branch without a negative proof admits every root. Do not speculate.
                let children = terms
                    .iter()
                    .map(|m| self.admission_at(m, depth + 1))
                    .collect::<Option<Vec<_>>>()?;
                Op::Any(children)
            }
            AstMatcher::Util(u) => {
                let definition = u
                    .table
                    .upgrade()?
                    .origins
                    .get(u.index)?
                    .as_ref()?
                    .definition
                    .clone();
                if let Some(id) = self.guards.get(&definition) {
                    return Some(*id);
                }
                let child = u.with_target(|m| self.admission_at(m, depth + 1))??;
                let id = self.ops.len() as u32;
                self.ops.push(Op::Call(child));
                self.guards.insert(definition, id);
                return Some(id);
            }
            AstMatcher::Constrained(c) => return self.admission_at(&c.rule, depth + 1),
            _ => return None,
        };
        let id = self.ops.len() as u32;
        self.ops.push(op);
        Some(id)
    }
    fn local_zero_cost(&self, program: u32) -> bool {
        match &self.ops[program as usize] {
            Op::Never | Op::Kind(_) | Op::Text(_) => true,
            Op::All(v) | Op::Any(v) => v.iter().all(|&p| self.local_zero_cost(p)),
            Op::Not(p) => self.local_zero_cost(*p),
            _ => false,
        }
    }
    fn walk(&mut self, m: &AstMatcher) -> Option<Walk> {
        let (axis, r, s) = match m {
            AstMatcher::Has(r, s) => (0, r, s),
            AstMatcher::Inside(r, s) => (1, r, s),
            AstMatcher::Precedes(r, s) => (2, r, s),
            AstMatcher::Follows(r, s) => (3, r, s),
            _ => return None,
        };
        let stop = match s {
            StopBy::Neighbor => Stop::Neighbor,
            StopBy::End => Stop::End,
            StopBy::Rule(m) => Stop::Rule(self.lower(m)?),
        };
        Some(Walk {
            axis,
            child: self.lower(r)?,
            stop,
            filter: WalkFilter::for_batch(r),
            count: None,
        })
    }
}

struct SubtreeFacts {
    domain: Vec<std::ops::Range<u32>>,
    hits: Vec<u32>,
}
impl SubtreeFacts {
    fn covers(&self, range: &std::ops::Range<u32>) -> bool {
        let at = self.domain.partition_point(|r| r.end <= range.start);
        self.domain
            .get(at)
            .is_some_and(|r| r.start <= range.start && r.end >= range.end)
    }
    fn matched(&self, range: &std::ops::Range<u32>) -> bool {
        let at = self.hits.partition_point(|&node| node < range.start);
        self.hits.get(at).is_some_and(|&node| node < range.end)
    }
}
pub(crate) struct Executor<'a> {
    programs: &'a Programs,
    documents: Vec<&'a TapeDoc>,
    memo: FxHashMap<Lane, Answer>,
    text: FxHashMap<(u32, &'a str), bool>,
    cancelled: Option<&'a dyn Fn(u32) -> bool>,
    kind_index: Option<&'a dyn Fn(u32, &[u16]) -> Vec<u32>>,
    postings: FxHashMap<(u32, u32), Arc<[u32]>>,
    posting_nodes: usize,
    subtrees: FxHashMap<(u32, u32), SubtreeFacts>,
    subtree_nodes: usize,
    poll_epoch: u64,
    document_poll: Vec<(u64, bool)>,
    pub metrics: Metrics,
}
impl<'a> Executor<'a> {
    pub fn new(programs: &'a Programs, documents: Vec<&'a TapeDoc>) -> Self {
        Self {
            programs,
            document_poll: vec![(0, false); documents.len()],
            poll_epoch: 0,
            kind_index: None,
            postings: FxHashMap::default(),
            posting_nodes: 0,
            subtrees: FxHashMap::default(),
            subtree_nodes: 0,
            documents,
            memo: FxHashMap::default(),
            text: FxHashMap::default(),
            cancelled: None,
            metrics: Metrics::default(),
        }
    }
    pub fn cancellation(&mut self, poll: &'a dyn Fn(u32) -> bool) {
        self.cancelled = Some(poll);
    }
    /// A native fact provider can supply an existing kind index; SQLite is not part of this API.
    pub fn kind_index(&mut self, index: &'a dyn Fn(u32, &[u16]) -> Vec<u32>) {
        self.kind_index = Some(index);
    }
    fn postings(&mut self, lane: Lane, walk: &Walk) -> Result<Arc<[u32]>, ()> {
        let key = (lane.document, lane.program);
        if let Some(found) = self.postings.get(&key) {
            return Ok(found.clone());
        }
        let filter = walk.filter.as_ref().unwrap();
        let doc = self.documents[lane.document as usize];
        let nodes = match self.kind_index {
            Some(index) => index(lane.document, &filter.kinds),
            None => {
                let mut found = vec![];
                for node in 0..doc.transducer_len() as u32 {
                    if node % 1024 == 0 && self.cancelled.is_some_and(|f| f(lane.document)) {
                        return Err(());
                    }
                    if filter
                        .kinds
                        .binary_search(&doc.transducer_kind(node))
                        .is_ok()
                    {
                        found.push(node);
                    }
                }
                found
            }
        };
        if self.cancelled.is_some_and(|f| f(lane.document)) {
            return Err(());
        }
        let mut selected = vec![];
        for (i, node) in nodes.into_iter().enumerate() {
            if i % 1024 == 0 && self.cancelled.is_some_and(|f| f(lane.document)) {
                return Err(());
            }
            if filter
                .needle
                .as_ref()
                .is_none_or(|n| n.holds(doc.transducer_text_str(node)))
            {
                selected.push(node);
            }
        }
        self.metrics.posting_builds += 1;
        self.metrics.posting_nodes += selected.len() as u64;
        let selected: Arc<[u32]> = selected.into();
        if selected.len() <= 131_072 {
            if self.posting_nodes + selected.len() > 131_072 {
                self.postings.clear();
                self.posting_nodes = 0;
            }
            self.posting_nodes += selected.len();
            self.postings.insert(key, selected.clone());
        }
        Ok(selected)
    }
    /// One bounded submission can mix both programs and documents. Results retain lane order.
    pub fn execute(&mut self, lanes: &[Lane]) -> Vec<Answer> {
        assert!(lanes.len() <= TILE);
        self.metrics.submissions += 1;
        self.metrics.lanes += lanes.len() as u64;
        let docs = lanes
            .iter()
            .map(|l| l.document)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let programs = lanes
            .iter()
            .map(|l| l.program)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        self.metrics.max_documents = self.metrics.max_documents.max(docs);
        self.metrics.max_programs = self.metrics.max_programs.max(programs);
        self.metrics.multi_document_submissions += u64::from(docs > 1);
        self.metrics.multi_program_submissions += u64::from(programs > 1);
        // Documents and programs are immutable for this executor. Reuse facts across tiles,
        // with a fixed capacity; dropping the executor invalidates the entire revision.
        if self.memo.len() >= CACHE_ENTRIES {
            self.memo.clear();
        }
        if self.text.len() >= CACHE_ENTRIES {
            self.text.clear();
        }
        let mut unique = vec![];
        let mut owners = FxHashMap::default();
        let mut slots = vec![];
        for &l in lanes {
            let key = l;
            let next = unique.len();
            let id = *owners.entry(key).or_insert_with(|| {
                unique.push(l);
                next
            });
            slots.push(id);
        }
        self.metrics.unique_lanes += unique.len() as u64;
        let results = self.eval(&unique, 0);
        self.metrics.max_memo_entries = self.metrics.max_memo_entries.max(self.memo.len());
        self.metrics.max_text_entries = self.metrics.max_text_entries.max(self.text.len());
        slots.into_iter().map(|i| results[i]).collect()
    }
    fn poll_document(&mut self, document: u32) -> bool {
        let poll = &mut self.document_poll[document as usize];
        if poll.0 != self.poll_epoch {
            *poll = (self.poll_epoch, self.cancelled.is_some_and(|f| f(document)));
        }
        poll.1
    }
    fn eval(&mut self, lanes: &[Lane], depth: u32) -> Vec<Answer> {
        if lanes.is_empty() {
            return vec![];
        }
        if depth > 512 {
            return vec![
                Answer {
                    incomplete: true,
                    ..Default::default()
                };
                lanes.len()
            ];
        }
        let mut out = vec![Answer::default(); lanes.len()];
        self.poll_epoch += 1;
        let mut cohorts: [Vec<usize>; 8] = std::array::from_fn(|_| vec![]);
        for (i, l) in lanes.iter().enumerate() {
            if self.poll_document(l.document) {
                out[i].incomplete = true
            } else {
                cohorts[self.programs.ops[l.program as usize].family()].push(i)
            }
        }
        for (family, ids) in cohorts
            .into_iter()
            .enumerate()
            .filter(|(_, v)| !v.is_empty())
        {
            self.metrics.kernel_dispatches += 1;
            self.metrics.kernel_lanes += ids.len() as u64;
            self.metrics.family_dispatches[family] += 1;
            self.metrics.family_lanes[family] += ids.len() as u64;
            self.metrics.max_kernel_width = self.metrics.max_kernel_width.max(ids.len());
            match family {
                0 => {}
                1 => {
                    for i in ids {
                        let l = lanes[i];
                        let Op::Kind(k) = &self.programs.ops[l.program as usize] else {
                            unreachable!()
                        };
                        out[i].matched = k
                            .binary_search(
                                &self.documents[l.document as usize].transducer_kind(l.node),
                            )
                            .is_ok();
                    }
                }
                2 => {
                    for i in ids {
                        let l = lanes[i];
                        let Op::Text(r) = &self.programs.ops[l.program as usize] else {
                            unreachable!()
                        };
                        let text = self.documents[l.document as usize].transducer_text_str(l.node);
                        out[i].matched = if matches!(r, Text::Literal { .. }) {
                            self.metrics.literal_evaluations += 1;
                            r.holds(text)
                        } else if text.len() <= 128 {
                            if let Some(v) = self.text.get(&(l.program, text)) {
                                self.metrics.text_hits += 1;
                                *v
                            } else {
                                self.metrics.text_evaluations += 1;
                                let v = r.holds(text);
                                if self.text.len() < CACHE_ENTRIES {
                                    self.text.insert((l.program, text), v);
                                }
                                v
                            }
                        } else {
                            self.metrics.text_evaluations += 1;
                            r.holds(text)
                        };
                    }
                }
                3 | 4 => {
                    let all = family == 3;
                    let mut active = ids;
                    let mut pos = 0;
                    for &i in &active {
                        out[i].matched = all;
                    }
                    while !active.is_empty() {
                        let mut pending = vec![];
                        let mut next = vec![];
                        for i in active {
                            let children = match &self.programs.ops[lanes[i].program as usize] {
                                Op::All(v) | Op::Any(v) => v,
                                _ => unreachable!(),
                            };
                            if let Some(&program) = children.get(pos) {
                                pending.push(Lane {
                                    program,
                                    ..lanes[i]
                                });
                                next.push(i);
                            }
                        }
                        if pending.is_empty() {
                            break;
                        }
                        let results = self.eval(&pending, depth + 1);
                        active = vec![];
                        for (i, r) in next.into_iter().zip(results) {
                            out[i].add(r);
                            out[i].matched = r.matched;
                            if !r.incomplete && r.matched == all {
                                active.push(i)
                            }
                        }
                        pos += 1;
                    }
                }
                5 | 6 => {
                    let call = family == 6;
                    let mut pending = vec![];
                    let mut next = vec![];
                    let mut owners = FxHashMap::default();
                    for i in ids {
                        let l = lanes[i];
                        if call && let Some(r) = self.memo.get(&l) {
                            self.metrics.utility_hits += 1;
                            out[i] = *r;
                            continue;
                        }
                        let program = match self.programs.ops[l.program as usize] {
                            Op::Call(p) | Op::Not(p) => p,
                            _ => unreachable!(),
                        };
                        let key = l;
                        let owner = if call {
                            if let Some(&owner) = owners.get(&key) {
                                self.metrics.utility_hits += 1;
                                owner
                            } else {
                                let owner = pending.len();
                                owners.insert(key, owner);
                                pending.push(Lane { program, ..l });
                                owner
                            }
                        } else {
                            let owner = pending.len();
                            pending.push(Lane { program, ..l });
                            owner
                        };
                        next.push((i, owner));
                    }
                    let results = self.eval(&pending, depth + 1);
                    for (i, owner) in next {
                        let mut r = results[owner];
                        if call {
                            r.steps = r.steps.saturating_add(1);
                            r.levels += 1;
                            r.incomplete |= r.levels > super::util::MAX_UTIL_DEPTH;
                            if !r.incomplete && self.memo.len() < CACHE_ENTRIES {
                                let l = lanes[i];
                                self.memo.insert(l, r);
                            }
                        } else {
                            r.matched = !r.matched;
                        }
                        out[i] = r;
                    }
                }
                7 => {
                    let (chains, walks): (Vec<_>, Vec<_>) = ids.into_iter().partition(|&i| {
                        matches!(&self.programs.ops[lanes[i].program as usize], Op::Walk(w) if w.axis != 0 && matches!(w.stop, Stop::End) && w.count.is_none())
                    });
                    self.chains(lanes, &chains, &mut out, depth);
                    self.walks(lanes, &walks, &mut out, depth);
                }
                _ => unreachable!(),
            }
        }
        out
    }
    /// Build the demanded suffix DAG once, rather than walking one chain per root.
    /// Each edge carries its child's logical cost; cache hits replay that cost, not its CPU work.
    fn chains(&mut self, lanes: &[Lane], ids: &[usize], out: &mut [Answer], depth: u32) {
        if ids.is_empty() {
            return;
        }
        struct State {
            key: Lane,
            next: Option<usize>,
            value: Answer,
            done: bool,
        }
        let mut states: Vec<State> = vec![];
        let mut index = FxHashMap::default();
        let mut frontier = vec![];
        let mut roots = vec![];
        for &i in ids {
            let lane = lanes[i];
            let Op::Walk(w) = &self.programs.ops[lane.program as usize] else {
                unreachable!()
            };
            let doc = self.documents[lane.document as usize];
            let first = if w.axis == 1 {
                doc.transducer_links(lane.node)[0]
            } else {
                doc.transducer_sibling_start(lane.node, w.axis == 2, false)
                    .unwrap_or(NONE)
            };
            if first == NONE {
                continue;
            }
            // Bit one identifies a structural suffix including this node, not a normalized
            // user root. Native sibling walks normalize only their initial position.
            let key = Lane {
                node: first,
                flags: lane.flags | 2,
                ..lane
            };
            roots.push((i, key));
            if let Some(value) = self.memo.get(&key) {
                self.metrics.closure_hits += 1;
                out[i] = *value;
            } else if !index.contains_key(&key) {
                let at = states.len();
                index.insert(key, at);
                states.push(State {
                    key,
                    next: None,
                    value: Answer::default(),
                    done: false,
                });
                frontier.push(at);
            }
        }
        while !frontier.is_empty() {
            if states.len() > 65_536 {
                for &i in ids {
                    out[i].incomplete = true;
                }
                return;
            }
            self.poll_epoch += 1;
            let mut pending = vec![];
            let mut destinations = vec![];
            let mut successors = vec![];
            for at in frontier {
                let key = states[at].key;
                self.metrics.closure_nodes += 1;
                self.metrics.walk_advances += 1;
                if self.poll_document(key.document) {
                    states[at].value.incomplete = true;
                    states[at].done = true;
                    continue;
                }
                if let Some(value) = self.memo.get(&key) {
                    self.metrics.closure_hits += 1;
                    states[at].value = *value;
                    states[at].done = true;
                    continue;
                }
                let Op::Walk(w) = &self.programs.ops[key.program as usize] else {
                    unreachable!()
                };
                let doc = self.documents[key.document as usize];
                let candidate = key.node;
                let next = doc.transducer_links(candidate)[match w.axis {
                    1 => 0,
                    2 => 2,
                    _ => 3,
                }];
                if next == NONE {
                    states[at].done = true;
                } else {
                    successors.push((at, Lane { node: next, ..key }));
                }
                if key.flags & 1 != 0
                    || w.filter.as_ref().is_none_or(|f| {
                        f.kinds
                            .binary_search(&doc.transducer_kind(candidate))
                            .is_ok()
                    })
                {
                    pending.push(Lane {
                        program: w.child,
                        node: candidate,
                        flags: key.flags & !2,
                        ..key
                    });
                    destinations.push(at);
                }
            }
            for (at, value) in destinations.into_iter().zip(self.eval(&pending, depth + 1)) {
                states[at].value = value;
                states[at].done |= value.matched || value.incomplete;
            }
            frontier = vec![];
            for (at, key) in successors {
                if states[at].done {
                    continue;
                }
                let next = if let Some(&next) = index.get(&key) {
                    next
                } else {
                    let next = states.len();
                    index.insert(key, next);
                    states.push(State {
                        key,
                        next: None,
                        value: Answer::default(),
                        done: false,
                    });
                    frontier.push(next);
                    next
                };
                states[at].next = Some(next);
            }
        }
        // Ancestor and directed sibling edges are acyclic. Resolve shared suffixes once.
        let mut path = vec![];
        for start in 0..states.len() {
            let mut at = start;
            while !states[at].done {
                path.push(at);
                at = states[at].next.expect("unfinished suffix has an edge");
            }
            let mut value = states[at].value;
            while let Some(at) = path.pop() {
                let mut own = states[at].value;
                own.add(value);
                own.matched = value.matched;
                states[at].value = own;
                states[at].done = true;
                value = own;
            }
        }
        for (i, key) in roots {
            if let Some(&at) = index.get(&key) {
                out[i] = states[at].value;
            }
        }
        for state in states {
            if !state.value.incomplete && self.memo.len() < CACHE_ENTRIES {
                self.memo.insert(state.key, state.value);
            }
        }
    }
    /// After repeated failed probes, materialize a zero-cost local predicate over only the
    /// demanded subtree union. Coverage is explicit: an uncovered range is never negative.
    fn subtree_batch(
        &mut self,
        lanes: &[Lane],
        ids: &[usize],
        walks: &[&Walk],
        cursor: &[u32],
        active: &[usize],
        out: &mut [Answer],
        depth: u32,
    ) -> Vec<bool> {
        let mut groups = BTreeMap::<(u32, u32), Vec<usize>>::new();
        for &j in active {
            let lane = lanes[ids[j]];
            let w = walks[j];
            if w.axis == 0
                && matches!(w.stop, Stop::End)
                && w.count.is_none()
                && (lane.flags & 1 != 0 || w.filter.is_none())
                && self.programs.local_zero_cost(w.child)
            {
                groups.entry((lane.document, w.child)).or_default().push(j);
            }
        }
        let mut done = vec![false; ids.len()];
        for (key, group) in groups {
            if self.cancelled.is_some_and(|f| f(key.0)) {
                for j in group {
                    out[ids[j]].incomplete = true;
                    done[j] = true;
                }
                continue;
            }
            let doc = self.documents[key.0 as usize];
            let demands = group
                .iter()
                .map(|&j| cursor[j]..doc.transducer_descendants(lanes[ids[j]].node).end)
                .collect::<Vec<_>>();
            if let Some(facts) = self
                .subtrees
                .get(&key)
                .filter(|f| demands.iter().all(|r| f.covers(r)))
            {
                self.metrics.subtree_hits += group.len() as u64;
                for (&j, range) in group.iter().zip(&demands) {
                    out[ids[j]].matched = facts.matched(range);
                    done[j] = true;
                }
                continue;
            }
            if group.len() < 16 {
                continue;
            }
            let probes = demands
                .iter()
                .map(|r| (r.end - r.start) as u64)
                .sum::<u64>();
            let mut ranges = demands.clone();
            ranges.sort_unstable_by_key(|r| r.start);
            let mut domain: Vec<std::ops::Range<u32>> = vec![];
            for range in ranges {
                if let Some(last) = domain.last_mut()
                    && range.start <= last.end
                {
                    last.end = last.end.max(range.end);
                } else {
                    domain.push(range);
                }
            }
            let nodes = domain.iter().map(|r| (r.end - r.start) as u64).sum::<u64>();
            if probes <= 4 * nodes {
                continue;
            }
            let mut facts = SubtreeFacts {
                domain,
                hits: vec![],
            };
            let mut pending = Vec::with_capacity(TILE);
            let mut incomplete = false;
            for range in &facts.domain {
                for node in range.clone() {
                    pending.push(Lane {
                        program: key.1,
                        document: key.0,
                        node,
                        flags: 0,
                    });
                    if pending.len() == TILE {
                        for (lane, answer) in pending.iter().zip(self.eval(&pending, depth + 1)) {
                            incomplete |= answer.incomplete;
                            if answer.matched {
                                facts.hits.push(lane.node);
                            }
                        }
                        self.metrics.subtree_nodes += pending.len() as u64;
                        self.metrics.walk_advances += pending.len() as u64;
                        pending.clear();
                        if incomplete {
                            break;
                        }
                    }
                }
                if incomplete {
                    break;
                }
            }
            if !incomplete {
                for (lane, answer) in pending.iter().zip(self.eval(&pending, depth + 1)) {
                    incomplete |= answer.incomplete;
                    if answer.matched {
                        facts.hits.push(lane.node);
                    }
                }
                self.metrics.subtree_nodes += pending.len() as u64;
                self.metrics.walk_advances += pending.len() as u64;
            }
            self.metrics.subtree_materializations += 1;
            for (&j, range) in group.iter().zip(&demands) {
                out[ids[j]].incomplete |= incomplete;
                out[ids[j]].matched = !incomplete && facts.matched(range);
                done[j] = true;
            }
            if !incomplete && facts.hits.len() <= 131_072 {
                if self.subtrees.len() >= 256 || self.subtree_nodes + facts.hits.len() > 131_072 {
                    self.subtrees.clear();
                    self.subtree_nodes = 0;
                }
                self.subtree_nodes += facts.hits.len();
                if let Some(previous) = self.subtrees.insert(key, facts) {
                    self.subtree_nodes -= previous.hits.len();
                }
            }
        }
        done
    }
    fn walks(&mut self, lanes: &[Lane], ids: &[usize], out: &mut [Answer], depth: u32) {
        if ids.is_empty() {
            return;
        }
        let programs = self.programs;
        let walks = ids
            .iter()
            .map(|&i| {
                let Op::Walk(w) = &programs.ops[lanes[i].program as usize] else {
                    unreachable!()
                };
                w
            })
            .collect::<Vec<_>>();
        let mut cursor = ids
            .iter()
            .zip(&walks)
            .map(|(&i, w)| {
                let l = lanes[i];
                let links = self.documents[l.document as usize].transducer_links(l.node);
                match w.axis {
                    0 => links[1],
                    1 => links[0],
                    _ => self.documents[l.document as usize]
                        .transducer_sibling_start(
                            l.node,
                            w.axis == 2,
                            matches!(w.stop, Stop::Neighbor),
                        )
                        .unwrap_or(NONE),
                }
            })
            .collect::<Vec<_>>();
        let mut indexed = vec![None; ids.len()];
        let mut local_postings: FxHashMap<(u32, u32), Result<Arc<[u32]>, ()>> =
            FxHashMap::default();
        for (j, (&i, w)) in ids.iter().zip(&walks).enumerate() {
            let lane = lanes[i];
            if w.axis == 0
                && matches!(w.stop, Stop::End)
                && w.count.is_none()
                && lane.flags & 1 == 0
                && w.filter.is_some()
            {
                let key = (lane.document, lane.program);
                let posting = if let Some(found) = local_postings.get(&key) {
                    found.clone()
                } else {
                    let found = self.postings(lane, w);
                    local_postings.insert(key, found.clone());
                    found
                };
                match posting {
                    Ok(nodes) => {
                        let range = self.documents[lane.document as usize]
                            .transducer_descendants(lane.node);
                        let start = nodes.partition_point(|&node| node < range.start);
                        let end = nodes.partition_point(|&node| node < range.end);
                        self.metrics.posting_seeks += 1;
                        cursor[j] = if start < end { nodes[start] } else { NONE };
                        indexed[j] = Some((nodes, start, end));
                    }
                    Err(()) => {
                        out[i].incomplete = true;
                        cursor[j] = NONE;
                    }
                }
            }
        }
        let mut found = vec![0usize; ids.len()];
        let mut active = (0..ids.len())
            .filter(|&j| cursor[j] != NONE)
            .collect::<Vec<_>>();
        let mut rounds = 0;
        while !active.is_empty() {
            rounds += 1;
            if rounds == 8 {
                let done = self.subtree_batch(lanes, ids, &walks, &cursor, &active, out, depth);
                active.retain(|&j| !done[j]);
            }
            self.poll_epoch += 1;
            active.retain(|&j| {
                if self.poll_document(lanes[ids[j]].document) {
                    out[ids[j]].incomplete = true;
                    false
                } else {
                    true
                }
            });
            self.metrics.walk_advances += active.len() as u64;
            let mut stop_values = vec![false; ids.len()];
            // Up/sibling stop predicates precede the child test; descendant stops follow it.
            self.stop_wave(
                lanes,
                ids,
                &walks,
                &cursor,
                &active,
                &mut stop_values,
                out,
                depth,
                false,
            );
            let mut pending = vec![];
            let mut destinations = vec![];
            for &j in &active {
                let i = ids[j];
                let l = lanes[i];
                let w = &walks[j];
                let doc = self.documents[l.document as usize];
                let filter = l.flags & 1 != 0
                    || w.filter.as_ref().is_none_or(|f| {
                        f.kinds
                            .binary_search(&doc.transducer_kind(cursor[j]))
                            .is_ok()
                            && (w.axis != 0
                                || !matches!(w.stop, Stop::End)
                                || f.needle
                                    .as_ref()
                                    .is_none_or(|n| n.holds(doc.transducer_text_str(cursor[j]))))
                    });
                if filter && !out[i].incomplete {
                    pending.push(Lane {
                        program: w.child,
                        node: cursor[j],
                        ..l
                    });
                    destinations.push(j);
                }
            }
            let results = self.eval(&pending, depth + 1);
            for (j, mut r) in destinations.into_iter().zip(results) {
                let i = ids[j];
                let w = &walks[j];
                if w.count.is_some() {
                    r.steps = r.steps.saturating_add(1)
                }
                out[i].add(r);
                if r.matched {
                    found[j] += 1;
                }
                if w.count.is_none() && r.matched {
                    out[i].matched = true;
                }
            }
            active.retain(|&j| {
                let i = ids[j];
                !out[i].incomplete
                    && !out[i].matched
                    && !walks[j].count.is_some_and(|(min, max)| {
                        max.map_or(found[j] > 0 && found[j] >= min, |m| found[j] > m)
                    })
            });
            self.stop_wave(
                lanes,
                ids,
                &walks,
                &cursor,
                &active,
                &mut stop_values,
                out,
                depth,
                true,
            );
            for &j in &active {
                let l = lanes[ids[j]];
                let w = &walks[j];
                let doc = self.documents[l.document as usize];
                if let Some((nodes, position, end)) = &mut indexed[j] {
                    *position += 1;
                    cursor[j] = if *position < *end {
                        nodes[*position]
                    } else {
                        NONE
                    };
                    continue;
                }
                let links = doc.transducer_links(cursor[j]);
                cursor[j] = match (w.axis, w.stop) {
                    (0, Stop::Neighbor) => links[2],
                    (0, _) => {
                        let next = if stop_values[j] {
                            doc.transducer_descendants(cursor[j]).end
                        } else {
                            cursor[j] + 1
                        };
                        if next < doc.transducer_descendants(l.node).end {
                            next
                        } else {
                            NONE
                        }
                    }
                    (_, Stop::Neighbor) => NONE,
                    _ if stop_values[j] => NONE,
                    (1, _) => links[0],
                    (2, _) => links[2],
                    _ => links[3],
                };
            }
            active.retain(|&j| cursor[j] != NONE && !out[ids[j]].incomplete);
        }
        for (j, &i) in ids.iter().enumerate() {
            if let Some((min, max)) = walks[j].count {
                out[i].matched = found[j] >= min && max.is_none_or(|m| found[j] <= m)
            }
        }
    }
    fn stop_wave(
        &mut self,
        lanes: &[Lane],
        ids: &[usize],
        walks: &[&Walk],
        cursor: &[u32],
        active: &[usize],
        stopped: &mut [bool],
        out: &mut [Answer],
        depth: u32,
        down: bool,
    ) {
        let mut pending = vec![];
        let mut destinations = vec![];
        for &j in active {
            if (walks[j].axis == 0) == down
                && let Stop::Rule(program) = walks[j].stop
            {
                let i = ids[j];
                pending.push(Lane {
                    program,
                    node: cursor[j],
                    ..lanes[i]
                });
                destinations.push(j)
            }
        }
        for (j, r) in destinations.into_iter().zip(self.eval(&pending, depth + 1)) {
            stopped[j] = r.matched;
            out[ids[j]].add(r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        CodeRuleDslRuntime,
        lines::{LineTable, Terminators},
        matcher,
        tape::{Grammar, TapeImage},
        test_support::{CAPABILITIES, grammar_matchers},
    };
    use ast_grep_core::meta_var::MetaVarEnv;
    use serde_json::json;
    use std::borrow::Cow;
    fn grammar(rule: serde_json::Value, utils: serde_json::Value) -> matcher::GrammarMatcher {
        let p=CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"waves","revision":1,"mode":"steer","clauses":[{"id":"c","forbid":{"ast":{"language":"javascript","rule":rule,"utils":utils}},"violation":{"code":"W","message":"wave","anchor":"witness.span"}}]}).to_string(),"json".into(),CAPABILITIES.into()).unwrap();
        grammar_matchers(&p)[0].clone()
    }
    #[test]
    fn shared_chain_closure_and_postings_bound_navigation_work() {
        let up = grammar(
            json!({"inside":{"stopBy":"end","kind":"number"}}),
            json!({}),
        );
        let down = grammar(json!({"has":{"stopBy":"end","kind":"number"}}), json!({}));
        let regex = grammar(json!({"has":{"stopBy":"end","regex":"^never$"}}), json!({}));
        let dialect = up.language;
        let source = format!("{}x;{}", "{".repeat(256), "}".repeat(256));
        let grammar = Grammar::new(dialect).unwrap();
        let image = TapeImage::parse(&grammar, &source, &|| false)
            .unwrap()
            .unwrap();
        let lines = Arc::new(LineTable::new(&source, Terminators::of(dialect)));
        let doc = TapeDoc::restore_with_lines(Arc::new(source), grammar, &image, lines, &|| false)
            .unwrap();
        let nodes = doc.transducer_len();
        let mut programs = Programs::default();
        let entries = [&up, &down, &regex].map(|g| programs.lower(&g.matcher).unwrap());
        let lanes = entries
            .into_iter()
            .flat_map(|program| {
                (0..nodes as u32).map(move |node| Lane {
                    program,
                    document: 0,
                    node,
                    flags: 0,
                })
            })
            .collect::<Vec<_>>();
        let mut executor = Executor::new(&programs, vec![&doc]);
        for tile in lanes.chunks(TILE) {
            assert!(
                executor
                    .execute(tile)
                    .iter()
                    .all(|r| !r.matched && !r.incomplete)
            );
        }
        assert!(executor.metrics.closure_nodes <= nodes as u64);
        assert!(
            executor.metrics.walk_advances <= 12 * nodes as u64,
            "navigation became quadratic"
        );
        assert!(executor.metrics.subtree_materializations > 0);
        assert!(executor.metrics.subtree_nodes <= 2 * nodes as u64);
        assert_eq!(executor.metrics.posting_builds, 1);
        assert_eq!(executor.metrics.posting_seeks, nodes as u64);
    }
    #[test]
    fn mixed_alternative_guards_only_reject_exact_negative_paths() {
        let g = grammar(
            json!({"any":[{"all":[{"matches":"number"},{"pattern":"$N"}]},{"all":[{"regex":"^f"},{"pattern":"f($X)"}]}]}),
            json!({"number":{"kind":"number"}}),
        );
        let dialect = g.language;
        let source = "f(x); g(1); h(y);";
        let grammar = Grammar::new(dialect).unwrap();
        let image = TapeImage::parse(&grammar, source, &|| false)
            .unwrap()
            .unwrap();
        let doc = TapeDoc::restore_with_lines(
            Arc::new(source.to_owned()),
            grammar,
            &image,
            Arc::new(LineTable::new(source, Terminators::of(dialect))),
            &|| false,
        )
        .unwrap();
        let root = ast_grep_core::AstGrep::doc(doc);
        let mut programs = Programs::default();
        assert!(programs.lower(&g.matcher).is_none());
        let program = programs
            .admission(&g.matcher)
            .expect("mixed branch admission");
        // Negating a partial proof would turn uncertainty into a false negative.
        assert!(
            programs
                .admission(&AstMatcher::Not(Box::new(g.matcher.clone())))
                .is_none()
        );
        let lanes = (0..root.root().get_doc().transducer_len() as u32)
            .map(|node| Lane {
                program,
                document: 0,
                node,
                flags: u32::from(g.three_valued),
            })
            .collect::<Vec<_>>();
        let answers = Executor::new(&programs, vec![root.root().get_doc()]).execute(&lanes);
        let mut rejected = 0;
        matcher::with_file_memo(&root, || {
            for (lane, answer) in lanes.iter().zip(answers) {
                matcher::begin_clause(&g, usize::MAX);
                let actual = matcher::fragment_at(
                    &g,
                    matcher::node_at(&root, lane.node),
                    &mut Cow::Owned(MetaVarEnv::new()),
                )
                .unwrap_or_else(|_| panic!("native stopped"));
                let left = matcher::end_clause().0;
                assert!(!answer.incomplete);
                if !answer.matched {
                    rejected += 1;
                    assert_eq!(actual, 0);
                    assert_eq!(answer.steps, usize::MAX - left);
                }
            }
        });
        assert!(rejected > lanes.len() / 2);
    }
    #[test]
    fn literal_lowering_preserves_regex_flags_and_anchors() {
        for expression in [
            "x",
            "^x",
            "x$",
            "^x$",
            "(?m)^x$",
            "(?i)x",
            "^é$",
            "x|y",
            "x.*",
            "^$",
            "\\bfoo\\b",
        ] {
            let regex = regex::Regex::new(expression).unwrap();
            let compiled = Text::lower(&regex);
            for text in ["", "x", "X", "xy", "yx", "x\n", "\nx", "é", "foo", "xfoo"] {
                assert_eq!(
                    compiled.holds(text),
                    regex.is_match(text),
                    "{expression:?} on {text:?}"
                );
            }
        }
    }
    #[test]
    fn cohorts_match_native_values_and_work_across_programs_documents_and_recursive_walks() {
        let rules = [
            json!({"kind":"identifier"}),
            json!({"regex":"^(x|f)$"}),
            json!({"all":[{"kind":"identifier"},{"not":{"regex":"^x$"}}]}),
            json!({"any":[{"regex":"^x$"},{"kind":"number"}]}),
            json!({"has":{"stopBy":"end","matches":"leaf"}}),
            json!({"inside":{"stopBy":{"kind":"statement_block"},"matches":"owner"}}),
            json!({"inside":{"stopBy":"end","matches":"owner"}}),
            json!({"precedes":{"stopBy":"end","matches":"leaf"}}),
            json!({"precedes":{"stopBy":"end","kind":"number"}}),
            json!({"follows":{"stopBy":{"regex":"^,$"},"matches":"leaf"}}),
            json!({"matches":"up"}),
            json!({"count":{"has":{"stopBy":"end","matches":"leaf"},"min":0,"max":0}}),
            json!({"count":{"has":{"stopBy":{"kind":"statement_block"},"kind":"identifier"},"min":1,"max":2}}),
        ];
        let utilities = json!({"leaf":{"kind":"identifier"},"owner":{"kind":"function_declaration"},"up":{"any":[{"kind":"function_declaration"},{"inside":{"stopBy":"neighbor","matches":"up"}}]}});
        let grammars = rules
            .iter()
            .flat_map(|r| {
                let a = grammar(r.clone(), utilities.clone());
                let mut b = a.clone();
                b.three_valued = true;
                [a, b]
            })
            .collect::<Vec<_>>();
        let docs = [
            "function f(x){ return f(x,1,2); }",
            "const x=2; f(x);",
            "function g(){ { f(y,3); } }",
            "f(x; const y = ;",
        ];
        let documents = docs
            .iter()
            .map(|source| {
                let dialect = grammars[0].language;
                let grammar = Grammar::new(dialect).unwrap();
                let image = TapeImage::parse(&grammar, source, &|| false)
                    .unwrap()
                    .unwrap();
                let lines = Arc::new(LineTable::new(source, Terminators::of(dialect)));
                let doc = TapeDoc::restore_with_lines(
                    Arc::new(source.to_string()),
                    grammar,
                    &image,
                    lines,
                    &|| false,
                )
                .unwrap();
                ast_grep_core::AstGrep::doc(doc)
            })
            .collect::<Vec<_>>();
        let mut programs = Programs::default();
        let entries = grammars
            .iter()
            .map(|g| programs.lower(&g.matcher).unwrap())
            .collect::<Vec<_>>();
        let mut lanes = vec![];
        let mut expected = vec![];
        for (file, root) in documents.iter().enumerate() {
            matcher::with_file_memo(root, || {
                for (g, &program) in grammars.iter().zip(&entries) {
                    for node in 0..root.root().get_doc().transducer_len() as u32 {
                        matcher::begin_clause(g, usize::MAX);
                        let verdict = matcher::fragment_at(
                            g,
                            matcher::node_at(root, node),
                            &mut Cow::Owned(MetaVarEnv::new()),
                        )
                        .unwrap_or_else(|_| panic!("native stopped"));
                        let left = matcher::end_clause().0;
                        lanes.push(Lane {
                            program,
                            document: file as u32,
                            node,
                            flags: u32::from(g.three_valued),
                        });
                        expected.push((verdict == 1, usize::MAX - left));
                    }
                }
            });
        }
        let mut executor = Executor::new(
            &programs,
            documents.iter().map(|f| f.root().get_doc()).collect(),
        );
        let actual = lanes
            .chunks(TILE)
            .flat_map(|tile| executor.execute(tile))
            .collect::<Vec<_>>();
        for (i, (answer, expected)) in actual.iter().zip(expected).enumerate() {
            assert!(!answer.incomplete, "lane {i}");
            assert_eq!(
                (answer.matched, answer.steps),
                expected,
                "lane {i}, request {:?}",
                lanes[i]
            );
        }
        assert!(executor.metrics.multi_document_submissions > 0);
        assert!(executor.metrics.multi_program_submissions > 0);
        assert!(executor.metrics.max_kernel_width > 32);
        assert!(executor.metrics.kernel_dispatches < executor.metrics.kernel_lanes / 4);
        let cancel = |document| document == 1;
        executor.cancellation(&cancel);
        let cancelled = lanes
            .chunks(TILE)
            .flat_map(|tile| executor.execute(tile))
            .collect::<Vec<_>>();
        for ((lane, original), answer) in lanes.iter().zip(&actual).zip(cancelled) {
            if lane.document == 1 {
                assert!(answer.incomplete);
            } else {
                assert_eq!(*original, answer);
            }
        }
    }
}
