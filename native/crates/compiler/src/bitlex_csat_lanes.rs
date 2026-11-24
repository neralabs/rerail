//! The device's search (`bitlex_csat_gpu.uglykl`, decision 0670, step 3) on the CPU, step for step: the same rounds
//! (a round's literals, their watch lists, their gates 32 at a time, what a step implies placed in lane order), the
//! same decisions (the integer activities' heap), the same analysis, jumps back and reductions. A formula's answer,
//! its model and every count are the kernel's on any backend, so a caller may take either's: the prover's answers
//! then do not depend on whether the machine has a device.
//!
//! The kernel runs a SIMD group's lanes at once; here a step's lanes are taken in lane order, each reading the state
//! as the step began where the kernel's do (a gate's check, a literal's value before the step's writes). Each function
//! names the kernel's it follows. Its working state is its own, as the kernel's is: the formula is read where it was
//! built (decision 0353).

use crate::bitlex_csat::{Fanout, Gates, Kind};
use crate::bitlex_sat::{Clauses, Lit};

/// A formula: its gates (the first `base` listed by `fan`, the rest a solve's own, listed by `own`), its plain
/// clauses, its variables (numbered from 1), and its budget of conflicts: with none, it stops at its first decision
/// (propagation at level 0 alone).
pub struct Formula<'a> {
    pub vars: usize,
    pub gates: &'a Gates,
    pub base: usize,
    pub fan: &'a Fanout,
    pub own: Option<&'a Fanout>,
    pub clauses: &'a Clauses,
    pub conflicts: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// A model: each variable's value (`[0]` unused).
    Sat(Vec<bool>),
    Unsat,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Solved {
    pub answer: Answer,
    /// Unknown because the learned clauses' room is full (else the budget, or the steps, ran out).
    pub roomless: bool,
    pub conflicts: u32,
    pub decisions: u32,
    pub rounds: u32,
    /// The clauses it holds learned at the end and their literals.
    pub learned: u32,
    pub learned_literals: u32,
    pub visits: u32,
    pub items: u32,
    pub reductions: u32,
    /// The device's cycles by part, over 256 (0 where its backend has no clock, and on the CPU): watch visits, gate
    /// items, analysis, jumps back, decisions, reductions.
    pub cycles: [u32; 6],
}

/// A formula's room for learned clauses, by its size: their literals (16 words a variable, from 4,096 to 2^18), and
/// a clause for each 16 of those. The search reduces them as they fill it.
pub fn arena(vars: usize) -> u32 {
    (16 * vars).clamp(1 << 12, 1 << 18) as u32
}
pub fn learned(vars: usize) -> u32 {
    arena(vars) / 16
}

/// The steps a search takes at most (a round, a conflict, a decision): the device's launches times a launch's steps
/// (`bitlex_csat_gpu`'s `LAUNCHES` and `STEPS`). Past them a formula is unknown.
pub const STEPS: u64 = 1 << 24;

const NONE: u32 = u32::MAX;
const GATE: u32 = 0x8000_0000;
const FIXED: u32 = 3;
const LANES: usize = 32;
const BUMP_FIRST: u32 = 1024;
const BUMP_MAX: u32 = 1 << 24;
const BUMP_SHIFT: u32 = 14;
const REDUCE_FIRST: u32 = 2000;
const REDUCE_STEP: u32 = 300;
const KEEP_LBD: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Running,
    Sat,
    Unsat,
    Unknown,
    Roomless,
    /// A loop past the kernel's bound: a fault in the search.
    Broken,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    None,
    /// One literal (`lit`, `why`).
    One,
    /// The output true: each unset argument.
    And,
    /// A `One`'s argument `why` held: each other unset argument false.
    Held,
    Conflict,
}

#[derive(Clone, Copy)]
struct Check {
    mode: Mode,
    lit: Lit,
    why: u32,
    cursor: usize,
}

const QUIET: Check = Check { mode: Mode::None, lit: 0, why: 0, cursor: 0 };

/// Where a watch node is linked from: a literal's head, or the node before it.
#[derive(Clone, Copy)]
enum Link {
    Head(usize),
    Node(usize),
}

fn lvar(l: Lit) -> usize {
    l.unsigned_abs() as usize
}
fn lslot(l: Lit) -> usize {
    2 * lvar(l) + usize::from(l < 0)
}

struct Search<'a> {
    f: &'a Formula<'a>,
    lcap: usize,
    acap: usize,
    // A variable's word: 0 unset, else its level `<< 2`, 1 true or 2 false.
    val: Vec<u32>,
    reason: Vec<u32>,
    why: Vec<u32>,
    trail: Vec<Lit>,
    limits: Vec<u32>,
    seen: Vec<bool>,
    act: Vec<u32>,
    heap: Vec<u32>,
    hat: Vec<u32>,
    whead: Vec<u32>,
    wnext: Vec<u32>,
    wlit: Vec<Lit>,
    lstart: Vec<u32>,
    arena: Vec<Lit>,
    lbd: Vec<u32>,
    lmark: Vec<u32>,
    cbuf: Vec<Lit>,
    ebuf: Vec<Lit>,
    head: usize,
    level: u32,
    heap_n: usize,
    bump: u32,
    conflict: u32,
    conflicts: u32,
    decisions: u32,
    rounds: u32,
    learned: usize,
    used: usize,
    visits: u32,
    items: u32,
    reduce_at: u32,
    reductions: u32,
    last_lbd: u32,
    status: Status,
}

/// The formula solved as the device solves it: the same answer, model and counts.
pub fn solve(f: &Formula) -> Solved {
    let mut s = Search::new(f);
    s.start();
    let mut steps = 0u64;
    while s.status == Status::Running {
        if steps >= STEPS {
            s.status = Status::Unknown;
            break;
        }
        steps += 1;
        s.step();
    }
    s.solved()
}

impl<'a> Search<'a> {
    fn new(f: &'a Formula<'a>) -> Self {
        let n = f.vars + 1;
        let (lcap, acap) = (learned(f.vars) as usize, arena(f.vars) as usize);
        let nodes = 2 * (f.clauses.len() + lcap);
        Search {
            f,
            lcap,
            acap,
            val: vec![0; n],
            reason: vec![0; n],
            why: vec![0; n],
            trail: Vec::with_capacity(n),
            limits: vec![0; n],
            seen: vec![false; n],
            act: vec![0; n],
            heap: vec![0; n],
            hat: vec![NONE; n],
            whead: vec![NONE; 2 * n],
            wnext: vec![NONE; nodes],
            wlit: vec![0; nodes],
            lstart: vec![0; lcap + 1],
            arena: vec![0; acap],
            lbd: vec![0; lcap],
            lmark: vec![0; n],
            cbuf: Vec::new(),
            ebuf: Vec::new(),
            head: 0,
            level: 0,
            heap_n: 0,
            bump: BUMP_FIRST,
            conflict: NONE,
            conflicts: 0,
            decisions: 0,
            rounds: 0,
            learned: 0,
            used: 0,
            visits: 0,
            items: 0,
            reduce_at: REDUCE_FIRST,
            reductions: 0,
            last_lbd: 0,
            status: Status::Running,
        }
    }

    fn solved(&self) -> Solved {
        Solved {
            answer: match self.status {
                Status::Sat => Answer::Sat(self.val.iter().map(|w| w & 1 != 0).collect()),
                Status::Unsat => Answer::Unsat,
                _ => Answer::Unknown,
            },
            roomless: self.status == Status::Roomless,
            conflicts: self.conflicts,
            decisions: self.decisions,
            rounds: self.rounds,
            learned: self.learned as u32,
            learned_literals: self.used as u32,
            visits: self.visits,
            items: self.items,
            reductions: self.reductions,
            cycles: [0; 6],
        }
    }

    /// `cs_finish`: a fault stays.
    fn finish(&mut self, s: Status) {
        if self.status != Status::Broken {
            self.status = s;
        }
    }

    fn value(&self, l: Lit) -> i32 {
        let w = self.val[lvar(l)];
        if w == 0 {
            return 0;
        }
        let t = if w & 1 != 0 { 1 } else { -1 };
        if l > 0 { t } else { -t }
    }

    fn level_of(&self, v: usize) -> u32 {
        self.val[v] >> 2
    }

    fn kind(&self, g: usize) -> u32 {
        match self.f.gates.kind(g) {
            Kind::And => 0,
            Kind::Xor => 1,
            Kind::One => 2,
        }
    }

    fn arg(&self, g: usize, i: usize) -> Lit {
        self.f.gates.args(g)[i]
    }

    /// A clause's literals: a plain one (below the plain clauses' count), else a learned one, in the arena.
    fn cl_len(&self, id: usize) -> usize {
        let c = self.f.clauses.len();
        if id < c { self.f.clauses.get(id).len() } else { (self.lstart[id - c + 1] - self.lstart[id - c]) as usize }
    }
    fn cl_lit(&self, id: usize, k: usize) -> Lit {
        let c = self.f.clauses.len();
        if id < c { self.f.clauses.get(id)[k] } else { self.arena[self.lstart[id - c] as usize + k] }
    }

    /// `cs_put`, at the trail's end.
    fn assign(&mut self, l: Lit, reason: u32, why: u32) {
        let v = lvar(l);
        self.val[v] = (self.level << 2) | if l > 0 { 1 } else { 2 };
        self.reason[v] = reason;
        self.why[v] = why;
        self.trail.push(l);
    }

    /// `cs_check`: gate `g` against the values; a conflict's clause into `cbuf` when `write`.
    fn check(&mut self, g: usize, write: bool) -> Check {
        let mut r = QUIET;
        let n = self.f.gates.args(g).len();
        let o = self.f.gates.out(g);
        if write {
            self.cbuf.clear();
        }
        match self.kind(g) {
            0 => {
                let vo = self.value(o);
                let (mut unset, mut unset_at) = (0u32, 0usize);
                for i in 0..n {
                    let a = self.arg(g, i);
                    let va = self.value(a);
                    if va == -1 {
                        if vo == 1 {
                            r.mode = Mode::Conflict;
                            if write {
                                self.cbuf.extend([-o, a]);
                            }
                        } else if vo == 0 {
                            r.mode = Mode::One;
                            r.lit = -o;
                            r.why = i as u32;
                        }
                        return r;
                    }
                    if va == 0 {
                        if unset == 0 {
                            unset = 1;
                            unset_at = i;
                        } else if a != self.arg(g, unset_at) {
                            unset += 1;
                        }
                    }
                }
                if vo == 1 {
                    if unset > 0 {
                        r.mode = Mode::And;
                    }
                } else if vo == 0 && unset == 0 {
                    r.mode = Mode::One;
                    r.lit = o;
                } else if vo == -1 && unset == 0 {
                    r.mode = Mode::Conflict;
                    if write {
                        self.cbuf.push(o);
                        for i in 0..n {
                            let a = self.arg(g, i);
                            self.cbuf.push(-a);
                        }
                    }
                } else if vo == -1 && unset == 1 {
                    r.mode = Mode::One;
                    r.lit = -self.arg(g, unset_at);
                    r.why = unset_at as u32;
                }
            }
            1 => {
                let (a, b) = (self.arg(g, 0), self.arg(g, 1));
                if lvar(a) == lvar(b) {
                    let want = if a == b { -o } else { o };
                    let vw = self.value(want);
                    if vw == 0 {
                        r.mode = Mode::One;
                        r.lit = want;
                        r.why = FIXED;
                    } else if vw == -1 {
                        r.mode = Mode::Conflict;
                        if write {
                            self.cbuf.push(want);
                        }
                    }
                } else {
                    let p = [o, a, b];
                    let (v0, v1, v2) = (self.value(o), self.value(a), self.value(b));
                    let unset = u32::from(v0 == 0) + u32::from(v1 == 0) + u32::from(v2 == 0);
                    let (t0, t1, t2) = (v0 == 1, v1 == 1, v2 == 1);
                    if unset == 0 && t0 != (t1 != t2) {
                        r.mode = Mode::Conflict;
                        if write {
                            for q in p {
                                let w = if self.value(q) == 1 { -q } else { q };
                                self.cbuf.push(w);
                            }
                        }
                    } else if unset == 1 {
                        let k = if v0 == 0 {
                            0
                        } else if v1 == 0 {
                            1
                        } else {
                            2
                        };
                        let holds = match k {
                            0 => t1 != t2,
                            1 => t0 != t2,
                            _ => t0 != t1,
                        };
                        r.mode = Mode::One;
                        r.lit = if holds { p[k] } else { -p[k] };
                        r.why = k as u32;
                    }
                }
            }
            _ => {
                let (mut held, mut unset, mut unset_at) = (NONE, 0u32, 0usize);
                for i in 0..n {
                    let a = self.arg(g, i);
                    let va = self.value(a);
                    if va == 1 {
                        if held != NONE {
                            r.mode = Mode::Conflict;
                            if write {
                                let h = self.arg(g, held as usize);
                                self.cbuf.extend([-h, -a]);
                            }
                            return r;
                        }
                        held = i as u32;
                    } else if va == 0 {
                        unset += 1;
                        unset_at = i;
                    }
                }
                if held != NONE {
                    if unset > 0 {
                        r.mode = Mode::Held;
                        r.why = held;
                    }
                } else if unset == 0 {
                    r.mode = Mode::Conflict;
                    if write {
                        for i in 0..n {
                            let a = self.arg(g, i);
                            self.cbuf.push(a);
                        }
                    }
                } else if unset == 1 {
                    r.mode = Mode::One;
                    r.lit = self.arg(g, unset_at);
                    r.why = unset_at as u32;
                }
            }
        }
        r
    }

    /// `cs_items`: up to 32 gates, a lane each in order. A conflict (the first lane's, its clause in `cbuf`), or each
    /// step's literals on the trail: of the lanes that set one variable the first, if it is still unset. True on a
    /// conflict.
    fn items(&mut self, gs: &[u32]) -> bool {
        let mut ks = [QUIET; LANES];
        for (k, &g) in ks.iter_mut().zip(gs) {
            *k = self.check(g as usize, false);
        }
        if let Some(i) = ks[..gs.len()].iter().position(|k| k.mode == Mode::Conflict) {
            self.check(gs[i] as usize, true);
            self.conflict = NONE;
            return true;
        }
        let bound = gs.iter().map(|&g| self.f.gates.args(g as usize).len()).max().unwrap_or(0) + 2;
        let mut once = [false; LANES];
        for (o, k) in once.iter_mut().zip(&ks) {
            *o = k.mode == Mode::One;
        }
        let mut step = 0usize;
        loop {
            if step > bound {
                self.status = Status::Broken;
                return true;
            }
            step += 1;
            // Each lane's next literal, all read before any is placed.
            let mut ls = [(0 as Lit, 0u32); LANES];
            for i in 0..gs.len() {
                let g = gs[i] as usize;
                let k = &mut ks[i];
                if once[i] {
                    ls[i] = (k.lit, k.why);
                    once[i] = false;
                    k.mode = Mode::None;
                } else if k.mode == Mode::And || k.mode == Mode::Held {
                    let args = self.f.gates.args(g);
                    let mut l = 0;
                    while k.cursor < args.len() {
                        let i2 = k.cursor;
                        k.cursor += 1;
                        if k.mode == Mode::Held && i2 as u32 == k.why {
                            continue;
                        }
                        let a = args[i2];
                        let w = self.val[lvar(a)];
                        if w == 0 {
                            l = if k.mode == Mode::And { a } else { -a };
                            ls[i] = (l, if k.mode == Mode::And { i2 as u32 } else { k.why });
                            break;
                        }
                    }
                    if l == 0 {
                        k.mode = Mode::None;
                    }
                }
            }
            if ls[..gs.len()].iter().all(|&(l, _)| l == 0) {
                return false;
            }
            let mut wins = [false; LANES];
            for i in 0..gs.len() {
                let l = ls[i].0;
                wins[i] = l != 0 && !ls[..i].iter().any(|&(m, _)| m != 0 && lvar(m) == lvar(l)) && self.value(l) == 0;
            }
            for i in 0..gs.len() {
                if wins[i] {
                    self.assign(ls[i].0, GATE | gs[i], ls[i].1);
                }
            }
        }
    }

    /// `cs_visit`: the clauses watching `x`, which turned false. True on a conflict (`conflict` its clause).
    fn visit(&mut self, x: Lit) -> bool {
        let mut link = Link::Head(lslot(x));
        let mut k = self.whead[lslot(x)];
        let nodes = 2 * (self.f.clauses.len() + self.lcap);
        let mut seen = 0usize;
        while k != NONE {
            if seen > nodes {
                self.status = Status::Broken;
                return true;
            }
            seen += 1;
            let ku = k as usize;
            let id = ku >> 1;
            let nk = self.wnext[ku];
            let other = self.wlit[ku ^ 1];
            let vo = self.value(other);
            self.visits += 1;
            if vo == 1 {
                link = Link::Node(ku);
                k = nk;
                continue;
            }
            let mut y = 0;
            for j in 0..self.cl_len(id) {
                let q = self.cl_lit(id, j);
                if q != x && q != other && self.value(q) != -1 {
                    y = q;
                    break;
                }
            }
            if y != 0 {
                match link {
                    Link::Head(h) => self.whead[h] = nk,
                    Link::Node(p) => self.wnext[p] = nk,
                }
                self.wlit[ku] = y;
                self.wnext[ku] = self.whead[lslot(y)];
                self.whead[lslot(y)] = k;
                k = nk;
                continue;
            }
            if vo == 0 {
                self.assign(other, id as u32, 0);
                link = Link::Node(ku);
                k = nk;
                continue;
            }
            self.conflict = id as u32;
            return true;
        }
        false
    }

    /// `cs_watch`: clause `id` watched by `a` and `b`.
    fn watch(&mut self, id: usize, a: Lit, b: Lit) {
        let k = 2 * id;
        self.wlit[k] = a;
        self.wnext[k] = self.whead[lslot(a)];
        self.whead[lslot(a)] = k as u32;
        self.wlit[k + 1] = b;
        self.wnext[k + 1] = self.whead[lslot(b)];
        self.whead[lslot(b)] = (k + 1) as u32;
    }

    /// A variable's gates: the window's, then the solve's own.
    fn fan_of(&self, v: usize) -> impl Iterator<Item = u32> + 'a {
        let f = self.f;
        let window: &'a [u32] = if v <= f.fan.vars() { f.fan.of_var(v) } else { &[] };
        let own: &'a [u32] = f.own.map_or(&[], |o| o.of_var(v));
        window.iter().chain(own).copied()
    }

    /// `cs_round`: the trail's next literals, up to 32; the watch lists that were not empty, in trail order; then
    /// their gates, 32 at a time. True on a conflict.
    fn round(&mut self) -> bool {
        let hd = self.head;
        let f = (self.trail.len() - hd).min(LANES);
        let mut lits = [0 as Lit; LANES];
        lits[..f].copy_from_slice(&self.trail[hd..hd + f]);
        let mut watched = [false; LANES];
        for i in 0..f {
            watched[i] = self.whead[lslot(-lits[i])] != NONE;
        }
        for i in 0..f {
            if watched[i] && self.visit(-lits[i]) {
                return true;
            }
        }
        if lits[..f].iter().any(|&l| lvar(l) > self.f.vars) {
            self.status = Status::Broken;
            return true;
        }
        let gs: Vec<u32> = lits[..f].iter().flat_map(|&l| self.fan_of(lvar(l))).collect();
        self.items = self.items.wrapping_add(gs.len() as u32);
        for chunk in gs.chunks(LANES) {
            if self.items(chunk) {
                return true;
            }
        }
        self.head = hd + f;
        false
    }

    // `bitlex_sat::Order`'s heap, as the kernel keeps it.
    fn before(&self, a: u32, b: u32) -> bool {
        let (x, y) = (self.act[a as usize], self.act[b as usize]);
        x > y || (x == y && a > b)
    }
    fn up(&mut self, mut i: usize) {
        let v = self.heap[i];
        while i > 0 {
            let p = (i + 1) / 2 - 1;
            let pv = self.heap[p];
            if !self.before(v, pv) {
                break;
            }
            self.heap[i] = pv;
            self.hat[pv as usize] = i as u32;
            i = p;
        }
        self.heap[i] = v;
        self.hat[v as usize] = i as u32;
    }
    fn down(&mut self, mut i: usize) {
        let v = self.heap[i];
        let n = self.heap_n;
        let mut depth = 0;
        loop {
            if depth > 32 {
                self.status = Status::Broken;
                return;
            }
            depth += 1;
            let mut k = 2 * i + 1;
            if k >= n {
                break;
            }
            if k + 1 < n && self.before(self.heap[k + 1], self.heap[k]) {
                k += 1;
            }
            let kv = self.heap[k];
            if !self.before(kv, v) {
                break;
            }
            self.heap[i] = kv;
            self.hat[kv as usize] = i as u32;
            i = k;
        }
        self.heap[i] = v;
        self.hat[v as usize] = i as u32;
    }
    fn insert(&mut self, v: usize) {
        if self.hat[v] != NONE {
            return;
        }
        let i = self.heap_n;
        self.heap_n += 1;
        self.heap[i] = v as u32;
        self.hat[v] = i as u32;
        self.up(i);
    }
    fn pop(&mut self) -> u32 {
        let n = self.heap_n;
        if n == 0 {
            return NONE;
        }
        let top = self.heap[0];
        self.hat[top as usize] = NONE;
        self.heap_n = n - 1;
        if n > 1 {
            self.heap[0] = self.heap[n - 1];
            self.down(0);
        }
        top
    }
    fn bump_var(&mut self, v: usize) {
        self.act[v] = self.act[v].wrapping_add(self.bump);
        if self.hat[v] != NONE {
            self.up(self.hat[v] as usize);
        }
    }

    /// `cs_explain`: the clause that implied `v`'s value, into `ebuf` (its literal first, every other false), or the
    /// number of the clause that did.
    fn explain(&mut self, v: usize) -> Option<usize> {
        let r = self.reason[v];
        let w = self.why[v] as usize;
        self.ebuf.clear();
        if r & GATE == 0 {
            return Some(r as usize);
        }
        let l = if self.val[v] & 1 != 0 { v as Lit } else { -(v as Lit) };
        let g = (r & !GATE) as usize;
        let f = self.f;
        let args = f.gates.args(g);
        let n = args.len();
        let o = f.gates.out(g);
        match self.kind(g) {
            0 => {
                if o != 0 && lvar(o) == v {
                    if l == o {
                        self.ebuf.push(o);
                        self.ebuf.extend(args.iter().map(|&a| -a));
                    } else {
                        self.ebuf.extend([-o, args[w]]);
                    }
                } else if l == args[w] {
                    self.ebuf.extend([l, -o]);
                } else {
                    self.ebuf.extend([l, o]);
                    self.ebuf.extend((0..n).filter(|&i| i != w).map(|i| -args[i]));
                }
            }
            1 => {
                self.ebuf.push(l);
                if w as u32 != FIXED {
                    let p = [o, args[0], args[1]];
                    for (k, q) in p.into_iter().enumerate() {
                        if k != w {
                            let e = if self.value(q) == 1 { -q } else { q };
                            self.ebuf.push(e);
                        }
                    }
                }
            }
            _ => {
                if l == args[w] {
                    self.ebuf.push(l);
                    self.ebuf.extend((0..n).filter(|&i| i != w).map(|i| args[i]));
                } else {
                    self.ebuf.extend([l, -args[w]]);
                }
            }
        }
        None
    }

    /// `cs_analyze`: the conflict's first unique implication point, learned at the arena's end (its asserting
    /// literal first, the highest of the rest second); its length and the level to jump back to, or none where the
    /// room is full (or a fault). A clause's literals are taken 32 at a time, as the kernel's lanes take them: the
    /// room is checked a chunk at a time.
    fn analyze(&mut self) -> Option<(usize, u32)> {
        let here = self.level;
        let u = self.used;
        let (mut len, mut count) = (1usize, 0usize);
        let mut at = self.trail.len();
        if self.learned >= self.lcap || u >= self.acap {
            return None;
        }
        // The clause read: a numbered one, or `cbuf`'s or `ebuf`'s literals.
        let mut id = self.conflict;
        let mut from_cbuf = true;
        let mut p: Lit = 0;
        let mut cands: Vec<(usize, Lit, bool)> = Vec::with_capacity(LANES);
        loop {
            let m = if id != NONE {
                self.cl_len(id as usize)
            } else if from_cbuf {
                self.cbuf.len()
            } else {
                self.ebuf.len()
            };
            for jb in (0..m).step_by(LANES) {
                cands.clear();
                for j in jb..(jb + LANES).min(m) {
                    let q = if id != NONE {
                        self.cl_lit(id as usize, j)
                    } else if from_cbuf {
                        self.cbuf[j]
                    } else {
                        self.ebuf[j]
                    };
                    let v = lvar(q);
                    let lv = self.level_of(v);
                    if q != p && lv != 0 && !self.seen[v] && !cands.iter().any(|&(w, _, _)| w == v) {
                        cands.push((v, q, lv != here));
                    }
                }
                let pushes = cands.iter().filter(|c| c.2).count();
                if u + len + pushes > self.acap {
                    return None;
                }
                for &(v, q, low) in &cands {
                    self.seen[v] = true;
                    if low {
                        self.arena[u + len] = q;
                        len += 1;
                    }
                }
                count += cands.len() - pushes;
                for i in 0..cands.len() {
                    let v = cands[i].0;
                    self.bump_var(v);
                }
            }
            // The trail's last marked entry below `at`.
            loop {
                if at == 0 || count == 0 {
                    self.status = Status::Broken;
                    return None;
                }
                at -= 1;
                if self.seen[lvar(self.trail[at])] {
                    break;
                }
            }
            let l = self.trail[at];
            count -= 1;
            self.seen[lvar(l)] = false;
            if count == 0 {
                self.arena[u] = -l;
                break;
            }
            id = self.explain(lvar(l)).map_or(NONE, |c| c as u32);
            from_cbuf = false;
            p = l;
        }
        // The rest unmarked; the highest level's first literal second; the levels counted (LBD).
        let (mut hi, mut hk, mark, mut lbd) = (0u32, 1usize, self.conflicts, 1u32);
        self.lmark[here as usize] = mark;
        for k in 1..len {
            let v = lvar(self.arena[u + k]);
            self.seen[v] = false;
            let lv = self.level_of(v);
            if lv > hi {
                hi = lv;
                hk = k;
            }
            if self.lmark[lv as usize] != mark {
                self.lmark[lv as usize] = mark;
                lbd += 1;
            }
        }
        if len > 1 {
            self.arena.swap(u + 1, u + hk);
        }
        self.bump += self.bump / 20;
        self.last_lbd = lbd;
        Some((len, hi))
    }

    /// `cs_back_to`: the trail back to level `to`, its variables unset; those out of the heap put back, in trail
    /// order.
    fn back_to(&mut self, to: u32) {
        let start = self.limits[to as usize] as usize;
        for i in start..self.trail.len() {
            let v = lvar(self.trail[i]);
            self.val[v] = 0;
            self.insert(v);
        }
        self.trail.truncate(start);
        self.head = start;
        self.level = to;
    }

    /// `cs_conflict`: a conflict met at a level past 0: learned, jumped back, its literal asserted.
    fn on_conflict(&mut self) {
        if self.status != Status::Running {
            return;
        }
        if self.level == 0 {
            self.finish(Status::Unsat);
            return;
        }
        if self.conflicts >= self.f.conflicts {
            self.finish(Status::Unknown);
            return;
        }
        self.conflicts += 1;
        let got = self.analyze();
        if self.bump > BUMP_MAX {
            for a in &mut self.act {
                *a >>= BUMP_SHIFT;
            }
            self.bump >>= BUMP_SHIFT;
        }
        let Some((len, back)) = got else {
            self.finish(Status::Roomless);
            return;
        };
        self.back_to(back);
        let u = self.used;
        let first = self.arena[u];
        if len == 1 {
            self.assign(first, NONE, 0);
        } else {
            let j = self.learned;
            let id = self.f.clauses.len() + j;
            self.lstart[j] = u as u32;
            self.lstart[j + 1] = (u + len) as u32;
            self.lbd[j] = self.last_lbd;
            self.used = u + len;
            self.learned = j + 1;
            let second = self.arena[u + 1];
            self.watch(id, first, second);
            self.assign(first, id as u32, 0);
        }
    }

    /// `cs_reduce`: the learned clauses reduced, at level 0. False where a clause is now false at level 0.
    fn reduce(&mut self) -> bool {
        if self.level > 0 {
            self.back_to(0);
        }
        self.whead.fill(NONE);
        let plain = self.f.clauses.len();
        for i in 0..self.trail.len() {
            let v = lvar(self.trail[i]);
            let r = self.reason[v];
            if r & GATE == 0 && r != NONE && r as usize >= plain {
                self.reason[v] = NONE;
            }
        }
        let n = self.learned;
        let room = self.acap / 2;
        let mut keep = vec![false; n];
        let mut used = 0usize;
        for i in (0..n).rev() {
            let (b, e) = (self.lstart[i] as usize, self.lstart[i + 1] as usize);
            let sat = (b..e).any(|k| self.value(self.arena[k]) == 1);
            let want = !sat && (self.lbd[i] <= KEEP_LBD || 2 * i >= n);
            if want && used + (e - b) <= room {
                used += e - b;
                keep[i] = true;
            }
        }
        let (mut kept, mut used) = (0usize, 0usize);
        for j in 0..n {
            let (b, e) = (self.lstart[j] as usize, self.lstart[j + 1] as usize);
            if !keep[j] {
                continue;
            }
            self.arena.copy_within(b..e, used);
            self.lstart[kept] = used as u32;
            self.lbd[kept] = self.lbd[j];
            used += e - b;
            kept += 1;
            self.lstart[kept] = used as u32;
        }
        self.lstart[kept] = used as u32;
        self.learned = kept;
        self.used = used;
        self.reduce_at = kept as u32 + REDUCE_FIRST + REDUCE_STEP * (self.reductions + 1);
        self.reductions += 1;
        for id in 0..plain + kept {
            let (mut a, mut b, mut sat) = (0, 0, false);
            for k in 0..self.cl_len(id) {
                let q = self.cl_lit(id, k);
                let vq = self.value(q);
                if vq == 1 {
                    sat = true;
                    break;
                } else if vq == 0 && a == 0 {
                    a = q;
                } else if vq == 0 && b == 0 && q != a {
                    b = q;
                }
            }
            if sat {
                continue;
            }
            if a == 0 {
                return false;
            } else if b == 0 {
                self.assign(a, id as u32, 0);
            } else {
                self.watch(id, a, b);
            }
        }
        true
    }

    /// `cs_decide`: the most active unset variable, false; none left, the formula holds.
    fn decide(&mut self) {
        if self.heap_n > self.f.vars {
            self.status = Status::Broken;
        }
        if self.status != Status::Running {
            return;
        }
        let mut v = self.pop();
        while v != NONE && self.val[v as usize] != 0 {
            v = self.pop();
        }
        if v == NONE {
            self.status = Status::Sat;
        } else if self.f.conflicts == 0 {
            // No conflict to spend: propagation at level 0 alone.
            self.status = Status::Unknown;
        } else {
            self.limits[self.level as usize] = self.trail.len() as u32;
            self.level += 1;
            self.decisions += 1;
            self.assign(-(v as Lit), NONE, 0);
        }
    }

    /// `cs_start`: the plain clauses watched (their units set), the gates that imply with nothing set checked.
    fn start(&mut self) {
        let vars = self.f.vars;
        for v in 1..=vars {
            self.hat[v] = (vars - v) as u32;
            self.heap[v - 1] = (vars - (v - 1)) as u32;
        }
        self.heap_n = vars;
        for id in 0..self.f.clauses.len() {
            if self.status != Status::Running {
                break;
            }
            let f = self.f;
            let c = f.clauses.get(id);
            let mut taut = false;
            let (mut a, mut b) = (0, 0);
            for (j, &q) in c.iter().enumerate() {
                if taut {
                    break;
                }
                if c[j + 1..].contains(&-q) {
                    taut = true;
                }
                if a == 0 {
                    a = q;
                } else if b == 0 && q != a {
                    b = q;
                }
            }
            if taut {
                continue;
            }
            if a == 0 {
                self.status = Status::Unsat;
            } else if b == 0 {
                match self.value(a) {
                    -1 => self.status = Status::Unsat,
                    0 => self.assign(a, NONE, 0),
                    _ => {}
                }
            } else {
                self.watch(id, a, b);
            }
        }
        if self.status != Status::Running {
            return;
        }
        let gates = self.f.gates.len();
        for g0 in (0..gates).step_by(LANES) {
            let gs: Vec<u32> = (g0..(g0 + LANES).min(gates)).filter(|&g| self.seed(g)).map(|g| g as u32).collect();
            if gs.is_empty() {
                continue;
            }
            if self.items(&gs) {
                self.finish(Status::Unsat);
                return;
            }
        }
    }

    /// `cs_seed`: whether a gate can imply with nothing set.
    fn seed(&self, g: usize) -> bool {
        let n = self.f.gates.args(g).len();
        match self.kind(g) {
            0 => n == 0,
            2 => n <= 1,
            _ => lvar(self.arg(g, 0)) == lvar(self.arg(g, 1)),
        }
    }

    /// A step of the kernel's main loop.
    fn step(&mut self) {
        let n = self.f.vars + 1;
        if self.trail.len() > n || self.head > self.trail.len() || self.level as usize >= n {
            self.finish(Status::Broken);
            return;
        }
        if self.head == self.trail.len() {
            let (l, lcap, acap) = (self.learned, self.lcap, self.acap);
            if l as u32 >= self.reduce_at || 4 * l >= 3 * lcap || 4 * self.used >= 3 * acap {
                if !self.reduce() {
                    self.finish(Status::Unsat);
                }
                return;
            }
            self.decide();
            return;
        }
        self.rounds += 1;
        if self.round() {
            self.on_conflict();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::bitlex_csat::Solver;
    use crate::bitlex_grown::FormulaBlock;
    use crate::bitlex_sat::Answer as CpuAnswer;

    pub(crate) struct Rng(pub(crate) u64);
    impl Rng {
        pub(crate) fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        pub(crate) fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        pub(crate) fn lit(&mut self, vars: usize) -> Lit {
            let v = self.below(vars as u64) as Lit + 1;
            if self.next() & 1 == 0 { v } else { -v }
        }
    }

    /// Whether a model holds: every gate (as `Gates::cnf` writes its clauses) and every plain clause.
    pub(crate) fn holds(model: &[bool], gates: &Gates, clauses: &Clauses) -> bool {
        let mut all = Clauses::default();
        gates.cnf(&mut all);
        let t = |l: Lit| model[l.unsigned_abs() as usize] == (l > 0);
        all.iter().chain(clauses.iter()).all(|c| c.iter().any(|&l| t(l)))
    }

    /// A random circuit over `vars` inputs, built in `block`: `And`s, `Xor`s and `One`s over earlier variables (a
    /// third of them wide), plain clauses of two to four literals over all; the last `own` gates are a solve's own.
    pub(crate) fn circuit(rng: &mut Rng, block: &FormulaBlock, inputs: usize, gates_n: usize, clauses_n: usize, own: usize) -> (Gates, Clauses, usize, usize) {
        let (mut g, mut c) = (Gates::in_block(block), Clauses::in_block(block));
        let mut vars = inputs;
        for _ in 0..gates_n {
            match rng.below(5) {
                0 | 1 => {
                    let n = if rng.below(3) == 0 { 3 + rng.below(4) as usize } else { 1 + rng.below(2) as usize };
                    let args: Vec<Lit> = (0..n).map(|_| rng.lit(vars)).collect();
                    vars += 1;
                    g.and(vars as Lit, &args);
                }
                2 | 3 => {
                    let (a, b) = (rng.lit(vars), rng.lit(vars));
                    vars += 1;
                    g.xor(vars as Lit, a, b);
                }
                _ => {
                    let n = 1 + rng.below(4) as usize;
                    let args: Vec<Lit> = (0..n).map(|_| rng.lit(vars)).collect();
                    g.one(&args);
                }
            }
        }
        for _ in 0..clauses_n {
            let n = 2 + rng.below(3) as usize;
            let lits: Vec<Lit> = (0..n).map(|_| rng.lit(vars)).collect();
            c.push(&lits);
        }
        let base = g.len().saturating_sub(own);
        (g, c, vars, base)
    }

    /// The device's search and this one's: the same answer, model and counts.
    pub(crate) fn same_search(i: usize, device: &Solved, here: &Solved) {
        let counts = |x: &Solved| (x.conflicts, x.decisions, x.rounds, x.learned, x.learned_literals, x.visits, x.items, x.reductions, x.roomless);
        assert_eq!(device.answer, here.answer, "formula {i}: the device's answer, and the CPU's of its search");
        assert_eq!(counts(device), counts(here), "formula {i}: (conflicts, decisions, rounds, learned, literals, visits, items, reductions, roomless) on the device, and on the CPU");
    }

    /// Random circuits, some with a solve's own gates: the answers are the CPU solver's (`bitlex_csat::Solver`), and
    /// each model holds.
    #[test]
    fn the_searchs_answers_are_the_solvers() {
        let block = FormulaBlock::new();
        let mut rng = Rng(0x2545f4914f6cdd1d);
        let (mut sat, mut unsat, mut searched) = (0, 0, 0);
        let mut s = Solver::default();
        for i in 0..600usize {
            let inputs = 4 + rng.below(40) as usize;
            let gates_n = rng.below(160) as usize;
            let clauses_n = rng.below(2 * inputs as u64 + 4) as usize;
            let own = if i % 3 == 0 { rng.below(10) as usize } else { 0 };
            let (g, c, vars, base) = circuit(&mut rng, &block, inputs, gates_n, clauses_n, own);
            let (fan, own_fan) = (Fanout::of(&g, base, vars), Fanout::of_range(&g, base..g.len(), vars));
            let f = Formula { vars, gates: &g, base, fan: &fan, own: (base < g.len()).then_some(&own_fan), clauses: &c, conflicts: 100_000 };
            let x = solve(&f);
            s.reset(vars);
            for cl in c.iter() {
                s.add(cl);
            }
            match (&x.answer, s.solve(&g, &fan, base, 100_000)) {
                (Answer::Sat(m), CpuAnswer::Sat) => {
                    assert!(holds(m, &g, &c), "formula {i}: the model does not hold");
                    sat += 1;
                }
                (Answer::Unsat, CpuAnswer::Unsat) => unsat += 1,
                (a, b) => panic!("formula {i} ({vars} vars, {} gates, {} clauses): the search {a:?}, the solver {b:?}", g.len(), c.len()),
            }
            searched += usize::from(x.conflicts > 0);
        }
        assert!(sat >= 60 && unsat >= 60 && searched >= 60, "{sat} sat, {unsat} unsat, {searched} searched");
    }
    /// The formulas in `RERAIL_CSAT_DUMPS` (`Gates::dump`s), solved by this search and by the CPU's solver: each one's
    /// answers, counts and time (decision 0670; a tool, so ignored).
    #[test]
    #[ignore]
    fn the_dumped_formulas_here() {
        let Some(dir) = std::env::var_os("RERAIL_CSAT_DUMPS") else { return };
        let mut paths: Vec<_> = std::fs::read_dir(dir).expect("the dumps").map(|e| e.expect("a dump").path()).collect();
        paths.sort();
        let block = FormulaBlock::new();
        let mut s = Solver::default();
        for p in &paths {
            let (g, vars, base, c) = Gates::undump_into(&std::fs::read_to_string(p).expect("a dump"), Gates::in_block(&block), Clauses::in_block(&block));
            let (fan, own) = (Fanout::of(&g, base, vars), Fanout::of_range(&g, base..g.len(), vars));
            let f = Formula { vars, gates: &g, base, fan: &fan, own: (base < g.len()).then_some(&own), clauses: &c, conflicts: 100_000 };
            let t = std::time::Instant::now();
            let x = solve(&f);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            s.reset(vars);
            for cl in c.iter() {
                s.add(cl);
            }
            let cpu = s.solve(&g, &fan, base, 100_000);
            match (&x.answer, cpu) {
                (Answer::Sat(m), CpuAnswer::Sat) => assert!(holds(m, &g, &c), "{p:?}: the model does not hold"),
                (Answer::Unsat, CpuAnswer::Unsat) | (Answer::Unknown, _) => {}
                (a, b) => panic!("{p:?}: the search {a:?}, the solver {b:?}"),
            }
            eprintln!(
                "{}: {:?}, {} conflicts, {} decisions, {} rounds, {} gate items, {} watch visits, {ms:.1} ms",
                p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
                x.answer == Answer::Unsat,
                x.conflicts,
                x.decisions,
                x.rounds,
                x.items,
                x.visits
            );
        }
    }
}
