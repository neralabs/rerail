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
pub(in crate::code_rule_dsl) fn input_views(prog: &front::Program) -> BTreeMap<String, Rule> {
    if !views_on() {
        return BTreeMap::new();
    }
    let mut by: BTreeMap<&str, Vec<&Rule>> = BTreeMap::new();
    for r in &prog.rules {
        by.entry(r.head.rel.as_str()).or_default().push(r);
    }
    by.into_iter()
        .filter_map(|(rel, rules)| {
            let [rule] = rules.as_slice() else { return None };
            let [Literal::Atom(a)] = rule.body.as_slice() else { return None };
            if a.neg || a.gated || !prog.inputs.iter().any(|x| x == &a.rel) || prog.lattice.contains_key(rel) || prog.natives.contains_key(rel) || prog.facts.contains_key(rel) || prog.outputs.iter().any(|o| o == rel) {
                return None;
            }
            let head: BTreeSet<&str> = rule.head.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v.as_str()) } else { None }).collect();
            let terms_ok = rule.head.args.iter().all(|t| matches!(t, Term::Var(_) | Term::Int(_) | Term::Branded(_) | Term::Str(_)));
            let one_to_one = a.args.iter().all(|t| match t {
                Term::Var(v) => head.contains(v.as_str()),
                Term::Int(_) | Term::Branded(_) | Term::Str(_) => true,
                _ => false,
            });
            (terms_ok && one_to_one).then(|| (rel.to_owned(), (*rule).clone()))
        })
        .collect()
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
    let part = cut.program(cross);
    // The cut program's version where it has one (a part of the cross part's, decision 0244), else its image.
    // Read from it, its tree is decoded by no reader here (`lower::Tree`).
    let of_version = part.version.as_ref().filter(|_| !part.tree_maintained());
    let p = match of_version {
        Some(v) => Placed::new(v.clone(), v.rules().map(|r| r.id()).collect(), true),
        None => Placed::of_program(&part.program)?,
    };
    let f = Fields::of(&p).ok_or_else(|| Error::Plan("the cut program has no image".into()))?;
    let gates = match &cross.version {
        Some(v) if !cross.tree_maintained() => crate::code_rule_dsl::logic::program_image::About::of(v).gates > 0,
        _ => !cross.program.gates.is_empty(),
    };
    let src = match of_version {
        Some(_) => {
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
        }
        None => Part {
            p: &p,
            f,
            rules: (0..part.program.rules.len()).collect(),
            inputs: part.program.inputs.clone(),
            outputs: part.program.outputs.clone(),
            gates,
            natives: part.program.natives.iter().map(|(r, n)| (r.clone(), n.reads().into_iter().map(str::to_owned).collect())).collect(),
            answers: part.program.answers.iter().map(|(q, by)| (q.clone(), by.clone())).collect(),
            domains: part.domains.clone(),
        },
    };
    let mut over = rewrite_over(&src, Builder::derive(&p.v), cut, kept, sccs, given, None)?;
    over.b.set_inputs(&over.inputs);
    let (v, _) = over.b.finish().map_err(Error::Plan)?;
    // The run reads `v` and its `Maint`; a reader of the tree decodes it (`lower::Tree`), none on the run path.
    let maint = match over.groups.is_empty() && over.together.is_empty() {
        true => None,
        false => Some(Arc::new(Maint::written(&v, &p.v, &over.groups, &over.together)?)),
    };
    let program = lower::Tree::maintained(v.clone(), maint.clone());
    if let Ok(show) = std::env::var("O125_RULES") {
        for r in program.rules.iter().filter(|r| r.head.rel.starts_with(show.as_str())) {
            eprintln!("O125_RULES {r}");
        }
        for r in cross.program.rules.iter().filter(|r| r.head.rel == show) {
            eprintln!("O125_RULES given {r}");
        }
    }
    // The views of one input (`input_views`), over the whole program: where the part is maintained, those whose
    // input's old rows are kept.
    let views = match kept.filter(|_| !gates) {
        _ if !views_on() => BTreeSet::new(),
        None => input_views(&cross.program).into_keys().collect(),
        Some(k) => input_views(&cross.program).into_iter().filter(|(_, r)| r.body[0].atom().is_some_and(|a| k.has_old(&a.rel))).map(|(rel, _)| rel).collect(),
    };
    Ok(Maintained { program: lower::Lowered { program, version: Some(v), maint, domains: over.domains, ops: part.ops, dialect: part.dialect }, sources: over.sources, maintained: over.maintained, derived_again: over.derived_again, recursive: over.recursive, again: over.again, views })
}

// -- The rewrite over a version (decision 0241) --

use crate::code_rule_dsl::logic::plan::body::{Group, Maint, Placed};
use crate::code_rule_dsl::logic::plan::fields::Fields;
use crate::code_rule_dsl::logic::program_image::{AtomRef, Builder, CmpOp, Lit as PLit, LitRef, NativeSpec, Rewrite, RuleId, RuleRef, TermRef, Version, T};

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
    pub(in crate::code_rule_dsl) maintained: usize,
    pub(in crate::code_rule_dsl) derived_again: usize,
    pub(in crate::code_rule_dsl) recursive: usize,
    pub(in crate::code_rule_dsl) again: Vec<(String, &'static str)>,
    pub(in crate::code_rule_dsl) views: BTreeSet<String>,
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
        self.r.head().args().all(|t| matches!(t, TermRef::Var(_) | TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_) | TermRef::Call(..)))
    }
}

/// A term of `v`'s rule `r`, to write into a builder under `v`'s ids.
fn t_of<'v>(v: &'v Version, r: RuleRef<'v>, t: TermRef<'v>) -> T<'v> {
    match t {
        TermRef::Var(s) => T::VarAtom(r.var(s)),
        TermRef::Wild => T::Wild,
        TermRef::Top => T::Top,
        TermRef::Int(i) => T::Int(i),
        TermRef::Branded(b) => T::Branded(b),
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
    if !n.r.head().args().all(|t| matches!(t, TermRef::Var(_) | TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_) | TermRef::Call(..))) {
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
            let terms_ok = n.r.head().args().all(|t| matches!(t, TermRef::Var(_) | TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_)));
            let one_to_one = a.args().all(|t| match t {
                TermRef::Var(s) => head.contains(&s),
                TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_) => true,
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
pub(in crate::code_rule_dsl) fn rewrite_over<'b>(src: &Part<'_>, mut b: Builder<'b>, cut: &Cut, kept: Option<&Retained>, sccs: &[BTreeSet<String>], given: &mut dyn FnMut(&str) -> Option<(Arc<dyn Source>, Arc<dyn Source>)>, one: Option<&One>) -> Result<Over<'b>> {
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
        return Ok(Over { b, inputs: src.inputs.clone(), domains: src.domains.clone(), groups: Vec::new(), together: Vec::new(), heads, sources: Vec::new(), maintained: 0, derived_again: dirty.len(), recursive: 0, again, views });
    };
    let views: BTreeMap<String, usize> = input_views_over(src).into_iter().filter(|(_, k)| src.named(*k).atom(0).is_some_and(|a| kept.has_old(src.named(*k).rel(a)))).collect();
    let view_input = |rel: &str| -> Option<String> { views.get(rel).and_then(|&k| { let n = src.named(k); n.atom(0).map(|a| n.rel(a).to_owned()) }) };
    let old = |rel: &str| kept.has_old(rel) || views.contains_key(rel);
    let mut changed: BTreeSet<String> = cut.changed.iter().cloned().collect();
    let mut rules: Vec<Ir> = Vec::new();
    let mut olds: BTreeSet<String> = BTreeSet::new();
    let mut deltas: BTreeSet<String> = BTreeSet::new();
    let (mut maintained, mut derived_again, mut recursive) = (0, 0, 0);
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
        let lv = is_recursive && levels_wanted() && mine.iter().all(joins) && scc.iter().all(|r| kept.kept(r).is_some_and(|(h, _)| h.has_levels()));
        if lv {
            BY_LEVELS.with(|n| n.set(n.get() + 1));
        }
        if is_recursive && levels_wanted() && std::env::var_os("RERAIL_LEVELS_WHY").is_some() {
            let trees: Vec<Rule> = at.iter().map(|&k| tree_named(src, k)).collect();
            let not_join = trees.iter().filter(|r| !r.body.iter().all(|l| matches!(l, Literal::Atom(_) | Literal::Cmp { .. }))).count();
            let no_levels: Vec<(&String, (usize, usize, bool))> = scc.iter().filter_map(|r| kept.kept(r).filter(|(h, _)| !h.has_levels()).map(|(h, _)| (r, h.levels_shape()))).take(4).collect();
            let unkept: Vec<&String> = scc.iter().filter(|r| kept.kept(r).is_none()).take(4).collect();
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
            let (source, held) = kept.old_source(rel, &named(rel, "old")).expect("a maintained read's old rows");
            sources.push((named(rel, "old"), source, held));
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
    Ok(Over { b, inputs, domains, groups, together, heads, sources, maintained, derived_again, recursive, again, views })
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
    pub(in crate::code_rule_dsl) maintained: usize,
    pub(in crate::code_rule_dsl) derived_again: usize,
    pub(in crate::code_rule_dsl) recursive: usize,
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

/// Stratum `scc`'s maintained form (decision 0117 step 3), its rules `rules` (places among the run's version `v0`'s
/// rules), whose reads `moved` changed: a derivation of `v0` holding the parts the stratum's program alone states
/// (its rules, the declarations, nullary, qualifier and tape rows of what they mention, its own lattice, natives and
/// facts, and every kind set), rewritten (`rewrite_over`). Its inputs are those `run_input` holds and the old rows.
/// It is written as an overlay of `atoms`, `v0`'s atoms and relations alone (`Builder::under`), made once per run:
/// a form's cost is its size.
pub(in crate::code_rule_dsl) fn form(atoms: &Arc<Version>, v0: &Placed, rules: &[usize], scc: &BTreeSet<String>, moved: &BTreeSet<String>, domains: &lower::Domains, kept: &Retained, one: &One, run_input: &dyn Fn(&str) -> bool) -> Result<Form> {
    let f = Fields::of(v0).ok_or_else(|| Error::Plan("the run's version holds its rules alone".into()))?;
    let v: &Version = &v0.v;
    // The stratum alone: its rules, and every relation they read declared as an input of it.
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
    for name in ["kindset", "kindset_kind"] {
        b.copy_about(v, name, |_| true);
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
    Ok(Form { placed: Placed::derived(fv, maint), sources: over.sources, maintained: over.maintained, derived_again: over.derived_again, recursive: over.recursive, again: over.again })
}
