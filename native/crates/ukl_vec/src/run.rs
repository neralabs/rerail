//! A kernel's code tape (`code`) run by the interpreter (decision 0567 §6, the first back end): a SIMD group of 32
//! lanes at a time, an instruction a whole register. The instructions are read in place from the tape's one node, and
//! each is one jump through a table (`match` on the fused opcode): its handler is a loop over the 32 lanes with
//! nothing left to decide, which the compiler makes vector code (NEON; AVX2 where the CPU has it, chosen at run
//! time). A mask's lanes are packed by NEON's adds across a vector.
//!
//! Compiled (`jit`, `Exec::Jit`), the same state runs: the compiled sections between their stops, and the driver here
//! doing at a stop what the interpreter's loop does.
//!
//! A threadgroup's SIMD groups take turns: each runs to its next threadgroup barrier (`Bar`) or its end, then the
//! next; once all have, they all go on. The barrier rule (decision 0567 §3), proven by `vector`, makes every SIMD
//! group that has not ended wait at the same barrier; one that does not fails the kernel.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::Error;
use crate::code::{CUR, HEADER, I, TABLE, WORDS};
use crate::k::{Builtin, M, Op, View};
use crate::lower::Program;
use crate::ops;
use crate::vector::Kernel;

/// The SIMD group's lanes.
pub const W: usize = 32;

type V32 = [u32; W];
type V64 = [u64; W];

/// A kernel argument: its bytes (a pointer's address, a structure's bytes), or an offset in the group's
/// threadgroup memory past the kernel's own (`set_thread_group_memory`).
#[derive(Clone, Debug)]
pub enum Arg {
    Bytes(Vec<u8>),
    Local(u64),
}

type R<T> = Result<T, Error>;

/// How a kernel's code tape runs: the interpreter's loop, or compiled (`jit`, its sections between the stops).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exec {
    Interpret,
    Jit,
    /// The kernel's SSA compiled (`cl`), its SIMD groups fibers (`exec`).
    Ssa,
}

/// Why a SIMD group's turn ended: its end, a threadgroup barrier (its site), or a poll of another group's progress.
enum Stop {
    Halt,
    Bar(u32),
    Yield,
}

/// What a SIMD group that has not ended waits for: nothing (it runs on its group's next round), its group at a
/// barrier (the site), or the other groups' turns (a poll).
#[derive(Clone, Copy, PartialEq)]
enum Wait {
    Run,
    Bar(u32),
    Yield,
}

/// A resident group's slot: its SIMD groups, their frames, its threadgroup memory and its checked decisions, and
/// the group it runs.
struct Slot {
    /// The memory its SIMD groups' frames are in (each `Sg::frame`), held.
    _frames: Vec<u128>,
    shared: Vec<u128>,
    g: Vec<Sg>,
    agreed: HashMap<u32, Decisions>,
    group: [u32; 3],
    busy: bool,
}

/// A SIMD group: where it is, its frame (its lanes' variables, struct of arrays) and its registers.
struct Sg {
    index: u32,
    pc: usize,
    done: bool,
    wait: Wait,
    frame: u64,
    s: Vec<u64>,
    v32: Vec<V32>,
    v64: Vec<V64>,
    m: Vec<u32>,
    /// Each lane's id in its group, by dimension.
    lid: [V32; 3],
    /// The running function's frame, and its windows of the scalar, 32-bit, 64-bit and mask files.
    fp: u64,
    win: [usize; 4],
    /// The calls it is in: each caller's instruction to go back to, frame and windows.
    calls: Vec<(usize, u64, [usize; 4])>,
}

thread_local! {
    /// The thread's last launches' slots, by their shape (`run`), the latest first: for its next launch of one.
    static SLOTS: RefCell<Vec<([usize; 11], Vec<Slot>)>> = const { RefCell::new(Vec::new()) };
}

/// The shapes of slots a thread keeps.
const KEPT_SHAPES: usize = 4;

/// A checked branch's decisions in the group, in order, and how many each SIMD group has made.
type Decisions = (Vec<u64>, Vec<usize>);

/// One dispatch's state.
struct Cx<'p> {
    p: &'p Program,
    kernel: &'p str,
    code: &'p [u32],
    /// Each function's advances (`code::TABLE`).
    table: &'p [u32],
    data: u64,
    shared: u64,
    grid: [u32; 3],
    block: [u32; 3],
    group: [u32; 3],
    agreed: RefCell<HashMap<u32, Decisions>>,
    /// The compiled sections, when they run it, and the interpreter's step they call (`jit::Places::step`).
    jit: Option<&'p crate::jit::Jit>,
    step: crate::jit::Step,
    /// The compiled sections' entries, which a direct call takes its callee's from (`jit::Places::entries`).
    entries: *const crate::jit::Entry,
    /// An interpreter's step's failure from the compiled code.
    fault: RefCell<Option<Error>>,
    #[cfg(target_arch = "x86_64")]
    avx2: bool,
}

/// The lanes of a mask, in order.
#[inline(always)]
fn lanes(mut m: u32) -> impl Iterator<Item = usize> {
    std::iter::from_fn(move || {
        if m == 0 {
            return None;
        }
        let l = m.trailing_zeros() as usize;
        m &= m - 1;
        Some(l)
    })
}

/// The first lane of a mask (lane 0 of an empty one).
#[inline(always)]
fn first(m: u32) -> usize {
    (m.trailing_zeros() as usize) & (W - 1)
}

/// The lanes of `v` that are not 0.
#[inline(always)]
fn tomask(v: &V32) -> u32 {
    #[cfg(target_arch = "aarch64")]
    // SAFETY: NEON is every aarch64 CPU's; the loads are within `v` and the constant.
    unsafe {
        use std::arch::aarch64::*;
        let bits = vld1q_u32([1u32, 2, 4, 8].as_ptr());
        let mut m = 0u32;
        for k in 0..W / 4 {
            let x = vld1q_u32(v.as_ptr().add(4 * k));
            m |= vaddvq_u32(vandq_u32(vtstq_u32(x, x), bits)) << (4 * k);
        }
        m
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let mut m = 0u32;
        for (l, x) in v.iter().enumerate() {
            m |= u32::from(*x != 0) << l;
        }
        m
    }
}

/// `o`'s lanes of `mask` set to `x`'s.
#[inline(always)]
fn blend32(o: &mut V32, x: &V32, mask: u32) {
    for l in 0..W {
        let k = 0u32.wrapping_sub((mask >> l) & 1);
        o[l] = (x[l] & k) | (o[l] & !k);
    }
}

#[inline(always)]
fn blend64(o: &mut V64, x: &V64, mask: u32) {
    for l in 0..W {
        let k = 0u64.wrapping_sub(u64::from((mask >> l) & 1));
        o[l] = (x[l] & k) | (o[l] & !k);
    }
}

/// The bits of `m`'s width.
fn width(m: M) -> u64 {
    if m.bits() >= 64 { u64::MAX } else { (1u64 << m.bits()) - 1 }
}

/// A 32-bit lane's value as `m`'s normalized `u64`.
#[inline(always)]
fn wide(m: M, x: u32) -> u64 {
    ops::norm(m, u64::from(x))
}

/// A launch: its grid of groups, its groups' threads, the threadgroup memory a group has past the kernel's own (its
/// `Arg::Local` arguments'), the groups that run together, and how the code runs.
#[derive(Clone, Copy, Debug)]
pub struct Launch {
    pub grid: [u32; 3],
    pub block: [u32; 3],
    pub local: u64,
    pub resident: usize,
    pub exec: Exec,
}

/// Runs `k` (a kernel of `p`) over `launch`'s grid of groups, with `args` for its parameters. Up to `resident` groups
/// run together, as the CPU's coroutines do (`ukl_cpu_shim.h`): each in a slot of its own that takes the launch's
/// next group when its own ends, the slots taking turns in order, a group's turn ending when all its SIMD groups
/// poll another's progress (`Yield`). So a persistent grid of up to `resident` groups meets its barriers, and a
/// larger one fails their bound, as on a GPU that holds fewer; a run is the same every time.
pub fn run(p: &Program, k: &Kernel, args: &[Arg], launch: Launch) -> R<()> {
    // A development hook: each launch's kernel, executor, groups and time, with `RERAIL_UKL_VEC_TIMES`.
    if std::env::var_os("RERAIL_UKL_VEC_TIMES").is_some() {
        let t = std::time::Instant::now();
        let r = run_in(p, k, args, launch);
        eprintln!("LAUNCH {} {:?} {:?}x{:?} {:.1} us", k.name, launch.exec, launch.grid, launch.block, t.elapsed().as_secs_f64() * 1e6);
        return r;
    }
    run_in(p, k, args, launch)
}

fn run_in(p: &Program, k: &Kernel, args: &[Arg], launch: Launch) -> R<()> {
    if launch.exec == Exec::Ssa {
        return crate::exec::run(p, k, args, launch);
    }
    let Launch { grid, block, local, resident, exec } = launch;
    let cv = View::new(&k.code);
    let words = cv.p(0);
    let (frame_bytes, own, n_params) = (words[0] as usize, u64::from(words[1]).next_multiple_of(16), words[2] as usize);
    let (ns, n32, n64, nm, ncode) = (words[3] as usize, words[4] as usize, words[5] as usize, words[6] as usize, words[7] as usize);
    let params = &words[HEADER..HEADER + 2 * n_params];
    let code = &words[HEADER + 2 * n_params..HEADER + 2 * n_params + ncode];
    let table = &words[HEADER + 2 * n_params + ncode..][..words[8] as usize * TABLE];
    if args.len() != n_params {
        return Err(Error(format!("{} takes {n_params} arguments, given {}", k.name, args.len())));
    }
    if k.lid && !(block[1] == 1 && block[2] == 1) && !(block[0] as usize).is_multiple_of(W) {
        return Err(Error(format!("{} takes a thread's id in its group as its SIMD group's and lane's: a group of {block:?} is not one", k.name)));
    }
    let threads = block.iter().map(|&b| b as usize).product::<usize>();
    let nsg = threads.div_ceil(W);
    let frame = frame_bytes.next_multiple_of(16) * W;
    let groups = grid.iter().map(|&g| g as usize).product::<usize>();
    if groups == 0 {
        return Ok(());
    }
    let jit = match exec {
        Exec::Interpret => None,
        Exec::Jit => Some(compiled(k)?),
        Exec::Ssa => unreachable!("run by `exec`"),
    };
    let data = p.data().as_ptr() as u64;
    let mut cx = Cx {
        p,
        kernel: &k.name,
        code,
        table,
        data,
        shared: 0,
        grid,
        block,
        group: [0; 3],
        agreed: RefCell::new(HashMap::new()),
        jit,
        step,
        entries: jit.map_or(std::ptr::null(), |j| j.entries().as_ptr()),
        fault: RefCell::new(None),
        #[cfg(target_arch = "x86_64")]
        avx2: std::arch::is_x86_feature_detected!("avx2"),
    };
    let (bx, by) = (block[0].max(1), block[1].max(1));
    // The slots: this thread's last launch's, if it had this one's shape (their frames and files as they were left:
    // a lane's variables start as C's locals do, unset, so a group's start clears only its threadgroup memory).
    let shape = [nsg, frame, ns, n32, n64, nm, (own + local) as usize, threads, bx as usize, by as usize, resident.clamp(1, groups)];
    let kept = SLOTS.with(|c| {
        let mut c = c.borrow_mut();
        let at = c.iter().position(|e| e.0 == shape)?;
        Some(c.remove(at).1)
    });
    let mut slots: Vec<Slot> = kept.filter(|v| !v.is_empty()).unwrap_or_else(|| (0..resident.clamp(1, groups))
        .map(|_| {
            let mut frames = vec![0u128; (nsg * frame).div_ceil(16).max(1)];
            let base = frames.as_mut_ptr() as u64;
            let g = (0..nsg as u32)
                .map(|s| {
                    let lin = |l: usize| s * W as u32 + l as u32;
                    let lid = [std::array::from_fn(|l| lin(l) % bx), std::array::from_fn(|l| lin(l) / bx % by), std::array::from_fn(|l| lin(l) / (bx * by))];
                    Sg {
                        index: s,
                        pc: 0,
                        done: false,
                        wait: Wait::Run,
                        frame: base + u64::from(s) * frame as u64,
                        fp: 0,
                        win: [0; 4],
                        calls: Vec::new(),
                        s: vec![0; ns.max(1)],
                        v32: vec![[0; W]; n32.max(1)],
                        v64: vec![[0; W]; n64.max(1)],
                        m: vec![0; nm.max(8)],
                        lid,
                    }
                })
                .collect();
            Slot { _frames: frames, shared: vec![0u128; (own + local).div_ceil(16) as usize + 1], g, agreed: HashMap::new(), group: [0; 3], busy: false }
        })
        .collect());
    for sl in &mut slots {
        sl.busy = false;
    }
    let live = |s: usize| {
        let n = (threads - s * W).min(W);
        if n == W { u32::MAX } else { (1u32 << n) - 1 }
    };
    // A slot's next group: its memory cleared, each SIMD group at the entry with its lanes and its arguments.
    let start = |sl: &mut Slot, at: usize| {
        let (gx, gy) = (grid[0] as usize, grid[1] as usize);
        sl.group = [(at % gx) as u32, (at / gx % gy) as u32, (at / (gx * gy)) as u32];
        sl.agreed.clear();
        sl.shared.fill(0);
        let sh = sl.shared.as_mut_ptr() as u64;
        for (s, sg) in sl.g.iter_mut().enumerate() {
            let mask = live(s);
            sg.m.fill(0);
            sg.m[CUR as usize] = mask;
            sg.pc = 0;
            (sg.fp, sg.win) = (sg.frame, [0; 4]);
            sg.calls.clear();
            (sg.done, sg.wait) = (false, Wait::Run);
            for (k, a) in args.iter().enumerate() {
                let (off, m) = (u64::from(params[2 * k]), M(params[2 * k + 1]));
                for l in lanes(mask) {
                    let at = sg.frame + off * W as u64 + l as u64 * u64::from(m.bytes());
                    match a {
                        // SAFETY: the parameter's place in the lane's frame, its type's bytes.
                        Arg::Bytes(b) => unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), at as usize as *mut u8, b.len().min(m.bytes() as usize)) },
                        // SAFETY: the same, a pointer's.
                        Arg::Local(o) => unsafe { ops::store(M::PTR, at, sh + own + o) },
                    }
                }
            }
        }
        sl.busy = true;
    };
    let mut next = 0usize;
    loop {
        let mut any = false;
        for sl in &mut slots {
            // The slot's turn: its group's, until all its SIMD groups poll; a group that ends gives way to the next.
            loop {
                if !sl.busy {
                    if next == groups {
                        break;
                    }
                    start(sl, next);
                    next += 1;
                }
                any = true;
                (cx.shared, cx.group) = (sl.shared.as_mut_ptr() as u64, sl.group);
                std::mem::swap(cx.agreed.get_mut(), &mut sl.agreed);
                let ended = cx.group_run(&mut sl.g);
                std::mem::swap(cx.agreed.get_mut(), &mut sl.agreed);
                if !ended? {
                    break;
                }
                sl.busy = false;
            }
        }
        if !any {
            SLOTS.with(|c| {
                let mut c = c.borrow_mut();
                c.insert(0, (shape, slots));
                c.truncate(KEPT_SHAPES);
            });
            return Ok(());
        }
    }
}

/// Kernel `k`'s code tape compiled (`jit`): on the first call, then kept with the kernel.
pub fn compiled(k: &Kernel) -> R<&crate::jit::Jit> {
    compiled_kept(k, || None, |_| {})
}

/// The same, from its kept form (`jit::load`), which `kept` reads back, else compiled, its kept form given to
/// `keep`.
pub fn compiled_kept(k: &Kernel, kept: impl FnOnce() -> Option<Vec<u8>>, keep: impl FnOnce(&[u8])) -> R<&crate::jit::Jit> {
    let cv = View::new(&k.code);
    let words = cv.p(0);
    let (n_params, ncode) = (words[2] as usize, words[7] as usize);
    let code = &words[HEADER + 2 * n_params..HEADER + 2 * n_params + ncode];
    let places = crate::jit::Places {
        pc: std::mem::offset_of!(Sg, pc) as i32,
        shared: std::mem::offset_of!(Cx, shared) as i32,
        data: std::mem::offset_of!(Cx, data) as i32,
        step: std::mem::offset_of!(Cx, step) as i32,
        fp: std::mem::offset_of!(Sg, fp) as i32,
        win: std::mem::offset_of!(Sg, win) as i32,
        entries: std::mem::offset_of!(Cx, entries) as i32,
    };
    let table = &words[HEADER + 2 * n_params + ncode..][..words[8] as usize * TABLE];
    k.jit
        .get_or_init(|| {
            if let Some(j) = kept().and_then(|b| crate::jit::load(code, &b)) {
                return Ok(j);
            }
            let (j, bytes) = crate::jit::compile(&k.name, code, table, places).map_err(|e| e.0)?;
            if let Some(b) = bytes {
                keep(&b);
            }
            Ok(j)
        })
        .as_ref()
        .map_err(|e| Error(format!("{}: {e}", k.name)))
}

impl Cx<'_> {
    fn site(&self, w: u32) -> &str {
        self.p.sites.get(w as usize).map_or("?", String::as_str)
    }

    fn err(&self, e: &str) -> Error {
        Error(format!("{}: {e}", self.kernel))
    }

    /// The group's SIMD groups, each to its next barrier, poll or end, in turns: whether the group has ended (else
    /// all its SIMD groups that have not wait for the other groups' turns, or some do and the rest wait at a barrier).
    fn group_run(&self, g: &mut [Sg]) -> R<bool> {
        loop {
            for sg in g.iter_mut().filter(|sg| !sg.done && sg.wait == Wait::Run) {
                match self.exec(sg)? {
                    Stop::Halt => sg.done = true,
                    Stop::Bar(site) => sg.wait = Wait::Bar(site),
                    Stop::Yield => sg.wait = Wait::Yield,
                }
            }
            let mut at: Option<(usize, u32)> = None;
            let (mut polls, mut waits) = (false, false);
            for sg in g.iter().filter(|sg| !sg.done) {
                match sg.wait {
                    Wait::Yield => polls = true,
                    Wait::Bar(site) => {
                        waits = true;
                        match at {
                            Some((pc, other)) if pc != sg.pc && !polls => {
                                return Err(Error(format!(
                                    "{}: SIMD group {} waits at {} while another waits at {}: the group's threads meet different barriers (decision 0567 §3)",
                                    self.kernel,
                                    sg.index,
                                    self.site(site),
                                    self.site(other)
                                )));
                            }
                            _ => at = Some((sg.pc, site)),
                        }
                    }
                    Wait::Run => unreachable!("a SIMD group that ran stopped"),
                }
            }
            if polls {
                // Those that poll go on at the group's next turn; those at a barrier wait there for them.
                for sg in g.iter_mut().filter(|sg| sg.wait == Wait::Yield) {
                    sg.wait = Wait::Run;
                }
                return Ok(false);
            }
            if !waits {
                return Ok(true);
            }
            for sg in g.iter_mut().filter(|sg| !sg.done) {
                sg.wait = Wait::Run;
            }
        }
    }

    /// A checked branch's decision `d`, by SIMD group `sg`: the same as the first SIMD group's, or the kernel fails.
    fn agree(&self, node: u32, sg: u32, d: u64, site: u32) -> R<()> {
        let mut a = self.agreed.borrow_mut();
        let (ds, at) = a.entry(node).or_default();
        let s = sg as usize;
        if at.len() <= s {
            at.resize(s + 1, 0);
        }
        let k = at[s];
        at[s] += 1;
        if k == ds.len() {
            ds.push(d);
        } else if ds[k] != d {
            return Err(Error(format!(
                "{}: {} decides differently in SIMD group {s} than in the first: an agreed value the barrier rule rests on differs across the group (decision 0567 §3)",
                self.kernel,
                self.site(site)
            )));
        }
        Ok(())
    }

    /// A builtin with no effect beyond its lane: a size or id's (`a` the dimension), or the lane's own operation on
    /// `a`, `x` (values of `from`, the result `m`).
    fn lane_builtin(&self, b: Builtin, m: M, from: M, a: u64, x: u64, y: u64) -> R<u64> {
        let d = a as usize;
        let dim = |v: [u32; 3], or: u32| if d < 3 { v[d] } else { or };
        Ok(match b {
            Builtin::LocalSize => u64::from(dim(self.block, 1)),
            Builtin::GroupId => u64::from(dim(self.group, 0)),
            Builtin::NumGroups => u64::from(dim(self.grid, 1)),
            Builtin::GlobalSize => u64::from(dim(self.grid, 1)) * u64::from(dim(self.block, 1)),
            Builtin::Popcount => u64::from((ops::norm(m, a) & width(m)).count_ones()),
            Builtin::Clz => u64::from((ops::norm(m, a) & width(m)).leading_zeros() - (64 - m.bits())),
            Builtin::MulHi => {
                if m.bits() == 64 {
                    if m.signed() { ((i128::from(a as i64) * i128::from(x as i64)) >> 64) as u64 } else { ((u128::from(a) * u128::from(x)) >> 64) as u64 }
                } else if m.signed() {
                    ops::norm(m, ((a as i64).wrapping_mul(x as i64) >> m.bits()) as u64)
                } else {
                    ops::norm(m, (ops::norm(m, a).wrapping_mul(ops::norm(m, x))) >> m.bits())
                }
            }
            Builtin::As => ops::norm(m, a & width(from)),
            Builtin::Min | Builtin::Max => {
                let less = ops::bin(Op::Lt, m, a, x).unwrap_or(0) != 0;
                if less == (b == Builtin::Min) { a } else { x }
            }
            // Rounded once, as the compiled code's `fmla`.
            Builtin::Fma if m == M::F32 => f32::from_bits(a as u32).mul_add(f32::from_bits(x as u32), f32::from_bits(y as u32)).to_bits() as u64,
            Builtin::Fma if m == M::F64 => f64::from_bits(a).mul_add(f64::from_bits(x), f64::from_bits(y)).to_bits(),
            _ => return Err(self.err(&format!("{b:?} as a lane's own"))),
        })
    }

    /// An atomic at `ptr`, one lane's.
    ///
    /// # Safety
    ///
    /// `ptr` is the kernel's address, of `m`'s bytes.
    unsafe fn atomic(b: Builtin, m: M, ptr: u64, x: u64, y: u64) -> u64 {
        // SAFETY: the caller's; the interpreter runs one lane at a time, so an atomic is its read and write.
        let old = unsafe { ops::load(m, ptr) };
        let new = match b {
            Builtin::AtomicAdd => old.wrapping_add(x),
            Builtin::AtomicSub => old.wrapping_sub(x),
            Builtin::AtomicAnd => old & x,
            Builtin::AtomicOr => old | x,
            Builtin::AtomicXor => old ^ x,
            Builtin::AtomicXchg => x,
            Builtin::AtomicInc => old.wrapping_add(1),
            Builtin::AtomicDec => old.wrapping_sub(1),
            Builtin::AtomicMin | Builtin::AtomicMax => {
                let less = ops::bin(Op::Lt, m, x, old).unwrap_or(0) != 0;
                if less == (b == Builtin::AtomicMin) { x } else { old }
            }
            _ => {
                if ops::norm(m, old) == ops::norm(m, x) {
                    y
                } else {
                    old
                }
            }
        };
        // SAFETY: the caller's.
        unsafe { ops::store(m, ptr, new) };
        old
    }

    /// SIMD group `sg` from its instruction to its next barrier or its end.
    fn exec(&self, sg: &mut Sg) -> R<Stop> {
        if let Some(jit) = self.jit {
            return self.exec_jit(jit, sg);
        }
        self.interpret::<false>(sg)
    }

    /// The interpreter's loop from `sg`'s instruction, or (`ONE`) that instruction alone.
    fn interpret<const ONE: bool>(&self, sg: &mut Sg) -> R<Stop> {
        #[cfg(target_arch = "x86_64")]
        if self.avx2 {
            // SAFETY: the CPU has AVX2 (`is_x86_feature_detected`); the code is `compile`'s.
            return unsafe { exec_avx2::<ONE>(self, sg) };
        }
        // SAFETY: the code tape's instructions are `compile`'s, their registers within the files.
        unsafe { exec_in::<ONE>(self, sg) }
    }

    /// The compiled sections from `sg`'s instruction: each runs to its next stop, and what the interpreter does
    /// there is done here (a call's and a return's frames, windows and masks), until a barrier, a poll or the end.
    fn exec_jit(&self, jit: &crate::jit::Jit, sg: &mut Sg) -> R<Stop> {
        loop {
            let (entry, at) = jit.at(sg.pc).ok_or_else(|| self.err(&format!("instruction {} is no section's resume point", sg.pc)))?;
            let win = sg.win;
            // SAFETY: the section is the code's, compiled; the windows are within the files (as `exec_in`'s).
            let r = unsafe {
                entry(
                    std::ptr::from_ref(self).cast(),
                    std::ptr::from_mut(sg).cast(),
                    sg.s.as_mut_ptr().add(win[0]),
                    sg.v32.as_mut_ptr().add(win[1]).cast(),
                    sg.v64.as_mut_ptr().add(win[2]).cast(),
                    sg.m.as_mut_ptr().add(win[3]),
                    sg.fp,
                    at,
                )
            };
            if r != 0 {
                return Err(self.fault.borrow_mut().take().unwrap_or_else(|| self.err("the compiled code entered no resume point")));
            }
            let i = (sg.pc - 1) * WORDS;
            let (a, b) = (self.code[i + 1] as usize, self.code[i + 2] as usize);
            // SAFETY: the compiled code stops past an instruction of the code, `compile`'s.
            match unsafe { I::of(self.code[i]) } {
                I::Halt => return Ok(Stop::Halt),
                I::Bar => return Ok(Stop::Bar(a as u32)),
                I::Yield => return Ok(Stop::Yield),
                I::Call => {
                    sg.calls.push((sg.pc * WORDS, sg.fp, sg.win));
                    let r = &self.table[b * TABLE..][..TABLE];
                    sg.fp += u64::from(r[0]);
                    let caller = sg.win[3];
                    for (w, &k) in sg.win.iter_mut().zip(&r[1..]) {
                        *w += k as usize;
                    }
                    // The five masks go with the lanes into the callee.
                    sg.m.copy_within(caller..caller + 5, sg.win[3]);
                    sg.pc = a;
                }
                I::Ret => {
                    let (back, f, w) = sg.calls.pop().expect("a call to return from");
                    let callee = sg.win[3];
                    (sg.pc, sg.fp, sg.win) = (back / WORDS, f, w);
                    sg.m.copy_within(callee..callee + 5, sg.win[3]);
                }
                op => return Err(self.err(&format!("the compiled code stopped at {op:?}"))),
            }
        }
    }
}

/// The interpreter's step of instruction `pc` for the compiled code (`jit::Step`).
///
/// # Safety
///
/// `cx` and `sg` are the dispatch's and the SIMD group's the compiled code was entered with, `pc` an instruction
/// of its code that neither jumps nor stops.
unsafe extern "C" fn step(cx: *const u8, sg: *mut u8, pc: u64) -> u64 {
    // SAFETY: the caller's.
    let (cx, sg) = unsafe { (&*cx.cast::<Cx>(), &mut *sg.cast::<Sg>()) };
    sg.pc = pc as usize;
    match cx.interpret::<true>(sg) {
        Ok(_) => 0,
        Err(e) => {
            *cx.fault.borrow_mut() = Some(e);
            1
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,popcnt,bmi1,bmi2,lzcnt")]
unsafe fn exec_avx2<const ONE: bool>(cx: &Cx, sg: &mut Sg) -> R<Stop> {
    // SAFETY: the caller's.
    unsafe { exec_in::<ONE>(cx, sg) }
}

/// The interpreter's loop: an instruction at a time, through the table of opcodes.
///
/// # Safety
///
/// The code is `compile`'s: its registers are within `sg`'s files, its jumps within the code, its addresses the
/// kernel's (its buffers', its frame's, threadgroup memory's).
#[inline(always)]
unsafe fn exec_in<const ONE: bool>(cx: &Cx, sg: &mut Sg) -> R<Stop> {
    let code = cx.code.as_ptr();
    let (s0, v320, v640, mk0) = (sg.s.as_mut_ptr(), sg.v32.as_mut_ptr(), sg.v64.as_mut_ptr(), sg.m.as_mut_ptr());
    let (mut fp, mut win) = (sg.fp, sg.win);
    // SAFETY: the windows are within the files (`compile`'s counts, the deepest chain of calls').
    let (mut s, mut v32, mut v64, mut mk) = unsafe { (s0.add(win[0]), v320.add(win[1]), v640.add(win[2]), mk0.add(win[3])) };
    let mut pc = sg.pc * WORDS;
    // SAFETY: the function's (`compile`'s registers, jumps and addresses).
    unsafe {
        macro_rules! s {
            ($i:expr) => {
                *s.add($i)
            };
        }
        macro_rules! m {
            ($i:expr) => {
                *mk.add($i)
            };
        }
        macro_rules! r32 {
            ($i:expr) => {
                &*v32.add($i)
            };
        }
        macro_rules! w32 {
            ($i:expr) => {
                &mut *v32.add($i)
            };
        }
        macro_rules! r64 {
            ($i:expr) => {
                &*v64.add($i)
            };
        }
        macro_rules! w64 {
            ($i:expr) => {
                &mut *v64.add($i)
            };
        }
        loop {
            let w0 = *code.add(pc);
            let a = *code.add(pc + 1) as usize;
            let b = *code.add(pc + 2) as usize;
            let c = *code.add(pc + 3) as usize;
            let d = *code.add(pc + 4) as usize;
            pc += WORDS;
            let (f1, f2, f3) = ((w0 >> 8) & 0xff, (w0 >> 16) & 0xff, w0 >> 24);
            // Lane by lane: `o[l] = f(x[l], y[l])`, the result's file and the operands'. A destination is never an
            // operand (`compile` allocates it while they live), so the loops read and write apart.
            macro_rules! bin {
                ($w:ident, $r:ident, $f:expr) => {{
                    let (x, y, o) = (*$r!(b), *$r!(c), $w!(a));
                    for l in 0..W {
                        o[l] = $f(x[l], y[l]);
                    }
                }};
            }
            macro_rules! un {
                ($w:ident, $r:ident, $f:expr) => {{
                    let (x, o) = (*$r!(b), $w!(a));
                    for l in 0..W {
                        o[l] = $f(x[l]);
                    }
                }};
            }
            let cur = m!(CUR as usize);
            match I::of(w0) {
                I::Halt => {
                    (sg.pc, sg.fp, sg.win) = (pc / WORDS, fp, win);
                    return Ok(Stop::Halt);
                }
                I::Call => {
                    sg.calls.push((pc, fp, win));
                    let r = &cx.table[b * TABLE..][..TABLE];
                    fp += u64::from(r[0]);
                    for (w, &k) in win.iter_mut().zip(&r[1..]) {
                        *w += k as usize;
                    }
                    let caller = mk;
                    (s, v32, v64, mk) = (s0.add(win[0]), v320.add(win[1]), v640.add(win[2]), mk0.add(win[3]));
                    // The five masks go with the lanes into the callee.
                    std::ptr::copy_nonoverlapping(caller, mk, 5);
                    pc = a * WORDS;
                }
                I::Ret => {
                    let (back, f, w) = sg.calls.pop().expect("a call to return from");
                    let callee = mk;
                    (pc, fp, win) = (back, f, w);
                    (s, v32, v64, mk) = (s0.add(win[0]), v320.add(win[1]), v640.add(win[2]), mk0.add(win[3]));
                    std::ptr::copy_nonoverlapping(callee, mk, 5);
                }
                I::Jmp => pc = a * WORDS,
                I::Switch => {
                    // The labels follow (`pc` is past the instruction), sorted by value: a binary search.
                    let x = s!(a);
                    let label = |e: usize| u64::from(*code.add(pc + e * WORDS + 1)) | u64::from(*code.add(pc + e * WORDS + 2)) << 32;
                    let (mut lo, mut hi) = (0, b);
                    while lo < hi {
                        let mid = (lo + hi) / 2;
                        if label(mid) < x {
                            lo = mid + 1;
                        } else {
                            hi = mid;
                        }
                    }
                    pc = if lo < b && label(lo) == x { *code.add(pc + lo * WORDS + 3) as usize } else { c } * WORDS;
                }
                I::Label => unreachable!("a switch's label run"),
                I::Jzm => {
                    if m!(a) == 0 {
                        pc = b * WORDS;
                    }
                }
                I::Jzs => {
                    if s!(a) == 0 {
                        pc = b * WORDS;
                    }
                }
                I::Jnzs => {
                    if s!(a) != 0 {
                        pc = b * WORDS;
                    }
                }
                I::MAnd => m!(a) = m!(b) & tomask(r32!(c)),
                I::MAndN => m!(a) = m!(b) & !tomask(r32!(c)),
                I::MSet => m!(a) = m!(b),
                I::MOr => m!(a) |= m!(b),
                I::MClr => m!(a) = 0,
                I::MLive => m!(0) &= !(m!(1) | m!(2) | m!(3) | m!(4)),
                I::MRejoin => m!(0) = m!(a) & !(m!(1) | m!(3) | m!(4)),
                I::MExit => m!(0) = m!(a) & !(m!(3) | m!(4)),
                I::MResume => m!(0) = m!(a) & !m!(4),
                I::MAndNM => m!(a) = m!(b) & !m!(c),
                I::MSwEnd => m!(0) = m!(a) & !(m!(2) | m!(3) | m!(4)),
                I::Agree => cx.agree(a as u32, sg.index, s!(c), b as u32)?,
                I::AgreeM => cx.agree(a as u32, sg.index, u64::from(m!(c) != 0), b as u32)?,
                I::Bar => {
                    (sg.pc, sg.fp, sg.win) = (pc / WORDS, fp, win);
                    return Ok(Stop::Bar(a as u32));
                }
                I::Yield => {
                    (sg.pc, sg.fp, sg.win) = (pc / WORDS, fp, win);
                    return Ok(Stop::Yield);
                }

                I::SConst => s!(a) = b as u64 | (c as u64) << 32,
                I::SShared => s!(a) = cx.shared + b as u64,
                I::SGlobal => s!(a) = cx.data + b as u64,
                I::SMov => s!(a) = s!(b),
                I::SBin => s!(a) = ops::bin(Op::of(f1), M(f2), s!(b), s!(c)).map_err(|e| cx.err(e))?,
                I::SUn => s!(a) = ops::un(Op::of(f1), M(f2), s!(b)),
                I::SCast => s!(a) = ops::cast(M(f1), M(f2), s!(b)),
                I::SBuiltin => s!(a) = cx.lane_builtin(Builtin::of(f1), M(f2), M(f3), s!(b), s!(c), s!(d))?,
                I::SLoad => s!(a) = ops::load(M(f1), s!(b)),
                I::SStore => ops::store(M(f1), s!(a), s!(b)),
                I::SAtomic => s!(a) = Cx::atomic(Builtin::of(f1), M(f2), s!(b), s!(c), s!(d)),
                I::LdLS => s!(a) = ops::load(M(f1), fp + b as u64 + (first(cur) * c) as u64),
                I::Extract32 => s!(a) = wide(M(f1), r32!(b)[first(cur)]),
                I::Extract64 => s!(a) = r64!(b)[first(cur)],

                I::Bcast32 => *w32!(a) = [s!(b) as u32; W],
                I::Bcast64 => *w64!(a) = [s!(b); W],
                I::VMovM32 => blend32(w32!(a), r32!(b), cur),
                I::VMovM64 => blend64(w64!(a), r64!(b), cur),

                I::LdL => {
                    let (m, at) = (M(f1), fp + b as u64);
                    if m.bytes() == 8 {
                        let o = w64!(a);
                        if c == 8 {
                            std::ptr::copy_nonoverlapping(at as usize as *const u64, o.as_mut_ptr(), W);
                        } else {
                            for (l, o) in o.iter_mut().enumerate() {
                                *o = ops::load(m, at + (l * c) as u64);
                            }
                        }
                    } else {
                        let o = w32!(a);
                        if c == 4 && m.bytes() == 4 {
                            std::ptr::copy_nonoverlapping(at as usize as *const u32, o.as_mut_ptr(), W);
                        } else {
                            for (l, o) in o.iter_mut().enumerate() {
                                *o = ops::load(m, at + (l * c) as u64) as u32;
                            }
                        }
                    }
                }
                I::StL => {
                    let (m, at) = (M(f1), fp + a as u64);
                    if m.bytes() == 8 && b == 8 {
                        blend64(&mut *(at as usize as *mut V64), r64!(c), cur);
                    } else if m.bytes() == 4 && b == 4 {
                        blend32(&mut *(at as usize as *mut V32), r32!(c), cur);
                    } else if m.bytes() == 8 {
                        let x = r64!(c);
                        for l in lanes(cur) {
                            ops::store(m, at + (l * b) as u64, x[l]);
                        }
                    } else {
                        let x = r32!(c);
                        for l in lanes(cur) {
                            ops::store(m, at + (l * b) as u64, u64::from(x[l]));
                        }
                    }
                }
                I::LAddr => {
                    let (o, at) = (w64!(a), fp + b as u64);
                    for (l, o) in o.iter_mut().enumerate() {
                        *o = at + (l * c) as u64;
                    }
                }
                I::VLoad => {
                    let (m, ad) = (M(f1), r64!(b));
                    let n = u64::from(m.bytes());
                    let dense = cur == u32::MAX && {
                        let mut ok = true;
                        for (l, &p) in ad.iter().enumerate() {
                            ok &= p == ad[0].wrapping_add(l as u64 * n);
                        }
                        ok
                    };
                    if m.bytes() == 8 {
                        let o = w64!(a);
                        if dense {
                            std::ptr::copy_nonoverlapping(ad[0] as usize as *const u8, o.as_mut_ptr().cast::<u8>(), W * 8);
                        } else {
                            for l in lanes(cur) {
                                o[l] = ops::load(m, ad[l]);
                            }
                        }
                    } else {
                        let o = w32!(a);
                        if dense && n == 4 {
                            std::ptr::copy_nonoverlapping(ad[0] as usize as *const u8, o.as_mut_ptr().cast::<u8>(), W * 4);
                        } else if dense && m == M::U8 {
                            let src = (ad[0] as usize as *const [u8; W]).read_unaligned();
                            for l in 0..W {
                                o[l] = u32::from(src[l]);
                            }
                        } else {
                            for l in lanes(cur) {
                                o[l] = ops::load(m, ad[l]) as u32;
                            }
                        }
                    }
                }
                I::VStore => {
                    let (m, ad) = (M(f1), r64!(a));
                    if m.bytes() == 8 {
                        let x = r64!(b);
                        for l in lanes(cur) {
                            ops::store(m, ad[l], x[l]);
                        }
                    } else {
                        let x = r32!(b);
                        let dense = cur == u32::MAX && m.bytes() == 4 && {
                            let mut ok = true;
                            for (l, &p) in ad.iter().enumerate() {
                                ok &= p == ad[0].wrapping_add(l as u64 * 4);
                            }
                            ok
                        };
                        if dense {
                            std::ptr::copy_nonoverlapping(x.as_ptr().cast::<u8>(), ad[0] as usize as *mut u8, W * 4);
                        } else {
                            for l in lanes(cur) {
                                ops::store(m, ad[l], u64::from(x[l]));
                            }
                        }
                    }
                }
                I::VCopy => {
                    let (to, from) = (r64!(a), r64!(b));
                    for l in lanes(cur) {
                        std::ptr::copy(from[l] as usize as *const u8, to[l] as usize as *mut u8, c);
                    }
                }
                I::VZero => {
                    let at = r64!(a);
                    for l in lanes(cur) {
                        std::ptr::write_bytes(at[l] as usize as *mut u8, 0, b);
                    }
                }

                I::VLid | I::VGid => {
                    let dim = s!(b) as usize;
                    let gid = I::of(w0) == I::VGid;
                    let (base, src) = if dim < 3 {
                        (if gid { u64::from(cx.group[dim]) * u64::from(cx.block[dim]) } else { 0 }, sg.lid[dim])
                    } else {
                        (0, [0; W])
                    };
                    if f1 == 1 {
                        let o = w64!(a);
                        for l in 0..W {
                            o[l] = base + u64::from(src[l]);
                        }
                    } else {
                        let o = w32!(a);
                        for l in 0..W {
                            o[l] = (base as u32).wrapping_add(src[l]);
                        }
                    }
                }
                I::VLane => {
                    for (l, o) in w32!(a).iter_mut().enumerate() {
                        *o = l as u32;
                    }
                }
                I::Ballot => s!(a) = u64::from(tomask(r32!(b)) & cur),
                I::Shuf32 => shuffle(Builtin::of(f1), cur, r32!(b), r32!(c), w32!(a)),
                I::Shuf64 => shuffle(Builtin::of(f1), cur, r64!(b), r32!(c), w64!(a)),
                I::Reduce32 => {
                    let m = M(f2);
                    s!(a) = reduce(Builtin::of(f1), m, lanes(cur).map(|l| wide(m, r32!(b)[l])));
                }
                I::Reduce64 => s!(a) = reduce(Builtin::of(f1), M(f2), lanes(cur).map(|l| r64!(b)[l])),
                I::VAtomic => {
                    let (op, m) = (Builtin::of(f1), M(f2));
                    let ad = *r64!(b);
                    if m.bytes() == 8 {
                        let (x, y) = (*r64!(c), *r64!(d));
                        let o = w64!(a);
                        for l in lanes(cur) {
                            o[l] = Cx::atomic(op, m, ad[l], x[l], y[l]);
                        }
                    } else {
                        let (x, y) = (*r32!(c), *r32!(d));
                        let o = w32!(a);
                        for l in lanes(cur) {
                            o[l] = Cx::atomic(op, m, ad[l], wide(m, x[l]), wide(m, y[l])) as u32;
                        }
                    }
                }

                I::VBinGen => {
                    let (op, m) = (Op::of(f1), M(f2));
                    let out64 = !op.compares() && m.bytes() == 8;
                    for l in lanes(cur) {
                        let (x, y) = if m.bytes() == 8 { (r64!(b)[l], r64!(c)[l]) } else { (wide(m, r32!(b)[l]), wide(m, r32!(c)[l])) };
                        let r = ops::bin(op, m, x, y).map_err(|e| cx.err(e))?;
                        if out64 {
                            w64!(a)[l] = r;
                        } else {
                            w32!(a)[l] = r as u32;
                        }
                    }
                }
                I::VUnGen => {
                    let (op, m) = (Op::of(f1), M(f2));
                    for l in lanes(cur) {
                        if m.bytes() == 8 {
                            w64!(a)[l] = ops::un(op, m, r64!(b)[l]);
                        } else {
                            w32!(a)[l] = ops::un(op, m, wide(m, r32!(b)[l])) as u32;
                        }
                    }
                }
                I::VCastGen => {
                    let (from, to) = (M(f1), M(f2));
                    for l in lanes(cur) {
                        let x = if from.bytes() == 8 { r64!(b)[l] } else { wide(from, r32!(b)[l]) };
                        let r = ops::cast(from, to, x);
                        if to.bytes() == 8 {
                            w64!(a)[l] = r;
                        } else {
                            w32!(a)[l] = r as u32;
                        }
                    }
                }
                I::VBuiltinGen => {
                    let (op, m, from) = (Builtin::of(f1), M(f2), M(f3));
                    for l in lanes(cur) {
                        let (x, y, z) = if from.bytes() == 8 { (r64!(b)[l], r64!(c)[l], r64!(d)[l]) } else { (wide(from, r32!(b)[l]), wide(from, r32!(c)[l]), wide(from, r32!(d)[l])) };
                        let r = cx.lane_builtin(op, m, from, x, y, z)?;
                        if m.bytes() == 8 {
                            w64!(a)[l] = r;
                        } else {
                            w32!(a)[l] = r as u32;
                        }
                    }
                }

                I::Zx8 => un!(w32, r32, |x: u32| x & 0xff),
                I::Sx8 => un!(w32, r32, |x: u32| x as u8 as i8 as i32 as u32),
                I::Zx16 => un!(w32, r32, |x: u32| x & 0xffff),
                I::Sx16 => un!(w32, r32, |x: u32| x as u16 as i16 as i32 as u32),
                I::Truth32 => un!(w32, r32, |x: u32| u32::from(x != 0)),
                I::Truth64 => un!(w32, r64, |x: u64| u32::from(x != 0)),
                I::Trunc => un!(w32, r64, |x: u64| x as u32),
                I::Zext => un!(w64, r32, |x: u32| u64::from(x)),
                I::Sext => un!(w64, r32, |x: u32| x as i32 as i64 as u64),

                I::Add32 => bin!(w32, r32, |x: u32, y: u32| x.wrapping_add(y)),
                I::Sub32 => bin!(w32, r32, |x: u32, y: u32| x.wrapping_sub(y)),
                I::Mul32 => bin!(w32, r32, |x: u32, y: u32| x.wrapping_mul(y)),
                I::And32 => bin!(w32, r32, |x: u32, y: u32| x & y),
                I::Or32 => bin!(w32, r32, |x: u32, y: u32| x | y),
                I::Xor32 => bin!(w32, r32, |x: u32, y: u32| x ^ y),
                I::Shl32 => bin!(w32, r32, |x: u32, y: u32| x << (y & 31)),
                I::ShrU32 => bin!(w32, r32, |x: u32, y: u32| x >> (y & 31)),
                I::ShrI32 => bin!(w32, r32, |x: u32, y: u32| ((x as i32) >> (y & 31)) as u32),
                I::Eq32 => bin!(w32, r32, |x: u32, y: u32| u32::from(x == y)),
                I::Ne32 => bin!(w32, r32, |x: u32, y: u32| u32::from(x != y)),
                I::LtU32 => bin!(w32, r32, |x: u32, y: u32| u32::from(x < y)),
                I::LtI32 => bin!(w32, r32, |x: u32, y: u32| u32::from((x as i32) < (y as i32))),
                I::LeU32 => bin!(w32, r32, |x: u32, y: u32| u32::from(x <= y)),
                I::LeI32 => bin!(w32, r32, |x: u32, y: u32| u32::from((x as i32) <= (y as i32))),
                I::MinU32 => bin!(w32, r32, |x: u32, y: u32| x.min(y)),
                I::MinI32 => bin!(w32, r32, |x: u32, y: u32| (x as i32).min(y as i32) as u32),
                I::MaxU32 => bin!(w32, r32, |x: u32, y: u32| x.max(y)),
                I::MaxI32 => bin!(w32, r32, |x: u32, y: u32| (x as i32).max(y as i32) as u32),

                I::Add64 => bin!(w64, r64, |x: u64, y: u64| x.wrapping_add(y)),
                I::Sub64 => bin!(w64, r64, |x: u64, y: u64| x.wrapping_sub(y)),
                I::Mul64 => bin!(w64, r64, |x: u64, y: u64| x.wrapping_mul(y)),
                I::And64 => bin!(w64, r64, |x: u64, y: u64| x & y),
                I::Or64 => bin!(w64, r64, |x: u64, y: u64| x | y),
                I::Xor64 => bin!(w64, r64, |x: u64, y: u64| x ^ y),
                I::Shl64 => bin!(w64, r64, |x: u64, y: u64| x << (y & 63)),
                I::ShrU64 => bin!(w64, r64, |x: u64, y: u64| x >> (y & 63)),
                I::ShrI64 => bin!(w64, r64, |x: u64, y: u64| ((x as i64) >> (y & 63)) as u64),
                I::Eq64 => bin!(w32, r64, |x: u64, y: u64| u32::from(x == y)),
                I::Ne64 => bin!(w32, r64, |x: u64, y: u64| u32::from(x != y)),
                I::LtU64 => bin!(w32, r64, |x: u64, y: u64| u32::from(x < y)),
                I::LtI64 => bin!(w32, r64, |x: u64, y: u64| u32::from((x as i64) < (y as i64))),
                I::LeU64 => bin!(w32, r64, |x: u64, y: u64| u32::from(x <= y)),
                I::LeI64 => bin!(w32, r64, |x: u64, y: u64| u32::from((x as i64) <= (y as i64))),
                I::MinU64 => bin!(w64, r64, |x: u64, y: u64| x.min(y)),
                I::MinI64 => bin!(w64, r64, |x: u64, y: u64| (x as i64).min(y as i64) as u64),
                I::MaxU64 => bin!(w64, r64, |x: u64, y: u64| x.max(y)),
                I::MaxI64 => bin!(w64, r64, |x: u64, y: u64| (x as i64).max(y as i64) as u64),

                I::AddF32 => bin!(w32, r32, |x: u32, y: u32| (f32::from_bits(x) + f32::from_bits(y)).to_bits()),
                I::SubF32 => bin!(w32, r32, |x: u32, y: u32| (f32::from_bits(x) - f32::from_bits(y)).to_bits()),
                I::MulF32 => bin!(w32, r32, |x: u32, y: u32| (f32::from_bits(x) * f32::from_bits(y)).to_bits()),
                I::DivF32 => bin!(w32, r32, |x: u32, y: u32| (f32::from_bits(x) / f32::from_bits(y)).to_bits()),
                I::EqF32 => bin!(w32, r32, |x: u32, y: u32| u32::from(f32::from_bits(x) == f32::from_bits(y))),
                I::NeF32 => bin!(w32, r32, |x: u32, y: u32| u32::from(f32::from_bits(x) != f32::from_bits(y))),
                I::LtF32 => bin!(w32, r32, |x: u32, y: u32| u32::from(f32::from_bits(x) < f32::from_bits(y))),
                I::LeF32 => bin!(w32, r32, |x: u32, y: u32| u32::from(f32::from_bits(x) <= f32::from_bits(y))),
                I::AddF64 => bin!(w64, r64, |x: u64, y: u64| (f64::from_bits(x) + f64::from_bits(y)).to_bits()),
                I::SubF64 => bin!(w64, r64, |x: u64, y: u64| (f64::from_bits(x) - f64::from_bits(y)).to_bits()),
                I::MulF64 => bin!(w64, r64, |x: u64, y: u64| (f64::from_bits(x) * f64::from_bits(y)).to_bits()),
                I::DivF64 => bin!(w64, r64, |x: u64, y: u64| (f64::from_bits(x) / f64::from_bits(y)).to_bits()),
                I::EqF64 => bin!(w32, r64, |x: u64, y: u64| u32::from(f64::from_bits(x) == f64::from_bits(y))),
                I::NeF64 => bin!(w32, r64, |x: u64, y: u64| u32::from(f64::from_bits(x) != f64::from_bits(y))),
                I::LtF64 => bin!(w32, r64, |x: u64, y: u64| u32::from(f64::from_bits(x) < f64::from_bits(y))),
                I::LeF64 => bin!(w32, r64, |x: u64, y: u64| u32::from(f64::from_bits(x) <= f64::from_bits(y))),

                I::Neg32 => un!(w32, r32, |x: u32| x.wrapping_neg()),
                I::Neg64 => un!(w64, r64, |x: u64| x.wrapping_neg()),
                I::NegF32 => un!(w32, r32, |x: u32| x ^ 0x8000_0000),
                I::NegF64 => un!(w64, r64, |x: u64| x ^ (1 << 63)),
                I::Not32 => un!(w32, r32, |x: u32| !x),
                I::Not64 => un!(w64, r64, |x: u64| !x),
                I::LNot32 => un!(w32, r32, |x: u32| u32::from(x == 0)),
                I::LNot64 => un!(w64, r64, |x: u64| u64::from(x == 0)),
                I::Popcount32 => un!(w32, r32, |x: u32| x.count_ones()),
            }
            if ONE {
                return Ok(Stop::Halt);
            }
        }
    }
}

/// A SIMD-group reduction (`b`) of type `m` over the running lanes' values `xs`.
fn reduce(b: Builtin, m: M, xs: impl Iterator<Item = u64>) -> u64 {
    let mut out: Option<u64> = None;
    for x in xs {
        out = Some(match out {
            None => x,
            Some(a) if b == Builtin::SubGroupReduceAdd => ops::bin(Op::Add, m, a, x).unwrap_or(0),
            Some(a) => {
                let less = ops::bin(Op::Lt, m, x, a).unwrap_or(0) != 0;
                if less == (b == Builtin::SubGroupReduceMin) { x } else { a }
            }
        });
    }
    out.unwrap_or(0)
}

/// A SIMD-group shuffle (`b`) of `x` by `idx` over the running lanes `cur`: a lane from a lane that is not running
/// takes 0 (a shuffle's, a xor's), or its own value (up, down).
#[inline(always)]
fn shuffle<T: Copy + Default>(b: Builtin, cur: u32, x: &[T; W], idx: &V32, o: &mut [T; W]) {
    let on = |s: usize| cur >> s & 1 != 0;
    match b {
        Builtin::SubGroupShuffle => {
            for l in 0..W {
                let s = idx[l] as usize % W;
                o[l] = if on(s) { x[s] } else { T::default() };
            }
        }
        Builtin::SubGroupShuffleXor => {
            for l in 0..W {
                let s = (l ^ idx[l] as usize) % W;
                o[l] = if on(s) { x[s] } else { T::default() };
            }
        }
        Builtin::SubGroupShuffleUp => {
            for l in 0..W {
                o[l] = match l.checked_sub(idx[l] as usize) {
                    Some(s) if on(s) => x[s],
                    _ => x[l],
                };
            }
        }
        _ => {
            for l in 0..W {
                let s = l + idx[l] as usize;
                o[l] = if s < W && on(s) { x[s] } else { x[l] };
            }
        }
    }
}
