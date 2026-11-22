//! A count below a bound, found by the form of the rules (decision 0361 step 7): `n = count : H(…)` where `H`'s one
//! rule compares one of its counted variables with a variable of the asked rows (`ky OP kx`, `OP` one of `<`, `<=`,
//! `>`, `>=`), and the comparison is all that ties the two. As rows, `H` holds every asked row with every counted row
//! below it: python/ast's `body_stmt`, a statement's place among its body's, held every statement with every earlier
//! one (80 K statements, 1.6 M pairs at 300 files).
//!
//! Without the comparison, `H`'s body falls in two parts, by the variables its literals share other than the ones
//! the reader groups by:
//! - the counted part `Y`: the literals tied to the counted columns (the reader's `_`) and to `ky`;
//! - the asked part `X`: the rest, which binds every grouped column and `kx`, and shares no variable with `Y` but
//!   grouped ones (the group `G`: those `Y` reads too).
//!
//! It is then made of the two parts' rows, each a relation of its own, and counted by `Native::Rank`
//! (`logic::native::rank`), which sorts each group's keys once:
//!
//! ```text
//! rr_rankq(G…, X…, kx) :- X.                     the asked rows, each with its bound
//! rr_rankg(G…) :- rr_rankq(G…, _…, _).           their groups
//! rr_rankb(G…, C…, ky) :- rr_rankg(G…), Y.       the counted rows of those groups
//! rr_rank(G…, X…, n)                              natively; the reader's `n = count : H(…)` becomes
//!                                                 `n = sum m : rr_rank(…, m)`: 0 where no row is asked
//! ```
//!
//! Nothing here names a relation: any rule of the form is made so. `RERAIL_RANK_FORM=off` finds none (a measurement
//! aid); `.decl H(…) no_rank` keeps `H` as rows.
use rerail_blocks::vector::Vector;
use crate::code_rule_dsl::logic::front::AggOp;
use crate::code_rule_dsl::logic::native::rank::Op;
use crate::code_rule_dsl::logic::plan::body::{VarSet, lit_vars};
use crate::code_rule_dsl::logic::program_image::{AggRef, Builder, Lit, LitRef, RuleId, RuleRef, Slot, T, TermRef, Version, build::t_of};
use std::collections::{BTreeMap, BTreeSet};

/// The qualifier keeping a relation as rows.
pub const NO_RANK: &str = "no_rank";

/// The form found in a relation's one rule `h`, for a reader that groups by its head's columns the reader binds. The
/// variables are `h`'s slots; the parts are `h`'s literals by position.
struct Shape {
    /// The group: grouped variables the counted part reads, in head order.
    group: Vec<Slot>,
    /// The other grouped variables, in head order.
    asked: Vec<Slot>,
    /// The counted variables (the reader's `_`), in head order.
    counted: Vec<Slot>,
    kx: Slot,
    ky: Slot,
    op: Op,
    x: Vector<usize>,
    y: Vector<usize>,
}

/// A rank the form makes of a counted relation's rule `h` (`Shape`): its four relations' names (the asked rows, their
/// groups, the counted rows and the rank) and, for each of the rank's columns but the count, the place in `h`'s head
/// of the variable it holds.
struct Rank {
    h: RuleId,
    shape: Shape,
    names: [String; 4],
    at: Vector<usize>,
}

/// What the form reads of a program besides its rules: the version's (`corpus::preparation::rank`).
pub(crate) trait Of {
    /// `rel`'s columns' types, where it is declared.
    fn types(&self, rel: &str) -> Option<Vec<String>>;
    /// Whether `rel` is held as rows whatever its rule: made natively, a lattice, an input, or `no_rank`.
    fn kept(&self, rel: &str) -> bool;
}

/// The form planned over a version (`plan`): each reader's counts it rewrites (the reader, and each literal's place
/// with its rank), and the ranks.
pub(crate) struct Plan {
    readers: BTreeMap<RuleId, Vec<(usize, usize)>>,
    ranks: Vec<Rank>,
}

impl Plan {
    /// How many readers' counts it rewrites.
    pub(crate) fn found(&self) -> usize {
        self.readers.values().map(|v| v.len()).sum()
    }
}

/// Whether rule `r` has a literal the form may read (`plan` decides).
pub(crate) fn counts(r: RuleRef<'_>) -> bool {
    r.body().any(|l| matches!(l.view(), Lit::Agg { op: AggRef::Count, over: None, atom, .. } if !atom.neg()))
}

/// Whether the form is looked for (`RERAIL_RANK_FORM=off`: not, a measurement aid).
pub(crate) fn on() -> bool {
    !std::env::var("RERAIL_RANK_FORM").is_ok_and(|v| v == "off")
}

/// The form over `v`'s readers (the rules with a count, `counts`), in the program's order.
pub(crate) fn plan(v: &Version, of: &dyn Of) -> Plan {
    let mut plan = Plan { readers: BTreeMap::new(), ranks: Vec::new() };
    let mut placed: BTreeMap<(String, Vec<bool>), Option<usize>> = BTreeMap::new();
    for r in v.rules().filter(|r| counts(*r)) {
        for (j, l) in r.body().enumerate() {
            let Lit::Agg { op: AggRef::Count, over: None, atom, .. } = l.view() else { continue };
            let rel = v.rel_name(atom.rel());
            if atom.neg() || of.kept(rel) {
                continue;
            }
            let grouped: Vec<bool> = atom.args().map(|t| !matches!(t, TermRef::Wild)).collect();
            let k = (rel.to_owned(), grouped);
            if !placed.contains_key(&k) {
                let rank = one_rule(v, rel).and_then(|h| shape(h, &k.1).map(|s| (h, s))).map(|(h, s)| {
                    let names = ["rr_rankq", "rr_rankg", "rr_rankb", "rr_rank"].map(|stem| fresh(v, &plan.ranks, stem));
                    let head: Vec<Slot> = h.head().args().filter_map(TermRef::var).collect();
                    let at = s.group.iter().chain(&s.asked).map(|x| head.iter().position(|y| y == x).expect("a head variable")).collect();
                    plan.ranks.push(Rank { h: h.id(), shape: s, names, at });
                    plan.ranks.len() - 1
                });
                placed.insert(k.clone(), rank);
            }
            if let Some(rank) = placed[&k] {
                plan.readers.entry(r.id()).or_default().push((j, rank));
            }
        }
    }
    plan
}

/// `rel`'s rule in `v`, where it has exactly one.
fn one_rule<'v>(v: &'v Version, rel: &str) -> Option<RuleRef<'v>> {
    let mut heads = v.heads(v.rel(rel)?);
    let (one, none) = (heads.next()?, heads.next().is_none());
    none.then_some(one)
}

/// A name of `stem` that neither `v` declares nor a rank of `ranks` takes.
fn fresh(v: &Version, ranks: &[Rank], stem: &str) -> String {
    (0..).map(|k| format!("{stem}{k}")).find(|n| v.decl_of(n).is_none() && !ranks.iter().any(|r| r.names.contains(n))).expect("a fresh name")
}

/// The form in `h` (the counted relation's one rule) for a reader grouping by `grouped`, if it has it.
fn shape(h: RuleRef<'_>, grouped: &[bool]) -> Option<Shape> {
    let head: Vec<Slot> = h.head().args().map(TermRef::var).collect::<Option<_>>()?;
    if head.iter().collect::<BTreeSet<_>>().len() != head.len() || head.len() != grouped.len() {
        return None;
    }
    let gv = VarSet::of(head.iter().zip(grouped).filter(|(_, g)| **g).map(|(v, _)| *v));
    let cv: Vec<Slot> = head.iter().zip(grouped).filter(|(_, g)| !**g).map(|(v, _)| *v).collect();
    if cv.is_empty() {
        return None;
    }
    let body: Vec<LitRef<'_>> = h.body().collect();
    // Variables the literals' positive atoms bind.
    let bound = |ls: &[usize]| -> VarSet {
        let mut out = VarSet::new();
        for &k in ls {
            if let Lit::Atom(a) = body[k].view()
                && !a.neg()
            {
                out.extend(&lit_vars(body[k]));
            }
        }
        out
    };
    for (ci, l) in body.iter().enumerate() {
        let Lit::Cmp { l: TermRef::Var(a), op, r: TermRef::Var(b) } = l.view() else { continue };
        let Some(op) = Op::parse(op.as_str()) else { continue };
        for (ky, kx, op) in [(a, b, op), (b, a, op.flipped())] {
            if gv.contains(ky) {
                continue;
            }
            let rest: Vector<usize> = (0..body.len()).filter(|k| *k != ci).collect();
            let own: Vec<VarSet> = rest.iter().map(|&k| lit_vars(body[k]).minus(&gv)).collect();
            // The counted part: the literals reached from the counted variables and ky through their own variables.
            let mut reach = VarSet::of(cv.iter().copied().chain([ky]));
            let mut in_y = vec![false; rest.len()];
            let mut grew = true;
            while grew {
                grew = false;
                for (k, o) in own.iter().enumerate() {
                    if !in_y[k] && !o.is_disjoint(&reach) {
                        in_y[k] = true;
                        reach.extend(o);
                        grew = true;
                    }
                }
            }
            if reach.contains(kx) && !gv.contains(kx) {
                continue;
            }
            let y: Vector<usize> = rest.iter().zip(&in_y).filter(|(_, y)| **y).map(|(k, _)| *k).collect();
            let x: Vector<usize> = rest.iter().zip(&in_y).filter(|(_, y)| !**y).map(|(k, _)| *k).collect();
            if y.is_empty() {
                continue;
            }
            let mut y_vars = VarSet::new();
            y.iter().for_each(|&k| y_vars.extend(&lit_vars(body[k])));
            let group: Vec<Slot> = head.iter().copied().filter(|v| gv.contains(*v) && y_vars.contains(*v)).collect();
            let asked: Vec<Slot> = head.iter().copied().filter(|v| gv.contains(*v) && !y_vars.contains(*v)).collect();
            // The asked part binds every grouped variable and the bound; the counted part, with the group, binds
            // the counted variables and the key.
            let (bx, by) = (bound(&x), bound(&y));
            if !gv.iter().chain([kx]).all(|v| bx.contains(v)) || !cv.iter().copied().chain([ky]).all(|v| by.contains(v) || group.contains(&v)) {
                continue;
            }
            return Some(Shape { group, asked, counted: cv, kx, ky, op, x, y });
        }
    }
    None
}

/// Writes `plan` on `b`, a derivation of `v`: each rank's relations declared, `v`'s rules (each reader's counts read
/// as a sum over its rank, `n = sum m : rr_rank(…, m)`: 0 where no row is asked), then the ranks' rules, in order.
/// The ranks' natives, by relation: `(rank, asked rows, counted rows, group's width, op)`.
pub(crate) fn write(v: &Version, of: &dyn Of, plan: &Plan, b: &mut Builder<'_>) -> Result<Vec<(String, String, String, usize, &'static str)>, String> {
    for rank in &plan.ranks {
        let h = v.rule(rank.h).expect("the counted rule");
        let s = &rank.shape;
        // A variable's type: its column's in the head, else in an atom of the body; a key is a number otherwise.
        let type_of = |x: Slot| -> String {
            let atoms = std::iter::once(h.head()).chain(h.body().filter_map(|l| if let Lit::Atom(a) = l.view() { Some(a) } else { None }));
            for a in atoms {
                if let Some(cols) = of.types(v.rel_name(a.rel()))
                    && let Some(c) = a.args().position(|t| t.var() == Some(x))
                    && let Some(t) = cols.get(c)
                {
                    return t.clone();
                }
            }
            "int".to_owned()
        };
        let decl = |b: &mut Builder<'_>, rel: &str, vars: &[Slot], last: Option<(&str, String)>| {
            let mut cols: Vec<(String, String)> = vars.iter().enumerate().map(|(i, x)| (format!("c{i}"), type_of(*x))).collect();
            cols.extend(last.map(|(n, t)| (n.to_owned(), t)));
            let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            b.fields().declare(rel, &cols);
        };
        let [q, g, bn, r] = &rank.names;
        let asked: Vec<Slot> = s.group.iter().chain(&s.asked).copied().collect();
        let counted: Vec<Slot> = s.group.iter().chain(&s.counted).copied().collect();
        decl(b, q, &asked, Some(("k", type_of(s.kx))));
        decl(b, g, &s.group, None);
        decl(b, bn, &counted, Some(("k", type_of(s.ky))));
        decl(b, r, &asked, Some(("n", "int".to_owned())));
    }
    for r in v.rules() {
        let Some(counts) = plan.readers.get(&r.id()) else {
            b.keep(v, r);
            continue;
        };
        // The sum's variable: a name the reader does not use.
        let taken: BTreeSet<&str> = r.vars().map(|a| v.text(a)).collect();
        let n = (0..).map(|k| format!("rr_rankn{k}")).find(|x| !taken.contains(x.as_str())).expect("a fresh name");
        let mut rb = b.rule().head_from(v, r, r.head());
        for (j, l) in r.body().enumerate() {
            match counts.iter().find(|(k, _)| *k == j) {
                Some(&(_, rank)) => {
                    let Lit::Agg { var, atom, .. } = l.view() else { unreachable!("a reader's count") };
                    let rank = &plan.ranks[rank];
                    let args: Vec<TermRef<'_>> = atom.args().collect();
                    let mut terms: Vec<T<'_>> = rank.at.iter().map(|&c| t_of(r, args[c])).collect();
                    terms.push(T::Var(&n));
                    rb = rb.agg(T::VarAtom(r.var(var)), &AggOp::Sum, Some(T::Var(&n)), rank.names[3].as_str(), &terms);
                }
                None => rb = rb.lit(v, r, l),
            }
        }
        rb.finish()?;
    }
    let mut natives = Vec::new();
    for rank in &plan.ranks {
        let h = v.rule(rank.h).expect("the counted rule");
        let s = &rank.shape;
        let body: Vec<LitRef<'_>> = h.body().collect();
        let var = |x: &Slot| T::VarAtom(h.var(*x));
        let [q, g, bn, r] = &rank.names;
        let asked: Vec<T<'_>> = s.group.iter().chain(&s.asked).map(var).collect();
        let q_args: Vec<T<'_>> = asked.iter().cloned().chain([var(&s.kx)]).collect();
        let g_args: Vec<T<'_>> = s.group.iter().map(var).collect();
        // rr_rankq(G…, X…, kx) :- X.
        let mut rb = b.rule().head(q.as_str(), &q_args);
        for &k in &s.x {
            rb = rb.lit(v, h, body[k]);
        }
        rb.finish()?;
        // rr_rankg(G…) :- rr_rankq(G…, _…, _).
        b.rule().head(g.as_str(), &g_args).atom(q.as_str(), &q_args).finish()?;
        // rr_rankb(G…, C…, ky) :- rr_rankg(G…), Y.
        let b_args: Vec<T<'_>> = s.group.iter().chain(&s.counted).map(var).chain([var(&s.ky)]).collect();
        let mut rb = b.rule().head(bn.as_str(), &b_args).atom(g.as_str(), &g_args);
        for &k in &s.y {
            rb = rb.lit(v, h, body[k]);
        }
        rb.finish()?;
        // Its one rule orders it after both relations it reads, and joins them on the group as `H` did: the split
        // (`file_local`) finds it local where its groups are one document's. The rows are made natively (as a
        // reach's, decision 0119).
        let b_read: Vec<T<'_>> = g_args.iter().cloned().chain(std::iter::repeat_n(T::Wild, s.counted.len() + 1)).collect();
        let r_args: Vec<T<'_>> = asked.into_iter().chain([T::Int(0)]).collect();
        b.rule().head(r.as_str(), &r_args).atom(q.as_str(), &q_args).atom(bn.as_str(), &b_read).finish()?;
        natives.push((r.clone(), q.clone(), bn.clone(), s.group.len(), s.op.text()));
    }
    Ok(natives)
}
