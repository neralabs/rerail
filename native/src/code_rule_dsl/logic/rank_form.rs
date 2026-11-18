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
use crate::code_rule_dsl::logic::front::{AggOp, Atom, Literal, Native, Program, Rule, Term};
use crate::code_rule_dsl::logic::native::rank::Op;
use std::collections::{BTreeMap, BTreeSet};

/// The qualifier keeping a relation as rows.
pub const NO_RANK: &str = "no_rank";

/// The form found in a relation's rule, for a reader that groups by `grouped` (its head's columns the reader binds).
struct Shape {
    /// The group: grouped variables the counted part reads, in head order.
    group: Vec<String>,
    /// The other grouped variables, in head order.
    asked: Vec<String>,
    /// The counted variables (the reader's `_`), in head order.
    counted: Vec<String>,
    kx: String,
    ky: String,
    op: Op,
    x: Vec<Literal>,
    y: Vec<Literal>,
}

/// Each reader of the form rewritten. Returns how many.
pub fn find(prog: &mut Program) -> usize {
    if std::env::var("RERAIL_RANK_FORM").is_ok_and(|v| v == "off") {
        return 0;
    }
    let mut rules_of: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, r) in prog.rules.iter().enumerate() {
        rules_of.entry(r.head.rel.to_string()).or_default().push(i);
    }
    // Each reader: its rule, its literal, the relation and which of its columns it groups by.
    let mut readers: Vec<(usize, usize, String, Vec<bool>)> = Vec::new();
    for (i, r) in prog.rules.iter().enumerate() {
        for (j, l) in r.body.iter().enumerate() {
            if let Literal::Agg { op: AggOp::Count, over: None, atom, .. } = l
                && !atom.neg
                && rules_of.get(atom.rel.as_str()).is_some_and(|rs| rs.len() == 1)
                && !prog.natives.contains_key(atom.rel.as_str())
                && !prog.lattice.contains_key(atom.rel.as_str())
                && !prog.inputs.iter().any(|x| x == atom.rel.as_str())
                && !prog.qualifiers.get(atom.rel.as_str()).is_some_and(|w| w.contains(NO_RANK))
            {
                readers.push((i, j, atom.rel.to_string(), atom.args.iter().map(|t| !matches!(t, Term::Wild)).collect()));
            }
        }
    }
    let mut made: BTreeMap<(String, Vec<bool>), Option<(String, Vec<usize>)>> = BTreeMap::new();
    let mut found = 0;
    for (i, j, rel, grouped) in readers {
        let key = (rel.clone(), grouped.clone());
        if !made.contains_key(&key) {
            let rule = prog.rules[rules_of[&rel][0]].clone();
            let made_now = shape(prog, &rule, &grouped).map(|s| place(prog, &rule, s));
            made.insert(key.clone(), made_now);
        }
        let Some((rank, at)) = made[&key].clone() else { continue };
        let Literal::Agg { var, atom, .. } = &prog.rules[i].body.as_slice()[j] else { unreachable!("a reader") };
        let mut taken: BTreeSet<String> = prog.rules[i].body.iter().flat_map(Literal::vars).collect();
        taken.extend(prog.rules[i].head.vars());
        let n = (0..).map(|k| format!("rr_rankn{k}")).find(|v| !taken.contains(v)).expect("a fresh name");
        let mut args: Vec<Term> = at.iter().map(|&c| atom.args.as_slice()[c].clone()).collect();
        args.push(Term::Var(n.clone().into()));
        let lit = Literal::Agg { var: var.clone(), op: AggOp::Sum, over: Some(n), atom: Atom { rel: rank.into(), args: args.into(), neg: false, gated: false } };
        let mut body: Vec<Literal> = prog.rules[i].body.iter().cloned().collect();
        body[j] = lit;
        prog.rules[i].body = body.into();
        found += 1;
    }
    found
}

/// The form in `rule` (the counted relation's one rule) for a reader grouping by `grouped`, if it has it.
fn shape(prog: &Program, rule: &Rule, grouped: &[bool]) -> Option<Shape> {
    let head: Vec<&str> = rule.head.args.iter().map(|t| if let Term::Var(v) = t { Some(v.as_str()) } else { None }).collect::<Option<_>>()?;
    if head.iter().collect::<BTreeSet<_>>().len() != head.len() || head.len() != grouped.len() {
        return None;
    }
    let gv: BTreeSet<String> = head.iter().zip(grouped).filter(|(_, g)| **g).map(|(v, _)| v.to_string()).collect();
    let cv: Vec<String> = head.iter().zip(grouped).filter(|(_, g)| !**g).map(|(v, _)| v.to_string()).collect();
    if cv.is_empty() {
        return None;
    }
    let body: Vec<&Literal> = rule.body.iter().collect();
    // Variables a literal's positive atoms bind.
    let bound = |ls: &[&Literal]| -> BTreeSet<String> { ls.iter().filter_map(|l| if let Literal::Atom(a) = l && !a.neg { Some(a.vars()) } else { None }).flatten().collect() };
    for (ci, l) in body.iter().enumerate() {
        let Literal::Cmp { l: Term::Var(a), op, r: Term::Var(b) } = l else { continue };
        let Some(op) = Op::parse(op) else { continue };
        for (ky, kx, op) in [(a.to_string(), b.to_string(), op), (b.to_string(), a.to_string(), op.flipped())] {
            if gv.contains(&ky) {
                continue;
            }
            let rest: Vec<&Literal> = body.iter().enumerate().filter(|(k, _)| *k != ci).map(|(_, l)| *l).collect();
            let own: Vec<BTreeSet<String>> = rest.iter().map(|l| l.vars().into_iter().filter(|v| !gv.contains(v)).collect()).collect();
            // The counted part: the literals reached from the counted variables and ky through their own variables.
            let mut reach: BTreeSet<String> = cv.iter().cloned().chain([ky.clone()]).collect();
            let mut in_y = vec![false; rest.len()];
            let mut grew = true;
            while grew {
                grew = false;
                for (k, o) in own.iter().enumerate() {
                    if !in_y[k] && !o.is_disjoint(&reach) {
                        in_y[k] = true;
                        reach.extend(o.iter().cloned());
                        grew = true;
                    }
                }
            }
            if reach.contains(&kx) && !gv.contains(&kx) {
                continue;
            }
            let y: Vec<&Literal> = rest.iter().zip(&in_y).filter(|(_, y)| **y).map(|(l, _)| *l).collect();
            let x: Vec<&Literal> = rest.iter().zip(&in_y).filter(|(_, y)| !**y).map(|(l, _)| *l).collect();
            if y.is_empty() {
                continue;
            }
            let y_vars: BTreeSet<String> = y.iter().flat_map(|l| l.vars()).collect();
            let group: Vec<String> = head.iter().map(|v| v.to_string()).filter(|v| gv.contains(v) && y_vars.contains(v)).collect();
            let asked: Vec<String> = head.iter().map(|v| v.to_string()).filter(|v| gv.contains(v) && !y_vars.contains(v)).collect();
            // The asked part binds every grouped variable and the bound; the counted part, with the group, binds
            // the counted variables and the key.
            let (bx, by) = (bound(&x), bound(&y));
            if !gv.iter().chain([&kx]).all(|v| bx.contains(v)) || !cv.iter().chain([&ky]).all(|v| by.contains(v) || group.contains(v)) {
                continue;
            }
            return Some(Shape { group, asked, counted: cv, kx, ky, op, x: x.into_iter().cloned().collect(), y: y.into_iter().cloned().collect() });
        }
    }
    let _ = prog;
    None
}

/// The relations of `s` made, and the rank's: its name, and for each of its columns but the count, the place in the
/// counted relation's head of the variable it holds.
fn place(prog: &mut Program, rule: &Rule, s: Shape) -> (String, Vec<usize>) {
    let fresh = |prog: &Program, stem: &str| (0..).map(|k| format!("{stem}{k}")).find(|n| !prog.decls.contains_key(n.as_str())).expect("a fresh name");
    // A variable's type: its column's in the head, else in an atom of the body; a key is a number otherwise.
    let type_of = |prog: &Program, v: &str| -> String {
        let atoms = std::iter::once(&rule.head).chain(rule.body.iter().filter_map(|l| if let Literal::Atom(a) = l { Some(a) } else { None }));
        for a in atoms {
            if let Some(cols) = prog.decls.get(a.rel.as_str())
                && let Some(c) = a.args.iter().position(|t| matches!(t, Term::Var(w) if w == v))
                && let Some((_, t)) = cols.get(c)
            {
                return t.to_string();
            }
        }
        "int".to_owned()
    };
    let var = |v: &str| Term::Var(v.to_owned().into());
    let atom = |rel: &str, args: Vec<Term>| Atom { rel: rel.to_owned().into(), args: args.into(), neg: false, gated: false };
    let decl = |prog: &mut Program, rel: &str, vars: &[String], last: Option<(&str, String)>| {
        let mut cols: Vec<(String, String)> = vars.iter().enumerate().map(|(i, v)| (format!("c{i}"), type_of(prog, v))).collect();
        cols.extend(last.map(|(n, t)| (n.to_owned(), t)));
        prog.decls.insert(rel.to_owned().into(), cols.into_iter().map(|(c, t)| (c.into(), t.into())).collect());
    };
    let (q, g, b, r) = (fresh(prog, "rr_rankq"), fresh(prog, "rr_rankg"), fresh(prog, "rr_rankb"), fresh(prog, "rr_rank"));
    let asked_cols: Vec<String> = s.group.iter().chain(&s.asked).cloned().collect();
    let q_args: Vec<Term> = asked_cols.iter().map(|v| var(v)).chain([var(&s.kx)]).collect();
    let kx_type = type_of(prog, &s.kx);
    decl(prog, &q, &asked_cols, Some(("k", kx_type)));
    prog.rules.push(Rule { head: atom(&q, q_args.clone()), body: s.x.clone().into() });
    decl(prog, &g, &s.group, None);
    let g_args: Vec<Term> = s.group.iter().map(|v| var(v)).collect();
    prog.rules.push(Rule { head: atom(&g, g_args.clone()), body: vec![Literal::Atom(atom(&q, q_args.clone()))].into() });
    let counted_cols: Vec<String> = s.group.iter().chain(&s.counted).cloned().collect();
    let ky_type = type_of(prog, &s.ky);
    decl(prog, &b, &counted_cols, Some(("k", ky_type)));
    let mut b_body = vec![Literal::Atom(atom(&g, g_args))];
    b_body.extend(s.y.iter().cloned());
    prog.rules.push(Rule { head: atom(&b, counted_cols.iter().map(|v| var(v)).chain([var(&s.ky)]).collect()), body: b_body.into() });
    decl(prog, &r, &asked_cols, Some(("n", "int".to_owned())));
    // Its one rule orders it after both relations it reads, and joins them on the group as `H` did: the split
    // (`file_local`) finds it local where its groups are one document's. The rows are made natively (as a reach's,
    // decision 0119).
    let b_read: Vec<Term> = s.group.iter().map(|v| var(v)).chain(std::iter::repeat_n(Term::Wild, s.counted.len() + 1)).collect();
    prog.rules.push(Rule {
        head: atom(&r, asked_cols.iter().map(|v| var(v)).chain([Term::Int(0)]).collect()),
        body: vec![Literal::Atom(atom(&q, q_args)), Literal::Atom(atom(&b, b_read))].into(),
    });
    prog.natives.insert(r.clone(), Native::Rank { query: q, counted: b, group: s.group.len(), op: s.op.text().to_owned() });
    let head: Vec<String> = rule.head.args.iter().map(|t| if let Term::Var(v) = t { v.to_string() } else { String::new() }).collect();
    let at = asked_cols.iter().map(|v| head.iter().position(|h| h == v).expect("a head variable")).collect();
    (r, at)
}

#[cfg(test)]
mod tests;
