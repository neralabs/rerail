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
use std::collections::{BTreeMap, BTreeSet};

use crate::code_rule_dsl::tape::DERIVED_CLASS_BASE as LABEL_BASE;

/// A lowered clause: label passes, in order, then the root program.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct LoweredMatcher {
    labels: Labels,
    program: Program,
    domain: Domain,
}

#[derive(Clone, Default)]
struct Labels {
    /// By label id less `LABEL_BASE` and `offset`.
    defs: Vec<Label>,
    /// Where this clause's labels start among a wave's (`LoweredWave`): 0 alone.
    offset: u16,
    /// Evaluation order: each step a group of labels evaluated together, to their least fixpoint
    /// where the group is a recursive utility's (one label otherwise).
    steps: Vec<Vec<u16>>,
    /// A recursive utility's label, by recursion id (`MatchTree::Recursive`).
    recursive: BTreeMap<usize, u16>,
}

/// The nodes a pass starts from: those whose kind the rule needs, or a label's (a walk's start).
#[derive(Clone)]
enum Domain {
    All,
    Kinds(Vec<u16>),
    Label(u16),
}

impl Domain {
    fn of(tree: &MatchTree) -> Self {
        tree.kinds()
            .map_or(Self::All, |kinds| Self::Kinds(kinds.into_iter().collect()))
    }
}

#[derive(Clone, Copy)]
enum Tally {
    Start,
    Capture,
}

/// Where a clause's passes run. On Metal every pass is a kernel in one command buffer
/// (`roots_metal`); pattern leaves are matched on the host first, as `demand`'s pattern producers.
#[derive(Clone, Copy, Debug)]
pub(in crate::code_rule_dsl) enum Backend {
    Cpu,
    #[cfg(target_os = "macos")]
    MetalInterpreter,
}

#[derive(Clone)]
enum Label {
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
    /// No node (`Never`).
    Empty,
}

impl Labels {
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

    fn def(&self, id: u16) -> &Label {
        &self.defs[usize::from(id - LABEL_BASE - self.offset)]
    }

    /// The kinds a domain's nodes can have, known before any label is (`None`: any kind). A
    /// label of roots, pattern matches or counts by root lies within its own domain; targets,
    /// counts by capture and complements can be any node. A label met again (recursion) is any.
    fn bound(&self, domain: &Domain, seen: &mut Vec<u16>) -> Option<BTreeSet<u16>> {
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
                    Label::Targets(..) | Label::Count { .. } | Label::Complement(_) => None,
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
                Label::Empty => Label::Empty,
            })
            .collect();
        Self {
            defs,
            offset: self.offset + by,
            steps: self.steps.iter().map(|step| step.iter().copied().map(up).collect()).collect(),
            recursive: self.recursive.iter().map(|(at, label)| (*at, up(*label))).collect(),
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
        }
    }
}

impl LoweredMatcher {
    pub(in crate::code_rule_dsl) fn lower(tree: &MatchTree) -> Result<Self, Error> {
        let mut labels = Labels::default();
        let program = Builder::new(&mut labels).roots(tree)?;
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
    pub(in crate::code_rule_dsl) fn passes(&self) -> usize {
        self.labels.defs.len()
    }

    /// The nodes of `tape` the clause's matcher matches at, in preorder.
    pub(in crate::code_rule_dsl) fn roots(&self, tape: &TapeDoc) -> Result<BTreeSet<u32>, Error> {
        let mut roots = self.roots_batch(Backend::Cpu, std::slice::from_ref(tape))?;
        Ok(roots.pop().expect("one tape"))
    }

    /// The nodes of each tape the clause's matcher matches at: each pass runs once over the batch.
    pub(in crate::code_rule_dsl) fn roots_batch(
        &self,
        backend: Backend,
        tapes: &[TapeDoc],
    ) -> Result<Vec<BTreeSet<u32>>, Error> {
        #[cfg(target_os = "macos")]
        if let Backend::MetalInterpreter = backend {
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
                    let found = self.evaluate(self.labels.def(id), &views, sets)?;
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
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn sweeps_needed(&self, tapes: &[TapeDoc]) -> Result<Vec<usize>, Error> {
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

    /// A label's set on each tape of the batch.
    fn evaluate(
        &self,
        label: &Label,
        views: &[TapeDoc],
        sets: &[Arc<Vec<Vec<u64>>>],
    ) -> Result<Vec<Vec<u64>>, Error> {
        let empty = |view: &TapeDoc| vec![0_u64; view.transducer_len().div_ceil(64)];
        let set = |bits: &mut Vec<u64>, node: u32| bits[node as usize / 64] |= 1 << (node % 64);
        match label {
            Label::Pattern(pattern, starts) | Label::Constrained(pattern, starts, _) => {
                let constraints = match label {
                    Label::Constrained(_, _, constraints) => &constraints[..],
                    _ => &[],
                };
                views
                    .iter()
                    .zip(sets)
                    .map(|(view, sets)| {
                        let mut bits = empty(view);
                        for node in domain(starts, view, sets) {
                            let Some(captures) = pattern.execute_ast_pattern(view, node)? else {
                                continue;
                            };
                            // As W's `Constrained`: an unbound or multi-node capture is not checked.
                            let holds = constraints.iter().all(|(name, label)| {
                                captures
                                    .iter()
                                    .find(|capture| capture.label == *name)
                                    .filter(|capture| !capture.multi)
                                    .is_none_or(|capture| has(sets, *label, capture.node))
                            });
                            if holds {
                                set(&mut bits, node);
                            }
                        }
                        Ok(bits)
                    })
                    .collect()
            }
            Label::Roots(program, starts) | Label::Targets(program, starts) | Label::Count { program, domain: starts, .. } => {
                let outputs = run(program, views, starts, sets)?;
                Ok(from_outputs(label, views, outputs.into_iter().map(|outputs| outputs.iter().map(Found::of).collect()).collect()))
            }
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
            Label::Empty => Ok(views.iter().map(empty).collect()),
        }
    }
}

impl Program {
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

/// A wave's lowered clauses evaluated together (06 S4): each clause's labels shifted past the
/// clauses' before it, so one set of label bitsets per tape holds them all and every pass and
/// root program reads its own. The roots then run as one batch of programs (`demand(batch)`'s
/// packed producers), or, on Metal, every clause's passes and roots in one command buffer.
pub(in crate::code_rule_dsl) struct LoweredWave {
    clauses: Vec<LoweredMatcher>,
    labels: usize,
}

impl LoweredWave {
    pub(in crate::code_rule_dsl) fn new(clauses: &[&LoweredMatcher]) -> Result<Self, Error> {
        let mut at = 0_usize;
        let mut shifted = Vec::with_capacity(clauses.len());
        for clause in clauses {
            let by = u16::try_from(at).map_err(|_| Error::InvalidOperand(0))?;
            shifted.push(clause.shifted(by));
            at += clause.labels.defs.len();
        }
        // Each clause's root label on Metal follows every pass's.
        if at + clauses.len() > usize::from(u16::MAX - LABEL_BASE) {
            return Err(Error::InvalidOperand(0));
        }
        Ok(Self { clauses: shifted, labels: at })
    }

    /// How many label passes the wave runs before its roots.
    pub(in crate::code_rule_dsl) fn passes(&self) -> usize {
        self.labels
    }

    /// Every clause's label programs with their domains, and whether each marks the nodes it
    /// captures (a targets label) rather than its starts.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn label_programs(&self) -> Vec<(&Program, bool)> {
        self.clauses
            .iter()
            .flat_map(|clause| clause.labels.defs.iter())
            .filter_map(|label| match label {
                Label::Roots(program, _) => Some((program, false)),
                Label::Targets(program, _) => Some((program, true)),
                _ => None,
            })
            .collect()
    }

    /// Each clause's root program, in clause order.
    pub(in crate::code_rule_dsl) fn programs(&self) -> Vec<Program> {
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
    pub(in crate::code_rule_dsl) fn labelled(&self, tapes: &[TapeDoc], active: &[Vec<bool>]) -> Result<Vec<TapeDoc>, Error> {
        let clauses = self.clauses.iter().collect::<Vec<_>>();
        let (steps, _) = levels(&clauses, |clause, id| Ok((clause, id)))?;
        let active = |clause: &LoweredMatcher, tape: usize| {
            let at = self.clauses.iter().position(|c| std::ptr::eq(c, clause)).expect("a clause of the wave");
            active[at][tape]
        };
        let active: &dyn Fn(&LoweredMatcher, usize) -> bool = &active;
        let mut sets = empty_sets(tapes, self.labels);
        let most = tapes.iter().map(TapeDoc::transducer_len).max().unwrap_or(0);
        for (recursive, levels) in &steps {
            if !recursive {
                for level in levels {
                    label_level(level, tapes, &mut sets, active)?;
                }
                continue;
            }
            if sweeps(levels) {
                for (at, (tape, sets)) in tapes.iter().zip(sets.iter_mut()).enumerate() {
                    let levels = levels
                        .iter()
                        .map(|level| level.iter().copied().filter(|&(clause, _)| active(clause, at)).collect::<Vec<_>>())
                        .collect::<Vec<_>>();
                    if levels.iter().any(|level| !level.is_empty()) {
                        sweep(&levels, tape, sets)?;
                    }
                }
                continue;
            }
            let mut settled = false;
            for _round in 0..=most + 1 {
                let mut changed = false;
                for level in levels {
                    changed |= label_level(level, tapes, &mut sets, active)?;
                }
                if !changed {
                    settled = true;
                    break;
                }
            }
            if !settled {
                return Err(Error::WorkLimit);
            }
        }
        Ok(tapes.iter().zip(sets).map(|(tape, sets)| tape.with_labels(sets)).collect())
    }

    /// Whether clause `clause`'s root program starts at `node` of `tape`: its root's kinds.
    pub(in crate::code_rule_dsl) fn starts(&self, clause: usize, tape: &TapeDoc, node: u32) -> bool {
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

fn empty_sets(tapes: &[TapeDoc], labels: usize) -> Vec<Arc<Vec<Vec<u64>>>> {
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
struct Entry(Vec<(u8, u16)>);

impl Entry {
    fn of(program: &Program) -> Self {
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

    fn admits(&self, tape: &TapeDoc, node: u32) -> bool {
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

#[cfg(target_os = "macos")]
impl LoweredMatcher {
    /// The batch's roots with every pass on Metal in one command buffer
    /// (`metal_program_backend::label_passes`), pattern leaves included.
    pub(in crate::code_rule_dsl) fn roots_metal(
        &self,
        tapes: &[TapeDoc],
    ) -> Result<(Vec<BTreeSet<u32>>, metal_program_backend::label_passes::Outcome), Error> {
        let no_labels = empty_sets(tapes, self.labels.defs.len());
        let roots = tapes.iter().zip(&no_labels).map(|(tape, sets)| domain(&self.domain, tape, sets).collect()).collect();
        let (mut found, outcome) = metal(&[self], self.labels.defs.len(), tapes, &[roots], None)?;
        Ok((found.pop().expect("one clause"), outcome))
    }
}

#[cfg(target_os = "macos")]
impl LoweredWave {
    /// Starts compiling the Metal label passes' pipelines, so a wave's first labels need not wait
    /// for them (`label_passes::warm`).
    pub(in crate::code_rule_dsl) fn warm_metal() {
        metal_program_backend::label_passes::warm();
    }

    /// Every clause's labels over the batch on Metal, in one command buffer (as `roots_metal`'s,
    /// without the roots), as views of its tapes the root programs read: `demand(batch)` then runs
    /// the roots with their results (captures, spans), which a label keeps only as bits.
    /// `active` as `labelled`'s.
    pub(in crate::code_rule_dsl) fn labelled_metal(
        &self,
        tapes: &[TapeDoc],
        active: &[Vec<bool>],
    ) -> Result<(Vec<TapeDoc>, metal_program_backend::label_passes::Outcome), Error> {
        let (_, outcome) = metal(&self.clauses.iter().collect::<Vec<_>>(), self.labels, tapes, &[], Some(active))?;
        let mut base = 0_usize;
        let _views = crate::code_rule_dsl::prof::Stage::start("labels.views");
        let views = tapes
            .iter()
            .map(|tape| {
                let len = tape.transducer_len();
                let sets = (0..self.labels)
                    .map(|label| {
                        let mut bits = vec![0_u64; len.div_ceil(64)];
                        for node in 0..len {
                            let global = base + node;
                            if outcome.labels[label * outcome.words + global / 32] >> (global % 32) & 1 != 0 {
                                bits[node / 64] |= 1 << (node % 64);
                            }
                        }
                        bits
                    })
                    .collect();
                base += len;
                tape.with_labels(Arc::new(sets))
            })
            .collect();
        Ok((views, outcome))
    }

    /// Each clause's matches of the batch from `roots` (per clause, per tape: the candidates its
    /// root program starts from), every clause's passes and roots in one command buffer.
    pub(in crate::code_rule_dsl) fn roots_metal(
        &self,
        tapes: &[TapeDoc],
        roots: &[Vec<Vec<u32>>],
    ) -> Result<(Vec<Vec<BTreeSet<u32>>>, metal_program_backend::label_passes::Outcome), Error> {
        metal(&self.clauses.iter().collect::<Vec<_>>(), self.labels, tapes, roots, None)
    }
}

/// `clauses`' matches of the batch on Metal, in one command buffer: their labels (disjoint, `labels`
/// in all) as passes, then each clause's root program from its `roots` (per tape) as one pass more.
/// The clauses' segments are merged by phase: a clause's runs of single-label steps take even
/// phases and its recursive groups odd ones, each past the clause's segment before it, and a
/// phase's segments of every clause are one segment. So non-recursive passes never re-run per
/// round, recursive groups of different clauses share their rounds, and the wave's levels are the
/// deepest clause's per phase, not their sum. `active` (per clause, per tape): where a clause's
/// passes start, none elsewhere (`LoweredWave::labelled`).
#[cfg(target_os = "macos")]
fn metal<'a>(
    clauses: &[&'a LoweredMatcher],
    labels: usize,
    tapes: &[TapeDoc],
    clause_roots: &[Vec<Vec<u32>>],
    active: Option<&[Vec<bool>]>,
) -> Result<(Vec<Vec<BTreeSet<u32>>>, metal_program_backend::label_passes::Outcome), Error> {
    use metal_program_backend::label_passes::{Pass, Plan, run};
    let bases = tapes
        .iter()
        .scan(0_u32, |base, tape| {
            let at = *base;
            *base += tape.transducer_len() as u32;
            Some(at)
        })
        .collect::<Vec<_>>();
    let nodes = tapes.iter().map(TapeDoc::transducer_len).sum::<usize>();
    let words = nodes.div_ceil(32).max(1);
    let no_labels = empty_sets(tapes, labels);
    // A pass's roots: its domain's nodes. A label domain's program tests the label first, so on
    // Metal, where the label is not known before the command buffer runs, it starts at the nodes
    // of the kinds the label can have (`Labels::bound`).
    let roots = |clause: &LoweredMatcher, starts: &Domain, program: Option<&Program>| {
        let entry = program.map(Entry::of);
        let bound = match starts {
            Domain::Label(_) => {
                Some(clause.labels.bound(starts, &mut Vec::new()).map_or(Domain::All, |kinds| Domain::Kinds(kinds.into_iter().collect())))
            }
            _ => None,
        };
        let at = clauses.iter().position(|c| std::ptr::eq(*c, clause)).expect("a clause of the batch");
        tapes
            .iter()
            .zip(&no_labels)
            .enumerate()
            .map(|(tape_at, (tape, sets))| {
                if active.is_some_and(|active| !active[at][tape_at]) {
                    return Vec::new();
                }
                domain(bound.as_ref().unwrap_or(starts), tape, sets)
                    .filter(|&node| entry.as_ref().is_none_or(|entry| entry.admits(tape, node)))
                    .collect()
            })
            .collect::<Vec<Vec<u32>>>()
    };
    let index = |label: u16| u32::from(label - LABEL_BASE);
    let mut slots = 0_u32;
    let mut patterns = Vec::new();
    let mut pass = |clause: &'a LoweredMatcher, id: u16| -> Result<Pass<'a>, Error> {
        let target = index(id);
        Ok(match clause.labels.def(id) {
            Label::Pattern(pattern, starts) | Label::Constrained(pattern, starts, _) => {
                let constraints = match clause.labels.def(id) {
                    Label::Constrained(_, _, constraints) => {
                        constraints.iter().map(|(name, label)| (name.clone(), index(*label))).collect()
                    }
                    _ => Vec::new(),
                };
                let packed = pattern.ast_pattern.as_ref().ok_or(Error::UnsupportedPattern)?;
                patterns.push(packed);
                Pass::Pattern {
                    target,
                    pattern: patterns.len() - 1,
                    roots: roots(clause, starts, None)
                        .into_iter()
                        .zip(&bases)
                        .flat_map(|(roots, base)| roots.into_iter().map(move |node| base + node))
                        .collect(),
                    constraints,
                }
            }
            Label::Roots(program, starts) | Label::Targets(program, starts) => Pass::Program {
                target,
                program,
                roots: roots(clause, starts, Some(program)),
                captured: matches!(clause.labels.def(id), Label::Targets(..)),
            },
            Label::Count {
                program,
                domain: starts,
                by,
                min,
                max,
            } => {
                slots += 1;
                Pass::Count {
                    target,
                    slot: slots - 1,
                    program,
                    roots: roots(clause, starts, Some(program)),
                    captured: matches!(by, Tally::Capture),
                    min: u32::try_from(*min).unwrap_or(u32::MAX),
                    max: max.map_or(u32::MAX, |max| u32::try_from(max).unwrap_or(u32::MAX)),
                }
            }
            Label::Complement(of) => Pass::Complement { target, of: index(*of) },
            Label::Empty => Pass::Empty { target },
        })
    };
    let (mut steps, mut chains) = crate::code_rule_dsl::prof::time("labels.plan", || levels(clauses, &mut pass))?;
    let all = clauses;
    // A clause's roots: one label past every pass's (none where the roots run apart, `labelled_metal`).
    let roots = !clause_roots.is_empty();
    let clauses = if roots { clauses } else { &[] };
    if roots {
    steps.push((
        false,
        vec![
            clauses
                .iter()
                .zip(clause_roots)
                .enumerate()
                .map(|(at, (clause, roots))| Pass::Program {
                    target: (labels + at) as u32,
                    program: &clause.program,
                    roots: roots.clone(),
                    captured: false,
                })
                .collect(),
        ],
    ));
    chains.push((None, false));
    }
    let outcome = match active {
        // The wave's labels: a recursive group that sweeps runs on the CPU's sweep, each file a
        // walk in node order, between the GPU's wide levels (O26).
        Some(active) => apart(all, labels, slots as usize, patterns, steps, chains, tapes, active)?,
        None => run(&Plan { labels: labels + clauses.len(), counts: slots as usize, patterns, steps, chains, initial: None }, tapes)?,
    };
    let found = (0..clauses.len())
        .map(|at| {
            tapes
                .iter()
                .zip(&bases)
                .map(|(tape, base)| {
                    (0..tape.transducer_len() as u32)
                        .filter(|node| {
                            let global = (base + node) as usize;
                            outcome.labels[(labels + at) * words + global / 32] >> (global % 32) & 1 != 0
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    Ok((found, outcome))
}

/// The wave's label plan with its sweeping groups apart: the steps between them as Metal runs
/// (each from the labels so far), each sweeping group as the CPU sweeps it (`sweep`, on the tapes
/// where its clauses are active). A sweep follows a file's nodes in order, one thread a file on
/// the GPU, where the interpreter's thread is far slower than a CPU core; the levels between are
/// a task per node and program, as wide as the GPU.
#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
fn apart<'a>(
    clauses: &[&'a LoweredMatcher],
    labels: usize,
    counts: usize,
    patterns: Vec<&'a PackedPattern>,
    steps: Vec<(bool, Vec<Vec<metal_program_backend::label_passes::Pass<'a>>>)>,
    chains: Vec<(Option<Vec<u16>>, bool)>,
    tapes: &[TapeDoc],
    active: &[Vec<bool>],
) -> Result<metal_program_backend::label_passes::Outcome, Error> {
    use metal_program_backend::label_passes::{Outcome, Plan, run};
    let (host, _) = levels(clauses, |clause, id| Ok((clause, id)))?;
    if host.len() != steps.len() {
        return Err(Error::BackendFailure);
    }
    let nodes = tapes.iter().map(TapeDoc::transducer_len).sum::<usize>();
    let words = nodes.div_ceil(32).max(1);
    let mut outcome = Outcome { labels: vec![0; labels * words], words, ..Outcome::default() };
    let mut segment = (Vec::new(), Vec::new());
    let flush = |segment: &mut (Vec<_>, Vec<_>), outcome: &mut Outcome| -> Result<(), Error> {
        if segment.0.is_empty() {
            return Ok(());
        }
        let plan = Plan {
            labels,
            counts,
            patterns: patterns.clone(),
            steps: std::mem::take(&mut segment.0),
            chains: std::mem::take(&mut segment.1),
            initial: Some(std::mem::take(&mut outcome.labels)),
        };
        let ran = crate::code_rule_dsl::prof::time("labels.gpu", || run(&plan, tapes))?;
        outcome.labels = ran.labels;
        outcome.command_buffers += ran.command_buffers;
        outcome.dispatches += ran.dispatches;
        outcome.rounds = outcome.rounds.max(ran.rounds);
        (0..3).for_each(|at| outcome.waits[at] += ran.waits[at]);
        Ok(())
    };
    for ((step, chain), (recursive, levels)) in steps.into_iter().zip(chains).zip(&host) {
        if !(*recursive && sweeps(levels)) {
            segment.0.push(step);
            segment.1.push(chain);
            continue;
        }
        flush(&mut segment, &mut outcome)?;
        let mut sets = crate::code_rule_dsl::prof::time("labels.to_sets", || to_sets(&outcome.labels, labels, words, tapes));
        // A file's sweep reads and marks its own labels only, so the files sweep in parallel.
        use rayon::prelude::*;
        let _sweep = crate::code_rule_dsl::prof::Stage::start("labels.sweep");
        tapes.par_iter().zip(sets.par_iter_mut()).enumerate().try_for_each(|(at, (tape, sets))| {
            let levels = levels
                .iter()
                .map(|level| {
                    level
                        .iter()
                        .copied()
                        .filter(|&(clause, _)| {
                            clauses.iter().position(|c| std::ptr::eq(*c, clause)).is_some_and(|clause| active[clause][at])
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            if levels.iter().any(|level| !level.is_empty()) {
                sweep(&levels, tape, sets)?;
            }
            Ok::<_, Error>(())
        })?;
        drop(_sweep);
        outcome.labels = crate::code_rule_dsl::prof::time("labels.to_words", || to_words(&sets, labels, words, tapes));
    }
    flush(&mut segment, &mut outcome)?;
    Ok(outcome)
}

/// Label words over the batch's nodes (label-major, `words` a label) as each tape's label sets.
fn to_sets(bits: &[u32], labels: usize, words: usize, tapes: &[TapeDoc]) -> Vec<Arc<Vec<Vec<u64>>>> {
    let at = |label: usize, bit: usize| -> u64 {
        let word = |i: usize| u128::from(bits.get(label * words + i).copied().unwrap_or(0));
        let i = bit / 32;
        let wide = word(i) | word(i + 1) << 32 | word(i + 2) << 64;
        (wide >> (bit % 32)) as u64
    };
    let mut base = 0;
    tapes
        .iter()
        .map(|tape| {
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
            base += len;
            Arc::new(sets)
        })
        .collect()
}

/// `to_sets` the other way.
fn to_words(sets: &[Arc<Vec<Vec<u64>>>], labels: usize, words: usize, tapes: &[TapeDoc]) -> Vec<u32> {
    let mut bits = vec![0_u32; labels * words];
    let mut base = 0;
    for (tape, sets) in tapes.iter().zip(sets) {
        for (label, set) in sets.iter().enumerate().take(labels) {
            for (k, &value) in set.iter().enumerate() {
                let bit = base + 64 * k;
                let wide = u128::from(value) << (bit % 32);
                for part in 0..3 {
                    if let Some(word) = bits.get_mut(label * words + bit / 32 + part) {
                        *word |= (wide >> (32 * part)) as u32;
                    }
                }
            }
        }
        base += tape.transducer_len();
    }
    bits
}

/// A wave's label passes as its plan runs them, on either backend: by wave level, its
/// non-recursive passes (one level each, past the levels of the labels they read), then its
/// recursive groups merged (their levels by position at a node, `local_levels`; their rounds
/// shared). Each step is (recursive, its levels of passes); with each, for Metal's round count, a
/// recursive step's labels' kinds and whether its recursion runs along siblings.
#[allow(clippy::type_complexity)]
fn levels<'a, T>(
    clauses: &[&'a LoweredMatcher],
    mut pass: impl FnMut(&'a LoweredMatcher, u16) -> Result<T, Error>,
) -> Result<(Vec<(bool, Vec<Vec<T>>)>, Vec<(Option<Vec<u16>>, bool)>), Error> {
    // By wave level: its non-recursive passes, and its recursive groups merged (their levels by
    // position, their rounds shared, their recursive labels' kinds and whether any steps to
    // siblings).
    let mut single: Vec<Vec<T>> = Vec::new();
    let mut groups: Vec<(Vec<Vec<T>>, Option<BTreeSet<u16>>, bool)> = Vec::new();
    for &clause in clauses {
        // When each label is final: (wave level, whether in the level's recursive part).
        let mut done = BTreeMap::<u16, (usize, bool)>::new();
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
fn label_level(
    level: &[(&LoweredMatcher, u16)],
    tapes: &[TapeDoc],
    sets: &mut [Arc<Vec<Vec<u64>>>],
    active: &dyn Fn(&LoweredMatcher, usize) -> bool,
) -> Result<bool, Error> {
    let views = tapes.iter().zip(sets.iter()).map(|(tape, sets)| tape.with_labels(Arc::clone(sets))).collect::<Vec<_>>();
    let found = level
        .iter()
        .map(|&(clause, id)| {
            let on = (0..tapes.len()).filter(|&at| active(clause, at)).collect::<Vec<_>>();
            let (views, read) = (on.iter().map(|&at| views[at].clone()).collect::<Vec<_>>(), on.iter().map(|&at| Arc::clone(&sets[at])).collect::<Vec<_>>());
            Ok::<_, Error>(on.into_iter().zip(clause.evaluate(clause.labels.def(id), &views, &read)?).collect::<Vec<_>>())
        })
        .collect::<Result<Vec<_>, _>>()?;
    drop(views);
    let mut changed = false;
    for (&(_, id), found) in level.iter().zip(found) {
        for (at, set) in found {
            let slot = &mut Arc::make_mut(&mut sets[at])[usize::from(id - LABEL_BASE)];
            if *slot != set {
                *slot = set;
                changed = true;
            }
        }
    }
    Ok(changed)
}

/// Whether a recursive step sweeps, as on Metal (`label_passes::sweeps`): its passes are programs
/// that mark their roots or captures (only adding to their labels), empty or complements; with
/// complements, its programs mark their roots (each node's verdicts are then its own, computed at
/// it in a reverse sweep, since walks through `not` move forward in preorder, `matcher::lower`).
fn sweeps(levels: &[Vec<(&LoweredMatcher, u16)>]) -> bool {
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
/// them. A recursion along the tree settles in a sweep or two, not a node depth per round.
fn sweep(levels: &[Vec<(&LoweredMatcher, u16)>], tape: &TapeDoc, sets: &mut Arc<Vec<Vec<u64>>>) -> Result<(), Error> {
    let complements = levels.iter().flatten().any(|&(clause, id)| matches!(clause.labels.def(id), Label::Complement(_)));
    let len = tape.transducer_len() as u32;
    // (node, level, pass) for every node a pass may start at; a label domain is tested at the node.
    let mut entries = Vec::new();
    let no_labels = vec![Vec::new(); 0];
    for (at, level) in levels.iter().enumerate() {
        for &(clause, id) in level {
            let starts: Box<dyn Iterator<Item = u32>> = match clause.labels.def(id) {
                Label::Roots(_, Domain::Kinds(kinds)) | Label::Targets(_, Domain::Kinds(kinds)) => {
                    Box::new(domain(&Domain::Kinds(kinds.clone()), tape, &no_labels).collect::<Vec<_>>().into_iter())
                }
                Label::Empty => Box::new(std::iter::empty()),
                _ => Box::new(0..len),
            };
            let entry = match clause.labels.def(id) {
                Label::Roots(program, _) | Label::Targets(program, _) => Some(Entry::of(program)),
                _ => None,
            };
            entries.extend(starts.filter(|&node| entry.as_ref().is_none_or(|entry| entry.admits(tape, node))).map(|node| (node, at, clause, id)));
        }
    }
    entries.sort_by_key(|&(node, at, ..)| (node, at));
    // Each program's micro walk, where it is one (O28): no interpreter task per (node, pass).
    let walks = levels
        .iter()
        .flatten()
        .map(|&(clause, id)| {
            // With its text bits on this tape (`Micro::texts`).
            let walk = match clause.labels.def(id) {
                Label::Roots(program, _) | Label::Targets(program, _) => {
                    Micro::of(program).and_then(|walk| walk.texts(program, tape).map(|texts| (walk, texts)))
                }
                _ => None,
            };
            ((std::ptr::from_ref(clause) as usize, id), walk)
        })
        .collect::<BTreeMap<_, _>>();
    let mut frames = Frames::default();
    let mut marks = Vec::new();
    let mut reverse = complements;
    for _sweep in 0..=len + 1 {
        let mut changed = false;
        let mut run = |&(node, _, clause, id): &(u32, usize, &LoweredMatcher, u16)| -> Result<(), Error> {
            let target = usize::from(id - LABEL_BASE);
            let marked = match clause.labels.def(id) {
                Label::Complement(of) => {
                    let set = !has(sets, *of, node);
                    let bits = &mut Arc::make_mut(sets)[target];
                    let was = bits[node as usize / 64] >> (node % 64) & 1 != 0;
                    if was != set {
                        bits[node as usize / 64] ^= 1 << (node % 64);
                        changed = true;
                    }
                    return Ok(());
                }
                Label::Roots(program, starts) | Label::Targets(program, starts) => {
                    if let Domain::Label(label) = starts
                        && !has(sets, *label, node)
                    {
                        return Ok(());
                    }
                    let targets = matches!(clause.labels.def(id), Label::Targets(..));
                    marks.clear();
                    let walked = walks[&(std::ptr::from_ref(clause) as usize, id)]
                        .as_ref()
                        .is_some_and(|(walk, texts)| walk.run(tape, sets, texts, node, targets, &mut frames, &mut |node| marks.push(node)).is_ok());
                    if !walked {
                        marks.clear();
                        let view = tape.with_labels(Arc::clone(sets));
                        let outputs = program.execute_roots(&view, [node])?;
                        drop(view);
                        if targets {
                            marks.extend(outputs.iter().flat_map(|output| output.captures.iter().map(|capture| capture.node)));
                        } else {
                            marks.extend(outputs.iter().map(|output| output.start));
                        }
                    }
                    std::mem::take(&mut marks)
                }
                _ => return Ok(()),
            };
            for &node in &marked {
                if !has(sets, id, node) {
                    Arc::make_mut(sets)[target][node as usize / 64] |= 1 << (node % 64);
                    changed = true;
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
    fn of(output: &Output) -> Self {
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

/// The nodes of `tape` in `domain`, in preorder.
fn domain<'a>(domain: &'a Domain, tape: &'a TapeDoc, sets: &'a [Vec<u64>]) -> impl Iterator<Item = u32> + 'a {
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
            _ => {}
        }
        let program = Builder::new(self.labels).roots(tree)?;
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
        let program = Builder::new(self.labels).roots(tree)?;
        self.labels.defs[usize::from(label - LABEL_BASE)] = Label::Roots(program, Domain::of(tree));
        // Only the labels that read the utility's, directly or through each other, are evaluated
        // round after round; the others are passes of their own ahead of the group.
        let members = self.labels.steps.drain(start..).flatten().collect::<Vec<_>>();
        let mut group = Vec::new();
        for member in members {
            let reads = self.labels.reads(member);
            if reads.iter().any(|read| *read == label || group.contains(read)) {
                group.push(member);
            } else {
                self.labels.steps.push(vec![member]);
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
        Ok(())
    }

    /// `top: step; FORK reached; SUPERTYPE bound; GOTO top; reached:` — every node the step
    /// reaches, going on past a node only where it has `bound`.
    fn bounded(&mut self, step: u8, bound: u16) -> Result<(), Error> {
        let top = self.here()?;
        self.push(step, 0, 0);
        let fork = self.push(FORK, 0, 0);
        self.push(SUPERTYPE, 0, bound);
        self.push(GOTO, 0, top);
        let reached = self.here()?;
        self.patch(fork, reached);
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
