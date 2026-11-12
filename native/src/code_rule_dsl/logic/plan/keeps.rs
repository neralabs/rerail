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
//! and inlining makes at most `COPIES` reader copies.
//!
//! **Observed in place of estimated.** The native strata record what each rule's steps produced, each step pattern's
//! selectivity, and each relation's rows and columns (`observe`, kept for the corpus run beside its plans). A later
//! chunk or part prices each candidate twice: by the estimates alone, and with the relations' measured columns, each
//! plan's steps re-priced by observed selectivities (`calibrated`), and the side that ran believed over the one that
//! did not (`weigh`). The estimates' choice stands unless the observations are evidence against it (`settle`).
//! `RERAIL_OBSERVED_STEPS=off` prices from estimates alone, for comparison.
//!
//! Stored always: outputs, inputs, lattices, relations with facts, natives and what they read,
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
use std::cell::RefCell;
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
    let _span = super::super::profile::span(|| "inline or store".into());
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
    // Read before the cone (below) is chosen.
    let only: Option<Vec<String>> = std::env::var("RERAIL_INLINE_ONLY").ok().map(|v| v.split(',').map(str::to_owned).collect());
    // The estimates below plan rules, which a maintained program has about ten times of. They price only a relation
    // that may be inlined: none (a session's part stores its relations, decision 0099), none are made.
    let open = |rel: &str| !prog.qualified(rel, "no_inline") && !reasons.contains_key(rel) && readers.contains_key(rel);
    if only.is_none() && !rules_of.keys().any(|rel| prog.qualified(rel, "inline") || open(rel)) {
        let stored = rules_of.keys().filter(|rel| prog.qualified(rel, "no_inline")).count();
        super::super::profile::decision("inline or store (decision 0099)", || format!("no relation may be inlined ({stored} `.decl … no_inline`): nothing estimated"));
        return Ok(None);
    }
    // The relations whose estimates the pricing reads: each candidate (a relation the loop below may price), what its
    // readers read, and upstream of those every relation their rules read; the outputs too when the profile names
    // the factorizable ones. Estimating every derived relation planned every rule of the program (10.7 k rules on
    // the pack's), most of them for relations nothing prices.
    let mut need: BTreeSet<&str> = BTreeSet::new();
    let mut candidates: BTreeSet<&str> = BTreeSet::new();
    if only.is_none() {
        for (rel, rules) in &rules_of {
            if reasons.contains_key(*rel) || rules.len() > K || prog.qualified(rel, "inline") || prog.qualified(rel, "no_inline") {
                continue;
            }
            let Some(reads) = readers.get(rel) else { continue };
            need.insert(rel);
            candidates.insert(rel);
            for &(ri, _) in reads {
                need.extend(prog.rules[ri].body.iter().filter_map(Literal::atom).map(|a| a.rel.as_str()));
            }
        }
    }
    if super::super::profile::on() {
        need.extend(prog.outputs.iter().map(String::as_str));
    }
    let mut work: Vec<&str> = need.iter().copied().collect();
    while let Some(rel) = work.pop() {
        for r in rules_of.get(rel).into_iter().flatten() {
            for a in r.body.iter().filter_map(Literal::atom) {
                if rules_of.contains_key(a.rel.as_str()) && need.insert(a.rel.as_str()) {
                    work.push(a.rel.as_str());
                }
            }
        }
    }
    // Each candidate priced twice: by the plan's estimates alone (what `RERAIL_OBSERVED_STEPS=off` decides), and,
    // once an earlier chunk or part observed anything, with its observations (`calibrated`, `weigh`). The choice is
    // the estimates' unless the observations are evidence against it (`settle`).
    let held = OBSERVED.with(|o| o.borrow_mut().as_mut().map(std::mem::take));
    let observed_any = held.as_ref().is_some_and(|o| !o.rules.is_empty());
    // The statistics the cone's plans read, if they are made when first planned with (`Model::resolve`); and its
    // readers' (the pricing below plans them).
    let read: Vec<&str> = need.iter().flat_map(|rel| rules_of.get(rel).into_iter().flatten()).flat_map(|r| r.body.iter().filter_map(Literal::atom)).map(|a| a.rel.as_str()).collect();
    model.resolve(prog, read).map_err(|e| e.to_string())?;
    let (estimates, inserted) = estimate(prog, model, &rules_of, &sccs, &need, &candidates);
    let by_estimates = price_all(prog, model, &rules_of, &readers, &reasons, &estimates, &need, &sccs);
    OBSERVED.with(|o| {
        if let (Some(o), Some(h)) = (o.borrow_mut().as_mut(), held) {
            *o = h;
        }
    });
    // The observations can change a choice only where the stored side ran (every reader as it is): a veto of an
    // inlining, or, where the inlined side ran too, a revert. Only those are priced again: over the estimates, each
    // relation an earlier chunk or part measured takes its measured rows and columns (`overlay`), and each build is
    // re-priced with the steps' observed selectivities.
    let mut again: BTreeSet<&str> = BTreeSet::new();
    if observed_any {
        for (rel, p) in &by_estimates {
            let inlined = matches!(p, Priced::Weighed(v) if v.inline);
            let reads = readers.get(rel.as_str()).map_or(&[][..], Vec::as_slice);
            let stored_ran = !reads.is_empty() && reads.iter().all(|&(ri, _)| observed_rule(&prog.rules[ri], 1.0).is_some());
            let copies_ran = reads.iter().all(|&(ri, _)| observed_copies(rel, &prog.rules[ri], 1.0).is_some());
            if stored_ran && (inlined || copies_ran) {
                again.insert(rules_of.get_key_value(rel.as_str()).map_or("", |(k, _)| *k));
            }
        }
        again.remove("");
    }
    let by_observed = if again.is_empty() {
        BTreeMap::new()
    } else {
        let saved = overlay(prog, model, need.iter().copied());
        let nodes = model.stats.nodes;
        let again_estimates: BTreeMap<String, (f64, f64, f64)> = again
            .iter()
            .filter_map(|rel| {
                let (rows, work, planned) = estimates.get(*rel).copied()?;
                let build = rules_of[rel].iter().map(|r| calibrated(prog, model, r)).sum::<Option<f64>>().unwrap_or(work);
                Some(((*rel).to_owned(), (observed_rows(rel, nodes).unwrap_or(rows), build, planned)))
            })
            .collect();
        let got = price_all(prog, model, &rules_of, &readers, &reasons, &again_estimates, &again, &sccs);
        restore(model, saved);
        got
    };
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
        let (to_inline, line) = settle(rel, by_estimates.get(rel), by_observed.get(rel));
        if let Some(line) = line {
            lines.push(line);
        }
        if to_inline {
            inline.insert(rel.to_owned());
        }
    }
    if super::super::profile::on() {
        for line in factorizable(prog, &rules_of, &estimates, model) {
            super::super::profile::decision("factorized store (decision 0099): stored flat, would shrink as parts", || line);
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
    // Each rule's origin: the program's rule it was unfolded from, and the relations unfolded into it, so what a
    // copy does when it runs is the inlined side's observation of that reader (`copied`).
    let (rules, origin) = unfold_reads(prog.rules.clone(), &inline);
    let unfinished: BTreeSet<String> = rules
        .iter()
        .filter(|r| !inline.contains(r.head.rel.as_str()))
        .flat_map(|r| r.body.iter().filter_map(Literal::atom))
        .filter(|a| inline.contains(a.rel.as_str()))
        .map(|a| a.rel.to_string())
        .collect();
    // An inlined relation some reader still reads (a chain deeper than the passes) keeps its rules.
    let mut out = prog.clone();
    let kept: Vec<(Rule, (usize, BTreeSet<String>))> = rules.into_iter().zip(origin).filter(|(r, _)| !inline.contains(r.head.rel.as_str()) || unfinished.contains(r.head.rel.as_str())).collect();
    copied(kept.iter().filter(|(_, (_, into))| !into.is_empty()).map(|(r, (o, into))| (super::obs_rule(&r.head, &r.body), super::obs_rule(&prog.rules[*o].head, &prog.rules[*o].body), into)));
    out.rules = kept.into_iter().map(|(r, _)| r).collect();
    let done: Vec<String> = inline.into_iter().filter(|r| !unfinished.contains(r)).collect();
    if done.is_empty() {
        return Ok(None);
    }
    Ok(Some((out, done)))
}

/// `rules` with the reads of the `inline` relations unfolded into their readers (`unfold`), and each rule's origin:
/// the index in `rules` of the rule it was unfolded from, and the relations unfolded into it. A relation inlined into
/// another inlined one is unfolded into its copies in turn; one inlined relation reading another is unfolded first,
/// readers after, so a pass takes one level. A chain deeper than the passes leaves its reads (the caller stores the
/// rest). A pass unfolds a rule's first such read, so a rule reading two is unfolded over two passes: a copy's
/// variables are named by its pass, or the second read's copy takes the first's names (`layer_num(gf, f),
/// layer_num(gt, t)` would join the two numbers' digits as one). A first pass's copy is so named as the one priced
/// (`unfold(…, 0)`), and an earlier chunk's observation of it is found by its text.
fn unfold_reads(mut rules: Vec<Rule>, inline: &BTreeSet<String>) -> (Vec<Rule>, Vec<(usize, BTreeSet<String>)>) {
    let mut origin: Vec<(usize, BTreeSet<String>)> = (0..rules.len()).map(|i| (i, BTreeSet::new())).collect();
    for pass in 0..8 {
        let bodies: BTreeMap<String, Vec<Rule>> = inline
            .iter()
            .map(|r| (r.clone(), rules.iter().filter(|x| x.head.rel == *r).cloned().collect()))
            .collect();
        let mut changed = false;
        let mut next = Vec::with_capacity(rules.len());
        let mut next_origin = Vec::with_capacity(rules.len());
        for (rule, from) in rules.into_iter().zip(origin) {
            let at = rule.body.iter().position(|l| l.atom().is_some_and(|a| inline.contains(a.rel.as_str())));
            match at {
                Some(i) => {
                    let rel = rule.body[i].atom().map(|a| a.rel.clone()).unwrap_or_default();
                    let defs: Vec<&Rule> = bodies[rel.as_str()].iter().collect();
                    match unfold(&rule, i, &defs, pass) {
                        Some(copies) => {
                            let mut into = from.1.clone();
                            into.insert(rel.to_string());
                            next_origin.extend(copies.iter().map(|_| (from.0, into.clone())));
                            next.extend(copies);
                            changed = true;
                        }
                        None => {
                            next.push(rule);
                            next_origin.push(from);
                        }
                    }
                }
                None => {
                    next.push(rule);
                    next_origin.push(from);
                }
            }
        }
        rules = next;
        origin = next_origin;
        if !changed {
            break;
        }
    }
    (rules, origin)
}

/// A candidate's price (`price_all`): too small to price, a reader copy that does not plan, or weighed.
enum Priced {
    Small(f64, f64),
    Unplanned,
    Weighed(Verdict),
}

/// Prices every relation the choice may inline (`need`'s candidates), with the observations kept now (none while
/// they are held aside): each reader term (the reader as it is, its copies), the build and the rows, weighed.
fn price_all(
    prog: &Program,
    model: &Model,
    rules_of: &BTreeMap<&str, Vec<&Rule>>,
    readers: &BTreeMap<&str, Vec<(usize, usize)>>,
    reasons: &BTreeMap<String, &'static str>,
    estimates: &BTreeMap<String, (f64, f64, f64)>,
    need: &BTreeSet<&str>,
    sccs: &[BTreeSet<String>],
) -> BTreeMap<String, Priced> {
    let nodes = model.stats.nodes;
    // Each reader's own price (by its delta atom read first, `None`: whole), planned once however many of the
    // relations it reads are priced.
    let mut own_price: BTreeMap<(usize, Option<usize>), Option<f64>> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for (rel, rules) in rules_of {
        let rel = *rel;
        if !need.contains(rel) || reasons.contains_key(rel) || rules.len() > K || prog.qualified(rel, "inline") || prog.qualified(rel, "no_inline") {
            continue;
        }
        let Some(reads) = readers.get(rel) else { continue };
        let Some(copies) = reads.iter().map(|&(ri, i)| unfold(&prog.rules[ri], i, rules, 0).map(|c| c.len())).sum::<Option<usize>>() else { continue };
        if copies > COPIES {
            continue;
        }
        let (rows, work, planned) = estimates.get(rel).copied().unwrap_or((0.0, 0.0, 0.0));
        // Small by its plan and by what an earlier chunk measured: stored unpriced.
        if rows < SMALL && planned < SMALL {
            out.insert(rel.to_owned(), Priced::Small(rows, planned));
            continue;
        }
        // Each reader term priced: the reader as it is, and its copies together, each by its plan (`calibrated`)
        // and by what it did where it ran (the copies: what the rules unfolded from that reader did, `copied`).
        let mut own: Vec<(f64, Option<f64>)> = Vec::new();
        let mut inl: Vec<(f64, Option<f64>)> = Vec::new();
        let mut priced = true;
        for &(ri, i) in reads {
            let reader = &prog.rules[ri];
            let copies = unfold(reader, i, rules, 0).unwrap_or_default();
            // A reader in a recursion runs as its delta plans, one per recursive atom read first, over each wave's
            // new rows: priced so, each copy with the same atom first. Its whole plan may start from the inlined
            // relation's own inputs, which no delta plan can: `rr_cx_reach(d, f) :- rr_cx_reach(d, e),
            // rr_cx_edge(e, f)` priced whole joined `rr_cx_edge`'s imports and modules first, and inlining it looked
            // 7.8 M work cheaper; run, each new reach row scanned every document's module (277 s of calls_cx).
            let scc = sccs.iter().find(|s| s.contains(reader.head.rel.as_str()));
            let deltas: Vec<usize> = (0..reader.body.len())
                .filter(|&k| k != i && reader.body[k].atom().is_some_and(|a| !a.neg && scc.is_some_and(|s| s.contains(a.rel.as_str()))))
                .collect();
            let firsts: Vec<Option<usize>> = if deltas.is_empty() { vec![None] } else { deltas.iter().map(|&k| Some(k)).collect() };
            let (mut own_est, mut est) = (0.0, 0.0);
            for first in firsts {
                let Some(o) = *own_price.entry((ri, first)).or_insert_with(|| calibrated_at(prog, model, reader, first)) else {
                    priced = false;
                    break;
                };
                own_est += o;
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
                    match calibrated_at(prog, model, copy, at) {
                        Some(w) => est += w,
                        None => priced = false,
                    }
                }
            }
            if !priced {
                break;
            }
            own.push((own_est, observed_rule(reader, nodes)));
            inl.push((est, observed_copies(rel, reader, nodes)));
        }
        if !priced {
            out.insert(rel.to_owned(), Priced::Unplanned);
            continue;
        }
        // The build as `estimate` priced it (`calibrated`, before its rows were the planner's).
        let build = (work, rules.iter().map(|r| observed_rule(r, nodes)).sum::<Option<f64>>());
        out.insert(rel.to_owned(), Priced::Weighed(weigh(&own, &inl, build, (rows, observed_rows(rel, nodes)))));
    }
    out
}

/// One verdict's profile line.
fn shown(rel: &str, v: &Verdict) -> String {
    if v.inline {
        format!("{rel}: inlined, readers add {:.0} work < {ALPHA} x (build {:.0} + {M} x {:.0} rows); {}", v.added, v.build, v.rows, v.how)
    } else {
        format!("{rel}: stored, readers would add {:.0} work >= {ALPHA} x (build {:.0} + {M} x {:.0} rows); {}", v.added, v.build, v.rows, v.how)
    }
}

/// The choice for one relation from its price by the estimates and by the observations, and its profile line.
///
/// The estimates' choice stands unless the observations are evidence against it. Two things are: both sides ran
/// (stored in one chunk or part, inlined in another), and their observed work decides, so an inlining that did worse
/// is taken back; and the estimates inline a relation whose stored side ran, while its copies, priced by what their
/// steps' patterns did and doubted by the error the estimates made on the side that ran, would cost more than the
/// stored side did. An alternative that never ran is never taken on estimates the estimates alone would not take:
/// with two chunks it never runs before the last chunk, and a wrong guess then costs the whole chunk (0099, "The
/// regression").
fn settle(rel: &str, estimated: Option<&Priced>, observed: Option<&Priced>) -> (bool, Option<String>) {
    let est_inline = matches!(estimated, Some(Priced::Weighed(v)) if v.inline);
    let est_line = match estimated {
        Some(Priced::Weighed(v)) => Some(shown(rel, v)),
        Some(Priced::Unplanned) => Some(format!("{rel}: stored, a reader copy does not plan")),
        _ => None,
    };
    match observed {
        Some(Priced::Weighed(v)) if v.how == "both sides observed" => (v.inline, Some(shown(rel, v))),
        Some(Priced::Weighed(v)) if est_inline && !v.inline && v.how.starts_with("stored observed") => {
            (false, Some(format!("{}; the estimates would inline it", shown(rel, v))))
        }
        Some(Priced::Weighed(v)) if v.inline != est_inline => {
            (est_inline, est_line.map(|l| format!("{l}; observed, it would be {} ({}), but its alternative never ran", if v.inline { "inlined" } else { "stored" }, v.how)))
        }
        Some(Priced::Small(rows, planned)) if est_inline => (true, est_line.map(|l| format!("{l}; observed small ({rows:.0} rows measured, {planned:.0} planned)"))),
        // The observations agree: their line, which says which side ran.
        Some(Priced::Weighed(v)) => (est_inline, Some(shown(rel, v))),
        _ => (est_inline, est_line),
    }
}

/// The largest factor an observed side's error may widen the other side's estimate by (`weigh`).
const DOUBT_MAX: f64 = 1000.0;

/// The inline-or-store verdict on one relation (`weigh`): the readers' added work, the build and rows it is set
/// against, and how each side was priced.
#[derive(Debug)]
struct Verdict {
    inline: bool,
    added: f64,
    build: f64,
    rows: f64,
    how: String,
}

/// Inline or store, from each side's terms as (estimate, observed): `own`, the readers as they are (the relation
/// stored); `copies`, the reader copies (inlined); `build` and `rows`, the relation's when stored. Inlined when the
/// copies' work, less the readers', is under `ALPHA` times the build and `M` per row.
///
/// The side that ran is believed and the other is doubted. A side is observed when every term of it ran in an earlier
/// chunk or part: the readers as they are (stored), or every copy (inlined). Where both are, both are priced by what
/// they did; where neither is, both by the plan's estimates. Where one is, it is priced by what it did, and the other
/// by its estimate times the planner's error on the side that ran (observed over estimated, or its inverse, the
/// larger): an alternative that never ran must win by the margin the estimates have just been seen to miss by. Once
/// it wins and runs, both sides are observed, and the next chunk or part compares what each did, so a choice that did
/// worse is taken back.
fn weigh(own: &[(f64, Option<f64>)], copies: &[(f64, Option<f64>)], build: (f64, Option<f64>), rows: (f64, Option<f64>)) -> Verdict {
    let seen = |t: &[(f64, Option<f64>)]| !t.is_empty() && t.iter().all(|x| x.1.is_some());
    let est = |t: &[(f64, Option<f64>)]| t.iter().map(|x| x.0).sum::<f64>();
    let obs = |t: &[(f64, Option<f64>)]| t.iter().map(|x| x.1.unwrap_or(x.0)).sum::<f64>();
    // The planner's error on terms that ran: the larger of observed over estimated and its inverse.
    let error = |o: f64, e: f64| {
        let (o, e) = (o.max(1.0), e.max(1.0));
        (o / e).max(e / o).min(DOUBT_MAX)
    };
    let (stored_seen, inline_seen) = (seen(own), seen(copies));
    let held = rows.1.unwrap_or(rows.0);
    let (own_w, copies_w, build_w, how) = match (stored_seen, inline_seen) {
        (true, true) => (obs(own), obs(copies), build.1.unwrap_or(build.0), "both sides observed".to_owned()),
        (false, false) => (est(own), est(copies), build.1.unwrap_or(build.0), "both sides estimated".to_owned()),
        (true, false) => {
            let (o, e) = (obs(own) + build.1.unwrap_or(0.0), est(own) + build.1.map_or(0.0, |_| build.0));
            let doubt = error(o, e);
            (obs(own), doubt * est(copies), build.1.unwrap_or(build.0), format!("stored observed (estimated {e:.0}, did {o:.0}), inlined estimated {:.0} x {doubt:.1}", est(copies)))
        }
        (false, true) => {
            let doubt = error(obs(copies), est(copies));
            let build_w = build.1.unwrap_or(doubt * build.0);
            (doubt * est(own), obs(copies), build_w, format!("inlined observed (estimated {:.0}, did {:.0}), stored estimated {:.0} x {doubt:.1}", est(copies), obs(copies), est(own)))
        }
    };
    let added = copies_w - own_w;
    let inline = added < ALPHA * (build_w + M * held);
    Verdict { inline, added, build: build_w, rows: held, how }
}

/// Why each derived relation that must be stored is (the module's list); a relation absent may be inlined.
fn refusals(prog: &Program, rules_of: &BTreeMap<&str, Vec<&Rule>>, sccs: &[BTreeSet<String>]) -> BTreeMap<String, &'static str> {
    let mut why: BTreeMap<String, &'static str> = BTreeMap::new();
    let natives_read: BTreeSet<&str> = prog.natives.values().flat_map(|n| n.reads()).collect();
    // What each test reads, gathered once: a test per relation over the program's lists was quadratic in relations.
    let outputs: BTreeSet<&str> = prog.outputs.iter().map(String::as_str).collect();
    let inputs: BTreeSet<&str> = prog.inputs.iter().map(String::as_str).collect();
    let demanded: BTreeSet<&str> = prog.demand.iter().map(String::as_str).chain(prog.gates.iter().map(|(g, _)| g.as_str())).collect();
    let answers: BTreeSet<&str> = prog.answers.keys().map(String::as_str).chain(prog.answers.values().flatten().map(String::as_str)).collect();
    let recursive: BTreeSet<&str> = sccs.iter().filter(|s| s.len() > 1).flatten().map(String::as_str).collect();
    for (rel, rules) in rules_of {
        let rel = *rel;
        let reason = if outputs.contains(rel) {
            Some("an output")
        } else if inputs.contains(rel) || prog.facts.get(rel).is_some_and(|f| !f.is_empty()) {
            Some("an input or a relation of facts")
        } else if prog.lattice.contains_key(rel)
            || prog.natives.contains_key(rel)
            || natives_read.contains(rel)
            // A native's rows are read by the strata its readers open (`components`, `dfs_first`): not copied into others.
            // The reach index (decision 0119) is read in place by any stratum: its readers may be inlined.
            || rules.iter().any(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| prog.natives.get(a.rel.as_str()).is_some_and(|n| !matches!(n, crate::code_rule_dsl::logic::front::Native::Reach { .. } | crate::code_rule_dsl::logic::front::Native::ReachThrough { .. })))))
        {
            Some("a lattice or a native's")
        } else if demanded.contains(rel) {
            Some("demanded or a gate")
        } else if answers.contains(rel) || prog.nullary.contains(rel) {
            Some("an answer or nullary")
        } else if super::corpus::guests::REQUESTS.contains(&rel) || super::super::rr::core_names().contains(rel) || rel.contains("__") {
            Some("the engine's (a request, core.rr's, or a session's copy)")
        } else if recursive.contains(rel) || rules.iter().any(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| a.rel == rel))) {
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
                    why.insert(a.rel.to_string(), "read by a gated negation");
                }
                Literal::Agg { atom, .. } => {
                    why.insert(atom.rel.to_string(), "read inside an aggregate");
                }
                Literal::Atom(a) if a.args.iter().any(|t| matches!(t, Term::Call(..) | Term::Top)) => {
                    why.insert(a.rel.to_string(), "read with a computed argument");
                }
                _ => {}
            }
        }
    }
    why
}

/// The relations that cross the chunk boundary (the local part's outputs: its exports and findings) stored as the
/// flat join of parts that meet only at a variable they project away: `fl853_p4_outer(f, x) :- rr_inside_ks5(f, p),
/// bound_in(p, x)` holds 777 k rows, its two parts about 100 k. Such an export is handed on as its parts before the
/// first chunk (`file_local::factorize_exports`, decision 0099); this names each relation still stored flat that would
/// shrink so (one the program's test refused: a finding, a part of one row a key, a reader binding none of it), with
/// its flat rows (observed where an earlier chunk measured them) and its parts' estimated rows, where the flat rows
/// are over twice the parts'.
fn factorizable(prog: &Program, rules_of: &BTreeMap<&str, Vec<&Rule>>, estimates: &BTreeMap<String, (f64, f64, f64)>, model: &Model) -> Vec<String> {
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
                        let head = Atom { rel: format!("{rel}_part").into(), args: vars.into_iter().map(Term::Var).collect(), neg: false, gated: false };
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
pub(in crate::code_rule_dsl) fn factors(rule: &Rule) -> Option<(String, Vec<Vec<Literal>>)> {
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

/// What the native strata observed during a corpus run (`observing`): kept across its chunks and parts, so a later
/// chunk's choice prices each relation from the rows its join steps produced. The run's thread's, shared by the jobs
/// its strata run as on other threads (decision 0113, `share_observed`): each stratum observes its own rules and
/// relations.
pub(in crate::code_rule_dsl) type SharedObserved = std::sync::Arc<parking_lot::RwLock<Option<Observed>>>;

thread_local! {
    static OBSERVED_AT: RefCell<SharedObserved> = RefCell::new(Default::default());
}

/// The observations as a cell: read (`borrow`, several at once) or written (`borrow_mut`).
struct ObservedCell(SharedObserved);
impl ObservedCell {
    fn borrow(&self) -> parking_lot::RwLockReadGuard<'_, Option<Observed>> {
        self.0.read_recursive()
    }
    fn borrow_mut(&self) -> parking_lot::RwLockWriteGuard<'_, Option<Observed>> {
        self.0.write()
    }
}
struct ObservedKey;
impl ObservedKey {
    fn with<R>(&self, f: impl FnOnce(&ObservedCell) -> R) -> R {
        f(&ObservedCell(OBSERVED_AT.with(|a| a.borrow().clone())))
    }
}
static OBSERVED: ObservedKey = ObservedKey;

/// This thread's observations, for a job on another thread to enter.
pub(in crate::code_rule_dsl) fn share_observed() -> SharedObserved {
    OBSERVED_AT.with(|a| a.borrow().clone())
}

/// While the guard lives, this thread's strata observe into `to` (another thread's, `share_observed`).
pub(in crate::code_rule_dsl) fn enter_observed(to: SharedObserved) -> impl Drop {
    struct Entered(Option<SharedObserved>);
    impl Drop for Entered {
        fn drop(&mut self) {
            if let Some(a) = self.0.take() {
                OBSERVED_AT.with(|o| *o.borrow_mut() = a);
            }
        }
    }
    Entered(Some(OBSERVED_AT.with(|a| std::mem::replace(&mut *a.borrow_mut(), to))))
}

/// Observed join work and relations, each with the corpus nodes of the runs that observed it (summed), so a later
/// chunk reads a rate per node: per rule (`obs_rule`: its head and body), the rows into and out of every step of every
/// probe of it (the planner's units of work: `native.tsv`'s `steps_in_out`); per relation, its rows when its stratum
/// ended and per column its distinct values and fanout (`Rel`).
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Observed {
    rules: rustc_hash::FxHashMap<u64, (f64, f64)>,
    rels: rustc_hash::FxHashMap<String, Rel>,
    /// Per (rule, step pattern) and per step pattern alone (`pattern`), the rows into and out of the steps of that
    /// pattern: what a step keeps of each binding it is given.
    steps: rustc_hash::FxHashMap<(u64, String), [f64; 2]>,
    pooled: rustc_hash::FxHashMap<String, [f64; 2]>,
    /// The rules inlining made (`copied`): each rule's hash, the reader's it was unfolded from and the relations
    /// unfolded into it; and what they did, per (relation, reader), with the nodes of the runs that observed it.
    copies: rustc_hash::FxHashMap<u64, (u64, BTreeSet<String>)>,
    inlined: rustc_hash::FxHashMap<(String, u64), (f64, f64)>,
}

/// A relation as its strata left it, summed over the runs that observed it: rows and nodes, and per column the
/// distinct values (summed: a rate per node) and the fanout of an equality probe weighted by the run's rows.
#[derive(Default)]
struct Rel {
    rows: f64,
    nodes: f64,
    ndv: Vec<(f64, f64)>,
    fan: Vec<(f64, f64)>,
}

/// A relation when its stratum ended (`observe`): its rows, and per column its distinct values and its fanout, where
/// the live counts measured them.
pub(super) struct Seen<'a> {
    pub(super) rel: &'a str,
    pub(super) rows: f64,
    pub(super) cols: Vec<(Option<f64>, Option<f64>)>,
}

/// Observations kept while held (a corpus run, `KeepPlans`); none outside one. `RERAIL_OBSERVED_STEPS=off` keeps none
/// (every choice from the plan's estimates), for comparison.
pub(super) fn observing(on: bool) {
    let on = on && !std::env::var("RERAIL_OBSERVED_STEPS").is_ok_and(|v| v == "off");
    OBSERVED.with(|o| *o.borrow_mut() = on.then(Observed::default));
}

/// A stratum's native run: per rule, the (in, out) rows of each step of each of its probes, and each relation it
/// made, over a corpus of `nodes`.
pub(super) fn observe(rules: &[(u64, &[(u64, u64)])], rels: &[Seen], nodes: f64) {
    OBSERVED.with(|o| {
        let mut o = o.borrow_mut();
        let Some(o) = o.as_mut() else { return };
        let mut per: BTreeMap<u64, f64> = BTreeMap::new();
        for (rule, steps) in rules {
            *per.entry(*rule).or_default() += steps.iter().map(|(a, b)| (a + b) as f64).sum::<f64>();
        }
        let mut through: BTreeMap<(String, u64), f64> = BTreeMap::new();
        for (rule, work) in per {
            let e = o.rules.entry(rule).or_default();
            e.0 += work;
            e.1 += nodes.max(1.0);
            if let Some((reader, into)) = o.copies.get(&rule) {
                for rel in into {
                    *through.entry((rel.clone(), *reader)).or_default() += work;
                }
            }
        }
        for (key, work) in through {
            let e = o.inlined.entry(key).or_default();
            e.0 += work;
            e.1 += nodes.max(1.0);
        }
        for seen in rels {
            let e = o.rels.entry(seen.rel.to_owned()).or_default();
            e.rows += seen.rows;
            e.nodes += nodes.max(1.0);
            if e.ndv.len() < seen.cols.len() {
                e.ndv.resize(seen.cols.len(), (0.0, 0.0));
                e.fan.resize(seen.cols.len(), (0.0, 0.0));
            }
            for (c, (ndv, fan)) in seen.cols.iter().enumerate() {
                if let Some(d) = ndv {
                    e.ndv[c].0 += d;
                    e.ndv[c].1 += nodes.max(1.0);
                }
                if let Some(f) = fan {
                    e.fan[c].0 += f * seen.rows.max(1.0);
                    e.fan[c].1 += seen.rows.max(1.0);
                }
            }
        }
    });
}

/// A step's pattern, what its selectivity is pooled by: an atom's relation, negation and binding pattern (its bound
/// columns, constants by value, as the planner's `Stats::obs` keys it); a comparison's operator, constants and which
/// sides are bound; a `nearest`'s kinds and direction.
fn pattern(l: &Literal, bound: &BTreeSet<String>) -> Option<String> {
    let term = |t: &Term| match t {
        Term::Var(v) if bound.contains(v) => "_".to_owned(),
        Term::Var(_) => "?".to_owned(),
        Term::Wild => "*".to_owned(),
        t => t.to_string(),
    };
    Some(match l {
        Literal::Atom(a) => {
            let (rel, cols) = super::atom_sig(a, bound);
            format!("{}{rel}{cols:?}", if a.neg { "!" } else { "" })
        }
        Literal::Cmp { l, op, r } => format!("{} {op} {}", term(l), term(r)),
        Literal::Nearest { n, kindset, .. } => format!("nearest {kindset} {}", if bound.contains(n) { "climb" } else { "descend" }),
        _ => return None,
    })
}

/// A native stratum's steps for one plan of `rule` (`order`), each with its rows (in, out): kept per pattern.
pub(super) fn observe_steps(rule: u64, order: &[super::Step], counts: &[(i64, i64)]) {
    OBSERVED.with(|o| {
        let mut o = o.borrow_mut();
        let Some(o) = o.as_mut() else { return };
        let mut bound = BTreeSet::new();
        for (i, (s, &(n_in, n_out))) in order.iter().zip(counts).enumerate() {
            if i > 0
                && n_in > 0
                && let Some(p) = pattern(&s.lit, &bound)
            {
                for e in [o.steps.entry((rule, p.clone())).or_default(), o.pooled.entry(p).or_default()] {
                    e[0] += n_in as f64;
                    e[1] += n_out as f64;
                }
            }
            if !matches!(&s.lit, Literal::Atom(l) if l.neg) {
                bound.extend(s.lit.vars());
            }
        }
    });
}

/// Bindings a step pattern must have seen before its observed selectivity replaces the estimate (the planner's
/// `OBS_MIN`).
const STEP_MIN: f64 = 32.0;

/// `rule`'s work as its plan would do it, in the native strata's units (each step's rows in and out): the plan the
/// planner makes for it, each step's selectivity observed where an earlier chunk or part ran a step of its pattern
/// (in this rule first, else in any), else the plan's estimate. A copy never ran, but its steps' patterns mostly did:
/// a probe of `look` by its name and scope, `$x != 8173` (decision 0099, "The regression").
fn calibrated(prog: &Program, model: &Model, rule: &Rule) -> Option<f64> {
    calibrated_at(prog, model, rule, None)
}

/// `calibrated`, the plan read with body literal `first` first (a delta plan's), else whole.
fn calibrated_at(prog: &Program, model: &Model, rule: &Rule, first: Option<usize>) -> Option<f64> {
    calibrated_rows_at(prog, model, rule, first).map(|(w, _)| w)
}

/// `calibrated`, and the rows the plan expects.
fn calibrated_rows(prog: &Program, model: &Model, rule: &Rule) -> Option<(f64, f64)> {
    calibrated_rows_at(prog, model, rule, None)
}

/// `calibrated_rows`, with body literal `first` read first.
fn calibrated_rows_at(prog: &Program, model: &Model, rule: &Rule, first: Option<usize>) -> Option<(f64, f64)> {
    let was = super::explaining(true);
    let got = model.plan_rule_explained(prog, &rule.head, &rule.body, first);
    super::explaining(was);
    let ((order, _, out), Some(x)) = got.ok()? else { return None };
    let key = super::obs_rule(&rule.head, &rule.body);
    OBSERVED.with(|o| {
        let o = o.borrow();
        let seen = |p: &str| -> Option<f64> {
            let o = o.as_ref()?;
            let e = o.steps.get(&(key, p.to_owned())).filter(|e| e[0] >= STEP_MIN).or_else(|| o.pooled.get(p).filter(|e| e[0] >= STEP_MIN))?;
            Some(e[1] / e[0])
        };
        let mut bound = BTreeSet::new();
        let (mut rows, mut work, mut before) = (1.0f64, 0.0f64, 1.0f64);
        for (i, (s, st)) in order.iter().zip(&x.steps).enumerate() {
            let estimated = if before > 0.0 { st.rows / before } else { 1.0 };
            before = st.rows;
            let factor = if i == 0 { st.rows } else { pattern(&s.lit, &bound).and_then(|p| seen(&p)).unwrap_or(estimated) };
            let out = rows * factor;
            work += rows + out;
            rows = out;
            if !matches!(&s.lit, Literal::Atom(l) if l.neg) {
                bound.extend(s.lit.vars());
            }
        }
        Some((work, out))
    })
}

/// `rule`'s observed work, scaled to a corpus of `nodes`.
fn observed_rule(rule: &Rule, nodes: f64) -> Option<f64> {
    let key = super::obs_rule(&rule.head, &rule.body);
    OBSERVED.with(|o| o.borrow().as_ref()?.rules.get(&key).map(|(w, n)| w / n * nodes.max(1.0)))
}

/// The rules inlining made, each as (its hash, the hash of the reader it was unfolded from, the relations unfolded
/// into it): what they do when they run is the inlined side of those relations' readers (`observed_copies`).
fn copied<'a>(made: impl Iterator<Item = (u64, u64, &'a BTreeSet<String>)>) {
    OBSERVED.with(|o| {
        if let Some(o) = o.borrow_mut().as_mut() {
            for (rule, reader, into) in made {
                o.copies.insert(rule, (reader, into.clone()));
            }
        }
    });
}

/// What `reader`'s copies did with `rel` inlined, scaled to a corpus of `nodes`.
fn observed_copies(rel: &str, reader: &Rule, nodes: f64) -> Option<f64> {
    let key = (rel.to_owned(), super::obs_rule(&reader.head, &reader.body));
    OBSERVED.with(|o| o.borrow().as_ref()?.inlined.get(&key).map(|(w, n)| w / n * nodes.max(1.0)))
}

/// `rel`'s observed rows, scaled to a corpus of `nodes`.
fn observed_rows(rel: &str, nodes: f64) -> Option<f64> {
    OBSERVED.with(|o| o.borrow().as_ref()?.rels.get(rel).map(|r| r.rows / r.nodes * nodes.max(1.0)))
}

/// `rel`'s observed columns, per column its distinct values (scaled to a corpus of `nodes`, at most `rows`) and its
/// fanout, where observed.
fn observed_cols(rel: &str, nodes: f64, rows: f64) -> Vec<(Option<f64>, Option<f64>)> {
    OBSERVED.with(|o| {
        let o = o.borrow();
        let Some(r) = o.as_ref().and_then(|o| o.rels.get(rel)) else { return Vec::new() };
        r.ndv
            .iter()
            .zip(&r.fan)
            .map(|(&(d, n), &(f, w))| ((n > 0.0).then(|| (d / n * nodes.max(1.0)).clamp(1.0, rows.max(1.0))), (w > 0.0).then(|| (f / w).max(1.0))))
            .collect()
    })
}

/// What `overlay` replaced: per relation, its card and per column its distinct values and fanout before.
type Saved = Vec<(String, Option<f64>, Vec<(Option<f64>, Option<f64>)>)>;

/// Each of `rels` an earlier chunk or part measured takes its measured rows and columns in the planner's statistics,
/// over what the estimates put there; `restore` puts those back.
fn overlay<'a>(prog: &Program, model: &mut Model, rels: impl Iterator<Item = &'a str>) -> Saved {
    let nodes = model.stats.nodes;
    let mut saved = Vec::new();
    for rel in rels {
        let Some(rows) = observed_rows(rel, nodes) else { continue };
        let width = prog.decls.get(rel).map_or(0, Vec::len);
        let cols = observed_cols(rel, nodes, rows);
        let before = (0..width).map(|c| (model.ndv.get(&(rel.to_owned(), c)).copied(), model.stats.fan.get(&(rel.to_owned(), c)).copied())).collect();
        saved.push((rel.to_owned(), model.stats.card.get(rel).copied(), before));
        model.stats.card.insert(rel.to_owned(), rows.max(1.0));
        for (c, (ndv, fan)) in cols.into_iter().enumerate().take(width) {
            if let Some(d) = ndv {
                model.ndv.insert((rel.to_owned(), c), d);
            }
            if let Some(f) = fan {
                model.stats.fan.insert((rel.to_owned(), c), f);
            }
        }
    }
    saved
}

fn restore(model: &mut Model, saved: Saved) {
    for (rel, card, cols) in saved {
        match card {
            Some(n) => model.stats.card.insert(rel.clone(), n),
            None => model.stats.card.remove(&rel),
        };
        for (c, (ndv, fan)) in cols.into_iter().enumerate() {
            let key = (rel.clone(), c);
            match ndv {
                Some(d) => model.ndv.insert(key.clone(), d),
                None => model.ndv.remove(&key),
            };
            match fan {
                Some(f) => model.stats.fan.insert(key, f),
                None => model.stats.fan.remove(&key),
            };
        }
    }
}

/// What `estimate` put into the planner's statistics, to take back.
#[derive(Default)]
struct Inserted {
    card: Vec<String>,
    ndv: Vec<(String, usize)>,
    fan: Vec<(String, usize)>,
}

/// Each derived relation's estimated (rows, work to build it, rows as planned), in evaluation order; each estimate is the planner's
/// statistics while its readers are priced (`unestimate` takes them back). A relation an earlier chunk or part measured
/// keeps its measured rows, and its columns' measured distinct values and fanouts: a column's distinct values are
/// otherwise guessed (the least of the body columns that bind it, or the rows), and a guess prices a probe of it
/// wrongly in either direction (0099, "The regression").
fn estimate(prog: &Program, model: &mut Model, rules_of: &BTreeMap<&str, Vec<&Rule>>, sccs: &[BTreeSet<String>], need: &BTreeSet<&str>, built: &BTreeSet<&str>) -> (BTreeMap<String, (f64, f64, f64)>, Inserted) {
    let mut out = BTreeMap::new();
    let mut inserted = Inserted { card: Vec::new(), ndv: Vec::new(), fan: Vec::new() };
    for scc in sccs {
        for rel in scc {
            let Some(rules) = rules_of.get(rel.as_str()).filter(|_| need.contains(rel.as_str())) else { continue };
            let (mut rows, mut work) = (0.0, 0.0);
            // A relation measured, whose build is not priced, takes its measured rows and columns without planning.
            let nodes = model.stats.nodes;
            let measured_only = !built.contains(rel.as_str()) && (model.stats.card.contains_key(rel) || observed_rows(rel, nodes).is_some());
            for r in rules.iter().filter(|_| !measured_only) {
                if let Some((w, n)) = calibrated_rows(prog, model, r) {
                    rows += n;
                    work += w;
                }
            }
            // Observed on an earlier chunk or part of this run (`Observed`), per node of the corpus: its rows when
            // it was stored, and its build's work when every rule of it ran.
            let measured = model.stats.card.get(rel).copied().or_else(|| observed_rows(rel, nodes));
            let planned = rows;
            let rows = measured.unwrap_or(rows);
            out.insert(rel.clone(), (rows, work, planned));
            if !model.stats.card.contains_key(rel) {
                model.stats.card.insert(rel.clone(), rows.max(1.0));
                inserted.card.push(rel.clone());
                // A column's distinct values: at most the rows, and at most those of the body column that binds it.
                let width = prog.decls.get(rel).map_or(0, Vec::len);
                let seen = observed_cols(rel, nodes, rows);
                for c in 0..width {
                    let (seen_ndv, seen_fan) = seen.get(c).copied().unwrap_or((None, None));
                    if let Some(f) = seen_fan
                        && !model.stats.fan.contains_key(&(rel.clone(), c))
                    {
                        model.stats.fan.insert((rel.clone(), c), f);
                        inserted.fan.push((rel.clone(), c));
                    }
                    if model.ndv.contains_key(&(rel.clone(), c)) {
                        continue;
                    }
                    if let Some(d) = seen_ndv {
                        model.ndv.insert((rel.clone(), c), d);
                        inserted.ndv.push((rel.clone(), c));
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
                                if let Some(&d) = model.ndv.get(&(a.rel.to_string(), k)) {
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
    for key in inserted.fan {
        model.stats.fan.remove(&key);
    }
}

#[cfg(test)]
pub mod tests {
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
        let derived: HashSet<String> = prog.rules.iter().map(|r| r.head.rel.to_string()).collect();
        let mut model = Model { docs_by_shift: false, analysis: Default::default(), input_set: Default::default(), stats: Stats::empty(&prog), ndv: HashMap::new(), docof: Default::default(), view_src: Default::default(), inputs: inputs.into(), derived: derived.into() };
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

    /// A head that names a variable twice equates the read's terms there, the read's own: a reader variable named as
    /// one of the definition's (`n` in both) stays the reader's. (`expr_name(n, n, x) :- ast_name(n, x)` read as
    /// `expr_name(t, n, x)` once came out `t = t`, leaving the reader's `n` free: every stored name of the batch.)
    #[test]
    fn a_head_variable_named_twice_equates_the_readers_terms() {
        let reader = rule("o(t) :- a(t), b(t, n), r(n).");
        let def = rule("b(n, n) :- a(n).");
        let copies = unfold(&reader, 1, &[&def], 1).unwrap();
        assert_eq!(copies[0].to_string(), "o($t) :- a($t), a($t), $t = $n, r($n)");
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

    /// The two-chunk regression's shape (0099, "The regression"): `ctx` is small, read by one reader that binds its
    /// columns. What the steps really do (below: each probe of `look` by name and scope keeps 4 rows a binding,
    /// `binding` 3, the comparison every row) makes the copy about 66 k rows of work against about 18 k stored, so
    /// `ctx` should stay stored. Before the measured key domains (2ab61d7b4, O148) the estimate priced the copy's
    /// probe of `look` as keeping a binding in a hundred and inlined it, and only the observed side took that back.
    /// The estimate now bounds the bindings by their constant filters and applies containment to every probe, and
    /// keeps `ctx` stored from the start; once the stored side has run, the copy is priced by those selectivities,
    /// and `ctx` still stays stored. (`the_side_that_ran_is_believed` pins `weigh`'s taking an estimate back.)
    #[test]
    fn observed_selectivities_price_a_copy_that_never_ran() {
        use super::super::{Inputs, Stats, make_plan};
        let prog = super::super::super::rr::parse(
            ".decl load(n: number, x: number)\n.decl look(n: number, x: number, s: number)\n.decl binding(s: number, x: number, b: number)\n\
             .decl ctx(n: number, s: number, x: number)\n.decl agg(n: number, s: number, x: number)\n.output agg\n\
             ctx(n, s, x) :- load(n, x), look(n, x, s), x != 7.\n\
             agg(n, s, x) :- ctx(n, s, x), binding(s, x, b), b < n.\n",
        )
        .unwrap();
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let mut stats = Stats::empty(&prog);
        for (rel, n) in [("load", 200_000.0), ("look", 500_000.0), ("binding", 30_000.0)] {
            stats.card.insert(rel.into(), n);
        }
        let mut inputs = Inputs::file();
        inputs.facts.clear();
        inputs.given = Some(stats);
        let mut model = make_plan(&prog, &db, inputs).unwrap();
        for (rel, n) in [("load", 200_000.0), ("look", 500_000.0), ("binding", 30_000.0)] {
            model.stats.card.insert(rel.into(), n);
        }
        for (rel, ndvs) in [("load", vec![200_000.0, 500.0]), ("look", vec![5_000.0, 400.0, 800.0]), ("binding", vec![1_000.0, 700.0, 3_000.0])] {
            for (c, d) in ndvs.into_iter().enumerate() {
                model.ndv.insert((rel.into(), c), d);
            }
        }
        let decided = |model: &mut Model| choose(&prog, model).unwrap().map(|(_, done)| done).unwrap_or_default();
        let reader = prog.rules.iter().find(|r| r.head.rel == "agg").unwrap();
        let def = prog.rules.iter().find(|r| r.head.rel == "ctx").unwrap();
        let copy = &unfold(reader, 0, &[def], 0).unwrap()[0];
        // Estimates alone: stored, as the steps' real selectivities say.
        observing(true);
        let priced = |model: &mut Model| {
            let mut rules_of: BTreeMap<&str, Vec<&Rule>> = BTreeMap::new();
            rules_of.entry("ctx").or_default().push(def);
            let (sccs, _) = strata(&prog).unwrap();
            let (e, ins) = estimate(&prog, model, &rules_of, &sccs, &BTreeSet::from(["ctx"]), &BTreeSet::from(["ctx"]));
            let got = format!("cards {:?} ndv {:?}; ctx {:?}; reader {:?}, copy {:?}, build {:?}", ["load", "look", "binding"].map(|r| model.stats.card.get(r).copied()), model.ndv.get(&("look".to_owned(), 0)), e.get("ctx"), calibrated(&prog, model, reader), calibrated(&prog, model, copy), calibrated(&prog, model, def));
            unestimate(model, ins);
            got
        };
        let shown = priced(&mut model);
        assert!(decided(&mut model).is_empty(), "estimated, the copy is priced above the stored side: {shown}");
        // The stored side ran: the reader and the build, with the rows each step made.
        let nodes = model.stats.nodes;
        let ran = |rule: &Rule, factors: &dyn Fn(&Literal) -> f64| -> Vec<(u64, u64)> {
            let (order, _, _) = model.plan_rule(&prog, &rule.head, &rule.body, None).unwrap();
            let mut rows = 2_000.0;
            let mut counts = vec![(1u64, 2_000u64)];
            for s in &order[1..] {
                let out = rows * factors(&s.lit);
                counts.push((rows as u64, out as u64));
                rows = out;
            }
            observe_steps(super::super::obs_rule(&rule.head, &rule.body), &order, &counts.iter().map(|&(a, b)| (a as i64, b as i64)).collect::<Vec<_>>());
            counts
        };
        // What the copy's steps did in other rules: `look` by name and scope 4 rows a binding, `x != 7` all of them.
        let elsewhere = |l: &Literal| match l {
            Literal::Atom(a) if a.rel == "look" => 4.0,
            Literal::Atom(a) if a.rel == "binding" => 3.0,
            _ => 1.0,
        };
        let (order, _, _) = model.plan_rule(&prog, &copy.head, &copy.body, None).unwrap();
        let mut rows = 2_000.0;
        let mut counts = vec![(1i64, 2_000i64)];
        for s in &order[1..] {
            let out = rows * elsewhere(&s.lit);
            counts.push((rows as i64, out as i64));
            rows = out;
        }
        observe_steps(0, &order, &counts);
        let own = ran(reader, &|l| if matches!(l, Literal::Atom(a) if a.rel == "binding") { 3.0 } else { 0.75 });
        let build = ran(def, &|_| 1.0);
        observe(&[(super::super::obs_rule(&reader.head, &reader.body), &own), (super::super::obs_rule(&def.head, &def.body), &build)], &[Seen { rel: "ctx", rows: 2_000.0, cols: vec![(Some(2_000.0), Some(1.0)), (Some(300.0), Some(7.0)), (Some(250.0), Some(8.0))] }], nodes);
        let shown = priced(&mut model);
        assert!(decided(&mut model).is_empty(), "observed, the copy is priced by what its steps did: ctx stays stored: {shown}");
        observing(false);
    }

    /// `weigh`: the side that ran is believed, the other doubted by the error the estimates were seen to make.
    #[test]
    fn the_side_that_ran_is_believed() {
        // Stored ran: the reader estimated 10 k and did 40 k, the build as estimated (50 k): the estimates missed by
        // 1.5 times. The copies' 70 k would save work against 40 k + 50 k + 4 x 1,000 rows; doubted, 105 k does not.
        let v = weigh(&[(10_000.0, Some(40_000.0))], &[(35_000.0, None), (35_000.0, None)], (50_000.0, Some(50_000.0)), (1_000.0, Some(1_000.0)));
        assert!(!v.inline && v.how.starts_with("stored observed"), "{v:?}");
        // Undoubted, the same copies are inlined.
        let v = weigh(&[(40_000.0, None)], &[(35_000.0, None), (35_000.0, None)], (50_000.0, None), (1_000.0, None));
        assert!(v.inline, "{v:?}");
        // Nothing ran: estimates against estimates.
        let v = weigh(&[(17_000.0, None)], &[(8_500.0, None), (8_500.0, None)], (55_000.0, None), (1_433.0, None));
        assert!(v.inline && v.how == "both sides estimated", "{v:?}");
        // Both ran, inlined did worse than stored: taken back.
        let v = weigh(&[(17_000.0, Some(30_000.0))], &[(8_500.0, Some(2_000_000.0))], (55_000.0, Some(55_000.0)), (1_433.0, Some(1_433.0)));
        assert!(!v.inline && v.how == "both sides observed", "{v:?}");
    }

    /// `.decl r(…) inline` and its kin are read as Soufflé spells them; two that disagree are an error.
    #[test]
    fn decl_qualifiers_are_read() {
        let prog = super::super::super::rr::parse(".decl r(x: node) inline\n.decl s(x: node) no_magic\n.decl t(x: node) no_inline magic\n").unwrap();
        assert!(prog.qualified("r", "inline") && prog.qualified("s", "no_magic") && prog.qualified("t", "no_inline") && prog.qualified("t", "magic"));
        assert!(!prog.qualified("r", "magic"));
        assert!(super::super::super::rr::parse(".decl r(x: node) inline no_inline\n").is_err());
    }

    /// A reader of one inlined relation twice has the second read unfolded a pass after the first, its copy's
    /// variables named apart from the first copy's: the same names would join the two reads' rows as one.
    #[test]
    fn two_reads_of_one_relation_unfold_apart() {
        let reader = rule("o(x) :- a(y), r(x), r(y).");
        let def = rule("r(z) :- b(z, w), a(w).");
        let (rules, _) = unfold_reads(vec![reader, def], &BTreeSet::from(["r".to_owned()]));
        assert_eq!(rules[0].to_string(), "o($x) :- a($y), b($x, $w_i0_1), a($w_i0_1), b($y, $w_i1_1), a($w_i1_1)");
    }
}
