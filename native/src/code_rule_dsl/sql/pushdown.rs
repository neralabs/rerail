//! Transducer pushdown (`docs/sql-set-composition-design.md`, "Speed levers", lever 3): a
//! capture-free, node-local sub-rule of a statement compiles to one transducer program
//! (`transducer::Program`), which the `program_match` terminal runs over the batch's files with
//! the node-set executor (`Program::holds`), instead of one statement per operator.
//!
//! The sub-rules that compile are the operators whose verdict at a node is a function of the
//! file's tree around it, with no capture and no unknown verdict: `kind`, `regex`, `all`/`any`/
//! `not` of such rules, `has`/`inside` (and their `field` forms) with a neighbour or end stop over
//! such a rule, and `count` without `as` over those walks. A three-valued statement's
//! `AllOpen`/`AnyOpen`/`Search` over such rules are two-valued and compile as `all`/`any`/
//! relations. A stage whose rule compiles whole is one `program_match` statement over its files.
//! Anything else (captures, patterns, facts, rule stops, sibling walks, recursion, parsed trees)
//! stays in SQL.
//!
//! A program is set-based: `program_match` receives a frame's distinct `(file, node)` keys in one
//! call and returns the keys at which the program holds; the node-set executor computes that set
//! one instruction at a time over whole node sets of each file. A program has no recursion and
//! no loop, and emits no recursive CTE: it replaces the frames, walks and semi-joins its SQL
//! lowering would run, never adds a statement.
//!
//! The SQL lowering of a pushed-down sub-rule stays its definition: `RERAIL_SQL_PUSHDOWN=0` lowers
//! every statement without pushdown, and the differential tests compare the two over the
//! fixtures, operator by operator.

use super::emit::{Effect, Emitted, FILES};
use super::lower::ProgramBuilder;
use super::plan::{AllTerm, OfRule, Op, Stop, Walk};
use super::terminals::{ProgramTerm, Terminal};
use crate::code_rule_dsl::transducer::{
    ANCESTOR, CHILD, COUNT, Count, DESCEND, EMIT, FIELD, FIELD_FIRST, FORK, KIND, NEGATE, PARENT,
    Program, STOP, TEST, TEXT, TextPredicate, word,
};
use regex::Regex;
use std::collections::HashMap;

/// A capture-free node-local sub-rule.
#[derive(Clone, Debug)]
pub(super) enum Rule {
    Kinds(Vec<u16>),
    Regex(Regex),
    All(Vec<Rule>),
    Any(Vec<Rule>),
    Not(Box<Rule>),
    Walk(Walk, Box<Rule>),
    Count {
        walk: Walk,
        sub: Box<Rule>,
        min: usize,
        max: Option<usize>,
    },
}

impl Rule {
    /// About how many statements its SQL lowering runs, which one program statement replaces: a
    /// rule of fewer than 2 is not worth a program. A kind test counts none (a walk reads its
    /// candidates by kind, and a frame whose nodes all pass it is not tested again), nor does an
    /// `all` (a pipeline of its terms); an `any` adds a remainder per branch after the first, a
    /// `not` an anti-join, a walk its candidates, a count its candidates and its counts.
    fn weight(&self) -> usize {
        match self {
            Self::Kinds(_) => 0,
            Self::Regex(_) => 1,
            Self::All(items) => items.iter().map(Self::weight).sum::<usize>(),
            Self::Any(items) => {
                items.len().saturating_sub(1) + items.iter().map(Self::weight).sum::<usize>()
            }
            Self::Not(inner) | Self::Walk(_, inner) => 1 + inner.weight(),
            Self::Count { sub, .. } => 2 + sub.weight(),
        }
    }
}

/// The least weight a sub-rule is pushed down at (a stage, at 1: a stage of kind tests alone
/// stays SQL, whose root frame reads the kind postings). Measured on the program stage: pushing
/// every capture-free rule of 2 operators or more (counting kind tests) ran 5% less statement
/// CPU than no pushdown, this weight 12% less, and a threshold of 3 no better. A small rule is
/// cheaper in SQL than its program: `text_match` scans a file once for a regex, and a walk reads
/// its candidates from the kind postings, where a program tests node by node. `EAGER` (tests)
/// pushes every capture-free rule down, to compare the most programs with their lowering.
fn threshold(stage: bool) -> usize {
    #[cfg(test)]
    if EAGER.get() {
        return 0;
    }
    if stage { 1 } else { 2 }
}

#[cfg(test)]
thread_local! {
    /// Whether this thread pushes every capture-free rule down, whatever its weight.
    pub(super) static EAGER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Each program's terminal by its rule: one terminal per distinct rule over the whole program
/// (a rule's program reads kind ids, so rules of two grammars that differ differ in them).
#[derive(Default)]
pub(super) struct Pushdown {
    terms: HashMap<String, usize>,
}

/// Whether statements are lowered with pushdown (`RERAIL_SQL_PUSHDOWN=0` turns it off; the tests
/// turn it off per thread to compare).
pub(super) fn enabled() -> bool {
    static ENABLED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        std::env::var("RERAIL_SQL_PUSHDOWN").map_or(true, |value| value != "0")
    });
    #[cfg(test)]
    if let Some(enabled) = OVERRIDE.get() {
        return enabled;
    }
    *ENABLED
}

#[cfg(test)]
thread_local! {
    /// This thread's pushdown setting, over the environment's.
    pub(super) static OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// While held, this thread lowers without pushdown: for tests of the SQL lowering itself (the
/// statements a walk or a recursion emits), which pushdown would replace by a program.
#[cfg(test)]
pub(super) struct SqlLowering(Option<bool>);

#[cfg(test)]
impl SqlLowering {
    pub(super) fn hold() -> Self {
        Self(OVERRIDE.replace(Some(false)))
    }
}

#[cfg(test)]
impl Drop for SqlLowering {
    fn drop(&mut self) {
        OVERRIDE.set(self.0);
    }
}

fn walkable(walk: Walk) -> bool {
    match walk {
        Walk::Has(stop)
        | Walk::Inside(stop)
        | Walk::FieldHas(_, stop)
        | Walk::FieldInside(_, stop) => {
            matches!(stop, Stop::Neighbor | Stop::End)
        }
        _ => false,
    }
}

/// `op` as a capture-free node-local rule, where it is one.
pub(super) fn rule(op: &Op, builder: &ProgramBuilder) -> Option<Rule> {
    let all = |items: &mut dyn Iterator<Item = &Op>| -> Option<Vec<Rule>> {
        items.map(|item| rule(item, builder)).collect()
    };
    Some(match op {
        Op::Kinds(kinds) => Rule::Kinds(kinds.clone()),
        Op::Regex(term) => match builder.terminals.get(*term)? {
            Terminal::Regex(regex) => Rule::Regex(regex.clone()),
            _ => return None,
        },
        Op::Never => Rule::Any(Vec::new()),
        Op::All(items) => Rule::All(all(&mut items.iter())?),
        Op::AllOpen(terms) => Rule::All(all(&mut terms.iter().map(|term| &term.op))?),
        Op::Any(items) => Rule::Any(all(&mut items.iter())?),
        Op::AnyOpen(branches) => Rule::Any(all(&mut branches.iter().map(|(op, _)| op))?),
        Op::Not(inner) => Rule::Not(Box::new(rule(inner, builder)?)),
        Op::Relation {
            walk,
            sub,
            binds: false,
            ..
        }
        | Op::Search {
            walk,
            sub,
            binds: false,
            ..
        } if walkable(*walk) => Rule::Walk(*walk, Box::new(rule(sub, builder)?)),
        Op::Count {
            walk,
            sub,
            min,
            max,
            bind: None,
            ..
        } if matches!(
            walk,
            Walk::Has(Stop::Neighbor | Stop::End) | Walk::Inside(Stop::Neighbor | Stop::End)
        ) =>
        {
            Rule::Count {
                walk: *walk,
                sub: Box::new(rule(sub, builder)?),
                min: *min,
                max: *max,
            }
        }
        _ => return None,
    })
}

/// The program terminal of `rule` (one per distinct rule).
fn term(rule: &Rule, builder: &mut ProgramBuilder) -> Option<usize> {
    let key = format!("{rule:?}");
    if let Some(&term) = builder.pushdown.terms.get(&key) {
        return Some(term);
    }
    let program = compile(rule)?;
    #[cfg(test)]
    SHAPES.with_borrow_mut(|shapes| {
        let mut found = std::collections::BTreeSet::new();
        rule.operators(&mut found);
        for operator in found {
            *shapes.entry(operator).or_default() += 1;
        }
    });
    let term = builder.terminals.len();
    builder
        .terminals
        .push(Terminal::Program(ProgramTerm { program }));
    builder.pushdown.terms.insert(key, term);
    Some(term)
}

#[cfg(test)]
thread_local! {
    /// Per operator a compiled program holds (`Rule::operators`), how many programs this thread
    /// compiled with it.
    pub(super) static SHAPES: std::cell::RefCell<std::collections::BTreeMap<String, usize>> =
        std::cell::RefCell::default();
}

#[cfg(test)]
impl Rule {
    /// The operators the rule holds, by name (walks with their stop: `has.nb`, `inside.end`…).
    fn operators(&self, out: &mut std::collections::BTreeSet<String>) {
        let walk = |walk: &Walk| {
            let (name, stop) = match walk {
                Walk::Has(stop) => ("has", stop),
                Walk::Inside(stop) => ("inside", stop),
                Walk::FieldHas(_, stop) => ("fieldHas", stop),
                Walk::FieldInside(_, stop) => ("fieldInside", stop),
                _ => unreachable!("only walkable walks compile"),
            };
            let stop = match stop {
                Stop::Neighbor => "nb",
                _ => "end",
            };
            format!("{name}.{stop}")
        };
        match self {
            Self::Kinds(_) => {
                out.insert("kind".to_owned());
            }
            Self::Regex(_) => {
                out.insert("regex".to_owned());
            }
            Self::All(items) | Self::Any(items) => {
                out.insert(
                    if matches!(self, Self::All(_)) {
                        "all"
                    } else {
                        "any"
                    }
                    .to_owned(),
                );
                items.iter().for_each(|item| item.operators(out));
            }
            Self::Not(inner) => {
                out.insert("not".to_owned());
                inner.operators(out);
            }
            Self::Walk(w, sub) => {
                out.insert(walk(w));
                sub.operators(out);
            }
            Self::Count { walk: w, sub, .. } => {
                out.insert(format!("count:{}", walk(w)));
                sub.operators(out);
            }
        }
    }
}

/// The statement of a stage whose lowered rule `op` compiles whole and is worth a program
/// (`threshold`): every node of its files the program holds at (`three`: with `unk` 0, as a
/// three-valued clause's stage rows carry).
pub(super) fn stage(op: &Op, three: bool, builder: &mut ProgramBuilder) -> Option<Emitted> {
    if !enabled() {
        return None;
    }
    let rule = rule(op, builder).filter(|rule| rule.weight() >= threshold(true))?;
    let term = term(&rule, builder)?;
    let unk = if three { ", 0" } else { "" };
    Some(Emitted {
        statements: vec![format!(
            "SELECT ek >> 32, ek & 4294967295{unk} FROM program_match WHERE term = {term} AND file IN ({FILES})"
        )],
        // It reads no counted table: it always runs.
        effects: vec![Effect::default()],
        root: None,
    })
}

/// `op` with its capture-free node-local sub-rules pushed down: each maximal one worth a program
/// becomes `Op::Program`, and an `all`'s capture-free terms become one program (they bind and
/// read nothing, so their order does not matter).
pub(super) fn push(op: Op, builder: &mut ProgramBuilder) -> Op {
    if !enabled() {
        return op;
    }
    if let Some(rule) = rule(&op, builder)
        && rule.weight() >= threshold(false)
        && let Some(term) = term(&rule, builder)
    {
        return Op::Program(term);
    }
    let op = match op {
        Op::All(items) => return Op::All(grouped(items, builder, |op| op)),
        Op::AllOpen(terms) => {
            return Op::AllOpen(grouped(terms, builder, |term: &mut AllTerm| &mut term.op));
        }
        op => op,
    };
    let mut push = |op: Op| push(op, builder);
    match op {
        Op::Any(items) => Op::Any(items.into_iter().map(push).collect()),
        Op::AnyOpen(branches) => Op::AnyOpen(
            branches
                .into_iter()
                .map(|(op, binds)| (push(op), binds))
                .collect(),
        ),
        Op::Seq(items) => Op::Seq(items.into_iter().map(push).collect()),
        Op::Not(inner) => Op::Not(Box::new(push(*inner))),
        Op::Relation {
            walk,
            sub,
            binds,
            need,
        } => Op::Relation {
            walk,
            sub: Box::new(push(*sub)),
            binds,
            need,
        },
        Op::Search {
            walk,
            sub,
            binds,
            strict,
            need,
        } => Op::Search {
            walk,
            sub: Box::new(push(*sub)),
            binds,
            strict,
            need,
        },
        Op::Count {
            walk,
            sub,
            min,
            max,
            bind,
            need,
        } => Op::Count {
            walk,
            sub: Box::new(push(*sub)),
            min,
            max,
            bind,
            need,
        },
        Op::Every { walk, sub } => Op::Every {
            walk,
            sub: Box::new(push(*sub)),
        },
        Op::Call {
            entry,
            body,
            returns,
        } => Op::Call {
            entry,
            body: Box::new(push(*body)),
            returns,
        },
        Op::Constrained { rule, constraints } => Op::Constrained {
            rule: Box::new(push(*rule)),
            constraints: constraints
                .into_iter()
                .map(|(slot, op)| (slot, push(op)))
                .collect(),
        },
        Op::Unknowable { op, masks, open } => Op::Unknowable {
            op: Box::new(push(*op)),
            masks,
            open,
        },
        Op::NthChild {
            step,
            offset,
            reverse,
            of_rule: Some(OfRule::Frame(rule)),
        } => Op::NthChild {
            step,
            offset,
            reverse,
            of_rule: Some(OfRule::Frame(Box::new(push(*rule)))),
        },
        Op::NthCommit { nth, commit } => Op::NthCommit {
            nth: Box::new(push(*nth)),
            commit: Box::new(push(*commit)),
        },
        Op::Follow {
            rule,
            base,
            out,
            sub,
        } => Op::Follow {
            rule,
            base,
            out,
            sub: Box::new(push(*sub)),
        },
        // A parsed tree's rules run at nodes of no file of the batch.
        op => op,
    }
}

/// An `all`'s terms (`op_of`: a term's operator), its capture-free node-local ones made one
/// program where they are worth one, at the first one's place; the others pushed down.
fn grouped<T>(
    mut terms: Vec<T>,
    builder: &mut ProgramBuilder,
    op_of: impl Fn(&mut T) -> &mut Op,
) -> Vec<T> {
    let rules: Vec<Option<Rule>> = terms
        .iter_mut()
        .map(|term| rule(op_of(term), builder))
        .collect();
    let local: Vec<Rule> = rules.iter().flatten().cloned().collect();
    let group = if local.iter().map(Rule::weight).sum::<usize>() >= threshold(false) {
        let rule = match local.len() {
            1 => local.into_iter().next().expect("one rule"),
            _ => Rule::All(local),
        };
        term(&rule, builder)
    } else {
        None
    };
    let mut placed = false;
    let mut out = Vec::with_capacity(terms.len());
    for (mut term, rule) in terms.into_iter().zip(rules) {
        let op = std::mem::replace(op_of(&mut term), Op::Never);
        *op_of(&mut term) = match (group, rule) {
            (Some(_), Some(_)) if placed => continue,
            (Some(program), Some(_)) => {
                placed = true;
                Op::Program(program)
            }
            _ => push(op, builder),
        };
        out.push(term);
    }
    out
}

/// A rule's program: block `0` emits where the rule holds; each sub-rule a block runs (`TEST`,
/// `COUNT`) is a block laid out after it, so control only goes forward.
pub(super) fn compile(rule: &Rule) -> Option<Program> {
    let mut compiler = Compiler::default();
    compiler.block(rule);
    while let Some((sites, sub)) = compiler.queue.pop_front() {
        let target = u16::try_from(compiler.words.len()).ok()?;
        compiler.block(&sub);
        for site in sites {
            compiler.patch(site, target);
        }
    }
    if compiler.words.len() > usize::from(u16::MAX) {
        return None;
    }
    Program::with_counts(
        compiler.words,
        compiler.strings,
        compiler.predicates,
        vec![0],
        Vec::new(),
        compiler.counts,
    )
    .ok()
}

/// Where a block's sub-rule blocks are referenced from: a `TEST`'s word, or a `COUNT` entry.
enum Site {
    Word(usize),
    Count(usize),
}

struct Compiler {
    words: Vec<u32>,
    strings: Vec<Vec<u8>>,
    predicates: Vec<TextPredicate>,
    counts: Vec<Count>,
    queue: std::collections::VecDeque<(Vec<Site>, Rule)>,
}

impl Default for Compiler {
    fn default() -> Self {
        Self {
            words: Vec::new(),
            // The output label.
            strings: vec![b"match".to_vec()],
            predicates: Vec::new(),
            counts: Vec::new(),
            queue: std::collections::VecDeque::new(),
        }
    }
}

impl Compiler {
    fn push(&mut self, opcode: u8, mode: u8, operand: u16) -> usize {
        self.words.push(word(opcode, mode, operand));
        self.words.len() - 1
    }

    fn patch(&mut self, site: Site, target: u16) {
        match site {
            Site::Word(at) => {
                self.words[at] = (self.words[at] & 0xffff) | (u32::from(target) << 16);
            }
            Site::Count(entry) => self.counts[entry].target = target,
        }
    }

    /// A `TEST` (`NEGATE`d or not) of `rule`'s block, laid out later.
    fn test(&mut self, rule: &Rule, mode: u8) {
        let at = self.push(TEST, mode, 0);
        self.queue.push_back((vec![Site::Word(at)], rule.clone()));
    }

    fn regex(&mut self, regex: &Regex) -> u16 {
        let source = self.strings.len() as u16;
        self.strings.push(regex.as_str().as_bytes().to_vec());
        self.predicates.push(TextPredicate::Regex {
            source,
            compiled: regex.clone(),
        });
        (self.predicates.len() - 1) as u16
    }

    /// Words from which a path at a node emits iff `rule` holds there.
    fn block(&mut self, rule: &Rule) {
        match rule {
            Rule::Kinds(kinds) => match kinds.as_slice() {
                [] => {
                    self.push(STOP, 0, 0);
                }
                [kind] => {
                    self.push(KIND, 0, *kind);
                    self.push(EMIT, 0, 0);
                }
                kinds => self.any(
                    &kinds
                        .iter()
                        .map(|kind| Rule::Kinds(vec![*kind]))
                        .collect::<Vec<_>>(),
                ),
            },
            Rule::Regex(regex) => {
                let predicate = self.regex(regex);
                self.push(TEXT, 0, predicate);
                self.push(EMIT, 0, 0);
            }
            Rule::All(items) => {
                for item in items {
                    match item {
                        Rule::Kinds(kinds) if kinds.len() == 1 => {
                            self.push(KIND, 0, kinds[0]);
                        }
                        Rule::Regex(regex) => {
                            let predicate = self.regex(regex);
                            self.push(TEXT, 0, predicate);
                        }
                        item => self.test(item, 0),
                    }
                }
                self.push(EMIT, 0, 0);
            }
            Rule::Any(items) => self.any(items),
            Rule::Not(inner) => {
                self.test(inner, NEGATE);
                self.push(EMIT, 0, 0);
            }
            Rule::Walk(walk, sub) => self.walk(*walk, sub),
            Rule::Count {
                walk,
                sub,
                min,
                max,
            } => {
                let nav = match walk {
                    Walk::Has(Stop::Neighbor) => CHILD,
                    Walk::Has(_) => DESCEND,
                    Walk::Inside(Stop::Neighbor) => PARENT,
                    _ => ANCESTOR,
                };
                let entry = self.counts.len();
                self.counts.push(Count {
                    nav,
                    mode: 0,
                    target: 0,
                    min: *min as u64,
                    max: max.map(|max| max as u64),
                });
                self.push(COUNT, 0, entry as u16);
                self.push(EMIT, 0, 0);
                self.queue
                    .push_back((vec![Site::Count(entry)], (**sub).clone()));
            }
        }
    }

    /// `FORK`s to each branch's block, laid out one after another.
    fn any(&mut self, items: &[Rule]) {
        let Some((last, first)) = items.split_last() else {
            self.push(STOP, 0, 0);
            return;
        };
        for item in first {
            let fork = self.push(FORK, 0, 0);
            self.block(item);
            let next = self.words.len() as u16;
            self.patch(Site::Word(fork), next);
        }
        self.block(last);
    }

    fn walk(&mut self, walk: Walk, sub: &Rule) {
        match walk {
            Walk::Has(Stop::Neighbor) => {
                self.push(CHILD, 0, 0);
                self.block(sub);
            }
            Walk::Has(_) => {
                self.push(DESCEND, 0, 0);
                self.block(sub);
            }
            Walk::Inside(Stop::Neighbor) => {
                self.push(PARENT, 0, 0);
                self.block(sub);
            }
            Walk::Inside(_) => {
                self.push(ANCESTOR, 0, 0);
                self.block(sub);
            }
            // The field's first child, then (to the end) it and its descendants.
            Walk::FieldHas(field, Stop::Neighbor) => {
                self.push(CHILD, 0, 0);
                self.push(FIELD, FIELD_FIRST, field);
                self.block(sub);
            }
            Walk::FieldHas(field, _) => {
                self.push(CHILD, 0, 0);
                self.push(FIELD, FIELD_FIRST, field);
                let fork = self.push(FORK, 0, 0);
                let here = self.push(TEST, 0, 0);
                self.push(EMIT, 0, 0);
                let next = self.words.len() as u16;
                self.patch(Site::Word(fork), next);
                self.push(DESCEND, 0, 0);
                let below = self.push(TEST, 0, 0);
                self.push(EMIT, 0, 0);
                self.queue
                    .push_back((vec![Site::Word(here), Site::Word(below)], sub.clone()));
            }
            // The parent of the node entered through its field's first child: the node itself
            // (a neighbour), or (to the end) it or any of its ancestors.
            Walk::FieldInside(field, Stop::Neighbor) => {
                self.push(FIELD, FIELD_FIRST, field);
                self.push(PARENT, 0, 0);
                self.block(sub);
            }
            Walk::FieldInside(field, _) => {
                let fork = self.push(FORK, 0, 0);
                self.push(FIELD, FIELD_FIRST, field);
                self.push(PARENT, 0, 0);
                let here = self.push(TEST, 0, 0);
                self.push(EMIT, 0, 0);
                let next = self.words.len() as u16;
                self.patch(Site::Word(fork), next);
                self.push(ANCESTOR, 0, 0);
                self.push(FIELD, FIELD_FIRST, field);
                self.push(PARENT, 0, 0);
                let above = self.push(TEST, 0, 0);
                self.push(EMIT, 0, 0);
                self.queue
                    .push_back((vec![Site::Word(here), Site::Word(above)], sub.clone()));
            }
            Walk::Precedes(_) | Walk::Follows(_) | Walk::Slot(_) | Walk::Related(_) => {
                unreachable!("only walkable walks compile")
            }
        }
    }
}
