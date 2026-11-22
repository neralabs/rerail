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
//! `RERAIL_OBSERVED_STEPS=off` prices from estimates alone, for comparison. `RERAIL_KEEPS_TRACE=<text>` prints each
//! estimated relation whose name holds the text: its rows and work, the rows and distinct values of what its rules
//! read, and each rule's steps' rows.
//!
//! Stored always: outputs, inputs, lattices, relations with facts, natives and what they read,
//! demanded relations, an aggregate's demand (the keys it enumerates from), gates, answers, recursive relations, relations read inside an aggregate (a count, a sum or a
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

use super::Model;
use super::body::{Body, Placed};
use super::fields::Fields;
use super::super::program_image::{RuleId, Version};
use std::cell::RefCell;
use std::sync::Arc;
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
/// At most this many copies of one rule of the program, over every pass of the inlining (`inline::inline`): each pass may multiply
/// a rule's copies (a negated read of a relation of several rules is their product), and a chain of inlined relations
/// compounds them. `lazy-architecture` (data-layer) took 8 GB in 7 s unfolding, over one file. A rule past it keeps
/// its read, and the relation read keeps its rules (stored).
pub const RULE_COPIES: usize = COPIES * 4;
/// A relation estimated under this many rows is stored without pricing: its rows cost little either way.
pub const SMALL: f64 = 1024.0;

pub mod inline;

/// The relations inlined, and the program's version with them unfolded into their readers (an overlay of `base`);
/// `None` when nothing is inlined. `Err` names a `.decl r(…) inline` that cannot be honoured.
pub fn choose(base: &Placed, model: &mut Model) -> Result<Option<(Vec<String>, Placed)>, String> {
    // The program's fields beside its rules, read from its version; a version of rules alone inlines nothing.
    let Some(f) = Fields::of(base) else {
        super::super::profile::decision("inline or store (decision 0099)", || "a version of rules alone: nothing estimated".to_owned());
        return Ok(None);
    };
    let v: &Version = &base.v;
    let n = base.len();
    let rel_of = |a: super::super::program_image::AtomRef<'_>| v.rel_name(a.rel());
    let off = std::env::var("RERAIL_INLINE").is_ok_and(|v| v == "off");
    if off {
        if !f.about.qualifiers.iter().any(|&(_, w)| v.text(w) == "inline") {
            return Ok(None);
        }
        super::super::profile::decision("inline or store (decision 0099)", || "RERAIL_INLINE=off: every relation stored, `.decl … inline` too".to_owned());
        return Ok(None);
    }
    // Each relation's rules (the program's rule indices, in its order), by head.
    let mut ids_of: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for k in 0..n {
        ids_of.entry(rel_of(base.rule(k).head())).or_default().push(k);
    }
    // A session's part (decision 0099): every relation it keeps is `no_inline`, and the rest are its maintenance
    // pieces (`__` in the name, which `refusals` stores as the engine's). Nothing may be inlined, which the names
    // and qualifiers alone tell, before the strata, refusals and readers of its some 17,000 rules are made (0117
    // step 3: the whole-program passes not made each step).
    if std::env::var_os("RERAIL_INLINE_ONLY").is_none() && !ids_of.keys().any(|rel| f.qualified(rel, "inline") || !(f.qualified(rel, "no_inline") || rel.contains("__"))) {
        super::super::profile::decision("inline or store (decision 0099)", || "no relation may be inlined (each `.decl … no_inline` or a session's piece): nothing estimated".to_owned());
        return Ok(None);
    }
    let Ok(strata) = super::super::program_image::analyses::strata(v, f.about) else { return Ok(None) };
    let sccs: Vec<BTreeSet<String>> = strata.sccs.iter().map(|s| s.iter().map(|&r| v.rel_name(r).to_owned()).collect()).collect();
    let reasons = refusals(&f, base, n, &ids_of, &sccs);
    // The rules as the planner reads them, rule `k` the program's rule `k`.
    let bodies: Vec<Body<'_>> = (0..n).map(|k| base.body(k)).collect();
    let bodies_of: BTreeMap<&str, Vec<&Body<'_>>> = ids_of.iter().map(|(rel, ks)| (*rel, ks.iter().map(|&k| &bodies[k]).collect())).collect();
    let src = Read { base, bodies: &bodies, ids_of: &ids_of };
    // Readers: (rule index, body position) of each read of a relation.
    let mut readers: BTreeMap<&str, Vec<(usize, usize)>> = BTreeMap::new();
    for k in 0..n {
        for (i, l) in base.rule(k).body().enumerate() {
            if let Some(a) = l.atom() {
                readers.entry(rel_of(a)).or_default().push((k, i));
            }
        }
    }
    // `RERAIL_INLINE_ONLY=r,…` (the lab's comparison): those relations inlined where they can be, every other stored.
    // Read before the cone (below) is chosen.
    let only: Option<Vec<String>> = std::env::var("RERAIL_INLINE_ONLY").ok().map(|v| v.split(',').map(str::to_owned).collect());
    // The estimates below plan rules, which a maintained program has about ten times of. They price only a relation
    // that may be inlined: none (a session's part stores its relations, decision 0099), none are made.
    let open = |rel: &str| !f.qualified(rel, "no_inline") && !reasons.contains_key(rel) && readers.contains_key(rel);
    if only.is_none() && !ids_of.keys().any(|rel| f.qualified(rel, "inline") || open(rel)) {
        let stored = ids_of.keys().filter(|rel| f.qualified(rel, "no_inline")).count();
        super::super::profile::decision("inline or store (decision 0099)", || format!("no relation may be inlined ({stored} `.decl … no_inline`): nothing estimated"));
        return Ok(None);
    }
    // The relations whose estimates the pricing reads: each candidate (a relation the loop below may price), what its
    // readers read, and upstream of those every relation their rules read; the outputs too when the profile names
    // the factorizable ones. Estimating every derived relation planned every rule of the program (10.7 k rules on
    // the pack's), most of them for relations nothing prices.
    let mut need: BTreeSet<&str> = BTreeSet::new();
    let mut candidates: BTreeSet<&str> = BTreeSet::new();
    // A candidate its inputs' counts bound under `SMALL` rows is small with nothing planned (`bounds`).
    let bounded = if only.is_none() { bounds(&f, base, n, model, &ids_of, &sccs) } else { BTreeMap::new() };
    let mut bounded_small: BTreeMap<&str, f64> = BTreeMap::new();
    let reads_of = |k: usize| base.rule(k).body().filter_map(|l| l.atom()).map(rel_of).collect::<Vec<&str>>();
    if only.is_none() {
        for (rel, ks) in &ids_of {
            if reasons.contains_key(*rel) || ks.len() > K || f.qualified(rel, "inline") || f.qualified(rel, "no_inline") {
                continue;
            }
            let Some(reads) = readers.get(rel) else { continue };
            if let Some(&(rows, _)) = bounded.get(*rel).filter(|b| b.0 < SMALL) {
                bounded_small.insert(rel, rows);
                continue;
            }
            need.insert(rel);
            candidates.insert(rel);
            for &(ri, _) in reads {
                need.extend(reads_of(ri));
            }
        }
    }
    if super::super::profile::on() {
        need.extend(v.outputs().map(|a| v.text(a)));
    }
    let mut work: Vec<&str> = need.iter().copied().collect();
    while let Some(rel) = work.pop() {
        for &k in ids_of.get(rel).into_iter().flatten() {
            for a in reads_of(k) {
                if ids_of.contains_key(a) && need.insert(a) {
                    work.push(a);
                }
            }
        }
    }
    // Each candidate priced twice: by the plan's estimates alone (what `RERAIL_OBSERVED_STEPS=off` decides), and,
    // once an earlier chunk or part observed anything, with its observations (`calibrated`, `weigh`). The choice is
    // the estimates' unless the observations are evidence against it (`settle`).
    // Decision 0301: the jobs' sums handed in so far folded in first, then hidden from the estimates on this thread (and
    // the jobs it starts) by an empty handle in the run's place, not taken out of the run's.
    OBSERVED.with(|o| o.0.fold(true));
    let run = share_observed();
    let (held, hidden) = match run.published.load().is_some() {
        true => (Some(run), Some(crate::code_rule_dsl::logic::profile::bind(ObservedAt(Arc::new(ObservedShared::observing(true)))))),
        false => (None, None),
    };
    let observed_any = held.as_ref().is_some_and(|run| run.published.load().as_ref().is_some_and(|o| !o.rules.is_empty()));
    // The statistics the cone's plans read, if they are made when first planned with (`Model::resolve`); and its
    // readers' (the pricing below plans them).
    let read: Vec<&str> = need.iter().flat_map(|rel| ids_of.get(rel).into_iter().flatten()).flat_map(|&k| reads_of(k)).collect();
    model.resolve(f, read).map_err(|e| e.to_string())?;
    let (estimates, inserted) = estimate(f, model, (base, &ids_of), &sccs, &need, &candidates);
    let mut by_estimates = price_all(model, &f, &src, &readers, &reasons, &estimates, &need, &sccs);
    for (rel, rows) in &bounded_small {
        by_estimates.insert((*rel).to_owned(), Priced::Small(*rows, *rows));
    }
    if !bounded_small.is_empty() {
        super::super::profile::decision("inline or store (decision 0099)", || format!("{} candidates stored small, their inputs' counts bounding them under {SMALL} rows: not planned", bounded_small.len()));
    }
    drop(hidden);
    // The observations can change a choice only where the stored side ran (every reader as it is): a veto of an
    // inlining, or, where the inlined side ran too, a revert. Only those are priced again: over the estimates, each
    // relation an earlier chunk or part measured takes its measured rows and columns (`overlay`), and each build is
    // re-priced with the steps' observed selectivities.
    let mut again: BTreeSet<&str> = BTreeSet::new();
    if observed_any {
        for (rel, p) in &by_estimates {
            let inlined = matches!(p, Priced::Weighed(v) if v.inline);
            let reads = readers.get(rel.as_str()).map_or(&[][..], Vec::as_slice);
            let stored_ran = !reads.is_empty() && reads.iter().all(|&(ri, _)| observed_rule(&bodies[ri], 1.0).is_some());
            let copies_ran = reads.iter().all(|&(ri, _)| observed_copies(rel, &bodies[ri], 1.0).is_some());
            if stored_ran && (inlined || copies_ran) {
                again.insert(ids_of.get_key_value(rel.as_str()).map_or("", |(k, _)| *k));
            }
        }
        again.remove("");
    }
    let by_observed = if again.is_empty() {
        BTreeMap::new()
    } else {
        let saved = overlay(f, model, need.iter().copied());
        let nodes = model.stats.nodes;
        let again_estimates: BTreeMap<String, (f64, f64, f64)> = again
            .iter()
            .filter_map(|rel| {
                let (rows, work, planned) = estimates.get(*rel).copied()?;
                let build = bodies_of[rel].iter().map(|b| calibrated(f, model, b)).sum::<Option<f64>>().unwrap_or(work);
                Some(((*rel).to_owned(), (observed_rows(rel, nodes).unwrap_or(rows), build, planned)))
            })
            .collect();
        let got = price_all(model, &f, &src, &readers, &reasons, &again_estimates, &again, &sccs);
        restore(model, saved);
        got
    };
    let mut inline: BTreeSet<String> = BTreeSet::new();
    let mut lines: Vec<String> = Vec::new();
    let mut refused_inline: Vec<String> = Vec::new();
    for (rel, ks) in &ids_of {
        let rel = *rel;
        if only.as_ref().is_some_and(|o| !o.iter().any(|x| x == rel)) {
            continue;
        }
        if only.is_some() && let Some(why) = reasons.get(rel) {
            lines.push(format!("{rel}: stored, {why} (RERAIL_INLINE_ONLY)"));
            continue;
        }
        let forced = f.qualified(rel, "inline") || only.is_some();
        if f.qualified(rel, "no_inline") {
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
        let def_refs: Vec<_> = ids_of[rel].iter().map(|&k| base.rule(k)).collect();
        let Some(copies) = reads.iter().map(|&(ri, i)| inline::count(&base.v, base.rule(ri), i, &def_refs)).sum::<Option<usize>>() else {
            if f.qualified(rel, "inline") {
                refused_inline.push(format!("{rel}: a negated read is not a negation of plain literals"));
            } else if only.is_some() {
                lines.push(format!("{rel}: stored, a negated read is not a negation of plain literals (RERAIL_INLINE_ONLY)"));
            }
            continue;
        };
        if forced {
            let by = if f.qualified(rel, "inline") { "`.decl … inline`" } else { "RERAIL_INLINE_ONLY" };
            lines.push(format!("{rel}: inlined ({by}), {copies} reader copies"));
            inline.insert(rel.to_owned());
            continue;
        }
        if ks.len() > K || copies > COPIES {
            lines.push(format!("{rel}: stored, {} rules and {copies} reader copies (at most {K} and {COPIES})", ks.len()));
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
    // Each candidate was priced with every other relation stored: its copies read the relations its rules read as
    // rows. Two chosen where one reads the other compose instead (the reader's copies hold the read relation's body),
    // a reader copy nobody priced: RUF/PYI's diagnostics' document key `xk_…_rr_diagnostic_3(d)`, read by an `xd_`
    // cut, over a diagnostic relation also inlined, became `node(d, x), text(x, …), line(x, …), doc(d, …)` joined on
    // the document alone, 197 M rows on 30 files. So a relation stays inlined only where nothing its rules read is
    // inlined too (bottom up, its reads decided first); a forced one (`.decl … inline`, RERAIL_INLINE_ONLY) always.
    if only.is_none() {
        fn kept<'a>(rel: &'a str, chosen: &BTreeSet<String>, forced: &dyn Fn(&str) -> bool, reads: &dyn Fn(&str) -> Vec<&'a str>, memo: &mut BTreeMap<&'a str, bool>) -> bool {
            if let Some(&k) = memo.get(rel) {
                return k;
            }
            // Inlined relations are not recursive (`refusals`): the walk ends. Marked first, so a cycle cannot loop.
            memo.insert(rel, false);
            let k = chosen.contains(rel) && (forced(rel) || !reads(rel).into_iter().any(|r| r != rel && kept(r, chosen, forced, reads, memo)));
            memo.insert(rel, k);
            k
        }
        let forced = |rel: &str| f.qualified(rel, "inline");
        let reads = |rel: &str| { ids_of.get(rel).into_iter().flatten().flat_map(|&k| reads_of(k)).collect::<Vec<_>>() };
        let mut memo: BTreeMap<&str, bool> = BTreeMap::new();
        let names: Vec<&str> = ids_of.keys().copied().filter(|r| inline.contains(*r)).collect();
        let dropped: Vec<&str> = names.into_iter().filter(|r| !kept(r, &inline, &forced, &reads, &mut memo)).collect();
        for rel in dropped {
            inline.remove(rel);
            let under: Vec<&str> = reads(rel).into_iter().filter(|r| memo.get(r).copied().unwrap_or(false)).collect();
            lines.push(format!("{rel}: stored, it reads {} inlined, and was priced reading it stored", under.join(", ")));
        }
    }
    if super::super::profile::on() {
        for line in factorizable(f, base, &ids_of, &estimates, model) {
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
    // The inlining, an overlay of the program's version (`inline::inline`). Each rule's origin is the program's rule
    // it was unfolded from, and the relations unfolded into it, so what a copy does when it runs is the inlined side's
    // observation of that reader (`copied`).
    let got = inline::inline(&base.v, &base.order(), &inline)?;
    copied(got.rules.iter().filter(|(_, _, into)| !into.is_empty()).map(|(id, o, into)| (super::body::Body::of(&got.v, got.v.rule(*id).expect("a rule")).observed_key(), super::body::Body::of(&base.v, base.v.rule(*o).expect("a base rule")).observed_key(), into)));
    let done: Vec<String> = inline.into_iter().filter(|r| !got.unfinished.contains(r)).collect();
    if done.is_empty() {
        return Ok(None);
    }
    let placed = Placed::new(Arc::clone(&got.v), got.rules.iter().map(|(id, _, _)| *id).collect(), base.whole);
    Ok(Some((done, placed)))
}

/// A candidate's price (`price_all`): too small to price, a reader copy that does not plan, or weighed.
enum Priced {
    Small(f64, f64),
    Unplanned,
    Weighed(Verdict),
}

/// The program's rules as the pricing reads them: its version, each rule's body by its index, and each relation's
/// rules' indices.
struct Read<'r, 'v> {
    base: &'r Placed,
    bodies: &'r [Body<'v>],
    ids_of: &'r BTreeMap<&'r str, Vec<usize>>,
}

/// Prices every relation the choice may inline (`need`'s candidates), with the observations kept now (none while
/// they are held aside): each reader term (the reader as it is, its copies), the build and the rows, weighed.
fn price_all(
    model: &Model,
    f: &Fields<'_>,
    read: &Read<'_, '_>,
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
    for (rel, ks) in read.ids_of {
        let rel = *rel;
        if !need.contains(rel) || reasons.contains_key(rel) || ks.len() > K || f.qualified(rel, "inline") || f.qualified(rel, "no_inline") {
            continue;
        }
        let Some(reads) = readers.get(rel) else { continue };
        let defs: Vec<RuleId> = read.ids_of[rel].iter().map(|&k| read.base.id(k)).collect();
        let def_refs: Vec<_> = defs.iter().map(|&d| read.base.v.rule(d).expect("a def")).collect();
        let Some(copies) = reads.iter().map(|&(ri, i)| inline::count(&read.base.v, read.base.rule(ri), i, &def_refs)).sum::<Option<usize>>() else { continue };
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
        // Every reader's copies, an overlay of the program's version made for the pricing alone.
        let Ok((cv, made)) = inline::copies(&read.base.v, &reads.iter().map(|&(ri, i)| (read.base.id(ri), i)).collect::<Vec<_>>(), &defs) else {
            out.insert(rel.to_owned(), Priced::Unplanned);
            continue;
        };
        for (&(ri, i), made) in reads.iter().zip(&made) {
            let reader = &read.bodies[ri];
            let copies: Vec<Body<'_>> = made.iter().map(|&id| Body::of(&cv, cv.rule(id).expect("a copy"))).collect();
            // A reader in a recursion runs as its delta plans, one per recursive atom read first, over each wave's
            // new rows: priced so, each copy with the same atom first. Its whole plan may start from the inlined
            // relation's own inputs, which no delta plan can: `rr_cx_reach(d, f) :- rr_cx_reach(d, e),
            // rr_cx_edge(e, f)` priced whole joined `rr_cx_edge`'s imports and modules first, and inlining it looked
            // 7.8 M work cheaper; run, each new reach row scanned every document's module (277 s of calls_cx).
            let scc = sccs.iter().find(|s| s.contains(reader.rel(reader.head())));
            let deltas: Vec<usize> = (0..reader.len())
                .filter(|&k| k != i && reader.plain_atom(k).is_some_and(|a| !a.neg() && scc.is_some_and(|s| s.contains(reader.rel(a)))))
                .collect();
            let firsts: Vec<Option<usize>> = if deltas.is_empty() { vec![None] } else { deltas.iter().map(|&k| Some(k)).collect() };
            let (mut own_est, mut est) = (0.0, 0.0);
            for first in firsts {
                let Some(o) = *own_price.entry((ri, first)).or_insert_with(|| calibrated_at(*f, model, reader, first)) else {
                    priced = false;
                    break;
                };
                own_est += o;
                for copy in &copies {
                    // The delta atom in the copy: the reader's literal, which unfolding leaves as it is.
                    let at = match first {
                        Some(k) => match (0..copy.len()).position(|j| copy.show(j) == reader.show(k)) {
                            Some(at) => Some(at),
                            None => {
                                priced = false;
                                continue;
                            }
                        },
                        None => None,
                    };
                    match calibrated_at(*f, model, copy, at) {
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
        let build = (work, read.ids_of[rel].iter().map(|&k| observed_rule(&read.bodies[k], nodes)).sum::<Option<f64>>());
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
pub struct Verdict {
    pub inline: bool,
    added: f64,
    build: f64,
    rows: f64,
    pub how: String,
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
pub fn weigh(own: &[(f64, Option<f64>)], copies: &[(f64, Option<f64>)], build: (f64, Option<f64>), rows: (f64, Option<f64>)) -> Verdict {
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

/// Why each derived relation that must be stored is (the module's list), over the program's version (`f`, its rules
/// `base`'s, `rules_of` by head); a relation absent may be inlined.
/// Whether `ks` are the lowering's projection of an input: `kind(n, k) :- node(_, n, k, _, _)`, and `docof` over nodes
/// (decision 0141). One rule of one positive input atom, each head variable once in it: priced as any candidate, not
/// refused as core.rerail's. Inlined, a reader reads the input in place (`node`'s view, by kind). Inlined unpriced in every
/// module of a cold PostHog run, instructions went 2,558 -> 4,734 G (a reader probing `node` by node, not `kind`'s
/// hash index), so the pricing and its copies' bound decide.
fn input_projection(f: &Fields<'_>, base: &Placed, ks: &[usize]) -> bool {
    let &[k] = ks else { return false };
    let r = base.rule(k);
    let mut body = r.body();
    let (Some(l), None) = (body.next(), body.next()) else { return false };
    let Some(a) = l.atom().filter(|a| !a.neg() && f.is_input(base.v.rel_name(a.rel()))) else { return false };
    let mut seen = Vec::new();
    r.head().args().all(|t| t.var().is_some_and(|h| !seen.contains(&h) && { seen.push(h); a.args().filter(|t| t.var() == Some(h)).count() == 1 }))
}

/// The relations `core.rerail` declares (`rr::core_names`): the language reads them, a crate above the planner, and
/// the program's composition registers the reader (boot, or a crate's own test binary as it loads).
pub static CORE_NAMES: rerail_registry::Slot<fn() -> &'static std::collections::BTreeSet<String>> = rerail_registry::Slot::new("the core's relation names (plan::keeps::CORE_NAMES)");

/// The relations `core.rerail` declares, through the registered reader (`CORE_NAMES`).
pub fn core_names() -> &'static std::collections::BTreeSet<String> {
    CORE_NAMES.get().unwrap_or_else(|missing| panic!("{missing}: no program registered the core's names"))()
}

pub fn refusals(f: &Fields<'_>, base: &Placed, n: usize, rules_of: &BTreeMap<&str, Vec<usize>>, sccs: &[BTreeSet<String>]) -> BTreeMap<String, &'static str> {
    use super::super::program_image::{Lit, NativeKind, TermRef};
    let v: &Version = &base.v;
    let mut why: BTreeMap<String, &'static str> = BTreeMap::new();
    let natives_read = f.natives_read();
    let recursive: BTreeSet<&str> = sccs.iter().filter(|s| s.len() > 1).flatten().map(String::as_str).collect();
    let core = core_names();
    for (rel, ks) in rules_of {
        let rel = *rel;
        let rules = || ks.iter().map(|&k| base.rule(k));
        let reads = |r: super::super::program_image::RuleRef<'_>| r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel())).collect::<Vec<_>>();
        let reason = if f.is_output(rel) {
            Some("an output")
        } else if f.is_input(rel) || f.has_facts(rel) {
            Some("an input or a relation of facts")
        } else if f.lattice(rel)
            || f.native(rel).is_some()
            || natives_read.contains(rel)
            // A native's rows are read by the strata its readers open (`components`, `dfs_first`): not copied into others.
            // The reach index (decision 0119) is read in place by any stratum: its readers may be inlined.
            || rules().any(|r| reads(r).iter().any(|a| f.native(a).is_some_and(|n| !matches!(n.kind, NativeKind::Reach | NativeKind::ReachThrough))))
        {
            Some("a lattice or a native's")
        } else if f.demanded_or_gate(rel) {
            Some("demanded or a gate")
        } else if f.answer(rel) || f.nullary(rel) {
            Some("an answer or nullary")
        } else if crate::code_rule_dsl::logic::rr::lower::DEMANDS.contains(&rel) || (core.contains(rel) && !input_projection(f, base, ks)) || rel.contains("__") {
            Some("the engine's (a request, core.rerail's, or a session's copy)")
        } else if recursive.contains(rel) || rules().any(|r| reads(r).contains(&rel)) {
            Some("recursive")
        } else if rules().any(|r| r.body().any(|l| !matches!(l.view(), Lit::Atom(_) | Lit::Cmp { .. } | Lit::Nearest { .. }))) {
            Some("its rules aggregate or match")
        } else if rules().any(|r| r.body().next().is_none()) {
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
    // A demand-restricted aggregate's demand (`Body::demand_first`): its rows are the keys the aggregate enumerates
    // witnesses from, the boundary its planning keeps first. Inlined, the aggregate reads the demand's own join in
    // their place (`need` and `reset` of every name in a document, unbounded: 25M rows priced for 805 keys, 45 s
    // of one stratum over 91 files of the data layer's 50 policies).
    for k in 0..n {
        let b = base.body(k);
        if let Some(i) = b.demand_first()
            && let Some(a) = b.atom(i)
        {
            why.insert(b.rel(a).to_owned(), "an aggregate's demand");
        }
    }
    // Read inside an aggregate, by a gated negation, or by an atom with a computed argument.
    for k in 0..n {
        for l in base.rule(k).body() {
            match l.view() {
                Lit::Atom(a) if a.gated() => {
                    why.insert(v.rel_name(a.rel()).to_owned(), "read by a gated negation");
                }
                Lit::Agg { atom, .. } => {
                    why.insert(v.rel_name(atom.rel()).to_owned(), "read inside an aggregate");
                }
                Lit::Atom(a) if a.args().any(|t| matches!(t, TermRef::Call(..) | TermRef::Top)) => {
                    why.insert(v.rel_name(a.rel()).to_owned(), "read with a computed argument");
                }
                _ => {}
            }
        }
    }
    why
}

/// Each derived relation's most rows, and per column its most distinct values, as its inputs' statistics count them
/// and with no plan made, over the program's version (`f`, its rules `base`'s, `rules_of` by head). A relation's
/// rows are distinct head tuples, so a rule makes at most the product of its head
/// variables' distinct values, each at most the fewest distinct values of a column binding it in a positive atom (a
/// constant: one), and at most the product of its positive atoms' rows (unless an `ast` match binds). An input's rows
/// and distinct values are counted (`input_stats`); a relation of the program bounds by its own rules, in evaluation
/// order. What is not bounded so (a recursion, a relation with facts, an input with no statistics yet) is unbounded:
/// `f64::INFINITY`.
///
/// A candidate whose rows are bounded under `SMALL` is stored unpriced, as `price_all` stores one whose estimates are
/// under it, with neither it nor what it reads planned: on the data-layer batch over 9 in 10 priced candidates were
/// small, and their estimates planned three quarters of the programs' rules.
pub fn bounds(f: &Fields<'_>, base: &Placed, n: usize, model: &Model, rules_of: &BTreeMap<&str, Vec<usize>>, sccs: &[BTreeSet<String>]) -> BTreeMap<String, (f64, Vec<f64>)> {
    use super::super::program_image::{AtomRef, CmpOp, Lit, TermRef};
    let v: &Version = &base.v;
    let facts = |rel: &str| v.find(rel).and_then(|a| v.facts(a)).is_some();
    let mut out: BTreeMap<String, (f64, Vec<f64>)> = BTreeMap::new();
    let mut counted: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for k in 0..n {
        for a in base.rule(k).body().filter_map(|l| l.atom()) {
            let rel = v.rel_name(a.rel());
            if counted.contains_key(rel) || rules_of.contains_key(rel) || facts(rel) {
                continue;
            }
            let Some(&n) = model.stats.card.get(rel) else { continue };
            let cols = (0..a.arity()).map(|c| model.ndv.at(rel, c).map_or(n, |d| d.min(n))).collect();
            counted.insert(rel, cols);
        }
    }
    let _ = f;
    for scc in sccs {
        let recursive = scc.len() > 1 || scc.iter().any(|rel| rules_of.get(rel.as_str()).into_iter().flatten().any(|&k| base.rule(k).body().filter_map(|l| l.atom()).any(|a| v.rel_name(a.rel()) == rel)));
        for rel in scc {
            let Some(ks) = rules_of.get(rel.as_str()) else { continue };
            if recursive || facts(rel) {
                continue;
            }
            let rows_of = |a: AtomRef<'_>| -> f64 {
                let r = v.rel_name(a.rel());
                match out.get(r) {
                    Some((n, _)) => *n,
                    None if rules_of.contains_key(r) || facts(r) => f64::INFINITY,
                    None if r == "node" => model.stats.nodes,
                    None => model.stats.card.get(r).copied().unwrap_or(f64::INFINITY),
                }
            };
            let ndv_of = |a: AtomRef<'_>, c: usize| -> f64 {
                let r = v.rel_name(a.rel());
                match out.get(r) {
                    Some((_, cols)) => cols.get(c).copied().unwrap_or(f64::INFINITY),
                    None if rules_of.contains_key(r) || facts(r) => f64::INFINITY,
                    None => counted.get(r).and_then(|cols| cols.get(c)).copied().unwrap_or(f64::INFINITY),
                }
            };
            let width = ks.first().map_or(0, |&k| base.rule(k).head().arity());
            let (mut rows, mut cols) = (0.0f64, vec![0.0f64; width]);
            for &k in ks {
                let r = base.rule(k);
                let mut var: BTreeMap<&str, f64> = BTreeMap::new();
                let name = |s| v.text(r.var(s));
                let mut product = if r.body().any(|l| matches!(l.view(), Lit::Ast { .. })) { f64::INFINITY } else { 1.0f64 };
                for a in r.body().filter_map(|l| l.atom()).filter(|a| !a.neg()) {
                    product *= rows_of(a);
                    for (c, t) in a.args().enumerate() {
                        if let TermRef::Var(s) = t {
                            let d = ndv_of(a, c);
                            var.entry(name(s)).and_modify(|x| *x = x.min(d)).or_insert(d);
                        }
                    }
                }
                for l in r.body() {
                    if let Lit::Cmp { l: TermRef::Var(s), op: CmpOp::Eq, r: TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_) } | Lit::Cmp { l: TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_), op: CmpOp::Eq, r: TermRef::Var(s) } = l.view() {
                        var.insert(name(s), 1.0);
                    }
                }
                let head: Vec<f64> = r.head().args().map(|t| match t {
                    TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_) => 1.0,
                    TermRef::Var(s) => var.get(name(s)).copied().unwrap_or(f64::INFINITY),
                    _ => f64::INFINITY,
                }).collect();
                rows += head.iter().product::<f64>().min(product);
                for (c, d) in head.into_iter().enumerate().take(width) {
                    cols[c] += d;
                }
            }
            for c in &mut cols {
                *c = c.min(rows);
            }
            out.insert(rel.clone(), (rows, cols));
        }
    }
    out
}

/// The relations that cross the chunk boundary (the local part's outputs: its exports and findings) stored as the
/// flat join of parts that meet only at a variable they project away: `fl853_p4_outer(f, x) :- rr_inside_ks5(f, p),
/// bound_in(p, x)` holds 777 k rows, its two parts about 100 k. Such an export is handed on as its parts before the
/// first chunk (`file_local::FactorizeExports`, decision 0099); this names each relation still stored flat that would
/// shrink so (one the program's test refused: a finding, a part of one row a key, a reader binding none of it), with
/// its flat rows (observed where an earlier chunk measured them) and its parts' estimated rows, where the flat rows
/// are over twice the parts'.
fn factorizable(f: Fields<'_>, base: &Placed, ids_of: &BTreeMap<&str, Vec<usize>>, estimates: &BTreeMap<String, (f64, f64, f64)>, model: &Model) -> Vec<String> {
    use super::super::program_image::T;
    let v: &Version = &base.v;
    let mut out = Vec::new();
    for rel in v.outputs().map(|a| v.text(a)) {
        let Some(ks) = ids_of.get(rel) else { continue };
        let nodes = model.stats.nodes;
        let (flat, how) = match observed_rows(rel, nodes) {
            Some(n) => (n, "observed"),
            None => (estimates.get(rel).map_or(0.0, |e| e.0), "estimated"),
        };
        if flat < SMALL {
            continue;
        }
        let mut parts_rows = 0.0;
        let mut shown: Vec<String> = Vec::new();
        let mut every = true;
        let part_name = format!("{rel}_part");
        for &k in ks {
            let r = base.rule(k);
            let b = base.body(k);
            match factors_of(v, r) {
                Some((key, parts)) => {
                    let mut texts = Vec::new();
                    for part in &parts {
                        // The part as a rule of its own, an overlay of the program's version, planned for its rows.
                        let names: BTreeSet<&str> = part.iter().flat_map(|&i| b.vars(i).iter().map(|s| b.var(s))).collect();
                        let args: Vec<T<'_>> = names.iter().map(|x| T::Var(x)).collect();
                        let mut bd = super::super::program_image::Builder::on(&base.v);
                        let mut rb = bd.rule().head(part_name.as_str(), &args);
                        for &i in part {
                            rb = rb.lit(v, r, r.body().nth(i).expect("a literal of the part"));
                        }
                        let rows = rb.finish().ok().and_then(|id| bd.finish_placing().ok().map(|(pv, _, order)| (pv, order.map_or(id, |o| o[&id])))).and_then(|(pv, id)| {
                            let pb = Body::of(&pv, pv.rule(id).expect("the part"));
                            model.plan_rule(f, &pb, None).ok().map(|p| p.2)
                        }).unwrap_or(f64::INFINITY);
                        parts_rows += rows;
                        texts.push(format!("{} [{rows:.0}]", part.iter().map(|&i| b.show(i)).collect::<Vec<_>>().join(", ")));
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

/// Rule `r`'s body split at a variable `key` the head does not name: the parts that meet only at `key`, each naming a
/// head variable, with each comparison and negation in the part whose variables it reads; the key's name, and each
/// part's literals by place. `None` where no variable splits it so, or it holds anything but atoms and comparisons.
pub fn factors_of(v: &Version, r: super::super::program_image::RuleRef<'_>) -> Option<(String, Vec<Vec<usize>>)> {
    factors_without(v, r, None)
}

/// `factors_of` over rule `r`'s body without its literal `skip` (a request that guards the rule, which names both
/// halves of its head): the parts' literals by their places in the whole body.
pub fn factors_without(v: &Version, r: super::super::program_image::RuleRef<'_>, skip: Option<usize>) -> Option<(String, Vec<Vec<usize>>)> {
    use super::super::program_image::Lit;
    let places: Vec<usize> = (0..r.body().count()).filter(|&i| Some(i) != skip).collect();
    let all: Vec<_> = r.body().collect();
    let lits: Vec<_> = places.iter().map(|&i| all[i]).collect();
    if !lits.iter().all(|l| matches!(l.view(), Lit::Atom(_) | Lit::Cmp { .. })) {
        return None;
    }
    let name = |s| v.text(r.var(s));
    let mut head = super::body::VarSet::new();
    for t in r.head().args() {
        super::body::term_vars(t, &mut head);
    }
    let positive: Vec<usize> = (0..lits.len()).filter(|&i| matches!(lits[i].view(), Lit::Atom(a) if !a.neg())).collect();
    let vars: Vec<super::body::VarSet> = lits.iter().map(|&l| super::body::lit_vars(l)).collect();
    let numbered = Numbered::of(head.iter().map(name), vars.iter().map(|vs| vs.iter().map(name)));
    let (key, parts) = factor_parts(&numbered, &positive)?;
    let parts = parts.into_iter().map(|p| p.into_iter().map(|i| places[i]).collect()).collect();
    Some((numbered.names[key].to_owned(), parts))
}

/// A body's variables for `factor_parts`, numbered in the order of their names (so a key tried in number order is
/// tried in name order, as by name), with the head's and each literal's as sets of numbers.
struct Numbered<'a> {
    names: Vec<&'a str>,
    head: Bits,
    vars: Vec<Bits>,
}

impl<'a> Numbered<'a> {
    fn of<L: IntoIterator<Item = &'a str>>(head: impl IntoIterator<Item = &'a str>, lits: impl IntoIterator<Item = L>) -> Numbered<'a> {
        let head: Vec<&str> = head.into_iter().collect();
        let lits: Vec<Vec<&str>> = lits.into_iter().map(|l| l.into_iter().collect()).collect();
        let mut names: Vec<&str> = head.iter().chain(lits.iter().flatten()).copied().collect();
        names.sort_unstable();
        names.dedup();
        let id = |v: &str| names.binary_search(&v).expect("a body's variable");
        let n = names.len();
        let set = |vs: &[&str]| {
            let mut b = Bits::new(n);
            vs.iter().for_each(|v| b.set(id(v)));
            b
        };
        let (head, vars) = (set(&head), lits.iter().map(|l| set(l)).collect());
        Numbered { names, head, vars }
    }
}

/// A set of a body's variable numbers.
#[derive(Clone, PartialEq)]
struct Bits(Vec<u64>);

impl Bits {
    fn new(n: usize) -> Bits {
        Bits(vec![0; n.div_ceil(64)])
    }
    fn set(&mut self, i: usize) {
        self.0[i / 64] |= 1 << (i % 64);
    }
    fn unset(&mut self, i: usize) {
        self.0[i / 64] &= !(1 << (i % 64));
    }
    fn has(&self, i: usize) -> bool {
        self.0[i / 64] & (1 << (i % 64)) != 0
    }
    fn meets(&self, o: &Bits) -> bool {
        self.0.iter().zip(&o.0).any(|(a, b)| a & b != 0)
    }
    fn add(&mut self, o: &Bits) {
        self.0.iter_mut().zip(&o.0).for_each(|(a, b)| *a |= b);
    }
    fn is_empty(&self) -> bool {
        self.0.iter().all(|w| *w == 0)
    }
    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().enumerate().flat_map(|(k, &w)| (0..64).filter(move |b| w & (1 << b) != 0).map(move |b| k * 64 + b))
    }
}

/// `factors_of`'s split of a body of literals (`b.vars`, each literal's variables; `positive`, the positive atoms' places)
/// under a head naming `b.head`: the key's number, and the parts' literals by place. On numbered variables: as sets
/// of names, each key's components cloned and compared names at every step.
fn factor_parts(b: &Numbered<'_>, positive: &[usize]) -> Option<(usize, Vec<Vec<usize>>)> {
    let (head, vars) = (&b.head, &b.vars);
    let mut keys = Bits::new(b.names.len());
    for &i in positive {
        for v in vars[i].iter() {
            if !head.has(v) && positive.iter().filter(|&&j| vars[j].has(v)).count() >= 2 {
                keys.set(v);
            }
        }
    }
    for key in keys.iter() {
        // Components over every literal, joined by the variables they share other than the key.
        let mut comps: Vec<(Vec<usize>, Bits)> = Vec::new();
        for (i, vs) in vars.iter().enumerate() {
            let mut vs = vs.clone();
            vs.unset(key);
            let (joined, rest): (Vec<_>, Vec<_>) = comps.into_iter().partition(|(_, v)| v.meets(&vs));
            let mut all = (vec![i], vs);
            for (ls, more) in joined {
                all.0.extend(ls);
                all.1.add(&more);
            }
            comps = rest;
            comps.push(all);
        }
        // A literal of the key alone (`$p < …`) goes with the first part that reads the key.
        let (lone, mut parts): (Vec<_>, Vec<_>) = comps.into_iter().partition(|(_, v)| v.is_empty());
        if parts.len() < 2 || !parts.iter().all(|(ls, v)| v.meets(head) && ls.iter().any(|i| positive.contains(i))) {
            continue;
        }
        for (ls, _) in lone {
            parts[0].0.extend(ls);
        }
        let parts = parts.into_iter().map(|(mut ls, _)| {
            ls.sort_unstable();
            ls
        }).collect();
        return Some((key, parts));
    }
    None
}


/// What the native strata observed during a corpus run (`observing`): kept across its chunks and parts, so a later
/// chunk's choice prices each relation from the rows its join steps produced. The run's thread's, shared by the jobs
/// its strata run as on other threads (decision 0113, `share_observed`): each stratum observes its own rules and
/// relations.
pub(in crate::code_rule_dsl) type SharedObserved = Arc<ObservedShared>;

/// The run's observations as its jobs share them (decision 0301): the sums published (RCU: read with no lock, `None`
/// while not observing), and the sums jobs handed in as they ended, folded in by one of them every `FOLD_EVERY`, and
/// all of them before a chunk's choice reads them (`fold`). Over PostHog the sums were read 1.1 M times and added to
/// 294,000 times, each add under the readers' lock.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct ObservedShared {
    published: arc_swap::ArcSwapOption<Observed>,
    pending: parking_lot::Mutex<Vec<Observed>>,
    folding: parking_lot::Mutex<()>,
    /// A chunk's (Q88, `chunk_observed`): what its jobs handed in over the run's as filed, apart, for the run's in the
    /// chunks' order (`file_observed`).
    made: Option<parking_lot::Mutex<Observed>>,
}

/// The jobs' sums handed in before one of them folds them in: the lag of an estimate read in a job.
const FOLD_EVERY: usize = 256;


impl ObservedShared {
    fn observing(on: bool) -> Self {
        let s = ObservedShared::default();
        s.published.store(on.then(|| Arc::new(Observed::default())));
        s
    }

    /// A job's sums, as it ends: folded in once `FOLD_EVERY` are waiting, by a job that finds no fold under way.
    fn hand_in(&self, mine: Observed) {
        let waiting = {
            let mut p = self.pending.lock();
            p.push(mine);
            p.len()
        };
        if waiting >= FOLD_EVERY {
            self.fold(false);
        }
    }

    /// The sums handed in, folded into a copy of the run's and published: waiting for a fold under way (`wait`), or
    /// leaving it to that one.
    pub(super) fn fold(&self, wait: bool) {
        let _folding = if wait { self.folding.lock() } else {
            let Some(g) = self.folding.try_lock() else { return };
            g
        };
        let pending = std::mem::take(&mut *self.pending.lock());
        if pending.is_empty() {
            return;
        }
        let Some(now) = self.published.load_full() else { return };
        let mut next = Observed::clone(&now);
        for p in pending {
            if let Some(made) = &self.made {
                made.lock().add(p.clone());
            }
            next.add(p);
        }
        self.published.store(Some(Arc::new(next)));
        run_counts().folds.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// `f` on the run's sums, as a thread observing outside a job does: folded and published at once.
    fn change(&self, f: impl FnOnce(&mut Observed)) {
        let _folding = self.folding.lock();
        let Some(now) = self.published.load_full() else { return };
        let mut next = Observed::clone(&now);
        for p in std::mem::take(&mut *self.pending.lock()) {
            next.add(p);
        }
        f(&mut next);
        self.published.store(Some(Arc::new(next)));
    }
}

/// The run's observations, in the run's context: its jobs observe into them (`enter_observed`).
#[derive(Clone, Default)]
struct ObservedAt(SharedObserved);

thread_local! {
    /// A job's own observations (a thread that entered another's, `enter_observed`): its strata add to them without
    /// a lock, and they are added to the run's once, as the job ends (strands observing took turns at the run's
    /// lock, rule by rule).
    static OWN_OBSERVED: RefCell<Option<Observed>> = const { RefCell::new(None) };
}

/// A job's own observations, on the job's thread (`run_state`): what it observes is added to the run's as it ends
/// (`enter_observed`). The run's themselves are in the run's context, which the job's run carries.
pub fn carried(c: &mut crate::code_rule_dsl::logic::run_state::Capture) {
    c.part(&["OWN_OBSERVED"], crate::code_rule_dsl::logic::run_state::Shared(share_observed(), enter_observed));
}

/// Adds to this thread's job's observations where it has them (`enter_observed`), else to the run's: `f` is given
/// them, and the run's (read: `copies`) where those are this job's too.
fn observing_into(f: impl FnOnce(&mut Observed, Option<&Observed>)) {
    let f = OWN_OBSERVED.with(|own| {
        let mut own = own.borrow_mut();
        let Some(mine) = own.as_mut() else { return Some(f) };
        OBSERVED.with(|o| f(mine, o.borrow().as_deref()));
        None
    });
    if let Some(f) = f {
        OBSERVED.with(|o| o.0.change(|o| f(o, None)));
    }
}

impl Observed {
    /// Adds `other` (a job's, `OWN_OBSERVED`) to these.
    fn add(&mut self, other: Observed) {
        for (k, (a, b)) in other.rules {
            let e = self.rules.entry(k).or_default();
            e.0 += a;
            e.1 += b;
        }
        for (k, r) in other.rels {
            let e = self.rels.entry(k).or_default();
            e.rows += r.rows;
            e.nodes += r.nodes;
            if e.ndv.len() < r.ndv.len() {
                e.ndv.resize(r.ndv.len(), (0.0, 0.0));
                e.fan.resize(r.fan.len().max(r.ndv.len()), (0.0, 0.0));
            }
            for (c, (x, y)) in r.ndv.into_iter().enumerate() {
                e.ndv[c].0 += x;
                e.ndv[c].1 += y;
            }
            for (c, (x, y)) in r.fan.into_iter().enumerate() {
                e.fan[c].0 += x;
                e.fan[c].1 += y;
            }
        }
        for (k, [a, b]) in other.steps {
            let e = self.steps.entry(k).or_default();
            e[0] += a;
            e[1] += b;
        }
        for (k, [a, b]) in other.pooled {
            let e = self.pooled.entry(k).or_default();
            e[0] += a;
            e[1] += b;
        }
        self.copies.extend(other.copies);
        for (k, (a, b)) in other.inlined {
            let e = self.inlined.entry(k).or_default();
            e.0 += a;
            e.1 += b;
        }
    }
}

/// The observations as last published (decision 0301): read with no lock.
struct ObservedCell(SharedObserved);
impl ObservedCell {
    fn borrow(&self) -> arc_swap::Guard<Option<Arc<Observed>>> {
        self.0.published.load()
    }
}
struct ObservedKey;
impl ObservedKey {
    fn with<R>(&self, f: impl FnOnce(&ObservedCell) -> R) -> R {
        f(&ObservedCell(share_observed()))
    }
}
static OBSERVED: ObservedKey = ObservedKey;

/// A chunk's observations (Q88): the run's as filed when it began (`published`), what it observes over them, and that
/// apart (`observed_made`).
pub(in crate::code_rule_dsl) fn chunk_observed(published: Option<Arc<Observed>>) -> SharedObserved {
    let s = ObservedShared { made: Some(Default::default()), ..Default::default() };
    s.published.store(published);
    Arc::new(s)
}

/// What a chunk observed (`chunk_observed`), every job's handed in.
pub(in crate::code_rule_dsl) fn observed_made(chunk: &SharedObserved) -> Observed {
    chunk.fold(true);
    chunk.made.as_ref().map(|m| std::mem::take(&mut *m.lock())).unwrap_or_default()
}

/// The observations as last published: `None` while not observing.
pub(in crate::code_rule_dsl) fn observed_now(s: &SharedObserved) -> Option<Arc<Observed>> {
    s.published.load_full()
}

/// Adds a chunk's observations to the run's, as the main thread files it (Q88: in the chunks' order).
pub(in crate::code_rule_dsl) fn file_observed(run: &SharedObserved, made: Observed) {
    run.change(|o| o.add(made));
}

/// Makes `observed` the run's observations while the guard lives (a chunk's, Q88: `plan::in_view`), for this thread
/// and the jobs it starts, which carry the context.
pub(in crate::code_rule_dsl) fn bind_observed(observed: SharedObserved) -> crate::code_rule_dsl::logic::profile::Bound {
    crate::code_rule_dsl::logic::profile::bind(ObservedAt(observed))
}

/// The run's observations (`ObservedAt`).
pub(in crate::code_rule_dsl) fn share_observed() -> SharedObserved {
    crate::code_rule_dsl::logic::profile::bound::<ObservedAt>().0
}

/// While the guard lives, this thread's job observes into observations of its own, added to `to` (the run's, its
/// caller's `share_observed`) as it ends.
pub(in crate::code_rule_dsl) fn enter_observed(to: SharedObserved) -> impl Drop {
    struct Entered(Option<SharedObserved>, Option<Observed>);
    impl Drop for Entered {
        fn drop(&mut self) {
            let mine = OWN_OBSERVED.with(|o| std::mem::replace(&mut *o.borrow_mut(), self.1.take()));
            if let (Some(mine), Some(to)) = (mine, self.0.take()) {
                to.hand_in(mine);
            }
        }
    }
    let own = to.published.load().is_some().then(Observed::default);
    Entered(Some(to), OWN_OBSERVED.with(|o| std::mem::replace(&mut *o.borrow_mut(), own)))
}

/// While the guard lives, the run reads and observes into `chunk`'s observations (`chunk_observed`, Q88): bound in
/// the run's context for the extent, so its jobs do too, and this thread's own added to them as it ends.
pub(in crate::code_rule_dsl) fn enter_chunk_observed(chunk: SharedObserved) -> impl Sized {
    // The thread's own observations end first, inside the binding (fields drop in order).
    struct In<E> {
        _own: E,
        _bound: crate::code_rule_dsl::logic::profile::Bound,
    }
    let bound = crate::code_rule_dsl::logic::profile::bind(ObservedAt(chunk.clone()));
    In { _own: enter_observed(chunk), _bound: bound }
}

/// Observed join work and relations, each with the corpus nodes of the runs that observed it (summed), so a later
/// chunk reads a rate per node: per rule (`obs_rule`: its head and body), the rows into and out of every step of every
/// probe of it (the planner's units of work: `native.tsv`'s `steps_in_out`); per relation, its rows when its stratum
/// ended and per column its distinct values and fanout (`Rel`).
#[derive(Default, Clone)]
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
#[derive(Default, Clone)]
struct Rel {
    rows: f64,
    nodes: f64,
    ndv: Vec<(f64, f64)>,
    fan: Vec<(f64, f64)>,
}

/// A relation when its stratum ended (`observe`): its rows, and per column its distinct values and its fanout, where
/// the live counts measured them.
pub struct Seen<'a> {
    pub rel: &'a str,
    pub rows: f64,
    pub cols: Vec<(Option<f64>, Option<f64>)>,
}

/// Observations kept while held (a corpus run, `KeepPlans`); none outside one. `RERAIL_OBSERVED_STEPS=off` keeps none
/// (every choice from the plan's estimates), for comparison.
pub fn observing(on: bool) {
    let on = on && !std::env::var("RERAIL_OBSERVED_STEPS").is_ok_and(|v| v == "off");
    // A new handle: what the last run's jobs handed in stays with theirs.
    crate::code_rule_dsl::logic::profile::bind_here(ObservedAt(Arc::new(ObservedShared::observing(on))));
}

/// A stratum's native run: per rule, the (in, out) rows of each step of each of its probes, and each relation it
/// made, over a corpus of `nodes`.
pub fn observe(rules: &[(u64, &[(u64, u64)])], rels: &[Seen], nodes: f64) {
    observing_into(|o, run| {
        let mut per: BTreeMap<u64, f64> = BTreeMap::new();
        for (rule, steps) in rules {
            *per.entry(*rule).or_default() += steps.iter().map(|(a, b)| (a + b) as f64).sum::<f64>();
        }
        let mut through: BTreeMap<(String, u64), f64> = BTreeMap::new();
        for (rule, work) in per {
            let e = o.rules.entry(rule).or_default();
            e.0 += work;
            e.1 += nodes.max(1.0);
            if let Some((reader, into)) = o.copies.get(&rule).or_else(|| run.and_then(|r| r.copies.get(&rule))) {
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
fn pattern(b: &super::body::Body<'_>, i: usize, bound: &super::body::VarSet) -> Option<String> {
    use super::super::program_image::{Lit, TermRef};
    let term = |t: TermRef<'_>| match t {
        TermRef::Var(v) if bound.contains(v) => "_".to_owned(),
        TermRef::Var(_) => "?".to_owned(),
        TermRef::Wild => "*".to_owned(),
        t => b.v.show_term(b.rule, t),
    };
    Some(match b.view(i) {
        Lit::Atom(a) => {
            let (rel, cols) = b.sig(i, bound);
            format!("{}{rel}{cols:?}", if a.neg() { "!" } else { "" })
        }
        Lit::Cmp { l, op, r } => format!("{} {} {}", term(l), op.as_str(), term(r)),
        Lit::Nearest { n, kindset, .. } => format!("nearest {} {}", b.v.text(kindset), if bound.contains(n) { "climb" } else { "descend" }),
        _ => return None,
    })
}

/// A native stratum's steps for one plan of `rule` (`order`), each with its rows (in, out): kept per pattern.
pub fn observe_steps(rule: u64, order: super::Order<'_, '_>, counts: &[(i64, i64)]) {
    let b = order.b;
    observing_into(|o, _| {
        let mut bound = super::body::VarSet::new();
        for (i, (s, &(n_in, n_out))) in order.steps.iter().zip(counts).enumerate() {
            if i > 0
                && n_in > 0
                && let Some(p) = pattern(b, s.at, &bound)
            {
                for e in [o.steps.entry((rule, p.clone())).or_default(), o.pooled.entry(p).or_default()] {
                    e[0] += n_in as f64;
                    e[1] += n_out as f64;
                }
            }
            if !b.plain_atom(s.at).is_some_and(|a| a.neg()) {
                bound.extend(b.vars(s.at));
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
pub fn calibrated(f: Fields<'_>, model: &Model, rule: &Body<'_>) -> Option<f64> {
    calibrated_at(f, model, rule, None)
}

/// `calibrated`, the plan read with body literal `first` first (a delta plan's), else whole.
fn calibrated_at(f: Fields<'_>, model: &Model, rule: &Body<'_>, first: Option<usize>) -> Option<f64> {
    calibrated_rows_at(f, model, rule, first).map(|(w, _)| w)
}

/// `calibrated`, and the rows the plan expects.
pub fn calibrated_rows(f: Fields<'_>, model: &Model, rule: &Body<'_>) -> Option<(f64, f64)> {
    calibrated_rows_at(f, model, rule, None)
}

/// `calibrated_rows`, with body literal `first` read first.
fn calibrated_rows_at(f: Fields<'_>, model: &Model, b: &Body<'_>, first: Option<usize>) -> Option<(f64, f64)> {
    // The rows are the steps' (this model's statistics, calibrated), as the work is: not the plan's own `out`, which
    // for a plan taken as prepared (decision 0361 step 5) is the facts-only model's at its nominal scale (`asgstmt`
    // priced at 0 rows where its steps make 146, and every relation built on it then joined as a product).
    let ((order, _, _), step_rows) = model.plan_rule_step_rows(f, b, first).ok()?;
    if step_rows.len() != order.len() {
        return None;
    }
    let key = b.observed_key();
    OBSERVED.with(|o| {
        let o = o.borrow();
        let seen = |p: &str| -> Option<f64> {
            let o = o.as_ref()?;
            let e = o.steps.get(&(key, p.to_owned())).filter(|e| e[0] >= STEP_MIN).or_else(|| o.pooled.get(p).filter(|e| e[0] >= STEP_MIN))?;
            Some(e[1] / e[0])
        };
        let mut bound = super::body::VarSet::new();
        let (mut rows, mut work, mut before) = (1.0f64, 0.0f64, 1.0f64);
        for (i, (s, &st_rows)) in order.iter().zip(&step_rows).enumerate() {
            let estimated = if before > 0.0 { st_rows / before } else { 1.0 };
            before = st_rows;
            let factor = if i == 0 { st_rows } else { pattern(b, s.at, &bound).and_then(|p| seen(&p)).unwrap_or(estimated) };
            let out = rows * factor;
            // The rows each step passes on, as the executor counts a line's work (decision 0297).
            work += out;
            rows = out;
            if !b.plain_atom(s.at).is_some_and(|a| a.neg()) {
                bound.extend(b.vars(s.at));
            }
        }
        Some((work, rows))
    })
}

/// `rule`'s observed work, scaled to a corpus of `nodes`.
fn observed_rule(rule: &Body<'_>, nodes: f64) -> Option<f64> {
    let key = rule.observed_key();
    OBSERVED.with(|o| o.borrow().as_ref()?.rules.get(&key).map(|(w, n)| w / n * nodes.max(1.0)))
}

/// The rules inlining made, each as (its hash, the hash of the reader it was unfolded from, the relations unfolded
/// into it): what they do when they run is the inlined side of those relations' readers (`observed_copies`).
fn copied<'a>(made: impl Iterator<Item = (u64, u64, &'a BTreeSet<String>)>) {
    observing_into(|o, _| {
        for (rule, reader, into) in made {
            o.copies.insert(rule, (reader, into.clone()));
        }
    });
}

/// What `reader`'s copies did with `rel` inlined, scaled to a corpus of `nodes`.
fn observed_copies(rel: &str, reader: &Body<'_>, nodes: f64) -> Option<f64> {
    let key = (rel.to_owned(), reader.observed_key());
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
fn overlay<'a>(f: Fields<'_>, model: &mut Model, rels: impl Iterator<Item = &'a str>) -> Saved {
    let nodes = model.stats.nodes;
    let mut saved = Vec::new();
    for rel in rels {
        let Some(rows) = observed_rows(rel, nodes) else { continue };
        let width = f.arity(rel).unwrap_or(0);
        let cols = observed_cols(rel, nodes, rows);
        let before = (0..width).map(|c| (model.ndv.at(rel, c).copied(), model.stats.fan.at(rel, c).copied())).collect();
        saved.push((rel.to_owned(), model.stats.card.get(rel).copied(), before));
        model.stats.card.insert(rel, rows.max(1.0));
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
pub struct Inserted {
    card: Vec<String>,
    ndv: Vec<(String, usize)>,
    fan: Vec<(String, usize)>,
}

/// Each derived relation's estimated (rows, work to build it, rows as planned), in evaluation order; each estimate is the planner's
/// statistics while its readers are priced (`unestimate` takes them back). A relation an earlier chunk or part measured
/// keeps its measured rows, and its columns' measured distinct values and fanouts: a column's distinct values are
/// otherwise guessed (the least of the body columns that bind it, or the rows), and a guess prices a probe of it
/// wrongly in either direction (0099, "The regression").
pub fn estimate(f: Fields<'_>, model: &mut Model, (base, ids_of): (&Placed, &BTreeMap<&str, Vec<usize>>), sccs: &[BTreeSet<String>], need: &BTreeSet<&str>, built: &BTreeSet<&str>) -> (BTreeMap<String, (f64, f64, f64)>, Inserted) {
    let mut out = BTreeMap::new();
    let mut inserted = Inserted { card: Vec::new(), ndv: Vec::new(), fan: Vec::new() };
    // The relations estimated, in evaluation order. Each is planned with the estimates of those before it, so its
    // rules are planned on the pool's threads beside every other relation of its level: one more than the levels of
    // the relations before it that its rules read. A relation a rule reads before it is estimated (a recursive
    // component's) takes a level after that rule's, so every plan reads the estimates the order gave it.
    use rayon::prelude::*;
    use super::super::program_image::{Lit, TermRef};
    let v: &Version = &base.v;
    let order: Vec<(&String, &Vec<usize>)> = sccs.iter().flatten().filter_map(|rel| ids_of.get(rel.as_str()).filter(|_| need.contains(rel.as_str())).map(|ks| (rel, ks))).collect();
    let at: std::collections::HashMap<&str, usize> = order.iter().enumerate().map(|(i, (rel, _))| (rel.as_str(), i)).collect();
    let (mut level, mut floor) = (vec![0usize; order.len()], vec![0usize; order.len()]);
    for (i, (_, ks)) in order.iter().enumerate() {
        let reads: Vec<usize> = ks.iter().flat_map(|&k| base.rule(k).body()).filter_map(|l| l.atom().and_then(|a| at.get(v.rel_name(a.rel())).copied())).collect();
        level[i] = reads.iter().filter(|&&j| j < i).map(|&j| level[j] + 1).fold(floor[i], usize::max);
        for &j in reads.iter().filter(|&&j| j > i) {
            floor[j] = floor[j].max(level[i] + 1);
        }
    }
    let mut levels: Vec<Vec<usize>> = vec![Vec::new(); level.iter().max().map_or(0, |m| m + 1)];
    for (i, &l) in level.iter().enumerate() {
        levels[l].push(i);
    }
    let mut planned: Vec<(f64, f64)> = vec![(0.0, 0.0); order.len()];
    let nodes = model.stats.nodes;
    for each in &levels {
        // A relation measured, whose build is not priced, takes its measured rows and columns without planning.
        let measured_only: Vec<bool> = each.iter().map(|&i| {
            let rel = order[i].0;
            !built.contains(rel.as_str()) && (model.stats.card.contains_key(rel) || observed_rows(rel, nodes).is_some())
        }).collect();
        let model_now: &Model = model;
        // In the run, on whichever thread prices a relation: its plans read the run's and the object's statistics.
        let carry = crate::code_rule_dsl::logic::run_state::Carry::here();
        let sums: Vec<(f64, f64)> = super::super::profile::on_cores(|| {
            each.par_iter().zip(&measured_only).map(|(&i, &measured_only)| {
                let _in = carry.enter();
                let (mut rows, mut work) = (0.0, 0.0);
                for &k in order[i].1.iter().filter(|_| !measured_only) {
                    if let Some((w, n)) = calibrated_rows(f, model_now, &base.body(k)) {
                        rows += n;
                        work += w;
                    }
                }
                (rows, work)
            }).collect()
        });
        for (&i, sum) in each.iter().zip(sums) {
            planned[i] = sum;
        }
        for &i in each {
            let (rel, ks) = order[i];
            let (rows, work) = planned[i];
            // Observed on an earlier chunk or part of this run (`Observed`), per node of the corpus: its rows when
            // it was stored, and its build's work when every rule of it ran.
            let measured = model.stats.card.get(rel).copied().or_else(|| observed_rows(rel, nodes));
            let planned = rows;
            let rows = measured.unwrap_or(rows);
            if let Ok(t) = std::env::var("RERAIL_KEEPS_TRACE")
                && rel.contains(t.as_str())
            {
                let reads: Vec<String> = ks
                    .iter()
                    .flat_map(|&k| base.rule(k).body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel()).to_owned()).collect::<Vec<_>>())
                    .map(|r| {
                        let w = v.decl_of(&r).map_or(0, |d| d.cols().len());
                        let ndv: Vec<String> = (0..w).map(|c| format!("{:?}", model.ndv.at(&r, c))).collect();
                        format!("{r}={:?} ndv[{}]", model.stats.card.get(&r), ndv.join(","))
                    })
                    .collect();
                eprintln!("KEEPS_TRACE {rel}: rows {rows:.0} work {work:.0} planned {planned:.0} measured {measured:?} nodes {nodes}; reads {}", reads.join(" "));
                for &k in ks.iter() {
                    if let Ok(((order, _, _), steps)) = model.plan_rule_step_rows(f, &base.body(k), None) {
                        let lits: Vec<String> = order.iter().zip(&steps).map(|(s, n)| format!("#{}={n:.1}", s.at)).collect();
                        eprintln!("KEEPS_TRACE   rule {k}: {}", lits.join(" "));
                    }
                }
            }
            out.insert(rel.clone(), (rows, work, planned));
            if !model.stats.card.contains_key(rel) {
                model.stats.card.insert(rel.clone(), rows.max(1.0));
                inserted.card.push(rel.clone());
                // A column's distinct values: at most the rows, and at most those of the body column that binds it.
                let width = v.decl_of(rel).map_or(0, |d| d.cols().len());
                let seen = observed_cols(rel, nodes, rows);
                for c in 0..width {
                    let (seen_ndv, seen_fan) = seen.get(c).copied().unwrap_or((None, None));
                    if let Some(f) = seen_fan
                        && model.stats.fan.at(rel, c).is_none()
                    {
                        model.stats.fan.insert((rel.clone(), c), f);
                        inserted.fan.push((rel.clone(), c));
                    }
                    if model.ndv.at(rel, c).is_some() {
                        continue;
                    }
                    if let Some(d) = seen_ndv {
                        model.ndv.insert((rel.clone(), c), d);
                        inserted.ndv.push((rel.clone(), c));
                        continue;
                    }
                    // The relation's values in column `c` are its rules' together: at most the sum of each rule's,
                    // and each rule's at most those of the body columns that bind it. A constant is one value; a
                    // recursive rule that passes the column on from the relation itself adds none. (The minimum
                    // over every rule's columns took one rule's few values for the relation's: `ast_name`'s 2,068
                    // from `rr_long_text_node`, `assign_target`'s 1 from an empty base case, and joins on them were
                    // priced as products.)
                    let mut sum = 0.0f64;
                    for &k in ks {
                        let r = base.rule(k);
                        sum += match r.head().args().nth(c) {
                            Some(TermRef::Var(x)) => {
                                let (mut bound, mut passed) = (f64::INFINITY, false);
                                for l in r.body() {
                                    if let Lit::Atom(a) = l.view()
                                        && !a.neg()
                                        && let Some(at) = a.args().position(|t| t.var() == Some(x))
                                    {
                                        if a.rel() == r.head().rel() && at == c {
                                            passed = true;
                                        } else if let Some(&d) = model.ndv.at(v.rel_name(a.rel()), at) {
                                            bound = bound.min(d);
                                        }
                                    }
                                }
                                if passed { 0.0 } else { bound }
                            }
                            Some(TermRef::Wild) | None => f64::INFINITY,
                            Some(_) => 1.0,
                        };
                    }
                    let ndv = rows.min(sum);
                    model.ndv.insert((rel.clone(), c), ndv.max(1.0));
                    inserted.ndv.push((rel.clone(), c));
                }
            }
        }
    }
    (out, inserted)
}

pub fn unestimate(model: &mut Model, inserted: Inserted) {
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

// cratify: `tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus)

/// The run's counts of this module's work (decision 0442: a run's own, `run::local`).
#[derive(Default)]
pub struct KeepsCounts {
    /// Folds of the observed sums (decision 0301), for the profile.
    pub folds: std::sync::atomic::AtomicU64,
}

/// The run's `KeepsCounts`.
#[inline]
pub fn run_counts() -> std::sync::Arc<KeepsCounts> {
    crate::code_rule_dsl::logic::run::local::<KeepsCounts>()
}
