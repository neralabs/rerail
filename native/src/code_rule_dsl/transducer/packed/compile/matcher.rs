//! W's composite matcher lowered to packed programs (06-first-slice S1).
//!
//! A clause becomes a sequence of node-label passes and one root program. Each pass computes a set
//! of nodes over the whole tape, a full-width phase (Bend's "sequence phases, do not fork them
//! against each other", `notes` of 06): an ast-grep pattern's roots, a program's roots, the nodes a
//! program reaches (`follows`), or another label's complement (`not`, a stop rule's walk bound).
//! Later programs test a label with `SUPERTYPE`, as a grammar class: the label is a derived class
//! of the tape (`TapeDoc::with_labels`), so the interpreters need no new operation (F1).
//!
//! Inside a program a relation is a walk: `PARENT`, `CHILD`, `DESCEND`, `NEXT` or `FOLLOWING`,
//! looped with `GOTO` where a stop rule bounds it. Every loop moves along one tree edge each turn
//! (`cycle_progress`), so it ends within the tape. A relation's rule that itself walks is a label,
//! so each program walks at most one relation deep and its paths stay linear in the tape.
//!
//! Patterns are labels too: their matcher is the packed pattern (`PackedPattern`), which the words
//! do not inline. Bindings are not carried: a clause where two leaves bind one metavariable is
//! refused by `matcher::lower`.

use super::*;
use crate::code_rule_dsl::matcher::{Axis, MatchTree, Stop};
use crate::code_rule_dsl::transducer::packed::evaluation_costs as costs;
use crate::code_rule_dsl::transducer::packed::{self as transducer_packed, WorkBudget};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::code_rule_dsl::tape::DERIVED_CLASS_BASE as LABEL_BASE;

// The label passes on Metal (`rerail-device-metal`), behind `transducer::packed::device`.
// cratify: `matcher_metal` is rerail-device-metal's (it mounts it)

/// A lowered clause: label passes, in order, then the root program.
#[derive(Clone)]
pub struct LoweredMatcher {
    pub labels: Labels,
    pub program: Program,
    pub domain: Domain,
}

#[derive(Clone, Default)]
pub struct Labels {
    /// By label id less `LABEL_BASE` and `offset`.
    pub defs: Vec<Label>,
    /// Where this clause's labels start among a wave's (`LoweredWave`): 0 alone.
    offset: u16,
    /// Evaluation order: each step a group of labels evaluated together, to their least fixpoint
    /// where the group is a recursive utility's (one label otherwise).
    steps: Vec<Vec<u16>>,
    /// A recursive utility's label, by recursion id (`MatchTree::Recursive`).
    recursive: BTreeMap<usize, u16>,
    /// In a wave, the labels this clause reads that an earlier clause owns (the combined graph,
    /// `LoweredWave::new`), with the labels they read in turn: their definitions, to read their
    /// domains (`bound`). They are evaluated as the owner's passes.
    borrowed: BTreeMap<u16, Label>,
}

impl Label {
    /// The label with every label it reads renamed by `map`.
    fn relabelled(&self, map: &dyn Fn(u16) -> u16) -> Self {
        let domain = |domain: &Domain| match domain {
            Domain::Label(label) => Domain::Label(map(*label)),
            other => other.clone(),
        };
        match self {
            Label::Pattern(program, starts) => Label::Pattern(program.relabelled(map), domain(starts)),
            Label::Constrained(program, starts, constraints) => Label::Constrained(
                program.relabelled(map),
                domain(starts),
                constraints.iter().map(|(name, label)| (name.clone(), map(*label))).collect(),
            ),
            Label::Roots(program, starts) => Label::Roots(program.relabelled(map), domain(starts)),
            Label::Targets(program, starts) => Label::Targets(program.relabelled(map), domain(starts)),
            Label::Count { program, domain: starts, by, min, max } => Label::Count {
                program: program.relabelled(map),
                domain: domain(starts),
                by: *by,
                min: *min,
                max: *max,
            },
            Label::Complement(of) => Label::Complement(map(*of)),
            Label::Fact(fact, slots) => Label::Fact(Arc::clone(fact), slots.iter().map(|slot| map(*slot)).collect()),
            Label::Empty => Label::Empty,
        }
    }

    /// The program a roots, targets, count or pattern label walks.
    fn program(&self) -> Option<&Program> {
        match self {
            Label::Pattern(program, _)
            | Label::Constrained(program, ..)
            | Label::Roots(program, _)
            | Label::Targets(program, _)
            | Label::Count { program, .. } => Some(program),
            Label::Complement(_) | Label::Fact(..) | Label::Empty => None,
        }
    }

    /// The label's structural identity, the labels it reads named in the wave: equal for labels
    /// that mark the same nodes of every tape (the combined graph's hash-consing key).
    fn key(&self) -> String {
        let tally = |by: &Tally| match by {
            Tally::Start => "start",
            Tally::Capture => "capture",
        };
        match self {
            Label::Pattern(program, starts) => format!("pattern|{}|{starts:?}", program.plain_digest()),
            Label::Constrained(program, starts, constraints) => format!("constrained|{}|{starts:?}|{constraints:?}", program.plain_digest()),
            Label::Roots(program, starts) => format!("roots|{}|{starts:?}", program.plain_digest()),
            Label::Targets(program, starts) => format!("targets|{}|{starts:?}", program.plain_digest()),
            Label::Count { program, domain, by, min, max } => {
                format!("count|{}|{domain:?}|{}|{min}|{max:?}", program.plain_digest(), tally(by))
            }
            Label::Complement(of) => format!("complement|{of}"),
            Label::Fact(fact, slots) => format!("fact|{}|{slots:?}", fact.key),
            Label::Empty => "empty".to_owned(),
        }
    }
}

/// The nodes a pass starts from: those whose kind the rule needs, or a label's (a walk's start).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Domain {
    All,
    Kinds(Vec<u16>),
    Label(u16),
}

impl Domain {
    pub fn of(tree: &MatchTree) -> Self {
        tree.kinds()
            .map_or(Self::All, |kinds| Self::Kinds(kinds.into_iter().collect()))
    }
}

#[derive(Clone, Copy)]
pub enum Tally {
    Start,
    Capture,
}

/// Where a clause's passes run. On Metal every pass is a kernel in one command buffer
/// (`roots_metal`); pattern leaves are matched on the host first, as `demand`'s pattern producers.
#[derive(Clone, Copy, Debug)]
pub enum Backend {
    Cpu,
    /// The device's label passes (`transducer::packed::device`), where one is registered.
    MetalInterpreter,
}

#[derive(Clone)]
pub enum Label {
    /// The roots an ast-grep pattern matches.
    Pattern(Program, Domain),
    /// The roots an ast-grep pattern matches where each constrained single-node capture has its
    /// label.
    Constrained(Program, Domain, Vec<(String, u16)>),
    /// The roots a program emits.
    Roots(Program, Domain),
    /// The nodes a program captures: where its walk ends.
    Targets(Program, Domain),
    /// The nodes with between `min` and `max` outputs of a counting program: by each output's
    /// root, or by the node it captured (a backward walk counted from its other end).
    Count {
        program: Program,
        domain: Domain,
        by: Tally,
        min: usize,
        max: Option<usize>,
    },
    /// The nodes without another label.
    Complement(u16),
    /// The nodes a semantic predicate's producer reports whose slots' targets have the slots' labels
    /// (O24, decision 0030). The rows read no label; the slots are labels of their own.
    Fact(Arc<crate::code_rule_dsl::matcher::Fact>, Vec<u16>),
    /// No node (`Never`).
    Empty,
}

/// Each fact's rows on each tape, by (fact, tape source) addresses: made before any pass.
type FactRows = HashMap<(usize, usize), crate::code_rule_dsl::matcher::FactFound>;

/// The rows of `facts` on each of `tapes`, before any label pass (decision 0030): the producers read
/// no label. Each tape is one file evaluation, so its scope table is built once and shared by every
/// fact on it, as W's run shares it across clauses; the tapes run in parallel.
pub fn fact_rows<'a>(facts: impl Iterator<Item = &'a Arc<crate::code_rule_dsl::matcher::Fact>>, tapes: &[TapeDoc]) -> FactRows {
    use rayon::prelude::*;
    let mut distinct: Vec<&Arc<crate::code_rule_dsl::matcher::Fact>> = Vec::new();
    for fact in facts {
        if !distinct.iter().any(|known| Arc::ptr_eq(known, fact)) {
            distinct.push(fact);
        }
    }
    if distinct.is_empty() {
        return FactRows::new();
    }
    crate::code_rule_dsl::prof::count("labels.facts.distinct", distinct.len());
    crate::code_rule_dsl::prof::count("labels.facts.anywhere", distinct.iter().filter(|fact| fact.kinds.is_none()).count());
    // `labels.facts` is the caller's thread CPU; each tape's rows, on whichever thread takes it, are `fact_rows.tape`
    // (the pool's work, which the caller's does not hold: Q135). Not a `labels.` stage: those take process CPU too,
    // which tapes in parallel would count again.
    crate::code_rule_dsl::prof::time("labels.facts", || {
        tapes
            .par_iter()
            .flat_map_iter(|tape| crate::code_rule_dsl::prof::time("fact_rows.tape", || {
                let file = ast_grep_core::AstGrep::doc(tape.clone());
                let _scopes = crate::code_rule_dsl::matcher::FileScopeSlot::install(Vec::new(), None, !tape.syntax_error_ranges().is_empty());
                // The tape's parents' gaps for its facts' gap tests (Q135's spike, `RERAIL_GAPS`).
                let _gaps = crate::code_rule_dsl::matcher::gap::GapScope::install(&file.root(), tape.grammar().dialect());
                let source = Arc::as_ptr(tape.document_source()) as usize;
                // The tape's sibling positions and `nthChild` answers, shared by its facts.
                let siblings = crate::code_rule_dsl::matcher::SiblingPositions::default();
                distinct
                    .iter()
                    .map(|fact| {
                        let starts = fact.kinds.as_ref().map(|kinds| Domain::Kinds(kinds.iter().copied().collect())).unwrap_or(Domain::All);
                        ((Arc::as_ptr(fact) as usize, source), fact.found(&file, domain(&starts, tape, &[]), &siblings))
                    })
                    .collect::<Vec<_>>()
            }))
            .collect()
    })
}

impl Labels {
    /// Where this clause's labels start among its wave's: label `LABEL_BASE + offset + i` is `defs[i]`.
    pub fn offset(&self) -> usize {
        usize::from(self.offset)
    }

    /// The facts its labels read.
    pub fn facts(&self) -> impl Iterator<Item = &Arc<crate::code_rule_dsl::matcher::Fact>> {
        self.defs.iter().filter_map(|label| match label {
            Label::Fact(fact, _) => Some(fact),
            _ => None,
        })
    }

    fn add(&mut self, label: Label) -> u16 {
        let id = self.reserve(label);
        self.steps.push(vec![id]);
        id
    }

    /// A label not yet in the evaluation order.
    fn reserve(&mut self, label: Label) -> u16 {
        self.defs.push(label);
        LABEL_BASE + self.offset + (self.defs.len() - 1) as u16
    }

    /// The recursive group label `id` is in (its step), if it is in one: the labels that settle together in rounds.
    pub fn group_of(&self, id: u16) -> Option<&[u16]> {
        self.steps.iter().find(|step| step.contains(&id) && self.recursive.values().any(|r| step.contains(r))).map(Vec::as_slice)
    }

    pub fn def(&self, id: u16) -> &Label {
        match usize::from(id - LABEL_BASE).checked_sub(usize::from(self.offset)) {
            Some(at) if at < self.defs.len() => &self.defs[at],
            _ => self.borrowed.get(&id).expect("a label of the clause or one it borrows"),
        }
    }

    /// The kinds a domain's nodes can have, known before any label is (`None`: any kind). A
    /// label of roots, pattern matches or counts by root lies within its own domain; targets,
    /// counts by capture and complements can be any node. A label met again (recursion) is any.
    pub fn bound(&self, domain: &Domain, seen: &mut Vec<u16>) -> Option<BTreeSet<u16>> {
        match domain {
            Domain::All => None,
            Domain::Kinds(kinds) => Some(kinds.iter().copied().collect()),
            Domain::Label(id) if seen.contains(id) => None,
            Domain::Label(id) => {
                seen.push(*id);
                let bound = match self.def(*id) {
                    Label::Pattern(_, starts) | Label::Constrained(_, starts, _) | Label::Roots(_, starts) => self.bound(starts, seen),
                    Label::Count { domain, by: Tally::Start, .. } => self.bound(domain, seen),
                    Label::Empty => Some(BTreeSet::new()),
                    Label::Targets(..) | Label::Count { .. } | Label::Complement(_) | Label::Fact(..) => None,
                };
                seen.pop();
                bound
            }
        }
    }

    /// The labels' ids, past the last label of the wave's clauses before this one.
    fn ids(&self) -> std::ops::Range<u16> {
        LABEL_BASE + self.offset..LABEL_BASE + self.offset + self.defs.len() as u16
    }

    /// The same labels with every id `by` higher.
    fn shifted(&self, by: u16) -> Self {
        let up = |label: u16| label + by;
        let domain = |domain: &Domain| match domain {
            Domain::Label(label) => Domain::Label(up(*label)),
            other => other.clone(),
        };
        let defs = self
            .defs
            .iter()
            .map(|label| match label {
                Label::Pattern(program, starts) => Label::Pattern(program.shifted(by), domain(starts)),
                Label::Constrained(program, starts, constraints) => Label::Constrained(
                    program.shifted(by),
                    domain(starts),
                    constraints.iter().map(|(name, label)| (name.clone(), up(*label))).collect(),
                ),
                Label::Roots(program, starts) => Label::Roots(program.shifted(by), domain(starts)),
                Label::Targets(program, starts) => Label::Targets(program.shifted(by), domain(starts)),
                Label::Count { program, domain: starts, by: tally, min, max } => Label::Count {
                    program: program.shifted(by),
                    domain: domain(starts),
                    by: *tally,
                    min: *min,
                    max: *max,
                },
                Label::Complement(of) => Label::Complement(up(*of)),
                Label::Fact(fact, slots) => Label::Fact(Arc::clone(fact), slots.iter().map(|slot| up(*slot)).collect()),
                Label::Empty => Label::Empty,
            })
            .collect();
        Self {
            defs,
            offset: self.offset + by,
            steps: self.steps.iter().map(|step| step.iter().copied().map(up).collect()).collect(),
            recursive: self.recursive.iter().map(|(at, label)| (*at, up(*label))).collect(),
            borrowed: self.borrowed.iter().map(|(id, label)| (up(*id), label.clone())).collect(),
        }
    }

    /// The labels a label's pass reads.
    fn reads(&self, id: u16) -> Vec<u16> {
        let tests = |program: &Program| {
            program
                .words
                .iter()
                .filter_map(|&word| match split(word) {
                    (SUPERTYPE, _, class) if class >= LABEL_BASE => Some(class),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let starts = |domain: &Domain| match domain {
            Domain::Label(label) => Some(*label),
            Domain::All | Domain::Kinds(_) => None,
        };
        match self.def(id) {
            Label::Pattern(_, _) | Label::Empty => Vec::new(),
            Label::Constrained(_, _, constraints) => constraints.iter().map(|(_, label)| *label).collect(),
            Label::Roots(program, domain)
            | Label::Targets(program, domain)
            | Label::Count { program, domain, .. } => {
                tests(program).into_iter().chain(starts(domain)).collect()
            }
            Label::Complement(of) => vec![*of],
            Label::Fact(_, slots) => slots.clone(),
        }
    }
}

impl LoweredMatcher {
    /// Every program the clause walks: its labels' and its root program.
    pub fn walked_programs(&self) -> Vec<&Program> {
        self.labels.defs.iter().filter_map(Label::program).chain(std::iter::once(&self.program)).collect()
    }

    pub fn lower(tree: &MatchTree) -> Result<Self, Error> {
        let mut labels = Labels::default();
        // Set mode (list step 14): the clause's matches are its roots, read as keys (the wave's
        // `match(c, d, n)` and spans), so the root program stops at its first emit per start.
        let program = Builder::new(&mut labels).roots(tree)?.in_set_mode();
        if labels.defs.len() > usize::from(u16::MAX - LABEL_BASE) {
            return Err(Error::InvalidOperand(0));
        }
        Ok(Self {
            labels,
            program,
            domain: Domain::of(tree),
        })
    }

    /// How many label passes run before the root program.
    pub fn passes(&self) -> usize {
        self.labels.defs.len()
    }

    /// The nodes of `tape` the clause's matcher matches at, in preorder.
    pub fn roots(&self, tape: &TapeDoc) -> Result<BTreeSet<u32>, Error> {
        let mut roots = self.roots_batch(Backend::Cpu, std::slice::from_ref(tape))?;
        Ok(roots.pop().expect("one tape"))
    }

    /// The nodes of each tape the clause's matcher matches at: each pass runs once over the batch.
    pub fn roots_batch(
        &self,
        backend: Backend,
        tapes: &[TapeDoc],
    ) -> Result<Vec<BTreeSet<u32>>, Error> {
        if let Backend::MetalInterpreter = backend
            && super::device::matcher().is_some()
        {
            return self.roots_metal(tapes).map(|(roots, _)| roots);
        }
        let mut sets = empty_sets(tapes, self.labels.defs.len());
        self.label(tapes, &mut sets)?;
        let views = tapes
            .iter()
            .zip(&sets)
            .map(|(tape, sets)| tape.with_labels(Arc::clone(sets)))
            .collect::<Vec<_>>();
        let outputs = run(&self.program, &views, &self.domain, &sets)?;
        Ok(outputs
            .into_iter()
            .map(|outputs| outputs.into_iter().map(|output| output.start).collect())
            .collect())
    }

    /// Evaluates the clause's labels over the batch into `sets`, which hold every label of the
    /// wave the clause is part of (its own from `labels.ids()`), each step to its fixpoint.
    fn label(&self, tapes: &[TapeDoc], sets: &mut [Arc<Vec<Vec<u64>>>]) -> Result<(), Error> {
        let most = tapes.iter().map(TapeDoc::transducer_len).max().unwrap_or(0);
        let facts = fact_rows(self.labels.facts(), tapes);
        for step in &self.labels.steps {
            let recursive = self.labels.recursive.values().any(|id| step.contains(id));
            // A group without complements of its own labels only grows; one with them reads only
            // later nodes' verdicts (`matcher::lower`), settled a node depth per round. Either way
            // the rounds end within the tape.
            let mut settled = false;
            for _round in 0..=most + 1 {
                let mut changed = false;
                for &id in step {
                    let views = tapes
                        .iter()
                        .zip(sets.iter())
                        .map(|(tape, sets)| tape.with_labels(Arc::clone(sets)))
                        .collect::<Vec<_>>();
                    let found = self.evaluate(self.labels.def(id), &views, sets, &facts)?;
                    for (sets, set) in sets.iter_mut().zip(found) {
                        let slot = &mut Arc::make_mut(sets)[usize::from(id - LABEL_BASE)];
                        if *slot != set {
                            *slot = set;
                            changed = true;
                        }
                    }
                }
                if !changed || !recursive {
                    settled = true;
                    break;
                }
            }
            if !settled {
                return Err(Error::WorkLimit);
            }
        }
        Ok(())
    }

    /// Probe: per recursive group of programs, the forward sweeps (each node in preorder, its
    /// results marked at once) that settle it from the other labels' final sets.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn sweeps_needed(&self, tapes: &[TapeDoc]) -> Result<Vec<usize>, Error> {
        let mut sets = empty_sets(tapes, self.labels.defs.len());
        self.label(tapes, &mut sets)?;
        let mut needed = Vec::new();
        for step in &self.labels.steps {
            if !self.labels.recursive.values().any(|id| step.contains(id))
                || !step.iter().all(|id| matches!(self.labels.def(*id), Label::Roots(..) | Label::Targets(..)))
            {
                continue;
            }
            for sets in sets.iter_mut() {
                for &id in step {
                    Arc::make_mut(sets)[usize::from(id - LABEL_BASE)].iter_mut().for_each(|word| *word = 0);
                }
            }
          let local = local_levels(&self.labels, step);
          let mut ordered = step.clone();
          ordered.sort_by_key(|id| local[id]);
          for reverse in [false, true] {
            for sets in sets.iter_mut() {
                for &id in step {
                    Arc::make_mut(sets)[usize::from(id - LABEL_BASE)].iter_mut().for_each(|word| *word = 0);
                }
            }
            let mut sweeps = 0;
            loop {
                sweeps += 1;
                let mut changed = false;
                for (tape, sets) in tapes.iter().zip(sets.iter_mut()) {
                    let len = tape.transducer_len() as u32;
                    for node in (0..len).map(|at| if reverse { len - 1 - at } else { at }) {
                        for &id in &ordered {
                            let (Label::Roots(program, _) | Label::Targets(program, _)) = self.labels.def(id) else { unreachable!() };
                            let view = tape.with_labels(Arc::clone(sets));
                            for output in program.execute_roots(&view, [node])? {
                                let nodes = match self.labels.def(id) {
                                    Label::Roots(..) => vec![output.start],
                                    _ => output.captures.iter().map(|capture| capture.node).collect(),
                                };
                                for marked in nodes {
                                    let bits = &mut Arc::make_mut(sets)[usize::from(id - LABEL_BASE)];
                                    if bits[marked as usize / 64] >> (marked % 64) & 1 == 0 {
                                        bits[marked as usize / 64] |= 1 << (marked % 64);
                                        changed = true;
                                    }
                                }
                            }
                        }
                    }
                }
                if !changed || sweeps > 64 {
                    break;
                }
            }
            needed.push(sweeps);
          }
        }
        Ok(needed)
    }

    /// [`evaluate`](Self::evaluate), a program's pass on the interpreter held to `budget` (decision 0334): `None` on a
    /// tape where its evaluation went to W's matcher instead (`run_budgeted`; the clause's, there).
    ///
    /// With `anchors` (each view's column sets: a level whose labels read only finished ones), each walk starts only
    /// within its anchor (`anchor_bits`, decision 0365).
    fn evaluate_budgeted(
        &self,
        label: &Label,
        views: &[TapeDoc],
        sets: &[Arc<Vec<Vec<u64>>>],
        facts: &FactRows,
        budget: WorkBudget,
        anchors: Option<&[&std::sync::OnceLock<super::compile_sets::Index>]>,
    ) -> Result<Vec<Option<Vec<u64>>>, Error> {
        let (Label::Roots(program, starts) | Label::Targets(program, starts) | Label::Count { program, domain: starts, .. }) = label else {
            return Ok(self.evaluate(label, views, sets, facts)?.into_iter().map(Some).collect());
        };
        // A file's verdicts are its own, so the files run in parallel; each by the program's
        // micro walk where it is one, as Metal's label passes and the sweeps run it, else
        // (or where the walk overflows) on the interpreter. 06 S7: the interpreter's task per
        // start was ~6.5 s of the CPU backend's ~12.5 s of labels.
        use rayon::prelude::*;
        let walk = Micro::of(program);
        let entry = Entry::of(program);
        views
            .par_iter()
            .zip(sets.par_iter())
            .enumerate()
            .map(crate::code_rule_dsl::logic::profile::carried("root programs (pool)", |(at, (view, sets)): (usize, (&TapeDoc, &Arc<Vec<Vec<u64>>>))| {
                let texts = walk.as_ref().and_then(|walk| walk.texts(program, view));
                let anchor = anchors.map(|indexes| {
                    let index = indexes[at].get_or_init(Default::default);
                    let anchor = anchor_bits(program, walk.as_ref(), texts.as_deref(), view, sets, index);
                    count_anchored(domain(starts, view, sets).filter(|&node| entry.admits(view, node)), &anchor);
                    anchor
                });
                let starts = || anchored(domain(starts, view, sets).filter(|&node| entry.admits(view, node)), anchor.as_deref());
                if let Some(walk) = &walk
                    && let Some(texts) = &texts
                    && let Some(bits) = walked_with(label, walk, texts, view, sets, starts())
                {
                    return Ok(Some(bits));
                }
                crate::code_rule_dsl::prof::count("labels.interpreted", 1);
                let starts: Vec<u32> = starts().collect();
                // The micro walk was the first plan, and is not one here (or outgrew its bound): none is weighed again.
                let Ran::Done(outputs) = run_budgeted(program, view, &starts, budget, &|| self.w_cost(view), &|_: &[u32]| None, "ast label pass")? else {
                    return Ok(None);
                };
                let found = vec![outputs.iter().map(Found::of).collect()];
                Ok(Some(from_outputs(label, std::slice::from_ref(view), found).pop().expect("one file")))
            }))
            .collect()
    }

    /// W's matcher's predicted steps to confirm this clause on `view`: its candidates (its domain, every node for a
    /// label's, past its entry) and their subtrees.
    fn w_cost(&self, view: &TapeDoc) -> u64 {
        let roots: Vec<u32> = self.unlabelled_candidates(view).collect();
        costs::predict_w(&self.program, subtree_nodes(view, &roots))
    }

    /// The clause's candidates on `view` with no label read: its domain's kinds (every node for a label's domain),
    /// past its entry. Where its labels went to W's matcher, they are W's candidates.
    fn unlabelled_candidates<'a>(&'a self, view: &'a TapeDoc) -> impl Iterator<Item = u32> + 'a {
        let entry = Entry::of(&self.program);
        (0..view.transducer_len() as u32)
            .filter(move |&node| match &self.domain {
                Domain::All | Domain::Label(_) => true,
                Domain::Kinds(kinds) => kinds.binary_search(&view.transducer_kind(node)).is_ok(),
            })
            .filter(move |&node| entry.admits(view, node))
    }

    /// A label's set on each tape of the batch.
    fn evaluate(
        &self,
        label: &Label,
        views: &[TapeDoc],
        sets: &[Arc<Vec<Vec<u64>>>],
        facts: &FactRows,
    ) -> Result<Vec<Vec<u64>>, Error> {
        let empty = |view: &TapeDoc| vec![0_u64; view.transducer_len().div_ceil(64)];
        let set = |bits: &mut Vec<u64>, node: u32| bits[node as usize / 64] |= 1 << (node % 64);
        match label {
            Label::Pattern(pattern, starts) | Label::Constrained(pattern, starts, _) => {
                let constraints = match label {
                    Label::Constrained(_, _, constraints) => &constraints[..],
                    _ => &[],
                };
                // Each constraint's slot in the pattern's bindings: a name the pattern binds nowhere
                // never constrains.
                let constraints: Vec<(usize, u16)> =
                    constraints.iter().filter_map(|(name, label)| Some((pattern.ast_slot(name)?, *label))).collect();
                // A file's verdicts are its own: the files run in parallel, each with bindings kept
                // from one node to the next.
                use rayon::prelude::*;
                views
                    .par_iter()
                    .zip(sets.par_iter())
                    .map(crate::code_rule_dsl::logic::profile::carried("label pass (pool)", |(view, sets): (&TapeDoc, &Arc<Vec<Vec<u64>>>)| {
                        let mut bits = empty(view);
                        let mut bindings = AstBindings::default();
                        for node in domain(starts, view, sets) {
                            if !pattern.matches_ast_pattern(view, node, &mut bindings)? {
                                continue;
                            }
                            // As W's `Constrained`: an unbound or multi-node capture is not checked.
                            let holds = constraints.iter().all(|&(slot, label)| match bindings.bound(slot) {
                                Bound::Single(node) => has(sets, label, node),
                                _ => true,
                            });
                            if holds {
                                set(&mut bits, node);
                            }
                        }
                        Ok(bits)
                    }))
                    .collect()
            }
            // At the ceiling, a stop: the caller has no W to give the clause to (`evaluate_budgeted`).
            Label::Roots(..) | Label::Targets(..) | Label::Count { .. } => self
                .evaluate_budgeted(label, views, sets, facts, WorkBudget::Full, None)?
                .into_iter()
                .map(|bits| bits.ok_or(Error::WorkLimit))
                .collect(),
            Label::Complement(of) => Ok(views
                .iter()
                .zip(sets)
                .map(|(view, sets)| {
                    let nodes = view.transducer_len();
                    let mut bits = sets[usize::from(of - LABEL_BASE)].iter().map(|word| !word).collect::<Vec<_>>();
                    if nodes % 64 != 0
                        && let Some(last) = bits.last_mut()
                    {
                        *last &= (1 << (nodes % 64)) - 1;
                    }
                    bits
                })
                .collect()),
            // The producer's rows read no label; each slot's target must have its slot's label.
            Label::Fact(fact, slots) => Ok(views
                .iter()
                .zip(sets)
                .map(|(view, sets)| {
                    let mut bits = empty(view);
                    let key = (Arc::as_ptr(fact) as usize, Arc::as_ptr(view.document_source()) as usize);
                    match facts.get(&key) {
                        // No slot: the nodes are the verdicts.
                        Some(crate::code_rule_dsl::matcher::FactFound::Nodes(nodes)) => bits.clone_from(nodes),
                        Some(found) => {
                            for (node, targets) in found.rows() {
                                if targets.iter().zip(slots).all(|(&target, &label)| has(sets, label, target)) {
                                    set(&mut bits, node);
                                }
                            }
                        }
                        None => {}
                    }
                    bits
                })
                .collect()),
            Label::Empty => Ok(views.iter().map(empty).collect()),
        }
    }
}

impl Program {
    /// The program with every label it tests renamed by `map`.
    fn relabelled(&self, map: &dyn Fn(u16) -> u16) -> Self {
        let mut program = self.clone();
        for word in &mut program.words {
            if let (SUPERTYPE, mode, class) = split(*word)
                && class >= LABEL_BASE
            {
                *word = super::word(SUPERTYPE, mode, map(class));
            }
        }
        program
    }

    /// The program with every label it tests `by` higher (a clause's place in a wave).
    fn shifted(&self, by: u16) -> Self {
        let mut program = self.clone();
        for word in &mut program.words {
            if let (SUPERTYPE, mode, class) = split(*word)
                && class >= LABEL_BASE
            {
                *word = super::word(SUPERTYPE, mode, class + by);
            }
        }
        program
    }
}

impl LoweredMatcher {
    /// The clause with its labels `by` higher.
    fn shifted(&self, by: u16) -> Self {
        Self {
            labels: self.labels.shifted(by),
            program: self.program.shifted(by),
            domain: self.domain.clone(),
        }
    }
}

/// How a wave's batch ran ([`LoweredWave::roots_batch`]): on Metal or the CPU, its command buffers and their seconds on
/// the GPU; and the (clause, tape) pairs whose roots outran the interpreter's work limit.
#[derive(Clone, Debug, Default)]
pub struct WaveRun {
    pub metal: bool,
    pub command_buffers: usize,
    pub device: f64,
    /// A clause's roots on a tape where its program outran the work limit: there, its found roots are every
    /// candidate (its domain, past its entry), which the caller confirms with W's matcher.
    pub limited: Vec<(usize, usize)>,
    /// How many times the run went again with twice the step budget (decision 0334, Metal's `Outcome::replanned`).
    pub replanned: usize,
}

/// A wave's lowered clauses evaluated together (06 S4): each clause's labels shifted past the
/// clauses' before it, so one set of label bitsets per tape holds them all and every pass and
/// root program reads its own. The roots then run as one batch of programs (`demand(batch)`'s
/// packed producers), or, on Metal, every clause's passes and roots in one command buffer.
pub struct LoweredWave {
    pub clauses: Vec<LoweredMatcher>,
    pub labels: usize,
    /// Per clause, the earlier clauses whose labels it reads (the combined graph), transitively:
    /// their passes run wherever it is active (`spread`).
    feeds: Vec<BTreeSet<usize>>,
    /// The level passes fused into one walk, built at their first batch (`Packs`).
    packs: Packs,
}

impl LoweredWave {
    /// The clauses as one combined graph (list step 14 → 15; t1 spec-fusion §4.1, §4.7, the
    /// fusion model's first priority): a label equal to an earlier one, every label it reads named
    /// in the wave (`Label::key`), is that label, so a sub-rule shared by clauses is one pass. A
    /// recursive group equal to an earlier one (its members' keys, members named by position) is
    /// that group, so a recursive utility shared by clauses runs its rounds once. Each clause's kept labels take the next
    /// ids, so its labels stay one range and the wave's bitsets hold each distinct label once.
    pub fn new(clauses: &[&LoweredMatcher]) -> Result<Self, Error> {
        let limit = usize::from(u16::MAX - LABEL_BASE);
        let mut next = 0_usize;
        let mut seen = HashMap::<String, u16>::new();
        let mut owner = HashMap::<u16, usize>::new();
        let mut combined: Vec<LoweredMatcher> = Vec::with_capacity(clauses.len());
        let mut feeds: Vec<BTreeSet<usize>> = Vec::with_capacity(clauses.len());
        let (mut distinct, mut total, mut shared_groups) = (0_usize, 0_usize, 0_usize);
        for (at, clause) in clauses.iter().enumerate() {
            let labels = &clause.labels;
            let offset = u16::try_from(next).map_err(|_| Error::InvalidOperand(0))?;
            let mut map = HashMap::<u16, u16>::new();
            let mut steps = Vec::new();
            let fresh = |next: &mut usize| -> Result<u16, Error> {
                let id = u16::try_from(*next).ok().filter(|id| usize::from(*id) < limit).ok_or(Error::InvalidOperand(0))?;
                *next += 1;
                Ok(LABEL_BASE + id)
            };
            for step in &labels.steps {
                total += step.len();
                let recursive = step.len() > 1 || labels.recursive.values().any(|id| step.contains(id));
                if recursive {
                    // A group's members read each other: the group's key is its members' in
                    // order, each member named by its position (`u16::MAX - position`, past every
                    // wave id while the wave's ids stay below them) and every other label in the
                    // wave. An equal group is the earlier one, member by member.
                    let placeholder = |position: usize| u16::MAX - position as u16;
                    let key = (next + usize::from(LABEL_BASE) + 2 * step.len() < usize::from(u16::MAX)).then(|| {
                        let inside = |label: u16| step.iter().position(|&id| id == label).map(placeholder);
                        let name = |label: u16| inside(label).or_else(|| map.get(&label).copied()).unwrap_or(label);
                        let members = step.iter().map(|&id| labels.def(id).relabelled(&name).key()).collect::<Vec<_>>();
                        format!("group|{}|{}", step.len(), members.join("\u{1f}"))
                    });
                    if let Some(shared) = key.as_ref().and_then(|key| seen.get(key)) {
                        let first = *shared;
                        for (position, &id) in step.iter().enumerate() {
                            map.insert(id, first + position as u16);
                        }
                        shared_groups += 1;
                        continue;
                    }
                    let ids = step.iter().map(|&id| fresh(&mut next).map(|new| (id, new))).collect::<Result<Vec<_>, _>>()?;
                    map.extend(ids.iter().copied());
                    if let (Some(key), Some(&(_, first))) = (key, ids.first()) {
                        seen.insert(key, first);
                    }
                    steps.push(ids.iter().map(|&(_, new)| new).collect());
                    continue;
                }
                // Its reads are earlier steps' (the steps are in evaluation order), named already.
                let id = step[0];
                let key = labels.def(id).relabelled(&|label| map.get(&label).copied().unwrap_or(label)).key();
                if let Some(&shared) = seen.get(&key) {
                    map.insert(id, shared);
                    continue;
                }
                let new = fresh(&mut next)?;
                map.insert(id, new);
                seen.insert(key, new);
                steps.push(vec![new]);
            }
            // A label in no step (none is expected) keeps its own.
            for local in 0..labels.defs.len() {
                let id = LABEL_BASE + labels.offset + local as u16;
                if !map.contains_key(&id) {
                    map.insert(id, fresh(&mut next)?);
                }
            }
            let rename = |label: u16| map.get(&label).copied().unwrap_or(label);
            // Each kept label's definition at its id's place: the ids from `offset` on are this clause's.
            let mut placed = vec![None; next - usize::from(offset)];
            for local in 0..labels.defs.len() {
                let new = map[&(LABEL_BASE + labels.offset + local as u16)];
                if let Some(at) = usize::from(new - LABEL_BASE).checked_sub(usize::from(offset)) {
                    placed[at].get_or_insert_with(|| labels.defs[local].relabelled(&rename));
                }
            }
            let defs = placed.into_iter().map(|def| def.expect("a definition per kept id")).collect::<Vec<_>>();
            distinct += defs.len();
            for local in 0..defs.len() {
                owner.insert(LABEL_BASE + offset + local as u16, at);
            }
            let mut own = Labels {
                defs,
                offset,
                steps,
                recursive: labels.recursive.iter().map(|(at, label)| (*at, rename(*label))).collect(),
                borrowed: BTreeMap::new(),
            };
            // The labels this clause reads that an earlier clause owns, with what they read.
            let mut wanted = (0..own.defs.len())
                .flat_map(|local| own.reads(LABEL_BASE + offset + local as u16))
                .chain(match &clause.domain {
                    Domain::Label(label) => Some(rename(*label)),
                    _ => None,
                })
                .chain(clause.program.relabelled(&rename).words.iter().filter_map(|&word| match split(word) {
                    (SUPERTYPE, _, class) if class >= LABEL_BASE => Some(class),
                    _ => None,
                }))
                .filter(|id| owner.get(id).is_some_and(|&by| by != at))
                .collect::<Vec<_>>();
            let mut fed = BTreeSet::new();
            while let Some(id) = wanted.pop() {
                if own.borrowed.contains_key(&id) {
                    continue;
                }
                let by = owner[&id];
                fed.insert(by);
                let source = &combined[by].labels;
                let def = source.def(id).clone();
                own.borrowed.insert(id, def);
                wanted.extend(source.reads(id).into_iter().filter(|read| owner.get(read).is_some_and(|&by| by != at)));
            }
            let fed = fed.iter().flat_map(|&by| std::iter::once(by).chain(feeds[by].iter().copied())).collect::<BTreeSet<_>>();
            feeds.push(fed);
            combined.push(LoweredMatcher {
                labels: own,
                program: clause.program.relabelled(&rename),
                domain: match &clause.domain {
                    Domain::Label(label) => Domain::Label(rename(*label)),
                    other => other.clone(),
                },
            });
        }
        // Each clause's root label on Metal follows every pass's.
        if next + clauses.len() > limit {
            return Err(Error::InvalidOperand(0));
        }
        crate::code_rule_dsl::prof::count("wave.labels", total);
        crate::code_rule_dsl::prof::count("wave.distinct_labels", distinct);
        crate::code_rule_dsl::prof::count("wave.shared_groups", shared_groups);
        Ok(Self { clauses: combined, labels: next, feeds, packs: Packs::default() })
    }

    /// The wave's plan identity (list step 16; t1 spec-fusion §4.9, the fused plan's cache key):
    /// its labels by id (each `Label::key`, which names what it reads), the evaluation steps and
    /// each clause's root program and domain. The interpreter and the compiled programs bind this
    /// one plan (`compilation::resolve_plan`); equal waves have equal identities, however lowered.
    pub fn identity(&self) -> super::compilation::PlanId {
        use sha2::{Digest as _, Sha256};
        let mut digest = Sha256::new();
        digest.update(b"rerail-wave-plan-v1\0");
        digest.update((self.labels as u64).to_le_bytes());
        for clause in &self.clauses {
            let labels = &clause.labels;
            digest.update(b"clause\0");
            digest.update(clause.program.plain_digest().as_bytes());
            digest.update(format!("|{:?}|{}|", clause.domain, labels.offset).as_bytes());
            for def in &labels.defs {
                digest.update(def.key().as_bytes());
                digest.update(b"\0");
            }
            digest.update(format!("{:?}|{:?}|{:?}", labels.steps, labels.recursive, labels.borrowed.keys().collect::<Vec<_>>()).as_bytes());
        }
        super::compilation::PlanId(digest.finalize().into())
    }

    /// The wave as `rir` (list step 18; P `spec-tooling.md` §2.2.1): its plan identity, each
    /// clause's root program, label table, steps and borrowed labels, then every program once, by
    /// digest; with `fused`, also the plan's passes by step and level with the packs they fuse
    /// into, and each pack's group (its members' dispatch, emit table and fused program).
    /// `names` are the clauses' names, in order; `grammar` names the kinds and fields.
    pub fn dump(&self, names: &[String], grammar: Option<&super::rir::Names>, fused_section: bool) -> Result<String, Error> {
        use std::fmt::Write as _;
        let hex = |bytes: &[u8]| bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        let domain = |domain: &Domain| match domain {
            Domain::All => "all".to_owned(),
            Domain::Kinds(kinds) => format!("kinds{}", super::rir::kinds(grammar, kinds)),
            Domain::Label(label) => format!("L{label}"),
        };
        let mut programs = BTreeMap::<String, &Program>::new();
        let mut program = |program: &'_ Program| -> String { format!("sha256:{}", program.plain_digest()) };
        let mut out = String::new();
        let _ = writeln!(out, "wave sha256:{} clauses={} labels={}", hex(&self.identity().0), self.clauses.len(), self.labels);
        let label = |out: &mut String, id: u16, def: &Label, program: &mut dyn FnMut(&Program) -> String| {
            let _ = match def {
                Label::Pattern(of, starts) => writeln!(out, "  label L{id} pattern {} domain={}", program(of), domain(starts)),
                Label::Constrained(of, starts, constraints) => {
                    let constraints = constraints.iter().map(|(name, label)| format!("{}=L{label}", super::rir::quote(name.as_bytes()))).collect::<Vec<_>>();
                    writeln!(out, "  label L{id} constrained {} domain={} constraints=[{}]", program(of), domain(starts), constraints.join(" "))
                }
                Label::Roots(of, starts) => writeln!(out, "  label L{id} roots {} domain={}", program(of), domain(starts)),
                Label::Targets(of, starts) => writeln!(out, "  label L{id} targets {} domain={}", program(of), domain(starts)),
                Label::Count { program: of, domain: starts, by, min, max } => {
                    let by = match by {
                        Tally::Start => "start",
                        Tally::Capture => "capture",
                    };
                    let max = max.map_or("-".to_owned(), |max| max.to_string());
                    writeln!(out, "  label L{id} count {} domain={} by={by} min={min} max={max}", program(of), domain(starts))
                }
                Label::Complement(of) => writeln!(out, "  label L{id} complement L{of}"),
                Label::Fact(fact, slots) => {
                    let slots = slots.iter().map(|slot| format!("L{slot}")).collect::<Vec<_>>();
                    writeln!(out, "  label L{id} fact {} slots=[{}]", fact.name, slots.join(" "))
                }
                Label::Empty => writeln!(out, "  label L{id} empty"),
            };
        };
        for (at, clause) in self.clauses.iter().enumerate() {
            let labels = &clause.labels;
            let name = names.get(at).map_or("-".to_owned(), |name| super::rir::quote(name.as_bytes()));
            programs.insert(clause.program.plain_digest().to_owned(), &clause.program);
            let _ = writeln!(out, "clause {at} {name} root={} domain={} offset={}", program(&clause.program), domain(&clause.domain), labels.offset);
            for (index, def) in labels.defs.iter().enumerate() {
                if let Some(of) = def.program() {
                    programs.insert(of.plain_digest().to_owned(), of);
                }
                label(&mut out, LABEL_BASE + labels.offset + index as u16, def, &mut program);
            }
            for step in &labels.steps {
                let recursive = step.len() > 1 || labels.recursive.values().any(|id| step.contains(id));
                let ids = step.iter().map(|id| format!("L{id}")).collect::<Vec<_>>().join(" ");
                let _ = writeln!(out, "  {} {ids}", if recursive { "group" } else { "step" });
            }
            for (recursion, id) in &labels.recursive {
                let _ = writeln!(out, "  recursive {recursion} L{id}");
            }
            for id in labels.borrowed.keys() {
                let _ = writeln!(out, "  borrowed L{id}");
            }
        }
        // The plan's passes as `labelled` runs them, and the packs (`Packs::groups`).
        let mut fused = Vec::new();
        if !fused_section {
            for program in programs.values() {
                out.push_str(&super::rir::disassemble(program, grammar));
            }
            return Ok(out);
        }
        let clauses = self.clauses.iter().collect::<Vec<_>>();
        let (steps, chains) = levels(&clauses, |clause, id| Ok((clause, id)))?;
        let at = |clause: &LoweredMatcher| self.clauses.iter().position(|c| std::ptr::eq(c, clause)).expect("a clause of the wave");
        out.push_str("plan\n");
        for (number, ((recursive, levels), (kinds, siblings))) in steps.iter().zip(&chains).enumerate() {
            if *recursive {
                let kinds = kinds.as_ref().map_or("any".to_owned(), |kinds| super::rir::kinds(grammar, kinds));
                let _ = writeln!(out, "  step {number} recursive kinds={kinds} siblings={}", u8::from(*siblings));
            } else {
                let _ = writeln!(out, "  step {number} single");
            }
            for (level, passes) in levels.iter().enumerate() {
                let members = passes.iter().map(|&(clause, id)| format!("c{}:L{id}", at(clause))).collect::<Vec<_>>();
                let _ = writeln!(out, "    level {level} {}", members.join(" "));
                for (targets, group) in Packs::groups(passes) {
                    let members = group.iter().map(|&index| passes[index].0.labels.def(passes[index].1).program()).collect::<Option<Vec<_>>>();
                    let Some(pack) = members.as_deref().and_then(super::fusion::Fused::of) else { continue };
                    let names = group.iter().map(|&index| format!("c{}:L{}", at(passes[index].0), passes[index].1)).collect::<Vec<_>>();
                    let _ = writeln!(out, "      pack {} [{}] sha256:{}", if targets { "targets" } else { "roots" }, names.join(" "), pack.program.plain_digest());
                    fused.push((pack, members.expect("packed members")));
                }
            }
        }
        for program in programs.values() {
            out.push_str(&super::rir::disassemble(program, grammar));
        }
        for (pack, members) in &fused {
            out.push_str(&super::rir::fused(pack, members, grammar));
        }
        Ok(out)
    }

    /// `active` (per clause, per tape) with each clause's labels also where a clause reading them
    /// is active.
    pub fn spread(&self, active: &[Vec<bool>]) -> Vec<Vec<bool>> {
        let mut out = active.to_vec();
        for (clause, fed) in self.feeds.iter().enumerate() {
            for &by in fed {
                for tape in 0..active[clause].len() {
                    out[by][tape] |= active[clause][tape];
                }
            }
        }
        out
    }

    /// How many label passes the wave runs before its roots.
    pub fn passes(&self) -> usize {
        self.labels
    }

    /// The count programs among `label_programs`, by address.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn count_programs(&self) -> BTreeSet<*const Program> {
        self.clauses
            .iter()
            .flat_map(|clause| clause.labels.defs.iter())
            .filter_map(|label| match label {
                Label::Count { program, .. } => Some(std::ptr::from_ref(program)),
                _ => None,
            })
            .collect()
    }

    /// Every clause's label and count programs, with what each marks (`Emit`).
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn label_programs(&self) -> Vec<(&Program, Emit)> {
        self.clauses
            .iter()
            .flat_map(|clause| clause.labels.defs.iter())
            .filter_map(|label| match label {
                Label::Roots(program, _) => Some((program, Emit::Start)),
                Label::Targets(program, _) => Some((program, Emit::Captures)),
                Label::Count { program, by: Tally::Start, .. } => Some((program, Emit::EveryStart)),
                Label::Count { program, by: Tally::Capture, .. } => Some((program, Emit::Captures)),
                _ => None,
            })
            .collect()
    }

    /// Every program the wave walks: each clause's own labels' and its root program.
    pub fn walked_programs(&self) -> Vec<&Program> {
        self.clauses
            .iter()
            .flat_map(|clause| clause.labels.defs.iter().filter_map(Label::program).chain(std::iter::once(&clause.program)))
            .collect()
    }

    /// Each clause's root program, in clause order.
    pub fn programs(&self) -> Vec<Program> {
        self.clauses.iter().map(|clause| clause.program.clone()).collect()
    }

    /// Every clause's labels over the batch, as views of its tapes the root programs read. The
    /// passes run by level as the plan has them (`levels`, as on Metal), on the interpreter: a
    /// label pass starts at most of a tape's nodes, where a packed batch's cost per task is the
    /// interpreter's per start. A recursive step sweeps each tape in node order where its passes
    /// allow (`sweep`), else repeats its levels until a round changes nothing.
    ///
    /// `active` (per clause, per tape) is where a clause's root program will start: a clause's
    /// labels are its own and read only by its walks, so they are computed only on those tapes
    /// (empty elsewhere).
    ///
    /// A pass at the ceiling is a stop (`WorkLimit`): this caller has no W to give the clause to.
    pub fn labelled(&self, tapes: &[TapeDoc], active: &[Vec<bool>]) -> Result<Vec<TapeDoc>, Error> {
        let (views, given) = self.labelled_sets(tapes, active, WorkBudget::Full)?;
        if !given.is_empty() {
            return Err(Error::WorkLimit);
        }
        Ok(views.into_iter().map(|(view, _)| view).collect())
    }

    /// Every clause's matches of the batch (per clause, per tape: the matched roots, tape preorder ids), the whole
    /// wave as one run: its labels' passes once, then each clause's root program from its starts. On Metal, every
    /// pass and root program in one command buffer (`roots_metal`, the batch halved where it outgrows a workspace or
    /// the launch bound); on the CPU, the passes by level (`labelled`) and the roots on the interpreter.
    /// `active` (per clause, per tape) is where a clause runs: a tape its gates exclude has no matches. A clause whose
    /// program outruns the work limit on a tape gives every candidate root there, listed in [`WaveRun::limited`], so
    /// one large file does not fail the wave: the caller confirms those roots with W's matcher.
    pub fn roots_batch(&self, backend: Backend, tapes: &[TapeDoc], active: &[Vec<bool>]) -> Result<(Vec<Vec<BTreeSet<u32>>>, WaveRun), Error> {
        self.roots_batch_budgeted(backend, tapes, active, crate::code_rule_dsl::transducer::packed::WorkBudget::current())
    }

    /// [`roots_batch`](Self::roots_batch) with the root programs held to `budget` on the CPU (decision 0334,
    /// `clause_roots`).
    pub fn roots_batch_budgeted(
        &self,
        backend: Backend,
        tapes: &[TapeDoc],
        active: &[Vec<bool>],
        budget: crate::code_rule_dsl::transducer::packed::WorkBudget,
    ) -> Result<(Vec<Vec<BTreeSet<u32>>>, WaveRun), Error> {
        if let Backend::MetalInterpreter = backend
            && super::device::matcher().is_some()
        {
            let roots: Vec<Vec<Vec<u32>>> = (0..self.clauses.len())
                .map(|c| tapes.iter().enumerate().map(|(t, tape)| if active[c][t] { (0..tape.transducer_len() as u32).filter(|&n| self.starts(c, tape, n)).collect() } else { Vec::new() }).collect())
                .collect();
            return self.roots_metal_split(tapes, &roots);
        }
        let _ = backend;
        let (views, given, indexes) = {
            let _stage = crate::code_rule_dsl::prof::Stage::start("labels.wave.labelled");
            self.labelled_indexed(tapes, active, budget)?
        };
        let anchors = anchors_on();
        let _stage = crate::code_rule_dsl::prof::Stage::start("labels.wave.roots");
        // A clause is W's on a tape where its labels, or those of a clause it reads, went to W there.
        let to_w = |c: usize, t: usize| given.contains(&(c, t)) || self.feeds[c].iter().any(|&fed| given.contains(&(fed, t)));
        let mut limited = Vec::new();
        let mut found = Vec::with_capacity(self.clauses.len());
        for (c, clause) in self.clauses.iter().enumerate() {
            let entry = Entry::of(&clause.program);
            let walk = Micro::of(&clause.program);
            let mut per = Vec::with_capacity(views.len());
            for (t, (view, sets)) in views.iter().enumerate() {
                if !active[c][t] {
                    per.push(BTreeSet::new());
                    continue;
                }
                if to_w(c, t) {
                    limited.push((c, t));
                    per.push(clause.unlabelled_candidates(view).collect());
                    continue;
                }
                // The root walk starts within its anchor (decision 0365): every label it reads is finished.
                let anchor = anchors.then(|| {
                    // The root walk runs on the interpreter, which tests text at each node it reaches: no rows of its own.
                    let anchor = anchor_bits(&clause.program, walk.as_ref(), None, view, sets, indexes[t].get_or_init(Default::default));
                    count_anchored(domain(&clause.domain, view, sets).filter(|&node| entry.admits(view, node)), &anchor);
                    anchor
                });
                let candidates = || anchored(domain(&clause.domain, view, sets).filter(|&node| entry.admits(view, node)), anchor.as_deref());
                per.push(match clause_roots(&clause.program, view, sets, candidates, budget)? {
                    Some(roots) => roots,
                    None => {
                        limited.push((c, t));
                        candidates().collect()
                    }
                });
            }
            found.push(per);
        }
        Ok((found, WaveRun { limited, ..WaveRun::default() }))
    }

    /// The cost store's key for W's matcher at clause `clause`'s roots (`evaluation_costs::w_key`).
    pub fn w_key(&self, clause: usize) -> String {
        crate::code_rule_dsl::transducer::packed::evaluation_costs::w_key(&self.clauses[clause].program)
    }

    /// [`roots_metal`](Self::roots_metal), each half of the batch on its own where the whole outgrows a workspace or
    /// the launch bound (a file's labels read only that file).
    fn roots_metal_split(&self, tapes: &[TapeDoc], roots: &[Vec<Vec<u32>>]) -> Result<(Vec<Vec<BTreeSet<u32>>>, WaveRun), Error> {
        match self.roots_metal(tapes, roots) {
            Err(Error::BackendCapacity | Error::BackendLaunchLimit) if tapes.len() > 1 => {
                let half = tapes.len() / 2;
                let part = |range: std::ops::Range<usize>| roots.iter().map(|clause| clause[range.clone()].to_vec()).collect::<Vec<_>>();
                let (mut left, a) = self.roots_metal_split(&tapes[..half], &part(0..half))?;
                let (right, b) = self.roots_metal_split(&tapes[half..], &part(half..tapes.len()))?;
                for (l, r) in left.iter_mut().zip(right) {
                    l.extend(r);
                }
                // The right half's tapes numbered in the whole batch.
                let limited = a.limited.into_iter().chain(b.limited.into_iter().map(|(c, t)| (c, t + half))).collect();
                Ok((left, WaveRun { metal: true, command_buffers: a.command_buffers + b.command_buffers, device: a.device + b.device, limited, replanned: a.replanned + b.replanned }))
            }
            found => found.map(|(mut found, outcome)| {
                // Where a pass went to W's matcher (decision 0334): its clauses' roots there are their candidates.
                let limited = self.unfinished_clauses(&outcome.unfinished, roots);
                for &(c, t) in &limited {
                    found[c][t] = roots[c][t].iter().copied().collect();
                }
                (found, WaveRun { metal: true, command_buffers: outcome.command_buffers, device: outcome.device, limited, replanned: outcome.replanned })
            }),
        }
    }

    /// The clause that owns label `label` (an index past `LABEL_BASE`; past the wave's, a clause's roots).
    fn label_owner(&self, label: u32) -> Option<usize> {
        let label = label as usize;
        if label >= self.labels {
            return Some(label - self.labels).filter(|&clause| clause < self.clauses.len());
        }
        self.clauses.iter().position(|clause| (usize::from(clause.labels.offset)..usize::from(clause.labels.offset) + clause.labels.defs.len()).contains(&label))
    }

    /// W's matcher's predicted steps on `tape` for what an unfinished pass of label `label` gives it (decision 0334):
    /// the clause that owns the label, and every clause that reads its labels.
    pub fn label_w_cost(&self, label: u32, tape: &TapeDoc) -> u64 {
        let Some(owner) = self.label_owner(label) else { return u64::MAX };
        (0..self.clauses.len()).filter(|&c| c == owner || self.feeds[c].contains(&owner)).map(|c| self.clauses[c].w_cost(tape)).sum()
    }

    /// The (clause, tape) W's matcher takes from a run's unfinished labels (`Outcome::unfinished`): the clause that owns
    /// each (a label past the wave's is a clause's roots) and every clause that reads its labels (`feeds`), where it
    /// has candidates (`roots`).
    fn unfinished_clauses(&self, unfinished: &[(u32, usize)], roots: &[Vec<Vec<u32>>]) -> Vec<(usize, usize)> {
        let given: BTreeSet<(usize, usize)> = unfinished.iter().filter_map(|&(label, tape)| Some((self.label_owner(label)?, tape))).collect();
        let mut limited = BTreeSet::new();
        for (c, feeds) in self.feeds.iter().enumerate() {
            for &(owner, tape) in &given {
                if (owner == c || feeds.contains(&owner)) && roots.get(c).and_then(|roots| roots.get(tape)).is_some_and(|roots| !roots.is_empty()) {
                    limited.insert((c, tape));
                }
            }
        }
        limited.into_iter().collect()
    }

    /// [`labelled`](Self::labelled)'s views, each with its label sets.
    /// [`labelled`](Self::labelled)'s views and sets, the interpreted passes held to `budget` (decision 0334); and the
    /// (clause, tape) whose labels there went to W's matcher (`label_level`, `sweep_with`).
    #[allow(clippy::type_complexity)]
    fn labelled_sets(&self, tapes: &[TapeDoc], active: &[Vec<bool>], budget: WorkBudget) -> Result<(Vec<(TapeDoc, Arc<Vec<Vec<u64>>>)>, BTreeSet<(usize, usize)>), Error> {
        self.labelled_indexed(tapes, active, budget).map(|(views, given, _)| (views, given))
    }

    /// [`labelled_sets`](Self::labelled_sets), with each tape's column sets as the passes built them, for the root
    /// walks' anchors (decision 0365).
    #[allow(clippy::type_complexity)]
    fn labelled_indexed(
        &self,
        tapes: &[TapeDoc],
        active: &[Vec<bool>],
        budget: WorkBudget,
    ) -> Result<(Vec<(TapeDoc, Arc<Vec<Vec<u64>>>)>, BTreeSet<(usize, usize)>, Vec<std::sync::OnceLock<super::compile_sets::Index>>), Error> {
        let active = &self.spread(active);
        let index = |clause: &LoweredMatcher| self.clauses.iter().position(|c| std::ptr::eq(c, clause)).expect("a clause of the wave");
        let mut given = BTreeSet::new();
        let clauses = self.clauses.iter().collect::<Vec<_>>();
        let (steps, _) = levels(&clauses, |clause, id| Ok((clause, id)))?;
        let active = |clause: &LoweredMatcher, tape: usize| {
            // The clause's index from its address in `clauses` (asked per pass and tape: no scan).
            let size = std::mem::size_of::<LoweredMatcher>().max(1);
            let guess = (std::ptr::from_ref(clause) as usize).wrapping_sub(self.clauses.as_ptr() as usize) / size;
            let at = if self.clauses.get(guess).is_some_and(|c| std::ptr::eq(c, clause)) {
                guess
            } else {
                self.clauses.iter().position(|c| std::ptr::eq(c, clause)).expect("a clause of the wave")
            };
            active[at][tape]
        };
        let active: &dyn Fn(&LoweredMatcher, usize) -> bool = &active;
        let mut sets = empty_sets(tapes, self.labels);
        let most = tapes.iter().map(TapeDoc::transducer_len).max().unwrap_or(0);
        // Facts that are node sets are the formulas' (`formulas`): only the others' rows are made here.
        let facts = fact_rows(self.clauses.iter().flat_map(|clause| clause.labels.facts()).filter(|fact| !(formulas_on() && fact.is_node_set())), tapes);
        // Each tape's columns as sets, for the passes that are formulas (`compile_sets`).
        let indexes = tapes.iter().map(|_| std::sync::OnceLock::new()).collect::<Vec<_>>();
        let anchors = anchors_on();
        for (recursive, levels) in &steps {
            if !recursive {
                for level in levels {
                    // A level past the recursive groups reads finished labels only: its walks start within their anchors.
                    let (_, to_w) = label_level(level, tapes, &mut sets, active, &self.packs, &facts, &indexes, budget, anchors)?;
                    given.extend(to_w.into_iter().map(|(clause, tape)| (index(clause), tape)));
                }
                continue;
            }
            if sweeps(levels) {
                for (at, (tape, sets)) in tapes.iter().zip(sets.iter_mut()).enumerate() {
                    let levels = levels
                        .iter()
                        .map(|level| level.iter().copied().filter(|&(clause, _)| active(clause, at)).collect::<Vec<_>>())
                        .collect::<Vec<_>>();
                    if levels.iter().any(|level| !level.is_empty()) && !sweep(&levels, tape, sets, budget)? {
                        given.extend(levels.iter().flatten().map(|&(clause, _)| (index(clause), at)));
                    }
                }
                continue;
            }
            let mut settled = false;
            for _round in 0..=most + 1 {
                let mut changed = false;
                for level in levels {
                    // A group's anchors would read its own labels as they settle: no anchor in its rounds.
                    let (now, to_w) = label_level(level, tapes, &mut sets, active, &self.packs, &facts, &indexes, budget, false)?;
                    changed |= now;
                    given.extend(to_w.into_iter().map(|(clause, tape)| (index(clause), tape)));
                }
                if !changed {
                    settled = true;
                    break;
                }
            }
            // Rounds past every chain the tapes hold, still changing: the ceiling. The group's clauses go to W
            // wherever they run.
            if !settled {
                for &(clause, _) in levels.iter().flatten() {
                    given.extend((0..tapes.len()).filter(|&tape| active(clause, tape)).map(|tape| (index(clause), tape)));
                }
            }
        }
        Ok((tapes.iter().zip(sets).map(|(tape, sets)| (tape.with_labels(Arc::clone(&sets)), sets)).collect(), given, indexes))
    }

    /// Whether clause `clause`'s root program starts at `node` of `tape`: its root's kinds.
    pub fn starts(&self, clause: usize, tape: &TapeDoc, node: u32) -> bool {
        match &self.clauses[clause].domain {
            Domain::All | Domain::Label(_) => true,
            Domain::Kinds(kinds) => kinds.binary_search(&tape.transducer_kind(node)).is_ok(),
        }
    }
}

/// Per tape, `labels` empty label sets.
/// A recursive group's labels by their order at a node: past every member the label reads at the
/// node itself (a class its program tests before it navigates). Evaluated in that order, a node's
/// labels see each other's verdicts in the same pass, so a pass in node order follows the
/// recursion along the tree rather than one level of it per pass.
fn local_levels(labels: &Labels, step: &[u16]) -> BTreeMap<u16, usize> {
    let here = |id: u16| -> Vec<u16> {
        let program = match labels.def(id) {
            Label::Roots(program, _) | Label::Targets(program, _) | Label::Count { program, .. } => program,
            Label::Complement(of) => return vec![*of],
            _ => return Vec::new(),
        };
        let mut read = Vec::new();
        for &word in &program.words {
            match split(word) {
                (SUPERTYPE, _, class) if step.contains(&class) => read.push(class),
                (CHILD | NEXT | PARENT | DESCEND | FOLLOWING | RESTORE_CURSOR, ..) => break,
                _ => {}
            }
        }
        read
    };
    let reads = step.iter().map(|&id| (id, here(id))).collect::<BTreeMap<_, _>>();
    let mut level = step.iter().map(|&id| (id, 0)).collect::<BTreeMap<_, _>>();
    // Longest same-node chains, bounded by the group (a cycle of same-node reads stops there).
    for _ in 0..step.len() {
        let mut changed = false;
        for &id in step {
            let deepest = reads[&id].iter().filter(|read| **read != id).map(|read| level[read] + 1).max().unwrap_or(0);
            let deepest = deepest.min(step.len() - 1);
            if deepest > level[&id] {
                level.insert(id, deepest);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    level
}

pub fn empty_sets(tapes: &[TapeDoc], labels: usize) -> Vec<Arc<Vec<Vec<u64>>>> {
    tapes
        .iter()
        .map(|tape| Arc::new(vec![vec![0_u64; tape.transducer_len().div_ceil(64)]; labels]))
        .collect()
}

fn has(sets: &[Vec<u64>], label: u16, node: u32) -> bool {
    sets[usize::from(label - LABEL_BASE)][node as usize / 64] >> (node % 64) & 1 != 0
}

/// The tests a program makes of its start before it moves, forks or captures: a kind, or a
/// grammar supertype (labels are not known on the host before Metal runs). A start that fails one
/// has no outputs, so it is not started.
pub struct Entry(pub Vec<(u8, u16)>);

impl Entry {
    pub fn of(program: &Program) -> Self {
        let mut tests = Vec::new();
        for &word in &program.words {
            match split(word) {
                (KIND, 0, kind) => tests.push((KIND, kind)),
                (SUPERTYPE, 0, class) if class < LABEL_BASE => tests.push((SUPERTYPE, class)),
                (SUPERTYPE, 0, _) => {}
                _ => break,
            }
        }
        Self(tests)
    }

    pub fn admits(&self, tape: &TapeDoc, node: u32) -> bool {
        self.0.iter().all(|&(op, arg)| match op {
            KIND => tape.transducer_kind(node) == arg,
            _ => tape.transducer_has_supertype(node, arg),
        })
    }
}

/// `program`'s outputs from each tape's `starts`.
fn run(
    program: &Program,
    views: &[TapeDoc],
    starts: &Domain,
    sets: &[Arc<Vec<Vec<u64>>>],
) -> Result<Vec<Vec<Output>>, Error> {
    let entry = Entry::of(program);
    views
        .iter()
        .zip(sets)
        .map(|(view, sets)| program.execute_roots(view, domain(starts, view, sets).filter(|&node| entry.admits(view, node))))
        .collect()
}

/// The device's label passes (`transducer::packed::device`: on macOS, Metal's `compile/matcher_metal.rs`), for
/// the batch: `UnsupportedBackend` where no device is registered.
impl LoweredMatcher {
    /// The batch's roots with every pass on the device in one command buffer, pattern leaves included.
    pub fn roots_metal(&self, tapes: &[TapeDoc]) -> Result<(Vec<BTreeSet<u32>>, super::device::Outcome), Error> {
        super::device::required()?.matcher_roots(self, tapes)
    }
}

/// One clause's roots on one tape (decision 0334), or `None` where W's matcher is to confirm its candidates
/// (`run_budgeted`).
///
/// The plans weighed at an overrun (§5): W's matcher, the transducer again on the roots not yet finished, and the
/// program's micro walk on them, where it is one with text tests (the prefilter's plan: each text test read as bits
/// from the term index, never tested per node, `Micro::slot`). A walk that outgrows its bound leaves the rest to the
/// transducer and W, by cost.
fn clause_roots<I: Iterator<Item = u32>>(program: &Program, view: &TapeDoc, sets: &[Vec<u64>], candidates: impl Fn() -> I, budget: WorkBudget) -> Result<Option<BTreeSet<u32>>, Error> {
    let roots: Vec<u32> = candidates().collect();
    let w = || costs::predict_w(program, subtree_nodes(view, &roots));
    let walk = Micro::of(program).filter(|walk| !walk.texts.is_empty());
    let bytes = view.transducer_span(0).end as u64;
    let walk_cost = |rest: &[u32]| walk.as_ref().map(|walk| costs::predict_walk(program, subtree_nodes(view, rest), walk.texts.len(), bytes));
    let starts = |outputs: Vec<transducer_packed::Output>| outputs.into_iter().map(|output| output.start);
    match run_budgeted(program, view, &roots, budget, &w, &walk_cost, "ast clause")? {
        Ran::Done(outputs) => Ok(Some(starts(outputs).collect())),
        Ran::W => Ok(None),
        Ran::Walk { kept, rest, budget } => {
            let mut found: BTreeSet<u32> = starts(kept).collect();
            let walk = walk.expect("a walk was weighed");
            if let Some(texts) = walk.texts(program, view) {
                let mut frames = Frames::default();
                let walked = roots[rest..].iter().try_for_each(|&start| walk.run(view, sets, &texts, start, Emit::Start, &mut frames, &mut |node| {
                    found.insert(node);
                }));
                if walked.is_ok() {
                    crate::code_rule_dsl::prof::count("replan.walked", 1);
                    return Ok(Some(found));
                }
            }
            // The walk outgrew its bound: the rest by the transducer or W, by cost, without the walk.
            let ran = run_budgeted(program, view, &roots[rest..], budget, &w, &|_: &[u32]| None, "ast clause, the walk outgrown")?;
            Ok(match ran {
                Ran::Done(outputs) => Some(found.into_iter().chain(starts(outputs)).collect()),
                _ => None,
            })
        }
    }
}

/// What `run_budgeted` ran, or which plan it chose for the rest (decision 0334 §5).
enum Ran {
    /// The transducer finished the evaluation: its outputs.
    Done(Vec<transducer_packed::Output>),
    /// W's matcher takes the evaluation.
    W,
    /// The caller's micro walk takes the roots from `rest` on; the roots before kept `kept`. `budget` is the one the
    /// transducer would have run the rest with, should the walk outgrow its bound.
    Walk { kept: Vec<transducer_packed::Output>, rest: usize, budget: WorkBudget },
}

/// The nodes of `roots`' subtrees, summed: what W's matcher visits to confirm them.
fn subtree_nodes(view: &TapeDoc, roots: &[u32]) -> u64 {
    roots.iter().map(|&root| u64::from(view.transducer_descendants(root).end.saturating_sub(root).max(1))).sum()
}

/// `program` from `roots` on one tape under `budget` (decision 0334): its outputs (`Ran::Done`), or the plan that takes
/// over: W's matcher (`Ran::W`: the caller's clauses on this tape, their candidates confirmed by W), or the caller's
/// micro walk on the roots not yet finished (`Ran::Walk`).
///
/// Under the full budget the interpreter runs to the ceiling, and W is a stop there. Replanned, it is held to its
/// predicted steps over the roots (its rate, else the prior) times the slack. At an overrun the roots that finished
/// keep their outputs (the evaluation split by root, `execute_roots_split`), and the rest is replanned by cost: W's
/// matcher (`w`, its predicted steps), the micro walk (`walk`, its prediction on the rest; `None` where there is
/// none) and the transducer again on the rest with twice the budget (at most that many steps); the cheapest runs.
/// Each run is observed: the program's rate follows at once.
fn run_budgeted(
    program: &Program,
    view: &TapeDoc,
    roots: &[u32],
    budget: WorkBudget,
    w: &dyn Fn() -> u64,
    walk: &dyn Fn(&[u32]) -> Option<u64>,
    what: &str,
) -> Result<Ran, Error> {
    // Under the full budget the limit is none (`evaluation_limit`): the run is observed all the same, for the rates.
    let size: u64 = roots
        .iter()
        .map(|&root| {
            let span = view.transducer_span(root);
            costs::size(program, view.transducer_descendants(root).end.saturating_sub(root).max(1), span.end.saturating_sub(span.start) as u32)
        })
        .sum();
    // One prediction for the evaluation (its floor is the evaluation's, not each root's).
    let predicted = costs::predict(program, costs::rate(program.plain_digest()), size);
    // The ceiling: every root's own (`MAX_WORK_PER_TASK`), where the interpreter stops with `WorkLimit`.
    let ceiling = (transducer_packed::MAX_WORK_PER_TASK as u64).saturating_mul(roots.len().max(1) as u64);
    let mut budget = budget;
    // Split by root (decision 0334 §5): the roots that finished before an overrun keep their outputs and steps, and a
    // rerun takes only the rest, within the evaluation's doubled budget less the steps kept.
    let (mut outputs, mut kept, mut start) = (Vec::new(), 0_usize, 0_usize);
    loop {
        let limit = budget.evaluation_limit(predicted, ceiling);
        let observed = |steps: usize, over_budget: bool| {
            costs::observe([costs::Observation { program: program.plain_digest().to_owned(), steps: steps as u64, size, predicted, over_budget }]);
        };
        let mut partial = transducer_packed::Partial::default();
        match program.execute_roots_split(view, roots[start..].iter().copied(), limit.saturating_sub(kept), &mut partial) {
            Ok((rest, used)) => {
                observed(kept + used, false);
                // Each output's ordinal is its index in the evaluation's outputs: the rest's follow the kept ones.
                let base = outputs.len() as u32;
                outputs.extend(rest.into_iter().map(|output| transducer_packed::Output { local_ordinal: output.local_ordinal + base, ..output }));
                return Ok(Ran::Done(outputs));
            }
            Err(Error::OverBudget) => {
                observed(limit, true);
                (kept, start) = (kept + partial.steps, start + partial.finished);
                outputs.extend(partial.outputs);
                // The transducer again: the doubled budget, less what the finished roots keep.
                let (w, again, walked) = (w(), (limit as u64).saturating_mul(2).saturating_sub(kept as u64), walk(&roots[start..]));
                let chosen = if walked.is_some_and(|walked| walked < w.min(again)) {
                    "the micro walk"
                } else if w < again {
                    "W"
                } else {
                    "again"
                };
                crate::code_rule_dsl::logic::profile::decision("over its budget, replanned by cost (0334)", || {
                    format!("{what}, {} roots ({start} finished): W {w} steps, the transducer again on the rest {again}, the walk {walked:?}: {chosen}", roots.len())
                });
                budget = budget.doubled();
                match chosen {
                    "the micro walk" => return Ok(Ran::Walk { kept: outputs, rest: start, budget }),
                    "W" => return Ok(Ran::W),
                    _ => {}
                }
            }
            Err(Error::WorkLimit) => return Ok(Ran::W),
            Err(e) => return Err(e),
        }
    }
}

impl LoweredWave {
    /// Starts compiling the device's label passes' pipelines, so a wave's first labels need not wait for them.
    pub fn warm_metal() {
        if let Some(device) = super::device::matcher() {
            device.warm_labels();
        }
    }

    /// Every clause's labels over the batch on the device, as views of its tapes the root programs read.
    pub fn labelled_metal(&self, tapes: &[TapeDoc], active: &[Vec<bool>], roots: Option<&super::device::RootJobs<'_>>) -> Result<(Vec<TapeDoc>, super::device::Outcome), Error> {
        super::device::required()?.wave_labelled(self, tapes, active, roots)
    }

    /// Each clause's matches of the batch from `roots`, every clause's passes and roots in one command buffer.
    pub fn roots_metal(&self, tapes: &[TapeDoc], roots: &[Vec<Vec<u32>>]) -> Result<(Vec<Vec<BTreeSet<u32>>>, super::device::Outcome), Error> {
        super::device::required()?.wave_roots(self, tapes, roots)
    }
}

/// Label words over the batch's nodes (label-major, `words` a label) as each tape's label sets.
pub fn to_sets(bits: &[u32], labels: usize, words: usize, tapes: &[TapeDoc]) -> Vec<Arc<Vec<Vec<u64>>>> {
    let at = |label: usize, bit: usize| -> u64 {
        let word = |i: usize| u128::from(bits.get(label * words + i).copied().unwrap_or(0));
        let i = bit / 32;
        let wide = word(i) | word(i + 1) << 32 | word(i + 2) << 64;
        (wide >> (bit % 32)) as u64
    };
    use rayon::prelude::*;
    let bases = tapes
        .iter()
        .scan(0, |base, tape| {
            let at = *base;
            *base += tape.transducer_len();
            Some(at)
        })
        .collect::<Vec<_>>();
    tapes
        .par_iter()
        .zip(bases)
        .map(crate::code_rule_dsl::logic::profile::carried("label rows (pool)", |(tape, base): (&TapeDoc, usize)| {
            let len = tape.transducer_len();
            let sets = (0..labels)
                .map(|label| {
                    (0..len.div_ceil(64))
                        .map(|k| {
                            let value = at(label, base + 64 * k);
                            let left = len - 64 * k;
                            if left < 64 { value & ((1 << left) - 1) } else { value }
                        })
                        .collect()
                })
                .collect();
            Arc::new(sets)
        }))
        .collect()
}

/// `to_sets` the other way.
pub fn to_words(sets: &[Arc<Vec<Vec<u64>>>], labels: usize, words: usize, tapes: &[TapeDoc]) -> Vec<u32> {
    use rayon::prelude::*;
    let mut bits = vec![0_u32; labels * words];
    if words == 0 {
        return bits;
    }
    let bases = tapes
        .iter()
        .scan(0, |base, tape| {
            let at = *base;
            *base += tape.transducer_len();
            Some(at)
        })
        .collect::<Vec<_>>();
    // Label-major: each label's row is its own (06 S7: one thread, ~10 ms a level).
    bits.par_chunks_mut(words).enumerate().for_each(|(label, row)| {
        for (&base, sets) in bases.iter().zip(sets) {
            let Some(set) = sets.get(label) else { continue };
            for (k, &value) in set.iter().enumerate() {
                if value == 0 {
                    continue;
                }
                let bit = base + 64 * k;
                let wide = u128::from(value) << (bit % 32);
                for part in 0..3 {
                    if let Some(word) = row.get_mut(bit / 32 + part) {
                        *word |= (wide >> (32 * part)) as u32;
                    }
                }
            }
        }
    });
    bits
}

/// A wave's label passes as its plan runs them, on either backend: by wave level, its
/// non-recursive passes (one level each, past the levels of the labels they read), then its
/// recursive groups merged (their levels by position at a node, `local_levels`; their rounds
/// shared). Each step is (recursive, its levels of passes); with each, for Metal's round count, a
/// recursive step's labels' kinds and whether its recursion runs along siblings.
#[allow(clippy::type_complexity)]
pub fn levels<'a, T>(
    clauses: &[&'a LoweredMatcher],
    mut pass: impl FnMut(&'a LoweredMatcher, u16) -> Result<T, Error>,
) -> Result<(Vec<(bool, Vec<Vec<T>>)>, Vec<(Option<Vec<u16>>, bool)>), Error> {
    // By wave level: its non-recursive passes, and its recursive groups merged (their levels by
    // position, their rounds shared, their recursive labels' kinds and whether any steps to
    // siblings).
    let mut single: Vec<Vec<T>> = Vec::new();
    let mut groups: Vec<(Vec<Vec<T>>, Option<BTreeSet<u16>>, bool)> = Vec::new();
    // When each label is final: (wave level, whether in the level's recursive part). Over the
    // wave: a clause reads labels an earlier clause owns (the combined graph).
    let mut done = BTreeMap::<u16, (usize, bool)>::new();
    for &clause in clauses {
        for step in &clause.labels.steps {
            let recursive = clause.labels.recursive.values().any(|id| step.contains(id));
            let outside = |id: u16| clause.labels.reads(id).into_iter().filter(|read| !step.contains(read)).collect::<Vec<_>>();
            if !recursive {
                for &id in step {
                    let at = outside(id).iter().filter_map(|read| done.get(read)).map(|(level, _)| level + 1).max().unwrap_or(0);
                    if single.len() <= at {
                        single.resize_with(at + 1, Vec::new);
                    }
                    single[at].push(pass(clause, id)?);
                    done.insert(id, (at, false));
                }
                continue;
            }
            // A group runs after the single passes of its level and the groups of the level before.
            let at = step
                .iter()
                .flat_map(|&id| outside(id))
                .filter_map(|read| done.get(&read))
                .map(|(level, group)| level + usize::from(*group))
                .max()
                .unwrap_or(0);
            if groups.len() <= at {
                groups.resize_with(at + 1, || (Vec::new(), Some(BTreeSet::new()), false));
            }
            // Inside the group a label's level is past the members it reads at its own node
            // (`local_levels`); a read of a member at the same or a later level is the last
            // round's (or, sweeping, an earlier node's).
            let local = local_levels(&clause.labels, step);
            let mut ordered = step.clone();
            ordered.sort_by_key(|id| local[id]);
            for &id in &ordered {
                let inner = local[&id];
                let levels = &mut groups[at].0;
                if levels.len() <= inner {
                    levels.resize_with(inner + 1, Vec::new);
                }
                levels[inner].push(pass(clause, id)?);
            }
            let (_, kinds, siblings) = &mut groups[at];
            for id in step.iter().filter(|id| clause.labels.recursive.values().any(|label| label == *id)) {
                match (clause.labels.def(*id), kinds.as_mut()) {
                    (Label::Roots(_, Domain::Kinds(own)), Some(kinds)) => kinds.extend(own.iter().copied()),
                    _ => *kinds = None,
                }
            }
            // The recursion runs along siblings only where a program that steps to siblings tests a
            // label of the group itself; a sibling walk to a label settled before the group does not
            // lengthen its chains.
            *siblings |= step.iter().any(|id| match clause.labels.def(*id) {
                Label::Roots(program, _) | Label::Targets(program, _) | Label::Count { program, .. } => {
                    program.words.iter().any(|&word| matches!(split(word).0, NEXT | FOLLOWING))
                        && program.words.iter().any(|&word| matches!(split(word), (SUPERTYPE, _, class) if step.contains(&class)))
                }
                _ => false,
            });
            for &id in step {
                done.insert(id, (at, true));
            }
        }
    }
    let mut steps: Vec<(bool, Vec<Vec<T>>)> = Vec::new();
    let mut chains: Vec<(Option<Vec<u16>>, bool)> = Vec::new();
    let mut groups = groups.into_iter();
    for at in 0..single.len().max(groups.len()) {
        if let Some(passes) = single.get_mut(at).filter(|passes| !passes.is_empty()) {
            steps.push((false, vec![std::mem::take(passes)]));
            chains.push((None, false));
        }
        if let Some((levels, kinds, siblings)) = groups.next().filter(|(levels, ..)| !levels.is_empty()) {
            steps.push((true, levels));
            chains.push((kinds.map(|kinds| kinds.into_iter().collect()), siblings));
        }
    }
    Ok((steps, chains))
}

/// One level of a wave's label passes over the batch, each as `evaluate` has it (they read only
/// levels before), each on the tapes where its clause is `active`. Whether a set changed.
/// A level's roots and targets passes over one domain, fused into one walk (t1 spec-fusion §2.6
/// multi-target packing, §4.3, §4.4: `fusion::Fused`), by the labels they mark: built once a wave
/// and shared by its batches. A start's walk answers every member; a member's emits mark its
/// label. Metal packs the same passes (`label_passes::Runner::size`).
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Packs(std::sync::Mutex<HashMap<Vec<u16>, Option<Arc<(Program, Micro, Vec<Entry>, Vec<usize>)>>>>);

impl Packs {
    /// A level's passes grouped to fuse: by pass kind (roots or targets) and domain, micro walks
    /// only, at most `fusion::MEMBERS` a group, two or more.
    fn groups(level: &[(&LoweredMatcher, u16)]) -> Vec<(bool, Vec<usize>)> {
        let mut by = BTreeMap::<(bool, String), Vec<usize>>::new();
        for (at, &(clause, id)) in level.iter().enumerate() {
            let (targets, program, domain) = match clause.labels.def(id) {
                Label::Roots(program, domain) => (false, program, domain),
                Label::Targets(program, domain) => (true, program, domain),
                _ => continue,
            };
            if Micro::of(program).is_some() {
                by.entry((targets, format!("{domain:?}"))).or_default().push(at);
            }
        }
        by.into_iter()
            .flat_map(|((targets, _), members)| {
                members.chunks(super::fusion::MEMBERS).map(|chunk| (targets, chunk.to_vec())).collect::<Vec<_>>()
            })
            .filter(|(_, members)| members.len() > 1)
            .collect()
    }

    /// Per pass of the level, its (tape, bits) where it ran fused; None where it runs alone (not
    /// grouped, or its group's walk failed on a tape).
    #[allow(clippy::type_complexity)]
    fn walk(
        &self,
        level: &[(&LoweredMatcher, u16)],
        views: &[TapeDoc],
        sets: &[Arc<Vec<Vec<u64>>>],
        active: &dyn Fn(&LoweredMatcher, usize) -> bool,
        anchors: Option<&[std::sync::OnceLock<super::compile_sets::Index>]>,
    ) -> Vec<Option<Vec<(usize, Vec<u64>)>>> {
        use rayon::prelude::*;
        let mut out = vec![None; level.len()];
        for (targets, members) in Self::groups(level) {
            let key = members.iter().map(|&at| level[at].1).collect::<Vec<_>>();
            let fused = {
                let mut cache = self.0.lock().expect("packs");
                cache
                    .entry(key)
                    .or_insert_with(|| {
                        let programs = members.iter().map(|&at| level[at].0.labels.def(level[at].1).program()).collect::<Option<Vec<_>>>()?;
                        let fused = super::fusion::Fused::of(&programs)?;
                        let walk = Micro::of(&fused.program)?;
                        let members = fused.emits.iter().map(|&(member, _)| member).collect();
                        Some(Arc::new((fused.program, walk, programs.iter().map(|program| Entry::of(program)).collect(), members)))
                    })
                    .clone()
            };
            let Some(fused) = fused else { continue };
            let (program, walk, entries, member_of) = &*fused;
            let domain = match level[members[0]].0.labels.def(level[members[0]].1) {
                Label::Roots(_, domain) | Label::Targets(_, domain) => domain.clone(),
                _ => unreachable!("a grouped pass is a roots or targets label"),
            };
            let emit = if targets { Emit::Captures } else { Emit::Start };
            let on = (0..views.len()).filter(|&tape| members.iter().any(|&at| active(level[at].0, tape))).collect::<Vec<_>>();
            let found = on
                .par_iter()
                .map(crate::code_rule_dsl::logic::profile::carried("text walk (pool)", |&tape: &usize| {
                    let (view, sets) = (&views[tape], &sets[tape]);
                    let texts = walk.texts(program, view)?;
                    // A start any member's anchor holds (decision 0365): the members' anchors, each from its own walk.
                    let anchor = anchors.map(|indexes| {
                        let index = indexes[tape].get_or_init(Default::default);
                        let mut union = vec![0_u64; view.transducer_len().div_ceil(64)];
                        for &at in &members {
                            let (clause, id) = level[at];
                            let Some(program) = clause.labels.def(id).program() else { continue };
                            // The fused walk's rows are by its own slots: a member's anchor reads the term index's.
                            let member = Micro::of(program);
                            for (word, bits) in union.iter_mut().zip(anchor_bits(program, member.as_ref(), None, view, sets, index)) {
                                *word |= bits;
                            }
                        }
                        union
                    });
                    if let Some(anchor) = &anchor {
                        count_anchored(self::domain(&domain, view, sets).filter(|&start| entries.iter().any(|entry| entry.admits(view, start))), anchor);
                    }
                    let mut bits = vec![vec![0_u64; view.transducer_len().div_ceil(64)]; members.len()];
                    let mut frames = Frames::default();
                    for start in anchored(self::domain(&domain, view, sets), anchor.as_deref()) {
                        if !entries.iter().any(|entry| entry.admits(view, start)) {
                            continue;
                        }
                        walk.run_labelled(view, sets, &texts, start, emit, &mut frames, &mut |label, node| {
                            bits[member_of[usize::from(label)]][node as usize / 64] |= 1 << (node % 64);
                        })
                        .ok()?;
                    }
                    Some((tape, bits))
                }))
                .collect::<Vec<_>>();
            // A failed tape (text slots, an overflow): the group runs alone.
            let Some(found) = found.into_iter().collect::<Option<Vec<_>>>() else {
                crate::code_rule_dsl::prof::count("labels.pack_failed", members.len());
                continue;
            };
            crate::code_rule_dsl::prof::count("labels.packed", members.len());
            crate::code_rule_dsl::prof::count("labels.packs", 1);
            let mut per = vec![Vec::new(); members.len()];
            for (tape, bits) in found {
                for (k, bits) in bits.into_iter().enumerate() {
                    if active(level[members[k]].0, tape) {
                        per[k].push((tape, bits));
                    }
                }
            }
            for (k, found) in per.into_iter().enumerate() {
                out[members[k]] = Some(found);
            }
        }
        out
    }
}

/// Per pass of the level, its (tape, bits) where its program is a node-set formula
/// (`compile_sets::Set`), evaluated on each active tape; `None` where it runs as a walk.
/// `RERAIL_SETS=walk` runs every pass as a walk; `RERAIL_SETS_CHECK` runs both and reports where
/// they differ.
#[allow(clippy::type_complexity)]
/// Why a pass is neither a formula nor a packed walk, as a counter: `labels.fallback.<label kind>`,
/// with the program's operations outside the formulas' fragment (hex) where it is a program, or
/// `micro` where it is not a micro walk. Each common reason is a formula form to add (`compile_sets`).
fn fallback(label: &Label) -> &'static str {
    static NAMES: std::sync::LazyLock<std::sync::Mutex<HashMap<String, &'static str>>> = std::sync::LazyLock::new(Default::default);
    const FRAGMENT: [u8; 15] = [KIND, FIELD, FLAGS, SUPERTYPE, TEXT, SAVE_CURSOR, RESTORE_CURSOR, CHILD, DESCEND, PARENT, FOLLOWING, FORK, GOTO, CAPTURE, EMIT];
    let outside = |program: &Program| match Micro::of(program) {
        None => "micro".to_owned(),
        Some(walk) => walk.words.iter().map(|&word| split(word).0).filter(|op| !FRAGMENT.contains(op)).collect::<BTreeSet<_>>().iter().map(|op| format!("{op:02x}")).collect::<Vec<_>>().join(","),
    };
    let name = match label {
        Label::Pattern(..) => "pattern".to_owned(),
        Label::Constrained(..) => "constrained".to_owned(),
        Label::Roots(program, _) => format!("roots[{}]", outside(program)),
        Label::Targets(program, _) => format!("targets[{}]", outside(program)),
        Label::Count { program, .. } => format!("count[{}]", outside(program)),
        Label::Complement(_) => "complement".to_owned(),
        Label::Fact(..) => "fact".to_owned(),
        Label::Empty => "empty".to_owned(),
    };
    NAMES.lock().expect("fallback names").entry(name).or_insert_with_key(|name| Box::leak(format!("labels.fallback.{name}").into_boxed_str()))
}

/// Whether label passes run as node-set formulas where they are ones (`compile_sets`); off with
/// `RERAIL_SETS=walk`.
pub fn formulas_on() -> bool {
    // Read at each level, so a check may run both ways in one process (`on_metal_anchored_walks_find_the_unanchored_roots`).
    std::env::var("RERAIL_SETS").map_or(true, |mode| mode != "walk")
}

fn formulas(
    level: &[(&LoweredMatcher, u16)],
    views: &[TapeDoc],
    sets: &[Arc<Vec<Vec<u64>>>],
    active: &dyn Fn(&LoweredMatcher, usize) -> bool,
    indexes: &[std::sync::OnceLock<super::compile_sets::Index>],
) -> Vec<Option<Vec<(usize, Vec<u64>)>>> {
    use super::compile_sets::{Index, Reader, Set};
    use rayon::prelude::*;
    static CHECK: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("RERAIL_SETS_CHECK").is_some());
    if !formulas_on() {
        return vec![None; level.len()];
    }
    // What each pass is as a formula, if it is one.
    enum Plan<'a> {
        /// A fact whose findings are a node set (`nthChild`): read from the tape's sibling positions.
        Fact(&'a crate::code_rule_dsl::matcher::Fact),
        Set { walk: Micro, program: &'a Program, domain: &'a Domain, set: Set },
    }
    let plans = level
        .iter()
        .map(|&(clause, id)| {
            let label = clause.labels.def(id);
            if let Label::Fact(fact, _) = label
                && fact.is_node_set()
            {
                return Some(Plan::Fact(fact));
            }
            let (Label::Roots(program, domain) | Label::Targets(program, domain)) = label else { return None };
            let walk = Micro::of(program)?;
            // The starts: the domain's nodes.
            let starts = match domain {
                Domain::All => Set::All,
                Domain::Kinds(kinds) => Set::Or(kinds.iter().map(|&kind| Set::Kind(kind)).collect()),
                Domain::Label(label) => Set::Class(*label),
            };
            let set = match label {
                Label::Targets(..) => Set::targets(&walk.words, starts)?,
                _ => Set::And(vec![starts, Set::of(&walk.words)?]),
            };
            Some(Plan::Set { walk, program, domain, set })
        })
        .collect::<Vec<_>>();
    let on = level.iter().map(|&(clause, _)| (0..views.len()).map(|tape| active(clause, tape)).collect::<Vec<_>>()).collect::<Vec<_>>();
    // File-major: each tape answers every formula of the level at once (its index and texts warm),
    // as one task; a formula's answer on one small file is too little work for a task of its own.
    let answers = (0..views.len())
        .into_par_iter()
        .map(crate::code_rule_dsl::logic::profile::carried("set formulas (pool)", |tape: usize| {
            let view = &views[tape];
            let index = indexes[tape].get_or_init(Index::default);
            plans
                .iter()
                .enumerate()
                .filter(|&(at, plan)| plan.is_some() && on[at][tape])
                .map(|(at, plan)| {
                    let bits = match plan.as_ref().expect("a plan") {
                        Plan::Fact(fact) => Some(Arc::unwrap_or_clone(fact.nth_bits(view, &index.siblings).expect("a node set"))),
                        Plan::Set { walk, program, domain, set } => walk.texts(program, view).map(|texts| {
                            let bits = Reader { tape: view, index, labels: &sets[tape], texts: &texts }.eval(set);
                            if *CHECK {
                                let starts = (0..view.transducer_len() as u32).filter(|&node| match domain {
                                    Domain::All => true,
                                    Domain::Kinds(kinds) => kinds.binary_search(&view.transducer_kind(node)).is_ok(),
                                    Domain::Label(label) => has(&sets[tape], *label, node),
                                });
                                let (clause, id) = level[at];
                                if let Some(walked) = walked(clause.labels.def(id), walk, program, view, &sets[tape], starts)
                                    && walked != bits
                                {
                                    crate::code_rule_dsl::prof::count("labels.formula_differs", 1);
                                    eprintln!("SETS_CHECK differs on label {id}: {set:?}");
                                }
                            }
                            bits
                        }),
                    };
                    (at, bits)
                })
                .collect::<Vec<_>>()
        }))
        .collect::<Vec<_>>();
    // By pass: each tape's bits, in tape order; a pass with a tape it could not answer (its texts)
    // is not a formula.
    let mut found = plans.iter().map(|plan| plan.as_ref().map(|_| Vec::new())).collect::<Vec<Option<Vec<(usize, Vec<u64>)>>>>();
    for (tape, answers) in answers.into_iter().enumerate() {
        for (at, bits) in answers {
            match bits {
                Some(bits) => {
                    if let Some(found) = &mut found[at] {
                        found.push((tape, bits));
                    }
                }
                None => found[at] = None,
            }
        }
    }
    crate::code_rule_dsl::prof::count("labels.formula", found.iter().flatten().count());
    found
}

/// Returns whether a label changed, and the (clause, tape) whose pass there went to W's matcher (`run_budgeted`): its
/// label there is left as it was, and the clause, with every clause that reads its labels, is W's on that tape.
fn label_level<'a>(
    level: &[(&'a LoweredMatcher, u16)],
    tapes: &[TapeDoc],
    sets: &mut [Arc<Vec<Vec<u64>>>],
    active: &dyn Fn(&LoweredMatcher, usize) -> bool,
    packs: &Packs,
    facts: &FactRows,
    indexes: &[std::sync::OnceLock<super::compile_sets::Index>],
    budget: WorkBudget,
    anchored: bool,
) -> Result<(bool, Vec<(&'a LoweredMatcher, usize)>), Error> {
    let views = tapes.iter().zip(sets.iter()).map(|(tape, sets)| tape.with_labels(Arc::clone(sets))).collect::<Vec<_>>();
    // Passes that are node-set formulas: no walk (`compile_sets`).
    let mut formulas = crate::code_rule_dsl::prof::time("labels.formulas", || formulas(level, &views, sets, active, indexes));
    // The others walked from the same starts, as one walk (multi-target packing, `Packs`).
    let walked = level.iter().zip(&formulas).filter(|(_, found)| found.is_none()).map(|(pass, _)| *pass).collect::<Vec<_>>();
    let mut packed_walked = crate::code_rule_dsl::prof::time("labels.packed_walk", || packs.walk(&walked, &views, sets, active, anchored.then_some(indexes))).into_iter();
    let mut packed = formulas.iter().map(|found| if found.is_some() { None } else { packed_walked.next().flatten() }).collect::<Vec<_>>();
    let found = level
        .iter()
        .enumerate()
        .map(|(at, &(clause, id))| {
            if let Some(found) = formulas[at].take() {
                return Ok(found.into_iter().map(|(at, set)| (at, Some(set))).collect());
            }
            if let Some(found) = packed[at].take() {
                return Ok(found.into_iter().map(|(at, set)| (at, Some(set))).collect());
            }
            let on = (0..tapes.len()).filter(|&at| active(clause, at)).collect::<Vec<_>>();
            let (views, read) = (on.iter().map(|&at| views[at].clone()).collect::<Vec<_>>(), on.iter().map(|&at| Arc::clone(&sets[at])).collect::<Vec<_>>());
            let why = fallback(clause.labels.def(id));
            crate::code_rule_dsl::prof::count(why, 1);
            let anchors = anchored.then(|| on.iter().map(|&at| &indexes[at]).collect::<Vec<_>>());
            let found = crate::code_rule_dsl::prof::time("labels.fallback", || clause.evaluate_budgeted(clause.labels.def(id), &views, &read, facts, budget, anchors.as_deref()))?;
            Ok::<_, Error>(on.into_iter().zip(found).collect::<Vec<_>>())
        })
        .collect::<Result<Vec<_>, _>>()?;
    drop(views);
    let (mut changed, mut given) = (false, Vec::new());
    for (&(clause, id), found) in level.iter().zip(found) {
        for (at, set) in found {
            let Some(set) = set else {
                given.push((clause, at));
                continue;
            };
            let slot = &mut Arc::make_mut(&mut sets[at])[usize::from(id - LABEL_BASE)];
            if *slot != set {
                *slot = set;
                changed = true;
            }
        }
    }
    Ok((changed, given))
}

/// Whether a recursive step sweeps, as on Metal (`label_passes::sweeps`): its passes are programs
/// that mark their roots or captures (only adding to their labels), empty or complements; with
/// complements, its programs mark their roots (each node's verdicts are then its own, computed at
/// it in a reverse sweep, since walks through `not` move forward in preorder, `matcher::lower`).
pub fn sweeps(levels: &[Vec<(&LoweredMatcher, u16)>]) -> bool {
    let defs = || levels.iter().flatten().map(|&(clause, id)| clause.labels.def(id));
    let complements = defs().any(|label| matches!(label, Label::Complement(_)));
    defs().all(|label| match label {
        Label::Roots(..) | Label::Empty | Label::Complement(_) => true,
        Label::Targets(..) => !complements,
        _ => false,
    }) && defs().any(|label| matches!(label, Label::Roots(..) | Label::Targets(..)))
}

/// A sweeping step on one tape (06 S4): the tape's nodes in preorder (reverse preorder with
/// complements, or every other sweep), each node's passes in their level's order (`local_levels`),
/// each program run at the node and its results marked at once, so later work of the sweep reads
/// them. With complements every verdict at a node is set or cleared there (O29), so the sweeps
/// reach the rounds' fixpoint. A recursion along the tree settles in a sweep or two, not a node
/// depth per round.
fn sweep(levels: &[Vec<(&LoweredMatcher, u16)>], tape: &TapeDoc, sets: &mut Arc<Vec<Vec<u64>>>, budget: WorkBudget) -> Result<bool, Error> {
    sweep_with(levels, tape, sets, &|walk, program| walk.texts(program, tape), budget, true)
}

/// `sweep`, each walk's text slots from `texts` (on Metal, the runs' rows: `TextRows`), its passes on the interpreter
/// held to `budget` (decision 0334). `false` where the group went to W's matcher on this tape (at the ceiling, or
/// where W is predicted cheaper than the sweep again with twice the budget): its labels there are as far as they got,
/// and its clauses, with every clause that reads their labels, are W's on the tape. Without a W (`w`), an overrun runs
/// again with twice the budget and a stop at the ceiling is `WorkLimit`.
pub fn sweep_with(
    levels: &[Vec<(&LoweredMatcher, u16)>],
    tape: &TapeDoc,
    sets: &mut Arc<Vec<Vec<u64>>>,
    texts: &(dyn Fn(&Micro, &Program) -> Option<Vec<Vec<u64>>> + Sync),
    budget: WorkBudget,
    w: bool,
) -> Result<bool, Error> {
    let mut budget = budget;
    loop {
        match sweep_run(levels, tape, sets, texts, budget) {
            Ok(()) => return Ok(true),
            Err(Error::WorkLimit) if w => return Ok(false),
            Err(Error::OverBudget) => {}
            Err(e) => return Err(e),
        }
        // Over its budget: W's matcher for the group's clauses here against the sweep again with twice the budget.
        let w_allowed = w;
        let mut clauses: Vec<&LoweredMatcher> = levels.iter().flatten().map(|&(clause, _)| clause).collect();
        clauses.sort_by_key(|clause| std::ptr::from_ref(*clause) as usize);
        clauses.dedup_by_key(|clause| std::ptr::from_ref(*clause) as usize);
        let w: u64 = clauses.iter().map(|clause| clause.w_cost(tape)).sum();
        let again = sweep_limit(levels, tape, budget.doubled()) as u64;
        crate::code_rule_dsl::logic::profile::decision("over its budget, replanned by cost (0334)", || {
            format!("ast label sweep, {} clauses: W {w} steps against the sweep again {again}: {}", clauses.len(), if w < again { "W" } else { "again" })
        });
        if w < again && w_allowed {
            return Ok(false);
        }
        // The sweep again from the other labels: the group's own cleared.
        for &(_, id) in levels.iter().flatten() {
            Arc::make_mut(sets)[usize::from(id - LABEL_BASE)].iter_mut().for_each(|word| *word = 0);
        }
        budget = budget.doubled();
    }
}

/// The group's passes on the interpreter (those with no micro walk) and the size of their starts on `tape`, for its
/// prediction and observations.
fn sweep_programs<'a>(levels: &[Vec<(&'a LoweredMatcher, u16)>], tape: &TapeDoc) -> Vec<(&'a Program, u64)> {
    levels
        .iter()
        .flatten()
        .filter_map(|&(clause, id)| match clause.labels.def(id) {
            Label::Roots(program, starts) | Label::Targets(program, starts) if Micro::of(program).is_none() => {
                let entry = Entry::of(program);
                let size = (0..tape.transducer_len() as u32)
                    .filter(|&node| match starts {
                        Domain::Kinds(kinds) => kinds.binary_search(&tape.transducer_kind(node)).is_ok(),
                        _ => true,
                    })
                    .filter(|&node| entry.admits(tape, node))
                    .map(|node| {
                        let span = tape.transducer_span(node);
                        costs::size(program, tape.transducer_descendants(node).end.saturating_sub(node).max(1), span.end.saturating_sub(span.start) as u32)
                    })
                    .sum();
                Some((program, size))
            }
            _ => None,
        })
        .collect()
}

/// A sweep's budget on `tape`: its interpreted passes' predicted steps over their starts (`sweep_programs`) under
/// `budget`, `usize::MAX` for none.
fn sweep_limit(levels: &[Vec<(&LoweredMatcher, u16)>], tape: &TapeDoc, budget: WorkBudget) -> usize {
    if !budget.replans() {
        return usize::MAX;
    }
    let programs = sweep_programs(levels, tape);
    let predicted: u64 = programs.iter().map(|&(program, size)| costs::predict(program, costs::rate(program.plain_digest()), size)).sum();
    budget.evaluation_limit(predicted, transducer_packed::MAX_WORK_PER_TASK as u64 * u64::from(tape.transducer_len().max(1) as u32))
}

/// One sweep of the group to its fixpoint, its interpreted passes held together to `budget` (`OverBudget` past it,
/// `WorkLimit` at the ceiling or where the sweeps do not settle).
fn sweep_run(
    levels: &[Vec<(&LoweredMatcher, u16)>],
    tape: &TapeDoc,
    sets: &mut Arc<Vec<Vec<u64>>>,
    texts: &(dyn Fn(&Micro, &Program) -> Option<Vec<Vec<u64>>> + Sync),
    budget: WorkBudget,
) -> Result<(), Error> {
    let limit = sweep_limit(levels, tape, budget);
    // The interpreted passes' steps, by program, for their rates (observed however the sweep ends).
    let mut spent = 0_usize;
    let mut steps_by_program: HashMap<*const Program, usize> = HashMap::new();
    let observe = |steps_by_program: &HashMap<*const Program, usize>, over_budget: bool| {
        if steps_by_program.is_empty() {
            return;
        }
        let observations = sweep_programs(levels, tape)
            .into_iter()
            .filter_map(|(program, size)| {
                let steps = *steps_by_program.get(&std::ptr::from_ref(program))?;
                let predicted = costs::predict(program, costs::rate(program.plain_digest()), size);
                Some(costs::Observation { program: program.plain_digest().to_owned(), steps: steps as u64, size, predicted, over_budget })
            })
            .collect::<Vec<_>>();
        costs::observe(observations);
    };
    let result = sweep_steps(levels, tape, sets, texts, limit, &mut spent, &mut steps_by_program);
    observe(&steps_by_program, matches!(result, Err(Error::OverBudget)));
    result
}

#[allow(clippy::too_many_lines)]
fn sweep_steps(
    levels: &[Vec<(&LoweredMatcher, u16)>],
    tape: &TapeDoc,
    sets: &mut Arc<Vec<Vec<u64>>>,
    texts: &(dyn Fn(&Micro, &Program) -> Option<Vec<Vec<u64>>> + Sync),
    limit: usize,
    spent: &mut usize,
    steps_by_program: &mut HashMap<*const Program, usize>,
) -> Result<(), Error> {
    let complements = levels.iter().flatten().any(|&(clause, id)| matches!(clause.labels.def(id), Label::Complement(_)));
    let len = tape.transducer_len() as u32;
    // (node, pass) for every node a pass may start at, a pass an index into `passes` (in level
    // order, so (node, pass) order is (node, level) order); a label domain is tested at the node.
    // 8 bytes an entry: a complement's or a full domain's passes enter every node, and the files
    // sweep in parallel (O30: 32-byte entries were S7's footprint peak, +350 MB in one sweep).
    let stage = crate::code_rule_dsl::prof::Stage::start("sweep.entries");
    let passes = levels
        .iter()
        .enumerate()
        .flat_map(|(at, level)| level.iter().map(move |&(clause, id)| (at, clause, id)))
        .collect::<Vec<_>>();
    // Each pass's starts: none, every node, or a node of its kinds; and its entry test. Built node
    // by node, the entries are in (node, pass) order without a sort (sorting them was ~70% of
    // building them on S7).
    let starts = passes
        .iter()
        .map(|&(_, clause, id)| {
            let entry = match clause.labels.def(id) {
                Label::Roots(program, _) | Label::Targets(program, _) => Some(Entry::of(program)),
                _ => None,
            };
            let kinds = match clause.labels.def(id) {
                Label::Roots(_, Domain::Kinds(kinds)) | Label::Targets(_, Domain::Kinds(kinds)) => Some(Some(kinds)),
                Label::Empty => None,
                _ => Some(None),
            };
            (kinds, entry)
        })
        .collect::<Vec<_>>();
    let mut entries: Vec<(u32, u32)> = Vec::new();
    for node in 0..len {
        let kind = tape.transducer_kind(node);
        for (pass, (kinds, entry)) in starts.iter().enumerate() {
            let Some(kinds) = kinds else { continue };
            if kinds.is_some_and(|kinds| kinds.binary_search(&kind).is_err()) {
                continue;
            }
            if entry.as_ref().is_none_or(|entry| entry.admits(tape, node)) {
                entries.push((node, u32::try_from(pass).map_err(|_| Error::BackendCapacity)?));
            }
        }
    }
    crate::code_rule_dsl::prof::count("sweep.entry_count", entries.len());
    drop(stage);
    let stage = crate::code_rule_dsl::prof::Stage::start("sweep.walks");
    // Each program's micro walk, where it is one (O28): no interpreter task per (node, pass).
    let walks = levels
        .iter()
        .flatten()
        .map(|&(clause, id)| {
            // With its text bits on this tape (`Micro::texts`).
            let walk = match clause.labels.def(id) {
                Label::Roots(program, _) | Label::Targets(program, _) => {
                    Micro::of(program).and_then(|walk| texts(&walk, program).map(|texts| (walk, texts)))
                }
                _ => None,
            };
            ((std::ptr::from_ref(clause) as usize, id), walk)
        })
        .collect::<BTreeMap<_, _>>();
    // Each pass's walk, by its index: no map lookup per (node, pass).
    let pass_walks = passes
        .iter()
        .map(|&(_, clause, id)| walks[&(std::ptr::from_ref(clause) as usize, id)].as_ref())
        .collect::<Vec<_>>();
    // The labels each pass reads (by index in `sets`): its walk's label class tests, its domain,
    // or its complement's source; `None` where it is not a walk (any). A pass reads the same
    // labels at a node as when it last ran there unless one of them changed since, and then it
    // writes the same verdicts: it is skipped (a sweep's second pass mostly confirms the first).
    let reads = passes
        .iter()
        .enumerate()
        .map(|(pass, &(_, clause, id))| match clause.labels.def(id) {
            Label::Complement(of) => Some(vec![usize::from(*of - LABEL_BASE)]),
            Label::Roots(_, starts) | Label::Targets(_, starts) => pass_walks[pass].map(|(walk, _)| {
                let mut reads = walk
                    .words
                    .iter()
                    .filter_map(|&word| {
                        let (op, _, arg) = split(word);
                        (op == SUPERTYPE && arg >= LABEL_BASE).then(|| usize::from(arg - LABEL_BASE))
                    })
                    .collect::<Vec<_>>();
                if let Domain::Label(label) = starts {
                    reads.push(usize::from(*label - LABEL_BASE));
                }
                reads.sort_unstable();
                reads.dedup();
                reads
            }),
            _ => Some(Vec::new()),
        })
        .collect::<Vec<_>>();
    drop(stage);
    let _run = crate::code_rule_dsl::prof::Stage::start("sweep.run");
    let mut frames = Frames::default();
    let mut marks = Vec::new();
    let mut reverse = complements;
    // The labels a sweep changed, and those the one before it changed (every label before the
    // first).
    let mut changed_before = vec![true; sets.len()];
    let mut changed_now = vec![false; sets.len()];
    for _sweep in 0..=len + 1 {
        crate::code_rule_dsl::prof::count("sweep.sweeps", 1);
        let mut changed = false;
        let mut run = |&(node, pass): &(u32, u32)| -> Result<(), Error> {
            if let Some(reads) = &reads[pass as usize]
                && !reads.iter().any(|&label| changed_before[label] || changed_now[label])
            {
                return Ok(());
            }
            let (_, clause, id) = passes[pass as usize];
            let target = usize::from(id - LABEL_BASE);
            let marked = match clause.labels.def(id) {
                Label::Complement(of) => {
                    let set = !has(sets, *of, node);
                    let bits = &mut Arc::make_mut(sets)[target];
                    let was = bits[node as usize / 64] >> (node % 64) & 1 != 0;
                    if was != set {
                        bits[node as usize / 64] ^= 1 << (node % 64);
                        changed = true;
                changed_now[target] = true;
                    }
                    return Ok(());
                }
                Label::Roots(program, starts) | Label::Targets(program, starts) => {
                    if let Domain::Label(label) = starts
                        && !has(sets, *label, node)
                    {
                        // Outside its domain now: with complements the verdict there is no.
                        if complements && has(sets, id, node) {
                            Arc::make_mut(sets)[target][node as usize / 64] &= !(1 << (node % 64));
                            changed = true;
                changed_now[target] = true;
                        }
                        return Ok(());
                    }
                    let targets = matches!(clause.labels.def(id), Label::Targets(..));
                    marks.clear();
                    let walked = pass_walks[pass as usize]
                        .is_some_and(|(walk, texts)| {
                            let emit = if targets { Emit::Captures } else { Emit::Start };
                            walk.run(tape, sets, texts, node, emit, &mut frames, &mut |node| marks.push(node)).is_ok()
                        });
                    if !walked {
                        crate::code_rule_dsl::prof::count("sweep.fallback", 1);
                        marks.clear();
                        let view = tape.with_labels(Arc::clone(sets));
                        let (outputs, used) = match program.execute_roots_budgeted(&view, [node], limit.saturating_sub(*spent)) {
                            Ok(found) => found,
                            Err(Error::OverBudget) => {
                                *steps_by_program.entry(std::ptr::from_ref(program)).or_default() += limit.saturating_sub(*spent);
                                return Err(Error::OverBudget);
                            }
                            Err(e) => return Err(e),
                        };
                        *spent = spent.saturating_add(used);
                        *steps_by_program.entry(std::ptr::from_ref(program)).or_default() += used;
                        drop(view);
                        if targets {
                            marks.extend(outputs.iter().flat_map(|output| output.captures.iter().map(|capture| capture.node)));
                        } else {
                            marks.extend(outputs.iter().map(|output| output.start));
                        }
                    }
                    // With complements a program marks its roots only, so its verdict at `node` is
                    // exactly whether it emitted there: set or cleared, as a complement's is. Marks
                    // that only add would keep a verdict read from a complement before its source
                    // was settled (O29: SIM300's `probably` two levels deep).
                    if complements {
                        let set = marks.contains(&node);
                        let bits = &mut Arc::make_mut(sets)[target];
                        if (bits[node as usize / 64] >> (node % 64) & 1 != 0) != set {
                            bits[node as usize / 64] ^= 1 << (node % 64);
                            changed = true;
                changed_now[target] = true;
                        }
                        return Ok(());
                    }
                    std::mem::take(&mut marks)
                }
                _ => return Ok(()),
            };
            for &node in &marked {
                if !has(sets, id, node) {
                    Arc::make_mut(sets)[target][node as usize / 64] |= 1 << (node % 64);
                    changed = true;
                changed_now[target] = true;
                }
            }
            marks = marked;
            Ok(())
        };
        if reverse {
            // Nodes from the last; at each node its levels in order.
            let mut end = entries.len();
            while end > 0 {
                let node = entries[end - 1].0;
                let start = entries[..end].partition_point(|entry| entry.0 < node);
                entries[start..end].iter().try_for_each(&mut run)?;
                end = start;
            }
        } else {
            entries.iter().try_for_each(&mut run)?;
        }
        if !changed {
            return Ok(());
        }
        std::mem::swap(&mut changed_before, &mut changed_now);
        changed_now.fill(false);
        reverse = complements || !reverse;
    }
    Err(Error::WorkLimit)
}

/// What a label reads of a program's result: where it started, and the nodes it captured.
struct Found {
    start: u32,
    captures: Vec<u32>,
}

impl Found {
    pub fn of(output: &Output) -> Self {
        Self { start: output.start, captures: output.captures.iter().map(|capture| capture.node).collect() }
    }
}

/// A program label's set on each tape from its program's results there.
fn from_outputs(label: &Label, views: &[TapeDoc], outputs: Vec<Vec<Found>>) -> Vec<Vec<u64>> {
    let set = |bits: &mut Vec<u64>, node: u32| bits[node as usize / 64] |= 1 << (node % 64);
    views
        .iter()
        .zip(outputs)
        .map(|(view, outputs)| {
            let mut bits = vec![0_u64; view.transducer_len().div_ceil(64)];
            match label {
                Label::Count { by, min, max, .. } => {
                    let mut counts = vec![0_usize; view.transducer_len()];
                    for output in outputs {
                        match by {
                            Tally::Start => counts[output.start as usize] += 1,
                            Tally::Capture => output.captures.iter().for_each(|&node| counts[node as usize] += 1),
                        }
                    }
                    for (node, &count) in counts.iter().enumerate() {
                        if count >= *min && max.is_none_or(|max| count <= max) {
                            set(&mut bits, node as u32);
                        }
                    }
                }
                Label::Targets(..) => {
                    for output in outputs {
                        output.captures.iter().for_each(|&node| set(&mut bits, node));
                    }
                }
                _ => outputs.iter().for_each(|output| set(&mut bits, output.start)),
            }
            bits
        })
        .collect()
}

/// A roots, targets or count label's bits on `tape` by `walk`, from each of `starts`: what
/// `from_outputs` makes of the interpreter's outputs (a roots label its emitting starts, a targets
/// label the nodes its emitting paths captured, a count its tallies in bounds). None where the
/// walk's text slots cannot be read or a walk overflows.
/// A walk's anchor on `tape` (decision 0365): the members of `Set::anchor` of its program, a superset of the
/// starts from which it emits, read from the tape's columns (`index`) and its labels so far. A start outside it is
/// not walked: its walk emits nothing.
///
/// A text test is a value anchor where it is a read (prior work, ast-index-spike: the value index's postings): the
/// walk's own rows, where it has them anyway (`texts`), else the term index's row where the tape has it
/// (`prefilter::terms`). An anchor never scans text of its own: elsewhere the test is dropped, and the walk tests
/// it at the starts it keeps. A program that is not a micro walk has no text slots, so its text tests are dropped.
pub(in crate::code_rule_dsl) fn anchor_bits(program: &Program, walk: Option<&Micro>, texts: Option<&[Vec<u64>]>, tape: &TapeDoc, sets: &[Vec<u64>], index: &super::compile_sets::Index) -> Vec<u64> {
    use super::compile_sets::{Reader, Set};
    let _stage = crate::code_rule_dsl::prof::Stage::start("anchor.eval");
    let Some(walk) = walk else {
        return Reader { tape, index, labels: sets, texts: &[] }.eval(&Set::anchor(&program.words, &|_| false));
    };
    let indexed: Vec<Vec<u64>>;
    let (rows, has): (&[Vec<u64>], Vec<bool>) = match texts {
        Some(texts) => (texts, vec![true; texts.len()]),
        None => {
            let read: Vec<Option<Vec<u64>>> = (0..walk.texts.len()).map(|slot| tape.transducer_text_row(&walk.slot_key(program, slot)).map(|row| row.to_vec())).collect();
            let has = read.iter().map(Option::is_some).collect();
            indexed = read.into_iter().map(Option::unwrap_or_default).collect();
            (&indexed, has)
        }
    };
    let set = Set::anchor(&walk.words, &|slot| has.get(usize::from(slot)).copied().unwrap_or(false));
    Reader { tape, index, labels: sets, texts: rows }.eval(&set)
}

/// Whether walks start within their anchors (decision 0365): off with `RERAIL_ANCHORS=off`, read at each batch (the
/// anchors' check against the walks without them, `anchored_walks_find_the_unanchored_roots`).
pub fn anchors_on() -> bool {
    std::env::var("RERAIL_ANCHORS").map_or(true, |mode| mode != "off")
}

/// Starts `starts` within `anchor`, counted for the profile: the starts kept and those the anchor dropped.
fn anchored<'a>(starts: impl Iterator<Item = u32> + 'a, anchor: Option<&'a [u64]>) -> impl Iterator<Item = u32> + 'a {
    starts.filter(move |&node| anchor.is_none_or(|bits| bits.get(node as usize / 64).is_some_and(|word| word >> (node % 64) & 1 != 0)))
}

/// The share of `starts` within `anchor`, for the profile (`anchor.kept`, `anchor.dropped`).
fn count_anchored(starts: impl Iterator<Item = u32>, anchor: &[u64]) {
    let (mut kept, mut dropped) = (0, 0);
    for node in starts {
        if anchor.get(node as usize / 64).is_some_and(|word| word >> (node % 64) & 1 != 0) {
            kept += 1;
        } else {
            dropped += 1;
        }
    }
    crate::code_rule_dsl::prof::count("anchor.kept", kept);
    crate::code_rule_dsl::prof::count("anchor.dropped", dropped);
}

fn walked(label: &Label, walk: &Micro, program: &Program, tape: &TapeDoc, sets: &[Vec<u64>], starts: impl Iterator<Item = u32>) -> Option<Vec<u64>> {
    let texts = walk.texts(program, tape)?;
    walked_with(label, walk, &texts, tape, sets, starts)
}

/// `walked` with the walk's text rows given.
fn walked_with(label: &Label, walk: &Micro, texts: &[Vec<u64>], tape: &TapeDoc, sets: &[Vec<u64>], starts: impl Iterator<Item = u32>) -> Option<Vec<u64>> {
    let len = tape.transducer_len();
    let mut bits = vec![0_u64; len.div_ceil(64)];
    let mut frames = Frames::default();
    let set = |bits: &mut Vec<u64>, node: u32| bits[node as usize / 64] |= 1 << (node % 64);
    match label {
        Label::Count { by, min, max, .. } => {
            let emit = match by {
                Tally::Start => Emit::EveryStart,
                Tally::Capture => Emit::Captures,
            };
            let mut counts = vec![0_usize; len];
            for start in starts {
                walk.run(tape, sets, texts, start, emit, &mut frames, &mut |node| counts[node as usize] += 1).ok()?;
            }
            for (node, &count) in counts.iter().enumerate() {
                if count >= *min && max.is_none_or(|max| count <= max) {
                    set(&mut bits, node as u32);
                }
            }
        }
        _ => {
            let emit = if matches!(label, Label::Targets(..)) { Emit::Captures } else { Emit::Start };
            for start in starts {
                walk.run(tape, sets, texts, start, emit, &mut frames, &mut |node| set(&mut bits, node)).ok()?;
            }
        }
    }
    Some(bits)
}

/// The nodes of `tape` in `domain`, in preorder.
pub fn domain<'a>(domain: &'a Domain, tape: &'a TapeDoc, sets: &'a [Vec<u64>]) -> impl Iterator<Item = u32> + 'a {
    (0..tape.transducer_len() as u32).filter(move |&node| match domain {
        Domain::All => true,
        Domain::Kinds(kinds) => kinds.binary_search(&tape.transducer_kind(node)).is_ok(),
        Domain::Label(label) => has(sets, *label, node),
    })
}

/// One program's words, with the clause's labels it adds to.
struct Builder<'a> {
    words: Vec<u32>,
    strings: Vec<Vec<u8>>,
    predicates: Vec<TextPredicate>,
    cursors: u16,
    labels: &'a mut Labels,
}

impl<'a> Builder<'a> {
    fn new(labels: &'a mut Labels) -> Self {
        Self {
            words: Vec::new(),
            strings: vec![b"match".to_vec(), b"matcher".to_vec()],
            predicates: Vec::new(),
            cursors: 0,
            labels,
        }
    }

    /// A program emitting each root `tree` matches.
    fn roots(mut self, tree: &MatchTree) -> Result<Program, Error> {
        self.test(tree)?;
        self.finish(&[word(CAPTURE, 0, 0), word(EMIT, 0, 1)])
    }

    fn finish(mut self, tail: &[u32]) -> Result<Program, Error> {
        self.words.extend_from_slice(tail);
        Program::new(self.words, self.strings, self.predicates, vec![0, 1], Vec::new())
    }

    fn push(&mut self, opcode: u8, mode: u8, operand: u16) -> usize {
        self.words.push(word(opcode, mode, operand));
        self.words.len() - 1
    }

    fn here(&self) -> Result<u16, Error> {
        u16::try_from(self.words.len()).map_err(|_| Error::InvalidTarget(self.words.len()))
    }

    fn patch(&mut self, at: usize, target: u16) {
        let (opcode, mode, _) = split(self.words[at]);
        self.words[at] = word(opcode, mode, target);
    }

    fn label(&mut self, label: Label) -> u16 {
        self.labels.add(label)
    }

    /// The label of the nodes `tree` matches at.
    fn label_of(&mut self, tree: &MatchTree) -> Result<u16, Error> {
        match tree {
            MatchTree::Pattern(pattern) => {
                let program = Program::compile_ast_grep_pattern(pattern)?;
                return Ok(self.label(Label::Pattern(program, Domain::of(tree))));
            }
            MatchTree::Constrained(pattern, constraints) => {
                let constraints = constraints
                    .iter()
                    .map(|(name, rule)| Ok((name.clone(), self.label_of(rule)?)))
                    .collect::<Result<Vec<_>, Error>>()?;
                let program = Program::compile_ast_grep_pattern(pattern)?;
                return Ok(self.label(Label::Constrained(program, Domain::of(tree), constraints)));
            }
            MatchTree::Recursive(id, inner) => return self.recursive(*id, inner),
            MatchTree::Recur(id) => {
                return self.labels.recursive.get(id).copied().ok_or(Error::InvalidOperand(0));
            }
            MatchTree::Fact(fact) => {
                let slots = fact.slots.iter().map(|slot| self.label_of(slot)).collect::<Result<Vec<_>, Error>>()?;
                return Ok(self.label(Label::Fact(Arc::clone(fact), slots)));
            }
            _ => {}
        }
        // A roots label is the starts that emit: set mode.
        let program = Builder::new(self.labels).roots(tree)?.in_set_mode();
        Ok(self.label(Label::Roots(program, Domain::of(tree))))
    }

    /// A recursive utility's label: its program and the labels compiled for it form one group,
    /// evaluated together until none changes. Through a complement, `matcher::lower` admits only
    /// recursion whose every walk moves forward in preorder, which has one answer the rounds reach.
    fn recursive(&mut self, id: usize, tree: &MatchTree) -> Result<u16, Error> {
        if let Some(&label) = self.labels.recursive.get(&id) {
            return Ok(label);
        }
        let label = self.labels.reserve(Label::Empty);
        self.labels.recursive.insert(id, label);
        let start = self.labels.steps.len();
        let program = Builder::new(self.labels).roots(tree)?.in_set_mode();
        self.labels.defs[usize::from(label - LABEL_BASE)] = Label::Roots(program, Domain::of(tree));
        // Only the labels that read the utility's, directly or through each other, are evaluated
        // round after round; the others are passes of their own ahead of the group. A recursive
        // utility inside this one is a step that is a group already: it stays one, or joins this
        // group whole, so its labels still settle round after round (split into steps of one,
        // each ran once and the inner recursion stopped at its first level).
        let steps = self.labels.steps.drain(start..).collect::<Vec<_>>();
        let mut group = Vec::new();
        for step in steps {
            let reads = step.iter().any(|&member| self.labels.reads(member).iter().any(|read| *read == label || group.contains(read)));
            if reads {
                group.extend(step);
            } else {
                self.labels.steps.push(step);
            }
        }
        group.push(label);
        self.labels.steps.push(group);
        Ok(label)
    }

    fn complement_of(&mut self, tree: &MatchTree) -> Result<u16, Error> {
        let label = self.label_of(tree)?;
        Ok(self.label(Label::Complement(label)))
    }

    /// Tests `tree` at the cursor's node: the paths that pass continue there, at that node.
    fn test(&mut self, tree: &MatchTree) -> Result<(), Error> {
        match tree {
            MatchTree::Kind(kind) => {
                self.push(KIND, 0, *kind);
            }
            MatchTree::Supertype(class, _) => {
                self.push(SUPERTYPE, 0, *class);
            }
            MatchTree::Regex(regex) => {
                let source = self.strings.len();
                self.strings.push(regex.as_str().as_bytes().to_vec());
                self.predicates.push(TextPredicate::Regex {
                    source: u16::try_from(source).map_err(|_| Error::InvalidOperand(source))?,
                    compiled: regex.clone(),
                });
                let predicate = (self.predicates.len() - 1) as u16;
                self.push(TEXT, 0, predicate);
            }
            MatchTree::Pattern(_)
            | MatchTree::Constrained(..)
            | MatchTree::Recursive(..)
            | MatchTree::Recur(_)
            | MatchTree::Fact(_)
            | MatchTree::Not(_)
            | MatchTree::Never => {
                let label = match tree {
                    MatchTree::Not(inner) => self.complement_of(inner)?,
                    MatchTree::Never => self.label(Label::Empty),
                    _ => self.label_of(tree)?,
                };
                self.push(SUPERTYPE, 0, label);
            }
            MatchTree::All(trees) => {
                for tree in trees {
                    self.test(tree)?;
                }
            }
            MatchTree::Any(trees) => {
                let Some((last, alternatives)) = trees.split_last() else {
                    let label = self.label(Label::Empty);
                    self.push(SUPERTYPE, 0, label);
                    return Ok(());
                };
                let mut ends = Vec::new();
                for tree in alternatives {
                    let fork = self.push(FORK, 0, 0);
                    self.test(tree)?;
                    ends.push(self.push(GOTO, 0, 0));
                    let next = self.here()?;
                    self.patch(fork, next);
                }
                self.test(last)?;
                let end = self.here()?;
                for at in ends {
                    self.patch(at, end);
                }
            }
            MatchTree::Walk {
                axis,
                field,
                rule,
                stop,
            } => self.walk(*axis, *field, rule, stop)?,
            MatchTree::Count {
                axis,
                rule,
                stop,
                min,
                max,
            } => {
                let label = self.count(*axis, rule, stop, *min, *max)?;
                self.push(SUPERTYPE, 0, label);
            }
        }
        Ok(())
    }

    /// `count`'s label: a program with one path per node the walk reaches that `rule`'s label
    /// has, tallied per node. A forward walk runs from every node and is tallied by its root. A
    /// backward one (`inside` past the parent, `follows`) runs forward from each node `rule`
    /// matches and is tallied by the node it reaches, as `reaches` does. The rule is a label, so
    /// no `any` forks a node into two paths.
    fn count(&mut self, axis: Axis, rule: &MatchTree, stop: &Stop, min: usize, max: Option<usize>) -> Result<u16, Error> {
        let rule = self.label_of(rule)?;
        let bound = match stop {
            Stop::Rule(stop) => Some(self.complement_of(stop)?),
            Stop::Neighbor | Stop::End => None,
        };
        let backward = axis == Axis::Follows || axis == Axis::Inside && !matches!(stop, Stop::Neighbor);
        let mut builder = Builder::new(self.labels);
        if backward {
            builder.push(SUPERTYPE, 0, rule);
        }
        let (once, all) = match axis {
            Axis::Has | Axis::Inside => (CHILD, DESCEND),
            Axis::Precedes | Axis::Follows => (NEXT, FOLLOWING),
        };
        match (axis, stop, bound) {
            (Axis::Inside, Stop::Neighbor, _) => {
                builder.push(PARENT, 0, 0);
            }
            (_, Stop::Neighbor, _) => {
                builder.push(once, 0, 0);
            }
            (_, _, None) => {
                builder.push(all, 0, 0);
            }
            (_, _, Some(bound)) => builder.bounded(once, bound)?,
        }
        if !backward {
            builder.push(SUPERTYPE, 0, rule);
        }
        let program = builder.finish(&[word(CAPTURE, 0, 0), word(EMIT, 0, 1)])?;
        let (domain, by) = if backward {
            (Domain::Label(rule), Tally::Capture)
        } else {
            (Domain::All, Tally::Start)
        };
        Ok(self.label(Label::Count {
            program,
            domain,
            by,
            min,
            max,
        }))
    }

    /// The test of `rule` a walk starts with, and the nodes it can start from.
    fn start(&mut self, rule: &MatchTree) -> Result<Domain, Error> {
        if rule.walks() {
            let label = self.label_of(rule)?;
            self.push(SUPERTYPE, 0, label);
            Ok(Domain::Label(label))
        } else {
            self.test(rule)?;
            Ok(Domain::of(rule))
        }
    }

    /// The test of a relation's rule at the node a walk reached: inline where it stays on the
    /// node, else its label.
    fn reached(&mut self, rule: &MatchTree) -> Result<(), Error> {
        if rule.walks() {
            let label = self.label_of(rule)?;
            self.push(SUPERTYPE, 0, label);
            Ok(())
        } else {
            self.test(rule)
        }
    }

    fn walk(&mut self, axis: Axis, field: Option<u16>, rule: &MatchTree, stop: &Stop) -> Result<(), Error> {
        if axis == Axis::Follows || axis == Axis::Inside && !matches!(stop, Stop::Neighbor) {
            // No backward walk that verifies: a loop must move forward in preorder
            // (`cycle_progress`). The nodes a forward walk from each match reaches are the nodes
            // the backward walk would find it from.
            let label = self.reaches(axis, field, rule, stop)?;
            self.push(SUPERTYPE, 0, label);
            return Ok(());
        }
        let bound = match stop {
            Stop::Rule(stop) => Some(self.complement_of(stop)?),
            Stop::Neighbor | Stop::End => None,
        };
        let cursor = self.cursors;
        self.cursors += 1;
        self.push(SAVE_CURSOR, 0, cursor);
        match (axis, field) {
            (Axis::Inside, field) => {
                // `neighbor`: the parent, entered through the field if one is given.
                if let Some(field) = field {
                    self.push(FIELD, 0, field);
                }
                self.push(PARENT, 0, 0);
            }
            (Axis::Has | Axis::Precedes, None) => {
                let (once, all) = if axis == Axis::Has { (CHILD, DESCEND) } else { (NEXT, FOLLOWING) };
                match (stop, bound) {
                    (Stop::Neighbor, _) => {
                        self.push(once, 0, 0);
                    }
                    (_, None) => {
                        self.push(all, 0, 0);
                    }
                    (_, Some(bound)) => {
                        // Inclusive: each node stepped to is tested; the walk goes past it only
                        // where the stop rule does not match it.
                        self.bounded(once, bound)?;
                    }
                }
            }
            (Axis::Has, Some(field)) => {
                // The field child, then nothing more (`neighbor`), its descendants (`end`), or its
                // children where the stop rule passes it (one level, as upstream).
                self.push(CHILD, 0, 0);
                self.push(FIELD, 0, field);
                if !matches!(stop, Stop::Neighbor) {
                    let fork = self.push(FORK, 0, 0);
                    match bound {
                        Some(bound) => {
                            self.push(SUPERTYPE, 0, bound);
                            self.push(CHILD, 0, 0);
                        }
                        None => {
                            self.push(DESCEND, 0, 0);
                        }
                    }
                    let reached = self.here()?;
                    self.patch(fork, reached);
                }
            }
            (Axis::Precedes | Axis::Follows, Some(_)) => return Err(Error::UnsupportedPattern),
            (Axis::Follows, None) => unreachable!("lowered as a label"),
        }
        self.reached(rule)?;
        self.push(RESTORE_CURSOR, 0, cursor);
        // Restored, the slot is free again: slots are a stack by walk nesting, so a program holds
        // as many as its deepest nesting, not one per walk (sibling walks share; forked paths
        // carry their own copies). A micro walk holds `micro::CURSORS`.
        self.cursors -= 1;
        Ok(())
    }

    /// `top: step; FORK on; GOTO reached; on: SUPERTYPE bound; GOTO top; reached:` — every node
    /// the step reaches, going on past a node only where it has `bound`. The path that stops at a
    /// node goes first and the one that goes on is the fork's frame: a walk then holds one frame
    /// for the loop, not one per node it passes (a module's 300 statements by `NEXT` outgrew a
    /// micro walk's 256 frames). A label is a set of nodes, so the order of its paths is free.
    fn bounded(&mut self, step: u8, bound: u16) -> Result<(), Error> {
        let top = self.here()?;
        self.push(step, 0, 0);
        let fork = self.push(FORK, 0, 0);
        let exit = self.push(GOTO, 0, 0);
        let on = self.here()?;
        self.patch(fork, on);
        self.push(SUPERTYPE, 0, bound);
        self.push(GOTO, 0, top);
        let reached = self.here()?;
        self.patch(exit, reached);
        Ok(())
    }

    /// The label of the nodes from which a backward walk (`inside`, `follows`) finds a node `rule`
    /// matches, as `stop` bounds it: the nodes a forward walk from each match reaches. Walking
    /// back, a node the stop rule matches ends the walk after it is tested; forward, the walk goes
    /// past a node only where the stop rule does not match it.
    fn reaches(&mut self, axis: Axis, field: Option<u16>, rule: &MatchTree, stop: &Stop) -> Result<u16, Error> {
        let bound = match stop {
            Stop::Rule(stop) => Some(self.complement_of(stop)?),
            Stop::Neighbor | Stop::End => None,
        };
        let mut builder = Builder::new(self.labels);
        let starts = builder.start(rule)?;
        match (axis, field) {
            (Axis::Follows, None) => match (stop, bound) {
                (Stop::Neighbor, _) => {
                    builder.push(NEXT, 0, 0);
                }
                (_, None) => {
                    builder.push(FOLLOWING, 0, 0);
                }
                (_, Some(bound)) => builder.bounded(NEXT, bound)?,
            },
            (Axis::Inside, None) => match bound {
                None => {
                    builder.push(DESCEND, 0, 0);
                }
                Some(bound) => builder.bounded(CHILD, bound)?,
            },
            (Axis::Inside, Some(field)) => {
                // The match is tested where the walk enters it through the field: the field
                // child, then below it as the stop rule allows.
                builder.push(CHILD, 0, 0);
                builder.push(FIELD, 0, field);
                match bound {
                    None => {
                        let fork = builder.push(FORK, 0, 0);
                        builder.push(DESCEND, 0, 0);
                        let reached = builder.here()?;
                        builder.patch(fork, reached);
                    }
                    Some(bound) => {
                        let top = builder.here()?;
                        let fork = builder.push(FORK, 0, 0);
                        builder.push(SUPERTYPE, 0, bound);
                        builder.push(CHILD, 0, 0);
                        builder.push(GOTO, 0, top);
                        let reached = builder.here()?;
                        builder.patch(fork, reached);
                    }
                }
            }
            _ => return Err(Error::UnsupportedPattern),
        }
        let program = builder.finish(&[word(CAPTURE, 0, 0), word(EMIT, 0, 1)])?;
        Ok(self.label(Label::Targets(program, starts)))
    }
}

#[cfg(any(test, feature = "test-hooks"))]
impl LoweredMatcher {
    /// The root program and each label's program, by label kind (the packs' census, decision 0028).
    pub fn census(&self) -> Vec<(&'static str, &Program)> {
        let mut programs = vec![("root", &self.program)];
        for label in &self.labels.defs {
            match label {
                Label::Pattern(program, _) => programs.push(("pattern", program)),
                Label::Constrained(program, _, _) => programs.push(("constrained", program)),
                Label::Roots(program, _) => programs.push(("roots", program)),
                Label::Targets(program, _) => programs.push(("targets", program)),
                Label::Count { program, .. } => programs.push(("count", program)),
                Label::Complement(_) | Label::Fact(..) | Label::Empty => {}
            }
        }
        programs
    }
}
