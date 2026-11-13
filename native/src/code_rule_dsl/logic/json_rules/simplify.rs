//! `simplify.py`: the meaning-preserving clean-up of a translated rule, run before metavariable sharing: merge
//! helpers with the same rules, inline helpers used once, drop repeated literals and grounding atoms something
//! else provides, and fold families of copies into one relation and a table (`fold`).

use super::program::Program;
use super::patterns::Patterns;
use super::pyre::{captures, esc, findall, g, re, sub};
use super::value::{Value, dumps};
use fancy_regex::Regex;
use indexmap::IndexMap;
use std::collections::BTreeSet;

pub type Set = BTreeSet<String>;

pub const MAX_DISJ: usize = 4;
pub const MAX_INLINE_LEN: usize = 400;

pub fn is_generated(px: &Patterns, s: &str) -> bool {
    super::pyre::search(&px.simplify_generated, s)
}

pub fn is_meaningless(px: &Patterns, s: &str) -> bool {
    super::pyre::search(&px.simplify_meaningless, s)
}

/// Alternate pieces: even indices are code, odd ones string or backtick literals.
pub fn split_protected(px: &Patterns, s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = 0;
    for c in captures(&px.simplify_prot, s) {
        let m = c.get(0).expect("group 0");
        out.push(s[at..m.start()].to_owned());
        out.push(m.as_str().to_owned());
        at = m.end();
    }
    out.push(s[at..].to_owned());
    out
}

fn mentions_metavariable(px: &Patterns, lits: &[String]) -> bool {
    for l in lits {
        for (i, p) in split_protected(px, l).iter().enumerate() {
            if i % 2 == 0 && super::pyre::search(&px.simplify_cap, p) {
                return true;
            }
            if i % 2 == 1 && p.starts_with('`') && super::pyre::search(&px.simplify_tick_mv, p) {
                return true;
            }
        }
    }
    false
}

pub fn code(px: &Patterns, s: &str) -> String {
    split_protected(px, s).into_iter().enumerate().filter(|(i, _)| i % 2 == 0).map(|(_, p)| p).collect()
}

fn rename(px: &Patterns, lit: &str, mapping: &IndexMap<String, String>) -> String {
    let mut parts = split_protected(px, lit);
    for i in (0..parts.len()).step_by(2) {
        parts[i] = sub(&px.simplify_var, &parts[i], |c| mapping.get(g(c, 1)).cloned().unwrap_or_else(|| g(c, 1).to_owned()));
    }
    parts.concat()
}

pub fn locals(px: &Patterns, lits: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in lits {
        for v in findall(&px.simplify_var, &code(px, l)) {
            if !out.contains(&v) {
                out.push(v);
            }
        }
    }
    out
}

/// Relation names a literal refers to: called, or passed as an argument.
pub fn calls(px: &Patterns, lit: &str) -> Set {
    let c = code(px, lit);
    let mut out: Set = findall(&px.simplify_call, &c).into_iter().collect();
    out.extend(findall(&px.simplify_bare_rel, &c));
    out
}

pub type Canon = Vec<(String, Vec<String>)>;

/// A definition with its local variables renamed in order of appearance: equal canon = same relation.
pub fn canon(px: &Patterns, heads_lits: &[(String, Vec<String>)]) -> Canon {
    let mut out: Canon = Vec::new();
    for (head_args, lits) in heads_lits {
        let mut names: IndexMap<String, String> = IndexMap::new();
        let mut all = vec![head_args.clone()];
        all.extend(lits.iter().cloned());
        for v in locals(px, &all) {
            let n = names.len();
            names.entry(v).or_insert_with(|| format!("_v{n}"));
        }
        out.push((rename(px, head_args, &names), lits.iter().map(|l| rename(px, l, &names)).collect()));
    }
    out.sort();
    out
}

/// All rules of one translated policy: violation rules (body entries), utils and helpers, as arena indices.
pub struct Rules {
    pub entries: Vec<usize>,
    pub helper_rules: Vec<usize>,
    pub facts: IndexMap<String, Vec<Value>>,
    pub protected: Set,
}

impl Rules {
    pub fn all(&self) -> Vec<usize> {
        self.entries.iter().chain(self.helper_rules.iter()).copied().collect()
    }

    pub fn defs(&self, prog: &Program) -> IndexMap<String, Vec<usize>> {
        let mut d: IndexMap<String, Vec<usize>> = IndexMap::new();
        for r in self.all() {
            d.entry(prog.arena[r].name().to_owned()).or_default().push(r);
        }
        d
    }

    /// Rename relations: a generated name wherever it appears, a short one only where it is called.
    pub fn rename_calls(&self, px: &Patterns, prog: &mut Program, mapping: &IndexMap<String, String>) {
        if mapping.is_empty() {
            return;
        }
        let bare: Vec<&String> = mapping.keys().filter(|k| is_generated(px, k)).collect();
        let alts = mapping.keys().map(|k| esc(k)).collect::<Vec<_>>().join("|");
        let mut p = format!(r"(?<![\w.@$])({alts})(?=\s*\()");
        if !bare.is_empty() {
            p += &format!(r"|(?<![\w.@$])({})\b", bare.iter().map(|k| esc(k)).collect::<Vec<_>>().join("|"));
        }
        let pat = re(&p);
        for r in self.all() {
            let new: Vec<String> = prog.arena[r]
                .lits
                .iter()
                .map(|l| {
                    let mut parts = split_protected(px, l);
                    for i in (0..parts.len()).step_by(2) {
                        parts[i] = sub(&pat, &parts[i], |c| {
                            let k = if super::pyre::has(c, 1) { g(c, 1) } else { g(c, 2) };
                            mapping[k].clone()
                        });
                    }
                    parts.concat()
                })
                .collect();
            prog.arena[r].lits = new;
        }
    }

    pub fn drop_named(&mut self, prog: &Program, drop: &Set) {
        self.helper_rules.retain(|&r| !drop.contains(prog.arena[r].name()));
        self.entries.retain(|&r| !drop.contains(prog.arena[r].name()));
    }
}

/// Identical definitions become one. Facts compare by their values.
fn merge(px: &Patterns, rs: &mut Rules, prog: &mut Program) -> usize {
    let mut changed = true;
    let mut merged = 0;
    while changed {
        changed = false;
        let mut mapping: IndexMap<String, String> = IndexMap::new();
        let mut seen: IndexMap<(String, String), String> = IndexMap::new();
        let mut names: Vec<&String> = rs.facts.keys().collect();
        names.sort();
        for name in names {
            let vals = &rs.facts[name];
            let decl = prog.decls.get(name).cloned().unwrap_or_default();
            let tail = decl.split_once('(').map_or(decl.clone(), |(_, t)| t.to_owned());
            let key = (dumps(&Value::List(vals.clone()), true, ", ", ": ", true), tail);
            if let Some(s) = seen.get(&key) {
                mapping.insert(name.clone(), s.clone());
            } else {
                seen.insert(key, name.clone());
            }
        }
        let defs = rs.defs(prog);
        let meaningless: Set = defs.keys().filter(|n| is_meaningless(px, n)).cloned().collect();
        let bare_used = bare_names(px, rs, prog, &meaningless);
        let mut seen: Vec<(Canon, String)> = Vec::new();
        let mut order: Vec<&String> = defs.keys().collect();
        order.sort_by(|a, b| {
            let ka = !(!rs.protected.contains(*a) && !is_generated(px, a));
            let kb = !(!rs.protected.contains(*b) && !is_generated(px, b));
            (ka, *a).cmp(&(kb, *b))
        });
        for name in order {
            if rs.protected.contains(name) || mapping.contains_key(name) {
                continue;
            }
            let rules = &defs[name];
            let head = format!("{name}(n)");
            if !rules.iter().all(|&r| prog.arena[r].head == head) {
                continue;
            }
            let key = canon(px, &rules.iter().map(|&r| ("n".to_owned(), prog.arena[r].lits.clone())).collect::<Vec<_>>());
            if let Some((_, s)) = seen.iter().find(|(k, _)| *k == key) {
                if is_generated(px, name) || (is_meaningless(px, name) && !bare_used.contains(name)) {
                    mapping.insert(name.clone(), s.clone());
                }
                continue;
            }
            seen.push((key, name.clone()));
        }
        if !mapping.is_empty() {
            changed = true;
            merged += mapping.len();
            rs.rename_calls(px, prog, &mapping);
            let drop: Set = mapping.keys().cloned().collect();
            rs.drop_named(prog, &drop);
            for n in &drop {
                rs.facts.shift_remove(n);
                prog.decls.shift_remove(n);
            }
        }
    }
    merged
}

/// Names among `names` that appear somewhere not as a call.
fn bare_names(px: &Patterns, rs: &Rules, prog: &Program, names: &Set) -> Set {
    let mut out = Set::new();
    for r in rs.all() {
        for l in &prog.arena[r].lits {
            out.extend(findall(&px.simplify_bare_name, &code(px, l)).into_iter().filter(|x| names.contains(x)));
        }
    }
    out
}

fn recursive(px: &Patterns, defs: &IndexMap<String, Vec<usize>>, prog: &Program) -> Set {
    let graph: IndexMap<&String, Set> = defs
        .iter()
        .map(|(n, rules)| {
            let mut s = Set::new();
            for &r in rules {
                for l in &prog.arena[r].lits {
                    s.extend(calls(px, l).into_iter().filter(|c| defs.contains_key(c)));
                }
            }
            (n, s)
        })
        .collect();
    let mut rec = Set::new();
    for (start, next) in &graph {
        let mut stack: Vec<String> = next.iter().cloned().collect();
        let mut seen = Set::new();
        while let Some(x) = stack.pop() {
            if x == **start {
                rec.insert((*start).clone());
                break;
            }
            if seen.insert(x.clone())
                && let Some(n) = graph.get(&x)
            {
                stack.extend(n.iter().cloned());
            }
        }
    }
    rec
}

/// Unfold helpers used once, in rounds.
fn inline(px: &Patterns, rs: &mut Rules, prog: &mut Program) -> usize {
    let mut inlined = 0;
    loop {
        let defs = rs.defs(prog);
        let rec = recursive(px, &defs, prog);
        let mut uses: IndexMap<String, Vec<(usize, String)>> = IndexMap::new();
        for r in rs.all() {
            for l in &prog.arena[r].lits {
                for c in calls(px, l) {
                    if defs.contains_key(&c) {
                        uses.entry(c).or_default().push((r, l.clone()));
                    }
                }
            }
        }
        let mut cands: IndexMap<String, (usize, String, Vec<usize>)> = IndexMap::new();
        for (name, rules) in &defs {
            if rs.protected.contains(name) || rec.contains(name) || !(is_generated(px, name) || is_meaningless(px, name)) {
                continue;
            }
            let head = format!("{name}(n)");
            if uses.get(name).map_or(0, Vec::len) != 1 || rules.len() > MAX_DISJ || !rules.iter().all(|&r| prog.arena[r].head == head) {
                continue;
            }
            if rules.iter().any(|&r| mentions_metavariable(px, &prog.arena[r].lits)) {
                continue;
            }
            let (r, lit) = &uses[name][0];
            if let Some(c) = super::pyre::fullmatch(&format!(r"{}\(([a-zA-Z_]\w*)\)", esc(name)), lit.trim()) {
                cands.insert(name.clone(), (*r, g(&c, 1).to_owned(), rules.clone()));
            }
        }
        let mut gone = Set::new();
        for (name, (r, arg, rules)) in &cands {
            if cands.contains_key(prog.arena[*r].name()) {
                continue;
            }
            let caller = prog.arena[*r].lits.clone();
            let lit = format!("{name}({arg})");
            let idx: Vec<usize> = caller.iter().enumerate().filter(|(_, l)| l.trim() == lit).map(|(i, _)| i).collect();
            if idx.len() != 1 {
                continue;
            }
            let i = idx[0];
            let mut all = vec![prog.arena[*r].head.clone()];
            all.extend(caller.iter().cloned());
            let mut taken: Set = locals(px, &all).into_iter().collect();
            taken.insert(arg.clone());
            let mut bodies: Vec<Vec<String>> = Vec::new();
            for &hr in rules {
                let body = prog.arena[hr].lits.clone();
                let mut mapping: IndexMap<String, String> = IndexMap::new();
                mapping.insert("n".into(), arg.clone());
                for v in locals(px, &body) {
                    if v == "n" {
                        continue;
                    }
                    let base = v.trim_end_matches(|c: char| c.is_ascii_digit());
                    let base = if base.is_empty() { "x" } else { base };
                    let mut k = 1;
                    while taken.contains(&format!("{base}{k}")) {
                        k += 1;
                    }
                    let nv = format!("{base}{k}");
                    taken.insert(nv.clone());
                    mapping.insert(v, nv);
                }
                bodies.push(body.iter().map(|l| rename(px, l, &mapping)).collect());
            }
            let new = if bodies.len() == 1 {
                bodies.remove(0)
            } else {
                let text = format!("({})", bodies.iter().map(|b| b.join(", ")).collect::<Vec<_>>().join(" ; "));
                if text.chars().count() > MAX_INLINE_LEN {
                    continue;
                }
                vec![text]
            };
            let mut lits = caller[..i].to_vec();
            lits.extend(new);
            lits.extend(caller[i + 1..].iter().cloned());
            prog.arena[*r].lits = lits;
            gone.insert(name.clone());
        }
        if gone.is_empty() {
            return inlined;
        }
        rs.drop_named(prog, &gone);
        for name in &gone {
            prog.decls.shift_remove(name);
        }
        inlined += gone.len();
    }
}


/// `node(_, x, _, _, _)` only grounds x; translate re-adds it where nothing else does.
fn drop_node_atoms(px: &Patterns, rs: &Rules, prog: &mut Program) {
    for r in rs.all() {
        let lits = prog.arena[r].lits.clone();
        let mut keep = Vec::new();
        for (j, l) in lits.iter().enumerate() {
            if let Some(c) = super::pyre::find(&px.simplify_node_atom, l.trim()) {
                let others = lits.iter().enumerate().filter(|(k, _)| *k != j).map(|(_, x)| code(px, x)).collect::<Vec<_>>().join(" ");
                if super::pyre::search(&re(&format!(r"(?<![\w@$]){}\b", esc(g(&c, 1)))), &others) {
                    continue;
                }
            }
            keep.push(l.clone());
        }
        if keep != lits {
            prog.arena[r].lits = if keep.is_empty() { lits } else { keep };
        }
    }
}

/// A conjunction does not change when a literal repeats: keep the first of each.
fn dedup_literals(rs: &Rules, prog: &mut Program) {
    for r in rs.all() {
        let lits = &prog.arena[r].lits;
        let mut seen = Set::new();
        let keep: Vec<String> = lits.iter().filter(|l| seen.insert(l.trim().to_owned())).cloned().collect();
        if keep.len() != lits.len() {
            prog.arena[r].lits = keep;
        }
    }
}

fn unused_facts(px: &Patterns, rs: &mut Rules, prog: &mut Program) {
    let mut called = Set::new();
    for r in rs.all() {
        for l in &prog.arena[r].lits {
            called.extend(calls(px, l));
        }
    }
    let names: Vec<String> = rs.facts.keys().cloned().collect();
    for n in names {
        if !called.contains(&n) {
            rs.facts.shift_remove(&n);
            prog.decls.shift_remove(&n);
        }
    }
}

/// Simplify in place. `entries` are the body's rules (arena indices, filtered in place); `prog.rules` holds helpers
/// and facts, rebuilt facts first.
pub fn simplify(px: &Patterns, entries: &mut Vec<usize>, prog: &mut Program, protected: Set, reserved: &Set) {
    let helper_rules: Vec<usize> = prog.rules.iter().filter_map(|r| if let super::program::PRule::Rule(id) = r { Some(*id) } else { None }).collect();
    let facts: IndexMap<String, Vec<Value>> = prog.rules.iter().filter_map(|r| if let super::program::PRule::Facts { name, vals } = r { Some((name.clone(), vals.clone())) } else { None }).collect();
    let mut rs = Rules { entries: std::mem::take(entries), helper_rules, facts, protected };
    let mut folded = false;
    for rnd in 0..10 {
        let a = merge(px, &mut rs, prog);
        let b = inline(px, &mut rs, prog);
        if a == 0 && b == 0 {
            if rnd < 9 && !folded {
                folded = true;
                if super::fold::fold(px, &mut rs, prog, None, reserved) > 0 {
                    continue;
                }
            }
            break;
        }
    }
    dedup_literals(&rs, prog);
    drop_node_atoms(px, &rs, prog);
    unused_facts(px, &mut rs, prog);
    let mut rules: Vec<super::program::PRule> = rs.facts.iter().map(|(n, v)| super::program::PRule::Facts { name: n.clone(), vals: v.clone() }).collect();
    rules.extend(rs.helper_rules.iter().map(|&id| super::program::PRule::Rule(id)));
    prog.rules = rules;
    *entries = rs.entries;
}
