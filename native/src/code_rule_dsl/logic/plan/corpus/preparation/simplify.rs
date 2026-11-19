//! Rule bodies simplified before planning (decision 0038), so a long or repeated rule costs what it says:
//!
//! - **Implied atoms drop.** A positive atom goes when another positive atom of the same relation has, at every
//!   position, the same term, or any term where it has `_`: `doc($d, $p, _)` beside `doc($d, $p, $m)`.
//! - **A kind test on a set member drops.** `node(_, $a, "k", _, _)` goes where `$a` is the ancestor of a
//!   `nearest($n, $a, set)` or `inside_<set>($n, $a)` of the same rule and the set is exactly `{k}`.
//! - **A variable equal to a constant is the constant.** `text($n, $v), $v = "get"` is `text($n, "get")`: the
//!   planner then reads the atom by its constant column (a lookup by string), where a separate `$v = "get"` binds
//!   `$v` only after the atom is scanned some other way. A variable the head names stays (a head column filled from a
//!   text column keeps its relation a string filter, decision 0037), as does one an aggregate, `nearest` or `ast` names.
//! - **A test of one node read only positively is inlined.** `m($n) :- node(_, $n, _, _, _), !k($n)` (a complement:
//!   the converter's `not`) read as `m($t)` becomes the reader's `node(_, $t, _, _, _), !k($t)`: where the reader has
//!   bound `$t`, a probe of `k`, not a relation of every node less `k`'s. Such a relation has one rule, over its one
//!   node variable alone and not recursive, and no negated or aggregated reader; kept relations (below) stay. (One
//!   rule: a relation of several would copy each reader once per rule, its whole join run again.)
//! - **A variable named once is `_`.** In a positive atom, `text($v122, $s)` where `$v122` appears nowhere else is
//!   `text(_, $s)`; it is then implied by any `text($n, $s)` beside it and goes.
//! - **Two atoms of one keyed row are one.** `node($d, $n, _, _, _), node(_, $n, "call", _, _)` is
//!   `node($d, $n, "call", _, _)`: a node's row is one (`KEYS`), so atoms with the same key term read the same row.
//!   Columns merge where one is `_` or both are the same; two variables in one column are one variable.
//! - **A part of a body that meets the rest only at a document is a relation of documents.**
//!   `m($n) :- node($d, $n, "module", _, _), node($d, $y, _, _, _), name($y, $s), @contains($s, "cache")` asks
//!   whether the file names something with `cache`, once per such name: every match multiplies the rest's rows. The
//!   part `node($d, $y, …), name($y, $s), @contains(…)` shares only `$d` with the rest and the head, so it becomes
//!   `exists_k($d) :- node($d, $y, _, _, _), name($y, $s), @contains($s, "cache")`, one row per file, and the rule reads
//!   `exists_k($d)` (a semi-join). The same set of rows: the part's other variables appear nowhere else. A part moves
//!   when it has two literals or more, a variable of its own, no aggregate, `nearest` or `ast`, and every variable
//!   it names bound by a positive atom of its own. A document variable is one in `node`'s or `doc`'s first column.
//! - **A relation of no columns has one.** `version_given()` is `version_given(1)` where it is made and
//!   `version_given(_)` where it is read: a table needs a column, and the one row says what the empty tuple did.
//! - **A rule of constants alone is facts.** `q(@dot("builtins", $x)) :- $x = "bool"` (a disjunction of constants,
//!   split into one rule per arm) reads no relation, so it has no SQL to run: it is the fact `q("builtins.bool")`.
//!   A rule whose body is only equations of a variable and a constant, and whose head is then constants (`@dot` of
//!   two constants included), becomes its facts; one that equates a variable with two different constants goes.
//! - **Relations no output reads go.** A derived relation that no output, demanded or kept relation reads,
//!   directly or through others, is not computed.
//! - **Identical relations merge.** Derived relations whose rules are equal once their variables are renamed in
//!   order and each relation is named by its class are one relation: the greatest such partition (`congruence`), so
//!   recursive relations equal only to each other merge too. A library relation every combined program has (`p0_qn`,
//!   `p1_qn`) is one relation in the shared namespace (`s_qn`); a program's own relations keep its namespace unless
//!   they are the same as another's. Outputs, inputs, lattices, relations with facts and demanded relations are kept
//!   as they are.
//!
//! Each simplification is a manager pass (decision 0281's path, step 6e), run in the order `passes` gives, the rules
//! kept in their order (each pass derives its output), and its decisions recorded as it makes them.
//!
//! A pass reads a rule through its handles. What it writes of a rule it changes is the rule's own literals (copied,
//! through a `Rewrite` where a variable or term changes) and the literals it makes; `Tm` is a term it makes or
//! compares, by text.

use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::build::{Rewrite, Site, T};
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW, RuleOut, RulePass, Select, Step};
use crate::code_rule_dsl::logic::program_image::{AggRef, AtomId, AtomRef, Builder, Cell, CmpOp, Lit, LitRef, RuleBuilder, RuleRef, Slot, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

type Result<T> = std::result::Result<T, String>;

const SPAN_WITHIN: &str = crate::code_rule_dsl::facts::vtab::syntax::SPAN_WITHIN;
const WITHIN: &str = crate::code_rule_dsl::facts::vtab::syntax::WITHIN;
/// The variables of decision 0091's scope guards (`corpus::scoping`): a node's document, `rr_sd0 = @doc($n)`.
pub(super) const GUARD_VAR: &str = "rr_sd";

/// What was simplified, added up over the passes.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub implied: usize,
    pub kind_tests: usize,
    pub constants: usize,
    pub merged: usize,
    pub inlined: Vec<String>,
    pub singletons: usize,
    pub keyed: usize,
    pub unread: Vec<String>,
    /// Request rules another of their relation's subsumes, dropped (`SubsumeRequests`).
    pub subsumed: usize,
    pub existentials: Vec<String>,
    pub within: usize,
    pub climbs: Vec<String>,
    pub shared: usize,
    pub projected: Vec<(String, Vec<usize>)>,
    pub existence: usize,
    pub inverses: usize,
    pub keyed_by: Vec<String>,
    /// Relations equal up to their constants: how many, in how many groups, and a few groups (profiled runs only).
    pub similar: (usize, usize, Vec<String>),
}

// ---- terms made or compared ----

/// A term by text: one a pass makes, or one it compares.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum Tm {
    Var(String),
    Wild,
    Top,
    Int(i64),
    Str(String),
    /// A branded atom (decision 0270), the store's: written as it is.
    Branded(crate::code_rule_dsl::logic::front::Branded),
    Call(String, Vec<Tm>),
}

impl Tm {
    pub(super) fn of(v: &Version, r: RuleRef<'_>, t: TermRef<'_>) -> Tm {
        match t {
            TermRef::Var(s) => Tm::Var(v.text(r.var(s)).to_owned()),
            TermRef::Wild => Tm::Wild,
            TermRef::Top => Tm::Top,
            TermRef::Int(i) => Tm::Int(i),
            TermRef::Str(a) => Tm::Str(v.text(a).to_owned()),
            TermRef::Branded(b) => Tm::Branded(b),
            TermRef::Call(f, args) => Tm::Call(v.text(f).to_owned(), args.map(|a| Tm::of(v, r, a)).collect()),
        }
    }
    /// The term to write, its names interned in `b`.
    pub(super) fn write(&self, b: &mut Builder<'_>) -> T<'static> {
        match self {
            Tm::Var(x) => T::VarAtom(b.atom(x)),
            Tm::Wild => T::Wild,
            Tm::Top => T::Top,
            Tm::Int(i) => T::Int(*i),
            Tm::Str(s) => T::StrAtom(b.atom(s)),
            Tm::Branded(x) => T::Branded(*x),
            Tm::Call(f, args) => T::CallAtom(b.atom(f), args.iter().map(|a| a.write(b)).collect()),
        }
    }
    fn vars(&self, out: &mut BTreeSet<String>) {
        match self {
            Tm::Var(x) => {
                out.insert(x.clone());
            }
            Tm::Call(_, args) => args.iter().for_each(|a| a.vars(out)),
            _ => {}
        }
    }
    fn is_const(&self) -> bool {
        matches!(self, Tm::Int(_) | Tm::Str(_) | Tm::Branded(_))
    }
    /// Each variable `to` names renamed at once.
    fn renamed(&self, to: &dyn Fn(&str) -> Option<String>) -> Tm {
        match self {
            Tm::Var(x) => Tm::Var(to(x).unwrap_or_else(|| x.clone())),
            Tm::Call(f, args) => Tm::Call(f.clone(), args.iter().map(|a| a.renamed(to)).collect()),
            t => t.clone(),
        }
    }
}

/// A term of `v`'s rule `r` to write as it is, under `v`'s ids (`program_image::t_of`, with no borrow).
fn own(v: &Version, r: RuleRef<'_>, t: TermRef<'_>) -> T<'static> {
    match t {
        TermRef::Var(s) => T::VarAtom(r.var(s)),
        TermRef::Wild => T::Wild,
        TermRef::Top => T::Top,
        TermRef::Int(i) => T::Int(i),
        TermRef::Str(a) => T::StrAtom(a),
        TermRef::Branded(b) => T::Branded(b),
        TermRef::Call(f, args) => T::CallAtom(f, args.map(|a| own(v, r, a)).collect()),
    }
}

/// A literal a pass makes: an atom or a comparison.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Made {
    Atom { rel: String, args: Vec<Tm>, neg: bool, gated: bool },
    Cmp { l: Tm, op: CmpOp, r: Tm },
}

impl Made {
    fn write<'b, 'p>(&self, w: RuleBuilder<'b, 'p>) -> RuleBuilder<'b, 'p> {
        let mut w = w;
        match self {
            Made::Atom { rel, args, neg, gated } => {
                let ts: Vec<T<'static>> = args.iter().map(|t| t.write(w.builder())).collect();
                let rel = w.builder().rel(rel);
                w.atom_with(rel, &ts, *neg, *gated)
            }
            Made::Cmp { l, op, r } => {
                let (l, r) = (l.write(w.builder()), r.write(w.builder()));
                w.cmp(l, *op, r)
            }
        }
    }
    fn vars(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        match self {
            Made::Atom { args, .. } => args.iter().for_each(|t| t.vars(&mut out)),
            Made::Cmp { l, r, .. } => {
                l.vars(&mut out);
                r.vars(&mut out);
            }
        }
        out
    }
}

/// A literal of `v`'s rule `r` by text, where it is an atom or a comparison (`Made`'s form, to compare).
fn made_of(v: &Version, r: RuleRef<'_>, l: LitRef<'_>) -> Option<Made> {
    match l.view() {
        Lit::Atom(a) => Some(Made::Atom { rel: v.rel_name(a.rel()).to_owned(), args: a.args().map(|t| Tm::of(v, r, t)).collect(), neg: a.neg(), gated: a.gated() }),
        Lit::Cmp { l, op, r: rt } => Some(Made::Cmp { l: Tm::of(v, r, l), op, r: Tm::of(v, r, rt) }),
        _ => None,
    }
}

/// Whether literals `a` and `b` of one rule are equal: the same nodes.
fn same_lit(r: RuleRef<'_>, a: LitRef<'_>, b: LitRef<'_>) -> bool {
    r.image().same_nodes(a.node(), b.node())
}

fn same_atom(r: RuleRef<'_>, a: AtomRef<'_>, b: AtomRef<'_>) -> bool {
    r.image().same_nodes(a.node(), b.node())
}

/// Whether terms `x` and `y` of one rule are equal.
fn same_term(x: TermRef<'_>, y: TermRef<'_>) -> bool {
    match (x, y) {
        (TermRef::Var(a), TermRef::Var(b)) => a == b,
        (TermRef::Wild, TermRef::Wild) | (TermRef::Top, TermRef::Top) => true,
        (TermRef::Int(a), TermRef::Int(b)) => a == b,
        (TermRef::Str(a), TermRef::Str(b)) => a == b,
        (TermRef::Call(f, xs), TermRef::Call(g, ys)) => f == g && xs.clone().count() == ys.clone().count() && xs.zip(ys).all(|(a, b)| same_term(a, b)),
        _ => false,
    }
}

fn term_slots(t: TermRef<'_>, out: &mut BTreeSet<Slot>) {
    match t {
        TermRef::Var(s) => {
            out.insert(s);
        }
        TermRef::Call(_, args) => args.for_each(|a| term_slots(a, out)),
        _ => {}
    }
}

/// The variables literal `l` names.
fn lit_slots(l: LitRef<'_>) -> BTreeSet<Slot> {
    let mut out = BTreeSet::new();
    match l.view() {
        Lit::Atom(a) => a.args().for_each(|t| term_slots(t, &mut out)),
        Lit::Cmp { l, r, .. } => {
            term_slots(l, &mut out);
            term_slots(r, &mut out);
        }
        Lit::Agg { var, over, atom, .. } => {
            atom.args().for_each(|t| term_slots(t, &mut out));
            out.insert(var);
            out.extend(over);
        }
        Lit::Nearest { n, a, .. } => {
            out.insert(n);
            out.insert(a);
        }
        Lit::Ast { document, node, captures, .. } => {
            out.insert(document);
            out.insert(node);
            out.extend(captures.map(|(_, s)| s));
        }
    }
    out
}

fn atom_slots(a: AtomRef<'_>) -> BTreeSet<Slot> {
    let mut out = BTreeSet::new();
    a.args().for_each(|t| term_slots(t, &mut out));
    out
}

fn head_slots(r: RuleRef<'_>) -> BTreeSet<Slot> {
    atom_slots(r.head())
}

/// Rewrites each variable `to` names to its term (at every site but where `only` leaves it), and `@doc` of a
/// constant node to the node's document where `doc` is set.
#[derive(Default)]
struct Subst {
    to: HashMap<Slot, T<'static>>,
    /// The constants the variables `to` names (for `@doc`'s folding).
    ints: HashMap<Slot, i64>,
    doc: Option<AtomId>,
    /// Literals left out, by node.
    drop: HashSet<u32>,
    /// Atoms (by node) whose top-level variables named here are made `_`, and those variables.
    wild_atoms: HashSet<u32>,
    wild: HashSet<Slot>,
    /// Whether the head is rewritten too (`fold_constants` leaves it as it is).
    head: bool,
}

impl Rewrite for Subst {
    fn term(&mut self, site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
        if matches!(site, Site::Head(_)) && !self.head {
            return None;
        }
        if let Site::Atom(a) = site
            && self.wild_atoms.contains(&a.node())
        {
            // Top-level arguments only: a call is written as it is.
            return match t {
                TermRef::Var(s) if self.wild.contains(&s) => Some(T::Wild),
                _ => None,
            };
        }
        match t {
            TermRef::Var(s) => self.to.get(&s).cloned(),
            TermRef::Call(f, mut args) if Some(f) == self.doc && args.clone().count() == 1 => match args.next() {
                Some(TermRef::Int(n)) => Some(T::Int(n >> 32)),
                Some(TermRef::Var(s)) => self.ints.get(&s).map(|n| T::Int(n >> 32)),
                _ => None,
            },
            _ => None,
        }
    }
    fn keep(&mut self, l: LitRef<'_>) -> bool {
        !self.drop.contains(&l.node())
    }
}

/// Renames variables (by name) in atoms, comparisons and the head; other literals as they are.
struct Names<'a> {
    v: &'a Version,
    r: RuleRef<'a>,
    to: &'a dyn Fn(&str) -> Option<String>,
    ids: HashMap<String, AtomId>,
}

impl<'a> Names<'a> {
    fn new(v: &'a Version, r: RuleRef<'a>, to: &'a dyn Fn(&str) -> Option<String>, b: &mut Builder<'_>) -> Names<'a> {
        let mut ids = HashMap::new();
        for a in r.vars() {
            if let Some(n) = to(v.text(a)) {
                let id = b.atom(&n);
                ids.insert(v.text(a).to_owned(), id);
            }
        }
        Names { v, r, to, ids }
    }
}

impl Rewrite for Names<'_> {
    fn term(&mut self, _site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
        let _ = self.to;
        match t {
            TermRef::Var(s) => self.ids.get(self.v.text(self.r.var(s))).map(|&a| T::VarAtom(a)),
            _ => None,
        }
    }
}

/// Runs a rule pass over every rule, the rules kept in their order (a derivation, as the tree's passes keep it).
pub struct InOrder<P>(pub P);

impl<P: RulePass> ProgramPass for InOrder<P> {
    const NAME: &'static str = P::NAME;
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        w.derive()?;
        w.apply(cx, &mut self.0, &Select::All)
    }
}

/// Writes rule `r`'s head and each literal `body` lists, in order: its own (copied, through `rw` where given) or
/// made.
pub(super) enum Out<'v> {
    Own(LitRef<'v>),
    Made(Made),
}

pub(super) fn write_rule(out: &mut RuleOut<'_, '_>, v: &Version, r: RuleRef<'_>, body: &[Out<'_>], rw: Option<&mut dyn RewriteDyn>) -> Result<Step> {
    let mut w = out.rule();
    match rw {
        None => {
            w = w.head_from(v, r, r.head());
            for o in body {
                w = match o {
                    Out::Own(l) => w.lit(v, r, *l),
                    Out::Made(m) => m.write(w),
                };
            }
        }
        Some(rw) => {
            let mut rw = Dyn(rw);
            w = w.head_with(v, r, r.head(), &mut rw);
            for o in body {
                w = match o {
                    Out::Own(l) => w.lit_with(v, r, *l, &mut rw),
                    Out::Made(m) => m.write(w),
                };
            }
        }
    }
    w.finish()?;
    Ok(Step::Written)
}

/// A `Rewrite` behind a reference.
pub(super) trait RewriteDyn {
    fn term_dyn(&mut self, site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>>;
}
impl<R: Rewrite> RewriteDyn for R {
    fn term_dyn(&mut self, site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
        self.term(site, t)
    }
}
struct Dyn<'a>(&'a mut dyn RewriteDyn);
impl Rewrite for Dyn<'_> {
    fn term(&mut self, site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
        self.0.term_dyn(site, t)
    }
}

// ---- what keeps a relation ----

/// Outputs, inputs, lattices, natives and what they read, relations with facts, demand, gates, and
/// an attribute's request.
fn kept(f: &Fields<'_>, rel: &str) -> bool {
    f.is_output(rel) || f.is_input(rel) || f.lattice(rel) || f.native(rel).is_some() || f.natives_read().contains(rel) || f.has_facts(rel) || f.demanded(rel) || f.gate(rel) || request(rel)
}

/// An attribute's request (decision 0174) is read by name (`syntax::VALUE_REQ`), so neither
/// inlined nor merged into another relation: inlined into its one reader, a request over one node left `value` empty.
///
/// An attribute's request (decision 0174): read by name, so neither inlined nor merged into another relation. The
/// fact layer answers `value` only where `rr_value_req` is derived (`syntax::VALUE_REQ`); inlined into its one
/// reader, a request over one node (`rr_value_req($n) :- node(_, $n, "number", _, _)`) left `value` empty.
fn request(rel: &str) -> bool {
    crate::code_rule_dsl::logic::rr::lower::ON_REQUEST.iter().flat_map(|round| round.iter()).any(|a| a.req == rel)
}

/// Inputs, natives, lattices, demand, gates, relations the program qualifies, and an attribute's
/// request.
///
/// The program's own choice of what it keeps (`.decl r(…) inline`, decision 0099) stays with its name.
///
/// Whether a relation's rules may not change: outputs, inputs, lattices, relations with facts, demanded relations and
/// gates.
/// A relation the merge leaves as it is: an input, a lattice, demanded, a gate. (An output merges, and keeps its name
/// as a copy of its class.)
fn fixed(f: &Fields<'_>, rel: &str) -> bool {
    f.is_input(rel) || f.native(rel).is_some() || f.lattice(rel) || f.demanded(rel) || f.gate(rel) || !f.qualifiers_of(rel).is_empty() || request(rel)
}

fn fields<'v>(cx: &Cx<'v>) -> Result<(&'v Arc<Version>, Arc<FieldsData>)> {
    Ok((cx.v(), cx.get::<FieldsData>()?))
}

// ---- the passes, in `simplify`'s order ----

/// A derived relation of no columns has one, `1` where it is made and `_` where read.
///
/// Replaces each variable the body equates to a constant (`$v = "s"`, `$v = 3`) by the constant, in the head and
/// every literal, and drops the equation. Returns how many variables went.
/// Gives each derived relation of no columns one integer column (above).
#[derive(Default)]
pub struct UnitColumns;

impl ProgramPass for UnitColumns {
    const NAME: &'static str = "unit columns";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let (v, d) = fields(cx)?;
        let f = Fields::over(v, &d);
        let units: BTreeSet<String> = f.decls().iter().filter(|d| d.arity() == 0 && !f.is_input(v.text(d.name()))).map(|d| v.text(d.name()).to_owned()).collect();
        if units.is_empty() {
            return Ok(());
        }
        struct Units<'a> {
            v: &'a Version,
            units: &'a BTreeSet<String>,
            head: u32,
        }
        impl Rewrite for Units<'_> {
            fn begin(&mut self, r: RuleRef<'_>) -> bool {
                self.head = r.head().node();
                std::iter::once(r.head()).chain(r.body().filter_map(|l| l.atom())).any(|a| a.arity() == 0 && self.units.contains(self.v.rel_name(a.rel())))
            }
            fn prefix(&mut self, a: AtomRef<'_>, out: &mut Vec<T<'static>>) {
                if a.arity() == 0 && self.units.contains(self.v.rel_name(a.rel())) {
                    out.push(if a.node() == self.head { T::Int(1) } else { T::Wild });
                }
            }
        }
        let input = w.input().clone();
        let b = w.derive()?;
        b.rewrite_all(&input, &mut Units { v: &input, units: &units, head: 0 })?;
        for rel in &units {
            b.declare(rel, &[("unit", "int")]);
            if let Some(rows) = f.facts(rel) {
                let n = rows.rows().len();
                b.set_fact_cells(rel, vec![vec![Cell::Int(1)]; n]);
            }
        }
        Ok(())
    }
}

/// A term's constant as a fact cell.
fn cell_of(t: TermRef<'_>) -> Option<Cell> {
    match t {
        TermRef::Int(i) => Some(Cell::Int(i)),
        TermRef::Str(a) => Some(Cell::Str(a)),
        _ => None,
    }
}

/// A rule of constants alone is facts. How many rules it replaced.
#[derive(Default)]
pub struct GroundRules(pub usize);

impl ProgramPass for GroundRules {
    const NAME: &'static str = "ground rules";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let (v, d) = fields(cx)?;
        let f = Fields::over(v, &d);
        // Per relation, its rows as the facts and the rules make them, by text.
        let mut facts: BTreeMap<String, Vec<Vec<Value>>> = BTreeMap::new();
        let mut gone: HashSet<crate::code_rule_dsl::logic::program_image::RuleId> = HashSet::new();
        for r in v.rules() {
            if r.body().count() == 0 || f.lattice(v.rel_name(r.head().rel())) {
                continue;
            }
            // The body's bindings, or None when it is not equations of constants alone; Some(None) when two disagree.
            let mut binds: HashMap<Slot, Value> = HashMap::new();
            let mut consistent = true;
            let all = r.body().all(|l| {
                let (s, c) = match l.view() {
                    Lit::Cmp { l: TermRef::Var(s), op: CmpOp::Eq, r: c } | Lit::Cmp { l: c, op: CmpOp::Eq, r: TermRef::Var(s) } => match Value::of(v, c) {
                        Some(c) => (s, c),
                        None => return false,
                    },
                    _ => return false,
                };
                match binds.get(&s) {
                    Some(old) if *old != c => consistent = false,
                    _ => {
                        binds.insert(s, c);
                    }
                }
                true
            });
            if !all {
                continue;
            }
            let value = |t: TermRef<'_>| -> Option<Value> {
                match t {
                    TermRef::Var(s) => binds.get(&s).cloned(),
                    TermRef::Call(fa, mut args) if v.text(fa) == "cat" && args.clone().count() == 2 => {
                        let part = |t: TermRef<'_>| match t {
                            TermRef::Var(s) => binds.get(&s).cloned(),
                            _ => Value::of(v, t),
                        };
                        let (a, b) = (args.next()?, args.next()?);
                        match (part(a)?, part(b)?) {
                            // `@dot("", y)` is `y` (`core.rr`).
                            (Value::Str(m), Value::Str(x)) => Some(Value::Str(if m.is_empty() { x } else { format!("{m}.{x}") })),
                            _ => None,
                        }
                    }
                    _ => Value::of(v, t),
                }
            };
            let Some(row) = r.head().args().map(value).collect::<Option<Vec<Value>>>() else { continue };
            let rel = v.rel_name(r.head().rel());
            if consistent {
                let rows = facts.entry(rel.to_owned()).or_insert_with(|| f.facts(rel).map(|x| x.rows().map(|row| row.map(|c| Value::cell(v, c)).collect()).collect()).unwrap_or_default());
                if !rows.contains(&row) {
                    rows.push(row);
                }
            }
            gone.insert(r.id());
        }
        if gone.is_empty() {
            return Ok(());
        }
        self.0 += gone.len();
        w.derive()?;
        let input = w.input().clone();
        let b = w.derive()?;
        for r in input.rules() {
            if !gone.contains(&r.id()) {
                b.keep(&input, r);
            }
        }
        for (rel, rows) in facts {
            let cells = rows.into_iter().map(|row| row.into_iter().map(|x| x.write(b)).collect()).collect();
            b.set_fact_cells(&rel, cells);
        }
        Ok(())
    }
}

/// A fact's value, by text.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Value {
    Int(i64),
    Str(String),
    Branded(crate::code_rule_dsl::logic::front::Branded),
}

impl Value {
    fn of(v: &Version, t: TermRef<'_>) -> Option<Value> {
        match t {
            TermRef::Int(i) => Some(Value::Int(i)),
            TermRef::Str(a) => Some(Value::Str(v.text(a).to_owned())),
            TermRef::Branded(b) => Some(Value::Branded(b)),
            _ => None,
        }
    }
    fn cell(v: &Version, c: Cell) -> Value {
        match c {
            Cell::Int(i) => Value::Int(i),
            Cell::Str(a) => Value::Str(v.text(a).to_owned()),
            Cell::Branded(b) => Value::Branded(b),
        }
    }
    fn write(&self, b: &mut Builder<'_>) -> Cell {
        match self {
            Value::Int(i) => Cell::Int(*i),
            Value::Str(s) => Cell::Str(b.atom(s)),
            Value::Branded(x) => Cell::Branded(*x),
        }
    }
}

/// A test of stated facts alone is decided before the run. How many tests and rules
/// it decided.
///
/// Facts stated: a relation of none may be filled from outside the program (a later phase's input).
///
/// Rules whose bodies are only `$v = constant` equations, made facts (above). Returns the rules replaced.
/// facts and no rules (`guest_policy("library", "off")`, a library's setting) holds or not. A positive one that holds,
/// or a negated one that does not, is dropped from its body; a rule with one that cannot hold goes. The rows are the
/// facts only when some are stated and no rule, input or engine relation adds to them.
#[derive(Default)]
pub struct DecidedFactTests(pub usize);

impl ProgramPass for DecidedFactTests {
    const NAME: &'static str = "decided fact tests";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let (v, d) = fields(cx)?;
        let f = Fields::over(v, &d);
        let derived: HashSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).collect();
        let stated = |rel: &str| f.has_facts(rel) && !derived.contains(rel) && !fixed(&f, rel) && !(f.is_input(rel) || f.native(rel).is_some());
        // Per rule: `None` where it goes, else the literals that go.
        let mut decided: Vec<Option<HashSet<u32>>> = Vec::new();
        let mut any = false;
        for r in v.rules() {
            let mut drop = HashSet::new();
            let mut holds = true;
            for l in r.body() {
                let Lit::Atom(a) = l.view() else { continue };
                let rel = v.rel_name(a.rel());
                if !stated(rel) {
                    continue;
                }
                let Some(row) = a.args().map(cell_of).collect::<Option<Vec<Cell>>>() else { continue };
                let present = f.facts(rel).is_some_and(|x| x.rows().any(|r| r.eq(row.iter().copied())));
                if present != a.neg() {
                    drop.insert(l.node());
                } else {
                    holds = false;
                    break;
                }
            }
            any |= !holds || !drop.is_empty();
            decided.push(holds.then_some(drop));
        }
        if !any {
            return Ok(());
        }
        // A relation whose every rule went is empty (it had rules, so nothing outside fills it): a positive read of it
        // cannot hold, and its rule goes; a negated one holds, and is dropped. To a fixpoint, as each emptied relation
        // may empty its readers (the tree's `decided_fact_tests`).
        let rules: Vec<RuleRef<'_>> = v.rules().collect();
        let mut alive: Vec<bool> = decided.iter().map(Option::is_some).collect();
        self.0 += alive.iter().filter(|a| !**a).count();
        let mut drops: Vec<HashSet<u32>> = decided.into_iter().map(Option::unwrap_or_default).collect();
        self.0 += drops.iter().map(HashSet::len).sum::<usize>();
        let live = |alive: &[bool]| -> BTreeSet<String> { rules.iter().zip(alive).filter(|(_, a)| **a).map(|(r, _)| v.rel_name(r.head().rel()).to_owned()).collect() };
        let empty_among = |alive: &[bool], among: BTreeSet<String>| -> BTreeSet<String> {
            let live = live(alive);
            among.into_iter().filter(|r| !live.contains(r) && !f.has_facts(r) && !fixed(&f, r) && !(f.is_input(r) || f.native(r).is_some())).collect()
        };
        let mut emptied = empty_among(&alive, derived.iter().map(|r| (*r).to_owned()).collect());
        while !emptied.is_empty() {
            let before = live(&alive);
            for (i, r) in rules.iter().enumerate() {
                if !alive[i] {
                    continue;
                }
                let reads = |l: LitRef<'_>, neg: bool| !drops[i].contains(&l.node()) && matches!(l.view(), Lit::Atom(a) if a.neg() == neg && emptied.contains(v.rel_name(a.rel())));
                if r.body().any(|l| reads(l, false)) {
                    alive[i] = false;
                    self.0 += 1;
                    continue;
                }
                let negated: Vec<u32> = r.body().filter(|&l| reads(l, true)).map(|l| l.node()).collect();
                self.0 += negated.len();
                drops[i].extend(negated);
            }
            emptied = empty_among(&alive, before);
        }
        let input = w.input().clone();
        let b = w.derive()?;
        for ((r, keep), drop) in input.rules().zip(alive).zip(drops) {
            if !keep {
                continue;
            }
            if drop.is_empty() {
                b.keep(&input, r);
            } else {
                b.rewrite(&input, r, &mut Subst { drop, ..Default::default() }).finish()?;
            }
        }
        Ok(())
    }
}

/// The counts of `r` that only test existence, by variable name in name order.
fn existence_counts(v: &Version, r: RuleRef<'_>) -> Vec<Slot> {
    let mut out: BTreeMap<&str, Slot> = BTreeMap::new();
    let head = head_slots(r);
    for l in r.body() {
        let Lit::Agg { var, op: AggRef::Count, .. } = l.view() else { continue };
        if head.contains(&var) {
            continue;
        }
        let is = |t: TermRef<'_>| matches!(t, TermRef::Var(s) if s == var);
        let zero = |t: TermRef<'_>| matches!(t, TermRef::Int(0));
        let only_zero = r.body().all(|m| match m.view() {
            Lit::Agg { var: x, .. } if x == var => true,
            Lit::Cmp { l, op, r: rt } => {
                let mut named = BTreeSet::new();
                term_slots(l, &mut named);
                term_slots(rt, &mut named);
                !named.contains(&var) || (((is(l) && zero(rt)) || (is(rt) && zero(l))) && matches!(op, CmpOp::Eq | CmpOp::Ne | CmpOp::Gt | CmpOp::Lt))
            }
            _ => !lit_slots(m).contains(&var),
        });
        if only_zero {
            out.insert(v.text(r.var(var)), var);
        }
    }
    out.into_values().collect()
}

/// An existence count is a negation (`= 0`) or a read (`> 0`).
///
/// **An existence count is a negation** (decision 0099 item 2). `$c = count { r(…) }` whose `$c` is named only in
/// comparisons with 0 asks whether a row exists: `$c = 0` is `!r(…)`, and `$c > 0` (`!=`, `<>`, `0 <`) is `r(…)`. The
/// count's own variables are `_` already (lowering makes them so), so the atom names only the rule's variables and
/// constants, as a negation must. Read as the atom, demand, inlining and projection see a plain read; a count over a
/// negated atom, or compared both ways, stays. Returns the counts replaced.
#[derive(Default)]
pub struct ExistenceAsReads(pub usize);

impl RulePass for ExistenceAsReads {
    const NAME: &'static str = "existence counts as reads";
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        let counts = existence_counts(v, r);
        if counts.is_empty() {
            return Ok(Step::Keep);
        }
        // The body as it becomes: own literals, and counts read as their atom (with the sign).
        #[derive(Clone, Copy)]
        enum B<'v> {
            Own(LitRef<'v>),
            Read(AtomRef<'v>, bool),
        }
        let mut body: Vec<B<'_>> = r.body().map(B::Own).collect();
        let mut done = 0;
        for var in counts {
            let is = |t: TermRef<'_>| matches!(t, TermRef::Var(s) if s == var);
            let zero = |t: TermRef<'_>| matches!(t, TermRef::Int(0));
            // Per comparison of `$c`: `Some(true)` none exists, `Some(false)` one does, `None` anything else.
            let mut none = BTreeSet::new();
            for b in &body {
                let B::Own(l) = b else { continue };
                let Lit::Cmp { l, op, r: rt } = l.view() else { continue };
                let class = match (is(l) && zero(rt), is(rt) && zero(l), op) {
                    (true, _, CmpOp::Eq) | (_, true, CmpOp::Eq) => Some(true),
                    (true, _, CmpOp::Ne | CmpOp::Gt) | (_, true, CmpOp::Ne | CmpOp::Lt) => Some(false),
                    (false, false, _) => continue,
                    _ => None,
                };
                none.insert(class);
            }
            let classes: Vec<Option<bool>> = none.into_iter().collect();
            let negated = match classes.as_slice() {
                [Some(n)] => *n,
                _ => continue,
            };
            let Some(at) = body.iter().position(|b| matches!(b, B::Own(l) if matches!(l.view(), Lit::Agg { var: x, op: AggRef::Count, atom, .. } if x == var && !atom.neg()))) else { continue };
            let B::Own(l) = body.remove(at) else { unreachable!("a count") };
            let Lit::Agg { atom, .. } = l.view() else { unreachable!("a count") };
            body.retain(|b| !matches!(b, B::Own(l) if matches!(l.view(), Lit::Cmp { l, r: rt, .. } if is(l) || is(rt))));
            body.insert(at, B::Read(atom, negated));
            done += 1;
        }
        if done == 0 {
            return Ok(Step::Keep);
        }
        self.0 += done;
        let mut w = out.rule().head_from(v, r, r.head());
        for b in &body {
            w = match *b {
                B::Own(l) => w.lit(v, r, l),
                B::Read(a, neg) => {
                    let args: Vec<T<'static>> = a.args().map(|t| own(v, r, t)).collect();
                    w.atom_with(a.rel(), &args, neg, a.gated())
                }
            };
        }
        w.finish()?;
        Ok(Step::Written)
    }
}

/// A relation of several rules each `node(_, $n, _, _, _), !x_i($n)` is one rule,
/// `node(_, $n, _, _, _), !r_all($n)`, with `r_all($n) :- x_1($n), …, x_k($n)`.
///
/// failing one of its parts) is one rule, `node(_, $n, _, _, _), !r_all($n)`, with `r_all($n) :- x_1($n), …, x_k($n)`
/// (De Morgan): a test of one node, then inlined. `r_all` holds the nodes passing every part, a join of relations
/// that are each a test already, not every node of the chunk.
#[derive(Default)]
pub struct JoinComplements;

impl ProgramPass for JoinComplements {
    const NAME: &'static str = "join complements";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let (v, d) = fields(cx)?;
        let f = Fields::over(v, &d);
        let mut by_rel: BTreeMap<&str, Vec<Option<&str>>> = BTreeMap::new();
        for r in v.rules() {
            let head = v.rel_name(r.head().rel());
            let body: Vec<LitRef<'_>> = r.body().collect();
            let h = match r.head().args().collect::<Vec<_>>().as_slice() {
                [TermRef::Var(h)] => Some(*h),
                _ => None,
            };
            let negated = match (h, body.as_slice()) {
                (Some(h), [n, x]) => match (n.view(), x.view()) {
                    (Lit::Atom(node), Lit::Atom(x))
                        if !node.neg()
                            && v.rel_name(node.rel()) == "node"
                            && node.arity() == 5
                            && node.args().enumerate().all(|(i, t)| if i == 1 { matches!(t, TermRef::Var(s) if s == h) } else { matches!(t, TermRef::Wild) })
                            && x.neg()
                            && !x.gated()
                            && v.rel_name(x.rel()) != head
                            && matches!(x.args().collect::<Vec<_>>().as_slice(), [TermRef::Var(s)] if *s == h) =>
                    {
                        Some(v.rel_name(x.rel()))
                    }
                    _ => None,
                },
                _ => None,
            };
            by_rel.entry(head).or_default().push(negated);
        }
        let mut declared: HashSet<String> = f.decls().iter().map(|d| v.text(d.name()).to_owned()).collect();
        let mut joined: Vec<(&str, String, Vec<&str>)> = Vec::new();
        for (rel, parts) in by_rel {
            if parts.len() < 2 || kept(&f, rel) {
                continue;
            }
            let Some(parts) = parts.into_iter().collect::<Option<Vec<&str>>>() else { continue };
            let all = format!("{rel}_all");
            if declared.contains(&all) {
                continue;
            }
            declared.insert(all.clone());
            joined.push((rel, all, parts));
        }
        if joined.is_empty() {
            return Ok(());
        }
        let gone: HashSet<&str> = joined.iter().map(|(r, _, _)| *r).collect();
        let input = w.input().clone();
        let b = w.derive()?;
        for r in input.rules() {
            if !gone.contains(input.rel_name(r.head().rel())) {
                b.keep(&input, r);
            }
        }
        for (rel, all, parts) in &joined {
            let n = [T::Var("n")];
            let mut wr = b.rule().head(all.as_str(), &n);
            for p in parts {
                wr = wr.atom(*p, &n);
            }
            wr.finish()?;
            b.rule().head(*rel, &n).atom("node", &[T::Wild, T::Var("n"), T::Wild, T::Wild, T::Wild]).not(all.as_str(), &n).finish()?;
            b.declare(all, &[("n", "node")]);
        }
        Ok(())
    }
}

/// A literal as an inlining writes it: one of rule `owner`'s, its variables renamed by `to` (by name).
#[derive(Clone)]
struct Placed<'v> {
    owner: RuleRef<'v>,
    lit: LitRef<'v>,
    to: Option<Arc<HashMap<String, String>>>,
}

/// The name variable `s` of `p`'s rule has where `p` is written.
fn placed_name(v: &Version, p: &Placed<'_>, s: Slot) -> String {
    let n = v.text(p.owner.var(s));
    p.to.as_ref().and_then(|m| m.get(n).cloned()).unwrap_or_else(|| n.to_owned())
}

/// Writes literal `p` into `w`, its variables renamed.
fn write_placed<'b, 'p>(v: &Version, w: RuleBuilder<'b, 'p>, p: &Placed<'_>) -> RuleBuilder<'b, 'p> {
    match &p.to {
        None => w.lit(v, p.owner, p.lit),
        Some(m) => {
            let mut w = w;
            let to = |x: &str| m.get(x).cloned();
            let mut rw = Names::new(v, p.owner, &to, w.builder());
            w.lit_with(v, p.owner, p.lit, &mut rw)
        }
    }
}

/// Each one-rule test of one node read only positively, inlined into its readers.
/// The relations inlined.
///
/// Read only as a positive atom over a variable, by rules of other relations.
///
/// A candidate's body may read another: inline readers in turn until none reads one (each pass removes a level).
///
/// Each read in turn: an inlined body may read another candidate, read on the next pass.
///
/// Inlines each one-rule test of one node read only positively (the module's list); returns the relations inlined.
#[derive(Default)]
pub struct InlineNodeTests(pub Vec<String>);

/// `Program::inlined_readers` and `replace_readers` on the version (decision 0331): in the request record, each
/// relation `gone` that a pass inlines into its readers is replaced by the relations whose rules read it (through
/// other relations `gone`), as `v` holds them before the pass. A request of an inlined relation serves its readers.
/// A request relation inlined away has no rules left, so no entry (`Program::set_rules_from` keeps none).
fn replace_inlined_readers(v: &Version, b: &mut Builder<'_>, gone: &BTreeSet<&str>) {
    if gone.is_empty() || v.asked().is_empty() {
        return;
    }
    let mut direct: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for r in v.rules() {
        for l in r.body() {
            if let Some(a) = l.atom()
                && gone.contains(v.rel_name(a.rel()))
            {
                direct.entry(v.rel_name(a.rel())).or_default().insert(v.rel_name(r.head().rel()));
            }
        }
    }
    let inlined: BTreeMap<&str, BTreeSet<&str>> = gone
        .iter()
        .map(|g| {
            let (mut out, mut seen, mut todo) = (BTreeSet::new(), BTreeSet::from([*g]), vec![*g]);
            while let Some(x) = todo.pop() {
                for &h in direct.get(x).into_iter().flatten() {
                    if !gone.contains(h) {
                        out.insert(h);
                    } else if seen.insert(h) {
                        todo.push(h);
                    }
                }
            }
            (*g, out)
        })
        .filter(|(_, out)| !out.is_empty())
        .collect();
    // Restated in the record's own order (`Builder::set_asked`), as the tree's image writes it.
    let was = b.asked();
    let mut asked = was.clone();
    asked.retain(|req, _| !gone.contains(req.as_str()));
    for readers in asked.values_mut().flatten() {
        if readers.iter().any(|x| inlined.contains_key(x.as_str())) {
            *readers = std::mem::take(readers).into_iter().flat_map(|x| inlined.get(x.as_str()).map_or_else(|| vec![x.clone()], |to| to.iter().map(|t| (*t).to_owned()).collect())).collect();
        }
    }
    if asked != was {
        b.set_asked(&asked);
    }
}

impl ProgramPass for InlineNodeTests {
    const NAME: &'static str = "inline node tests";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let (v, d) = fields(cx)?;
        let f = Fields::over(v, &d);
        // Each candidate's body and its node variable.
        // Each candidate's bodies, over the variable `$h`.
        let mut bodies: BTreeMap<&str, Vec<(RuleRef<'_>, Slot)>> = BTreeMap::new();
        let mut refused: BTreeSet<&str> = BTreeSet::new();
        for r in v.rules() {
            let rel = v.rel_name(r.head().rel());
            let one = match r.head().args().collect::<Vec<_>>().as_slice() {
                [TermRef::Var(h)] => {
                    let h = *h;
                    let tests_node = r.body().any(|l| matches!(l.view(), Lit::Atom(a) if !a.neg() && v.rel_name(a.rel()) == "node" && matches!(a.args().nth(1), Some(TermRef::Var(s)) if s == h)));
                    // A scope guard's document (`rr_sd0 = @doc($h)`, decision 0091) is a function of the node.
                    let alone = r.body().all(|l| matches!(l.view(), Lit::Atom(_) | Lit::Cmp { .. }) && lit_slots(l).iter().all(|&s| s == h || v.text(r.var(s)).starts_with(GUARD_VAR)));
                    let recursive = r.body().any(|l| matches!(l.view(), Lit::Atom(a) if a.rel() == r.head().rel()));
                    (tests_node && alone && !recursive).then_some(h)
                }
                _ => None,
            };
            match one {
                Some(h) if !kept(&f, rel) => bodies.entry(rel).or_default().push((r, h)),
                _ => {
                    refused.insert(rel);
                }
            }
        }
        for r in v.rules() {
            for l in r.body() {
                match l.view() {
                    Lit::Atom(a) if !a.neg() && a.arity() == 1 && matches!(a.args().next(), Some(TermRef::Var(_))) => {}
                    Lit::Atom(a) | Lit::Agg { atom: a, .. } => {
                        refused.insert(v.rel_name(a.rel()));
                    }
                    _ => {}
                }
            }
        }
        bodies.retain(|rel, rules| !refused.contains(rel) && rules.len() == 1);
        if bodies.is_empty() {
            return Ok(());
        }
        let cand = |p: &Placed<'_>| -> Option<(&str, String)> {
            match p.lit.view() {
                Lit::Atom(a) if !a.neg() && bodies.contains_key(v.rel_name(a.rel())) => match a.args().next() {
                    Some(TermRef::Var(t)) => Some((v.rel_name(a.rel()), placed_name(v, p, t))),
                    _ => None,
                },
                _ => None,
            }
        };
        // Every rule's body as the rounds leave it.
        let mut rules: Vec<(RuleRef<'_>, Vec<Placed<'_>>)> = v.rules().map(|r| (r, r.body().map(|lit| Placed { owner: r, lit, to: None }).collect())).collect();
        // The guard's variables renamed apart: a reader may inline several tests, each with its own.
        let mut fresh = 0usize;
        for _ in 0..8 {
            let mut changed = false;
            for (_, body) in &mut rules {
                let mut at = 0;
                while at < body.len() {
                    let Some((rel, t)) = cand(&body[at]) else {
                        at += 1;
                        continue;
                    };
                    changed = true;
                    fresh += 1;
                    let (def, h) = bodies[rel][0];
                    let mut to: HashMap<String, String> = HashMap::new();
                    for s in 0..def.var_count() as u32 {
                        let s = Slot(s);
                        let name = v.text(def.var(s));
                        if s == h {
                            to.insert(name.to_owned(), t.clone());
                        } else if name.starts_with(GUARD_VAR) {
                            to.insert(name.to_owned(), format!("{name}_t{fresh}"));
                        }
                    }
                    let to = Arc::new(to);
                    let inlined: Vec<Placed<'_>> = def.body().map(|lit| Placed { owner: def, lit, to: Some(to.clone()) }).collect();
                    let len = inlined.len();
                    body.splice(at..=at, inlined);
                    at += len;
                }
            }
            if !changed {
                break;
            }
        }
        // Unfinished (a chain of reads deeper than the rounds): nothing is inlined.
        let reads = |body: &[Placed<'_>]| body.iter().any(|p| matches!(p.lit.view(), Lit::Atom(a) if bodies.contains_key(v.rel_name(a.rel()))));
        // Unfinished (a chain of reads deeper than the passes): nothing is inlined.
        if rules.iter().any(|(r, body)| !bodies.contains_key(v.rel_name(r.head().rel())) && reads(body)) {
            return Ok(());
        }
        let input = w.input().clone();
        let b = w.derive()?;
        for (r, body) in &rules {
            if bodies.contains_key(input.rel_name(r.head().rel())) {
                continue;
            }
            if body.iter().all(|p| p.to.is_none()) && body.len() == r.body().count() {
                b.keep(&input, *r);
                continue;
            }
            let mut wr = b.rule().source(r.id()).head_from(&input, *r, r.head());
            for p in body {
                wr = write_placed(&input, wr, p);
            }
            wr.finish()?;
        }
        for rel in bodies.keys() {
            b.undeclare(rel);
        }
        replace_inlined_readers(v, b, &bodies.keys().copied().collect());
        self.0.extend(bodies.keys().map(|r| (*r).to_owned()));
        Ok(())
    }
}

/// At most this many rules in a filter inlined.
const FILTER_RULES: usize = 2;

/// A base relation a negation may test inside a filter: the functional lookups' relations.
fn base_lookup(rel: &str) -> bool {
    matches!(rel, "docof" | "visit_index" | "node_order" | "kind" | "text" | "name" | "line" | "op" | "node" | "field" | "child")
}

/// A literal an inlined filter writes: a literal placed, or `l = r`, the reader's term `l` (of `reader`) and the
/// definition's renamed (`r`).
#[derive(Clone)]
enum FLit<'v> {
    Placed(Placed<'v>),
    Eq { reader: RuleRef<'v>, l: TermRef<'v>, r: Tm },
}

/// Each filter (a relation whose every rule reads only functional lookups and tests of
/// what its head binds), inlined into its readers, and each computed view where its reader reads its domain. The
/// relations inlined.
///
/// A rule is a filter when every literal is a functional lookup or a test of what its head (and earlier
/// lookups) bind.
///
/// `core.rr`'s relations (`kind`, `precedes`) are the engine's, read by name (kind views, kind sets).
///
/// A computed view is inlined only where its reader reads its domain already (each of its atoms implied by one of
/// the reader's), so the copy adds the function and nothing else.
///
/// Each rule with its place among the program's, so the request record follows its copies (`Program::set_rules_from`).
///
/// Inlines each filter: a relation whose every rule, once its head's variables are bound, reads only functional
/// lookups of them and tests (`gis_before(i, n) :- kind(n, "call"), earlier(i, n)`, lowered to `docof` and
/// `visit_index` of both and a comparison). Built whole it holds every pair the test admits (9.8M on 50 files: every
/// node before every call of its file); inlined, each reader tests its own pairs. A lookup is functional when its
/// key is bound: `docof`, `visit_index`, `kind`, `text`, `name`, `line`, `op` and `node` by the node; `field` and
/// `child` by the child (one parent). Read only positively, with variables or constants, by other relations; at
/// most `FILTER_RULES` rules, each reader becoming one copy per rule. Returns the relations inlined.
#[derive(Default)]
pub struct InlineFilters(pub Vec<String>);

impl ProgramPass for InlineFilters {
    const NAME: &'static str = "inline filters";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let (v, d) = fields(cx)?;
        let f = Fields::over(v, &d);
        let rel_of = |a: AtomRef<'_>| v.rel_name(a.rel());
        let functional = |a: AtomRef<'_>, bound: &BTreeSet<Slot>| -> bool {
            let on = |c: usize| match a.args().nth(c) {
                Some(TermRef::Var(s)) => bound.contains(&s),
                Some(TermRef::Wild) | None => false,
                Some(_) => true,
            };
            match rel_of(a) {
                "docof" | "visit_index" | "node_order" | "kind" | "text" | "name" | "line" | "op" => on(0),
                "node" => on(1),
                "field" => on(2),
                "child" => on(1),
                _ => false,
            }
        };
        // A derived relation read with every variable bound is a test of those values (`er_sym(f, _)`, `!is_async(f)`):
        // the reader reached it, and its negation, through the filter already, so inlining keeps the strata. Only in a
        // rule that also looks up a base relation, which built whole is the cross product (`er_wrapped(f, v, v) :-
        // er_sym(f, _), !is_async(f), kind(v, "call")`: every def with every call, 1.4 GB on 200 files); an alias or an
        // intersection of derived relations (`u19(n) :- m25(n)`) is no bigger than what it reads.
        let derived: BTreeSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).collect();
        let test = |a: AtomRef<'_>, bound: &BTreeSet<Slot>| derived.contains(rel_of(a)) && a.args().all(|t| matches!(t, TermRef::Wild | TermRef::Int(_) | TermRef::Str(_)) || matches!(t, TermRef::Var(s) if bound.contains(&s)));
        let doc = v.find("doc");
        let filter = |r: RuleRef<'_>| -> bool {
            let head_rel = r.head().rel();
            let mut bound = head_slots(r);
            let mut todo: Vec<LitRef<'_>> = r.body().collect();
            loop {
                let before = todo.len();
                todo.retain(|&l| {
                    let ok = match l.view() {
                        Lit::Atom(a) if a.rel() != head_rel && test(a, &bound) => true,
                        Lit::Atom(a) if !a.neg() => functional(a, &bound),
                        Lit::Atom(a) => base_lookup(rel_of(a)) && lit_slots(l).is_subset(&bound),
                        // `g = @doc(n)` (decision 0091's scope guards) is a function of a bound node: a lookup.
                        Lit::Cmp { l: TermRef::Var(_), op: CmpOp::Eq, r: TermRef::Call(fa, xs) } if Some(fa) == doc => xs.into_iter().all(|x| matches!(x, TermRef::Var(s) if bound.contains(&s))),
                        Lit::Cmp { .. } => lit_slots(l).is_subset(&bound),
                        _ => false,
                    };
                    if ok {
                        bound.extend(lit_slots(l));
                    }
                    !ok
                });
                if todo.is_empty() {
                    let tests = r.body().any(|l| matches!(l.view(), Lit::Atom(a) if a.rel() != head_rel && derived.contains(rel_of(a))));
                    let looks_up = r.body().any(|l| matches!(l.view(), Lit::Atom(a) if !a.neg() && base_lookup(rel_of(a))));
                    return !tests || looks_up;
                }
                if todo.len() == before {
                    return false;
                }
            }
        };
        let computed = |rules: &[RuleRef<'_>]| -> bool {
            let [r] = rules else { return false };
            let mut bound = BTreeSet::new();
            for l in r.body() {
                if let Lit::Atom(a) = l.view() {
                    bound.extend(atom_slots(a));
                }
            }
            r.head().args().any(|t| matches!(t, TermRef::Call(..))) && r.body().count() > 0 && r.body().all(|l| matches!(l.view(), Lit::Atom(a) if !a.neg() && a.rel() != r.head().rel())) && head_slots(*r).is_subset(&bound)
        };
        let mut rules_of: BTreeMap<&str, Vec<RuleRef<'_>>> = BTreeMap::new();
        for r in v.rules() {
            rules_of.entry(v.rel_name(r.head().rel())).or_default().push(r);
        }
        let core = crate::code_rule_dsl::logic::rr::core_names();
        let mut candidates: BTreeSet<&str> = rules_of
            .iter()
            .filter(|(rel, rules)| {
                // A request the engine reads (a guest's `embed`, the demand a `parsed` read makes) stays a relation.
                !kept(&f, rel)
                    && !crate::code_rule_dsl::logic::plan::corpus::guests::REQUESTS.contains(rel)
                    && !core.contains(**rel)
                    && rules.len() <= FILTER_RULES
                    && (rules.iter().all(|r| r.head().args().all(|t| matches!(t, TermRef::Var(_) | TermRef::Int(_) | TermRef::Str(_))) && filter(*r)) || computed(rules))
            })
            .map(|(rel, _)| *rel)
            .collect();
        for r in v.rules() {
            for l in r.body() {
                match l.view() {
                    Lit::Atom(a) if !a.neg() && a.args().all(|t| matches!(t, TermRef::Var(_) | TermRef::Int(_) | TermRef::Str(_))) && a.rel() != r.head().rel() => {}
                    Lit::Atom(a) | Lit::Agg { atom: a, .. } => {
                        candidates.remove(rel_of(a));
                    }
                    _ => {}
                }
            }
        }
        // A computed view only where its reader reads its domain already (each of the view's atoms, with the head's variables as the read names them and the others `_`, implied by one of the reader's).
        let domain_read = |def: RuleRef<'_>, reader: RuleRef<'_>, read: AtomRef<'_>| -> bool {
            let mut as_read: HashMap<Slot, Tm> = HashMap::new();
            // A computed column (`@replace_all($raw, …)`) or a constant, once the domain binds its variables. Only the
            // head's side is renamed: the read's term is the reader's, whose variables may share a name with the
            // definition's.
            for (h, t) in def.head().args().zip(read.args()) {
                if let TermRef::Var(s) = h {
                    let t = Tm::of(v, reader, t);
                    if as_read.insert(s, t.clone()).is_some_and(|o| o != t) {
                        return false;
                    }
                }
            }
            def.body().all(|l| {
                let Lit::Atom(a) = l.view() else { return false };
                let want: Vec<Tm> = a
                    .args()
                    .map(|t| match t {
                        TermRef::Var(s) => as_read.get(&s).cloned().unwrap_or(Tm::Wild),
                        t => Tm::of(v, def, t),
                    })
                    .collect();
                // `implies(b, a)`: the reader's atom `b` and the definition's `a` as read.
                !a.neg()
                    && reader.body().any(|m| {
                        matches!(m.view(), Lit::Atom(b) if !b.neg() && rel_of(b) == rel_of(a) && b.arity() == want.len() && b.args().zip(&want).all(|(y, x)| *x == Tm::Wild || Tm::of(v, reader, y) == *x))
                    })
            })
        };
        let mut drop = Vec::new();
        for r in v.rules() {
            for l in r.body() {
                if let Lit::Atom(read) = l.view()
                    && candidates.contains(rel_of(read))
                    && let [def] = rules_of[rel_of(read)].as_slice()
                    && !filter(*def)
                    && !domain_read(*def, r, read)
                {
                    drop.push(rel_of(read));
                }
            }
        }
        for rel in drop {
            candidates.remove(rel);
        }
        // A filter's body reads no other candidate (one level; deeper chains stay built).
        let bodies: BTreeMap<&str, Vec<RuleRef<'_>>> = candidates
            .iter()
            .filter(|rel| rules_of[*rel].iter().all(|r| r.body().all(|l| !matches!(l.view(), Lit::Atom(a) if candidates.contains(rel_of(a))))))
            .map(|rel| (*rel, rules_of[rel].clone()))
            .collect();
        if bodies.is_empty() {
            return Ok(());
        }
        let mut fresh = 0usize;
        let mut out: Vec<(RuleRef<'_>, Vec<FLit<'_>>)> = Vec::new();
        let mut work: Vec<(RuleRef<'_>, Vec<FLit<'_>>)> = v
            .rules()
            .filter(|r| !bodies.contains_key(v.rel_name(r.head().rel())))
            .map(|r| (r, r.body().map(|lit| FLit::Placed(Placed { owner: r, lit, to: None })).collect()))
            .collect();
        work.reverse();
        while let Some((rule, body)) = work.pop() {
            let at = body.iter().position(|x| matches!(x, FLit::Placed(p) if matches!(p.lit.view(), Lit::Atom(a) if !a.neg() && bodies.contains_key(rel_of(a)))));
            let Some(at) = at else {
                out.push((rule, body));
                continue;
            };
            let FLit::Placed(read_p) = body[at].clone() else { unreachable!("a read") };
            let Lit::Atom(read) = read_p.lit.view() else { unreachable!("a read") };
            for &def in &bodies[rel_of(read)] {
                fresh += 1;
                // The definition's variables renamed apart; a head variable read as a variable becomes it, anything else
                // an equality.
                let mut local: BTreeSet<Slot> = head_slots(def);
                def.body().for_each(|l| local.extend(lit_slots(l)));
                let mut to: HashMap<String, String> = local.iter().map(|&s| (v.text(def.var(s)).to_owned(), format!("{}_f{fresh}", v.text(def.var(s))))).collect();
                let mut inlined: Vec<FLit<'_>> = Vec::new();
                for (h, rt) in def.head().args().zip(read.args()) {
                    match (h, rt) {
                        (TermRef::Var(s), TermRef::Var(x)) if to.get(v.text(def.var(s))) == Some(&format!("{}_f{fresh}", v.text(def.var(s)))) => {
                            to.insert(v.text(def.var(s)).to_owned(), placed_name(v, &read_p, x));
                        }
                        (TermRef::Var(s), rt) => inlined.push(FLit::Eq { reader: read_p.owner, l: rt, r: Tm::Var(to[v.text(def.var(s))].clone()) }),
                        _ => {}
                    }
                }
                let to = Arc::new(to);
                inlined.extend(def.body().map(|lit| FLit::Placed(Placed { owner: def, lit, to: Some(to.clone()) })));
                for (h, rt) in def.head().args().zip(read.args()) {
                    if !matches!(h, TermRef::Var(_)) {
                        let m = to.clone();
                        let h = Tm::of(v, def, h).renamed(&|x| m.get(x).cloned());
                        inlined.push(FLit::Eq { reader: read_p.owner, l: rt, r: h });
                    }
                }
                let mut c = body.clone();
                c.splice(at..=at, inlined);
                work.push((rule, c));
            }
        }
        let input = w.input().clone();
        let b = w.derive()?;
        for (r, body) in &out {
            let own = body.len() == r.body().count() && body.iter().all(|x| matches!(x, FLit::Placed(p) if p.to.is_none()));
            if own {
                b.keep(&input, *r);
                continue;
            }
            let mut wr = b.rule().source(r.id()).head_from(&input, *r, r.head());
            for x in body {
                wr = match x {
                    FLit::Placed(p) => write_placed(&input, wr, p),
                    FLit::Eq { reader, l, r: rt } => {
                        let lt = own_t(&input, *reader, *l);
                        let rt = rt.write(wr.builder());
                        wr.cmp(lt, CmpOp::Eq, rt)
                    }
                };
            }
            wr.finish()?;
        }
        for rel in bodies.keys() {
            b.undeclare(rel);
        }
        replace_inlined_readers(v, b, &bodies.keys().copied().collect());
        self.0.extend(bodies.keys().map(|r| (*r).to_owned()));
        Ok(())
    }
}

fn own_t(v: &Version, r: RuleRef<'_>, t: TermRef<'_>) -> T<'static> {
    own(v, r, t)
}

// ---- each rule alone ----

/// The parts of a concatenation (`@concat`), in order: variables and constant texts only.
fn concat_parts(v: &Version, r: RuleRef<'_>, t: TermRef<'_>) -> Option<Vec<Tm>> {
    match t {
        TermRef::Call(f, mut args) if v.text(f) == "concat" && args.clone().count() == 2 => {
            let (a, b) = (args.next()?, args.next()?);
            let mut parts = concat_parts(v, r, a)?;
            parts.extend(concat_parts(v, r, b)?);
            Some(parts)
        }
        TermRef::Var(_) | TermRef::Str(_) => Some(vec![Tm::of(v, r, t)]),
        _ => None,
    }
}

fn concat_of(parts: &[Tm]) -> Tm {
    let mut it = parts.iter().cloned();
    match it.next() {
        None => Tm::Str(String::new()),
        Some(first) => it.fold(first, |acc, p| Tm::Call("concat".into(), vec![acc, p])),
    }
}

/// `$v = cat($a, $x, $b)` gets `$x = @str_between($v, cat(before), cat(after))`.
///
/// `$v = cat($a, $x, $b)` runs backwards: given `$v` and every other part, `$x` is the text between them, and there is
/// none when `$v` does not start and end with them (concatenation is one-to-one in any one part). So each variable
/// part, named once there, gets `$x = @str_between($v, cat(before), cat(after))` beside the concatenation. Whichever
/// the plan binds first, the other is then a test that holds of what it bound: the rule's rows are the same, and a
/// rule that reaches `$v` first (a delta of the relation giving it) binds the parts from it instead of crossing to
/// the relation that gives them.
#[derive(Default)]
pub struct ConcatInverses(pub usize);

impl RulePass for ConcatInverses {
    const NAME: &'static str = "concatenation inverses";
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        let mut added: Vec<Made> = Vec::new();
        let mut have: Option<Vec<Made>> = None;
        for l in r.body() {
            let Lit::Cmp { l: a, op: CmpOp::Eq, r: b } = l.view() else { continue };
            let (x, made) = match (a, b) {
                (TermRef::Var(x), c @ TermRef::Call(f, _)) | (c @ TermRef::Call(f, _), TermRef::Var(x)) if v.text(f) == "concat" => (x, c),
                _ => continue,
            };
            let Some(parts) = concat_parts(v, r, made) else { continue };
            let vname = v.text(r.var(x)).to_owned();
            for (i, part) in parts.iter().enumerate() {
                let Tm::Var(p) = part else { continue };
                if *p == vname || parts.iter().filter(|q| *q == part).count() > 1 {
                    continue;
                }
                let inverse = Made::Cmp { l: Tm::Var(p.clone()), op: CmpOp::Eq, r: Tm::Call("str_between".into(), vec![Tm::Var(vname.clone()), concat_of(&parts[..i]), concat_of(&parts[i + 1..])]) };
                let have = have.get_or_insert_with(|| r.body().filter_map(|l| made_of(v, r, l)).collect());
                if !have.contains(&inverse) && !added.contains(&inverse) {
                    added.push(inverse);
                }
            }
        }
        if added.is_empty() {
            return Ok(Step::Keep);
        }
        self.0 += added.len();
        let body: Vec<Out<'_>> = r.body().map(Out::Own).chain(added.into_iter().map(Out::Made)).collect();
        write_rule(out, v, r, &body, None)
    }
}

/// A constant part of `@cat` is a variable equal to it.
///
/// `$q = @cat("builtins", $x)` (`split_dot(q, "builtins", x)`) as `$q = @cat($k, $x), $k = "builtins"`: `@cat` runs
/// backwards over variables only, and `fold_constants` leaves those in place.
#[derive(Default)]
pub struct CatVariables;

impl RulePass for CatVariables {
    const NAME: &'static str = "cat variables";
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        // Each comparison's sides as written, with its constant parts of `@cat` made variables.
        let mut added: Vec<Made> = Vec::new();
        let mut sides: HashMap<u32, (Tm, CmpOp, Tm)> = HashMap::new();
        for l in r.body() {
            let Lit::Cmp { l: a, op: CmpOp::Eq, r: b } = l.view() else { continue };
            let mut changed = false;
            let mut side = |t: TermRef<'_>| -> Tm {
                match t {
                    TermRef::Call(f, args) if v.text(f) == "cat" && args.clone().count() == 2 => Tm::Call(
                        "cat".into(),
                        args.map(|x| match x {
                            TermRef::Int(_) | TermRef::Str(_) => {
                                let name = format!("cat_k{}", added.len());
                                added.push(Made::Cmp { l: Tm::Var(name.clone()), op: CmpOp::Eq, r: Tm::of(v, r, x) });
                                changed = true;
                                Tm::Var(name)
                            }
                            x => Tm::of(v, r, x),
                        })
                        .collect(),
                    ),
                    t => Tm::of(v, r, t),
                }
            };
            let (a, b) = (side(a), side(b));
            if changed {
                sides.insert(l.node(), (a, CmpOp::Eq, b));
            }
        }
        if added.is_empty() {
            return Ok(Step::Keep);
        }
        let body: Vec<Out<'_>> = r
            .body()
            .map(|l| match sides.remove(&l.node()) {
                Some((a, op, b)) => Out::Made(Made::Cmp { l: a, op, r: b }),
                None => Out::Own(l),
            })
            .chain(added.into_iter().map(Out::Made))
            .collect();
        write_rule(out, v, r, &body, None)
    }
}

/// Whether `t` is `@cat` of variables alone.
fn is_cat(v: &Version, t: TermRef<'_>) -> bool {
    matches!(t, TermRef::Call(f, args) if v.text(f) == "cat" && args.clone().all(|a| matches!(a, TermRef::Var(_))))
}

/// A variable equal to a constant is the constant.
///
/// `$q = @cat($m, $x)` runs backwards (splitting a bound $q) only over variables: a constant folded into
/// it (`split_dot(q, m, x), m = "typing"`) would leave it runnable in neither direction.
///
/// A node's document of a constant node is a constant (`DocOf`: the node's high 32 bits).
#[derive(Default)]
pub struct FoldConstants(pub usize);

impl RulePass for FoldConstants {
    const NAME: &'static str = "fold constants";
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        if !r.body().any(|l| matches!(l.view(), Lit::Cmp { op: CmpOp::Eq, .. })) {
            return Ok(Step::Keep);
        }
        let mut named = head_slots(r);
        for l in r.body() {
            match l.view() {
                Lit::Agg { var, over, .. } => {
                    // Variables the head names, or a literal other than as a term (an aggregate's result, `nearest`, `ast`), stay
                    // variables. A head column the rule fills from a text column is what keeps its relation a string filter, run per
                    // chunk (`typing::file_local`'s split); `t($s) :- doc(_, $s, _), $s = "a.py"` stays as it is.
                    named.insert(var);
                    named.extend(over);
                }
                Lit::Nearest { .. } | Lit::Ast { .. } => named.extend(lit_slots(l)),
                Lit::Cmp { l: a, r: b, .. } => {
                    for t in [a, b] {
                        if is_cat(v, t) {
                            term_slots(t, &mut named);
                        }
                    }
                }
                _ => {}
            }
        }
        // The constants found, in order; a comparison's side read through them.
        let mut to: HashMap<Slot, TermRef<'_>> = HashMap::new();
        let mut gone: HashSet<u32> = HashSet::new();
        fn constant<'v>(t: TermRef<'v>, to: &HashMap<Slot, TermRef<'v>>) -> Option<TermRef<'v>> {
            match t {
                TermRef::Int(_) | TermRef::Str(_) => Some(t),
                TermRef::Var(s) => to.get(&s).copied(),
                _ => None,
            }
        }
        loop {
            let found = r.body().filter(|l| !gone.contains(&l.node())).find_map(|l| match l.view() {
                Lit::Cmp { l: a, op: CmpOp::Eq, r: b } => {
                    let var = |t: TermRef<'_>| match t {
                        TermRef::Var(s) if !to.contains_key(&s) => Some(s),
                        _ => None,
                    };
                    match (var(a), constant(b, &to), var(b), constant(a, &to)) {
                        (Some(s), Some(c), _, _) if !named.contains(&s) => Some((l.node(), s, c)),
                        (_, _, Some(s), Some(c)) if !named.contains(&s) => Some((l.node(), s, c)),
                        _ => None,
                    }
                }
                _ => None,
            });
            let Some((at, s, c)) = found else { break };
            gone.insert(at);
            to.insert(s, c);
        }
        if gone.is_empty() {
            return Ok(Step::Keep);
        }
        self.0 += gone.len();
        let mut rw = Subst { drop: gone, doc: v.find("doc"), ..Default::default() };
        for (s, c) in &to {
            rw.to.insert(*s, own(v, r, *c));
            if let TermRef::Int(n) = c {
                rw.ints.insert(*s, *n);
            }
        }
        // Atoms, aggregates' atoms and comparisons; `nearest` and `ast` as they are (they name no folded variable).
        out.rewrite(&mut rw)
    }
}

/// `node(d, a, _, sa, ea), node(d, b, _, sb, eb), sb <= sa, ea <= eb` reads
/// `span_within(a, b)` in place of the comparisons.
///
/// Replaces each pair `node(d, a, _, sa, ea), node(d, b, _, sb, eb), sb <= sa, ea <= eb` (one document variable
/// `d`, either side of each comparison) by `span_within(a, b)`: the comparisons, not the atoms, which may bind or test
/// more. A join of every pair of a file's nodes becomes a walk of the tree (`syntax::within`). Returns the pairs
/// replaced.
#[derive(Default)]
pub struct SpanContainment(pub usize);

impl RulePass for SpanContainment {
    const NAME: &'static str = "span containment";
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        #[derive(Clone, Copy)]
        enum B<'v> {
            Own(LitRef<'v>),
            Within(Slot, Slot),
        }
        // `x <= y` as written, either way round.
        let var = |t: Option<TermRef<'_>>| match t {
            Some(TermRef::Var(s)) => Some(s),
            _ => None,
        };
        let mut body: Vec<B<'_>> = r.body().map(B::Own).collect();
        // The rule changed: look again for another pair.
        let mut made = 0;
        'again: loop {
            // (d, n, start, end) of each positive node atom with variables there.
            let nodes: Vec<(Slot, Slot, Slot, Slot)> = body
                .iter()
                .filter_map(|b| match b {
                    B::Own(l) => match l.view() {
                        Lit::Atom(a) if !a.neg() && v.rel_name(a.rel()) == "node" && a.arity() == 5 => {
                            let args: Vec<TermRef<'_>> = a.args().collect();
                            Some((var(Some(args[0]))?, var(Some(args[1]))?, var(Some(args[3]))?, var(Some(args[4]))?))
                        }
                        _ => None,
                    },
                    B::Within(..) => None,
                })
                .collect();
            let le = |b: &B<'_>| -> Option<(Slot, Slot)> {
                let B::Own(l) = b else { return None };
                match l.view() {
                    Lit::Cmp { l: a, op: CmpOp::Le, r: c } => Some((var(Some(a))?, var(Some(c))?)),
                    Lit::Cmp { l: a, op: CmpOp::Ge, r: c } => Some((var(Some(c))?, var(Some(a))?)),
                    _ => None,
                }
            };
            for (i, inner) in nodes.iter().enumerate() {
                for (j, outer) in nodes.iter().enumerate() {
                    if i == j || inner.0 != outer.0 || inner.1 == outer.1 {
                        continue;
                    }
                    let starts = body.iter().position(|b| le(b) == Some((outer.2, inner.2)));
                    let ends = body.iter().position(|b| le(b) == Some((inner.3, outer.3)));
                    let (Some(starts), Some(ends)) = (starts, ends) else { continue };
                    body[starts.min(ends)] = B::Within(inner.1, outer.1);
                    body.remove(starts.max(ends));
                    made += 1;
                    continue 'again;
                }
            }
            break;
        }
        if made == 0 {
            return Ok(Step::Keep);
        }
        self.0 += made;
        let mut w = out.rule().head_from(v, r, r.head());
        for b in &body {
            w = match *b {
                B::Own(l) => w.lit(v, r, l),
                B::Within(a, o) => w.atom(SPAN_WITHIN, &[T::VarAtom(r.var(a)), T::VarAtom(r.var(o))]),
            };
        }
        w.finish()?;
        Ok(Step::Written)
    }
}

/// The base relations whose rows are keyed: (relation, arity, key column).
const KEYS: &[(&str, usize, usize)] = &[("node", 5, 1), ("doc", 3, 0), ("kind", 2, 0), ("text", 2, 0), ("name", 2, 0), ("line", 2, 0), ("docof", 2, 0)];

/// Positive atoms of a keyed relation with the same key term are one.
///
/// Inside the head's computed terms too: `rn(n, @lower(t)) :- …, text(n, t)` merged into a
/// `text(n, u)` reads u there, or t is left unbound.
///
/// Merges positive atoms of a keyed relation (`KEYS`) with the same key term into one; returns how many went.
/// Two columns merge where they are equal or one is `_`; two different variables merge by renaming the later one
/// (one the head, atoms and comparisons name, so `rename_var` reaches every use). Anything else stays two atoms.
#[derive(Default)]
pub struct MergeKeyed(pub usize);

impl RulePass for MergeKeyed {
    const NAME: &'static str = "merge keyed atoms";
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        fn keyed<'v>(v: &Version, l: LitRef<'v>) -> Option<(AtomRef<'v>, usize)> {
            match l.view() {
                Lit::Atom(a) if !a.neg() && !a.gated() => keyed_rel(v, a).map(|k| (a, k)),
                _ => None,
            }
        }
        if r.body().filter(|&l| keyed(v, l).is_some()).count() < 2 {
            return Ok(Step::Keep);
        }
        let special: BTreeSet<Slot> = r.body().filter(|l| matches!(l.view(), Lit::Agg { .. } | Lit::Nearest { .. } | Lit::Ast { .. })).flat_map(lit_slots).collect();
        // The renames made (each variable to the one it became), resolved in turn.
        let mut ren: HashMap<Slot, Slot> = HashMap::new();
        let find = |ren: &HashMap<Slot, Slot>, mut s: Slot| {
            while let Some(&t) = ren.get(&s) {
                s = t;
            }
            s
        };
        let teq = |ren: &HashMap<Slot, Slot>, x: TermRef<'_>, y: TermRef<'_>| -> bool { Tm::of_renamed(v, r, x, &|s| find(ren, s)) == Tm::of_renamed(v, r, y, &|s| find(ren, s)) };
        #[derive(Clone)]
        enum B<'v> {
            Own(LitRef<'v>),
            /// A merged atom: its relation and the terms it took from either.
            Merged(AtomRef<'v>, Vec<TermRef<'v>>),
        }
        let mut body: Vec<B<'_>> = r.body().map(B::Own).collect();
        #[allow(clippy::type_complexity)]
        fn atom_of<'v>(v: &Version, b: &B<'v>) -> Option<(AtomRef<'v>, Vec<TermRef<'v>>, usize)> {
            match b {
                B::Own(l) => keyed(v, *l).map(|(a, k)| (a, a.args().collect(), k)),
                B::Merged(a, args) => keyed_rel(v, *a).map(|k| (*a, args.clone(), k)),
            }
        }
        let mut merged = 0;
        'again: loop {
            for i in 0..body.len() {
                let Some((a, aargs, k)) = atom_of(v, &body[i]) else { continue };
                if matches!(aargs[k], TermRef::Wild | TermRef::Call(..)) {
                    continue;
                }
                for j in i + 1..body.len() {
                    let Some((b, bargs, _)) = atom_of(v, &body[j]) else { continue };
                    if b.rel() != a.rel() || !teq(&ren, bargs[k], aargs[k]) {
                        continue;
                    }
                    let mut args = Vec::with_capacity(aargs.len());
                    let mut renames = Vec::new();
                    let mut ok = true;
                    for (&x, &y) in aargs.iter().zip(&bargs) {
                        match (x, y) {
                            _ if teq(&ren, x, y) => args.push(x),
                            (TermRef::Wild, y) => args.push(y),
                            (x, TermRef::Wild) => args.push(x),
                            (TermRef::Var(x), TermRef::Var(y)) if !special.contains(&find(&ren, y)) => {
                                args.push(TermRef::Var(x));
                                renames.push((find(&ren, y), find(&ren, x)));
                            }
                            _ => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if !ok {
                        continue;
                    }
                    body[i] = B::Merged(a, args);
                    body.remove(j);
                    for (from, to) in renames {
                        let to = find(&ren, to);
                        if from != to {
                            ren.insert(from, to);
                        }
                    }
                    merged += 1;
                    continue 'again;
                }
            }
            break;
        }
        if merged == 0 {
            return Ok(Step::Keep);
        }
        self.0 += merged;
        let mut rw = Subst { head: true, ..Default::default() };
        for s in 0..r.var_count() as u32 {
            let s = Slot(s);
            let t = find(&ren, s);
            if t != s {
                rw.to.insert(s, T::VarAtom(r.var(t)));
            }
        }
        let mut w = out.rule().head_with(v, r, r.head(), &mut rw);
        for b in &body {
            w = match b {
                B::Own(l) => match l.view() {
                    Lit::Atom(_) | Lit::Cmp { .. } => w.lit_with(v, r, *l, &mut rw),
                    _ => w.lit(v, r, *l),
                },
                B::Merged(a, args) => {
                    let ts: Vec<T<'static>> = args.iter().map(|&t| renamed_t(v, r, t, &|s| find(&ren, s))).collect();
                    w.atom_with(a.rel(), &ts, a.neg(), a.gated())
                }
            };
        }
        w.finish()?;
        Ok(Step::Written)
    }
}

fn keyed_rel(v: &Version, a: AtomRef<'_>) -> Option<usize> {
    KEYS.iter().find(|(rel, n, _)| *rel == v.rel_name(a.rel()) && *n == a.arity()).map(|&(_, _, k)| k)
}

/// Term `t` of rule `r` to write, each variable as `to` maps it.
fn renamed_t(v: &Version, r: RuleRef<'_>, t: TermRef<'_>, to: &dyn Fn(Slot) -> Slot) -> T<'static> {
    match t {
        TermRef::Var(s) => T::VarAtom(r.var(to(s))),
        TermRef::Call(f, args) => T::CallAtom(f, args.map(|a| renamed_t(v, r, a, to)).collect()),
        t => own(v, r, t),
    }
}

impl Tm {
    fn of_renamed(v: &Version, r: RuleRef<'_>, t: TermRef<'_>, to: &dyn Fn(Slot) -> Slot) -> Tm {
        match t {
            TermRef::Var(s) => Tm::Var(v.text(r.var(to(s))).to_owned()),
            TermRef::Call(f, args) => Tm::Call(v.text(f).to_owned(), args.map(|a| Tm::of_renamed(v, r, a, to)).collect()),
            t => Tm::of(v, r, t),
        }
    }
}

/// A variable of a positive atom named nowhere else is `_`.
///
/// Makes `_` each variable of a positive atom that appears nowhere else in `rule`; returns how many.
#[derive(Default)]
pub struct WildSingletons(pub usize);

/// How many times each variable appears in `r` .
fn occurrences(r: RuleRef<'_>) -> HashMap<Slot, usize> {
    fn term(t: TermRef<'_>, n: &mut HashMap<Slot, usize>) {
        match t {
            TermRef::Var(s) => *n.entry(s).or_default() += 1,
            TermRef::Call(_, args) => args.for_each(|a| term(a, n)),
            _ => {}
        }
    }
    let mut n = HashMap::new();
    r.head().args().for_each(|t| term(t, &mut n));
    for l in r.body() {
        match l.view() {
            Lit::Atom(a) => a.args().for_each(|t| term(t, &mut n)),
            Lit::Cmp { l, r: rt, .. } => {
                term(l, &mut n);
                term(rt, &mut n);
            }
            Lit::Agg { var, over, atom, .. } => {
                atom.args().for_each(|t| term(t, &mut n));
                *n.entry(var).or_default() += 1;
                if let Some(o) = over {
                    *n.entry(o).or_default() += 1;
                }
            }
            Lit::Nearest { n: x, a, .. } => {
                *n.entry(x).or_default() += 1;
                *n.entry(a).or_default() += 1;
            }
            Lit::Ast { document, node, captures, .. } => {
                for s in [document, node].into_iter().chain(captures.map(|(_, s)| s)) {
                    *n.entry(s).or_default() += 1;
                }
            }
        }
    }
    n
}

/// The variables of `r` named once, and the positive atoms (by node) where some is an argument.
fn singletons(r: RuleRef<'_>) -> (HashSet<Slot>, HashSet<u32>, usize) {
    let once: HashSet<Slot> = occurrences(r).into_iter().filter(|(_, k)| *k == 1).map(|(s, _)| s).collect();
    let mut atoms = HashSet::new();
    let mut made = 0;
    for l in r.body() {
        if let Lit::Atom(a) = l.view()
            && !a.neg()
        {
            let k = a.args().filter(|t| matches!(t, TermRef::Var(s) if once.contains(s))).count();
            if k > 0 {
                atoms.insert(a.node());
                made += k;
            }
        }
    }
    (once, atoms, made)
}

impl RulePass for WildSingletons {
    const NAME: &'static str = "variables named once";
    fn rule(&mut self, _cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let (once, atoms, made) = singletons(r);
        if made == 0 {
            return Ok(Step::Keep);
        }
        self.0 += made;
        out.rewrite(&mut Subst { wild: once, wild_atoms: atoms, ..Default::default() })
    }
}

/// A kind test on a member of a set of one kind drops.
///
/// `node`'s columns as declared: the node and its kind.
#[derive(Default)]
pub struct DropKindTests(pub usize);

impl RulePass for DropKindTests {
    const NAME: &'static str = "kind tests on set members";
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(v, &d);
        let Some(cols) = f.columns("node") else { return Ok(Step::Keep) };
        let (Some(n), Some(k)) = (cols.iter().position(|(c, _)| *c == "n"), cols.iter().position(|(c, _)| *c == "kind")) else { return Ok(Step::Keep) };
        // The members of a single-kind set, by variable: `nearest`'s and `inside`'s ancestor, `has`'s descendant.
        let mut member: HashMap<Slot, &str> = HashMap::new();
        for l in r.body() {
            let (a, set) = match l.view() {
                Lit::Nearest { a, kindset, .. } => (a, v.text(kindset)),
                Lit::Atom(x) if !x.neg() && x.arity() == 2 => match crate::code_rule_dsl::logic::front::ast::axis(v.rel_name(x.rel())) {
                    Some((set, has)) => match x.args().nth(if has { 0 } else { 1 }) {
                        Some(TermRef::Var(a)) => (a, set),
                        _ => continue,
                    },
                    None => continue,
                },
                _ => continue,
            };
            if let Some([kind]) = f.kindset(set) {
                member.insert(a, v.text(*kind));
            }
        }
        if member.is_empty() {
            return Ok(Step::Keep);
        }
        let mut drop = HashSet::new();
        for l in r.body() {
            let Lit::Atom(x) = l.view() else { continue };
            if v.rel_name(x.rel()) != "node" || x.neg() || x.arity() != cols.len() {
                continue;
            }
            let args: Vec<TermRef<'_>> = x.args().collect();
            let only = args.iter().enumerate().all(|(c, t)| c == n || c == k || matches!(t, TermRef::Wild));
            let tested = match (args[n], args[k]) {
                (TermRef::Var(a), TermRef::Str(kind)) => member.get(&a) == Some(&v.text(kind)),
                _ => false,
            };
            if only && tested {
                drop.insert(l.node());
            }
        }
        if drop.is_empty() {
            return Ok(Step::Keep);
        }
        self.0 += drop.len();
        out.rewrite(&mut Subst { drop, ..Default::default() })
    }
}

/// Whether atom `b` implies `a` in one rule .
fn implies(r: RuleRef<'_>, b: AtomRef<'_>, a: AtomRef<'_>) -> bool {
    b.rel() == a.rel() && !a.neg() && !b.neg() && a.arity() == b.arity() && a.args().zip(b.args()).all(|(x, y)| same_term(x, y) || matches!(x, TermRef::Wild)) && {
        let _ = r;
        true
    }
}

/// The literals `DropImplied` drops from `r`, by node.
fn implied(r: RuleRef<'_>) -> HashSet<u32> {
    let lits: Vec<LitRef<'_>> = r.body().collect();
    let atoms: Vec<Option<AtomRef<'_>>> = lits.iter().map(|l| if let Lit::Atom(a) = l.view() { Some(a) } else { None }).collect();
    (0..lits.len())
        .filter(|&i| {
            if lits[..i].iter().any(|&e| same_lit(r, e, lits[i])) {
                return true;
            }
            let Some(a) = atoms[i] else { return false };
            (0..atoms.len()).any(|j| j != i && atoms[j].is_some_and(|b| implies(r, b, a) && !same_atom(r, b, a)))
        })
        .map(|i| lits[i].node())
        .collect()
}

/// An atom implied by another, or a literal equal to an earlier one, drops.
///
/// A dropped atom is implied by one that stays: by a different atom, or by an equal one earlier. Any literal
/// equal to an earlier one goes too.
#[derive(Default)]
pub struct DropImplied(pub usize);

impl RulePass for DropImplied {
    const NAME: &'static str = "implied atoms";
    fn rule(&mut self, _cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let drop = implied(r);
        if drop.is_empty() {
            return Ok(Step::Keep);
        }
        self.0 += drop.len();
        out.rewrite(&mut Subst { drop, ..Default::default() })
    }
}

// ---- the whole program again ----

/// A join on a function of a relation's columns, a join on a relation keyed by the
/// function's value. The relations made.
///
/// The world's relations and the program's inputs are as large as what was read: not copied.
///
/// An atom keyed by this variable already (an earlier pass of the loop) is not keyed again.
///
/// The function over R's columns: each variable as the column it is first at.
///
/// The comparison stays: a plan that has R's columns first computes `$v` forward from them, and the keyed
/// atom is then a test (as a concatenation and its inverse are).
///
/// a join on a column: `R__f<k>($v, …) :- R(…), $v = @f(…)` holds each row of R with the function's value first, and
/// the rule reads it in place of R and the comparison. A plan that has `$v` before R's columns then probes the new
/// relation by `$v`, where it would otherwise cross every row of R with what it has (an index is of columns, not of
/// functions). The new relation is a relation like any other: kept, maintained and planned as one.
/// Only for relations without a node column, whose rows are few (a vocabulary, not a document's tree), and for
/// pure functions of R's columns alone; `@concat` runs backwards instead (`concat_inverses`). One relation for each
/// R, function and argument shape, shared by every rule that reads it.
#[derive(Default)]
pub struct FunctionKeys(pub Vec<String>);

impl ProgramPass for FunctionKeys {
    const NAME: &'static str = "join keys from functions";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        use crate::code_rule_dsl::logic::strings::functions::{Kind, native_call};
        let (v, d) = fields(cx)?;
        let f = Fields::over(v, &d);
        // (relation, shape, name) of each relation made, in order.
        let mut made: Vec<(String, Tm, String)> = Vec::new();
        let mut new_rules: Vec<(String, String, usize, Tm)> = Vec::new();
        // Per rule: the atoms keyed (by node), each with its relation and key variable.
        let mut keyed_atoms: Vec<HashMap<u32, (String, Slot)>> = Vec::new();
        let mut decls: Vec<(String, Vec<(String, String)>)> = Vec::new();
        for r in v.rules() {
            let mut keyed: HashMap<u32, (String, Slot)> = HashMap::new();
            let rel_now = |l: LitRef<'_>, keyed: &HashMap<u32, (String, Slot)>| -> String { keyed.get(&l.node()).map_or_else(|| l.atom().map_or_else(String::new, |a| v.rel_name(a.rel()).to_owned()), |(n, _)| n.clone()) };
            loop {
                let mut found: Option<(u32, String, Tm, String, Slot)> = None;
                'cmps: for (ci, lit) in r.body().enumerate() {
                    let Lit::Cmp { l, op: CmpOp::Eq, r: rt } = lit.view() else { continue };
                    let (x, call) = match (l, rt) {
                        (TermRef::Var(x), c @ TermRef::Call(..)) | (c @ TermRef::Call(..), TermRef::Var(x)) => (x, c),
                        _ => continue,
                    };
                    let TermRef::Call(fa, args) = call else { continue };
                    let fname = v.text(fa);
                    let Some((_, _, kind)) = native_call(fname) else { continue };
                    // A concatenation's inverse binds from a column it already has (`concat_inverses`).
                    if fname.starts_with("git_") || fname == "str_between" || args.clone().any(|a| !matches!(a, TermRef::Var(_) | TermRef::Str(_) | TermRef::Int(_))) {
                        continue;
                    }
                    let mut vars = BTreeSet::new();
                    term_slots(call, &mut vars);
                    if vars.is_empty() || vars.contains(&x) {
                        continue;
                    }
                    // `$v` is a key only where another atom joins on it (or a concatenation's inverse gives it from one):
                    // a value only comparisons read is computed forward, never probed.
                    let read_elsewhere = r.body().enumerate().any(|(j, other)| {
                        j != ci
                            && match other.view() {
                                Lit::Atom(a) => !a.neg() && (keyed.get(&other.node()).is_some_and(|(_, k)| *k == x) || a.args().any(|t| matches!(t, TermRef::Var(y) if y == x))),
                                Lit::Cmp { l: TermRef::Var(y), op: CmpOp::Eq, r: TermRef::Call(g, _) } => y == x && v.text(g) == "str_between",
                                _ => false,
                            }
                    });
                    if !read_elsewhere {
                        continue;
                    }
                    for other in r.body() {
                        let Lit::Atom(a) = other.view() else { continue };
                        let rel = rel_now(other, &keyed);
                        let world = crate::code_rule_dsl::logic::rr::WORLD.iter().any(|(w, _)| *w == rel) || rel == "loaded";
                        let prefix = format!("{rel}__key_");
                        let already = r.body().any(|o| keyed.get(&o.node()).is_some_and(|(n, k)| n.starts_with(&prefix) && *k == x) || matches!(o.view(), Lit::Atom(k) if !keyed.contains_key(&o.node()) && v.rel_name(k.rel()).starts_with(&prefix) && matches!(k.args().next(), Some(TermRef::Var(s)) if s == x)));
                        if a.neg() || a.gated() || rel.contains("__key_") || world || already || f.is_input(&rel) {
                            continue;
                        }
                        let Some(cols) = f.columns(&rel) else { continue };
                        if cols.len() != a.arity() || cols.iter().any(|(_, t)| *t == "node") {
                            continue;
                        }
                        let avars = atom_slots(a);
                        if avars.contains(&x) || !vars.is_subset(&avars) {
                            continue;
                        }
                        let aargs: Vec<TermRef<'_>> = a.args().collect();
                        let col = |s: Slot| aargs.iter().position(|t| matches!(t, TermRef::Var(y) if *y == s));
                        let shape = Tm::Call(
                            fname.to_owned(),
                            args.clone()
                                .map(|t| match t {
                                    TermRef::Var(s) => col(s).map_or_else(|| Tm::of(v, r, t), |i| Tm::Var(format!("c{i}"))),
                                    t => Tm::of(v, r, t),
                                })
                                .collect(),
                        );
                        let ty = if kind == Kind::Number { "int" } else { "sym" };
                        found = Some((other.node(), rel, shape, ty.to_owned(), x));
                        break 'cmps;
                    }
                }
                let Some((at, rel, shape, ty, x)) = found else { break };
                let name = match made.iter().position(|(r2, s, _)| *r2 == rel && *s == shape) {
                    Some(k) => made[k].2.clone(),
                    None => {
                        let Tm::Call(fname, _) = &shape else { unreachable!("a call") };
                        let name = format!("{rel}__key_{fname}{}", made.iter().filter(|(r2, _, _)| *r2 == rel).count());
                        let cols: Vec<(String, String)> = f.columns(&rel).unwrap_or_default().into_iter().map(|(c, t)| (c.to_owned(), t.to_owned())).collect();
                        let mut decl = vec![("k".to_owned(), ty.clone())];
                        decl.extend(cols.iter().cloned());
                        decls.push((name.clone(), decl));
                        new_rules.push((name.clone(), rel.clone(), cols.len(), shape.clone()));
                        made.push((rel, shape, name.clone()));
                        name
                    }
                };
                keyed.insert(at, (name, x));
            }
            keyed_atoms.push(keyed);
        }
        if made.is_empty() {
            return Ok(());
        }
        let input = w.input().clone();
        let b = w.derive()?;
        for (r, keyed) in input.rules().zip(&keyed_atoms) {
            if keyed.is_empty() {
                b.keep(&input, r);
                continue;
            }
            let mut wr = b.rule().source(r.id()).head_from(&input, r, r.head());
            for l in r.body() {
                wr = match (keyed.get(&l.node()), l.view()) {
                    (Some((name, x)), Lit::Atom(a)) => {
                        let mut args = vec![T::VarAtom(r.var(*x))];
                        args.extend(a.args().map(|t| own(&input, r, t)));
                        let rel = wr.builder().rel(name);
                        wr.atom_with(rel, &args, a.neg(), a.gated())
                    }
                    _ => wr.lit(&input, r, l),
                };
            }
            wr.finish()?;
        }
        for (name, decl) in &decls {
            let cols: Vec<(&str, &str)> = decl.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
            b.declare(name, &cols);
        }
        for (name, rel, n, shape) in &new_rules {
            let cols: Vec<String> = (0..*n).map(|i| format!("c{i}")).collect();
            let mut head = vec![T::Var("k")];
            head.extend(cols.iter().map(|c| T::Var(c.as_str())));
            let colts: Vec<T<'_>> = cols.iter().map(|c| T::Var(c.as_str())).collect();
            let mut wr = b.rule().head(name.as_str(), &head).atom(rel.as_str(), &colts);
            let s = shape.write(wr.builder());
            wr = wr.cmp(T::Var("k"), CmpOp::Eq, s);
            wr.finish()?;
        }
        self.0 = made.into_iter().map(|(_, _, n)| n).collect();
        Ok(())
    }
}

/// A literal of a rule as `project_existentials` leaves it: its own, or the read of a part made a relation.
#[derive(Clone)]
enum EItem<'v> {
    Own(LitRef<'v>),
    Exists(String, Slot),
}

/// A part of a body that meets the rest only at a document is a relation of
/// documents. The relations made.
///
/// The same rule again: it may have another such part.
///
/// Moves each part of a body that meets the rest only at a document variable into a relation of documents
/// (`exists_<k>`, the module's list); returns the relations made. A part equal to one moved before (the same template,
/// `template`) reads that relation: the same test in several rules, a library's in each policy, was a relation per
/// rule that `merge` found equal again.
#[derive(Default)]
pub struct ProjectExistentials(pub Vec<String>);

impl ProgramPass for ProjectExistentials {
    const NAME: &'static str = "parts of bodies as relations of documents";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let (v, d) = fields(cx)?;
        let f = Fields::over(v, &d);
        let mut declared: HashSet<String> = f.decls().iter().map(|d| v.text(d.name()).to_owned()).collect();
        let mut made: Vec<String> = Vec::new();
        // The parts made, by their columns' types and their template: their name.
        let mut seen: HashMap<(Vec<String>, Vec<Made>), String> = HashMap::new();
        // Per rule its body; and the rules made: (name, the rule read, its document variable, the literals).
        let mut bodies: Vec<Vec<EItem<'_>>> = Vec::new();
        let mut new_rules: Vec<(String, RuleRef<'_>, Slot, Vec<LitRef<'_>>)> = Vec::new();
        let mut decls: Vec<(String, String)> = Vec::new();
        for r in v.rules() {
            let mut body: Vec<EItem<'_>> = r.body().map(EItem::Own).collect();
            loop {
                let mut next = made.len();
                let name = loop {
                    let name = format!("exists_{next}");
                    if !declared.contains(&name) {
                        break name;
                    }
                    next += 1;
                };
                match existential(v, &f, r, &mut body, &name) {
                    Some((d, lits, ty)) => {
                        let key = (vec![ty.clone()], template_of(v, r, d, &lits));
                        if let Some(before) = seen.get(&key) {
                            for it in &mut body {
                                if let EItem::Exists(n, _) = it
                                    && *n == name
                                {
                                    *n = before.clone();
                                }
                            }
                            continue;
                        }
                        declared.insert(name.clone());
                        decls.push((name.clone(), ty));
                        seen.insert(key, name.clone());
                        new_rules.push((name.clone(), r, d, lits));
                        made.push(name);
                    }
                    None => break,
                }
            }
            bodies.push(body);
        }
        if made.is_empty() {
            return Ok(());
        }
        let input = w.input().clone();
        let b = w.derive()?;
        for (r, body) in input.rules().zip(&bodies) {
            if body.iter().all(|i| matches!(i, EItem::Own(_))) && body.len() == r.body().count() {
                b.keep(&input, r);
                continue;
            }
            let mut wr = b.rule().source(r.id()).head_from(&input, r, r.head());
            for it in body {
                wr = match it {
                    EItem::Own(l) => wr.lit(&input, r, *l),
                    EItem::Exists(n, d) => {
                        let rel = wr.builder().rel(n);
                        wr.atom(rel, &[T::VarAtom(r.var(*d))])
                    }
                };
            }
            wr.finish()?;
        }
        for (name, r, d, lits) in &new_rules {
            let rel = b.rel(name);
            let mut wr = b.rule().head(rel, &[T::VarAtom(r.var(*d))]);
            for l in lits {
                wr = wr.lit(&input, *r, *l);
            }
            wr.finish()?;
        }
        for (name, ty) in &decls {
            b.declare(name, &[("d", ty.as_str())]);
        }
        self.0 = made;
        Ok(())
    }
}

/// A part's template (of `exists(d) :- lits`): its literals by text, its variables
/// numbered by first appearance, the head's first.
fn template_of(v: &Version, r: RuleRef<'_>, d: Slot, lits: &[LitRef<'_>]) -> Vec<Made> {
    let mut names: Vec<String> = vec![v.text(r.var(d)).to_owned()];
    let mut out = Vec::new();
    let mut number = |m: Made| -> Made {
        let mut first = |x: &str| -> String {
            let k = names.iter().position(|n| n == x).unwrap_or_else(|| {
                names.push(x.to_owned());
                names.len() - 1
            });
            format!("v{k}")
        };
        fn t(x: &Tm, first: &mut dyn FnMut(&str) -> String) -> Tm {
            match x {
                Tm::Var(n) => Tm::Var(first(n)),
                Tm::Call(f, a) => Tm::Call(f.clone(), a.iter().map(|y| t(y, first)).collect()),
                x => x.clone(),
            }
        }
        match m {
            Made::Atom { rel, args, neg, gated } => Made::Atom { rel, args: args.iter().map(|x| t(x, &mut first)).collect(), neg, gated },
            Made::Cmp { l, op, r } => {
                let l = t(&l, &mut first);
                Made::Cmp { l, op, r: t(&r, &mut first) }
            }
        }
    };
    for &l in lits {
        if let Some(m) = made_of(v, r, l) {
            out.push(number(m));
        }
    }
    out
}

/// The variables an item names.
fn item_slots(it: &EItem<'_>) -> BTreeSet<Slot> {
    match it {
        EItem::Own(l) => lit_slots(*l),
        EItem::Exists(_, d) => BTreeSet::from([*d]),
    }
}

/// The first part of the body meeting the rest and the head only at one document
/// variable, replaced by `name($d)`: the variable, the part and its column's type.
///
/// Only `$d` may be shared with the rest or the head (it is: other shared variables join parts).
///
/// The first part of `rule`'s body that meets the rest and the head only at one document variable, replaced by
/// `name($d)`: that relation's rule and columns.
fn existential<'v>(v: &'v Version, f: &Fields<'_>, r: RuleRef<'v>, body: &mut Vec<EItem<'v>>, name: &str) -> Option<(Slot, Vec<LitRef<'v>>, String)> {
    let head = head_slots(r);
    let name_of = |s: Slot| v.text(r.var(s));
    let docs: BTreeMap<&str, Slot> = body
        .iter()
        .filter_map(|it| match it {
            EItem::Own(l) => match l.view() {
                Lit::Atom(a) if (v.rel_name(a.rel()) == "node" && a.arity() == 5) || v.rel_name(a.rel()) == "doc" => match a.args().next() {
                    Some(TermRef::Var(d)) => Some((name_of(d), d)),
                    _ => None,
                },
                _ => None,
            },
            EItem::Exists(..) => None,
        })
        .collect();
    let vars: Vec<BTreeSet<Slot>> = body.iter().map(item_slots).collect();
    let n = body.len();
    for (_, &d) in &docs {
        // Parts: literals joined by a shared variable other than `$d`.
        let mut part: Vec<usize> = (0..n).collect();
        fn root(part: &mut [usize], i: usize) -> usize {
            let mut r = i;
            while part[r] != r {
                r = part[r];
            }
            part[i] = r;
            r
        }
        let mut first: HashMap<&str, usize> = HashMap::new();
        for (i, vs) in vars.iter().enumerate() {
            // In name order, as the tree's sets of names are.
            let mut names: Vec<&str> = vs.iter().filter(|&&s| s != d).map(|&s| name_of(s)).collect();
            names.sort_unstable();
            for x in names {
                match first.get(x) {
                    Some(&j) => {
                        let (a, b) = (root(&mut part, i), root(&mut part, j));
                        part[a] = b;
                    }
                    None => {
                        first.insert(x, i);
                    }
                }
            }
        }
        let roots: BTreeSet<usize> = (0..n).map(|i| root(&mut part, i)).collect();
        for rt in roots {
            let members: Vec<usize> = (0..n).filter(|&i| root(&mut part, i) == rt).collect();
            if members.len() < 2 || members.len() == n {
                continue;
            }
            let own: BTreeSet<Slot> = members.iter().flat_map(|&i| vars[i].iter().copied()).collect();
            if own.iter().any(|&s| s != d && head.contains(&s)) || !own.contains(&d) || own.len() < 2 {
                continue;
            }
            let shares = head.contains(&d) || (0..n).any(|i| !members.contains(&i) && vars[i].contains(&d));
            if !shares {
                continue;
            }
            let lits: Vec<LitRef<'v>> = members
                .iter()
                .filter_map(|&i| match &body[i] {
                    EItem::Own(l) => Some(*l),
                    EItem::Exists(..) => None,
                })
                .collect();
            if lits.len() != members.len() || lits.iter().any(|l| matches!(l.view(), Lit::Agg { .. } | Lit::Nearest { .. } | Lit::Ast { .. })) {
                continue;
            }
            let bound: BTreeSet<Slot> = lits
                .iter()
                .filter_map(|l| match l.view() {
                    Lit::Atom(a) if !a.neg() => Some(atom_slots(a)),
                    _ => None,
                })
                .flatten()
                .collect();
            if !own.is_subset(&bound) {
                continue;
            }
            // `$d`'s column type, from a positive atom of the part that names it.
            let ty = lits.iter().find_map(|l| match l.view() {
                Lit::Atom(a) if !a.neg() => a.args().position(|t| matches!(t, TermRef::Var(s) if s == d)).and_then(|c| f.column_type(v.rel_name(a.rel()), c).map(str::to_owned)),
                _ => None,
            })?;
            let at = members[0];
            let mut kept = Vec::with_capacity(n - members.len() + 1);
            for (i, it) in std::mem::take(body).into_iter().enumerate() {
                if i == at {
                    kept.push(EItem::Exists(name.to_owned(), d));
                }
                if !members.contains(&i) {
                    kept.push(it);
                }
            }
            *body = kept;
            return Some((d, lits, ty));
        }
    }
    None
}

/// A recursive component of unary node relations that steps one `child` edge at a
/// time, all one way, reads `within`. The components rewritten, by their members.
///
/// Each relation's rules, by place: made again only when a component is rewritten.
///
/// and steps along one `child` edge, all one way (`R(x) :- S(y), child(x, y, _)`: up; `child(y, x, _)`: down),
/// is a closure along the tree: each member holds its bases' nodes that a path of copies reaches with no step, and
/// their strict ancestors (descendants) that paths of every length from one step reach. That is `within` (decision
/// 0050: read off the tree per node looked up), with no fixpoint: one wave a level of nesting otherwise.
/// Rewritten so; the components rewritten, by their members.
#[derive(Default)]
pub struct ClimbsAsAxes(pub Vec<String>);

impl ProgramPass for ClimbsAsAxes {
    const NAME: &'static str = "recursion along the tree as an axis";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let (v, d) = fields(cx)?;
        let f = Fields::over(v, &d);
        let Ok(sccs) = cx.get::<crate::code_rule_dsl::logic::program_image::analyses::Components>() else { return Ok(()) };
        let mut by_head: HashMap<&str, Vec<RuleRef<'_>>> = HashMap::new();
        for r in v.rules() {
            by_head.entry(v.rel_name(r.head().rel())).or_default().push(r);
        }
        let unary_node = |rel: &str| f.columns(rel).is_some_and(|c| c.len() == 1 && c[0].1 == "node");
        // Per component rewritten: its members and its new rules (member, base rule, step: the axis's direction).
        #[allow(clippy::type_complexity)]
        let mut done: Vec<(BTreeSet<String>, Vec<(String, RuleRef<'_>, Option<bool>)>)> = Vec::new();
        for scc in sccs.iter() {
            let members: BTreeSet<String> = scc.iter().cloned().collect();
            if !members.iter().all(|r| unary_node(r) && !kept(&f, r)) {
                continue;
            }
            let mut rules: Vec<RuleRef<'_>> = members.iter().flat_map(|m| by_head.get(m.as_str()).cloned().unwrap_or_default()).collect();
            rules.sort_by_key(|r| r.id());
            // (from, to, step) edges and (relation, base rule) pairs.
            let mut edges: Vec<(String, String, bool)> = Vec::new();
            let mut bases: Vec<(String, RuleRef<'_>)> = Vec::new();
            let mut up = None;
            let mut ok = true;
            for &r in &rules {
                let hargs: Vec<TermRef<'_>> = r.head().args().collect();
                let [TermRef::Var(x)] = hargs.as_slice() else {
                    ok = false;
                    break;
                };
                let x = *x;
                let head = v.rel_name(r.head().rel()).to_owned();
                let reads_member = r.body().any(|l| l.atom().is_some_and(|a| members.contains(v.rel_name(a.rel()))));
                if !reads_member {
                    bases.push((head, r));
                    continue;
                }
                let atoms: Vec<AtomRef<'_>> = r.body().filter_map(|l| if let Lit::Atom(a) = l.view() { Some(a) } else { None }).collect();
                if atoms.len() != r.body().count() || atoms.iter().any(|a| a.neg()) {
                    ok = false;
                    break;
                }
                match atoms.as_slice() {
                    [s] if members.contains(v.rel_name(s.rel())) && matches!(s.args().collect::<Vec<_>>().as_slice(), [TermRef::Var(y)] if *y == x) => edges.push((v.rel_name(s.rel()).to_owned(), head, false)),
                    [a, b] => {
                        let (s, c) = if members.contains(v.rel_name(a.rel())) { (a, b) } else { (b, a) };
                        let sargs: Vec<TermRef<'_>> = s.args().collect();
                        let cargs: Vec<TermRef<'_>> = c.args().collect();
                        let (Some(TermRef::Var(y)), true) = (sargs.first().copied(), members.contains(v.rel_name(s.rel())) && sargs.len() == 1 && v.rel_name(c.rel()) == "child" && cargs.len() == 3) else {
                            ok = false;
                            break;
                        };
                        let third_free = match cargs[2] {
                            TermRef::Wild => true,
                            TermRef::Var(z) => z != x && z != y,
                            _ => false,
                        };
                        let dir = match (cargs[0], cargs[1]) {
                            (TermRef::Var(p), TermRef::Var(q)) if p == x && q == y && x != y => true,
                            (TermRef::Var(p), TermRef::Var(q)) if p == y && q == x && x != y => false,
                            _ => {
                                ok = false;
                                break;
                            }
                        };
                        if !third_free || up.is_some_and(|u| u != dir) {
                            ok = false;
                            break;
                        }
                        up = Some(dir);
                        edges.push((v.rel_name(s.rel()).to_owned(), head, true));
                    }
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            let Some(up) = up else { continue };
            if !ok {
                continue;
            }
            // Depths of steps by which each base relation reaches each member, capped at n (n stands for n or more).
            let n = 2 * members.len() + 2;
            let mut valid = true;
            let base_rels: BTreeSet<&String> = bases.iter().map(|(r, _)| r).collect();
            let mut reach: BTreeMap<(&String, &String), BTreeSet<usize>> = BTreeMap::new();
            for q in &base_rels {
                let mut seen: BTreeSet<(String, usize)> = BTreeSet::new();
                let mut todo = vec![((*q).clone(), 0usize)];
                while let Some((r, dd)) = todo.pop() {
                    if !seen.insert((r.clone(), dd)) {
                        continue;
                    }
                    for (from, to, step) in &edges {
                        if *from == r {
                            todo.push((to.clone(), (dd + usize::from(*step)).min(n)));
                        }
                    }
                }
                for m in &members {
                    let depths: BTreeSet<usize> = seen.iter().filter(|(r, _)| r == m).map(|(_, dd)| *dd).collect();
                    reach.insert((*q, m), depths);
                }
            }
            let mut new_rules: Vec<(String, RuleRef<'_>, Option<bool>)> = Vec::new();
            for m in &members {
                for q in &base_rels {
                    let depths = &reach[&(*q, m)];
                    let stepped: BTreeSet<usize> = depths.iter().copied().filter(|dd| *dd > 0).collect();
                    // Every depth from one step on, or none: else the closure is not the axis.
                    if !(stepped.is_empty() || stepped == (1..=n).collect::<BTreeSet<_>>()) {
                        valid = false;
                    }
                    for (_, b) in bases.iter().filter(|(r, _)| r == *q) {
                        let hargs: Vec<TermRef<'_>> = b.head().args().collect();
                        let [TermRef::Var(_)] = hargs.as_slice() else {
                            valid = false;
                            continue;
                        };
                        if depths.contains(&0) {
                            new_rules.push((m.clone(), *b, None));
                        }
                        if !stepped.is_empty() {
                            new_rules.push((m.clone(), *b, Some(up)));
                        }
                    }
                }
            }
            if !valid {
                continue;
            }
            done.push((members, new_rules));
        }
        if done.is_empty() {
            return Ok(());
        }
        let gone: HashSet<&str> = done.iter().flat_map(|(m, _)| m.iter().map(String::as_str)).collect();
        let input = w.input().clone();
        let declared_within = f.declared(WITHIN);
        let input_within = f.is_input(WITHIN);
        let inputs: Vec<String> = f.inputs().map(str::to_owned).collect();
        let b = w.derive()?;
        for r in input.rules() {
            if !gone.contains(input.rel_name(r.head().rel())) {
                b.keep(&input, r);
            }
        }
        for (_, new_rules) in &done {
            for (m, base, step) in new_rules {
                let rel = b.rel(m);
                let TermRef::Var(t) = base.head().args().next().expect("one column") else { unreachable!("a variable") };
                let tname = input.text(base.var(t)).to_owned();
                match step {
                    None => {
                        let args: Vec<T<'static>> = base.head().args().map(|x| own(&input, *base, x)).collect();
                        let mut wr = b.rule().head(rel, &args);
                        for l in base.body() {
                            wr = wr.lit(&input, *base, l);
                        }
                        wr.finish()?;
                    }
                    Some(up) => {
                        let x = format!("{tname}_axis");
                        let (inner, outer) = if *up { (tname.as_str(), x.as_str()) } else { (x.as_str(), tname.as_str()) };
                        let mut wr = b.rule().head(rel, &[T::Var(x.as_str())]);
                        for l in base.body() {
                            wr = wr.lit(&input, *base, l);
                        }
                        wr.atom(WITHIN, &[T::Var(inner), T::Var(outer)]).finish()?;
                    }
                }
            }
        }
        if !declared_within {
            b.declare(WITHIN, &[("n", "node"), ("a", "node")]);
        }
        if !input_within {
            let mut inputs = inputs;
            inputs.push(WITHIN.to_owned());
            b.set_inputs(&inputs);
        }
        self.0.extend(done.into_iter().map(|(m, _)| m.into_iter().collect::<Vec<_>>().join("/")));
        Ok(())
    }
}

/// Drops each request rule (the program's `asked_by`) whose body holds every literal of another rule of its relation
/// with the same head: that one derives all it does. The request kept whole for a reader that asks for all, beside the
/// one its other readers' asks narrow (decision 0291, `rr::lower`'s `demand_of`), where both are reached. The rule
/// kept takes the dropped one's readers into its gate, so it is kept wherever the dropped one would have been. Run by
/// `simplify`, before relations merge, between the prunes that keep the reached ones. How many were dropped.
#[derive(Default)]
pub struct SubsumeRequests(pub usize);

impl ProgramPass for SubsumeRequests {
    const NAME: &'static str = "request rules another subsumes";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let v = cx.v();
        let mut asked = v.asked();
        if asked.is_empty() {
            return Ok(());
        }
        let rules: Vec<RuleRef<'_>> = v.rules().collect();
        let head = |i: usize| v.show_atom(rules[i], rules[i].head());
        let body = |i: usize| -> Vec<String> { rules[i].body().map(|l| v.show_lit(rules[i], l)).collect() };
        let mut dropped = vec![false; rules.len()];
        for (req, readers) in asked.iter_mut() {
            let at: Vec<usize> = (0..rules.len()).filter(|&i| v.rel_name(rules[i].head().rel()) == req.as_str()).collect();
            let aligned = at.len() == readers.len();
            let bodies: Vec<Vec<String>> = at.iter().map(|&i| body(i)).collect();
            for (x, &a) in at.iter().enumerate() {
                let by = at.iter().enumerate().find(|&(y, &b)| {
                    y != x && !dropped[b] && head(b) == head(a) && bodies[y].len() < bodies[x].len() && bodies[y].iter().all(|l| bodies[x].contains(l))
                });
                if let Some((y, _)) = by {
                    dropped[a] = true;
                    if aligned {
                        let gate = readers[x].clone();
                        readers[y].extend(gate);
                    }
                }
            }
            if aligned {
                let mut it = at.iter();
                readers.retain(|_| !dropped[*it.next().unwrap()]);
            }
        }
        let n = dropped.iter().filter(|d| **d).count();
        if n == 0 {
            return Ok(());
        }
        self.0 += n;
        let gone: HashSet<_> = rules.iter().zip(&dropped).filter(|(_, d)| **d).map(|(r, _)| r.id()).collect();
        w.keep(|r| !gone.contains(&r.id()))?;
        w.on()?.set_asked(&asked);
        Ok(())
    }
}

/// The rules of relations no kept relation reads, directly or through others. Those
/// relations.
///
/// Drops the rules of derived relations that no kept relation (`kept`: outputs, demanded relations, gates, …)
/// reads, directly or through others; returns those relations.
#[derive(Default)]
pub struct DropUnread(pub Vec<String>);

impl ProgramPass for DropUnread {
    const NAME: &'static str = "relations no output reads";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let (v, d) = fields(cx)?;
        let f = Fields::over(v, &d);
        let mut reads: HashMap<&str, BTreeSet<&str>> = HashMap::new();
        for r in v.rules() {
            reads.entry(v.rel_name(r.head().rel())).or_default().extend(r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel())));
        }
        for n in f.natives() {
            reads.entry(v.rel_name(n.rel)).or_default().extend(n.reads.iter().map(|&x| v.rel_name(x)));
        }
        let mut live: BTreeSet<&str> = BTreeSet::new();
        let mut todo: Vec<&str> = reads.keys().copied().filter(|r| kept(&f, r)).collect();
        while let Some(r) = todo.pop() {
            if live.insert(r) {
                todo.extend(reads.get(r).into_iter().flatten().copied());
            }
        }
        let unread: BTreeSet<&str> = reads.keys().copied().filter(|r| !live.contains(r)).collect();
        if unread.is_empty() {
            return Ok(());
        }
        let gone: HashSet<_> = v.rules().filter(|r| unread.contains(v.rel_name(r.head().rel()))).map(|r| r.id()).collect();
        if !gone.is_empty() {
            let input = w.input().clone();
            let b = w.derive()?;
            for r in input.rules() {
                if !gone.contains(&r.id()) {
                    b.keep(&input, r);
                }
            }
            // A request relation dropped has no rules, so no entry in the request record (decision 0331).
            let was = b.asked();
            let mut asked = was.clone();
            asked.retain(|req, _| !unread.contains(req.as_str()));
            if asked != was {
                b.set_asked(&asked);
            }
        }
        self.0 = unread.into_iter().map(str::to_owned).collect();
        Ok(())
    }
}

/// Whether column `c` of derived relation `rel` holds a location.
///
/// **Columns no reader reads go** (decision 0098 §1: `count only`, inferred). A derived relation whose column is `_` in
/// every atom that reads it, positive or negated, keeps only its other columns: its rows become their projection, so
/// a reader that asks only whether a row exists (`!r($x, _)`, `r($x, _)`) reads one row per key, not every row the
/// rules make. The rows the readers see are the same: the column decides nothing they test.
/// Not projected: kept relations (outputs, inputs, lattices, natives and what they read, facts, demand, gates),
/// answered and nullary ones, and a relation an aggregate reads (a count, a sum or a string join counts its rows, which
/// a projection would merge). One column at least stays. Dropping a head column can leave its variable named once in
/// the body, made `_` there, which can free another relation's column: so it repeats until nothing changes.
/// base locations): declared a node, or filled by every rule from a node column, or a document column, of a base
/// relation it reads. A location through another derived relation's number column is not seen (none is projected
/// today: such a column holds a document key only where it is filled from one).
fn location_column(v: &Version, f: &Fields<'_>, rel: &str, c: usize) -> bool {
    if f.column_type(rel, c) == Some("node") {
        return true;
    }
    let base_location = |a: AtomRef<'_>, i: usize| {
        let name = v.rel_name(a.rel());
        crate::code_rule_dsl::typing::file_local::document_column(name) == Some(i) || (name == "docof" && i == 1) || f.column_type(name, i) == Some("node")
    };
    let rules: Vec<RuleRef<'_>> = v.rules().filter(|r| v.rel_name(r.head().rel()) == rel).collect();
    !rules.is_empty()
        && rules.iter().all(|r| {
            let Some(TermRef::Var(x)) = r.head().args().nth(c) else { return false };
            r.body().filter_map(|l| if let Lit::Atom(a) = l.view() { Some(a) } else { None }).filter(|a| !a.neg()).any(|a| a.args().enumerate().any(|(i, t)| matches!(t, TermRef::Var(y) if y == x) && base_location(a, i)))
        })
}

/// The columns each relation's readers read (`project_unread_columns`' first step): `None` where something reads it
/// whole.
fn columns_read(v: &Version) -> HashMap<String, Option<BTreeSet<usize>>> {
    let mut read: HashMap<String, Option<BTreeSet<usize>>> = HashMap::new();
    for r in v.rules() {
        let exists: BTreeSet<Slot> = existence_counts(v, r).into_iter().collect();
        for l in r.body() {
            match l.view() {
                Lit::Agg { var, op: AggRef::Count, atom, .. } if exists.contains(&var) => {
                    let e = read.entry(v.rel_name(atom.rel()).to_owned()).or_insert_with(|| Some(BTreeSet::new()));
                    if let Some(cols) = e {
                        cols.extend(atom.args().enumerate().filter(|(_, t)| !matches!(t, TermRef::Wild)).map(|(c, _)| c));
                    }
                }
                Lit::Atom(a) => {
                    let e = read.entry(v.rel_name(a.rel()).to_owned()).or_insert_with(|| Some(BTreeSet::new()));
                    if let Some(cols) = e {
                        cols.extend(a.args().enumerate().filter(|(_, t)| !matches!(t, TermRef::Wild)).map(|(c, _)| c));
                    }
                }
                Lit::Agg { atom, .. } => {
                    read.insert(v.rel_name(atom.rel()).to_owned(), None);
                }
                _ => {}
            }
        }
    }
    read
}

/// One round of `project_unread_columns`: each relation projected on the columns its readers read (and a location),
/// where that drops one, all in one rewrite of the program. Within a round one relation's projection decides nothing
/// of another's: the columns read are the round's (`columns_read`), and a projection drops only arguments no reader
/// names. The columns each projected relation kept.
///
/// Every reader, an existence count's atom too (any other aggregate's read keeps the relation whole).
///
/// Columns as the relation first had them.
struct ProjectAll {
    // Per relation: the columns a reader reads; `None` once something reads it whole.
    read: HashMap<String, Option<BTreeSet<usize>>>,
    kept: BTreeMap<String, Vec<usize>>,
}

impl ProgramPass for ProjectAll {
    const NAME: &'static str = "columns no reader reads";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let (v, d) = fields(cx)?;
        let f = Fields::over(v, &d);
        let heads: BTreeSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).collect();
        let mut keep_of: HashMap<String, Vec<usize>> = HashMap::new();
        for rel in heads {
            if kept(&f, rel) || f.nullary(rel) || f.answer(rel) {
                continue;
            }
            let Some(Some(cols)) = self.read.get(rel) else { continue };
            let Some(n) = f.arity(rel) else { continue };
            let mut keep: Vec<usize> = (0..n).filter(|c| cols.contains(c)).collect();
            // A location stays though no reader reads it: the split partitions a file-local relation's rows by one,
            // and a load hands on a document's rows by it (`py_docmod(d, p, q)`, read as `py_docmod(_, p, q)`). One
            // is enough: the first, when no column read is one.
            if !keep.iter().any(|&c| location_column(v, &f, rel, c))
                && let Some(c) = (0..n).find(|&c| location_column(v, &f, rel, c))
            {
                keep.push(c);
                keep.sort_unstable();
            }
            if keep.is_empty() {
                keep.push(0);
            }
            if keep.len() < n {
                keep_of.insert(rel.to_owned(), keep);
            }
        }
        if keep_of.is_empty() {
            return Ok(());
        }
        struct Project<'a> {
            v: &'a Version,
            keep: &'a HashMap<String, Vec<usize>>,
        }
        impl Rewrite for Project<'_> {
            fn begin(&mut self, r: RuleRef<'_>) -> bool {
                std::iter::once(r.head()).chain(r.body().filter_map(|l| l.atom())).any(|a| self.keep.contains_key(self.v.rel_name(a.rel())))
            }
            fn drop_arg(&mut self, a: AtomRef<'_>, c: usize) -> bool {
                self.keep.get(self.v.rel_name(a.rel())).is_some_and(|k| !k.contains(&c))
            }
        }
        let decls: Vec<(String, Vec<(String, String)>)> = keep_of
            .iter()
            .map(|(rel, keep)| {
                let cols: Vec<(String, String)> = f.columns(rel).unwrap_or_default().into_iter().map(|(c, t)| (c.to_owned(), t.to_owned())).collect();
                (rel.clone(), keep.iter().map(|&c| cols[c].clone()).collect())
            })
            .collect();
        let input = w.input().clone();
        let b = w.derive()?;
        b.rewrite_all(&input, &mut Project { v: &input, keep: &keep_of })?;
        for (rel, cols) in &decls {
            let decl: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            b.declare(rel, &decl);
        }
        self.kept = keep_of.into_iter().collect();
        Ok(())
    }
}

// ---- the driver ----

/// The simplifications' passes on the pipeline, in their order, and their decisions (when the run is profiled).
///
/// Each rule rewritten alone: each pass reads only its rule (and the kind sets and
/// declarations, which none changes).
///
/// A request rule another of its relation subsumes (decision 0291's whole request beside its narrowed one), before
/// relations merge: a request and its widened copy are compared with what is left.
///
/// A merge can leave a literal twice (`!m1($n), !m2($n)` became `!m1($n), !m1($n)`).
pub fn passes(p: &mut crate::code_rule_dsl::logic::program_image::manager::Pipeline<'_>) -> Result<Report> {
    let mut report = Report::default();
    p.program(&mut UnitColumns);
    let mut ground = GroundRules::default();
    p.program(&mut ground);
    report.constants += ground.0;
    let mut decided = DecidedFactTests::default();
    p.program(&mut decided);
    report.constants += decided.0;
    // `RERAIL_EXISTENCE_COUNTS=off`: existence counts stay counts, for comparison.
    if std::env::var("RERAIL_EXISTENCE_COUNTS").map_or(true, |x| x != "off") {
        let mut e = InOrder(ExistenceAsReads::default());
        p.program(&mut e);
        report.existence += e.0.0;
    }
    p.program(&mut JoinComplements);
    let mut inline = InlineNodeTests::default();
    p.program(&mut inline);
    report.inlined = inline.0;
    // `RERAIL_INLINE_FILTERS=off`: filters are built, for comparison.
    if std::env::var("RERAIL_INLINE_FILTERS").map_or(true, |x| x != "off") {
        let mut filters = InlineFilters::default();
        p.program(&mut filters);
        report.inlined.extend(filters.0);
    }
    // `RERAIL_JOIN_KEYS=off`: no concatenation runs backwards and no relation is keyed by a function, for comparison.
    let join_keys = std::env::var("RERAIL_JOIN_KEYS").map_or(true, |x| x != "off");
    if join_keys {
        let mut x = InOrder(ConcatInverses::default());
        p.program(&mut x);
        report.inverses += x.0.0;
    }
    p.program(&mut InOrder(CatVariables));
    let mut fold = InOrder(FoldConstants::default());
    p.program(&mut fold);
    report.constants += fold.0.0;
    let mut within = InOrder(SpanContainment::default());
    p.program(&mut within);
    report.within += within.0.0;
    let mut keyed = InOrder(MergeKeyed::default());
    p.program(&mut keyed);
    report.keyed += keyed.0.0;
    let mut once = InOrder(WildSingletons::default());
    p.program(&mut once);
    report.singletons += once.0.0;
    let mut kinds = InOrder(DropKindTests::default());
    p.program(&mut kinds);
    report.kind_tests += kinds.0.0;
    let mut implied = InOrder(DropImplied::default());
    p.program(&mut implied);
    report.implied += implied.0.0;
    if report.within > 0 && p.current()?.decl_of(SPAN_WITHIN).is_none() {
        p.program(&mut DeclareInput { rel: SPAN_WITHIN, cols: &[("inner", "node"), ("outer", "node")] });
    }
    if join_keys {
        let mut keys = FunctionKeys::default();
        p.program(&mut keys);
        report.keyed_by = keys.0;
    }
    let mut parts = ProjectExistentials::default();
    p.program(&mut parts);
    report.existentials = parts.0;
    let mut climbs = ClimbsAsAxes::default();
    p.program(&mut climbs);
    report.climbs = climbs.0;
    // A request rule another of its relation subsumes (decision 0291's whole request beside its narrowed one), before
    // relations merge.
    let mut subsumed = SubsumeRequests::default();
    p.program(&mut subsumed);
    report.subsumed = subsumed.0;
    let mut merge = super::merge::MergeIdentical::default();
    p.program(&mut merge);
    if merge.merged > 0 {
        report.merged += merge.merged;
        report.shared += merge.shared;
        let mut again = InOrder(DropImplied::default());
        p.program(&mut again);
        report.implied += again.0.0;
        if crate::code_rule_dsl::logic::profile::on() {
            let v = p.current()?.clone();
            let d = p.get::<FieldsData>()?;
            report.similar = super::merge::similar_relations(&v, Fields::over(&v, &d));
        }
    }
    let mut unread = DropUnread::default();
    p.program(&mut unread);
    report.unread = unread.0;
    if !std::env::var("RERAIL_PROJECT_COLUMNS").is_ok_and(|x| x == "off") {
        // Columns as each relation first had them, the ones it lost.
        let mut lost: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        loop {
            let v = p.current()?.clone();
            let mut round = ProjectAll { read: columns_read(&v), kept: BTreeMap::new() };
            p.program(&mut round);
            let changed = !round.kept.is_empty();
            for (rel, keep) in round.kept {
                let n = v.decl_of(&rel).map_or(0, |d| d.arity());
                let gone: Vec<usize> = (0..n).filter(|c| !keep.contains(c)).collect();
                let orig = lost.entry(rel).or_default();
                let mut at: Vec<usize> = (0..n + orig.len()).filter(|c| !orig.contains(c)).collect();
                for g in gone.iter().rev() {
                    orig.push(at.remove(*g));
                }
                orig.sort_unstable();
            }
            if !changed {
                break;
            }
            p.program(&mut InOrder(WildSingletons::default()));
        }
        report.projected = lost.into_iter().collect();
    }
    Ok(report)
}

/// Declares relation `rel` an input, where it is not.
struct DeclareInput {
    rel: &'static str,
    cols: &'static [(&'static str, &'static str)],
}

impl ProgramPass for DeclareInput {
    const NAME: &'static str = "declare an input";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let v = cx.v();
        let mut inputs: Vec<String> = v.inputs().map(|a| v.text(a).to_owned()).collect();
        let b = w.on()?;
        b.declare(self.rel, self.cols);
        inputs.push(self.rel.to_owned());
        b.set_inputs(&inputs);
        Ok(())
    }
}

/// The decisions `passes` makes when the run is profiled.
pub fn decide(report: &Report) {
    use crate::code_rule_dsl::logic::profile;
    if !profile::on() {
        return;
    }
    let (i, k, c, m, n) = (report.implied, report.kind_tests, report.constants, report.merged, report.inlined.len());
    let (w, y, u, x, s) = (report.singletons, report.keyed, report.unread.len(), report.existentials.len(), report.within);
    let h = report.shared;
    if !report.climbs.is_empty() {
        profile::decision("recursion along the tree made an axis", || {
            format!("{} recursive components that step one `child` edge at a time read `within` instead: {}", report.climbs.len(), report.climbs.iter().take(8).cloned().collect::<Vec<_>>().join(", "))
        });
    }
    if report.inverses > 0 || !report.keyed_by.is_empty() {
        profile::decision("join keys from functions", || {
            format!("{} concatenations run backwards from their value (a part the rest leaves one way); {} relations keyed by a function a join compares: {}", report.inverses, report.keyed_by.len(), report.keyed_by.join(", "))
        });
    }
    if report.existence > 0 {
        profile::decision("existence counts read as their atom (decision 0099)", || {
            format!("{} counts compared with 0 read as a negation (`= 0`) or a positive atom (`> 0`), so demand, inlining and projection read them as plain reads", report.existence)
        });
    }
    if !report.projected.is_empty() {
        let cols: usize = report.projected.iter().map(|(_, c)| c.len()).sum();
        profile::decision("columns no reader reads dropped (decision 0098, count only)", || {
            let some: Vec<String> = report.projected.iter().take(8).map(|(r, c)| format!("{r} {c:?}")).collect();
            format!("{} relations lost {cols} columns no reader reads, so their rows are their projection: {}", report.projected.len(), some.join(", "))
        });
    }
    if report.similar.0 > 0 {
        profile::decision("relations equal up to their constants (not merged)", || format!("{} relations in {} groups, e.g. {}", report.similar.0, report.similar.1, report.similar.2.join("; ")));
    }
    profile::decision("simplified bodies (decision 0038)", || {
        format!("{n} tests of one node inlined into their readers; {i} implied atoms and {k} kind tests on set members dropped; {c} variables equal to a constant replaced by it; {m} relations merged into an identical one ({h} classes shared across programs, `s_`); {w} variables named once made `_`; {y} atoms merged into another of their keyed row; {x} parts of bodies that meet the rest only at a document made relations of documents; {s} span comparisons of two nodes made `span_within`; {u} relations no output reads dropped")
    });
}

// ---- before the prune: the program as the batches bind it ----

/// The base relations the program reads and does not derive are inputs (`select::input_columns` names a base
/// relation).
#[derive(Default)]
pub struct BaseInputs;

impl ProgramPass for BaseInputs {
    const NAME: &'static str = "base relations as inputs";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let (v, d) = fields(cx)?;
        let f = Fields::over(v, &d);
        let mut derived: HashSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).collect();
        derived.extend(f.fact_rels().into_iter().filter(|x| !x.about()).map(|x| v.text(x.name())));
        let mut inputs: Vec<String> = f.inputs().map(str::to_owned).collect();
        let n = inputs.len();
        for r in v.rules() {
            for a in r.body().filter_map(|l| l.atom()) {
                let rel = v.rel_name(a.rel());
                if !derived.contains(rel) && f.declared(rel) && crate::code_rule_dsl::logic::sql::select::input_columns(rel).is_some() && !inputs.iter().any(|i| i == rel) {
                    inputs.push(rel.to_owned());
                }
            }
        }
        if inputs.len() > n {
            w.on()?.set_inputs(&inputs);
        }
        Ok(())
    }
}

/// core.rr's functors only CPython gives, each of a node, and the relations the batches bind them as
/// (`facts::vtab::syntax::cpython`): `@py_unparse(n)` is the `s` of `cpython_unparse(n, s)`. `@unparse` is decision
/// 0058's, a tape's text (`rr::lower`, `unparse_of`), not intercepted here.
const CPYTHON_FUNCTORS: [(&str, &str); 2] = [("py_unparse", crate::code_rule_dsl::facts::core::cpython_relations::UNPARSE), ("ast_dump", crate::code_rule_dsl::facts::core::cpython_relations::AST_DUMP)];

/// Each call of a functor only CPython gives (`CPYTHON_FUNCTORS`) in a rule, lowered to a variable its relation binds
/// in the rule's body: `v(@sha256(@ast_dump(n))) :- kind(n, "call").` is `v(@sha256(x)) :- kind(n, "call"),
/// cpython_ast_dump(n, x).` (one variable per functor and argument). Where the node has no value (no `ast` node of its
/// kind at its span, or CPython raises) the rule has no row, as a functor's undefined value gives none. A call in an
/// aggregate's atom is refused: its relation would have to be read inside the aggregate.
#[derive(Default)]
pub struct CpythonFunctors;

impl ProgramPass for CpythonFunctors {
    const NAME: &'static str = "functors only CPython gives";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let v = cx.v();
        let functor = |f: AtomId| CPYTHON_FUNCTORS.iter().find(|(g, _)| *g == v.text(f)).map(|&(_, rel)| rel);
        fn calls(v: &Version, t: TermRef<'_>, functor: &dyn Fn(AtomId) -> Option<&'static str>) -> bool {
            matches!(t, TermRef::Call(f, args) if functor(f).is_some() || args.clone().any(|a| calls(v, a, functor)))
        }
        let any = v.rules().any(|r| std::iter::once(r.head()).chain(r.body().filter_map(|l| l.atom())).any(|a| a.args().any(|t| calls(v, t, &functor))) || r.body().any(|l| matches!(l.view(), Lit::Cmp { l, r: rt, .. } if calls(v, l, &functor) || calls(v, rt, &functor))));
        if !any {
            return Ok(());
        }
        /// A term with each functor's call lifted to the variable that holds its value.
        fn lift(v: &Version, r: RuleRef<'_>, t: TermRef<'_>, functor: &dyn Fn(AtomId) -> Option<&'static str>, bound: &mut Vec<(&'static str, Tm, String)>) -> Tm {
            match t {
                TermRef::Call(f, args) => {
                    let args: Vec<Tm> = args.map(|a| lift(v, r, a, functor, bound)).collect();
                    if let (Some(rel), [arg]) = (functor(f), args.as_slice()) {
                        let var = match bound.iter().find(|(x, a, _)| *x == rel && a == arg) {
                            Some((_, _, n)) => n.clone(),
                            None => {
                                let n = format!("{rel}_{}", bound.len());
                                bound.push((rel, arg.clone(), n.clone()));
                                n
                            }
                        };
                        return Tm::Var(var);
                    }
                    Tm::Call(v.text(f).to_owned(), args)
                }
                t => Tm::of(v, r, t),
            }
        }
        let input = w.input().clone();
        let mut used: BTreeSet<&str> = BTreeSet::new();
        let b = w.derive()?;
        for r in input.rules() {
            let touches = std::iter::once(r.head()).chain(r.body().filter_map(|l| l.atom())).any(|a| a.args().any(|t| calls(&input, t, &functor))) || r.body().any(|l| matches!(l.view(), Lit::Cmp { l, r: rt, .. } if calls(&input, l, &functor) || calls(&input, rt, &functor)));
            if !touches {
                b.keep(&input, r);
                continue;
            }
            let mut bound: Vec<(&'static str, Tm, String)> = Vec::new();
            let mut body: Vec<Out<'_>> = Vec::new();
            for l in r.body() {
                match l.view() {
                    Lit::Atom(a) => body.push(Out::Made(Made::Atom { rel: input.rel_name(a.rel()).to_owned(), args: a.args().map(|t| lift(&input, r, t, &functor, &mut bound)).collect(), neg: a.neg(), gated: a.gated() })),
                    Lit::Cmp { l: x, op, r: y } => {
                        let x = lift(&input, r, x, &functor, &mut bound);
                        body.push(Out::Made(Made::Cmp { l: x, op, r: lift(&input, r, y, &functor, &mut bound) }));
                    }
                    Lit::Agg { atom, .. } if atom.args().any(|t| calls(&input, t, &functor)) => {
                        return Err(format!("@py_unparse or @ast_dump in an aggregate over {}: bind it to a variable outside the aggregate", input.rel_name(atom.rel())));
                    }
                    _ => body.push(Out::Own(l)),
                }
            }
            let head: Vec<Tm> = r.head().args().map(|t| lift(&input, r, t, &functor, &mut bound)).collect();
            let rel = b.rel(input.rel_name(r.head().rel()));
            let mut wr = b.rule().source(r.id());
            let hs: Vec<T<'static>> = head.iter().map(|t| t.write(wr.builder())).collect();
            wr = wr.head(rel, &hs);
            for o in &body {
                wr = match o {
                    Out::Own(l) => wr.lit(&input, r, *l),
                    Out::Made(m) => m.write(wr),
                };
            }
            for (x, arg, var) in &bound {
                used.insert(x);
                wr = Made::Atom { rel: (*x).to_owned(), args: vec![arg.clone(), Tm::Var(var.clone())], neg: false, gated: false }.write(wr);
            }
            wr.finish()?;
        }
        for rel in used {
            if input.decl_of(rel).is_none() {
                let cols = crate::code_rule_dsl::facts::core::cpython_relations::columns(rel);
                b.declare(rel, &[(cols[0], "node"), (cols[1], "sym")]);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
