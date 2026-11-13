//! `translate.translate(doc)`: one JSON policy as display `.rr` text. The clauses are lowered construct by
//! construct (lower.rs), simplified (simplify.rs, fold.rs), given shared metavariable columns (share.rs) and
//! demand (demand.rs), then printed: notes, declarations, rules, the library part and the helpers.

use super::demand::{demand, hoist_ancestor_steps, node_demand, unroll_negative};
use super::lower::{R, col, fix_edits, grouped, ident, lower, mv, render_util, snake, transform_lits, transform_targets, violation_columns, violation_lines};
use super::program::{Entry, PRule, Program, Vars};
use super::patterns::Patterns;
use super::pyre::{find, findall, findall_groups, g, is_full, re, search, split1, sub};
use super::share::{drop_dead_columns, inline_unsafe, passed_in_patterns, share_metavariables, stop_with_columns};
use super::simplify::simplify;
use super::souffle::check_rule;
use super::text::{Set, binders, bind_read_nodes, head_span_ok, mask_ticks, split_top_text, strip_strings, transform_defs_for, wrap};
use super::value::{Obj, Value, compact, parse, py_str, q, q_val};
use fancy_regex::Regex;
use std::collections::{BTreeMap, BTreeSet};

/// An item of the body, in order (`("opt" | "rule" | "note" | "viol" | "raw", item)`).
#[derive(Clone, Debug)]
enum Item {
    Line(String),
    Rule(usize),
    Note(Vec<String>),
    Viol(Vec<String>),
}


fn empty_obj() -> &'static Obj {
    static E: Obj = Obj(Vec::new());
    &E
}

fn obj_of<'a>(v: Option<&'a Value>) -> &'a Obj {
    v.and_then(Value::as_obj).unwrap_or(empty_obj())
}

fn name_of(head: &str) -> &str {
    head.split('(').next().unwrap_or("")
}

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn common_prefix(xs: &[Vec<char>]) -> Vec<char> {
    let mut n = xs.iter().map(Vec::len).min().unwrap_or(0);
    for x in &xs[1..] {
        n = n.min(xs[0].iter().zip(x).take_while(|(a, b)| a == b).count());
    }
    xs[0][..n].to_vec()
}

fn st(cs: &[char]) -> String {
    cs.iter().collect()
}

/// `fold_messages`: clauses that differ only in their messages become one violation with a `detail` column.
fn fold_messages(px: &Patterns, prog: &mut Program, body: Vec<Item>) -> Vec<Item> {
    let rels_with_rules: Set = body.iter().filter_map(|i| if let Item::Rule(e) = i { Some(prog.arena[*e].name().to_owned()) } else { None }).collect();
    let mut viols: Vec<((String, String, Vec<String>), Vec<(usize, String, String)>)> = Vec::new();
    for (i, item) in body.iter().enumerate() {
        let Item::Viol(lines) = item else { continue };
        let Some(m) = find(&px.translate_viol_head, &lines[0]) else { continue };
        if !rels_with_rules.contains(g(&m, 1)) || g(&m, 2).contains("detail") {
            continue;
        }
        let code = if super::pyre::has(&m, 3) { g(&m, 3).to_owned() } else { format!(r#" code "{}""#, g(&m, 1)) };
        let key = (g(&m, 2).to_owned(), code, lines[1..].to_vec());
        let msg = match parse(g(&m, 4)) {
            Ok(Value::Str(s)) => s,
            _ => String::new(),
        };
        let val = (i, g(&m, 1).to_owned(), msg);
        match viols.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(val),
            None => viols.push((key, vec![val])),
        }
    }
    let groups: Vec<Vec<(usize, String, String)>> = viols.into_iter().map(|(_, v)| v).filter(|v| v.len() > 1).collect();
    if groups.is_empty() {
        return body;
    }
    let mut rename: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    let mut drop: BTreeSet<usize> = BTreeSet::new();
    let mut header: BTreeMap<usize, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for grp in &groups {
        let msgs: Vec<Vec<char>> = grp.iter().map(|x| chars(&x.2)).collect();
        let mut pre = common_prefix(&msgs);
        let rev: Vec<Vec<char>> = msgs.iter().map(|x| x.iter().rev().cloned().collect()).collect();
        let mut suf: Vec<char> = common_prefix(&rev).into_iter().rev().collect();
        let short = msgs.iter().map(Vec::len).min().unwrap_or(0);
        if pre.len() + suf.len() > short {
            suf = suf[pre.len() + suf.len() - short..].to_vec();
        }
        if !pre.is_empty() && *pre.last().expect("a char") != ' ' && msgs.iter().any(|x| x.len() > pre.len() && x[pre.len()] != ' ') {
            let cut = pre.iter().rposition(|c| *c == ' ').map_or(0, |p| p + 1);
            pre.truncate(cut);
        }
        if !suf.is_empty() && !" ,.;:)".contains(suf[0]) && msgs.iter().any(|x| x.len() > suf.len() && x[x.len() - suf.len() - 1] != ' ') {
            let found: Vec<usize> = " ,.;:)".chars().filter_map(|c| suf.iter().position(|x| *x == c)).collect();
            let cut = found.into_iter().min().unwrap_or(suf.len());
            suf = suf[cut..].to_vec();
        }
        let holes: Vec<String> = msgs
            .iter()
            .map(|x| {
                let end = if suf.is_empty() { x.len() } else { x.len() - suf.len() };
                if end > pre.len() { st(&x[pre.len()..end]) } else { String::new() }
            })
            .collect();
        if holes.iter().any(|h| h.contains('{') || h.contains('}')) || grp.iter().any(|x| x.2.contains("{detail}")) {
            continue;
        }
        let same = msgs.iter().all(|x| *x == msgs[0]);
        let name = grp.iter().map(|x| x.1.clone()).min_by(|a, b| (a.chars().count(), a).cmp(&(b.chars().count(), b))).expect("a member");
        let first_i = grp[0].0;
        let Item::Viol(first_lines) = &body[first_i] else { unreachable!() };
        let m = find(&px.translate_viol_head, &first_lines[0]).expect("a violation head");
        let cols = format!("{}{}", g(&m, 2), if same { "" } else { ", detail: symbol" });
        let tmpl = if same { grp[0].2.clone() } else { format!("{}{{detail}}{}", st(&pre), st(&suf)) };
        let (mut cids, mut extra): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
        for (i, _, _) in grp {
            if *i > 0
                && let Item::Note(lines) = &body[i - 1]
            {
                cids.extend(findall(&px.translate_from_clause, &lines[0]));
                let add: Vec<String> = lines.iter().skip(2).filter(|l| !extra.contains(l)).cloned().collect();
                extra.extend(add);
            }
        }
        let mut shown = cids.iter().take(4).map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", ");
        if cids.len() > 4 {
            shown.push_str(&format!(" and {} more", cids.len() - 4));
        }
        let has3 = super::pyre::has(&m, 3);
        let code = if has3 { g(&m, 3).to_owned() } else { format!(" code {}", q(&grp[0].1)) };
        let mut lines = vec![format!(".violation {name}({cols}){} {}", if has3 || name != grp[0].1 { code.as_str() } else { "" }, q(&tmpl))];
        lines.extend(first_lines[1..].iter().cloned());
        let mut note = if same {
            vec![format!("// {name}(n): findings from clauses {shown}, which say the same."), "// Each rule is one way a JSON `rule` can match.".to_owned()]
        } else {
            vec![
                format!("// {name}(n, detail): findings from clauses {shown}. Their messages differ only in"),
                "// `detail`: each rule's head gives its clause's words. Each rule is one way a JSON `rule` can match.".to_owned(),
            ]
        };
        note.extend(extra);
        header.insert(first_i, (lines, note));
        for ((i, rel, _), h) in grp.iter().zip(holes) {
            rename.insert(rel.clone(), (name.clone(), if same { None } else { Some(h) }));
            if *i != first_i {
                drop.insert(*i);
            }
        }
    }
    if rename.is_empty() {
        return body;
    }
    let group_of: BTreeMap<String, String> = rename.iter().map(|(r, v)| (r.clone(), v.0.clone())).collect();
    let mut out: Vec<Item> = Vec::new();
    let mut moved: BTreeSet<usize> = BTreeSet::new();
    for (i, item) in body.iter().enumerate() {
        if drop.contains(&i) {
            if matches!(out.last(), Some(Item::Note(_))) {
                out.pop();
            }
            continue;
        }
        if let Item::Rule(e) = item
            && rename.contains_key(prog.arena[*e].name())
            && moved.contains(e)
        {
            continue;
        }
        if let Some((lines, note)) = header.get(&i) {
            if matches!(out.last(), Some(Item::Note(_))) {
                *out.last_mut().expect("a note") = Item::Note(note.clone());
            }
            out.push(Item::Viol(lines.clone()));
            let Item::Viol(own) = item else { unreachable!() };
            let target = group_of[g(&find(&px.translate_viol_head, &own[0]).expect("a violation head"), 1)].clone();
            for it2 in &body {
                let Item::Rule(e) = it2 else { continue };
                let rel = prog.arena[*e].name().to_owned();
                if group_of.get(&rel) == Some(&target) {
                    let head = prog.arena[*e].head.clone();
                    let t = head.split_once('(').map_or("", |(_, t)| t);
                    let args = &t[..t.len().saturating_sub(1)];
                    prog.arena[*e].head = match &rename[&rel].1 {
                        None => format!("{target}({args})"),
                        Some(h) => format!("{target}({args}, {})", q(h)),
                    };
                    out.push(Item::Rule(*e));
                    moved.insert(*e);
                }
            }
            continue;
        }
        out.push(item.clone());
    }
    out
}

/// The text of one translated policy.
pub fn translate(px: &Patterns, doc: &Value, reserved: &Set) -> R<String> {
    let empty_list = Vec::new();
    let clauses: &Vec<Value> = doc.get("clauses").and_then(Value::as_list).unwrap_or(&empty_list);
    let mut langs: Vec<Option<&Value>> = Vec::new();
    for c in clauses {
        if obj_of(c.get("forbid")).has("ast") {
            langs.push(obj_of(c.get("forbid")).get("ast").and_then(|a| a.get("language")));
        }
    }
    for c in clauses {
        if c.has("define") {
            langs.push(c.get("define").and_then(|d| d.get("language")));
        }
    }
    let lang = langs.into_iter().flatten().find(|l| l.truthy()).map(py_str).unwrap_or_default();
    let mut prog = Program::new(&lang);
    let mut head = vec![
        format!(".policy {} revision {}", doc.get("id").map_or(q(""), q_val), doc.get("revision").map_or("1".to_owned(), py_str)),
        ".guests off".to_owned(),
    ];
    if let Some(d) = doc.get("description").filter(|d| d.truthy()) {
        head.push(format!(".description {}", q_val(d)));
    }
    if !lang.is_empty() {
        head.push(format!(".language {lang}"));
    }
    head.push(format!(".mode {}", doc.get("mode").map_or("log".to_owned(), py_str)));
    let scope_v = doc.get("scope").filter(|s| s.truthy());
    let scope = obj_of(scope_v);
    for (k, word) in [("include", "include"), ("exclude", "exclude")] {
        if let Some(gs) = scope.get(k).filter(|v| v.truthy()) {
            let items: Vec<String> = gs.as_list().map(|l| l.iter().map(q_val).collect()).unwrap_or_default();
            head.push(format!(".scope {word} {}", items.join(", ")));
        }
    }
    for (k, val) in scope.iter() {
        if k != "include" && k != "exclude" {
            head.push(format!(".scope {k} {}", compact(val)));
        }
    }

    let mut body: Vec<Item> = Vec::new();
    let mut seen_rel = Set::new();
    for c in clauses {
        let cid = c.get("id").map_or("clause".to_owned(), py_str);
        if let Some(d) = c.get("define") {
            prog.scope = Default::default();
            let mut utils = Vec::new();
            for (name, rule) in obj_of(d.get("utils").filter(|u| u.truthy())).iter() {
                render_util(px, &mut prog, name, rule, &mut utils)?;
            }
            if let Some(o) = d.get("options").filter(|o| o.truthy()) {
                body.push(Item::Line(format!(".options {}", compact(o))));
            }
            for (h, lits) in utils {
                let id = prog.entry(Entry { head: h, lits, ..Default::default() });
                body.push(Item::Rule(id));
            }
            continue;
        }
        let forbid = obj_of(c.get("forbid"));
        let viol = obj_of(c.get("violation"));
        let code = viol.get("code").map_or(cid.clone(), py_str);
        let mut rel = ident(&code);
        if seen_rel.contains(&rel) {
            rel = format!("{rel}_{}", snake(&cid));
        }
        seen_rel.insert(rel.clone());
        if !forbid.has("ast") {
            body.push(Item::Note(vec![format!("// {rel}: a finding from a text rule. It has no AST pattern, so it stays as JSON.")]));
            body.push(Item::Viol(violation_lines(viol, &rel, &[], false, &BTreeSet::new(), &BTreeSet::new())));
            body.push(Item::Line(format!("{rel}(d) :- text_rule(d, {}).", compact(&Value::Obj(forbid.clone())))));
            prog.untranslated += 1;
            continue;
        }
        let a = obj_of(forbid.get("ast"));
        let utils = obj_of(a.get("utils").filter(|u| u.truthy()));
        let mut sc = indexmap::IndexMap::new();
        for name in utils.keys() {
            let s = snake(name);
            let v = if prog.util_rels.contains(&s) { format!("{s}_{}", snake(&cid)) } else { s };
            sc.insert(name.to_owned(), v);
        }
        prog.scope = sc;
        let mut local = Vec::new();
        for (name, rule) in utils.iter() {
            render_util(px, &mut prog, name, rule, &mut local)?;
        }
        let cols = violation_columns(viol);
        let mut note = vec![format!("// {rel}(n): a finding at node n, from clause `{cid}`."), "// Each rule below is one way the JSON `rule` can match.".to_owned()];
        if !cols.is_empty() {
            note.push("// Columns after `at` are the metavariables the JSON's message, anchor or `each` names: one finding per distinct row.".to_owned());
        }
        let opts = a.without(&["language", "rule", "utils", "constraints", "transform"]);
        let mut vs = Vars::default();
        let tr_v = a.get("transform").filter(|t| t.is_obj() && t.truthy());
        let tlits = match tr_v {
            Some(tr) => transform_lits(&mut prog, tr, &mut vs, &transform_targets(tr))?,
            None => Vec::new(),
        };
        if let Some(tr) = tr_v {
            note.push(format!(
                "// The JSON `transform` computes {} from other metavariables' text; its literals end each rule.",
                transform_targets(tr).into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        let mut vl = violation_lines(viol, &rel, &cols, true, &BTreeSet::new(), &prog.symbolic);
        let edits = fix_edits(viol);
        if edits.as_ref().is_some_and(|e| !e.is_empty()) {
            note.push("// `=> replace(…)` is today's fix template: the edit suggested with each finding.".to_owned());
        }
        if !opts.is_empty() {
            vl.push(format!("    options {}", compact(&Value::Obj(opts.clone()))));
        }
        if let Some(cons) = a.get("constraints").filter(|c| c.truthy()) {
            let names: Vec<String> = match cons {
                Value::Obj(o) => o.keys().map(str::to_owned).collect(),
                Value::List(l) => l.iter().map(py_str).collect(),
                Value::Str(s) => s.chars().map(String::from).collect(),
                _ => Vec::new(),
            };
            note.push(format!("// The JSON `constraints` on {} are added to every rule.", names.join(", ")));
        }
        if !opts.is_empty() {
            note.push("// `options` carries JSON settings that have no .rr form yet.".to_owned());
        }
        body.push(Item::Note(note));
        body.push(Item::Viol(vl));
        let default_rule = Value::Obj(Obj(Vec::new()));
        let rule = a.get("rule").unwrap_or(&default_rule);
        let bodies = match rule {
            Value::Obj(o) if o.key_set_is(&["any"]) && o.get("any").is_some_and(Value::is_list) => grouped(px, &mut prog, o.get("any").and_then(Value::as_list).expect("a list"), "n", &mut vs)?,
            _ => vec![lower(px, &mut prog, rule, "n", &mut vs, &[])?],
        };
        let mut cons = Vec::new();
        for (var, crule) in obj_of(a.get("constraints").filter(|c| c.truthy())).iter() {
            cons.extend(lower(px, &mut prog, crule, &mv(var), &mut vs, &[])?);
        }
        let hd = format!("{rel}(n{})", cols.iter().map(|c| format!(", {c}")).collect::<String>());
        for b in bodies {
            let mut lits = b;
            lits.extend(cons.iter().cloned());
            lits.extend(tlits.iter().cloned());
            let id = prog.entry(Entry { head: hd.clone(), lits, edits: edits.clone(), ..Default::default() });
            body.push(Item::Rule(id));
        }
        for (h, lits) in local {
            let id = prog.entry(Entry { head: h, lits, ..Default::default() });
            body.push(Item::Rule(id));
        }
    }

    // simplify.py: duplicate helpers, helpers used once, grounding atoms something else provides
    let mut entries: Vec<usize> = body.iter().filter_map(|i| if let Item::Rule(e) = i { Some(*e) } else { None }).collect();
    let mut protected = seen_rel.clone();
    for &e in &entries {
        if prog.arena[e].head.contains(',') {
            protected.insert(prog.arena[e].name().to_owned());
        }
    }
    simplify(px, &mut entries, &mut prog, protected, reserved);
    let keep: BTreeSet<usize> = entries.iter().copied().collect();
    body.retain(|i| !matches!(i, Item::Rule(e) if !keep.contains(e)));

    // helper relations share the JSON's metavariables with their callers
    let entries: Vec<usize> = body.iter().filter_map(|i| if let Item::Rule(e) = i { Some(*e) } else { None }).collect();
    let mut helper_entries = Vec::new();
    for r in prog.rules.clone() {
        if let PRule::Rule(id) = r {
            let (h, l) = (prog.arena[id].head.clone(), prog.arena[id].lits.clone());
            helper_entries.push(prog.entry(Entry { head: h, lits: l, ..Default::default() }));
        }
    }
    let helpers: Set = prog.decls.keys().cloned().collect();
    let mut all: Vec<usize> = entries.clone();
    all.extend(helper_entries.iter().copied());
    let mut shared = share_metavariables(px, &mut prog, &all, &helpers);
    for (name, ps) in &shared.order {
        if !ps.is_empty() && prog.decls.contains_key(name) {
            let d = prog.decls[name].clone();
            let add: String = ps
                .iter()
                .map(|p| format!(", {}: {}", col(p), if prog.numeric.contains(p) { "number" } else if prog.symbolic.contains(p) { "symbol" } else { "loc" }))
                .collect();
            prog.decls.insert(name.clone(), format!("{}{add})", &d[..d.len() - 1]));
            let ins: Vec<String> = ps.iter().filter(|p| shared.inputs[name].contains(*p)).cloned().collect();
            let outs: Vec<String> = ps.iter().filter(|p| !ins.contains(p)).cloned().collect();
            if !ins.is_empty() {
                let s = if ins.len() > 1 { "s" } else { "" };
                prog.notes.entry(name.clone()).or_default().push(format!("//   {}: metavariable{s} of the calling rule, passed in (bound by the caller)", ins.join(", ")));
            }
            if !outs.is_empty() {
                let s = if outs.len() > 1 { "s" } else { "" };
                prog.notes.entry(name.clone()).or_default().push(format!("//   {}: metavariable{s} this relation binds for the calling rule", outs.join(", ")));
            }
        }
    }
    passed_in_patterns(&mut prog, &all, &shared.inputs);
    drop_dead_columns(px, &mut prog, &all, &helpers, &shared);
    stop_with_columns(&mut prog, &all);
    inline_unsafe(px, &mut prog, &all, &helpers);
    let sites = demand(px, &mut prog, &all, &helpers, &shared.inputs)?;
    node_demand(px, &mut prog, &all, &helpers)?;
    for (dname, owner, i) in sites {
        for &e in &all {
            if prog.arena[e].drop || prog.arena[e].name() != owner {
                continue;
            }
            let first = prog.arena[e].lits.first().cloned().unwrap_or_default();
            let rules = &mut prog.lib.get_mut(&dname).expect("a demand relation").2;
            if first.starts_with(&format!("demand_{owner}(")) && !rules[i].1.contains(&first) {
                rules[i].1.insert(0, first);
            }
            break;
        }
    }
    let (unrolled, empty) = unroll_negative(px, &mut prog, &all, &helpers, &mut shared.inputs);
    body.extend(unrolled.iter().map(|&e| Item::Rule(e)));
    let live: Vec<usize> = all.iter().chain(&unrolled).copied().filter(|&e| !prog.arena[e].drop).collect();
    hoist_ancestor_steps(px, &mut prog, &live);
    for (name, decl) in empty {
        prog.lib.insert(name.clone(), (decl, vec![format!("// {name}: level -1 of an unrolled recursion, empty")], Vec::new()));
    }
    body.retain(|i| !matches!(i, Item::Rule(e) if prog.arena[*e].drop));
    let body = fold_messages(px, &mut prog, body);
    let mut hi = helper_entries.iter();
    enum HR {
        Facts(String, Vec<Value>),
        Rule(String, Vec<String>),
    }
    let mut helper_rules = Vec::new();
    for r in &prog.rules {
        match r {
            PRule::Facts { name, vals } => helper_rules.push(HR::Facts(name.clone(), vals.clone())),
            PRule::Rule(_) => {
                let &e = hi.next().expect("a helper entry");
                if !prog.arena[e].drop {
                    helper_rules.push(HR::Rule(prog.arena[e].head.clone(), prog.arena[e].lits.clone()));
                }
            }
        }
    }

    let mut out: Vec<String> = head;
    out.push("// Generated from today's JSON by docs/rule-syntax/site/translate.py. Display only: not compiled or run.".to_owned());
    out.push(String::new());
    if !prog.uses.is_empty() {
        out.extend(prog.uses.iter().map(|u| format!("#include \"{u}.rr\"")));
        out.push(String::new());
    }
    let mut em = Emit { px, prog: &prog, inputs: &shared.inputs, declared: Set::new(), out };
    for item in &body {
        match item {
            Item::Rule(e) => {
                let en = &prog.arena[*e];
                em.decl(name_of(&en.head));
                em.rule(&en.head, &en.lits, en.edits.as_deref())?;
            }
            Item::Note(lines) => {
                if em.out.last().is_some_and(|l| !l.is_empty()) {
                    em.out.push(String::new());
                }
                em.out.extend(lines.iter().cloned());
            }
            Item::Viol(lines) => {
                let mut first = lines[0].clone();
                for c in &prog.numeric {
                    let l = c.to_lowercase();
                    first = sub(&re(&format!(r"\b{l}: loc\b")), &first, |_| format!("{l}: number"));
                }
                em.out.push(first);
                em.out.extend(lines[1..].iter().cloned());
            }
            Item::Line(l) => em.out.push(l.clone()),
        }
    }
    for (_, (decl, note, rules)) in &prog.lib {
        em.out.push(String::new());
        em.out.extend(note.iter().cloned());
        em.out.push(decl.clone());
        for (h, lits) in rules {
            em.out.push(wrap(h, &bind_read_nodes(px, lits.clone()), None));
        }
    }
    if !helper_rules.is_empty() {
        em.out.push(String::new());
        for r in &helper_rules {
            match r {
                HR::Facts(name, vals) => {
                    em.decl(name);
                    let mut line = String::new();
                    for v in vals {
                        let args = match v {
                            Value::List(l) => l.iter().map(q_val).collect::<Vec<_>>().join(", "),
                            _ => q_val(v),
                        };
                        let f = format!("{name}({args}).");
                        if !line.is_empty() && line.chars().count() + f.chars().count() + 1 > 88 {
                            em.out.push(std::mem::take(&mut line));
                        }
                        line = format!("{line} {f}").trim().to_owned();
                    }
                    if !line.is_empty() {
                        em.out.push(line);
                    }
                }
                HR::Rule(h, lits) => {
                    em.decl(name_of(h));
                    em.rule(h, lits, None)?;
                }
            }
        }
    }
    let mut out = em.out;
    if out.iter().any(|l| !l.trim_start().starts_with("//") && l.contains("ast(")) {
        let i = out.iter().position(String::is_empty).unwrap_or(out.len());
        out.insert(i, "// ast(n, {…}) atoms are JSON terms with no .rr form yet; the engine matches them as it does today.".to_owned());
    }
    Ok(format!("{}\n", out.join("\n").trim_end()))
}

struct Emit<'a> {
    px: &'a Patterns,
    prog: &'a Program,
    inputs: &'a BTreeMap<String, Set>,
    declared: Set,
    out: Vec<String>,
}

fn check(px: &Patterns, text: &str, inputs: &[String]) -> R<Vec<String>> {
    check_rule(px, text, &BTreeSet::new(), inputs).map_err(|e| e.0)
}

impl Emit<'_> {
    fn decl(&mut self, name: &str) {
        if self.prog.decls.contains_key(name) && !self.declared.contains(name) {
            if self.out.last().is_some_and(|l| !l.is_empty()) {
                self.out.push(String::new());
            }
            self.out.extend(self.prog.notes.get(name).cloned().unwrap_or_default());
            self.out.push(self.prog.decls[name].clone());
            self.declared.insert(name.to_owned());
        }
    }

    fn rule(&mut self, h: &str, lits: &[String], edits: Option<&[String]>) -> R<()> {
        let (prog, px) = (self.prog, self.px);
        let mut lits: Vec<String> = lits.to_vec();
        let hname = name_of(h);
        if hname.chars().next().is_some_and(char::is_lowercase)
            && let Some(defs) = prog.transform_defs.as_ref().filter(|d| !d.is_empty())
        {
            let add = transform_defs_for(px, defs, &lits);
            lits.extend(add);
        }
        if !prog.symbolic.is_empty() {
            let syms = prog.symbolic.iter().cloned().collect::<Vec<_>>().join("|");
            let pat = re(&format!(r"(?<![\w$@])(?:text|value)\(({syms}), (\w+)\)"));
            lits = lits.iter().map(|l| sub(&pat, l, |c| format!("{} = {}", g(c, 2), g(c, 1)))).collect();
            let body = lits.join(", ");
            let pairs: BTreeSet<(String, String)> = findall_groups(&re(&format!(r"(?<![\w$@])([a-z]\w*) = ({syms})\b")), &body).into_iter().map(|v| (v[0].clone(), v[1].clone())).collect();
            for (t, x) in pairs {
                if findall(&re(&format!(r"(?<![\w$@]){t} = ")), &body).len() != 1 {
                    continue;
                }
                let eq = format!(r"(?<![\w$@]){t} = {x}\b");
                let (r1, r2, r3) = (re(&format!(r",\s*{eq}")), re(&format!(r"{eq},\s*")), re(&eq));
                let tv = re(&format!(r"(?<![\w$@]){t}\b"));
                lits = lits
                    .iter()
                    .map(|l| {
                        let d = sub(&r3, &sub(&r2, &sub(&r1, l, |_| String::new()), |_| String::new()), |_| String::new());
                        sub(&tv, &d, |_| x.clone())
                    })
                    .filter(|l| !l.trim().is_empty())
                    .collect();
            }
        }
        let mut lits = bind_read_nodes(px, lits);
        let mut text = wrap(h, &lits, edits);
        let inputs: Vec<String> = self.inputs.get(hname).map(|s| s.iter().cloned().collect()).unwrap_or_default();
        let mut bad = check(px, &text, &inputs)?;
        if bad.iter().any(|x| x == "n") && h.split_once('(').is_some_and(|(_, t)| t.starts_with('n')) {
            lits.insert(0, "node(_, n, _, _, _)".to_owned());
            text = wrap(h, &lits, edits);
            bad = check(px, &text, &inputs)?;
        }
        let masked = mask_ticks(px, &strip_strings(px, &text));
        let once: Vec<String> = bad.iter().filter(|x| is_full(r"[a-z]\w*", x) && findall(&re(&format!(r#"(?<![\w"@$]){x}\b"#)), &masked).len() == 1).cloned().collect();
        if !once.is_empty() {
            let pat = re(&format!(r"(?<![\w@$])({})\b(?!\s*\()", once.join("|")));
            lits = lits.iter().map(|l| split1(&px.text_pieces, l).into_iter().map(|p| if p.starts_with('"') || p.starts_with('`') { p } else { sub(&pat, &p, |_| "_".into()) }).collect()).collect();
            text = wrap(h, &lits, edits);
            bad = check(px, &text, &inputs)?;
        }
        let empty: Vec<String> = bad.iter().filter(|x| is_full(r"[A-Z][A-Z0-9_]*", x)).cloned().collect();
        if !empty.is_empty() {
            let mut rest = Vec::new();
            for x in &empty {
                let mut done = false;
                for i in 0..lits.len() {
                    let t = lits[i].trim().to_owned();
                    if !(t.starts_with('(') && t.ends_with(')') && head_span_ok(px, &t)) {
                        continue;
                    }
                    let branches = split_top_text(&t[1..t.len() - 1], ';');
                    let call = re(&format!(r"(?<![!\w.@])[a-z_]\w*\([^()]*(?<![\w$]){x}\b[^()]*\)"));
                    let binding: Vec<bool> = branches
                        .iter()
                        .map(|b| binders(px, &sub(&px.translate_no_match_group, b, |_| String::new()), false).contains(x) || b.contains(&format!("unset({x})")) || search(&call, b))
                        .collect();
                    if branches.len() > 1 && binding.iter().any(|b| *b) {
                        lits[i] = format!(
                            "({})",
                            branches.iter().zip(&binding).map(|(b, ok)| if *ok { b.trim().to_owned() } else { format!("{}, unset({x})", b.trim()) }).collect::<Vec<_>>().join(" ; ")
                        );
                        done = true;
                        break;
                    }
                }
                if !done {
                    rest.push(x.clone());
                }
            }
            lits.extend(rest.iter().map(|x| format!("unset({x})")));
            text = wrap(h, &lits, edits);
            bad = check(px, &text, &inputs)?;
            let all = rest == empty;
            self.out.push(format!(
                "// {}: not bound by {}, so empty (unset) {}, as in ast-grep.",
                empty.join(", "),
                if all { "this rule" } else { "every branch" },
                if all { "here" } else { "where not bound" }
            ));
        }
        if !bad.is_empty() {
            self.out.push(format!("// Not grounded: no positive atom binds {}.", bad.join(", ")));
        }
        self.out.push(text);
        Ok(())
    }
}
