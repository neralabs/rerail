//! Array programs on the device (decision 0422): `rerail_compiler::array`'s operations as
//! launches of `array_program.metal`'s kernels, in program order, one command buffer between
//! the host's reads. The interpreter (`ArrayProgram::values_at`) is the meaning: a run here
//! gives its outputs, and its refusal at the same operation.
//!
//! What the host does between launches:
//! - **Lengths.** Every array's length is the host's. An operation whose length is its data's (a
//!   `compact`'s, an `expand`'s, a scatter's `n`) ends the command buffer and reads the count:
//!   a sync. Scalars made of constants, parameters and lengths are known without one.
//! - **Calls.** A `call`'s elements are read back, answered by `Calls`, and their rows sent:
//!   the host's evaluation, counted (`DeviceRun::calls`), until the elements' readings are the
//!   device's (0408's lockstep batch).
//! - **Fixed points.** A group's rounds compare each carry with its next on the device, and the
//!   host reads the verdict.
//!
//! - **Executions.** Each command buffer is one execution: the execution lock and the launch
//!   ledger's scope (`runtime::metal_execution_guard`) are taken when it is begun and let go when
//!   it is done, and a buffer at the ledger's bound (`KERNEL_LAUNCH_LIMIT`) is run and the next
//!   begun. The host's work between buffers (calls, counts) holds neither.
//!
//! Every array is a column of `i64` in shared memory, so a read is a load once the device is
//! done. Errors: the first failing operation's index and reason, kept by the kernels and read at
//! each sync.

use crate::Error;
use metal::{ComputePipelineState, MTLSize};
use rerail_compiler::array::{ArrayOp, ArrayProgram, Calls, Combine, MAX_ROUNDS, MapOp};
use std::collections::HashMap;
use std::rc::Rc;

const SOURCE: &str = include_str!("array_program.metal");
const TILE: usize = 256;

/// The kernels' reasons, by code (the interpreter's words).
const REASONS: [&str; 5] = ["", "a gather's index outside its array", "two writes of a set scatter to one element", "a search of an array not sorted", "a negative count"];

/// A launch's parameters (`L` in the kernels).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Launch {
    n: u32,
    la: u32,
    lb: u32,
    lc: u32,
    op: i32,
    flag: i32,
    at: u32,
    tiles: u32,
    c: i64,
    d: i64,
}

/// An array on the device: its block (shared memory) and length; a scalar's value where the
/// host knows it.
#[derive(Clone)]
struct Arr {
    block: Rc<crate::pool::Block>,
    len: usize,
    known: Option<i64>,
}

/// A run's account.
#[derive(Clone, Debug, Default)]
pub struct DeviceRun {
    pub outputs: Vec<Vec<i64>>,
    pub gpu_seconds: f64,
    pub dispatches: usize,
    /// Command buffers ended to read a count, a verdict or a call's elements.
    pub syncs: usize,
    /// Elements the host answered (`call`).
    pub calls: usize,
}

struct Gpu {
    queue: metal::CommandQueue,
    pipelines: HashMap<&'static str, ComputePipelineState>,
    command: Option<metal::CommandBuffer>,
    encoder: Option<metal::ComputeCommandEncoder>,
    err: Rc<crate::pool::Block>,
    /// The blocks the launches not yet run read or write: a block goes back to the pool only
    /// once its command buffer is done (the pool's next lease writes it on the host).
    held: Vec<Rc<crate::pool::Block>>,
    /// The execution lock and launch ledger of the command buffer being encoded, and its launches.
    guard: Option<crate::runtime::MetalExecutionGuard>,
    launched: usize,
    run: DeviceRun,
}

impl Gpu {
    fn pipeline(&mut self, name: &'static str) -> Result<ComputePipelineState, Error> {
        if let Some(p) = self.pipelines.get(name) {
            return Ok(p.clone());
        }
        let p = crate::runtime::pipeline_named(SOURCE, name)?;
        self.pipelines.insert(name, p.clone());
        Ok(p)
    }

    /// `name` over `threads` threads (tiles of 256), its buffers then its parameters.
    fn launch(&mut self, name: &'static str, buffers: &[&Rc<crate::pool::Block>], p: Launch, threads: usize) -> Result<(), Error> {
        if threads == 0 {
            return Ok(());
        }
        let pipeline = self.pipeline(name)?;
        if self.launched == crate::launch_budget::KERNEL_LAUNCH_LIMIT {
            // A command buffer is an execution: one at the ledger's bound is run, and the next begun.
            self.sync()?;
        }
        if self.command.is_none() {
            self.guard = Some(crate::runtime::metal_execution_guard()?);
            let command = self.queue.new_command_buffer().to_owned();
            command.set_label("Rerail array program");
            let encoder = command.new_compute_command_encoder().to_owned();
            self.command = Some(command);
            self.encoder = Some(encoder);
        }
        let (command, encoder) = (self.command.as_ref().unwrap(), self.encoder.as_ref().unwrap());
        encoder.set_compute_pipeline_state(&pipeline);
        for (k, b) in buffers.iter().enumerate() {
            encoder.set_buffer(k as u64, Some(b.buffer()), 0);
        }
        self.held.extend(buffers.iter().map(|&b| b.clone()));
        encoder.set_bytes(buffers.len() as u64, size_of::<Launch>() as u64, (&raw const p).cast());
        use crate::launch::CheckedComputeEncoder;
        encoder.checked_dispatch_thread_groups(command, MTLSize::new(threads.div_ceil(TILE) as u64, 1, 1), MTLSize::new(TILE as u64, 1, 1));
        self.run.dispatches += 1;
        self.launched += 1;
        Ok(())
    }

    /// The launches so far run; the first error they found.
    fn sync(&mut self) -> Result<Option<(usize, &'static str)>, Error> {
        if let (Some(command), Some(encoder)) = (self.command.take(), self.encoder.take()) {
            encoder.end_encoding();
            if let Err(e) = crate::launch::commit_checked(&command) {
                crate::launch::release_refused(&command);
                return Err(e);
            }
            crate::launch::wait_checked(&command)?;
            self.run.gpu_seconds += crate::launch::gpu_seconds(&command);
            self.run.syncs += 1;
        }
        self.held.clear();
        (self.guard, self.launched) = (None, 0);
        let e = self.err.read::<u32>(1)[0];
        Ok((e != u32::MAX).then(|| ((e >> 8) as usize, REASONS.get((e & 255) as usize).copied().unwrap_or("a device error"))))
    }

    fn new(&mut self, len: usize) -> Result<Arr, Error> {
        let block = Rc::new(crate::pool::zeroed::<i64>(len.max(1))?);
        Ok(Arr { block, len, known: None })
    }

    fn upload(&mut self, v: &[i64]) -> Result<Arr, Error> {
        let block = if v.is_empty() { crate::pool::zeroed::<i64>(1)? } else { crate::pool::upload(v)? };
        Ok(Arr { block: Rc::new(block), len: v.len(), known: (v.len() == 1).then(|| v[0]) })
    }

    /// A block of `count` `u32`s outside the arrays (flags, bit sets).
    fn words(&mut self, count: usize) -> Result<Rc<crate::pool::Block>, Error> {
        Ok(Rc::new(crate::pool::zeroed::<u32>(count)?))
    }

    fn scalar(&mut self, v: i64) -> Result<Arr, Error> {
        self.upload(&[v])
    }

    /// An array's values (the device done with it).
    fn read(&mut self, a: &Arr) -> Vec<i64> {
        a.block.read::<i64>(a.len)
    }

    /// `x` scanned (`op` a combine's code), inclusive, segmented by `seg`.
    fn scan_incl(&mut self, x: &Arr, seg: Option<&Arr>, op: i32, at: u32) -> Result<Arr, Error> {
        let n = x.len;
        let out = self.new(n)?;
        if n == 0 {
            return Ok(out);
        }
        let fl = self.new(n)?;
        let tiles = n.div_ceil(TILE);
        let (agg, aggf) = (self.new(tiles)?, self.new(tiles)?);
        let f = seg.unwrap_or(x);
        let p = Launch { n: n as u32, op, flag: i32::from(seg.is_some()), at, ..Launch::default() };
        self.launch("a_scan_tile", &[&x.block, &f.block, &out.block, &fl.block, &agg.block, &aggf.block], p, tiles * TILE)?;
        if tiles > 1 {
            let aggi = self.scan_incl(&agg, Some(&aggf), op, at)?;
            self.launch("a_scan_fix", &[&out.block, &fl.block, &aggi.block], p, n)?;
        }
        Ok(out)
    }

    fn scan(&mut self, x: &Arr, seg: Option<&Arr>, how: Combine, excl: bool, at: u32) -> Result<Arr, Error> {
        let op = combine_code(how);
        let incl = self.scan_incl(x, seg, op, at)?;
        if !excl {
            return Ok(incl);
        }
        let out = self.new(x.len)?;
        let f = seg.unwrap_or(x);
        self.launch("a_scan_excl", &[&incl.block, &f.block, &out.block], Launch { n: x.len as u32, op, flag: i32::from(seg.is_some()), ..Launch::default() }, x.len)?;
        Ok(out)
    }

    /// The bytes where `key`'s elements differ (a sync).
    fn varying_bytes(&mut self, key: &Arr) -> Result<Vec<u32>, Error> {
        let bits = self.words(2)?;
        self.launch("a_differs", &[&key.block, &bits], Launch { n: key.len as u32, ..Launch::default() }, key.len)?;
        self.sync()?;
        let w = bits.read::<u32>(2);
        let x = u64::from(w[0]) | (u64::from(w[1]) << 32);
        Ok((0..8).filter(|b| (x >> (8 * b)) & 255 != 0).collect())
    }

    /// The stable permutation that sorts by `keys` (the first the most significant).
    fn sort(&mut self, keys: &[&Arr], at: u32) -> Result<Arr, Error> {
        let n = keys[0].len;
        let mut perm = self.new(n)?;
        self.launch("a_iota", &[&perm.block], Launch { n: n as u32, ..Launch::default() }, n)?;
        if n < 2 {
            return Ok(perm);
        }
        let tiles = n.div_ceil(TILE);
        for key in keys.iter().rev() {
            for b in self.varying_bytes(key)? {
                let (ldig, lsrc, lrank, hist) = (self.new(tiles * TILE)?, self.new(tiles * TILE)?, self.new(tiles * TILE)?, self.new(256 * tiles)?);
                let p = Launch { n: n as u32, flag: 8 * b as i32, tiles: tiles as u32, at, ..Launch::default() };
                self.launch("a_radix_tile", &[&key.block, &perm.block, &ldig.block, &lsrc.block, &lrank.block, &hist.block], p, tiles * TILE)?;
                let offs = self.scan(&hist, None, Combine::Add, true, at)?;
                let next = self.new(n)?;
                self.launch("a_radix_scatter", &[&ldig.block, &lsrc.block, &lrank.block, &offs.block, &next.block], p, tiles * TILE)?;
                perm = next;
            }
        }
        Ok(perm)
    }

    fn gather(&mut self, s: &Arr, idx: &Arr, at: u32) -> Result<Arr, Error> {
        let out = self.new(idx.len)?;
        let err = self.err.clone();
        self.launch("a_gather", &[&s.block, &idx.block, &out.block, &err], Launch { n: idx.len as u32, la: s.len as u32, at, ..Launch::default() }, idx.len)?;
        Ok(out)
    }

    /// A scalar's value: known, or read (a sync).
    fn value(&mut self, a: &Arr) -> Result<Result<i64, (usize, &'static str)>, Error> {
        if let Some(v) = a.known {
            return Ok(Ok(v));
        }
        if let Some(e) = self.sync()? {
            return Ok(Err(e));
        }
        Ok(Ok(self.read(a)[0]))
    }
}

fn combine_code(how: Combine) -> i32 {
    match how {
        Combine::Set => 0,
        Combine::Add => 1,
        Combine::Max => 2,
        Combine::Min => 3,
    }
}

fn map_code(op: MapOp) -> i32 {
    MapOp::ALL.iter().position(|(o, _)| *o == op).expect("a map operation") as i32
}

/// The length a broadcast of `xs` makes: each one's that is not 1, all equal.
fn width(xs: &[&Arr]) -> Result<usize, &'static str> {
    let long: Vec<usize> = xs.iter().map(|x| x.len).filter(|&l| l != 1).collect();
    match long.first() {
        None => Ok(1),
        Some(&l) if long.iter().all(|&m| m == l) => Ok(l),
        _ => Err("operands of unequal lengths"),
    }
}

/// `p` run on the device over `inputs` and `params`, its calls answered by `calls`: its
/// outputs, or its refusal (the operation and why), as `ArrayProgram::values_at` refuses.
pub fn run(p: &ArrayProgram, inputs: &[&[i64]], params: &[i64], calls: &mut dyn Calls) -> Result<Result<DeviceRun, (usize, &'static str)>, Error> {
    if inputs.len() != usize::from(p.inputs) || params.len() != usize::from(p.params) {
        return Ok(Err((0, "the run's columns or parameters are not the program's")));
    }
    metal::objc::rc::autoreleasepool(|| run_in(p, inputs, params, calls))
}

fn run_in(p: &ArrayProgram, inputs: &[&[i64]], params: &[i64], calls: &mut dyn Calls) -> Result<Result<DeviceRun, (usize, &'static str)>, Error> {
    let err = Rc::new(crate::pool::upload(&[u32::MAX])?);
    let mut g = Gpu { queue: crate::queue()?, pipelines: HashMap::new(), command: None, encoder: None, err, held: Vec::new(), guard: None, launched: 0, run: DeviceRun::default() };
    let n = p.ops.len();
    let mut v: Vec<Option<Arr>> = vec![None; n];
    let mut parts: Vec<Vec<Arr>> = vec![Vec::new(); n];
    let mut ends: Vec<(usize, &Vec<(u16, u16)>)> = p.rounds.iter().map(|r| (usize::from(r.iter().map(|q| q.1).max().unwrap()), r)).collect();
    ends.sort_unstable_by_key(|e| e.0);
    let mut next_end = 0;
    for i in 0..n {
        if let Err(e) = step(&mut g, p, i, &mut v, &mut parts, inputs, params, calls, None)? {
            return Ok(Err(first(&mut g, e)?));
        }
        while next_end < ends.len() && ends[next_end].0 == i {
            if let Err(e) = settle(&mut g, p, ends[next_end].1, &mut v, &mut parts, inputs, params, calls)? {
                return Ok(Err(first(&mut g, e)?));
            }
            next_end += 1;
        }
    }
    if let Some(e) = g.sync()? {
        return Ok(Err(e));
    }
    g.run.outputs = p.outputs.iter().map(|&o| g.read(v[usize::from(o)].as_ref().unwrap())).collect();
    Ok(Ok(g.run))
}

/// A refusal the host found, or an earlier one the device did.
fn first(g: &mut Gpu, e: (usize, &'static str)) -> Result<(usize, &'static str), Error> {
    Ok(match g.sync()? {
        Some(d) if d.0 <= e.0 => d,
        _ => e,
    })
}

type Step = Result<(), (usize, &'static str)>;

/// A group's rounds, as the interpreter's `settle`: its span's dependents run again until each
/// carry is its next.
#[allow(clippy::too_many_arguments)]
fn settle(g: &mut Gpu, p: &ArrayProgram, group: &[(u16, u16)], v: &mut [Option<Arr>], parts: &mut [Vec<Arr>], inputs: &[&[i64]], params: &[i64], calls: &mut dyn Calls) -> Result<Step, Error> {
    let lo = usize::from(group.iter().map(|q| q.0).min().unwrap());
    let hi = usize::from(group.iter().map(|q| q.1).max().unwrap());
    let mut moved = vec![false; hi + 1];
    for i in lo..=hi {
        moved[i] = group.iter().any(|q| usize::from(q.0) == i) || p.ops[i].operands().iter().any(|&o| usize::from(o) >= lo && moved[usize::from(o)]);
    }
    let mut rounds = 1;
    loop {
        // Whether every carry is its next: lengths on the host, values on the device.
        let flag = g.words(1)?;
        let mut same = true;
        for &(c, x) in group {
            let (c, x) = (v[usize::from(c)].clone().unwrap(), v[usize::from(x)].clone().unwrap());
            if c.len != x.len {
                same = false;
                break;
            }
            g.launch("a_differ", &[&c.block, &x.block, &flag], Launch { n: c.len as u32, ..Launch::default() }, c.len)?;
        }
        if let Some(e) = g.sync()? {
            return Ok(Err(e));
        }
        if same && flag.read::<u32>(1)[0] == 0 {
            return Ok(Ok(()));
        }
        if rounds >= MAX_ROUNDS {
            return Ok(Err((hi, "past the rounds' bound")));
        }
        rounds += 1;
        let nexts: Vec<(usize, Arr)> = group.iter().map(|&(c, x)| (usize::from(c), v[usize::from(x)].clone().unwrap())).collect();
        for i in (lo..=hi).filter(|&i| moved[i]) {
            let carried = nexts.iter().find(|q| q.0 == i).map(|q| q.1.clone());
            if let Err(e) = step(g, p, i, v, parts, inputs, params, calls, carried)? {
                return Ok(Err(e));
            }
        }
    }
}

/// Operation `i` encoded (a carry's value `carried` after the first round).
#[allow(clippy::too_many_arguments)]
fn step(g: &mut Gpu, p: &ArrayProgram, i: usize, v: &mut [Option<Arr>], parts: &mut [Vec<Arr>], inputs: &[&[i64]], params: &[i64], calls: &mut dyn Calls, carried: Option<Arr>) -> Result<Step, Error> {
    let at = i as u32;
    let a = |k: u16| -> Arr { v[usize::from(k)].clone().expect("an earlier value") };
    let fail = |why: &'static str| Ok(Err((i, why)));
    let out = match &p.ops[i] {
        ArrayOp::Input(k) => g.upload(inputs[usize::from(*k)])?,
        ArrayOp::Param(k) => g.scalar(params[usize::from(*k)])?,
        ArrayOp::Const(c) => g.scalar(*c)?,
        ArrayOp::Iota(x) => {
            let len = a(*x).len;
            let out = g.new(len)?;
            g.launch("a_iota", &[&out.block], Launch { n: len as u32, ..Launch::default() }, len)?;
            out
        }
        ArrayOp::Len(x) => g.scalar(a(*x).len as i64)?,
        ArrayOp::Map(op, x, y) => {
            let (x, y) = (a(*x), a(*y));
            let w = match width(&[&x, &y]) {
                Ok(w) => w,
                Err(e) => return fail(e),
            };
            if let (Some(kx), Some(ky), 1) = (x.known, y.known, w) {
                // Scalars the host knows (constants, parameters, lengths): made here, known.
                g.scalar(op.apply(kx, ky))?
            } else {
                let out = g.new(w)?;
                g.launch("a_map", &[&x.block, &y.block, &out.block], Launch { n: w as u32, la: x.len as u32, lb: y.len as u32, op: map_code(*op), ..Launch::default() }, w)?;
                out
            }
        }
        ArrayOp::Select(m, x, y) => {
            let (m, x, y) = (a(*m), a(*x), a(*y));
            let w = match width(&[&m, &x, &y]) {
                Ok(w) => w,
                Err(e) => return fail(e),
            };
            let out = g.new(w)?;
            g.launch("a_select", &[&m.block, &x.block, &y.block, &out.block], Launch { n: w as u32, la: m.len as u32, lb: x.len as u32, lc: y.len as u32, ..Launch::default() }, w)?;
            out
        }
        ArrayOp::Scan { how, excl, a: x, seg } => {
            let x = a(*x);
            let seg = seg.map(a);
            if seg.as_ref().is_some_and(|s| s.len != x.len) {
                return fail("a scan's flags are not as long as its values");
            }
            g.scan(&x, seg.as_ref(), *how, *excl, at)?
        }
        ArrayOp::Sort(keys) => {
            let keys: Vec<Arr> = keys.iter().map(|&k| a(k)).collect();
            if keys.iter().any(|k| k.len != keys[0].len) {
                return fail("a sort's keys of unequal lengths");
            }
            let refs: Vec<&Arr> = keys.iter().collect();
            g.sort(&refs, at)?
        }
        ArrayOp::Gather(s, idx) => {
            let (s, idx) = (a(*s), a(*idx));
            g.gather(&s, &idx, at)?
        }
        ArrayOp::Scatter { how, src, idx, n, init } => {
            let len = match g.value(&a(*n))? {
                Ok(l) => l,
                Err(e) => return Ok(Err(e)),
            };
            let Ok(len) = usize::try_from(len) else { return fail("a scatter's negative length") };
            let (src, idx, init) = (a(*src), a(*idx), a(*init));
            let w = match width(&[&src, &idx]) {
                Ok(w) => w,
                Err(e) => return fail(e),
            };
            if init.len != 1 {
                return fail("a scalar operand is not one element");
            }
            let out = g.new(len)?;
            g.launch("a_fill", &[&out.block, &init.block], Launch { n: len as u32, ..Launch::default() }, len)?;
            // Sorted by index (those outside last), each index's run combined, its last written.
            let key = g.new(w)?;
            g.launch("a_scatter_key", &[&idx.block, &key.block], Launch { n: w as u32, la: idx.len as u32, c: len as i64, ..Launch::default() }, w)?;
            let sv = g.new(w)?;
            g.launch("a_copy", &[&src.block, &sv.block], Launch { n: w as u32, la: src.len as u32, ..Launch::default() }, w)?;
            let perm = g.sort(&[&key], at)?;
            let (sk, ss) = (g.gather(&key, &perm, at)?, g.gather(&sv, &perm, at)?);
            let heads = g.new(w)?;
            let err = g.err.clone();
            let op = combine_code(*how);
            g.launch("a_heads", &[&sk.block, &heads.block, &err], Launch { n: w as u32, op, c: len as i64, at, ..Launch::default() }, w)?;
            let red = g.scan_incl(&ss, Some(&heads), op, at)?;
            g.launch("a_scatter_write", &[&sk.block, &red.block, &out.block, &init.block], Launch { n: w as u32, op, c: len as i64, ..Launch::default() }, w)?;
            out
        }
        ArrayOp::Search { le, sorted, q } => {
            let (s, q) = (a(*sorted), a(*q));
            let err = g.err.clone();
            g.launch("a_sorted", &[&s.block, &err], Launch { n: s.len as u32, at, ..Launch::default() }, s.len)?;
            let out = g.new(q.len)?;
            g.launch("a_search", &[&s.block, &q.block, &out.block], Launch { n: q.len as u32, la: s.len as u32, flag: i32::from(*le), ..Launch::default() }, q.len)?;
            out
        }
        ArrayOp::Compact(m) => {
            let m = a(*m);
            let zero = g.scalar(0)?;
            let bit = g.new(m.len)?;
            g.launch("a_map", &[&m.block, &zero.block, &bit.block], Launch { n: m.len as u32, la: m.len as u32, lb: 1, op: map_code(MapOp::Ne), ..Launch::default() }, m.len)?;
            let incl = g.scan_incl(&bit, None, 1, at)?;
            let count = if m.len == 0 {
                0
            } else {
                if let Some(e) = g.sync()? {
                    return Ok(Err(e));
                }
                incl.block.read::<i64>(m.len)[m.len - 1] as usize
            };
            let out = g.new(count)?;
            g.launch("a_compact", &[&m.block, &incl.block, &out.block], Launch { n: m.len as u32, ..Launch::default() }, m.len)?;
            out
        }
        ArrayOp::Expand(c) => {
            let c = a(*c);
            let err = g.err.clone();
            g.launch("a_nonneg", &[&c.block, &err], Launch { n: c.len as u32, at, ..Launch::default() }, c.len)?;
            let incl = g.scan_incl(&c, None, 1, at)?;
            let total = if c.len == 0 {
                0
            } else {
                if let Some(e) = g.sync()? {
                    return Ok(Err(e));
                }
                incl.block.read::<i64>(c.len)[c.len - 1] as usize
            };
            let out = g.new(total)?;
            g.launch("a_expand", &[&incl.block, &out.block], Launch { n: total as u32, la: c.len as u32, ..Launch::default() }, total)?;
            out
        }
        ArrayOp::Concat(x, y) => {
            let (x, y) = (a(*x), a(*y));
            let out = g.new(x.len + y.len)?;
            g.launch("a_copy", &[&x.block, &out.block], Launch { n: x.len as u32, la: x.len as u32, ..Launch::default() }, x.len)?;
            g.launch("a_copy", &[&y.block, &out.block], Launch { n: y.len as u32, la: y.len as u32, d: x.len as i64, ..Launch::default() }, y.len)?;
            out
        }
        ArrayOp::Call { f, args, env } => {
            // The host's: the elements read, each answered, the rows sent.
            if let Some(e) = g.sync()? {
                return Ok(Err(e));
            }
            let fun = &p.funs[usize::from(*f)];
            let xs: Vec<Vec<i64>> = args.iter().map(|&x| g.read(&a(x))).collect();
            let es: Vec<Vec<i64>> = env.iter().map(|&x| g.read(&a(x))).collect();
            let xr: Vec<&[i64]> = xs.iter().map(Vec::as_slice).collect();
            let er: Vec<&[i64]> = es.iter().map(Vec::as_slice).collect();
            let w = if xr.is_empty() {
                1
            } else {
                let long: Vec<usize> = xr.iter().map(|x| x.len()).filter(|&l| l != 1).collect();
                match long.first() {
                    None => 1,
                    Some(&l) if long.iter().all(|&m| m == l) => l,
                    _ => return fail("operands of unequal lengths"),
                }
            };
            let fields = usize::from(fun.fields);
            let mut counts = Vec::with_capacity(w);
            let mut rows = Vec::new();
            let mut one = Vec::with_capacity(xr.len());
            for j in 0..w {
                one.clear();
                one.extend(xr.iter().map(|x| if x.len() == 1 { x[0] } else { x[j] }));
                let before = rows.len();
                if let Err(e) = calls.call(*f, fun, &one, &er, &mut rows) {
                    return fail(e);
                }
                let made = rows.len() - before;
                if fields == 0 || made % fields != 0 {
                    if fields == 0 && made == 0 {
                        counts.push(0);
                        continue;
                    }
                    return fail("a call's results are not whole rows");
                }
                counts.push((made / fields) as i64);
            }
            g.run.calls += w;
            parts[i] = (0..fields).map(|k| g.upload(&rows.iter().skip(k).step_by(fields).copied().collect::<Vec<_>>())).collect::<Result<_, _>>()?;
            g.upload(&counts)?
        }
        ArrayOp::Part(c, k) => parts[usize::from(*c)][usize::from(*k)].clone(),
        ArrayOp::Carry(init) => carried.unwrap_or_else(|| a(*init)),
    };
    v[i] = Some(out);
    Ok(Ok(()))
}

#[cfg(test)]
#[path = "array_program_tests.rs"]
mod tests;
