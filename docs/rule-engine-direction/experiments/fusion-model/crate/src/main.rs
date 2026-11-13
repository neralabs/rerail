//! Calibration harness for the fusion performance model.
//!
//! Real data-layer rule queries (option-resolved by analyze.py) are compiled into one hash-consed rule
//! DAG (identical sub-rules and utility expansions share an id) and evaluated over the real gate corpus:
//!
//! - `sep`:       separate anchored programs: per query, per file, root-kind candidates, recursive eval.
//! - `fused`:     one pass per file, dense root-kind dispatch to every query (trie root sharing only).
//! - `memo`:      `fused` + per-file memo columns for shared DAG ids (utility / sub-rule dedup).
//! - `mask`:      `memo` + eager per-file stratum mask columns for scanning relations
//!                (has/inside/precedes/follows with stopBy end or rule).
//! - `maskall`:   `mask` for every relation, neighbor axes too.
//!
//! All variants must produce the same finding set (asserted by count + order-free hash).
//! Non-structural Rerail keys (count, compare, layout, binds, calls, ...) evaluate to true, so the
//! finding set is a superset of the real rule's; the structural work is the real rule's.
//!
//! Usage: fusion-model QUERIES_FULL TEXT_REGEX CORPUS_ROOT FILES_TXT OUT_JSON [modes,comma] [reps]

use rayon::prelude::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;

const NONE: u32 = u32::MAX;
const NOKIND: u16 = u16::MAX - 1;

// ---------- kinds ----------
struct Lang {
    ts: tree_sitter::Language,
    ts2: Vec<u16>,
    by_name: HashMap<String, u16>,
    names: Vec<String>,
}

impl Lang {
    fn new() -> Lang {
        let ts: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
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
        self.by_name.get(name).copied().unwrap_or(NOKIND)
    }
    fn field(&self, name: &str) -> u16 {
        self.ts.field_id_for_name(name).map_or(u16::MAX, |f| f.get())
    }
}

// ---------- regular tape ----------
struct Doc {
    path: String,
    src: String,
    kind: Vec<u16>,
    field: Vec<u16>,
    named: Vec<bool>,
    parent: Vec<u32>,
    end: Vec<u32>,
    sb: Vec<u32>,
    eb: Vec<u32>,
    nth: Vec<u32>,
    nthr: Vec<u32>,
    depth: Vec<u16>,
    parse_s: f64,
}

impl Doc {
    fn text(&self, n: u32) -> &str {
        &self.src[self.sb[n as usize] as usize..self.eb[n as usize] as usize]
    }
    fn len(&self) -> usize {
        self.kind.len()
    }
}

fn build_doc(lang: &Lang, path: String, src: String, tree: &tree_sitter::Tree) -> Doc {
    let mut d = Doc { path, src, kind: vec![], field: vec![], named: vec![], parent: vec![], end: vec![], sb: vec![], eb: vec![], nth: vec![], nthr: vec![], depth: vec![], parse_s: 0.0 };
    let mut cursor = tree.walk();
    let mut stack: Vec<u32> = vec![];
    'outer: loop {
        let node = cursor.node();
        let id = d.kind.len() as u32;
        d.kind.push(lang.map(node.kind_id()));
        d.field.push(cursor.field_id().map_or(0, |f| f.get()));
        d.named.push(node.is_named());
        d.parent.push(stack.last().copied().unwrap_or(NONE));
        d.depth.push(stack.len() as u16);
        d.end.push(0);
        d.sb.push(node.start_byte() as u32);
        d.eb.push(node.end_byte() as u32);
        if cursor.goto_first_child() {
            stack.push(id);
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
            d.end[p as usize] = d.kind.len() as u32;
        }
    }
    let n = d.len();
    d.nth = vec![0; n];
    d.nthr = vec![0; n];
    let mut kids = vec![];
    for p in 0..n {
        kids.clear();
        let mut j = p as u32 + 1;
        while j < d.end[p] {
            if d.named[j as usize] {
                kids.push(j);
            }
            j = d.end[j as usize];
        }
        let k = kids.len() as u32;
        for (i, &c) in kids.iter().enumerate() {
            d.nth[c as usize] = i as u32 + 1;
            d.nthr[c as usize] = k - i as u32;
        }
    }
    d
}

// ---------- patterns ----------
struct PNode {
    kind: u16,
    meta: u8, // 0 none, 1 single metavariable, 2 multi ($$$)
    leaf: Option<String>,
    kids: Vec<PNode>,
}

fn pnode(lang: &Lang, src: &str, n: tree_sitter::Node) -> PNode {
    let text = &src[n.start_byte()..n.end_byte()];
    if n.kind() == "identifier" && text.starts_with('µ') {
        return PNode { kind: 0, meta: if text.starts_with("µµµ") { 2 } else { 1 }, leaf: None, kids: vec![] };
    }
    let mut kids = vec![];
    let mut c = n.walk();
    for k in n.named_children(&mut c) {
        if k.kind() != "comment" {
            kids.push(pnode(lang, src, k));
        }
    }
    let leaf = if kids.is_empty() { Some(text.to_string()) } else { None };
    PNode { kind: lang.map(n.kind_id()), meta: 0, leaf, kids }
}

fn compile_pattern(lang: &Lang, p: &Value) -> Option<PNode> {
    let (ctx, sel) = match p {
        Value::String(s) => (s.clone(), None),
        Value::Object(o) => (o.get("context").and_then(|v| v.as_str()).unwrap_or("").to_string(), o.get("selector").and_then(|v| v.as_str()).map(String::from)),
        _ => return None,
    };
    let src = ctx.replace('$', "µ");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang.ts).unwrap();
    let tree = parser.parse(&src, None)?;
    let mut node = tree.root_node();
    if let Some(sel) = sel {
        let mut stack = vec![node];
        let mut found = None;
        while let Some(x) = stack.pop() {
            if x.kind() == sel {
                found = Some(x);
                break;
            }
            let mut c = x.walk();
            let ks: Vec<_> = x.children(&mut c).collect();
            for k in ks.into_iter().rev() {
                stack.push(k);
            }
        }
        node = found?;
    } else {
        while (node.kind() == "module" || node.kind() == "expression_statement") && node.named_child_count() == 1 {
            node = node.named_child(0).unwrap();
        }
    }
    Some(pnode(lang, &src, node))
}

fn pmatch(p: &PNode, d: &Doc, n: u32, buf: &mut Vec<u32>) -> bool {
    if p.meta == 1 {
        return true;
    }
    if p.kind != d.kind[n as usize] {
        return false;
    }
    let start = buf.len();
    let mut j = n + 1;
    while j < d.end[n as usize] {
        if d.named[j as usize] && d.kind[j as usize] != COMMENT.with(|c| c.get()) {
            buf.push(j);
        }
        j = d.end[j as usize];
    }
    let r = if p.kids.is_empty() {
        buf.len() == start && p.leaf.as_deref().map_or(true, |t| d.text(n) == t)
    } else {
        let ts: Vec<u32> = buf[start..].to_vec();
        seq_match(&p.kids, &ts, d, buf)
    };
    buf.truncate(start);
    r
}

fn seq_match(ps: &[PNode], ts: &[u32], d: &Doc, buf: &mut Vec<u32>) -> bool {
    if ps.is_empty() {
        return ts.is_empty();
    }
    if ps[0].meta == 2 {
        return (0..=ts.len()).any(|k| seq_match(&ps[1..], &ts[k..], d, buf));
    }
    !ts.is_empty() && pmatch(&ps[0], d, ts[0], buf) && seq_match(&ps[1..], &ts[1..], d, buf)
}

thread_local!(static COMMENT: std::cell::Cell<u16> = const { std::cell::Cell::new(NOKIND) });

// ---------- regex ----------
enum Re {
    R(regex::Regex),
    F(fancy_regex::Regex),
    Never,
}
impl Re {
    fn new(p: &str) -> Re {
        match regex::Regex::new(p) {
            Ok(r) => Re::R(r),
            Err(_) => fancy_regex::Regex::new(p).map(Re::F).unwrap_or(Re::Never),
        }
    }
    fn is_match(&self, s: &str) -> bool {
        match self {
            Re::R(r) => r.is_match(s),
            Re::F(r) => r.is_match(s).unwrap_or(false),
            Re::Never => false,
        }
    }
}

// ---------- rule DAG ----------
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
enum Op {
    True,
    False,
    Kind(u16),
    Regex(u32),
    Pattern(u32),
    FilePath(u32),
    Nth { pos: u32, rev: bool, of: u32 },
    All(Vec<u32>),
    Any(Vec<u32>),
    Not(u32),
    Rel { axis: u8, stop: u8, stopr: u32, field: u16, inner: u32 },
    Util(u32),
}

const HAS: u8 = 0;
const INSIDE: u8 = 1;
const PRECEDES: u8 = 2;
const FOLLOWS: u8 = 3;

#[derive(Default)]
struct Dag {
    ops: Vec<Op>,
    map: HashMap<Op, u32>,
    util_body: Vec<u32>,
    util_slot: HashMap<String, u32>,
    regex: Vec<Re>,
    regex_map: HashMap<String, u32>,
    pats: Vec<PNode>,
    pat_map: HashMap<String, Option<u32>>,
    refs: Vec<u32>,
    expansions: u64,
    stats: HashMap<&'static str, u64>,
}

impl Dag {
    fn intern(&mut self, op: Op) -> u32 {
        self.expansions += 1;
        if let Some(&id) = self.map.get(&op) {
            return id;
        }
        let id = self.ops.len() as u32;
        let kids: Vec<u32> = match &op {
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
            _ => vec![],
        };
        for k in kids {
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
        self.regex_map.insert(p.to_string(), r);
        r
    }

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
                    let key = v.to_string();
                    let pid = if let Some(p) = self.pat_map.get(&key) {
                        *p
                    } else {
                        let p = compile_pattern(lang, v).and_then(|p| {
                            if p.meta == 1 {
                                None
                            } else {
                                self.pats.push(p);
                                Some(self.pats.len() as u32 - 1)
                            }
                        });
                        self.pat_map.insert(key, p);
                        p
                    };
                    pid.map(Op::Pattern)
                }
                "regex" => v.as_str().map(|s| Op::Regex(self.regex(s))),
                "value" => v.get("regex").and_then(|s| s.as_str()).map(|s| Op::Regex(self.regex(s))),
                "filePath" => v.get("regex").and_then(|s| s.as_str()).map(|s| Op::FilePath(self.regex(s))),
                "nthChild" => match v {
                    Value::Number(n) => Some(Op::Nth { pos: n.as_u64().unwrap_or(1) as u32, rev: false, of: NONE }),
                    Value::Object(no) => no.get("position").and_then(|p| p.as_u64()).map(|pos| {
                        let of = no.get("ofRule").map_or(NONE, |r| self.compile(lang, r, env, stack));
                        Op::Nth { pos: pos as u32, rev: no.get("reverse").and_then(|b| b.as_bool()).unwrap_or(false), of }
                    }),
                    _ => None,
                },
                "all" | "any" => {
                    let parts: Vec<u32> = v.as_array().map(|a| a.iter().map(|x| self.compile(lang, x, env, stack)).collect()).unwrap_or_default();
                    let t = self.intern(Op::True);
                    let f = self.intern(Op::False);
                    if k == "all" {
                        let p: Vec<u32> = parts.into_iter().filter(|&x| x != t).collect();
                        if p.contains(&f) {
                            Some(Op::False)
                        } else if p.is_empty() {
                            None
                        } else if p.len() == 1 {
                            conj.push(p[0]);
                            None
                        } else {
                            Some(Op::All(p))
                        }
                    } else {
                        let p: Vec<u32> = parts.into_iter().filter(|&x| x != f).collect();
                        if p.contains(&t) {
                            None
                        } else if p.is_empty() {
                            Some(Op::False)
                        } else if p.len() == 1 {
                            conj.push(p[0]);
                            None
                        } else {
                            Some(Op::Any(p))
                        }
                    }
                }
                "not" => {
                    let x = self.compile(lang, v, env, stack);
                    Some(Op::Not(x))
                }
                "has" | "inside" | "precedes" | "follows" => {
                    let axis = match k.as_str() {
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
                        Some(Value::String(_)) => (1, NONE),
                        Some(sr) => (2, self.compile(lang, sr, env, stack)),
                    };
                    let inner = self.compile(lang, &Value::Object(body), env, stack);
                    *self.stats.entry(match (axis, stop) {
                        (HAS, 0) => "has.neighbor",
                        (HAS, 1) => "has.end",
                        (HAS, _) => "has.rule",
                        (INSIDE, 0) => "inside.neighbor",
                        (INSIDE, 1) => "inside.end",
                        (INSIDE, _) => "inside.rule",
                        (_, 0) => "sibling.neighbor",
                        _ => "sibling.range",
                    }).or_default() += 1;
                    Some(Op::Rel { axis, stop, stopr, field, inner })
                }
                "matches" => {
                    let name = v.as_str().map(String::from).unwrap_or_default();
                    if let Some(&(_, slot)) = stack.iter().find(|(n, _)| *n == name) {
                        *self.stats.entry("util.recursive_ref").or_default() += 1;
                        Some(Op::Util(slot))
                    } else if let Some(body) = env.get(&name) {
                        *self.stats.entry("util.inline").or_default() += 1;
                        let key = format!("{name}\u{0}{body}");
                        let slot = *self.util_slot.entry(key).or_insert_with(|| {
                            self.util_body.push(NONE);
                            self.util_body.len() as u32 - 1
                        });
                        stack.push((name, slot));
                        let id = self.compile(lang, body, env, stack);
                        stack.pop();
                        self.util_body[slot as usize] = id;
                        conj.push(id);
                        None
                    } else {
                        None
                    }
                }
                _ => None, // stopBy/field handled by the relation; semantic keys evaluate to true
            };
            if let Some(op) = c {
                let id = self.intern(op);
                conj.push(id);
            }
        }
        let t = self.intern(Op::True);
        conj.retain(|&x| x != t);
        match conj.len() {
            0 => t,
            1 => conj[0],
            _ => self.intern(Op::All(conj)),
        }
    }

    /// Root kinds of a match (None: any node).
    fn root_kinds(&self, id: u32, memo: &mut HashMap<u32, Option<Vec<u16>>>) -> Option<Vec<u16>> {
        if let Some(v) = memo.get(&id) {
            return v.clone();
        }
        let r = match &self.ops[id as usize] {
            Op::Kind(k) => Some(vec![*k]),
            Op::Pattern(p) => Some(vec![self.pats[*p as usize].kind]),
            Op::False => Some(vec![]),
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
                let mut any = false;
                for &x in v {
                    match self.root_kinds(x, memo) {
                        None => {
                            any = true;
                            break;
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
                if any {
                    None
                } else {
                    Some(out)
                }
            }
            _ => None,
        };
        memo.insert(id, r.clone());
        r
    }
}

// ---------- evaluator ----------
struct Ev<'a> {
    g: &'a Dag,
    d: &'a Doc,
    memo_on: &'a [bool],
    mask_on: &'a [bool],
    rk: &'a [Option<Vec<bool>>],
    memo: Vec<Vec<u8>>,
    cols: Vec<Vec<u8>>,
    touched: Vec<u32>,
    fp: Vec<i8>,
    fp_touched: Vec<u32>,
    buf: Vec<u32>,
    depth: u32,
    steps: u64,
    mask_nodes: u64,
    re_calls: u64,
    re_bytes: u64,
    pat_calls: u64,
    ctr: [u64; NC],
}

const NC: usize = 23;
const CNAMES: [&str; NC] = ["const", "kind", "regex", "pattern", "filepath", "nth", "all", "any", "not", "has_nb", "has_end", "has_rule", "inside", "sibling", "util", "mask_lookup", "memo_hit", "scan_visit", "regex_bytes", "mask_nodes", "cand_scan", "dispatch", "memo_alloc_bytes"];

impl<'a> Ev<'a> {
    fn new(g: &'a Dag, memo_on: &'a [bool], mask_on: &'a [bool], rk: &'a [Option<Vec<bool>>], d: &'a Doc) -> Ev<'a> {
        Ev { g, d, memo_on, mask_on, rk, memo: vec![vec![]; g.ops.len()], cols: vec![vec![]; g.ops.len()], touched: vec![], fp: vec![-1; g.regex.len()], fp_touched: vec![], buf: vec![], depth: 0, steps: 0, mask_nodes: 0, re_calls: 0, re_bytes: 0, pat_calls: 0, ctr: [0; NC] }
    }
    fn reset(&mut self, d: &'a Doc) {
        for &id in &self.touched {
            self.memo[id as usize].clear();
            self.cols[id as usize].clear();
        }
        self.touched.clear();
        for &r in &self.fp_touched {
            self.fp[r as usize] = -1;
        }
        self.fp_touched.clear();
        self.d = d;
    }
    #[inline]
    fn kind_ok(&self, id: u32, n: u32) -> bool {
        match &self.rk[id as usize] {
            None => true,
            Some(s) => s[self.d.kind[n as usize] as usize],
        }
    }
    fn eval(&mut self, id: u32, n: u32) -> bool {
        if self.memo_on[id as usize] {
            let len = self.d.len();
            let m = &mut self.memo[id as usize];
            if m.is_empty() {
                m.resize(len, 0);
                self.ctr[22] += len as u64;
                self.touched.push(id);
            }
            let v = m[n as usize];
            if v != 0 {
                self.ctr[16] += 1;
                return v == 2;
            }
            let r = self.eval_raw(id, n);
            self.memo[id as usize][n as usize] = if r { 2 } else { 1 };
            return r;
        }
        self.eval_raw(id, n)
    }
    fn fok(&self, f: u16, n: u32) -> bool {
        f == 0 || self.d.field[n as usize] == f
    }
    fn eval_raw(&mut self, id: u32, n: u32) -> bool {
        self.steps += 1;
        let g = self.g;
        let d = self.d;
        let class = match &g.ops[id as usize] {
            Op::True | Op::False => 0,
            Op::Kind(_) => 1,
            Op::Regex(_) => 2,
            Op::Pattern(_) => 3,
            Op::FilePath(_) => 4,
            Op::Nth { .. } => 5,
            Op::All(_) => 6,
            Op::Any(_) => 7,
            Op::Not(_) => 8,
            Op::Rel { axis: HAS, stop: 0, .. } => 9,
            Op::Rel { axis: HAS, stop: 1, .. } => 10,
            Op::Rel { axis: HAS, .. } => 11,
            Op::Rel { axis: INSIDE, .. } => 12,
            Op::Rel { .. } => 13,
            Op::Util(_) => 14,
        };
        self.ctr[class] += 1;
        match &g.ops[id as usize] {
            Op::True => true,
            Op::False => false,
            Op::Kind(k) => d.kind[n as usize] == *k,
            Op::Regex(r) => {
                self.re_calls += 1;
                let t = d.text(n);
                self.re_bytes += t.len() as u64;
                self.ctr[18] += t.len() as u64;
                g.regex[*r as usize].is_match(t)
            }
            Op::Pattern(p) => {
                self.pat_calls += 1;
                let mut buf = std::mem::take(&mut self.buf);
                let r = pmatch(&g.pats[*p as usize], d, n, &mut buf);
                self.buf = buf;
                r
            }
            Op::FilePath(r) => {
                if self.fp[*r as usize] < 0 {
                    self.fp[*r as usize] = g.regex[*r as usize].is_match(&d.path) as i8;
                    self.fp_touched.push(*r);
                }
                self.fp[*r as usize] == 1
            }
            Op::Nth { pos, rev, of } => {
                if !d.named[n as usize] {
                    return false;
                }
                if *of == NONE {
                    return if *rev { d.nthr[n as usize] } else { d.nth[n as usize] } == *pos;
                }
                if !self.eval(*of, n) {
                    return false;
                }
                let p = d.parent[n as usize];
                if p == NONE {
                    return *pos == 1;
                }
                let mut idx = 1;
                let mut j = p + 1;
                let pe = d.end[p as usize];
                while j < pe {
                    if j != n && ((!*rev && j < n) || (*rev && j > n)) && d.named[j as usize] && self.eval(*of, j) {
                        idx += 1;
                    }
                    j = d.end[j as usize];
                }
                idx == *pos
            }
            Op::All(v) => v.iter().all(|&x| self.eval(x, n)),
            Op::Any(v) => v.iter().any(|&x| self.eval(x, n)),
            Op::Not(x) => !self.eval(*x, n),
            Op::Util(s) => {
                if self.depth > 256 {
                    return false;
                }
                self.depth += 1;
                let r = self.eval(g.util_body[*s as usize], n);
                self.depth -= 1;
                r
            }
            Op::Rel { axis, stop, stopr, field, inner } => {
                if self.mask_on[id as usize] {
                    if self.cols[id as usize].is_empty() {
                        self.build_col(id);
                    }
                    self.ctr[15] += 1;
                    return self.cols[id as usize][n as usize] != 0;
                }
                let (axis, stop, stopr, f, inner) = (*axis, *stop, *stopr, *field, *inner);
                let hit = |s: &mut Self, j: u32| {
                    s.ctr[17] += 1;
                    s.fok(f, j) && s.kind_ok(inner, j) && s.eval(inner, j)
                };
                match axis {
                    HAS => match stop {
                        0 => {
                            let mut j = n + 1;
                            while j < d.end[n as usize] {
                                if hit(self, j) {
                                    return true;
                                }
                                j = d.end[j as usize];
                            }
                            false
                        }
                        1 => (n + 1..d.end[n as usize]).any(|j| hit(self, j)),
                        _ => {
                            let mut j = n + 1;
                            let e = d.end[n as usize];
                            while j < e {
                                if hit(self, j) {
                                    return true;
                                }
                                if self.kind_ok(stopr, j) && self.eval(stopr, j) {
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
                            if self.fok(f, cur) && self.kind_ok(inner, p) && self.eval(inner, p) {
                                return true;
                            }
                            if stop == 0 || (stop == 2 && self.kind_ok(stopr, p) && self.eval(stopr, p)) {
                                return false;
                            }
                            cur = p;
                        }
                    }
                    _ => {
                        let p = d.parent[n as usize];
                        if p == NONE {
                            return false;
                        }
                        // siblings of n, nearest first
                        let mut sib: Vec<u32> = vec![];
                        let mut j = p + 1;
                        while j < d.end[p as usize] {
                            sib.push(j);
                            j = d.end[j as usize];
                        }
                        let i = sib.iter().position(|&x| x == n).unwrap();
                        let order: Vec<u32> = if axis == PRECEDES { sib[i + 1..].to_vec() } else { sib[..i].iter().rev().copied().collect() };
                        for s in order {
                            if hit(self, s) {
                                return true;
                            }
                            if stop == 0 || (stop == 2 && self.kind_ok(stopr, s) && self.eval(stopr, s)) {
                                return false;
                            }
                        }
                        false
                    }
                }
            }
        }
    }

    fn column(&mut self, id: u32) -> Vec<bool> {
        let n = self.d.len();
        (0..n as u32).map(|j| self.kind_ok(id, j) && self.eval(id, j)).collect()
    }

    fn build_col(&mut self, id: u32) {
        let (axis, stop, stopr, f, inner) = match &self.g.ops[id as usize] {
            Op::Rel { axis, stop, stopr, field, inner } => (*axis, *stop, *stopr, *field, *inner),
            _ => unreachable!(),
        };
        let d = self.d;
        let n = d.len();
        self.mask_nodes += n as u64;
        self.ctr[19] += n as u64;
        let ic = self.column(inner);
        let sc = if stop == 2 { self.column(stopr) } else { vec![] };
        let h = |j: usize| (f == 0 || d.field[j] == f) && ic[j];
        let mut col = vec![0u8; n];
        match axis {
            HAS => {
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
                let mut kids = vec![];
                for p in 0..n {
                    kids.clear();
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
        self.cols[id as usize] = col;
        self.touched.push(id);
    }
}

// ---------- queries ----------
struct Query {
    rule: String,
    clause: String,
    role: String,
    root: u32,
    kinds: Option<Vec<bool>>,
    files: Vec<bool>,
}

fn glob(gs: &Value) -> globset::GlobSet {
    let mut b = globset::GlobSetBuilder::new();
    for g in gs.as_array().into_iter().flatten() {
        if let Some(s) = g.as_str() {
            if let Ok(gl) = globset::GlobBuilder::new(s).literal_separator(true).build() {
                b.add(gl);
            }
        }
    }
    b.build().unwrap()
}

fn mix(a: u64, b: u64, c: u64) -> u64 {
    let mut x = a.wrapping_mul(0x9E3779B97F4A7C15) ^ b.wrapping_mul(0xC2B2AE3D27D4EB4F) ^ c.wrapping_mul(0x165667B19E3779F9);
    x ^= x >> 31;
    x = x.wrapping_mul(0xD6E8FEB86659FD93);
    x ^ (x >> 32)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (qpath, tpath, root, files_txt, out) = (&a[1], &a[2], &a[3], &a[4], &a[5]);
    let modes: Vec<String> = a.get(6).map_or("sep,fused,memo,mask,maskall".into(), |s| s.clone()).split(',').map(String::from).collect();
    let reps: usize = a.get(7).map_or(1, |s| s.parse().unwrap());
    let lang = Lang::new();
    COMMENT.with(|c| c.set(lang.kind("comment")));

    // corpus
    let t0 = Instant::now();
    let paths: Vec<String> = std::fs::read_to_string(files_txt).unwrap().lines().filter(|l| l.ends_with(".py")).map(String::from).collect();
    let parsed: Vec<(Doc, f64)> = paths
        .par_iter()
        .filter_map(|p| {
            let bytes = std::fs::read(format!("{root}/{p}")).ok()?;
            let src = String::from_utf8_lossy(&bytes).into_owned();
            let t = Instant::now();
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&lang.ts).unwrap();
            let tree = parser.parse(&src, None)?;
            let tp = t.elapsed().as_secs_f64();
            let tb = Instant::now();
            let mut d = build_doc(&lang, p.clone(), src, &tree);
            d.parse_s = tp + tb.elapsed().as_secs_f64();
            let ps = d.parse_s;
            Some((d, ps))
        })
        .collect();
    let load_wall = t0.elapsed().as_secs_f64();
    let parse_cpu: f64 = parsed.iter().map(|x| x.1).sum();
    let docs: Vec<Doc> = parsed.into_iter().map(|x| x.0).collect();
    let nodes: usize = docs.iter().map(|d| d.len()).sum();
    let bytes: usize = docs.iter().map(|d| d.src.len()).sum();
    eprintln!("corpus: {} files, {} nodes, {} bytes, load wall {:.1}s parse+build cpu {:.1}s", docs.len(), nodes, bytes, load_wall, parse_cpu);

    // corpus statistics
    let nk = lang.names.len();
    let mut kc = vec![0u64; nk];
    let mut sub: Vec<Vec<u32>> = vec![vec![]; nk];
    let mut named = 0u64;
    let mut maxd = 0u16;
    let mut sumd = 0u64;
    let mut edges: HashMap<(u16, u16, u16), u64> = HashMap::new();
    let mut fsizes: Vec<usize> = vec![];
    for d in &docs {
        fsizes.push(d.len());
        for i in 0..d.len() {
            let k = d.kind[i] as usize;
            kc[k] += 1;
            if d.named[i] {
                named += 1;
                sub[k].push(d.end[i] - i as u32);
                if d.parent[i] != NONE {
                    *edges.entry((d.kind[d.parent[i] as usize], d.field[i], d.kind[i])).or_default() += 1;
                }
            }
            maxd = maxd.max(d.depth[i]);
            sumd += d.depth[i] as u64;
        }
    }
    let pct = |v: &mut Vec<u32>, q: f64| -> u32 {
        if v.is_empty() {
            return 0;
        }
        v.sort_unstable();
        v[((v.len() - 1) as f64 * q) as usize]
    };
    let mut kinds_json = vec![];
    let mut order: Vec<usize> = (0..nk).filter(|&k| kc[k] > 0).collect();
    order.sort_by_key(|&k| std::cmp::Reverse(kc[k]));
    for &k in &order {
        let v = &mut sub[k];
        if v.is_empty() {
            continue;
        }
        let mean = v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64;
        let (p50, p90, p99, mx) = (pct(v, 0.5), pct(v, 0.9), pct(v, 0.99), *v.iter().max().unwrap());
        kinds_json.push(json!({"kind": lang.names[k], "count": kc[k], "subtree_mean": mean, "p50": p50, "p90": p90, "p99": p99, "max": mx}));
    }
    fsizes.sort_unstable();
    let fpct = |q: f64| fsizes[((fsizes.len() - 1) as f64 * q) as usize];

    // compile queries
    let qs: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(qpath).unwrap()).unwrap();
    let mut g = Dag::default();
    let mut queries: Vec<Query> = vec![];
    let mut rkmemo = HashMap::new();
    let tc = Instant::now();
    for q in &qs {
        if !q["active"].as_bool().unwrap_or(false) || q["query"]["language"] != "python" {
            continue;
        }
        let env = q["utils"].as_object().cloned().unwrap_or_default();
        let mut stack = vec![];
        let root_id = g.compile(&lang, &q["query"]["rule"], &env, &mut stack);
        let inc = glob(&q["include"]);
        let exc = glob(&q["exclude"]);
        let notre = q["notRegex"].as_str().map(Re::new);
        let yesre = q["pathRegex"].as_str().map(Re::new);
        let files = docs
            .iter()
            .map(|d| inc.is_match(&d.path) && !exc.is_match(&d.path) && notre.as_ref().map_or(true, |r| !r.is_match(&d.path)) && yesre.as_ref().map_or(true, |r| r.is_match(&d.path)))
            .collect();
        queries.push(Query { rule: q["rule"].as_str().unwrap().into(), clause: q["clause"].as_str().unwrap_or("").into(), role: q["role"].as_str().unwrap_or("").into(), root: root_id, kinds: None, files });
    }
    let compile_s = tc.elapsed().as_secs_f64();
    let ops = g.ops.len();
    let mut rk: Vec<Option<Vec<bool>>> = vec![None; ops];
    for id in 0..ops as u32 {
        rk[id as usize] = g.root_kinds(id, &mut rkmemo).map(|ks| {
            let mut b = vec![false; nk.max(NOKIND as usize + 1)];
            for k in ks {
                b[k as usize] = true;
            }
            b
        });
    }
    for q in queries.iter_mut() {
        q.kinds = rk[q.root as usize].clone();
    }
    // reaches_util: ids whose evaluation can re-enter a recursive utility (not stratifiable)
    let mut reach = vec![false; ops];
    for id in 0..ops {
        reach[id] = match &g.ops[id] {
            Op::Util(_) => true,
            Op::All(v) | Op::Any(v) => v.iter().any(|&x| reach[x as usize]),
            Op::Not(x) => reach[*x as usize],
            Op::Nth { of, .. } => *of != NONE && reach[*of as usize],
            Op::Rel { stopr, inner, .. } => reach[*inner as usize] || (*stopr != NONE && reach[*stopr as usize]),
            _ => false,
        };
    }
    let rels: Vec<usize> = (0..ops).filter(|&i| matches!(g.ops[i], Op::Rel { .. })).collect();
    let rel_inners: std::collections::HashSet<u32> = rels.iter().map(|&i| if let Op::Rel { inner, .. } = g.ops[i] { inner } else { 0 }).collect();
    let mut memo_on = vec![false; ops];
    for id in 0..ops {
        let cheap = matches!(g.ops[id], Op::True | Op::False | Op::Kind(_) | Op::FilePath(_));
        memo_on[id] = !cheap && (g.refs[id] >= 2 || rel_inners.contains(&(id as u32)) || g.util_body.contains(&(id as u32)));
    }
    let no = vec![false; ops];
    let mut mask_scan = vec![false; ops];
    let mut mask_all = vec![false; ops];
    for &i in &rels {
        if let Op::Rel { stop, .. } = g.ops[i] {
            if !reach[i] {
                mask_all[i] = true;
                mask_scan[i] = stop != 0;
            }
        }
    }
    // fusion statistics
    let mut by_kind: HashMap<u16, usize> = HashMap::new();
    let mut unconstrained = 0;
    for q in &queries {
        match &q.kinds {
            None => unconstrained += 1,
            Some(b) => {
                for (k, &on) in b.iter().enumerate() {
                    if on {
                        *by_kind.entry(k as u16).or_default() += 1;
                    }
                }
            }
        }
    }
    let mut groups: Vec<(String, usize)> = by_kind.iter().map(|(&k, &c)| (lang.names.get(k as usize).cloned().unwrap_or("?".into()), c)).collect();
    groups.sort_by_key(|x| std::cmp::Reverse(x.1));
    // strata: longest chain of Rel nesting through DAG
    let mut stratum = vec![0u32; ops];
    for id in 0..ops {
        let kids: Vec<u32> = match &g.ops[id] {
            Op::All(v) | Op::Any(v) => v.clone(),
            Op::Not(x) => vec![*x],
            Op::Nth { of, .. } if *of != NONE => vec![*of],
            Op::Rel { stopr, inner, .. } => {
                let mut v = vec![*inner];
                if *stopr != NONE {
                    v.push(*stopr)
                }
                v
            }
            _ => vec![],
        };
        let m = kids.iter().map(|&k| stratum[k as usize]).max().unwrap_or(0);
        stratum[id] = m + matches!(g.ops[id], Op::Rel { .. }) as u32;
    }
    let max_stratum = queries.iter().map(|q| stratum[q.root as usize]).max().unwrap_or(0);
    let mut strata_hist: HashMap<u32, usize> = HashMap::new();
    for q in &queries {
        *strata_hist.entry(stratum[q.root as usize]).or_default() += 1;
    }
    let mut per_stratum_targets: HashMap<u32, usize> = HashMap::new();
    for &i in &rels {
        if !reach[i] {
            *per_stratum_targets.entry(stratum[i]).or_default() += 1;
        }
    }
    let shared_rel = rels.iter().filter(|&&i| g.refs[i] >= 2).count();
    eprintln!("compiled {} queries in {:.2}s: {} DAG ops ({} expansions), {} relations ({} shared, {} recursive-dependent), max stratum {}", queries.len(), compile_s, ops, g.expansions, rels.len(), shared_rel, rels.iter().filter(|&&i| reach[i]).count(), max_stratum);

    // candidates per query
    let mut cand = vec![0u64; queries.len()];
    let mut infiles = vec![0u64; queries.len()];
    for (qi, q) in queries.iter().enumerate() {
        for (fi, d) in docs.iter().enumerate() {
            if !q.files[fi] {
                continue;
            }
            infiles[qi] += 1;
            cand[qi] += match &q.kinds {
                None => d.len() as u64,
                Some(b) => d.kind.iter().filter(|&&k| b[k as usize]).count() as u64,
            };
        }
    }

    // dispatch table: kind -> queries (plus unconstrained)
    let mut table: Vec<Vec<u32>> = vec![vec![]; nk.max(NOKIND as usize + 1)];
    let mut anyq: Vec<u32> = vec![];
    for (qi, q) in queries.iter().enumerate() {
        match &q.kinds {
            None => anyq.push(qi as u32),
            Some(b) => {
                for (k, &on) in b.iter().enumerate() {
                    if on {
                        table[k].push(qi as u32)
                    }
                }
            }
        }
    }

    let mut results = vec![];
    let mut per_query_sep: Vec<f64> = vec![0.0; queries.len()];
    let mut per_query_matches = vec![0u64; queries.len()];
    let mut per_file: HashMap<String, Vec<f64>> = HashMap::new();
    let mut per_file_c: HashMap<String, Vec<[u64; NC]>> = HashMap::new();
    let mut reference: Option<(u64, u64)> = None;
    for mode in &modes {
        let (memo, mask): (&[bool], &[bool]) = match mode.as_str() {
            "sep" | "fused" => (&no, &no),
            "memo" => (&memo_on, &no),
            "mask" => (&memo_on, &mask_scan),
            "maskall" => (&memo_on, &mask_all),
            _ => panic!("mode {mode}"),
        };
        let mut best = f64::MAX;
        let mut summary = (0u64, 0u64);
        let mut steps = 0u64;
        let mut mask_nodes = 0u64;
        let mut ctr = (0u64, 0u64, 0u64);
        let mut totals = [0u64; NC];
        let mut file_t = vec![0.0f64; docs.len()];
        let mut file_c = vec![[0u64; NC]; docs.len()];
        for _ in 0..reps {
            let mut count = 0u64;
            let mut hash = 0u64;
            let mut ev = Ev::new(&g, memo, mask, &rk, &docs[0]);
            file_t.iter_mut().for_each(|x| *x = 0.0);
            file_c.iter_mut().for_each(|x| *x = [0; NC]);
            let t = Instant::now();
            if mode == "sep" {
                for (qi, q) in queries.iter().enumerate() {
                    let tq = Instant::now();
                    let mut m = 0;
                    for (fi, d) in docs.iter().enumerate() {
                        if !q.files[fi] {
                            continue;
                        }
                        let tf = Instant::now();
                        let c0 = ev.ctr;
                        ev.reset(d);
                        ev.ctr[20] += d.len() as u64;
                        for i in 0..d.len() as u32 {
                            if q.kinds.as_ref().map_or(true, |b| b[d.kind[i as usize] as usize]) && ev.eval(q.root, i) {
                                count += 1;
                                m += 1;
                                hash = hash.wrapping_add(mix(qi as u64, fi as u64, i as u64));
                            }
                        }
                        file_t[fi] += tf.elapsed().as_secs_f64();
                        for c in 0..NC {
                            file_c[fi][c] += ev.ctr[c] - c0[c];
                        }
                    }
                    per_query_sep[qi] = tq.elapsed().as_secs_f64();
                    per_query_matches[qi] = m;
                }
            } else {
                for (fi, d) in docs.iter().enumerate() {
                    let tf = Instant::now();
                    let c0 = ev.ctr;
                    ev.reset(d);
                    ev.ctr[20] += d.len() as u64;
                    for i in 0..d.len() as u32 {
                        let k = d.kind[i as usize] as usize;
                        ev.ctr[21] += (table[k].len() + anyq.len()) as u64;
                        for &qi in table[k].iter().chain(anyq.iter()) {
                            let q = &queries[qi as usize];
                            if q.files[fi] && ev.eval(q.root, i) {
                                count += 1;
                                hash = hash.wrapping_add(mix(qi as u64, fi as u64, i as u64));
                            }
                        }
                    }
                    file_t[fi] = tf.elapsed().as_secs_f64();
                    for c in 0..NC {
                        file_c[fi][c] = ev.ctr[c] - c0[c];
                    }
                }
            }
            let el = t.elapsed().as_secs_f64();
            best = best.min(el);
            summary = (count, hash);
            steps = ev.steps;
            mask_nodes = ev.mask_nodes;
            ctr = (ev.re_calls, ev.re_bytes, ev.pat_calls);
            totals = ev.ctr;
        }
        match reference {
            None => reference = Some(summary),
            Some(r) => assert_eq!(r, summary, "mode {mode} finding set differs"),
        }
        let mut ft = file_t.clone();
        ft.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let fq = |q: f64| ft[((ft.len() - 1) as f64 * q) as usize] * 1e3;
        eprintln!("{mode:8} {best:8.2}s  {:7.1} ns/node  findings {} steps {} mask_nodes {} re {}/{}MB pat {}  per-file ms p50 {:.3} p90 {:.3} p99 {:.3} max {:.1}", best * 1e9 / nodes as f64, summary.0, steps, mask_nodes, ctr.0, ctr.1 / 1000000, ctr.2, fq(0.5), fq(0.9), fq(0.99), fq(1.0));
        results.push(json!({"mode": mode, "seconds": best, "ns_per_node": best * 1e9 / nodes as f64, "findings": summary.0, "steps": steps, "mask_nodes": mask_nodes, "counters": CNAMES.iter().zip(totals.iter()).map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(), "regex_calls": ctr.0, "regex_bytes": ctr.1, "pattern_calls": ctr.2,
            "file_ms": {"p50": fq(0.5), "p90": fq(0.9), "p99": fq(0.99), "max": fq(1.0), "mean": ft.iter().sum::<f64>() * 1e3 / ft.len() as f64}}));
        per_file.insert(mode.clone(), file_t);
        per_file_c.insert(mode.clone(), file_c);
    }

    // file-content regex clauses
    let tr: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(tpath).unwrap()).unwrap();
    let tt = Instant::now();
    let mut text_hits = 0u64;
    let mut text_pairs = 0u64;
    let mut text_bytes = 0u64;
    for c in tr.iter().filter(|c| c["active"].as_bool().unwrap_or(false)) {
        let re = Re::new(c["pattern"].as_str().unwrap_or(""));
        let inc = glob(&c["include"]);
        let exc = glob(&c["exclude"]);
        for d in &docs {
            if inc.is_match(&d.path) && !exc.is_match(&d.path) {
                text_pairs += 1;
                text_bytes += d.src.len() as u64;
                if let Re::R(r) = &re {
                    text_hits += r.find_iter(&d.src).count() as u64;
                } else {
                    text_hits += re.is_match(&d.src) as u64;
                }
            }
        }
    }
    let text_s = tt.elapsed().as_secs_f64();
    eprintln!("text regex: {} clause×file pairs, {:.1} MB scanned, {:.2}s, {} hits", text_pairs, text_bytes as f64 / 1e6, text_s, text_hits);

    let qjson: Vec<Value> = queries
        .iter()
        .enumerate()
        .map(|(i, q)| {
            json!({"rule": q.rule, "clause": q.clause, "role": q.role, "root": q.root, "stratum": stratum[q.root as usize],
                "root_kinds": q.kinds.as_ref().map(|b| b.iter().enumerate().filter(|x| *x.1).map(|(k, _)| lang.names.get(k).cloned().unwrap_or("?".into())).collect::<Vec<_>>()),
                "files": infiles[i], "candidates": cand[i], "sep_seconds": per_query_sep[i], "matches": per_query_matches[i]})
        })
        .collect();
    // per-file cost vs size, for the per-edit model
    let pf: Vec<Value> = docs
        .iter()
        .enumerate()
        .map(|(fi, d)| {
            let mut o = serde_json::Map::new();
            o.insert("nodes".into(), json!(d.len()));
            o.insert("path".into(), json!(d.path));
            o.insert("bytes".into(), json!(d.src.len()));
            o.insert("parse_ms".into(), json!(d.parse_s * 1e3));
            for (m, v) in &per_file {
                o.insert(m.clone(), json!(v[fi] * 1e3));
                o.insert(format!("{m}_c"), json!(per_file_c[m][fi].to_vec()));
            }
            Value::Object(o)
        })
        .collect();
    let report = json!({
        "corpus": {"files": docs.len(), "nodes": nodes, "named": named, "bytes": bytes, "load_wall_s": load_wall, "parse_build_cpu_s": parse_cpu,
            "max_depth": maxd, "mean_depth": sumd as f64 / nodes as f64, "distinct_edges": edges.len(),
            "file_nodes": {"p50": fpct(0.5), "p90": fpct(0.9), "p99": fpct(0.99), "max": fpct(1.0)}},
        "kinds": kinds_json,
        "dag": {"ops": ops, "expansions": g.expansions, "relations": rels.len(), "shared_relations": shared_rel,
            "recursive_dependent_relations": rels.iter().filter(|&&i| reach[i]).count(), "memo_ids": memo_on.iter().filter(|&&x| x).count(),
            "mask_scan_columns": mask_scan.iter().filter(|&&x| x).count(), "mask_all_columns": mask_all.iter().filter(|&&x| x).count(),
            "patterns": g.pats.len(), "regexes": g.regex.len(), "util_slots": g.util_body.len(), "compile_s": compile_s,
            "stats": g.stats, "max_stratum": max_stratum,
            "strata_hist": strata_hist.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
            "targets_per_stratum": per_stratum_targets.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>()},
        "counter_names": CNAMES, "trie_groups": groups.iter().map(|(k, c)| json!([k, c])).collect::<Vec<_>>(),
        "unconstrained_queries": unconstrained,
        "runs": results,
        "text_regex": {"pairs": text_pairs, "bytes": text_bytes, "seconds": text_s, "hits": text_hits},
        "queries": qjson,
        "per_file": pf,
    });
    std::fs::write(out, serde_json::to_string(&report).unwrap()).unwrap();
}
