//! A batch's candidates as set algebra over the wave's node space (decision 0024 §2).
//!
//! The plans lower to one program of set operations, shared across plans by structure:
//! - leaves: every node, kind postings, a text's term postings, and the nodes the terms do not
//!   cover;
//! - operators: union, intersection, difference, and "within the files where these sets have a
//!   node".
//!
//! Lowering keeps `lower.rs`'s semantics: each plan's set is a superset of its matches, exact where
//! that lowering's is. A `not` subtracts only an exact inner set. The statistics' order of a
//! pattern's requirements does not change a set: intersections commute. Unlike the SQL there is
//! no admission limit, so no plan is broadened to its root kinds.
//!
//! SQL reads the leaves: the space's node counts, the posting blobs by key, and the vocabulary's
//! terms for each regex, filled by the vocabulary stages first. The operations run on the
//! space's bitmaps (`space::NodeSet`).
use super::lower::{VOCABULARY, anchored};
use super::LANG_STRIDE;
use crate::code_rule_dsl::matcher::{AnchorPlan, Condition, Required, Requirement};
use crate::code_rule_dsl::space::{NodeSet, NodeSpace};
use crate::content_store::{ContentStore, StoreResult};
use std::collections::{BTreeSet, HashMap};

/// A set operation; operands are earlier operations' indices.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Op {
    Empty,
    All,
    /// Nodes of these kinds (grammar symbols; 0 labels no node).
    Kinds(Vec<u16>),
    /// Nodes whose text is a term equal to `expression` (`literal`) or matching it as a regex.
    Terms { expression: String, literal: bool },
    /// Nodes whose text the term postings do not cover (`term_coverage.missing`).
    Uncovered,
    Union(Vec<usize>),
    Intersect(usize, usize),
    Except(usize, usize),
    /// The first set's nodes in files where each of the others has a node.
    Within(usize, Vec<usize>),
}

/// A lowered set and whether it is exactly the plan's matches.
#[derive(Clone, Copy)]
struct Lowered {
    id: usize,
    exact: bool,
}

/// The batch's set program: its operations in dependency order and each plan's output.
pub(super) struct Program {
    pub(super) ops: Vec<Op>,
    pub(super) outputs: Vec<usize>,
    ids: HashMap<Op, usize>,
    utilities: HashMap<(usize, usize), Lowered>,
}

impl Program {
    pub(super) fn lower(plans: &[&AnchorPlan]) -> Self {
        let mut program = Self { ops: Vec::new(), outputs: Vec::new(), ids: HashMap::new(), utilities: HashMap::new() };
        let all = program.all();
        program.outputs = plans.iter().map(|plan| program.plan(plan, all).id).collect();
        program
    }

    fn push(&mut self, op: Op) -> usize {
        if let Some(&id) = self.ids.get(&op) {
            return id;
        }
        let id = self.ops.len();
        self.ops.push(op.clone());
        self.ids.insert(op, id);
        id
    }

    fn all(&mut self) -> Lowered {
        Lowered { id: self.push(Op::All), exact: true }
    }

    fn empty(&mut self) -> usize {
        self.push(Op::Empty)
    }

    fn kinds(&mut self, kinds: &BTreeSet<u16>) -> Lowered {
        let kinds = kinds.iter().copied().filter(|&kind| kind != 0).collect::<Vec<_>>();
        let id = if kinds.is_empty() { self.empty() } else { self.push(Op::Kinds(kinds)) };
        Lowered { id, exact: true }
    }

    fn intersect(&mut self, a: usize, b: usize) -> usize {
        let all = self.push(Op::All);
        match (a, b) {
            _ if a == b || b == all => a,
            _ if a == all => b,
            _ => self.push(Op::Intersect(a.min(b), a.max(b))),
        }
    }

    fn union(&mut self, branches: &[Lowered]) -> Lowered {
        let exact = branches.iter().all(|set| set.exact);
        let empty = self.empty();
        let mut ids = branches.iter().map(|set| set.id).filter(|&id| id != empty).collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        let id = match ids.as_slice() {
            [] => empty,
            [id] => *id,
            _ => self.push(Op::Union(ids)),
        };
        Lowered { id, exact }
    }

    fn within(&mut self, scope: Lowered, sets: &[Lowered]) -> Lowered {
        if sets.is_empty() {
            return Lowered { exact: false, ..scope };
        }
        let mut tests = sets.iter().map(|set| set.id).collect::<Vec<_>>();
        tests.sort_unstable();
        tests.dedup();
        Lowered { id: self.push(Op::Within(scope.id, tests)), exact: false }
    }

    /// `scope`'s nodes whose text is `expression`'s, or not covered by the terms: a superset.
    fn text(&mut self, scope: Lowered, expression: &str, literal: bool) -> Lowered {
        let terms = self.push(Op::Terms { expression: expression.to_owned(), literal });
        let uncovered = self.push(Op::Uncovered);
        let texts = self.push(Op::Union(vec![terms.min(uncovered), terms.max(uncovered)]));
        Lowered { id: self.intersect(scope.id, texts), exact: false }
    }

    fn plan(&mut self, plan: &AnchorPlan, scope: Lowered) -> Lowered {
        let scope = match &plan.root_kinds {
            Some(kinds) => {
                let kinds = self.kinds(kinds);
                Lowered { id: self.intersect(scope.id, kinds.id), exact: scope.exact }
            }
            None => scope,
        };
        let mut selected = match &plan.condition {
            Condition::Always => scope,
            Condition::Text { regex, .. } => self.text(scope, regex.as_str(), false),
            Condition::Pattern(required) => self.pattern(required, scope),
            Condition::All(plans) => {
                let mut selected = scope;
                for plan in plans.iter().filter(|p| anchored(p)).chain(plans.iter().filter(|p| !anchored(p))) {
                    selected = self.plan(plan, selected);
                }
                selected
            }
            Condition::Any(plans) => {
                let (only, rest): (Vec<_>, Vec<_>) =
                    plans.iter().partition(|plan| plan.root_kinds.is_some() && matches!(plan.condition, Condition::Always));
                let mut branches = rest.into_iter().map(|plan| self.plan(plan, scope)).collect::<Vec<_>>();
                if let [plan] = only.as_slice() {
                    branches.push(self.plan(plan, scope));
                } else if !only.is_empty() {
                    let kinds = only.iter().flat_map(|plan| plan.root_kinds.iter().flatten().copied()).collect();
                    let kinds = self.kinds(&kinds);
                    let exact = scope.exact && only.iter().all(|plan| plan.exact);
                    branches.push(Lowered { id: self.intersect(scope.id, kinds.id), exact });
                }
                self.union(&branches)
            }
            Condition::Not(inner) => {
                let inner = self.plan(inner, scope);
                if inner.exact {
                    let id = if inner.id == scope.id { self.empty() } else { self.push(Op::Except(scope.id, inner.id)) };
                    Lowered { id, exact: scope.exact }
                } else {
                    Lowered { exact: false, ..scope }
                }
            }
            Condition::Utility(inner) => {
                let key = (std::sync::Arc::as_ptr(inner) as usize, scope.id);
                if let Some(&selected) = self.utilities.get(&key) {
                    return Lowered { exact: selected.exact && plan.exact, ..selected };
                }
                let selected = self.plan(inner, scope);
                self.utilities.insert(key, selected);
                selected
            }
            Condition::Relation { inner, .. } => {
                if anchored(inner) {
                    let all = self.all();
                    let inner = self.plan(inner, all);
                    self.within(scope, &[inner])
                } else {
                    Lowered { exact: false, ..scope }
                }
            }
        };
        selected.exact &= plan.exact;
        selected
    }

    /// A pattern's roots: in `scope`, holding its parentless requirements, in files holding the
    /// others (`lower.rs`'s `pattern`).
    fn pattern(&mut self, required: &[Requirement], scope: Lowered) -> Lowered {
        if required.is_empty() {
            return scope;
        }
        let mut roots = scope;
        let mut inside = Vec::new();
        for requirement in required {
            let (kind, literal) = match &requirement.value {
                Required::Kind(kind) => (*kind, None),
                Required::Terminal(kind, text) => (*kind, Some(text)),
            };
            let mut posting = self.kinds(&BTreeSet::from([kind]));
            if let Some(text) = literal {
                posting = self.text(posting, text, true);
            }
            if requirement.parents.is_empty() {
                roots = Lowered { id: self.intersect(roots.id, posting.id), exact: false };
            } else {
                inside.push(posting);
            }
        }
        self.within(roots, &inside)
    }

    /// The regex expressions the program's texts read (the vocabulary's predicates).
    fn regexes(&self) -> BTreeSet<&str> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                Op::Terms { expression, literal: false } => Some(expression.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// Each plan's candidates over `files` (tape rows, in the caller's order) of language `lang`: the
/// space of their nodes (a file of another language, or without prefilter rows, has none) and
/// one set per plan, the operations run on `device`.
pub(super) fn select(
    store: &ContentStore,
    lang: u32,
    plans: &[&AnchorPlan],
    files: &[i64],
    device: Device,
) -> StoreResult<(NodeSpace, Vec<NodeSet>)> {
    let program = crate::code_rule_dsl::prof::time("sets.lower", || Program::lower(plans));
    let regexes = program.regexes();
    let lang = i64::from(lang);
    let (space, leaves) = store.candidate_transaction(!regexes.is_empty(), |db| {
        // The vocabulary's terms for each regex, as the SQL's vocabulary stages fill them.
        if !regexes.is_empty() {
            let _stage = crate::code_rule_dsl::prof::Stage::start("sets.vocabulary");
            let mut register = db.prepare_cached("INSERT OR IGNORE INTO vocabulary_predicate(lang,expression,ready) VALUES (?1,?2,0)")?;
            for expression in &regexes {
                register.execute(rusqlite::params![lang, expression])?;
            }
            for (_, sql) in VOCABULARY {
                db.execute_batch(sql)?;
            }
        }
        let _stage = crate::code_rule_dsl::prof::Stage::start("sets.read");
        let space = {
            let mut nodes = db.prepare_cached(
                "SELECT s.nodes FROM prefilter_sets s JOIN tape_objects t ON t.tape_id=s.tape_id WHERE s.tape_id=?1 AND t.grammar_id=?2",
            )?;
            let counts = files
                .iter()
                .map(|&file| {
                    let count: Option<i64> = nodes.query_row(rusqlite::params![file, lang], |row| row.get(0)).map(Some).or_else(|error| match error {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        error => Err(error),
                    })?;
                    Ok(u32::try_from(count.unwrap_or(0)).unwrap_or(0))
                })
                .collect::<rusqlite::Result<Vec<_>>>()?;
            NodeSpace::new(counts)
        };
        let index = files.iter().enumerate().filter(|&(at, _)| space.count(at) > 0).map(|(at, &file)| (file, at)).collect::<HashMap<_, _>>();
        let atom = |kind: u16| lang * i64::from(LANG_STRIDE) + if kind == u16::MAX { 0 } else { i64::from(kind) };
        // Each kind's postings, read once per file for every kind the program reads.
        let wanted = program
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Kinds(kinds) => Some(kinds.iter().map(|&kind| (atom(kind), kind))),
                _ => None,
            })
            .flatten()
            .collect::<HashMap<_, _>>();
        let mut by_kind: HashMap<u16, NodeSet> = HashMap::new();
        let mut uncovered = NodeSet::empty(&space);
        let reads_uncovered = program.ops.contains(&Op::Uncovered);
        {
            let mut kinds = db.prepare_cached("SELECT kind, postings FROM tape_kind_counts WHERE tape_id=?1")?;
            let mut missing = db.prepare_cached("SELECT missing FROM term_coverage WHERE tape_id=?1 AND length(missing)>0")?;
            for (at, &file) in files.iter().enumerate() {
                if space.count(at) == 0 {
                    continue;
                }
                let base = space.base(at);
                if !wanted.is_empty() {
                    let mut rows = kinds.query([file])?;
                    while let Some(row) = rows.next()? {
                        let Some(&kind) = wanted.get(&row.get::<_, i64>(0)?) else { continue };
                        let set = by_kind.entry(kind).or_insert_with(|| NodeSet::empty(&space));
                        decode(row.get_ref(1)?.as_blob()?, base, space.count(at), set)?;
                    }
                }
                if reads_uncovered {
                    let mut rows = missing.query([file])?;
                    while let Some(row) = rows.next()? {
                        decode(row.get_ref(0)?.as_blob()?, base, space.count(at), &mut uncovered)?;
                    }
                }
            }
        }
        // A text's term postings, by term.
        let terms_of = |expression: &str, literal: bool| -> rusqlite::Result<NodeSet> {
            let mut set = NodeSet::empty(&space);
            let terms = if literal {
                db.prepare_cached("SELECT term FROM term_freq WHERE lang=?1 AND term=?2")?
                    .query_map(rusqlite::params![lang, expression], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            } else {
                db.prepare_cached("SELECT term FROM vocabulary_term WHERE lang=?1 AND expression=?2")?
                    .query_map(rusqlite::params![lang, expression], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            let mut postings = db.prepare_cached("SELECT tape_id, postings FROM term_postings WHERE lang=?1 AND term=?2")?;
            for term in terms {
                let mut rows = postings.query(rusqlite::params![lang, term])?;
                while let Some(row) = rows.next()? {
                    if let Some(&at) = index.get(&row.get::<_, i64>(0)?) {
                        decode(row.get_ref(1)?.as_blob()?, space.base(at), space.count(at), &mut set)?;
                    }
                }
            }
            Ok(set)
        };
        // The leaves the operations read: kinds, texts, and the uncovered nodes.
        let mut leaves = HashMap::new();
        for (id, op) in program.ops.iter().enumerate() {
            let leaf = match op {
                Op::Kinds(kinds) => {
                    let mut set = NodeSet::empty(&space);
                    kinds.iter().filter_map(|kind| by_kind.get(kind)).for_each(|kind| set.union_with(kind));
                    set
                }
                Op::Terms { expression, literal } => terms_of(expression, *literal)?,
                Op::Uncovered => uncovered.clone(),
                _ => continue,
            };
            leaves.insert(id, leaf);
        }
        Ok((space, leaves))
    })?;
    let _stage = crate::code_rule_dsl::prof::Stage::start("sets.evaluate");
    let outputs = match device {
        Device::Cpu => evaluate(&program, &space, leaves),
        #[cfg(target_os = "macos")]
        Device::Metal => evaluate_metal(&program, &space, &leaves)?,
    };
    Ok((space, outputs))
}

/// Where a set program's operations run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Device {
    Cpu,
    #[cfg(target_os = "macos")]
    Metal,
}

/// The program's outputs, its operations on the host's bitmaps.
fn evaluate(program: &Program, space: &NodeSpace, mut leaves: HashMap<usize, NodeSet>) -> Vec<NodeSet> {
    let mut values: Vec<NodeSet> = Vec::with_capacity(program.ops.len());
    for (id, op) in program.ops.iter().enumerate() {
        let value = match op {
            Op::Empty => NodeSet::empty(space),
            Op::All => NodeSet::all(space),
            Op::Kinds(_) | Op::Terms { .. } | Op::Uncovered => leaves.remove(&id).expect("a read leaf"),
            Op::Union(ids) => {
                let mut set = values[ids[0]].clone();
                ids[1..].iter().for_each(|&id| set.union_with(&values[id]));
                set
            }
            Op::Intersect(a, b) => {
                let mut set = values[*a].clone();
                set.intersect_with(&values[*b]);
                set
            }
            Op::Except(a, b) => {
                let mut set = values[*a].clone();
                set.difference_with(&values[*b]);
                set
            }
            Op::Within(scope, tests) => {
                let mut set = values[*scope].clone();
                set.within_files(space, &tests.iter().map(|&id| &values[id]).collect::<Vec<_>>());
                set
            }
        };
        values.push(value);
    }
    program.outputs.iter().map(|&id| values[id].clone()).collect()
}

/// The program's outputs, its operations as Metal set steps: a slot per operation, the leaves
/// written in theirs (`set_steps`).
#[cfg(target_os = "macos")]
fn evaluate_metal(program: &Program, space: &NodeSpace, leaves: &HashMap<usize, NodeSet>) -> StoreResult<Vec<NodeSet>> {
    use crate::code_rule_dsl::transducer::packed::set_steps::{self, SetStep};
    let slot = |id: usize| id as u32;
    let mut steps = Vec::new();
    for (id, op) in program.ops.iter().enumerate() {
        let out = slot(id);
        match op {
            Op::Empty => steps.push(SetStep::Clear { out }),
            Op::All => steps.push(SetStep::Fill { out }),
            Op::Kinds(_) | Op::Terms { .. } | Op::Uncovered => {}
            Op::Union(ids) => {
                steps.push(SetStep::Copy { out, a: slot(ids[0]) });
                steps.extend(ids[1..].iter().map(|&b| SetStep::Or { out, a: out, b: slot(b) }));
            }
            Op::Intersect(a, b) => steps.push(SetStep::And { out, a: slot(*a), b: slot(*b) }),
            Op::Except(a, b) => steps.push(SetStep::AndNot { out, a: slot(*a), b: slot(*b) }),
            Op::Within(scope, tests) => {
                steps.push(SetStep::Copy { out, a: slot(*scope) });
                steps.extend(tests.iter().map(|&test| SetStep::Within { out, test: slot(test) }));
            }
        }
    }
    let files = (0..space.files()).map(|file| (space.base(file), space.count(file))).collect::<Vec<_>>();
    let leaves = leaves.iter().map(|(&id, set)| (id, set.words())).collect::<Vec<_>>();
    let words = set_steps::run(&files, space.len(), program.ops.len(), &leaves, &steps, &program.outputs)
        .map_err(|error| crate::content_store::StoreError::Invalid(format!("the set steps failed on Metal: {error:?}")))?;
    Ok(words.into_iter().map(|words| NodeSet::from_words(space, words)).collect())
}

/// Adds a posting segment's nodes (preorders as varint deltas, from 0) of a file of `count`
/// nodes at `base` to `set`.
fn decode(bytes: &[u8], base: u32, count: u32, set: &mut NodeSet) -> rusqlite::Result<()> {
    let invalid = || rusqlite::Error::UserFunctionError("invalid posting".into());
    let (mut at, mut pre) = (0, 0_u32);
    while at < bytes.len() {
        let mut delta = 0_u32;
        let mut shift = 0;
        loop {
            let byte = *bytes.get(at).ok_or_else(invalid)?;
            at += 1;
            if shift == 28 && byte > 15 {
                return Err(invalid());
            }
            delta |= u32::from(byte & 127) << shift;
            if byte & 128 == 0 {
                break;
            }
            shift += 7;
            if shift > 28 {
                return Err(invalid());
            }
        }
        pre = pre.checked_add(delta).filter(|&pre| pre < count).ok_or_else(invalid)?;
        set.insert(base + pre);
    }
    Ok(())
}
