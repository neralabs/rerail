//! M2/M7 executor: fused, hash-consed, memoized set-mode evaluation of data-layer rules over a regular SoA tape.
//!
//! Semantics follow ast-grep as used by the native matcher (sequential capture environment, first-match
//! relations), plus the Rerail keys these rules use: compare, transform(replace), parametric utilities,
//! layout{before: capture}, count{has}, value{regex|eq|type:str}, filePath, $option, when, file.path.
//!
//! Fusion features (spec-fusion §4.11): one hash-consed DAG for all programs; lazy per-file memo columns for
//! env-free shared nodes; bind-only memo for capture-producing nodes; interned regexes (one evaluation per
//! distinct leaf text, corpus-wide); masks (adaptive promotion) for env-free range-scanning relations;
//! root-kind dispatch shared by all programs.
//!
//! Usage: m2exec CORPUS_ROOT FILES_TXT OUT_JSON MODES REPS RULE.json [RULE.json ...]
//!   MODES: comma list of plain,memo,mask

use rayon::prelude::*;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

const NONE: u32 = u32::MAX;
const NOKIND: u16 = u16::MAX - 1;

// ---------------- hashing ----------------
#[derive(Default, Clone, Copy)]
struct Fx(u64);
impl std::hash::Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, b: &[u8]) {
        for &x in b {
            self.0 = (self.0.rotate_left(5) ^ x as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }
    fn write_u64(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}
type FxB = std::hash::BuildHasherDefault<Fx>;
type FxMap<K, V> = HashMap<K, V, FxB>;

fn thread_cpu() -> f64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
}

// ---------------- language ----------------
extern "C" {
    fn tree_sitter_python() -> *const ();
}
struct Lang {
    ts: tree_sitter::Language,
    ts2: Vec<u16>,
    by_name: HashMap<String, u16>,
    names: Vec<String>,
}
impl Lang {
    fn new() -> Lang {
        let ts: tree_sitter::Language = unsafe { tree_sitter::Language::from_raw(tree_sitter_python() as *const _) };
        let mut by_name = HashMap::new();
        let mut names = vec![];
        let mut ts2 = vec![];
        for id in 0..ts.node_kind_count() as u16 {
            let name = ts.node_kind_for_id(id).unwrap_or("?");
            let key = if ts.node_kind_is_named(id) { name.to_string() } else { format!("'{name}") };
            let n = names.len() as u16;
            let v = *by_name.entry(key.clone()).or_insert_with(|| {
                names.push(key);
                n
            });
            ts2.push(v);
        }
        if !by_name.contains_key("ERROR") {
            by_name.insert("ERROR".into(), names.len() as u16);
            names.push("ERROR".into());
        }
        Lang { ts, ts2, by_name, names }
    }
    fn map(&self, id: u16) -> u16 {
        self.ts2.get(id as usize).copied().unwrap_or_else(|| self.kind("ERROR"))
    }
    fn kind(&self, name: &str) -> u16 {
        self.by_name.get(name).copied().unwrap_or(self.names.len() as u16)
    }
    fn field(&self, name: &str) -> u16 {
        self.ts.field_id_for_name(name).map_or(u16::MAX, |f| f.get())
    }
}

// ---------------- regular SoA tape ----------------
struct Doc {
    path: String,
    src: String,
    kind: Vec<u16>,
    field: Vec<u16>,
    named: Vec<bool>,
    end: Vec<u32>,
    parent: Vec<u32>,
    prev: Vec<u32>,
    sb: Vec<u32>,
    eb: Vec<u32>,
    line: Vec<u32>,
    sym: Vec<u32>,
    has_error: bool,
}
impl Doc {
    #[inline]
    fn text(&self, n: u32) -> &str {
        &self.src[self.sb[n as usize] as usize..self.eb[n as usize] as usize]
    }
    fn len(&self) -> usize {
        self.kind.len()
    }
}

fn build_doc(lang: &Lang, path: String, src: String, tree: &tree_sitter::Tree) -> Doc {
    let mut d = Doc { path, src, kind: vec![], field: vec![], named: vec![], end: vec![], parent: vec![], prev: vec![], sb: vec![], eb: vec![], line: vec![], sym: vec![], has_error: tree.root_node().has_error() };
    let mut cursor = tree.walk();
    let mut stack: Vec<u32> = vec![];
    let mut last_child: Vec<u32> = vec![NONE];
    'outer: loop {
        let node = cursor.node();
        let id = d.kind.len() as u32;
        d.kind.push(lang.map(node.kind_id()));
        d.field.push(cursor.field_id().map_or(0, |f| f.get()));
        d.named.push(node.is_named());
        d.parent.push(stack.last().copied().unwrap_or(NONE));
        let lc = last_child.last_mut().unwrap();
        d.prev.push(*lc);
        *lc = id;
        d.end.push(0);
        d.sb.push(node.start_byte() as u32);
        d.eb.push(node.end_byte() as u32);
        d.line.push(node.start_position().row as u32);
        if cursor.goto_first_child() {
            stack.push(id);
            last_child.push(NONE);
            continue;
        }
        d.end[id as usize] = id + 1;
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                break 'outer;
            }
            let p = stack.pop().unwrap();
            last_child.pop();
            d.end[p as usize] = d.kind.len() as u32;
        }
    }
    d.sym = vec![NONE; d.len()];
    d
}

// ---------------- regex ----------------
enum Re {
    R(regex::Regex),
    F(fancy_regex::Regex),
    Never,
}
impl Re {
    fn new(p: &str) -> Re {
        match regex::Regex::new(p) {
            Ok(r) => Re::R(r),
            Err(_) => fancy_regex::Regex::new(p).map(Re::F).unwrap_or_else(|e| {
                eprintln!("bad regex {p}: {e}");
                Re::Never
            }),
        }
    }
    fn is_match(&self, s: &str) -> bool {
        match self {
            Re::R(r) => r.is_match(s),
            Re::F(r) => r.is_match(s).unwrap_or(false),
            Re::Never => false,
        }
    }
    fn replace_all(&self, s: &str, by: &str) -> String {
        match self {
            Re::R(r) => r.replace_all(s, by).into_owned(),
            Re::F(r) => r.replace_all(s, by).into_owned(),
            Re::Never => s.to_string(),
        }
    }
}

// ---------------- rule DAG ----------------
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
enum Op {
    True,
    False,
    Kind(u16),
    Named,
    Regex(u32),
    ValueRe(u32),
    ValueEq(String),
    ValueStr,
    FilePath { re: u32, neg: bool },
    Nth { pos: u32, rev: bool, of: u32 },
    All(Vec<u32>),
    Any(Vec<u32>),
    Not(u32),
    Rel { axis: u8, stop: u8, stopr: u32, field: u16, inner: u32 },
    Util(u32),
    Bind(u16),
    Compare { l: u16, r: u16, ne: bool },
    Layout(u16),
    Count { inner: u32, min: u32, max: u32 },
    Call { slot: u32, params: Vec<(u16, u16)> },
}
const HAS: u8 = 0;
const INSIDE: u8 = 1;
const PRECEDES: u8 = 2;
const FOLLOWS: u8 = 3;

#[derive(Default)]
struct Dag {
    ops: Vec<Op>,
    map: HashMap<Op, u32>,
    refs: Vec<u32>,
    util_body: Vec<u32>,
    util_slot: HashMap<String, u32>,
    putil_slot: HashMap<String, u32>,
    putil_body: Vec<u32>,
    regex: Vec<Re>,
    regex_src: Vec<String>,
    regex_map: HashMap<String, u32>,
    vars: Vec<String>,
    var_map: HashMap<String, u16>,
    expansions: u64,
    unknown_keys: HashMap<String, u64>,
}

impl Dag {
    fn intern(&mut self, op: Op) -> u32 {
        self.expansions += 1;
        if let Some(&id) = self.map.get(&op) {
            return id;
        }
        let id = self.ops.len() as u32;
        for k in op_kids(&op) {
            self.refs[k as usize] += 1;
        }
        self.ops.push(op.clone());
        self.refs.push(0);
        self.map.insert(op, id);
        id
    }
    fn regex(&mut self, p: &str) -> u32 {
        if let Some(&r) = self.regex_map.get(p) {
            return r;
        }
        let r = self.regex.len() as u32;
        self.regex.push(Re::new(p));
        self.regex_src.push(p.to_string());
        self.regex_map.insert(p.to_string(), r);
        r
    }
    fn var(&mut self, name: &str) -> u16 {
        let name = name.trim_start_matches('$');
        if let Some(&v) = self.var_map.get(name) {
            return v;
        }
        let v = self.vars.len() as u16;
        self.vars.push(name.to_string());
        self.var_map.insert(name.to_string(), v);
        v
    }

    /// Compile one rule object. `env`: utilities in scope (name -> body). `stack`: utilities being expanded.
    fn compile(&mut self, lang: &Lang, r: &Value, env: &serde_json::Map<String, Value>, stack: &mut Vec<(String, u32)>) -> u32 {
        let o = match r.as_object() {
            Some(o) => o,
            None => return self.intern(Op::True),
        };
        let mut conj: Vec<u32> = vec![];
        for (k, v) in o {
            let c = match k.as_str() {
                "kind" => Some(Op::Kind(lang.kind(v.as_str().unwrap_or("")))),
                "pattern" => {
                    let p = v.as_str().expect("pattern must be a string here");
                    let t = p.trim();
                    assert!(t.starts_with('$') && t[1..].chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'), "structural pattern unsupported: {p}");
                    if t == "$_" {
                        Some(Op::Named)
                    } else {
                        Some(Op::Bind(self.var(t)))
                    }
                }
                "regex" => Some(Op::Regex(self.regex(v.as_str().expect("regex str")))),
                "value" => {
                    let vo = v.as_object().expect("value obj");
                    let mut parts = vec![];
                    for (vk, vv) in vo {
                        let op = match vk.as_str() {
                            "regex" => Op::ValueRe(self.regex(vv.as_str().unwrap())),
                            "eq" => Op::ValueEq(vv.as_str().unwrap().to_string()),
                            "type" => {
                                assert_eq!(vv.as_str(), Some("str"));
                                Op::ValueStr
                            }
                            other => panic!("value key {other}"),
                        };
                        parts.push(self.intern(op));
                    }
                    conj.extend(parts);
                    None
                }
                "filePath" => {
                    let fo = v.as_object().unwrap();
                    for (fk, fv) in fo {
                        let re = self.regex(fv.as_str().unwrap());
                        let op = Op::FilePath { re, neg: fk == "notRegex" };
                        let id = self.intern(op);
                        conj.push(id);
                    }
                    None
                }
                "nthChild" => match v {
                    Value::Number(n) => Some(Op::Nth { pos: n.as_u64().unwrap() as u32, rev: false, of: NONE }),
                    Value::Object(no) => {
                        let pos = no["position"].as_u64().unwrap() as u32;
                        let of = no.get("ofRule").map_or(NONE, |r| self.compile(lang, r, env, stack));
                        Some(Op::Nth { pos, rev: no.get("reverse").and_then(|b| b.as_bool()).unwrap_or(false), of })
                    }
                    _ => panic!("nthChild"),
                },
                "all" | "any" => {
                    let parts: Vec<u32> = v.as_array().unwrap().iter().map(|x| self.compile(lang, x, env, stack)).collect();
                    let id = if k == "all" { self.mk_all(parts) } else { self.mk_any(parts) };
                    conj.push(id);
                    None
                }
                "not" => {
                    let x = self.compile(lang, v, env, stack);
                    Some(Op::Not(x))
                }
                "has" | "inside" | "precedes" | "follows" => Some(self.relation(lang, k, v, env, stack)),
                "count" => {
                    let co = v.as_object().unwrap();
                    let rel = ["has", "inside", "precedes", "follows"].iter().find(|r| co.contains_key(**r)).expect("count relation");
                    assert_eq!(*rel, "has", "only count.has supported");
                    let body = co[*rel].as_object().unwrap();
                    let stop = body.get("stopBy").and_then(|s| s.as_str()).unwrap_or("neighbor");
                    assert_eq!(stop, "neighbor");
                    assert!(!body.contains_key("field"));
                    let mut b = body.clone();
                    b.remove("stopBy");
                    let inner = self.compile(lang, &Value::Object(b), env, stack);
                    let min = co.get("min").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                    let max = co.get("max").and_then(|x| x.as_u64()).map_or(u32::MAX, |x| x as u32);
                    Some(Op::Count { inner, min, max })
                }
                "compare" => {
                    let l = self.var(v["left"].as_str().unwrap());
                    let r = self.var(v["right"].as_str().unwrap());
                    let ne = match v["op"].as_str().unwrap() {
                        "eq" => false,
                        "ne" => true,
                        o => panic!("compare op {o}"),
                    };
                    assert!(v.get("collation").is_none() && v.get("text").is_none(), "compare options: {v}");
                    Some(Op::Compare { l, r, ne })
                }
                "layout" => {
                    let lo = v.as_object().unwrap();
                    assert!(lo.len() == 1 && lo.contains_key("before"), "layout: {v}");
                    let b = &lo["before"];
                    let to = b["to"].as_str().unwrap();
                    let var = self.var(to.strip_prefix("capture.").expect("capture"));
                    let br = &b["breaks"];
                    assert!(br.get("min").and_then(|x| x.as_u64()) == Some(0) && br.get("max").is_none(), "layout breaks {b}");
                    Some(Op::Layout(var))
                }
                "matches" => {
                    match v {
                        Value::String(name) => {
                            if let Some(&(_, slot)) = stack.iter().find(|(n, _)| n == name) {
                                Some(Op::Util(slot))
                            } else {
                                let body = env.get(name).unwrap_or_else(|| panic!("unknown util {name}")).clone();
                                assert!(body.get("params").is_none(), "parametric util used without args: {name}");
                                let key = format!("{name}\u{0}{body}");
                                let slot = *self.util_slot.entry(key).or_insert_with(|| {
                                    self.util_body.push(NONE);
                                    self.util_body.len() as u32 - 1
                                });
                                stack.push((name.clone(), slot));
                                let id = self.compile(lang, &body, env, stack);
                                stack.pop();
                                self.util_body[slot as usize] = id;
                                conj.push(id);
                                None
                            }
                        }
                        Value::Object(call) => {
                            assert_eq!(call.len(), 1);
                            let (name, args) = call.iter().next().unwrap();
                            let def = env.get(name).unwrap_or_else(|| panic!("unknown putil {name}")).clone();
                            let params: Vec<String> = def["params"].as_array().unwrap().iter().map(|p| p.as_str().unwrap().to_string()).collect();
                            let mut pm = vec![];
                            for p in &params {
                                let a = args[p].as_str().unwrap();
                                pm.push((self.var(p), self.var(a)));
                            }
                            let key = format!("{name}\u{0}{}", def);
                            let slot = match self.putil_slot.get(&key) {
                                Some(&s) => s,
                                None => {
                                    let s = self.putil_body.len() as u32;
                                    self.putil_body.push(NONE);
                                    self.putil_slot.insert(key, s);
                                    // compile the body (fresh util stack: its own recursion goes through Call)
                                    let mut st = vec![];
                                    let id = self.compile(lang, &def["rule"], env, &mut st);
                                    self.putil_body[s as usize] = id;
                                    s
                                }
                            };
                            Some(Op::Call { slot, params: pm })
                        }
                        _ => panic!("matches"),
                    }
                }
                "stopBy" | "field" => None,
                other => {
                    *self.unknown_keys.entry(other.to_string()).or_default() += 1;
                    panic!("unsupported rule key {other}");
                }
            };
            if let Some(op) = c {
                let id = self.intern(op);
                conj.push(id);
            }
        }
        // ast-grep evaluates `all` in order; conjuncts of one object are ordered as written.
        self.mk_all(conj)
    }

    /// Conjunction, flattened (order-preserving), constants folded.
    fn mk_all(&mut self, parts: Vec<u32>) -> u32 {
        let t = self.intern(Op::True);
        let f = self.intern(Op::False);
        let mut out = vec![];
        for x in parts {
            match &self.ops[x as usize] {
                Op::True => {}
                Op::All(v) if FLATTEN.with(|c| c.get()) => out.extend(v.clone()),
                _ => out.push(x),
            }
        }
        if out.contains(&f) {
            return f;
        }
        match out.len() {
            0 => t,
            1 => out[0],
            _ => self.intern(Op::All(out)),
        }
    }

    /// Disjunction; consecutive branches with a common conjunct prefix are factored (order-preserving:
    /// first-match capture semantics are unchanged because a shared prefix binds identically).
    fn mk_any(&mut self, parts: Vec<u32>) -> u32 {
        let t = self.intern(Op::True);
        let f = self.intern(Op::False);
        let parts: Vec<u32> = parts.into_iter().filter(|&x| x != f).collect();
        if parts.contains(&t) && parts.iter().all(|&x| x == t || self.is_pure_test(x)) {
            return t;
        }
        let mut out: Vec<u32> = vec![];
        if FACTOR.with(|c| c.get()) {
            let list = |g: &Dag, x: u32| -> Vec<u32> {
                match &g.ops[x as usize] {
                    Op::All(v) => v.clone(),
                    _ => vec![x],
                }
            };
            let mut i = 0;
            while i < parts.len() {
                let li = list(self, parts[i]);
                // longest run j>i sharing li[0]
                let mut j = i + 1;
                while j < parts.len() && list(self, parts[j]).first() == li.first() {
                    j += 1;
                }
                if j - i >= 2 {
                    let lists: Vec<Vec<u32>> = (i..j).map(|k| list(self, parts[k])).collect();
                    let mut pre = 0;
                    while lists.iter().all(|l| l.len() > pre && l[pre] == lists[0][pre]) {
                        pre += 1;
                    }
                    let prefix = lists[0][..pre].to_vec();
                    let suffixes: Vec<u32> = lists.iter().map(|l| self.mk_all(l[pre..].to_vec())).collect();
                    let tail = self.mk_any(suffixes);
                    let mut all = prefix;
                    all.push(tail);
                    out.push(self.mk_all(all));
                } else {
                    out.push(parts[i]);
                }
                i = j;
            }
        } else {
            out = parts;
        }
        match out.len() {
            0 => f,
            1 => out[0],
            _ => self.intern(Op::Any(out)),
        }
    }

    fn is_pure_test(&self, _x: u32) -> bool {
        false
    }

    fn relation(&mut self, lang: &Lang, k: &str, v: &Value, env: &serde_json::Map<String, Value>, stack: &mut Vec<(String, u32)>) -> Op {
        let axis = match k {
            "has" => HAS,
            "inside" => INSIDE,
            "precedes" => PRECEDES,
            _ => FOLLOWS,
        };
        let mut body = v.as_object().cloned().unwrap_or_default();
        let stop_v = body.remove("stopBy");
        let field = body.remove("field").and_then(|f| f.as_str().map(|s| lang.field(s))).unwrap_or(0);
        let (stop, stopr) = match &stop_v {
            None => (0, NONE),
            Some(Value::String(s)) if s == "neighbor" => (0, NONE),
            Some(Value::String(s)) if s == "end" => (1, NONE),
            Some(sr) => (2, self.compile(lang, sr, env, stack)),
        };
        let inner = self.compile(lang, &Value::Object(body), env, stack);
        Op::Rel { axis, stop, stopr, field, inner }
    }

    fn root_kinds(&self, id: u32, memo: &mut HashMap<u32, Option<Vec<u16>>>) -> Option<Vec<u16>> {
        if let Some(v) = memo.get(&id) {
            return v.clone();
        }
        memo.insert(id, None); // cycle guard
        let r = match &self.ops[id as usize] {
            Op::Kind(k) => Some(vec![*k]),
            Op::False => Some(vec![]),
            Op::ValueStr => None,
            Op::All(v) => {
                let mut out: Option<Vec<u16>> = None;
                for &x in v {
                    if let Some(s) = self.root_kinds(x, memo) {
                        out = Some(match out {
                            None => s,
                            Some(o) => o.into_iter().filter(|k| s.contains(k)).collect(),
                        });
                    }
                }
                out
            }
            Op::Any(v) => {
                let mut out = vec![];
                for &x in v {
                    match self.root_kinds(x, memo) {
                        None => {
                            memo.insert(id, None);
                            return None;
                        }
                        Some(s) => {
                            for k in s {
                                if !out.contains(&k) {
                                    out.push(k)
                                }
                            }
                        }
                    }
                }
                Some(out)
            }
            Op::Util(s) => self.root_kinds(self.util_body[*s as usize], memo),
            _ => None,
        };
        memo.insert(id, r.clone());
        r
    }
}

fn op_kids(op: &Op) -> Vec<u32> {
    match op {
        Op::All(v) | Op::Any(v) => v.clone(),
        Op::Not(x) => vec![*x],
        Op::Nth { of, .. } if *of != NONE => vec![*of],
        Op::Rel { stopr, inner, .. } => {
            if *stopr != NONE {
                vec![*stopr, *inner]
            } else {
                vec![*inner]
            }
        }
        Op::Count { inner, .. } => vec![*inner],
        _ => vec![],
    }
}

// ---------------- options ----------------
fn resolve_options(x: &Value, opts: &HashMap<String, Value>) -> Value {
    match x {
        Value::Object(o) => {
            if let Some(name) = o.get("$option").and_then(|n| n.as_str()) {
                let val = opts.get(name).cloned().unwrap_or(Value::Null);
                if let Some(cases) = o.get("cases").and_then(|c| c.as_object()) {
                    let key = match &val {
                        Value::Bool(b) => b.to_string(),
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    let c = cases.get(&key).unwrap_or_else(|| panic!("option {name} case {key}"));
                    return resolve_options(c, opts);
                }
                if let Some(t) = o.get("template").and_then(|t| t.as_str()) {
                    let items: Vec<String> = val.as_array().unwrap_or_else(|| panic!("option {name} array")).iter().map(|s| s.as_str().unwrap().to_string()).collect();
                    let esc: Vec<String> = match o.get("escape").and_then(|e| e.as_str()) {
                        Some("regex") => items.iter().map(|s| regex::escape(s)).collect(),
                        None => items,
                        Some(e) => panic!("escape {e}"),
                    };
                    let joined = esc.join(o.get("join").and_then(|j| j.as_str()).unwrap_or(""));
                    return Value::String(t.replace("{}", &joined));
                }
                panic!("unsupported $option form {x}");
            }
            Value::Object(o.iter().map(|(k, v)| (k.clone(), resolve_options(v, opts))).collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(|v| resolve_options(v, opts)).collect()),
        _ => x.clone(),
    }
}

fn when_active(w: Option<&Value>, opts: &HashMap<String, Value>) -> bool {
    let Some(w) = w else { return true };
    for (name, want) in w.get("option").and_then(|o| o.as_object()).into_iter().flatten() {
        if opts.get(name) != Some(want) {
            return false;
        }
    }
    true
}

fn glob(gs: &Value) -> globset::GlobSet {
    let mut b = globset::GlobSetBuilder::new();
    for g in gs.as_array().into_iter().flatten() {
        if let Some(s) = g.as_str() {
            b.add(globset::GlobBuilder::new(s).literal_separator(true).build().unwrap());
        }
    }
    b.build().unwrap()
}

// ---------------- programs ----------------
struct Transform {
    src: u16,
    re: u32,
    by: String,
}
struct Program {
    idx: u32,
    rule: usize,
    clause: String,
    code: String,
    root: u32,
    kinds: Option<Vec<bool>>,
    path_re: Option<Re>,
    path_not: Option<Re>,
    syntax_valid: bool,
    transforms: FxMap<u16, Transform>,
}
struct Rule {
    id: String,
    inc: globset::GlobSet,
    exc: globset::GlobSet,
}

// ---------------- evaluator ----------------
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Plain,
    Memo,
    Mask,
}

struct Cfg {
    memo_free: Vec<bool>,  // env-free ids that get a memo column
    env_free: Vec<bool>,   // ids whose result does not depend on / change the capture env
    bind_memo: Vec<bool>,  // capture-producing ids without reads: memo (bool, bindings)
    binds: Vec<Vec<u16>>,  // vars bound by an id (for bind_memo ids)
    range_rel: Vec<bool>,  // env-free range-scanning relations (mask candidates)
    rk: Vec<Option<Vec<bool>>>,
    leaf_regex: bool,
    // keyed set stage for env-ful has(stopBy: end) relations: (name field, key vars) — see Ev::keyed_scan
    keyed: Vec<Option<(u16, Vec<u16>)>>,
}


const NC: usize = 12;
const CN: [&str; NC] = ["evals", "memo_hits", "memo_cells", "regex_calls", "regex_bytes", "regex_sym_hits", "scan_visits", "mask_builds", "mask_cells", "bind_memo_hits", "dispatch", "util_depth_cut"];

struct Ev<'a> {
    g: &'a Dag,
    cfg: &'a Cfg,
    mode: Mode,
    d: &'a Doc,
    memo: Vec<Vec<u16>>,
    epoch: u16,
    maxlen: usize,
    touched: Vec<u32>,
    mask: Vec<Vec<u8>>,
    mask_touched: Vec<u32>,
    scan_spent: Vec<u32>,
    bmemo: FxMap<u64, (bool, u32, u32)>,
    tcache: FxMap<u64, u32>,
    tstr: Vec<String>,
    op_evals: Vec<u64>,
    calib: Vec<(u64, u64, u64)>,
    cmemo: FxMap<u64, (bool, u32, u32)>,
    kindex: FxMap<u64, HashMap<String, Vec<u32>, FxB>>,
    carena: Vec<(u16, u32)>,
    call_memo: bool,
    prof: bool,
    barena: Vec<(u16, u32)>,
    env: Vec<(u16, u32)>,
    base: usize,
    prog: Option<&'a Program>,
    depth: u32,
    ctr: [u64; NC],
    // interned regex results over leaf symbols: [regex][sym] -> 0 unknown, 1 false, 2 true (corpus-wide)
    sym_re: Vec<Vec<u8>>,
}

impl<'a> Ev<'a> {
    fn new(g: &'a Dag, cfg: &'a Cfg, mode: Mode, d: &'a Doc) -> Ev<'a> {
        let n = g.ops.len();
        Ev { g, cfg, mode, d, memo: vec![vec![]; n], epoch: 0, maxlen: 0, touched: vec![], mask: vec![vec![]; n], mask_touched: vec![], scan_spent: vec![0; n], bmemo: FxMap::default(), tcache: FxMap::default(), tstr: vec![], op_evals: vec![0; n], calib: vec![], cmemo: FxMap::default(), kindex: FxMap::default(), carena: vec![], call_memo: std::env::var("CALL_MEMO").map_or(true, |v| v != "0"), prof: std::env::var("PROF").is_ok(), barena: vec![], env: Vec::with_capacity(64), base: 0, prog: None, depth: 0, ctr: [0; NC], sym_re: (0..g.regex.len()).map(|_| vec![]).collect() }
    }
    fn reset(&mut self, d: &'a Doc) {
        // epoch-stamped memo columns: no per-file clearing (cell = epoch*4 + 1 + result)
        self.epoch += 1;
        if self.epoch >= 16000 {
            for &id in &self.touched {
                self.memo[id as usize].iter_mut().for_each(|c| *c = 0);
            }
            self.epoch = 1;
        }
        for &id in &self.mask_touched {
            self.mask[id as usize].clear();
            self.scan_spent[id as usize] = 0;
        }
        self.mask_touched.clear();
        self.bmemo.clear();
        self.barena.clear();
        self.tcache.clear();
        self.tstr.clear();
        self.cmemo.clear();
        self.carena.clear();
        self.kindex.clear();
        self.d = d;
    }
    #[inline]
    fn kind_ok(&self, id: u32, n: u32) -> bool {
        match &self.cfg.rk[id as usize] {
            None => true,
            Some(s) => s[self.d.kind[n as usize] as usize],
        }
    }
    #[inline]
    fn lookup(&self, v: u16) -> Option<u32> {
        self.env[self.base..].iter().rev().find(|(x, _)| *x == v).map(|e| e.1)
    }
    /// Text of a capture or (top frame) a transform of one: (is_cached, index) — node id or cache slot.
    fn var_ref(&mut self, v: u16, depth: u32) -> Option<(bool, u32)> {
        if let Some(n) = self.lookup(v) {
            return Some((false, n));
        }
        if self.base == 0 && depth < 8 {
            let t = self.prog.and_then(|p| p.transforms.get(&v))?;
            let (src, re) = (t.src, t.re);
            let by: *const String = &t.by;
            let sref = self.var_ref(src, depth + 1)?;
            // transforms are per program: the cache key carries the program (by address-stable index)
            let pid = self.prog.map_or(0, |p| p.idx) as u64;
            let key = (pid << 48) | ((v as u64) << 33) | ((sref.0 as u64) << 32) | sref.1 as u64;
            if let Some(&slot) = self.tcache.get(&key) {
                return Some((true, slot));
            }
            let out = {
                let s = self.ref_str(sref);
                self.g.regex[re as usize].replace_all(s, unsafe { &*by })
            };
            let slot = self.tstr.len() as u32;
            self.tstr.push(out);
            self.tcache.insert(key, slot);
            return Some((true, slot));
        }
        None
    }
    #[inline]
    fn ref_str(&self, r: (bool, u32)) -> &str {
        if r.0 {
            &self.tstr[r.1 as usize]
        } else {
            self.d.text(r.1)
        }
    }

    /// Evaluate with env restore on failure.
    #[inline]
    fn eval(&mut self, id: u32, n: u32) -> bool {
        if self.calib.is_empty() {
            return self.eval_in(id, n);
        }
        let c0 = self.ctr[0];
        let r = self.eval_in(id, n);
        let st = &mut self.calib[id as usize];
        st.0 += 1;
        st.1 += r as u64;
        st.2 += self.ctr[0] - c0 + 1;
        r
    }

    #[inline]
    fn eval_in(&mut self, id: u32, n: u32) -> bool {
        let id_u = id as usize;
        // leaf fast path: no call, no memo, no env
        match &self.g.ops[id_u] {
            Op::Kind(k) => {
                self.ctr[0] += 1;
                return self.d.kind[n as usize] == *k;
            }
            Op::Named => {
                self.ctr[0] += 1;
                return self.d.named[n as usize];
            }
            Op::True => return true,
            Op::False => return false,
            _ => {}
        }
        if self.mode != Mode::Plain {
            if self.cfg.memo_free[id_u] {
                let len = self.d.len();
                if self.memo[id_u].len() < len {
                    let m = self.maxlen.max(len);
                    self.memo[id_u].resize(m, 0);
                    self.ctr[2] += m as u64;
                    self.touched.push(id);
                }
                let v = self.memo[id_u][n as usize];
                let e4 = self.epoch * 4;
                if v >= e4 && v < e4 + 4 {
                    self.ctr[1] += 1;
                    return v == e4 + 2;
                }
                let r = self.eval_raw(id, n);
                self.memo[id_u][n as usize] = e4 + 1 + r as u16;
                return r;
            }
            if self.cfg.bind_memo[id_u] && self.cfg.binds[id_u].iter().all(|&v| self.lookup(v).is_none()) {
                let key = ((id as u64) << 32) | n as u64;
                if let Some(&(ok, s, e)) = self.bmemo.get(&key) {
                    self.ctr[9] += 1;
                    if ok {
                        for i in s..e {
                            let b = self.barena[i as usize];
                            self.env.push(b);
                        }
                    }
                    return ok;
                }
                let mark = self.env.len();
                let r = self.eval_raw(id, n);
                if r {
                    let s = self.barena.len() as u32;
                    for i in mark..self.env.len() {
                        let b = self.env[i];
                        self.barena.push(b);
                    }
                    self.bmemo.insert(key, (true, s, self.barena.len() as u32));
                } else {
                    self.env.truncate(mark);
                    self.bmemo.insert(key, (false, 0, 0));
                }
                return r;
            }
        }
        let mark = self.env.len();
        let r = self.eval_raw(id, n);
        if !r {
            self.env.truncate(mark);
        }
        r
    }

    #[inline]
    fn fok(&self, f: u16, n: u32) -> bool {
        f == 0 || self.d.field[n as usize] == f
    }

    fn regex_node(&mut self, r: u32, n: u32) -> bool {
        let d = self.d;
        let s = d.sym[n as usize];
        if self.cfg.leaf_regex && s != NONE {
            // interned: one evaluation per distinct leaf text, corpus-wide
            if (s as usize) < self.sym_re[r as usize].len() && self.sym_re[r as usize][s as usize] != 0 {
                let col = &self.sym_re[r as usize];
                self.ctr[5] += 1;
                return col[s as usize] == 2;
            }
            let t = d.text(n);
            self.ctr[3] += 1;
            self.ctr[4] += t.len() as u64;
            let v = self.g.regex[r as usize].is_match(t);
            let col = &mut self.sym_re[r as usize];
            if col.len() <= s as usize {
                col.resize(s as usize + 1, 0);
            }
            col[s as usize] = 1 + v as u8;
            return v;
        }
        let t = d.text(n);
        self.ctr[3] += 1;
        self.ctr[4] += t.len() as u64;
        self.g.regex[r as usize].is_match(t)
    }

    fn eval_raw(&mut self, id: u32, n: u32) -> bool {
        self.ctr[0] += 1;
        if self.prof {
            self.op_evals[id as usize] += 1;
        }
        let g = self.g;
        let d = self.d;
        match &g.ops[id as usize] {
            Op::True => true,
            Op::False => false,
            Op::Kind(k) => d.kind[n as usize] == *k,
            Op::Named => d.named[n as usize],
            Op::Regex(r) => self.regex_node(*r, n),
            Op::ValueRe(r) => match string_value(d, n) {
                Some(v) => {
                    self.ctr[3] += 1;
                    g.regex[*r as usize].is_match(&v)
                }
                None => false,
            },
            Op::ValueEq(s) => string_value(d, n).as_deref() == Some(s.as_str()),
            Op::ValueStr => string_value(d, n).is_some(),
            Op::FilePath { re, neg } => g.regex[*re as usize].is_match(&d.path) != *neg,
            Op::Nth { pos, rev, of } => {
                if !d.named[n as usize] {
                    return false;
                }
                let p = d.parent[n as usize];
                if *of != NONE && !self.eval(*of, n) {
                    return false;
                }
                if p == NONE {
                    return *pos == 1;
                }
                let mut idx = 1;
                let mut j = p + 1;
                let pe = d.end[p as usize];
                while j < pe {
                    if j != n && ((!*rev && j < n) || (*rev && j > n)) && d.named[j as usize] {
                        let ok = if *of == NONE {
                            true
                        } else {
                            let m = self.env.len();
                            let r = self.eval(*of, j);
                            self.env.truncate(m);
                            r
                        };
                        if ok {
                            idx += 1;
                        }
                    }
                    j = d.end[j as usize];
                }
                idx == *pos
            }
            Op::All(v) => {
                for &x in v {
                    if !self.eval(x, n) {
                        return false;
                    }
                }
                true
            }
            Op::Any(v) => {
                for &x in v {
                    if self.eval(x, n) {
                        return true;
                    }
                }
                false
            }
            Op::Not(x) => {
                let m = self.env.len();
                let r = self.eval(*x, n);
                self.env.truncate(m);
                !r
            }
            Op::Util(s) => {
                if self.depth > 200 {
                    self.ctr[11] += 1;
                    return false;
                }
                self.depth += 1;
                let r = self.eval(g.util_body[*s as usize], n);
                self.depth -= 1;
                r
            }
            Op::Bind(v) => {
                if !d.named[n as usize] {
                    return false;
                }
                match self.lookup(*v) {
                    Some(m) => m == n || d.text(m) == d.text(n),
                    None => {
                        self.env.push((*v, n));
                        true
                    }
                }
            }
            Op::Compare { l, r, ne } => match (self.var_ref(*l, 0), self.var_ref(*r, 0)) {
                (Some(a), Some(b)) => (self.ref_str(a) == self.ref_str(b)) != *ne,
                _ => false,
            },
            Op::Layout(v) => match self.lookup(*v) {
                Some(m) => d.eb[m as usize] <= d.sb[n as usize],
                None => false,
            },
            Op::Count { inner, min, max } => {
                let mut c = 0u32;
                let mut j = n + 1;
                let e = d.end[n as usize];
                while j < e {
                    let m = self.env.len();
                    if self.kind_ok(*inner, j) && self.eval(*inner, j) {
                        c += 1;
                        if c > *max {
                            self.env.truncate(m);
                            return false;
                        }
                    }
                    self.env.truncate(m);
                    j = d.end[j as usize];
                }
                c >= *min && c <= *max
            }
            Op::Call { slot, params } => {
                if self.depth > 64 {
                    self.ctr[11] += 1;
                    return false;
                }
                let old_base = self.base;
                let mark = self.env.len();
                let mut pre = [(0u16, 0u32); 8];
                let mut np = 0;
                for &(p, a) in params.iter() {
                    if let Some(n) = self.lookup(a) {
                        pre[np] = (p, n);
                        np += 1;
                    }
                }
                // call memo: a parametric utility runs in a fresh frame, so its result and exports depend only
                // on (utility, node, pre-bound parameters)
                let ckey = if self.mode != Mode::Plain && self.call_memo {
                    let mut h = (id as u64).wrapping_mul(0x9E3779B97F4A7C15) ^ (n as u64);
                    for &(p, m) in &pre[..np] {
                        h = (h.rotate_left(17) ^ ((p as u64) << 32 | m as u64)).wrapping_mul(0xC2B2AE3D27D4EB4F);
                    }
                    Some(h)
                } else {
                    None
                };
                if let Some(k) = ckey {
                    if let Some(&(ok, s, e)) = self.cmemo.get(&k) {
                        self.ctr[9] += 1;
                        if ok {
                            for i in s..e {
                                let (a, m) = self.carena[i as usize];
                                if self.lookup(a).is_none() {
                                    self.env.push((a, m));
                                }
                            }
                        }
                        return ok;
                    }
                }
                self.base = mark;
                self.env.extend_from_slice(&pre[..np]);
                self.depth += 1;
                let r = self.eval(g.putil_body[*slot as usize], n);
                self.depth -= 1;
                let mut exports = [(0u16, 0u32); 8];
                let mut ne = 0;
                if r {
                    for &(p, a) in params.iter() {
                        if let Some(m) = self.lookup(p) {
                            exports[ne] = (a, m);
                            ne += 1;
                        }
                    }
                }
                self.env.truncate(mark);
                self.base = old_base;
                if let Some(k) = ckey {
                    let s0 = self.carena.len() as u32;
                    if r {
                        self.carena.extend_from_slice(&exports[..ne]);
                    }
                    self.cmemo.insert(k, (r, s0, self.carena.len() as u32));
                }
                if r {
                    for &(a, m) in &exports[..ne] {
                        if self.lookup(a).is_none() {
                            self.env.push((a, m));
                        }
                    }
                }
                r
            }
            Op::Rel { axis, stop, stopr, field, inner } => {
                let (axis, stop, stopr, f, inner) = (*axis, *stop, *stopr, *field, *inner);
                if self.mode == Mode::Mask && self.cfg.range_rel[id as usize] {
                    if !self.mask[id as usize].is_empty() {
                        return self.mask[id as usize][n as usize] != 0;
                    }
                    // adaptive promotion: build the mask once scans in this file exceed the node count
                    if self.scan_spent[id as usize] as usize > d.len() {
                        self.build_mask(id);
                        return self.mask[id as usize][n as usize] != 0;
                    }
                }
                let before = self.ctr[6];
                if self.mode != Mode::Plain && self.cfg.keyed[id as usize].is_some() {
                    if let Some(r) = self.keyed_scan(id, f, inner, n) {
                        return r;
                    }
                }
                let r = self.rel_scan(axis, stop, stopr, f, inner, n);
                if self.mode == Mode::Mask && self.cfg.range_rel[id as usize] {
                    let spent = (self.ctr[6] - before) as u32;
                    if self.scan_spent[id as usize] == 0 {
                        self.mask_touched.push(id);
                    }
                    self.scan_spent[id as usize] = self.scan_spent[id as usize].saturating_add(spent.max(1));
                }
                r
            }
        }
    }

    /// Keyed set stage: candidates of an env-ful has(stopBy: end) bucketed by their `name` child's text, built once
    /// per (relation, scope node) and file. The key comes from the env (a bound capture, or the outer side of an
    /// equality compare). Buckets keep preorder, so the first match equals the full scan's. None: key unknown.
    fn keyed_scan(&mut self, id: u32, f: u16, inner: u32, n: u32) -> Option<bool> {
        let (nf, srcs) = self.cfg.keyed[id as usize].clone()?;
        let mut key: Option<String> = None;
        for &v in &srcs {
            if let Some(r) = self.var_ref(v, 0) {
                key = Some(self.ref_str(r).to_string());
                break;
            }
        }
        let key = key?;
        let d = self.d;
        let ik = ((id as u64) << 32) | n as u64;
        if !self.kindex.contains_key(&ik) {
            self.ctr[7] += 1;
            let mut map: HashMap<String, Vec<u32>, FxB> = HashMap::default();
            let e = d.end[n as usize];
            for j in n + 1..e {
                if !self.kind_ok(inner, j) || !(f == 0 || d.field[j as usize] == f) {
                    continue;
                }
                self.ctr[8] += 1;
                let mut c = j + 1;
                while c < d.end[j as usize] {
                    if d.field[c as usize] == nf {
                        map.entry(d.text(c).to_string()).or_default().push(j);
                    }
                    c = d.end[c as usize];
                }
            }
            self.kindex.insert(ik, map);
        }
        let bucket: Vec<u32> = match self.kindex.get(&ik).and_then(|m| m.get(&key)) {
            Some(b) => b.clone(),
            None => return Some(false),
        };
        for j in bucket {
            self.ctr[6] += 1;
            if self.eval(inner, j) {
                return Some(true);
            }
        }
        Some(false)
    }

    #[inline]
    fn hit(&mut self, f: u16, inner: u32, j: u32) -> bool {
        self.ctr[6] += 1;
        self.fok(f, j) && self.kind_ok(inner, j) && self.eval(inner, j)
    }

    fn rel_scan(&mut self, axis: u8, stop: u8, stopr: u32, f: u16, inner: u32, n: u32) -> bool {
        let d = self.d;
        match axis {
            HAS => match stop {
                0 => {
                    let mut j = n + 1;
                    let e = d.end[n as usize];
                    while j < e {
                        if self.hit(f, inner, j) {
                            return true;
                        }
                        j = d.end[j as usize];
                    }
                    false
                }
                1 => {
                    // lazy DESCEND_KIND: contiguous kind-column scan over [n+1, end) when inner has root kinds
                    let e = d.end[n as usize];
                    match &self.cfg.rk[inner as usize] {
                        Some(ks) => {
                            let ks: &Vec<bool> = unsafe { &*(ks as *const Vec<bool>) };
                            for j in n + 1..e {
                                if ks[d.kind[j as usize] as usize] {
                                    self.ctr[6] += 1;
                                    if self.fok(f, j) && self.eval(inner, j) {
                                        return true;
                                    }
                                }
                            }
                            false
                        }
                        None => (n + 1..e).any(|j| self.hit(f, inner, j)),
                    }
                }
                _ => {
                    let mut j = n + 1;
                    let e = d.end[n as usize];
                    while j < e {
                        if self.hit(f, inner, j) {
                            return true;
                        }
                        let m = self.env.len();
                        let stopped = self.kind_ok(stopr, j) && self.eval(stopr, j);
                        self.env.truncate(m);
                        if stopped {
                            j = d.end[j as usize];
                        } else {
                            j += 1;
                        }
                    }
                    false
                }
            },
            INSIDE => {
                let mut cur = n;
                loop {
                    let p = d.parent[cur as usize];
                    if p == NONE {
                        return false;
                    }
                    self.ctr[6] += 1;
                    if self.fok(f, cur) && self.kind_ok(inner, p) && self.eval(inner, p) {
                        return true;
                    }
                    if stop == 0 {
                        return false;
                    }
                    if stop == 2 {
                        let m = self.env.len();
                        let stopped = self.kind_ok(stopr, p) && self.eval(stopr, p);
                        self.env.truncate(m);
                        if stopped {
                            return false;
                        }
                    }
                    cur = p;
                }
            }
            _ => {
                let mut j = if axis == PRECEDES {
                    let p = d.parent[n as usize];
                    if p == NONE || d.end[n as usize] >= d.end[p as usize] {
                        NONE
                    } else {
                        d.end[n as usize]
                    }
                } else {
                    d.prev[n as usize]
                };
                let pend = {
                    let p = d.parent[n as usize];
                    if p == NONE {
                        0
                    } else {
                        d.end[p as usize]
                    }
                };
                while j != NONE {
                    if self.hit(f, inner, j) {
                        return true;
                    }
                    if stop == 0 {
                        return false;
                    }
                    if stop == 2 {
                        let m = self.env.len();
                        let stopped = self.kind_ok(stopr, j) && self.eval(stopr, j);
                        self.env.truncate(m);
                        if stopped {
                            return false;
                        }
                    }
                    j = if axis == PRECEDES {
                        let nx = d.end[j as usize];
                        if nx >= pend {
                            NONE
                        } else {
                            nx
                        }
                    } else {
                        d.prev[j as usize]
                    };
                }
                false
            }
        }
    }

    fn column(&mut self, id: u32) -> Vec<bool> {
        let n = self.d.len();
        (0..n as u32)
            .map(|j| {
                let m = self.env.len();
                let r = self.kind_ok(id, j) && self.eval(id, j);
                self.env.truncate(m);
                r
            })
            .collect()
    }

    /// Set-stage pass: one column for an env-free range relation (has/inside/sibling, stopBy end or rule).
    fn build_mask(&mut self, id: u32) {
        let (axis, stop, stopr, f, inner) = match &self.g.ops[id as usize] {
            Op::Rel { axis, stop, stopr, field, inner } => (*axis, *stop, *stopr, *field, *inner),
            _ => unreachable!(),
        };
        let d = self.d;
        let n = d.len();
        self.ctr[7] += 1;
        self.ctr[8] += n as u64;
        let ic = self.column(inner);
        let sc = if stop == 2 { self.column(stopr) } else { vec![] };
        let h = |j: usize| (f == 0 || d.field[j] == f) && ic[j];
        let mut col = vec![0u8; n];
        match axis {
            HAS => {
                // reverse preorder: sub[parent] |= hit(child) | (descend-through(child) & sub[child])
                for j in (1..n).rev() {
                    let p = d.parent[j] as usize;
                    let v = h(j)
                        || match stop {
                            0 => false,
                            1 => col[j] != 0,
                            _ => !sc[j] && col[j] != 0,
                        };
                    if v {
                        col[p] = 1;
                    }
                }
            }
            INSIDE => {
                for j in 1..n {
                    let p = d.parent[j] as usize;
                    let up = (f == 0 || d.field[j] == f) && ic[p];
                    let v = up
                        || match stop {
                            0 => false,
                            1 => col[p] != 0,
                            _ => !sc[p] && col[p] != 0,
                        };
                    col[j] = v as u8;
                }
            }
            _ => {
                // siblings: forward (follows) or backward (precedes) running OR per parent
                for p in 0..n {
                    let mut kids: Vec<usize> = vec![];
                    let mut j = p as u32 + 1;
                    while j < d.end[p] {
                        kids.push(j as usize);
                        j = d.end[j as usize];
                    }
                    if axis == PRECEDES {
                        kids.reverse();
                    }
                    let mut acc = false;
                    for &c in &kids {
                        col[c] = acc as u8;
                        acc = h(c)
                            || match stop {
                                0 => false,
                                1 => acc,
                                _ => !sc[c] && acc,
                            };
                    }
                }
            }
        }
        if self.scan_spent[id as usize] == 0 {
            self.mask_touched.push(id);
        }
        self.scan_spent[id as usize] = u32::MAX;
        self.mask[id as usize] = col;
    }
}


/// Decoded `str` value of a string literal (Ruff-like): no interpolation, no b/t prefix; escapes kept raw.
fn string_value(d: &Doc, n: u32) -> Option<String> {
    let k = d.kind[n as usize];
    let ks = KINDS.with(|c| c.get());
    let piece = |i: u32| -> Option<String> {
        // interpolation child -> unknown
        let mut j = i + 1;
        while j < d.end[i as usize] {
            if d.kind[j as usize] == ks.2 {
                return None;
            }
            j = d.end[j as usize];
        }
        let t = d.text(i);
        let q = t.find(['\'', '"'])?;
        let pre = t[..q].to_ascii_lowercase();
        if !matches!(pre.as_str(), "" | "r" | "u" | "f" | "fr" | "rf") {
            return None;
        }
        let rest = &t[q..];
        let ql = if rest.starts_with("'''") || rest.starts_with("\"\"\"") { 3 } else { 1 };
        if rest.len() < 2 * ql {
            return None;
        }
        let mut body = rest[ql..rest.len() - ql].to_string();
        if pre.contains('f') {
            body = body.replace("{{", "{").replace("}}", "}");
        }
        Some(body)
    };
    if k == ks.0 {
        piece(n)
    } else if k == ks.1 {
        let mut out = String::new();
        let mut j = n + 1;
        while j < d.end[n as usize] {
            if d.kind[j as usize] == ks.0 {
                out.push_str(&piece(j)?);
            }
            j = d.end[j as usize];
        }
        Some(out)
    } else {
        None
    }
}
thread_local!(static FACTOR: std::cell::Cell<bool> = const { std::cell::Cell::new(true) });
thread_local!(static FLATTEN: std::cell::Cell<bool> = const { std::cell::Cell::new(true) });
thread_local!(static KINDS: std::cell::Cell<(u16, u16, u16)> = const { std::cell::Cell::new((NOKIND, NOKIND, NOKIND)) });

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (root, files_txt, out) = (&a[1], &a[2], &a[3]);
    let modes: Vec<Mode> = a[4]
        .split(',')
        .map(|m| match m {
            "plain" => Mode::Plain,
            "memo" => Mode::Memo,
            "mask" => Mode::Mask,
            _ => panic!("mode {m}"),
        })
        .collect();
    let reps: usize = a[5].parse().unwrap();
    let rule_paths = &a[6..];
    let lang = Lang::new();
    FACTOR.with(|c| c.set(std::env::var("FACTOR").map_or(true, |v| v != "0")));
    FLATTEN.with(|c| c.set(std::env::var("FLATTEN").map_or(true, |v| v != "0")));
    KINDS.with(|c| c.set((lang.kind("string"), lang.kind("concatenated_string"), lang.kind("interpolation"))));

    // ---- compile rules ----
    let tc = Instant::now();
    let mut g = Dag::default();
    let mut rules: Vec<Rule> = vec![];
    let mut progs: Vec<Program> = vec![];
    for rp in rule_paths {
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(rp).unwrap()).unwrap();
        let mut opts: HashMap<String, Value> = HashMap::new();
        let mut utils = serde_json::Map::new();
        for c in raw["clauses"].as_array().unwrap() {
            if let Some(def) = c.get("define") {
                for (k, v) in def.get("options").and_then(|o| o.as_object()).into_iter().flatten() {
                    opts.insert(k.clone(), v["default"].clone());
                }
            }
        }
        let rule = resolve_options(&raw, &opts);
        for c in rule["clauses"].as_array().unwrap() {
            if let Some(def) = c.get("define") {
                for (k, v) in def.get("utils").and_then(|o| o.as_object()).into_iter().flatten() {
                    utils.insert(k.clone(), v.clone());
                }
            }
        }
        let ri = rules.len();
        rules.push(Rule { id: rule["id"].as_str().unwrap().to_string(), inc: glob(&rule["scope"]["include"]), exc: glob(&rule["scope"]["exclude"]) });
        for c in rule["clauses"].as_array().unwrap() {
            let (body, ast) = if let Some(f) = c.get("forbid") {
                (f, f.get("ast"))
            } else if let Some(b) = c.get("budget") {
                (b, b.pointer("/query/evidence/query/ast"))
            } else {
                continue;
            };
            let Some(ast) = ast else {
                eprintln!("skip non-ast clause {}", c["id"]);
                continue;
            };
            if let Some(b) = c.get("budget") {
                // budget{count lte 0, group_by path,line} over rows projected at capture.X: a finding per row line
                assert_eq!(b["count"], json!({"lte": 0}));
            }
            if !when_active(body.get("when"), &opts) || !when_active(ast.get("when"), &opts) {
                continue;
            }
            let mut uenv = utils.clone();
            for (k, v) in ast.get("utils").and_then(|o| o.as_object()).into_iter().flatten() {
                uenv.insert(k.clone(), v.clone());
            }
            let mut st = vec![];
            let root = g.compile(&lang, &ast["rule"], &uenv, &mut st);
            let mut transforms = FxMap::default();
            for (k, v) in ast.get("transform").and_then(|o| o.as_object()).into_iter().flatten() {
                let rep = &v["replace"];
                let var = g.var(k);
                let src = g.var(rep["source"].as_str().unwrap());
                let re = g.regex(rep["replace"].as_str().unwrap());
                transforms.insert(var, Transform { src, re, by: rep["by"].as_str().unwrap().to_string() });
            }
            let fp = ast.pointer("/file/path");
            progs.push(Program {
                idx: progs.len() as u32,
                rule: ri,
                clause: c["id"].as_str().unwrap().to_string(),
                code: c["violation"]["code"].as_str().unwrap().to_string(),
                root,
                kinds: None,
                path_re: fp.and_then(|p| p.get("regex")).and_then(|r| r.as_str()).map(Re::new),
                path_not: fp.and_then(|p| p.get("notRegex")).and_then(|r| r.as_str()).map(Re::new),
                syntax_valid: ast.get("syntax").and_then(|s| s.as_str()) == Some("valid"),
                transforms,
            });
        }
    }
    let compile_s = tc.elapsed().as_secs_f64();
    let nops = g.ops.len();
    let nk = lang.names.len() + 1;

    // ---- static analysis of the DAG ----
    let mut rkmemo = HashMap::new();
    let rk: Vec<Option<Vec<bool>>> = (0..nops as u32)
        .map(|id| {
            g.root_kinds(id, &mut rkmemo).map(|ks| {
                let mut b = vec![false; nk];
                for k in ks {
                    if (k as usize) < nk {
                        b[k as usize] = true;
                    }
                }
                b
            })
        })
        .collect();
    for p in progs.iter_mut() {
        p.kinds = rk[p.root as usize].clone();
    }
    // env-free fixpoint (Util/Call back-edges assumed free, iterate)
    let mut free = vec![true; nops];
    let mut reads = vec![false; nops]; // contains Compare/Layout/Call
    loop {
        let mut changed = false;
        for id in 0..nops {
            let (f, r) = match &g.ops[id] {
                Op::Bind(_) => (false, false),
                Op::Compare { .. } | Op::Layout(_) => (false, true),
                Op::Call { .. } => (false, true),
                Op::Util(s) => {
                    let b = g.util_body[*s as usize] as usize;
                    (free[b], reads[b])
                }
                op => {
                    let ks = op_kids(op);
                    (ks.iter().all(|&k| free[k as usize]), ks.iter().any(|&k| reads[k as usize]))
                }
            };
            if f != free[id] || r != reads[id] {
                free[id] = f;
                reads[id] = r;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // vars bound per id (for bind-memo of capture producers that read nothing)
    let mut binds: Vec<Vec<u16>> = vec![vec![]; nops];
    loop {
        let mut changed = false;
        for id in 0..nops {
            let mut s: Vec<u16> = match &g.ops[id] {
                Op::Bind(v) => vec![*v],
                Op::Util(sl) => binds[g.util_body[*sl as usize] as usize].clone(),
                // every var a subtree may bind or compare-by-binding, negated contexts included: the bind-memo
                // is valid only when none of them is bound on entry
                op => op_kids(op).iter().flat_map(|&k| binds[k as usize].clone()).collect(),
            };
            s.sort_unstable();
            s.dedup();
            if s != binds[id] {
                binds[id] = s;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let cheap = |op: &Op| matches!(op, Op::True | Op::False | Op::Kind(_) | Op::Named | Op::FilePath { .. } | Op::Bind(_));
    let util_roots: HashSet<u32> = g.util_body.iter().copied().collect();
    // memo policy: "all" = every shared env-free node (fusion-model `memo`); "lean" = only nodes whose evaluation
    // scans (range relations, counts, nth-of-rule, non-leaf regex/value) or contains a range scan.
    let lean = std::env::var("MEMO_POLICY").map_or(true, |v| v == "lean");
    let mut scans = vec![false; nops];
    loop {
        let mut changed = false;
        for id in 0..nops {
            let v = match &g.ops[id] {
                Op::Rel { stop, .. } if *stop != 0 => true,
                Op::Count { .. } | Op::ValueRe(_) | Op::ValueEq(_) | Op::ValueStr => true,
                Op::Nth { of, .. } if *of != NONE => true,
                Op::Util(sl) => scans[g.util_body[*sl as usize] as usize],
                op => op_kids(op).iter().any(|&k| scans[k as usize]),
            };
            if v != scans[id] {
                scans[id] = v;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let memo_free: Vec<bool> = (0..nops)
        .map(|id| {
            free[id]
                && !cheap(&g.ops[id])
                && if lean {
                    matches!(g.ops[id], Op::Regex(_)) || (scans[id] && (g.refs[id] >= 2 || util_roots.contains(&(id as u32)) || matches!(g.ops[id], Op::Rel { .. } | Op::Count { .. } | Op::Nth { .. })))
                } else {
                    g.refs[id] >= 2 || util_roots.contains(&(id as u32)) || matches!(g.ops[id], Op::Regex(_) | Op::ValueRe(_) | Op::Rel { .. } | Op::Nth { .. })
                }
        })
        .collect();
    let bind_memo: Vec<bool> = (0..nops).map(|id| !free[id] && !reads[id] && !binds[id].is_empty() && !cheap(&g.ops[id]) && (g.refs[id] >= 2 || util_roots.contains(&(id as u32)))).collect();
    let range_rel: Vec<bool> = (0..nops).map(|id| matches!(g.ops[id], Op::Rel { stop, .. } if stop != 0) && free[id]).collect();
    // keyed relations: has(stopBy end) whose inner conjunction pins the candidate's `name` child to a capture
    // (has{field: name, pattern: $V}) and V is either bound on entry or compared equal to an outer capture.
    let name_field = lang.field("name");
    let keyed_on = std::env::var("KEYED").map_or(true, |v| v != "0");
    let keyed: Vec<Option<(u16, Vec<u16>)>> = (0..nops)
        .map(|id| {
            if !keyed_on {
                return None;
            }
            let Op::Rel { axis: HAS, stop: 1, inner, .. } = g.ops[id] else { return None };
            if free[id] {
                return None;
            }
            let list = match &g.ops[inner as usize] {
                Op::All(v) => v.clone(),
                _ => return None,
            };
            let mut key_var = None;
            for &e in &list {
                if let Op::Rel { axis: HAS, stop: 0, field, inner: b, .. } = g.ops[e as usize] {
                    if field == name_field {
                        if let Op::Bind(v) = g.ops[b as usize] {
                            key_var = Some(v);
                        }
                    }
                }
            }
            let v = key_var?;
            let mut srcs = vec![v];
            for &e in &list {
                if let Op::Compare { l, r, ne: false } = g.ops[e as usize] {
                    if l == v {
                        srcs.push(r);
                    } else if r == v {
                        srcs.push(l);
                    }
                }
            }
            Some((name_field, srcs))
        })
        .collect();
    let cfg = Cfg { memo_free, env_free: free.clone(), bind_memo, binds, range_rel, rk, leaf_regex: true, keyed };
    let mut cfg = cfg;
    cfg.leaf_regex = std::env::var("LEAF_REGEX").map_or(true, |v| v != "0");
    let _ = &cfg.env_free;
    let mut opc: HashMap<&str, u64> = HashMap::new();
    for op in &g.ops {
        *opc.entry(match op {
            Op::True | Op::False => "const",
            Op::Kind(_) => "kind",
            Op::Named => "named",
            Op::Regex(_) => "regex",
            Op::ValueRe(_) | Op::ValueEq(_) | Op::ValueStr => "value",
            Op::FilePath { .. } => "filePath",
            Op::Nth { .. } => "nth",
            Op::All(_) => "all",
            Op::Any(_) => "any",
            Op::Not(_) => "not",
            Op::Rel { stop: 0, .. } => "rel.neighbor",
            Op::Rel { .. } => "rel.range",
            Op::Util(_) => "util.backref",
            Op::Bind(_) => "bind",
            Op::Compare { .. } => "compare",
            Op::Layout(_) => "layout",
            Op::Count { .. } => "count",
            Op::Call { .. } => "call",
        })
        .or_default() += 1;
    }
    eprintln!("compiled {} programs into {} ops ({} expansions) in {:.2}s; regexes {}; memo-free {} bind-memo {} range-rel {}", progs.len(), nops, g.expansions, compile_s, g.regex.len(), cfg.memo_free.iter().filter(|x| **x).count(), cfg.bind_memo.iter().filter(|x| **x).count(), cfg.range_rel.iter().filter(|x| **x).count());

    // ---- corpus: only files in some rule's scope ----
    let t0 = Instant::now();
    let paths: Vec<String> = std::fs::read_to_string(files_txt).unwrap().lines().filter(|l| l.ends_with(".py")).filter(|p| rules.iter().any(|r| r.inc.is_match(p) && !r.exc.is_match(p))).map(String::from).collect();
    if paths.is_empty() {
        std::fs::write(out, serde_json::to_string_pretty(&json!({"keys": [], "files": 0})).unwrap()).unwrap();
        return;
    }
    let mut docs: Vec<Doc> = paths
        .par_iter()
        .filter_map(|p| {
            let bytes = std::fs::read(format!("{root}/{p}")).ok()?;
            let src = String::from_utf8_lossy(&bytes).into_owned();
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&lang.ts).unwrap();
            let tree = parser.parse(&src, None)?;
            Some(build_doc(&lang, p.clone(), src, &tree))
        })
        .collect();
    docs.sort_by(|a, b| a.path.cmp(&b.path));
    // intern leaf texts (named leaves up to 128 bytes) corpus-wide
    let mut symtab: HashMap<String, u32> = HashMap::new();
    for d in docs.iter_mut() {
        for i in 0..d.len() {
            if d.end[i] == i as u32 + 1 && d.named[i] {
                let t = &d.src[d.sb[i] as usize..d.eb[i] as usize];
                if t.len() <= 128 {
                    let nsym = symtab.len() as u32;
                    let s = *symtab.entry(t.to_string()).or_insert(nsym);
                    d.sym[i] = s;
                }
            }
        }
    }
    let load_s = t0.elapsed().as_secs_f64();
    let total_nodes: usize = docs.iter().map(|d| d.len()).sum();
    eprintln!("corpus: {} files, {} nodes, {} distinct leaf texts, load {:.1}s", docs.len(), total_nodes, symtab.len(), load_s);

    // per-file active program sets + program-node accounting
    let np = progs.len();
    let mut active: Vec<Vec<u32>> = vec![];
    let mut prog_nodes_actual = 0u64; // Σ programs × nodes of files the program applies to
    let mut rule_scope_nodes = vec![0u64; rules.len()];
    let mut rule_scope_files = vec![0u64; rules.len()];
    for d in &docs {
        let mut v = vec![];
        for (ri, r) in rules.iter().enumerate() {
            if r.inc.is_match(&d.path) && !r.exc.is_match(&d.path) {
                rule_scope_nodes[ri] += d.len() as u64;
                rule_scope_files[ri] += 1;
            }
        }
        for (pi, p) in progs.iter().enumerate() {
            let r = &rules[p.rule];
            if !(r.inc.is_match(&d.path) && !r.exc.is_match(&d.path)) {
                continue;
            }
            if p.path_re.as_ref().is_some_and(|re| !re.is_match(&d.path)) || p.path_not.as_ref().is_some_and(|re| re.is_match(&d.path)) {
                continue;
            }
            if p.syntax_valid && d.has_error {
                continue;
            }
            v.push(pi as u32);
            prog_nodes_actual += d.len() as u64;
        }
        active.push(v);
    }
    let mut prog_nodes_scope = 0u64; // heavy-rule-profiles definition: programs × rule-scope nodes
    for p in &progs {
        prog_nodes_scope += rule_scope_nodes[p.rule];
    }
    let syntax_skipped: Vec<&str> = docs.iter().filter(|d| d.has_error).map(|d| d.path.as_str()).collect();

    let prog_kinds: Vec<Option<Vec<u16>>> = progs.iter().map(|p| p.kinds.as_ref().map(|ks| ks.iter().enumerate().filter(|(_, b)| **b).map(|(k, _)| k as u16).collect())).collect();
    // ---- run ----
    let trie = std::env::var("TRIE").map_or(true, |v| v != "0");
    let index = std::env::var("DISPATCH").map_or(false, |v| v == "index");
    let reorder = std::env::var("REORDER").map_or(true, |v| v != "0");
    let reorder_inner_only = std::env::var("REORDER").map_or(true, |v| v == "inner");
    let root_ids: HashSet<u32> = progs.iter().map(|p| p.root).collect();
    // root conjunct lists (flattened All) per program
    let conj_list = |g: &Dag, id: u32| -> Vec<u32> {
        match &g.ops[id as usize] {
            Op::All(v) => v.clone(),
            _ => vec![id],
        }
    };
    // kind postings per file for index dispatch (built at ingest in the real system: not timed)
    let need_kind: Vec<bool> = {
        let mut b = vec![false; nk];
        for ks in prog_kinds.iter().flatten() {
            for &k in ks {
                b[k as usize] = true;
            }
        }
        b
    };
    let any_unconstrained = prog_kinds.iter().any(|k| k.is_none());
    let postings: Vec<Vec<u32>> = if index && !any_unconstrained {
        docs.iter().map(|d| (0..d.len() as u32).filter(|&n| need_kind[d.kind[n as usize] as usize]).collect()).collect()
    } else {
        vec![]
    };

    // Root trie: programs dispatched at one kind are merged on their leading reads-free conjuncts (a prefix tree);
    // each program's remaining suffix is evaluated under the env its shared prefix produced.
    #[derive(Default, Clone)]
    struct Trie {
        op: u32,
        kids: Vec<Trie>,
        progs: Vec<(u32, usize)>, // (program, suffix start)
    }
    fn trie_insert(t: &mut Trie, list: &[u32], at: usize, pi: u32, reads: &[bool]) {
        // keep at least one conjunct for the program itself
        if at + 1 >= list.len() || reads[list[at] as usize] {
            t.progs.push((pi, at));
            return;
        }
        let op = list[at];
        let idx = match t.kids.iter().position(|k| k.op == op) {
            Some(i) => i,
            None => {
                t.kids.push(Trie { op, ..Default::default() });
                t.kids.len() - 1
            }
        };
        trie_insert(&mut t.kids[idx], list, at + 1, pi, reads);
    }
    fn trie_walk<'a>(ev: &mut Ev<'a>, t: &Trie, n: u32, d: &Doc, progs: &'a [Program], roots: &[Vec<u32>], keys: &mut HashSet<(usize, String, u32, String)>, pp: &mut [u64]) {
        let mark = ev.env.len();
        for &(pi, at) in &t.progs {
            let p = &progs[pi as usize];
            ev.ctr[10] += 1;
            ev.env.truncate(mark);
            ev.prog = Some(p);
            let e0 = ev.ctr[0];
            let mut ok = true;
            for &c in &roots[pi as usize][at..] {
                if !ev.eval(c, n) {
                    ok = false;
                    break;
                }
            }
            if ok {
                if ev.prof {
                    eprintln!("HIT {} node {} line {}", p.clause, n, d.line[n as usize] + 1);
                }
                keys.insert((p.rule, d.path.clone(), d.line[n as usize] + 1, p.code.clone()));
            }
            pp[pi as usize] += ev.ctr[0] - e0;
        }
        for k in &t.kids {
            ev.env.truncate(mark);
            if ev.eval(k.op, n) {
                trie_walk(ev, k, n, d, progs, roots, keys, pp);
            }
        }
        ev.env.truncate(mark);
    }

    // One pass over a set of files. Returns findings, per-program evals, per-file times.
    #[allow(clippy::too_many_arguments)]
    fn run_pass<'a>(
        ev: &mut Ev<'a>, docs: &'a [Doc], files: &[usize], active: &[Vec<u32>], progs: &'a [Program], prog_kinds: &[Option<Vec<u16>>],
        roots: &[Vec<u32>], cfg_reads: &[bool], trie: bool, postings: &[Vec<u32>], nk: usize,
    ) -> (HashSet<(usize, String, u32, String)>, Vec<u64>, Vec<(f64, usize, String)>) {
        let np = progs.len();
        let mut keys = HashSet::new();
        let mut pp = vec![0u64; np];
        let mut pf = vec![];
        let mut disp: Vec<Vec<u32>> = vec![vec![]; nk];
        let mut any_kind: Vec<u32> = vec![];
        let mut tries: Vec<Trie> = vec![Trie::default(); nk];
        let mut any_trie = Trie::default();
        let mut used_kinds: Vec<usize> = vec![];
        let no_reads: Vec<bool>;
        let reads_for_trie: &[bool] = if trie {
            cfg_reads
        } else {
            no_reads = vec![true; cfg_reads.len()];
            &no_reads
        };
        let build = |list: &[u32]| -> Trie {
            let mut t = Trie::default();
            for &pi in list {
                trie_insert(&mut t, &roots[pi as usize], 0, pi, reads_for_trie);
            }
            t
        };
        for &fi in files {
            let d = &docs[fi];
            if active[fi].is_empty() {
                continue;
            }
            let tf = Instant::now();
            ev.reset(d);
            for &k in &used_kinds {
                disp[k].clear();
                tries[k] = Trie::default();
            }
            used_kinds.clear();
            any_kind.clear();
            for &pi in &active[fi] {
                match &prog_kinds[pi as usize] {
                    Some(ks) => {
                        for &k in ks {
                            if disp[k as usize].is_empty() {
                                used_kinds.push(k as usize);
                            }
                            disp[k as usize].push(pi);
                        }
                    }
                    None => any_kind.push(pi),
                }
            }
            for &k in &used_kinds {
                tries[k] = build(&disp[k]);
            }
            any_trie = build(&any_kind);
            let mut visit = |ev: &mut Ev<'a>, n: u32, keys: &mut HashSet<(usize, String, u32, String)>, pp: &mut Vec<u64>| {
                let k = d.kind[n as usize] as usize;
                ev.env.clear();
                ev.base = 0;
                trie_walk(ev, &tries[k], n, d, progs, roots, keys, pp);
                if !any_trie.progs.is_empty() || !any_trie.kids.is_empty() {
                    ev.env.clear();
                    trie_walk(ev, &any_trie, n, d, progs, roots, keys, pp);
                }
            };
            if !postings.is_empty() {
                for &n in &postings[fi] {
                    visit(ev, n, &mut keys, &mut pp);
                }
            } else {
                for n in 0..d.len() as u32 {
                    let k = d.kind[n as usize] as usize;
                    if disp[k].is_empty() && any_kind.is_empty() {
                        continue;
                    }
                    visit(ev, n, &mut keys, &mut pp);
                }
            }
            pf.push((tf.elapsed().as_secs_f64(), d.len(), d.path.clone()));
        }
        (keys, pp, pf)
    }

    // ---- plan feedback: calibrate on a sample, reorder env-free conjuncts by cost/(1-selectivity) ----
    let mut calib_s = 0.0;
    let mut reordered = 0usize;
    let reorder_max: usize = std::env::var("REORDER_MAX").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    if reorder {
        let tcal = Instant::now();
        let roots0: Vec<Vec<u32>> = progs.iter().map(|p| conj_list(&g, p.root)).collect();
        let sample: Vec<usize> = (0..docs.len()).filter(|i| i % 10 == 3).collect();
        let stats = {
            let mut ev = Ev::new(&g, &cfg, Mode::Memo, &docs[0]);
            ev.maxlen = docs.iter().map(|d| d.len()).max().unwrap_or(0);
            ev.calib = vec![(0, 0, 0); nops];
            let _ = run_pass(&mut ev, &docs, &sample, &active, &progs, &prog_kinds, &roots0, &reads, false, &[], nk);
            ev.calib
        };
        let rank = |id: u32| -> f64 {
            let (c, t, w) = stats[id as usize];
            if c == 0 {
                return f64::INFINITY;
            }
            let cost = w as f64 / c as f64;
            let p = t as f64 / c as f64;
            cost / (1.0 - p).max(1e-3)
        };
        for id in 0..nops {
            if reorder_inner_only && root_ids.contains(&(id as u32)) {
                continue;
            }
            let Op::All(v) = &g.ops[id] else { continue };
            let v = v.clone();
            let fixed: Vec<u32> = v.iter().copied().filter(|&x| !free[x as usize]).collect();
            let mut movable: Vec<u32> = v.iter().copied().filter(|&x| free[x as usize]).collect();
            movable.sort_by(|a, b| rank(*a).partial_cmp(&rank(*b)).unwrap());
            let mut out = vec![];
            let (mut i, mut j) = (0, 0);
            while i < fixed.len() || j < movable.len() {
                let take_fixed = j >= movable.len() || (i < fixed.len() && rank(fixed[i]) <= rank(movable[j]));
                if take_fixed {
                    out.push(fixed[i]);
                    i += 1;
                } else {
                    out.push(movable[j]);
                    j += 1;
                }
            }
            if out != v {
                if reordered < reorder_max {
                    if std::env::var("REORDER_DEBUG").is_ok() {
                        eprintln!("reorder {id}: {:?} -> {:?}", v, out);
                    }
                    g.ops[id] = Op::All(out);
                }
                reordered += 1;
            }
        }
        calib_s = tcal.elapsed().as_secs_f64();
        eprintln!("plan feedback: {} sample files, {} All nodes reordered, {:.2}s", sample.len(), reordered, calib_s);
    }
    if let Ok(ids) = std::env::var("DUMP") {
        fn dump(g: &Dag, free: &[bool], id: u32, depth: usize, max: usize, seen: &mut HashSet<u32>) {
            let mut d = format!("{:?}", g.ops[id as usize]);
            d.truncate(100);
            eprintln!("{}{} free={} {}", "  ".repeat(depth), id, free[id as usize] as u8, d);
            if depth >= max || !seen.insert(id) {
                return;
            }
            let ks = match &g.ops[id as usize] {
                Op::Util(s) => vec![g.util_body[*s as usize]],
                Op::Call { slot, .. } => vec![g.putil_body[*slot as usize]],
                op => op_kids(op),
            };
            for k in ks {
                dump(g, free, k, depth + 1, max, seen);
            }
        }
        for id in ids.split(',') {
            dump(&g, &free, id.parse().unwrap(), 0, 6, &mut HashSet::new());
        }
    }
    let roots: Vec<Vec<u32>> = progs.iter().map(|p| conj_list(&g, p.root)).collect();
    let all_files: Vec<usize> = (0..docs.len()).collect();

    let mut results = vec![];
    let mut reference: Option<Vec<String>> = None;
    for &mode in &modes {
        let mut best = f64::INFINITY;
        let mut best_wall = f64::INFINITY;
        let mut keys_out = vec![];
        let mut ctr_out = [0u64; NC];
        let mut per_prog = vec![0u64; np];
        let mut per_file: Vec<(f64, usize, String)> = vec![];
        for _rep in 0..reps {
            let mut ev = Ev::new(&g, &cfg, mode, &docs[0]);
            ev.maxlen = docs.iter().map(|d| d.len()).max().unwrap_or(0);
            let t = Instant::now();
            let c0 = thread_cpu();
            let (keys, pp, pf) = run_pass(&mut ev, &docs, &all_files, &active, &progs, &prog_kinds, &roots, &reads, trie, &postings, nk);
            let wall = t.elapsed().as_secs_f64();
            let el = thread_cpu() - c0;
            eprintln!("  mode {:?} rep: wall {:.3}s cpu {:.3}s evals {} ", mode as u8, wall, el, ev.ctr[0]);
            if ev.prof {
                let mut v: Vec<(u64, usize)> = ev.op_evals.iter().copied().zip(0..).filter(|x| x.0 > 0).collect();
                v.sort_by(|a, b| b.0.cmp(&a.0));
                for (c, id) in v.iter().take(40) {
                    let mut desc = format!("{:?}", g.ops[*id]);
                    desc.truncate(110);
                    eprintln!("    op {id:5} evals {c:10} memo={} free={} rk={} {desc}", cfg.memo_free[*id] as u8, cfg.env_free[*id] as u8, cfg.rk[*id].is_some() as u8);
                }
            }
            if wall < best_wall {
                best_wall = wall;
            }
            if el < best {
                best = el;
                ctr_out = ev.ctr;
                per_prog = pp;
                per_file = pf;
            }
            let mut kv: Vec<String> = keys.iter().map(|(r, p, l, c)| json!([rules[*r].id, p, l, c]).to_string()).collect();
            kv.sort();
            keys_out = kv;
        }
        if let Some(r) = &reference {
            assert_eq!(r, &keys_out, "modes disagree");
        } else {
            reference = Some(keys_out.clone());
        }
        per_file.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let ctr: serde_json::Map<String, Value> = CN.iter().zip(ctr_out.iter()).map(|(k, v)| (k.to_string(), json!(v))).collect();
        let mut pp: Vec<(u64, usize)> = per_prog.iter().copied().zip(0..).collect();
        pp.sort_by(|a, b| b.0.cmp(&a.0));
        results.push(json!({
            "mode": match mode { Mode::Plain => "plain", Mode::Memo => "memo", Mode::Mask => "mask" },
            "seconds_min": best,
            "wall_seconds_min": best_wall,
            "ns_per_program_node_scope": best * 1e9 / prog_nodes_scope as f64,
            "ns_per_program_node_actual": best * 1e9 / prog_nodes_actual.max(1) as f64,
            "ns_per_scope_node": best * 1e9 / total_nodes as f64,
            "ns_per_eval": best * 1e9 / ctr_out[0].max(1) as f64,
            "counters": ctr,
            "top_files": per_file.iter().take(8).map(|(s, n, p)| json!([p, n, s])).collect::<Vec<_>>(),
            "top_programs_by_evals": pp.iter().take(10).map(|(e, i)| json!([rules[progs[*i].rule].id, progs[*i].clause, e])).collect::<Vec<_>>(),
            "findings": keys_out.len(),
        }));
    }
    let report = json!({
        "rules": rules.iter().enumerate().map(|(i, r)| json!({"id": r.id, "scope_files": rule_scope_files[i], "scope_nodes": rule_scope_nodes[i], "programs": progs.iter().filter(|p| p.rule == i).count()})).collect::<Vec<_>>(),
        "programs": np, "ops": nops, "expansions": g.expansions, "regexes": g.regex.len(), "op_classes": opc,
        "compile_s": compile_s, "load_s": load_s, "calibration_s": calib_s, "reordered_all_nodes": reordered, "trie": trie, "index_dispatch": index, "files": docs.len(), "nodes": total_nodes, "distinct_leaf_texts": symtab.len(),
        "program_nodes_scope": prog_nodes_scope, "program_nodes_actual": prog_nodes_actual,
        "syntax_error_files": syntax_skipped,
        "results": results,
        "keys": reference.unwrap_or_default(),
    });
    std::fs::write(out, serde_json::to_string_pretty(&report).unwrap()).unwrap();
}
