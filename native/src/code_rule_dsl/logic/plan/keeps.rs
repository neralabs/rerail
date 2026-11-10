//! What a relation keeps (decision 0099): stored, or recomputed in its readers.
//!
//! A derived relation that is built large and read small costs its rows twice: once to build them, once more to
//! write, index and keep them, though its readers keep a few. Recomputed inside each reader instead (inlined), a
//! reader that binds the relation's key does only the work that key reaches.
//!
//! The choice is made per relation, after `simplify` and before demand (`demand::split_negations`), from the
//! planner's own estimates:
//!
//! - **The relation's cost when stored:** the work to build it and its rows (`plan_rule` on each of its rules), each
//!   row priced `M` more for writing, indexing and keeping it.
//! - **Its cost when inlined:** the work its readers add once each reads the relation's rules in place of its rows
//!   (`plan_rule` on every reader copy, less the reader as it is).
//!
//! A relation is inlined when the added work is under `ALPHA` times its cost stored, and it has at most `K` rules,
//! and inlining makes at most `COPIES` reader copies. A relation of observed rows (an earlier chunk measured it) is
//! priced from them. Stored always: outputs, inputs, lattices, relations with facts, natives and what they read,
//! demanded relations, gates, answers, recursive relations, relations read inside an aggregate (a count, a sum or a
//! string join reads the distinct rows), and relations read by a negation whose unfolding is not a negation of plain
//! literals (below).
//!
//! **Unfolding.** A positive read `r(t…)` becomes one reader copy per rule of `r`, the rule's variables renamed apart
//! and its head's terms matched to the read's. A negated read `!r(t…)` is the negation of every rule of `r`:
//!
//! - a rule whose body is one positive atom, whose own variables appear once (`r(x) :- b(x, y)`), is `!b(t, _)`;
//! - a rule whose every literal names only its head's variables, all bound by the read, is the disjunction of its
//!   literals negated (`r(x) :- a(x), x != 3` read as `!r(t)` is `!a(t)` or `t = 3`): one copy per literal.
//!
//! Any other negated read keeps the relation stored: a negation of a join with variables of its own is no literal.
//!
//! **The program's override.** `.decl r(…) inline` inlines `r` whatever the price (an error where it cannot be);
//! `.decl r(…) no_inline` stores it. `RERAIL_INLINE=off` stores every relation, for comparison, and
//! `RERAIL_INLINE_ONLY=r,…` inlines those wherever they can be and stores every other (the lab's comparison).
//!
//! The profile names each choice and why (`inline or store (decision 0099)`).

use super::super::front::{Atom, Literal, Program, Rule, Term, strata};
use super::Model;
use std::collections::{BTreeMap, BTreeSet};

/// Inlined when the readers' added work is under this share of the relation's cost stored (decision 0099, "Built":
/// the relational lab's comparison on the Python pack's fixtures). The estimates are in the planner's units of join
/// work on both sides, so 1 is the break-even; no case measured was near it.
pub(super) const ALPHA: f64 = 1.0;
/// The price of a stored row beyond the work that made it: written, then indexed and read in by each stratum that
/// reads it, in units of join work. Measured on `module_of` (129 k rows): its readers spent 303 ms more CPU over the
/// stored rows than inlined, at 0.6 µs per unit of its build's work, about 4 units a row.
pub(super) const M: f64 = 4.0;
/// At most this many rules in an inlined relation.
pub(super) const K: usize = 4;
/// At most this many reader copies made by inlining one relation.
pub(super) const COPIES: usize = 8;
/// A relation estimated under this many rows is stored without pricing: its rows cost little either way.
const SMALL: f64 = 1024.0;

/// The choices, and the program with the inlined relations unfolded into their readers; `None` when nothing is
/// inlined. `Err` names a `.decl r(…) inline` that cannot be honoured.
pub(super) fn choose(prog: &Program, model: &mut Model) -> Result<Option<(Program, Vec<String>)>, String> {
    let off = std::env::var("RERAIL_INLINE").is_ok_and(|v| v == "off");
    if off {
        if !prog.qualifiers.values().flatten().any(|w| w == "inline") {
            return Ok(None);
        }
        super::super::profile::decision("inline or store (decision 0099)", || "RERAIL_INLINE=off: every relation stored, `.decl … inline` too".to_owned());
        return Ok(None);
    }
    let Ok((sccs, _)) = strata(prog) else { return Ok(None) };
    let mut rules_of: BTreeMap<&str, Vec<&Rule>> = BTreeMap::new();
    for r in &prog.rules {
        rules_of.entry(r.head.rel.as_str()).or_default().push(r);
    }
    let reasons = refusals(prog, &rules_of, &sccs);
    // Readers: (rule index, body position) of each read of a relation.
    let mut readers: BTreeMap<&str, Vec<(usize, usize)>> = BTreeMap::new();
    for (ri, r) in prog.rules.iter().enumerate() {
        for (i, l) in r.body.iter().enumerate() {
            if let Some(a) = l.atom() {
                readers.entry(a.rel.as_str()).or_default().push((ri, i));
            }
        }
    }
    // `RERAIL_INLINE_ONLY=r,…` (the lab's comparison): those relations inlined where they can be, every other stored.
    let only: Option<Vec<String>> = std::env::var("RERAIL_INLINE_ONLY").ok().map(|v| v.split(',').map(str::to_owned).collect());
    // The estimates below plan every rule, which a maintained program has about ten times of. They price only a
    // relation that may be inlined: none (a session's part stores its relations, decision 0099), none are made.
    let open = |rel: &str| !prog.qualified(rel, "no_inline") && !reasons.contains_key(rel) && readers.contains_key(rel);
    if only.is_none() && !rules_of.keys().any(|rel| prog.qualified(rel, "inline") || open(rel)) {
        let stored = rules_of.keys().filter(|rel| prog.qualified(rel, "no_inline")).count();
        super::super::profile::decision("inline or store (decision 0099)", || format!("no relation may be inlined ({stored} `.decl … no_inline`): nothing estimated"));
        return Ok(None);
    }
    // Estimated rows and work of every derived relation, in evaluation order; observed rows where an earlier chunk
    // measured them. The estimates are the planner's while it prices the readers, then taken back.
    let (estimates, inserted) = estimate(prog, model, &rules_of, &sccs);
    let mut inline: BTreeSet<String> = BTreeSet::new();
    let mut lines: Vec<String> = Vec::new();
    let mut refused_inline: Vec<String> = Vec::new();
    for (rel, rules) in &rules_of {
        let rel = *rel;
        if only.as_ref().is_some_and(|o| !o.iter().any(|x| x == rel)) {
            continue;
        }
        if only.is_some() && let Some(why) = reasons.get(rel) {
            lines.push(format!("{rel}: stored, {why} (RERAIL_INLINE_ONLY)"));
            continue;
        }
        let forced = prog.qualified(rel, "inline") || only.is_some();
        if prog.qualified(rel, "no_inline") {
            lines.push(format!("{rel}: stored (`.decl … no_inline`)"));
            continue;
        }
        if let Some(why) = reasons.get(rel) {
            if forced {
                refused_inline.push(format!("{rel}: {why}"));
            }
            continue;
        }
        let reads = readers.get(rel).cloned().unwrap_or_default();
        if reads.is_empty() {
            continue;
        }
        let Some(copies) = reads.iter().map(|&(ri, i)| unfold(&prog.rules[ri], i, rules, 0).map(|c| c.len())).sum::<Option<usize>>() else {
            if prog.qualified(rel, "inline") {
                refused_inline.push(format!("{rel}: a negated read is not a negation of plain literals"));
            } else if only.is_some() {
                lines.push(format!("{rel}: stored, a negated read is not a negation of plain literals (RERAIL_INLINE_ONLY)"));
            }
            continue;
        };
        let (rows, work) = estimates.get(rel).copied().unwrap_or((0.0, 0.0));
        if forced {
            let by = if prog.qualified(rel, "inline") { "`.decl … inline`" } else { "RERAIL_INLINE_ONLY" };
            lines.push(format!("{rel}: inlined ({by}), {copies} reader copies"));
            inline.insert(rel.to_owned());
            continue;
        }
        if rules.len() > K || copies > COPIES {
            lines.push(format!("{rel}: stored, {} rules and {copies} reader copies (at most {K} and {COPIES})", rules.len()));
            continue;
        }
        if rows < SMALL {
            continue;
        }
        // The work the readers add: each copy's plan, less the reader's own.
        let mut added = 0.0;
        let mut priced = true;
        for &(ri, i) in &reads {
            let reader = &prog.rules[ri];
            let copies = unfold(reader, i, rules, 0).unwrap_or_default();
            // A reader in a recursion runs as its delta plans, one per recursive atom read first, over each wave's
            // new rows: priced so, each copy with the same atom first. Its whole plan may start from the inlined
            // relation's own inputs, which no delta plan can: `rr_cx_reach(d, f) :- rr_cx_reach(d, e),
            // rr_cx_edge(e, f)` priced whole joined `rr_cx_edge`'s imports and modules first, and inlining it looked
            // 7.8 M work cheaper; run, each new reach row scanned every document's module (277 s of calls_cx).
            let scc = sccs.iter().find(|s| s.contains(&reader.head.rel));
            let deltas: Vec<usize> = (0..reader.body.len())
                .filter(|&k| k != i && reader.body[k].atom().is_some_and(|a| !a.neg && scc.is_some_and(|s| s.contains(&a.rel))))
                .collect();
            let firsts: Vec<Option<usize>> = if deltas.is_empty() { vec![None] } else { deltas.iter().map(|&k| Some(k)).collect() };
            for first in firsts {
                let Ok((_, own, _)) = model.plan_rule(prog, &reader.head, &reader.body, first) else {
                    priced = false;
                    break;
                };
                for copy in &copies {
                    // The delta atom in the copy: the reader's literal, which unfolding leaves as it is.
                    let at = match first {
                        Some(k) => match copy.body.iter().position(|l| *l == reader.body[k]) {
                            Some(at) => Some(at),
                            None => {
                                priced = false;
                                continue;
                            }
                        },
                        None => None,
                    };
                    match model.plan_rule(prog, &copy.head, &copy.body, at) {
                        Ok((_, w, _)) => added += w,
                        Err(_) => priced = false,
                    }
                }
                added -= own;
            }
        }
        if !priced {
            lines.push(format!("{rel}: stored, a reader copy does not plan"));
            continue;
        }
        let stored = work + M * rows;
        if added < ALPHA * stored {
            lines.push(format!("{rel}: inlined, readers add {added:.0} work < {ALPHA} x (build {work:.0} + {M} x {rows:.0} rows)"));
            inline.insert(rel.to_owned());
        } else {
            lines.push(format!("{rel}: stored, readers would add {added:.0} work >= {ALPHA} x (build {work:.0} + {M} x {rows:.0} rows)"));
        }
    }
    unestimate(model, inserted);
    for line in &lines {
        super::super::profile::decision("inline or store (decision 0099)", || line.clone());
    }
    if !refused_inline.is_empty() {
        return Err(format!("`.decl … inline` cannot be honoured: {}", refused_inline.join("; ")));
    }
    if inline.is_empty() {
        return Ok(None);
    }
    // A relation inlined into another inlined one is unfolded into its copies in turn; one inlined relation reading
    // another is unfolded first, readers after, so a pass takes one level. A chain deeper than the passes stores
    // the rest.
    let mut rules: Vec<Rule> = prog.rules.clone();
    for _ in 0..8 {
        let bodies: BTreeMap<String, Vec<Rule>> = inline
            .iter()
            .map(|r| (r.clone(), rules.iter().filter(|x| x.head.rel == *r).cloned().collect()))
            .collect();
        let mut changed = false;
        let mut next = Vec::with_capacity(rules.len());
        let mut fresh = 0usize;
        for rule in rules {
            let at = rule.body.iter().position(|l| l.atom().is_some_and(|a| inline.contains(&a.rel)));
            match at {
                Some(i) => {
                    let rel = rule.body[i].atom().map(|a| a.rel.clone()).unwrap_or_default();
                    let defs: Vec<&Rule> = bodies[&rel].iter().collect();
                    fresh += 1;
                    match unfold(&rule, i, &defs, fresh) {
                        Some(copies) => {
                            next.extend(copies);
                            changed = true;
                        }
                        None => next.push(rule),
                    }
                }
                None => next.push(rule),
            }
        }
        rules = next;
        if !changed {
            break;
        }
    }
    let unfinished: BTreeSet<String> = rules
        .iter()
        .filter(|r| !inline.contains(&r.head.rel))
        .flat_map(|r| r.body.iter().filter_map(Literal::atom))
        .filter(|a| inline.contains(&a.rel))
        .map(|a| a.rel.clone())
        .collect();
    // An inlined relation some reader still reads (a chain deeper than the passes) keeps its rules.
    let mut out = prog.clone();
    out.rules = rules.into_iter().filter(|r| !inline.contains(&r.head.rel) || unfinished.contains(&r.head.rel)).collect();
    let done: Vec<String> = inline.into_iter().filter(|r| !unfinished.contains(r)).collect();
    if done.is_empty() {
        return Ok(None);
    }
    Ok(Some((out, done)))
}

/// Why each derived relation that must be stored is (the module's list); a relation absent may be inlined.
fn refusals(prog: &Program, rules_of: &BTreeMap<&str, Vec<&Rule>>, sccs: &[BTreeSet<String>]) -> BTreeMap<String, &'static str> {
    let mut why: BTreeMap<String, &'static str> = BTreeMap::new();
    let natives_read: BTreeSet<&str> = prog.natives.values().flat_map(|n| n.reads()).collect();
    for (rel, rules) in rules_of {
        let rel = *rel;
        let reason = if prog.outputs.iter().any(|o| o == rel) {
            Some("an output")
        } else if prog.inputs.iter().any(|i| i == rel) || prog.facts.get(rel).is_some_and(|f| !f.is_empty()) {
            Some("an input or a relation of facts")
        } else if prog.lattice.contains_key(rel)
            || prog.natives.contains_key(rel)
            || natives_read.contains(rel)
            // A native's rows are read by the strata its readers open (`components`, `dfs_first`): not copied into others.
            || rules.iter().any(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| prog.natives.contains_key(&a.rel))))
        {
            Some("a lattice or a native's")
        } else if prog.demand.iter().any(|d| d == rel) || prog.gates.iter().any(|(g, _)| g == rel) {
            Some("demanded or a gate")
        } else if prog.answers.contains_key(rel) || prog.answers.values().flatten().any(|a| a == rel) || prog.nullary.contains(rel) {
            Some("an answer or nullary")
        } else if super::corpus::guests::REQUESTS.contains(&rel) || super::super::rr::core_names().contains(rel) || rel.contains("__") {
            Some("the engine's (a request, core.rr's, or a session's copy)")
        } else if sccs.iter().any(|s| s.contains(rel) && (s.len() > 1 || rules.iter().any(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| a.rel == rel))))) {
            Some("recursive")
        } else if rules.iter().any(|r| r.body.iter().any(|l| !matches!(l, Literal::Atom(_) | Literal::Cmp { .. } | Literal::Nearest { .. }))) {
            Some("its rules aggregate or match")
        } else if rules.iter().any(|r| r.body.is_empty()) {
            Some("a rule of no body")
        } else {
            None
        };
        if let Some(r) = reason {
            why.insert(rel.to_owned(), r);
        }
    }
    for rel in rules_of.keys() {
        if rel.starts_with(super::super::strings::RELATION) || rel.starts_with("magic_") || rel.starts_with("done_") {
            why.insert((*rel).to_owned(), "the engine's (a text test or demand's own)");
        }
    }
    // Read inside an aggregate, by a gated negation, or by an atom with a computed argument.
    for r in &prog.rules {
        for l in &r.body {
            match l {
                Literal::Atom(a) if a.gated => {
                    why.insert(a.rel.clone(), "read by a gated negation");
                }
                Literal::Agg { atom, .. } => {
                    why.insert(atom.rel.clone(), "read inside an aggregate");
                }
                Literal::Atom(a) if a.args.iter().any(|t| matches!(t, Term::Call(..) | Term::Top)) => {
                    why.insert(a.rel.clone(), "read with a computed argument");
                }
                _ => {}
            }
        }
    }
    why
}

/// `reader` with its read at `at` of a relation whose rules are `defs` unfolded (the module's "Unfolding"): the
/// reader copies, or `None` where a negated read is not a negation of plain literals. `fresh` names the copies'
/// variables apart.
fn unfold(reader: &Rule, at: usize, defs: &[&Rule], fresh: usize) -> Option<Vec<Rule>> {
    let Literal::Atom(read) = &reader.body[at] else { return None };
    let mut n = 0usize;
    let mut apart = |def: &Rule| -> BTreeMap<String, String> {
        n += 1;
        let mut local = BTreeSet::new();
        def.body.iter().for_each(|l| local.extend(l.vars()));
        def.head.args.iter().for_each(|t| t.vars(&mut local));
        local.into_iter().map(|v| (v.clone(), format!("{v}_i{fresh}_{n}"))).collect()
    };
    if !read.neg {
        let mut out = Vec::new();
        for def in defs {
            let mut to = apart(def);
            let mut eqs: Vec<(&Term, &Term)> = Vec::new();
            let mut matched: BTreeSet<String> = BTreeSet::new();
            for (h, r) in def.head.args.iter().zip(&read.args) {
                match (h, r) {
                    (_, Term::Wild) => {}
                    // A head variable read as a variable becomes it, once.
                    (Term::Var(v), Term::Var(x)) if matched.insert(v.clone()) => {
                        to.insert(v.clone(), x.clone());
                    }
                    // A head variable named twice, or a head term against a read's: the head's equal to the read's.
                    _ => eqs.push((h, r)),
                }
            }
            let rename = |l: &Literal| rename_all(l, &to);
            // Only the head's side is the rule's, renamed: the read's terms are the reader's own variables, which may
            // share a name with one of the rule's (`expr_name(n, n, x)` read as `expr_name($t, $n, $x)`).
            let mut body: Vec<Literal> = eqs
                .iter()
                .map(|&(h, r)| match rename(&Literal::Cmp { l: h.clone(), op: "=", r: Term::Wild }) {
                    Literal::Cmp { l, op, .. } => Literal::Cmp { l, op, r: r.clone() },
                    other => other,
                })
                .collect();
            body.splice(0..0, def.body.iter().map(rename));
            let mut copy = reader.clone();
            copy.body.splice(at..=at, body);
            out.push(copy);
        }
        return Some(out);
    }
    // A negation: each rule's negation, a set of alternatives (one literal each); the copies are their product.
    let mut alternatives: Vec<Vec<Literal>> = Vec::new();
    let read_vars = read.vars();
    for def in defs {
        let heads: Vec<&str> = def.head.args.iter().map(|t| match t {
            Term::Var(v) => Some(v.as_str()),
            _ => None,
        }).collect::<Option<_>>()?;
        if heads.iter().collect::<BTreeSet<_>>().len() != heads.len() {
            return None;
        }
        let mut to: BTreeMap<String, Term> = heads.iter().zip(&read.args).map(|(h, t)| ((*h).to_owned(), t.clone())).collect();
        let mut count: BTreeMap<String, usize> = BTreeMap::new();
        for l in &def.body {
            if let Literal::Atom(a) = l {
                for t in &a.args {
                    if let Term::Var(v) = t {
                        *count.entry(v.clone()).or_default() += 1;
                    }
                }
            }
        }
        match def.body.as_slice() {
            [Literal::Atom(b)] if !b.neg => {
                // Each variable not matched to a bound term (the rule's own, or read as `_`) appears once: `_`.
                for t in &b.args {
                    if let Term::Var(v) = t {
                        let free = !matches!(to.get(v), Some(Term::Var(_) | Term::Int(_) | Term::Str(_)));
                        if free && count[v] != 1 {
                            return None;
                        }
                        if free {
                            to.insert(v.clone(), Term::Wild);
                        }
                    }
                }
                let args = b.args.iter().map(|t| subst(t, &to)).collect();
                alternatives.push(vec![Literal::Atom(Atom { args, neg: true, ..b.clone() })]);
            }
            body => {
                // Every literal over the head's variables, each bound by the read.
                let bound = |v: &String| matches!(to.get(v), Some(Term::Var(x)) if read_vars.contains(x)) || matches!(to.get(v), Some(Term::Int(_) | Term::Str(_)));
                let mut alts = Vec::new();
                for l in body {
                    if !l.vars().iter().all(bound) {
                        return None;
                    }
                    alts.push(match l {
                        Literal::Atom(a) => Literal::Atom(Atom { args: a.args.iter().map(|t| subst(t, &to)).collect(), neg: !a.neg, ..a.clone() }),
                        Literal::Cmp { l, op, r } => Literal::Cmp { l: subst(l, &to), op: negate(op)?, r: subst(r, &to) },
                        _ => return None,
                    });
                }
                alternatives.push(alts);
            }
        }
    }
    let mut copies: Vec<Vec<Literal>> = vec![Vec::new()];
    for alts in alternatives {
        copies = copies.into_iter().flat_map(|c| alts.iter().map(move |a| {
            let mut c = c.clone();
            c.push(a.clone());
            c
        })).collect();
        if copies.len() > COPIES * 4 {
            return None;
        }
    }
    Some(copies.into_iter().map(|lits| {
        let mut copy = reader.clone();
        copy.body.splice(at..=at, lits);
        copy
    }).collect())
}

/// The comparison that holds where `op` does not.
fn negate(op: &str) -> Option<&'static str> {
    Some(match op {
        "=" => "!=",
        "!=" | "<>" => "=",
        "<" => ">=",
        ">=" => "<",
        ">" => "<=",
        "<=" => ">",
        _ => return None,
    })
}

fn subst(t: &Term, to: &BTreeMap<String, Term>) -> Term {
    match t {
        Term::Var(v) => to.get(v).cloned().unwrap_or_else(|| t.clone()),
        Term::Call(f, args) => Term::Call(f.clone(), args.iter().map(|a| subst(a, to)).collect()),
        t => t.clone(),
    }
}

/// `l` with each variable `to` names renamed at once.
fn rename_all(l: &Literal, to: &BTreeMap<String, String>) -> Literal {
    fn term(t: &Term, to: &BTreeMap<String, String>) -> Term {
        match t {
            Term::Var(v) => Term::Var(to.get(v).cloned().unwrap_or_else(|| v.clone())),
            Term::Call(f, args) => Term::Call(f.clone(), args.iter().map(|a| term(a, to)).collect()),
            t => t.clone(),
        }
    }
    let name = |v: &String| to.get(v).cloned().unwrap_or_else(|| v.clone());
    match l {
        Literal::Atom(a) => Literal::Atom(Atom { args: a.args.iter().map(|t| term(t, to)).collect(), ..a.clone() }),
        Literal::Cmp { l, op, r } => Literal::Cmp { l: term(l, to), op: *op, r: term(r, to) },
        Literal::Nearest { n, a, kindset } => Literal::Nearest { n: name(n), a: name(a), kindset: kindset.clone() },
        other => other.clone(),
    }
}

/// What `estimate` put into the planner's statistics, to take back.
struct Inserted {
    card: Vec<String>,
    ndv: Vec<(String, usize)>,
}

/// Each derived relation's estimated (rows, work to build it), in evaluation order; each estimate is the planner's
/// statistics while its readers are priced (`unestimate` takes them back). A relation an earlier chunk measured keeps
/// its measured rows.
fn estimate(prog: &Program, model: &mut Model, rules_of: &BTreeMap<&str, Vec<&Rule>>, sccs: &[BTreeSet<String>]) -> (BTreeMap<String, (f64, f64)>, Inserted) {
    let mut out = BTreeMap::new();
    let mut inserted = Inserted { card: Vec::new(), ndv: Vec::new() };
    for scc in sccs {
        for rel in scc {
            let Some(rules) = rules_of.get(rel.as_str()) else { continue };
            let (mut rows, mut work) = (0.0, 0.0);
            for r in rules {
                if let Ok((_, w, n)) = model.plan_rule(prog, &r.head, &r.body, None) {
                    rows += n;
                    work += w;
                }
            }
            let measured = model.stats.card.get(rel).copied();
            let rows = measured.unwrap_or(rows);
            out.insert(rel.clone(), (rows, work));
            if measured.is_none() {
                model.stats.card.insert(rel.clone(), rows.max(1.0));
                inserted.card.push(rel.clone());
                // A column's distinct values: at most the rows, and at most those of the body column that binds it.
                let width = prog.decls.get(rel).map_or(0, Vec::len);
                for c in 0..width {
                    if model.ndv.contains_key(&(rel.clone(), c)) {
                        continue;
                    }
                    let mut ndv = rows.max(1.0);
                    for r in rules {
                        let Some(Term::Var(v)) = r.head.args.get(c) else { continue };
                        for l in &r.body {
                            if let Literal::Atom(a) = l
                                && !a.neg
                                && let Some(k) = a.args.iter().position(|t| matches!(t, Term::Var(x) if x == v))
                            {
                                if let Some(&d) = model.ndv.get(&(a.rel.clone(), k)) {
                                    ndv = ndv.min(d);
                                }
                            }
                        }
                    }
                    model.ndv.insert((rel.clone(), c), ndv.max(1.0));
                    inserted.ndv.push((rel.clone(), c));
                }
            }
        }
    }
    (out, inserted)
}

fn unestimate(model: &mut Model, inserted: Inserted) {
    for rel in inserted.card {
        model.stats.card.remove(&rel);
    }
    for key in inserted.ndv {
        model.ndv.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(text: &str) -> Rule {
        let prog = super::super::super::rr::parse(&format!(
            ".decl a(x: node)\n.decl b(x: node, y: node)\n.decl c(x: node, y: node)\n.decl r(x: node)\n.decl o(x: node)\n.input a\n.input b\n.output o\n{text}"
        ))
        .unwrap();
        prog.rules.into_iter().next().unwrap()
    }

    /// A positive read is one copy per rule; a negated one of a single atom with its own variable is that atom
    /// negated with `_`; a negated one of literals over the head is their disjunction; a join is refused.
    #[test]
    fn reads_unfold_into_their_readers() {
        let reader = rule("o(x) :- a(x), r(x).");
        let def = rule("r(z) :- b(z, w), a(w).");
        let copies = unfold(&reader, 1, &[&def], 1).unwrap();
        assert_eq!(copies.len(), 1);
        assert_eq!(copies[0].to_string(), "o($x) :- a($x), b($x, $w_i1_1), a($w_i1_1)");
        let negated = rule("o(x) :- a(x), !r(x).");
        let one = rule("r(z) :- b(z, w).");
        assert_eq!(unfold(&negated, 1, &[&one], 1).unwrap()[0].to_string(), "o($x) :- a($x), !b($x, _)");
        let over_head = rule("r(z) :- a(z), z != 3.");
        let two: Vec<String> = unfold(&negated, 1, &[&over_head], 1).unwrap().iter().map(|r| r.to_string()).collect();
        assert_eq!(two, vec!["o($x) :- a($x), !a($x)", "o($x) :- a($x), $x = 3"]);
        assert!(unfold(&negated, 1, &[&def], 1).is_none());
        // A head variable named twice equates the read's terms, the reader's own variables as they are, though one of
        // them has the name of the rule's.
        let reader = rule("o(n) :- a(t), c(t, n).");
        let twice = rule("c(n, n) :- a(n).");
        assert_eq!(unfold(&reader, 1, &[&twice], 1).unwrap()[0].to_string(), "o($n) :- a($t), a($t), $t = $n");
    }

    /// A reader in a recursion is priced by its delta plans: inlined into `reach`'s recursive rule, `edge`'s join
    /// is made again for each new reach row, though priced whole the copy joins `imp` and `md` first and looks as
    /// cheap as reading `edge` (`rr_cx_edge`, priced 7.8 M work cheaper inlined; 277 s of calls_cx when it was).
    #[test]
    fn a_relation_read_in_a_recursion_is_priced_by_its_delta_plans() {
        use super::super::{Inputs, Stats};
        use std::collections::{HashMap, HashSet};
        let prog = super::super::super::rr::parse(
            ".decl imp(d: number, q: number)\n.decl md(e: number, q: number)\n.decl own(d: number)\n.input imp\n.input md\n.input own\n\
             .decl edge(d: number, e: number)\nedge(d, e) :- imp(d, q), md(e, q).\n\
             .decl reach(d: number, e: number)\nreach(d, d) :- own(d).\nreach(d, f) :- reach(d, e), edge(e, f).\n.output reach\n",
        )
        .unwrap();
        let inputs = Inputs { facts: "main.".to_owned(), stats: "main.".to_owned(), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
        let derived: HashSet<String> = prog.rules.iter().map(|r| r.head.rel.clone()).collect();
        let mut model = Model { analysis: Default::default(), stats: Stats::empty(&prog), ndv: HashMap::new(), docof: HashMap::new(), view_src: HashMap::new(), inputs, derived };
        model.stats.nodes = 450_000.0;
        // Measured rows (an earlier chunk's), then distinct values by column.
        for (rel, rows, ndv) in [
            ("imp", 40_000.0, vec![4_000.0, 4_000.0]),
            ("md", 4_000.0, vec![4_000.0, 4_000.0]),
            ("own", 4_000.0, vec![4_000.0]),
            ("edge", 40_000.0, vec![4_000.0, 4_000.0]),
            ("reach", 100_000.0, vec![4_000.0, 4_000.0]),
        ] {
            model.stats.card.insert(rel.to_owned(), rows);
            for (c, n) in ndv.into_iter().enumerate() {
                model.ndv.insert((rel.to_owned(), c), n);
                model.stats.fan.insert((rel.to_owned(), c), rows / n);
            }
        }
        let chosen = choose(&prog, &mut model).unwrap();
        assert!(chosen.is_none(), "edge inlined into reach's recursion");
    }

    /// `.decl r(…) inline` and its kin are read as Soufflé spells them; two that disagree are an error.
    #[test]
    fn decl_qualifiers_are_read() {
        let prog = super::super::super::rr::parse(".decl r(x: node) inline\n.decl s(x: node) no_magic\n.decl t(x: node) no_inline magic\n").unwrap();
        assert!(prog.qualified("r", "inline") && prog.qualified("s", "no_magic") && prog.qualified("t", "no_inline") && prog.qualified("t", "magic"));
        assert!(!prog.qualified("r", "magic"));
        assert!(super::super::super::rr::parse(".decl r(x: node) inline no_inline\n").is_err());
    }
}
