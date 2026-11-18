//! Moved from `front.rs` by the crate split (tools/cratify/moves.tsv).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use crate::code_rule_dsl::logic::front::name::{List, Name};

/// A term. Its names and texts are `Name`s, interned for the process as relation names are: a program read or
/// copied makes no string for a name it has met before (decision 0166).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Term {
    Var(Name),
    Wild,
    Int(i64),
    Str(Name),
    Top,
    Call(Name, List<Term>),
    /// An atom a string became when the program was lowered for a store (`plan::lower`, decision 0270): never an
    /// integer, so no pass takes one for the other, and a program moved to another store finds every name it holds.
    Branded(Branded),
}

/// What an atom names (decision 0270): a name is the store's (interned, numbered as the store met it); a kind or a
/// field is the grammar's (`kind_atom`, `field_atom`), the same in every store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    Name,
    Kind,
    Field,
}

/// An atom with what it names (decision 0270). Its value at run time is `atom`, as an integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Branded {
    pub class: Class,
    pub atom: u32,
}

impl Branded {
    pub fn name(atom: u32) -> Branded {
        Branded { class: Class::Name, atom }
    }
    /// Its value as the run holds it.
    pub fn value(self) -> i64 {
        i64::from(self.atom)
    }
    /// Its printed tag (`#n12`, `#k1034`, `#f3`), which the rr parser reads back.
    pub fn tag(self) -> char {
        match self.class {
            Class::Name => 'n',
            Class::Kind => 'k',
            Class::Field => 'f',
        }
    }
    pub fn of_tag(tag: char, atom: u32) -> Option<Branded> {
        let class = match tag {
            'n' => Class::Name,
            'k' => Class::Kind,
            'f' => Class::Field,
            _ => return None,
        };
        Some(Branded { class, atom })
    }
}

impl fmt::Display for Branded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}{}", self.tag(), self.atom)
    }
}

impl Term {
    pub fn vars(&self, out: &mut BTreeSet<String>) {
        match self {
            Term::Var(v) => {
                out.insert(v.to_string());
            }
            Term::Call(_, args) => args.iter().for_each(|a| a.vars(out)),
            _ => {}
        }
    }

    /// The integer the run holds for a constant: an integer's, or a branded atom's (decision 0270).
    pub fn integer(&self) -> Option<i64> {
        match self {
            Term::Int(i) => Some(*i),
            Term::Branded(a) => Some(a.value()),
            _ => None,
        }
    }

    /// `@cat` over variables only: the one builtin that can run backwards, splitting a bound result.
    pub fn is_cat(&self) -> bool {
        matches!(self, Term::Call(f, args) if f == "cat" && args.iter().all(|a| matches!(a, Term::Var(_))))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Atom {
    pub rel: Name,
    pub args: List<Term>,
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
    if op == "=" && let Term::Var(x) = l && !bound.contains(x.as_str()) {
        return rv.is_subset(bound).then(|| BTreeSet::from([x.to_string()]));
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
            Term::Branded(b) => write!(f, "{b}"),
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Rule {
    pub head: Atom,
    pub body: List<Literal>,
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    Int(i64),
    Str(Name),
    /// A string a fact held, lowered to its atom (decision 0270).
    Branded(Branded),
}

#[derive(Clone, Debug, Default)]
pub struct Program {
    /// Each relation's columns: (name, type).
    pub decls: BTreeMap<Name, List<(Name, Name)>>,
    /// Logically nullary relations encoded with one physical unit column.
    pub nullary: BTreeSet<String>,
    pub lattice: BTreeMap<String, LatticeDecl>,
    pub facts: BTreeMap<String, Vec<List<Value>>>,
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
    /// The rules lowering made to ask for a node's attribute (decision 0174's `rr_node_text_req` …), by request
    /// relation: for each of its rules, in order, the relations whose readers it serves. `prune` keeps a request
    /// rule only while one of them is reached, as a reader's request is kept with the reader.
    pub asked_by: BTreeMap<String, Vec<BTreeSet<String>>>,
    /// Join shape directives (`.shape`, O-join-shape-directives), each on the rules of its relation and every form
    /// made from them.
    pub shapes: Vec<Shape>,
    /// What each relation's rows are known to be (`.layout`, decision 0361), by relation: facts about the rows, so
    /// they hold for every form made from its rules. Several lines add up; each fact once, in the order first written.
    pub layouts: BTreeMap<String, Vec<LayoutFact>>,
}

/// A join shape directive written after a rule (`.shape posarg after pv`, `.shape retparam bound f`): in the rules of
/// `head`, and in every form made from them (their deltas' and maintenance's, `shape_base`), the literal of `lit` is
/// joined only once `wait` holds. A rule's driving delta is never held back. The planner searches within it, so a
/// directive narrows the search; it does not fix an order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    pub head: String,
    pub lit: String,
    pub wait: ShapeWait,
    /// As written, for the profile.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShapeWait {
    /// Once a literal of this relation is joined.
    After(String),
    /// Once these variables are bound.
    Bound(Vec<String>),
}

/// The relation a form's relation was made from, for matching directives (`Shape`): `pv__ins` and `pv__old` are
/// `pv`'s.
pub fn shape_base(rel: &str) -> &str {
    const SUFFIXES: [&str; 12] = ["__ins", "__del", "__plus", "__minus", "__old", "__redo", "__held", "__gone", "__cand", "__chk", "__live", "__prev"];
    let mut r = rel;
    while let Some(s) = SUFFIXES.iter().find_map(|s| r.strip_suffix(s)) {
        r = s;
    }
    r
}

impl Shape {
    /// `B after A` or `B bound x, y` (the directive's text), on the rules of `head`.
    pub fn parse(head: &str, text: &str) -> Result<Shape, String> {
        let words: Vec<&str> = text.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).collect();
        let wait = match words.as_slice() {
            [_, "after", a] => ShapeWait::After((*a).to_owned()),
            [_, "bound", vars @ ..] if !vars.is_empty() => ShapeWait::Bound(vars.iter().map(|v| v.trim_start_matches('$').to_owned()).collect()),
            _ => return Err(format!("`.shape {text}`: write `.shape <relation> after <relation>` or `.shape <relation> bound <variable>, …`")),
        };
        Ok(Shape { head: head.to_owned(), lit: words[0].to_owned(), wait, text: text.trim().to_owned() })
    }
}

/// How many rows a probe on a fan-out's columns returns (`.layout r fanout(p -> c, few)`, decision 0361): one, few
/// (16 or fewer), many, or a measured number (`~4`), which is kept for the estimate but plans by its class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fan {
    One,
    Few,
    Many,
    About(u32),
}

/// The most a few-fan-out probe returns (decision 0361: "few (16 or fewer)").
pub const FEW: u32 = 16;

impl Fan {
    /// The class a plan is keyed by: a measured number is the class it falls in.
    pub fn class(self) -> Fan {
        match self {
            Fan::About(n) if n <= 1 => Fan::One,
            Fan::About(n) if n <= FEW => Fan::Few,
            Fan::About(_) => Fan::Many,
            f => f,
        }
    }
}

/// A relation's size (`.layout r size(…)`, decision 0361): at most `n` rows per value of a column (`per n`, `<= 2 per
/// n`), one per document or per kind, or small (a table of the rules' own).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Size {
    Per { most: u32, col: usize },
    PerDocument,
    PerKind,
    Small,
}

/// The native operator that makes a relation, where its rules' form does not show it (`.layout r by …`, decision
/// 0361 item 4): an override a profiled run reports once the facts find it alone.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Algo {
    Reach,
    /// A walk along a tree relation.
    Walk(String),
    /// A running aggregate along a column ordered within a group.
    Running { col: usize, within: Vec<usize> },
}

/// One fact a relation's rows hold (`.layout`, decision 0361), its columns by place. A fact is about the rows, not
/// a plan, so it holds for every form made from the relation's rules (their deltas', maintenance's and hiding's).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LayoutFact {
    /// The columns are a key: a probe on them returns at most one row.
    Key(Vec<usize>),
    /// A probe on `from` returns `fan` rows of `to`.
    Fanout { from: Vec<usize>, to: Vec<usize>, fan: Fan },
    Size(Size),
    /// Every column that holds a node holds one of one document: no row crosses documents.
    Local,
    /// `col` is ordered within each group of `within`; dense: numbered 0, 1, 2, … in each.
    Order { col: usize, within: Vec<usize>, dense: bool },
    /// `from -> to` is a forest: each `to` has at most one `from`, and no cycle.
    Tree { from: usize, to: usize },
    By(Algo),
}

/// A `.layout` line's facts, read: `text` is the line after the relation's name, `col` names a column's place (a
/// declared name, or `$k` for the k-th, as `LayoutFact::show` writes them).
pub fn layout_facts(text: &str, col: &dyn Fn(&str) -> Result<usize, String>) -> Result<Vec<LayoutFact>, String> {
    // Words, with `(`, `)`, `,` and `->` apart.
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let punct = match c {
            '(' | ')' | ',' => Some(c.to_string()),
            '-' if chars.peek() == Some(&'>') => {
                chars.next();
                Some("->".to_owned())
            }
            c if c.is_whitespace() => Some(String::new()),
            _ => None,
        };
        match punct {
            Some(p) => {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
                if !p.is_empty() {
                    words.push(p);
                }
            }
            None => cur.push(c),
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    let mut at = 0;
    let mut facts = Vec::new();
    // The words inside the parentheses that follow the fact's name.
    let args = |at: &mut usize, name: &str| -> Result<Vec<String>, String> {
        if words.get(*at).map(String::as_str) != Some("(") {
            return Err(format!("`{name}` needs its columns in parentheses"));
        }
        let start = *at + 1;
        let end = words[start..].iter().position(|w| w == ")").map(|k| start + k).ok_or_else(|| format!("`{name}(` is not closed"))?;
        *at = end + 1;
        Ok(words[start..end].to_vec())
    };
    let cols = |ws: &[String]| -> Result<Vec<usize>, String> {
        let names: Vec<&String> = ws.iter().filter(|w| *w != ",").collect();
        if names.is_empty() {
            return Err("no column named".to_owned());
        }
        names.into_iter().map(|w| col(w)).collect()
    };
    let one = |ws: &[String]| -> Result<usize, String> {
        match cols(ws)?.as_slice() {
            [c] => Ok(*c),
            _ => Err(format!("one column, not `{}`", ws.join(" "))),
        }
    };
    let number = |w: &str| w.parse::<u32>().map_err(|_| format!("`{w}`: a number"));
    while at < words.len() {
        let name = words[at].clone();
        at += 1;
        let fact = match name.as_str() {
            "key" => LayoutFact::Key(cols(&args(&mut at, "key")?)?),
            "local" => LayoutFact::Local,
            "fanout" => {
                let a = args(&mut at, "fanout")?;
                let arrow = a.iter().position(|w| w == "->").ok_or("`fanout(from -> to, class)`")?;
                let comma = a.iter().rposition(|w| w == ",").filter(|&k| k > arrow).ok_or("`fanout(from -> to, class)`: no class")?;
                let fan = match a[comma + 1..].iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
                    ["one"] => Fan::One,
                    ["few"] => Fan::Few,
                    ["many"] => Fan::Many,
                    [n] if n.starts_with('~') => Fan::About(number(&n[1..])?),
                    other => return Err(format!("`fanout`: a class is one, few, many or ~N, not `{}`", other.join(" "))),
                };
                LayoutFact::Fanout { from: cols(&a[..arrow])?, to: cols(&a[arrow + 1..comma])?, fan }
            }
            "size" => {
                let a = args(&mut at, "size")?;
                let a: Vec<&str> = a.iter().map(String::as_str).collect();
                LayoutFact::Size(match a.as_slice() {
                    ["small"] => Size::Small,
                    ["per", "document"] => Size::PerDocument,
                    ["per", "kind"] => Size::PerKind,
                    ["per", c] => Size::Per { most: 1, col: col(c)? },
                    ["<=", n, "per", c] => Size::Per { most: number(n)?, col: col(c)? },
                    _ => return Err(format!("`size({})`: write `per <column>`, `<= N per <column>`, `per document`, `per kind` or `small`", a.join(" "))),
                })
            }
            "order" => {
                let a = args(&mut at, "order")?;
                let inn = a.iter().position(|w| w == "in").ok_or("`order(column [dense] in group)`")?;
                let (head, within) = (&a[..inn], &a[inn + 1..]);
                let dense = head.last().map(String::as_str) == Some("dense");
                let head = if dense { &head[..head.len() - 1] } else { head };
                LayoutFact::Order { col: one(head)?, within: cols(within)?, dense }
            }
            "tree" => {
                let a = args(&mut at, "tree")?;
                let arrow = a.iter().position(|w| w == "->").ok_or("`tree(parent -> child)`")?;
                LayoutFact::Tree { from: one(&a[..arrow])?, to: one(&a[arrow + 1..])? }
            }
            "by" => {
                let algo = words.get(at).cloned().ok_or("`by` names an algorithm: reach, walk(r) or running(c in g)")?;
                at += 1;
                LayoutFact::By(match algo.as_str() {
                    "reach" => Algo::Reach,
                    "walk" => match args(&mut at, "walk")?.as_slice() {
                        [r] => Algo::Walk(r.clone()),
                        _ => return Err("`by walk(relation)`".to_owned()),
                    },
                    "running" => {
                        let a = args(&mut at, "running")?;
                        let inn = a.iter().position(|w| w == "in").ok_or("`by running(column in group)`")?;
                        Algo::Running { col: one(&a[..inn])?, within: cols(&a[inn + 1..])? }
                    }
                    other => return Err(format!("`by {other}`: an algorithm is reach, walk(r) or running(c in g)")),
                })
            }
            other => return Err(format!("`{other}`: a layout fact is key, fanout, size, local, order, tree or by")),
        };
        facts.push(fact);
    }
    if facts.is_empty() {
        return Err("no fact".to_owned());
    }
    Ok(facts)
}

impl LayoutFact {
    /// The fact as `layout_facts` reads it back, its columns named by `name` (`$k` keeps their places).
    pub fn show(&self, name: &dyn Fn(usize) -> String) -> String {
        let list = |cs: &[usize]| cs.iter().map(|&c| name(c)).collect::<Vec<_>>().join(", ");
        match self {
            LayoutFact::Key(cs) => format!("key({})", list(cs)),
            LayoutFact::Fanout { from, to, fan } => {
                let fan = match fan {
                    Fan::One => "one".to_owned(),
                    Fan::Few => "few".to_owned(),
                    Fan::Many => "many".to_owned(),
                    Fan::About(n) => format!("~{n}"),
                };
                format!("fanout({} -> {}, {fan})", list(from), list(to))
            }
            LayoutFact::Size(Size::Small) => "size(small)".to_owned(),
            LayoutFact::Size(Size::PerDocument) => "size(per document)".to_owned(),
            LayoutFact::Size(Size::PerKind) => "size(per kind)".to_owned(),
            LayoutFact::Size(Size::Per { most: 1, col }) => format!("size(per {})", name(*col)),
            LayoutFact::Size(Size::Per { most, col }) => format!("size(<= {most} per {})", name(*col)),
            LayoutFact::Local => "local".to_owned(),
            LayoutFact::Order { col, within, dense } => format!("order({}{} in {})", name(*col), if *dense { " dense" } else { "" }, list(within)),
            LayoutFact::Tree { from, to } => format!("tree({} -> {})", name(*from), name(*to)),
            LayoutFact::By(Algo::Reach) => "by reach".to_owned(),
            LayoutFact::By(Algo::Walk(r)) => format!("by walk({r})"),
            LayoutFact::By(Algo::Running { col, within }) => format!("by running({} in {})", name(*col), list(within)),
        }
    }

    /// The fact with its columns by place (`$k`), as the program's facts and its image keep it.
    pub fn placed(&self) -> String {
        self.show(&|c| format!("${c}"))
    }

    /// A placed fact (`placed`) read back.
    pub fn of_placed(text: &str) -> Result<LayoutFact, String> {
        match layout_facts(text, &placed_col)?.as_slice() {
            [f] => Ok(f.clone()),
            _ => Err(format!("`{text}`: one layout fact")),
        }
    }

    /// The relations the fact names besides its own (`by walk(r)`), which `combine` renames.
    pub fn rename(&mut self, to: &dyn Fn(&str) -> String) {
        if let LayoutFact::By(Algo::Walk(r)) = self {
            *r = to(r);
        }
    }
}

/// A column named by place, `$k`.
pub fn placed_col(w: &str) -> Result<usize, String> {
    w.strip_prefix('$').and_then(|k| k.parse().ok()).ok_or_else(|| format!("`{w}`: a column by place, `$k`"))
}

/// Adds `facts` to a relation's layout: each fact once, in the order first written.
pub fn add_layout(to: &mut Vec<LayoutFact>, facts: impl IntoIterator<Item = LayoutFact>) {
    for f in facts {
        if !to.contains(&f) {
            to.push(f);
        }
    }
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

/// `Program::readers_of_rules` over the rules' heads alone, in order: a pass that holds its rules in another form
/// (the version's `Hide`) reads the record as the tree's does.
pub fn readers_of_heads(asked_by: &BTreeMap<String, Vec<BTreeSet<String>>>, heads: &[&str]) -> Vec<Option<BTreeSet<String>>> {
    if asked_by.is_empty() {
        return vec![None; heads.len()];
    }
    let mut at: BTreeMap<&str, usize> = BTreeMap::new();
    let counts: BTreeMap<&str, usize> = asked_by.keys().map(|req| (req.as_str(), heads.iter().filter(|h| **h == req.as_str()).count())).collect();
    heads
        .iter()
        .map(|&req| {
            let readers = asked_by.get(req)?;
            let k = at.entry(req).or_default();
            *k += 1;
            Some(if counts[req] == readers.len() { readers[*k - 1].clone() } else { BTreeSet::from([req.to_owned()]) })
        })
        .collect()
}

/// The request record's step for a copy with head `head` of the rule at `from`, before it is appended to the rules
/// whose heads are `heads` (`Program::push_rule_from`): a copy under another name is a reader where its source is,
/// and a copy of a request rule serves its source's readers.
pub fn record_copy(asked_by: &mut BTreeMap<String, Vec<BTreeSet<String>>>, heads: &[&str], from: usize, head: &str) {
    if let Some(&src) = heads.get(from)
        && src != head
    {
        for readers in asked_by.values_mut().flatten().filter(|by| by.contains(src)) {
            readers.insert(head.to_owned());
        }
    }
    if asked_by.contains_key(head) {
        let serves = readers_of_heads(asked_by, heads).get(from).cloned().flatten().filter(|_| heads[from] == head).unwrap_or_else(|| BTreeSet::from([head.to_owned()]));
        let rules = heads.iter().filter(|h| **h == head).count();
        let readers = asked_by.get_mut(head).expect("recorded");
        if rules == readers.len() {
            readers.push(serves);
        }
    }
}

/// The `.decl` qualifiers that choose what a relation keeps (decision 0099), as Soufflé spells them.
pub const QUALIFIERS: &[&str] = &["inline", "no_inline", "magic", "no_magic", "reach", "no_reach", "sorted"];

impl Program {
    /// Whether `.decl rel(…)` carries the qualifier `word` (one of `QUALIFIERS`).
    pub fn qualified(&self, rel: &str, word: &str) -> bool {
        self.qualifiers.get(rel).is_some_and(|q| q.contains(word))
    }

    /// Each rule's readers in the request record (`asked_by`), by its place in `rules`: `None` for a rule of no
    /// request relation, or of one whose rules no longer match the record (then its own relation, which keeps it).
    pub fn readers_of_rules(&self) -> Vec<Option<BTreeSet<String>>> {
        let heads: Vec<&str> = self.rules.iter().map(|r| r.head.rel.as_str()).collect();
        readers_of_heads(&self.asked_by, &heads)
    }

    /// Replaces the rules, each with the places in the old `rules` it came from, and carries the request record
    /// (`asked_by`) across: a request rule serves the readers of the rules it came from, together, and one that
    /// came from none (or from a rule the record did not cover) its own relation, which keeps it wherever its
    /// relation is needed (`front::prune`). A pass that rewrites, splits, merges or drops request rules states their
    /// sources here, and the record holds through it.
    pub fn set_rules_from(&mut self, rules: Vec<(Rule, Vec<usize>)>) {
        let carry = self.carry();
        self.set_rules_carried(&carry, rules);
    }
    /// Appends `rule`, a copy made from the rule at `from`: a copy of a request rule serves the readers its source
    /// serves (`set_rules_from`). A copy under another name is no request rule, but reads what its source reads: where
    /// its source is a reader, so is it (a guest's copy of a reader, read in its place, asks what the reader asked).
    pub fn push_rule_from(&mut self, rule: Rule, from: usize) {
        if !self.asked_by.is_empty() {
            let heads: Vec<&str> = self.rules.iter().map(|r| r.head.rel.as_str()).collect();
            record_copy(&mut self.asked_by, &heads, from, rule.head.rel.as_str());
        }
        self.rules.push(rule);
    }
    /// Renames readers in the request record, as a pass that merges or folds relations renames their reads
    /// (`simplify::merge_identical`, hiding's `fold_own`): a request of a merged reader serves the relation it was
    /// merged into.
    pub fn rename_readers(&mut self, rename: impl Fn(&str) -> Option<String>) {
        for readers in self.asked_by.values_mut().flatten() {
            if readers.iter().any(|b| rename(b).is_some()) {
                *readers = std::mem::take(readers).into_iter().map(|b| rename(&b).unwrap_or(b)).collect();
            }
        }
    }
    /// For relations `gone` that a pass inlines into their readers, the relations whose rules read them (through other
    /// relations `gone`), taken before the pass: what `replace_readers` puts in their place.
    pub fn inlined_readers<'a>(&self, gone: impl IntoIterator<Item = &'a String>) -> BTreeMap<String, BTreeSet<String>> {
        let gone: BTreeSet<&str> = gone.into_iter().map(String::as_str).collect();
        if self.asked_by.is_empty() || gone.is_empty() {
            return BTreeMap::new();
        }
        let mut direct: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for r in &self.rules {
            for a in r.body.iter().filter_map(Literal::atom).filter(|a| gone.contains(a.rel.as_str())) {
                direct.entry(a.rel.as_str()).or_default().insert(r.head.rel.as_str());
            }
        }
        gone.iter()
            .map(|g| {
                let (mut out, mut seen, mut todo) = (BTreeSet::new(), BTreeSet::from([*g]), vec![*g]);
                while let Some(x) = todo.pop() {
                    for &h in direct.get(x).into_iter().flatten() {
                        if !gone.contains(h) {
                            out.insert(h.to_owned());
                        } else if seen.insert(h) {
                            todo.push(h);
                        }
                    }
                }
                ((*g).to_owned(), out)
            })
            .filter(|(_, out)| !out.is_empty())
            .collect()
    }
    /// Puts each reader `inlined` names in the record in place of its readers (`inlined_readers`): a request of a
    /// relation inlined into its readers serves them.
    pub fn replace_readers(&mut self, inlined: &BTreeMap<String, BTreeSet<String>>) {
        if inlined.is_empty() {
            return;
        }
        for readers in self.asked_by.values_mut().flatten() {
            if readers.iter().any(|b| inlined.contains_key(b)) {
                *readers = std::mem::take(readers).into_iter().flat_map(|b| inlined.get(&b).cloned().unwrap_or_else(|| BTreeSet::from([b]))).collect();
            }
        }
    }
    /// The request record by rule (`readers_of_rules`), taken before a pass that takes the rules apart; empty when
    /// the program has no requests.
    pub fn carry(&self) -> Carry {
        Carry(if self.asked_by.is_empty() { Vec::new() } else { self.readers_of_rules() })
    }
    /// `set_rules_from`, the old rules' readers taken (`carry`) before the rules were.
    pub fn set_rules_carried(&mut self, carry: &Carry, rules: Vec<(Rule, Vec<usize>)>) {
        if self.asked_by.is_empty() {
            self.rules = rules.into_iter().map(|(r, _)| r).collect();
            return;
        }
        let asked = carried_record(&self.asked_by, &carry.0, rules.iter().map(|(r, from)| (r.head.rel.as_str(), from.as_slice())));
        self.asked_by = asked;
        self.rules = rules.into_iter().map(|(r, _)| r).collect();
    }
}

/// The request record after a pass that writes `rules` (each a head and the places of the old rules it came from), the
/// old rules' readers `old` (`Program::carry`): a request rule serves the readers of the rules it came from, together,
/// and one that came from none (or from a rule the record did not cover) its own relation (`Program::set_rules_from`).
pub fn carried_record<'r>(asked_by: &BTreeMap<String, Vec<BTreeSet<String>>>, old: &[Option<BTreeSet<String>>], rules: impl IntoIterator<Item = (&'r str, &'r [usize])>) -> BTreeMap<String, Vec<BTreeSet<String>>> {
    let mut asked: BTreeMap<String, Vec<BTreeSet<String>>> = BTreeMap::new();
    for (req, from) in rules {
        if !asked_by.contains_key(req) {
            continue;
        }
        let mut readers: BTreeSet<String> = BTreeSet::new();
        for &i in from {
            match old.get(i) {
                Some(Some(by)) => readers.extend(by.iter().cloned()),
                _ => {
                    readers.insert(req.to_owned());
                }
            }
        }
        if readers.is_empty() {
            readers.insert(req.to_owned());
        }
        asked.entry(req.to_owned()).or_default().push(readers);
    }
    asked
}

/// Each rule's readers in a program's request record, by its place (`Program::carry`).
pub struct Carry(Vec<Option<BTreeSet<String>>>);

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
    /// The children of nodes of this kind: a node a rule reaches as such a child (`child(p, n, _), kind(p, "block")`),
    /// whose own kinds it does not name.
    ChildOf(String),
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
    /// A text test (`@op(s, patterns) = 1`) over a column the program derives: the rows of `input` whose `column`
    /// passes it, each distinct string tested by the run's vocabulary with every other test at once (decision 0037),
    /// once a run (`logic::strings::as_relations`). A row is there as its input row is and its string passes.
    TextTest { input: String, column: usize, op: String, patterns: Vec<String> },
    /// `components(d, c, key, edge, own)`: each document's strongly connected component of `edge(d, e)` (its
    /// smallest document) and the component's Merkle key over `own(d, digest)` (`logic::native::components`).
    Components { edge: String, own: String },
    /// `r(x, y) :- seed(x, y). r(x, z) :- r(x, y), edge(y, z).`: each start's nodes reached over `edge` from its seeds,
    /// held as the reach index (decision 0119, `logic::native::reach`), not as rows.
    Reach { seed: String, edge: String },
    /// `s(x, q) :- r(x, e), map(e, q).` over a `Reach` relation `r`: each start's `map` values over what it reaches,
    /// held the same way.
    ReachThrough { reach: String, map: String },
    /// `r(g…, q…, n)`: for each distinct tuple of `query(g…, q…, kx)` but its bound, `n` distinct `(g…, c…)` tuples of
    /// `counted(g…, c…, ky)` have `ky op kx` (`logic::native::rank`), its first `group` columns the group. A count
    /// below a bound made without the pairs (`rank_form`, decision 0361 step 7).
    Rank { query: String, counted: String, group: usize, op: String },
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
            Native::MatchAll { input, .. } | Native::TextTest { input, .. } => *input = rename(input),
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
            Native::Rank { query, counted, .. } => {
                *query = rename(query);
                *counted = rename(counted);
            }
        }
    }
    /// The relations it reads.
    pub fn reads(&self) -> Vec<&str> {
        match self {
            Native::DfsFirst { edge, target, roots } => [Some(edge.as_str()), Some(target.as_str()), roots.as_deref()].into_iter().flatten().collect(),
            Native::MatchAll { input, .. } | Native::TextTest { input, .. } => vec![input.as_str()],
            Native::PythonCandidates {input,targets,..} => vec![input.as_str(),targets.as_str()],
            Native::Python {input,targets,..} => std::iter::once(input.as_str()).chain(targets.as_deref()).collect(),
            Native::Components { edge, own } => vec![edge.as_str(), own.as_str()],
            Native::Reach { seed, edge } => vec![seed.as_str(), edge.as_str()],
            Native::ReachThrough { reach, map } => vec![reach.as_str(), map.as_str()],
            Native::Rank { query, counted, .. } => vec![query.as_str(), counted.as_str()],
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

/// A term's parts by their addresses (`Shallow`): its names and its call's arguments.
impl crate::code_rule_dsl::logic::front::name::Shallow for Term {
    fn shallow_hash(&self, h: &mut rustc_hash::FxHasher) {
        use std::hash::Hasher;
        match self {
            Term::Var(v) => {
                h.write_u8(0);
                h.write_usize(v.address());
            }
            Term::Wild => h.write_u8(1),
            Term::Int(i) => {
                h.write_u8(2);
                h.write_i64(*i);
            }
            Term::Str(s) => {
                h.write_u8(3);
                h.write_usize(s.address());
            }
            Term::Top => h.write_u8(4),
            Term::Call(f, args) => {
                h.write_u8(5);
                h.write_usize(f.address());
                h.write_usize(args.address());
            }
            Term::Branded(b) => {
                h.write_u8(6);
                h.write_u8(b.class as u8);
                h.write_u32(b.atom);
            }
        }
    }
    fn shallow_eq(&self, other: &Term) -> bool {
        match (self, other) {
            (Term::Call(f, a), Term::Call(g, b)) => f == g && a.same(b),
            (a, b) => a == b,
        }
    }
}

impl Atom {
    fn shallow_hash(&self, h: &mut rustc_hash::FxHasher) {
        use std::hash::Hasher;
        h.write_usize(self.rel.address());
        h.write_usize(self.args.address());
        h.write_u8(u8::from(self.neg) | u8::from(self.gated) << 1);
    }
    fn shallow_eq(&self, other: &Atom) -> bool {
        self.rel == other.rel && self.args.same(&other.args) && self.neg == other.neg && self.gated == other.gated
    }
}

/// A literal's parts by their addresses (`Shallow`): its atom's relation and arguments, its terms.
impl crate::code_rule_dsl::logic::front::name::Shallow for Literal {
    fn shallow_hash(&self, h: &mut rustc_hash::FxHasher) {
        use std::hash::{Hash, Hasher};
        match self {
            Literal::Atom(a) => {
                h.write_u8(0);
                a.shallow_hash(h);
            }
            Literal::Cmp { l, op, r } => {
                h.write_u8(1);
                l.shallow_hash(h);
                op.hash(h);
                r.shallow_hash(h);
            }
            Literal::Agg { var, op, over, atom } => {
                h.write_u8(2);
                var.hash(h);
                op.hash(h);
                over.hash(h);
                atom.shallow_hash(h);
            }
            other => {
                h.write_u8(3);
                other.hash(h);
            }
        }
    }
    fn shallow_eq(&self, other: &Literal) -> bool {
        match (self, other) {
            (Literal::Atom(a), Literal::Atom(b)) => a.shallow_eq(b),
            (Literal::Cmp { l, op, r }, Literal::Cmp { l: l2, op: op2, r: r2 }) => op == op2 && l.shallow_eq(l2) && r.shallow_eq(r2),
            (Literal::Agg { var, op, over, atom }, Literal::Agg { var: v2, op: o2, over: w2, atom: a2 }) => var == v2 && op == o2 && over == w2 && atom.shallow_eq(a2),
            (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b) && !matches!(a, Literal::Atom(_) | Literal::Cmp { .. } | Literal::Agg { .. }) && a == b,
        }
    }
}

impl Value {
    /// The integer the run holds for it: an integer's, or a branded atom's (decision 0270).
    pub fn integer(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Branded(a) => Some(a.value()),
            Value::Str(_) => None,
        }
    }
}

/// A value by its name's address (`Shallow`).
impl crate::code_rule_dsl::logic::front::name::Shallow for Value {
    fn shallow_hash(&self, h: &mut rustc_hash::FxHasher) {
        use std::hash::Hasher;
        match self {
            Value::Int(i) => {
                h.write_u8(0);
                h.write_i64(*i);
            }
            Value::Str(s) => {
                h.write_u8(1);
                h.write_usize(s.address());
            }
            Value::Branded(b) => {
                h.write_u8(2);
                h.write_u8(b.class as u8);
                h.write_u32(b.atom);
            }
        }
    }
    fn shallow_eq(&self, other: &Value) -> bool {
        self == other
    }
}

/// The parser's punctuation, longest first where one begins another.
pub const PUNCT: [&str; 22] = [":-", "!=", "<=", ">=", "(", ")", ",", ".", ";", ":", "=", "<", ">", "!", "{", "}", "|", "@", "*", "[", "]", "-"];

/// The parser's own spelling of the operator `op`, if it is one (a comparison read back from facts or the store).
pub fn operator(op: &str) -> Option<&'static str> {
    PUNCT.iter().copied().find(|p| *p == op)
}
