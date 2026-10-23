//! The physical plan's node types (agreed with S2, who constructs them), their `EXPLAIN` text
//! and the plan audit (INVARIANTS I5): plans are compiled once per rule, printed stably, and
//! a plan that scans a relation per outer row (an inner scan) is refused.
//!
//! A select runs as a `Pipeline` of FROM-order items: the first item is scanned; every later
//! item must be reached by a key: an index nested loop (hash index built once per table per
//! statement, probed per binding, candidates in table order), an OR probe (union of branch
//! lookups, sorted and deduplicated, the OR re-checked), a lateral `json_each`, or a constant
//! relation. Correlated subqueries probe the same indexes; uncorrelated ones run once per
//! statement.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A full scan of `rel`, with the conjuncts placed at this level.
    Scan { rel: String, pred: Vec<String> },
    /// Probe a hash index on `key_cols` of `rel` with the binding's values.
    IndexNestedLoop { rel: String, key_cols: Vec<u16>, key_affs: Vec<String>, residual: Vec<String> },
    /// Union of one index probe per OR branch, in table order, the OR re-checked.
    OrIndexProbe { rel: String, branches: Vec<Vec<u16>>, residual: Vec<String> },
    /// R14: rows of `rel` whose `range_col` value (under `range_aff`) lies within `bounds` (`lo`,
    /// `hi` or `lo,hi`), by binary search in a sorted index within the `key_cols` hash group (none:
    /// the whole table), in table order; the comparisons are re-checked in `residual`.
    RangeProbe { rel: String, key_cols: Vec<u16>, range_col: u16, range_aff: String, bounds: String, residual: Vec<String> },
    /// `json_each(arg)` over the binding.
    LateralJsonEach { arg: String },
    /// A relation known to hold at most a constant number of rows (literals, `Unit`).
    Constant { rel: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PipelineItem {
    pub item: Item,
    /// `LEFT JOIN`: a binding with no match is null-extended.
    pub left: bool,
    /// Per probe branch (one for an index or range probe, one per branch of an OR probe), per
    /// probed column, the value probed when it is a plain column of an earlier item or of an
    /// enclosing row: `(depth, item, column)` (`None`: an expression or a literal). Read by the
    /// cost model (`Analyzer`), not printed.
    pub refs: Vec<Vec<Option<Ref>>>,
}

/// A plain column of a pipeline's binding: `(depth, item, column)`, depth 0 the pipeline's own
/// items, depth k the k-th enclosing select's.
pub type Ref = (u16, u16, u16);

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Pipeline { items: Vec<PipelineItem> },
    SemiJoin { correlated: bool },
    AntiJoin { correlated: bool },
    Scalar { correlated: bool },
    Group { keys: Vec<String>, aggs: Vec<String> },
    Window { partition: Vec<String>, order: Vec<String> },
    Sort { keys: Vec<String>, limit: Option<String>, offset: Option<String> },
    Union { all: bool },
    Distinct { prefix: u16 },
    /// `closure`: the executor's closure operator (a set fixpoint whose step joins the previous
    /// level to one edge relation); otherwise evaluated level by level.
    Fixpoint { seminaive: bool, set: bool, closure: bool },
    Project { exprs: Vec<String> },
    /// Reserved for rewrite R-tree-interval: descendants of `root_col` as `(id, subtree_end]`.
    IntervalProbe { root_col: u16 },
    /// Reserved: a per-node predicate memoised by its rule digest.
    MemoPredicate { digest: String },
}

/// A physical node: its operator, the digest of the logical relation it implements, and its
/// inputs (subqueries of a pipeline are children after its items' inputs).
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub op: Op,
    pub digest: String,
    pub children: Vec<Node>,
    /// What the cost model needs beyond the printed operator (not printed, not in the digest).
    pub meta: Meta,
}

/// Structure of a node the builder knows and the printed text does not carry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    /// The name later items read this node's rows by: a CTE (`cte<slot>`) or a FROM subquery
    /// (`derived<i>`).
    pub binds: Option<String>,
    /// `Project`: per output column, the binding column it copies (`None`: an expression).
    /// `Group`: per grouping key, likewise.
    pub cols: Vec<Option<Ref>>,
    /// `Project`: per output column, the items of the binding its expression reads (`u16::MAX`:
    /// a subquery or an enclosing row, unknown); empty for a literal.
    pub reads: Vec<Vec<u16>>,
    /// `Fixpoint`: how many children are initial arms (the rest are recursive arms), and the
    /// name the recursive arms read the previous round's rows by (default: `binds`).
    pub init: usize,
    pub delta: Option<String>,
    /// `Fixpoint`: a bound the builder proved for a bag fixpoint, named, with the most rows one
    /// initial row derives (`n`: a counter over values of the content; `d`: a walk along a
    /// forest checked per content, W1).
    pub measure: Option<(String, Mono)>,
    /// `Fixpoint`: `measure` holds only where a per-content check passes (W1's forest walk, M1's
    /// shrinking text, M2's key): without the checks (`Analyzer::unchecked`) it is not used.
    pub measure_checked: bool,
    /// `Fixpoint`: relations and columns the executor checks to be keys on every content before
    /// the fixpoint runs (W1's lookups); the fixpoint's arms may rely on them.
    pub checked_keys: Vec<(String, Vec<u16>)>,
    /// `Fixpoint`: the recursion extends a path value (`path || step`): simple-path enumeration.
    pub paths: bool,
    /// A reachability stage whose edges the rule declares a forest.
    pub forest: bool,
}

fn list(v: &[String]) -> String {
    v.join(", ")
}

fn item_text(i: &PipelineItem) -> String {
    let l = if i.left { "left " } else { "" };
    match &i.item {
        Item::Scan { rel, pred } => format!("{l}scan {rel}{}", if pred.is_empty() { String::new() } else { format!(" where {}", list(pred)) }),
        Item::IndexNestedLoop { rel, key_cols, key_affs, residual } => format!(
            "{l}index {rel} on ({}) affs ({}){}",
            key_cols.iter().map(|c| format!("#{c}")).collect::<Vec<_>>().join(", "),
            list(key_affs),
            if residual.is_empty() { String::new() } else { format!(" where {}", list(residual)) }
        ),
        Item::OrIndexProbe { rel, branches, residual } => format!(
            "{l}or-probe {rel} branches ({}){}",
            branches.iter().map(|b| b.iter().map(|c| format!("#{c}")).collect::<Vec<_>>().join("+")).collect::<Vec<_>>().join(" | "),
            if residual.is_empty() { String::new() } else { format!(" where {}", list(residual)) }
        ),
        Item::RangeProbe { rel, key_cols, range_col, range_aff, bounds, residual } => format!(
            "{l}range {rel} on ({}) #{range_col} aff {range_aff} bounds {bounds}{}",
            key_cols.iter().map(|c| format!("#{c}")).collect::<Vec<_>>().join(", "),
            if residual.is_empty() { String::new() } else { format!(" where {}", list(residual)) }
        ),
        Item::LateralJsonEach { arg } => format!("{l}json_each({arg})"),
        Item::Constant { rel } => format!("{l}constant {rel}"),
    }
}

fn op_text(op: &Op) -> String {
    match op {
        Op::Pipeline { .. } => "Pipeline".into(),
        Op::SemiJoin { correlated } => format!("SemiJoin{}", if *correlated { " correlated" } else { " cached" }),
        Op::AntiJoin { correlated } => format!("AntiJoin{}", if *correlated { " correlated" } else { " cached" }),
        Op::Scalar { correlated } => format!("Scalar{}", if *correlated { " correlated" } else { " cached" }),
        Op::Group { keys, aggs } => format!("Group keys ({}) aggs ({})", list(keys), list(aggs)),
        Op::Window { partition, order } => format!("Window partition ({}) order ({})", list(partition), list(order)),
        Op::Sort { keys, limit, offset } => format!("Sort ({}){}{}", list(keys), limit.as_ref().map(|l| format!(" limit {l}")).unwrap_or_default(), offset.as_ref().map(|o| format!(" offset {o}")).unwrap_or_default()),
        Op::Union { all } => format!("Union{}", if *all { " all" } else { "" }),
        Op::Distinct { prefix } => format!("Distinct prefix {prefix}"),
        Op::Fixpoint { seminaive, set, closure } => format!("Fixpoint{}{}{}", if *seminaive { " semi-naive" } else { "" }, if *set { " set" } else { " bag" }, if *closure { " closure" } else { " levels" }),
        Op::Project { exprs } => format!("Project ({})", list(exprs)),
        Op::IntervalProbe { root_col } => format!("IntervalProbe root #{root_col}"),
        Op::MemoPredicate { digest } => format!("MemoPredicate {digest}"),
    }
}

/// Stable `EXPLAIN` text of a physical plan.
pub fn explain(n: &Node) -> String {
    let mut out = String::new();
    fn walk(n: &Node, indent: usize, out: &mut String) {
        let _ = writeln!(out, "{:indent$}{} [{}]", "", op_text(&n.op), n.digest);
        if let Op::Pipeline { items } = &n.op {
            for (k, i) in items.iter().enumerate() {
                let _ = writeln!(out, "{:w$}{k}: {}", "", item_text(i), w = indent + 2);
            }
        }
        for c in &n.children {
            walk(c, indent + 2, out);
        }
    }
    walk(n, 0, &mut out);
    out
}

/// The plan audit: every refused placement, named. An inner scan is a full `Scan` at a
/// pipeline item after the first (it runs once per binding), or a correlated subquery whose
/// pipeline starts with a full scan (it runs once per outer row).
pub fn audit(n: &Node) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(n: &Node, correlated: bool, path: &str, out: &mut Vec<String>) {
        if let Op::Pipeline { items } = &n.op {
            for (k, i) in items.iter().enumerate() {
                if let Item::Scan { rel, .. } = &i.item {
                    // Mutant 8: the audit admits an inner scan.
                    if (k > 0 || correlated) && !crate::mutant::on(8) {
                        out.push(format!("{path}: inner scan of {rel} at item {k}{}", if correlated && k == 0 { " of a correlated subquery" } else { "" }));
                    }
                }
            }
        }
        // A correlated subquery runs once per outer row down to its first pipeline (through the
        // projections, sorts and groups on top of it).
        let child_corr = matches!(n.op, Op::SemiJoin { correlated: true } | Op::AntiJoin { correlated: true } | Op::Scalar { correlated: true }) || (correlated && !matches!(n.op, Op::Pipeline { .. }));
        for (j, c) in n.children.iter().enumerate() {
            walk(c, child_corr, &format!("{path}/{j}"), out);
        }
    }
    walk(n, false, "plan", &mut out);
    out
}

// ---------------------------------------------------------------------------------------------
// Cost bounds and the plan class (docs/facts/PHASE-3.md § P3b)
//
// Every physical operator declares its work, and the rows it emits, as a function of its inputs'
// cardinalities, in the content's size: `n` nodes and tree depth `d` (I2: `d ≤ n`). Bounds come
// only from declared structure (SCHEMA § 1.2.1's keys and row bounds, given to `Analyzer::declare`
// by the builder for inputs and syntax tables) and from what the plan proves (a probe on a key,
// grouping keys, `DISTINCT`); `B` is the bindings reaching an item:
//
// | operator | work | rows out |
// |---|---|---|
// | scan R, first item | `|R|` | `|R|` |
// | scan R, later item (a cross product) | `B·|R|` | `B·|R|` |
// | index probe on a key of R | build `|R|` once per statement, `B` probes | `≤ B` |
// | index probe whose probed values hold a key of the binding (or of the enclosing row, amortised over a correlated subquery's runs) | `B + |R|` | `≤ |R|` |
// | index probe without a usable key | `B·|R|` | `B·|R|` |
// | OR probe | the sum of its branches | the sum |
// | range probe outside a key group (no declared interval) | `B·|R|` | `B·|R|` |
// | `json_each` over a value (elements ≤ its length ≤ the content) | `B·n` | `B·n` |
// | group, distinct, union, the pipeline's bindings | rows (hashing) | `≤` rows |
// | sort, window | rows `· log n` | rows |
// | correlated subquery | its work per run × the bindings it runs for | — |
// | interval probe over roots keyed by node (I2: descendants are an id interval) | `n·d` | `n·d` |
// | set fixpoint whose recursive arms copy columns | each admitted row through the arms once; rows `≤` initial × what the arms join | |
// | bag fixpoint with a decreasing measure (`Meta::measure`: a counter, or W1's forest walk) | likewise; rows `≤` initial × its reach (`n`; `d`) | |
// | any other fixpoint | unbounded: "fixpoint without a decreasing measure" | |
//
// A plan's work is the sum of its terms; its class is the largest class of a term, and the
// operator of the first such term is named as the one that sets it.

/// One term of a bound, `n^n · d^d · (log n)^log` (constant factors dropped).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mono {
    pub n: u8,
    pub d: u8,
    pub log: u8,
}

impl Mono {
    pub const ONE: Mono = Mono { n: 0, d: 0, log: 0 };
    pub const N: Mono = Mono { n: 1, d: 0, log: 0 };
    pub const ND: Mono = Mono { n: 1, d: 1, log: 0 };
    pub const LOG: Mono = Mono { n: 0, d: 0, log: 1 };

    pub fn mul(self, o: Mono) -> Mono {
        Mono { n: self.n.saturating_add(o.n), d: self.d.saturating_add(o.d), log: self.log.saturating_add(o.log) }
    }

    /// `self ≤ o` on every content (up to a constant): `self`'s surplus depth and log factors
    /// are paid by `o`'s surplus node factors (`d ≤ n`, `log n ≤ n`).
    pub fn within(self, o: Mono) -> bool {
        self.n <= o.n && self.d.saturating_sub(o.d) as u16 + self.log.saturating_sub(o.log) as u16 <= (o.n - self.n) as u16
    }

    pub fn text(self) -> String {
        let part = |k: u8, s: &str| match k {
            0 => None,
            1 => Some(s.to_string()),
            k => Some(format!("{s}^{k}")),
        };
        let parts: Vec<String> = [part(self.n, "n"), part(self.d, "d"), part(self.log, "log n")].into_iter().flatten().collect();
        if parts.is_empty() { "1".into() } else { parts.join("·") }
    }
}

/// The plan classes, in the order they are reported and compared (`O(n log n)` and `O(n·d)`
/// are incomparable in general; a bound is placed in the first class that holds it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    N,
    NLogN,
    ND,
    N2,
    Unbounded,
}

impl Class {
    pub const ALL: [Class; 5] = [Class::N, Class::NLogN, Class::ND, Class::N2, Class::Unbounded];

    /// The monomial a bounded class stands for.
    pub fn mono(self) -> Option<Mono> {
        match self {
            Class::N => Some(Mono::N),
            Class::NLogN => Some(Mono { n: 1, d: 0, log: 1 }),
            Class::ND => Some(Mono::ND),
            Class::N2 => Some(Mono { n: 2, d: 0, log: 0 }),
            Class::Unbounded => None,
        }
    }

    /// The first class holding `m`. `O(n·d)` and `O(n²)` are stated up to a log factor (a sort
    /// over `n·d` rows is `O(n·d)`): a polylog factor is below the exponent the gate's lane fits
    /// (`facts-gate.py` `exponent`), and the class set stops at `O(n²)`.
    pub fn of(m: Mono) -> Class {
        let bare = Mono { log: 0, ..m };
        Class::ALL
            .into_iter()
            .find(|c| match c {
                Class::N | Class::NLogN => c.mono().is_some_and(|b| m.within(b)),
                Class::ND | Class::N2 => c.mono().is_some_and(|b| bare.within(b)),
                Class::Unbounded => false,
            })
            .unwrap_or(Class::Unbounded)
    }

    pub fn name(self) -> &'static str {
        match self {
            Class::N => "O(n)",
            Class::NLogN => "O(n log n)",
            Class::ND => "O(n·d)",
            Class::N2 => "O(n²)",
            Class::Unbounded => "unbounded",
        }
    }

    /// A rule's `class` declaration: the printed name, or `n`, `n log n`, `n·d` (`n*d`), `n^2`.
    pub fn parse(s: &str) -> Option<Class> {
        let s = s.trim();
        let bare = s.strip_prefix("O(").and_then(|x| x.strip_suffix(')')).unwrap_or(s);
        match bare {
            "n" => Some(Class::N),
            "n log n" => Some(Class::NLogN),
            "n·d" | "n*d" => Some(Class::ND),
            "n²" | "n^2" => Some(Class::N2),
            "unbounded" => Some(Class::Unbounded),
            _ => None,
        }
    }
}

impl std::fmt::Display for Class {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A bound in the content's size: a sum of monomials none of which dominates another (empty:
/// zero), or no bound, with the reason.
#[derive(Clone, Debug, PartialEq)]
pub enum Bound {
    Terms(Vec<Mono>),
    Unbounded(String),
}

impl Bound {
    pub fn of(m: Mono) -> Bound {
        Bound::Terms(vec![m])
    }
    pub fn zero() -> Bound {
        Bound::Terms(Vec::new())
    }
    pub fn one() -> Bound {
        Bound::of(Mono::ONE)
    }
    pub fn n() -> Bound {
        Bound::of(Mono::N)
    }

    fn norm(mut v: Vec<Mono>) -> Bound {
        v.sort();
        v.dedup();
        let keep = v.iter().copied().filter(|m| !v.iter().any(|o| o != m && m.within(*o))).collect();
        Bound::Terms(keep)
    }

    pub fn add(&self, o: &Bound) -> Bound {
        match (self, o) {
            (Bound::Unbounded(w), _) | (_, Bound::Unbounded(w)) => Bound::Unbounded(w.clone()),
            (Bound::Terms(a), Bound::Terms(b)) => Bound::norm(a.iter().chain(b).copied().collect()),
        }
    }

    pub fn mul(&self, o: &Bound) -> Bound {
        match (self, o) {
            (Bound::Unbounded(w), _) | (_, Bound::Unbounded(w)) => Bound::Unbounded(w.clone()),
            (Bound::Terms(a), Bound::Terms(b)) => Bound::norm(a.iter().flat_map(|x| b.iter().map(move |y| x.mul(*y))).collect()),
        }
    }

    /// The largest class of a term (zero: `O(n)`, the content is read).
    pub fn class(&self) -> Class {
        match self {
            Bound::Unbounded(_) => Class::Unbounded,
            Bound::Terms(t) => t.iter().map(|m| Class::of(*m)).max().unwrap_or(Class::N),
        }
    }

    pub fn text(&self) -> String {
        match self {
            Bound::Unbounded(w) => format!("unbounded ({w})"),
            Bound::Terms(t) if t.is_empty() => "0".into(),
            Bound::Terms(t) => t.iter().rev().map(|m| m.text()).collect::<Vec<_>>().join(" + "),
        }
    }
}

/// A relation's size: its rows' bound, its candidate keys (column sets no two rows agree on; the
/// empty set: at most one row) and its columns constant over one content (the path: no key of a
/// per-content relation lies within them).
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub rows: Bound,
    pub keys: Vec<BTreeSet<u16>>,
    pub width: Option<usize>,
    pub consts: BTreeSet<u16>,
}

impl Card {
    pub fn new(rows: Bound, keys: &[&[u16]]) -> Card {
        Card { rows, keys: keys.iter().map(|k| k.iter().copied().collect()).collect(), width: None, consts: BTreeSet::new() }
    }

    fn plain(rows: Bound, keys: Vec<BTreeSet<u16>>, width: Option<usize>) -> Card {
        Card { rows, keys, width, consts: BTreeSet::new() }
    }
}

/// One term of a plan's work: its bound and the operator it comes from (`stage: operator`).
#[derive(Clone, Debug, PartialEq)]
pub struct Term {
    pub work: Bound,
    pub what: String,
}

/// A plan's cost: its work, its class and the operator that sets the class.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanCost {
    pub class: Class,
    pub work: Bound,
    pub setter: String,
    pub terms: Vec<Term>,
    /// K1: keys of stored relations the class relies on, checked per content by the executor
    /// (`(relation, columns)`); where one does not hold, the content runs under the algebraic
    /// budget and is counted (`bounds_unproven`).
    pub checked_keys: Vec<(String, Vec<u16>)>,
    /// The class without any per-content check (no K1 key, no W1/M1/M2 measure), and the
    /// operator that sets it: the class of a content where a check fails. Equal to `class` when
    /// declared structure alone proves the class (`proven`). Set by `with_fallback`.
    pub fallback: Class,
    pub fallback_setter: String,
}

impl PlanCost {
    /// The `EXPLAIN` line.
    pub fn line(&self) -> String {
        let assumed = if self.proven() { String::new() } else { format!("; per-content checks assumed, {} where one fails, set by {}", self.fallback, self.fallback_setter) };
        format!("class: {} (work {}; set by {}{assumed})", self.class, self.work.text(), self.setter)
    }

    /// The class holds on every content: no per-content check stands behind it.
    pub fn proven(&self) -> bool {
        self.fallback == self.class
    }

    /// This cost with `unchecked`'s class (the same plan analysed by `Analyzer::unchecked`) as
    /// its fallback.
    pub fn with_fallback(mut self, unchecked: &PlanCost) -> PlanCost {
        self.fallback = unchecked.class;
        self.fallback_setter = unchecked.setter.clone();
        self
    }

    /// The class checked against a rule's declaration: `None` while it holds; otherwise the
    /// refusal, naming the operator. An undeclared plan may not be unbounded; a declared one may
    /// not exceed its declaration.
    pub fn refusal(&self, declared: Option<Class>) -> Option<String> {
        match declared {
            None if self.class == Class::Unbounded => Some(format!("unbounded class, set by {} (declare `class: unbounded` to admit it)", self.setter)),
            Some(d) if self.class > d => Some(format!("class {} exceeds the declared {d}, set by {}", self.class, self.setter)),
            _ => None,
        }
    }
}

type Keys = Vec<BTreeSet<(u16, u16)>>;

/// A select's bindings: their bound, their keys (sets of `(item, column)`), and the most
/// bindings any level of the pipeline holds (what a subquery runs for).
struct State {
    rows: Bound,
    keys: Keys,
    peak: Bound,
    /// `rows` counts the bindings of every run of the innermost correlated subquery together.
    amortized: bool,
    /// Columns constant over one content.
    consts: BTreeSet<(u16, u16)>,
}

/// Composes the cost of one plan from its physical nodes: `declare` the leaves (inputs, syntax
/// tables), then `stage` each stage in order (its rows become a relation later stages read),
/// then `finish`.
#[derive(Default)]
pub struct Analyzer {
    rels: BTreeMap<String, Card>,
    /// CTEs and FROM subqueries in scope, innermost last; `true`: a fixpoint's delta.
    scopes: Vec<BTreeMap<String, (Card, bool)>>,
    /// The keys of the enclosing selects' bindings, innermost last.
    outer: Vec<Keys>,
    /// The runs of the enclosing correlated subqueries, fixpoint rounds and per-row statements.
    mults: Vec<Bound>,
    terms: Vec<Term>,
    at: String,
    /// K1: keys of stored relations the plan relies on and the executor checks per content.
    assumed: Vec<(String, Vec<u16>)>,
    /// No per-content check is assumed: no K1 key, no checked measure (the fallback class).
    unchecked: bool,
}

fn is_marker(exprs: &[String], m: &str) -> bool {
    exprs.len() == 1 && exprs[0].starts_with(m)
}

fn at_item(keys: &[BTreeSet<u16>], k: u16) -> Keys {
    keys.iter().map(|kk| kk.iter().map(|c| (k, *c)).collect()).collect()
}

fn product(a: &Keys, b: &[BTreeSet<u16>], k: u16) -> Keys {
    a.iter().flat_map(|x| at_item(b, k).into_iter().map(move |y| x.union(&y).copied().collect())).collect()
}

impl Analyzer {
    pub fn new() -> Self {
        Self::default()
    }

    /// An analyzer that assumes no per-content check: the class of a content where one fails.
    pub fn unchecked() -> Self {
        Analyzer { unchecked: true, ..Self::default() }
    }

    /// A leaf relation's declared size (names are case-insensitive, as SQL's).
    pub fn declare(&mut self, name: &str, card: Card) {
        self.rels.insert(name.to_ascii_lowercase(), card);
    }

    /// Charges one stage's plan; its rows become relation `name`.
    pub fn stage(&mut self, name: &str, n: &Node) -> Card {
        self.at = name.to_string();
        let c = self.rel(n);
        self.declare(name, c.clone());
        c
    }

    /// Work outside the physical plan (a matcher input, a regex scan).
    pub fn charge_external(&mut self, work: Bound, what: String) {
        self.terms.push(Term { work, what });
    }

    pub fn finish(&self) -> PlanCost {
        let work = self.terms.iter().fold(Bound::zero(), |a, t| a.add(&t.work));
        let class = work.class();
        // An unbounded input names its own cause (the fixpoint or relation it comes from).
        let setter = match &work {
            Bound::Unbounded(w) => w.clone(),
            _ => self.terms.iter().find(|t| t.work.class() == class).map_or_else(|| "reading the content".to_string(), |t| format!("{} ({})", t.what, t.work.text())),
        };
        PlanCost { class, work, setter: setter.clone(), terms: self.terms.clone(), checked_keys: self.assumed.clone(), fallback: class, fallback_setter: setter }
    }

    fn mult(&self, skip_last: bool) -> Bound {
        let n = self.mults.len() - (skip_last && !self.mults.is_empty()) as usize;
        self.mults[..n].iter().fold(Bound::one(), |a, m| a.mul(m))
    }

    fn push(&mut self, work: Bound, what: String) {
        let what = if self.at.is_empty() { what } else { format!("{}: {what}", self.at) };
        self.terms.push(Term { work, what });
    }

    /// Work done once per run of the innermost enclosing loop (subquery run, fixpoint row).
    fn charge(&mut self, work: Bound, what: impl Into<String>) {
        let w = work.mul(&self.mult(false));
        let what = what.into();
        let what = if self.mults.is_empty() { what } else { format!("{what}, per enclosing run") };
        self.push(w, what);
    }

    /// Work done once over all runs of the innermost correlated subquery.
    fn charge_amortized(&mut self, work: Bound, what: String) {
        let w = work.mul(&self.mult(true));
        self.push(w, what);
    }

    /// A hash index: built once per statement, or per round for a fixpoint's delta.
    fn build(&mut self, rel: &str, card: &Card, delta: bool) {
        if delta {
            self.charge(card.rows.clone(), format!("index {rel} (built per round)"));
        } else {
            self.push(card.rows.clone(), format!("index {rel} (built once)"));
        }
    }

    fn lookup(&self, rel: &str) -> (Card, bool) {
        let key = rel.to_ascii_lowercase();
        for s in self.scopes.iter().rev() {
            if let Some((c, delta)) = s.get(&key) {
                return (c.clone(), *delta);
            }
        }
        match self.rels.get(&key) {
            Some(c) => (c.clone(), false),
            None => (Card::plain(Bound::Unbounded(format!("undeclared relation {rel}")), Vec::new(), None), false),
        }
    }

    fn bind(&mut self, name: &str, card: Card, delta: bool) {
        if self.scopes.is_empty() {
            self.scopes.push(BTreeMap::new());
        }
        self.scopes.last_mut().unwrap().insert(name.to_ascii_lowercase(), (card, delta));
    }

    fn rel(&mut self, n: &Node) -> Card {
        match &n.op {
            Op::Project { exprs } if is_marker(exprs, "with") && !n.children.is_empty() => {
                self.scopes.push(BTreeMap::new());
                let (body, ctes) = n.children.split_last().unwrap();
                for c in ctes {
                    let card = self.rel(c);
                    if let Some(b) = &c.meta.binds {
                        self.bind(b, card, false);
                    }
                }
                let out = self.rel(body);
                self.scopes.pop();
                out
            }
            // A stage's deferred column: a statement evaluated per stored row.
            Op::Project { exprs } if is_marker(exprs, "deferred #") && n.children.len() == 2 => {
                let base = self.rel(&n.children[0]);
                self.mults.push(base.rows.clone());
                self.rel(&n.children[1]);
                self.mults.pop();
                base
            }
            Op::Project { .. } if !n.children.is_empty() => {
                let st = self.state(&n.children[0]);
                let pos = |r: &(u16, u16)| n.meta.cols.iter().position(|c| *c == Some((0, r.0, r.1))).map(|p| p as u16);
                let keys = st.keys.iter().filter_map(|k| k.iter().map(pos).collect::<Option<BTreeSet<u16>>>()).collect();
                let consts = n.meta.cols.iter().enumerate().filter(|(_, c)| c.is_some_and(|c| c.0 == 0 && st.consts.contains(&(c.1, c.2)))).map(|(j, _)| j as u16).collect();
                Card { rows: st.rows, keys, width: Some(n.meta.cols.len()), consts }
            }
            Op::Distinct { prefix } if !n.children.is_empty() => {
                let c = self.rel(&n.children[0]);
                self.charge(c.rows.clone(), "distinct");
                let mut keys: Vec<BTreeSet<u16>> = c.keys.into_iter().filter(|k| k.iter().all(|x| x < prefix)).collect();
                keys.push((0..*prefix).collect());
                Card { rows: c.rows, keys, width: c.width, consts: c.consts }
            }
            Op::Sort { .. } if !n.children.is_empty() => {
                let c = self.rel(&n.children[0]);
                self.charge(c.rows.mul(&Bound::of(Mono::LOG)), "sort");
                c
            }
            Op::Union { all } => {
                let parts: Vec<Card> = n.children.iter().map(|c| self.rel(c)).collect();
                let rows = parts.iter().fold(Bound::zero(), |a, c| a.add(&c.rows));
                let width = parts.first().and_then(|c| c.width);
                let keys = if *all {
                    Vec::new()
                } else {
                    self.charge(rows.clone(), "union (distinct)");
                    width.map(|w| vec![(0..w as u16).collect()]).unwrap_or_default()
                };
                let consts = parts.iter().map(|c| c.consts.clone()).reduce(|a, b| a.intersection(&b).copied().collect()).unwrap_or_default();
                Card { rows, keys, width, consts }
            }
            Op::Fixpoint { set, .. } if n.meta.binds.is_some() => self.fixpoint(n, *set),
            Op::Fixpoint { .. } => {
                // A reachability stage: pairs (root, reached) over the stage's edges.
                let parts: Vec<Card> = n.children.iter().map(|c| self.rel(c)).collect();
                let roots = parts.first().map_or_else(Bound::zero, |c| c.rows.clone());
                let edges = parts.get(1).map_or_else(Bound::n, |c| c.rows.clone());
                let rows = roots.mul(&edges);
                self.charge(rows.clone(), format!("reachability{} (roots × edges)", if n.meta.forest { " over a forest" } else { "" }));
                Card::plain(rows, Vec::new(), None)
            }
            Op::IntervalProbe { root_col } => {
                let roots = n.children.first().map(|c| self.rel(c));
                let keyed = roots.as_ref().is_some_and(|r| r.keys.iter().any(|k| k.iter().all(|c| c == root_col)));
                // I2: the descendants of a node are an id interval; over distinct roots, every
                // node is reached from at most its `d` ancestors.
                let rows = if keyed { Bound::of(Mono::ND) } else { roots.map_or_else(Bound::zero, |r| r.rows).mul(&Bound::n()) };
                self.charge(rows.clone(), if keyed { "interval probe (descendants of distinct roots)" } else { "interval probe over roots without a key" });
                Card::plain(rows, Vec::new(), None)
            }
            Op::MemoPredicate { .. } => {
                self.charge(Bound::n(), "memoised predicate");
                Card::new(Bound::n(), &[])
            }
            _ => {
                let st = self.state(n);
                Card::plain(st.rows, Vec::new(), None)
            }
        }
    }

    /// A recursive CTE bound as `meta.binds`: its admitted rows, each run once through the
    /// recursive arms (semi-naive). An arm that only probes (the delta scanned, every later item
    /// an index or OR probe, no subquery) costs one lookup per item per derivation, and its
    /// derivations are what the bound counts (initial × reach, or initial × joined rows); any
    /// other arm is charged per admitted row, as a delta of one row.
    fn fixpoint(&mut self, n: &Node, set: bool) -> Card {
        let name = n.meta.binds.clone().unwrap_or_default();
        let delta = n.meta.delta.clone().unwrap_or_else(|| name.clone());
        let (init, rec) = n.children.split_at(n.meta.init.min(n.children.len()));
        // W1's lookups, checked per content before the fixpoint runs, are keys for its arms; a
        // checked measure holds only with its checks.
        let checked_keys: &[(String, Vec<u16>)] = if self.unchecked { &[] } else { &n.meta.checked_keys };
        let measure = n.meta.measure.as_ref().filter(|_| !(self.unchecked && n.meta.measure_checked));
        let mut checked = BTreeMap::new();
        for (rel, cols) in checked_keys {
            let (mut card, delta) = self.lookup(rel);
            card.keys.push(cols.iter().copied().collect());
            // A stored relation's key is checked per content (K1) where the executor does not
            // check it inline (W1).
            let entry = (rel.to_ascii_lowercase(), cols.clone());
            if self.rels.contains_key(&entry.0) && !self.assumed.contains(&entry) {
                self.assumed.push(entry);
            }
            checked.insert(rel.to_ascii_lowercase(), (card, delta));
        }
        self.scopes.push(checked);
        let mut first = Bound::zero();
        for a in init {
            first = first.add(&self.rel(a).rows);
        }
        // What the recursive arms join besides the fixpoint, and whether an output value is
        // computed from the recursion's own row (a value the next round may grow again); a value
        // computed from joined rows only is drawn from those rows.
        let (mut joined, mut computes) = (Bound::one(), false);
        for a in rec {
            let mut top = a;
            while matches!(top.op, Op::Distinct { .. } | Op::Sort { .. }) && !top.children.is_empty() {
                top = &top.children[0];
            }
            let Some(Node { op: Op::Pipeline { items }, .. }) = top.children.first().filter(|_| matches!(top.op, Op::Project { .. })) else {
                computes = true;
                continue;
            };
            let own = |rel: &str| rel.eq_ignore_ascii_case(&delta) || rel.eq_ignore_ascii_case(&name);
            let mut recursive_items = Vec::new();
            for (k, it) in items.iter().enumerate() {
                let rel = match &it.item {
                    Item::Scan { rel, .. } | Item::IndexNestedLoop { rel, .. } | Item::OrIndexProbe { rel, .. } | Item::RangeProbe { rel, .. } => rel,
                    Item::LateralJsonEach { .. } => {
                        computes = true;
                        continue;
                    }
                    Item::Constant { .. } => continue,
                };
                if own(rel) {
                    recursive_items.push(k as u16);
                } else {
                    joined = joined.mul(&self.lookup(rel).0.rows);
                }
            }
            for (j, c) in top.meta.cols.iter().enumerate() {
                let grows = match c {
                    // A column of the recursion's row is carried in place (its tuple of carried
                    // values is its initial row's), or the bound does not hold.
                    Some((0, i, col)) => recursive_items.contains(i) && *col as usize != j,
                    Some(_) => true,
                    None => top.meta.reads.get(j).is_none_or(|r| r.iter().any(|i| *i == u16::MAX || recursive_items.contains(i))),
                };
                computes |= grows;
            }
        }
        let rows = match measure {
            Some((_, reach)) => first.mul(&Bound::of(*reach)),
            None if set && !computes => first.mul(&joined),
            None if n.meta.paths => Bound::Unbounded(format!("{}: fixpoint {name} is a simple-path enumeration (it extends a path value; bounded only by the number of simple paths)", self.at)),
            None => Bound::Unbounded(format!("{}: fixpoint {name} without a decreasing measure", self.at)),
        };
        let pure = |a: &Node| {
            let mut top = a;
            while matches!(top.op, Op::Distinct { .. } | Op::Sort { .. }) && !top.children.is_empty() {
                top = &top.children[0];
            }
            match top.children.first() {
                Some(p @ Node { op: Op::Pipeline { items }, .. }) if matches!(top.op, Op::Project { .. }) => {
                    p.children.iter().all(|c| c.meta.binds.is_some())
                        && matches!(items.first().map(|i| &i.item), Some(Item::Scan { rel, .. }) if rel.eq_ignore_ascii_case(&delta))
                        && items[1..].iter().all(|i| matches!(i.item, Item::IndexNestedLoop { .. } | Item::OrIndexProbe { .. }) && !i.left)
                }
                _ => false,
            }
        };
        if !rec.is_empty() && rec.iter().all(pure) && !matches!(rows, Bound::Unbounded(_)) {
            for a in rec {
                let mut top = a;
                while !matches!(top.op, Op::Project { .. }) {
                    top = &top.children[0];
                }
                let Op::Pipeline { items } = &top.children[0].op else { continue };
                for it in &items[1..] {
                    if let Item::IndexNestedLoop { rel, .. } | Item::OrIndexProbe { rel, .. } = &it.item {
                        let (card, _) = self.lookup(rel);
                        self.build(rel, &card, false);
                        self.charge(rows.clone(), format!("fixpoint {name}: probe {rel} per derivation"));
                    }
                }
            }
        } else {
            self.scopes.push(BTreeMap::new());
            self.bind(&delta, Card::new(Bound::one(), &[&[]]), true);
            self.mults.push(rows.clone());
            for a in rec {
                self.rel(a);
            }
            self.mults.pop();
            self.scopes.pop();
        }
        let how = match measure {
            Some((m, reach)) => format!("fixpoint {name}, {m} (rows ≤ initial × {})", reach.text()),
            None if set && !computes => format!("fixpoint {name} (set, copies columns: rows ≤ initial × joined)"),
            None => format!("fixpoint {name}"),
        };
        self.charge(rows.clone(), how);
        self.scopes.pop();
        // A stored relation's checked key holds for the rest of the clause: the relation does not
        // change once stored, and the check ran before any later stage.
        for (rel, cols) in checked_keys {
            if let Some(c) = self.rels.get_mut(&rel.to_ascii_lowercase()) {
                c.keys.push(cols.iter().copied().collect());
            }
        }
        let keys = if set { vec![(0..n.meta.cols.len().max(1) as u16).collect()] } else { Vec::new() };
        Card { rows, keys: if set && n.meta.cols.is_empty() { Vec::new() } else { keys }, width: None, consts: BTreeSet::new() }
    }

    fn state(&mut self, n: &Node) -> State {
        match &n.op {
            Op::Pipeline { items } => self.pipeline(n, items),
            Op::Group { .. } if !n.children.is_empty() => {
                let st = self.state(&n.children[0]);
                if st.amortized { self.charge_amortized(st.rows.clone(), "group, over all runs".into()) } else { self.charge(st.rows.clone(), "group") }
                if n.meta.cols.is_empty() {
                    return State { rows: Bound::one(), keys: vec![BTreeSet::new()], peak: st.peak, amortized: false, consts: st.consts };
                }
                let key: Option<BTreeSet<(u16, u16)>> = n.meta.cols.iter().map(|r| r.filter(|r| r.0 == 0).map(|r| (r.1, r.2))).collect();
                State { rows: st.rows, keys: key.into_iter().collect(), peak: st.peak, amortized: st.amortized, consts: st.consts }
            }
            Op::Window { .. } if !n.children.is_empty() => {
                let st = self.state(&n.children[0]);
                let w = st.rows.mul(&Bound::of(Mono::LOG));
                if st.amortized { self.charge_amortized(w, "window (sort), over all runs".into()) } else { self.charge(w, "window (sort)") }
                st
            }
            _ => {
                let c = self.rel(n);
                State { peak: c.rows.clone(), rows: c.rows, keys: Vec::new(), amortized: false, consts: c.consts.iter().map(|x| (0, *x)).collect() }
            }
        }
    }

    fn pipeline(&mut self, n: &Node, items: &[PipelineItem]) -> State {
        self.scopes.push(BTreeMap::new());
        for c in &n.children {
            if let Some(b) = &c.meta.binds {
                let card = self.rel(c);
                self.bind(b, card, false);
            }
        }
        let (mut rows, mut keys, mut peak) = (Bound::one(), vec![BTreeSet::new()], Bound::one());
        let mut consts: BTreeSet<(u16, u16)> = BTreeSet::new();
        // After a correlated probe keyed by the enclosing row, the bindings are counted over all
        // runs at once: the innermost run count is set aside for the rest of the pipeline.
        let mut amortized: Option<Bound> = None;
        for (k, it) in items.iter().enumerate() {
            let k16 = k as u16;
            let (before, before_keys) = (rows.clone(), keys.clone());
            let branch = |b: usize| it.refs.get(b).map(Vec::as_slice).unwrap_or(&[]);
            if let Item::Scan { rel, .. } | Item::IndexNestedLoop { rel, .. } | Item::OrIndexProbe { rel, .. } | Item::RangeProbe { rel, .. } = &it.item {
                consts.extend(self.lookup(rel).0.consts.iter().map(|c| (k16, *c)));
            }
            match &it.item {
                Item::Scan { rel, .. } => {
                    let (card, _) = self.lookup(rel);
                    let w = rows.mul(&card.rows);
                    self.charge(w.clone(), if k == 0 { format!("scan {rel}") } else { format!("cross product: scan {rel} at item {k}") });
                    keys = product(&keys, &card.keys, k16);
                    rows = w;
                }
                Item::IndexNestedLoop { rel, key_cols, .. } => {
                    let (card, delta) = self.lookup(rel);
                    self.build(rel, &card, delta);
                    let amort;
                    (rows, keys, amort) = self.probe(k16, rel, &card, key_cols, branch(0), &before, &before_keys, "index");
                    if amort && amortized.is_none() {
                        amortized = self.mults.last_mut().map(|m| std::mem::replace(m, Bound::one()));
                    }
                }
                Item::OrIndexProbe { rel, branches, .. } => {
                    let (card, delta) = self.lookup(rel);
                    self.build(rel, &card, delta);
                    rows = Bound::zero();
                    for (b, cols) in branches.iter().enumerate() {
                        rows = rows.add(&self.probe(k16, rel, &card, cols, branch(b), &before, &before_keys, "or-probe branch").0);
                    }
                    keys = product(&before_keys, &card.keys, k16);
                }
                Item::RangeProbe { rel, key_cols, range_col, .. } => {
                    let (card, delta) = self.lookup(rel);
                    self.build(rel, &card, delta);
                    let group: BTreeSet<u16> = key_cols.iter().copied().collect();
                    if card.keys.iter().any(|kk| kk.is_subset(&group)) {
                        self.charge(rows.mul(&Bound::of(Mono::LOG)), format!("range {rel} within a key group"));
                    } else {
                        let w = rows.mul(&card.rows);
                        self.charge(w.clone(), format!("range probe {rel} on #{range_col} without a declared interval bound"));
                        keys = product(&keys, &card.keys, k16);
                        rows = w;
                    }
                }
                Item::LateralJsonEach { .. } => {
                    let w = rows.mul(&Bound::n());
                    self.charge(w.clone(), "json_each over a value (elements ≤ its length)");
                    keys = Vec::new();
                    rows = w;
                }
                Item::Constant { .. } => keys = Vec::new(),
            }
            if it.left {
                rows = rows.add(&before);
            }
            peak = peak.add(&rows);
        }
        if amortized.is_some() {
            self.push(rows.mul(&self.mult(false)), "bindings, over all runs".into());
        } else {
            self.charge(rows.clone(), "bindings");
        }
        // Subqueries run per binding of the level that tests them, at most `peak`.
        for c in &n.children {
            let correlated = match c.op {
                Op::SemiJoin { correlated } | Op::AntiJoin { correlated } | Op::Scalar { correlated } => correlated,
                _ => continue,
            };
            let Some(body) = c.children.first() else { continue };
            if correlated {
                self.outer.push(keys.clone());
                self.mults.push(peak.clone());
                self.rel(body);
                self.mults.pop();
                self.outer.pop();
            } else {
                self.rel(body);
            }
        }
        self.scopes.pop();
        if let (Some(m), Some(last)) = (amortized.clone(), self.mults.last_mut()) {
            *last = m;
        }
        State { rows, keys, peak, amortized: amortized.is_some(), consts }
    }

    /// An index probe of `rel` on `cols` with `refs` probed per binding: the bindings it emits
    /// and their keys.
    #[allow(clippy::too_many_arguments)]
    fn probe(&mut self, k: u16, rel: &str, card: &Card, cols: &[u16], refs: &[Option<Ref>], rows: &Bound, keys: &Keys, what: &str) -> (Bound, Keys, bool) {
        let on: BTreeSet<u16> = cols.iter().copied().collect();
        let cols_text = cols.iter().map(|c| format!("#{c}")).collect::<Vec<_>>().join(", ");
        // Mutant 57 (cost model): every probe is taken as a lookup on a key.
        if crate::mutant::on(57) || card.keys.iter().any(|kk| kk.is_subset(&on)) {
            self.charge(rows.clone(), format!("{what} {rel} on a key ({cols_text})"));
            return (rows.clone(), keys.clone(), false);
        }
        // A correlated lookup whose probed values hold a key of the enclosing row: each row of
        // `rel` meets at most one run, so the candidates over all runs are at most `|rel|`.
        let up: BTreeSet<(u16, u16)> = refs.iter().flatten().filter(|r| r.0 == 1).map(|r| (r.1, r.2)).collect();
        if k == 0 && self.outer.last().is_some_and(|o| o.iter().any(|kk| kk.is_subset(&up))) {
            self.charge(rows.clone(), format!("{what} {rel} ({cols_text})"));
            self.charge_amortized(card.rows.clone(), format!("{what} {rel} ({cols_text}) keyed by the enclosing row, over all runs"));
            return (card.rows.clone(), at_item(&card.keys, k), true);
        }
        // The probed values hold a key of the binding: each row of `rel` meets one binding.
        let local: BTreeSet<(u16, u16)> = refs.iter().flatten().filter(|r| r.0 == 0).map(|r| (r.1, r.2)).collect();
        if keys.iter().any(|kk| !kk.is_empty() && kk.is_subset(&local)) {
            self.charge(rows.add(&card.rows), format!("{what} {rel} ({cols_text}) keyed by the binding"));
            return (card.rows.clone(), at_item(&card.keys, k), false);
        }
        // K1: a stored relation probed on plain values, not only on columns constant over the
        // content (the path, which every row shares): the probed columns are taken as its key,
        // checked per content before the plan relies on it (`checked_keys`).
        let stored = self.scopes.iter().all(|s| !s.contains_key(&rel.to_ascii_lowercase())) && self.rels.contains_key(&rel.to_ascii_lowercase());
        if !self.unchecked && stored && !refs.is_empty() && refs.iter().all(Option::is_some) && !on.is_subset(&card.consts) {
            let entry = (rel.to_ascii_lowercase(), cols.to_vec());
            if !self.assumed.contains(&entry) {
                self.assumed.push(entry);
            }
            self.charge(rows.clone(), format!("{what} {rel} on a key checked per content, K1 ({cols_text})"));
            return (rows.clone(), keys.clone(), false);
        }
        if keys.iter().any(|kk| kk.is_subset(&local)) {
            let how = if k == 0 { format!("{what} {rel} ({cols_text}), no key of {rel}") } else { format!("{what} {rel} ({cols_text}) keyed by the binding") };
            self.charge(rows.add(&card.rows), how);
            return (card.rows.clone(), at_item(&card.keys, k), false);
        }
        let w = rows.mul(&card.rows);
        let why = if on.is_subset(&card.consts) { "a join on columns constant over the content (a cross product within it)" } else { "join without a usable key" };
        self.charge(w.clone(), format!("{why}: {what} {rel} on ({cols_text})"));
        (w, product(keys, &card.keys, k), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(op: Op, children: Vec<Node>) -> Node {
        Node { op, digest: "d".into(), children, meta: Meta::default() }
    }

    fn pipeline(items: Vec<Item>) -> Node {
        node(Op::Pipeline { items: items.into_iter().map(|item| PipelineItem { item, left: false, refs: vec![] }).collect() }, vec![])
    }

    #[test]
    fn audit_refuses_inner_scan() {
        let ok = pipeline(vec![Item::Scan { rel: "nodes".into(), pred: vec![] }, Item::IndexNestedLoop { rel: "edges".into(), key_cols: vec![1], key_affs: vec!["None".into()], residual: vec![] }]);
        assert!(audit(&ok).is_empty());
        let bad = pipeline(vec![Item::Scan { rel: "nodes".into(), pred: vec![] }, Item::Scan { rel: "edges".into(), pred: vec!["#1 = #5".into()] }]);
        assert_eq!(audit(&bad).len(), 1);
        let corr = node(Op::SemiJoin { correlated: true }, vec![pipeline(vec![Item::Scan { rel: "calls".into(), pred: vec![] }])]);
        assert_eq!(audit(&corr).len(), 1);
        // Through a projection on top of the subquery's pipeline.
        let deep = node(Op::SemiJoin { correlated: true }, vec![node(Op::Project { exprs: vec![] }, vec![pipeline(vec![Item::Scan { rel: "calls".into(), pred: vec![] }])])]);
        assert_eq!(audit(&deep).len(), 1);
        let probed = node(Op::SemiJoin { correlated: true }, vec![pipeline(vec![Item::IndexNestedLoop { rel: "calls".into(), key_cols: vec![0], key_affs: vec!["None".into()], residual: vec![] }])]);
        assert!(audit(&probed).is_empty());
        let cached = node(Op::SemiJoin { correlated: false }, vec![pipeline(vec![Item::Scan { rel: "calls".into(), pred: vec![] }])]);
        assert!(audit(&cached).is_empty());
    }

    #[test]
    fn audit_explain_is_stable() {
        let n = node(Op::Project { exprs: vec!["#0".into()] }, vec![pipeline(vec![Item::Scan { rel: "a".into(), pred: vec![] }, Item::LateralJsonEach { arg: "#2".into() }])]);
        let n = Node { digest: "p".into(), ..n };
        assert_eq!(explain(&n), explain(&n.clone()));
        assert_eq!(explain(&n), "Project (#0) [p]\n  Pipeline [d]\n    0: scan a\n    1: json_each(#2)\n");
    }

    // Cost model (PHASE-3 P3b).

    fn index(rel: &str, cols: &[u16], refs: &[Option<Ref>]) -> PipelineItem {
        PipelineItem { item: Item::IndexNestedLoop { rel: rel.into(), key_cols: cols.to_vec(), key_affs: vec![], residual: vec![] }, left: false, refs: vec![refs.to_vec()] }
    }

    fn scan(rel: &str) -> PipelineItem {
        PipelineItem { item: Item::Scan { rel: rel.into(), pred: vec![] }, left: false, refs: vec![] }
    }

    /// `Project` over a pipeline of `items` (with `children` as its subqueries), copying `cols`.
    fn select(items: Vec<PipelineItem>, children: Vec<Node>, cols: &[Ref]) -> Node {
        let p = node(Op::Pipeline { items }, children);
        let mut n = node(Op::Project { exprs: vec![] }, vec![p]);
        n.meta.cols = cols.iter().map(|c| Some(*c)).collect();
        n
    }

    /// `nodes` keyed by #0; `calls` and `names` bags of at most n rows whose #0 is the path
    /// (constant over the content).
    fn analyzer() -> Analyzer {
        let mut a = Analyzer::new();
        a.declare("nodes", Card::new(Bound::n(), &[&[0]]));
        let bag = Card { consts: [0].into(), ..Card::new(Bound::n(), &[]) };
        a.declare("calls", bag.clone());
        a.declare("names", bag);
        a
    }

    fn cost(plan: &Node) -> PlanCost {
        let mut a = analyzer();
        a.stage("s", plan);
        a.finish()
    }

    /// The fallback class (`Analyzer::unchecked`): no K1 key and no checked measure is assumed,
    /// so a class that rests on one falls back; a class proven by declared keys does not.
    #[test]
    fn cost_unchecked_assumes_no_per_content_check() {
        let both = |plan: &Node| {
            let mut u = Analyzer::unchecked();
            u.declare("nodes", Card::new(Bound::n(), &[&[0]]));
            let bag = Card { consts: [0].into(), ..Card::new(Bound::n(), &[]) };
            u.declare("calls", bag.clone());
            u.declare("names", bag);
            u.stage("s", plan);
            cost(plan).with_fallback(&u.finish())
        };
        let k1 = both(&select(vec![scan("calls"), index("names", &[1], &[Some((0, 0, 2))])], vec![], &[(0, 0, 0)]));
        assert_eq!((k1.class, k1.fallback), (Class::N, Class::N2), "{}", k1.line());
        assert!(!k1.proven() && k1.line().contains("per-content checks assumed, O(n²) where one fails"), "{}", k1.line());
        let keyed = both(&select(vec![scan("calls"), index("nodes", &[0], &[Some((0, 0, 3))])], vec![], &[(0, 0, 0)]));
        assert!(keyed.proven(), "{}", keyed.line());
        // A bag fixpoint bounded by a checked measure (W1): unbounded without its check.
        let init = select(vec![scan("nodes")], vec![], &[(0, 0, 0), (0, 0, 1)]);
        let step = select(vec![scan("cte0"), index("nodes", &[0], &[Some((0, 0, 1))])], vec![], &[(0, 0, 0), (0, 1, 1)]);
        let mut f = node(Op::Fixpoint { seminaive: true, set: false, closure: false }, vec![init, step]);
        f.meta = Meta { binds: Some("cte0".into()), init: 1, measure: Some(("W1 walk".into(), Mono { n: 0, d: 1, log: 0 })), measure_checked: true, ..Meta::default() };
        let with = node(Op::Project { exprs: vec!["with".into()] }, vec![f, select(vec![scan("cte0")], vec![], &[(0, 0, 0)])]);
        let w1 = both(&with);
        assert_eq!((w1.class, w1.fallback), (Class::ND, Class::Unbounded), "{}", w1.line());
    }

    #[test]
    fn cost_monomials_order_with_depth_below_nodes() {
        assert!(Mono::ND.within(Mono { n: 2, d: 0, log: 0 }));
        assert!(!Mono { n: 2, d: 0, log: 0 }.within(Mono::ND));
        assert!(!Mono { n: 1, d: 0, log: 1 }.within(Mono::ND));
        assert_eq!(Class::of(Mono { n: 0, d: 1, log: 1 }), Class::NLogN);
        assert_eq!(Class::of(Mono { n: 1, d: 1, log: 1 }), Class::ND);
        assert_eq!(Class::of(Mono { n: 2, d: 0, log: 1 }), Class::N2);
        assert_eq!(Class::of(Mono { n: 2, d: 1, log: 0 }), Class::Unbounded);
        assert_eq!(Class::of(Mono { n: 3, d: 0, log: 0 }), Class::Unbounded);
        assert_eq!(Bound::n().add(&Bound::of(Mono::ND)), Bound::of(Mono::ND));
        assert_eq!(Class::parse("O(n·d)"), Some(Class::ND));
        assert_eq!(Class::parse("unbounded"), Some(Class::Unbounded));
    }

    #[test]
    fn cost_key_probe_is_linear() {
        // calls ⋈ nodes on nodes' key.
        let c = cost(&select(vec![scan("calls"), index("nodes", &[0], &[Some((0, 0, 3))])], vec![], &[(0, 0, 0)]));
        assert_eq!(c.class, Class::N, "{}", c.line());
    }

    #[test]
    fn cost_join_without_a_usable_key_is_quadratic() {
        // On the path alone: every row of one content shares it.
        let c = cost(&select(vec![scan("calls"), index("names", &[0], &[Some((0, 0, 0))])], vec![], &[(0, 0, 0)]));
        assert_eq!(c.class, Class::N2, "{}", c.line());
        assert!(c.setter.contains("a join on columns constant over the content (a cross product within it): index names"), "{}", c.setter);
        assert!(c.checked_keys.is_empty());
        // K1: on another column of a stored relation, a key checked per content.
        let c = cost(&select(vec![scan("calls"), index("names", &[1], &[Some((0, 0, 2))])], vec![], &[(0, 0, 0)]));
        assert_eq!(c.class, Class::N, "{}", c.line());
        assert_eq!(c.checked_keys, vec![("names".to_string(), vec![1])]);
        // The probed values a key of the binding (nodes' id): each names row meets one binding.
        let c = cost(&select(vec![scan("nodes"), index("names", &[1], &[Some((0, 0, 0))])], vec![], &[(0, 0, 0)]));
        assert_eq!(c.class, Class::N, "{}", c.line());
        assert!(c.checked_keys.is_empty());
    }

    #[test]
    fn cost_cross_product_and_three_way_products() {
        let c = cost(&select(vec![scan("calls"), scan("names")], vec![], &[]));
        assert_eq!(c.class, Class::N2);
        assert!(c.setter.contains("cross product: scan names at item 1"), "{}", c.setter);
        let c = cost(&select(vec![scan("calls"), scan("names"), index("calls", &[1], &[None])], vec![], &[]));
        assert_eq!(c.class, Class::Unbounded, "{}", c.line());
        assert!(c.refusal(None).is_some());
    }

    #[test]
    fn cost_correlated_subquery_amortised_on_the_enclosing_key() {
        // nodes n WHERE EXISTS (SELECT 1 FROM calls c WHERE c.#1 = n.#0): n's key probed.
        let sub = |r: Ref| node(Op::SemiJoin { correlated: true }, vec![select(vec![index("calls", &[1], &[Some(r)])], vec![], &[])]);
        let c = cost(&select(vec![scan("nodes")], vec![sub((1, 0, 0))], &[(0, 0, 0)]));
        assert_eq!(c.class, Class::N, "{}", c.line());
        // Probed on the path only: every run may meet every call.
        let sub0 = node(Op::SemiJoin { correlated: true }, vec![select(vec![index("calls", &[0], &[Some((1, 0, 4))])], vec![], &[])]);
        let c = cost(&select(vec![scan("nodes")], vec![sub0], &[(0, 0, 0)]));
        assert_eq!(c.class, Class::N2, "{}", c.line());
    }

    #[test]
    fn cost_interval_probe_and_sort() {
        let roots = select(vec![scan("nodes")], vec![], &[(0, 0, 1), (0, 0, 0)]);
        let c = cost(&node(Op::IntervalProbe { root_col: 1 }, vec![roots]));
        assert_eq!(c.class, Class::ND, "{}", c.line());
        let c = cost(&node(Op::Sort { keys: vec![], limit: None, offset: None }, vec![select(vec![scan("calls")], vec![], &[])]));
        assert_eq!(c.class, Class::NLogN, "{}", c.line());
    }

    #[test]
    fn cost_fixpoints_need_a_bound() {
        // WITH RECURSIVE r AS (SELECT … FROM nodes UNION SELECT r.#0, e.#1 FROM r JOIN nodes e ON e.#0 = r.#1)
        let fix = |set: bool, copy: bool, measure: Option<&str>| {
            let init = select(vec![scan("nodes")], vec![], &[(0, 0, 0), (0, 0, 1)]);
            let mut step = select(vec![scan("cte0"), index("nodes", &[0], &[Some((0, 0, 1))])], vec![], &[(0, 0, 0), (0, 1, 1)]);
            if !copy {
                step.meta.cols[1] = None;
            }
            let mut f = node(Op::Fixpoint { seminaive: true, set, closure: false }, vec![init, step]);
            f.meta = Meta { binds: Some("cte0".into()), init: 1, measure: measure.map(|m| (m.to_owned(), Mono::N)), ..Meta::default() };
            let body = select(vec![scan("cte0")], vec![], &[(0, 0, 0)]);
            let mut with = node(Op::Project { exprs: vec!["with".into()] }, vec![f, body]);
            with.meta.cols = vec![];
            cost(&with)
        };
        assert_eq!(fix(true, true, None).class, Class::N2);
        let bag = fix(false, true, None);
        assert_eq!(bag.class, Class::Unbounded);
        assert!(bag.setter.contains("fixpoint cte0 without a decreasing measure"), "{}", bag.setter);
        assert_eq!(fix(true, false, None).class, Class::Unbounded);
        assert_eq!(fix(false, false, Some("counter #1")).class, Class::N2);
    }

    #[test]
    fn cost_declarations_are_upper_bounds() {
        let quadratic = cost(&select(vec![scan("calls"), scan("names")], vec![], &[]));
        assert_eq!(quadratic.refusal(None), None);
        assert_eq!(quadratic.refusal(Some(Class::N2)), None);
        assert!(quadratic.refusal(Some(Class::N)).unwrap().contains("exceeds the declared O(n)"));
        let unbounded = cost(&select(vec![scan("calls"), scan("names"), scan("nodes")], vec![], &[]));
        assert!(unbounded.refusal(None).unwrap().contains("cross product"));
        assert_eq!(unbounded.refusal(Some(Class::Unbounded)), None);
    }
}
