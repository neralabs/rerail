//! The device's search (`bitlex_csat_gpu.uglykl`, decision 0670, step 3) on the CPU, step for step: the same rounds
//! (a round's literals, their watch lists, their values added to their gates' counters, their gates 32 at a time,
//! each checked only where its counter says it can imply, what a step implies placed in lane order), the
//! same decisions (the integer activities' heap), the same analysis, jumps back and reductions. A formula's answer,
//! its model and every count are the kernel's on any backend, so a caller may take either's: the prover's answers
//! then do not depend on whether the machine has a device.
//!
//! The kernel runs a SIMD group's lanes at once; here a step's lanes are taken in lane order, each reading the state
//! as the step began where the kernel's do (a gate's check, a literal's value before the step's writes). Each function
//! names the kernel's it follows. Its working state is its own, as the kernel's is: the formula is read where it was
//! built (decision 0353).

use crate::bitlex_csat::{Fanout, Gates, Kind};
use crate::bitlex_grown::Grown;
use crate::bitlex_sat::{Clauses, Lit};

/// A formula: its gates (the first `base` listed by `fan`, the rest a solve's own, listed by `own`), its plain
/// clauses, its variables (numbered from 1), and its budget of conflicts: with none, it stops at its first decision
/// (propagation at level 0 alone).
///
/// A window's solve (`resume`): the window's state at level 0 (`Start`, made by `start` from its first
/// `window_clauses` plain clauses and its first `base` gates), what its earlier solves learned that holds without
/// them (`carried`), the solve's own plain clauses (those past `window_clauses`) each with `-assume`, and `assume`
/// decided first. Its report then holds what it learned that holds without `assume` over the first `carry_vars`
/// variables (`Solved::carry`) and, unsatisfiable, the variables its contradiction rests on (`Solved::used`).
pub struct Formula<'a> {
    pub vars: usize,
    pub gates: &'a Gates,
    pub base: usize,
    pub fan: &'a Fanout,
    pub own: Option<&'a Fanout>,
    pub clauses: &'a Clauses,
    pub conflicts: u32,
    pub resume: Option<Resume<'a>>,
}

/// A window's solve: what it resumes from (`Formula`).
#[derive(Clone, Copy)]
pub struct Resume<'a> {
    pub start: &'a Start,
    pub window_clauses: usize,
    pub carried: &'a Clauses,
    pub assume: Lit,
    pub carry_vars: usize,
}

impl Formula<'_> {
    /// A hash of what its search reads (its gates' fanout follows from its gates): two formulas with one are the
    /// same search, to the same result. Two of foldhash's quality hashes, of two seeds: a result is kept by it
    /// (`bitlex_prove::Solves`), so a collision would be a wrong answer.
    pub fn fingerprint(&self) -> u128 {
        use std::hash::{BuildHasher as _, Hasher as _};
        let seeds = [0x243f_6a88_85a3_08d3, 0x1319_8a2e_0370_7344];
        let mut h = seeds.map(|s| foldhash::quality::FixedState::with_seed(s).build_hasher());
        let mut put = |x: u64| {
            h[0].write_u64(x);
            h[1].write_u64(x);
        };
        put(self.vars as u64);
        put(self.base as u64);
        put(u64::from(self.conflicts));
        for g in 0..self.gates.len() {
            put(self.gates.kind(g) as u64 | (self.gates.out(g) as u32 as u64) << 8);
            put(self.gates.args(g).len() as u64);
            for &a in self.gates.args(g) {
                put(a as u32 as u64);
            }
        }
        put(self.gates.len() as u64);
        put(self.clauses.lits().len() as u64);
        put(self.clauses.ends().len() as u64);
        for &l in self.clauses.lits() {
            put(l as u32 as u64);
        }
        for &e in self.clauses.ends() {
            put(u64::from(e));
        }
        if let Some(r) = self.resume {
            put(u64::from(r.start.broken));
            put(r.start.set.len() as u64);
            for &v in r.start.set.iter() {
                put(u64::from(v));
            }
            put(r.start.conflict.len() as u64);
            for &l in r.start.conflict.iter() {
                put(l as u32 as u64);
            }
            put(r.window_clauses as u64);
            put(r.assume as u32 as u64);
            put(r.carry_vars as u64);
            put(r.carried.lits().len() as u64);
            put(r.carried.ends().len() as u64);
            for &l in r.carried.lits() {
                put(l as u32 as u64);
            }
            for &e in r.carried.ends() {
                put(u64::from(e));
            }
        }
        u128::from(h[0].finish()) << 64 | u128::from(h[1].finish())
    }
}

impl<'a> Formula<'a> {
    /// A formula solved whole, from nothing.
    pub fn new(vars: usize, gates: &'a Gates, base: usize, fan: &'a Fanout, own: Option<&'a Fanout>, clauses: &'a Clauses, conflicts: u32) -> Self {
        Formula { vars, gates, base, fan, own, clauses, conflicts, resume: None }
    }
}

/// A window at level 0 (`start`): the values its plain clauses and gates set there, in order, each three words (the
/// literal, its reason, its argument; a clause's reason its number among the window's); or the contradiction it
/// met, its clause. Made once a window, in its block, where the device reads it.
#[derive(Clone, Debug, Default)]
pub struct Start {
    pub set: Grown<u32>,
    pub broken: bool,
    pub conflict: Grown<Lit>,
}

/// The longest learned clause a window keeps for its next solves.
pub const CARRIED_LONGEST: usize = 64;
/// What a solve carries out at most (`Solved::carry`), in words: each clause its length and its literals. A clause
/// that does not fit in what is left is passed over, and the next tried.
pub const CARRY_ROOM: usize = 4096;

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
    /// A window's solve (`Resume`): what it learned that holds without its assumption, in order (clauses, then
    /// values learned at level 0); and, unsatisfiable, each variable whether its contradiction rests on it.
    pub carry: Vec<Vec<Lit>>,
    pub used: Option<Vec<bool>>,
    /// The device's cycles by part, over 256 (0 where its backend has no clock, and on the CPU): watch visits, gate
    /// items, analysis, jumps back, decisions, reductions.
    pub cycles: [u32; 6],
}

/// A formula's room for learned clauses, by its size: their literals (64 words a variable, from 16,384 to 2^20), and
/// a clause for each 16 of those. The search reduces them as they fill it. (A quarter of it reduced the hardest
/// Kotlin solves 99 times: they took 93,450 conflicts where the CPU's solver took 5,651, and with this room 7,387.
/// It is room, not memory touched: a search writes only the clauses it learns.)
pub fn arena(vars: usize) -> u32 {
    (64 * vars).clamp(1 << 14, 1 << 20) as u32
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
    /// Each gate's counter (`cs_cnt`): its arguments false (low 16 bits) and true (high 16), of the trail's first
    /// `counted` values; a round counts its literals before it judges their gates (`may`).
    cnt: Vec<u32>,
    counted: usize,
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
    /// The plain clauses: the formula's, then the carried ones; a learned clause's number is past them.
    plain: usize,
    /// The solve's own plain clauses (from `own_from` to the formula's count) end with `-assume`.
    own_from: usize,
    assume: Lit,
    /// Level 0's variables analysis left out of a clause (what the clauses it learned rest on), and the
    /// contradiction's clause: the roots of `used`.
    rested: Vec<bool>,
    refuted: Vec<Lit>,
    /// The trail's length when the search began: values learned at level 0 lie past it.
    began: usize,
    /// The formula's plain clauses read and gates checked: all of them, or a window's (`start`).
    given: usize,
    gates_n: usize,
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

/// The variables a window's solve carries out what it learned over (`Solved::carry`): the window's, where it
/// assumes a literal and its own gates each define a fresh variable from earlier ones (so they allow any values of
/// the window's the rest allow); else 0, nothing carried.
pub fn carry_vars(f: &Formula) -> usize {
    let Some(r) = f.resume else { return 0 };
    let g = f.gates;
    let defines = |i: usize| {
        let o = lvar(g.out(i));
        g.kind(i) != Kind::One && o > r.carry_vars && g.args(i).iter().all(|&a| lvar(a) < o)
    };
    if r.assume == 0 || !(f.base..g.len()).all(defines) || !g.out_distinct(f.base) {
        return 0;
    }
    r.carry_vars
}

/// A window at level 0 (decision 0670): its first `window_clauses` plain clauses and first `base` gates (`fan`'s),
/// over its first `vars` variables, propagated as the search propagates at level 0. Each of its solves resumes
/// from it (`Resume`).
pub fn start(vars: usize, gates: &Gates, base: usize, fan: &Fanout, clauses: &Clauses, window_clauses: usize) -> Start {
    let f = Formula::new(vars, gates, base, fan, None, clauses, 0);
    let mut s = Search::new(&f);
    (s.given, s.plain, s.gates_n) = (window_clauses, window_clauses, base);
    s.start();
    while s.status == Status::Running && s.head < s.trail.len() {
        s.rounds += 1;
        if s.round() {
            s.refuted = s.conflict_lits();
            s.status = Status::Unsat;
        }
    }
    let (mut set, mut conflict) = match gates.placed() {
        Some((b, _, _)) => (Grown::in_block(b), Grown::in_block(b)),
        None => (Grown::default(), Grown::default()),
    };
    for &l in &s.trail {
        set.extend_from_slice(&[l as u32, s.reason[lvar(l)], s.why[lvar(l)]]);
    }
    conflict.extend_from_slice(&s.refuted);
    Start { set, broken: s.status != Status::Running, conflict }
}

impl<'a> Search<'a> {
    fn new(f: &'a Formula<'a>) -> Self {
        let n = f.vars + 1;
        let (lcap, acap) = (learned(f.vars) as usize, arena(f.vars) as usize);
        let plain = f.clauses.len() + f.resume.map_or(0, |r| r.carried.len());
        let nodes = 2 * (plain + lcap);
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
            cnt: vec![0; f.gates.len()],
            counted: 0,
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
            plain,
            own_from: f.resume.map_or(usize::MAX, |r| if r.assume != 0 { r.window_clauses } else { usize::MAX }),
            assume: f.resume.map_or(0, |r| r.assume),
            rested: vec![false; n],
            refuted: Vec::new(),
            began: 0,
            given: f.clauses.len(),
            gates_n: f.gates.len(),
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
            carry: self.carry(),
            used: (self.status == Status::Unsat && self.f.resume.is_some()).then(|| self.used()),
            cycles: [0; 6],
        }
    }

    /// What a window's solve learned that holds without its assumption, over the window's variables (as
    /// `bitlex_csat::Solver::learned_without`): each learned clause it holds at the end of at most
    /// `CARRIED_LONGEST` literals without `-assume`, then each value it learned at level 0. Each rests on the window,
    /// the carried clauses and the solve's own clauses and gates; one without `-assume` used none of the own clauses,
    /// and the own gates, each defining a variable past the window's and its arguments, allow any values of the
    /// window's the rest allow. Where they do not, nothing is carried.
    fn carry(&self) -> Vec<Vec<Lit>> {
        let Some(r) = self.f.resume else { return Vec::new() };
        let vars = carry_vars(self.f);
        if vars == 0 {
            return Vec::new();
        }
        let within = |c: &[Lit]| c.iter().all(|&l| lvar(l) <= vars) && !c.contains(&-r.assume);
        let (mut out, mut used): (Vec<Vec<Lit>>, usize) = (Vec::new(), 0);
        let mut take = |c: &[Lit]| {
            if used + 1 + c.len() <= CARRY_ROOM {
                used += 1 + c.len();
                out.push(c.to_vec());
            }
        };
        for j in 0..self.learned {
            let c = &self.arena[self.lstart[j] as usize..self.lstart[j + 1] as usize];
            if c.len() <= CARRIED_LONGEST && within(c) {
                take(c);
            }
        }
        let zero = if self.level == 0 { self.trail.len() } else { self.limits[0] as usize };
        for &l in self.trail.get(self.began..zero).unwrap_or(&[]) {
            if self.reason[lvar(l)] == NONE && within(&[l]) {
                take(&[l]);
            }
        }
        out
    }

    /// After a contradiction, the variables it rests on: its clause's, those analysis left out at level 0, and back
    /// through each one's reason (a decision's, or a value given, stops it). One pass back along the trail: a
    /// reason's values lie before the value it implied.
    fn used(&self) -> Vec<bool> {
        let mut used = self.rested.clone();
        for &l in &self.refuted {
            used[lvar(l)] = true;
        }
        let mut buf = Vec::new();
        for i in (0..self.trail.len()).rev() {
            let v = lvar(self.trail[i]);
            if !used[v] || self.reason[v] == NONE {
                continue;
            }
            self.explain_into(v, &mut buf);
            for &q in &buf {
                used[lvar(q)] = true;
            }
        }
        used
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

    /// A clause's literals: the formula's plain ones (a solve's own with `-assume` last), the carried ones, then the
    /// learned ones, in the arena.
    fn cl_len(&self, id: usize) -> usize {
        let c = self.given;
        if id < c {
            self.f.clauses.get(id).len() + usize::from(id >= self.own_from)
        } else if id < self.plain {
            self.f.resume.map_or(0, |r| r.carried.get(id - c).len())
        } else {
            (self.lstart[id - self.plain + 1] - self.lstart[id - self.plain]) as usize
        }
    }
    fn cl_lit(&self, id: usize, k: usize) -> Lit {
        let c = self.given;
        if id < c {
            let lits = self.f.clauses.get(id);
            if k < lits.len() { lits[k] } else { -self.assume }
        } else if id < self.plain {
            self.f.resume.map_or(0, |r| r.carried.get(id - c)[k])
        } else {
            self.arena[self.lstart[id - self.plain] as usize + k]
        }
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
        let gates: &'a Gates = self.f.gates;
        let args = gates.args(g);
        let n = args.len();
        let o = gates.out(g);
        if write {
            self.cbuf.clear();
        }
        match self.kind(g) {
            0 => {
                let vo = self.value(o);
                let (mut unset, mut unset_at) = (0u32, 0usize);
                for i in 0..n {
                    let a = args[i];
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
                        } else if a != args[unset_at] {
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
                            let a = args[i];
                            self.cbuf.push(-a);
                        }
                    }
                } else if vo == -1 && unset == 1 {
                    r.mode = Mode::One;
                    r.lit = -args[unset_at];
                    r.why = unset_at as u32;
                }
            }
            1 => {
                let (a, b) = (args[0], args[1]);
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
                    let a = args[i];
                    let va = self.value(a);
                    if va == 1 {
                        if held != NONE {
                            r.mode = Mode::Conflict;
                            if write {
                                let h = args[held as usize];
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
                            let a = args[i];
                            self.cbuf.push(a);
                        }
                    }
                } else if unset == 1 {
                    r.mode = Mode::One;
                    r.lit = args[unset_at];
                    r.why = unset_at as u32;
                }
            }
        }
        r
    }

    /// `cs_count`: the trail's values from `from` to `to` added to (or taken from) their gates' counters.
    fn count(&mut self, from: usize, to: usize, add: bool) {
        let f = self.f;
        for i in from..to {
            let l = self.trail[i];
            let v = lvar(l);
            let window: (&[u32], &[u32]) = if v <= f.fan.vars() { (f.fan.of_var(v), f.fan.counts_of(v)) } else { (&[], &[]) };
            let own: (&[u32], &[u32]) = f.own.map_or((&[], &[]), |o| (o.of_var(v), o.counts_of(v)));
            for (gs, cs) in [window, own] {
                for (&g, &pn) in gs.iter().zip(cs) {
                    let (pos, neg) = (pn & 0xFFFF, pn >> 16);
                    let x = if l > 0 { neg | pos << 16 } else { pos | neg << 16 };
                    let c = &mut self.cnt[g as usize];
                    *c = if add { c.wrapping_add(x) } else { c.wrapping_sub(x) };
                }
            }
        }
    }

    /// `cs_may`: whether gate `g`'s check can find anything, from its counter and its output's value alone (its
    /// arguments not read). The counter lags the trail (its last values not counted yet): a gate passed over here
    /// is judged again when they are, in their round.
    fn may(&self, g: usize) -> bool {
        let gates = self.f.gates;
        let args = gates.args(g);
        let n = args.len() as u32;
        let x = self.cnt[g];
        let (f, t) = (x & 0xFFFF, x >> 16);
        match self.kind(g) {
            1 => lvar(args[0]) == lvar(args[1]) || f + t + u32::from(self.value(gates.out(g)) != 0) >= 2,
            _ if n >= 0x10000 => true,
            2 => {
                let u = n - f - t;
                t >= 2 || (t == 1 && u > 0) || (t == 0 && u <= 1)
            }
            _ => {
                let u = n - f - t;
                let vo = self.value(gates.out(g));
                if f > 0 {
                    vo != -1
                } else if vo == 1 {
                    u > 0
                } else if vo == 0 {
                    u == 0
                } else {
                    u <= 1 || gates.repeats(g)
                }
            }
        }
    }

    /// `cs_items`: up to 32 gates, a lane each in order. A conflict (the first lane's, its clause in `cbuf`), or each
    /// step's literals on the trail: of the lanes that set one variable the first, if it is still unset. True on a
    /// conflict.
    fn items(&mut self, gs: &[u32]) -> bool {
        let mut ks = [QUIET; LANES];
        let mut acts = false;
        for (i, &g) in gs.iter().enumerate() {
            let k = if self.may(g as usize) { self.check(g as usize, false) } else { QUIET };
            if k.mode == Mode::Conflict {
                // The first lane's conflict: the lanes before it are quiet or imply, and a conflict ends the round.
                self.check(g as usize, true);
                self.conflict = NONE;
                return true;
            }
            acts |= k.mode != Mode::None;
            ks[i] = k;
        }
        // Most items imply nothing.
        if !acts {
            return false;
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
                if !once[i] && ks[i].mode != Mode::And && ks[i].mode != Mode::Held {
                    continue;
                }
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
            // The lanes with a literal, in order; of those on one variable the first wins, if it is still unset (all
            // read before any is placed).
            let mut live = [0u8; LANES];
            let mut nl = 0;
            for i in 0..gs.len() {
                if ls[i].0 != 0 {
                    live[nl] = i as u8;
                    nl += 1;
                }
            }
            if nl == 0 {
                return false;
            }
            let mut wins = [false; LANES];
            for a in 0..nl {
                let i = live[a] as usize;
                let l = ls[i].0;
                wins[i] = !live[..a].iter().any(|&j| lvar(ls[j as usize].0) == lvar(l)) && self.value(l) == 0;
            }
            for &i in &live[..nl] {
                let i = i as usize;
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
        let nodes = 2 * (self.plain + self.lcap);
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

    fn fan_len(&self, v: usize) -> usize {
        let f = self.f;
        (if v <= f.fan.vars() { f.fan.of_var(v).len() } else { 0 }) + f.own.map_or(0, |o| o.of_var(v).len())
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
        // Their values counted, then their gates in order, 32 at a time (the order `cs_round` takes them in).
        let from = self.counted.max(hd);
        self.count(from, hd + f, true);
        self.counted = hd + f;
        let total: usize = lits[..f].iter().map(|&l| self.fan_len(lvar(l))).sum();
        self.items = self.items.wrapping_add(total as u32);
        let mut chunk = [0u32; LANES];
        let mut n = 0;
        for &l in &lits[..f] {
            for g in self.fan_of(lvar(l)) {
                chunk[n] = g;
                n += 1;
                if n == LANES {
                    if self.items(&chunk) {
                        return true;
                    }
                    n = 0;
                }
            }
        }
        if n > 0 {
            if self.items(&chunk[..n]) {
                return true;
            }
        }
        self.head = hd + f;
        false
    }

    // The decisions' heap (`bitlex_sat::Order`'s): its top is always the most active variable in it, the kernel's
    // tree's top (`cs_top`).
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
        if r & GATE == 0 {
            return Some(r as usize);
        }
        let mut buf = std::mem::take(&mut self.ebuf);
        self.explain_into(v, &mut buf);
        self.ebuf = buf;
        None
    }

    /// `v`'s reason's clause into `buf`: a numbered clause's literals, or a gate's explanation (`explain`).
    fn explain_into(&self, v: usize, buf: &mut Vec<Lit>) {
        let r = self.reason[v];
        let w = self.why[v] as usize;
        buf.clear();
        if r & GATE == 0 {
            buf.extend((0..self.cl_len(r as usize)).map(|k| self.cl_lit(r as usize, k)));
            return;
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
                        buf.push(o);
                        buf.extend(args.iter().map(|&a| -a));
                    } else {
                        buf.extend([-o, args[w]]);
                    }
                } else if l == args[w] {
                    buf.extend([l, -o]);
                } else {
                    buf.extend([l, o]);
                    buf.extend((0..n).filter(|&i| i != w).map(|i| -args[i]));
                }
            }
            1 => {
                buf.push(l);
                if w as u32 != FIXED {
                    let p = [o, args[0], args[1]];
                    for (k, q) in p.into_iter().enumerate() {
                        if k != w {
                            let e = if self.value(q) == 1 { -q } else { q };
                            buf.push(e);
                        }
                    }
                }
            }
            _ => {
                if l == args[w] {
                    buf.push(l);
                    buf.extend((0..n).filter(|&i| i != w).map(|i| args[i]));
                } else {
                    buf.extend([l, -args[w]]);
                }
            }
        }
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
                    if q != p && lv == 0 && q != 0 {
                        self.rested[v] = true;
                    }
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
        if self.counted > start {
            self.count(start, self.counted, false);
            self.counted = start;
        }
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
        // At level 0, or at the assumption's: unsatisfiable, no conflict spent.
        if self.level == 0 || (self.level == 1 && self.assume != 0 && self.trail[self.limits[0] as usize] == self.assume) {
            self.refuted = self.conflict_lits();
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
            // Two activities scaled to one are ordered by their numbers now: the heap made again, so its top is
            // the most active variable as the kernel's tree's is.
            for i in (0..self.heap_n / 2).rev() {
                self.down(i);
            }
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
            let id = self.plain + j;
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

    /// The conflict's clause: a numbered one, or `cbuf`'s.
    fn conflict_lits(&self) -> Vec<Lit> {
        if self.conflict != NONE {
            let id = self.conflict as usize;
            (0..self.cl_len(id)).map(|k| self.cl_lit(id, k)).collect()
        } else {
            self.cbuf.clone()
        }
    }

    /// `cs_reduce`: the learned clauses reduced, at level 0. False where a clause is now false at level 0.
    fn reduce(&mut self) -> bool {
        if self.level > 0 {
            self.back_to(0);
        }
        self.whead.fill(NONE);
        let plain = self.plain;
        for i in 0..self.trail.len() {
            let v = lvar(self.trail[i]);
            let r = self.reason[v];
            if r & GATE == 0 && r != NONE && r as usize >= plain {
                // What the clause read stays among `used`'s roots: the clause goes.
                for k in 0..self.cl_len(r as usize) {
                    let q = self.cl_lit(r as usize, k);
                    self.rested[lvar(q)] = true;
                }
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
                self.refuted = (0..self.cl_len(id)).map(|k| self.cl_lit(id, k)).collect();
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
        // The assumption first, at level 1; false at level 0, the solve is refuted by what implied it.
        if self.level == 0 && self.assume != 0 {
            match self.value(self.assume) {
                0 => {
                    self.limits[0] = self.trail.len() as u32;
                    self.level = 1;
                    self.assign(self.assume, NONE, 0);
                    return;
                }
                -1 => {
                    self.refuted = vec![self.assume];
                    self.status = Status::Unsat;
                    return;
                }
                _ => {}
            }
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

    /// `cs_start`: the plain clauses watched (their units set), the gates that imply with nothing set checked. A
    /// window's solve (`cs_resume`): the window's values at level 0 set as its `Start` set them; every plain and
    /// carried clause watched by two literals not false (one level 0 satisfies left out, one with a single such
    /// literal implying it); each of the solve's own gates checked once.
    fn start(&mut self) {
        let vars = self.f.vars;
        for v in 1..=vars {
            self.hat[v] = (vars - v) as u32;
            self.heap[v - 1] = (vars - (v - 1)) as u32;
        }
        self.heap_n = vars;
        if let Some(r) = self.f.resume {
            for t in r.start.set.chunks(3) {
                self.assign(t[0] as Lit, t[1], t[2]);
            }
            self.head = self.trail.len();
            self.count(0, self.head, true);
            self.counted = self.head;
            if r.start.broken {
                self.refuted = r.start.conflict.to_vec();
                self.status = Status::Unsat;
            }
            for id in 0..self.plain {
                if self.status != Status::Running {
                    break;
                }
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
                    self.refuted = (0..self.cl_len(id)).map(|k| self.cl_lit(id, k)).collect();
                    self.status = Status::Unsat;
                } else if b == 0 {
                    self.assign(a, id as u32, 0);
                } else {
                    self.watch(id, a, b);
                }
            }
            self.began = self.trail.len();
            if self.status != Status::Running {
                return;
            }
            let own: Vec<u32> = (self.f.base as u32..self.gates_n as u32).collect();
            for gs in own.chunks(LANES) {
                if self.items(gs) {
                    self.refuted = self.conflict_lits();
                    self.status = Status::Unsat;
                    return;
                }
            }
            return;
        }
        for id in 0..self.given {
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
                    -1 => {
                        self.refuted = vec![a];
                        self.status = Status::Unsat;
                    }
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
        let gates = self.gates_n;
        for g0 in (0..gates).step_by(LANES) {
            let gs: Vec<u32> = (g0..(g0 + LANES).min(gates)).filter(|&g| self.seed(g)).map(|g| g as u32).collect();
            if gs.is_empty() {
                continue;
            }
            if self.items(&gs) {
                self.refuted = self.conflict_lits();
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
        assert_eq!(device.carry, here.carry, "formula {i}: what it carries out, on the device and on the CPU");
        assert_eq!(device.used, here.used, "formula {i}: its core, on the device and on the CPU");
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
            let f = Formula::new(vars, &g, base, &fan, (base < g.len()).then_some(&own_fan), &c, 100_000);
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
    /// A window's solves resumed from its `Start`, each with its own clauses (with `-assume`) and the clauses its
    /// earlier solves carried (as `bitlex_csat`'s test of its resumed windows): each answer is the whole formula's
    /// (`bitlex_sat`), each model holds, each carried clause the window implies, and an own unit `used` leaves out
    /// is not needed.
    #[test]
    fn a_resumed_window_answers_as_a_solve() {
        use crate::bitlex_sat;
        let block = FormulaBlock::new();
        let mut rng = Rng(0x6a09e667f3bcc908);
        let (mut sat, mut unsat, mut carried, mut dropped, mut searched) = (0, 0, 0, 0, 0);
        for i in 0..1500usize {
            let inputs = 4 + rng.below(30) as usize;
            let (gn, cn) = (rng.below(100) as usize, rng.below(2 * inputs as u64 + 4) as usize);
            let own_gates = rng.below(6) as usize;
            let (gates, clauses, vars, base) = circuit(&mut rng, &block, inputs, gn, cn, own_gates);
            let wvars = (0..base).filter(|&g| gates.kind(g) != Kind::One).map(|g| lvar(gates.out(g))).chain((0..base).flat_map(|g| gates.args(g).iter().map(|&a| lvar(a)))).max().unwrap_or(1).max(1);
            let (mut window, mut rest) = (Vec::new(), Vec::new());
            for c in clauses.iter() {
                if c.iter().all(|&l| lvar(l) <= wvars) { window.push(c.to_vec()) } else { rest.push(c.to_vec()) }
            }
            let mut wcnf = Clauses::default();
            for c in &window {
                wcnf.push(c);
            }
            let mut wg = gates.clone();
            wg.truncate(base);
            wg.cnf(&mut wcnf);
            let fan = Fanout::of(&gates, base, wvars);
            let sel = (vars + 1) as Lit;
            let own_fan = Fanout::of_range(&gates, base..gates.len(), vars + 1);
            let mut learned = Clauses::in_block(&block);
            let mut start_of: Option<Start> = None;
            for k in 0..4 {
                let mut own: Vec<Vec<Lit>> = rest.clone();
                for _ in 0..rng.below(6) {
                    own.push(vec![((rng.below(vars as u64) + 1) as Lit) * if rng.below(2) == 0 { 1 } else { -1 }]);
                }
                let mut all = Clauses::in_block(&block);
                for c in window.iter().chain(&own) {
                    all.push(c);
                }
                let start = start_of.get_or_insert_with(|| start(wvars, &gates, base, &fan, &all, window.len()));
                let r = Resume { start, window_clauses: window.len(), carried: &learned, assume: sel, carry_vars: wvars };
                let f = Formula { resume: Some(r), ..Formula::new(vars + 1, &gates, base, &fan, (base < gates.len()).then_some(&own_fan), &all, 100_000) };
                let x = solve(&f);
                searched += usize::from(x.conflicts > 0);
                let mut whole = Clauses::default();
                for c in window.iter().chain(&own) {
                    whole.push(c);
                }
                gates.cnf(&mut whole);
                let mut o = bitlex_sat::Solver::new(vars);
                o.add_all(&whole);
                match (&x.answer, o.solve(u32::MAX)) {
                    (Answer::Sat(m), CpuAnswer::Sat) => {
                        let mut plain = Clauses::default();
                        for c in window.iter().chain(&own) {
                            plain.push(c);
                        }
                        assert!(holds(m, &gates, &plain), "formula {i}, solve {k}: not a model");
                        sat += 1;
                    }
                    (Answer::Unsat, CpuAnswer::Unsat) => {
                        unsat += 1;
                        let used = x.used.as_ref().expect("a resumed solve's core");
                        let mut kept = wcnf.clone();
                        gates.cnf(&mut kept);
                        for c in &own {
                            if c.len() == 1 && !used[lvar(c[0])] {
                                dropped += 1;
                            } else {
                                kept.push(c);
                            }
                        }
                        let mut o = bitlex_sat::Solver::new(vars);
                        o.add_all(&kept);
                        assert_eq!(o.solve(u32::MAX), CpuAnswer::Unsat, "formula {i}, solve {k}: a unit it did not use was needed");
                    }
                    (a, b) => panic!("formula {i}, solve {k}: the search {a:?}, the whole formula {b:?}"),
                }
                for c in &x.carry {
                    // Implied by the window's: with its negation added, they are unsatisfiable.
                    let mut w = wcnf.clone();
                    for &l in c {
                        w.push(&[-l]);
                    }
                    let mut o = bitlex_sat::Solver::new(vars);
                    o.add_all(&w);
                    assert_eq!(o.solve(u32::MAX), CpuAnswer::Unsat, "formula {i}, solve {k}: {c:?} carried, not the window's");
                    learned.push(c);
                    carried += 1;
                }
            }
        }
        assert!(sat > 300 && unsat > 300 && dropped > 100 && carried > 50 && searched > 100, "{sat} satisfiable, {unsat} not, {dropped} units dropped, {carried} carried, {searched} searched");
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
            let f = Formula::new(vars, &g, base, &fan, (base < g.len()).then_some(&own), &c, 100_000);
            let t = std::time::Instant::now();
            let x = solve(&f);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            s.reset(vars);
            for cl in c.iter() {
                s.add(cl);
            }
            let (t, before) = (std::time::Instant::now(), s.conflicts);
            let cpu = s.solve(&g, &fan, base, 100_000);
            let (cpu_ms, cpu_conflicts) = (t.elapsed().as_secs_f64() * 1e3, s.conflicts - before);
            match (&x.answer, cpu) {
                (Answer::Sat(m), CpuAnswer::Sat) => assert!(holds(m, &g, &c), "{p:?}: the model does not hold"),
                (Answer::Unsat, CpuAnswer::Unsat) | (Answer::Unknown, _) => {}
                (a, b) => panic!("{p:?}: the search {a:?}, the solver {b:?}"),
            }
            eprintln!(
                "{}: {:?}, {} conflicts, {} decisions, {} rounds, {} gate items, {} watch visits, {} reductions, {ms:.1} ms; the CPU solver {cpu_conflicts} conflicts, {cpu_ms:.1} ms",
                p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
                x.answer == Answer::Unsat,
                x.conflicts,
                x.decisions,
                x.rounds,
                x.items,
                x.visits,
                x.reductions
            );
        }
    }
}
