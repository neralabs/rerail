//! A native join's plan, in a section (decision 0569 §4): its steps, its head's terms and what the engine reads by
//! step, encoded once where `join_rule` makes them and read in place by every walk. The run's objects a step reads
//! (a derived relation and its index, a bag's summaries, a compiled matcher, a node read's binding, a fused row
//! program) are the section's handles (`Bound`), held with it.
//!
//! The owned types here are only written: `join_rule` makes them, and the edits its analyses ask for (`Edit`) are
//! made on them before the section is encoded again. Everything else reads the views (`ArchivedStep`, …), which
//! `setfix` names `Step`, `Src` and `Plan`.

use rerail_blocks::vector::Vector;
use rerail_section::{Archive, Bound, Decode, FlatDecode, Names, Section};
use std::sync::Arc;

use super::{Csr, InRel, NodeTexts, Ranged, Term, rowcode, scalars};
use crate::code_rule_dsl::logic::analysis::Decision;
use crate::code_rule_dsl::logic::analysis::scalars::Kind as ScalarKind;
use crate::code_rule_dsl::logic::strings::{Matcher, functions::Kind};

/// A step's value: a slot's or a constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Slot(usize),
    Const(i64),
}

rerail_section::plain_archive!(Key, Term, Ranged);

/// Where an atom's rows come from: the component's relation's delta or whole, or a derived relation read through
/// one of its indexes.
#[derive(Archive)]
pub enum Src {
    Delta(#[archive(flat)] usize),
    Full(#[archive(flat)] usize),
    FullMulti(#[archive(flat)] usize, #[archive(flat)] Vector<usize>),
    In(Bound<InRel>, Bound<Csr>),
}

/// One step of a join's walk. Each variant's meaning is the view's (`ArchivedStep`, `setfix::Step`).
#[derive(Archive)]
pub enum Step {
    Unused {
        proof: Decision,
    },
    /// Key columns (bound), columns that bind a slot, and column pairs that must be equal (a variable repeated in
    /// the atom, first seen here).
    Atom {
        src: Src,
        neg: bool,
        #[archive(flat)]
        keys: Vec<(usize, Key)>,
        #[archive(flat)]
        binds: Vec<(usize, usize)>,
        #[archive(flat)]
        tests: Vec<(usize, usize)>,
    },
    Cat {
        #[archive(flat)]
        out: usize,
        a: Key,
        b: Key,
        bind: bool,
    },
    /// `whole` split as `@dot` makes it into `a` and `b` (`""` and the whole, or at each "." with a part before it);
    /// `Some` slots bind, `None` parts (`ka`/`kb`) are tested.
    Split {
        whole: Key,
        #[archive(flat)]
        a: Option<usize>,
        #[archive(flat)]
        b: Option<usize>,
        ka: Key,
        kb: Key,
    },
    Eq {
        a: Key,
        b: Key,
    },
    Lt {
        a: Key,
        b: Key,
        or_eq: bool,
    },
    /// Direct non-null scalar. The shared logical proof selects a bag; the source owns versioned summaries.
    /// `bound`: a bounded min or max (decision 0511), the bound and whether it is strict: only the values on its
    /// side (under it for a max, over it for a min) count, and no summary of the group alone holds them.
    Scalar {
        src: Src,
        #[archive(flat)]
        keys: Vec<(usize, Key)>,
        #[archive(flat)]
        tests: Vec<(usize, usize)>,
        #[archive(flat)]
        col: Option<usize>,
        kind: ScalarKind,
        #[archive(flat)]
        out: usize,
        bind: bool,
        bag: Option<Bound<scalars::Bag>>,
        #[archive(flat)]
        bound: Option<(Key, bool)>,
    },
    Let {
        #[archive(flat)]
        a: usize,
        b: Key,
    },
    Ne {
        a: Key,
        b: Key,
    },
    /// `out = @f(args)` for a `strings::functions` function: text arguments are symbols' names, a text result is
    /// interned; no value (NULL), no binding.
    /// `texts`: for a node read (`syntax::NODE_READS`), the run's binding of the long-text nodes it reads through
    /// (`functions::node_read`).
    /// `kept`: the id its text is kept by in the store's rolling cache (decision 0478 step 2e), by its symbol, when it
    /// is a text function of a symbol whose other arguments are constants (as the device keeps it).
    Func {
        name: &'static str,
        kinds: &'static [Kind],
        args: Vec<Key>,
        #[archive(flat)]
        out: usize,
        bind: bool,
        texts: Option<Bound<NodeTexts>>,
        #[archive(flat)]
        kept: Option<u32>,
    },
    /// `@doc(a) = @doc(b)` (`eq`) or `!=`: the nodes' documents, `d` of `d << 32 | index`, compared.
    Doc {
        a: Key,
        b: Key,
        eq: bool,
    },
    /// `out = @doc(n)`: node n's document, bound or tested.
    DocOf {
        #[archive(flat)]
        out: usize,
        n: Key,
        bind: bool,
    },
    /// `@op(s, p) = want`, a text test (`strings::TESTS`) of symbol `s`'s name: by `matcher` when the pattern is a
    /// constant that compiles (once, with the spec), else by `strings::test` of symbol `p`'s name.
    /// `test`: its id for its verdicts in the store's rolling cache (decision 0478 step 2d), when its patterns are
    /// constants.
    Test {
        op: &'static str,
        s: Key,
        p: Key,
        matcher: Option<Bound<Matcher>>,
        #[archive(flat)]
        test: Option<u32>,
        want: bool,
    },
    /// `a` is (`want`) or is not one of the constants `set` (`@equals(x, {…})`, `@in(x, {…})` once lowered).
    In {
        a: Key,
        set: Vector<i64>,
        want: bool,
    },
    /// `out = join x by (k…) sep s : { atom }`: the matching rows' distinct (k…, x), ordered by k… then x (numbers by
    /// value, symbols by their names' bytes), their x joined by `sep`'s name, as SQL's `rr_join` makes it; "" for none.
    /// `x` and each `k` are columns, each with whether it holds symbols.
    StrJoin {
        src: Src,
        #[archive(flat)]
        keys: Vec<(usize, Key)>,
        #[archive(flat)]
        tests: Vec<(usize, usize)>,
        #[archive(flat)]
        x: (usize, bool),
        #[archive(flat)]
        by: Vec<(usize, bool)>,
        sep: i64,
        #[archive(flat)]
        out: usize,
        bind: bool,
    },
}

/// A composite index a join's steps read (relation, key columns), which the component maintains.
#[derive(Archive)]
pub struct Multi {
    #[archive(flat)]
    pub rel: usize,
    #[archive(flat)]
    pub cols: Vector<usize>,
}

/// A native join's plan. `multi`: the composite indexes its steps read. `probes`: per step, the texts it makes that
/// are only looked up or not read (`setfix::probes`). `fused`: by step, an atom fused with the filters after it into
/// a row program (`rowcode`). `spec`: the digest of the spec it was read from, which a level's batch keeps its pairs
/// by (`level`). `from`: per step, the plan step it was made for (`JoinSpec::from`; `plan_steps` for a step of the
/// head's computed terms), and `plan_steps` the plan's steps: what the engine reports by step is by plan step.
/// `ranges`: per step, a range of the column it binds that later comparisons bound (`setfix::ranges`).
#[derive(Archive)]
pub struct Plan {
    pub steps: Vec<Step>,
    #[archive(flat)]
    pub out: Vec<Term>,
    #[archive(flat)]
    pub nslots: usize,
    pub multi: Vec<Multi>,
    pub probes: Vec<u8>,
    pub fused: Vec<Option<Bound<rowcode::Fused>>>,
    pub spec: u64,
    #[archive(flat)]
    pub from: Vector<usize>,
    #[archive(flat)]
    pub plan_steps: usize,
    #[archive(flat)]
    pub ranges: Vec<Option<Ranged>>,
}

/// An edit an analysis of the plan's view asks for, made on the owned steps before they are encoded again.
pub enum Edit {
    /// The step's evaluation is elided (`liveness::sums`).
    Unused(usize, Decision),
    /// The scalar step's summaries are its bag's (`scalars::plan`).
    Bag(usize, Arc<scalars::Bag>),
    /// The atom reads its derived relation through this index (`index_then`).
    Index(usize, Arc<Csr>),
}

impl Plan {
    /// Makes the edits on its steps.
    pub fn edit(&mut self, edits: impl IntoIterator<Item = Edit>) {
        for e in edits {
            match e {
                Edit::Unused(i, proof) => self.steps[i] = Step::Unused { proof },
                Edit::Bag(i, b) => {
                    if let Step::Scalar { bag, .. } = &mut self.steps[i] {
                        *bag = Some(Bound(b));
                    }
                }
                Edit::Index(i, ix) => {
                    if let Step::Atom { src: Src::In(_, at), .. } = &mut self.steps[i] {
                        *at = Bound(ix);
                    }
                }
            }
        }
    }

    pub fn encode(&self) -> Section<Plan> {
        Section::of(self)
    }
}

/// A function's static name and kinds (`strings::functions`), or a text test's (`strings::TESTS`), by its text.
fn function(name: &str) -> (&'static str, &'static [Kind]) {
    let (n, k, _) = crate::code_rule_dsl::logic::strings::functions::signature(name).expect("a step's function");
    (n, k)
}

fn test(op: &str) -> &'static str {
    crate::code_rule_dsl::logic::strings::TESTS.iter().find(|&&t| t == op).copied().expect("a step's text test")
}

impl Decode for Src {
    fn decode(a: &ArchivedSrc, n: &Names<'_>) -> Src {
        match a {
            ArchivedSrc::Delta(r) => Src::Delta(*r),
            ArchivedSrc::Full(r) => Src::Full(*r),
            ArchivedSrc::FullMulti(r, cols) => Src::FullMulti(*r, Vector::from(cols.as_slice())),
            ArchivedSrc::In(rel, ix) => Src::In(Bound(n.get(rel)), Bound(n.get(ix))),
        }
    }
}

impl Decode for Step {
    /// A step made again from its view: its function's and test's names are the engine's own (`function`, `test`).
    fn decode(a: &ArchivedStep, n: &Names<'_>) -> Step {
        match a {
            ArchivedStep::Unused { proof } => Step::Unused { proof: Decision::decode(proof, n) },
            ArchivedStep::Atom { src, neg, keys, binds, tests } => {
                Step::Atom { src: Src::decode(src, n), neg: *neg, keys: keys.to_vec(), binds: binds.to_vec(), tests: tests.to_vec() }
            }
            ArchivedStep::Cat { out, a, b, bind } => Step::Cat { out: *out, a: *a, b: *b, bind: *bind },
            ArchivedStep::Split { whole, a, b, ka, kb } => Step::Split { whole: *whole, a: *a, b: *b, ka: *ka, kb: *kb },
            ArchivedStep::Eq { a, b } => Step::Eq { a: *a, b: *b },
            ArchivedStep::Lt { a, b, or_eq } => Step::Lt { a: *a, b: *b, or_eq: *or_eq },
            ArchivedStep::Scalar { src, keys, tests, col, kind, out, bind, bag, bound: limit } => Step::Scalar {
                src: Src::decode(src, n),
                keys: keys.to_vec(),
                tests: tests.to_vec(),
                col: *col,
                kind: *kind,
                out: *out,
                bind: *bind,
                bag: bag.as_ref().map(|h| Bound(n.get(h))),
                bound: *limit,
            },
            ArchivedStep::Let { a, b } => Step::Let { a: *a, b: *b },
            ArchivedStep::Ne { a, b } => Step::Ne { a: *a, b: *b },
            ArchivedStep::Func { name, args, out, bind, texts, kept, .. } => {
                let (name, kinds) = function(name);
                Step::Func { name, kinds, args: args.to_vec(), out: *out, bind: *bind, texts: texts.as_ref().map(|h| Bound(n.get(h))), kept: *kept }
            }
            ArchivedStep::Doc { a, b, eq } => Step::Doc { a: *a, b: *b, eq: *eq },
            ArchivedStep::DocOf { out, n: node, bind } => Step::DocOf { out: *out, n: *node, bind: *bind },
            ArchivedStep::Test { op, s, p, matcher, test: id, want } => {
                Step::Test { op: test(op), s: *s, p: *p, matcher: matcher.as_ref().map(|h| Bound(n.get(h))), test: *id, want: *want }
            }
            ArchivedStep::In { a, set, want } => Step::In { a: *a, set: Vector::from(set.as_slice()), want: *want },
            ArchivedStep::StrJoin { src, keys, tests, x, by, sep, out, bind } => Step::StrJoin {
                src: Src::decode(src, n),
                keys: keys.to_vec(),
                tests: tests.to_vec(),
                x: *x,
                by: by.to_vec(),
                sep: *sep,
                out: *out,
                bind: *bind,
            },
        }
    }
}

impl Plan {
    /// A plan made again from its section, to be edited into another (`closure`, a test's sources).
    pub fn decoded(p: &Section<Plan>) -> Plan {
        let n = p.names();
        Plan {
            steps: p.steps.iter().map(|s| Step::decode(s, &n)).collect(),
            out: p.out.to_vec(),
            nslots: p.nslots,
            multi: p.multi.iter().map(|m| Multi { rel: m.rel, cols: FlatDecode::decode(&m.cols) }).collect(),
            probes: p.probes.to_vec(),
            fused: p.fused.iter().map(|f| f.as_ref().map(|h| Bound(n.get(h)))).collect(),
            spec: p.spec,
            from: FlatDecode::decode(&p.from),
            plan_steps: p.plan_steps,
            ranges: p.ranges.to_vec(),
        }
    }
}
