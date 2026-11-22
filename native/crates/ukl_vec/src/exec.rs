//! A kernel's SSA compiled (`cl`) and run (decision 0567, step 3): a launch's groups in resident slots as `run`'s,
//! each SIMD group a fiber (`fiber`) of the compiled kernel, a barrier or a poll a switch back to its group's turns.
//!
//! The compiled code reads what it needs of the launch through `Env` and of its SIMD group through `Sg`, by the
//! offsets `places` gives it; the rest of `Sg` is the runtime's.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::Error;
use crate::cl::{self, Compiled, HELPERS, Places};
use crate::code::HEADER;
use crate::k::{M, View};
use crate::lower::Program;
use crate::ops;
use crate::run::{Arg, Launch, W};
use crate::vector::Kernel;

type R<T> = Result<T, Error>;

/// The launch's, as the compiled code reads it.
#[repr(C)]
struct Env {
    data: u64,
    shared: u64,
    grid: [u32; 3],
    block: [u32; 3],
    group: [u32; 3],
    _pad: u32,
    scratch: u64,
    helpers: [u64; HELPERS],
    /// The runtime's: its launch.
    cx: *const Cx<'static>,
    scratch_mem: [u128; 4],
}

/// What a SIMD group waits for, as `run`'s.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Wait {
    Run,
    Bar(u32),
    Yield,
}

/// A SIMD group: the compiled code's part first (`places`), then the runtime's.
#[repr(C)]
struct Sg {
    env: u64,
    frame: u64,
    live: u32,
    _pad: u32,
    args: u64,
    lid: [[u32; W]; 3],
    index: u32,
    done: bool,
    wait: Wait,
    /// The compiled kernel its fiber runs.
    entry: Option<cl::Entry>,
    own: crate::fiber::Ctx,
    back: crate::fiber::Ctx,
    stack: crate::fiber::Stack,
}

/// A resident group's slot.
struct Slot {
    _frames: Vec<u128>,
    shared: Vec<u128>,
    g: Vec<Box<Sg>>,
    agreed: HashMap<u32, (Vec<u64>, Vec<usize>)>,
    group: [u32; 3],
    busy: bool,
}

/// One launch's state, for the helpers.
struct Cx<'p> {
    p: &'p Program,
    kernel: &'p str,
    agreed: RefCell<HashMap<u32, (Vec<u64>, Vec<usize>)>>,
    fault: RefCell<Option<Error>>,
}

thread_local! {
    /// The thread's last launches' slots, by their shape, the latest first.
    static SLOTS: RefCell<Vec<([usize; 7], Vec<Slot>)>> = const { RefCell::new(Vec::new()) };
}

const KEPT_SHAPES: usize = 4;

pub(crate) fn places() -> Places {
    Places {
        env: std::mem::offset_of!(Sg, env) as i32,
        frame: std::mem::offset_of!(Sg, frame) as i32,
        live: std::mem::offset_of!(Sg, live) as i32,
        args: std::mem::offset_of!(Sg, args) as i32,
        lid: std::mem::offset_of!(Sg, lid) as i32,
        data: std::mem::offset_of!(Env, data) as i32,
        shared: std::mem::offset_of!(Env, shared) as i32,
        grid: std::mem::offset_of!(Env, grid) as i32,
        block: std::mem::offset_of!(Env, block) as i32,
        group: std::mem::offset_of!(Env, group) as i32,
        scratch: std::mem::offset_of!(Env, scratch) as i32,
        helpers: std::mem::offset_of!(Env, helpers) as i32,
    }
}

/// The width a kernel's code runs at when nothing else chooses it (`RERAIL_UKL_WIDTH`, else 8).
fn width() -> u32 {
    std::env::var("RERAIL_UKL_WIDTH").ok().and_then(|s| s.parse().ok()).unwrap_or(8)
}

/// Kernel `k`'s SSA compiled: on its first launch, then kept with it.
pub fn compiled<'k>(p: &Program, k: &'k Kernel) -> R<&'k Compiled> {
    compiled_kept(p, k, || None, |_| {})
}

/// The same, from its kept form (`cl::load`), which `kept` reads back, else compiled, its kept form given to `keep`.
pub fn compiled_kept<'k>(p: &Program, k: &'k Kernel, kept: impl FnOnce() -> Option<Vec<u8>>, keep: impl FnOnce(&[u8])) -> R<&'k Compiled> {
    k.cl.get_or_init(|| {
        if let Some(c) = kept().and_then(|b| cl::load(&b)) {
            return Ok(c);
        }
        let t = std::time::Instant::now();
        let ssa = crate::ssa::build(&k.tape, &|id| if id == u32::MAX { k.name.clone() } else { p.funcs[id as usize].name.clone() }).map_err(|e| e.0)?;
        let st = t.elapsed().as_secs_f64();
        let (c, bytes) = cl::compile(&ssa, &k.name, width(), &places()).map_err(|e| e.0)?;
        if std::env::var("RERAIL_UKL_VEC_TIMES").is_ok() {
            let values = ssa.funcs.iter().map(|f| f.insts.len()).sum::<usize>();
            eprintln!("COMPILE {}: SSA {st:.3} s ({} functions, {values} values), Cranelift {:.3} s", k.name, ssa.funcs.len(), t.elapsed().as_secs_f64() - st);
        }
        if let Some(b) = bytes {
            keep(&b);
        }
        Ok(c)
    })
    .as_ref()
    .map_err(|e| Error(format!("{}: {e}", k.name)))
}

/// The width a kernel compiled for this process runs at, for its kept form's key.
pub fn kept_width() -> u32 {
    width()
}

/// Runs `k` over `launch`'s grid as `run::run` does, its compiled SSA.
pub fn run(p: &Program, k: &Kernel, args: &[Arg], launch: Launch) -> R<()> {
    let Launch { grid, block, local, resident, .. } = launch;
    let cv = View::new(&k.code);
    let words = cv.p(0);
    let (frame_bytes, own, n_params) = (words[0] as usize, u64::from(words[1]).next_multiple_of(16), words[2] as usize);
    let params = &words[HEADER..HEADER + 2 * n_params];
    if args.len() != n_params {
        return Err(Error(format!("{} takes {n_params} arguments, given {}", k.name, args.len())));
    }
    if k.lid && !(block[1] == 1 && block[2] == 1) && !(block[0] as usize).is_multiple_of(W) {
        return Err(Error(format!("{} takes a thread's id in its group as its SIMD group's and lane's: a group of {block:?} is not one", k.name)));
    }
    let c = compiled(p, k)?;
    let threads = block.iter().map(|&b| b as usize).product::<usize>();
    let nsg = threads.div_ceil(W);
    let frame = frame_bytes.next_multiple_of(16) * W;
    let groups = grid.iter().map(|&g| g as usize).product::<usize>();
    if groups == 0 {
        return Ok(());
    }
    // The arguments as the compiled code reads a kernel's (a `u64` each, as its type holds it).
    let argv: Vec<u64> = args
        .iter()
        .enumerate()
        .map(|(j, a)| {
            let m = M(params[2 * j + 1]);
            match a {
                Arg::Bytes(b) if !m.is_agg() => {
                    let mut x = [0u8; 8];
                    let n = b.len().min(8);
                    x[..n].copy_from_slice(&b[..n]);
                    ops::norm(m, u64::from_le_bytes(x))
                }
                _ => 0,
            }
        })
        .collect();
    let cx = Cx { p, kernel: &k.name, agreed: RefCell::new(HashMap::new()), fault: RefCell::new(None) };
    let mut env = Box::new(Env {
        data: p.data().as_ptr() as u64,
        shared: 0,
        grid,
        block,
        group: [0; 3],
        _pad: 0,
        scratch: 0,
        helpers: [h_bar as *const () as u64, h_yield as *const () as u64, h_agree as *const () as u64, h_fail as *const () as u64, h_copy as *const () as u64, h_zero as *const () as u64],
        // SAFETY: the launch's state outlives every call of the compiled code, all within this function.
        cx: unsafe { std::mem::transmute::<*const Cx<'_>, *const Cx<'static>>(&cx) },
        scratch_mem: [0; 4],
    });
    env.scratch = env.scratch_mem.as_mut_ptr() as u64;
    let envp = &mut *env as *mut Env as u64;
    let (bx, by) = (block[0].max(1), block[1].max(1));
    let shape = [nsg, frame, (own + local) as usize, threads, bx as usize, by as usize, resident.clamp(1, groups)];
    let kept = SLOTS.with(|c| {
        let mut c = c.borrow_mut();
        let at = c.iter().position(|e| e.0 == shape)?;
        Some(c.remove(at).1)
    });
    let mut slots: Vec<Slot> = kept.filter(|v| !v.is_empty()).unwrap_or_else(|| {
        (0..resident.clamp(1, groups))
            .map(|_| {
                let mut frames = vec![0u128; (nsg * frame).div_ceil(16).max(1)];
                let base = frames.as_mut_ptr() as u64;
                let g = (0..nsg as u32)
                    .map(|s| {
                        let lin = |l: usize| s * W as u32 + l as u32;
                        Box::new(Sg {
                            env: 0,
                            frame: base + u64::from(s) * frame as u64,
                            live: 0,
                            _pad: 0,
                            args: 0,
                            lid: [std::array::from_fn(|l| lin(l) % bx), std::array::from_fn(|l| lin(l) / bx % by), std::array::from_fn(|l| lin(l) / (bx * by))],
                            index: s,
                            done: false,
                            wait: Wait::Run,
                            entry: None,
                            own: Default::default(),
                            back: Default::default(),
                            stack: crate::fiber::Stack::new(),
                        })
                    })
                    .collect();
                Slot { _frames: frames, shared: vec![0u128; (own + local).div_ceil(16) as usize + 1], g, agreed: HashMap::new(), group: [0; 3], busy: false }
            })
            .collect()
    });
    let live = |s: usize| {
        let n = (threads - s * W).min(W);
        if n == W { u32::MAX } else { (1u32 << n) - 1 }
    };
    let entry = c.entry;
    let start = |sl: &mut Slot, at: usize| {
        let (gx, gy) = (grid[0] as usize, grid[1] as usize);
        sl.group = [(at % gx) as u32, (at / gx % gy) as u32, (at / (gx * gy)) as u32];
        sl.agreed.clear();
        sl.shared.fill(0);
        let sh = sl.shared.as_mut_ptr() as u64;
        for (s, sg) in sl.g.iter_mut().enumerate() {
            let mask = live(s);
            sg.env = envp;
            sg.live = mask;
            sg.args = argv.as_ptr() as u64;
            (sg.done, sg.wait, sg.entry) = (false, Wait::Run, Some(entry));
            for (j, a) in args.iter().enumerate() {
                let (off, m) = (u64::from(params[2 * j]), M(params[2 * j + 1]));
                for l in 0..W {
                    if mask >> l & 1 == 0 {
                        continue;
                    }
                    let at = sg.frame + off * W as u64 + l as u64 * u64::from(m.bytes());
                    match a {
                        // SAFETY: the parameter's place in the lane's frame, its type's bytes.
                        Arg::Bytes(b) => unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), at as usize as *mut u8, b.len().min(m.bytes() as usize)) },
                        // SAFETY: the same, a pointer's.
                        Arg::Local(o) => unsafe { ops::store(M::PTR, at, sh + own + o) },
                    }
                }
            }
            let sgp = &mut **sg as *mut Sg;
            // SAFETY: the stack is the SIMD group's, kept while its fiber runs; `fiber_start` never returns.
            unsafe { crate::fiber::prepare(&mut (*sgp).own, &mut (*sgp).stack, fiber_start, sgp as u64) };
        }
        sl.busy = true;
    };
    let mut next = 0usize;
    let result = loop {
        let mut any = false;
        let mut failed = None;
        'slots: for sl in &mut slots {
            loop {
                if !sl.busy {
                    if next == groups {
                        break;
                    }
                    start(sl, next);
                    next += 1;
                }
                any = true;
                (env.shared, env.group) = (sl.shared.as_mut_ptr() as u64, sl.group);
                std::mem::swap(&mut *cx.agreed.borrow_mut(), &mut sl.agreed);
                let ended = group_run(&cx, &mut sl.g);
                std::mem::swap(&mut *cx.agreed.borrow_mut(), &mut sl.agreed);
                match ended {
                    Err(e) => {
                        // The slot's fibers are left where they stopped: the next launch starts them afresh.
                        sl.busy = false;
                        failed = Some(e);
                        break 'slots;
                    }
                    Ok(false) => break,
                    Ok(true) => sl.busy = false,
                }
            }
        }
        if let Some(e) = failed {
            break Err(e);
        }
        if !any {
            break Ok(());
        }
    };
    for sl in &mut slots {
        sl.busy = false;
    }
    SLOTS.with(|c| {
        let mut c = c.borrow_mut();
        c.insert(0, (shape, slots));
        c.truncate(KEPT_SHAPES);
    });
    result
}

/// A SIMD group's fiber: the kernel, then its end, a switch back that is never resumed.
unsafe extern "C" fn fiber_start(arg: u64) -> ! {
    let sg = arg as *mut Sg;
    // SAFETY: the SIMD group's state, live while its fiber runs.
    unsafe {
        let entry = (*sg).entry.expect("a prepared SIMD group's kernel");
        entry(sg as *mut u8);
        (*sg).done = true;
        crate::fiber::switch(&mut (*sg).own, &(*sg).back);
    }
    unreachable!("an ended fiber resumed")
}

/// The fiber running `sg` back to its group's turns.
///
/// # Safety
///
/// Called on `sg`'s fiber.
unsafe fn back(sg: *mut Sg) {
    // SAFETY: the caller's.
    unsafe { crate::fiber::switch(&mut (*sg).own, &(*sg).back) }
}

fn cx_of(sg: *mut Sg) -> &'static Cx<'static> {
    // SAFETY: a live SIMD group's environment, and its launch's state.
    unsafe { &*(*((*sg).env as *const Env)).cx }
}

/// The group's SIMD groups in turns, as `run`'s: whether it has ended.
fn group_run(cx: &Cx, g: &mut [Box<Sg>]) -> R<bool> {
    loop {
        for sg in g.iter_mut().filter(|sg| !sg.done && sg.wait == Wait::Run) {
            let p = &mut **sg as *mut Sg;
            // SAFETY: the fiber is prepared or stopped at a helper; it switches back here.
            unsafe { crate::fiber::switch(&mut (*p).back, &(*p).own) };
            if let Some(e) = cx.fault.borrow_mut().take() {
                return Err(e);
            }
        }
        let mut at: Option<u32> = None;
        let (mut polls, mut waits) = (false, false);
        for sg in g.iter().filter(|sg| !sg.done) {
            match sg.wait {
                Wait::Yield => polls = true,
                Wait::Bar(site) => {
                    waits = true;
                    match at {
                        Some(other) if other != site && !polls => {
                            return Err(Error(format!(
                                "{}: SIMD group {} waits at {} while another waits at {}: the group's threads meet different barriers (decision 0567 §3)",
                                cx.kernel,
                                sg.index,
                                site_name(cx, site),
                                site_name(cx, other)
                            )));
                        }
                        _ => at = Some(site),
                    }
                }
                Wait::Run => unreachable!("a SIMD group that ran stopped"),
            }
        }
        if polls {
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

fn site_name<'a>(cx: &'a Cx, w: u32) -> &'a str {
    cx.p.sites.get(w as usize).map_or("?", String::as_str)
}

// The helpers the compiled code calls.

unsafe extern "C" fn h_bar(sg: u64, site: u64) {
    let sg = sg as *mut Sg;
    // SAFETY: on the SIMD group's fiber.
    unsafe {
        (*sg).wait = Wait::Bar(site as u32);
        back(sg);
    }
}

unsafe extern "C" fn h_yield(sg: u64) {
    let sg = sg as *mut Sg;
    // SAFETY: on the SIMD group's fiber.
    unsafe {
        (*sg).wait = Wait::Yield;
        back(sg);
    }
}

/// Fails the launch: its SIMD group stops where it is, never resumed.
fn fail(sg: *mut Sg, e: Error) -> ! {
    let cx = cx_of(sg);
    *cx.fault.borrow_mut() = Some(e);
    // SAFETY: on the SIMD group's fiber; it is not switched to again.
    unsafe { back(sg) };
    unreachable!("a failed SIMD group resumed")
}

unsafe extern "C" fn h_agree(sg: u64, node: u64, d: u64, site: u64) {
    let sg = sg as *mut Sg;
    let cx = cx_of(sg);
    // SAFETY: a live SIMD group.
    let s = unsafe { (*sg).index } as usize;
    let bad = {
        let mut a = cx.agreed.borrow_mut();
        let (ds, at) = a.entry(node as u32).or_default();
        if at.len() <= s {
            at.resize(s + 1, 0);
        }
        let k = at[s];
        at[s] += 1;
        if k == ds.len() {
            ds.push(d);
            false
        } else {
            ds[k] != d
        }
    };
    if bad {
        let e = Error(format!(
            "{}: {} decides differently in SIMD group {s} than in the first: an agreed value the barrier rule rests on differs across the group (decision 0567 §3)",
            cx.kernel,
            site_name(cx, site as u32)
        ));
        fail(sg, e);
    }
}

unsafe extern "C" fn h_fail(sg: u64, what: u64) {
    let sg = sg as *mut Sg;
    let cx = cx_of(sg);
    let e = Error(format!(
        "{}: {}",
        cx.kernel,
        match what as i64 {
            cl::FAIL_DIV => "a division by zero",
            cl::FAIL_REM => "a remainder by zero",
            _ => "a failure",
        }
    ));
    fail(sg, e);
}

unsafe extern "C" fn h_copy(to: u64, from: u64, n: u64) {
    // SAFETY: the kernel's addresses, of `n` bytes.
    unsafe { std::ptr::copy(from as usize as *const u8, to as usize as *mut u8, n as usize) }
}

unsafe extern "C" fn h_zero(to: u64, n: u64) {
    // SAFETY: the kernel's address, of `n` bytes.
    unsafe { std::ptr::write_bytes(to as usize as *mut u8, 0, n as usize) }
}
