//! A parsed lexer to a stream program (`ops.rs`).
//!
//! Each construct becomes block kernels (`03-bitlex.md` §3, 09 §3.3):
//!
//! - segment edges (`hypotheses`' lines) are masks: shifts are cut by them, scans restart at them;
//!   file edges are block edges, where every carry restarts;
//! - `aligned` is a fill per residue, `after_odd_run` an aligned run end, `line_has` a fill;
//! - machines, reflections and lane choice are scans of maps over states (`MapScan`);
//! - counts are scans of clamp maps; `frame`, `since` and `head(a, C)` are a fill per level up to
//!   the count's declared `max`;
//! - which files have a byte of a mask is a fill forward and a fill back;
//! - blocks expand to prefixed copies, each hypothesis lane gets its own copy of the body, and
//!   `rounds` become a loop over a range of operations.
//!
//! A construct without such a form is refused.

use super::ops::{Id, Membership, Op, Program, SINK, TokenDef};
use super::syntax::{Arg, Expr, MachineDef, Stmt, UClass};
use std::collections::HashMap;

#[derive(Clone, Debug)]
struct ScanV {
    names: Vec<String>,
    groups: Vec<(String, Vec<String>)>,
    /// Per state, the bytes after which the scan is in it, and before which it was.
    ins: Vec<Id>,
    pres: Vec<Id>,
    ev: Id,
    /// The state after each byte, as lanes.
    state: Id,
}

#[derive(Clone, Debug)]
enum V {
    M(Id),
    S(ScanV),
    /// Count lanes and the count's bound.
    C(Id, u8),
}

#[derive(Clone, Copy)]
struct Seg {
    first: Id,
    last: Id,
}

struct Lower<'a> {
    ops: Vec<Op>,
    memo: HashMap<String, Id>,
    groups: Vec<Box<[u8; 256]>>,
    group_ops: Vec<Id>,
    classes: HashMap<Vec<(u32, u32)>, Id>,
    env: HashMap<String, V>,
    templates: HashMap<String, (Vec<String>, Vec<Stmt>)>,
    instances: HashMap<String, Vec<String>>,
    file_first: Id,
    file_last: Id,
    hyp: Option<(Seg, String)>,
    lane_rejects: Vec<(String, Id)>,
    lane_exit: Option<String>,
    prefix: String,
    tokens: Vec<TokenDef>,
    munch: Vec<(String, Id, Vec<(Vec<u8>, Option<String>)>)>,
    keywords: Vec<(String, Vec<(Vec<u8>, Option<String>)>, Vec<Vec<u8>>, bool)>,
    skip: Id,
    tables: &'a HashMap<String, Vec<(u32, u32)>>,
}

type R<T> = Result<T, String>;

pub(in crate::code_rule_dsl) fn lower(prog: &[Stmt], tables: &HashMap<String, Vec<(u32, u32)>>) -> R<Program> {
    let mut l = Lower {
        ops: vec![],
        memo: HashMap::new(),
        groups: vec![],
        group_ops: vec![],
        classes: HashMap::new(),
        env: HashMap::new(),
        templates: HashMap::new(),
        instances: HashMap::new(),
        file_first: 0,
        file_last: 0,
        hyp: None,
        lane_rejects: vec![],
        lane_exit: None,
        prefix: String::new(),
        tokens: vec![],
        munch: vec![],
        keywords: vec![],
        skip: 0,
        tables,
    };
    l.file_first = l.push(Op::FileFirst);
    l.file_last = l.push(Op::FileLast);
    l.skip = l.push(Op::Empty);
    l.exec(prog)?;
    let mut p = Program { ops: l.ops, live: vec![], groups: l.groups, tokens: l.tokens, munch: l.munch, keywords: l.keywords, skip: l.skip };
    p.mark_live();
    Ok(p)
}

impl Lower<'_> {
    /// One operation; an equal one already pushed is shared (inputs are ids, so equal text is an
    /// equal value). Carries, control and outputs are never shared.
    fn push(&mut self, op: Op) -> Id {
        let shareable = !matches!(op, Op::Carry(_) | Op::RoundsBegin | Op::RoundsEnd { .. } | Op::Errors { .. });
        let key = shareable.then(|| format!("{op:?}"));
        if let Some(id) = key.as_ref().and_then(|k| self.memo.get(k)) {
            return *id;
        }
        self.ops.push(op);
        let id = self.ops.len() - 1;
        if let Some(k) = key {
            self.memo.insert(k, id);
        }
        id
    }

    // --- Logic ---
    fn or(&mut self, a: Id, b: Id) -> Id {
        self.push(Op::Or(a, b))
    }
    fn and(&mut self, a: Id, b: Id) -> Id {
        self.push(Op::And(a, b))
    }
    fn andnot(&mut self, a: Id, b: Id) -> Id {
        self.push(Op::AndNot(a, b))
    }
    fn not(&mut self, a: Id) -> Id {
        self.push(Op::Not(a))
    }
    fn empty(&mut self) -> Id {
        self.push(Op::Empty)
    }
    fn cnst(&mut self, v: u8) -> Id {
        self.push(Op::LaneConst(v))
    }
    fn sel(&mut self, m: Id, a: Id, b: Id) -> Id {
        self.push(Op::LaneSelect { m, a, b })
    }
    fn eq(&mut self, a: Id, k: u8) -> Id {
        self.push(Op::LaneEq { a, k })
    }
    fn ge(&mut self, a: Id, k: u8) -> Id {
        self.push(Op::LaneGe { a, k })
    }
    fn lmap(&mut self, a: Id, map: &[u8]) -> Id {
        let mut table = Box::new([SINK; 256]);
        table[..map.len()].copy_from_slice(map);
        self.push(Op::LaneMap { a, table })
    }

    fn seg(&self) -> Option<Seg> {
        self.hyp.as_ref().map(|h| h.0)
    }

    // --- Shifts, cut at segment edges ---
    fn prev_plain(&mut self, a: Id, k: usize) -> R<Id> {
        if k > 64 {
            return Err(format!("prev by {k}: at most 64"));
        }
        Ok(if k == 0 { a } else { self.push(Op::Prev { a, k: k as u32 }) })
    }
    fn next_plain(&mut self, a: Id, k: usize) -> R<Id> {
        if k > 64 {
            return Err(format!("next by {k}: at most 64"));
        }
        Ok(if k == 0 { a } else { self.push(Op::Next { a, k: k as u32 }) })
    }
    /// `prev(a, k)` with no bit carried past a segment's first byte.
    fn prev(&mut self, a: Id, k: usize) -> R<Id> {
        let p = self.prev_plain(a, k)?;
        let Some(s) = self.seg() else { return Ok(p) };
        if k == 0 {
            return Ok(p);
        }
        let mut cut = s.first;
        for j in 1..k {
            let x = self.prev_plain(s.first, j)?;
            cut = self.or(cut, x);
        }
        Ok(self.andnot(p, cut))
    }
    fn next(&mut self, a: Id, k: usize) -> R<Id> {
        let n = self.next_plain(a, k)?;
        let Some(s) = self.seg() else { return Ok(n) };
        if k == 0 {
            return Ok(n);
        }
        let mut cut = s.last;
        for j in 1..k {
            let x = self.next_plain(s.last, j)?;
            cut = self.or(cut, x);
        }
        Ok(self.andnot(n, cut))
    }

    // --- Scans, restarted at segment edges ---
    fn fill(&mut self, s: Id, e: Id) -> Id {
        let e = match self.seg() {
            Some(g) => self.or(e, g.first),
            None => e,
        };
        self.push(Op::Fill { s, e })
    }
    fn back_fill(&mut self, s: Id, e: Id) -> Id {
        let e = match self.seg() {
            Some(g) => self.or(e, g.last),
            None => e,
        };
        self.push(Op::BackFill { s, e })
    }
    fn toggle(&mut self, a: Id) -> R<Id> {
        let t = self.push(Op::Toggle(a));
        let Some(g) = self.seg() else { return Ok(t) };
        // Each segment's incoming parity removed: a fill from the segment starts that inherit a 1.
        let p = self.prev_plain(t, 1)?;
        let x = self.and(g.first, p);
        let c = self.push(Op::Fill { s: x, e: g.first });
        Ok(self.push(Op::Xor(t, c)))
    }
    /// Bytes of `a` at an offset in their run that is a multiple of `k`: a fill per residue.
    fn aligned(&mut self, a: Id, k: u32) -> R<Id> {
        if k <= 1 {
            return Ok(a);
        }
        let p = self.prev(a, 1)?;
        let rs = self.andnot(a, p);
        let na = self.not(a);
        let mut out = self.empty();
        for c in 0..k {
            let pat = self.push(Op::Pattern { k, c });
            let s = self.and(rs, pat);
            let other = self.andnot(rs, pat);
            let e = self.or(na, other);
            let r = self.push(Op::Fill { s, e });
            let hit = self.and(r, pat);
            out = self.or(out, hit);
        }
        Ok(self.and(out, a))
    }
    /// The bytes of each file that has a byte of `t`: a fill to the file's end, then back.
    fn in_files(&mut self, t: Id) -> Id {
        let e = self.empty();
        let f = self.push(Op::Fill { s: t, e });
        let at = self.and(f, self.file_last);
        self.push(Op::BackFill { s: at, e })
    }
    fn at_end(&mut self, a: Id) -> Id {
        let e = self.empty();
        let at = self.and(a, self.file_last);
        self.push(Op::BackFill { s: at, e })
    }

    fn class(&mut self, items: &[(u32, u32)]) -> Id {
        let mut key = items.to_vec();
        key.sort();
        if let Some(&id) = self.classes.get(&key) {
            return id;
        }
        let n = self.classes.len();
        let (g, bit) = (n / 8, (n % 8) as u8);
        if g == self.groups.len() {
            self.groups.push(Box::new([0; 256]));
            let op = self.push(Op::Classify(g));
            self.group_ops.push(op);
        }
        for b in 0..256u32 {
            if items.iter().any(|&(lo, hi)| lo <= b && b <= hi) {
                self.groups[g][b as usize] |= 1 << bit;
            }
        }
        let id = self.push(Op::LaneTest { a: self.group_ops[g], bit: 1 << bit });
        self.classes.insert(key, id);
        id
    }

    fn error(&mut self, msg: String, m: Id) {
        self.push(Op::Errors { parts: vec![(msg, m)], all: false, byte: true });
    }

    fn exec(&mut self, prog: &[Stmt]) -> R<()> {
        for s in prog {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn opt(&mut self, e: &Option<Expr>) -> R<Option<Id>> {
        e.as_ref().map(|e| self.mask(e)).transpose()
    }

    /// A scan's masks from its state lanes.
    fn scan(&mut self, names: Vec<String>, groups: Vec<(String, Vec<String>)>, state: Id, ev: Id, start: usize) -> R<ScanV> {
        let first = self.seg().map(|s| s.first).unwrap_or(self.file_first);
        let mut ins = vec![];
        let mut pres = vec![];
        for s in 0..names.len() {
            let m = self.eq(state, s as u8);
            ins.push(m);
            let p = self.prev(m, 1)?;
            pres.push(if s == start { self.or(p, first) } else { p });
        }
        Ok(ScanV { names, groups, ins, pres, ev, state })
    }

    fn stmt(&mut self, s: &Stmt) -> R<()> {
        match s {
            Stmt::Lexer(_) => {}
            Stmt::Class(name, items) => {
                let m = self.class(items);
                self.env.insert(name.clone(), V::M(m));
            }
            Stmt::Mask(name, e) => {
                let m = self.mask(e)?;
                self.env.insert(name.clone(), V::M(m));
            }
            Stmt::Reflect { name, a, an, b, bn, reset, start_lane } => {
                let (a, b) = (self.mask(a)?, self.mask(b)?);
                let reset = self.opt(reset)?;
                let names = vec!["out".to_string(), an.clone(), bn.clone()];
                let start = if *start_lane {
                    let lane = &self.hyp.as_ref().ok_or("`start lane` outside hypotheses")?.1;
                    names.iter().position(|x| x == lane).ok_or(format!("lane {lane} is no state of {name}"))?
                } else {
                    0
                };
                // Byte i's map: at a segment's first byte the state is `start` first; then a reset
                // gives 0, a byte of A sends x to 1 - x, a byte of B to 2 - x (mod 3).
                let mut entries = vec![];
                for x in 0..3u8 {
                    let xs = self.cnst(x);
                    let xp = match self.seg() {
                        Some(g) => {
                            let st = self.cnst(start as u8);
                            self.sel(g.first, st, xs)
                        }
                        None => xs,
                    };
                    let via_a = self.lmap(xp, &[1, 0, 2]);
                    let via_b = self.lmap(xp, &[2, 1, 0]);
                    let tb = self.sel(b, via_b, xp);
                    let mut v = self.sel(a, via_a, tb);
                    if let Some(r) = reset {
                        let z = self.cnst(0);
                        v = self.sel(r, z, v);
                    }
                    entries.push(v);
                }
                let state = self.push(Op::MapScan { entries, init: start as u8 });
                let ab = self.or(a, b);
                let ev = match reset {
                    Some(r) => self.andnot(ab, r),
                    None => ab,
                };
                let sc = self.scan(names, vec![], state, ev, start)?;
                self.env.insert(name.clone(), V::S(sc));
            }
            Stmt::Count { name, up, down, reset, max } => {
                let (up, down) = (self.mask(up)?, self.mask(down)?);
                let reset = self.opt(reset)?;
                if *max > 254 {
                    return Err(format!("{name}: max {max} is above 254"));
                }
                let first = self.seg().map(|s| s.first);
                let c = self.push(Op::Count { up, down, reset, first, max: *max as u8 });
                let over = self.ge(c, *max as u8 + 1);
                let msg = format!("{name} exceeds max {max}");
                if self.hyp.is_some() {
                    self.lane_rejects.push((msg, over));
                } else {
                    let msg = format!("{}{msg}", self.prefix);
                    self.error(msg, over);
                }
                self.env.insert(name.clone(), V::C(c, *max as u8));
            }
            Stmt::Machine(name, def) => {
                let sc = self.machine(def)?;
                self.env.insert(name.clone(), V::S(sc));
            }
            Stmt::Block { name, params, body } => {
                self.templates.insert(name.clone(), (params.clone(), body.clone()));
            }
            Stmt::Inst { name, template, args } => self.instance(name, template, args)?,
            Stmt::Hyp { name, split, lanes, body } => self.hypotheses(name, split, lanes, body)?,
            Stmt::Reject(msg, e) => {
                let m = self.mask(e)?;
                if self.hyp.is_none() {
                    return Err("reject outside hypotheses".into());
                }
                self.lane_rejects.push((msg.clone(), m));
            }
            Stmt::Exit(name) => self.lane_exit = Some(name.clone()),
            Stmt::Rounds { n, settle, body } => self.rounds(*n, *settle, body)?,
            Stmt::Carry(..) | Stmt::Next(..) => return Err("carry or next outside rounds".into()),
            Stmt::Ucode { over, classes } => {
                let over = self.mask(over)?;
                let mut sets = vec![];
                for (_, c) in classes {
                    sets.push(Membership::new(&match c {
                        UClass::Points(v) => v.clone(),
                        UClass::Table(file, sym) => self.tables.get(&format!("{file}:{sym}")).ok_or(format!("no table {file}:{sym}"))?.clone(),
                    }));
                }
                let multi = self.push(Op::Ucode { over, classes: sets });
                for (k, (name, _)) in classes.iter().enumerate() {
                    let m = self.push(Op::Pick(multi, k));
                    self.env.insert(name.clone(), V::M(m));
                }
            }
            Stmt::Token { kind, start, through } => {
                let start = self.mask(start)?;
                let through = self.opt(through)?;
                self.tokens.push(TokenDef { kind: kind.clone(), start, through });
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
                self.skip = self.or(self.skip, m);
            }
            Stmt::Error(msg, e) => {
                let m = self.mask(e)?;
                if self.hyp.is_some() {
                    return Err(format!("error {msg:?} inside hypotheses: use reject"));
                }
                let msg = format!("{}{msg}", self.prefix);
                self.error(msg, m);
            }
        }
        Ok(())
    }

    fn instance(&mut self, name: &str, template: &str, args: &[Expr]) -> R<()> {
        let (params, body) = self.templates.get(template).cloned().ok_or(format!("no block {template}"))?;
        if params.len() != args.len() {
            return Err(format!("{name}: {} arguments for {} parameters", args.len(), params.len()));
        }
        let vals: Vec<Id> = args.iter().map(|a| self.mask(a)).collect::<R<_>>()?;
        let mut defined = params.clone();
        body.iter().for_each(|s| s.defines(&mut defined));
        let saved: Vec<(String, Option<V>)> = defined.iter().map(|d| (d.clone(), self.env.remove(d))).collect();
        for (p, v) in params.iter().zip(vals) {
            self.env.insert(p.clone(), V::M(v));
        }
        let new_prefix = format!("{}{name}: ", self.prefix);
        let old_prefix = std::mem::replace(&mut self.prefix, new_prefix);
        let r = self.exec(&body);
        self.prefix = old_prefix;
        r?;
        for d in &defined {
            if let Some(v) = self.env.remove(d) {
                self.env.insert(format!("{name}.{d}"), v);
            }
        }
        for (d, v) in saved {
            if let Some(v) = v {
                self.env.insert(d, v);
            }
        }
        let inst = self.instances.entry(template.to_string()).or_default();
        if !inst.iter().any(|x| x == name) {
            inst.push(name.to_string());
        }
        Ok(())
    }

    fn rounds(&mut self, n: usize, settle: bool, body: &[Stmt]) -> R<()> {
        let carries: Vec<(&String, &Expr)> = body.iter().filter_map(|s| if let Stmt::Carry(a, e) = s { Some((a, e)) } else { None }).collect();
        let nexts: Vec<(&String, &Expr)> = body.iter().filter_map(|s| if let Stmt::Next(a, e) = s { Some((a, e)) } else { None }).collect();
        let label = format!("{}rounds({})", self.prefix, carries.first().map(|c| c.0.as_str()).unwrap_or(""));
        let mut slots: HashMap<String, Id> = HashMap::new();
        for (name, e) in &carries {
            let init = self.mask(e)?;
            let c = self.push(Op::Carry(init));
            slots.insert((*name).clone(), c);
            self.env.insert((*name).clone(), V::M(c));
        }
        let begin = self.push(Op::RoundsBegin);
        let rest: Vec<Stmt> = body.iter().filter(|s| !matches!(s, Stmt::Carry(..) | Stmt::Next(..))).cloned().collect();
        self.exec(&rest)?;
        let mut pairs = vec![];
        for (name, e) in &nexts {
            let m = self.mask(e)?;
            let c = *slots.get(*name).ok_or(format!("next {name}: no carry of that name"))?;
            pairs.push((c, m));
        }
        let end = self.push(Op::RoundsEnd { begin, n, settle, carries: pairs });
        if self.hyp.is_some() {
            self.lane_rejects.push((format!("{label} did not settle"), end));
        } else {
            self.push(Op::Errors { parts: vec![(format!("{label} did not settle in {n}"), end)], all: false, byte: false });
        }
        Ok(())
    }

    fn hypotheses(&mut self, name: &str, split: &Expr, lanes: &[String], body: &[Stmt]) -> R<()> {
        if self.hyp.is_some() {
            return Err("nested hypotheses".into());
        }
        if lanes.len() > SINK as usize {
            return Err(format!("hypotheses {name}: at most {SINK} lanes"));
        }
        let split = self.mask(split)?;
        let p = self.prev_plain(split, 1)?;
        let first = self.or(p, self.file_first);
        let last = self.or(split, self.file_last);
        let seg = Seg { first, last };
        let mut defined = vec![];
        body.iter().for_each(|s| s.defines(&mut defined));
        let saved: Vec<(String, Option<V>)> = defined.iter().map(|d| (d.clone(), self.env.remove(d))).collect();
        let mut lane_vals: Vec<HashMap<String, V>> = vec![];
        let (mut exits, mut rejects) = (vec![], vec![]);
        for lane in lanes {
            self.hyp = Some((seg, lane.clone()));
            self.lane_rejects.clear();
            self.lane_exit = None;
            let r = self.exec(body);
            self.hyp = None;
            r?;
            let exit = self.lane_exit.clone().ok_or(format!("hypotheses {name}: no exit"))?;
            let sc = match self.env.get(&exit) {
                Some(V::S(s)) => s.clone(),
                _ => return Err(format!("exit {exit}: not a scan")),
            };
            let map: Vec<u8> = sc.names.iter().map(|s| lanes.iter().position(|l| l == s).map(|p| p as u8).unwrap_or(SINK)).collect();
            exits.push((sc.state, map));
            rejects.push(std::mem::take(&mut self.lane_rejects));
            let mut vals = HashMap::new();
            for d in &defined {
                if let Some(v) = self.env.remove(d) {
                    vals.insert(d.clone(), v);
                }
            }
            lane_vals.push(vals);
        }
        for (d, v) in saved {
            if let Some(v) = v {
                self.env.insert(d, v);
            }
        }
        let full = format!("{}{name}", self.prefix);
        // Each segment's map over lanes sits on its last byte: lane l goes to the lane its exit
        // scan ends in, or to the first lane if l rejects the segment. A scan of those maps gives
        // the lane after each byte; the lane before a byte is its segment's.
        let mut hits: Vec<Vec<Id>> = vec![];
        let mut entries = vec![];
        for (l, (state, map)) in exits.iter().enumerate() {
            let mut lane_hits = vec![];
            let mut any = self.empty();
            for (_, m) in &rejects[l] {
                let f = self.push(Op::Fill { s: *m, e: first });
                let h = self.and(last, f);
                lane_hits.push(h);
                any = self.or(any, h);
            }
            let to = self.lmap(*state, map);
            let zero = self.cnst(0);
            let next = self.sel(any, zero, to);
            let me = self.cnst(l as u8);
            entries.push(self.sel(last, next, me));
            hits.push(lane_hits);
        }
        let after = self.push(Op::MapScan { entries, init: 0 });
        let lane = self.push(Op::LanePrev { a: after, init: 0 });
        let sel: Vec<Id> = (0..lanes.len()).map(|l| self.eq(lane, l as u8)).collect();
        // A rejected segment's error is its lane's first rejection in order, at its first byte.
        let mut parts = vec![];
        for (l, lane_hits) in hits.iter().enumerate() {
            let mut earlier = self.empty();
            for (j, &h) in lane_hits.iter().enumerate() {
                let pri = self.andnot(h, earlier);
                earlier = self.or(earlier, h);
                let spread = self.push(Op::BackFill { s: pri, e: last });
                let at = self.and(first, sel[l]);
                let m = self.and(at, spread);
                parts.push((format!("{full}: {}", rejects[l][j].0), m));
            }
        }
        self.push(Op::Errors { parts, all: true, byte: true });
        let mut ends = vec![];
        for (l, lane_name) in lanes.iter().enumerate().skip(1) {
            let e = self.eq(after, l as u8);
            let m = self.and(self.file_last, e);
            ends.push((format!("{full}: the file ends in lane {lane_name}"), m));
        }
        self.push(Op::Errors { parts: ends, all: false, byte: false });
        // Every name the body defines: per byte, its value in the byte's lane.
        for d in &defined {
            let vs: Vec<&V> = lane_vals.iter().filter_map(|m| m.get(d)).collect();
            if vs.len() != lanes.len() {
                continue;
            }
            let v = match vs[0] {
                V::M(_) => {
                    let ids: Vec<Id> = vs.iter().map(|v| if let V::M(m) = v { *m } else { unreachable!() }).collect();
                    V::M(self.select_masks(&ids, &sel))
                }
                V::C(_, max) => {
                    let ids: Vec<Id> = vs.iter().map(|v| if let V::C(c, _) = v { *c } else { unreachable!() }).collect();
                    V::C(self.select_lanes(&ids, &sel), *max)
                }
                V::S(s0) => {
                    let scans: Vec<ScanV> = vs.iter().map(|v| if let V::S(s) = v { s.clone() } else { unreachable!() }).collect();
                    let ins = (0..s0.ins.len()).map(|k| self.select_masks(&scans.iter().map(|s| s.ins[k]).collect::<Vec<_>>(), &sel)).collect();
                    let pres = (0..s0.pres.len()).map(|k| self.select_masks(&scans.iter().map(|s| s.pres[k]).collect::<Vec<_>>(), &sel)).collect();
                    let ev = self.select_masks(&scans.iter().map(|s| s.ev).collect::<Vec<_>>(), &sel);
                    let state = self.select_lanes(&scans.iter().map(|s| s.state).collect::<Vec<_>>(), &sel);
                    V::S(ScanV { names: s0.names.clone(), groups: s0.groups.clone(), ins, pres, ev, state })
                }
            };
            self.env.insert(d.clone(), v);
        }
        Ok(())
    }

    fn select_masks(&mut self, ids: &[Id], sel: &[Id]) -> Id {
        let mut out = self.empty();
        for (l, &v) in ids.iter().enumerate() {
            let m = self.and(v, sel[l]);
            out = self.or(out, m);
        }
        out
    }
    fn select_lanes(&mut self, ids: &[Id], sel: &[Id]) -> Id {
        let mut out = ids[0];
        for (l, &v) in ids.iter().enumerate().skip(1) {
            out = self.sel(sel[l], v, out);
        }
        out
    }

    fn machine(&mut self, def: &MachineDef) -> R<ScanV> {
        if self.hyp.is_some() {
            return Err("machines inside hypotheses are not supported".into());
        }
        let events: Vec<Id> = def.events.iter().map(|(_, e)| self.mask(e)).collect::<R<_>>()?;
        let sidx = |s: &str| def.states.iter().position(|x| x == s).ok_or(format!("no state {s}"));
        let ns = def.states.len();
        if ns > SINK as usize {
            return Err(format!("a machine of {ns} states: at most {SINK}"));
        }
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
        for (s, row) in table.iter_mut().enumerate() {
            for t in row.iter_mut() {
                if *t == usize::MAX {
                    *t = s
                }
            }
        }
        if let Some(e) = err_state {
            table[e] = vec![e; nsym];
        }
        // Byte i's map: the first listed event at the byte picks its column of the table.
        let mut entries = vec![];
        for (s, row) in table.iter().enumerate() {
            let mut v = self.cnst(s as u8);
            for k in (0..nsym).rev() {
                let to = self.cnst(row[k] as u8);
                v = self.sel(events[k], to, v);
            }
            entries.push(v);
        }
        let start = sidx(&def.start)?;
        let state = self.push(Op::MapScan { entries, init: start as u8 });
        let mut ev = self.empty();
        for &e in &events {
            ev = self.or(ev, e);
        }
        let mut groups = def.groups.clone();
        groups.push(("__final".into(), def.finals.clone()));
        self.scan(def.states.clone(), groups, state, ev, start)
    }

    fn state_set(sc: &ScanV, names: &[String]) -> R<Vec<bool>> {
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

    fn union(&mut self, ids: &[Id], set: &[bool]) -> Id {
        let mut out = self.empty();
        for (k, &m) in ids.iter().enumerate() {
            if set[k] {
                out = self.or(out, m);
            }
        }
        out
    }

    fn lookup(&self, name: &str) -> R<Vec<V>> {
        if let Some((t, rest)) = name.split_once("*.") {
            let insts = self.instances.get(t).ok_or(format!("no instances of {t}"))?;
            return insts.iter().map(|i| self.env.get(&format!("{i}.{rest}")).cloned().ok_or(format!("no {i}.{rest}"))).collect();
        }
        Ok(vec![self.env.get(name).cloned().ok_or(format!("undefined: {name}"))?])
    }

    /// `C.frame(m)` / `C.since(m)`: per level `L` up to the bound, whether the level-`L` frame open
    /// now was opened by a byte of `m` (`frame`), or has had a byte of `m` at its own level
    /// (`since`). A frame opens where the count rises into `L` and closes where it falls below.
    fn frame(&mut self, c: Id, max: u8, m: Id, since: bool) -> R<Id> {
        let p = self.push(Op::LanePrev { a: c, init: 0 });
        let before = match self.seg() {
            Some(g) => {
                let z = self.cnst(0);
                self.sel(g.first, z, p)
            }
            None => p,
        };
        let mut out = self.empty();
        for level in 1..=max + 1 {
            let at = self.eq(c, level);
            let was = self.eq(before, level - 1);
            let open = self.and(at, was);
            let ge = self.ge(c, level);
            let drop = self.not(ge);
            let x = if since {
                let stay = self.eq(before, level);
                let s0 = self.and(at, stay);
                let s = self.and(s0, m);
                let e = self.or(open, drop);
                self.fill(s, e)
            } else {
                let s = self.and(open, m);
                let other = self.andnot(open, m);
                let e = self.or(other, drop);
                self.fill(s, e)
            };
            let here = self.and(at, x);
            out = self.or(out, here);
        }
        self.prev(out, 1)
    }

    /// `head(a, C)`: the first `C` bytes of each run of `a`, `C` read before the run; per value
    /// `v` up to the bound, the runs whose count is `v`, by a fill, and their first `v` bytes.
    fn head_count(&mut self, a: Id, c: Id, max: u8) -> R<Id> {
        let p = self.prev(a, 1)?;
        let rs = self.andnot(a, p);
        let before = self.push(Op::LanePrev { a: c, init: 0 });
        let na = self.not(a);
        let mut out = self.empty();
        for v in 1..=max + 1 {
            let is_v = self.eq(before, v);
            let s = self.and(rs, is_v);
            let other = self.andnot(rs, is_v);
            let e = self.or(na, other);
            let runs = self.push(Op::Fill { s, e });
            let h = self.head(a, v as usize)?;
            let hit = self.and(runs, h);
            out = self.or(out, hit);
        }
        Ok(out)
    }

    /// The first `k` bytes of each run: not all of the `k` bytes before are in it.
    fn head(&mut self, a: Id, k: usize) -> R<Id> {
        if k == 0 {
            return Ok(self.empty());
        }
        let mut all = self.prev(a, 1)?;
        for j in 2..=k {
            let p = self.prev(a, j)?;
            all = self.and(all, p);
        }
        Ok(self.andnot(a, all))
    }

    fn mask(&mut self, e: &Expr) -> R<Id> {
        Ok(match e {
            Expr::Empty => self.empty(),
            Expr::Name(nm) => {
                if let Some((g, _)) = &self.hyp {
                    if nm == "first" {
                        return Ok(g.first);
                    }
                    if nm == "last" {
                        return Ok(g.last);
                    }
                }
                let vs = self.lookup(nm)?;
                let mut out: Option<Id> = None;
                for v in vs {
                    match v {
                        V::M(m) => out = Some(match out { None => m, Some(o) => self.or(o, m) }),
                        _ => return Err(format!("{nm} is not a mask")),
                    }
                }
                match out {
                    Some(o) => o,
                    None => self.empty(),
                }
            }
            Expr::Not(a) => {
                let a = self.mask(a)?;
                self.not(a)
            }
            Expr::And(a, b) => {
                let (a, b) = (self.mask(a)?, self.mask(b)?);
                self.and(a, b)
            }
            Expr::Or(a, b) => {
                let (a, b) = (self.mask(a)?, self.mask(b)?);
                self.or(a, b)
            }
            Expr::Xor(a, b) => {
                let (a, b) = (self.mask(a)?, self.mask(b)?);
                self.push(Op::Xor(a, b))
            }
            Expr::Test { target, method, from, to, k, other, arg } => {
                let vs = self.lookup(target)?;
                let mut out: Option<Id> = None;
                for v in vs {
                    let m = match (&v, method.as_str()) {
                        (V::S(sc), "in") => {
                            let set = Self::state_set(sc, from)?;
                            self.union(&sc.ins, &set)
                        }
                        (V::S(sc), "on") => {
                            let (a, b) = (Self::state_set(sc, from)?, Self::state_set(sc, to)?);
                            let pre = self.union(&sc.pres, &a);
                            let st = self.union(&sc.ins, &b);
                            let x = self.and(sc.ev, pre);
                            self.and(x, st)
                        }
                        (V::S(sc), "final") => {
                            let set = Self::state_set(sc, from)?;
                            let u = self.union(&sc.ins, &set);
                            self.at_end(u)
                        }
                        (V::S(sc), "reached") => {
                            let set = Self::state_set(sc, from)?;
                            let u = self.union(&sc.ins, &set);
                            self.in_files(u)
                        }
                        (V::C(c, _), "eq" | "ge") => match other {
                            Some(o) => match self.env.get(o).cloned() {
                                Some(V::C(oc, _)) if method == "eq" => self.push(Op::LaneEqLanes(*c, oc)),
                                _ => return Err(format!("{target}.{method}({o}): needs a count, and eq")),
                            },
                            None => {
                                let kk = k.unwrap_or(0);
                                if kk > 255 {
                                    return Err(format!("{target}.{method}({kk}): above 255"));
                                }
                                if method == "ge" { self.ge(*c, kk as u8) } else { self.eq(*c, kk as u8) }
                            }
                        },
                        (V::C(c, max), "frame" | "since") => {
                            let m = self.mask(arg.as_ref().ok_or("frame/since needs a mask")?)?;
                            self.frame(*c, *max, m, method == "since")?
                        }
                        _ => return Err(format!("{target}.{method}: wrong kind of value")),
                    };
                    out = Some(match out { None => m, Some(o) => self.or(o, m) });
                }
                match out {
                    Some(o) => o,
                    None => self.empty(),
                }
            }
            Expr::Call(f, args) => self.call(f, args)?,
        })
    }

    fn call(&mut self, f: &str, args: &[Arg]) -> R<Id> {
        let ea = |me: &mut Self, i: usize| -> R<Id> {
            match args.get(i) {
                Some(Arg::E(e)) => me.mask(e),
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
        Ok(match f {
            "prev" => {
                let a = ea(self, 0)?;
                self.prev(a, na(1, 1)?)?
            }
            "next" => {
                let a = ea(self, 0)?;
                self.next(a, na(1, 1)?)?
            }
            "starts" => {
                let a = ea(self, 0)?;
                let p = self.prev(a, 1)?;
                self.andnot(a, p)
            }
            "lasts" => {
                let a = ea(self, 0)?;
                let n = self.next(a, 1)?;
                self.andnot(a, n)
            }
            "after" => {
                let a = ea(self, 0)?;
                let p = self.prev(a, 1)?;
                self.andnot(p, a)
            }
            "span" => {
                let (a, k) = (ea(self, 0)?, na(1, 1)?);
                let mut out = a;
                for j in 1..k {
                    let p = self.prev(a, j)?;
                    out = self.or(out, p);
                }
                out
            }
            "toggle" => {
                let a = ea(self, 0)?;
                self.toggle(a)?
            }
            "fill" => {
                let (s, e) = (ea(self, 0)?, ea(self, 1)?);
                self.fill(s, e)
            }
            "back_fill" => {
                let (s, e) = (ea(self, 0)?, ea(self, 1)?);
                self.back_fill(s, e)
            }
            "after_odd_run" => {
                // The byte after a run whose last byte is at an even offset in it.
                let a = ea(self, 0)?;
                let even = self.aligned(a, 2)?;
                let p = self.prev(even, 1)?;
                self.andnot(p, a)
            }
            "text" => {
                // Not cut by hypothesis segments: the evaluator reads the file's bytes.
                let Some(Arg::S(t)) = args.first() else { return Err("text needs a string".into()) };
                let mut out = self.push(Op::All);
                for (j, &b) in t.iter().enumerate() {
                    let c = self.class(&[(b as u32, b as u32)]);
                    let s = self.next_plain(c, j)?;
                    out = self.and(out, s);
                }
                out
            }
            "aligned" => {
                let a = ea(self, 0)?;
                self.aligned(a, na(1, 1)? as u32)?
            }
            "head" => {
                let a = ea(self, 0)?;
                if let Some(Arg::E(Expr::Name(c))) = args.get(1) {
                    if let Some(V::C(c, max)) = self.env.get(c).cloned() {
                        return self.head_count(a, c, max);
                    }
                }
                self.head(a, na(1, 1)?)?
            }
            "line_has" => {
                // Not cut by hypothesis segments, as in the evaluator: a fill to each line end.
                let (a, nl) = (ea(self, 0)?, ea(self, 1)?);
                let s = self.andnot(a, nl);
                let f = self.push(Op::Fill { s, e: nl });
                let p = self.prev_plain(f, 1)?;
                let x = self.or(a, p);
                self.and(nl, x)
            }
            "at_end" => {
                let a = ea(self, 0)?;
                self.at_end(a)
            }
            _ => return Err(format!("unknown operation {f}")),
        })
    }
}
