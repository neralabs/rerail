//! Array programs on the device (decision 0430): `rerail_compiler::array`'s operations as
//! launches of `array_program.metal`'s kernels, in program order, for a batch of units at once.
//! The interpreter (`ArrayProgram::values_at`) run on each unit alone is the meaning: a unit's
//! run here gives its outputs, or its refusal at the same operation.
//!
//! **A batch.** Each array is its units' arrays end to end, with their offsets (`Dom`) on the
//! host and on the device. A launch serves every unit; a kernel finds an element's unit by a
//! binary search of the offsets, so each unit keeps its own lengths and broadcasts. A unit
//! refused (by a kernel or by the host) stays refused at its first operation; its later arrays
//! are empty where their lengths are the host's to choose, and the batch goes on without it.
//!
//! What the host does between launches:
//! - **Lengths.** Every unit's length of every array is the host's. An operation whose
//!   length is its data's (a `compact`'s, an `expand`'s, a scatter's `n`) ends the command
//!   buffer and reads the counts: a sync, once a batch. Scalars made of constants,
//!   parameters and lengths are known without one.
//! - **Calls.** A `call`'s elements are read back, each unit's answered by its own `Calls`, and
//!   their rows sent: the host's evaluation, counted (`DeviceRun::calls`), until the elements'
//!   readings are the device's (0408's lockstep batch).
//! - **Fixed points.** A group's rounds compare each carry with its next on the device, unit by
//!   unit, and the host reads the verdicts; the batch's rounds go on while a unit's moves.
//! - **Executions.** Each command buffer is one execution: the execution lock and the launch
//!   ledger's scope (`runtime::metal_execution_guard`) are taken when it is begun and let go when
//!   it is done, and a buffer at the ledger's bound (`KERNEL_LAUNCH_LIMIT`) is run and the next
//!   begun. The host's work between buffers (calls, counts) holds neither.
//!
//! Every array is a column of `i64` in shared memory, so a read is a load once the device is
//! done. A value is let go after its last use, and a block goes back to the pool only once the
//! launches that read it are done.

use crate::Error;
use crate::pool::Block;
use metal::{ComputePipelineState, MTLSize};
use rerail_compiler::array::{ArrayOp, ArrayProgram, Calls, Combine, Each, MAX_ROUNDS, MapOp, UnitCall, UnitCalls};
use std::collections::{BTreeMap, HashMap};
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
    nb: u32,
    la: u32,
    lb: u32,
    op: i32,
    flag: i32,
    at: u32,
    tiles: u32,
    c: i64,
    d: i64,
}

/// Where each unit's elements of a batch's array begin, and the end: on the host and on the
/// device.
struct Dom {
    off: Vec<usize>,
    dev: Rc<Block>,
}

impl Dom {
    fn units(&self) -> usize {
        self.off.len() - 1
    }

    fn len(&self, u: usize) -> usize {
        self.off[u + 1] - self.off[u]
    }

    fn total(&self) -> usize {
        *self.off.last().unwrap()
    }
}

/// A batch's array: its block and its units' offsets; each unit's value where every unit has
/// one element and the host knows them.
#[derive(Clone)]
struct Arr {
    block: Rc<Block>,
    dom: Rc<Dom>,
    known: Option<Rc<Vec<i64>>>,
}

/// A batch's account.
#[derive(Clone, Debug, Default)]
pub struct DeviceRun {
    pub gpu_seconds: f64,
    pub dispatches: usize,
    /// Command buffers ended to read a count, a verdict or a call's elements.
    pub syncs: usize,
    /// Elements the host answered (`call`).
    pub calls: usize,
    /// The syncs by what read: a sort's varying bytes, a count, a call's elements, the rounds' verdict.
    pub syncs_by: BTreeMap<&'static str, usize>,
    /// The most bytes the process's device leases held at once during the run (sampled at each
    /// launch): the batch's live arrays, not the pool's idle buffers.
    pub peak_bytes: usize,
    /// The dispatches by kernel.
    pub by_kernel: BTreeMap<&'static str, usize>,
}

/// A unit of a batch: its columns, its parameters, and the host that answers its calls.
pub struct Job<'a> {
    pub inputs: Vec<&'a [i64]>,
    pub params: Vec<i64>,
    pub calls: &'a mut dyn Calls,
}

/// A unit's columns and parameters, its calls answered by the batch's `UnitCalls`.
pub struct Columns<'a> {
    pub inputs: Vec<&'a [i64]>,
    pub params: Vec<i64>,
}

/// A unit's outputs, or its refusal: the operation and why.
pub type Outcome = Result<Vec<Vec<i64>>, (usize, &'static str)>;

type Refusal = (usize, &'static str);

struct Gpu {
    queue: metal::CommandQueue,
    pipelines: HashMap<&'static str, ComputePipelineState>,
    command: Option<metal::CommandBuffer>,
    encoder: Option<metal::ComputeCommandEncoder>,
    /// Each unit's first error the kernels found: the operation and the reason's code.
    err: Rc<Block>,
    /// Each unit's refusal so far, the host's and the kernels' at the last sync.
    dead: Vec<Option<Refusal>>,
    nb: usize,
    /// Every unit one element (scalars).
    ones: Rc<Dom>,
    /// The blocks the launches not yet run read or write: a block goes back to the pool only
    /// once its command buffer is done (the pool's next lease writes it on the host).
    held: Vec<Rc<Block>>,
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
    fn launch(&mut self, name: &'static str, buffers: &[&Rc<Block>], p: Launch, threads: usize) -> Result<(), Error> {
        if threads == 0 {
            return Ok(());
        }
        let pipeline = self.pipeline(name)?;
        if self.launched == crate::launch_budget::KERNEL_LAUNCH_LIMIT {
            // A command buffer is an execution: one at the ledger's bound is run, and the next begun.
            self.sync("bound")?;
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
        encoder.set_bytes(buffers.len() as u64, size_of::<Launch>() as u64, (&raw const p).cast());
        use crate::launch::CheckedComputeEncoder;
        encoder.checked_dispatch_thread_groups(command, MTLSize::new(threads.div_ceil(TILE) as u64, 1, 1), MTLSize::new(TILE as u64, 1, 1));
        self.held.extend(buffers.iter().map(|&b| b.clone()));
        self.run.peak_bytes = self.run.peak_bytes.max(rerail_accel::gpu::shared::held_bytes().0);
        self.run.dispatches += 1;
        *self.run.by_kernel.entry(name).or_default() += 1;
        self.launched += 1;
        Ok(())
    }

    /// The launches so far run; each unit's refusal updated with the kernels' first errors.
    fn sync(&mut self, why: &'static str) -> Result<(), Error> {
        if let (Some(command), Some(encoder)) = (self.command.take(), self.encoder.take()) {
            *self.run.syncs_by.entry(why).or_default() += 1;
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
        for (u, e) in self.err.read::<u32>(self.nb).into_iter().enumerate() {
            if e != u32::MAX {
                let d = ((e >> 8) as usize, REASONS.get((e & 255) as usize).copied().unwrap_or("a device error"));
                self.kill(u, d);
            }
        }
        Ok(())
    }

    /// Unit `u` refused at `r`, unless at an earlier operation already.
    fn kill(&mut self, u: usize, r: Refusal) {
        if self.dead[u].is_none_or(|d| r.0 < d.0) {
            self.dead[u] = Some(r);
        }
    }

    fn block(&mut self, n: usize) -> Result<Rc<Block>, Error> {
        Ok(Rc::new(crate::pool::zeroed::<i64>(n.max(1))?))
    }

    /// Offsets from each unit's length.
    fn dom(&mut self, lens: &[usize]) -> Result<Rc<Dom>, Error> {
        let mut off = Vec::with_capacity(lens.len() + 1);
        off.push(0);
        for &l in lens {
            off.push(off.last().unwrap() + l);
        }
        let words: Vec<i64> = off.iter().map(|&o| o as i64).collect();
        Ok(Rc::new(Dom { off, dev: Rc::new(crate::pool::upload(&words)?) }))
    }

    /// One unit of `n` elements: the batch's internal arrays (a scatter's places, a scan's tiles).
    fn plain(&mut self, n: usize) -> Result<Rc<Dom>, Error> {
        self.dom(&[n])
    }

    /// Each unit's length: `lens`, 0 for a unit refused.
    fn live(&self, mut lens: Vec<usize>) -> Vec<usize> {
        for (u, l) in lens.iter_mut().enumerate() {
            if self.dead[u].is_some() {
                *l = 0;
            }
        }
        lens
    }

    /// The offsets `lens` make: an operand's, where they are the same.
    fn dom_like(&mut self, lens: &[usize], like: &[&Arr]) -> Result<Rc<Dom>, Error> {
        for x in like {
            if x.dom.units() == lens.len() && (0..lens.len()).all(|u| x.dom.len(u) == lens[u]) {
                return Ok(x.dom.clone());
            }
        }
        self.dom(lens)
    }

    fn new(&mut self, dom: Rc<Dom>) -> Result<Arr, Error> {
        Ok(Arr { block: self.block(dom.total())?, dom, known: None })
    }

    /// Each unit's values, end to end.
    fn upload(&mut self, units: &[&[i64]]) -> Result<Arr, Error> {
        let lens: Vec<usize> = units.iter().map(|v| v.len()).collect();
        let dom = if lens.iter().all(|&l| l == 1) { self.ones.clone() } else { self.dom(&lens)? };
        let all: Vec<i64> = units.concat();
        let block = if all.is_empty() { crate::pool::zeroed::<i64>(1)? } else { crate::pool::upload(&all)? };
        let known = Rc::ptr_eq(&dom, &self.ones).then(|| Rc::new(all));
        Ok(Arr { block: Rc::new(block), dom, known })
    }

    /// A scalar for each unit, known.
    fn scalars(&mut self, v: Vec<i64>) -> Result<Arr, Error> {
        let block = crate::pool::upload(&v)?;
        Ok(Arr { block: Rc::new(block), dom: self.ones.clone(), known: Some(Rc::new(v)) })
    }

    /// An array's values (the device done with it).
    fn read(&mut self, a: &Arr) -> Vec<i64> {
        a.block.read::<i64>(a.dom.total())
    }

    /// Each unit's length of the broadcast of `xs` (each one's length that is not 1, all
    /// equal); a unit whose lengths differ refused at `i`.
    fn width(&mut self, i: usize, xs: &[&Arr]) -> Result<Rc<Dom>, Error> {
        if xs.iter().all(|x| Rc::ptr_eq(&x.dom, &xs[0].dom)) {
            return Ok(xs[0].dom.clone());
        }
        let mut lens = vec![0; self.nb];
        for (u, l) in lens.iter_mut().enumerate() {
            let mut w = None;
            let mut bad = false;
            for x in xs {
                let n = x.dom.len(u);
                if n != 1 {
                    bad |= w.is_some_and(|w| w != n);
                    w = Some(n);
                }
            }
            if bad {
                self.kill(u, (i, "operands of unequal lengths"));
            }
            *l = w.unwrap_or(1);
        }
        let lens = self.live(lens);
        self.dom_like(&lens, xs)
    }

    /// `x` scanned (`op` a combine's code), inclusive, segmented by `f` and by `dom`'s units.
    fn scan_incl(&mut self, x: &Rc<Block>, f: Option<&Rc<Block>>, dom: &Rc<Dom>, op: i32, at: u32) -> Result<Rc<Block>, Error> {
        let n = dom.total();
        let out = self.block(n)?;
        if n == 0 {
            return Ok(out);
        }
        let fl = self.block(n)?;
        let tiles = n.div_ceil(TILE);
        let (agg, aggf) = (self.block(tiles)?, self.block(tiles)?);
        let p = Launch { n: n as u32, nb: dom.units() as u32, op, flag: i32::from(f.is_some()), at, ..Launch::default() };
        self.launch("a_scan_tile", &[x, f.unwrap_or(x), &dom.dev, &out, &fl, &agg, &aggf], p, tiles * TILE)?;
        if tiles > 1 {
            let plain = self.plain(tiles)?;
            let aggi = self.scan_incl(&agg, Some(&aggf), &plain, op, at)?;
            self.launch("a_scan_fix", &[&out, &fl, &aggi], p, n)?;
        }
        Ok(out)
    }

    fn scan_excl(&mut self, x: &Rc<Block>, f: Option<&Rc<Block>>, dom: &Rc<Dom>, op: i32, at: u32) -> Result<Rc<Block>, Error> {
        let incl = self.scan_incl(x, f, dom, op, at)?;
        let n = dom.total();
        let out = self.block(n)?;
        let p = Launch { n: n as u32, nb: dom.units() as u32, op, flag: i32::from(f.is_some()), ..Launch::default() };
        self.launch("a_scan_excl", &[&incl, f.unwrap_or(x), &dom.dev, &out], p, n)?;
        Ok(out)
    }

    /// The bytes where `key`'s `n` elements differ (a sync).
    fn varying_bytes(&mut self, key: &Rc<Block>, n: usize) -> Result<Vec<u32>, Error> {
        let bits = Rc::new(crate::pool::zeroed::<u32>(2)?);
        self.launch("a_differs", &[key, &bits], Launch { n: n as u32, ..Launch::default() }, n)?;
        self.sync("sort bytes")?;
        let w = bits.read::<u32>(2);
        let x = u64::from(w[0]) | (u64::from(w[1]) << 32);
        Ok((0..8).filter(|b| (x >> (8 * b)) & 255 != 0).collect())
    }

    /// The stable permutation that sorts each unit of `dom` by `keys` (the first the most
    /// significant), as indexes in the unit.
    fn sort(&mut self, keys: &[Rc<Block>], dom: &Rc<Dom>, at: u32) -> Result<Rc<Block>, Error> {
        let n = dom.total();
        let plain = self.plain(n)?;
        let mut perm = self.block(n)?;
        self.launch("a_iota", &[&perm, &plain.dev], Launch { n: n as u32, nb: 1, ..Launch::default() }, n)?;
        if n < 2 {
            return Ok(perm);
        }
        let mut keys = keys.to_vec();
        if dom.units() > 1 {
            let unit = self.block(n)?;
            self.launch("a_unit", &[&unit, &dom.dev], Launch { n: n as u32, nb: dom.units() as u32, ..Launch::default() }, n)?;
            keys.insert(0, unit);
        }
        let tiles = n.div_ceil(TILE);
        let hist_dom = self.plain(256 * tiles)?;
        for key in keys.iter().rev() {
            for b in self.varying_bytes(key, n)? {
                let (ldig, lsrc, lrank, hist) = (self.block(tiles * TILE)?, self.block(tiles * TILE)?, self.block(tiles * TILE)?, self.block(256 * tiles)?);
                let p = Launch { n: n as u32, flag: 8 * b as i32, tiles: tiles as u32, at, ..Launch::default() };
                self.launch("a_radix_tile", &[key, &perm, &ldig, &lsrc, &lrank, &hist], p, tiles * TILE)?;
                let offs = self.scan_excl(&hist, None, &hist_dom, 1, at)?;
                let next = self.block(n)?;
                self.launch("a_radix_scatter", &[&ldig, &lsrc, &lrank, &offs, &next], p, tiles * TILE)?;
                perm = next;
            }
        }
        if dom.units() > 1 {
            self.launch("a_localize", &[&perm, &dom.dev], Launch { n: n as u32, nb: dom.units() as u32, ..Launch::default() }, n)?;
        }
        Ok(perm)
    }

    fn gather(&mut self, s: &Arr, idx: &Arr, at: u32) -> Result<Arr, Error> {
        let out = self.new(idx.dom.clone())?;
        let err = self.err.clone();
        let n = idx.dom.total();
        self.launch("a_gather", &[&s.block, &s.dom.dev, &idx.block, &out.block, &idx.dom.dev, &err], Launch { n: n as u32, nb: idx.dom.units() as u32, at, ..Launch::default() }, n)?;
        Ok(out)
    }

    /// `x` over `dom`'s lengths (broadcast where its unit has one element).
    fn bcast(&mut self, x: &Arr, dom: &Rc<Dom>) -> Result<Arr, Error> {
        if Rc::ptr_eq(&x.dom, dom) {
            return Ok(x.clone());
        }
        let out = self.new(dom.clone())?;
        let n = dom.total();
        self.launch("a_bcast", &[&x.block, &x.dom.dev, &out.block, &dom.dev], Launch { n: n as u32, nb: dom.units() as u32, ..Launch::default() }, n)?;
        Ok(out)
    }

    /// Each unit's scalar: known, or read (a sync); a unit whose value is not one element
    /// refused at `i`.
    fn values(&mut self, i: usize, a: &Arr) -> Result<Vec<i64>, Error> {
        if let Some(v) = &a.known {
            return Ok(v.to_vec());
        }
        self.sync("scalar")?;
        let all = self.read(a);
        let mut v = vec![0; self.nb];
        for (u, x) in v.iter_mut().enumerate() {
            if a.dom.len(u) == 1 {
                *x = all[a.dom.off[u]];
            } else {
                self.kill(u, (i, "a scalar operand is not one element"));
            }
        }
        Ok(v)
    }

    /// Each unit's last element of inclusive counts `incl` over `dom` (a sync): its total.
    fn totals(&mut self, incl: &Rc<Block>, dom: &Dom, why: &'static str) -> Result<Vec<usize>, Error> {
        self.sync(why)?;
        let all = incl.read::<i64>(dom.total().max(1));
        let ends: Vec<i64> = (0..dom.units()).map(|u| if dom.off[u + 1] == 0 { 0 } else { all[dom.off[u + 1] - 1] }).collect();
        Ok((0..dom.units()).map(|u| (ends[u] - if u == 0 { 0 } else { ends[u - 1] }) as usize).collect())
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

/// `p` run on the device over `inputs` and `params`, its calls answered by `calls`: its
/// outputs, or its refusal (the operation and why), as `ArrayProgram::values_at` refuses.
pub fn run(p: &ArrayProgram, inputs: &[&[i64]], params: &[i64], calls: &mut dyn Calls) -> Result<(Outcome, DeviceRun), Error> {
    let mut jobs = [Job { inputs: inputs.to_vec(), params: params.to_vec(), calls }];
    let (mut out, run) = run_batch(p, &mut jobs)?;
    Ok((out.pop().unwrap(), run))
}

/// `p` run on the device for each of `jobs` at once, in the same launches: each one's outcome,
/// as `run` gives it alone.
pub fn run_batch(p: &ArrayProgram, jobs: &mut [Job<'_>]) -> Result<(Vec<Outcome>, DeviceRun), Error> {
    let (cols, mut calls) = split(jobs);
    run_units(p, &cols, &mut calls)
}

/// A batch's columns, and its hosts as `Each`.
fn split<'j>(jobs: &'j mut [Job<'_>]) -> (Vec<Columns<'j>>, Each<'j>) {
    let mut cols = Vec::with_capacity(jobs.len());
    let mut calls: Vec<&'j mut dyn Calls> = Vec::with_capacity(jobs.len());
    for j in jobs.iter_mut() {
        cols.push(Columns { inputs: j.inputs.clone(), params: j.params.clone() });
        calls.push(&mut *j.calls);
    }
    (cols, Each(calls))
}

/// `run_batch` with every unit's calls answered by one host (`calls`), which may read all the
/// units' elements of a call together.
pub fn run_units(p: &ArrayProgram, units: &[Columns<'_>], calls: &mut dyn UnitCalls) -> Result<(Vec<Outcome>, DeviceRun), Error> {
    let shaped = |j: &Columns| j.inputs.len() == usize::from(p.inputs) && j.params.len() == usize::from(p.params);
    if units.is_empty() {
        return Ok((Vec::new(), DeviceRun::default()));
    }
    if !units.iter().all(shaped) {
        return Ok((units.iter().map(|j| if shaped(j) { Err((0, "a batch with a unit of another shape")) } else { Err((0, "the run's columns or parameters are not the program's")) }).collect(), DeviceRun::default()));
    }
    metal::objc::rc::autoreleasepool(|| run_in(p, units, calls, None))
}

/// Every value of a batch's run, each unit's (`None` where it was refused): a debugging aid,
/// for the first operation whose value is not the interpreter's.
pub fn values_batch(p: &ArrayProgram, jobs: &mut [Job<'_>]) -> Result<Vec<Vec<Option<Vec<i64>>>>, Error> {
    let mut all = Vec::new();
    let (cols, mut calls) = split(jobs);
    metal::objc::rc::autoreleasepool(|| run_in(p, &cols, &mut calls, Some(&mut all)))?;
    Ok(all)
}

fn run_in(p: &ArrayProgram, jobs: &[Columns<'_>], calls: &mut dyn UnitCalls, mut keep: Option<&mut Vec<Vec<Option<Vec<i64>>>>>) -> Result<(Vec<Outcome>, DeviceRun), Error> {
    let nb = jobs.len();
    let err = Rc::new(crate::pool::upload(&vec![u32::MAX; nb])?);
    let ones: Vec<i64> = (0..=nb as i64).collect();
    let ones = Rc::new(Dom { off: (0..=nb).collect(), dev: Rc::new(crate::pool::upload(&ones)?) });
    let mut g = Gpu { queue: crate::queue()?, pipelines: HashMap::new(), command: None, encoder: None, err, dead: vec![None; nb], nb, ones, held: Vec::new(), guard: None, launched: 0, run: DeviceRun::default() };
    let n = p.ops.len();
    // Each value's last use: a use inside a fixed point's span lasts to the span's end.
    let spans: Vec<(usize, usize)> = p.rounds.iter().map(|r| (usize::from(r.iter().map(|q| q.0).min().unwrap()), usize::from(r.iter().map(|q| q.1).max().unwrap()))).collect();
    let mut last = vec![0usize; n];
    for (j, op) in p.ops.iter().enumerate() {
        let until = spans.iter().filter(|s| s.0 <= j && j <= s.1).map(|s| s.1).max().unwrap_or(j);
        for o in op.operands() {
            last[usize::from(o)] = last[usize::from(o)].max(until);
        }
        last[j] = last[j].max(spans.iter().filter(|s| s.0 <= j && j <= s.1).map(|s| s.1).max().unwrap_or(j));
    }
    for &o in &p.outputs {
        last[usize::from(o)] = n;
    }
    if keep.is_some() {
        last.fill(n);
    }
    let mut v: Vec<Option<Arr>> = vec![None; n];
    let mut parts: Vec<Vec<Arr>> = vec![Vec::new(); n];
    let mut ends: Vec<(usize, &Vec<(u16, u16)>)> = p.rounds.iter().map(|r| (usize::from(r.iter().map(|q| q.1).max().unwrap()), r)).collect();
    ends.sort_unstable_by_key(|e| e.0);
    let mut next_end = 0;
    for i in 0..n {
        step(&mut g, p, i, &mut v, &mut parts, jobs, calls, None)?;
        while next_end < ends.len() && ends[next_end].0 == i {
            settle(&mut g, p, ends[next_end].1, &mut v, &mut parts, jobs, calls)?;
            next_end += 1;
        }
        for k in 0..=i {
            if last[k] <= i && v[k].is_some() {
                v[k] = None;
                parts[k].clear();
            }
        }
    }
    g.sync("end")?;
    if let Some(keep) = keep.as_mut() {
        let arrays: Vec<Option<(Vec<i64>, Rc<Dom>)>> = v.iter().map(|a| a.as_ref().map(|a| (g.read(a), a.dom.clone()))).collect();
        **keep = (0..nb).map(|u| arrays.iter().map(|a| a.as_ref().filter(|_| g.dead[u].is_none()).map(|(x, d)| x[d.off[u]..d.off[u + 1]].to_vec())).collect()).collect();
    }
    let outs: Vec<Arr> = p.outputs.iter().map(|&o| v[usize::from(o)].clone().unwrap()).collect();
    let all: Vec<Vec<i64>> = outs.iter().map(|a| g.read(a)).collect();
    let outcomes = (0..nb)
        .map(|u| match g.dead[u] {
            Some(r) => Err(r),
            None => Ok(outs.iter().zip(&all).map(|(a, x)| x[a.dom.off[u]..a.dom.off[u + 1]].to_vec()).collect()),
        })
        .collect();
    Ok((outcomes, g.run))
}

/// A group's rounds, as the interpreter's `settle`: its span's dependents run again until each
/// unit's carries are their nexts (a unit whose carries are already stays so).
fn settle(g: &mut Gpu, p: &ArrayProgram, group: &[(u16, u16)], v: &mut [Option<Arr>], parts: &mut [Vec<Arr>], jobs: &[Columns<'_>], calls: &mut dyn UnitCalls) -> Result<(), Error> {
    let lo = usize::from(group.iter().map(|q| q.0).min().unwrap());
    let hi = usize::from(group.iter().map(|q| q.1).max().unwrap());
    let mut moved = vec![false; hi + 1];
    for i in lo..=hi {
        moved[i] = group.iter().any(|q| usize::from(q.0) == i) || p.ops[i].operands().iter().any(|&o| usize::from(o) >= lo && moved[usize::from(o)]);
    }
    let mut rounds = 1;
    loop {
        // Which units' carries differ from their nexts: lengths on the host, values on the device.
        let flags = Rc::new(crate::pool::zeroed::<u32>(g.nb)?);
        let mut differ = vec![false; g.nb];
        for &(c, x) in group {
            let (c, x) = (v[usize::from(c)].clone().unwrap(), v[usize::from(x)].clone().unwrap());
            for (u, d) in differ.iter_mut().enumerate() {
                *d |= c.dom.len(u) != x.dom.len(u);
            }
            let n = c.dom.total();
            g.launch("a_differ", &[&c.block, &c.dom.dev, &x.block, &x.dom.dev, &flags], Launch { n: n as u32, nb: g.nb as u32, ..Launch::default() }, n)?;
        }
        g.sync("rounds")?;
        let on_device = flags.read::<u32>(g.nb);
        let moving: Vec<usize> = (0..g.nb).filter(|&u| g.dead[u].is_none() && (differ[u] || on_device[u] != 0)).collect();
        if moving.is_empty() {
            return Ok(());
        }
        if rounds >= MAX_ROUNDS {
            for u in moving {
                g.kill(u, (hi, "past the rounds' bound"));
            }
            return Ok(());
        }
        rounds += 1;
        let nexts: Vec<(usize, Arr)> = group.iter().map(|&(c, x)| (usize::from(c), v[usize::from(x)].clone().unwrap())).collect();
        for i in (lo..=hi).filter(|&i| moved[i]) {
            let carried = nexts.iter().find(|q| q.0 == i).map(|q| q.1.clone());
            step(g, p, i, v, parts, jobs, calls, carried)?;
        }
    }
}

/// Operation `i` encoded for the batch (a carry's value `carried` after the first round).
fn step(g: &mut Gpu, p: &ArrayProgram, i: usize, v: &mut [Option<Arr>], parts: &mut [Vec<Arr>], jobs: &[Columns<'_>], calls: &mut dyn UnitCalls, carried: Option<Arr>) -> Result<(), Error> {
    let at = i as u32;
    let nb = g.nb;
    let a = |k: u16| -> Arr { v[usize::from(k)].clone().expect("an earlier value") };
    let out = match &p.ops[i] {
        ArrayOp::Input(k) => {
            let units: Vec<&[i64]> = jobs.iter().map(|j| j.inputs[usize::from(*k)]).collect();
            g.upload(&units)?
        }
        ArrayOp::Param(k) => g.scalars(jobs.iter().map(|j| j.params[usize::from(*k)]).collect())?,
        ArrayOp::Const(c) => g.scalars(vec![*c; nb])?,
        ArrayOp::Iota(x) => {
            let x = a(*x);
            let out = g.new(x.dom.clone())?;
            let n = x.dom.total();
            g.launch("a_iota", &[&out.block, &x.dom.dev], Launch { n: n as u32, nb: nb as u32, ..Launch::default() }, n)?;
            out
        }
        ArrayOp::Len(x) => {
            let x = a(*x);
            g.scalars((0..nb).map(|u| x.dom.len(u) as i64).collect())?
        }
        ArrayOp::Map(op, x, y) => {
            let (x, y) = (a(*x), a(*y));
            let dom = g.width(i, &[&x, &y])?;
            if let (Some(kx), Some(ky), true) = (&x.known, &y.known, Rc::ptr_eq(&dom, &g.ones)) {
                // Scalars the host knows (constants, parameters, lengths): made here, known.
                g.scalars(kx.iter().zip(ky.iter()).map(|(&a, &b)| op.apply(a, b)).collect())?
            } else {
                let out = g.new(dom.clone())?;
                let n = dom.total();
                g.launch("a_map", &[&x.block, &x.dom.dev, &y.block, &y.dom.dev, &out.block, &dom.dev], Launch { n: n as u32, nb: nb as u32, op: map_code(*op), ..Launch::default() }, n)?;
                out
            }
        }
        ArrayOp::Select(m, x, y) => {
            let (m, x, y) = (a(*m), a(*x), a(*y));
            let dom = g.width(i, &[&m, &x, &y])?;
            let out = g.new(dom.clone())?;
            let n = dom.total();
            g.launch("a_select", &[&m.block, &m.dom.dev, &x.block, &x.dom.dev, &y.block, &y.dom.dev, &out.block, &dom.dev], Launch { n: n as u32, nb: nb as u32, ..Launch::default() }, n)?;
            out
        }
        ArrayOp::Scan { how, excl, a: x, seg } => {
            let x = a(*x);
            let f = match seg.map(a) {
                Some(s) => {
                    for u in 0..nb {
                        if s.dom.len(u) != x.dom.len(u) {
                            g.kill(u, (i, "a scan's flags are not as long as its values"));
                        }
                    }
                    Some(g.bcast(&s, &x.dom)?.block)
                }
                None => None,
            };
            let op = combine_code(*how);
            let block = if *excl { g.scan_excl(&x.block, f.as_ref(), &x.dom, op, at)? } else { g.scan_incl(&x.block, f.as_ref(), &x.dom, op, at)? };
            Arr { block, dom: x.dom.clone(), known: None }
        }
        ArrayOp::Sort(keys) => {
            let keys: Vec<Arr> = keys.iter().map(|&k| a(k)).collect();
            let dom = keys[0].dom.clone();
            for u in 0..nb {
                if keys.iter().any(|k| k.dom.len(u) != dom.len(u)) {
                    g.kill(u, (i, "a sort's keys of unequal lengths"));
                }
            }
            let blocks: Vec<Rc<Block>> = keys.iter().map(|k| g.bcast(k, &dom).map(|k| k.block)).collect::<Result<_, _>>()?;
            Arr { block: g.sort(&blocks, &dom, at)?, dom, known: None }
        }
        ArrayOp::Gather(s, idx) => {
            let (s, idx) = (a(*s), a(*idx));
            g.gather(&s, &idx, at)?
        }
        ArrayOp::Scatter { how, src, idx, n, init } => {
            // The host's checks in the interpreter's order: the length, the operands, the initial value.
            let ns = g.values(i, &a(*n))?;
            for (u, &k) in ns.iter().enumerate() {
                if k < 0 {
                    g.kill(u, (i, "a scatter's negative length"));
                }
            }
            let (src, idx, init) = (a(*src), a(*idx), a(*init));
            let w = g.width(i, &[&src, &idx])?;
            for u in 0..nb {
                if init.dom.len(u) != 1 {
                    g.kill(u, (i, "a scalar operand is not one element"));
                }
            }
            let lens = g.live(ns.iter().map(|&k| k.max(0) as usize).collect());
            let dom = g.dom(&lens)?;
            let out = g.new(dom.clone())?;
            let total = dom.total();
            let ones = g.ones.clone();
            let init = g.bcast(&init, &ones)?;
            g.launch("a_fill", &[&out.block, &dom.dev, &init.block, &init.dom.dev], Launch { n: total as u32, nb: nb as u32, ..Launch::default() }, total)?;
            // Sorted by place in the batch's output (those outside last), each place's run
            // combined, its last written.
            let (idx, src) = (g.bcast(&idx, &w)?, g.bcast(&src, &w)?);
            let m = w.total();
            let plain = g.plain(m)?;
            let key = g.block(m)?;
            g.launch("a_scatter_key", &[&idx.block, &w.dev, &dom.dev, &key], Launch { n: m as u32, nb: nb as u32, c: total as i64, ..Launch::default() }, m)?;
            let perm = g.sort(std::slice::from_ref(&key), &plain, at)?;
            let (key, src, perm) = (Arr { block: key, dom: plain.clone(), known: None }, Arr { block: src.block, dom: plain.clone(), known: None }, Arr { block: perm, dom: plain.clone(), known: None });
            let (sk, ss) = (g.gather(&key, &perm, at)?, g.gather(&src, &perm, at)?);
            let heads = g.block(m)?;
            let err = g.err.clone();
            let op = combine_code(*how);
            g.launch("a_heads", &[&sk.block, &heads, &dom.dev, &err], Launch { n: m as u32, nb: nb as u32, op, c: total as i64, at, ..Launch::default() }, m)?;
            let red = g.scan_incl(&ss.block, Some(&heads), &plain, op, at)?;
            g.launch("a_scatter_write", &[&sk.block, &red, &out.block, &dom.dev, &init.block, &init.dom.dev], Launch { n: m as u32, nb: nb as u32, op, c: total as i64, ..Launch::default() }, m)?;
            out
        }
        ArrayOp::Search { le, sorted, q } => {
            let (s, q) = (a(*sorted), a(*q));
            let err = g.err.clone();
            let ns = s.dom.total();
            g.launch("a_sorted", &[&s.block, &s.dom.dev, &err], Launch { n: ns as u32, nb: nb as u32, at, ..Launch::default() }, ns)?;
            let out = g.new(q.dom.clone())?;
            let n = q.dom.total();
            g.launch("a_search", &[&s.block, &s.dom.dev, &q.block, &out.block, &q.dom.dev], Launch { n: n as u32, nb: nb as u32, flag: i32::from(*le), ..Launch::default() }, n)?;
            out
        }
        ArrayOp::Compact(m) => {
            let m = a(*m);
            let zero = g.scalars(vec![0; nb])?;
            let bit = g.new(m.dom.clone())?;
            let n = m.dom.total();
            g.launch("a_map", &[&m.block, &m.dom.dev, &zero.block, &zero.dom.dev, &bit.block, &m.dom.dev], Launch { n: n as u32, nb: nb as u32, op: map_code(MapOp::Ne), ..Launch::default() }, n)?;
            // The batch's inclusive count: each unit's elements at their places in the batch's output.
            let plain = g.plain(n)?;
            let incl = g.scan_incl(&bit.block, None, &plain, 1, at)?;
            let lens = g.totals(&incl, &m.dom, "compact")?;
            let dom = g.dom(&lens)?;
            let out = g.new(dom)?;
            g.launch("a_compact", &[&m.block, &m.dom.dev, &incl, &out.block], Launch { n: n as u32, nb: nb as u32, ..Launch::default() }, n)?;
            out
        }
        ArrayOp::Expand(c) => {
            let c = a(*c);
            let err = g.err.clone();
            let n = c.dom.total();
            let dead: Vec<u32> = g.dead.iter().map(|d| u32::from(d.is_some())).collect();
            let dead = Rc::new(crate::pool::upload(&dead)?);
            let counts = g.block(n)?;
            g.launch("a_counts", &[&c.block, &c.dom.dev, &err, &dead, &counts], Launch { n: n as u32, nb: nb as u32, at, ..Launch::default() }, n)?;
            let plain = g.plain(n)?;
            let incl = g.scan_incl(&counts, None, &plain, 1, at)?;
            let lens = g.totals(&incl, &c.dom, "expand")?;
            let dom = g.dom(&lens)?;
            let out = g.new(dom.clone())?;
            let total = dom.total();
            g.launch("a_expand", &[&incl, &c.dom.dev, &out.block], Launch { n: total as u32, nb: nb as u32, la: n as u32, ..Launch::default() }, total)?;
            out
        }
        ArrayOp::Concat(x, y) => {
            let (x, y) = (a(*x), a(*y));
            let lens: Vec<usize> = (0..nb).map(|u| x.dom.len(u) + y.dom.len(u)).collect();
            let dom = g.dom_like(&lens, &[])?;
            let out = g.new(dom.clone())?;
            let n = dom.total();
            g.launch("a_concat", &[&x.block, &x.dom.dev, &y.block, &y.dom.dev, &out.block, &dom.dev], Launch { n: n as u32, nb: nb as u32, ..Launch::default() }, n)?;
            out
        }
        ArrayOp::Call { f, args, env } => {
            // The host's: the elements read, each unit's answered by its own host, the rows sent.
            g.sync("call")?;
            let fun = &p.funs[usize::from(*f)];
            let fields = usize::from(fun.fields);
            let xs: Vec<Arr> = args.iter().map(|&x| a(x)).collect();
            let es: Vec<Arr> = env.iter().map(|&x| a(x)).collect();
            let xv: Vec<Vec<i64>> = xs.iter().map(|x| g.read(x)).collect();
            let ev: Vec<Vec<i64>> = es.iter().map(|x| g.read(x)).collect();
            let mut counts: Vec<i64> = Vec::new();
            let mut count_lens = vec![0usize; nb];
            let mut cols: Vec<Vec<i64>> = vec![Vec::new(); fields];
            let mut part_lens = vec![0usize; nb];
            let mut rows = Vec::new();
            // Every live unit's elements, then one call for all: the host may read them together.
            let mut asked: Vec<UnitCall> = Vec::new();
            for u in 0..nb {
                if g.dead[u].is_some() {
                    continue;
                }
                let xr: Vec<&[i64]> = xs.iter().zip(&xv).map(|(x, all)| &all[x.dom.off[u]..x.dom.off[u + 1]]).collect();
                let er: Vec<&[i64]> = es.iter().zip(&ev).map(|(x, all)| &all[x.dom.off[u]..x.dom.off[u + 1]]).collect();
                let long: Vec<usize> = xr.iter().map(|x| x.len()).filter(|&l| l != 1).collect();
                let w = match long.first() {
                    _ if xr.is_empty() => 1,
                    None => 1,
                    Some(&l) if long.iter().all(|&m| m == l) => l,
                    _ => {
                        g.kill(u, (i, "operands of unequal lengths"));
                        continue;
                    }
                };
                asked.push(UnitCall { unit: u, args: xr, w, env: er });
            }
            let answers = if asked.is_empty() { Vec::new() } else { calls.call_units(*f, fun, &asked) };
            debug_assert_eq!(answers.len(), asked.len());
            for (c, answer) in asked.iter().zip(answers) {
                let (u, w) = (c.unit, c.w);
                rows.clear();
                let mut mine = Vec::with_capacity(w);
                let mut refused = answer.as_ref().err().copied();
                if let Ok(each) = answer {
                    for made in each {
                        if fields == 0 || made.len() % fields != 0 {
                            if fields == 0 && made.is_empty() {
                                mine.push(0);
                                continue;
                            }
                            refused = Some("a call's results are not whole rows");
                            break;
                        }
                        mine.push((made.len() / fields) as i64);
                        rows.extend(made);
                    }
                }
                if let Some(e) = refused {
                    g.kill(u, (i, e));
                    continue;
                }
                g.run.calls += w;
                count_lens[u] = w;
                counts.extend(mine);
                if fields > 0 {
                    part_lens[u] = rows.len() / fields;
                    for (k, col) in cols.iter_mut().enumerate() {
                        col.extend(rows.iter().skip(k).step_by(fields));
                    }
                }
            }
            let part_dom = g.dom(&part_lens)?;
            parts[i] = cols
                .iter()
                .map(|col| -> Result<Arr, Error> {
                    let block = if col.is_empty() { crate::pool::zeroed::<i64>(1)? } else { crate::pool::upload(col)? };
                    Ok(Arr { block: Rc::new(block), dom: part_dom.clone(), known: None })
                })
                .collect::<Result<_, _>>()?;
            let dom = g.dom(&count_lens)?;
            let block = if counts.is_empty() { crate::pool::zeroed::<i64>(1)? } else { crate::pool::upload(&counts)? };
            Arr { block: Rc::new(block), dom, known: None }
        }
        ArrayOp::Part(c, k) => parts[usize::from(*c)][usize::from(*k)].clone(),
        ArrayOp::Carry(init) => carried.unwrap_or_else(|| a(*init)),
    };
    v[i] = Some(out);
    Ok(())
}

#[cfg(test)]
#[path = "array_program_tests.rs"]
mod tests;
