//! A Bitlex lexer as a stream program (`docs/parser-generators/09-compile-and-lower.md` §3): a list
//! of operations over byte masks of a space of files (`space::NodeSpace`, a byte per node), each
//! operation one value, in the shape of the prefilter's set program (`prefilter/sets.rs` `Op`)
//! and the Metal set steps (`metal_set_steps.rs`), whose operations are the first seven here.
//!
//! Lowering expands what the reference evaluator (`reference.rs`) interprets: blocks become
//! prefixed copies, each hypothesis lane gets its own copy of the body, `rounds` become a loop
//! over a range of operations with explicit carries. Scans become masks: a machine's or a
//! reflection's state per byte is one mask per state, and what the evaluator reads off a scan
//! (`in`, `on`, `final`, `reached`) is logic over those masks.

use super::syntax::{Arg, Expr, MachineDef, Stmt, UClass};
use std::collections::HashMap;

pub(in crate::code_rule_dsl) type Id = usize;

/// Where shifts and scans restart: a segment's first bytes and its last bytes. Outside a
/// `hypotheses` block the segments are the files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Seg {
    pub first: Id,
    pub last: Id,
}

#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) enum Op {
    // The set program's operations (`prefilter/sets.rs`, `SetStep`).
    Empty,
    All,
    Or(Id, Id),
    And(Id, Id),
    AndNot(Id, Id),
    /// `scope`'s bytes, in the files where `test` has a byte.
    Within(Id, Id),
    // Logic the set program does not need.
    Xor(Id, Id),
    Not(Id),
    // Maps: bytes to masks.
    /// Each file's first byte.
    FileFirst,
    /// Each file's last byte.
    FileLast,
    /// Bytes in any of the ranges.
    Class(Vec<(u32, u32)>),
    /// Code points from the bytes in `over`, decoded as UTF-8; a mask per class of ranges.
    Ucode { over: Id, classes: Vec<Vec<(u32, u32)>> },
    /// Value `k` of a multi-mask operation.
    Pick(Id, usize),
    // Shifts, cut at segment edges.
    Prev { a: Id, k: usize, seg: Option<Seg> },
    Next { a: Id, k: usize, seg: Option<Seg> },
    // Scans: a carry crosses blocks.
    Toggle { a: Id, seg: Seg },
    Fill { s: Id, e: Id, seg: Seg },
    BackFill { s: Id, e: Id, seg: Seg },
    /// Bytes of `a` whose offset in their run is a multiple of `k`.
    Aligned { a: Id, k: u32, seg: Seg },
    /// The lines (ended by `nl`) that hold a byte of `a`: their `nl` byte.
    LineHas { a: Id, nl: Id, seg: Seg },
    /// A table walk over event bytes: a mask per state (bytes after which the walk is in it),
    /// then the event mask.
    Machine { events: Vec<Id>, table: Vec<Vec<usize>>, start: usize, seg: Seg },
    /// A reflection's three states per byte, then the event mask (`03-bitlex.md` §2.7).
    Reflect { a: Id, b: Id, reset: Option<Id>, start: u8, seg: Seg },
    // Counts: an integer per byte.
    Count { up: Id, down: Id, reset: Option<Id>, seg: Seg },
    CountTest { c: Id, k: i32, other: Option<Id>, ge: bool },
    /// Bytes whose innermost open frame was opened by a byte of `m` (`since`: has seen one).
    Frame { c: Id, m: Id, since: bool, seg: Seg },
    /// The first `c` bytes of each run of `a`, `c` read just before the run.
    HeadCount { a: Id, c: Id, seg: Seg },
    /// Per byte, the count of the lane its segment is in.
    CountSelect(Vec<(Id, Id)>),
    // Control and outputs.
    /// A carry of a `rounds` loop: `init` on entry, then the loop's `next` each round.
    Carry(Id),
    RoundsBegin,
    /// The end of a loop from `begin` (a `RoundsBegin`): repeats while a carry changed, at most
    /// `n` times. Its value is the bytes that still changed after the last round.
    RoundsEnd { begin: usize, n: usize, carries: Vec<(Id, Id)>, label: String, in_hyp: bool },
    /// The lane each segment is in (a mask per lane), and the errors of the walk.
    Hyp(Box<HypOp>),
    /// An error at each file's first byte of `a`.
    Error(String, Id),
}

#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct HypOp {
    pub name: String,
    pub lanes: Vec<String>,
    pub first: Id,
    /// Per lane: the exit scan's state masks, and the lane each state continues in.
    pub exits: Vec<(Vec<Id>, Vec<Option<usize>>)>,
    /// Per lane: its rejections in order.
    pub rejects: Vec<Vec<(String, Id)>>,
}

impl Op {
    /// The values the operation reads.
    pub(in crate::code_rule_dsl) fn inputs(&self) -> Vec<Id> {
        let sg = |s: &Seg| [s.first, s.last];
        let oseg = |s: &Option<Seg>| s.map(|s| vec![s.first, s.last]).unwrap_or_default();
        match self {
            Op::Empty | Op::All | Op::FileFirst | Op::FileLast | Op::Class(_) | Op::RoundsBegin => vec![],
            Op::Or(a, b) | Op::And(a, b) | Op::AndNot(a, b) | Op::Within(a, b) | Op::Xor(a, b) => vec![*a, *b],
            Op::Not(a) | Op::Pick(a, _) | Op::Carry(a) | Op::Error(_, a) => vec![*a],
            Op::Ucode { over, .. } => vec![*over],
            Op::Prev { a, seg, .. } | Op::Next { a, seg, .. } => [vec![*a], oseg(seg)].concat(),
            Op::Toggle { a, seg } | Op::Aligned { a, seg, .. } => [vec![*a], sg(seg).to_vec()].concat(),
            Op::Fill { s, e, seg } | Op::BackFill { s, e, seg } => [vec![*s, *e], sg(seg).to_vec()].concat(),
            Op::LineHas { a, nl, seg } => [vec![*a, *nl], sg(seg).to_vec()].concat(),
            Op::Machine { events, seg, .. } => [events.clone(), sg(seg).to_vec()].concat(),
            Op::Reflect { a, b, reset, seg, .. } => [vec![*a, *b], reset.iter().copied().collect(), sg(seg).to_vec()].concat(),
            Op::Count { up, down, reset, seg } => [vec![*up, *down], reset.iter().copied().collect(), sg(seg).to_vec()].concat(),
            Op::CountTest { c, other, .. } => [vec![*c], other.iter().copied().collect()].concat(),
            Op::Frame { c, m, seg, .. } => [vec![*c, *m], sg(seg).to_vec()].concat(),
            Op::HeadCount { a, c, seg } => [vec![*a, *c], sg(seg).to_vec()].concat(),
            Op::CountSelect(v) => v.iter().flat_map(|&(c, s)| [c, s]).collect(),
            Op::RoundsEnd { carries, .. } => carries.iter().flat_map(|&(c, n)| [c, n]).collect(),
            Op::Hyp(h) => {
                let mut v = vec![h.first];
                for (ins, _) in &h.exits {
                    v.extend(ins);
                }
                for r in &h.rejects {
                    v.extend(r.iter().map(|x| x.1));
                }
                v
            }
        }
    }
}

/// A token kind's starts and the bytes it may continue through.
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct TokenDef {
    pub kind: String,
    pub start: Id,
    pub through: Option<Id>,
}

#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct Program {
    pub ops: Vec<Op>,
    /// Operations whose value nothing needs (`live` is false) are skipped.
    pub live: Vec<bool>,
    pub tokens: Vec<TokenDef>,
    pub munch: Vec<(String, Id, Vec<(Vec<u8>, Option<String>)>)>,
    pub keywords: Vec<(String, Vec<(Vec<u8>, Option<String>)>, Vec<Vec<u8>>, bool)>,
    pub skip: Id,
}

impl Program {
    /// Live operations: what the token stage reads, the errors, and every loop's carries.
    fn mark_live(&mut self) {
        let mut live = vec![false; self.ops.len()];
        let mut stack: Vec<Id> = vec![self.skip];
        stack.extend(self.tokens.iter().flat_map(|t| std::iter::once(t.start).chain(t.through)));
        stack.extend(self.munch.iter().map(|m| m.1));
        for (id, op) in self.ops.iter().enumerate() {
            if matches!(op, Op::Error(..) | Op::Hyp(_) | Op::RoundsBegin | Op::RoundsEnd { .. }) {
                stack.push(id);
            }
        }
        while let Some(id) = stack.pop() {
            if !live[id] {
                live[id] = true;
                stack.extend(self.ops[id].inputs());
            }
        }
        self.live = live;
    }

    /// Live operations: a cost measure, one operation over every block.
    pub(in crate::code_rule_dsl) fn live_ops(&self) -> usize {
        self.live.iter().filter(|&&l| l).count()
    }

    /// For each value, the operation after which nothing reads it: its last reader, or the end of
    /// the outermost loop that reads it without defining it.
    pub(in crate::code_rule_dsl) fn last_uses(&self) -> Vec<usize> {
        let loops: Vec<(usize, usize)> = self
            .ops
            .iter()
            .enumerate()
            .filter_map(|(e, op)| if let Op::RoundsEnd { begin, .. } = op { Some((*begin, e)) } else { None })
            .collect();
        let mut last: Vec<usize> = (0..self.ops.len()).collect();
        for (u, op) in self.ops.iter().enumerate() {
            if !self.live[u] {
                continue;
            }
            for d in op.inputs() {
                let mut at = u;
                for &(b, e) in &loops {
                    if b <= u && u <= e && !(b <= d && d <= e) {
                        at = at.max(e);
                    }
                }
                last[d] = last[d].max(at);
            }
        }
        // Carries are written at their loop's end.
        for (e, op) in self.ops.iter().enumerate() {
            if let Op::RoundsEnd { carries, .. } = op {
                for &(c, n) in carries {
                    last[c] = last[c].max(e);
                    last[n] = last[n].max(e);
                }
            }
        }
        let end = self.ops.len();
        for t in &self.tokens {
            last[t.start] = end;
            if let Some(x) = t.through {
                last[x] = end;
            }
        }
        for m in &self.munch {
            last[m.1] = end;
        }
        last[self.skip] = end;
        last
    }
}

/// A scan's value: a mask per state of the bytes after which it is in that state, a mask per
/// state of the bytes before which it was, and the event bytes.
#[derive(Clone, Debug)]
struct ScanV {
    names: Vec<String>,
    groups: Vec<(String, Vec<String>)>,
    ins: Vec<Id>,
    pres: Vec<Id>,
    ev: Id,
}

#[derive(Clone, Debug)]
enum V {
    M(Id),
    S(ScanV),
    C(Id),
}

struct HypSeg {
    seg: Seg,
    lane: String,
}

struct Lower<'a> {
    ops: Vec<Op>,
    memo: HashMap<String, Id>,
    env: HashMap<String, V>,
    templates: HashMap<String, (Vec<String>, Vec<Stmt>)>,
    instances: HashMap<String, Vec<String>>,
    file: Seg,
    hyp: Option<HypSeg>,
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

/// Lowers a parsed lexer. `tables` are the code point tables `ucode` classes name.
pub(in crate::code_rule_dsl) fn lower(prog: &[Stmt], tables: &HashMap<String, Vec<(u32, u32)>>) -> R<Program> {
    let mut l = Lower {
        ops: vec![],
        memo: HashMap::new(),
        env: HashMap::new(),
        templates: HashMap::new(),
        instances: HashMap::new(),
        file: Seg { first: 0, last: 0 },
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
    let first = l.push(Op::FileFirst);
    let last = l.push(Op::FileLast);
    l.file = Seg { first, last };
    l.skip = l.push(Op::Empty);
    l.exec(prog)?;
    let mut p = Program { ops: l.ops, live: vec![], tokens: l.tokens, munch: l.munch, keywords: l.keywords, skip: l.skip };
    p.mark_live();
    Ok(p)
}

impl Lower<'_> {
    /// One operation; an identical one already pushed is shared.
    fn push(&mut self, op: Op) -> Id {
        // Operations are shared by their text: inputs are ids, so equal text is an equal value.
        // Carries and control are never shared.
        let shareable = !matches!(op, Op::Carry(_) | Op::RoundsBegin | Op::RoundsEnd { .. } | Op::Hyp(_) | Op::Error(..));
        let key = shareable.then(|| format!("{op:?}"));
        if let Some(k) = &key {
            if let Some(&id) = self.memo.get(k) {
                return id;
            }
        }
        self.ops.push(op);
        let id = self.ops.len() - 1;
        if let Some(k) = key {
            self.memo.insert(k, id);
        }
        id
    }

    fn seg(&self) -> Seg {
        self.hyp.as_ref().map(|h| h.seg).unwrap_or(self.file)
    }

    fn or(&mut self, a: Id, b: Id) -> Id {
        self.push(Op::Or(a, b))
    }
    fn and(&mut self, a: Id, b: Id) -> Id {
        self.push(Op::And(a, b))
    }
    fn prev(&mut self, a: Id, k: usize) -> Id {
        let seg = Some(self.seg());
        if k == 0 { a } else { self.push(Op::Prev { a, k, seg }) }
    }
    fn next(&mut self, a: Id, k: usize) -> Id {
        let seg = Some(self.seg());
        if k == 0 { a } else { self.push(Op::Next { a, k, seg }) }
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

    /// A scan's `pre` masks from its state masks: the state before each byte.
    fn scan(&mut self, names: Vec<String>, groups: Vec<(String, Vec<String>)>, multi: Id, n: usize, start: usize, seg: Seg) -> ScanV {
        let ins: Vec<Id> = (0..n).map(|k| self.push(Op::Pick(multi, k))).collect();
        let ev = self.push(Op::Pick(multi, n));
        let pres = ins
            .iter()
            .enumerate()
            .map(|(s, &m)| {
                let p = self.push(Op::Prev { a: m, k: 1, seg: Some(seg) });
                if s == start { self.or(p, seg.first) } else { p }
            })
            .collect();
        ScanV { names, groups, ins, pres, ev }
    }

    fn stmt(&mut self, s: &Stmt) -> R<()> {
        match s {
            Stmt::Lexer(_) => {}
            Stmt::Class(name, items) => {
                let m = self.push(Op::Class(items.clone()));
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
                    let h = self.hyp.as_ref().ok_or("`start lane` outside hypotheses")?;
                    names.iter().position(|x| *x == h.lane).ok_or(format!("lane {} is no state of {name}", h.lane))?
                } else {
                    0
                };
                let seg = self.seg();
                let multi = self.push(Op::Reflect { a, b, reset, start: start as u8, seg });
                let sc = self.scan(names, vec![], multi, 3, start, seg);
                self.env.insert(name.clone(), V::S(sc));
            }
            Stmt::Count { name, up, down, reset } => {
                let (up, down) = (self.mask(up)?, self.mask(down)?);
                let reset = self.opt(reset)?;
                let seg = self.seg();
                let c = self.push(Op::Count { up, down, reset, seg });
                self.env.insert(name.clone(), V::C(c));
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
            Stmt::Rounds { n, body } => self.rounds(*n, body)?,
            Stmt::Carry(..) | Stmt::Next(..) => return Err("carry or next outside rounds".into()),
            Stmt::Ucode { over, classes } => {
                let over = self.mask(over)?;
                let mut sets = vec![];
                for (_, c) in classes {
                    sets.push(match c {
                        UClass::Points(v) => v.clone(),
                        UClass::Table(file, sym) => self.tables.get(&format!("{file}:{sym}")).ok_or(format!("no table {file}:{sym}"))?.clone(),
                    });
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
                let msg = format!("{}{msg}", self.prefix);
                self.push(Op::Error(msg, m));
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

    fn rounds(&mut self, n: usize, body: &[Stmt]) -> R<()> {
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
        // Sharing across a loop's edge is sound: an operation's inputs are ids, so an operation
        // shared from inside a loop reads no carry it would not read if recomputed.
        let begin = self.push(Op::RoundsBegin);
        let rest: Vec<Stmt> = body.iter().filter(|s| !matches!(s, Stmt::Carry(..) | Stmt::Next(..))).cloned().collect();
        self.exec(&rest)?;
        let mut pairs = vec![];
        for (name, e) in &nexts {
            let m = self.mask(e)?;
            let c = *slots.get(*name).ok_or(format!("next {name}: no carry of that name"))?;
            pairs.push((c, m));
        }
        let in_hyp = self.hyp.is_some();
        let end = self.push(Op::RoundsEnd { begin, n, carries: pairs, label: label.clone(), in_hyp });
        if in_hyp {
            self.lane_rejects.push((format!("{label} did not settle"), end));
        }
        Ok(())
    }

    fn hypotheses(&mut self, name: &str, split: &Expr, lanes: &[String], body: &[Stmt]) -> R<()> {
        if self.hyp.is_some() {
            return Err("nested hypotheses".into());
        }
        let split = self.mask(split)?;
        let p = self.prev(split, 1);
        let first = self.or(p, self.file.first);
        let last = self.or(split, self.file.last);
        let seg = Seg { first, last };
        let mut defined = vec![];
        body.iter().for_each(|s| s.defines(&mut defined));
        let saved: Vec<(String, Option<V>)> = defined.iter().map(|d| (d.clone(), self.env.remove(d))).collect();
        let mut lane_vals: Vec<HashMap<String, V>> = vec![];
        let (mut exits, mut rejects) = (vec![], vec![]);
        for lane in lanes {
            self.hyp = Some(HypSeg { seg, lane: lane.clone() });
            self.lane_rejects.clear();
            self.lane_exit = None;
            // Lanes share what they compute alike; `start lane` makes their scans differ.
            let r = self.exec(body);
            self.hyp = None;
            r?;
            let exit = self.lane_exit.clone().ok_or(format!("hypotheses {name}: no exit"))?;
            let sc = match self.env.get(&exit) {
                Some(V::S(s)) => s.clone(),
                _ => return Err(format!("exit {exit}: not a scan")),
            };
            let map = sc.names.iter().map(|s| lanes.iter().position(|l| l == s)).collect();
            exits.push((sc.ins.clone(), map));
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
        let hyp = self.push(Op::Hyp(Box::new(HypOp { name: format!("{}{name}", self.prefix), lanes: lanes.to_vec(), first, exits, rejects })));
        let sel: Vec<Id> = (0..lanes.len()).map(|l| self.push(Op::Pick(hyp, l))).collect();
        let select = |me: &mut Self, ids: &[Id]| -> Id {
            let mut out = me.push(Op::Empty);
            for (l, &v) in ids.iter().enumerate() {
                let m = me.and(v, sel[l]);
                out = me.or(out, m);
            }
            out
        };
        for d in &defined {
            let vs: Vec<&V> = lane_vals.iter().filter_map(|m| m.get(d)).collect();
            if vs.len() != lanes.len() {
                continue;
            }
            let v = match vs[0] {
                V::M(_) => {
                    let ids: Vec<Id> = vs.iter().map(|v| if let V::M(m) = v { *m } else { unreachable!() }).collect();
                    V::M(select(self, &ids))
                }
                V::C(_) => {
                    let pairs = vs.iter().enumerate().map(|(l, v)| (if let V::C(c) = v { *c } else { unreachable!() }, sel[l])).collect();
                    V::C(self.push(Op::CountSelect(pairs)))
                }
                V::S(s0) => {
                    let scans: Vec<&ScanV> = vs.iter().map(|v| if let V::S(s) = v { s } else { unreachable!() }).collect();
                    let per = |me: &mut Self, f: &dyn Fn(&ScanV) -> Id| {
                        let ids: Vec<Id> = scans.iter().map(|s| f(s)).collect();
                        select(me, &ids)
                    };
                    let ins = (0..s0.ins.len()).map(|k| per(self, &|s| s.ins[k])).collect();
                    let pres = (0..s0.pres.len()).map(|k| per(self, &|s| s.pres[k])).collect();
                    let ev = per(self, &|s| s.ev);
                    V::S(ScanV { names: s0.names.clone(), groups: s0.groups.clone(), ins, pres, ev })
                }
            };
            self.env.insert(d.clone(), v);
        }
        Ok(())
    }

    fn machine(&mut self, def: &MachineDef) -> R<ScanV> {
        if self.hyp.is_some() {
            return Err("machines inside hypotheses are not supported".into());
        }
        let events: Vec<Id> = def.events.iter().map(|(_, e)| self.mask(e)).collect::<R<_>>()?;
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
        let start = sidx(&def.start)?;
        let seg = self.file;
        let multi = self.push(Op::Machine { events, table, start, seg });
        let mut groups = def.groups.clone();
        groups.push(("__final".into(), def.finals.clone()));
        Ok(self.scan(def.states.clone(), groups, multi, ns, start, seg))
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
        let mut out = self.push(Op::Empty);
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

    fn mask(&mut self, e: &Expr) -> R<Id> {
        Ok(match e {
            Expr::Empty => self.push(Op::Empty),
            Expr::Name(nm) => {
                if let Some(h) = &self.hyp {
                    if nm == "first" {
                        return Ok(h.seg.first);
                    }
                    if nm == "last" {
                        return Ok(h.seg.last);
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
                    None => self.push(Op::Empty),
                }
            }
            Expr::Not(a) => {
                let a = self.mask(a)?;
                self.push(Op::Not(a))
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
                            let at = self.and(u, self.file.last);
                            let all = self.push(Op::All);
                            self.push(Op::Within(all, at))
                        }
                        (V::S(sc), "reached") => {
                            let set = Self::state_set(sc, from)?;
                            let u = self.union(&sc.ins, &set);
                            let all = self.push(Op::All);
                            self.push(Op::Within(all, u))
                        }
                        (V::C(c), "eq" | "ge") => {
                            let o = match other {
                                Some(o) => match self.env.get(o) {
                                    Some(V::C(oc)) => Some(*oc),
                                    _ => return Err(format!("{o} is not a count")),
                                },
                                None => None,
                            };
                            self.push(Op::CountTest { c: *c, k: k.unwrap_or(0) as i32, other: o, ge: method == "ge" })
                        }
                        (V::C(c), "frame" | "since") => {
                            let m = self.mask(arg.as_ref().ok_or("frame/since needs a mask")?)?;
                            let seg = self.seg();
                            self.push(Op::Frame { c: *c, m, since: method == "since", seg })
                        }
                        _ => return Err(format!("{target}.{method}: wrong kind of value")),
                    };
                    out = Some(match out { None => m, Some(o) => self.or(o, m) });
                }
                match out {
                    Some(o) => o,
                    None => self.push(Op::Empty),
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
        let seg = self.seg();
        Ok(match f {
            "prev" => {
                let a = ea(self, 0)?;
                self.prev(a, na(1, 1)?)
            }
            "next" => {
                let a = ea(self, 0)?;
                self.next(a, na(1, 1)?)
            }
            "starts" => {
                let a = ea(self, 0)?;
                let p = self.prev(a, 1);
                self.push(Op::AndNot(a, p))
            }
            "lasts" => {
                let a = ea(self, 0)?;
                let n = self.next(a, 1);
                self.push(Op::AndNot(a, n))
            }
            "after" => {
                let a = ea(self, 0)?;
                let p = self.prev(a, 1);
                self.push(Op::AndNot(p, a))
            }
            "span" => {
                let (a, k) = (ea(self, 0)?, na(1, 1)?);
                let mut out = a;
                for j in 1..k {
                    let p = self.prev(a, j);
                    out = self.or(out, p);
                }
                out
            }
            "toggle" => {
                let a = ea(self, 0)?;
                self.push(Op::Toggle { a, seg })
            }
            "fill" => {
                let (s, e) = (ea(self, 0)?, ea(self, 1)?);
                self.push(Op::Fill { s, e, seg })
            }
            "back_fill" => {
                let (s, e) = (ea(self, 0)?, ea(self, 1)?);
                self.push(Op::BackFill { s, e, seg })
            }
            "after_odd_run" => {
                // The byte after a run whose last byte is at an even offset in it.
                let a = ea(self, 0)?;
                let even = self.push(Op::Aligned { a, k: 2, seg });
                let p = self.prev(even, 1);
                self.push(Op::AndNot(p, a))
            }
            "text" => {
                let Some(Arg::S(t)) = args.first() else { return Err("text needs a string".into()) };
                // Not cut by hypothesis segments: the evaluator reads the file's bytes.
                let file = Some(self.file);
                let mut out = self.push(Op::All);
                for (j, &b) in t.iter().enumerate() {
                    let c = self.push(Op::Class(vec![(b as u32, b as u32)]));
                    let s = if j == 0 { c } else { self.push(Op::Next { a: c, k: j, seg: file }) };
                    out = self.and(out, s);
                }
                out
            }
            "aligned" => {
                let a = ea(self, 0)?;
                self.push(Op::Aligned { a, k: na(1, 1)? as u32, seg })
            }
            "head" => {
                let a = ea(self, 0)?;
                if let Some(Arg::E(Expr::Name(c))) = args.get(1) {
                    if let Some(V::C(c)) = self.env.get(c).cloned() {
                        return Ok(self.push(Op::HeadCount { a, c, seg }));
                    }
                }
                // The first k bytes of each run: not all of the k bytes before are in it.
                let k = na(1, 1)?;
                if k == 0 {
                    return Ok(self.push(Op::Empty));
                }
                let mut all = self.prev(a, 1);
                for j in 2..=k {
                    let p = self.prev(a, j);
                    all = self.and(all, p);
                }
                self.push(Op::AndNot(a, all))
            }
            "line_has" => {
                let (a, nl) = (ea(self, 0)?, ea(self, 1)?);
                let file = self.file;
                self.push(Op::LineHas { a, nl, seg: file })
            }
            "at_end" => {
                let a = ea(self, 0)?;
                let at = self.and(a, self.file.last);
                let all = self.push(Op::All);
                self.push(Op::Within(all, at))
            }
            _ => return Err(format!("unknown operation {f}")),
        })
    }
}
