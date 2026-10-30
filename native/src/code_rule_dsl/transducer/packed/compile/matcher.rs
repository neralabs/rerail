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
pub(in crate::code_rule_dsl) struct LoweredMatcher {
    labels: Labels,
    program: Program,
    domain: Domain,
}

#[derive(Default)]
struct Labels {
    /// By label id less `LABEL_BASE`.
    defs: Vec<Label>,
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
        LABEL_BASE + (self.defs.len() - 1) as u16
    }

    fn def(&self, id: u16) -> &Label {
        &self.defs[usize::from(id - LABEL_BASE)]
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
        let mut sets = tapes
            .iter()
            .map(|tape| {
                Arc::new(vec![vec![0_u64; tape.transducer_len().div_ceil(64)]; self.labels.defs.len()])
            })
            .collect::<Vec<_>>();
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
                        .zip(&sets)
                        .map(|(tape, sets)| tape.with_labels(Arc::clone(sets)))
                        .collect::<Vec<_>>();
                    let found = self.evaluate(self.labels.def(id), &views, &sets)?;
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
            Label::Roots(program, starts) | Label::Targets(program, starts) => {
                let outputs = run(program, views, starts, sets)?;
                Ok(views
                    .iter()
                    .zip(outputs)
                    .map(|(view, outputs)| {
                        let mut bits = empty(view);
                        for output in outputs {
                            match label {
                                Label::Roots(..) => set(&mut bits, output.start),
                                _ => output.captures.iter().for_each(|capture| set(&mut bits, capture.node)),
                            }
                        }
                        bits
                    })
                    .collect())
            }
            Label::Count {
                program,
                domain: starts,
                by,
                min,
                max,
            } => {
                let outputs = run(program, views, starts, sets)?;
                Ok(views
                    .iter()
                    .zip(outputs)
                    .map(|(view, outputs)| {
                        let mut counts = vec![0_usize; view.transducer_len()];
                        for output in outputs {
                            match by {
                                Tally::Start => counts[output.start as usize] += 1,
                                Tally::Capture => {
                                    output.captures.iter().for_each(|capture| counts[capture.node as usize] += 1);
                                }
                            }
                        }
                        let mut bits = empty(view);
                        for (node, &count) in counts.iter().enumerate() {
                            if count >= *min && max.is_none_or(|max| count <= max) {
                                set(&mut bits, node as u32);
                            }
                        }
                        bits
                    })
                    .collect())
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

fn has(sets: &[Vec<u64>], label: u16, node: u32) -> bool {
    sets[usize::from(label - LABEL_BASE)][node as usize / 64] >> (node % 64) & 1 != 0
}

/// `program`'s outputs from each tape's `starts`.
fn run(
    program: &Program,
    views: &[TapeDoc],
    starts: &Domain,
    sets: &[Arc<Vec<Vec<u64>>>],
) -> Result<Vec<Vec<Output>>, Error> {
    views
        .iter()
        .zip(sets)
        .map(|(view, sets)| program.execute_roots(view, domain(starts, view, sets)))
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
        let labels = self.labels.defs.len() + 1;
        let no_labels = tapes
            .iter()
            .map(|tape| Arc::new(vec![vec![0_u64; tape.transducer_len().div_ceil(64)]; self.labels.defs.len()]))
            .collect::<Vec<_>>();
        // A pass's roots: its domain's nodes. A label domain's program tests the label first, so on
        // Metal, where the label is not known before the command buffer runs, it starts everywhere.
        let roots = |starts: &Domain| {
            tapes
                .iter()
                .zip(&no_labels)
                .map(|(tape, sets)| match starts {
                    Domain::Label(_) => (0..tape.transducer_len() as u32).collect(),
                    _ => domain(starts, tape, sets).collect(),
                })
                .collect::<Vec<Vec<u32>>>()
        };
        let index = |label: u16| u32::from(label - LABEL_BASE);
        // Segments: each run of single-label steps, and each recursive group, as its levels. A
        // label's level is one past the deepest label of its segment it reads that is computed
        // before it; in a recursive group, a read of a later member is the last round's.
        let mut segments: Vec<(bool, Vec<u16>)> = Vec::new();
        for step in &self.labels.steps {
            let recursive = self.labels.recursive.values().any(|id| step.contains(id));
            match segments.last_mut() {
                Some((false, members)) if !recursive => members.extend(step),
                _ => segments.push((recursive, step.clone())),
            }
        }
        let mut slots = 0_u32;
        let mut patterns = Vec::new();
        let mut steps = Vec::new();
        for (recursive, members) in &segments {
            let mut level = BTreeMap::<u16, usize>::new();
            let mut levels: Vec<Vec<Pass<'_>>> = Vec::new();
            for &id in members {
                let at = self
                    .labels
                    .reads(id)
                    .iter()
                    .filter_map(|read| level.get(read))
                    .map(|deepest| deepest + 1)
                    .max()
                    .unwrap_or(0);
                let target = index(id);
                let pass = match self.labels.def(id) {
                    Label::Pattern(pattern, starts) | Label::Constrained(pattern, starts, _) => {
                        let constraints = match self.labels.def(id) {
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
                            roots: roots(starts)
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
                        roots: roots(starts),
                        captured: matches!(self.labels.def(id), Label::Targets(..)),
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
                            roots: roots(starts),
                            captured: matches!(by, Tally::Capture),
                            min: u32::try_from(*min).unwrap_or(u32::MAX),
                            max: max.map_or(u32::MAX, |max| u32::try_from(max).unwrap_or(u32::MAX)),
                        }
                    }
                    Label::Complement(of) => Pass::Complement { target, of: index(*of) },
                    Label::Empty => Pass::Empty { target },
                };
                level.insert(id, at);
                if levels.len() <= at {
                    levels.resize_with(at + 1, Vec::new);
                }
                levels[at].push(pass);
            }
            levels.retain(|level| !level.is_empty());
            steps.push((*recursive, levels));
        }
        let last = labels as u32 - 1;
        steps.push((
            false,
            vec![vec![Pass::Program {
                target: last,
                program: &self.program,
                roots: roots(&self.domain),
                captured: false,
            }]],
        ));
        // Per step: a recursive group's recursive labels' kinds, and whether it steps to siblings.
        let chains = segments
            .iter()
            .map(|(recursive, members)| {
                if !recursive {
                    return (None, false);
                }
                let mut kinds = Some(BTreeSet::new());
                for id in members.iter().filter(|id| self.labels.recursive.values().any(|label| label == *id)) {
                    match (self.labels.def(*id), kinds.as_mut()) {
                        (Label::Roots(_, Domain::Kinds(own)), Some(kinds)) => kinds.extend(own.iter().copied()),
                        _ => kinds = None,
                    }
                }
                let siblings = members.iter().any(|id| match self.labels.def(*id) {
                    Label::Roots(program, _) | Label::Targets(program, _) | Label::Count { program, .. } => {
                        program.words.iter().any(|&word| matches!(split(word).0, NEXT | FOLLOWING))
                    }
                    _ => false,
                });
                (kinds.map(|kinds| kinds.into_iter().collect()), siblings)
            })
            .chain([(None, false)])
            .collect();
        let outcome = run(
            &Plan {
                labels,
                counts: slots as usize,
                patterns,
                steps,
                chains,
            },
            tapes,
        )?;
        let found = tapes
            .iter()
            .zip(&bases)
            .map(|(tape, base)| {
                (0..tape.transducer_len() as u32)
                    .filter(|node| {
                        let global = (base + node) as usize;
                        outcome.labels[last as usize * words + global / 32] >> (global % 32) & 1 != 0
                    })
                    .collect()
            })
            .collect();
        Ok((found, outcome))
    }
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
