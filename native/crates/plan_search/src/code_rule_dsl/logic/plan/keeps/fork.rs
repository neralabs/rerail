//! The design's S1 as one pass a name (the page "S1 Per-Name Pass", 2026-11-27), the planner's rewrite. A functional
//! chain `r` keyed by `K` (`chain`), read by two or more rules that each bind `K` from the same atom `D`
//! (`nameref(n, x)` for `wstart(n, s)`), gets one relation for the row they share,
//!
//! ```text
//! F(d…, v…) :- D(d…), r(k…, v…).        // r's rules unfolded into it: r is made nowhere else
//! ```
//!
//! and each reader reads `F(…)` in place of its `D` and its `r`. Then `r`'s value is made once a driver row, in
//! `F`'s rules, not once a reader. Structure only, never a name. `RERAIL_SHAPE_PER_NAME=off` turns it off.

use super::super::super::program_image::{AtomRef, Builder, Lit, LitRef, RelId, RuleId, RuleRef, T, TermRef, Version};
use super::super::body::Placed;
use super::{chain, inline, latest};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Whether the pass is on (`RERAIL_SHAPE_PER_NAME`, on by default).
pub fn on() -> bool {
    !std::env::var("RERAIL_SHAPE_PER_NAME").is_ok_and(|v| v == "off")
}

/// The fork of a chain: its relation and key, the driver's relation and, per key column, the driver's column that
/// binds it; the readers' rules.
#[derive(Debug)]
struct Fork {
    r: RelId,
    key: Vec<usize>,
    d: RelId,
    at: Vec<usize>,
    readers: Vec<RuleId>,
}

/// A term of rule `r` as the builder writes it.
fn term<'a>(v: &'a Version, r: RuleRef<'_>, t: TermRef<'_>) -> Option<T<'a>> {
    Some(match t {
        TermRef::Var(s) => T::Var(v.text(r.var(s))),
        TermRef::Wild => T::Wild,
        TermRef::Int(n) => T::Int(n),
        TermRef::Str(a) => T::Str(v.text(a)),
        TermRef::Branded(b) => T::Branded(b),
        _ => return None,
    })
}

/// Chain `rel`'s fork (`Fork`), where it has one: every read positive, two or more reader rules each reading it once
/// with its key's columns plain variables, each with a positive atom of one relation `D` that binds them at the same
/// columns, `D` not `rel` and not read twice in the rule.
fn fork_of(v: &Version, rel: RelId, key: &[usize]) -> Option<Fork> {
    let reads: Vec<(RuleRef<'_>, AtomRef<'_>)> = v.readers(rel).collect();
    let mut ids: Vec<RuleId> = reads.iter().map(|(r, _)| r.id()).collect();
    ids.sort();
    ids.dedup();
    if ids.len() < 2 || ids.len() != reads.len() || reads.iter().any(|(r, a)| a.neg() || r.head().rel() == rel) {
        return None;
    }
    let mut d: Option<(RelId, Vec<usize>)> = None;
    for (r, a) in &reads {
        let args: Vec<TermRef<'_>> = a.args().collect();
        let kv: Vec<_> = key.iter().map(|&c| args.get(c).and_then(|t| t.var())).collect::<Option<Vec<_>>>()?;
        // The driver: a positive atom of another relation binding every key variable.
        let found: Vec<(RelId, Vec<usize>)> = r
            .body()
            .filter_map(|l| match l.view() {
                Lit::Atom(b) if !b.neg() && b.rel() != rel => {
                    let bs: Vec<TermRef<'_>> = b.args().collect();
                    let at: Option<Vec<usize>> = kv.iter().map(|s| bs.iter().position(|t| t.var() == Some(*s))).collect();
                    at.map(|at| (b.rel(), at))
                }
                _ => None,
            })
            .collect();
        let pick = match &d {
            None => found.into_iter().next(),
            Some(want) => found.into_iter().find(|f| f == want),
        }?;
        // One atom of that relation in the rule.
        if r.body().filter(|l| matches!(l.view(), Lit::Atom(b) if b.rel() == pick.0)).count() != 1 {
            return None;
        }
        d.get_or_insert(pick);
    }
    let (d, at) = d?;
    // No fork where a reader makes the driver: `walk`'s recursive rules read `scope_up(s, p)` beside `walk(n, x, s)`,
    // and the fork stored `pn_scope_up`, a copy of every `walk` row and its parent, in the recursion (E7: 7,629 units of
    // `walk` and 2,306 of `pn_scope_up`, against 3,241 of `walk` alone, the same rows).
    if reads.iter().any(|(r, _)| r.head().rel() == d) {
        return None;
    }
    Some(Fork { r: rel, key: key.to_vec(), d, at, readers: ids })
}

/// The program with each fork made (`Fork`): the relations whose value is now made once a driver row, and the
/// version, an overlay of `base`'s. `None` where none forks.
pub fn fork(base: &Placed) -> Result<Option<(Vec<String>, Placed)>, String> {
    if !on() {
        return Ok(None);
    }
    let v: &Version = &base.v;
    let chains = chain::of(base);
    let forks: Vec<Fork> = chains.iter().filter_map(|(_, c)| fork_of(v, c.rel, &c.key)).collect();
    // One fork a rule: a reader of two chains keeps the first.
    let mut taken: BTreeSet<RuleId> = BTreeSet::new();
    let forks: Vec<Fork> = forks.into_iter().filter(|f| f.readers.iter().all(|r| !taken.contains(r)) && { taken.extend(f.readers.iter().copied()); true }).collect();
    if forks.is_empty() {
        return Ok(None);
    }
    let mut b = Builder::on(&base.v);
    let mut order: Vec<RuleId> = base.order().to_vec();
    let mut moved: BTreeMap<RuleId, RuleId> = BTreeMap::new();
    let mut made: Vec<(RuleId, RuleId)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for f in &forks {
        let rname = v.rel_name(f.r).to_owned();
        let dname = v.rel_name(f.d).to_owned();
        let (Some(rd), Some(dd)) = (v.decl_of(&rname), v.decl_of(&dname)) else { continue };
        let rcols: Vec<(String, String)> = rd.cols().map(|(n, t)| (v.text(n).to_owned(), v.text(t).to_owned())).collect();
        let dcols: Vec<(String, String)> = dd.cols().map(|(n, t)| (v.text(n).to_owned(), v.text(t).to_owned())).collect();
        // F's columns: the driver's, then r's other than its key.
        let rest: Vec<usize> = (0..rcols.len()).filter(|c| !f.key.contains(c)).collect();
        let fname = format!("pn_{rname}");
        if v.rel(&fname).is_some() {
            continue;
        }
        let mut fcols: Vec<(String, String)> = dcols.iter().enumerate().map(|(i, (_, t))| (format!("d{i}"), t.clone())).collect();
        fcols.extend(rest.iter().map(|&c| (format!("v{c}"), rcols[c].1.clone())));
        b.fields().declare(&fname, &fcols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect::<Vec<_>>());
        // F's rule: D with a variable a column, r keyed by D's columns `at`.
        let dv: Vec<String> = (0..dcols.len()).map(|i| format!("pn_d{i}")).collect();
        let rv: Vec<String> = (0..rcols.len()).map(|c| match f.key.iter().position(|&k| k == c) {
            Some(i) => dv[f.at[i]].clone(),
            None => format!("pn_v{c}"),
        }).collect();
        let head: Vec<T<'_>> = dv.iter().map(|x| T::Var(x)).chain(rest.iter().map(|&c| T::Var(rv[c].as_str()))).collect();
        let fid = b.rule().head(fname.as_str(), &head).atom(dname.as_str(), &dv.iter().map(|x| T::Var(x)).collect::<Vec<_>>()).atom(rname.as_str(), &rv.iter().map(|x| T::Var(x)).collect::<Vec<_>>()).finish()?;
        // Each reader: F in place of its D and its r.
        for &rid in &f.readers {
            let r = v.rule(rid).expect("a reader");
            let lits: Vec<LitRef<'_>> = r.body().collect();
            let (Some(dl), Some(rl)) = (
                lits.iter().find(|l| matches!(l.view(), Lit::Atom(a) if a.rel() == f.d)),
                lits.iter().find(|l| matches!(l.view(), Lit::Atom(a) if a.rel() == f.r)),
            ) else { return Ok(None) };
            let (Lit::Atom(da), Lit::Atom(ra)) = (dl.view(), rl.view()) else { unreachable!() };
            let dargs: Vec<TermRef<'_>> = da.args().collect();
            let rargs: Vec<TermRef<'_>> = ra.args().collect();
            let fargs: Option<Vec<T<'_>>> = dargs.iter().map(|&t| term(v, r, t)).chain(rest.iter().map(|&c| term(v, r, rargs[c]))).collect();
            let Some(fargs) = fargs else { return Ok(None) };
            let mut rb = b.rule().head_from(v, r, r.head());
            let mut placed = false;
            for l in &lits {
                if l.node() == dl.node() || l.node() == rl.node() {
                    if !placed {
                        rb = rb.atom(fname.as_str(), &fargs);
                        placed = true;
                    }
                    continue;
                }
                rb = rb.lit(v, r, *l);
            }
            let new = rb.finish()?;
            b.remove(rid);
            moved.insert(rid, new);
        }
        made.push((f.readers[0], fid));
        names.push(rname);
    }
    if names.is_empty() {
        return Ok(None);
    }
    let (v2, _, placing) = b.finish_placing()?;
    let place = |id: RuleId| placing.as_ref().map_or(id, |p| p[&id]);
    // F's rule before its first reader; each reader where it was.
    let mut seq: Vec<RuleId> = Vec::new();
    for id in order.drain(..) {
        if let Some((_, fid)) = made.iter().find(|(first, _)| *first == id) {
            seq.push(place(*fid));
        }
        seq.push(place(moved.get(&id).copied().unwrap_or(id)));
    }
    // r's rules unfolded into F, its one reader now; the driver's reads a key makes one merged.
    let chosen: BTreeSet<String> = names.iter().cloned().collect();
    let got = inline::inline(&v2, &seq, &chosen)?;
    let layouts = super::super::super::program_image::manager::analysis::<super::super::super::program_image::layouts::Layouts>(&got.v).ok();
    let keys = |rel: &str| -> Vec<Vec<usize>> {
        let (Some(l), Some(id)) = (layouts.as_deref(), got.v.rel(rel)) else { return Vec::new() };
        l.of(id).filter_map(|x| match x {
            crate::code_rule_dsl::logic::front::LayoutFact::Key(cols) => Some(cols.iter().copied().collect()),
            _ => None,
        }).collect()
    };
    let mut cur = Arc::clone(&got.v);
    let mut order: Vec<RuleId> = got.rules.iter().map(|(id, _, _)| *id).collect();
    let mut todo: BTreeSet<RuleId> = got.rules.iter().filter(|(_, _, into)| into.iter().any(|r| chosen.contains(r))).map(|(id, _, _)| *id).collect();
    for _ in 0..4 {
        let mut b = Builder::on(&cur);
        let mut moved: BTreeMap<RuleId, RuleId> = BTreeMap::new();
        for &id in &todo {
            let Some(r) = cur.rule(id) else { continue };
            if let Some(new) = latest::merged(&mut b, &cur, r, &keys)? {
                b.remove(id);
                moved.insert(id, new);
            }
        }
        if moved.is_empty() {
            break;
        }
        let (v3, _, placing) = b.finish_placing()?;
        let place = |id: RuleId| placing.as_ref().map_or(id, |p| p[&id]);
        for id in order.iter_mut() {
            *id = place(moved.get(id).copied().unwrap_or(*id));
        }
        todo = moved.values().map(|&id| place(id)).collect();
        cur = v3;
    }
    let done: Vec<String> = names.into_iter().filter(|n| !got.unfinished.contains(n)).collect();
    super::super::super::profile::decision("one pass a name (the design's S1)", || format!("{} chains made once a driver row: {}", done.len(), done.join(", ")));
    Ok(Some((done, Placed::new(cur, order, base.whole))))
}

#[cfg(test)]
mod tests {
    use super::super::super::super::program_image::{Builder, T};

    /// A chain keyed by `s` read by two rules beside one driver atom `d(s)` forks; read by a recursive relation's two
    /// rules beside its own atom (`walk(n, x, s)` reading `up(s, p)`, as `walk` reads `scope_up`), it does not: the
    /// fork would store a copy of the recursion's rows.
    #[test]
    fn a_chain_a_recursive_reader_drives_is_not_forked() {
        let mut b = Builder::base();
        for (rel, cols) in [
            ("d", &[("s", "node")][..]),
            ("c", &[("s", "node")]),
            ("up", &[("s", "node"), ("p", "node")]),
            ("a", &[("s", "node"), ("p", "node")]),
            ("b", &[("s", "node"), ("p", "node")]),
            ("start", &[("n", "node"), ("s", "node")]),
            ("walk", &[("n", "node"), ("s", "node")]),
        ] {
            b.fields().declare(rel, cols);
        }
        let v = |x: &'static str| T::Var(x);
        b.rule().head("a", &[v("s"), v("p")]).atom("d", &[v("s")]).atom("up", &[v("s"), v("p")]).finish().unwrap();
        b.rule().head("b", &[v("s"), v("p")]).atom("d", &[v("s")]).atom("up", &[v("s"), v("p")]).atom("c", &[v("s")]).finish().unwrap();
        b.rule().head("walk", &[v("n"), v("s")]).atom("start", &[v("n"), v("s")]).finish().unwrap();
        b.rule().head("walk", &[v("n"), v("p")]).atom("walk", &[v("n"), v("s")]).atom("up", &[v("s"), v("p")]).finish().unwrap();
        b.rule().head("walk", &[v("n"), v("p")]).atom("walk", &[v("n"), v("s")]).atom("up", &[v("s"), v("p")]).atom("c", &[v("s")]).finish().unwrap();
        let ver = b.finish().unwrap().0;
        let up = ver.rel("up").unwrap();
        // Read by a, b and walk's two rules: the drivers differ (d, walk), so no fork of them all.
        assert!(super::fork_of(&ver, up, &[0]).is_none());
        // Read by walk's two rules alone: walk drives them and makes them.
        let mut b = Builder::base();
        for (rel, cols) in [("c", &[("s", "node")][..]), ("up", &[("s", "node"), ("p", "node")]), ("start", &[("n", "node"), ("s", "node")]), ("walk", &[("n", "node"), ("s", "node")])] {
            b.fields().declare(rel, cols);
        }
        b.rule().head("walk", &[v("n"), v("s")]).atom("start", &[v("n"), v("s")]).finish().unwrap();
        b.rule().head("walk", &[v("n"), v("p")]).atom("walk", &[v("n"), v("s")]).atom("up", &[v("s"), v("p")]).finish().unwrap();
        b.rule().head("walk", &[v("n"), v("p")]).atom("walk", &[v("n"), v("s")]).atom("up", &[v("s"), v("p")]).atom("c", &[v("s")]).finish().unwrap();
        let ver = b.finish().unwrap().0;
        assert!(super::fork_of(&ver, ver.rel("up").unwrap(), &[0]).is_none(), "a recursive reader drives the chain");
        // The near miss: two rules of other relations beside d(s) fork.
        let mut b = Builder::base();
        for (rel, cols) in [("d", &[("s", "node")][..]), ("c", &[("s", "node")]), ("up", &[("s", "node"), ("p", "node")]), ("a", &[("s", "node"), ("p", "node")]), ("b", &[("s", "node"), ("p", "node")])] {
            b.fields().declare(rel, cols);
        }
        b.rule().head("a", &[v("s"), v("p")]).atom("d", &[v("s")]).atom("up", &[v("s"), v("p")]).finish().unwrap();
        b.rule().head("b", &[v("s"), v("p")]).atom("d", &[v("s")]).atom("up", &[v("s"), v("p")]).atom("c", &[v("s")]).finish().unwrap();
        let ver = b.finish().unwrap().0;
        let f = super::fork_of(&ver, ver.rel("up").unwrap(), &[0]).expect("two readers beside d(s) fork");
        assert_eq!((ver.rel_name(f.d), f.at.as_slice(), f.readers.len()), ("d", &[0][..], 2));
    }
}
