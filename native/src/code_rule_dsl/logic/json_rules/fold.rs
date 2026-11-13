//! `fold.py`: families of copy-pasted relations (alike up to their strings and to which member of another family
//! they call) folded into one relation with a key column and a table of the strings that differed.

use super::program::Program;
use super::patterns::Patterns;
use super::pyre::{captures, findall, g, has, re, search, split};
use super::simplify::{Canon, Rules, Set, calls as _calls, canon, code, is_generated, locals, split_protected};
use super::value::{Value, py_repr_str};
use fancy_regex::Regex;
use indexmap::IndexMap;
use std::collections::BTreeMap;

const MIN_FAMILY: usize = 3;
const PH_S: &str = "\u{0}S\u{0}";
const PH_R: &str = "\u{0}R\u{0}";

/// The relation and functor names the library declares (`^\.(decl|edit|functor) name`), and the built-in
/// names a family must not take.
pub fn reserved(lib: &std::path::Path) -> Set {
    let mut names = Set::new();
    let decl = re(r"(?m)^\.(?:decl|edit|functor)\s+(\w+)");
    let mut stack = vec![lib.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rr")
                && let Ok(text) = std::fs::read_to_string(&p)
            {
                names.extend(findall(&decl, &text));
            }
        }
    }
    for n in ["node", "kind", "field", "child", "text", "match", "glob", "count", "cat", "strlen", "min", "max", "sum", "mean", "range", "as", "true", "false", "join", "by", "sep"] {
        names.insert(n.to_owned());
    }
    names
}

#[derive(Clone, Debug, PartialEq)]
enum Slot {
    S(String),
    R(String, String),
}

impl Slot {
    fn text(&self) -> &str {
        match self {
            Slot::S(s) | Slot::R(s, _) => s,
        }
    }
    fn repr(&self) -> String {
        match self {
            Slot::S(s) => format!("('S', {})", py_repr_str(s)),
            Slot::R(n, a) => format!("('R', {}, {})", py_repr_str(n), py_repr_str(a)),
        }
    }
}


/// Lit with each string literal and each call of a candidate as a placeholder; the slots in text order.
fn template(px: &Patterns, lit: &str, cands: &Set) -> (String, Vec<Slot>) {
    let parts = split_protected(px, lit);
    let (mut out, mut slots) = (String::new(), Vec::new());
    for (i, p) in parts.iter().enumerate() {
        if i % 2 == 1 {
            if p.starts_with('"') && !search(&px.fold_named, p) {
                out.push_str(PH_S);
                slots.push(Slot::S(p.clone()));
            } else {
                out.push_str(p);
            }
            continue;
        }
        out.push_str(&super::pyre::sub(&px.fold_template_call, p, |c| {
            if cands.contains(g(c, 1)) {
                slots.push(Slot::R(g(c, 1).to_owned(), g(c, 2).to_owned()));
                format!("{PH_R}({})", g(c, 2))
            } else {
                g(c, 0).to_owned()
            }
        }));
    }
    (out, slots)
}

fn bare_uses(px: &Patterns, rs: &Rules, prog: &Program, names: &Set) -> Set {
    let mut bare = Set::new();
    for r in rs.all() {
        for l in &prog.arena[r].lits {
            bare.extend(findall(&px.fold_bare, &code(px, l)).into_iter().filter(|x| names.contains(x)));
        }
    }
    bare
}

fn key_and_name(members: &[String]) -> (IndexMap<String, String>, String) {
    let toks: Vec<Vec<&str>> = members.iter().map(|m| m.split('_').collect()).collect();
    let mut pre = 0;
    while toks.iter().all(|t| t.len() > pre) && toks.iter().map(|t| t[pre]).collect::<std::collections::BTreeSet<_>>().len() == 1 {
        pre += 1;
    }
    let mut suf = 0;
    while toks.iter().all(|t| t.len() > pre + suf) && toks.iter().map(|t| t[t.len() - 1 - suf]).collect::<std::collections::BTreeSet<_>>().len() == 1 {
        suf += 1;
    }
    let mut keys: IndexMap<String, String> = members.iter().zip(&toks).map(|(m, t)| (m.clone(), t[pre..t.len() - suf].join("_"))).collect();
    let distinct: std::collections::BTreeSet<&String> = keys.values().collect();
    if keys.values().any(String::is_empty) || distinct.len() != members.len() {
        keys = members.iter().map(|m| (m.clone(), m.clone())).collect();
    }
    let t0 = &toks[0];
    let mut parts: Vec<&str> = t0[..pre].to_vec();
    if suf > 0 {
        parts.extend(t0[t0.len() - suf..].iter());
    }
    (keys, parts.join("_"))
}

#[derive(Clone, Debug)]
enum Plan {
    Const(Option<String>),
    Col(Vec<String>),
    CallSame(usize),
    CallCol(usize, Vec<String>),
}

#[derive(Clone, Debug)]
struct Fam {
    id: usize,
    members: Vec<String>,
    keys: IndexMap<String, String>,
    name: String,
    plan: Vec<Plan>,
    keyless: bool,
    key: String,
}

/// One rule of a member: (canon, templates, slots, the rule).
type Row = ((String, Vec<String>), Vec<String>, Vec<Slot>, usize);

/// Fold in place; the number of relations folded away. `allowed`, on a retry, limits which relations may join.
pub fn fold(px: &Patterns, rs: &mut Rules, prog: &mut Program, allowed: Option<&Set>, reserved: &Set) -> i64 {
    let defs = rs.defs(prog);
    let mut cands: Set = defs
        .iter()
        .filter(|(n, rules)| !rs.protected.contains(*n) && !rs.facts.contains_key(*n) && rules.iter().all(|&r| prog.arena[r].head == format!("{n}(n)")))
        .map(|(n, _)| n.clone())
        .collect();
    if let Some(a) = allowed {
        cands = cands.intersection(a).cloned().collect();
    }
    let bu = bare_uses(px, rs, prog, &cands);
    cands = cands.difference(&bu).cloned().collect();
    let nc = nested_calls(px, rs, prog, &cands);
    cands = cands.difference(&nc).cloned().collect();
    let mut info: IndexMap<String, Vec<Row>> = IndexMap::new();
    for n in &cands {
        let mut rows: Vec<Row> = Vec::new();
        for &r in &defs[n] {
            let tl: Vec<(String, Vec<Slot>)> = prog.arena[r].lits.iter().map(|l| template(px, l, &cands)).collect();
            let tmpls: Vec<String> = tl.iter().map(|(t, _)| t.clone()).collect();
            let c: Canon = canon(px, &[("n".to_owned(), tmpls.clone())]);
            let slots: Vec<Slot> = tl.into_iter().flat_map(|(_, s)| s).collect();
            rows.push((c[0].clone(), tmpls, slots, r));
        }
        rows.sort_by(|a, b| {
            let ra = format!("[{}]", a.2.iter().map(Slot::repr).collect::<Vec<_>>().join(", "));
            let rb = format!("[{}]", b.2.iter().map(Slot::repr).collect::<Vec<_>>().join(", "));
            (&a.0, ra).cmp(&(&b.0, rb))
        });
        info.insert(n.clone(), rows);
    }
    let mut groups: IndexMap<Vec<(String, Vec<String>)>, Vec<String>> = IndexMap::new();
    for (n, rows) in &info {
        groups.entry(rows.iter().map(|x| x.0.clone()).collect()).or_default().push(n.clone());
    }
    let mut fams: Vec<Fam> = Vec::new();
    for (_, mut members) in groups {
        if members.len() >= MIN_FAMILY {
            members.sort();
            let (keys, name) = key_and_name(&members);
            fams.push(Fam { id: fams.len(), members, keys, name, plan: Vec::new(), keyless: false, key: String::new() });
        }
    }
    loop {
        let fam_of: BTreeMap<String, usize> = fams.iter().flat_map(|f| f.members.iter().map(move |m| (m.clone(), f.id))).collect();
        let mut drop = Vec::new();
        for fi in 0..fams.len() {
            let f = &fams[fi];
            let slots: Vec<Vec<Slot>> = f.members.iter().map(|m| info[m].iter().flat_map(|x| x.2.iter().cloned()).collect()).collect();
            let mut plan = Vec::new();
            let mut ok = true;
            for j in 0..slots[0].len() {
                let col: Vec<&Slot> = slots.iter().map(|s| &s[j]).collect();
                if let Slot::S(_) = col[0] {
                    let texts: Vec<String> = col.iter().map(|c| c.text().to_owned()).collect();
                    let distinct: std::collections::BTreeSet<&String> = texts.iter().collect();
                    plan.push(if distinct.len() == 1 { Plan::Const(Some(texts[0].clone())) } else { Plan::Col(texts) });
                    continue;
                }
                let targets: Vec<String> = col.iter().map(|c| c.text().to_owned()).collect();
                if targets.iter().collect::<std::collections::BTreeSet<_>>().len() == 1 {
                    plan.push(Plan::Const(None));
                    continue;
                }
                let gid = fam_of.get(&targets[0]).copied();
                let Some(gid) = gid.filter(|gid| targets.iter().all(|t| fam_of.get(t) == Some(gid))) else {
                    ok = false;
                    break;
                };
                let gf = fams.iter().find(|x| x.id == gid).expect("a family");
                let gkeys: Vec<String> = targets.iter().map(|t| gf.keys[t].clone()).collect();
                if gkeys.iter().zip(&f.members).all(|(gk, m)| *gk == f.keys[m]) {
                    plan.push(Plan::CallSame(gid));
                } else {
                    plan.push(Plan::CallCol(gid, gkeys.iter().map(|k| format!("\"{k}\"")).collect()));
                }
            }
            if ok {
                fams[fi].plan = plan;
            } else {
                drop.push(fams[fi].id);
            }
        }
        if drop.is_empty() {
            break;
        }
        fams.retain(|f| !drop.contains(&f.id));
    }
    if fams.is_empty() {
        return 0;
    }
    let node_of: BTreeMap<String, String> = fams.iter().flat_map(|f| f.members.iter().map(move |m| (m.clone(), format!("\u{0}{}", f.id)))).collect();
    let bad = unstratified(px, prog, &defs, &node_of);
    if !bad.is_empty() {
        let mut keep: Set = fams.iter().filter(|f| !bad.contains(&format!("\u{0}{}", f.id))).flat_map(|f| f.members.iter().cloned()).collect();
        if let Some(a) = allowed {
            keep = keep.intersection(a).cloned().collect();
        }
        return if keep.is_empty() { 0 } else { fold(px, rs, prog, Some(&keep), reserved) };
    }
    let mut used: Set = defs.keys().cloned().collect();
    used.extend(rs.facts.keys().cloned());
    used.extend(prog.decls.keys().cloned());
    used.extend(reserved.iter().cloned());
    for f in fams.iter_mut() {
        let mut base = if f.name.is_empty() { format!("{}_family", f.members[0]) } else { f.name.clone() };
        while used.contains(&base) || (is_generated(px, &base) && !f.members.contains(&base)) {
            base += "_of";
        }
        if f.members.contains(&base) {
            base += "_of";
        }
        f.name = base.clone();
        used.insert(base);
    }
    let fam_of: BTreeMap<String, usize> = fams.iter().flat_map(|f| f.members.iter().map(move |m| (m.clone(), f.id))).collect();
    for f in fams.iter_mut() {
        f.keyless = f.plan.iter().all(|p| matches!(p, Plan::Const(_) | Plan::CallSame(_)));
    }
    loop {
        let keyless: BTreeMap<usize, bool> = fams.iter().map(|f| (f.id, f.keyless)).collect();
        let lose: Vec<usize> = fams.iter().filter(|f| f.keyless && f.plan.iter().any(|p| matches!(p, Plan::CallSame(gid) if !keyless.get(gid).copied().unwrap_or(false)))).map(|f| f.id).collect();
        if lose.is_empty() {
            break;
        }
        for f in fams.iter_mut().filter(|f| lose.contains(&f.id)) {
            f.keyless = false;
        }
    }
    let by_id: BTreeMap<usize, (String, bool, IndexMap<String, String>)> = fams.iter().map(|f| (f.id, (f.name.clone(), f.keyless, f.keys.clone()))).collect();
    let call_of = |fid: usize, args: &str, key: &str| -> String {
        let (name, keyless, _) = &by_id[&fid];
        if *keyless { format!("{name}({args})") } else { format!("{name}({args}, {key})") }
    };
    let rewrite_calls = |lit: &str| -> String {
        let mut parts = split_protected(px, lit);
        for i in (0..parts.len()).step_by(2) {
            parts[i] = super::pyre::sub(&px.fold_template_call, &parts[i], |c| match fam_of.get(g(c, 1)) {
                Some(fid) => call_of(*fid, g(c, 2), &format!("\"{}\"", by_id[fid].2[g(c, 1)])),
                None => g(c, 0).to_owned(),
            });
        }
        parts.concat()
    };
    let mut gone = Set::new();
    let mut news: Vec<(usize, Vec<(usize, Vec<String>)>)> = Vec::new();
    for fi in 0..fams.len() {
        let f = fams[fi].clone();
        let (ms, plan, name) = (&f.members, &f.plan, &f.name);
        let first = &info[&ms[0]];
        let mut taken: Set = Set::new();
        for (_, tmpl, _, _) in first {
            taken.extend(locals(px, tmpl));
            taken.insert("n".into());
        }
        let fresh = |base: &str, taken: &mut Set| {
            let mut k = 1;
            while taken.contains(&format!("{base}{k}")) {
                k += 1;
            }
            let v = format!("{base}{k}");
            taken.insert(v.clone());
            v
        };
        let key = fresh("q", &mut taken);
        fams[fi].key = key.clone();
        let mut cols: Vec<Vec<String>> = Vec::new();
        let mut colvar: Vec<(Vec<String>, String)> = Vec::new();
        let keyvals: Vec<String> = ms.iter().map(|m| format!("\"{}\"", f.keys[m])).collect();
        let mut column = |vals: &[String], taken: &mut Set| -> String {
            if vals == keyvals.as_slice() {
                return key.clone();
            }
            if let Some((_, v)) = colvar.iter().find(|(k, _)| k.as_slice() == vals) {
                return v.clone();
            }
            let v = fresh("s", taken);
            colvar.push((vals.to_vec(), v.clone()));
            cols.push(vals.to_vec());
            v
        };
        enum Sub {
            Keep,
            Var(String),
            Call(usize, String),
        }
        let mut subst: Vec<Sub> = Vec::new();
        for p in plan {
            subst.push(match p {
                Plan::Const(_) => Sub::Keep,
                Plan::Col(vals) => Sub::Var(column(vals, &mut taken)),
                Plan::CallSame(gid) => Sub::Call(*gid, key.clone()),
                Plan::CallCol(gid, vals) => Sub::Call(*gid, column(vals, &mut taken)),
            });
        }
        let mut table = format!("{name}_case");
        while used.contains(&table) {
            table += "_of";
        }
        used.insert(table.clone());
        let jnames: Vec<String> = ms.iter().map(|m| json_name(prog.notes.get(m).map(Vec::as_slice).unwrap_or(&[]), m)).collect();
        let shown = jnames.iter().take(3).map(|j| format!("`{j}`")).collect::<Vec<_>>().join(", ") + &if ms.len() > 3 { format!(" and {} more", ms.len() - 3) } else { String::new() };
        let colvar_of = |c: &Vec<String>| colvar.iter().find(|(k, _)| k == c).map(|(_, v)| v.clone()).expect("a column");
        if f.keyless {
            prog.decls.insert(name.clone(), format!(".decl {name}(n: node)"));
            prog.notes.insert(name.clone(), vec![format!("// {name}(n): {} identical copies of one rule set ({shown}), folded into one.", ms.len())]);
        } else {
            let mut rows: Vec<Value> = Vec::new();
            for (i, m) in ms.iter().enumerate() {
                let mut row = vec![format!("\"{}\"", f.keys[m])];
                row.extend(cols.iter().map(|c| c[i].clone()));
                let vals: Vec<Value> = row.iter().map(|x| super::value::parse(x).unwrap_or(Value::Str(x.trim_matches('"').to_owned()))).collect();
                rows.push(Value::List(vals));
            }
            rs.facts.insert(table.clone(), rows);
            prog.decls.insert(table.clone(), format!(".decl {table}(key: symbol{})", cols.iter().map(|c| format!(", {}: symbol", colvar_of(c))).collect::<String>()));
            prog.notes.insert(table.clone(), vec![format!("// {table}: one row per relation folded into {name}: its key, then the strings that differed.")]);
            prog.decls.insert(name.clone(), format!(".decl {name}(n: node, key: symbol)"));
            prog.notes.insert(
                name.clone(),
                vec![format!("// {name}(n, key): {} copies of one rule set ({shown}), folded into one: row `key` of", ms.len()), format!("// {table} gives the strings each copy had.")],
            );
        }
        let mut si = 0usize;
        let mut new_rules = Vec::new();
        let mut before = 0usize;
        for (_, tmpl, slots0, r) in first {
            let mut lits = if f.keyless {
                Vec::new()
            } else {
                let mut a = vec![key.clone()];
                a.extend(cols.iter().map(colvar_of));
                vec![format!("{table}({})", a.join(", "))]
            };
            for t in tmpl {
                let pieces = split(&px.fold_piece, t);
                let mut out = String::new();
                let mut k = 0;
                while k < pieces.len() {
                    out.push_str(pieces[k].as_deref().unwrap_or(""));
                    if k + 1 < pieces.len() {
                        let whole = pieces[k + 1].clone().unwrap_or_default();
                        let arg = pieces[k + 2].clone().unwrap_or_default();
                        let slot = &slots0[si - before];
                        if whole == PH_S {
                            match &subst[si] {
                                Sub::Var(v) => out.push_str(v),
                                _ => out.push_str(slot.text()),
                            }
                        } else {
                            let callee = slot.text().to_owned();
                            match &subst[si] {
                                Sub::Call(gid, kexpr) => out.push_str(&call_of(*gid, &arg, kexpr)),
                                _ => out.push_str(&rewrite_calls(&format!("{callee}({arg})"))),
                            }
                        }
                        si += 1;
                    }
                    k += 3;
                }
                lits.push(out);
            }
            before += slots0.len();
            new_rules.push((*r, lits));
        }
        news.push((f.id, new_rules));
        gone.extend(ms.iter().cloned());
    }
    let mut keep = std::collections::BTreeSet::new();
    for (fid, new_rules) in &news {
        let f = fams.iter().find(|f| f.id == *fid).expect("a family");
        for (r, lits) in new_rules {
            prog.arena[*r].head = if f.keyless { format!("{}(n)", f.name) } else { format!("{}(n, {})", f.name, f.key) };
            prog.arena[*r].lits = lits.clone();
            keep.insert(*r);
        }
    }
    rs.helper_rules.retain(|h| keep.contains(h) || !gone.contains(prog.arena[*h].name()));
    rs.entries.retain(|h| keep.contains(h) || !gone.contains(prog.arena[*h].name()));
    for r in rs.all() {
        if !keep.contains(&r) {
            let lits: Vec<String> = prog.arena[r].lits.iter().map(|l| rewrite_calls(l)).collect();
            prog.arena[r].lits = lits;
        }
    }
    for n in &gone {
        prog.decls.shift_remove(n);
    }
    gone.len() as i64 - fams.len() as i64
}

fn json_name(note: &[String], m: &str) -> String {
    let r = re(r"the JSON util `([^`]+)`");
    for line in note {
        if let Some(c) = super::pyre::find(&r, line) {
            return g(&c, 1).to_owned();
        }
    }
    m.to_owned()
}

/// Collapsed family nodes on a cycle that has an edge through `!` or an aggregate.
fn unstratified(px: &Patterns, prog: &Program, defs: &IndexMap<String, Vec<usize>>, node_of: &BTreeMap<String, String>) -> Set {
    let mut graph: BTreeMap<String, Set> = BTreeMap::new();
    let mut neg: Vec<(String, String)> = Vec::new();
    for (name, rules) in defs {
        let a = node_of.get(name).cloned().unwrap_or_else(|| name.clone());
        for &r in rules {
            for l in &prog.arena[r].lits {
                let c = code(px, l);
                let negative = l.trim_start().starts_with('!') || l.contains(" : {") || c.contains('!');
                let mut called: Set = findall(&px.simplify_call, &c).into_iter().collect();
                called.extend(findall(&px.fold_word, &c).into_iter().filter(|x| is_generated(px, x)));
                for x in called {
                    if defs.contains_key(&x) {
                        let b = node_of.get(&x).cloned().unwrap_or_else(|| x.clone());
                        graph.entry(a.clone()).or_default().insert(b.clone());
                        if negative {
                            neg.push((a.clone(), b));
                        }
                    }
                }
            }
        }
    }
    let comp = scc(&graph);
    let fam_nodes: Set = node_of.values().cloned().collect();
    let mut bad = Set::new();
    for (a, b) in neg {
        if comp.get(&a) == comp.get(&b) {
            for x in [a, b] {
                if fam_nodes.contains(&x) {
                    bad.insert(x);
                }
            }
        }
    }
    bad
}

/// Tarjan's strongly connected components: node -> a representative of its component.
fn scc(graph: &BTreeMap<String, Set>) -> BTreeMap<String, String> {
    let mut nodes: Set = graph.keys().cloned().collect();
    for bs in graph.values() {
        nodes.extend(bs.iter().cloned());
    }
    let (mut index, mut low, mut comp): (BTreeMap<String, usize>, BTreeMap<String, usize>, BTreeMap<String, String>) = Default::default();
    let mut stack: Vec<String> = Vec::new();
    let mut on = Set::new();
    let empty = Set::new();
    for root in &nodes {
        if index.contains_key(root) {
            continue;
        }
        let n = index.len();
        index.insert(root.clone(), n);
        low.insert(root.clone(), n);
        stack.push(root.clone());
        on.insert(root.clone());
        let mut work: Vec<(String, Vec<String>, usize)> = vec![(root.clone(), graph.get(root).unwrap_or(&empty).iter().cloned().collect(), 0)];
        while let Some(top) = work.last_mut() {
            if top.2 < top.1.len() {
                let nxt = top.1[top.2].clone();
                top.2 += 1;
                let v = top.0.clone();
                if !index.contains_key(&nxt) {
                    let n = index.len();
                    index.insert(nxt.clone(), n);
                    low.insert(nxt.clone(), n);
                    stack.push(nxt.clone());
                    on.insert(nxt.clone());
                    let next_list = graph.get(&nxt).unwrap_or(&empty).iter().cloned().collect();
                    work.push((nxt, next_list, 0));
                } else if on.contains(&nxt) {
                    let l = low[&v].min(index[&nxt]);
                    low.insert(v, l);
                }
                continue;
            }
            let (v, _, _) = work.pop().expect("a frame");
            if let Some(parent) = work.last() {
                let l = low[&parent.0].min(low[&v]);
                low.insert(parent.0.clone(), l);
            }
            if low[&v] == index[&v] {
                loop {
                    let w = stack.pop().expect("a node");
                    on.remove(&w);
                    comp.insert(w.clone(), v.clone());
                    if w == v {
                        break;
                    }
                }
            }
        }
    }
    comp
}

/// Relations called with an argument that holds parentheses: the call rewrite cannot take them.
fn nested_calls(px: &Patterns, rs: &Rules, prog: &Program, names: &Set) -> Set {
    let mut out = Set::new();
    let tail = re(r"^[^()]*\)");
    for r in rs.all() {
        for l in &prog.arena[r].lits {
            let c = code(px, l);
            for m in captures(&px.simplify_call, &c) {
                let end = m.get(0).expect("group 0").end();
                if names.contains(g(&m, 1)) && !search(&tail, &c[end..]) {
                    out.insert(g(&m, 1).to_owned());
                }
            }
        }
    }
    out
}

