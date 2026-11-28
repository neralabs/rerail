//! The delta executor's device (decision 0686, E3): `rerail_delta_exec::op::Kernels` on the process's device, any
//! backend (`rerail_gpu::shared`: Metal, CUDA, HIP, OpenCL, or the CPU's, on its vector path with
//! `RERAIL_CPU_EXEC=vec`). The executor pins a launch and drives its version's phases; this crate makes the launch's
//! device descriptor table (E2), hands it to the launch's fence (E1), compiles the version's UKL once a program and
//! runs the launch's phases as one command (`OnDevice::program`): each phase's count kernel sizes its dispatch, which
//! is indirect, with no wait for the host between phases.
//!
//! A version's kernels take the same arguments, by name: `t`, the device table (read with `gens::DESC_READS`);
//! `slots`, the slots' block ids; `p`, the params' bytes; `ph`, the phase (`PhaseP`); `ctl`, the launch's control
//! words; and `meter`, its slots' capacities, needs and fronts (`<ukl/delta/op.h>`). `PRELUDE` comes before the
//! version's source.
//!
//! `Gpu::host` runs in the host's memory domain (each block read in place: unified memory, the CPU's); `Gpu::discrete`
//! in a device domain of its own (1 and up), each block a replica there (`region::DeviceCopy`, placed by
//! `Gpu::replicate`), the table's own block too (`gens::device_table_in`).

#![cfg(any(target_os = "macos", target_os = "linux", windows))]

use rerail_blocks::ledger::Fence;
use rerail_delta_exec::dir::{BlockId, Directory, Launch};
use rerail_delta_exec::op::{DevicePhase, Devices, Instance, Kernels, MAX_SLOTS, Meter, OnDevice};
use rerail_gpu::device::{Buffer, ComputeQueue, Device, alloc_uninitialized_array_sync, transfer_to_host_sync};
use rerail_gpu::kernels::{Program, launch};
use rerail_gpu::shared::Shared;
use std::collections::HashMap;
use std::sync::Arc;

/// What every version's source starts with, after the backend and the descriptor table's reads: a kernel's phase
/// argument (its index, and whether an item is a threadgroup's: `<ukl/delta/op.h>`'s count kernels size the dispatch).
pub const PRELUDE: &str = r#"
typedef struct PhaseP { uint32_t k; uint32_t group; uint32_t w; uint32_t ns; uint32_t ps; uint32_t pad; } PhaseP;
UKL_REFLECT_STRUCT(PhaseP, k, group, w, ns, ps, pad)
"#;

/// A kernel's phase argument: the phase, whether an item is a threadgroup's, and the launch's width: its instances, and
/// each one's slots and params' bytes (`<ukl/delta/op.h>`'s wide launch).
#[repr(C)]
#[derive(Clone, Copy)]
struct PhaseP {
    k: u32,
    group: u32,
    w: u32,
    ns: u32,
    ps: u32,
    pad: u32,
}

// SAFETY: u32s, no padding.
unsafe impl rerail_gpu::Pod for PhaseP {}

/// The phases a launch runs at most, and the control words (`<ukl/delta/op.h>`: `DELTA_PHASES`, `DELTA_CTL_WORDS`:
/// each phase's count, whether it runs, its grid, its turns run).
const PHASES: usize = 32;
const CTL_WORDS: usize = 6 * PHASES;
const TURNS_AT: usize = 5 * PHASES;
/// The meter's words: each slot's capacity, need and front, the refusal and whether it overflowed; then the control
/// words, after it in the launch's one state buffer.
const METER_WORDS: usize = 3 * MAX_SLOTS + 2;
const CTL_AT: usize = METER_WORDS.next_multiple_of(16);
const STATE_WORDS: usize = CTL_AT + CTL_WORDS;
/// A wide launch's instances at most, and each phase's words past every instance's state: its whole grid and the
/// prefix of the instances' grids (`<ukl/delta/op.h>`'s `DELTA_MAX_WIDE`, `DELTA_WIDE_WORDS`).
pub const MAX_WIDE: usize = 32;
const WIDE_WORDS: usize = 4 + MAX_WIDE;
const _: () = assert!(MAX_WIDE == rerail_delta_exec::op::MAX_WIDE, "the executor's and the kernels' widths");
const _: () = assert!(MAX_SLOTS == 30, "the prelude's grid kernel reads the meter at 3 * 30");

/// The threads of a threadgroup a phase of lanes runs in, and of a phase whose item is a threadgroup's
/// (`<ukl/delta/op.h>`'s `DELTA_TG`).
const GROUP: u64 = 64;
const DELTA_TG: u32 = 64;

/// The process's device, as the executor's `Kernels`, in one memory domain.
pub struct Gpu {
    shared: &'static Shared,
    domain: u32,
}

impl Gpu {
    /// The process's device in the host's memory domain (0): each block read where it lies.
    pub fn host() -> Option<Gpu> {
        Some(Gpu { shared: rerail_gpu::shared::get()?, domain: 0 })
    }

    /// The process's device in memory domain `domain` (1 and up) of its own: each block a kernel reads a replica there
    /// (`replicate`).
    pub fn discrete(domain: u32) -> Option<Gpu> {
        assert_ne!(domain, 0, "a device domain of its own (the host's is `host`)");
        Some(Gpu { shared: rerail_gpu::shared::get()?, domain })
    }

    /// Places block `id` in the device's domain: a copy of it in the device's memory (`region::DeviceCopy`), kept by
    /// the directory (`Directory::replicate`), brought to its latest version when a launch there reads it.
    pub fn replicate(&self, dir: &mut Directory, id: BlockId) -> Result<(), String> {
        let b = dir.block(id).ok_or_else(|| format!("a replica of {id}, which nothing holds"))?;
        dir.replicate(id, self.domain, Box::new(rerail_blocks::region::DeviceCopy::new(self.shared, b.reserved())))
    }
}

impl Kernels for Gpu {
    fn devices(&self) -> Devices {
        match self.shared.device {
            Device::Cpu(_) => Devices::CPU,
            Device::OpenCl(_) => Devices::OPENCL,
            Device::Native(_) if cfg!(target_os = "macos") => Devices::METAL,
            Device::Native(_) => Devices::CUDA,
        }
    }

    fn domain(&self) -> u32 {
        self.domain
    }

    fn bind(&self, launch: &Launch, fence: &Arc<Fence>, wide: &[Instance<'_>], rooms: &[Meter]) -> Result<Box<dyn OnDevice + '_>, String> {
        let t = launch.table();
        let pins = launch.pins();
        let ns = wide.first().map_or(0, |w| w.slots.len());
        if wide.is_empty() || wide.len() > MAX_WIDE || wide.iter().any(|w| w.slots.len() != ns) {
            return Err(format!("a wide launch of {} instances (1 to {MAX_WIDE}, each of {ns} slots)", wide.len()));
        }
        // A slot's block wrapped up to its capacity, where its kernels claim past its used bytes (the host's own
        // memory: a device domain of its own has the block's whole reservation).
        if self.domain == 0 {
            // The launch pins its instances' slots first, in order.
            for (k, p) in pins.iter().take(ns * wide.len()).enumerate().filter(|(_, p)| p.writes()) {
                p.block().device_buffer_reaching(&self.shared.device, rooms[k / ns].capacity[k % ns])?;
            }
        }
        let dt = if self.domain == 0 {
            rerail_blocks::gens::device_table(&t, &pins, &self.shared.device)?
        } else {
            rerail_blocks::gens::device_table_in(&t, &pins, self.domain, self.shared)?
        };
        // The kernels bind the table's buffer and reach the blocks' (device handles, which the fence's uses keep).
        let (table, at, reached) = (dt.buffer.clone(), dt.at, dt.reached.clone());
        dt.to_fence(fence, "a version's device table")?;
        // The kernels' arguments, written by the program's command (not block data): each instance's slots' ids and
        // params, end to end, the params at 8-byte strides.
        let ps = wide[0].params.len().next_multiple_of(8);
        let slots: Vec<u32> = wide.iter().flat_map(|w| w.slots.iter().copied()).collect(); // copy: not block data (a launch's arguments)
        let mut params = vec![0u8; ps * wide.len()]; // copy: not block data (a launch's arguments)
        for (k, w) in wide.iter().enumerate() {
            params[ps * k..ps * k + w.params.len()].copy_from_slice(w.params); // copy: not block data (a launch's arguments)
        }
        let words = |n: usize| alloc_uninitialized_array_sync::<u64>(&self.shared.device, n.max(1));
        let slots_b = words(slots.len().div_ceil(2));
        let params_b = words(params.len().div_ceil(8));
        let state = words(state_words(wide.len()).div_ceil(2));
        Ok(Box::new(GpuBound { shared: self.shared, table, at, reached, slots: slots_b, params: params_b, state, wide: (wide.len(), ns, ps), args: (slots, params), programs: HashMap::new() }))
    }
}

/// A wide launch's state of `w` instances: each one's meter and control words, then each phase's wide words.
fn state_words(w: usize) -> usize {
    w * STATE_WORDS + PHASES * WIDE_WORDS
}

fn as_bytes(v: &[u32]) -> &[u8] {
    // SAFETY: u32s are plain bytes, read only.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) }
}

/// A launch bound on the device: its table's buffer, the blocks' buffers it reaches, its arguments and the programs
/// compiled for it (once a source: `rerail_gpu::kernels::program` keeps them for the process).
struct GpuBound {
    shared: &'static Shared,
    table: Buffer,
    at: u64,
    reached: Vec<Buffer>,
    slots: Buffer,
    params: Buffer,
    /// Each instance's meter (each slot's capacity, the room an item found it needs and its front, u32s, `MAX_SLOTS`
    /// each; the refusal; whether it overflowed) and control words, then each phase's wide words: `<ukl/delta/op.h>`'s.
    state: Buffer,
    /// Its instances, each one's slots and params' stride.
    wide: (usize, usize, usize),
    /// The slots' ids and the params' bytes, written into `slots` and `params` by the program's command.
    args: (Vec<u32>, Vec<u8>),
    programs: HashMap<usize, &'static Program>,
}

impl OnDevice for GpuBound {
    fn program(&mut self, source: &'static str, entry: &str, phases: &[DevicePhase], rooms: &mut [Meter]) -> Result<Vec<u32>, String> {
        if phases.len() > PHASES {
            return Err(format!("{entry}: {} phases, past the device's {PHASES}", phases.len()));
        }
        let (w, ns, ps) = self.wide;
        if rooms.len() != w {
            return Err(format!("{entry}: {} meters for {w} instances", rooms.len()));
        }
        let program = match self.programs.get(&(source.as_ptr() as usize)) {
            Some(p) => *p,
            None => {
                let text = format!("#include <ukl/backend.h>\n{}{PRELUDE}{source}", rerail_blocks::gens::DESC_READS);
                let p = rerail_gpu::kernels::program("delta_version.uglykl", &text)?;
                self.programs.insert(source.as_ptr() as usize, p);
                p
            }
        };
        // Each instance's meter: capacities (a block past 4 GiB held to 4 GiB), no need yet, the fronts, no refusal;
        // every phase set to run.
        let mut state = vec![0u32; state_words(w)]; // copy: not block data (the launch's state words)
        for (j, room) in rooms.iter().enumerate() {
            let st = &mut state[j * STATE_WORDS..(j + 1) * STATE_WORDS];
            for (r, &c) in st.iter_mut().zip(&room.capacity) {
                *r = u32::try_from(c).unwrap_or(u32::MAX);
            }
            for (r, &f) in st[2 * MAX_SLOTS..].iter_mut().zip(&room.fronts) {
                *r = u32::try_from(f).map_err(|_| format!("{entry}: a slot's front past 4 GiB"))?;
            }
            for go in &mut st[CTL_AT + PHASES..CTL_AT + 2 * PHASES] {
                *go = 1;
            }
        }
        let began = std::time::Instant::now();
        let mut q = ComputeQueue::new(&self.shared.queue);
        q.enqueue_copy_from_host(&self.state, 0, as_bytes(&state));
        q.enqueue_copy_from_host(&self.slots, 0, as_bytes(&self.args.0));
        if !self.args.1.is_empty() {
            q.enqueue_copy_from_host(&self.params, 0, &self.args.1);
        }
        let (meter_at, ctl_at) = (0u64, 4 * CTL_AT as u64);
        // Each phase: its count kernel (which sizes its dispatch: every instance's grid, end to end), then each turn's
        // dispatch, a round phase's each followed by its `again` kernel (which sizes the next turn's, or none).
        // A measurement: each phase its own command, waited for, its time printed (the same work, in order).
        let split = std::env::var_os("RERAIL_DELTA_SPLIT").is_some();
        for (k, ph) in phases.iter().enumerate() {
            let name = format!("{entry}_{}", ph.name);
            let threads = if ph.group { u64::from(DELTA_TG) } else { GROUP };
            let at = std::time::Instant::now();
            {
                let kernel = |name: &str, k: usize, group: bool| -> Result<_, String> {
                    let mut run = launch(&q, program.function(name)?)
                        .buffer_at("t", &self.table, self.at)
                        .buffer("slots", &self.slots)
                        .buffer("p", &self.params)
                        .value("ph", &PhaseP { k: k as u32, group: u32::from(group), w: w as u32, ns: ns as u32, ps: ps as u32, pad: 0 })
                        .buffer_at("ctl", &self.state, ctl_at)
                        .buffer_at("meter", &self.state, meter_at);
                    for r in &self.reached {
                        run = run.uses(r);
                    }
                    Ok(run)
                };
                let grid = 4 * (w * STATE_WORDS + k * WIDE_WORDS) as u64;
                kernel(&format!("{name}_items"), k, ph.group)?.run(&[1], &[1]).map_err(|e| format!("{name}_items: {e}"))?;
                for _ in 0..ph.turns {
                    kernel(&name, k, ph.group)?.run_indirect(&self.state, grid, &[MAX_GROUPS], &[threads]).map_err(|e| format!("{name}: {e}"))?;
                    if ph.rounds {
                        kernel(&format!("{name}_again"), k, ph.group)?.run(&[1], &[1]).map_err(|e| format!("{name}_again: {e}"))?;
                    }
                }
            }
            if split {
                q.flush().await_().map_err(|e| format!("{name}: {e}"))?;
                eprintln!("delta-phase {name} {} us", at.elapsed().as_micros());
            }
        }
        let encoded = began.elapsed();
        q.flush().await_().map_err(|e| format!("{entry}: {e}"))?;
        if std::env::var_os("RERAIL_KERNEL_TIMES").is_some() {
            eprintln!("delta-launch {entry} {} us (encoded in {} us, {w} wide, {} buffers reached)", began.elapsed().as_micros(), encoded.as_micros(), self.reached.len());
        }
        let words = transfer_to_host_sync::<u64>(&self.state, state_words(w).div_ceil(2));
        let word = |k: usize| (words[k / 2] >> (32 * (k % 2)) & 0xFFFF_FFFF) as u32;
        // Each phase's turns run, the most of any instance's (its count kernel's); a round phase still going after its
        // bound ran past it.
        let mut turns = vec![0u32; phases.len()]; // copy: not block data (the turns each phase ran)
        for (j, meter) in rooms.iter_mut().enumerate() {
            let at = j * STATE_WORDS;
            for (k, n) in meter.needs.iter_mut().enumerate() {
                *n = (*n).max(word(at + MAX_SLOTS + k) as usize);
            }
            for (k, f) in meter.fronts.iter_mut().enumerate() {
                *f = word(at + 2 * MAX_SLOTS + k) as usize;
            }
            if word(at + 3 * MAX_SLOTS) != 0 {
                return Err(format!("{entry}: the kernel refused the launch (why {}): one its kernels do not do", word(at + 3 * MAX_SLOTS)));
            }
            let overflowed = word(at + 3 * MAX_SLOTS + 1) != 0;
            for (k, ph) in phases.iter().enumerate() {
                if ph.rounds && word(at + CTL_AT + PHASES + k) != 0 && !overflowed {
                    return Err(format!("{entry}: phase {} ran past its bound of {} turns", ph.name, ph.turns));
                }
                turns[k] = turns[k].max(word(at + CTL_AT + TURNS_AT + k));
            }
        }
        Ok(turns)
    }
}

/// The threadgroups an indirect dispatch takes at most (a backend that launches them all: its kernels end past their
/// phase's items).
const MAX_GROUPS: u64 = 65535;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod flat_tests;
