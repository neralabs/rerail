//! Liveness: which of a program's facts its outputs read (note t26).
//!
//! A fact is live if some derivation of an output row uses it. Liveness flows backwards through the rules, so it is
//! itself a program over the program's relations, run to its fixed point after them:
//!
//! ```text
//! // a rule of the program                 // the rules `program` makes from it
//! calls(c, f) :- call(c, x), look(c, x, s), bv(s, f).
//!                                          live_call(c, x)    :- live_calls(c, f), call(c, x), look(c, x, s), bv(s, f).
//!                                          live_look(c, x, s) :- live_calls(c, f), call(c, x), look(c, x, s), bv(s, f).
//!                                          live_bv(s, f)      :- live_calls(c, f), call(c, x), look(c, x, s), bv(s, f).
//! // an output                              live_calls(c, f)   :- calls(c, f).
//! ```
//!
//! - A read inside an aggregate (`n = count : { r(x, _) }`) makes every row of its group live.
//! - A negated read `!r(k)` makes live the rows of `r` that block a derivation: those matching `k` where the rest
//!   of the body holds. Without them the negation would pass, and the outputs would change. These are the keys a
//!   demand path for negations asks for (decision 0048's `demand_R`, `magic::gate_negations`), so the counts show
//!   what such a path would compute.
//! - A head's computed argument (`@dot(q, x)`) is matched by computing it again; a lattice relation is matched by its
//!   keys alone (its value is the join of every derivation's).
//!
//! The counts (`COUNTS`, one output): for every relation read or derived, its rows and live rows, its keys (first
//! column) and live keys, and, for a relation whose first column is a node, the rows about loaded documents (a
//! library's, such as typeshed's stubs: `doc(d, p, _), loaded(p, _, _)`) and the live ones among them. The keys are
//! what `.demand r` (`preparation::magic::KeyDemand`) restricts a relation to: live keys against all keys is what demand by key
//! would save.
//!
//! This is the measurement before deriving less: `Mode::After` runs the backward rules over the finished fixpoint.
//! The demand path (`Mode::Before`, not built) is the same rules with each body cut to the literals before the read
//! (the sideways order `magic` passes), so that the head is asked for before it is derived.

use std::collections::BTreeSet;

use super::front::ast::AggOp;
use super::program_image::{Builder, CmpOp, Lit, T, TermRef, Version, t_of};
use std::sync::Arc;

/// The counts' relation: `liveof_count(rel, what, n)`.
pub const COUNTS: &str = "liveof_count";

/// What `COUNTS` counts, in the order the report prints it.
pub const MEASURES: [&str; 6] = ["rows", "live", "keys", "live keys", "library", "library live"];

/// `rel`'s liveness relation, under a prefix no program uses: a program may name its own relations `live_…`
/// (calls_cx has `live_def`).
pub(in crate::code_rule_dsl) fn live(rel: &str) -> String {
    format!("liveof_{rel}")
}

/// A count of `of`'s rows: `liveof_count(rel, what, n) :- n = count : { of(args) }`.
fn count(b: &mut Builder<'_>, rel: &str, what: &str, of: &str, args: &[T<'_>]) -> Result<(), String> {
    b.rule().head(COUNTS, &[T::Str(rel), T::Str(what), T::Var("n")]).agg(T::Var("n"), &AggOp::Count, None, of, args).finish().map(|_| ())
}

/// The backward rules and the counts for `v` (its rules, declarations and outputs, as `rr::parse_version` gives them):
/// a program of their own, under `v`'s names, to print after `v`'s text, which declares what they read.
pub fn program(v: &Version) -> Result<Arc<Version>, String> {
    // A program naming a relation as these do would read the liveness rules' rows as its own.
    if let Some(r) = v.declared_names().into_iter().find(|r| r.starts_with(&live(""))) {
        panic!("{r}: the liveness rules' prefix is taken");
    }
    let nullary = v.nullary_names()?;
    let lattice = v.lattice_decls()?;
    let declared = |r: &str| v.declares(r) && !nullary.contains(r);
    let arity = |r: &str| v.columns_of(r).map_or(0, |c| c.len());
    let mut b = Builder::under(v);
    let mut fresh = 0usize;
    let mut targets: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for o in v.output_names().into_iter().filter(|o| declared(o)) {
        let names: Vec<String> = (0..arity(&o)).map(|i| format!("liveof_vo{i}")).collect();
        let args: Vec<T<'_>> = names.iter().map(|n| T::Var(n)).collect();
        b.rule().head(live(&o).as_str(), &args).atom(o.as_str(), &args).finish()?;
        targets.insert(o);
    }
    for r in v.rules() {
        let h = v.rel_name(r.head().rel());
        if !declared(h) {
            continue;
        }
        // Every derived relation is counted, read or not: a relation nothing reads is all dead rows.
        targets.insert(h.to_owned());
        // The head as a read of its liveness: a computed argument is a variable the body computes again.
        let lattice_keys = lattice.get(h).map(|l| l.keys);
        let mut names: Vec<String> = Vec::new();
        let mut name = || {
            fresh += 1;
            format!("liveof_v{fresh}")
        };
        // Each head argument: its term, or a fresh variable's index in `names` (with the call it computes).
        let head: Vec<(Option<TermRef<'_>>, Option<usize>, bool)> = r
            .head()
            .args()
            .enumerate()
            .map(|(i, t)| match t {
                _ if lattice_keys.is_some_and(|k| i >= k) => (None, Some({ names.push(name()); names.len() - 1 }), false),
                TermRef::Call(..) => (Some(t), Some({ names.push(name()); names.len() - 1 }), true),
                TermRef::Wild | TermRef::Top => (None, Some({ names.push(name()); names.len() - 1 }), false),
                other => (Some(other), None, false),
            })
            .collect();
        let lits: Vec<_> = r.body().collect();
        // Each read's fresh variables for its wildcards, made before any rule borrows the names.
        let mut wild: Vec<Vec<Option<usize>>> = Vec::new();
        for l in &lits {
            let read = match l.view() {
                Lit::Atom(a) => Some(a),
                Lit::Agg { atom, .. } => Some(atom),
                _ => None,
            };
            wild.push(read.map(|a| a.args().map(|t| matches!(t, TermRef::Wild).then(|| { names.push(name()); names.len() - 1 })).collect()).unwrap_or_default());
        }
        let head_args: Vec<T<'_>> = head.iter().map(|(t, n, _)| match n { Some(k) => T::Var(&names[*k]), None => t_of(r, t.expect("a term")) }).collect();
        for (i, l) in lits.iter().enumerate() {
            let (read, negated, aggregated) = match l.view() {
                Lit::Atom(a) => (a, a.neg(), false),
                Lit::Agg { atom, .. } => (atom, false, true),
                _ => continue,
            };
            let rel = v.rel_name(read.rel());
            if !declared(rel) {
                continue;
            }
            let args: Vec<T<'_>> = read.args().zip(&wild[i]).map(|(t, n)| match n { Some(k) => T::Var(&names[*k]), None => t_of(r, t) }).collect();
            let live_rel = live(rel);
            let live_h = live(h);
            let mut rule = b.rule().head(live_rel.as_str(), &args);
            if !negated {
                rule = rule.atom(live_h.as_str(), &head_args);
            }
            for (j, other) in lits.iter().enumerate() {
                if j != i {
                    rule = rule.lit(v, r, *other);
                } else if aggregated {
                    rule = rule.lit(v, r, *other).atom(rel, &args);
                } else {
                    rule = rule.atom(rel, &args);
                }
            }
            if !negated {
                for (k, (t, n, call)) in head.iter().enumerate() {
                    if *call {
                        rule = rule.cmp(head_args[k].clone(), CmpOp::Eq, t_of(r, t.expect("a call")));
                        let _ = n;
                    }
                }
            }
            rule.finish()?;
            targets.insert(rel.to_owned());
        }
    }
    b.fields().declare(COUNTS, &[("rel", "symbol"), ("what", "symbol"), ("n", "number")]);
    let library = v.declares("doc") && v.declares("loaded");
    for t in &targets {
        let cols = v.columns_of(t).unwrap_or_default();
        let col_refs: Vec<(&str, &str)> = cols.iter().map(|(n, ty)| (n.as_str(), ty.as_str())).collect();
        let c_names: Vec<String> = (0..cols.len()).map(|i| format!("c{i}")).collect();
        let vars: Vec<T<'_>> = c_names.iter().map(|n| T::Var(n)).collect();
        b.fields().declare(&live(t), &col_refs);
        count(&mut b, t, MEASURES[0], t, &vars)?;
        count(&mut b, t, MEASURES[1], &live(t), &vars)?;
        // Keys: the first column's values, of all rows and of the live ones.
        let key = |of: &str| format!("liveof_key{of}_{t}");
        for (name, rel) in [(key(""), t.clone()), (key("l"), live(t))] {
            b.fields().declare(&name, &col_refs[..1]);
            let mut args = vec![T::Var("k")];
            args.extend((1..cols.len()).map(|_| T::Wild));
            b.rule().head(name.as_str(), &[T::Var("k")]).atom(rel.as_str(), &args).finish()?;
        }
        count(&mut b, t, MEASURES[2], &key(""), &[T::Var("k")])?;
        count(&mut b, t, MEASURES[3], &key("l"), &[T::Var("k")])?;
        // Rows about a loaded document: a library's.
        if library && cols[0].1 == "node" {
            for (name, rel, what) in [(format!("liveof_lib_{t}"), t.clone(), MEASURES[4]), (format!("liveof_libl_{t}"), live(t), MEASURES[5])] {
                b.fields().declare(&name, &col_refs);
                let (d, p) = (T::Var("liveof_vd"), T::Var("liveof_vp"));
                b.rule()
                    .head(name.as_str(), &vars)
                    .atom(rel.as_str(), &vars)
                    .cmp(d.clone(), CmpOp::Eq, T::Call("doc", vec![vars[0].clone()]))
                    .atom("doc", &[d, p.clone(), T::Wild])
                    .atom("loaded", &[p, T::Wild, T::Wild])
                    .finish()?;
                count(&mut b, t, what, &name, &vars)?;
            }
        }
    }
    super::program_image::fields::FieldsMut::add_output(&mut b.fields(), COUNTS);
    Ok(b.finish()?.0)
}


// Its tests run in the engine's test binary (its facade of this module mounts `tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rerail` libraries).

