//! `translate.py`'s demand passes (magic sets by hand) and the passes after them: `demand` (a helper that reads
//! metavariables its callers pass in), `node_demand` (a recursion that looks at every descendant),
//! `hoist_ancestor_steps` and `unroll_negative` (a recursion through `!` as UNROLL levels).

use super::lower::R;
use super::program::{Entry, LibEntry, Program, Rule};
use super::patterns::Patterns;
use super::pyre::{captures, find, findall, g, has, is_full, re, search, split1, sub};
use super::souffle::check_rule;
use super::text::{Set, alternation, balanced, calls, mask_ticks, split_alts, split_args, split_top_text, strip_strings, strip_strings_keep, wrap};
use fancy_regex::Regex;
use indexmap::IndexMap;
use std::collections::{BTreeMap, BTreeSet};


/// Levels of nesting a recursion through negation is exact for (tuples of tuples, nested parentheses).
pub const UNROLL: usize = 8;

const UPPER: &str = r"[A-Z][A-Z0-9_]*";

/// The demand rules made for a caller's call: (demand relation, owner, index of the rule), fixed up after
/// `node_demand` (`demand.sites`).
pub type Sites = Vec<(String, String, usize)>;

fn head_params(head: &str) -> Vec<String> {
    let t = head.split_once('(').map_or("", |(_, t)| t);
    t[..t.len().saturating_sub(1)].split(',').map(|a| a.trim().to_owned()).collect()
}

fn ungrounded(px: &Patterns, stmt: &str) -> R<bool> {
    check_rule(px, stmt, &BTreeSet::new(), &[]).map(|b| !b.is_empty()).map_err(|e| e.0)
}

fn matches(r: &Regex, s: &str) -> bool {
    search(r, s)
}

fn by_head(prog: &Program, entries: &[usize]) -> IndexMap<String, Vec<usize>> {
    let mut by: IndexMap<String, Vec<usize>> = IndexMap::new();
    for &e in entries {
        if !prog.arena[e].drop {
            by.entry(prog.arena[e].name().to_owned()).or_default().push(e);
        }
    }
    by
}

/// Relations a text calls (strings kept as `""`, patterns blanked).
fn names_keep(px: &Patterns, t: &str) -> Set {
    findall(&px.text_called, &mask_ticks(px, &strip_strings_keep(px, t))).into_iter().collect()
}

type Site = Option<(Vec<String>, String)>;

/// `_call_sites`: (context positives, call text) for each call of h in a body; None where a call sits where this
/// cannot say.
fn call_sites(px: &Patterns, text: &str, h: &str, ctx: &[String]) -> Vec<Site> {
    let parts = split_top_text(text, ',');
    let plain = |p: &str| -> bool { (matches(&px.demand_positive, p) && balanced(&p[p.find('(').expect("a call")..]) && calls(px, p, h).is_empty()) || matches(&px.demand_eq, p) };
    let plain_alts = |p: &str| -> bool {
        if !(p.starts_with('(') && p.ends_with(')') && balanced(p)) || !calls(px, p, h).is_empty() {
            return false;
        }
        let bs = split_alts(&p[1..p.len() - 1]);
        bs.len() > 1 && bs.iter().all(|b| split_top_text(b, ',').iter().all(|q| plain(q)))
    };
    let here: Vec<String> = parts.iter().filter(|p| plain(p) || plain_alts(p)).cloned().collect();
    let mut scope: Vec<String> = ctx.to_vec();
    scope.extend(here);
    let mut out = Vec::new();
    for p in &parts {
        if calls(px, p, h).is_empty() {
            continue;
        }
        let m = find(&px.demand_agg, p);
        let bare = p.strip_prefix('!').unwrap_or(p);
        let bc = calls(px, bare, h);
        if matches(&px.demand_positive, bare) && bc.len() == 1 && bc[0] == (0, bare.len() - 1) {
            out.push(Some((scope.clone(), bare.to_owned())));
        } else if let Some(m) = m.filter(|m| calls(px, g(m, 2), h).is_empty()) {
            out.extend(call_sites(px, g(&m, 1), h, &scope));
        } else if p.starts_with('(') && p.ends_with(')') && split_alts(&p[1..p.len() - 1]).len() > 1 {
            for b in split_alts(&p[1..p.len() - 1]) {
                out.extend(call_sites(px, &b, h, &scope));
            }
        } else {
            out.push(None);
        }
    }
    out
}

/// Each helper's callees, transitively (the helpers in `by` its rules name).
fn reach_of(px: &Patterns, prog: &Program, by: &IndexMap<String, Vec<usize>>, keep: bool) -> BTreeMap<String, Set> {
    let mut reach = BTreeMap::new();
    for h in by.keys() {
        let mut r = Set::new();
        let mut todo = vec![h.clone()];
        while let Some(t) = todo.pop() {
            for &e in by.get(&t).map(Vec::as_slice).unwrap_or(&[]) {
                let text = prog.arena[e].lits.join(", ");
                let names = if keep { findall(&px.text_called, &mask_ticks(px, &strip_strings_keep(px, &text))) } else { findall(&px.text_called, &mask_ticks(px, &strip_strings(px, &text))) };
                for gname in names {
                    if by.contains_key(&gname) && r.insert(gname.clone()) {
                        todo.push(gname);
                    }
                }
            }
        }
        reach.insert(h.clone(), r);
    }
    reach
}

struct Demand<'a> {
    entries: &'a [usize],
    inputs: &'a BTreeMap<String, Set>,
    by: IndexMap<String, Vec<usize>>,
    reach: BTreeMap<String, Set>,
    sites: Sites,
}

impl Demand<'_> {
    fn callees(&self, h: &str) -> &Set {
        &self.reach[h]
    }

    fn has_inputs(&self, x: &str) -> bool {
        self.inputs.get(x).is_some_and(|s| !s.is_empty())
    }

    fn try_demand(&mut self, px: &Patterns, prog: &mut Program, group: &[String]) -> R<bool> {
        let gset: Set = group.iter().cloned().collect();
        let mut pos: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for h in &gset {
            let empty = Set::new();
            let ins = self.inputs.get(h).unwrap_or(&empty);
            if ins.is_empty() || !self.by.contains_key(h) {
                return Ok(false);
            }
            let params = head_params(&prog.arena[self.by[h][0]].head);
            if params[1..].iter().any(|p| !ins.contains(p) && !is_full(UPPER, p)) {
                return Ok(false);
            }
            let mut ps = vec![0];
            ps.extend(params.iter().enumerate().filter(|(_, p)| ins.contains(*p)).map(|(i, _)| i));
            pos.insert(h.clone(), ps);
        }
        let reaches = |l: &str| -> bool {
            findall(&px.text_called, &mask_ticks(px, &strip_strings(px, l))).iter().any(|gname| {
                self.by.contains_key(gname)
                    && (gset.contains(gname)
                        || !gset.is_disjoint(self.callees(gname))
                        || self.callees(gname).iter().chain(std::iter::once(gname)).any(|x| self.has_inputs(x)))
            })
        };
        let mut made: Vec<(String, Vec<Rule>, Vec<String>)> = Vec::new();
        for h in &gset {
            let dname = format!("demand_{h}");
            let (mut rules, mut owners): (Vec<Rule>, Vec<String>) = (Vec::new(), Vec::new());
            for &e in self.entries {
                if prog.arena[e].drop {
                    continue;
                }
                let owner = prog.arena[e].name().to_owned();
                for site in call_sites(px, &prog.arena[e].lits.join(", "), h, &[]) {
                    let Some((scope, call)) = site else { return Ok(false) };
                    let args = split_args(&call[h.len() + 1..call.len() - 1]);
                    let mut want = Vec::new();
                    for &i in &pos[h] {
                        want.push(args.get(i).cloned().ok_or("IndexError: list index out of range")?);
                    }
                    if want.iter().any(|a| a == "_") {
                        return Ok(false);
                    }
                    let mut lits: Vec<String> = scope.into_iter().filter(|l| !reaches(l)).collect();
                    if gset.contains(&owner) {
                        let hp = head_params(&prog.arena[e].head);
                        let d = pos[&owner].iter().map(|&i| hp[i].clone()).collect::<Vec<_>>().join(", ");
                        lits.insert(0, format!("demand_{owner}({d})"));
                    } else if gset.iter().any(|gname| self.callees(gname).contains(&owner)) {
                        return Ok(false);
                    }
                    let head = format!("{dname}({})", want.join(", "));
                    if ungrounded(px, &wrap(&head, &lits, None))? || lits.is_empty() {
                        return Ok(false);
                    }
                    rules.push((head, lits));
                    owners.push(owner.clone());
                }
            }
            if rules.is_empty() {
                return Ok(false);
            }
            made.push((h.clone(), rules, owners));
        }
        for (h, rules, owners) in made {
            let dname = format!("demand_{h}");
            let decl = &prog.decls[&h];
            let cols: Vec<String> = decl[decl.find('(').expect("a declaration") + 1..decl.len() - 1].split(',').map(|c| c.trim().to_owned()).collect();
            let keep: Vec<usize> = (0..rules.len()).filter(|&i| !rules[..i].contains(&rules[i])).collect();
            let entry: LibEntry = (
                format!(".decl {dname}({})", pos[&h].iter().map(|&i| cols[i].clone()).collect::<Vec<_>>().join(", ")),
                vec![format!("// {dname}: the rows of {h} its callers ask for (the metavariable columns they pass in)")],
                keep.iter().map(|&i| rules[i].clone()).collect(),
            );
            prog.lib.insert(dname.clone(), entry);
            for (k, &i) in keep.iter().enumerate() {
                if !gset.contains(&owners[i]) {
                    self.sites.push((dname.clone(), owners[i].clone(), k));
                }
            }
            for &e in &self.by[&h] {
                let hp = head_params(&prog.arena[e].head);
                let bound: Set = pos[&h][1..].iter().map(|&i| hp[i].clone()).filter(|x| is_full(UPPER, x)).collect();
                let d = format!("{dname}({})", pos[&h].iter().map(|&i| hp[i].clone()).collect::<Vec<_>>().join(", "));
                let mut lits = vec![d];
                lits.extend(prog.arena[e].lits.iter().filter(|l| !bound.iter().any(|x| **l == format!("node(_, {x}, _, _, _)"))).cloned());
                prog.arena[e].lits = lits;
            }
        }
        Ok(true)
    }
}

/// Magic sets, by hand: a helper that reads a metavariable its caller binds starts each rule with
/// `demand_h(n, N)`, from the positive literals in scope at each call. Returns `demand.sites`.
pub fn demand(px: &Patterns, prog: &mut Program, entries: &[usize], helpers: &Set, inputs: &BTreeMap<String, Set>) -> R<Sites> {
    let by = by_head(prog, entries);
    let reach = reach_of(px, prog, &by, true);
    let mut d = Demand { entries, inputs, by, reach, sites: Vec::new() };
    let mut unit: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for h in helpers {
        if d.by.contains_key(h) && !unit.contains_key(h) {
            let mut scc: Set = d.callees(h).iter().filter(|gname| helpers.contains(*gname) && d.callees(gname).contains(h)).cloned().collect();
            scc.insert(h.clone());
            let t: Vec<String> = scc.iter().cloned().collect();
            for gname in &scc {
                unit.insert(gname.clone(), t.clone());
            }
        }
    }
    let units: Vec<Vec<String>> = unit.values().cloned().collect::<BTreeSet<_>>().into_iter().collect();
    let mut calls_u: BTreeMap<Vec<String>, Set> = units.iter().map(|u| (u.clone(), Set::new())).collect();
    for &e in entries {
        if prog.arena[e].drop {
            continue;
        }
        let owner = prog.arena[e].name().to_owned();
        for gname in names_keep(px, &prog.arena[e].lits.join(", ")) {
            if let Some(u) = unit.get(&gname)
                && !u.contains(&owner)
                && helpers.contains(&owner)
                && d.has_inputs(&owner)
            {
                calls_u.get_mut(u).expect("a unit").insert(owner.clone());
            }
        }
    }
    let mut done: Vec<String> = Vec::new();
    let mut tried: BTreeSet<Vec<String>> = BTreeSet::new();
    loop {
        let tried_names: Set = tried.iter().flatten().cloned().collect();
        let mut ready: Vec<Vec<String>> = units.iter().filter(|u| !tried.contains(*u) && calls_u[*u].is_subset(&tried_names)).cloned().collect();
        if ready.is_empty() {
            ready = units.iter().filter(|u| !tried.contains(*u)).take(1).cloned().collect();
            if ready.is_empty() {
                break;
            }
        }
        for u in ready {
            tried.insert(u.clone());
            if d.try_demand(px, prog, &u)? {
                done.extend(u);
            }
        }
    }
    loop {
        let done_set: Set = done.iter().cloned().collect();
        let mut new = Vec::new();
        for u in &units {
            if u.iter().all(|x| !done_set.contains(x)) && d.try_demand(px, prog, u)? {
                new.push(u.clone());
            }
        }
        if new.is_empty() {
            break;
        }
        done.extend(new.into_iter().flatten());
    }
    Ok(d.sites)
}

/// Magic sets on the node column, for a recursive helper that looks at every descendant or later sibling.
pub fn node_demand(px: &Patterns, prog: &mut Program, entries: &[usize], helpers: &Set) -> R<()> {
    let by = by_head(prog, entries);
    let body_of: BTreeMap<String, String> = by.iter().map(|(h, es)| (h.clone(), es.iter().flat_map(|&e| prog.arena[e].lits.iter().cloned()).collect::<Vec<_>>().join(", "))).collect();
    let edges: BTreeMap<String, Set> = by.keys().map(|h| (h.clone(), names_keep(px, &body_of[h]).into_iter().filter(|x| by.contains_key(x)).collect())).collect();
    let mut reach: BTreeMap<String, Set> = BTreeMap::new();
    for h in by.keys() {
        let mut r = Set::new();
        let mut todo = vec![h.clone()];
        while let Some(t) = todo.pop() {
            for gname in edges.get(&t).into_iter().flatten() {
                if r.insert(gname.clone()) {
                    todo.push(gname.clone());
                }
            }
        }
        reach.insert(h.clone(), r);
    }
    let callees = |h: &str| -> &Set { &reach[h] };
    let mut done = Set::new();
    let mut sccs: Vec<Set> = Vec::new();
    let sorted_by: Set = by.keys().cloned().collect();
    for h in &sorted_by {
        if done.contains(h) || !helpers.contains(h) || !callees(h).contains(h) {
            continue;
        }
        let scc: Set = callees(h).iter().filter(|gname| callees(gname).contains(h)).cloned().collect();
        done.extend(scc.iter().cloned());
        sccs.push(scc);
    }
    let mut lone = Set::new();
    for h in &sorted_by {
        if !done.contains(h) && helpers.contains(h) && !by[h].is_empty() && by[h].iter().all(|&e| prog.arena[e].lits.first().is_some_and(|l| search(&px.demand_up, l))) {
            sccs.push([h.clone()].into_iter().collect());
            lone.insert(h.clone());
        }
    }
    sccs.sort_by_key(|c| {
        let m = c.iter().next().expect("a member").clone();
        (std::cmp::Reverse(callees(&m).len()), m)
    });
    let starts_demand = |prog: &Program, e: usize| prog.arena[e].lits.first().is_some_and(|l| l.starts_with("demand_"));
    for scc in sccs {
        if scc.iter().any(|gname| !helpers.contains(gname)) || !(!scc.is_disjoint(&lone) || scc.iter().any(|gname| search(&px.demand_fan, &body_of[gname]))) {
            continue;
        }
        if scc.iter().any(|gname| by[gname].iter().any(|&e| starts_demand(prog, e))) {
            continue;
        }
        let heads: BTreeMap<String, Vec<String>> = scc.iter().map(|gname| (gname.clone(), head_params(&prog.arena[by[gname][0]].head))).collect();
        if heads.values().any(|hp| hp[0] != "n") {
            continue;
        }
        let reaches = |prog: &Program, l: &str| -> bool {
            names_keep(px, l).iter().any(|gname| {
                by.contains_key(gname)
                    && (scc.contains(gname)
                        || !scc.is_disjoint(callees(gname))
                        || callees(gname).iter().chain(std::iter::once(gname)).any(|x| by.get(x).into_iter().flatten().any(|&e| starts_demand(prog, e))))
            })
        };
        let mut rules: BTreeMap<String, Vec<Rule>> = scc.iter().map(|gname| (gname.clone(), Vec::new())).collect();
        let mut ok = true;
        'entries: for &e in entries {
            if prog.arena[e].drop || !ok {
                continue;
            }
            let owner = prog.arena[e].name().to_owned();
            for gname in &scc {
                for site in call_sites(px, &prog.arena[e].lits.join(", "), gname, &[]) {
                    let Some((scope, call)) = site else {
                        ok = false;
                        break 'entries;
                    };
                    let x = split_args(&call[gname.len() + 1..call.len() - 1]).first().cloned().ok_or("IndexError: list index out of range")?;
                    if x == "_" {
                        ok = false;
                        break 'entries;
                    }
                    let mut lits: Vec<String> = scope.into_iter().filter(|l| !reaches(prog, l)).collect();
                    if scc.contains(&owner) {
                        let hp = head_params(&prog.arena[e].head);
                        lits.insert(0, format!("demand_{owner}({})", hp[0]));
                    } else if callees(gname).contains(&owner) || lits.is_empty() {
                        ok = false;
                        break 'entries;
                    }
                    let head = format!("demand_{gname}({x})");
                    if ungrounded(px, &wrap(&head, &lits, None))? {
                        ok = false;
                        break 'entries;
                    }
                    rules.get_mut(gname).expect("a member").push((head, lits));
                }
            }
        }
        if !ok || !scc.iter().all(|gname| !rules[gname].is_empty()) {
            continue;
        }
        for gname in &scc {
            let decl = &prog.decls[gname];
            let colv = decl[decl.find('(').expect("a declaration") + 1..decl.len() - 1].split(',').next().unwrap_or("").trim().to_owned();
            let rs = &rules[gname];
            let note = format!("// demand_{gname}: the nodes {gname} is asked at {}", if lone.contains(gname) { "(it looks at every ancestor of every node)" } else { "(its recursion looks at every descendant)" });
            let entry: LibEntry = (format!(".decl demand_{gname}({colv})"), vec![note], rs.iter().enumerate().filter(|(i, r)| !rs[..*i].contains(r)).map(|(_, r)| r.clone()).collect());
            prog.lib.insert(format!("demand_{gname}"), entry);
            for &e in &by[gname] {
                let hp = head_params(&prog.arena[e].head);
                prog.arena[e].lits.insert(0, format!("demand_{gname}({})", hp[0]));
            }
        }
    }
    Ok(())
}

/// An `inside` step whose ancestor must match a util, in a body that limits n by a util too, is its own relation.
pub fn hoist_ancestor_steps(px: &Patterns, prog: &mut Program, entries: &[usize]) {
    let mut made: Vec<((Vec<String>, String, Vec<String>), String)> = Vec::new();
    for &e in entries {
        let lits = prog.arena[e].lits.clone();
        prog.arena[e].lits = hoist(px, &mut prog.lib, &mut made, &lits, &[]);
    }
    let names: Vec<String> = prog.lib.keys().cloned().collect();
    for name in names {
        if name.starts_with("demand_") {
            let (decl, notes, rules) = prog.lib[&name].clone();
            let rules = rules.into_iter().map(|(h, lits)| (h, hoist(px, &mut prog.lib, &mut made, &lits, &[]))).collect();
            prog.lib.insert(name, (decl, notes, rules));
        }
    }
}

fn unary(px: &Patterns, x: &str) -> Option<(String, String)> {
    find(&px.demand_unary, x).map(|c| (g(&c, 1).to_owned(), g(&c, 2).to_owned()))
}

fn hoist(px: &Patterns, lib: &mut IndexMap<String, LibEntry>, made: &mut Vec<((Vec<String>, String, Vec<String>), String)>, lits: &[String], ctx: &[String]) -> Vec<String> {
    let mut out: Vec<String> = lits.to_vec();
    for j in 0..out.len() {
        let l = out[j].clone();
        if l.starts_with('(') && l.ends_with(')') && balanced(&l) && split_alts(&l[1..l.len() - 1]).len() > 1 {
            let mut outer: Vec<String> = out.iter().filter(|x| **x != l).cloned().collect();
            outer.extend(ctx.iter().cloned());
            let alts: Vec<Vec<String>> = split_alts(&l[1..l.len() - 1]).iter().map(|b| split_top_text(b.trim(), ',').into_iter().map(|q| q.trim().to_owned()).collect()).collect();
            let new: Vec<Vec<String>> = alts.iter().map(|b| hoist(px, lib, made, b, &outer)).collect();
            if new != alts {
                out[j] = format!("({})", new.iter().map(|b| b.join(", ")).collect::<Vec<_>>().join(" ; "));
            }
        }
    }
    for l in out.clone() {
        let Some(m) = find(&px.demand_ancestor, &l) else { continue };
        if !out.contains(&l) {
            continue;
        }
        let (a, p) = (g(&m, 1).to_owned(), g(&m, 2).to_owned());
        if a != "n" {
            continue;
        }
        let ua: Vec<String> = out.iter().chain(ctx).filter(|x| unary(px, x).is_some_and(|(r, v)| v == a && r != "node")).cloned().collect();
        let kind_p = format!(r#"kind\({p}, "\w+"\)"#);
        let up: Vec<String> = out.iter().filter(|x| unary(px, x).is_some_and(|(_, v)| v == p) || is_full(&kind_p, x)).cloned().collect();
        if ua.is_empty() || up.is_empty() {
            continue;
        }
        let from_above = format!(r#"(?:child\(n, {p}, \w+\)|field\(n, "\w+", {p}\))"#);
        if out.iter().chain(ctx).any(|x| is_full(&from_above, x)) {
            continue;
        }
        let key = (ua.clone(), l.clone(), up.clone());
        let name = match made.iter().find(|(k, _)| *k == key) {
            Some((_, n)) => n.clone(),
            None => {
                let name = format!("up_{}", made.len() + 1);
                let renamed = a != "p" && p != "n";
                let (pa, pn) = (re(&format!(r"\b{p}\b")), re(&format!(r"\b{a}\b")));
                let ren = |t: &str| if renamed { sub(&pn, &sub(&pa, t, |_| "p".into()), |_| "n".into()) } else { t.to_owned() };
                made.push((key, name.clone()));
                let rule: Rule = if renamed {
                    let mut b: Vec<String> = ua.iter().map(|x| ren(x)).collect();
                    b.push(ren(&l));
                    b.extend(up.iter().map(|x| ren(x)));
                    (format!("{name}(n, p)"), b)
                } else {
                    let mut b = ua.clone();
                    b.push(l.clone());
                    b.extend(up.iter().cloned());
                    (format!("{name}({a}, {p})"), b)
                };
                lib.insert(
                    name.clone(),
                    (format!(".decl {name}(n: node, p: node)"), vec![format!("// {name}(n, p): p is an ancestor of n the `inside` step stops at and matches, from the n the rule allows")], vec![rule]),
                );
                name
            }
        };
        let i = out.iter().position(|x| *x == l).expect("present");
        let mut next: Vec<String> = out[..i].iter().filter(|x| !up.contains(x)).cloned().collect();
        next.push(format!("{name}({a}, {p})"));
        next.extend(out[i + 1..].iter().filter(|x| !up.contains(x)).cloned());
        out = next;
    }
    out
}

struct Tarjan<'a> {
    graph: &'a BTreeMap<String, Vec<(String, bool)>>,
    index: BTreeMap<String, usize>,
    low: BTreeMap<String, usize>,
    stack: Vec<String>,
    on: Set,
    sccs: Vec<Vec<String>>,
    counter: usize,
}

impl Tarjan<'_> {
    fn strong(&mut self, v: &str) {
        self.index.insert(v.to_owned(), self.counter);
        self.low.insert(v.to_owned(), self.counter);
        self.counter += 1;
        self.stack.push(v.to_owned());
        self.on.insert(v.to_owned());
        for (w, _) in &self.graph[v] {
            if !self.index.contains_key(w) {
                self.strong(w);
                let lw = self.low[w];
                let lv = self.low.get_mut(v).expect("visited");
                *lv = (*lv).min(lw);
            } else if self.on.contains(w) {
                let iw = self.index[w];
                let lv = self.low.get_mut(v).expect("visited");
                *lv = (*lv).min(iw);
            }
        }
        if self.low[v] == self.index[v] {
            let mut comp = Vec::new();
            loop {
                let w = self.stack.pop().expect("on the stack");
                self.on.remove(&w);
                let stop = w == v;
                comp.push(w);
                if stop {
                    break;
                }
            }
            self.sccs.push(comp);
        }
    }
}

/// A recursive group of helpers through `!` or an aggregate becomes UNROLL levels, level k reading level k-1.
/// Returns the new entries and the relations declared empty.
pub fn unroll_negative(px: &Patterns, prog: &mut Program, entries: &[usize], helpers: &Set, inputs: &mut BTreeMap<String, Set>) -> (Vec<usize>, Vec<(String, String)>) {
    let by = by_head(prog, entries);
    let names: Vec<String> = helpers.iter().filter(|h| by.contains_key(*h)).cloned().collect();
    let mut graph: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();
    for h in &names {
        let mut out = Vec::new();
        for &e in &by[h] {
            let text = mask_ticks(px, &strip_strings_keep(px, &prog.arena[e].lits.join(", ")));
            for c in captures(&px.text_called, &text) {
                let gname = g(&c, 1);
                if by.contains_key(gname) && helpers.contains(gname) {
                    let before = &text[..c.get(0).expect("a match").start()];
                    let depth = before.matches('{').count() as i64 - before.matches('}').count() as i64;
                    out.push((gname.to_owned(), depth > 0 || before.trim_end().ends_with('!')));
                }
            }
        }
        graph.insert(h.clone(), out);
    }
    let mut t = Tarjan { graph: &graph, index: BTreeMap::new(), low: BTreeMap::new(), stack: Vec::new(), on: Set::new(), sccs: Vec::new(), counter: 0 };
    for h in &names {
        if !t.index.contains_key(h) {
            t.strong(h);
        }
    }
    let sccs = t.sccs;
    let (mut new, mut empty): (Vec<usize>, Vec<(String, String)>) = (Vec::new(), Vec::new());
    for comp in sccs {
        let members: Set = comp.iter().cloned().collect();
        if !comp.iter().any(|v| graph[v].iter().any(|(w, neg)| *neg && members.contains(w))) {
            continue;
        }
        let ms: Vec<String> = members.iter().cloned().collect();
        let pat = re(&format!(r"(?<![\w.])({})\(", alternation(&ms)));
        for h in &members {
            let sig = prog.decls[h].clone();
            for k in 0..UNROLL {
                let uk = format!("{h}_u{k}");
                prog.decls.insert(uk.clone(), sig.replacen(&format!(".decl {h}("), &format!(".decl {uk}("), 1));
                inputs.insert(uk.clone(), inputs.get(h).cloned().unwrap_or_default());
                let before = new.len();
                for &e in &by[h] {
                    let lits: Vec<String> = prog.arena[e]
                        .lits
                        .iter()
                        .map(|l| {
                            split1(&px.text_pieces, l)
                                .into_iter()
                                .map(|p| if p.starts_with('"') || p.starts_with('`') { p } else { sub(&pat, &p, |c| if k >= 1 { format!("{}_u{}(", g(c, 1), k - 1) } else { format!("{}_u0_empty(", g(c, 1)) }) })
                                .collect()
                        })
                        .collect();
                    if k == 0 && lits.iter().filter(|l| !l.starts_with('!') && !l.contains('{')).any(|l| l.contains("_u0_empty(")) {
                        continue;
                    }
                    let lits: Vec<String> = lits.iter().map(|l| sub(&px.demand_empty_neg, l, |c| if has(c, 1) && has(c, 2) { ", ".into() } else { String::new() })).filter(|l| !l.trim().is_empty()).collect();
                    let head = prog.arena[e].head.replacen(&format!("{h}("), &format!("{uk}("), 1);
                    let id = prog.entry(Entry { head, lits, ..Default::default() });
                    new.push(id);
                }
                if new.len() == before {
                    empty.push((uk.clone(), prog.decls[&uk].clone()));
                }
            }
            empty.push((format!("{h}_u0_empty"), sig.replacen(&format!(".decl {h}("), &format!(".decl {h}_u0_empty("), 1)));
            let first = by[h][0];
            let args = prog.arena[first].head.split_once('(').map_or("", |(_, t)| t).to_owned();
            prog.arena[first].head_note = Some(format!("// {h}: level {} of its recursion through `!` (unroll_negative)", UNROLL - 1));
            prog.arena[first].lits = vec![format!("{h}_u{}({args}", UNROLL - 1)];
            for &e in &by[h][1..] {
                prog.arena[e].drop = true;
            }
        }
    }
    (new, empty)
}
