//! Moved from `front.rs` by the crate split (tools/cratify/moves.tsv).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use crate::code_rule_dsl::logic::front::name::Name;

#[derive(Clone, Debug, PartialEq, Hash)]
pub enum Term {
    Var(String),
    Wild,
    Int(i64),
    Str(String),
    Top,
    Call(String, Vec<Term>),
}

impl Term {
    pub fn vars(&self, out: &mut BTreeSet<String>) {
        match self {
            Term::Var(v) => {
                out.insert(v.clone());
            }
            Term::Call(_, args) => args.iter().for_each(|a| a.vars(out)),
            _ => {}
        }
    }

    /// `@cat` over variables only: the one builtin that can run backwards, splitting a bound result.
    pub fn is_cat(&self) -> bool {
        matches!(self, Term::Call(f, args) if f == "cat" && args.iter().all(|a| matches!(a, Term::Var(_))))
    }
}

#[derive(Clone, Debug, PartialEq, Hash)]
pub struct Atom {
    pub rel: Name,
    pub args: Vec<Term>,
    pub neg: bool,
    /// A negation held until its relation's `done_` gate passes (magic sets, row 4): not a negative edge.
    pub gated: bool,
}

impl Atom {
    pub fn vars(&self) -> BTreeSet<String> {
        // A head's `@dot(p, s)` names p and s too.
        let mut out = BTreeSet::new();
        for a in &self.args {
            a.vars(&mut out);
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AggOp {
    Count,
    Min,
    Max,
    /// `sum x : { … }`: the sum of `x` over the body's rows, 0 when there are none.
    Sum,
    /// `product x : {…}`: the product of `x` over the rows, 1 over no rows, saturating at i64's range (a count of
    /// combinations, as pytest's stacked parametrize marks multiply).
    Product,
    /// `join x by (k, …) sep "s" : { … }`: the distinct `(k, …, x)` rows' `x` texts ordered by `k, …` then `x`
    /// (symbols by code point, numbers by value), separated by `sep`; `""` when there are none.
    Join { by: Vec<String>, sep: String },
}

#[derive(Clone, Debug, PartialEq, Hash)]
pub enum Literal {
    Atom(Atom),
    Cmp { l: Term, op: &'static str, r: Term },
    /// `$var = count { atom }`, or `$var = min $over : { atom }` (`max`, `sum` and `join` likewise).
    Agg { var: String, op: AggOp, over: Option<String>, atom: Atom },
    /// `nearest($n, $a, kindset)`: `$a` is `$n`'s nearest strict ancestor of a kind in the set.
    Nearest { n: String, a: String, kindset: String },
    /// `ast($document, $node, { matcher }, NAME: $v, …)`: the matcher's text, lowered to a transducer program (§3),
    /// and the metavariables it exports, each `(NAME, v)` binding `$v` to what `$NAME` captured.
    Ast { document: String, node: String, matcher: String, captures: Vec<(String, String)> },
}

impl Literal {
    pub fn vars(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        match self {
            Literal::Atom(a) => out = a.vars(),
            Literal::Cmp { l, r, .. } => {
                l.vars(&mut out);
                r.vars(&mut out);
            }
            Literal::Agg { var, over, atom, .. } => {
                out = atom.vars();
                out.insert(var.clone());
                out.extend(over.clone());
            }
            Literal::Nearest { n, a, .. } => out.extend([n.clone(), a.clone()]),
            Literal::Ast { document, node, captures, .. } => {
                out.extend([document.clone(), node.clone()]);
                out.extend(captures.iter().map(|(_, v)| v.clone()));
            }
        }
        out
    }

    /// The atom a literal reads a relation through: itself, or an aggregate's.
    pub fn atom(&self) -> Option<&Atom> {
        match self {
            Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a),
            _ => None,
        }
    }
}

/// A comparison's variables once it runs, or `None` if it cannot run yet: `$x = e` assigns an unbound `$x` once
/// `e`'s variables are bound; `$m = @cat($q, $x)` with `$m` bound splits it, binding `$q` and `$x` where unbound;
/// otherwise every variable must be bound. T's `cmp_binds`.
pub fn cmp_binds(l: &Term, op: &str, r: &Term, bound: &BTreeSet<String>) -> Option<BTreeSet<String>> {
    let mut rv = BTreeSet::new();
    r.vars(&mut rv);
    if op == "=" && let Term::Var(x) = l && !bound.contains(x) {
        return rv.is_subset(bound).then(|| BTreeSet::from([x.clone()]));
    }
    let mut all = rv.clone();
    l.vars(&mut all);
    if all.is_subset(bound) {
        return Some(BTreeSet::new());
    }
    if op == "=" && matches!(l, Term::Var(_)) && r.is_cat() {
        return Some(rv.difference(bound).cloned().collect());
    }
    None
}

impl fmt::Display for Term {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Term::Var(v) => write!(f, "${v}"),
            Term::Wild => f.write_str("_"),
            Term::Int(n) => write!(f, "{n}"),
            Term::Str(s) => f.write_str(&serde_json::to_string(s).expect("a string serializes")),
            Term::Top => f.write_str("top"),
            Term::Call(name, args) => write!(f, "@{name}({})", join(args)),
        }
    }
}

impl fmt::Display for Atom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}({})", if self.neg { "!" } else { "" }, self.rel, join(&self.args))
    }
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Atom(a) => write!(f, "{a}"),
            Literal::Cmp { l, op, r } => write!(f, "{l} {op} {r}"),
            Literal::Agg { var, op: AggOp::Count, atom, .. } => write!(f, "${var} = count {{ {atom} }}"),
            Literal::Agg { var, op: AggOp::Join { by, sep }, over, atom } => {
                let by: Vec<String> = by.iter().map(|b| format!("${b}")).collect();
                let sep = serde_json::to_string(sep).expect("a string serializes");
                write!(f, "${var} = join ${} by ({}) sep {sep} : {{ {atom} }}", over.as_deref().unwrap_or_default(), by.join(", "))
            }
            Literal::Agg { var, op, over, atom } => {
                let op = match op {
                    AggOp::Min => "min",
                    AggOp::Sum => "sum",
                    AggOp::Product => "product",
                    _ => "max",
                };
                write!(f, "${var} = {op} ${} : {{ {atom} }}", over.as_deref().unwrap_or_default())
            }
            Literal::Nearest { n, a, kindset } => write!(f, "nearest(${n}, ${a}, {kindset})"),
            Literal::Ast { document, node, matcher, captures } => {
                write!(f, "ast(${document}, ${node}, {matcher}")?;
                captures.iter().try_for_each(|(name, v)| write!(f, ", {name}: ${v}"))?;
                f.write_str(")")
            }
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} :- {}", self.head, join(&self.body))
    }
}

pub fn join<T: fmt::Display>(items: &[T]) -> String {
    items.iter().map(T::to_string).collect::<Vec<_>>().join(", ")
}

#[derive(Clone, Debug, PartialEq, Hash)]
pub struct Rule {
    pub head: Atom,
    pub body: Vec<Literal>,
}

/// A relation's lattice column: the last, after `keys` key columns.
#[derive(Clone, Debug, PartialEq)]
pub struct LatticeDecl {
    pub keys: usize,
    /// The value column's type, which names the lattice (`flat`, `min`, `max`) unless `merge` does.
    pub ty: String,
    pub merge: Option<String>,
    pub widen: Option<u32>,
}

impl LatticeDecl {
    /// The lattice's name for `logic::native` (`declare`d when `merge` names a native join).
    pub fn lattice(&self) -> &str {
        self.merge.as_deref().unwrap_or(&self.ty)
    }

    pub fn bounded(&self) -> bool {
        self.widen.is_some() || (self.merge.is_none() && self.ty == "flat")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Str(String),
}

#[derive(Clone, Debug, Default)]
pub struct Program {
    /// Each relation's columns: (name, type).
    pub decls: BTreeMap<String, Vec<(String, String)>>,
    /// Logically nullary relations encoded with one physical unit column.
    pub nullary: BTreeSet<String>,
    pub lattice: BTreeMap<String, LatticeDecl>,
    pub facts: BTreeMap<String, Vec<Vec<Value>>>,
    pub rules: Vec<Rule>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub kindsets: BTreeMap<String, Vec<String>>,
    /// `.demand` relations, or `*` for every derived one.
    pub demand: Vec<String>,
    /// Set by `magic`: demanded relations computed in full, and why.
    pub demand_dropped: Vec<(String, String)>,
    /// Set by `magic`: the extras its magic rules kept.
    pub magic_extras: Option<usize>,
    pub magic_report: Option<Report>,
    /// Gated negations (`magic::gate_negations`), by their `done_` relation, in the order they were made.
    pub gates: Vec<(String, Gate)>,
    /// `.policy { … }`: lint-rule documents, each without its matchers, which are the rules' `ast` atoms (`yaml`).
    pub policies: Vec<serde_json::Value>,
    /// `.rr` setting directives (`.mode block`, `.scope include "…"`, …), by name, as written (`logic::rr`).
    pub settings: Vec<(String, String)>,
    /// Relations the evaluator makes from their inputs' rows rather than from rules (`dfs_first`), by name. Each
    /// has one rule over its inputs, which orders it after them; its rows are the native operator's.
    pub natives: BTreeMap<String, Native>,
    /// The regexes `match_all` and `match` run over `source` or `src_line` spans (`core.rr`: their captures are
    /// spans), and over which: the batch makes `rx_span(p, c, j, g, m)` for them, each match's groups as spans.
    pub rx_spans: std::collections::BTreeSet<(SpanIn, String)>,
    pub ast_patterns: std::collections::BTreeSet<String>,
    pub doc_scopes: Option<Vec<(Vec<String>, Vec<String>)>>,
    pub answers: BTreeMap<String, Vec<String>>,
    /// `.scope text`'s globs: a document matching one that has no grammar of the run (`.md`, `.rst`) is read as text
    /// only, its `doc`, `source` and `src_line` and no tree, when a scope selects it. Every program's, in a combined
    /// one.
    pub text_globs: Vec<String>,
    /// `.decl r(…) inline`, `no_inline`, `magic`, `no_magic` (Soufflé's qualifiers, decision 0099): the program's own
    /// choice of what a relation keeps, by relation, overriding the planner's (`plan::keeps`).
    pub qualifiers: BTreeMap<String, BTreeSet<String>>,
    /// The columns declared `tape` (decision 0103 §3), by relation: stored as `sym`, checked before lowering
    /// (`rr::tape_type`) and printed back as `tape`.
    pub tapes: BTreeMap<String, BTreeSet<usize>>,
    /// A session's maintained strata, each with the rules that derive it again instead (decision 0090 §4,
    /// `maintain::rewrite`): the run chooses per stratum, once its input deltas are known (`plan::run`).
    pub again: Vec<Again>,
    /// Relations run as one component though no rule makes them one (decision 0101 §3, `maintain::together`): a
    /// maintained stratum's pieces at one level, ordered inside it by unit gates (`gates`), each in one set.
    pub together: Vec<BTreeSet<String>>,
}

/// A maintained stratum's other form (decision 0090 §4). Both make its relations and their deltas (`r__minus`,
/// `r__plus`), which are all that other strata read of it, so each stratum chooses alone.
#[derive(Clone, Debug, Default)]
pub struct Again {
    /// The stratum's own relations (one fixpoint when derived again).
    pub scc: BTreeSet<String>,
    /// Every relation the maintained form makes for it: its relations, their pieces (`__del`, `__ins`, …) and deltas.
    pub owned: BTreeSet<String>,
    /// Its rules over the current inputs, then its deltas against its old rows.
    pub rules: Vec<Rule>,
    pub deltas: Vec<Rule>,
    /// The input deltas its maintained form reads, and its relations' old rows: the sizes that choose.
    pub reads: Vec<String>,
    pub olds: Vec<String>,
}

/// The `.decl` qualifiers that choose what a relation keeps (decision 0099), as Soufflé spells them.
pub const QUALIFIERS: &[&str] = &["inline", "no_inline", "magic", "no_magic", "reach", "no_reach"];

impl Program {
    /// Whether `.decl rel(…)` carries the qualifier `word` (one of `QUALIFIERS`).
    pub fn qualified(&self, rel: &str, word: &str) -> bool {
        self.qualifiers.get(rel).is_some_and(|q| q.contains(word))
    }
}

/// A `.scope` entry taken from a policy option (`.scope dirs option("applicationRoots")`), as `doc_scopes` holds it
/// until the run's options are known: `rr-option:<form>:<name>`, `form` one of `dirs`, `include`, `exclude`. As a glob
/// it holds no path; `resolve_scopes` adds the option's values beside it.
pub const OPTION_GLOB: &str = "rr-option:";

/// `.global r, …`: relations of a policy that a combined run shares, as it shares a library's, when every policy that
/// has one defines it alike (decision 0091). `.global library r, …` names an included file's relations (`rr::read_file`
/// writes it): shared where alike, else each policy's own.
pub const GLOBAL: &str = "global";

/// `.global`'s first word for an included file's relations.
pub const LIBRARY: &str = "library";

/// A program's `.global` relations: (named by the policy, an included file's).
pub fn globals(settings: &[(String, String)]) -> (BTreeSet<String>, BTreeSet<String>) {
    let (mut named, mut library) = (BTreeSet::new(), BTreeSet::new());
    for (_, text) in settings.iter().filter(|(k, _)| k == GLOBAL) {
        let (to, rest) = match text.trim().strip_prefix(LIBRARY).filter(|r| r.starts_with(char::is_whitespace)) {
            Some(rest) => (&mut library, rest),
            None => (&mut named, text.as_str()),
        };
        to.extend(rest.split(|c: char| c == ',' || c.is_whitespace()).filter(|w| !w.is_empty()).map(str::to_owned));
    }
    (named, library)
}

/// The documents out of a policy's scope (`.scope`'s `dirs`, `include`, `exclude`), as lowering derives them:
/// `rr_scope_not(d)`. In a combined run, the guards on the policy's reads negate it (decision 0091).
pub const SCOPE_NOT: &str = "rr_scope_not";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SpanIn {
    Source,
    Line,
    Comment,
    /// Nodes of this kind.
    Kind(String),
    /// The spans group `.1` of pattern `.0` captured.
    Capture(String, String),
}

/// A relation made natively from relations of the program (`Program::natives`).
#[derive(Clone, Debug, PartialEq)]
pub enum Native {
    /// `dfs_first(r, hit, first, edge, target)`; with `roots`, `dfs_first_memo` (roots in the order of their
    /// second column, one memo shared).
    DfsFirst { edge: String, target: String, roots: Option<String> },
    /// `match_all(re, s)` over symbols: `(s, g…)`, a row per non-overlapping match of `pattern` in each `s` of
    /// `input`'s one column whose `groups` all take part, each group's text (`logic::native::match_all`).
    MatchAll { input: String, pattern: String, groups: Vec<String>, positions: bool },
    /// `components(d, c, key, edge, own)`: each document's strongly connected component of `edge(d, e)` (its
    /// smallest document) and the component's Merkle key over `own(d, digest)` (`logic::native::components`).
    Components { edge: String, own: String },
    /// `r(x, y) :- seed(x, y). r(x, z) :- r(x, y), edge(y, z).`: each start's nodes reached over `edge` from its seeds,
    /// held as the reach index (decision 0119, `logic::native::reach`), not as rows.
    Reach { seed: String, edge: String },
    /// `s(x, q) :- r(x, e), map(e, q).` over a `Reach` relation `r`: each start's `map` values over what it reaches,
    /// held the same way.
    ReachThrough { reach: String, map: String },
    Python { kind: String, input: String, targets: Option<String> },
    /// Conservative call candidates over demanded documents; exact semantic operators remain separate.
    PythonCandidates { input: String, targets: String, context: String, member: String },
}

impl Native {
    /// Rewrite producer inputs with the same relation mapping as the rest of its program.
    pub fn rename_reads(&mut self, rename: impl Fn(&str) -> String) {
        match self {
            Native::DfsFirst { edge, target, roots } => {
                *edge = rename(edge);
                *target = rename(target);
                if let Some(roots) = roots { *roots = rename(roots); }
            }
            Native::MatchAll { input, .. } => *input = rename(input),
            Native::PythonCandidates {input,targets,..} => { *input=rename(input); *targets=rename(targets); },
            Native::Python {input,targets,..} => { *input=rename(input); if let Some(t)=targets {*t=rename(t);} },
            Native::Components { edge, own } => {
                *edge = rename(edge);
                *own = rename(own);
            }
            Native::Reach { seed, edge } => {
                *seed = rename(seed);
                *edge = rename(edge);
            }
            Native::ReachThrough { reach, map } => {
                *reach = rename(reach);
                *map = rename(map);
            }
        }
    }
    /// The relations it reads.
    pub fn reads(&self) -> Vec<&str> {
        match self {
            Native::DfsFirst { edge, target, roots } => [Some(edge.as_str()), Some(target.as_str()), roots.as_deref()].into_iter().flatten().collect(),
            Native::MatchAll { input, .. } => vec![input.as_str()],
            Native::PythonCandidates {input,targets,..} => vec![input.as_str(),targets.as_str()],
            Native::Python {input,targets,..} => std::iter::once(input.as_str()).chain(targets.as_deref()).collect(),
            Native::Components { edge, own } => vec![edge.as_str(), own.as_str()],
            Native::Reach { seed, edge } => vec![seed.as_str(), edge.as_str()],
            Native::ReachThrough { reach, map } => vec![reach.as_str(), map.as_str()],
        }
    }
}

/// The prefix of the relations the engine names itself (helpers, axes, places): a rule file may not declare one
/// (`rr::parse_file`), so a rule's relation never collides with them.
pub const RESERVED: &str = "rr_";

/// The prefix of a tree axis's relation: `rr_inside_<set>(n, a)`, a an ancestor of n whose kind is in the set
/// (decision 0036).
pub const AXIS: &str = "rr_inside_";

/// The prefix of the descendant axis's relation: `rr_has_<set>(n, a)`, n of a kind in the set and a any strict
/// ancestor of n (`core.rr`'s `has(a, n, set)`: the set tests the descendant).
pub const DESCENDANTS: &str = "rr_has_";

/// The prefix of a `nearest(n, a, set)` literal's relation: `rr_nearest_<set>(n, a)` (`plan::lower`).
pub const NEAREST: &str = "rr_nearest_";

/// Whether `rel` is a tree axis's relation, which the transducer makes: `(n, a)` rows.
pub fn is_axis(rel: &str) -> bool {
    rel.starts_with(AXIS) || rel.starts_with(DESCENDANTS)
}

/// Whether the transducer makes `rel`: a tree axis's relation or a `nearest` one, `(n, a)` rows.
pub fn is_transduced(rel: &str) -> bool {
    is_axis(rel) || rel.starts_with(NEAREST)
}

/// A tree axis relation's set, and whether it is `has`'s (`DESCENDANTS`: the set tested on its first column, the
/// descendant) rather than `inside`'s (`AXIS`: on its second, the ancestor).
pub fn axis(rel: &str) -> Option<(&str, bool)> {
    rel.strip_prefix(DESCENDANTS).map(|set| (set, true)).or_else(|| rel.strip_prefix(AXIS).map(|set| (set, false)))
}

/// A kind set's one member meaning every kind of the dialect: core.rr's `within(n, a)` is the tree axis over it
/// (`rr::lower::core`); lowering for a dialect makes it the dialect's kinds (`plan::lower`).
pub const EVERY_KIND: &str = "*";

/// What `magic_adorn` did (T's `magic_report`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub adornments: Vec<String>,
    pub magic_rules: usize,
    pub full: Vec<String>,
    pub extras_kept: usize,
    pub gates: usize,
    pub full_for_negation: Vec<String>,
}

/// A gated negation's gate: `done` copies `src` (the negated relation's demand) once `quiet` is quiet, after the
/// gates in `deps`; `src` is `None` for a unit gate (`done_N(0)`).
#[derive(Clone, Debug, PartialEq)]
pub struct Gate {
    pub src: Option<String>,
    pub rel: String,
    pub level: usize,
    pub quiet: Vec<String>,
    pub deps: Vec<String>,
}
