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
use crate::code_rule_dsl::space::{Builder, NodeSet, NodeSpace};
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
    /// Per operation, what is known of its members (shape-fixes §16).
    known: Vec<Known>,
}

/// What lowering knows of a set's members, for containment (`lower.rs`'s `within`, `factors`
/// and `kinds`).
#[derive(Clone, Default)]
struct Known {
    /// Sets known to contain it, itself included.
    containing: BTreeSet<usize>,
    /// The sets it is the intersection of: itself, unless an intersection.
    factors: BTreeSet<usize>,
    /// The kinds its members can have, when known.
    kinds: Option<BTreeSet<u16>>,
}

impl Program {
    pub(super) fn lower(plans: &[&AnchorPlan]) -> Self {
        let mut program = Self { ops: Vec::new(), outputs: Vec::new(), ids: HashMap::new(), utilities: HashMap::new(), known: Vec::new() };
        let all = program.all();
        program.outputs = plans.iter().map(|plan| program.plan(plan, all).id).collect();
        program
    }

    fn push(&mut self, op: Op) -> usize {
        if let Some(&id) = self.ids.get(&op) {
            return id;
        }
        let id = self.ops.len();
        let known = self.known_of(id, &op);
        self.known.push(known);
        self.ops.push(op.clone());
        self.ids.insert(op, id);
        id
    }

    /// What follows of operation `id`'s members from its operands'.
    fn known_of(&self, id: usize, op: &Op) -> Known {
        let of = |at: &usize| &self.known[*at];
        let (mut containing, factors, kinds) = match op {
            Op::Kinds(kinds) => (BTreeSet::new(), BTreeSet::new(), Some(kinds.iter().copied().collect())),
            Op::Intersect(a, b) => (
                &of(a).containing | &of(b).containing,
                &of(a).factors | &of(b).factors,
                match (&of(a).kinds, &of(b).kinds) {
                    (Some(x), Some(y)) => Some(x & y),
                    (Some(x), None) | (None, Some(x)) => Some(x.clone()),
                    (None, None) => None,
                },
            ),
            Op::Union(ids) => {
                let containing = ids.iter().map(|at| of(at).containing.clone()).reduce(|x, y| &x & &y).unwrap_or_default();
                let kinds = ids.iter().map(|at| of(at).kinds.clone()).reduce(|x, y| Some(&x? | &y?)).flatten();
                (containing, BTreeSet::new(), kinds)
            }
            Op::Except(scope, _) | Op::Within(scope, _) => (of(scope).containing.clone(), BTreeSet::new(), of(scope).kinds.clone()),
            Op::Empty => (BTreeSet::new(), BTreeSet::new(), Some(BTreeSet::new())),
            Op::All | Op::Terms { .. } | Op::Uncovered => (BTreeSet::new(), BTreeSet::new(), None),
        };
        containing.insert(id);
        let factors = if factors.is_empty() { BTreeSet::from([id]) } else { factors };
        Known { containing, factors, kinds }
    }

    /// The kinds of a kind posting set, unless it holds the error-node convention of kind MAX
    /// (kind 0 labels no node and is never a posting).
    fn root_kinds(&self, id: usize) -> Option<&[u16]> {
        match &self.ops[id] {
            Op::Kinds(kinds) if !kinds.contains(&u16::MAX) => Some(kinds),
            _ => None,
        }
    }

    /// Whether `set` is known to be a subset of `outer`: `outer` contains it, `outer` is a kind
    /// posting holding every kind `set` can have, or this holds of each of `outer`'s factors.
    fn inside(&self, set: usize, outer: usize) -> bool {
        let known = &self.known[set];
        let within = |outer: usize| {
            known.containing.contains(&outer)
                || self.root_kinds(outer).is_some_and(|kinds| known.kinds.as_ref().is_some_and(|own| own.iter().all(|kind| kinds.contains(kind))))
        };
        within(outer) || self.known[outer].factors.iter().all(|&factor| within(factor))
    }

    /// A union of restrictions of one set to kinds is that set restricted to all their kinds:
    /// `(r ∩ kinds{1}) ∪ (r ∩ kinds{5,6})` is `r ∩ kinds{1,5,6}`.
    fn factored(&mut self, ids: &[usize]) -> Option<usize> {
        let mut common: Option<BTreeSet<usize>> = None;
        let mut kinds = BTreeSet::new();
        for &id in ids {
            let (roots, rest): (BTreeSet<usize>, BTreeSet<usize>) = self.known[id].factors.iter().partition(|&&factor| self.root_kinds(factor).is_some());
            let restricted = roots
                .iter()
                .map(|&root| self.root_kinds(root).expect("a kind posting").iter().copied().collect::<BTreeSet<_>>())
                .reduce(|x, y| &x & &y)?;
            if rest.is_empty() || common.as_ref().is_some_and(|common| *common != rest) {
                return None;
            }
            kinds.extend(restricted);
            common = Some(rest);
        }
        let base = common?.into_iter().reduce(|x, y| self.intersect(x, y))?;
        let kinds = self.kinds(&kinds);
        Some(self.intersect(base, kinds.id))
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
            // An intersection with a known superset is the subset.
            _ if self.inside(a, b) => a,
            _ if self.inside(b, a) => b,
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
            _ => match self.factored(&ids) {
                Some(id) => id,
                // A branch that contains every other branch is the union.
                None => match ids.iter().find(|&&outer| ids.iter().all(|&id| self.inside(id, outer))) {
                    Some(&outer) => outer,
                    None => self.push(Op::Union(ids)),
                },
            },
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

/// The plans' set program as `rir` (list step 18; P `spec-tooling.md` §2.2.1, `setplan`): each
/// operation `S<n>` on the earlier ones, then each plan's output.
pub(in crate::code_rule_dsl) fn dump(plans: &[&AnchorPlan], grammar: Option<&crate::code_rule_dsl::transducer::packed::rir::Names>) -> String {
    use std::fmt::Write as _;
    let program = Program::lower(plans);
    let sets = |ids: &[usize]| format!("[{}]", ids.iter().map(|id| format!("S{id}")).collect::<Vec<_>>().join(" "));
    let mut out = String::new();
    let _ = writeln!(out, "setplan ops={} outputs={}", program.ops.len(), program.outputs.len());
    for (id, op) in program.ops.iter().enumerate() {
        let body = match op {
            Op::Empty => "empty".to_owned(),
            Op::All => "all".to_owned(),
            // A leaf's kinds are the grammar's (0 labels no node).
            Op::Kinds(kinds) => format!("kinds {}", crate::code_rule_dsl::transducer::packed::rir::kinds(grammar, kinds)),
            Op::Terms { expression, literal } => {
                let quoted = crate::code_rule_dsl::transducer::packed::rir::quote(expression.as_bytes());
                format!("terms {} {quoted}", if *literal { "literal" } else { "regex" })
            }
            Op::Uncovered => "uncovered".to_owned(),
            Op::Union(ids) => format!("union {}", sets(ids)),
            Op::Intersect(a, b) => format!("intersect S{a} S{b}"),
            Op::Except(a, b) => format!("except S{a} S{b}"),
            Op::Within(scope, tests) => format!("within S{scope} {}", sets(tests)),
        };
        let _ = writeln!(out, "  S{id} = {body}");
    }
    for (plan, id) in program.outputs.iter().enumerate() {
        let _ = writeln!(out, "  output {plan} S{id}");
    }
    out
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
    // The gates write compiled plans, so the transaction always writes.
    let (space, leaves, gated) = store.candidate_transaction(true, |db| {
        let gated = {
            let _stage = crate::code_rule_dsl::prof::Stage::start("sets.gates");
            super::gates::select(db, lang as u32, plans, files)?
        };
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
                    // The working set's image of the tape where it holds one (decision 0024 §5).
                    if let Some(image) = store.working_set().kinds(file) {
                        return Ok(if i64::from(image.lang) == lang { image.nodes } else { 0 });
                    }
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
        // Files are read in space order, so each kind's set is built in node order.
        let mut by_kind: HashMap<u16, Builder> = HashMap::new();
        let mut uncovered = Builder::default();
        let reads_uncovered = program.ops.contains(&Op::Uncovered);
        {
            let mut kinds = db.prepare_cached("SELECT kind, postings FROM tape_kind_counts WHERE tape_id=?1")?;
            let mut missing = db.prepare_cached("SELECT missing FROM term_coverage WHERE tape_id=?1 AND length(missing)>0")?;
            for (at, &file) in files.iter().enumerate() {
                // A file no plan's gate holds has no candidates: its postings are not read.
                if space.count(at) == 0 || gated.any.as_ref().is_some_and(|any| !any.contains(&file)) {
                    continue;
                }
                if let (false, Some(image)) = (wanted.is_empty(), store.working_set().kinds(file)) {
                    crate::code_rule_dsl::prof::count("working_set.kind_hits", 1);
                    for (&atom, &kind) in &wanted {
                        let Some(postings) = u32::try_from(atom).ok().and_then(|atom| image.postings(atom)) else { continue };
                        let set = by_kind.entry(kind).or_default();
                        push_kinds(postings, space.base(at), space.count(at), set)?;
                    }
                } else if !wanted.is_empty() {
                    let mut rows = kinds.query([file])?;
                    while let Some(row) = rows.next()? {
                        let Some(&kind) = wanted.get(&row.get::<_, i64>(0)?) else { continue };
                        let set = by_kind.entry(kind).or_default();
                        push_kinds(row.get_ref(1)?.as_blob()?, space.base(at), space.count(at), set)?;
                    }
                }
                if reads_uncovered {
                    // A file's rows are each ascending, not one after another.
                    let mut positions = Vec::new();
                    let mut rows = missing.query([file])?;
                    while let Some(row) = rows.next()? {
                        positions.extend(decode(row.get_ref(0)?.as_blob()?, space.count(at))?);
                    }
                    positions.sort_unstable();
                    positions.dedup();
                    positions.into_iter().for_each(|p| uncovered.node(space.base(at) + p));
                }
            }
        }
        let by_kind = by_kind.into_iter().map(|(kind, set)| (kind, set.finish())).collect::<HashMap<_, _>>();
        let uncovered = uncovered.finish();
        // A text's term postings, by term.
        let terms_of = |expression: &str, literal: bool| -> rusqlite::Result<NodeSet> {
            let mut set = NodeSet::empty(&space);
            // Terms are atoms (`atoms`).
            let terms = if literal {
                db.prepare_cached("SELECT v.term FROM atoms a CROSS JOIN term_freq v ON v.lang=?1 AND v.term=a.atom WHERE a.value=?2")?
                    .query_map(rusqlite::params![lang, expression], |row| row.get::<_, i64>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            } else {
                db.prepare_cached("SELECT term FROM vocabulary_term WHERE lang=?1 AND expression=?2")?
                    .query_map(rusqlite::params![lang, expression], |row| row.get::<_, i64>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            let mut postings = db.prepare_cached("SELECT tape_id, postings FROM term_postings WHERE lang=?1 AND term=?2")?;
            for term in terms {
                let mut files = Vec::new();
                let mut rows = postings.query(rusqlite::params![lang, term])?;
                while let Some(row) = rows.next()? {
                    if let Some(&at) = index.get(&row.get::<_, i64>(0)?) {
                        files.push((at, decode(row.get_ref(1)?.as_blob()?, space.count(at))?));
                    }
                }
                // One term's set, built in node order (a file's rows merged), then joined to the
                // others'.
                files.sort_unstable_by_key(|&(at, _)| at);
                let mut term_set = Builder::default();
                for group in files.chunk_by_mut(|x, y| x.0 == y.0) {
                    let at = group[0].0;
                    let mut positions = group.iter_mut().flat_map(|(_, p)| std::mem::take(p)).collect::<Vec<_>>();
                    if group.len() > 1 {
                        positions.sort_unstable();
                        positions.dedup();
                    }
                    positions.into_iter().for_each(|p| term_set.node(space.base(at) + p));
                }
                set.union_with(&term_set.finish());
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
        Ok((space, leaves, gated))
    })?;
    crate::code_rule_dsl::prof::peak("2a1.leaves");
    let _stage = crate::code_rule_dsl::prof::Stage::start("sets.evaluate");
    let started = std::time::Instant::now();
    let mut outputs = match device {
        Device::Cpu => evaluate(&program, &space, leaves),
        #[cfg(target_os = "macos")]
        Device::Metal => evaluate_metal(&program, &space, &leaves)?,
    };
    crate::code_rule_dsl::prof::peak("2a2.evaluated");
    // Each plan keeps its gated files' nodes only.
    for (set, gate) in outputs.iter_mut().zip(&gated.plans) {
        let Some(gate) = gate else { continue };
        let mut kept = Builder::default();
        files.iter().enumerate().filter(|(_, file)| gate.contains(file)).for_each(|(at, _)| {
            let range = space.range(at);
            kept.run(range.start, range.end);
        });
        set.intersect_with(&kept.finish());
    }
    let ns_per_plan = started.elapsed().as_nanos() as f64 / outputs.len().max(1) as f64;
    let candidates = outputs.iter().map(NodeSet::len).collect::<Vec<_>>();
    let backend = match device {
        Device::Cpu => "cpu",
        #[cfg(target_os = "macos")]
        Device::Metal => "metal",
    };
    store.candidate_transaction(true, |db| super::gates::observe(db, &gated, &candidates, space.len(), backend, ns_per_plan))?;
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
    // Each value is freed after the last operation that reads it (outputs are kept), and that
    // operation takes it instead of copying it: the program holds its live values only.
    let reads = |op: &Op| -> Vec<usize> {
        match op {
            Op::Empty | Op::All | Op::Kinds(_) | Op::Terms { .. } | Op::Uncovered => Vec::new(),
            Op::Union(ids) => ids.clone(),
            Op::Intersect(a, b) | Op::Except(a, b) => vec![*a, *b],
            Op::Within(scope, tests) => std::iter::once(*scope).chain(tests.iter().copied()).collect(),
        }
    };
    let mut last = vec![None::<usize>; program.ops.len()];
    for (id, op) in program.ops.iter().enumerate() {
        reads(op).into_iter().for_each(|read| last[read] = Some(id));
    }
    let outputs = program.outputs.iter().copied().collect::<std::collections::HashSet<_>>();
    let dies_at = |value: usize, at: usize| !outputs.contains(&value) && last[value].is_none_or(|l| l == at);
    let mut values: Vec<NodeSet> = Vec::with_capacity(program.ops.len());
    // The leaves are read before the program runs: live from the start.
    let (mut live, mut live_sets) = (leaves.values().map(NodeSet::bytes).sum::<usize>(), leaves.len());
    let (mut peak, mut peak_sets) = (live, live_sets);
    let (mut sizes, mut freed) = (Vec::with_capacity(program.ops.len()), Vec::with_capacity(program.ops.len()));
    for (id, op) in program.ops.iter().enumerate() {
        // The first operand, taken when this is its last read and no other operand is the same value.
        let first = |values: &mut Vec<NodeSet>, first: usize, rest: &[usize]| -> NodeSet {
            if dies_at(first, id) && !rest.contains(&first) { std::mem::take(&mut values[first]) } else { values[first].clone() }
        };
        let value = match op {
            Op::Empty => NodeSet::empty(space),
            Op::All => NodeSet::all(space),
            Op::Kinds(_) | Op::Terms { .. } | Op::Uncovered => leaves.remove(&id).expect("a read leaf"),
            Op::Union(ids) => {
                let mut set = first(&mut values, ids[0], &ids[1..]);
                ids[1..].iter().for_each(|&id| set.union_with(&values[id]));
                set
            }
            // An intersection or difference writes a new set: its first operand is read, not copied.
            Op::Intersect(a, b) => values[*a].intersection(&values[*b]),
            Op::Except(a, b) => values[*a].difference(&values[*b]),
            Op::Within(scope, tests) => {
                let mut set = first(&mut values, *scope, tests);
                set.within_files(space, &tests.iter().map(|&id| &values[id]).collect::<Vec<_>>());
                set
            }
        };
        sizes.push(value.bytes());
        freed.push(false);
        if !matches!(op, Op::Kinds(_) | Op::Terms { .. } | Op::Uncovered) {
            live += sizes[id];
            live_sets += 1;
        }
        values.push(value);
        (peak, peak_sets) = (peak.max(live), peak_sets.max(live_sets));
        for read in reads(op).into_iter().chain(std::iter::once(id)) {
            if dies_at(read, id) && (read != id || last[id].is_none()) && !freed[read] {
                live -= sizes[read];
                live_sets -= 1;
                freed[read] = true;
                values[read] = NodeSet::default();
            }
        }
    }
    // The most the program held at once, and what dense words over the space would have held.
    crate::code_rule_dsl::prof::count("sets.peak_live", peak_sets);
    crate::code_rule_dsl::prof::count("sets.peak_bytes", peak);
    crate::code_rule_dsl::prof::count("sets.peak_dense_bytes", peak_sets * space.words() * 8);
    crate::code_rule_dsl::prof::count("sets.ops", program.ops.len());
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
    crate::code_rule_dsl::prof::count("set.steps", steps.len());
    crate::code_rule_dsl::prof::count("set.withins", steps.iter().filter(|step| matches!(step, SetStep::Within { .. })).count());
    crate::code_rule_dsl::prof::count("set.programs", 1);
    let files = (0..space.files()).map(|file| (space.base(file), space.count(file))).collect::<Vec<_>>();
    let leaves = leaves.iter().map(|(&id, set)| (id, set.to_words(space))).collect::<Vec<_>>();
    let leaves = leaves.iter().map(|(id, words)| (*id, words.as_slice())).collect::<Vec<_>>();
    let words = set_steps::run(&files, space.len(), program.ops.len(), &leaves, &steps, &program.outputs)
        .map_err(|error| crate::content_store::StoreError::Invalid(format!("the set steps failed on Metal: {error:?}")))?;
    Ok(words.into_iter().map(|words| NodeSet::from_words(space, &words)).collect())
}

/// Adds a kind's container segment (`content_store::postings::segment_positions`) of the file at
/// `base`, of `count` nodes, to `set`: arrays and runs move by `base`, bitmaps are shifted.
fn push_kinds(bytes: &[u8], base: u32, count: u32, set: &mut Builder) -> rusqlite::Result<()> {
    let invalid = |why: &str| rusqlite::Error::UserFunctionError(format!("invalid kind segment: {why}").into());
    set.segment(base, bytes, count).map_err(invalid)
}

/// A posting segment's positions (preorders as varint deltas, from 0) in a file of `count` nodes.
fn decode(bytes: &[u8], count: u32) -> rusqlite::Result<Vec<u32>> {
    let invalid = || rusqlite::Error::UserFunctionError("invalid posting".into());
    let (mut at, mut pre) = (0, 0_u32);
    let mut positions = Vec::new();
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
        positions.push(pre);
    }
    positions.dedup();
    Ok(positions)
}
