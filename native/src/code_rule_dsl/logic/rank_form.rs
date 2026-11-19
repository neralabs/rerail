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

/// What the form reads of a program besides the rules it rewrites: the tree's (`find`) or the version's
/// (`corpus::preparation::rank`), so both make the same.
pub(crate) trait Of {
    /// `rel`'s columns' types, where it is declared.
    fn types(&self, rel: &str) -> Option<Vec<String>>;
    /// Whether `rel` is held as rows whatever its rule: made natively, a lattice, an input, or `no_rank`.
    fn kept(&self, rel: &str) -> bool;
    /// `rel`'s rule, where it has exactly one.
    fn one_rule(&mut self, rel: &str) -> Option<Rule>;
}

/// What the form made: the new relations' columns, their rules (in order, after the program's) and the ranks'
/// natives; and how many readers it rewrote.
#[derive(Default)]
pub(crate) struct Made {
    pub decls: Vec<(String, Vec<(String, String)>)>,
    pub rules: Vec<Rule>,
    pub natives: Vec<(String, Native)>,
    pub found: usize,
}

impl Made {
    fn declared(&self, of: &dyn Of, rel: &str) -> bool {
        of.types(rel).is_some() || self.decls.iter().any(|(r, _)| r == rel)
    }
}

/// Whether `rule` has a literal the form may read (`plan` decides).
pub(crate) fn counts(rule: &Rule) -> bool {
    rule.body.iter().any(|l| matches!(l, Literal::Agg { op: AggOp::Count, over: None, atom, .. } if !atom.neg))
}

/// Whether the form is looked for (`RERAIL_RANK_FORM=off`: not, a measurement aid).
pub(crate) fn on() -> bool {
    !std::env::var("RERAIL_RANK_FORM").is_ok_and(|v| v == "off")
}

/// The form over `readers` (each a rule, by the caller's key, in the program's order): each reader it rewrote, and
/// what it made.
pub(crate) fn plan<K: Clone>(of: &mut dyn Of, readers: &[(K, Rule)]) -> (Vec<(K, Rule)>, Made) {
    let mut made = Made::default();
    let mut placed: BTreeMap<(String, Vec<bool>), Option<(String, Vec<usize>)>> = BTreeMap::new();
    let mut changed = Vec::new();
    for (key, rule) in readers {
        let mut rule = rule.clone();
        let mut touched = false;
        for j in 0..rule.body.len() {
            let Literal::Agg { op: AggOp::Count, over: None, atom, .. } = &rule.body.as_slice()[j] else { continue };
            if atom.neg || of.kept(atom.rel.as_str()) {
                continue;
            }
            let (rel, grouped): (String, Vec<bool>) = (atom.rel.to_string(), atom.args.iter().map(|t| !matches!(t, Term::Wild)).collect());
            let k = (rel.clone(), grouped.clone());
            if !placed.contains_key(&k) {
                let now = of.one_rule(&rel).and_then(|h| shape(&h, &grouped).map(|s| place(of, &mut made, &h, s)));
                placed.insert(k.clone(), now);
            }
            let Some((rank, at)) = placed[&k].clone() else { continue };
            let Literal::Agg { var, atom, .. } = &rule.body.as_slice()[j] else { unreachable!("a reader") };
            let mut taken: BTreeSet<String> = rule.body.iter().flat_map(Literal::vars).collect();
            taken.extend(rule.head.vars());
            let n = (0..).map(|k| format!("rr_rankn{k}")).find(|v| !taken.contains(v)).expect("a fresh name");
            let mut args: Vec<Term> = at.iter().map(|&c| atom.args.as_slice()[c].clone()).collect();
            args.push(Term::Var(n.clone().into()));
            let lit = Literal::Agg { var: var.clone(), op: AggOp::Sum, over: Some(n), atom: Atom { rel: rank.into(), args: args.into(), neg: false, gated: false } };
            let mut body: Vec<Literal> = rule.body.iter().cloned().collect();
            body[j] = lit;
            rule.body = body.into();
            made.found += 1;
            touched = true;
        }
        if touched {
            changed.push((key.clone(), rule));
        }
    }
    (changed, made)
}

/// The tree's program as the form reads it.
struct TreeOf<'p> {
    prog: &'p Program,
    rules_of: BTreeMap<&'p str, Vec<usize>>,
}

impl Of for TreeOf<'_> {
    fn types(&self, rel: &str) -> Option<Vec<String>> {
        self.prog.decls.get(rel).map(|cols| cols.iter().map(|(_, t)| t.to_string()).collect())
    }
    fn kept(&self, rel: &str) -> bool {
        self.prog.natives.contains_key(rel)
            || self.prog.lattice.contains_key(rel)
            || self.prog.inputs.iter().any(|x| x == rel)
            || self.prog.qualifiers.get(rel).is_some_and(|w| w.contains(NO_RANK))
    }
    fn one_rule(&mut self, rel: &str) -> Option<Rule> {
        match self.rules_of.get(rel).map(Vec::as_slice) {
            Some(&[i]) => Some(self.prog.rules[i].clone()),
            _ => None,
        }
    }
}

/// Each reader of the form rewritten, on the tree. Returns how many.
pub fn find(prog: &mut Program) -> usize {
    if !on() {
        return 0;
    }
    let readers: Vec<(usize, Rule)> = prog.rules.iter().enumerate().filter(|(_, r)| counts(r)).map(|(i, r)| (i, r.clone())).collect();
    if readers.is_empty() {
        return 0;
    }
    let (changed, made) = {
        let mut rules_of: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, r) in prog.rules.iter().enumerate() {
            rules_of.entry(r.head.rel.as_str()).or_default().push(i);
        }
        plan(&mut TreeOf { prog, rules_of }, &readers)
    };
    for (i, r) in changed {
        prog.rules[i] = r;
    }
    for (rel, cols) in made.decls {
        prog.decls.insert(rel.into(), cols.into_iter().map(|(c, t)| (c.into(), t.into())).collect());
    }
    prog.rules.extend(made.rules);
    prog.natives.extend(made.natives);
    made.found
}

/// The form in `rule` (the counted relation's one rule) for a reader grouping by `grouped`, if it has it.
fn shape(rule: &Rule, grouped: &[bool]) -> Option<Shape> {
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
    None
}

/// The relations of `s` made, and the rank's: its name, and for each of its columns but the count, the place in the
/// counted relation's head of the variable it holds.
fn place(of: &dyn Of, made: &mut Made, rule: &Rule, s: Shape) -> (String, Vec<usize>) {
    let fresh = |made: &Made, stem: &str| (0..).map(|k| format!("{stem}{k}")).find(|n| !made.declared(of, n)).expect("a fresh name");
    // A variable's type: its column's in the head, else in an atom of the body; a key is a number otherwise.
    let type_of = |v: &str| -> String {
        let atoms = std::iter::once(&rule.head).chain(rule.body.iter().filter_map(|l| if let Literal::Atom(a) = l { Some(a) } else { None }));
        for a in atoms {
            if let Some(cols) = of.types(a.rel.as_str())
                && let Some(c) = a.args.iter().position(|t| matches!(t, Term::Var(w) if w == v))
                && let Some(t) = cols.get(c)
            {
                return t.clone();
            }
        }
        "int".to_owned()
    };
    let var = |v: &str| Term::Var(v.to_owned().into());
    let atom = |rel: &str, args: Vec<Term>| Atom { rel: rel.to_owned().into(), args: args.into(), neg: false, gated: false };
    let decl = |made: &mut Made, rel: &str, vars: &[String], last: Option<(&str, String)>| {
        let mut cols: Vec<(String, String)> = vars.iter().enumerate().map(|(i, v)| (format!("c{i}"), type_of(v))).collect();
        cols.extend(last.map(|(n, t)| (n.to_owned(), t)));
        made.decls.push((rel.to_owned(), cols));
    };
    let q = fresh(made, "rr_rankq");
    let asked_cols: Vec<String> = s.group.iter().chain(&s.asked).cloned().collect();
    let q_args: Vec<Term> = asked_cols.iter().map(|v| var(v)).chain([var(&s.kx)]).collect();
    decl(made, &q, &asked_cols, Some(("k", type_of(&s.kx))));
    made.rules.push(Rule { head: atom(&q, q_args.clone()), body: s.x.clone().into() });
    let g = fresh(made, "rr_rankg");
    decl(made, &g, &s.group, None);
    let g_args: Vec<Term> = s.group.iter().map(|v| var(v)).collect();
    made.rules.push(Rule { head: atom(&g, g_args.clone()), body: vec![Literal::Atom(atom(&q, q_args.clone()))].into() });
    let b = fresh(made, "rr_rankb");
    let counted_cols: Vec<String> = s.group.iter().chain(&s.counted).cloned().collect();
    decl(made, &b, &counted_cols, Some(("k", type_of(&s.ky))));
    let mut b_body = vec![Literal::Atom(atom(&g, g_args))];
    b_body.extend(s.y.iter().cloned());
    made.rules.push(Rule { head: atom(&b, counted_cols.iter().map(|v| var(v)).chain([var(&s.ky)]).collect()), body: b_body.into() });
    let r = fresh(made, "rr_rank");
    decl(made, &r, &asked_cols, Some(("n", "int".to_owned())));
    // Its one rule orders it after both relations it reads, and joins them on the group as `H` did: the split
    // (`file_local`) finds it local where its groups are one document's. The rows are made natively (as a reach's,
    // decision 0119).
    let b_read: Vec<Term> = s.group.iter().map(|v| var(v)).chain(std::iter::repeat_n(Term::Wild, s.counted.len() + 1)).collect();
    made.rules.push(Rule {
        head: atom(&r, asked_cols.iter().map(|v| var(v)).chain([Term::Int(0)]).collect()),
        body: vec![Literal::Atom(atom(&q, q_args)), Literal::Atom(atom(&b, b_read))].into(),
    });
    made.natives.push((r.clone(), Native::Rank { query: q, counted: b, group: s.group.len(), op: s.op.text().to_owned() }));
    let head: Vec<String> = rule.head.args.iter().map(|t| if let Term::Var(v) = t { v.to_string() } else { String::new() }).collect();
    let at = asked_cols.iter().map(|v| head.iter().position(|h| h == v).expect("a head variable")).collect();
    (r, at)
}

#[cfg(test)]
mod tests;
