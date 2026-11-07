//! Evaluates a parsed Bitlex lexer over one input.
//!
//! Masks are word operations (`mask.rs`). Scans, counts, machines, run operations and every
//! operation inside a `hypotheses` block are evaluated byte by byte here: this file gives the
//! semantics; the branch-free lowerings in 03-bitlex.md §3 and 08-without-automata.md must agree
//! with it.

use crate::mask::Mask;
use crate::syntax::{Arg, Expr, MachineDef, Stmt, UClass};
use std::collections::HashMap;

/// A state per byte: after the events at bytes `<= i` (`st`), before the event at `i` (`pre`),
/// and whether byte `i` was an event (`ev`).
#[derive(Clone)]
pub struct Scan {
    pub names: Vec<String>,
    pub groups: Vec<(String, Vec<String>)>,
    pub st: Vec<u8>,
    pub pre: Vec<u8>,
    pub ev: Mask,
    pub end: u8,
}

#[derive(Clone)]
pub enum Val {
    M(Mask),
    S(Scan),
    C(Vec<i32>),
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: String,
    pub start: usize,
    pub end: usize,
    pub soft: bool,
}

/// A `tree` section's declarations, its masks evaluated.
#[derive(Clone, Default)]
pub struct TreeDecl {
    /// Each node kind in order (its rank): kind, start mask, end mask, level (a count after each
    /// byte).
    pub nodes: Vec<(String, Mask, Mask, Option<Vec<i32>>)>,
    /// Code facts: whether a field (else a supertype), its name, the kinds, the mask.
    pub facts: Vec<(bool, String, Vec<String>, Mask)>,
    pub extra: Vec<String>,
    pub untoken: Vec<String>,
}

pub struct Output {
    /// The `tree` section, if the file has one.
    pub tree: Option<TreeDecl>,
    pub tokens: Vec<Token>,
    pub errors: Vec<String>,
    /// For each `rounds` block reached: a label and the rounds it took, the confirming one included.
    pub rounds: Vec<(String, usize)>,
    /// Every named value at the end, for checks against the oracles.
    pub vars: HashMap<String, Val>,
}

/// Segments of a `hypotheses` block, and the lane being evaluated.
struct Seg {
    first: Mask,
    last: Mask,
    lane: String,
}

struct TokDef {
    kind: String,
    start: Mask,
    through: Option<Mask>,
}

pub struct Ev<'a> {
    src: &'a [u8],
    n: usize,
    vars: HashMap<String, Val>,
    templates: HashMap<String, (Vec<String>, Vec<Stmt>)>,
    instances: HashMap<String, Vec<String>>,
    seg: Option<Seg>,
    lane_rejects: Vec<(String, Mask)>,
    lane_exit: Option<String>,
    errors: Vec<String>,
    rounds: Vec<(String, usize)>,
    prefix: String,
    toks: Vec<TokDef>,
    munch: Vec<(String, Mask, Vec<(Vec<u8>, Option<String>)>)>,
    keywords: Vec<(String, Vec<(Vec<u8>, Option<String>)>, Vec<Vec<u8>>, bool)>,
    skip: Mask,
    tables: &'a HashMap<String, Vec<(u32, u32)>>,
    tree: Option<TreeDecl>,
}

type R<T> = Result<T, String>;

/// Character ranges named `sym` in a tree-sitter `parser.c` (`static const TSCharacterRange`).
/// Expression nodes evaluated (each over the whole file), for cost estimates.
pub static OPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn load_table(parser_c: &str, sym: &str) -> R<Vec<(u32, u32)>> {
    let key = format!("{sym}[] = {{");
    let at = parser_c.find(&key).ok_or(format!("{sym} not in parser.c"))?;
    let body = &parser_c[at + key.len()..];
    let body = &body[..body.find("};").unwrap()];
    let lit = |s: &str| -> u32 {
        let s = s.trim();
        if let Some(h) = s.strip_prefix("0x") {
            u32::from_str_radix(h, 16).unwrap()
        } else if s.starts_with('\'') {
            let inner = &s[1..s.len() - 1];
            match inner {
                "\\\\" => b'\\' as u32,
                "\\'" => b'\'' as u32,
                _ => inner.chars().next().unwrap() as u32,
            }
        } else {
            s.parse().unwrap()
        }
    };
    let mut v = vec![];
    for part in body.split('}') {
        if let Some(p) = part.find('{') {
            let pair = &part[p + 1..];
            let (a, b) = pair.split_once(", ").ok_or("bad pair")?;
            v.push((lit(a), lit(b)));
        }
    }
    Ok(v)
}

impl<'a> Ev<'a> {
    pub fn run(prog: &[Stmt], src: &'a [u8], tables: &'a HashMap<String, Vec<(u32, u32)>>) -> R<Output> {
        let n = src.len();
        let mut ev = Ev {
            src,
            n,
            vars: HashMap::new(),
            templates: HashMap::new(),
            instances: HashMap::new(),
            seg: None,
            lane_rejects: vec![],
            lane_exit: None,
            errors: vec![],
            rounds: vec![],
            prefix: String::new(),
            toks: vec![],
            munch: vec![],
            keywords: vec![],
            skip: Mask::zeros(n),
            tables,
            tree: None,
        };
        ev.exec(prog)?;
        let tokens = ev.finish()?;
        Ok(Output { tree: ev.tree, tokens, errors: ev.errors, rounds: ev.rounds, vars: ev.vars })
    }

    pub fn get_mask(&self, name: &str) -> Option<&Mask> {
        match self.vars.get(name) {
            Some(Val::M(m)) => Some(m),
            _ => None,
        }
    }

    fn exec(&mut self, prog: &[Stmt]) -> R<()> {
        for s in prog {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn err(&mut self, msg: &str, m: &Mask) {
        if let Some(i) = m.first() {
            self.errors.push(format!("{}{msg} (byte {i})", self.prefix));
        }
    }

    fn stmt(&mut self, s: &Stmt) -> R<()> {
        match s {
            Stmt::Lexer(_) => {}
            Stmt::Class(name, items) => {
                let m = Mask::from_fn(self.n, |i| {
                    let b = self.src[i] as u32;
                    items.iter().any(|&(lo, hi)| lo <= b && b <= hi)
                });
                self.vars.insert(name.clone(), Val::M(m));
            }
            Stmt::Mask(name, e) => {
                let m = self.mask(e)?;
                self.vars.insert(name.clone(), Val::M(m));
            }
            Stmt::Reflect { name, a, an, b, bn, reset, start_lane } => {
                let (a, b) = (self.mask(a)?, self.mask(b)?);
                let reset = match reset {
                    Some(r) => self.mask(r)?,
                    None => Mask::zeros(self.n),
                };
                let names = vec!["out".to_string(), an.clone(), bn.clone()];
                let start = if *start_lane {
                    let seg = self.seg.as_ref().ok_or("`start lane` outside hypotheses")?;
                    names.iter().position(|x| *x == seg.lane).ok_or(format!("lane {} is no state of {name}", seg.lane))? as u8
                } else {
                    0
                };
                let first = self.seg.as_ref().map(|s| s.first.clone());
                let (mut st, mut pre, mut evm) = (vec![0u8; self.n], vec![0u8; self.n], Mask::zeros(self.n));
                let mut x = 0u8;
                for i in 0..self.n {
                    if i == 0 || first.as_ref().is_some_and(|f| f.get(i)) {
                        x = start
                    }
                    pre[i] = x;
                    if reset.get(i) {
                        x = 0
                    } else if a.get(i) {
                        x = (4 - x) % 3; // 1 - x mod 3
                        evm.set(i)
                    } else if b.get(i) {
                        x = (5 - x) % 3; // 2 - x mod 3
                        evm.set(i)
                    }
                    st[i] = x;
                }
                self.vars.insert(name.clone(), Val::S(Scan { names, groups: vec![], st, pre, ev: evm, end: x }));
            }
            Stmt::Count { name, up, down, reset, max } => {
                let (u, d) = (self.mask(up)?, self.mask(down)?);
                let reset = match reset {
                    Some(r) => self.mask(r)?,
                    None => Mask::zeros(self.n),
                };
                let first = self.seg.as_ref().map(|s| s.first.clone());
                let mut v = vec![0i32; self.n];
                let mut x = 0i32;
                for i in 0..self.n {
                    if first.as_ref().is_some_and(|f| f.get(i)) {
                        x = 0
                    }
                    if reset.get(i) {
                        x = 0
                    } else if u.get(i) {
                        x = (x + 1).min(*max as i32 + 1)
                    } else if d.get(i) && x > 0 {
                        x -= 1
                    }
                    v[i] = x;
                }
                let over = Mask::from_fn(self.n, |i| v[i] > *max as i32);
                let msg = format!("{name} exceeds max {max}");
                if self.seg.is_some() {
                    self.lane_rejects.push((msg, over));
                } else {
                    self.err(&msg, &over);
                }
                self.vars.insert(name.clone(), Val::C(v));
            }
            Stmt::Machine(name, def) => {
                let sc = self.machine(def)?;
                self.vars.insert(name.clone(), Val::S(sc));
            }
            Stmt::Block { name, params, body } => {
                self.templates.insert(name.clone(), (params.clone(), body.clone()));
            }
            Stmt::Inst { name, template, args } => self.instance(name, template, args)?,
            Stmt::Hyp { name, split, lanes, body } => self.hypotheses(name, split, lanes, body)?,
            Stmt::Reject(msg, e) => {
                let m = self.mask(e)?;
                if self.seg.is_none() {
                    return Err("reject outside hypotheses".into());
                }
                self.lane_rejects.push((msg.clone(), m));
            }
            Stmt::Exit(name) => self.lane_exit = Some(name.clone()),
            Stmt::Rounds { n, body, .. } => self.rounds(*n, body)?,
            Stmt::Carry(..) | Stmt::Next(..) => return Err("carry or next outside rounds".into()),
            Stmt::Ucode { over, classes } => {
                let over = self.mask(over)?;
                let mut ms: Vec<Mask> = classes.iter().map(|_| Mask::zeros(self.n)).collect();
                let mut i = 0;
                while i < self.n {
                    if !over.get(i) {
                        i += 1;
                        continue;
                    }
                    let (cp, len) = decode(&self.src[i..]);
                    for (k, (_, c)) in classes.iter().enumerate() {
                        let hit = match c {
                            UClass::Points(v) => v.iter().any(|&(lo, hi)| lo <= cp && cp <= hi),
                            UClass::Table(file, sym) => {
                                let t = self.tables.get(&format!("{file}:{sym}")).ok_or(format!("no table {file}:{sym}"))?;
                                t.iter().any(|&(lo, hi)| lo <= cp && cp <= hi)
                            }
                        };
                        if hit {
                            for j in i..(i + len).min(self.n) {
                                ms[k].set(j)
                            }
                        }
                    }
                    i += len;
                }
                for ((name, _), m) in classes.iter().zip(ms) {
                    self.vars.insert(name.clone(), Val::M(m));
                }
            }
            Stmt::Token { kind, start, through } => {
                let start = self.mask(start)?;
                let through = match through {
                    Some(t) => Some(self.mask(t)?),
                    None => None,
                };
                self.toks.push(TokDef { kind: kind.clone(), start, through });
            }
            Stmt::Munch { kind, over, lits } => {
                let over = self.mask(over)?;
                self.munch.push((kind.clone(), over, lits.clone()));
            }
            Stmt::Keywords { kind, words, soft, required } => {
                self.keywords.push((kind.clone(), words.clone(), soft.clone(), *required));
            }
            Stmt::Skip(e) => {
                let m = self.mask(e)?;
                self.skip = self.skip.or(&m);
            }
            Stmt::Tree { body, .. } => {
                self.tree.get_or_insert_with(TreeDecl::default);
                self.exec(body)?;
            }
            Stmt::Node { kind, start, end, level } => {
                let (start, end) = (self.mask(start)?, self.mask(end)?);
                let level = match level {
                    Some(l) => match self.vars.get(l) {
                        Some(Val::C(c)) => Some(c.clone()),
                        _ => return Err(format!("node {kind}: level `{l}` is not a count")),
                    },
                    None => None,
                };
                self.tree.get_or_insert_with(TreeDecl::default).nodes.push((kind.clone(), start, end, level));
            }
            Stmt::Fact { field, name, kinds, expr } => {
                let m = self.mask(expr)?;
                self.tree.get_or_insert_with(TreeDecl::default).facts.push((*field, name.clone(), kinds.clone(), m));
            }
            Stmt::Extra(kinds) => self.tree.get_or_insert_with(TreeDecl::default).extra.extend(kinds.iter().cloned()),
            Stmt::Untoken(kinds) => self.tree.get_or_insert_with(TreeDecl::default).untoken.extend(kinds.iter().cloned()),
            Stmt::Error(msg, e) => {
                let m = self.mask(e)?;
                self.err(msg, &m);
            }
        }
        Ok(())
    }

    fn instance(&mut self, name: &str, template: &str, args: &[Expr]) -> R<()> {
        let (params, body) = self.templates.get(template).cloned().ok_or(format!("no block {template}"))?;
        if params.len() != args.len() {
            return Err(format!("{name}: {} arguments for {} parameters", args.len(), params.len()));
        }
        let vals: Vec<Mask> = args.iter().map(|a| self.mask(a)).collect::<R<_>>()?;
        let mut defined = params.clone();
        body.iter().for_each(|s| s.defines(&mut defined));
        let saved: Vec<(String, Option<Val>)> = defined.iter().map(|d| (d.clone(), self.vars.remove(d))).collect();
        for (p, v) in params.iter().zip(vals) {
            self.vars.insert(p.clone(), Val::M(v));
        }
        let new_prefix = format!("{}{name}: ", self.prefix);
        let old_prefix = std::mem::replace(&mut self.prefix, new_prefix);
        let r = self.exec(&body);
        self.prefix = old_prefix;
        r?;
        for d in &defined {
            if let Some(v) = self.vars.remove(d) {
                self.vars.insert(format!("{name}.{d}"), v);
            }
        }
        for (d, v) in saved {
            if let Some(v) = v {
                self.vars.insert(d, v);
            }
        }
        let inst = self.instances.entry(template.to_string()).or_default();
        if !inst.iter().any(|x| x == name) {
            inst.push(name.to_string());
        }
        Ok(())
    }

    fn rounds(&mut self, n: usize, body: &[Stmt]) -> R<()> {
        let carries: Vec<(&String, &Expr)> = body.iter().filter_map(|s| if let Stmt::Carry(a, e) = s { Some((a, e)) } else { None }).collect();
        let nexts: Vec<(&String, &Expr)> = body.iter().filter_map(|s| if let Stmt::Next(a, e) = s { Some((a, e)) } else { None }).collect();
        let label = format!("{}rounds({})", self.prefix, carries.first().map(|c| c.0.as_str()).unwrap_or(""));
        for (name, e) in &carries {
            let m = self.mask(e)?;
            self.vars.insert((*name).clone(), Val::M(m));
        }
        let err_mark = self.errors.len();
        let rej_mark = self.lane_rejects.len();
        let rest: Vec<Stmt> = body.iter().filter(|s| !matches!(s, Stmt::Carry(..) | Stmt::Next(..))).cloned().collect();
        for r in 1..=n {
            self.errors.truncate(err_mark);
            self.lane_rejects.truncate(rej_mark);
            self.exec(&rest)?;
            let next: Vec<Mask> = nexts.iter().map(|(_, e)| self.mask(e)).collect::<R<_>>()?;
            let mut changed = Mask::zeros(self.n);
            for ((name, _), m) in nexts.iter().zip(&next) {
                let cur = match self.vars.get(*name) {
                    Some(Val::M(c)) => c,
                    _ => return Err(format!("next {name}: no carry of that name")),
                };
                changed = changed.or(&cur.xor(m));
            }
            if std::env::var("BITLEX_TRACE").is_ok_and(|v| label.contains(&v)) {
                let show = |m: &Mask| m.ones_iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
                for ((name, _), m) in nexts.iter().zip(&next) {
                    eprintln!("{label} round {r}: next {name} = [{}]", show(m));
                }
            }
            if !changed.any() {
                self.rounds.push((label, r));
                return Ok(());
            }
            if r == n {
                self.rounds.push((label.clone(), n + 1));
                if self.seg.is_some() {
                    self.lane_rejects.push((format!("{label} did not settle"), changed));
                } else {
                    self.errors.push(format!("{label} did not settle in {n}"));
                }
                return Ok(());
            }
            for ((name, _), m) in nexts.iter().zip(next) {
                self.vars.insert((*name).clone(), Val::M(m));
            }
        }
        Ok(())
    }

    fn hypotheses(&mut self, name: &str, split: &Expr, lanes: &[String], body: &[Stmt]) -> R<()> {
        if self.seg.is_some() {
            return Err("nested hypotheses".into());
        }
        let n = self.n;
        if n == 0 {
            // No segments: evaluate once so every name is defined, over no bytes.
            self.seg = Some(Seg { first: Mask::zeros(0), last: Mask::zeros(0), lane: lanes[0].clone() });
            let r = self.exec(body);
            self.seg = None;
            return r;
        }
        let split = self.mask(split)?;
        let mut first = split.prev(1);
        first.set(0);
        let mut last = split.clone();
        last.set(n - 1);
        let starts: Vec<usize> = first.ones_iter().collect();
        let seg_of = |i: usize| starts.partition_point(|&s| s <= i) - 1;
        let mut defined = vec![];
        body.iter().for_each(|s| s.defines(&mut defined));
        let saved: Vec<(String, Option<Val>)> = defined.iter().map(|d| (d.clone(), self.vars.remove(d))).collect();
        let mut lane_vals: Vec<HashMap<String, Val>> = vec![];
        let mut exits: Vec<Vec<usize>> = vec![];
        let mut rejects: Vec<Vec<Option<String>>> = vec![];
        let err_mark = self.errors.len();
        for lane in lanes {
            self.seg = Some(Seg { first: first.clone(), last: last.clone(), lane: lane.clone() });
            self.lane_rejects.clear();
            self.lane_exit = None;
            let r = self.exec(body);
            self.seg = None;
            r?;
            if self.errors.len() > err_mark {
                return Err(format!("errors inside hypotheses {name}: {:?}", &self.errors[err_mark..]));
            }
            let exit = self.lane_exit.clone().ok_or(format!("hypotheses {name}: no exit"))?;
            let sc = match self.vars.get(&exit) {
                Some(Val::S(s)) => s.clone(),
                _ => return Err(format!("exit {exit}: not a scan")),
            };
            let map: Vec<usize> = sc.names.iter().map(|s| lanes.iter().position(|l| l == s).unwrap_or(usize::MAX)).collect();
            exits.push(starts.iter().enumerate().map(|(k, _)| {
                let e = if k + 1 < starts.len() { starts[k + 1] - 1 } else { n - 1 };
                map[sc.st[e] as usize]
            }).collect());
            let mut rj: Vec<Option<String>> = vec![None; starts.len()];
            for (msg, m) in &self.lane_rejects {
                for i in m.ones_iter() {
                    let k = seg_of(i);
                    if rj[k].is_none() {
                        rj[k] = Some(msg.clone())
                    }
                }
            }
            rejects.push(rj);
            let mut vals = HashMap::new();
            for d in &defined {
                if let Some(v) = self.vars.remove(d) {
                    vals.insert(d.clone(), v);
                }
            }
            lane_vals.push(vals);
        }
        for (d, v) in saved {
            if let Some(v) = v {
                self.vars.insert(d, v);
            }
        }
        // The real lane of each segment: a walk here; a scan of 3-state maps when lowered.
        let mut lane = 0usize;
        let mut lane_of = vec![0u8; n];
        for k in 0..starts.len() {
            let e = if k + 1 < starts.len() { starts[k + 1] } else { n };
            for x in &mut lane_of[starts[k]..e] {
                *x = lane as u8
            }
            if let Some(msg) = &rejects[lane][k] {
                self.errors.push(format!("{}{name}: {msg} (byte {})", self.prefix, starts[k]));
                lane = 0;
            } else {
                lane = exits[lane][k];
                if lane == usize::MAX {
                    return Err(format!("hypotheses {name}: exit state is no lane"));
                }
            }
        }
        if lane != 0 {
            self.errors.push(format!("{}{name}: the file ends in lane {}", self.prefix, lanes[lane]));
        }
        let sel: Vec<Mask> = (0..lanes.len()).map(|l| Mask::from_fn(n, |i| lane_of[i] as usize == l)).collect();
        for d in &defined {
            let vs: Vec<&Val> = lane_vals.iter().filter_map(|m| m.get(d)).collect();
            if vs.len() != lanes.len() {
                continue;
            }
            let v = match vs[0] {
                Val::M(_) => {
                    let mut out = Mask::zeros(n);
                    for (l, v) in vs.iter().enumerate() {
                        if let Val::M(m) = v {
                            out = out.or(&m.and(&sel[l]))
                        }
                    }
                    Val::M(out)
                }
                Val::C(_) => Val::C((0..n).map(|i| if let Val::C(c) = vs[lane_of[i] as usize] { c[i] } else { 0 }).collect()),
                Val::S(s0) => {
                    let mut s = s0.clone();
                    for i in 0..n {
                        if let Val::S(sl) = vs[lane_of[i] as usize] {
                            s.st[i] = sl.st[i];
                            s.pre[i] = sl.pre[i];
                            if sl.ev.get(i) {
                                s.ev.set(i)
                            } else {
                                s.ev.clear(i)
                            }
                        }
                    }
                    s.end = s.st[n - 1];
                    Val::S(s)
                }
            };
            self.vars.insert(d.clone(), v);
        }
        Ok(())
    }

    fn machine(&mut self, def: &MachineDef) -> R<Scan> {
        if self.seg.is_some() {
            return Err("machines inside hypotheses are not supported".into());
        }
        let evs: Vec<Mask> = def.events.iter().map(|(_, e)| self.mask(e)).collect::<R<_>>()?;
        let sidx = |s: &str| def.states.iter().position(|x| x == s).ok_or(format!("no state {s}"));
        let ns = def.states.len();
        let nsym = def.events.len();
        let err_state = def.states.iter().position(|x| x == "error");
        let mut table = vec![vec![usize::MAX; nsym]; ns];
        for (st, syms, to) in &def.rules {
            let (a, b) = (sidx(st)?, sidx(to)?);
            for sy in syms {
                if sy == "any" {
                    continue;
                }
                let k = def.events.iter().position(|e| &e.0 == sy).ok_or(format!("no event {sy}"))?;
                table[a][k] = b;
            }
        }
        for (st, syms, to) in &def.rules {
            if syms.iter().any(|s| s == "any") {
                let (a, b) = (sidx(st)?, sidx(to)?);
                for k in 0..nsym {
                    if table[a][k] == usize::MAX {
                        table[a][k] = b
                    }
                }
            }
        }
        for s in 0..ns {
            for k in 0..nsym {
                if table[s][k] == usize::MAX {
                    table[s][k] = s
                }
            }
        }
        if let Some(e) = err_state {
            table[e] = vec![e; nsym];
        }
        let mut x = sidx(&def.start)?;
        let (mut st, mut pre, mut evm) = (vec![0u8; self.n], vec![0u8; self.n], Mask::zeros(self.n));
        for i in 0..self.n {
            pre[i] = x as u8;
            if let Some(k) = evs.iter().position(|m| m.get(i)) {
                x = table[x][k];
                evm.set(i);
            }
            st[i] = x as u8;
        }
        let end = x as u8;
        let mut sc = Scan { names: def.states.clone(), groups: def.groups.clone(), st, pre, ev: evm, end };
        // `final` is checked by the lexer's own error statements; record it for `final()`.
        sc.groups.push(("__final".into(), def.finals.clone()));
        Ok(sc)
    }

    fn state_set(&self, sc: &Scan, names: &[String]) -> R<Vec<bool>> {
        let mut v = vec![false; sc.names.len()];
        for nm in names {
            if let Some(i) = sc.names.iter().position(|x| x == nm) {
                v[i] = true;
            } else if let Some((_, g)) = sc.groups.iter().find(|g| &g.0 == nm) {
                for s in g {
                    v[sc.names.iter().position(|x| x == s).ok_or(format!("no state {s}"))?] = true;
                }
            } else {
                return Err(format!("no state or group {nm}"));
            }
        }
        Ok(v)
    }

    fn lookup(&self, name: &str) -> R<Vec<&Val>> {
        if let Some((t, rest)) = name.split_once("*.") {
            let insts = self.instances.get(t).ok_or(format!("no instances of {t}"))?;
            return insts.iter().map(|i| self.vars.get(&format!("{i}.{rest}")).ok_or(format!("no {i}.{rest}"))).collect();
        }
        Ok(vec![self.vars.get(name).ok_or(format!("undefined: {name}"))?])
    }

    fn seg_prev(&self, a: &Mask, k: usize) -> Mask {
        let m = a.prev(k);
        match &self.seg {
            Some(s) if k > 0 => m.andnot(&s.first.span(k)),
            _ => m,
        }
    }
    fn seg_next(&self, a: &Mask, k: usize) -> Mask {
        let m = a.next(k);
        match &self.seg {
            Some(s) if k > 0 => {
                let mut cut = s.last.clone();
                for j in 1..k {
                    cut = cut.or(&s.last.next(j));
                }
                m.andnot(&cut)
            }
            _ => m,
        }
    }

    fn mask(&self, e: &Expr) -> R<Mask> {
        let n = self.n;
        if !matches!(e, Expr::Name(_) | Expr::Empty) {
            OPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(match e {
            Expr::Empty => Mask::zeros(n),
            Expr::Name(nm) => {
                if let Some(s) = &self.seg {
                    if nm == "first" {
                        return Ok(s.first.clone());
                    }
                    if nm == "last" {
                        return Ok(s.last.clone());
                    }
                }
                let vs = self.lookup(nm)?;
                let mut out = Mask::zeros(n);
                for v in vs {
                    match v {
                        Val::M(m) => out = out.or(m),
                        _ => return Err(format!("{nm} is not a mask")),
                    }
                }
                out
            }
            Expr::Not(a) => self.mask(a)?.not(),
            Expr::And(a, b) => self.mask(a)?.and(&self.mask(b)?),
            Expr::Or(a, b) => self.mask(a)?.or(&self.mask(b)?),
            Expr::Xor(a, b) => self.mask(a)?.xor(&self.mask(b)?),
            Expr::Test { target, method, from, to, k, other, arg } => {
                let vs = self.lookup(target)?;
                let mut out = Mask::zeros(n);
                for v in vs {
                    let m = match (v, method.as_str()) {
                        (Val::S(sc), "in") => {
                            let set = self.state_set(sc, from)?;
                            Mask::from_fn(n, |i| set[sc.st[i] as usize])
                        }
                        (Val::S(sc), "on") => {
                            let (a, b) = (self.state_set(sc, from)?, self.state_set(sc, to)?);
                            Mask::from_fn(n, |i| sc.ev.get(i) && a[sc.pre[i] as usize] && b[sc.st[i] as usize])
                        }
                        (Val::S(sc), "final") => {
                            let set = self.state_set(sc, from)?;
                            if set[sc.end as usize] { Mask::ones(n) } else { Mask::zeros(n) }
                        }
                        (Val::S(sc), "reached") => {
                            let set = self.state_set(sc, from)?;
                            if sc.st.iter().any(|&s| set[s as usize]) { Mask::ones(n) } else { Mask::zeros(n) }
                        }
                        (Val::C(c), "eq" | "ge") => {
                            let o: Option<&Vec<i32>> = match other {
                                Some(o) => match self.vars.get(o) {
                                    Some(Val::C(oc)) => Some(oc),
                                    _ => return Err(format!("{o} is not a count")),
                                },
                                None => None,
                            };
                            let kk = k.unwrap_or(0) as i32;
                            let ge = method == "ge";
                            Mask::from_fn(n, |i| {
                                let r = o.map(|o| o[i]).unwrap_or(kk);
                                if ge { c[i] >= r } else { c[i] == r }
                            })
                        }
                        (Val::C(c), "frame" | "since") => {
                            // A stack of the frames open before each byte: its opener is in M
                            // (frame), or a byte of M has been seen at its own level (since).
                            let m = self.mask(arg.as_ref().ok_or("frame/since needs a mask")?)?;
                            let mut out = Mask::zeros(n);
                            let mut stack: Vec<bool> = vec![];
                            let mut before = 0i32;
                            let first = self.seg.as_ref().map(|s| &s.first);
                            for i in 0..n {
                                if first.is_some_and(|f| f.get(i)) {
                                    stack.clear();
                                    before = 0
                                }
                                if stack.last() == Some(&true) {
                                    out.set(i)
                                }
                                let after = c[i];
                                if after > before {
                                    stack.push(method == "frame" && m.get(i));
                                } else if after < before {
                                    stack.truncate(after.max(0) as usize);
                                } else if method == "since" && m.get(i) {
                                    if let Some(t) = stack.last_mut() {
                                        *t = true
                                    }
                                }
                                before = after;
                            }
                            out
                        }
                        _ => return Err(format!("{target}.{method}: wrong kind of value")),
                    };
                    out = out.or(&m);
                }
                out
            }
            Expr::Call(f, args) => {
                let ea = |i: usize| -> R<Mask> {
                    match args.get(i) {
                        Some(Arg::E(e)) => self.mask(e),
                        _ => Err(format!("{f}: argument {i} must be a mask")),
                    }
                };
                let na = |i: usize, d: u64| -> R<usize> {
                    match args.get(i) {
                        Some(Arg::N(v)) => Ok(*v as usize),
                        None => Ok(d as usize),
                        _ => Err(format!("{f}: argument {i} must be a number")),
                    }
                };
                match f.as_str() {
                    "prev" => self.seg_prev(&ea(0)?, na(1, 1)?),
                    "next" => self.seg_next(&ea(0)?, na(1, 1)?),
                    "starts" => {
                        let a = ea(0)?;
                        a.andnot(&self.seg_prev(&a, 1))
                    }
                    "lasts" => {
                        let a = ea(0)?;
                        a.andnot(&self.seg_next(&a, 1))
                    }
                    "after" => {
                        let a = ea(0)?;
                        self.seg_prev(&a, 1).andnot(&a)
                    }
                    "span" => {
                        let (a, k) = (ea(0)?, na(1, 1)?);
                        let mut out = a.clone();
                        for j in 1..k {
                            out = out.or(&self.seg_prev(&a, j));
                        }
                        out
                    }
                    "toggle" => {
                        let a = ea(0)?;
                        match &self.seg {
                            None => a.toggle(),
                            Some(s) => {
                                let mut out = Mask::zeros(n);
                                let mut p = false;
                                for i in 0..n {
                                    if s.first.get(i) {
                                        p = false
                                    }
                                    p ^= a.get(i);
                                    if p {
                                        out.set(i)
                                    }
                                }
                                out
                            }
                        }
                    }
                    "fill" => {
                        let (s, mut e) = (ea(0)?, ea(1)?);
                        if let Some(sg) = &self.seg {
                            e = e.or(&sg.first);
                        }
                        Mask::fill(&s, &e)
                    }
                    "back_fill" => {
                        let (s, mut e) = (ea(0)?, ea(1)?);
                        if let Some(sg) = &self.seg {
                            e = e.or(&sg.last);
                        }
                        Mask::back_fill(&s, &e)
                    }
                    "after_odd_run" => {
                        let a = ea(0)?;
                        match &self.seg {
                            None => a.after_odd_run(),
                            Some(s) => {
                                let mut out = Mask::zeros(n);
                                let mut run = 0;
                                for i in 0..n {
                                    if s.first.get(i) {
                                        run = 0
                                    }
                                    if a.get(i) {
                                        run += 1
                                    } else {
                                        if run % 2 == 1 {
                                            out.set(i)
                                        }
                                        run = 0
                                    }
                                }
                                out
                            }
                        }
                    }
                    "text" => {
                        let Some(Arg::S(t)) = args.first() else { return Err("text needs a string".into()) };
                        Mask::from_fn(n, |i| self.src[i..].starts_with(t))
                    }
                    "aligned" | "head" => {
                        let a = ea(0)?;
                        // head(a, C): the first C bytes of each run, C a count read before the run.
                        let by_count: Option<&Vec<i32>> = match args.get(1) {
                            Some(Arg::E(Expr::Name(c))) => match self.vars.get(c) {
                                Some(Val::C(v)) => Some(v),
                                _ => None,
                            },
                            _ => None,
                        };
                        let k = if by_count.is_some() { 0 } else { na(1, 1)? };
                        let mut lim = k;
                        let mut out = Mask::zeros(n);
                        let first = self.seg.as_ref().map(|s| &s.first);
                        let mut off = 0usize;
                        for i in 0..n {
                            if !a.get(i) {
                                continue;
                            }
                            if i == 0 || !a.get(i - 1) || first.is_some_and(|f| f.get(i)) {
                                off = 0;
                                if let Some(c) = by_count {
                                    lim = if i == 0 { 0 } else { c[i - 1].max(0) as usize };
                                }
                            } else {
                                off += 1
                            }
                            let hit = if f == "aligned" { off % k == 0 } else { off < lim };
                            if hit {
                                out.set(i)
                            }
                        }
                        out
                    }
                    "line_has" => {
                        let (a, nl) = (ea(0)?, ea(1)?);
                        let mut out = Mask::zeros(n);
                        let mut has = false;
                        for i in 0..n {
                            has |= a.get(i);
                            if nl.get(i) {
                                if has {
                                    out.set(i)
                                }
                                has = false
                            }
                        }
                        out
                    }
                    "at_end" => {
                        let a = ea(0)?;
                        if n > 0 && a.get(n - 1) { Mask::ones(n) } else { Mask::zeros(n) }
                    }
                    _ => return Err(format!("unknown operation {f}")),
                }
            }
        })
    }

    fn finish(&mut self) -> R<Vec<Token>> {
        let n = self.n;
        let mut kind_at: Vec<u32> = vec![0; n]; // 1 + index into kinds
        let mut len_at: Vec<u32> = vec![0; n]; // fixed length (munch), or 0
        let mut kinds: Vec<String> = vec![];
        let mut throughs: Vec<Option<Mask>> = vec![];
        let mut overlap: Option<String> = None;
        for t in &self.toks {
            kinds.push(t.kind.clone());
            throughs.push(t.through.clone());
            let k = kinds.len() as u32;
            for i in t.start.ones_iter() {
                if kind_at[i] != 0 && overlap.is_none() {
                    overlap = Some(format!("tokens {} and {} both start at byte {i}", kinds[kind_at[i] as usize - 1], t.kind));
                }
                kind_at[i] = k;
            }
        }
        for (_, over, lits) in &self.munch {
            let mut runs = vec![];
            over.runs(|a, b| runs.push((a, b)));
            for (a, b) in runs {
                let mut i = a;
                while i < b {
                    let best = lits.iter().filter(|(l, _)| self.src[i..b].starts_with(l)).max_by_key(|(l, _)| l.len());
                    let Some((l, alias)) = best else {
                        self.errors.push(format!("munch: no operator at byte {i}"));
                        break;
                    };
                    let kind = alias.clone().unwrap_or_else(|| String::from_utf8_lossy(l).into_owned());
                    let k = match kinds.iter().position(|x| *x == kind) {
                        Some(p) => p as u32 + 1,
                        None => {
                            kinds.push(kind.clone());
                            throughs.push(None);
                            kinds.len() as u32
                        }
                    };
                    if kind_at[i] != 0 && overlap.is_none() {
                        overlap = Some(format!("tokens {} and {kind} both start at byte {i}", kinds[kind_at[i] as usize - 1]));
                    }
                    kind_at[i] = k;
                    len_at[i] = l.len() as u32;
                    i += l.len();
                }
            }
        }
        if let Some(o) = overlap {
            self.errors.push(o);
        }
        let mut toks = vec![];
        let mut covered = Mask::zeros(n);
        for i in 0..n {
            if kind_at[i] == 0 {
                continue;
            }
            let k = kind_at[i] as usize - 1;
            let mut end = i + 1;
            if len_at[i] > 0 {
                end = i + len_at[i] as usize;
            } else if let Some(t) = &throughs[k] {
                while end < n && t.get(end) && kind_at[end] == 0 {
                    end += 1
                }
            }
            for j in i..end {
                covered.set(j)
            }
            toks.push(Token { kind: kinds[k].clone(), start: i, end, soft: false });
        }
        let gap = covered.or(&self.skip).not();
        if let Some(i) = gap.first() {
            self.errors.push(format!("byte {i} is in no token"));
        }
        for (kind, words, soft, required) in &self.keywords {
            for t in toks.iter_mut().filter(|t| &t.kind == kind) {
                let text = &self.src[t.start..t.end];
                if let Some((w, alias)) = words.iter().find(|(w, _)| w == text) {
                    t.kind = alias.clone().unwrap_or_else(|| String::from_utf8_lossy(w).into_owned());
                } else if soft.iter().any(|w| w == text) {
                    t.soft = true;
                } else if *required {
                    self.errors.push(format!("{kind} {:?} is no keyword (byte {})", String::from_utf8_lossy(text), t.start));
                }
            }
        }
        Ok(toks)
    }
}

/// One UTF-8 character: its code point and length. An invalid sequence is one byte, U+FFFD.
fn decode(b: &[u8]) -> (u32, usize) {
    let c = b[0];
    let (len, init) = match c {
        0x00..=0x7f => return (c as u32, 1),
        0xc0..=0xdf => (2, (c & 0x1f) as u32),
        0xe0..=0xef => (3, (c & 0x0f) as u32),
        0xf0..=0xf7 => (4, (c & 0x07) as u32),
        _ => return (0xfffd, 1),
    };
    if b.len() < len || !b[1..len].iter().all(|&x| x & 0xc0 == 0x80) {
        return (0xfffd, 1);
    }
    (b[1..len].iter().fold(init, |a, &x| (a << 6) | (x & 0x3f) as u32), len)
}
