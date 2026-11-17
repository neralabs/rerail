//! A session's changed strata maintain their rows from deltas (decision 0077 §3, stage 2): delete-and-rederive,
//! written as rules the native engine runs as it runs any program. One program, one evaluation, no second engine.
//!
//! For a changed relation `r` (an input whose rows changed, or a relation of a stratum that runs) the program has its
//! new rows under its own name, its old rows as input `r__old` (from what the last run kept), and its deltas:
//!
//! ```text
//! r__minus(x) :- r__old(x), !r(x).        rows gone
//! r__plus(x)  :- r(x), !r__old(x).        rows new
//! ```
//!
//! A stratum `S` whose rules are joins, negations and comparisons is maintained. For each rule `h :- B` and each
//! position `k` of `B` that can change (an atom of `S`, or of a changed relation):
//!
//! ```text
//! overdelete  h__del(..) :- B over old rows, k as its loss     (q__del, r__minus; a negated !n as n__plus)
//! survivors   h(..)      :- h__old(..), !h__del(..).
//! rederive    h__redo(..) :- h__del(..), B over new rows.        h(..) :- h__redo(..).
//! insert      h__ins(..) :- B over new rows, k as its gain     (q__ins, r__plus; a negated !n as n__minus, !n)
//!             h(..)      :- h__ins(..).
//! ```
//!
//! A maintained relation's net deltas, for the strata after it, are made from those three, each a delta's size: a
//! row gone was overdeleted and neither rederived nor inserted, a row new was inserted and not there before.
//!
//! ```text
//! h__minus(..) :- h__del(..), !h__redo(..), !h__ins(..).
//! h__plus(..)  :- h__ins(..), !h__old(..).
//! ```
//!
//! An input's, or a stratum's derived again, are a diff of its old and new rows (`r__minus`, `r__plus` above).
//!
//! An overdeleted row is a superset of the rows that lost every derivation; a survivor holds in the new rows. A new
//! row not among the old has a derivation using a gained row at some position, by induction on its derivation, so
//! the insert rules reach it. A head's computed term (`@dot(p, s)`) is first named: `h(@dot(p, s)) :- B` is read as
//! `h(v) :- B, v = @dot(p, s)`, so the rows above are rows of variables. A `nearest` literal reads the tree (`node`,
//! `child`): while neither changed it is copied as a comparison is, and once either did its stratum is derived
//! again.
//!
//! An aggregate over a changed relation `a` is a position too, by its groups: the atom's correlated variables `g`
//! over `a`'s rows gone and new are the groups whose value may have changed.
//!
//! ```text
//! grp(g)      :- a__minus(..).   grp(g) :- a__plus(..).
//! overdelete  h__del(..) :- grp(g), B over old rows (the aggregate over a__old)
//! insert      h__ins(..) :- grp(g), B over new rows
//! ```
//!
//! A head row that read an unchanged group read the same value before and after, so the other positions' rules cover
//! it (with the aggregate over old rows in an overdelete). A text test made natively is a filter of its input's
//! strings: its rows of strings gone are deleted, and only the new strings are tested (`h__ins` over `i__plus`, made
//! natively as the whole was). A stratum with another relation made natively, a lattice, an `ast` literal, or a
//! changed read whose old rows were not kept, is derived again from its inputs: its rules unchanged.
//! Either way its relations' deltas feed the strata after it, and an empty delta derives nothing there.

use super::retain::{Cut, Retained};
use super::{Error, Result, lower};
use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::logic::front::{self, Atom, Literal, Rule, Term};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// The part as a run evaluates it, the old rows it reads, and how its strata were evaluated.
pub(in crate::code_rule_dsl) struct Maintained {
    pub(in crate::code_rule_dsl) program: lower::Lowered,
    /// `r__old` for each changed relation read, from what was kept: a source, and for a derived relation the
    /// engine's relation, adopted by the caller (read in place).
    pub(in crate::code_rule_dsl) sources: Vec<(String, Arc<dyn Source>, Option<Arc<crate::code_rule_dsl::logic::native::component::InRel>>)>,
    /// Strata maintained from deltas, and derived again.
    pub(in crate::code_rule_dsl) maintained: usize,
    pub(in crate::code_rule_dsl) derived_again: usize,
    /// Of those maintained, strata that are recursive.
    pub(in crate::code_rule_dsl) recursive: usize,
    /// Each stratum derived again, by one of its relations, and why.
    pub(in crate::code_rule_dsl) again: Vec<(String, &'static str)>,
    /// The relations that are views of one input (`input_views`): never kept, their old rows and deltas the input's.
    pub(in crate::code_rule_dsl) views: BTreeSet<String>,
}

/// Whether views of one input are made (`RERAIL_SESSION_VIEWS=on`, `input_views`): a caller holding a version asks
/// before reading its tree.
pub(in crate::code_rule_dsl) fn views_on() -> bool {
    std::env::var("RERAIL_SESSION_VIEWS").is_ok_and(|v| v == "on")
}

/// The derived relations that are views of one input: one rule, one positive atom of an input, every column of the
/// input in the head (a variable the head names, or a constant), so a row of it is one row of the input. With
/// `RERAIL_SESSION_VIEWS=on` a session keeps such a relation as a view (decision 0090): its old rows are the view
/// over the input's old rows, its delta the view over the input's delta, and none of its rows are kept.
///
/// By default none is: a native run makes no views (decision 0104), so a view's stratum copied its input twice
/// each run, as itself and as its old rows (`kind__sup0` and `kind__sup0__old` each a scan of `node`'s 243,092
/// rows, 0.8 s of an O125 step for four such views). Kept and maintained as other relations, a view costs its
/// delta, and its rows' memory (about 10 MB each there).
///
/// Read from the program's version by its handles: each such relation, with the input its rule reads.
pub(in crate::code_rule_dsl) fn input_views(p: &Placed) -> Result<BTreeMap<String, String>> {
    if !views_on() {
        return Ok(BTreeMap::new());
    }
    let (v, f) = (&p.v, p.fields()?);
    let mut by: BTreeMap<&str, Vec<RuleRef<'_>>> = BTreeMap::new();
    for r in v.rules() {
        by.entry(v.rel_name(r.head().rel())).or_default().push(r);
    }
    Ok(by
        .into_iter()
        .filter_map(|(rel, rules)| {
            let [rule] = rules.as_slice() else { return None };
            let mut body = rule.body();
            let (Some(l), None) = (body.next(), body.next()) else { return None };
            let PLit::Atom(a) = l.view() else { return None };
            let input = v.rel_name(a.rel());
            if a.neg() || a.gated() || !f.is_input(input) || f.lattice(rel) || f.native(rel).is_some() || f.facts(rel).is_some() || f.is_output(rel) {
                return None;
            }
            let head: BTreeSet<_> = rule.head().args().filter_map(|t| t.var()).collect();
            let terms_ok = rule.head().args().all(|t| matches!(t, TermRef::Var(_) | TermRef::Int(_) | TermRef::Str(_)));
            let one_to_one = a.args().all(|t| match t {
                TermRef::Var(s) => head.contains(&s),
                TermRef::Int(_) | TermRef::Str(_) => true,
                _ => false,
            });
            (terms_ok && one_to_one).then(|| (rel.to_owned(), input.to_owned()))
        })
        .collect())
}

fn named(rel: &str, suffix: &str) -> String {
    format!("{rel}__{suffix}")
}

/// Whether recursive strata are maintained by levels (decision 0090), not delete-and-rederive.
pub(in crate::code_rule_dsl) fn levels_wanted() -> bool {
    // The default (decision 0090, levels checked); `RERAIL_RECURSIVE=dred` keeps delete-and-rederive.
    std::env::var("RERAIL_RECURSIVE").map_or(true, |v| v != "dred")
}

/// `rule` with each computed head term named by a variable the body binds: `h(@f(x)) :- B` as `h(v) :- B, v = @f(x)`.
fn named_heads(rule: &Rule) -> Rule {
    let mut rule = rule.clone();
    for (i, t) in rule.head.args.iter_mut().enumerate() {
        if matches!(t, Term::Call(..)) {
            let v = Term::Var(format!("_h{i}").into());
            let call = std::mem::replace(t, v.clone());
            rule.body.push(Literal::Cmp { l: v, op: "=", r: call });
        }
    }
    rule
}

/// `part` (the cut's program) with each changed stratum maintained where it can be, and the old rows it reads.
/// `given` answers for an input whose delta it has (its rows new and gone, found by its documents), bound as
/// `r__plus` and `r__minus` in place of their diff of every old and new row; asked only for the inputs a maintained
/// stratum reads the delta of. Written over the cut program's version (`rewrite_over`) and read back as a tree by
/// the session's run until it takes a version (t36 item 7 step 9).
///
/// One stratum's form, made when a run reaches it (decision 0117 step 3): the rewrite over that stratum alone
/// (`form`). Its relations' deltas are always made, since the run asks whether they are empty; a delta or an old
/// source that is a relation of the run already (an earlier stratum's form made or bound it) is read, not made again.
pub(in crate::code_rule_dsl) struct One<'m> {
    pub(in crate::code_rule_dsl) has_delta: &'m dyn Fn(&str) -> bool,
    pub(in crate::code_rule_dsl) has_old: &'m dyn Fn(&str) -> bool,
}

/// `rewrite`'s `sccs`: `cross`'s strata, computed once by the caller (as `retain::cut` was given them).
pub(in crate::code_rule_dsl) fn rewrite(cut: &Cut, kept: Option<&Retained>, cross: &lower::Lowered, sccs: &[BTreeSet<String>], given: &mut dyn FnMut(&str) -> Option<(Arc<dyn Source>, Arc<dyn Source>)>) -> Result<Maintained> {
    let part = cut.program(cross)?;
    // The cut program's version (a part of the cross part's, decision 0244).
    let cut_v = &part.version;
    let p = Placed::new(cut_v.clone(), cut_v.rules().map(|r| r.id()).collect(), true);
    let f = Fields::of(&p).ok_or_else(|| Error::Plan("the cut program has no image".into()))?;
    let cross_placed = cross.placed()?;
    let gates = !cross_placed.fields()?.gates().is_empty();
    let src = {
        let mut answers: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for &(q, by) in f.answers() {
            answers.entry(f.rel_name(q).to_owned()).or_default().push(f.rel_name(by).to_owned());
        }
        Part {
            p: &p,
            f,
            rules: (0..p.v.rules().count()).collect(),
            inputs: f.inputs().map(str::to_owned).collect(),
            outputs: f.outputs().map(str::to_owned).collect(),
            gates,
            natives: f.natives().iter().map(|n| (f.rel_name(n.rel).to_owned(), n.reads.iter().map(|r| f.rel_name(*r).to_owned()).collect())).collect(),
            answers: answers.into_iter().collect(),
            domains: part.domains.clone(),
        }
    };
    let mut over = rewrite_over(&src, Builder::derive(&p.v), cut, kept.map(|k| k as &dyn Olds), sccs, given, None)?;
    if let Some(k) = kept {
        bind_olds(&mut over, k);
    }
    over.b.set_inputs(&over.inputs);
    let (v, _) = over.b.finish().map_err(Error::Plan)?;
    // The run reads `v` and its `Maint`.
    let maint = match over.groups.is_empty() && over.together.is_empty() {
        true => None,
        false => Some(Arc::new(Maint::written(&v, &p.v, &over.groups, &over.together)?)),
    };
    if let Ok(show) = std::env::var("O125_RULES") {
        for r in v.rules().filter(|r| v.rel_name(r.head().rel()).starts_with(show.as_str())) {
            eprintln!("O125_RULES {}", v.show_rule(r));
        }
        let given = &cross_placed.v;
        for r in given.rules().filter(|r| given.rel_name(r.head().rel()) == show) {
            eprintln!("O125_RULES given {}", given.show_rule(r));
        }
    }
    // The views of one input (`input_views`), over the whole program: where the part is maintained, those whose
    // input's old rows are kept.
    let views = match kept.filter(|_| !gates) {
        _ if !views_on() => BTreeSet::new(),
        None => input_views(&cross_placed)?.into_keys().collect(),
        Some(k) => input_views(&cross_placed)?.into_iter().filter(|(_, input)| k.has_old(input)).map(|(rel, _)| rel).collect(),
    };
    Ok(Maintained { program: lower::Lowered { version: v, maint, domains: over.domains, ops: part.ops, dialect: part.dialect }, sources: over.sources, maintained: over.maintained, derived_again: over.derived_again, recursive: over.recursive, again: over.again, views })
}

// -- The rewrite over a version (decision 0241) --

use crate::code_rule_dsl::logic::plan::body::{AgainRef, Group, Maint, Placed};
use crate::code_rule_dsl::logic::plan::fields::Fields;
use crate::code_rule_dsl::logic::program_image::manager::{StepKey, StepRecord, Steps};
use crate::code_rule_dsl::logic::program_image::{AtomRef, Builder, CmpOp, Lit as PLit, LitRef, NativeSpec, RelId, Rewrite, RuleId, RuleRef, TermRef, Version, T};

/// What the rewrite reads: a part of a program, by its version.
pub(in crate::code_rule_dsl) struct Part<'v> {
    /// The version the part's rules are rules of, and its fields: a session's cut, or the run's version for one
    /// stratum's form. Every relation the rewrite asks about is one the part mentions, so the run's fields answer
    /// for a form as its one-stratum program did.
    pub(in crate::code_rule_dsl) p: &'v Placed,
    pub(in crate::code_rule_dsl) f: Fields<'v>,
    /// The part's rules, as places among `p`'s program's rules, in order.
    pub(in crate::code_rule_dsl) rules: Vec<usize>,
    /// The part's inputs and outputs, in order.
    pub(in crate::code_rule_dsl) inputs: Vec<String>,
    pub(in crate::code_rule_dsl) outputs: Vec<String>,
    /// Whether the part's program has gates (it is then not maintained).
    pub(in crate::code_rule_dsl) gates: bool,
    /// The part's natives (each relation and what it reads) and answers (each request and what it produces), for
    /// `together`'s check of its sets (`front::apart`).
    pub(in crate::code_rule_dsl) natives: Vec<(String, Vec<String>)>,
    pub(in crate::code_rule_dsl) answers: Vec<(String, Vec<String>)>,
    pub(in crate::code_rule_dsl) domains: lower::Domains,
}

/// The part maintained, written into the builder it was given (its rules and the parts it adds), with what the
/// caller states or binds after it.
pub(in crate::code_rule_dsl) struct Over<'b> {
    pub(in crate::code_rule_dsl) b: Builder<'b>,
    /// The part's inputs, and those it adds (old rows, given deltas, the deltas a run holds), in order: stated by
    /// the caller.
    pub(in crate::code_rule_dsl) inputs: Vec<String>,
    pub(in crate::code_rule_dsl) domains: lower::Domains,
    pub(in crate::code_rule_dsl) groups: Vec<Group>,
    pub(in crate::code_rule_dsl) together: Vec<BTreeSet<String>>,
    /// The relations the rules written derive.
    pub(in crate::code_rule_dsl) heads: BTreeSet<String>,
    pub(in crate::code_rule_dsl) sources: Vec<(String, Arc<dyn Source>, Option<Arc<crate::code_rule_dsl::logic::native::component::InRel>>)>,
    /// The relations whose old rows (`r__old`) the rules read and the caller binds (`bind_olds`).
    pub(in crate::code_rule_dsl) old_reads: Vec<String>,
    pub(in crate::code_rule_dsl) maintained: usize,
    pub(in crate::code_rule_dsl) derived_again: usize,
    pub(in crate::code_rule_dsl) recursive: usize,
    pub(in crate::code_rule_dsl) by_levels: usize,
    pub(in crate::code_rule_dsl) again: Vec<(String, &'static str)>,
    pub(in crate::code_rule_dsl) views: BTreeSet<String>,
}

/// What a form reads of a run's kept part (decision 0298): so it is written without one, its old rows bound after.
pub(in crate::code_rule_dsl) trait Olds {
    /// Whether `rel`'s rows as the last run had them are kept.
    fn has_old(&self, rel: &str) -> bool;
    /// Whether `rel`'s kept rows hold their levels (decision 0090), and their shape; `None` where it is not kept.
    fn levels(&self, rel: &str) -> Option<(bool, (usize, usize, bool))>;
}

impl Olds for Retained {
    fn has_old(&self, rel: &str) -> bool {
        Retained::has_old(self, rel)
    }
    fn levels(&self, rel: &str) -> Option<(bool, (usize, usize, bool))> {
        self.kept(rel).map(|(h, _)| (h.has_levels(), h.levels_shape()))
    }
}

/// Binds the old rows `over` reads (`Over::old_reads`) from the kept part, beside its other sources.
fn bind_olds(over: &mut Over<'_>, kept: &Retained) {
    bind_old_rows(&mut over.sources, std::mem::take(&mut over.old_reads), kept);
}

/// Each of `rels`' old rows (`r__old`) from the kept part, as a source, after `sources`.
#[allow(clippy::type_complexity)]
pub(in crate::code_rule_dsl) fn bind_old_rows(sources: &mut Vec<(String, Arc<dyn Source>, Option<Arc<crate::code_rule_dsl::logic::native::component::InRel>>)>, rels: Vec<String>, kept: &Retained) {
    for rel in rels {
        let (source, held) = kept.old_source(&rel, &named(&rel, "old")).expect("a maintained read's old rows");
        sources.push((named(&rel, "old"), source, held));
    }
}

#[derive(Clone, Debug)]
enum Tm {
    Var(String),
    Int(i64),
    Wild,
}

/// A literal of a rule the rewrite writes.
#[derive(Clone, Debug)]
enum L {
    /// Body literal `i` of the source rule, its atom (its own or its aggregate's) read as `rel` with flags
    /// `(neg, gated)` where given.
    Src { i: usize, rel: Option<String>, flags: Option<(bool, bool)> },
    /// The comparison naming head term `i` (`named_heads`): `_h{i} = t`.
    HeadName(usize),
    /// An atom over literal `i`'s atom's arguments.
    ArgsOf { rel: String, i: usize, neg: bool, gated: bool },
    /// An atom over the source rule's head's arguments, each computed one named (`_h{i}`).
    HeadArgs { rel: String, neg: bool },
    Atom { rel: String, args: Vec<Tm>, neg: bool, gated: bool },
    /// A text test of term `at`: `@op(at, @set(patterns)) = 1` (`strings::test_literal`).
    Test { at: Tm, op: String, patterns: Vec<String> },
}

#[derive(Clone, Debug)]
enum H {
    /// The source rule's head, each computed term named, as relation `rel`.
    Named(String),
    /// The source rule's head as given, as relation `rel`.
    Given(String),
    Terms(String, Vec<Tm>),
}

/// A rule the rewrite writes: from a source rule (its place among the part's rules) or none, or that rule copied as
/// it is.
#[derive(Clone, Debug)]
enum Ir {
    Copy(usize),
    Rule { src: Option<usize>, head: H, body: Vec<L> },
}

/// One source rule read as `named_heads` reads it: its literals, then a comparison naming each computed head term.
struct Named<'v> {
    v: &'v Version,
    r: RuleRef<'v>,
    lits: Vec<LitRef<'v>>,
    calls: Vec<usize>,
}

impl<'v> Named<'v> {
    fn of(v: &'v Version, r: RuleRef<'v>) -> Named<'v> {
        let calls = r.head().args().enumerate().filter(|(_, t)| matches!(t, TermRef::Call(..))).map(|(i, _)| i).collect();
        Named { v, r, lits: r.body().collect(), calls }
    }
    fn len(&self) -> usize {
        self.lits.len() + self.calls.len()
    }
    fn head(&self) -> &'v str {
        self.v.rel_name(self.r.head().rel())
    }
    /// Literal `j`'s atom (its own or its aggregate's), if it reads one.
    fn atom(&self, j: usize) -> Option<AtomRef<'v>> {
        self.lits.get(j).and_then(|l| l.atom())
    }
    fn lit(&self, j: usize) -> Option<PLit<'v>> {
        self.lits.get(j).map(|l| l.view())
    }
    fn rel(&self, a: AtomRef<'_>) -> &'v str {
        self.v.rel_name(a.rel())
    }
    /// Its body as given.
    fn body(&self) -> Vec<L> {
        (0..self.lits.len()).map(|i| L::Src { i, rel: None, flags: None }).chain(self.calls.iter().map(|&p| L::HeadName(p))).collect()
    }
    /// Whether its (named) head is of variables and constants only.
    fn plain_head(&self) -> bool {
        self.r.head().args().all(|t| matches!(t, TermRef::Var(_) | TermRef::Int(_) | TermRef::Str(_) | TermRef::Call(..)))
    }
}

/// A term of `v`'s rule `r`, to write into a builder under `v`'s ids.
fn t_of<'v>(v: &'v Version, r: RuleRef<'v>, t: TermRef<'v>) -> T<'v> {
    match t {
        TermRef::Var(s) => T::VarAtom(r.var(s)),
        TermRef::Wild => T::Wild,
        TermRef::Top => T::Top,
        TermRef::Int(i) => T::Int(i),
        TermRef::Str(a) => T::StrAtom(a),
        TermRef::Call(f, args) => T::Call(v.text(f), args.map(|a| t_of(v, r, a)).collect()),
    }
}

fn tm(t: &Tm) -> T<'_> {
    match t {
        Tm::Var(v) => T::Var(v),
        Tm::Int(i) => T::Int(*i),
        Tm::Wild => T::Wild,
    }
}

/// The atom of a copied literal written as another relation, or with other flags.
struct AtomAs(Option<(String, bool, bool)>);

impl Rewrite for AtomAs {
    fn atom_as(&mut self, _a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        self.0.clone()
    }
}

impl<'v> Part<'v> {
    fn named(&self, k: usize) -> Named<'v> {
        Named::of(&self.p.v, self.p.rule(self.rules[k]))
    }
    fn width(&self, rel: &str, made: &HashMap<String, usize>) -> usize {
        made.get(rel).copied().or_else(|| self.f.arity(rel)).unwrap_or_else(|| panic!("no declaration of {rel}"))
    }
}

/// Writes rule `ir` into `b`, a builder under the source's ids.
fn write_ir(b: &mut Builder<'_>, src: &Part<'_>, ir: &Ir) -> std::result::Result<RuleId, String> {
    let v: &Version = &src.p.v;
    match ir {
        Ir::Copy(k) => Ok(b.copy(v, src.p.rule(src.rules[*k]))),
        Ir::Rule { src: s, head, body } => {
            let n = s.map(|k| src.named(k));
            let names: Vec<String> = n.as_ref().map_or_else(Vec::new, |n| n.calls.iter().map(|p| format!("_h{p}")).collect());
            let mut rb = b.rule();
            // Its source rule, which its plan starts from (decision 0258).
            if let Some(k) = s {
                rb = rb.source(src.p.id(src.rules[*k]));
            }
            fn named_head<'a>(n: &Named<'a>, names: &'a [String]) -> Vec<T<'a>> {
                n.r.head().args().enumerate().map(|(i, t)| match t {
                    TermRef::Call(..) => T::Var(names[n.calls.iter().position(|&p| p == i).expect("a call")].as_str()),
                    t => t_of(n.v, n.r, t),
                }).collect()
            }
            rb = match head {
                H::Named(rel) => {
                    let n = n.as_ref().expect("a source rule");
                    rb.head(rel.as_str(), &named_head(n, &names))
                }
                H::Given(rel) => {
                    let n = n.as_ref().expect("a source rule");
                    rb.head(rel.as_str(), &n.r.head().args().map(|t| t_of(n.v, n.r, t)).collect::<Vec<_>>())
                }
                H::Terms(rel, args) => rb.head(rel.as_str(), &args.iter().map(tm).collect::<Vec<_>>()),
            };
            for l in body {
                rb = match l {
                    L::Src { i, rel, flags } => {
                        let n = n.as_ref().expect("a source rule");
                        let lit = n.lits[*i];
                        match (rel, flags, lit.atom()) {
                            (None, None, _) | (_, _, None) => rb.lit(v, n.r, lit),
                            (rel, flags, Some(a)) => {
                                let rel = rel.clone().unwrap_or_else(|| n.rel(a).to_owned());
                                let (neg, gated) = flags.unwrap_or((a.neg(), a.gated()));
                                rb.lit_with(v, n.r, lit, &mut AtomAs(Some((rel, neg, gated))))
                            }
                        }
                    }
                    L::HeadName(p) => {
                        let n = n.as_ref().expect("a source rule");
                        let t = n.r.head().args().nth(*p).expect("a head term");
                        let name = &names[n.calls.iter().position(|q| q == p).expect("a call")];
                        rb.cmp(T::Var(name), CmpOp::Eq, t_of(v, n.r, t))
                    }
                    L::ArgsOf { rel, i, neg, gated } => {
                        let n = n.as_ref().expect("a source rule");
                        let a = n.lits[*i].atom().expect("an atom");
                        let args: Vec<T<'_>> = a.args().map(|t| t_of(v, n.r, t)).collect();
                        rb.atom_with(rel.as_str(), &args, *neg, *gated)
                    }
                    L::HeadArgs { rel, neg } => {
                        let n = n.as_ref().expect("a source rule");
                        rb.atom_with(rel.as_str(), &named_head(n, &names), *neg, false)
                    }
                    L::Atom { rel, args, neg, gated } => rb.atom_with(rel.as_str(), &args.iter().map(tm).collect::<Vec<_>>(), *neg, *gated),
                    L::Test { at, op, patterns } => {
                        let set = T::Call(crate::code_rule_dsl::logic::strings::SET, patterns.iter().map(|p| T::Str(p)).collect());
                        rb.cmp(T::Call(op, vec![tm(at), set]), CmpOp::Eq, T::Int(1))
                    }
                };
            }
            rb.finish()
        }
    }
}

/// What IR rule `ir` reads: each body atom's relation (its own or its aggregate's), whether negated, and whether an
/// aggregate's.
fn reads_ir(src: &Part<'_>, ir: &Ir) -> Vec<(String, bool, bool)> {
    match ir {
        Ir::Copy(k) => {
            let n = src.named(*k);
            n.lits.iter().filter_map(|l| l.atom().map(|a| (n.rel(a).to_owned(), a.neg(), matches!(l.view(), PLit::Agg { .. })))).collect()
        }
        Ir::Rule { src: s, body, .. } => {
            let n = s.map(|k| src.named(k));
            body.iter().filter_map(|l| match l {
                L::Src { i, rel, flags } => {
                    let n = n.as_ref().expect("a source rule");
                    let a = n.lits[*i].atom()?;
                    let agg = matches!(n.lits[*i].view(), PLit::Agg { .. });
                    Some((rel.clone().unwrap_or_else(|| n.rel(a).to_owned()), flags.map_or(a.neg(), |f| f.0), agg))
                }
                L::HeadName(_) | L::Test { .. } => None,
                L::ArgsOf { rel, neg, .. } | L::HeadArgs { rel, neg } | L::Atom { rel, neg, .. } => Some((rel.clone(), *neg, false)),
            }).collect()
        }
    }
}

/// The relation IR rule `ir` derives.
fn head_ir(src: &Part<'_>, ir: &Ir) -> String {
    match ir {
        Ir::Copy(k) => src.named(*k).head().to_owned(),
        Ir::Rule { head: H::Named(r) | H::Given(r) | H::Terms(r, _), .. } => r.clone(),
    }
}

/// An insert rule of `h`, carrying only rows new against the survivors (`h__old` and not `h__del`): an insert
/// stops where a row survived, as semi-naive evaluation stops at a row already held. Without it one new edge
/// derived a recursive relation's whole closure again (9,000,000 inserted rows for 2 new, `session_costs`).
/// Not a survivor is not old, or deleted: two rules, each a lower stratum's read. A head with a call keeps the
/// rule unguarded.
fn insert_ir(rules: &mut Vec<Ir>, n: &Named<'_>, k: usize, h: &str, body: Vec<L>, gone: &str) {
    let head = H::Named(named(h, "ins"));
    if !n.r.head().args().all(|t| matches!(t, TermRef::Var(_) | TermRef::Int(_) | TermRef::Str(_) | TermRef::Call(..))) {
        rules.push(Ir::Rule { src: Some(k), head, body });
        return;
    }
    let mut new = body.clone();
    new.push(L::HeadArgs { rel: named(h, "old"), neg: true });
    rules.push(Ir::Rule { src: Some(k), head: head.clone(), body: new });
    let mut deleted = body;
    deleted.push(L::HeadArgs { rel: named(h, gone), neg: false });
    rules.push(Ir::Rule { src: Some(k), head, body: deleted });
}

/// States `rel` a text test (`NativeSpec::TextTest`) of `input`'s column `column`: its about rows
/// (`native_text_test`, and `native_text_pattern` for each pattern in order), as `about_of` writes a program's.
fn text_test(b: &mut Builder<'_>, rel: &str, input: &str, column: usize, op: &str, patterns: &[String]) {
    use crate::code_rule_dsl::logic::program_image::Cell;
    let p = Cell::Str(b.atom(crate::code_rule_dsl::logic::program_image::ABOUT));
    let r = Cell::Str(b.atom(rel));
    let mut tests = b.about_rows("native_text_test");
    tests.push(vec![p, r, Cell::Str(b.atom(input)), Cell::Int(column as i64), Cell::Str(b.atom(op))]);
    b.set_about_rows("native_text_test", tests);
    let mut rows = b.about_rows("native_text_pattern");
    for (i, pattern) in patterns.iter().enumerate() {
        rows.push(vec![p, r, Cell::Int(i as i64), Cell::Str(b.atom(pattern))]);
    }
    b.set_about_rows("native_text_pattern", rows);
}

fn row_tm(width: usize) -> Vec<Tm> {
    (0..width).map(|i| Tm::Var(format!("_m{i}"))).collect()
}

fn atom_tm(rel: String, args: Vec<Tm>, neg: bool) -> L {
    L::Atom { rel, args, neg, gated: false }
}

/// `input_views` over a version: each such relation and its rule's place among the part's rules.
fn input_views_over(src: &Part<'_>) -> BTreeMap<String, usize> {
    if !views_on() {
        return BTreeMap::new();
    }
    let mut by: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for k in 0..src.rules.len() {
        let v: &Version = &src.p.v;
        by.entry(v.rel_name(src.p.rule(src.rules[k]).head().rel())).or_default().push(k);
    }
    let f = src.f;
    by.into_iter()
        .filter_map(|(rel, ks)| {
            let [k] = ks.as_slice() else { return None };
            let n = src.named(*k);
            let [l] = n.lits.as_slice() else { return None };
            let PLit::Atom(a) = l.view() else { return None };
            if a.neg() || a.gated() || !src.inputs.iter().any(|x| x == n.rel(a)) || f.lattice(rel) || f.native(rel).is_some() || f.facts(rel).is_some() || src.outputs.iter().any(|o| o == rel) {
                return None;
            }
            let head: BTreeSet<_> = n.r.head().args().filter_map(|t| t.var()).collect();
            let terms_ok = n.r.head().args().all(|t| matches!(t, TermRef::Var(_) | TermRef::Int(_) | TermRef::Str(_)));
            let one_to_one = a.args().all(|t| match t {
                TermRef::Var(s) => head.contains(&s),
                TermRef::Int(_) | TermRef::Str(_) => true,
                _ => false,
            });
            (terms_ok && one_to_one).then(|| (rel.to_owned(), *k))
        })
        .collect()
}

/// Why a stratum's rules cannot be maintained, if they cannot: joins, negations, comparisons, `nearest` and aggregates over
/// earlier strata only, heads of variables and constants (their computed terms named, `Named`), and every read that changed has its old rows.
fn unmaintainable_over(f: Fields<'_>, scc: &BTreeSet<String>, rules: &[Named<'_>], changed: &BTreeSet<String>, old: &dyn Fn(&str) -> bool) -> Option<&'static str> {
    for r in scc {
        if f.native(r).is_some() {
            return Some("a relation made natively");
        }
        if f.lattice(r) {
            return Some("a lattice");
        }
        if !old(r) {
            return Some("its own old rows not kept");
        }
    }
    if super::supports::counting()
        && std::env::var("RERAIL_COUNT_RECURSIVE").is_ok_and(|v| v == "again")
        && rules.iter().any(|r| r.lits.iter().any(|l| l.atom().is_some_and(|a| scc.contains(r.rel(a)))))
    {
        return Some("recursive, under counting");
    }
    for rule in rules {
        if !rule.plain_head() {
            return Some("a head term");
        }
        for l in &rule.lits {
            match l.view() {
                PLit::Atom(a) if a.gated() => return Some("a gated negation"),
                PLit::Atom(a) if changed.contains(rule.rel(a)) && !scc.contains(rule.rel(a)) && !old(rule.rel(a)) => return Some("a changed read's old rows not kept"),
                PLit::Nearest { .. } if super::retain::TREE.iter().any(|r| changed.contains(*r)) => return Some("a nearest literal over a changed tree"),
                PLit::Atom(_) | PLit::Cmp { .. } | PLit::Nearest { .. } => {}
                PLit::Agg { atom: a, .. } if a.gated() => return Some("a gated negation"),
                PLit::Agg { atom: a, .. } if scc.contains(rule.rel(a)) => return Some("an aggregate over its own stratum"),
                PLit::Agg { atom: a, .. } if a.args().any(|t| matches!(t, TermRef::Call(..))) => return Some("an aggregate over a computed term"),
                PLit::Agg { atom: a, .. } if changed.contains(rule.rel(a)) && !old(rule.rel(a)) => return Some("a changed read's old rows not kept"),
                PLit::Agg { .. } => {}
                PLit::Ast { .. } => return Some("an ast literal"),
            }
        }
    }
    None
}

/// Source rule `k` as a tree, its computed head terms named: for the debugging aids that print rules.
fn tree_named(src: &Part<'_>, k: usize) -> Rule {
    let lists = front::Lists::default();
    named_heads(&crate::code_rule_dsl::logic::program_image::Tree::new(&src.p.v, &lists).rule(src.p.rule(src.rules[k])))
}

/// The part `src` with each changed stratum maintained where it can be, and the old rows it reads, over its version
/// (decision 0241): its rules and the parts it adds (declarations, nullary relations, gates) written into `b`, a
/// builder under `src`'s ids that holds the part's other parts as the caller stated them. Its inputs, and what the
/// caller binds and counts, are returned beside it.
pub(in crate::code_rule_dsl) fn rewrite_over<'b>(src: &Part<'_>, mut b: Builder<'b>, cut: &Cut, kept: Option<&dyn Olds>, sccs: &[BTreeSet<String>], given: &mut dyn FnMut(&str) -> Option<(Arc<dyn Source>, Arc<dyn Source>)>, one: Option<&One>) -> Result<Over<'b>> {
    let f = src.f;
    let dirty: Vec<&BTreeSet<String>> = sccs.iter().filter(|scc| scc.iter().all(|r| !cut.clean.contains(r))).collect();
    let why = if kept.is_none() { "nothing kept" } else { "the program has gates" };
    let Some(kept) = kept.filter(|_| !src.gates) else {
        let again = dirty.iter().map(|scc| (scc.iter().next().cloned().unwrap_or_default(), why)).collect();
        let views = input_views_over(src).into_keys().collect();
        let mut heads = BTreeSet::new();
        for k in 0..src.rules.len() {
            heads.insert(head_ir(src, &Ir::Copy(k)));
            write_ir(&mut b, src, &Ir::Copy(k)).map_err(Error::Plan)?;
        }
        return Ok(Over { b, inputs: src.inputs.clone(), domains: src.domains.clone(), groups: Vec::new(), together: Vec::new(), heads, sources: Vec::new(), old_reads: Vec::new(), maintained: 0, derived_again: dirty.len(), recursive: 0, by_levels: 0, again, views });
    };
    let views: BTreeMap<String, usize> = input_views_over(src).into_iter().filter(|(_, k)| src.named(*k).atom(0).is_some_and(|a| kept.has_old(src.named(*k).rel(a)))).collect();
    let view_input = |rel: &str| -> Option<String> { views.get(rel).and_then(|&k| { let n = src.named(k); n.atom(0).map(|a| n.rel(a).to_owned()) }) };
    let old = |rel: &str| kept.has_old(rel) || views.contains_key(rel);
    let mut changed: BTreeSet<String> = cut.changed.iter().cloned().collect();
    let mut rules: Vec<Ir> = Vec::new();
    let mut olds: BTreeSet<String> = BTreeSet::new();
    let mut deltas: BTreeSet<String> = BTreeSet::new();
    let (mut maintained, mut derived_again, mut recursive, mut by_levels) = (0, 0, 0, 0);
    let mut again: Vec<(String, &'static str)> = Vec::new();
    let mut fresh: Vec<(String, String)> = Vec::new();
    let mut maint: BTreeSet<String> = BTreeSet::new();
    let mut gone_of: BTreeSet<String> = BTreeSet::new();
    let mut exact_rels: BTreeSet<String> = BTreeSet::new();
    let mut groups: Vec<(BTreeSet<String>, usize, usize, Vec<usize>)> = Vec::new();
    let mut inputs = src.inputs.clone();
    let mut domains = src.domains.clone();
    // The widths of the relations the rewrite declares (groups, pieces, gates), and the relations of no columns it
    // adds.
    let mut made: HashMap<String, usize> = HashMap::new();
    let mut nullary_added: Vec<String> = Vec::new();
    let mut by_head: HashMap<&str, Vec<usize>> = HashMap::new();
    for k in 0..src.rules.len() {
        let v: &Version = &src.p.v;
        by_head.entry(v.rel_name(src.p.rule(src.rules[k]).head().rel())).or_default().push(k);
    }
    let count_of = |rel: &str| by_head.get(rel).map_or(0, Vec::len);
    for scc in dirty {
        let mut at: Vec<usize> = scc.iter().filter_map(|r| by_head.get(r.as_str())).flatten().copied().collect();
        at.sort_unstable();
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let Some(&view) = views.get(*rel)
        {
            rules.push(Ir::Copy(view));
            if view_input(rel).is_some_and(|a| changed.contains(&a)) {
                changed.insert((*rel).clone());
            }
            continue;
        }
        let only = std::env::var("RERAIL_CROSS_MAINTAIN_ONLY").ok().map(|v| v.split(',').filter(|x| !x.is_empty()).map(str::to_owned).collect::<BTreeSet<String>>());
        // A text test made natively (`front::Native::TextTest`) is a filter of its input's strings: a row is there
        // while its string is and passes. It is maintained exactly: the rows of strings gone are deleted, and only
        // the new strings are tested, natively as the whole input was (`h__ins` over `i__plus`).
        if let [rel] = scc.iter().collect::<Vec<_>>().as_slice()
            && let Some(NativeSpec::TextTest { input, column, op, patterns }) = f.native(rel).map(|n| &n.spec)
            && let input = f.rel_name(*input).to_owned()
            && changed.contains(&input)
            && old(rel)
            && old(&input)
            && only.as_ref().is_none_or(|o| o.contains(*rel))
        {
            let (op, patterns): (String, Vec<String>) = (f.text(*op).to_owned(), patterns.iter().map(|&p| f.text(p).to_owned()).collect());
            maintained += 1;
            let start = rules.len();
            let v = row_tm(src.width(rel, &made));
            let head = |r: String| H::Terms(r, v.clone());
            rules.push(Ir::Rule { src: None, head: head(named(rel, "del")), body: vec![atom_tm(named(rel, "old"), v.clone(), false), atom_tm(named(&input, "minus"), v.clone(), false)] });
            // The test in the rule too: a maintained form's pieces are rules (`plan`'s maintained run), which test
            // the new strings row by row.
            let test = L::Test { at: v[*column].clone(), op: op.clone(), patterns: patterns.clone() };
            rules.push(Ir::Rule { src: None, head: head(named(rel, "ins")), body: vec![atom_tm(named(&input, "plus"), v.clone(), false), test] });
            text_test(&mut b, &named(rel, "ins"), &named(&input, "plus"), *column, &op, &patterns);
            // Its rules are now its old rows and the new ones, which the run makes as rules (`text_test_defined`):
            // it stays a native, so a run that derives it again (`Again`) makes it from the vocabulary.
            rules.push(Ir::Rule { src: None, head: head((*rel).clone()), body: vec![atom_tm(named(rel, "old"), v.clone(), false), atom_tm(named(rel, "del"), v.clone(), true)] });
            rules.push(Ir::Rule { src: None, head: head((*rel).clone()), body: vec![atom_tm(named(rel, "ins"), v.clone(), false)] });
            for piece in ["del", "ins"] {
                fresh.push(((*rel).clone(), named(rel, piece)));
            }
            olds.insert((*rel).clone());
            olds.insert(input.clone());
            deltas.insert(input);
            exact_rels.insert((*rel).clone());
            maint.insert((*rel).clone());
            changed.insert((*rel).clone());
            groups.push((scc.clone(), start, rules.len(), at));
            continue;
        }
        let mine: Vec<Named<'_>> = at.iter().map(|&k| src.named(k)).collect();
        let why = if only.as_ref().is_some_and(|o| !scc.iter().any(|r| o.contains(r))) { Some("not chosen") } else { unmaintainable_over(f, scc, &mine, &changed, &old) };
        if std::env::var_os("O125_LIST").is_some() && why.is_none() {
            eprintln!("O125_LIST {}", scc.iter().next().unwrap());
        }
        if let Some(why) = why {
            let name = scc.iter().max_by_key(|r| count_of(r)).cloned().unwrap_or_default();
            again.push((format!("{name} ({} relations)", scc.len()), why));
            derived_again += 1;
            rules.extend(at.iter().map(|&k| Ir::Copy(k)));
            changed.extend(scc.iter().cloned());
            continue;
        }
        maintained += 1;
        let start = rules.len();
        let exact = scc.iter().all(|r| super::supports::is_support(r));
        let projection = !exact
            && scc.len() == 1
            && mine.iter().all(|r| r.len() == 1 && matches!(r.lit(0), Some(PLit::Atom(a)) if !a.neg() && super::supports::is_support(r.rel(a))));
        let lost = |h: &str| if projection { named(h, "lost") } else { named(h, "del") };
        let is_recursive = mine.iter().any(|r| r.lits.iter().any(|l| l.atom().is_some_and(|a| scc.contains(r.rel(a)))));
        if is_recursive {
            recursive += 1;
        }
        let joins = |r: &Named<'_>| r.lits.iter().all(|l| matches!(l.view(), PLit::Atom(_) | PLit::Cmp { .. }));
        let lv = is_recursive && levels_wanted() && mine.iter().all(joins) && scc.iter().all(|r| kept.levels(r).is_some_and(|(has, _)| has));
        if lv {
            by_levels += 1;
            BY_LEVELS.with(|n| n.set(n.get() + 1));
        }
        if is_recursive && levels_wanted() && std::env::var_os("RERAIL_LEVELS_WHY").is_some() {
            let trees: Vec<Rule> = at.iter().map(|&k| tree_named(src, k)).collect();
            let not_join = trees.iter().filter(|r| !r.body.iter().all(|l| matches!(l, Literal::Atom(_) | Literal::Cmp { .. }))).count();
            let no_levels: Vec<(&String, (usize, usize, bool))> = scc.iter().filter_map(|r| kept.levels(r).filter(|(has, _)| !has).map(|(_, shape)| (r, shape))).take(4).collect();
            let unkept: Vec<&String> = scc.iter().filter(|r| kept.levels(r).is_none()).take(4).collect();
            eprintln!("RERAIL_LEVELS_WHY {:?} ({} relations): levels {lv}; rules not joins {not_join}; without levels {no_levels:?}; not kept {unkept:?}", scc.iter().next(), scc.len());
            let kind = |l: &Literal| match l {
                Literal::Atom(_) | Literal::Cmp { .. } => None,
                Literal::Agg { .. } => Some("aggregate"),
                other => Some(match format!("{other:?}").split(['(', ' ', '{']).next() { Some("Assign") => "assignment", Some("Nearest") => "nearest", Some("Ast") => "ast", _ => "other" }),
            };
            let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
            for r in &trees {
                for l in &r.body {
                    if let Some(k) = kind(l) {
                        let shown = if k == "other" { format!("other: {}", format!("{l:?}").split(['(', ' ', '{']).next().unwrap_or("")) } else { k.to_owned() };
                        *kinds.entry(shown).or_default() += 1;
                    }
                }
            }
            eprintln!("RERAIL_LEVELS_WHY   literals not joins, by kind: {kinds:?}");
            for r in trees.iter().filter(|r| !r.body.iter().all(|l| matches!(l, Literal::Atom(_) | Literal::Cmp { .. }))).take(6) {
                eprintln!("RERAIL_LEVELS_WHY   {r}");
            }
        }
        let gone = if lv { "gone" } else { "del" };
        let moves = |rel: &str| scc.contains(rel) || changed.contains(rel);
        let old_of = |rel: &str| if moves(rel) { named(rel, "old") } else { rel.to_owned() };
        // A literal over old rows: an atom, or an aggregate's atom, that moves reads `r__old` (an atom of no gate,
        // as the tree's `atom` writes it; an aggregate's keeps its flags).
        let old_lit = |n: &Named<'_>, j: usize| -> L {
            if j >= n.lits.len() {
                return L::HeadName(n.calls[j - n.lits.len()]);
            }
            match n.lit(j) {
                Some(PLit::Atom(b)) => L::Src { i: j, rel: Some(old_of(n.rel(b))), flags: Some((b.neg(), false)) },
                Some(PLit::Agg { atom: b, .. }) if moves(n.rel(b)) => L::Src { i: j, rel: Some(old_of(n.rel(b))), flags: None },
                _ => L::Src { i: j, rel: None, flags: None },
            }
        };
        for (r, (n, &k)) in mine.iter().zip(&at).enumerate() {
            let h = n.head();
            let body = n.body();
            for j in 0..n.lits.len() {
                let lit = n.lits[j];
                if let PLit::Agg { var, op, over, atom: a } = lit.view()
                    && moves(n.rel(a))
                {
                    let mut g: BTreeSet<&str> = crate::code_rule_dsl::logic::plan::body::atom_vars(a).iter().map(|s| n.v.text(n.r.var(s))).collect();
                    g.remove(n.v.text(n.r.var(var)));
                    if let Some(o) = over {
                        g.remove(n.v.text(n.r.var(o)));
                    }
                    if let crate::code_rule_dsl::logic::program_image::AggRef::Join { by, .. } = op {
                        for s in by {
                            g.remove(n.v.text(n.r.var(s)));
                        }
                    }
                    let grp = format!("{h}__grp{r}_{j}");
                    let cols = f.columns(n.rel(a)).unwrap_or_else(|| panic!("no declaration of {}", n.rel(a)));
                    let args: Vec<TermRef<'_>> = a.args().collect();
                    let mut decl: Vec<(String, String)> = g
                        .iter()
                        .map(|v| {
                            let c = args.iter().position(|t| matches!(t, TermRef::Var(x) if n.v.text(n.r.var(*x)) == *v)).expect("a group variable is the atom's");
                            ((*v).to_owned(), cols[c].1.to_owned())
                        })
                        .collect();
                    let mut key: Vec<Tm> = g.iter().map(|v| Tm::Var((*v).to_owned())).collect();
                    if decl.is_empty() {
                        decl.push(("rr_unit".into(), "int".into()));
                        key.push(Tm::Int(1));
                        nullary_added.push(grp.clone());
                    }
                    b.declare(&grp, &decl.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect::<Vec<_>>());
                    made.insert(grp.clone(), decl.len());
                    for d in ["minus", "plus"] {
                        rules.push(Ir::Rule { src: Some(k), head: H::Terms(grp.clone(), key.clone()), body: vec![L::ArgsOf { rel: named(n.rel(a), d), i: j, neg: false, gated: false }] });
                    }
                    deltas.insert(n.rel(a).to_owned());
                    let mut over_old = vec![atom_tm(grp.clone(), key.clone(), false)];
                    over_old.extend((0..n.len()).map(|jj| old_lit(n, jj)));
                    rules.push(Ir::Rule { src: Some(k), head: H::Named(named(h, "del")), body: over_old });
                    let mut over_new = vec![atom_tm(grp, key, false)];
                    over_new.extend(body.iter().cloned());
                    insert_ir(&mut rules, n, k, h, over_new, "del");
                    continue;
                }
                let PLit::Atom(at) = lit.view() else { continue };
                let arel = n.rel(at);
                if !moves(arel) {
                    continue;
                }
                let (lose, gain) = match (at.neg(), scc.contains(arel)) {
                    (false, true) => (named(arel, "del"), named(arel, "ins")),
                    (false, false) => (named(arel, "minus"), named(arel, "plus")),
                    (true, _) => (named(arel, "plus"), named(arel, "minus")),
                };
                if !scc.contains(arel) {
                    deltas.insert(arel.to_owned());
                }
                let lose = if lv && !at.neg() && scc.contains(arel) { named(arel, "gone") } else { lose };
                let over: Vec<L> = (0..n.len()).map(|jj| if jj == j { L::ArgsOf { rel: lose.clone(), i: j, neg: false, gated: false } } else { old_lit(n, jj) }).collect();
                rules.push(Ir::Rule { src: Some(k), head: H::Named(if lv { named(h, "cand") } else { lost(h) }), body: over });
                let ins: Vec<L> = body
                    .iter()
                    .enumerate()
                    .flat_map(|(jj, l)| match jj == j {
                        true if at.neg() => vec![L::ArgsOf { rel: gain.clone(), i: j, neg: false, gated: false }, l.clone()],
                        true => vec![L::ArgsOf { rel: gain.clone(), i: j, neg: false, gated: false }],
                        false => vec![l.clone()],
                    })
                    .collect();
                insert_ir(&mut rules, n, k, h, ins, gone);
            }
            if lv {
                let mut held = vec![L::HeadArgs { rel: named(h, "chk"), neg: false }];
                held.extend(body.iter().enumerate().map(|(jj, l)| match n.lit(jj) {
                    Some(PLit::Atom(b)) if !b.neg() && scc.contains(n.rel(b)) => L::ArgsOf { rel: named(n.rel(b), "live"), i: jj, neg: false, gated: false },
                    _ => l.clone(),
                }));
                rules.push(Ir::Rule { src: Some(k), head: H::Named(named(h, "held")), body: held });
            }
            for j in 0..n.lits.len() {
                if let Some(b) = n.atom(j)
                    && matches!(n.lit(j), Some(PLit::Atom(_) | PLit::Agg { .. }))
                    && moves(n.rel(b))
                    && !scc.contains(n.rel(b))
                {
                    olds.insert(n.rel(b).to_owned());
                }
            }
            if exact {
                continue;
            }
            if projection {
                let mut held = vec![L::HeadArgs { rel: named(h, "lost"), neg: false }];
                held.extend(body.iter().cloned());
                rules.push(Ir::Rule { src: Some(k), head: H::Named(named(h, "held")), body: held });
                continue;
            }
            let mut red = vec![L::HeadArgs { rel: named(h, gone), neg: false }];
            red.extend(body.iter().cloned());
            rules.push(Ir::Rule { src: Some(k), head: H::Named(named(h, "redo")), body: red });
        }
        let exact = exact || projection;
        for rel in scc {
            let width = f.arity(rel).unwrap_or_else(|| panic!("no declaration of {rel}"));
            let v = row_tm(width);
            let a = |s: &str, neg: bool| atom_tm(named(rel, s), v.clone(), neg);
            olds.insert(rel.clone());
            if projection {
                rules.push(Ir::Rule { src: None, head: H::Terms(named(rel, "del"), v.clone()), body: vec![a("lost", false), a("held", true)] });
                fresh.push((rel.clone(), named(rel, "lost")));
                fresh.push((rel.clone(), named(rel, "held")));
            }
            if exact {
                exact_rels.insert(rel.clone());
            }
            rules.push(Ir::Rule { src: None, head: H::Terms(rel.clone(), v.clone()), body: vec![a("old", false), a(gone, true)] });
            if lv {
                let gated = |s: &str| L::Atom { rel: named(rel, s), args: v.clone(), neg: true, gated: true };
                rules.push(Ir::Rule { src: None, head: H::Terms(named(rel, "chk"), v.clone()), body: vec![a("cand", false)] });
                rules.push(Ir::Rule { src: None, head: H::Terms(named(rel, "gone"), v.clone()), body: vec![a("chk", false), gated("held")] });
                rules.push(Ir::Rule { src: None, head: H::Terms(named(rel, "live"), v.clone()), body: vec![a("old", false), gated("gone")] });
                for s in ["cand", "chk", "held", "gone", "live"] {
                    fresh.push((rel.clone(), named(rel, s)));
                }
                gone_of.insert(rel.clone());
            }
            rules.push(Ir::Rule { src: None, head: H::Terms(rel.clone(), v.clone()), body: vec![a("ins", false)] });
            if !exact {
                rules.push(Ir::Rule { src: None, head: H::Terms(rel.clone(), v.clone()), body: vec![a("redo", false)] });
            }
            if !lv {
                fresh.push((rel.clone(), named(rel, "del")));
            }
            fresh.push((rel.clone(), named(rel, "ins")));
            if !exact {
                fresh.push((rel.clone(), named(rel, "redo")));
            }
        }
        maint.extend(scc.iter().cloned());
        changed.extend(scc.iter().cloned());
        groups.push((scc.clone(), start, rules.len(), at.clone()));
    }
    if one.is_some() {
        for rel in sccs.iter().filter(|scc| scc.iter().all(|r| !cut.clean.contains(r))).flatten() {
            if maint.contains(rel) {
                deltas.insert(rel.clone());
            } else if old(rel) {
                deltas.insert(rel.clone());
                olds.insert(rel.clone());
            }
        }
    }
    let mut sources = Vec::new();
    // The old rows the form reads that the caller binds (`bind_olds`): a form is written without a run (decision 0298).
    let mut old_reads: Vec<String> = Vec::new();
    let view_deltas: Vec<String> = deltas.iter().filter_map(|r| view_input(r)).collect();
    let view_inputs: BTreeSet<String> = view_deltas.iter().cloned().collect();
    deltas.extend(view_deltas);
    let view_olds: Vec<String> = olds.iter().filter_map(|r| view_input(r)).collect();
    olds.extend(view_olds);
    let over_input = |rel: &str, suffix: &str| -> Ir {
        let k = views[rel];
        let n = src.named(k);
        let a = n.atom(0).expect("a view's atom");
        Ir::Rule { src: Some(k), head: H::Named(named(rel, suffix)), body: vec![L::ArgsOf { rel: named(n.rel(a), suffix), i: 0, neg: false, gated: false }] }
    };
    for rel in &deltas {
        let v = row_tm(src.width(rel, &made));
        let a = |r: String, neg: bool| atom_tm(r, v.clone(), neg);
        let head = |suffix: &str| H::Terms(named(rel, suffix), v.clone());
        if one.is_some_and(|o| !maint.contains(rel) && (o.has_delta)(rel)) {
            if !inputs.contains(&named(rel, "minus")) {
                inputs.push(named(rel, "minus"));
                inputs.push(named(rel, "plus"));
            }
        } else if views.contains_key(rel) {
            rules.push(over_input(rel, "minus"));
            rules.push(over_input(rel, "plus"));
        } else if maint.contains(rel) {
            let body = if exact_rels.contains(rel) {
                vec![a(named(rel, "del"), false), a(named(rel, "ins"), true)]
            } else {
                let gone = if gone_of.contains(rel) { "gone" } else { "del" };
                vec![a(named(rel, gone), false), a(named(rel, "redo"), true), a(named(rel, "ins"), true)]
            };
            rules.push(Ir::Rule { src: None, head: head("minus"), body });
            rules.push(Ir::Rule { src: None, head: head("plus"), body: vec![a(named(rel, "ins"), false), a(named(rel, "old"), true)] });
        } else if let Some((plus, minus)) = given(rel) {
            for (suffix, source) in [("plus", plus), ("minus", minus)] {
                sources.push((named(rel, suffix), source, None));
                inputs.push(named(rel, suffix));
            }
        } else {
            if view_inputs.contains(rel) {
                olds.insert(rel.clone());
            }
            rules.push(Ir::Rule { src: None, head: head("minus"), body: vec![a(named(rel, "old"), false), a(rel.clone(), true)] });
            rules.push(Ir::Rule { src: None, head: head("plus"), body: vec![a(rel.clone(), false), a(named(rel, "old"), true)] });
        }
        fresh.push((rel.clone(), named(rel, "minus")));
        fresh.push((rel.clone(), named(rel, "plus")));
    }
    // Each maintained stratum's form derived again (decision 0090 §4): its rules as given, and its deltas against
    // its old rows.
    let delta_of: HashMap<String, &str> = deltas.iter().filter(|r| maint.contains(*r)).flat_map(|r| [(named(r, "minus"), r.as_str()), (named(r, "plus"), r.as_str())]).collect();
    let mut delta_rules: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, r) in rules.iter().enumerate() {
        if let Some(&rel) = delta_of.get(&head_ir(src, r)) {
            delta_rules.entry(rel).or_default().push(i);
        }
    }
    let mut out_groups: Vec<(BTreeSet<String>, BTreeSet<String>, Vec<usize>, Vec<(String, usize)>, Vec<String>, Vec<String>)> = Vec::new();
    for (scc, start, end, given) in groups {
        let mine: Vec<usize> = (start..end).chain(scc.iter().filter_map(|s| delta_rules.get(s.as_str())).flatten().copied()).collect();
        let owned: BTreeSet<String> = mine.iter().map(|&i| head_ir(src, &rules[i])).chain(scc.iter().cloned()).collect();
        let reads: BTreeSet<String> = mine.iter().flat_map(|&i| reads_ir(src, &rules[i])).map(|(r, _, _)| r).filter(|r| !owned.contains(r) && (r.ends_with("__minus") || r.ends_with("__plus"))).collect();
        let again_deltas: Vec<(String, usize)> = scc.iter().filter(|r| deltas.contains(*r)).map(|r| (r.clone(), src.width(r, &made))).collect();
        let olds = scc.iter().map(|r| named(r, "old")).collect();
        out_groups.push((scc, owned, given, again_deltas, reads.into_iter().collect(), olds));
    }
    for rel in &olds {
        if views.contains_key(rel) {
            rules.push(over_input(rel, "old"));
            fresh.push((rel.clone(), named(rel, "old")));
            continue;
        }
        if !one.is_some_and(|o| (o.has_old)(rel)) {
            old_reads.push(rel.clone());
        }
        fresh.push((rel.clone(), named(rel, "old")));
        inputs.push(named(rel, "old"));
    }
    for (rel, m) in fresh {
        let cols = f.columns(&rel).unwrap_or_else(|| panic!("no declaration of {rel}"));
        b.declare(&m, &cols);
        made.insert(m.clone(), cols.len());
        if let Some(d) = domains.get(&rel).cloned() {
            domains.insert(m.clone(), d);
        }
        if f.nullary(&rel) {
            nullary_added.push(m);
        }
    }
    let together = together_ir(src, &mut b, &mut rules, &mut out_groups, &mut made);
    let mut heads = BTreeSet::new();
    for r in &rules {
        heads.insert(head_ir(src, r));
        write_ir(&mut b, src, r).map_err(Error::Plan)?;
    }
    b.add_nullary(&nullary_added);
    let groups = out_groups
        .into_iter()
        .map(|(scc, owned, given, deltas, reads, olds)| Group { scc: scc.into_iter().collect(), owned: owned.into_iter().collect(), given: given.iter().map(|&k| src.p.id(src.rules[k])).collect(), deltas, reads, olds })
        .collect();
    let views = views.into_keys().collect();
    Ok(Over { b, inputs, domains, groups, together, heads, sources, old_reads, maintained, derived_again, recursive, by_levels, again, views })
}

thread_local! {
    /// The recursive strata this thread's runs maintained by levels (decision 0090), and the sets of pieces they ran
    /// as one component (`together`). Counted by thread, not by process: a test reads its own runs' counts while
    /// other tests run beside it.
    static BY_LEVELS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static TOGETHER: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The recursive strata maintained by levels so far by this thread's runs (decision 0090), not by delete-and-rederive.
pub fn strata_by_levels() -> usize {
    BY_LEVELS.with(std::cell::Cell::get)
}

/// The sets of pieces run as one component so far by this thread's runs (`together`).
pub fn pieces_together() -> usize {
    TOGETHER.with(std::cell::Cell::get)
}

/// Decision 0101 §3: a maintained stratum's pieces (`h__del`, `h__ins`, its deltas, …) run as one component per
/// level, not one each, so they pay one component's fixed cost (its spec, open and statistics) between them.
///
/// A piece is ordered after the pieces it reads as stratified sub-steps of the one fixpoint: a negation of a piece
/// of the same component waits for a unit gate (`n__ready`, as magic's `done_` gates do) that passes once `n` and
/// what it reads there are quiet, and reads `n` whole. A survivor (`h :- h__old, !h__del`) starts the next level:
/// it is made in place from the kept rows less the deleted ones (`plan::survivors`), which are whole by then. A
/// recursive stratum, or one with a relation made natively or by a lattice, keeps its pieces apart.
///
/// Not under a device binding: the device takes no component with gates (`plan.rs`, "a component with gates or
/// requests"), so a set placed there would run on the host. Each piece there stays a stratum of seed joins, which
/// the device runs. (The device's own form of this is one submission with a dispatch per level, not gates.)
///
/// `RERAIL_PIECES_APART` runs each piece as its own component, as before (to compare).
fn together_ir(src: &Part<'_>, b: &mut Builder<'_>, rules: &mut Vec<Ir>, groups: &mut [(BTreeSet<String>, BTreeSet<String>, Vec<usize>, Vec<(String, usize)>, Vec<String>, Vec<String>)], made: &mut HashMap<String, usize>) -> Vec<BTreeSet<String>> {
    if std::env::var_os("RERAIL_PIECES_APART").is_some() {
        return Vec::new();
    }
    if crate::code_rule_dsl::logic::plan::placement::device() {
        crate::code_rule_dsl::logic::profile::decision("pieces together (decision 0101 §3)", || "apart: the run's binding is the device, which takes no gates".to_owned());
        return Vec::new();
    }
    let f = src.f;
    let heads: Vec<String> = rules.iter().map(|r| head_ir(src, r)).collect();
    let reads: Vec<Vec<(String, bool, bool)>> = rules.iter().map(|r| reads_ir(src, r)).collect();
    let derived: BTreeSet<&String> = heads.iter().collect();
    let mut rules_of: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, h) in heads.iter().enumerate() {
        rules_of.entry(h.as_str()).or_default().push(i);
    }
    // A read as `together`'s `reads_of` reads it: an atom with its negation, an aggregate's as negated.
    let reads_of = |i: usize| reads[i].iter().map(|(r, neg, agg)| (r.clone(), *neg || *agg)).collect::<Vec<_>>();
    let mut sets: Vec<(usize, BTreeSet<String>)> = Vec::new();
    'group: for (g, again) in groups.iter().enumerate() {
        let members: BTreeSet<&String> = again.1.iter().filter(|r| derived.contains(*r)).collect();
        if members.len() < 2 || members.iter().any(|r| f.native(r).is_some() || f.lattice(r) || f.has_facts(r)) {
            continue;
        }
        let inner: BTreeMap<&String, BTreeSet<String>> = members.iter().map(|r| {
            let reads = rules_of[r.as_str()].iter().flat_map(|&i| reads_of(i)).map(|(x, _)| x).filter(|x| members.contains(&x)).collect();
            (*r, reads)
        }).collect();
        let survivor = |h: &str| rules_of[h].iter().any(|&i| {
            let rd = &reads[i];
            let two_atoms = match &rules[i] {
                Ir::Rule { body, .. } => body.len() == 2 && rd.len() == 2 && rd.iter().all(|x| !x.2),
                Ir::Copy(k) => src.named(*k).lits.len() == 2 && rd.len() == 2 && rd.iter().all(|x| !x.2),
            };
            two_atoms && !rd[0].1 && rd[1].1 && rd[0].0 == named(h, "old") && (rd[1].0 == named(h, "del") || rd[1].0 == named(h, "gone")) && members.iter().any(|m| **m == rd[1].0)
        });
        let mut level: BTreeMap<&String, usize> = BTreeMap::new();
        let mut on: BTreeSet<&String> = BTreeSet::new();
        fn visit<'m>(r: &'m String, inner: &'m BTreeMap<&'m String, BTreeSet<String>>, survivor: &dyn Fn(&str) -> bool, level: &mut BTreeMap<&'m String, usize>, on: &mut BTreeSet<&'m String>) -> bool {
            if level.contains_key(r) {
                return true;
            }
            if !on.insert(r) {
                return false;
            }
            let mut l = 0;
            for x in &inner[r] {
                let Some((&x, _)) = inner.get_key_value(x) else { continue };
                if !visit(x, inner, survivor, level, on) {
                    return false;
                }
                l = l.max(level[x] + usize::from(survivor(r)));
            }
            on.remove(r);
            level.insert(r, l);
            true
        }
        for r in &members {
            if !visit(r, &inner, &survivor, &mut level, &mut on) {
                continue 'group;
            }
        }
        let mut by_level: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
        for (r, l) in level {
            by_level.entry(l).or_default().insert(r.clone());
        }
        sets.extend(by_level.into_values().filter(|s| s.len() > 1).map(|s| (g, s)));
    }
    // Only sets that are one component each as the strata are made (`front::apart`, over the rules as written).
    let apart: BTreeSet<usize> = {
        let mut names: Vec<&str> = heads.iter().map(String::as_str).collect();
        for (_, produced) in &src.answers {
            names.extend(produced.iter().map(String::as_str));
        }
        names.sort_unstable();
        names.dedup();
        let id: HashMap<&str, u32> = names.iter().enumerate().map(|(i, n)| (*n, i as u32)).collect();
        let mut succ: Vec<Vec<(u32, bool)>> = vec![Vec::new(); names.len()];
        for (request, produced) in &src.answers {
            if let Some(&r) = id.get(request.as_str()) {
                for p in produced {
                    succ[id[p.as_str()] as usize].push((r, false));
                }
            }
        }
        for (rel, nreads) in &src.natives {
            let Some(&h) = id.get(rel.as_str()) else { continue };
            succ[h as usize].extend(nreads.iter().filter_map(|read| id.get(read.as_str()).map(|&r| (r, false))));
        }
        for (i, h) in heads.iter().enumerate() {
            let h = id[h.as_str()];
            for (r, neg, agg) in &reads[i] {
                let Some(&d) = id.get(r.as_str()) else { continue };
                succ[h as usize].push((d, *neg || *agg));
            }
        }
        for s in &mut succ {
            s.sort_unstable();
            s.dedup();
        }
        let all: Vec<BTreeSet<String>> = sets.iter().map(|(_, s)| s.clone()).collect();
        front::apart_of_graph(names, succ, &all).into_iter().collect()
    };
    let sets: Vec<(usize, BTreeSet<String>)> = sets.into_iter().enumerate().filter(|(i, _)| !apart.contains(i)).map(|(_, s)| s).collect();
    let mut together = Vec::new();
    let mut gates = Vec::new();
    for (g, set) in sets {
        let inner: BTreeMap<&String, BTreeSet<String>> = set.iter().map(|r| {
            (r, rules_of[r.as_str()].iter().flat_map(|&i| reads_of(i)).map(|(x, _)| x).filter(|x| set.contains(x)).collect())
        }).collect();
        let negated: BTreeSet<String> = set.iter().flat_map(|r| rules_of[r.as_str()].iter().flat_map(|&i| reads_of(i))).filter(|(x, neg)| *neg && set.contains(x)).map(|(x, _)| x).collect();
        let reach = |n: &String| -> BTreeSet<String> {
            let (mut seen, mut todo) = (BTreeSet::new(), vec![n.clone()]);
            while let Some(v) = todo.pop() {
                if seen.insert(v.clone()) {
                    todo.extend(inner[&v].iter().cloned());
                }
            }
            seen
        };
        let depth = |n: &String| -> usize {
            fn d(v: &String, inner: &BTreeMap<&String, BTreeSet<String>>) -> usize {
                inner[v].iter().map(|x| 1 + d(x, inner)).max().unwrap_or(0)
            }
            d(n, &inner)
        };
        let ready = |n: &str| named(n, "ready");
        let mut members = set.clone();
        for n in &negated {
            let quiet = reach(n);
            let deps: BTreeSet<String> = quiet.iter().flat_map(|h| rules_of[h.as_str()].iter().flat_map(|&i| reads_of(i))).filter(|(x, neg)| *neg && negated.contains(x) && x != n).map(|(x, _)| ready(&x)).collect();
            let level = depth(n);
            gates.push((ready(n), n.clone(), level, quiet.into_iter().collect::<Vec<_>>(), deps.into_iter().collect::<Vec<_>>()));
            b.declare(&ready(n), &[("k", "int")]);
            made.insert(ready(n), 1);
            let width = src.width(n, made);
            rules.push(Ir::Rule { src: None, head: H::Terms(ready(n), vec![Tm::Int(0)]), body: vec![atom_tm(n.clone(), vec![Tm::Wild; width], false)] });
            members.insert(ready(n));
        }
        // A negation (or an aggregate) of a piece of the set reads it once its gate passed.
        for h in &set {
            for &i in &rules_of[h.as_str()] {
                let body = match &mut rules[i] {
                    Ir::Rule { body, .. } => std::mem::take(body),
                    Ir::Copy(k) => {
                        // A rule as given, its literals stated so they can be gated.
                        let k = *k;
                        rules[i] = Ir::Rule { src: Some(k), head: H::Given(heads[i].clone()), body: Vec::new() };
                        (0..src.named(k).lits.len()).map(|i| L::Src { i, rel: None, flags: None }).collect()
                    }
                };
                let src_rule = match &rules[i] {
                    Ir::Rule { src, .. } => *src,
                    Ir::Copy(_) => unreachable!(),
                };
                let n = src_rule.map(|k| src.named(k));
                let mut out = Vec::with_capacity(body.len() + 2);
                for l in body {
                    let read = match &l {
                        L::Src { i, rel, flags } => {
                            let n = n.as_ref().expect("a source rule");
                            n.lits[*i].atom().map(|a| (rel.clone().unwrap_or_else(|| n.rel(a).to_owned()), flags.map_or(a.neg(), |f| f.0), matches!(n.lits[*i].view(), PLit::Agg { .. })))
                        }
                        L::HeadName(_) | L::Test { .. } => None,
                        L::ArgsOf { rel, neg, .. } | L::HeadArgs { rel, neg } | L::Atom { rel, neg, .. } => Some((rel.clone(), *neg, false)),
                    };
                    // A negation, or an aggregate, of a piece of the set.
                    let Some((rel, neg, _)) = read.filter(|(rel, neg, agg)| (*neg || *agg) && set.contains(rel)) else {
                        out.push(l);
                        continue;
                    };
                    out.push(atom_tm(ready(&rel), vec![Tm::Int(0)], false));
                    out.push(match l {
                        L::Src { i, rel: r, flags } => L::Src { i, rel: r, flags: Some((flags.map_or(neg, |f| f.0), true)) },
                        L::ArgsOf { rel, i, neg, .. } => L::ArgsOf { rel, i, neg, gated: true },
                        L::Atom { rel, args, neg, .. } => L::Atom { rel, args, neg, gated: true },
                        l @ L::HeadArgs { .. } => unreachable!("a head's old rows are an input, never a piece: {l:?}"),
                        l => l,
                    });
                }
                if let Ir::Rule { body, .. } = &mut rules[i] {
                    *body = out;
                }
            }
        }
        groups[g].1.extend(members.iter().cloned());
        together.push(members);
        TOGETHER.with(|n| n.set(n.get() + 1));
    }
    b.add_gates(&gates);
    together
}

/// One stratum's maintained form, a derivation of the run's version (decision 0241), and what the run binds and
/// counts of it.
pub(in crate::code_rule_dsl) struct Form {
    pub(in crate::code_rule_dsl) placed: Placed,
    pub(in crate::code_rule_dsl) sources: Vec<(String, Arc<dyn Source>, Option<Arc<crate::code_rule_dsl::logic::native::component::InRel>>)>,
    /// The relations whose old rows the form reads and the caller binds (`bind_olds`).
    pub(in crate::code_rule_dsl) old_reads: Vec<String>,
    pub(in crate::code_rule_dsl) maintained: usize,
    pub(in crate::code_rule_dsl) derived_again: usize,
    pub(in crate::code_rule_dsl) recursive: usize,
    /// Of those recursive, strata maintained by levels (decision 0090).
    pub(in crate::code_rule_dsl) by_levels: usize,
    pub(in crate::code_rule_dsl) again: Vec<(String, &'static str)>,
}

/// The about rows of the run's version a form keeps of its stratum's relations (its lattices and natives: few
/// rows, each a relation's).
const STRATUM_ABOUT: [&str; 14] = [
    "lattice",
    "lattice_merge",
    "lattice_widen",
    "native_dfs",
    "native_roots",
    "native_match",
    "native_group",
    "native_python_candidates",
    "native_python",
    "native_components",
    "native_reach",
    "native_reach_through",
    "native_text_test",
    "native_text_pattern",
];

/// Stratum `scc` alone (its rules `rules`, places among `v0`'s): every relation its rules read and its natives read,
/// with its own, and its natives with what each reads.
fn mentioned(v0: &Placed, f: &Fields<'_>, rules: &[usize], scc: &BTreeSet<String>) -> (BTreeSet<String>, Vec<(String, Vec<String>)>) {
    let v: &Version = &v0.v;
    let mut mentioned: BTreeSet<String> = scc.clone();
    for &k in rules {
        let r = v0.rule(k);
        for l in r.body() {
            if let Some(a) = l.atom() {
                mentioned.insert(v.rel_name(a.rel()).to_owned());
            }
            if matches!(l.view(), PLit::Nearest { .. }) {
                mentioned.extend(super::retain::TREE.iter().map(|t| (*t).to_owned()));
            }
        }
    }
    let mut natives = Vec::new();
    for rel in scc {
        if let Some(n) = f.native(rel) {
            let reads: Vec<String> = n.reads.iter().map(|&r| v.rel_name(r).to_owned()).collect();
            mentioned.extend(reads.iter().cloned());
            natives.push((rel.clone(), reads));
        }
    }
    (mentioned, natives)
}

/// Stratum `scc`'s maintained form (decision 0117 step 3), its rules `rules` (places among the run's version `v0`'s
/// rules), whose reads `moved` changed: a derivation of `v0` holding the parts the stratum's program alone states
/// (its rules, the declarations, nullary, qualifier and tape rows of what they mention, its own lattice, natives and
/// facts, and the kind sets it reads), rewritten (`rewrite_over`). Its inputs are those `run_input` holds and the old rows.
/// It is written as an overlay of `atoms`, `v0`'s atoms and relations alone (`Builder::under`), made once per run:
/// a form's cost is its size.
pub(in crate::code_rule_dsl) fn form(atoms: &Arc<Version>, v0: &Placed, rules: &[usize], scc: &BTreeSet<String>, moved: &BTreeSet<String>, domains: &lower::Domains, kept: &dyn Olds, one: &One, run_input: &dyn Fn(&str) -> bool) -> Result<Form> {
    let f = Fields::of(v0).ok_or_else(|| Error::Plan("the run's version holds its rules alone".into()))?;
    let v: &Version = &v0.v;
    let (mentioned, natives) = mentioned(v0, &f, rules, scc);
    let mut b = Builder::on(atoms);
    b.copy_about(v, "program", |_| true);
    for rel in &mentioned {
        if let Some(cols) = f.columns(rel) {
            b.declare(rel, &cols);
        }
    }
    // What the run states of each relation it mentions: read from the run's fields by relation (sorted there), so a
    // form reads its own relations' rows, not every relation's.
    let (mut nullary, mut words, mut tapes) = (Vec::new(), Vec::new(), Vec::new());
    for rel in &mentioned {
        if f.nullary(rel) {
            nullary.push(rel.as_str());
        }
        if let Some(a) = v.find(rel) {
            let q = &f.about.qualifiers;
            words.extend(q[q.partition_point(|x| x.0 < a)..].iter().take_while(|x| x.0 == a).map(|x| (rel.as_str(), v.text(x.1))));
        }
        if let Some(r) = v.rel(rel) {
            let t = &f.about.tapes;
            tapes.extend(t[t.partition_point(|x| x.0 < r)..].iter().take_while(|x| x.0 == r).map(|x| (rel.as_str(), x.1)));
        }
    }
    b.add_nullary(&nullary);
    b.add_qualifiers(&words);
    if !tapes.is_empty() {
        b.set_tapes(&tapes);
    }
    let in_scc: BTreeSet<crate::code_rule_dsl::logic::program_image::AtomId> = scc.iter().filter_map(|n| v.find(n)).collect();
    let first = |r: &[crate::code_rule_dsl::logic::program_image::Cell]| matches!(r.get(1), Some(crate::code_rule_dsl::logic::program_image::Cell::Str(a)) if in_scc.contains(a));
    for name in STRATUM_ABOUT {
        b.copy_about(v, name, first);
    }
    // The kind sets the stratum reads: a `nearest` literal's, and a tree axis's or nearest relation's it mentions (as
    // `lower` names them), not the program's every set: a form holds its own.
    let mut sets: BTreeSet<&str> = BTreeSet::new();
    for &k in rules {
        sets.extend(v0.rule(k).body().filter_map(|l| match l.view() {
            PLit::Nearest { kindset, .. } => Some(v.text(kindset)),
            _ => None,
        }));
    }
    use crate::code_rule_dsl::logic::front::{AXIS, DESCENDANTS, NEAREST};
    sets.extend(mentioned.iter().filter_map(|r| r.strip_prefix(AXIS).or_else(|| r.strip_prefix(DESCENDANTS)).or_else(|| r.strip_prefix(NEAREST))));
    if !sets.is_empty() {
        let set_of = |r: &[crate::code_rule_dsl::logic::program_image::Cell]| matches!(r.get(1), Some(crate::code_rule_dsl::logic::program_image::Cell::Str(a)) if sets.contains(v.text(*a)));
        for name in ["kindset", "kindset_kind"] {
            b.copy_about(v, name, set_of);
        }
    }
    for rel in scc {
        if let Some(facts) = f.facts(rel) {
            b.set_fact_cells(rel, facts.rows().map(|r| r.collect()).collect());
        }
    }
    let inputs: Vec<String> = mentioned.iter().filter(|r| !scc.contains(*r)).cloned().collect();
    let part = Part {
        p: v0,
        f,
        rules: rules.to_vec(),
        inputs: inputs.clone(),
        outputs: Vec::new(),
        gates: false,
        natives,
        answers: Vec::new(),
        domains: mentioned.iter().filter_map(|r| domains.get(r).map(|d| (r.clone(), d.clone()))).collect(),
    };
    let cut = Cut { clean: inputs.iter().cloned().collect(), strata: (1, 1), changed: moved.iter().cloned().collect() };
    let sccs = vec![scc.clone()];
    let mut given = |_: &str| None;
    let mut over = rewrite_over(&part, b, &cut, Some(kept), &sccs, &mut given, Some(one))?;
    // Its form derived again (decision 0090 §4) stays: the run chooses between the two by its input deltas.
    if levels_wanted() {
        let words: Vec<(&str, &str)> = over.heads.iter().map(|h| (h.as_str(), "levels")).collect();
        over.b.add_qualifiers(&words);
    }
    // Its inputs are the run's and the old rows: a relation of an earlier stratum is the engine's, as derived.
    let inputs: Vec<&String> = over.inputs.iter().filter(|r| run_input(r) || r.ends_with("__old")).collect();
    over.b.set_inputs(&inputs);
    let fv = over.b.finish_overlay().map_err(Error::Plan)?;
    let maint = if over.groups.is_empty() && over.together.is_empty() { None } else { Some(Arc::new(Maint::written(&fv, v, &over.groups, &over.together)?)) };
    Ok(Form { placed: Placed::derived(fv, maint), sources: over.sources, old_reads: over.old_reads, maintained: over.maintained, derived_again: over.derived_again, recursive: over.recursive, by_levels: over.by_levels, again: over.again })
}

// -- General forms (decision 0298) --

/// A stratum's general form (decision 0298): its form with every read whose old rows are kept taken as changed, no
/// old rows bound (`Form::old_reads` lists every one it reads), and the read each rule is driven by.
pub(in crate::code_rule_dsl) struct General {
    pub(in crate::code_rule_dsl) form: Form,
    /// The reads the form maintains: the stratum's reads whose old rows are kept.
    pub(in crate::code_rule_dsl) reads: BTreeSet<String>,
    /// Per rule of the form's version, in order: the read whose delta it is driven by, or `None` for a rule every
    /// selection keeps (`selected`).
    pub(in crate::code_rule_dsl) drivers: Vec<Option<String>>,
    /// The form's inputs and outputs, and per rule the inputs it reads (places in `inputs`): what a selection states
    /// of its part, read without the form's fields.
    pub(in crate::code_rule_dsl) inputs: Vec<String>,
    pub(in crate::code_rule_dsl) outputs: Vec<String>,
    pub(in crate::code_rule_dsl) rule_inputs: Vec<Vec<u32>>,
}

/// Stratum `scc`'s general form (decision 0298), its rules `rules` (places among `v0`'s rules): `form` with the
/// reads whose old rows `olds` keeps as the reads that moved, each with a delta, and no old rows bound.
pub(in crate::code_rule_dsl) fn general(atoms: &Arc<Version>, v0: &Placed, rules: &[usize], scc: &BTreeSet<String>, domains: &lower::Domains, olds: &dyn Olds, run_input: &dyn Fn(&str) -> bool) -> Result<General> {
    let v: &Version = &v0.v;
    let reads: BTreeSet<String> = rules
        .iter()
        .flat_map(|&k| v0.rule(k).body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel()).to_owned()).collect::<Vec<_>>())
        .filter(|r| !scc.contains(r) && olds.has_old(r))
        .collect();
    let (all, none) = (|_: &str| true, |_: &str| false);
    let one = One { has_delta: &all, has_old: &none };
    let form = form(atoms, v0, rules, scc, &reads, domains, olds, &one, run_input)?;
    let drivers = drivers(&form.placed, scc, &reads);
    let p = &form.placed;
    let f = Fields::of(p).ok_or_else(|| Error::Plan("a general form holds its rules alone".into()))?;
    let inputs: Vec<String> = f.inputs().map(str::to_owned).collect();
    let outputs: Vec<String> = f.outputs().map(str::to_owned).collect();
    let at: HashMap<&str, u32> = inputs.iter().enumerate().map(|(i, r)| (r.as_str(), i as u32)).collect();
    let rule_inputs = (0..p.len())
        .map(|k| {
            let mut read: Vec<u32> = p.rule(k).body().filter_map(|l| l.atom()).filter_map(|a| at.get(p.v.rel_name(a.rel())).copied()).collect();
            read.sort_unstable();
            read.dedup();
            read
        })
        .collect();
    Ok(General { form, reads, drivers, inputs, outputs, rule_inputs })
}

/// The read each of `p`'s rules is driven by: one that reads a read's delta (`r__minus`, `r__plus`) as a positive
/// atom, or an aggregate's group of one (`h__grp…`, every rule of which is driven by the read). Every other rule
/// (survivors, `redo`, a stratum's own deltas, its pieces' gates) is `None`: kept by every selection.
fn drivers(p: &Placed, scc: &BTreeSet<String>, reads: &BTreeSet<String>) -> Vec<Option<String>> {
    let v: &Version = &p.v;
    let n = v.size();
    let delta_of = |name: &str| -> Option<String> { name.strip_suffix("__minus").or_else(|| name.strip_suffix("__plus")).filter(|r| reads.contains(*r) && !scc.contains(*r)).map(str::to_owned) };
    let positive = |i: usize| -> Vec<String> { p.rule(i).body().filter_map(|l| match l.view() { PLit::Atom(a) if !a.neg() => Some(v.rel_name(a.rel()).to_owned()), _ => None }).collect() };
    let heads: Vec<String> = (0..n).map(|i| v.rel_name(p.rule(i).head().rel()).to_owned()).collect();
    let mut out: Vec<Option<String>> = (0..n).map(|i| positive(i).iter().find_map(|r| delta_of(r))).collect();
    // A group of an aggregate's keys is fed by one read's deltas alone; a rule reading it is driven by that read.
    let mut groups: BTreeMap<&str, Option<Option<String>>> = BTreeMap::new();
    for (i, h) in heads.iter().enumerate().filter(|(_, h)| h.contains("__grp")) {
        let e = groups.entry(h.as_str()).or_insert_with(|| Some(out[i].clone()));
        if e.as_ref().is_some_and(|d| *d != out[i]) {
            *e = None;
        }
    }
    for i in 0..n {
        if out[i].is_none() {
            out[i] = positive(i).iter().find_map(|r| groups.get(r.as_str()).cloned().flatten().flatten());
        }
    }
    out
}

/// The name of stratum `scc` in its general form's key: its relations, joined by `,`.
pub(in crate::code_rule_dsl) fn stratum_key(scc: &BTreeSet<String>) -> String {
    scc.iter().map(String::as_str).collect::<Vec<_>>().join(",")
}

/// What a part's general forms take as kept (decision 0298): the old rows the kept part holds, read or not yet
/// (`Retained::holds_old`); with levels or not (`.1`). Whether kept rows have levels is known when they are read,
/// so a recursive stratum has a general form for each, and a run takes the one its kept rows match.
pub(in crate::code_rule_dsl) struct Holds<'k>(pub(in crate::code_rule_dsl) &'k Retained, pub(in crate::code_rule_dsl) bool);

impl Olds for Holds<'_> {
    fn has_old(&self, rel: &str) -> bool {
        self.0.holds_old(rel)
    }
    fn levels(&self, rel: &str) -> Option<(bool, (usize, usize, bool))> {
        // Only whether there are levels chooses a form; their shape is for diagnostics.
        self.0.holds_old(rel).then_some((self.1, (0, 0, false)))
    }
}

/// The settings a form's writer reads beside its input and kept rows (each the variable's value, or none): part of a
/// stored general form's key.
fn writer_settings() -> Vec<u8> {
    let mut out = Vec::new();
    for name in ["RERAIL_SESSION_VIEWS", "RERAIL_RECURSIVE", "RERAIL_COUNT_RECURSIVE", "RERAIL_CROSS_MAINTAIN_ONLY", "RERAIL_PIECES_APART"] {
        out.extend(name.as_bytes());
        out.push(b'=');
        if let Some(v) = std::env::var_os(name) {
            out.extend(v.as_encoded_bytes());
        }
        out.push(0);
    }
    out.push(super::supports::counting() as u8);
    out
}

/// The identity a part's general forms are keyed by (decision 0298 §4): its version's (`manager::identity`), and its
/// atoms' and relations' names in order, which a stored form's layers are opened over (`StoredForms`) and are not
/// fixed by the rules alone.
fn forms_identity(v: &Version) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(b"rerail.general-forms/v1\0");
    h.update(crate::code_rule_dsl::logic::program_image::manager::identity(v));
    for a in 0..v.atom_end() {
        let t = v.text(crate::code_rule_dsl::logic::program_image::AtomId(a));
        h.update((t.len() as u64).to_le_bytes());
        h.update(t.as_bytes());
    }
    for r in 0..v.rel_end() {
        h.update(v.rel_atom(RelId(r)).0.to_le_bytes());
    }
    h.finalize().into()
}

/// What a stratum's general form takes of the kept part (decision 0298): for each relation the stratum mentions,
/// whether its old rows are kept, and whether they have levels (`levels`, the same for all). Two kept parts alike in
/// these have the same general form.
fn kept_of(v0: &Placed, f: &Fields<'_>, rules: &[usize], scc: &BTreeSet<String>, kept: &Retained, levels: bool) -> Vec<u8> {
    let mut out = vec![u8::from(levels)];
    for r in mentioned(v0, f, rules, scc).0 {
        out.extend(r.as_bytes());
        out.push(0);
        out.push(u8::from(kept.holds_old(&r)));
    }
    out
}

/// Whether stratum `scc` (its rules `rules`, places among `v0`'s) is recursive: more than one relation, or a rule
/// reading its own. Only a recursive stratum's form can be by levels (decision 0090), so only its general form is
/// keyed by them.
pub(in crate::code_rule_dsl) fn recursive(v0: &Placed, rules: &[usize], scc: &BTreeSet<String>) -> bool {
    scc.len() > 1 || rules.iter().any(|&k| v0.rule(k).body().filter_map(|l| l.atom()).any(|a| scc.contains(v0.v.rel_name(a.rel()))))
}

/// Whether stratum `scc` has a general form (decision 0298): not one made natively, but a text test.
fn has_general(f: &Fields<'_>, rules: &[usize], scc: &BTreeSet<String>) -> bool {
    !rules.is_empty() && !scc.iter().any(|r| f.native(r).is_some_and(|n| !matches!(n.spec, NativeSpec::TextTest { .. })))
}

/// The pass that writes a stratum's general form, as a keyed step names it (0281 §6); its version is the writer's.
const GENERAL_PASS: &str = "a stratum's general form (decision 0298)";
const GENERAL_VERSION: u32 = 1;

/// A general form's record (decision 0298 §4): what a selection reads, stored before the form's layers. Varints: each
/// name an atom of the form's image, each relation its id there, each rule a place in it (the form's rules first, then
/// its maintenance's).
fn put(out: &mut Vec<u8>, mut n: u64) {
    while n >= 0x80 {
        out.push(n as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

/// Reads a record `put` wrote.
struct Record<'b>(&'b [u8]);

impl Record<'_> {
    fn n(&mut self) -> Result<u64> {
        let mut n = 0u64;
        for shift in (0..64).step_by(7) {
            let (&b, rest) = self.0.split_first().ok_or_else(|| Error::Plan("a stored general form's record is cut short".into()))?;
            self.0 = rest;
            n |= u64::from(b & 0x7f) << shift;
            if b < 0x80 {
                return Ok(n);
            }
        }
        Err(Error::Plan("a stored general form's record holds a number too long".into()))
    }
    fn list<T>(&mut self, mut item: impl FnMut(&mut Self) -> Result<T>) -> Result<Vec<T>> {
        let len = self.n()?;
        (0..len).map(|_| item(self)).collect()
    }
}

impl General {
    /// The image and the record the general form is stored as: its maintenance's version (its rules after the form's),
    /// or the form's, and the rest (`put`). A form derived again in part is not selected from (its strata's reasons are
    /// the writer's), so its `again` is not kept.
    fn stored(&self) -> Result<(Arc<Version>, Vec<u8>)> {
        let p = &self.form.placed;
        let image = p.maint.as_ref().map_or_else(|| Arc::clone(&p.v), |m| Arc::clone(&m.v));
        let place: HashMap<RuleId, u32> = image.rules().enumerate().map(|(k, r)| (r.id(), k as u32)).collect();
        if (0..p.len()).any(|k| place.get(&p.id(k)) != Some(&(k as u32))) {
            return Err(Error::Plan("a general form's rules are not its image's first".into()));
        }
        let mut out = Vec::new();
        let atom = |n: &str| image.find(n).map(|a| u64::from(a.0)).ok_or_else(|| Error::Plan(format!("a general form's name {n} is not in its image")));
        let names = |out: &mut Vec<u8>, ns: &mut dyn ExactSizeIterator<Item = &String>| -> Result<()> {
            put(out, ns.len() as u64);
            for n in ns {
                put(out, atom(n)?);
            }
            Ok(())
        };
        let rels = |out: &mut Vec<u8>, rs: &[RelId]| {
            put(out, rs.len() as u64);
            rs.iter().for_each(|r| put(out, u64::from(r.0)));
        };
        let rules = |out: &mut Vec<u8>, ids: &[RuleId]| -> Result<()> {
            put(out, ids.len() as u64);
            ids.iter().try_for_each(|id| Ok(put(out, u64::from(*place.get(id).ok_or_else(|| Error::Plan("a maintained rule is not in its image".into()))?))))
        };
        let f = &self.form;
        put(&mut out, p.len() as u64);
        names(&mut out, &mut self.reads.iter())?;
        put(&mut out, self.drivers.len() as u64);
        for d in &self.drivers {
            put(&mut out, d.as_deref().map_or(Ok(0), |n| atom(n).map(|a| a + 1))?);
        }
        names(&mut out, &mut self.inputs.iter())?;
        names(&mut out, &mut self.outputs.iter())?;
        put(&mut out, self.rule_inputs.len() as u64);
        for r in &self.rule_inputs {
            put(&mut out, r.len() as u64);
            r.iter().for_each(|&i| put(&mut out, u64::from(i)));
        }
        names(&mut out, &mut f.old_reads.iter())?;
        [f.maintained, f.derived_again, f.recursive, f.by_levels].iter().for_each(|&c| put(&mut out, c as u64));
        match &p.maint {
            None => put(&mut out, 0),
            Some(m) => {
                put(&mut out, 1);
                put(&mut out, m.again.len() as u64);
                for g in &m.again {
                    rels(&mut out, &g.scc);
                    rels(&mut out, &g.owned);
                    rules(&mut out, &g.rules)?;
                    rules(&mut out, &g.deltas)?;
                    rels(&mut out, &g.reads);
                    rels(&mut out, &g.olds);
                }
                put(&mut out, m.together.len() as u64);
                m.together.iter().for_each(|t| rels(&mut out, t));
            }
        }
        Ok((image, out))
    }

    /// The general form stored as `image` and `record` (`stored`), opened.
    fn opened(image: Arc<Version>, record: &[u8]) -> Result<General> {
        let ids: Vec<RuleId> = image.rules().map(|r| r.id()).collect();
        let bad = |what: &str| Error::Plan(format!("a stored general form's {what} is not in its image"));
        let mut r = Record(record);
        let n = |r: &mut Record<'_>| r.n().and_then(|n| u32::try_from(n).map_err(|_| bad("number")));
        let name = |r: &mut Record<'_>| {
            let a = n(r)?;
            (a < image.atom_end()).then(|| image.text(crate::code_rule_dsl::logic::program_image::AtomId(a)).to_owned()).ok_or_else(|| bad("name"))
        };
        let rel = |r: &mut Record<'_>| {
            let k = n(r)?;
            (k < image.rel_end()).then_some(RelId(k)).ok_or_else(|| bad("relation"))
        };
        let rule = |r: &mut Record<'_>| ids.get(n(r)? as usize).copied().ok_or_else(|| bad("rule"));
        let rules = n(&mut r)? as usize;
        if rules > ids.len() {
            return Err(bad("rules"));
        }
        let reads = r.list(name)?;
        let drivers = r.list(|r| match n(r)? {
            0 => Ok(None),
            a => (a - 1 < image.atom_end()).then(|| Some(image.text(crate::code_rule_dsl::logic::program_image::AtomId(a - 1)).to_owned())).ok_or_else(|| bad("name")),
        })?;
        let inputs = r.list(name)?;
        let outputs = r.list(name)?;
        let rule_inputs = r.list(|r| r.list(n))?;
        let old_reads = r.list(name)?;
        let [maintained, derived_again, recursive, by_levels] = [(); 4].map(|()| r.n().map(|c| c as usize));
        let maint = match r.n()? {
            0 => None,
            _ => {
                let again = r.list(|r| Ok(AgainRef { scc: r.list(rel)?, owned: r.list(rel)?, rules: r.list(rule)?, deltas: r.list(rule)?, reads: r.list(rel)?, olds: r.list(rel)? }))?;
                let together = r.list(|r| r.list(rel))?;
                Some(Arc::new(Maint { v: Arc::clone(&image), again, together }))
            }
        };
        if !r.0.is_empty() {
            return Err(Error::Plan("a stored general form's record is longer than its fields".into()));
        }
        let mut placed = Placed::new(Arc::clone(&image), ids[..rules].to_vec(), rules == ids.len());
        placed.maint = maint;
        let form = Form { placed, sources: Vec::new(), old_reads, maintained: maintained?, derived_again: derived_again?, recursive: recursive?, by_levels: by_levels?, again: Vec::new() };
        Ok(General { form, reads: reads.into_iter().collect(), drivers, inputs, outputs, rule_inputs })
    }
}

/// The keyed pipeline of stratum `scc`'s general form (0281 §6): over the part's version (`identity`), keyed by the
/// stratum and what it takes of the kept part (`kept_of`).
#[allow(clippy::too_many_arguments)]
fn general_pipeline<'s>(v0: &Placed, f: &Fields<'_>, identity: [u8; 32], steps: &'s StoredForms<'_>, rules: &[usize], scc: &BTreeSet<String>, kept: &Retained, levels: bool) -> crate::code_rule_dsl::logic::program_image::manager::Pipeline<'s> {
    let mut p = crate::code_rule_dsl::logic::program_image::manager::Pipeline::keyed_by("general form", Arc::clone(&v0.v), identity, steps);
    p.key_part("stratum", stratum_key(scc).as_bytes()).key_part("kept", &kept_of(v0, f, rules, scc, kept, levels));
    p
}

/// Stratum `scc`'s general form (decision 0298), its rules `rules` (places among `v0`'s), for the old rows `kept`
/// holds, with levels or not (`levels`): read from `steps` where a run stored it for the same part (`identity`, the
/// part's version), the same kept old rows of the relations the stratum mentions and the same settings, else written
/// and stored (0281 §6's keyed step). `None` for a stratum made natively (but a text test), which has none.
#[allow(clippy::too_many_arguments)]
pub(in crate::code_rule_dsl) fn general_of(atoms: &Arc<Version>, v0: &Placed, identity: [u8; 32], steps: &StoredForms<'_>, rules: &[usize], scc: &BTreeSet<String>, domains: &lower::Domains, kept: &Retained, levels: bool) -> Result<Option<General>> {
    let f = Fields::of(v0).ok_or_else(|| Error::Plan("a maintained part holds its rules alone".into()))?;
    if !has_general(&f, rules, scc) {
        return Ok(None);
    }
    let olds = Holds(kept, levels);
    let mut p = general_pipeline(v0, &f, identity, steps, rules, scc, kept, levels);
    let mut written = None;
    p.made(GENERAL_PASS, GENERAL_VERSION, &writer_settings(), |_| {
        let g = general(atoms, v0, rules, scc, domains, &olds, &|_| true).map_err(|e| e.to_string())?;
        let (image, record) = g.stored().map_err(|e| e.to_string())?;
        *steps.writing.borrow_mut() = Some(record);
        written = Some(g);
        Ok((image, Vec::new()))
    });
    // A stored form that does not open over the run's atoms is a cache's miss: the run writes its form.
    let image = match p.finish() {
        Ok((image, _)) => image,
        Err(e) if written.is_none() => {
            crate::code_rule_dsl::logic::profile::decision("a stored general form not read (decision 0298)", || format!("{}: {e}", stratum_key(scc)));
            return Ok(None);
        }
        Err(e) => return Err(Error::Plan(e)),
    };
    match written {
        Some(g) => Ok(Some(g)),
        None => {
            let record = steps.found.borrow_mut().take().ok_or_else(|| Error::Plan("a stored general form read without its record".into()))?;
            General::opened(image, &record.1).map(Some)
        }
    }
}

/// Where a run's general forms are kept (decision 0298 §4): a source of the store per form, under its step's key (0281
/// §6), holding its record (`General::stored`) and then its layers over the run's atoms, each varint-prefixed. The
/// step's pass and version are in its key, and opening a layer checks it, so nothing else is stored. Read inside the
/// waves by leave (`profile::read_in_waves`, the user's call, 2026-11-16: a reached stratum's key is known only once
/// the run reaches it), and written, with the pins of what it read, behind the run.
pub(in crate::code_rule_dsl) struct StoredForms<'s> {
    store: &'s crate::content_store::ContentStore,
    /// The run's version's atoms and relations alone (`Builder::under`), which every form is an overlay of: a form is
    /// stored as its own layers over them, so it holds its own names alone, not the part's, and is opened over them.
    atoms: std::cell::OnceCell<Arc<Version>>,
    /// The part's forms' identity (`forms_identity`), read at the first stratum reached.
    identity: std::cell::OnceCell<[u8; 32]>,
    /// The sources the run read, pinned for it (O95) in one write behind it, when it is dropped.
    held: std::cell::RefCell<Vec<crate::content_store::SourceId>>,
    /// The form `find` read, opened, and its record, until its pipeline opens it and `general_of` takes the record.
    found: std::cell::RefCell<Option<(Arc<Version>, Vec<u8>)>>,
    /// The record of the form being written, from its writer to `keep`.
    writing: std::cell::RefCell<Option<Vec<u8>>>,
}

/// The alias a general form's source is kept under: its step's key, in the image format its layers are written in.
fn form_alias(key: &StepKey) -> crate::content_store::HashKey {
    let format = crate::code_rule_dsl::logic::program_image::FORMAT;
    crate::content_store::HashKey::sha256(&[format!("rerail general form {format}\0").as_bytes(), key.as_slice()].concat())
}

impl<'s> StoredForms<'s> {
    pub(in crate::code_rule_dsl) fn new(store: &'s crate::content_store::ContentStore) -> StoredForms<'s> {
        StoredForms { store, atoms: Default::default(), identity: Default::default(), held: Default::default(), found: Default::default(), writing: Default::default() }
    }
    /// The part `v0` whose forms the run reads, and the atoms they are overlays of (`Builder::under(v0)`, made once by
    /// the run: the first set is every form's): the forms' identity, their keys' start.
    pub(in crate::code_rule_dsl) fn over(&self, v0: &Placed, atoms: &Arc<Version>) -> [u8; 32] {
        let _ = self.atoms.set(Arc::clone(atoms));
        *self.identity.get_or_init(|| forms_identity(&v0.v))
    }
    /// The source of form `v` and its `record`: the record, then `v`'s layers above the atoms, lowest first, each
    /// varint-prefixed.
    fn source(&self, record: &[u8], v: &Version) -> std::result::Result<Vec<u8>, String> {
        let atoms = self.atoms.get().ok_or("a general form stored before its atoms")?;
        let mut layers = Vec::new();
        let mut at = v;
        while let Some((bytes, parent)) = at.layer() {
            layers.push(bytes);
            at = parent;
        }
        if !std::ptr::eq(at, &**atoms) {
            return Err("a general form is not layers over the run's atoms".into());
        }
        let mut out = Vec::with_capacity(record.len() + layers.iter().map(|l| l.len() + 4).sum::<usize>() + 4);
        put(&mut out, record.len() as u64);
        out.extend_from_slice(record);
        for bytes in layers.iter().rev() {
            put(&mut out, bytes.len() as u64);
            out.extend_from_slice(bytes);
        }
        Ok(out)
    }
    /// The form stored as `bytes` (`source`), opened over the atoms, and its record.
    fn opened_source(&self, bytes: &[u8]) -> std::result::Result<(Arc<Version>, Vec<u8>), String> {
        let mut v = Arc::clone(self.atoms.get().ok_or("a general form opened before its atoms")?);
        let mut r = Record(bytes);
        let part = |r: &mut Record<'_>| -> std::result::Result<Vec<u8>, String> {
            let len = r.n().map_err(|e| e.to_string())? as usize;
            let (part, rest) = r.0.split_at_checked(len).ok_or("a stored general form is cut short")?;
            r.0 = rest;
            Ok(part.to_vec())
        };
        let record = part(&mut r)?;
        while !r.0.is_empty() {
            v = Version::over(&v, &part(&mut r)?)?;
        }
        Ok((v, record))
    }
}

impl Drop for StoredForms<'_> {
    fn drop(&mut self) {
        let held = std::mem::take(self.held.get_mut());
        if !held.is_empty() {
            self.store.write_back("the pins of the general forms read (decision 0298)", 0, Vec::new(), Box::new(move |s| s.pin_held_sources(&held)));
        }
    }
}

impl Steps for StoredForms<'_> {
    /// The form stored under `key`, read and opened at once (its source is one read); the record names the opened
    /// form by its key (`open`).
    fn find(&self, key: &StepKey) -> std::result::Result<Option<StepRecord>, String> {
        let store = self.store;
        let found = crate::code_rule_dsl::logic::profile::read_in_waves(|| -> std::result::Result<_, crate::content_store::StoreError> {
            let Some(id) = store.source_by_alias(form_alias(key))? else { return Ok(None) };
            Ok(Some((store.get_source(id)?, id)))
        })
        .map_err(|e| e.to_string())?;
        let Some((bytes, id)) = found else { return Ok(None) };
        let (v, record) = self.opened_source(&bytes)?;
        self.held.borrow_mut().push(id);
        let output = (v.size(), v.digest());
        *self.found.borrow_mut() = Some((v, record));
        Ok(Some(StepRecord { pass: GENERAL_PASS.into(), version: GENERAL_VERSION, image: *key, output, decided: Vec::new() }))
    }
    fn open(&self, _: &[u8; 32]) -> std::result::Result<Option<Arc<Version>>, String> {
        Ok(self.found.borrow().as_ref().map(|f| Arc::clone(&f.0)))
    }
    /// Its record and layers (`source`), queued for the store's writer (decision 0102): no wave waits for it.
    fn keep(&self, key: &StepKey, _: &StepRecord, output: &Version) -> std::result::Result<(), String> {
        let record = self.writing.borrow_mut().take().ok_or("a general form kept without its record")?;
        let bytes = self.source(&record, output)?;
        let alias = form_alias(key);
        self.store.write_back(
            "a general form (decision 0298)",
            bytes.len(),
            Vec::new(),
            Box::new(move |s| {
                let id = match s.source_by_alias(alias)? {
                    Some(id) => id,
                    None => s.put_source(&bytes, &[alias])?,
                };
                s.pin_held_sources(&[id])
            }),
        );
        Ok(())
    }
}

/// The places of `g`'s rules a stratum whose reads `moved` changed runs (decision 0298): those driven by a read in
/// `moved`, and those every selection keeps. `None` where the general form does not cover `moved`.
pub(in crate::code_rule_dsl) fn selected(g: &General, moved: &BTreeSet<String>) -> Option<Vec<usize>> {
    moved.iter().all(|r| g.reads.contains(r)).then(|| g.drivers.iter().enumerate().filter(|(_, d)| d.as_ref().is_none_or(|r| moved.contains(r))).map(|(i, _)| i).collect())
}

#[cfg(test)]
mod general_tests {
    use super::*;

    struct AllKept;
    impl Olds for AllKept {
        fn has_old(&self, _: &str) -> bool {
            true
        }
        fn levels(&self, _: &str) -> Option<(bool, (usize, usize, bool))> {
            None
        }
    }

    /// The rules of `p` at places `at`, printed, with each of `unmoved`'s old rows read as its rows (`x__old` as `x`),
    /// sorted.
    fn shown(p: &Placed, at: impl Iterator<Item = usize>, unmoved: &[&str]) -> Vec<String> {
        let mut out: Vec<String> = at
            .map(|i| {
                let mut s = p.v.show_rule(p.rule(i));
                for x in unmoved {
                    s = s.replace(&format!("{x}__old("), &format!("{x}("));
                }
                s
            })
            .collect();
        out.sort();
        out
    }

    /// Decision 0298: the rules a stratum runs, selected from its general form for the reads that moved, are the
    /// rules its form for those reads alone is written with, but that a read that did not move is read as its old
    /// rows, which are its rows.
    #[test]
    fn a_general_forms_selection_is_the_form_of_the_reads_that_moved() {
        let text = r#"
.decl a(x: number, y: number)
.decl b(y: number, z: number)
.decl c(x: number)
.decl h(x: number, z: number)
.input a
.input b
.input c
h(x, z) :- a(x, y), b(y, z), !c(x).
h(x, n) :- c(x), n = count : { b(x, _) }.
.output h
"#;
        let prog = crate::code_rule_dsl::logic::rr::parse(text).unwrap();
        let (v, _) = crate::code_rule_dsl::logic::program_image::Builder::of_program(&prog).unwrap().finish().unwrap();
        let v0 = Placed::whole(v);
        let atoms = Builder::under(&v0.v).finish().unwrap().0;
        let scc: BTreeSet<String> = ["h".to_owned()].into();
        let rules: Vec<usize> = (0..v0.v.size()).filter(|&i| v0.v.rel_name(v0.rule(i).head().rel()) == "h").collect();
        let input = |r: &str| ["a", "b", "c"].iter().any(|i| r == *i || r == format!("{i}__minus") || r == format!("{i}__plus"));
        let domains = lower::Domains::default();
        let g = general(&atoms, &v0, &rules, &scc, &domains, &AllKept, &input).unwrap();
        assert_eq!(g.reads, ["a", "b", "c"].iter().map(|s| (*s).to_owned()).collect());
        assert_eq!((g.form.maintained, g.form.derived_again), (1, 0), "the stratum is maintained");
        let heads: Vec<&str> = (0..g.form.placed.v.size()).map(|i| g.form.placed.v.rel_name(g.form.placed.rule(i).head().rel())).collect();
        assert!(heads.iter().any(|h| h.contains("__grp")), "the aggregate's groups: {heads:?}");
        assert!(g.drivers.iter().any(Option::is_none) && g.drivers.iter().any(|d| d.as_deref() == Some("b")));
        let (all, none) = (|_: &str| true, |_: &str| false);
        let one = One { has_delta: &all, has_old: &none };
        for moved in [vec!["a"], vec!["c"], vec!["a", "b"], vec!["a", "b", "c"]] {
            let m: BTreeSet<String> = moved.iter().map(|s| (*s).to_owned()).collect();
            let narrow = form(&atoms, &v0, &rules, &scc, &m, &domains, &AllKept, &one, &input).unwrap();
            let at = selected(&g, &m).expect("covered");
            let unmoved: Vec<&str> = ["a", "b", "c"].into_iter().filter(|r| !m.contains(*r)).collect();
            assert_eq!(shown(&g.form.placed, at.into_iter(), &unmoved), shown(&narrow.placed, 0..narrow.placed.v.size(), &[]), "moved {moved:?}");
        }
        // Stored (its image, here folded to a base, and its record) and opened in a new process: the same rules,
        // drivers, inputs and maintenance.
        let (image, record) = g.stored().unwrap();
        let folded = image.fold().unwrap();
        let opened = General::opened(Version::open(folded.base_bytes().unwrap()).unwrap(), &record).unwrap();
        let p = &g.form.placed;
        let q = &opened.form.placed;
        assert_eq!(shown(q, 0..q.len(), &[]), shown(p, 0..p.len(), &[]));
        assert_eq!((&opened.drivers, &opened.inputs, &opened.outputs, &opened.rule_inputs, &opened.reads), (&g.drivers, &g.inputs, &g.outputs, &g.rule_inputs, &g.reads));
        assert_eq!(q.maint.as_ref().map(|m| m.digest()), p.maint.as_ref().map(|m| m.digest()), "its maintenance");
        assert_eq!((opened.form.maintained, opened.form.recursive, &opened.form.old_reads), (g.form.maintained, g.form.recursive, &g.form.old_reads));
    }
}
