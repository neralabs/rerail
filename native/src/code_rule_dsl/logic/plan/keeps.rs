//! What a relation keeps (decision 0097): stored, or recomputed in its readers.
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
//! and inlining makes at most `COPIES` reader copies.
//!
//! **Observed in place of estimated.** The native strata record what each rule's steps produced and each relation
//! held (`observe`, kept for the corpus run beside its plans): a later chunk or part prices a relation's rows and
//! build, and each reader term (the reader as it is, each copy), from what an earlier one observed, per node of the
//! corpus. A term not observed is the plan's estimate scaled by observed over estimated for the reader's terms that
//! were. `RERAIL_OBSERVED_STEPS=on` prices from observations (off by default until its regression is fixed). Stored always: outputs, inputs, lattices, relations with facts, natives and what they read,
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
//! The profile names each choice and why (`inline or store (decision 0097)`).

use super::super::front::{Atom, Literal, Program, Rule, Term, strata};
use super::Model;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// Inlined when the readers' added work is under this share of the relation's cost stored (decision 0097, "Built":
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
        super::super::profile::decision("inline or store (decision 0097)", || "RERAIL_INLINE=off: every relation stored, `.decl … inline` too".to_owned());
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
    // Estimated rows and work of every derived relation, in evaluation order; observed rows where an earlier chunk
    // measured them. The estimates are the planner's while it prices the readers, then taken back.
    let (estimates, inserted) = estimate(prog, model, &rules_of, &sccs);
    let mut inline: BTreeSet<String> = BTreeSet::new();
    let mut lines: Vec<String> = Vec::new();
    let mut refused_inline: Vec<String> = Vec::new();
    // `RERAIL_INLINE_ONLY=r,…` (the lab's comparison): those relations inlined where they can be, every other stored.
    let only: Option<Vec<String>> = std::env::var("RERAIL_INLINE_ONLY").ok().map(|v| v.split(',').map(str::to_owned).collect());
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
        // Each term observed where an earlier chunk or part ran that rule (`Observed`), else the plan's estimate
        // calibrated by the reader's: scaled by observed over estimated for the reader's terms that were observed
        // (its own rule when the relation was stored, its copies when it was inlined). Unscaled, an observed reader
        // stored (every wave's reads) set against its copies' first estimates read as a saving it was not.
        let nodes = model.stats.nodes;
        let (mut terms, mut seen) = (0usize, 0usize);
        let mut added = 0.0;
        let mut priced = true;
        for &(ri, i) in &reads {
            let reader = &prog.rules[ri];
            let Ok((_, own_est, _)) = model.plan_rule(prog, &reader.head, &reader.body, None) else {
                priced = false;
                break;
            };
            let mut copies: Vec<(f64, Option<f64>)> = Vec::new();
            for copy in unfold(reader, i, rules, 0).unwrap_or_default() {
                match model.plan_rule(prog, &copy.head, &copy.body, None) {
                    Ok((_, w, _)) => copies.push((w, observed_rule(&copy, nodes))),
                    Err(_) => priced = false,
                }
            }
            let own_obs = observed_rule(reader, nodes);
            let (o, e) = own_obs.map_or((0.0, 0.0), |w| (w, own_est.max(1.0)));
            let (o, e) = copies.iter().filter_map(|(e, w)| Some((*w)?).map(|w| (w, e.max(1.0)))).fold((o, e), |(a, b), (w, e)| (a + w, b + e));
            let scale = if e > 0.0 { o / e } else { 1.0 };
            terms += 1 + copies.len();
            seen += usize::from(own_obs.is_some()) + copies.iter().filter(|c| c.1.is_some()).count();
            added += copies.iter().map(|(e, w)| w.unwrap_or(scale * e)).sum::<f64>() - own_obs.unwrap_or(scale * own_est);
        }
        let built_seen = rules.iter().all(|r| observed_rule(r, nodes).is_some());
        let rows_seen = observed_rows(rel, nodes).is_some();
        let observed = if seen + usize::from(built_seen) + usize::from(rows_seen) == 0 {
            String::new()
        } else {
            format!(" (observed: {seen} of {terms} reader terms{}{})", if built_seen { ", build" } else { "" }, if rows_seen { ", rows" } else { "" })
        };
        if !priced {
            lines.push(format!("{rel}: stored, a reader copy does not plan"));
            continue;
        }
        let stored = work + M * rows;
        if added < ALPHA * stored {
            lines.push(format!("{rel}: inlined, readers add {added:.0} work < {ALPHA} x (build {work:.0} + {M} x {rows:.0} rows){observed}"));
            inline.insert(rel.to_owned());
        } else {
            lines.push(format!("{rel}: stored, readers would add {added:.0} work >= {ALPHA} x (build {work:.0} + {M} x {rows:.0} rows){observed}"));
        }
    }
    if super::super::profile::on() {
        for line in factorizable(prog, &rules_of, &estimates, model) {
            super::super::profile::decision("factorized store (decision 0097)", || line);
        }
    }
    unestimate(model, inserted);
    for line in &lines {
        super::super::profile::decision("inline or store (decision 0097)", || line.clone());
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
    // A copy's variables are named apart by the pass (each rule unfolds one read a pass), so a first-level copy is
    // the one priced above, and an earlier chunk's observation of it is found by its text.
    for pass in 0..8 {
        let bodies: BTreeMap<String, Vec<Rule>> = inline
            .iter()
            .map(|r| (r.clone(), rules.iter().filter(|x| x.head.rel == *r).cloned().collect()))
            .collect();
        let mut changed = false;
        let mut next = Vec::with_capacity(rules.len());
        for rule in rules {
            let at = rule.body.iter().position(|l| l.atom().is_some_and(|a| inline.contains(&a.rel)));
            match at {
                Some(i) => {
                    let rel = rule.body[i].atom().map(|a| a.rel.clone()).unwrap_or_default();
                    let defs: Vec<&Rule> = bodies[&rel].iter().collect();
                    match unfold(&rule, i, &defs, pass) {
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

/// The relations that cross the chunk boundary (the local part's outputs: its exports and findings) stored as the
/// flat join of parts that meet only at a variable they project away: `fl853_p4_outer(f, x) :- rr_inside_ks5(f, p),
/// bound_in(p, x)` holds 777 k rows, its two parts about 100 k. Stored factorized, the parts and their key, a reader
/// would join them where it binds its own columns. Not built (decision 0097, "Factorized store"): this names each
/// such relation with its flat rows (observed where an earlier chunk measured them) and its parts' estimated rows,
/// where the flat rows are over twice the parts'.
fn factorizable(prog: &Program, rules_of: &BTreeMap<&str, Vec<&Rule>>, estimates: &BTreeMap<String, (f64, f64)>, model: &Model) -> Vec<String> {
    let mut out = Vec::new();
    for rel in &prog.outputs {
        let Some(rules) = rules_of.get(rel.as_str()) else { continue };
        let nodes = model.stats.nodes;
        let (flat, how) = match observed_rows(rel, nodes) {
            Some(n) => (n, "observed"),
            None => (estimates.get(rel.as_str()).map_or(0.0, |e| e.0), "estimated"),
        };
        if flat < SMALL {
            continue;
        }
        let mut parts_rows = 0.0;
        let mut shown: Vec<String> = Vec::new();
        let mut every = true;
        for rule in rules {
            match factors(rule) {
                Some((key, parts)) => {
                    let mut texts = Vec::new();
                    for part in &parts {
                        let mut vars: BTreeSet<String> = BTreeSet::new();
                        part.iter().for_each(|l| vars.extend(l.vars()));
                        let head = Atom { rel: format!("{rel}_part"), args: vars.into_iter().map(Term::Var).collect(), neg: false, gated: false };
                        let rows = model.plan_rule(prog, &head, part, None).map_or(f64::INFINITY, |p| p.2);
                        parts_rows += rows;
                        texts.push(format!("{} [{rows:.0}]", part.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")));
                    }
                    shown.push(format!("by ${key}: {}", texts.join(" | ")));
                }
                None => every = false,
            }
        }
        if every && !shown.is_empty() && flat > 2.0 * parts_rows {
            out.push(format!("{rel}: {flat:.0} rows stored flat ({how}), {parts_rows:.0} as parts (estimated); {}", shown.join("; ")));
        }
    }
    out
}

/// `rule`'s body split at a variable `key` the head does not name: the parts that meet only at `key`, each naming a
/// head variable, with each comparison and negation in the part whose variables it reads. `None` where no variable
/// splits it so, or it holds anything but atoms and comparisons.
fn factors(rule: &Rule) -> Option<(String, Vec<Vec<Literal>>)> {
    if !rule.body.iter().all(|l| matches!(l, Literal::Atom(_) | Literal::Cmp { .. })) {
        return None;
    }
    let mut head: BTreeSet<String> = BTreeSet::new();
    rule.head.args.iter().for_each(|t| t.vars(&mut head));
    let positive: Vec<usize> = (0..rule.body.len()).filter(|&i| matches!(&rule.body[i], Literal::Atom(a) if !a.neg)).collect();
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for &i in &positive {
        for v in rule.body[i].vars() {
            if !head.contains(&v) && positive.iter().filter(|&&j| rule.body[j].vars().contains(&v)).count() >= 2 {
                keys.insert(v);
            }
        }
    }
    for key in keys {
        // Components over every literal, joined by the variables they share other than the key.
        let mut comps: Vec<(Vec<usize>, BTreeSet<String>)> = Vec::new();
        for (i, l) in rule.body.iter().enumerate() {
            let mut vars = l.vars();
            vars.remove(&key);
            let (joined, rest): (Vec<_>, Vec<_>) = comps.into_iter().partition(|(_, v)| !v.is_disjoint(&vars));
            let mut all = (vec![i], vars);
            for (ls, vs) in joined {
                all.0.extend(ls);
                all.1.extend(vs);
            }
            comps = rest;
            comps.push(all);
        }
        // A literal of the key alone (`$p < …`) goes with the first part that reads the key.
        let (lone, mut parts): (Vec<_>, Vec<_>) = comps.into_iter().partition(|(_, v)| v.is_empty());
        if parts.len() < 2 || !parts.iter().all(|(ls, v)| !v.is_disjoint(&head) && ls.iter().any(|&i| positive.contains(&i))) {
            continue;
        }
        for (ls, _) in lone {
            parts[0].0.extend(ls);
        }
        let parts = parts.into_iter().map(|(mut ls, _)| {
            ls.sort_unstable();
            ls.into_iter().map(|i| rule.body[i].clone()).collect()
        }).collect();
        return Some((key, parts));
    }
    None
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
            let mut body: Vec<Literal> = Vec::new();
            let mut matched: BTreeSet<String> = BTreeSet::new();
            for (h, r) in def.head.args.iter().zip(&read.args) {
                match (h, r) {
                    (_, Term::Wild) => {}
                    // A head variable read as a variable becomes it, once.
                    (Term::Var(v), Term::Var(x)) if matched.insert(v.clone()) => {
                        to.insert(v.clone(), x.clone());
                    }
                    // A head variable named twice, or a head term against a read's: the head's equal to the read's.
                    _ => body.push(Literal::Cmp { l: h.clone(), op: "=", r: r.clone() }),
                }
            }
            let rename = |l: &Literal| rename_all(l, &to);
            let mut body: Vec<Literal> = body.iter().map(rename).collect();
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

thread_local! {
    /// What the native strata observed on this thread during a corpus run (`observing`): kept across its chunks and
    /// parts, so a later chunk's choice prices each relation from the rows its join steps produced.
    static OBSERVED: RefCell<Option<Observed>> = const { RefCell::new(None) };
}

/// Observed join work and rows, each with the corpus nodes of the runs that observed it (summed), so a later chunk
/// reads a rate per node: per rule (`obs_rule`: its head and body), the rows into and out of every step of every probe
/// of it (the planner's units of work: `native.tsv`'s `steps_in_out`); per relation, its rows when its stratum ended.
#[derive(Default)]
struct Observed {
    rules: rustc_hash::FxHashMap<u64, (f64, f64)>,
    rels: rustc_hash::FxHashMap<String, (f64, f64)>,
}

/// Observations kept while held (a corpus run, `KeepPlans`); none outside one, and none under
/// `RERAIL_OBSERVED_STEPS=on` (off by default: every choice from the plan's estimates).
pub(super) fn observing(on: bool) {
    // Off by default: with two chunks a relation stored by its estimate is repriced from its observation while its
    // inlined copies are still estimated, and inlining then costs more (0097, observed counts). `on` turns it on.
    let on = on && std::env::var("RERAIL_OBSERVED_STEPS").is_ok_and(|v| v == "on");
    OBSERVED.with(|o| *o.borrow_mut() = on.then(Observed::default));
}

/// A stratum's native run: per rule, the (in, out) rows of each step of each of its probes, over a corpus of `nodes`.
pub(super) fn observe(rules: &[(u64, &[(u64, u64)])], rels: &[(&str, f64)], nodes: f64) {
    OBSERVED.with(|o| {
        let mut o = o.borrow_mut();
        let Some(o) = o.as_mut() else { return };
        let mut per: BTreeMap<u64, f64> = BTreeMap::new();
        for (rule, steps) in rules {
            *per.entry(*rule).or_default() += steps.iter().map(|(a, b)| (a + b) as f64).sum::<f64>();
        }
        for (rule, work) in per {
            let e = o.rules.entry(rule).or_default();
            e.0 += work;
            e.1 += nodes.max(1.0);
        }
        for (rel, rows) in rels {
            let e = o.rels.entry((*rel).to_owned()).or_default();
            e.0 += rows;
            e.1 += nodes.max(1.0);
        }
    });
}

/// `rule`'s observed work, scaled to a corpus of `nodes`.
fn observed_rule(rule: &Rule, nodes: f64) -> Option<f64> {
    let key = super::obs_rule(&rule.head, &rule.body);
    OBSERVED.with(|o| o.borrow().as_ref()?.rules.get(&key).map(|(w, n)| w / n * nodes.max(1.0)))
}

/// `rel`'s observed rows, scaled to a corpus of `nodes`.
fn observed_rows(rel: &str, nodes: f64) -> Option<f64> {
    OBSERVED.with(|o| o.borrow().as_ref()?.rels.get(rel).map(|(r, n)| r / n * nodes.max(1.0)))
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
            // Observed on an earlier chunk or part of this run (`Observed`), per node of the corpus: its rows when
            // it was stored, and its build's work when every rule of it ran.
            let nodes = model.stats.nodes;
            let seen_work: Option<f64> = rules.iter().map(|r| observed_rule(r, nodes)).sum();
            let work = seen_work.unwrap_or(work);
            let measured = model.stats.card.get(rel).copied().or_else(|| observed_rows(rel, nodes));
            let rows = measured.unwrap_or(rows);
            out.insert(rel.clone(), (rows, work));
            if !model.stats.card.contains_key(rel) {
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
            ".decl a(x: node)\n.decl b(x: node, y: node)\n.decl r(x: node)\n.decl o(x: node)\n.input a\n.input b\n.output o\n{text}"
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
    }

    /// A body that meets only at a variable the head projects away splits there; a join through a head variable does
    /// not.
    #[test]
    fn products_split_at_their_key() {
        let r = rule("o(x) :- a(x), b(x, y), b(y, z), a(z), y != 3.");
        assert!(factors(&r).is_none(), "the head names only x");
        let prog = super::super::super::rr::parse(
            ".decl a(x: node)\n.decl b(x: node, y: node)\n.decl o(x: node, z: node)\n.input a\n.input b\n.output o\no(x, z) :- b(x, p), p != 3, b(p, z), a(z).\n",
        )
        .unwrap();
        let (key, parts) = factors(&prog.rules[0]).unwrap();
        assert_eq!(key, "p");
        let shown: Vec<String> = parts.iter().map(|p| p.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")).collect();
        assert_eq!(shown, ["b($x, $p), $p != 3", "b($p, $z), a($z)"]);
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
