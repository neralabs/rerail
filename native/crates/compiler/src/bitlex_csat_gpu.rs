//! The prover's circuit solver on the device (decision 0670, step 3): CDCL over a batch of formulas, a SIMD group a
//! formula (`bitlex_csat_gpu.uglykl`), each read where it was built. A formula's gates, fanout and plain clauses are
//! in the compile's formula block (`bitlex_grown`), as the CPU's solver reads them; the kernel binds the block once
//! and finds each formula by its descriptor's offsets. Nothing is uploaded (decision 0353): what the device has of
//! its own is each formula's state (values, trail, watches, learned clauses), its working memory.
//!
//! The answers are the CPU's (`bitlex_csat::Solver`): satisfiable, unsatisfiable, or unknown past the conflicts'
//! budget or the learned clauses' room, sound either way. The search is its own: a round's literals' occurrences
//! over the lanes, a wide gate's long clause decided by the word its counter's atomic add returns (Nadel et al.
//! 2026), the integer activities' heap. `bitlex_csat_lanes` is a search of the same parts on the CPU, not the same
//! steps: their answers agree where both have one, their models, cores and what they carry may differ, each sound.
//!
//! Memory, a batch: a descriptor of `DESC` words a formula, in the block; the slots' states, each its largest
//! formula's `state_words`. A slot is a threadgroup of `LANES` (a SIMD group), which takes the batch's formulas from
//! a queue one after another: as many slots as fit in `STATE_BYTES`, so a batch's memory is bounded however many
//! formulas it holds, and the device has as many searches running at once as it keeps resident, each hiding the
//! others' memory reads. (With a state a formula, a batch of 768 of a 31,841-variable formula, 2.5 GB, took 28
//! times what 384 did.) Each launch runs a slot `STEPS` steps at most, the GPU's time a launch bounded.

pub use crate::bitlex_csat_lanes::{Answer, Formula, Solved};
use crate::bitlex_csat_lanes::{arena, carry_vars, learned, CARRY_ROOM};
use crate::bitlex_grown::FormulaBlock;
use rerail_gpu::device::ComputeQueue;
use rerail_gpu::kernels::{alloc, alloc_working, launch, program, read};

const KERNEL: &str = include_str!("bitlex_csat_gpu.uglykl");

/// Whether the prover's batches solve on the GPU first (`RERAIL_PROVER_GPU=1`).
pub fn enabled() -> bool {
    std::env::var_os("RERAIL_PROVER_GPU").is_some_and(|v| v == "1")
}

/// A descriptor's words (the kernel's `DESC`), and where a batch places a formula's model and what it carries.
const DESC: usize = 40;
const D_MODEL: usize = 12;
const D_CARRY: usize = 30;
/// A formula's lanes: one SIMD group.
const LANES: u64 = 32;
/// The steps a launch runs a slot at most (a round of propagation, a conflict, a decision): a launch's time bounded.
/// A slot ends its launch sooner where formulas wait that no slot takes (`cs_look`).
const STEPS: u32 = 1 << 12;
/// The slots' states at most, in bytes.
pub const STATE_BYTES: usize = 1 << 30;
/// The slots at most.
const SLOTS: usize = 1 << 13;
/// A slot's threadgroup memory at most, in words: 5 KiB, so that it does not bound the groups an SM keeps resident
/// (16, by registers, on GB10's 100 KiB a SM, 1 KiB a group of it the driver's).
const SHARED_WORDS: usize = 1280;
/// A variable's words in a slot's state, each in its own array (the kernel's `VR`).
const VR: usize = 6;
/// Analysis's bump queue in threadgroup memory (the kernel's `BQ`): variables, and their activities.
const BQ: usize = 32;
/// The state's header (the kernel's `HEADER`) and the statuses it reports.
const HEADER: usize = 32;
const SAT: u32 = 1;
const UNSAT: u32 = 2;
const ROOMLESS: u32 = 4;
const BROKEN: u32 = 5;
/// The words a formula's report holds (`out`).
const OUT: usize = 16;

#[repr(C)]
#[derive(Clone, Copy)]
struct Args {
    n: u32,
    steps: u32,
    init: u32,
    slots: u32,
    slot_words: u32,
    room: u32,
    pad1: u32,
    pad2: u32,
}
// SAFETY: eight u32s, no padding.
unsafe impl rerail_gpu::Pod for Args {}

/// A formula's state, in words (the kernel's layout, `cs_setup`), its widest gate `wide - 2` wide, its plain
/// clauses (and carried ones) `clauses`, its wide gates `wides`: a variable's record (`VR` words), its trail entry,
/// level limit, activity and level mark; the tree; a counter a wide gate; the buffers, the learned clauses and
/// their watches. Whole 32-byte sectors.
fn state_words(vars: usize, clauses: usize, wide: usize, wides: usize) -> usize {
    let (n, l, a) = (vars + 1, learned(vars) as usize, arena(vars) as usize);
    (HEADER + (VR + 4) * n + 2 * tree_nodes(n) + wides + 2 * wide + l + (l + 1) + 4 * (clauses + l) + a).next_multiple_of(8)
}

/// The decisions' tree's nodes over `n` activities (the kernel's `cs_tcount` levels): `n` divided by 32 again and
/// again, rounded up, to one.
fn tree_nodes(n: usize) -> usize {
    let (mut total, mut m) = (0, n);
    loop {
        m = m.div_ceil(32);
        total += m;
        if m == 1 {
            return total;
        }
    }
}

/// A slot's threadgroup memory past its header and bump queue, in words, whatever its batch: `SHARED_WORDS` in all
/// (`RERAIL_CSAT_SHARED_WORDS`, a measurement aid). The kernel splits it by the slot's formula alone, its wide
/// gates' counters (`CW_ROOM` at most) and then its values, 16 variables a word; a counter or a value past them is
/// the state's.
fn shared_room() -> usize {
    std::env::var("RERAIL_CSAT_SHARED_WORDS").ok().and_then(|w| w.parse().ok()).unwrap_or(SHARED_WORDS).saturating_sub(HEADER + 2 * BQ)
}

/// A formula's widest gate's width and 2 (at least 4): the room of a conflict's clause or an explanation.
fn wide(gates: &crate::bitlex_csat::Gates) -> usize {
    (0..gates.len()).map(|g| gates.args(g).len()).max().unwrap_or(0).max(2) + 2
}

/// Formulas of fewer gates and clauses than this propagate on one lane, their counters plain words (Nadel et al.
/// 2026, §4.5: below about 2,000 clauses the group's steps cost more than they share).
const SERIAL: usize = 2000;

/// `SERIAL`, or `RERAIL_CSAT_SERIAL` (a measurement aid, and the tests' way to the group's propagation).
fn serial_below() -> usize {
    std::env::var("RERAIL_CSAT_SERIAL").ok().and_then(|w| w.parse().ok()).unwrap_or(SERIAL)
}

/// A byte offset in the block as the kernel reads it: in words.
fn words(at: usize) -> Result<u32, String> {
    if at % 4 != 0 {
        return Err(format!("the prover's GPU: an offset {at} not of a word"));
    }
    u32::try_from(at / 4).map_err(|_| format!("the prover's GPU: an offset {at} past 16 GiB"))
}

/// Each formula solved on the process's device. An error where there is no device, the kernel does not compile, or
/// a formula is not in the block the others are.
pub fn solve(formulas: &[Formula]) -> Result<Vec<Solved>, String> {
    solve_in(formulas, STATE_BYTES)
}

/// `solve`, with the slots' states in `state_bytes` at most (one slot at least).
pub fn solve_in(formulas: &[Formula], state_bytes: usize) -> Result<Vec<Solved>, String> {
    let mut out: Vec<Option<Solved>> = (0..formulas.len()).map(|_| None).collect();
    if formulas.is_empty() {
        return Ok(Vec::new());
    }
    let mut batch = Batch::new(formulas, state_bytes)?;
    while !batch.finished() {
        for (i, x) in batch.step()? {
            out[i] = Some(x);
        }
    }
    // With `RERAIL_DEVICE_SOLVE_TRACE`: the batch's launches and the time they ran (decision 0670).
    if std::env::var_os("RERAIL_DEVICE_SOLVE_TRACE").is_some() {
        eprintln!("gpu solve: {} formulas, {} slots of {} words, {} launches, {:.1} ms launched", formulas.len(), batch.slots, batch.slot_words, batch.launches, batch.ran.as_secs_f64() * 1e3);
    }
    Ok(out.into_iter().map(|x| x.expect("every formula the device's")).collect())
}

/// A formula as the kernel reads it: its descriptor (where its model and what it carries go, the batch's to fill),
/// what its report holds, and its state's words. Owned: a solve hands it to the device's thread (`Service`) and
/// waits, its formula unchanged in the block meanwhile.
pub struct Desc {
    words: [u32; DESC],
    vars: usize,
    resume: bool,
    carries: bool,
    slot_words: usize,
    block: FormulaBlock,
}

/// `f`'s descriptor: an error where a part of it is not in its gates' block.
pub fn describe(f: &Formula) -> Result<Desc, String> {
    let elsewhere = || "the prover's GPU: a formula not in its block".to_string();
    let (b, rec, args) = f.gates.placed().ok_or_else(elsewhere)?;
    let (b2, lits, ends) = f.clauses.placed().ok_or_else(elsewhere)?;
    let (fs, fg) = f.fan.placed_narrow().ok_or_else(elsewhere)?;
    if !b2.same(b) {
        return Err(elsewhere());
    }
    let (fos, fo) = f.fan.placed_occ().ok_or_else(elsewhere)?;
    let (os, og, oos, oo) = match f.own {
        Some(o) => {
            let (s, g) = o.placed_narrow().ok_or_else(elsewhere)?;
            let (qs, q) = o.placed_occ().ok_or_else(elsewhere)?;
            (words(s)?, words(g)?, words(qs)?, words(q)?)
        }
        None => (u32::MAX, u32::MAX, u32::MAX, u32::MAX),
    };
    let wide = wide(f.gates);
    if wide - 1 >= 1 << 16 {
        return Err(format!("the prover's GPU: a gate {} wide, past a counter's 16 bits", wide - 2));
    }
    // A window's solve: its window's level 0 and the clauses earlier solves carried, in the block too.
    let none = u32::MAX;
    let mut r = [0u32, none, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, none];
    let carried = match f.resume {
        Some(x) => {
            let (bs, set) = x.start.set.placed().map_or((None, 0), |(b, a)| (Some(b), a));
            let (bc, conflict) = x.start.conflict.placed().map_or((None, 0), |(b, a)| (Some(b), a));
            let (bl, clits, cends) = x.carried.placed().map_or((None, 0, 0), |(b, l, e)| (Some(b), l, e));
            // An empty one may be on the heap: nothing is read from it.
            let off = |o: Option<&FormulaBlock>, n: usize| o.is_some_and(|o| o.same(b)) || n == 0;
            if !off(bs, x.start.set.len()) || !off(bc, x.start.conflict.len()) || !off(bl, x.carried.len()) {
                return Err(elsewhere());
            }
            let cv = carry_vars(f);
            r = [
                1,
                if x.assume != 0 { x.window_clauses as u32 } else { none },
                x.assume as u32,
                cv as u32,
                if bs.is_some() { words(set)? } else { 0 },
                (x.start.set.len() / 3) as u32,
                if bc.is_some() { words(conflict)? } else { 0 },
                x.start.conflict.len() as u32,
                u32::from(x.start.broken),
                if bl.is_some() { words(clits)? } else { 0 },
                if bl.is_some() { words(cends)? } else { 0 },
                x.carried.len() as u32,
                if cv != 0 { 0 } else { none },
            ];
            x.carried.len()
        }
        None => 0
    };
    let d: [u32; DESC] = [
        f.vars as u32,
        f.base as u32,
        f.gates.len() as u32,
        words(rec)?,
        words(args)?,
        words(fs)?,
        words(fg)?,
        os,
        og,
        words(lits)?,
        words(ends)?,
        f.clauses.len() as u32,
        0,
        learned(f.vars),
        arena(f.vars),
        f.conflicts,
        f.fan.vars() as u32,
        wide as u32,
        r[0],
        r[1],
        r[2],
        r[3],
        r[4],
        r[5],
        r[6],
        r[7],
        r[8],
        r[9],
        r[10],
        r[11],
        r[12],
        words(fos)?,
        words(fo)?,
        oos,
        oo,
        f.gates.wides() as u32,
        u32::from(f.gates.len() + f.clauses.len() < serial_below()),
        0,
        0,
        0,
    ];
    let cv = carry_vars(f);
    Ok(Desc {
        words: d,
        resume: f.resume.is_some(),
        carries: cv != 0,
        vars: f.vars,
        slot_words: state_words(f.vars, f.clauses.len() + carried, wide, f.gates.wides()),
        block: b.clone(),
    })
}

/// A batch on the device: one launch at a time (`poll` launches where formulas wait and none runs), which runs
/// while the host adds formulas (`add`) and reads their results (`poll`), all in pinned memory both see. Its
/// descriptors are its own buffer, which never moves under a launch. It holds no borrow of its formulas: what it
/// reads of them is in the block, which their owners leave as it is while they wait for their results.
pub struct Batch {
    n: usize,
    /// The formulas published to the launches (`Q_AVAIL`): those within the block's extent the running launch sees.
    published: usize,
    /// Its room: formulas, model words and carry words.
    cap: usize,
    model_cap: usize,
    carry_cap: usize,
    model_words: usize,
    carry_words: usize,
    block: FormulaBlock,
    /// Each formula's extent: the block's bytes in use when it was added, which hold it.
    need: Vec<usize>,
    /// The block's bytes the running launch's wrap covers.
    extent: usize,
    slots: usize,
    slot_words: usize,
    function: &'static rerail_gpu::device::ComputeFunction,
    q: ComputeQueue,
    blk: rerail_gpu::device::Buffer,
    table: rerail_gpu::device::Buffer,
    state: rerail_gpu::device::Buffer,
    out: rerail_gpu::device::Buffer,
    models: rerail_gpu::device::Buffer,
    carry: rerail_gpu::device::Buffer,
    queue: rerail_gpu::device::Buffer,
    /// Each formula's variables, whether it resumes a window, where its model and what it carries are.
    meta: Vec<(usize, bool, usize, Option<usize>)>,
    /// The formulas not yet reported, to look at.
    open: Vec<usize>,
    left: usize,
    running: Option<rerail_gpu::device::ComputeEvent>,
    pub launches: usize,
    /// A measurement aid: the launches stopped for a formula past the block's extent, and the time launches ran.
    pub stops: usize,
    pub ran: std::time::Duration,
    launched_at: Option<std::time::Instant>,
}

/// `n` words of a buffer the host sees, from word `at`.
fn words_of(b: &rerail_gpu::device::Buffer, at: usize, n: usize) -> Vec<u32> {
    let p = b.contents().cast::<u32>();
    if p.is_null() {
        return read::<u32>(b, at + n)[at..].to_vec();
    }
    // SAFETY: the buffer holds at least `at + n` words (the batch sized it); what a launch may still write is read
    // only once its status says it is written.
    unsafe { std::slice::from_raw_parts(p.add(at), n).to_vec() }
}

/// Word `i` of a buffer the host and a running launch share.
fn shared_word(b: &rerail_gpu::device::Buffer, i: usize) -> &std::sync::atomic::AtomicU32 {
    let p = b.contents().cast::<std::sync::atomic::AtomicU32>();
    assert!(!p.is_null(), "the prover's GPU: a buffer the host can not see");
    // SAFETY: the batch sized the buffer past `i`; the device's accesses to it are atomic or ordered by fences.
    unsafe { &*p.add(i) }
}

/// The queue's words (the kernel's): the formulas taken (word 0), those published, the stop, the slots whose launch
/// ended, each slot's formula.
const Q_AVAIL: usize = 1;
const Q_STOP: usize = 2;
const Q_ENDED: usize = 3;
const Q_SLOT: usize = 4;

impl Batch {
    /// The batch laid out. An error where there is no device, the kernel does not compile, or a formula is not in the
    /// block the others are.
    pub fn new(formulas: &[Formula], state_bytes: usize) -> Result<Batch, String> {
        let descs = formulas.iter().map(describe).collect::<Result<Vec<_>, _>>()?;
        Batch::of(&descs, state_bytes)
    }

    /// A batch of formulas described (`describe`), all in one block.
    pub fn of(descs: &[Desc], state_bytes: usize) -> Result<Batch, String> {
        let first = descs.first().ok_or("the prover's GPU: an empty batch")?;
        let models = descs.iter().map(|d| (d.vars + 1).div_ceil(32)).sum();
        let carries = descs.iter().filter(|d| d.carries).count() * (1 + CARRY_ROOM);
        let slot_words = descs.iter().map(|d| d.slot_words).max().unwrap_or(1);
        let slots = (state_bytes / (4 * slot_words)).clamp(1, SLOTS).min(descs.len());
        let mut b = Batch::open(&first.block, descs.len(), models, carries, slot_words, slots)?;
        for d in descs {
            b.add(d)?.ok_or("the prover's GPU: a batch without the room its formulas took")?;
        }
        Ok(b)
    }

    /// An empty batch of `slots` slots of `slot_words`, with room for `cap` formulas, `model_cap` words of models
    /// and `carry_cap` of what they carry, its queue cleared.
    pub fn open(block: &FormulaBlock, cap: usize, model_cap: usize, carry_cap: usize, slot_words: usize, slots: usize) -> Result<Batch, String> {
        let shared = rerail_gpu::shared::get().ok_or("the prover's GPU: no device")?;
        let function = program("bitlex_csat_gpu.uglykl", KERNEL)?.function("csat_solve")?;
        if slots * slot_words > u32::MAX as usize {
            return Err("the prover's GPU: the slots' states past 16 GiB".to_string());
        }
        let b = Batch {
            n: 0,
            published: 0,
            cap,
            model_cap,
            carry_cap,
            model_words: 0,
            carry_words: 0,
            block: block.clone(),
            need: Vec::with_capacity(cap),
            extent: 0,
            slots,
            slot_words,
            function,
            q: ComputeQueue::new(&shared.queue),
            blk: block.block().device_buffer(&shared.device)?,
            table: alloc::<u32>(cap.max(1) * DESC)?,
            // The device's own memory: the host never reads a state.
            state: alloc_working::<u32>(slots * slot_words)?,
            out: alloc::<u32>(cap.max(1) * OUT)?,
            models: alloc::<u32>(model_cap.max(1))?,
            carry: alloc::<u32>(carry_cap.max(1))?,
            queue: alloc::<u32>(Q_SLOT + slots)?,
            meta: Vec::with_capacity(cap),
            open: Vec::with_capacity(cap),
            left: 0,
            running: None,
            launches: 0,
            stops: 0,
            ran: std::time::Duration::ZERO,
            launched_at: None,
        };
        use std::sync::atomic::Ordering::Relaxed;
        for i in 0..Q_SLOT {
            shared_word(&b.queue, i).store(0, Relaxed);
        }
        for s in 0..slots {
            shared_word(&b.queue, Q_SLOT + s).store(u32::MAX, Relaxed);
        }
        Ok(b)
    }

    /// `d` added, at any time: its place, or `None` where the batch has no room for it (or it is in another block, or
    /// its state is larger than a slot). A running launch takes it where the block's extent it sees holds it, else the
    /// launch is stopped and the next takes it.
    pub fn add(&mut self, d: &Desc) -> Result<Option<usize>, String> {
        use std::sync::atomic::Ordering::Relaxed;
        let model = (d.vars + 1).div_ceil(32);
        let carry = if d.carries { 1 + CARRY_ROOM } else { 0 };
        if self.n == self.cap || self.model_words + model > self.model_cap || self.carry_words + carry > self.carry_cap || d.slot_words > self.slot_words || !d.block.same(&self.block) {
            return Ok(None);
        }
        let mut w = d.words;
        w[D_MODEL] = u32::try_from(self.model_words).map_err(|_| "the prover's GPU: a batch's models past 16 GiB".to_string())?;
        let carry_at = d.carries.then_some(self.carry_words);
        if let Some(c) = carry_at {
            w[D_CARRY] = c as u32;
        }
        let k = self.n;
        for (i, &x) in w.iter().enumerate() {
            shared_word(&self.table, k * DESC + i).store(x, Relaxed);
        }
        shared_word(&self.out, k * OUT).store(0, Relaxed);
        self.meta.push((d.vars, d.resume, self.model_words, carry_at));
        self.open.push(k);
        self.need.push(self.block.block().used());
        self.model_words += model;
        self.carry_words += carry;
        self.n += 1;
        self.left += 1;
        if self.running.is_some() {
            self.publish();
        }
        Ok(Some(k))
    }

    /// The formulas added that the running launch can read, published to it (in order); where one lies past its
    /// extent, the launch is stopped.
    fn publish(&mut self) {
        use std::sync::atomic::Ordering;
        let mut p = self.published;
        while p < self.n && self.need[p] <= self.extent {
            p += 1;
        }
        if p > self.published {
            self.published = p;
            std::sync::atomic::fence(Ordering::SeqCst);
            shared_word(&self.queue, Q_AVAIL).store(p as u32, Ordering::Release);
        }
        if p < self.n {
            if shared_word(&self.queue, Q_STOP).swap(1, Ordering::AcqRel) == 0 {
                self.stops += 1;
            }
        }
    }

    /// Whether every formula's result is out and no launch runs.
    pub fn finished(&self) -> bool {
        self.left == 0 && self.running.is_none()
    }

    /// Whether the batch has no room left for formulas.
    pub fn full(&self) -> bool {
        self.n == self.cap
    }

    /// The formulas whose results are out since the last poll, each its result. A launch ended is awaited, and one
    /// made where formulas wait.
    pub fn poll(&mut self) -> Result<Vec<(usize, Solved)>, String> {
        use std::sync::atomic::Ordering;
        if self.running.as_ref().is_some_and(|e| e.is_complete()) {
            self.running.take().expect("running").await_()?;
            if let Some(t) = self.launched_at.take() {
                self.ran += t.elapsed();
            }
        }
        let mut done = Vec::new();
        let open = std::mem::take(&mut self.open);
        for &i in &open {
            let st = shared_word(&self.out, i * OUT).load(Ordering::Acquire);
            match st {
                0 => continue,
                BROKEN => return Err(format!("the device's solver broke on formula {i}: a loop past its bound")),
                _ => {
                    let o = words_of(&self.out, i * OUT, OUT);
                    done.push((i, self.decode(i, &o)));
                }
            }
        }
        let finished: std::collections::HashSet<usize> = done.iter().map(|&(i, _)| i).collect();
        self.open = open.into_iter().filter(|i| !finished.contains(i)).collect();
        self.left -= done.len();
        // A launch where formulas wait for one: added since, or held by a slot.
        if self.running.is_none() && self.left > 0 {
            self.launch()?;
        }
        Ok(done)
    }

    /// The results out, waiting for the running launch where none is.
    pub fn step(&mut self) -> Result<Vec<(usize, Solved)>, String> {
        let got = self.poll()?;
        if got.is_empty()
            && let Some(e) = self.running.take()
        {
            e.await_()?;
            return self.poll();
        }
        Ok(got)
    }

    fn launch(&mut self) -> Result<(), String> {
        use std::sync::atomic::Ordering;
        // Each formula's every launch makes steps (7 at least before it may end early): past the launches that
        // bounds, the device is at fault.
        let most = self.n * ((crate::bitlex_csat_lanes::STEPS as usize).div_ceil(7) + 2) + 1;
        if self.launches > most {
            return Err(format!("the prover's GPU: a batch past {most} launches"));
        }
        self.launches += 1;
        // The block as it is now: the formulas added lie within it.
        let shared = rerail_gpu::shared::get().ok_or("the prover's GPU: no device")?;
        self.blk = self.block.block().device_buffer(&shared.device)?;
        self.extent = self.block.block().device_extent();
        shared_word(&self.queue, Q_STOP).store(0, Ordering::Release);
        shared_word(&self.queue, Q_ENDED).store(0, Ordering::Release);
        self.publish();
        let room = shared_room();
        let a = Args { n: self.cap as u32, steps: STEPS, init: 0, slots: self.slots as u32, slot_words: self.slot_words as u32, room: room as u32, pad1: 0, pad2: 0 };
        launch(&self.q, self.function)
            .value("args", &a)
            .buffer("blk", &self.blk)
            .buffer("table", &self.table)
            .buffer("state", &self.state)
            .buffer("out", &self.out)
            .buffer("models", &self.models)
            .buffer("carry", &self.carry)
            .buffer("queue", &self.queue)
            .local("sh", (4 * (HEADER + 2 * BQ + room)) as u64)
            .run(&[self.slots as u64], &[LANES])?;
        self.running = Some(self.q.flush());
        self.launched_at = Some(std::time::Instant::now());
        Ok(())
    }

    /// Formula `i`'s result, from its report `o`.
    fn decode(&self, i: usize, o: &[u32]) -> Solved {
        let (vars, resume, m, c) = self.meta[i];
        // The model, or a refuted window's solve's core, in its words.
        let bits = (o[0] == SAT || (o[0] == UNSAT && resume)).then(|| {
            let w = words_of(&self.models, m, (vars + 1).div_ceil(32));
            (0..=vars).map(|v| w[v / 32] >> (v % 32) & 1 != 0).collect::<Vec<bool>>()
        });
        let carry = c.map_or_else(Vec::new, |c| {
            let n = words_of(&self.carry, c, 1)[0] as usize;
            let w = words_of(&self.carry, c + 1, n);
            let (mut k, mut out) = (0usize, Vec::new());
            while k < n {
                let len = w[k] as usize;
                out.push(w[k + 1..k + 1 + len].iter().map(|&l| l as crate::bitlex_sat::Lit).collect());
                k += 1 + len;
            }
            out
        });
        let (answer, used) = match o[0] {
            SAT => (Answer::Sat(bits.expect("a model")), None),
            UNSAT => (Answer::Unsat, bits),
            _ => (Answer::Unknown, None),
        };
        Solved {
                answer,
                roomless: o[0] == ROOMLESS,
                conflicts: o[1],
                decisions: o[2],
                rounds: o[3],
                learned: o[4],
                learned_literals: o[5],
                visits: o[6],
                items: o[7],
                reductions: o[8],
                carry,
                used,
                cycles: [o[9], o[10], o[11], o[12], o[13], o[14]],
        }
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        // Its buffers outlive the launch reading them.
        if let Some(e) = self.running.take() {
            let _ = e.await_();
        }
    }
}

/// The device as a service to the threads that solve (decision 0670): a work queue. A thread hands it a formula
/// (`solve`) and waits for its result; the device's thread (`run`) adds it to the batch on the device
/// (`Batch::add`), which a running launch takes at once, and looks at the results every `POLL`, handing each back the
/// moment it is out; a launch is made only where formulas wait and none runs. Every solve handed in is the device's:
/// the device's throughput is its searches at once, so the threads asking are many, those waiting leaving the cores
/// to those building their formulas. Only a device that fails (or a formula it cannot describe) leaves solves to the
/// CPU.
pub struct Service {
    state: std::sync::Mutex<Waiting>,
    wake: std::sync::Condvar,
    trace: bool,
    solve_trace: bool,
}

struct Waiting {
    handed: Vec<(Desc, std::sync::Arc<Reply>)>,
    closed: bool,
    failed: Option<String>,
}

/// A handed formula's result: the device's, or `None` (the device failed: the CPU's).
#[derive(Default)]
struct Reply {
    got: std::sync::Mutex<Option<Option<Solved>>>,
    ready: std::sync::Condvar,
    /// The device done with it (a measurement aid).
    given: std::sync::atomic::AtomicBool,
}

impl Reply {
    fn give(&self, x: Option<Solved>) {
        let mut got = self.got.lock().expect("a reply");
        if got.is_none() {
            *got = Some(x);
        }
        drop(got);
        self.given.store(true, std::sync::atomic::Ordering::Release);
        self.ready.notify_all();
    }
}

/// How often the device's thread looks at a running launch's results.
const POLL: std::time::Duration = std::time::Duration::from_micros(20);

impl Default for Service {
    fn default() -> Self {
        Service {
            state: std::sync::Mutex::new(Waiting { handed: Vec::new(), closed: false, failed: None }),
            wake: std::sync::Condvar::new(),
            trace: std::env::var_os("RERAIL_PROVING_TRACE").is_some(),
            solve_trace: std::env::var_os("RERAIL_DEVICE_SOLVE_TRACE").is_some(),
        }
    }
}

/// A batch on the device and the threads waiting on its formulas.
struct Live {
    batch: Batch,
    replies: Vec<std::sync::Arc<Reply>>,
    /// When each formula was handed in (`RERAIL_DEVICE_SOLVE_TRACE`).
    handed_at: Vec<std::time::Instant>,
    at: std::time::Instant,
}

impl Service {
    /// `f` solved on the device: the CPU's result (the same search).
    pub fn solve(&self, f: &Formula) -> Solved {
        let reply = std::sync::Arc::new(Reply::default());
        {
            let mut w = self.state.lock().expect("the service");
            let d = match describe(f) {
                Ok(d) if w.failed.is_none() && !w.closed => d,
                _ => {
                    drop(w);
                    return crate::bitlex_csat_lanes::solve(f);
                }
            };
            w.handed.push((d, reply.clone()));
        }
        self.wake.notify_all();
        let got = reply.got.lock().expect("a reply");
        let mut got = reply.ready.wait_while(got, |g| g.is_none()).expect("a reply");
        match got.take() {
            Some(Some(x)) => x,
            // The device failed: the CPU's.
            _ => {
                drop(got);
                crate::bitlex_csat_lanes::solve(f)
            }
        }
    }

    /// No more formulas: `run` returns once those handed in are done.
    pub fn close(&self) {
        self.state.lock().expect("the service").closed = true;
        self.wake.notify_all();
    }

    /// The device's thread, until `close`: an error where the device failed (every solve then the CPU's).
    pub fn run(&self) -> Result<(), String> {
        let mut live: Vec<Live> = Vec::new();
        let r = self.serve(&mut live);
        if let Err(e) = &r {
            let mut w = self.state.lock().expect("the service");
            w.failed = Some(e.clone());
            for (_, reply) in w.handed.drain(..) {
                reply.give(None);
            }
            drop(w);
            for l in &live {
                for r in &l.replies {
                    r.give(None);
                }
            }
        }
        r
    }

    fn serve(&self, live: &mut Vec<Live>) -> Result<(), String> {
        let mut batches = 0usize;
        // A measurement aid (`RERAIL_PROVING_TRACE`): the formulas on the device at once, over time.
        let mut busy = Occupancy::default();
        loop {
            if self.trace {
                busy.at(live.iter().map(|l| l.replies.iter().filter(|r| !r.given.load(std::sync::atomic::Ordering::Acquire)).count()).sum());
            }
            let handed = {
                let mut w = self.state.lock().expect("the service");
                loop {
                    if !w.handed.is_empty() {
                        break;
                    }
                    // A launch running or formulas waiting: its results looked at again soon.
                    if live.iter().any(|l| !l.batch.finished()) {
                        w = self.wake.wait_timeout(w, POLL).expect("the service").0;
                        break;
                    }
                    if w.closed {
                        for l in live.drain(..) {
                            self.retired(&l, &mut batches);
                        }
                        if self.trace {
                            busy.report();
                        }
                        return Ok(());
                    }
                    w = self.wake.wait(w).expect("the service");
                }
                std::mem::take(&mut w.handed)
            };
            // Each added to the batch on the device, the next launch on; a new batch where it has no room.
            for (d, reply) in handed {
                let added = match live.last_mut() {
                    Some(l) => l.batch.add(&d)?,
                    None => None,
                };
                let l = match added {
                    Some(_) => live.last_mut().expect("added"),
                    None => {
                        let slot_words = 2 * d.slot_words;
                        // A measurement aid: `RERAIL_PROVER_STATE_MB`, the slots' states at most.
                        let state_bytes = std::env::var("RERAIL_PROVER_STATE_MB").ok().and_then(|v| v.parse::<usize>().ok()).map_or(STATE_BYTES, |m| m << 20);
                        let slots = (state_bytes / (4 * slot_words)).clamp(1, SLOTS);
                        let models = MODEL_ROOM.max((d.vars + 1).div_ceil(32));
                        let t_open = std::time::Instant::now();
                        let batch = Batch::open(&d.block, BATCH_ROOM, models, BATCH_ROOM * (1 + CARRY_ROOM), slot_words, slots)?;
                        if self.trace {
                            eprintln!("gpu batch opened in {:.1} ms: {slots} slots of {slot_words} words", t_open.elapsed().as_secs_f64() * 1e3);
                        }
                        live.push(Live { batch, replies: Vec::new(), handed_at: Vec::new(), at: std::time::Instant::now() });
                        let l = live.last_mut().expect("opened");
                        l.batch.add(&d)?.ok_or("the prover's GPU: a formula without room in an empty batch")?;
                        l
                    }
                };
                l.replies.push(reply);
                l.handed_at.push(std::time::Instant::now());
            }
            // Each batch's results out, and a launch where formulas wait and none runs.
            for l in live.iter_mut() {
                if l.batch.finished() {
                    continue;
                }
                for (k, x) in l.batch.poll()? {
                    // A measurement aid: each result, its time from handed in, its search's counts and the cycles of
                    // its parts.
                    if self.solve_trace {
                        let ms = l.handed_at[k].elapsed().as_secs_f64() * 1e3;
                        eprintln!("device solve {ms:.2} ms: {} conflicts, {} decisions, {} rounds, {} visits, {} items, {} reductions, kcycles visit/items/analyze/back/decide/reduce {:?}", x.conflicts, x.decisions, x.rounds, x.visits, x.items, x.reductions, x.cycles.map(|c| u64::from(c) * 256 / 1000));
                    }
                    l.replies[k].give(Some(x));
                }
            }
            // A batch out of room is done with once its formulas are.
            let last = live.len().saturating_sub(1);
            let mut i = 0;
            live.retain(|l| {
                let keep = !(l.batch.finished() && (l.batch.full() || i < last));
                if !keep {
                    self.retired(l, &mut batches);
                }
                i += 1;
                keep
            });
        }
    }

    fn retired(&self, l: &Live, batches: &mut usize) {
        if self.trace {
            eprintln!("gpu batch {batches}: {} formulas, {} launches, {:.1} ms, {} stopped for the block, {:.1} ms launched, {} slots of {} words", l.replies.len(), l.batch.launches, l.at.elapsed().as_secs_f64() * 1e3, l.batch.stops, l.batch.ran.as_secs_f64() * 1e3, l.batch.slots, l.batch.slot_words);
        }
        *batches += 1;
    }
}

/// The formulas on the device at once, over time (a measurement aid): the time at each count, in powers of two.
#[derive(Default)]
struct Occupancy {
    last: Option<(std::time::Instant, usize)>,
    by_log: [f64; 16],
    weighted: f64,
    most: usize,
}

impl Occupancy {
    fn at(&mut self, n: usize) {
        let now = std::time::Instant::now();
        if let Some((t, k)) = self.last {
            let dt = (now - t).as_secs_f64();
            self.by_log[(usize::BITS - k.leading_zeros()) as usize] += dt;
            self.weighted += dt * k as f64;
        }
        self.last = Some((now, n));
        self.most = self.most.max(n);
    }

    fn report(&self) {
        let total: f64 = self.by_log.iter().sum();
        let busy: f64 = self.by_log[1..].iter().sum();
        let parts: Vec<String> = (0..16).filter(|&i| self.by_log[i] > 0.0).map(|i| format!("{}: {:.3} s", if i == 0 { 0 } else { 1usize << (i - 1) }, self.by_log[i])).collect();
        eprintln!("gpu occupancy: {total:.3} s served, {busy:.3} s with formulas on the device, {:.1} on average while busy, {} at most; time at least n: {}", self.weighted / busy.max(1e-9), self.most, parts.join(", "));
    }
}

/// A service's batch's room: formulas, and words of their models.
const BATCH_ROOM: usize = 1 << 12;
const MODEL_ROOM: usize = 1 << 22;

#[cfg(test)]
mod tests {
    use super::*;

    /// A measurement (decision 0670): a chain of dependent reads, one lane, over pinned host memory (`alloc`), host
    /// memory registered for the device (`wrap_host_memory`, as the formula block is) and the device's own
    /// (`alloc_working`), each in a size the L2 cache holds and one it does not: cycles a read.
    #[test]
    #[ignore]
    fn memory_latency_by_kind() {
        const CHASE: &str = r#"
#include <ukl/simd.h>
typedef struct ChaseArgs { uint32_t steps; uint32_t pad0; uint32_t pad1; uint32_t pad2; } ChaseArgs;
UKL_REFLECT_STRUCT(ChaseArgs, steps, pad0, pad1, pad2)
DEFINE_KERNEL2(chase, chase,
               ARGS,     ChaseArgs, args, ,
               ROBUFFER, uint32_t,  buf,  "[n]",
               RWBUFFER, uint32_t,  out,  "[2]",
               TGID0,    uint32_t,  g,    ,
               LANE,     uint32_t,  lane, )
{
    uint32_t i = 0u;
    uint64_t t = ukl_clock();
    for (uint32_t s = 0u; s < args.steps; s++)
        i = buf[i];
    if (lane == 0u) {
        out[0] = i;
        out[1] = (uint32_t)(ukl_clock() - t);
    }
}
"#;
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct ChaseArgs {
            steps: u32,
            pad: [u32; 3],
        }
        // SAFETY: four u32s.
        unsafe impl rerail_gpu::Pod for ChaseArgs {}
        let Some(shared) = rerail_gpu::shared::get() else { return };
        let f = program("chase.uglykl", CHASE).and_then(|p| p.function("chase")).expect("the chase kernel");
        let mut q = ComputeQueue::new(&shared.queue);
        let steps = 20_000u32;
        for words in [1usize << 12, 1 << 16, 1 << 26] {
            // A cycle through every 64-byte line, in a shuffled order.
            let lines = words / 16;
            let mut order: Vec<usize> = (0..lines).collect();
            let mut r = 0x2545_f491_4f6c_dd1du64;
            for i in (1..lines).rev() {
                r ^= r << 13;
                r ^= r >> 7;
                r ^= r << 17;
                order.swap(i, (r % (i as u64 + 1)) as usize);
            }
            let mut next = vec![0u32; words];
            for k in 0..lines {
                next[order[k] * 16] = (order[(k + 1) % lines] * 16) as u32;
            }
            let pinned = alloc::<u32>(words).expect("pinned");
            // SAFETY: `words` u32s.
            unsafe { std::ptr::copy_nonoverlapping(next.as_ptr(), pinned.contents().cast::<u32>(), words) };
            let layout = std::alloc::Layout::from_size_align(4 * words, 1 << 16).expect("a layout");
            // SAFETY: a non-zero size.
            let host = unsafe { std::alloc::alloc(layout) };
            // SAFETY: `host` holds `words` u32s.
            unsafe { std::ptr::copy_nonoverlapping(next.as_ptr(), host.cast::<u32>(), words) };
            // SAFETY: `host` outlives the wrap (freed below, after the launches).
            let wrapped = unsafe { rerail_gpu::device::wrap_host_memory(&shared.device, host, 4 * words) }.expect("a wrap");
            let device = alloc_working::<u32>(words).expect("device memory");
            q.enqueue_copy_between_device_regions(&pinned, &device, 0, 0, 4 * words as u64);
            q.flush().await_().expect("the copy");
            let out = alloc::<u32>(2).expect("out");
            for (name, b) in [("pinned host", &pinned), ("registered host", &wrapped), ("device", &device)] {
                let mut cycles = 0u32;
                for _ in 0..2 {
                    launch(&q, f).value("args", &ChaseArgs { steps, pad: [0; 3] }).buffer("buf", b).buffer("out", &out).run(&[1], &[LANES]).expect("a launch");
                    q.flush().await_().expect("the launch");
                    cycles = words_of(&out, 1, 1)[0];
                }
                eprintln!("{} KB, {name}: {:.0} cycles a read", 4 * words / 1024, f64::from(cycles) / f64::from(steps));
            }
            drop(wrapped);
            // SAFETY: allocated above with `layout`; the wrap is dropped and no launch runs.
            unsafe { std::alloc::dealloc(host, layout) };
        }
    }
    /// A measurement (decision 0670): one SIMD group's chain of dependent operations of one kind, cycles each: a
    /// shuffle, a ballot, a match, a reduction, an exclusive sum, a group barrier with a threadgroup read, a
    /// threadgroup read, a threadgroup atomic add, and a read of the device's memory that L1 holds.
    #[test]
    #[ignore]
    fn warp_operation_latency() {
        const OPS: &str = r#"
#include <ukl/simd.h>
typedef struct OpArgs { uint32_t steps; uint32_t op; uint32_t pad1; uint32_t pad2; } OpArgs;
UKL_REFLECT_STRUCT(OpArgs, steps, op, pad1, pad2)
DEFINE_KERNEL2(ops, ops,
               ARGS,     OpArgs,   args, ,
               RWBUFFER, uint32_t, buf,  "[1024]",
               RWBUFFER, uint32_t, out,  "[2]",
               RWLOCAL,  uint32_t, sh,   "[1024]",
               TGID0,    uint32_t, g,    ,
               LANE,     uint32_t, lane, )
{
    for (uint32_t k = lane; k < 1024u; k += UKL_SIMD_WIDTH)
        sh[k] = (k * 7u + 1u) & 1023u;
    ukl_sg_barrier();
    uint32_t x = lane, op = args.op;
    uint64_t t = ukl_clock();
    for (uint32_t s = 0u; s < args.steps; s++) {
        if (op == 0u)
            x = ukl_sg_shuffle(x, (x + 1u) & 31u __UKL_ARGS);
        else if (op == 1u)
            x = (uint32_t)ukl_sg_ballot(((x >> (lane & 7u)) & 1u) != 0u __UKL_ARGS) + lane;
        else if (op == 2u)
            x = (uint32_t)ukl_sg_match(x & 3u __UKL_ARGS) + lane;
        else if (op == 3u)
            x = ukl_sg_sum(x __UKL_ARGS) + lane;
        else if (op == 4u)
            x = ukl_sg_exclusive_sum(x & 7u, lane __UKL_ARGS) + 1u;
        else if (op == 5u) {
            sh[lane] = x;
            ukl_sg_barrier();
            x = sh[(x + 1u) & 31u];
        } else if (op == 6u)
            x = sh[x & 1023u];
        else if (op == 7u)
            x = ukl_atomic_add_local(sh + (x & 1023u), 1u) & 1023u;
        else
            x = buf[x & 1023u];
    }
    if (lane == 0u) {
        out[0] = x;
        out[1] = (uint32_t)(ukl_clock() - t);
    }
}
"#;
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct OpArgs {
            steps: u32,
            op: u32,
            pad: [u32; 2],
        }
        // SAFETY: four u32s.
        unsafe impl rerail_gpu::Pod for OpArgs {}
        let Some(shared) = rerail_gpu::shared::get() else { return };
        let mut q = ComputeQueue::new(&shared.queue);
        let f = program("ops.uglykl", OPS).expect("the kernel").function("ops").expect("ops");
        let buf = alloc_working::<u32>(1024).expect("device memory");
        let init: Vec<u32> = (0..1024u32).map(|k| (k * 7 + 1) & 1023).collect();
        let host = rerail_gpu::kernels::buffer_of(&init).expect("a buffer");
        q.enqueue_copy_between_device_regions(&host, &buf, 0, 0, 4096);
        q.flush().await_().expect("the copy");
        let out = alloc::<u32>(2).expect("out");
        let steps = 1u32 << 14;
        let names = ["shuffle", "ballot", "match", "sum", "exclusive sum", "barrier and threadgroup read", "threadgroup read", "threadgroup atomic add", "device read (L1)"];
        for (op, name) in names.iter().enumerate() {
            let mut cycles = 0u32;
            for _ in 0..2 {
                launch(&q, f).value("args", &OpArgs { steps, op: op as u32, pad: [0; 2] }).buffer("buf", &buf).buffer("out", &out).local("sh", 4096).run(&[1], &[LANES]).expect("a launch");
                q.flush().await_().expect("the launch");
                cycles = words_of(&out, 1, 1)[0];
            }
            eprintln!("{name}: {:.1} cycles", f64::from(cycles) / f64::from(steps));
        }
    }

    use crate::bitlex_csat::{Fanout, Gates, Solver};
    use crate::bitlex_csat_lanes::tests::{circuit, holds, Rng};

    /// The device's solve of `f` against the CPU's search (`here`): their answers agree where both have one (the
    /// device's search is its own, not the CPU's step for step, decision 0670); its model holds; unsatisfiable, the
    /// whole formula is, and still is without the own units its core (`used`) leaves out; with no conflict to spend,
    /// it is unsatisfiable where the CPU's is (level 0's propagation, the same in any order); each clause it carries
    /// the window's clauses and gates imply.
    fn checked(i: usize, f: &Formula, device: &Solved, here: &Solved) {
        use crate::bitlex_sat;
        let lvar = |l: crate::bitlex_sat::Lit| l.unsigned_abs() as usize;
        let unsat = |cs: &Clauses| {
            let mut o = bitlex_sat::Solver::new(f.vars);
            o.add_all(cs);
            o.solve(u32::MAX) == CpuAnswer::Unsat
        };
        match (&device.answer, &here.answer) {
            (Answer::Sat(m), a) => {
                assert!(holds(m, f.gates, f.clauses), "formula {i}: the device's model does not hold");
                assert!(!matches!(a, Answer::Unsat), "formula {i}: the device satisfiable, the CPU not");
            }
            (Answer::Unsat, a) => {
                assert!(!matches!(a, Answer::Sat(_)), "formula {i}: the device unsatisfiable, the CPU not");
                let mut whole = Clauses::default();
                for c in f.clauses.iter() {
                    let dropped = c.len() == 1 && device.used.as_ref().is_some_and(|u| !u[lvar(c[0])]);
                    if !dropped {
                        whole.push(c);
                    }
                }
                f.gates.cnf(&mut whole);
                assert!(unsat(&whole), "formula {i}: unsatisfiable on the device, but not without the units its core left out");
            }
            (Answer::Unknown, a) => {
                assert!(f.conflicts < 100_000 || device.roomless, "formula {i}: the device has no answer, the CPU {a:?}");
            }
        }
        if f.conflicts == 0 {
            assert_eq!(matches!(device.answer, Answer::Unsat), matches!(here.answer, Answer::Unsat), "formula {i}: level 0's propagation");
        }
        if let Some(r) = &f.resume {
            let mut window = Clauses::default();
            for c in f.clauses.iter().take(r.window_clauses) {
                window.push(c);
            }
            let mut wg = f.gates.clone();
            wg.truncate(f.base);
            wg.cnf(&mut window);
            for c in &device.carry {
                let mut w = window.clone();
                for &l in c {
                    w.push(&[-l]);
                }
                assert!(unsat(&w), "formula {i}: {c:?} carried, not the window's");
            }
        }
    }
    use crate::bitlex_sat::{Answer as CpuAnswer, Clauses};

    /// Random circuits, some with a solve's own gates, through fewer slots than formulas: the device's answers are the
    /// CPU solver's, each model it gives holds, and its search is `bitlex_csat_lanes`'s.
    #[test]
    fn the_devices_answers_are_the_cpus() {
        if rerail_gpu::shared::get().is_none() {
            eprintln!("no device: the prover's GPU solver test did not run");
            return;
        }
        let block = FormulaBlock::new();
        let mut rng = Rng(0x2545f4914f6cdd1d);
        let mut made = Vec::new();
        for i in 0..400usize {
            let inputs = 4 + rng.below(40) as usize;
            let gates_n = rng.below(120) as usize;
            let clauses_n = rng.below(2 * inputs as u64 + 4) as usize;
            let own = if i % 3 == 0 { rng.below(10) as usize } else { 0 };
            made.push(circuit(&mut rng, &block, inputs, gates_n, clauses_n, own));
        }
        let fans: Vec<(Fanout, Fanout)> = made.iter().map(|(g, _, vars, base)| (Fanout::of(g, *base, *vars), Fanout::of_range(g, *base..g.len(), *vars))).collect();
        let formulas: Vec<Formula> = made
            .iter()
            .zip(&fans)
            .map(|((g, c, vars, base), (fan, own))| Formula::new(*vars, g, *base, fan, (*base < g.len()).then_some(own), c, 100_000))
            .collect();
        // A megabyte of states: a few dozen slots, each taking formulas from the queue one after another.
        let got = solve_in(&formulas, 1 << 20).unwrap_or_else(|e| panic!("{}: {e}", rerail_gpu::device::backend()));
        for (i, (f, x)) in formulas.iter().zip(&got).enumerate() {
            checked(i, f, x, &crate::bitlex_csat_lanes::solve(f));
        }
        let (mut sat, mut unsat, mut searched) = (0, 0, 0);
        let mut s = Solver::default();
        for (i, (((g, c, vars, base), (fan, _)), x)) in made.iter().zip(&fans).zip(&got).enumerate() {
            s.reset(*vars);
            for cl in c.iter() {
                s.add(cl);
            }
            let cpu = s.solve(g, fan, *base, 100_000);
            match (&x.answer, cpu) {
                (Answer::Sat(m), CpuAnswer::Sat) => {
                    assert!(holds(m, g, c), "formula {i}: the device's model does not hold");
                    sat += 1;
                }
                (Answer::Unsat, CpuAnswer::Unsat) => unsat += 1,
                (a, b) => panic!("formula {i} ({vars} vars, {} gates, {} clauses): the device {a:?}, the CPU {b:?}", g.len(), c.len()),
            }
            if x.conflicts > 0 {
                searched += 1;
            }
        }
        eprintln!("{sat} satisfiable, {unsat} unsatisfiable, {searched} with conflicts");
        assert!(sat >= 40 && unsat >= 40 && searched >= 40, "{sat} sat, {unsat} unsat, {searched} searched");
    }

    /// Windows' solves resumed on the device (`Resume`), a batch of each window's next solve, what each carries out
    /// carried to the window's next: the device's search is the CPU copy's, its answers, counts, what it carries and
    /// its core.
    #[test]
    fn a_resumed_window_on_the_device_is_the_cpus() {
        use crate::bitlex_csat::Kind;
        use crate::bitlex_csat_lanes::{start, Resume, Start};
        let lvar_of = |l: i32| l.unsigned_abs() as usize;
        if rerail_gpu::shared::get().is_none() {
            eprintln!("no device: the prover's GPU solver test did not run");
            return;
        }
        let block = FormulaBlock::new();
        let mut rng = Rng(0x6a09e667f3bcc908);
        struct W {
            g: crate::bitlex_csat::Gates,
            vars: usize,
            base: usize,
            wvars: usize,
            window: Vec<Vec<i32>>,
            rest: Vec<Vec<i32>>,
            fan: Fanout,
            own: Fanout,
            learned: Clauses,
            start: Option<Start>,
        }
        let mut ws: Vec<W> = (0..300usize)
            .map(|_| {
                let inputs = 4 + rng.below(30) as usize;
                let (gn, cn) = (rng.below(100) as usize, rng.below(2 * inputs as u64 + 4) as usize);
                let own_gates = rng.below(6) as usize;
                let (g, c, vars, base) = circuit(&mut rng, &block, inputs, gn, cn, own_gates);
                let wvars = (0..base).filter(|&i| g.kind(i) != Kind::One).map(|i| lvar_of(g.out(i))).chain((0..base).flat_map(|i| g.args(i).iter().map(|&a| lvar_of(a)))).max().unwrap_or(1).max(1);
                let (mut window, mut rest) = (Vec::new(), Vec::new());
                for cl in c.iter() {
                    if cl.iter().all(|&l| lvar_of(l) <= wvars) { window.push(cl.to_vec()) } else { rest.push(cl.to_vec()) }
                }
                let (fan, own) = (Fanout::of(&g, base, wvars), Fanout::of_range(&g, base..g.len(), vars + 1));
                W { g, vars, base, wvars, window, rest, fan, own, learned: Clauses::in_block(&block), start: None }
            })
            .collect();
        let (mut sat, mut unsat, mut carried, mut searched) = (0, 0, 0, 0);
        for k in 0..4 {
            let mut alls = Vec::new();
            for w in &mut ws {
                let mut all = Clauses::in_block(&block);
                for cl in &w.window {
                    all.push(cl);
                }
                for cl in &w.rest {
                    all.push(cl);
                }
                for _ in 0..rng.below(6) {
                    all.push(&[((rng.below(w.vars as u64) + 1) as i32) * if rng.below(2) == 0 { 1 } else { -1 }]);
                }
                if w.start.is_none() {
                    w.start = Some(start(w.wvars, &w.g, w.base, &w.fan, &all, w.window.len()));
                }
                alls.push(all);
            }
            let formulas: Vec<Formula> = ws
                .iter()
                .zip(&alls)
                .map(|(w, all)| {
                    let r = Resume { start: w.start.as_ref().expect("made"), window_clauses: w.window.len(), carried: &w.learned, assume: (w.vars + 1) as i32, carry_vars: w.wvars };
                    Formula { resume: Some(r), ..Formula::new(w.vars + 1, &w.g, w.base, &w.fan, (w.base < w.g.len()).then_some(&w.own), all, 100_000) }
                })
                .collect();
            let got = solve_in(&formulas, 1 << 20).unwrap_or_else(|e| panic!("{}: {e}", rerail_gpu::device::backend()));
            let here: Vec<Solved> = formulas.iter().map(crate::bitlex_csat_lanes::solve).collect();
            // Through the service, from many threads at once: each the same, the device's or the CPU's.
            let service = Service::default();
            let served: Vec<Solved> = std::thread::scope(|scope| {
                let d = scope.spawn(|| service.run());
                let got = std::thread::scope(|inner| {
                    let hs: Vec<_> = formulas.chunks(8).map(|fs| inner.spawn(|| fs.iter().map(|f| service.solve(f)).collect::<Vec<_>>())).collect();
                    hs.into_iter().flat_map(|h| h.join().expect("a solving thread")).collect()
                });
                service.close();
                d.join().expect("the device's thread").unwrap_or_else(|e| panic!("{}: {e}", rerail_gpu::device::backend()));
                got
            });
            for (i, ((f, x), h)) in formulas.iter().zip(&served).zip(&here).enumerate() {
                checked(1000 * k + i, f, x, h);
            }
            for (i, ((f, x), h)) in formulas.iter().zip(&got).zip(&here).enumerate() {
                checked(1000 * k + i, f, x, h);
                match x.answer {
                    Answer::Sat(_) => sat += 1,
                    Answer::Unsat => unsat += 1,
                    Answer::Unknown => {}
                }
                searched += usize::from(x.conflicts > 0);
            }
            drop(formulas);
            for (w, x) in ws.iter_mut().zip(&got) {
                for c in &x.carry {
                    w.learned.push(c);
                    carried += 1;
                }
            }
        }
        eprintln!("{sat} satisfiable, {unsat} unsatisfiable, {searched} with conflicts, {carried} clauses carried");
        assert!(sat >= 100 && unsat >= 100 && searched >= 40 && carried >= 50, "{sat} sat, {unsat} unsat, {searched} searched, {carried} carried");
    }

    /// With no conflict to spend, the device is unsatisfiable exactly where the CPU's solver is with none spent (a
    /// conflict at level 0): the prover's rounds rest on it (decision 0654, its kernel's test ported).
    #[test]
    fn with_no_conflicts_the_device_is_level_0s_propagation() {
        if rerail_gpu::shared::get().is_none() {
            eprintln!("no device: the prover's GPU solver test did not run");
            return;
        }
        let block = FormulaBlock::new();
        let mut rng = Rng(0x9e3779b97f4a7c15);
        let made: Vec<_> = (0..400usize).map(|i| {
            let inputs = 4 + rng.below(40) as usize;
            let (g, c) = (rng.below(120) as usize, rng.below(2 * inputs as u64 + 4) as usize);
            let own = if i % 3 == 0 { rng.below(10) as usize } else { 0 };
            circuit(&mut rng, &block, inputs, g, c, own)
        }).collect();
        let fans: Vec<(Fanout, Fanout)> = made.iter().map(|(g, _, vars, base)| (Fanout::of(g, *base, *vars), Fanout::of_range(g, *base..g.len(), *vars))).collect();
        let formulas: Vec<Formula> = made
            .iter()
            .zip(&fans)
            .map(|((g, c, vars, base), (fan, own))| Formula::new(*vars, g, *base, fan, (*base < g.len()).then_some(own), c, 0))
            .collect();
        let got = solve(&formulas).unwrap_or_else(|e| panic!("{}: {e}", rerail_gpu::device::backend()));
        for (i, (f, x)) in formulas.iter().zip(&got).enumerate() {
            checked(i, f, x, &crate::bitlex_csat_lanes::solve(f));
        }
        let (mut at_0, mut searched) = (0, 0);
        let mut s = Solver::default();
        for (i, (((g, c, vars, base), (fan, _)), x)) in made.iter().zip(&fans).zip(&got).enumerate() {
            s.reset(*vars);
            for cl in c.iter() {
                s.add(cl);
            }
            let before = s.conflicts;
            let cpu = s.solve(g, fan, *base, 100_000);
            let cpu_at_0 = cpu == CpuAnswer::Unsat && s.conflicts == before;
            assert_eq!(x.answer == Answer::Unsat, cpu_at_0, "formula {i}: the device {:?}, the CPU {cpu:?} after {} conflicts", x.answer, s.conflicts - before);
            assert_eq!(x.conflicts, 0, "formula {i}");
            at_0 += usize::from(cpu_at_0);
            searched += usize::from(s.conflicts > before);
        }
        assert!(at_0 >= 100 && searched >= 40, "{at_0} unsatisfiable at level 0, {searched} searched");
    }

    /// The formulas in `RERAIL_CSAT_DUMPS` (`Gates::dump`s), solved on the device together and on the CPU: each one's
    /// answers, conflicts and time (decision 0670; a tool, so ignored).
    #[test]
    #[ignore]
    fn the_dumped_formulas_on_the_device() {
        let Some(dir) = std::env::var_os("RERAIL_CSAT_DUMPS") else { return };
        let mut paths: Vec<_> = std::fs::read_dir(dir).expect("the dumps").map(|e| e.expect("a dump").path()).collect();
        paths.sort();
        // `RERAIL_CSAT_ONLY`: one dump, by name; `RERAIL_CSAT_COPIES`: each dump that many times in the batch (what
        // a batch of searches costs, not one's latency).
        if let Some(only) = std::env::var_os("RERAIL_CSAT_ONLY") {
            paths.retain(|p| p.file_name() == Some(only.as_os_str()));
        }
        let copies: usize = std::env::var("RERAIL_CSAT_COPIES").ok().and_then(|c| c.parse().ok()).unwrap_or(1);
        let paths: Vec<_> = paths.iter().flat_map(|p| std::iter::repeat_n(p.clone(), copies)).collect();
        let block = FormulaBlock::new();
        let made: Vec<(Gates, usize, usize, Clauses)> = paths
            .iter()
            .map(|p| Gates::undump_into(&std::fs::read_to_string(p).expect("a dump"), Gates::in_block(&block), Clauses::in_block(&block)))
            .collect();
        let fans: Vec<(Fanout, Fanout)> = made.iter().map(|(g, vars, base, _)| (Fanout::of(g, *base, *vars), Fanout::of_range(g, *base..g.len(), *vars))).collect();
        let formulas: Vec<Formula> = made
            .iter()
            .zip(&fans)
            .map(|((g, vars, base, c), (fan, own))| Formula::new(*vars, g, *base, fan, (*base < g.len()).then_some(own), c, 100_000))
            .collect();
        let t = std::time::Instant::now();
        let bytes = std::env::var("RERAIL_CSAT_STATE_MB").ok().and_then(|m| m.parse::<usize>().ok()).map_or(STATE_BYTES, |m| m << 20);
        let got = solve_in(&formulas, bytes).unwrap_or_else(|e| panic!("{}: {e}", rerail_gpu::device::backend()));
        eprintln!("the device: {} formulas, {:.1} ms", formulas.len(), t.elapsed().as_secs_f64() * 1e3);
        let mut s = Solver::default();
        for ((p, (g, vars, base, c)), ((x, (fan, _)), f)) in paths.iter().zip(&made).zip(got.iter().zip(&fans).zip(&formulas)).step_by(copies) {
            let t = std::time::Instant::now();
            let twin = crate::bitlex_csat_lanes::solve(f);
            eprintln!("the device's search on the CPU: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
            checked(0, f, x, &twin);
            let t = std::time::Instant::now();
            s.reset(*vars);
            for cl in c.iter() {
                s.add(cl);
            }
            let c0 = s.conflicts;
            let cpu = s.solve(g, fan, *base, 100_000);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            let answer = match &x.answer {
                Answer::Sat(_) => "Sat",
                Answer::Unsat => "Unsat",
                Answer::Unknown => "Unknown",
            };
            eprintln!(
                "{}: the device {answer}{}, {} conflicts, {} decisions, {} rounds, {} learned ({} literals), {} watch visits, {} gate items, {} reductions, Mcycles visit/items/analyze/back/decide/reduce {:?}; the CPU {cpu:?}, {} conflicts, {ms:.1} ms",
                p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
                if x.roomless { " (room full)" } else { "" },
                x.conflicts,
                x.decisions,
                x.rounds,
                x.learned,
                x.learned_literals,
                x.visits,
                x.items,
                x.reductions,
                x.cycles.map(|c| u64::from(c) * 256 / 1_000_000),
                s.conflicts - c0
            );
        }
    }
}
