//! The delta executor's device (decision 0686, E3): `rerail_delta_exec::op::Kernels` on the process's device, any
//! backend (`rerail_gpu::shared`: Metal, CUDA, HIP, OpenCL, or the CPU's, on its vector path with
//! `RERAIL_CPU_EXEC=vec`). The executor pins a launch and drives its version's phases; this crate makes the launch's
//! device descriptor table (E2), hands it to the launch's fence (E1), compiles the version's UKL once a program and
//! launches each kernel on a tile of items.
//!
//! A version's kernels take the same arguments, by name: `t`, the device table (read with `gens::DESC_READS`);
//! `slots`, the slots' block ids; `p`, the params' bytes; `tile`, the items `lo..hi` (`TileP`), thread `i` item
//! `lo + i`; and `res`, one word a count or a round's kernel writes. `PRELUDE` comes before the version's source.
//!
//! `Gpu::host` runs in the host's memory domain (each block read in place: unified memory, the CPU's); `Gpu::discrete`
//! in a device domain of its own (1 and up), each block a replica there (`region::DeviceCopy`, placed by
//! `Gpu::replicate`), the table's own block too (`gens::device_table_in`).

#![cfg(any(target_os = "macos", target_os = "linux", windows))]

use rerail_blocks::ledger::Fence;
use rerail_delta_exec::dir::{BlockId, Directory, Launch};
use rerail_delta_exec::op::{Devices, Kernels, OnDevice};
use rerail_gpu::device::{Buffer, ComputeQueue, Device, alloc_uninitialized_array_sync, transfer_to_host_sync};
use rerail_gpu::kernels::{Program, launch};
use rerail_gpu::shared::Shared;
use std::collections::HashMap;
use std::sync::Arc;

/// What every version's source starts with: the backend, the descriptor table's reads, and the tile's argument.
pub const PRELUDE: &str = r#"
typedef struct TileP { uint32_t lo; uint32_t hi; } TileP;
UKL_REFLECT_STRUCT(TileP, lo, hi)
"#;

#[repr(C)]
#[derive(Clone, Copy)]
struct TileP {
    lo: u32,
    hi: u32,
}

// SAFETY: two u32s, no padding.
unsafe impl rerail_gpu::Pod for TileP {}

/// The threads of a threadgroup a kernel's items run in.
const GROUP: u64 = 64;

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

    fn bind(&self, launch: &Launch, fence: &Arc<Fence>, slots: &[u32], params: &[u8]) -> Result<Box<dyn OnDevice + '_>, String> {
        let t = launch.table();
        let pins = launch.pins();
        let dt = if self.domain == 0 {
            rerail_blocks::gens::device_table(&t, &pins, &self.shared.device)?
        } else {
            rerail_blocks::gens::device_table_in(&t, &pins, self.domain, self.shared)?
        };
        // The kernels bind the table's buffer and reach the blocks' (device handles, which the fence's uses keep).
        let (table, at, reached) = (dt.buffer.clone(), dt.at, dt.reached.clone());
        dt.to_fence(fence, "a version's device table")?;
        let words = |n: usize| alloc_uninitialized_array_sync::<u64>(&self.shared.device, n.max(1));
        let slots_b = words(slots.len().div_ceil(2));
        let params_b = words(params.len().div_ceil(8));
        let res = words(1);
        let mut q = ComputeQueue::new(&self.shared.queue);
        // The kernels' arguments, written by the queue: the slots' ids and the params' bytes (not block data).
        q.enqueue_copy_from_host(&slots_b, 0, as_bytes(slots));
        if !params.is_empty() {
            q.enqueue_copy_from_host(&params_b, 0, params);
        }
        q.flush().await_()?;
        Ok(Box::new(GpuBound { shared: self.shared, table, at, reached, slots: slots_b, params: params_b, res, programs: HashMap::new() }))
    }
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
    res: Buffer,
    programs: HashMap<usize, &'static Program>,
}

impl OnDevice for GpuBound {
    fn run(&mut self, source: &'static str, name: &str, lo: u32, hi: u32) -> Result<u64, String> {
        let program = match self.programs.get(&(source.as_ptr() as usize)) {
            Some(p) => *p,
            None => {
                let text = format!("#include <ukl/backend.h>\n{}{PRELUDE}{source}", rerail_blocks::gens::DESC_READS);
                let p = rerail_gpu::kernels::program("delta_version.uglykl", &text)?;
                self.programs.insert(source.as_ptr() as usize, p);
                p
            }
        };
        let f = program.function(name)?;
        let mut q = ComputeQueue::new(&self.shared.queue);
        q.enqueue_fill_array(&self.res, &0u64, 0, Some(1));
        let mut run = launch(&q, f).buffer_at("t", &self.table, self.at).buffer("slots", &self.slots).buffer("p", &self.params).value("tile", &TileP { lo, hi }).buffer("res", &self.res);
        for r in &self.reached {
            run = run.uses(r);
        }
        run.run(&[u64::from(hi - lo).div_ceil(GROUP)], &[GROUP]).map_err(|e| format!("{name}: {e}"))?;
        q.flush().await_().map_err(|e| format!("{name}: {e}"))?;
        Ok(transfer_to_host_sync::<u64>(&self.res, 1)[0])
    }
}

#[cfg(test)]
mod tests;
