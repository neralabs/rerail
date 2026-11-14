//! `translate.py`'s passes over helper relations' metavariables: `share_metavariables` (a column for each
//! metavariable that crosses a helper's boundary), `passed_in_patterns`, `drop_dead_columns`,
//! `stop_with_columns` and `inline_unsafe` (unfold a helper whose rule leaves a column unbound).

use super::program::Program;
use super::patterns::Patterns;
use super::pyre::{captures, esc, findall, g, is_meta, re, sub};
use super::text::{Set, binders, calls, drop_aggregates, mask_ticks, masked, rename_vars, split_alts, split_args, strip_strings, top_binders, upper_vars};
use fancy_regex::Regex;
use indexmap::IndexMap;
use std::collections::{BTreeMap, BTreeSet};

/// What `share_metavariables` found: each helper's columns (in order), and which of them are passed in.
#[derive(Default, Clone)]
pub struct Shared {
    pub order: BTreeMap<String, Vec<String>>,
    pub inputs: BTreeMap<String, Set>,
    pub params: BTreeMap<String, Set>,
}

struct RInfo {
    e: usize,
    owner: String,
    head: Set,
    up: Set,
    binds: Set,
    calls: Vec<String>,
    local: Set,
    tbinds: Set,
    tcalls: Vec<String>,
    top_calls: Vec<String>,
    params: Set,
}


fn head_tail(head: &str) -> &str {
    head.split_once('(').map_or("", |(_, t)| t)
}

fn is_top(l: &str) -> bool {
    !l.starts_with('(') && !l.starts_with('!') && !l.contains(" : {")
}

fn sorted_in(names: Vec<String>, helpers: &Set) -> Vec<String> {
    let s: Set = names.into_iter().filter(|x| helpers.contains(x)).collect();
    s.into_iter().collect()
}

fn union_of<'a>(it: impl Iterator<Item = &'a Set>) -> Set {
    let mut out = Set::new();
    for s in it {
        out.extend(s.iter().cloned());
    }
    out
}

struct Ctx<'a> {
    px: &'a Patterns,
    prog: &'a Program,
    helpers: &'a Set,
    rules: &'a [RInfo],
    by_owner: &'a BTreeMap<String, Vec<usize>>,
    callers: &'a BTreeMap<String, Vec<usize>>,
    forced: &'a BTreeMap<String, Set>,
    always: &'a BTreeMap<String, Set>,
}

impl Ctx<'_> {
    fn before(&self, owner: &str, x: &str, seen: &Set) -> bool {
        if !self.helpers.contains(owner) || seen.contains(owner) || self.callers[owner].is_empty() {
            return false;
        }
        let mut s2 = seen.clone();
        s2.insert(owner.to_owned());
        self.callers[owner].iter().all(|&c| self.bound_at(c, x, owner, &s2))
    }

    fn param_bound(&self, owner: &str, x: &str, seen: &Set) -> bool {
        let px = self.px;
        if seen.contains(owner) || self.callers.get(owner).is_none_or(Vec::is_empty) {
            return false;
        }
        let first = self.by_owner[owner][0];
        let head = &self.prog.arena[self.rules[first].e].head;
        let t = head_tail(head);
        let names = split_args(&t[..t.len().saturating_sub(1)]);
        let Some(j) = names.iter().position(|n| n == x) else { return false };
        let mut s2 = seen.clone();
        s2.insert(owner.to_owned());
        for &c in &self.callers[owner] {
            let text = self.prog.arena[self.rules[c].e].lits.join(", ");
            for (a, b) in calls(px, &text, owner) {
                let args = split_args(&text[a + owner.len() + 1..b]);
                let arg = args.get(j).cloned().unwrap_or_else(|| "_".into());
                if arg == "_" || (is_meta(&arg) && !self.bound_at(c, &arg, owner, &s2)) {
                    return false;
                }
            }
        }
        true
    }

    fn bound_at(&self, c: usize, x: &str, h: &str, seen: &Set) -> bool {
        let r = &self.rules[c];
        let empty = Set::new();
        let forced = self.forced.get(&r.owner).unwrap_or(&empty);
        let in_set = forced.contains(x)
            || (r.tbinds.contains(x) && !r.local.contains(x) && !forced.contains(x))
            || r.tcalls.iter().filter(|g| *g != h).any(|g| self.always[g].contains(x));
        in_set || (r.params.contains(x) && self.param_bound(&r.owner, x, seen)) || self.before(&r.owner, x, seen)
    }
}

/// ast-grep keeps one set of metavariables per match; a helper relation gets a column for each metavariable that
/// crosses its boundary. Each call passes the caller's variable of that name, or `_`.
pub fn share_metavariables(px: &Patterns, prog: &mut Program, entries: &[usize], helpers: &Set) -> Shared {
    let mut rules: Vec<RInfo> = Vec::new();
    for &e in entries {
        let en = &prog.arena[e];
        let text = en.lits.join(", ");
        let calls_ = sorted_in(findall(&px.text_called, &masked(px, &text)), helpers);
        let head_vars = upper_vars(px, head_tail(&en.head));
        let inside = binders(px, &text, true);
        let outside = upper_vars(px, &drop_aggregates(px, &text));
        let local: Set = inside.into_iter().filter(|x| !outside.contains(x) && !head_vars.contains(x)).collect();
        let mut up: Set = upper_vars(px, &text).into_iter().filter(|x| !local.contains(x)).collect();
        up.extend(findall(&px.text_mv_ref, &en.edits.clone().unwrap_or_default().join(" ")));
        let top_text = en.lits.iter().filter(|l| is_top(l)).cloned().collect::<Vec<_>>().join(", ");
        let owner = en.name().to_owned();
        rules.push(RInfo {
            e,
            owner: owner.clone(),
            head: head_vars,
            up,
            binds: binders(px, &text, false),
            calls: calls_,
            local,
            tbinds: top_binders(px, &en.lits),
            tcalls: sorted_in(findall(&px.share_top_call, &masked(px, &top_text)), helpers),
            top_calls: sorted_in(findall(&px.share_top_call, &masked(px, &drop_aggregates(px, &text))), helpers),
            params: if helpers.contains(&owner) { upper_vars(px, head_tail(&en.head)) } else { Set::new() },
        });
    }
    let mut by_owner: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, r) in rules.iter().enumerate() {
        by_owner.entry(r.owner.clone()).or_default().push(i);
    }
    let mut callers: BTreeMap<String, Vec<usize>> = helpers.iter().map(|h| (h.clone(), Vec::new())).collect();
    for (i, r) in rules.iter().enumerate() {
        for h in &r.calls {
            callers.get_mut(h).expect("a helper").push(i);
        }
    }
    let new_map = || -> BTreeMap<String, Set> { helpers.iter().map(|h| (h.clone(), Set::new())).collect() };
    let (mut provides, mut free_in, mut needed, mut iface, mut forced) = (new_map(), new_map(), new_map(), new_map(), new_map());
    let mut always: BTreeMap<String, Set> = new_map();
    for _ in 0..50 {
        let old: Vec<(Set, Set, Set, Set)> = helpers.iter().map(|h| (provides[h].clone(), free_in[h].clone(), needed[h].clone(), forced[h].clone())).collect();
        let mut fresh = new_map();
        for h in helpers {
            for &ri in by_owner.get(h).map(Vec::as_slice).unwrap_or(&[]) {
                let r = &rules[ri];
                let own_binds: Set = r.binds.difference(&forced[h]).cloned().collect();
                let mut bound = own_binds.clone();
                bound.extend(r.params.iter().cloned());
                bound.extend(r.local.iter().cloned());
                for gg in &r.calls {
                    bound.extend(provides[gg].intersection(&iface[gg]).cloned());
                }
                let mut prov = own_binds.clone();
                for gg in &r.calls {
                    prov.extend(provides[gg].iter().cloned());
                }
                let add: Vec<String> = prov.into_iter().filter(|x| !r.params.contains(x)).collect();
                provides.get_mut(h).expect("a helper").extend(add);
                let mut use_ = r.up.clone();
                for gg in &r.calls {
                    use_.extend(free_in[gg].iter().cloned());
                }
                fresh.get_mut(h).expect("a helper").extend(use_.into_iter().filter(|x| !bound.contains(x)));
            }
            let fh = forced[h].clone();
            provides.get_mut(h).expect("a helper").retain(|x| !fh.contains(x));
            fresh.get_mut(h).expect("a helper").extend(fh);
        }
        free_in = fresh;
        let every = union_of(rules.iter().map(|r| &r.binds));
        always = helpers.iter().map(|h| (h.clone(), every.clone())).collect();
        for _ in 0..50 {
            let prev = always.clone();
            for gname in helpers {
                let mut acc: Option<Set> = None;
                for &ri in by_owner.get(gname).map(Vec::as_slice).unwrap_or(&[]) {
                    let r = &rules[ri];
                    let mut b: Set = r.tbinds.difference(&r.local).cloned().collect();
                    for c in &r.tcalls {
                        b.extend(always[c].iter().cloned());
                    }
                    acc = Some(match acc {
                        None => b,
                        Some(a) => a.intersection(&b).cloned().collect(),
                    });
                }
                always.insert(gname.clone(), acc.unwrap_or_default());
            }
            if always == prev {
                break;
            }
        }
        for h in helpers {
            if callers[h].is_empty() {
                continue;
            }
            let mut tprov = Set::new();
            for &ri in by_owner.get(h).map(Vec::as_slice).unwrap_or(&[]) {
                let r = &rules[ri];
                tprov.extend(r.binds.difference(&r.local).cloned());
                for gg in &r.top_calls {
                    tprov.extend(provides[gg].iter().cloned());
                }
            }
            let xs: Vec<String> = provides[h].intersection(&tprov).cloned().collect();
            for x in xs {
                if x.starts_with('_') {
                    continue;
                }
                let ok = {
                    let ctx = Ctx { px, prog, helpers, rules: &rules, by_owner: &by_owner, callers: &callers, forced: &forced, always: &always };
                    let seen: Set = [h.clone()].into_iter().collect();
                    callers[h].iter().all(|&c| ctx.bound_at(c, &x, h, &seen))
                };
                if ok {
                    forced.get_mut(h).expect("a helper").insert(x);
                }
            }
            let fh = forced[h].clone();
            provides.get_mut(h).expect("a helper").retain(|x| !fh.contains(x));
        }
        for h in helpers {
            for &ri in &callers[h] {
                let r = &rules[ri];
                let mut add = Set::new();
                if let Some(s) = iface.get(&r.owner) {
                    add.extend(s.iter().cloned());
                }
                if let Some(s) = needed.get(&r.owner) {
                    add.extend(s.iter().cloned());
                }
                for gg in r.calls.iter().filter(|gg| *gg != h) {
                    add.extend(iface[gg].iter().cloned());
                }
                add.extend(r.head.iter().cloned());
                add.extend(r.up.iter().cloned());
                needed.get_mut(h).expect("a helper").extend(add);
            }
        }
        for h in helpers {
            let mut i: Set = free_in[h].clone();
            i.extend(needed[h].intersection(&provides[h]).cloned());
            iface.insert(h.clone(), i);
        }
        let same = helpers.iter().zip(&old).all(|(h, o)| o.0 == provides[h] && o.1 == free_in[h] && o.2 == needed[h] && o.3 == forced[h]);
        if same {
            break;
        }
    }
    let _ = always;
    let order: BTreeMap<String, Vec<String>> = helpers.iter().map(|h| (h.clone(), iface[h].iter().cloned().collect())).collect();
    let shared = Shared {
        order: order.clone(),
        inputs: helpers.iter().map(|h| (h.clone(), free_in[h].clone())).collect(),
        params: helpers.iter().map(|h| (h.clone(), union_of(by_owner.get(h).map(Vec::as_slice).unwrap_or(&[]).iter().map(|&ri| &rules[ri].params)))).collect(),
    };
    for r in &rules {
        let e = r.e;
        if let Some(cols) = order.get(&r.owner).filter(|c| !c.is_empty()) {
            let h = &prog.arena[e].head;
            prog.arena[e].head = format!("{}, {})", &h[..h.len() - 1], cols.join(", "));
        }
        let hit: Vec<&String> = r.calls.iter().filter(|h| !order[*h].is_empty()).collect();
        if hit.is_empty() {
            continue;
        }
        let mut relevant: Set = r.head.clone();
        relevant.extend(r.up.iter().cloned());
        relevant.extend(r.binds.iter().cloned());
        relevant.extend(r.local.iter().cloned());
        if let Some(s) = iface.get(&r.owner) {
            relevant.extend(s.iter().cloned());
        }
        let mut new = Vec::new();
        for l in prog.arena[e].lits.clone() {
            let mut l = l;
            for h in &hit {
                let mut rel = relevant.clone();
                for gg in r.calls.iter().filter(|gg| gg != h) {
                    rel.extend(iface[gg].iter().cloned());
                }
                let args = order[*h].iter().map(|v| if rel.contains(v) { v.clone() } else { "_".into() }).collect::<Vec<_>>().join(", ");
                for (_, b) in calls(px, &l, h).into_iter().rev() {
                    l = format!("{}, {args}{}", &l[..b], &l[b..]);
                }
            }
            new.push(l);
        }
        prog.arena[e].lits = new;
    }
    shared
}

/// `pattern: "$X"` where the caller passes X in matches a node with X's text, not X itself.
pub fn passed_in_patterns(prog: &mut Program, entries: &[usize], inputs: &BTreeMap<String, Set>) {
    let mut k = 0usize;
    for &e in entries {
        let h = prog.arena[e].name().to_owned();
        if prog.arena[e].drop {
            continue;
        }
        let Some(ins) = inputs.get(&h).filter(|s| !s.is_empty()) else { continue };
        for x in ins {
            let xl = x.to_lowercase();
            let p1 = re(&format!(r"(?<![\w$])([a-z][a-z0-9_]*) ~ `\${}`", esc(x)));
            let lits: Vec<String> = prog.arena[e]
                .lits
                .iter()
                .map(|l| {
                    sub(&p1, l, |c| {
                        k += 1;
                        format!("text({}, t_{xl}{k}), text({x}, t_{xl}{k})", g(c, 1))
                    })
                })
                .collect();
            let p2 = re(&format!(r"(?<![\w$@]){} = ([a-z][a-z0-9_]*)\b(?!\s*\()", esc(x)));
            let lits: Vec<String> = lits
                .iter()
                .map(|l| {
                    sub(&p2, l, |c| {
                        k += 1;
                        format!("text({}, t_{xl}{k}), text({x}, t_{xl}{k})", g(c, 1))
                    })
                })
                .collect();
            prog.arena[e].lits = lits;
        }
    }
}

type Call = (usize, usize, String, Vec<String>, bool);

struct DInfo {
    e: usize,
    owner: String,
    hargs: Vec<String>,
    direct: Set,
    passed: BTreeMap<String, Vec<(String, usize, bool)>>,
}

/// A column passed in that no call feeds is removed from the declaration, the heads and every call.
pub fn drop_dead_columns(px: &Patterns, prog: &mut Program, entries: &[usize], helpers: &Set, shared: &Shared) {
    let live: Vec<usize> = entries.iter().copied().filter(|&e| !prog.arena[e].drop).collect();
    let mut heads: IndexMap<String, Vec<usize>> = IndexMap::new();
    for &e in &live {
        let h = prog.arena[e].name().to_owned();
        if helpers.contains(&h) {
            heads.entry(h).or_default().push(e);
        }
    }
    let head_args = |prog: &Program, e: usize| -> Vec<String> {
        let t = head_tail(&prog.arena[e].head);
        split_args(&t[..t.len().saturating_sub(1)])
    };
    // a param the util's body captures (a pattern's `$X`, `X = …`, a regex group), or passes to a column of a rule
    // it calls that captures it: an output. One it only passes on to a column that reads it (a helper's input) is
    // read, not captured: the caller's value goes through.
    fn binds_in(px: &Patterns, prog: &Program, heads: &IndexMap<String, Vec<usize>>, g: &str, x: &str, seen: &mut BTreeSet<(String, String)>) -> bool {
        if !seen.insert((g.to_owned(), x.to_owned())) {
            return false;
        }
        let r = re(&format!(r"\$\$?\$?{}\b", esc(x)));
        for &e in &heads[g] {
            let t = prog.arena[e].lits.join(", ");
            if binders(px, &t, false).contains(x) || binders(px, &t, true).contains(x) || super::pyre::search(&r, &t) {
                return true;
            }
            for (g2, es2) in heads {
                let names = {
                    let h = head_tail(&prog.arena[es2[0]].head);
                    split_args(&h[..h.len().saturating_sub(1)])
                };
                for (a, b) in calls(px, &t, g2) {
                    let args = split_args(&t[a + g2.len() + 1..b]);
                    for (j, arg) in args.iter().enumerate() {
                        if arg == x && j < names.len() && binds_in(px, prog, heads, g2, &names[j], &mut seen.clone()) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
    let binds_in = |prog: &Program, gname: &str, x: &str| binds_in(px, prog, &heads, gname, x, &mut BTreeSet::new());
    let empty = Set::new();
    let input_col = |prog: &Program, gname: &str, j: usize| -> bool {
        let names = head_args(prog, heads[gname][0]);
        0 < j && j < names.len() && (shared.inputs.get(gname).unwrap_or(&empty).contains(&names[j]) || (shared.params.get(gname).unwrap_or(&empty).contains(&names[j]) && !binds_in(prog, gname, &names[j])))
    };
    let calls_in = |prog: &Program, e: usize| -> (String, Vec<Call>) {
        let text = prog.arena[e].lits.join(", ");
        let mut out = Vec::new();
        for gname in heads.keys() {
            for (a, b) in calls(px, &text, gname) {
                if !text[..a].ends_with('@') {
                    out.push((a, b, gname.clone(), split_args(&text[a + gname.len() + 1..b]), text[..a].trim_end().ends_with('!')));
                }
            }
        }
        (text, out)
    };
    let mut info: Vec<DInfo> = Vec::new();
    for &e in &live {
        let (text, cs) = calls_in(prog, e);
        let mut rest = masked(px, &text).into_bytes();
        for (a, b, ..) in &cs {
            for k in *a..=*b {
                rest[k] = b' ';
            }
        }
        let rest = String::from_utf8_lossy(&rest).into_owned();
        let mut direct = upper_vars(px, &rest);
        direct.extend(findall(&px.text_mv_ref, &text));
        direct.extend(findall(&px.text_mv_ref, &prog.arena[e].edits.clone().unwrap_or_default().join(" ")));
        let mut passed: BTreeMap<String, Vec<(String, usize, bool)>> = BTreeMap::new();
        for (_, _, gname, args, neg) in &cs {
            for (j, x) in args.iter().enumerate() {
                if is_meta(x) {
                    passed.entry(x.clone()).or_default().push((gname.clone(), j, *neg));
                } else if x != "_" {
                    direct.extend(upper_vars(px, x));
                }
            }
        }
        info.push(DInfo { e, owner: prog.arena[e].name().to_owned(), hargs: head_args(prog, e), direct, passed });
    }
    let mut cols: std::collections::BTreeSet<(String, usize)> = std::collections::BTreeSet::new();
    for gname in heads.keys() {
        for j in 0..head_args(prog, heads[gname][0]).len() {
            if input_col(prog, gname, j) {
                cols.insert((gname.clone(), j));
            }
        }
    }
    let mut named = Set::new();
    let bare: Vec<(String, Regex)> = heads.keys().map(|gname| (gname.clone(), re(&format!(r"(?<![\w.]){}(?![\w(])", esc(gname))))).collect();
    for &e in &live {
        let t = mask_ticks(px, &strip_strings(px, &prog.arena[e].lits.join(", ")));
        for (gname, r) in &bare {
            if super::pyre::search(r, &t) {
                named.insert(gname.clone());
            }
        }
    }
    let mut fed: std::collections::BTreeSet<(String, usize)> = cols.iter().filter(|(gname, _)| named.contains(gname)).cloned().collect();
    let bound = |fed: &std::collections::BTreeSet<(String, usize)>, d: &DInfo, var: &str| -> bool {
        if d.direct.contains(var) {
            return true;
        }
        if d.passed.get(var).is_some_and(|ps| ps.iter().any(|(gname, j, neg)| !cols.contains(&(gname.clone(), *j)) && !neg)) {
            return true;
        }
        d.hargs.iter().enumerate().any(|(i, x)| x == var && fed.contains(&(d.owner.clone(), i))) || (!heads.contains_key(&d.owner) && d.hargs.iter().any(|x| x == var))
    };
    let mut changed = true;
    while changed {
        changed = false;
        for d in &info {
            let (_, cs) = calls_in(prog, d.e);
            for (_, _, gname, args, _) in cs {
                for (j, x) in args.iter().enumerate() {
                    let key = (gname.clone(), j);
                    if cols.contains(&key) && !fed.contains(&key) && x != "_" && (!is_meta(x) || bound(&fed, d, x)) {
                        fed.insert(key);
                        changed = true;
                    }
                }
            }
        }
    }
    let dead: Vec<(String, usize)> = cols.iter().filter(|c| !fed.contains(*c)).cloned().collect();
    let mut by: IndexMap<String, std::collections::BTreeSet<usize>> = IndexMap::new();
    for (gname, j) in &dead {
        by.entry(gname.clone()).or_default().insert(*j);
    }
    let gone: BTreeMap<String, Set> = by.iter().map(|(gname, js)| (gname.clone(), js.iter().map(|&j| head_args(prog, heads[gname][0])[j].clone()).collect())).collect();
    let rewrite = |prog: &Program, text: &str, d: &DInfo| -> String {
        let mut text = text.to_owned();
        for gname in heads.keys() {
            for (a, b) in calls(px, &text, gname).into_iter().rev() {
                if text[..a].ends_with('@') {
                    continue;
                }
                let args = split_args(&text[a + gname.len() + 1..b]);
                let neg = text[..a].trim_end().ends_with('!');
                let drop = by.get(gname);
                let new: Vec<String> = args
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| !drop.is_some_and(|s| s.contains(j)))
                    .map(|(j, x)| if (cols.contains(&(gname.clone(), j)) || neg) && is_meta(x) && !bound(&fed, d, x) { "_".to_owned() } else { x.clone() })
                    .collect();
                text = format!("{}{gname}({}){}", &text[..a], new.join(", "), &text[b + 1..]);
            }
        }
        let _ = prog;
        text
    };
    for d in &info {
        let lits: Vec<String> = prog.arena[d.e].lits.iter().map(|l| rewrite(prog, l, d)).collect();
        prog.arena[d.e].lits = lits;
        if let Some(js) = by.get(&d.owner) {
            let args: Vec<String> = d.hargs.iter().enumerate().filter(|(j, _)| !js.contains(j)).map(|(_, x)| x.clone()).collect();
            prog.arena[d.e].head = format!("{}({})", d.owner, args.join(", "));
        }
    }
    let decl_re = re(r"^(\.decl \w+)\((.*)\)(.*)");
    let note_re = re(r"^(//\s+)([A-Z0-9_, ]+): (metavariables? of the calling rule, passed in.*)");
    for (gname, js) in &by {
        let decl = prog.decls[gname].clone();
        let c = super::pyre::find(&decl_re, &decl).expect("a declaration");
        let parts: Vec<String> = split_args(g(&c, 2)).into_iter().enumerate().filter(|(j, _)| !js.contains(j)).map(|(_, x)| x).collect();
        let nd = format!("{}({}){}", g(&c, 1), parts.join(", "), g(&c, 3));
        prog.decls.insert(gname.clone(), nd);
        let mut kept = Vec::new();
        for l in prog.notes.get(gname).cloned().unwrap_or_default() {
            let mut l = l;
            if let Some(c) = super::pyre::find(&note_re, &l) {
                let names: Vec<String> = g(&c, 2).split(',').map(str::trim).filter(|x| !gone[gname].contains(*x)).map(str::to_owned).collect();
                if names.is_empty() {
                    continue;
                }
                l = format!("{}{}: {}", g(&c, 1), names.join(", "), g(&c, 3));
            }
            kept.push(l);
        }
        prog.notes.insert(gname.clone(), kept);
    }
}

/// A stopBy rule that reads a metavariable of the caller is no set of nodes: the axis with no node of it between.
pub fn stop_with_columns(prog: &mut Program, entries: &[usize]) {
    let decl_re = re(r"^\.decl (\w+)\((.*)\)");
    let mut wide: IndexMap<String, ()> = IndexMap::new();
    for (name, d) in &prog.decls {
        if let Some(c) = super::pyre::find(&decl_re, d)
            && g(&c, 1) == name
            && g(&c, 2).matches(':').count() > 1
        {
            wide.insert(name.clone(), ());
        }
    }
    let mut heads: IndexMap<String, String> = IndexMap::new();
    for &e in entries {
        if !prog.arena[e].drop {
            heads.insert(prog.arena[e].name().to_owned(), prog.arena[e].head.clone());
        }
    }
    let wide: BTreeMap<String, Vec<String>> = wide
        .keys()
        .filter(|r| heads.contains_key(*r))
        .map(|r| (r.clone(), head_tail(&heads[r]).trim_end_matches(')').split(',').map(|a| a.trim().to_owned()).collect()))
        .collect();
    if wide.is_empty() {
        return;
    }
    let mut k = 0usize;
    let pat = re(r"(within|precedes)_stop\((\w+), (\w+), (\w+)\)");
    for &e in entries {
        let lits: Vec<String> = prog.arena[e]
            .lits
            .iter()
            .map(|l| {
                sub(&pat, l, |c| {
                    let (axis, a, b, rel) = (g(c, 1), g(c, 2), g(c, 3), g(c, 4));
                    let Some(cols) = wide.get(rel) else { return g(c, 0).to_owned() };
                    k += 1;
                    let x = format!("m_s{k}");
                    let mut args = vec![x.clone()];
                    args.extend(cols[1..].iter().cloned());
                    format!("{axis}({a}, {b}), count : {{ {axis}({a}, {x}), {axis}({x}, {b}), {rel}({}) }} = 0", args.join(", "))
                })
            })
            .collect();
        prog.arena[e].lits = lits;
    }
}

const NOT_VARS: [&str; 11] = ["count", "sum", "min", "max", "mean", "join", "by", "sep", "true", "false", "nil"];

fn bound_on_every_path(px: &Patterns, lits: &[String]) -> Set {
    let mut out = Set::new();
    for l in lits {
        let l = l.trim();
        let alts = if l.starts_with('(') && l.ends_with(')') && super::text::balanced(l) { split_alts(&super::text::chars_slice(l, 1, 1)) } else { vec![l.to_owned()] };
        if alts.len() > 1 {
            let mut sets = alts.iter().map(|a| bound_on_every_path(px, &split_args(a)));
            let first = sets.next().unwrap_or_default();
            out.extend(sets.fold(first, |acc, s| acc.intersection(&s).cloned().collect::<Set>()));
            continue;
        }
        let mut flat = l.to_owned();
        loop {
            let next = sub(&px.share_agg_body, &flat, |_| String::new());
            if next == flat {
                break;
            }
            flat = next;
        }
        out.extend(upper_vars(px, &flat));
        out.extend(binders(px, &flat, false));
    }
    out
}

fn head_vars(head: &str) -> Vec<String> {
    let t = head_tail(head);
    t[..t.len().saturating_sub(1)].split(',').map(|a| a.trim().to_owned()).collect()
}

/// A non-recursive helper whose rule leaves one of its metavariable columns unbound is unfolded at each call.
pub fn inline_unsafe(px: &Patterns, prog: &mut Program, entries: &[usize], helpers: &Set) {
    let mut by: IndexMap<String, Vec<usize>> = IndexMap::new();
    for &e in entries {
        by.entry(prog.arena[e].name().to_owned()).or_default().push(e);
    }
    let unsafe_ = |prog: &Program, e: usize| -> bool {
        let present = bound_on_every_path(px, &prog.arena[e].lits);
        head_vars(&prog.arena[e].head).iter().any(|v| is_meta(v) && !present.contains(v))
    };
    let calls_of = |prog: &Program, h: &str| -> Set {
        let mut out = Set::new();
        for &e in by.get(h).map(Vec::as_slice).unwrap_or(&[]) {
            if prog.arena[e].drop {
                continue;
            }
            for gname in findall(&px.text_called, &masked(px, &prog.arena[e].lits.join(", "))) {
                if by.contains_key(&gname) {
                    out.insert(gname);
                }
            }
        }
        out
    };
    let recursive = |prog: &Program, h: &str| -> bool {
        let mut seen = Set::new();
        let mut todo = vec![h.to_owned()];
        while let Some(t) = todo.pop() {
            for gname in calls_of(prog, &t) {
                if gname == h {
                    return true;
                }
                if seen.insert(gname.clone()) {
                    todo.push(gname);
                }
            }
        }
        false
    };
    let mut counter = 0usize;
    for _ in 0..10 {
        let targets: Vec<String> = helpers
            .iter()
            .filter(|h| by.contains_key(*h) && by[*h].iter().any(|&e| !prog.arena[e].drop && unsafe_(prog, e)) && !recursive(prog, h))
            .cloned()
            .collect();
        let Some(h) = targets.first().cloned() else { break };
        for &e in entries {
            if prog.arena[e].drop || prog.arena[e].name() == h {
                continue;
            }
            let orig = prog.arena[e].lits.clone();
            let mut new = Vec::new();
            for (i, l0) in orig.iter().enumerate() {
                let mut l = l0.clone();
                for (s, end) in calls(px, &l, &h).into_iter().rev() {
                    let args = split_args(&l[s + h.len() + 1..end]);
                    let mut others_v: Vec<String> = orig[..i].to_vec();
                    others_v.push(format!("{}{}", &l[..s], &l[end + 1..]));
                    others_v.extend(orig[i + 1..].iter().cloned());
                    let known = binders(px, &others_v.join(", "), false);
                    let alts = expansion(px, prog, &by, helpers, &h, &args, &known, &mut counter);
                    if s > 0 && l.as_bytes()[s - 1] == b'!' {
                        let rep = alts.iter().map(|a| format!("count : {{ {a} }} = 0")).collect::<Vec<_>>().join(", ");
                        l = format!("{}{rep}{}", &l[..s - 1], &l[end + 1..]);
                    } else {
                        let rep = if alts.len() == 1 { alts[0].clone() } else { format!("({})", alts.join(" ; ")) };
                        l = format!("{}{rep}{}", &l[..s], &l[end + 1..]);
                    }
                }
                new.push(l);
            }
            prog.arena[e].lits = new;
        }
        for &e in &by[&h] {
            prog.arena[e].drop = true;
        }
    }
}

/// The alternatives of h's rules at a call with args.
fn expansion(px: &Patterns, prog: &Program, by: &IndexMap<String, Vec<usize>>, helpers: &Set, h: &str, args: &[String], known: &Set, counter: &mut usize) -> Vec<String> {
    let mut alts = Vec::new();
    let mut outputs = Set::new();
    for &e in &by[h] {
        if !prog.arena[e].drop {
            outputs.extend(binders(px, &prog.arena[e].lits.join(", "), false));
        }
    }
    for &e in &by[h] {
        if prog.arena[e].drop {
            continue;
        }
        *counter += 1;
        let k = *counter;
        let params = head_vars(&prog.arena[e].head);
        let mut mapping: Vec<(String, String)> = Vec::new();
        let set = |mapping: &mut Vec<(String, String)>, key: &str, val: String| {
            if let Some(slot) = mapping.iter_mut().find(|(k, _)| k == key) {
                slot.1 = val;
            } else {
                mapping.push((key.to_owned(), val));
            }
        };
        for (p, a) in params.iter().zip(args) {
            let val = if a != "_" { a.clone() } else { format!("w{k}_{}", p.to_lowercase()) };
            set(&mut mapping, p, val);
        }
        let body = prog.arena[e].lits.join(", ");
        let vars: Set = findall(&px.share_lower_var, &mask_ticks(px, &strip_strings(px, &body))).into_iter().collect();
        for v in vars {
            if !mapping.iter().any(|(k, _)| *k == v) && !NOT_VARS.contains(&v.as_str()) && !helpers.contains(&v) {
                let val = format!("{v}_{k}");
                set(&mut mapping, &v, val);
            }
        }
        let every = bound_on_every_path(px, &prog.arena[e].lits);
        let missing: Vec<&String> = params
            .iter()
            .filter(|p| is_meta(p) && !every.contains(*p) && outputs.contains(*p) && !mapping.iter().find(|(k, _)| k == *p).is_some_and(|(_, v)| known.contains(v)))
            .collect();
        let text = body + &missing.iter().map(|p| format!(", unset({p})")).collect::<String>();
        alts.push(rename_vars(px, &text, &mapping));
    }
    alts
}

