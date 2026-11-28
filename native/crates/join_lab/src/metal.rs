//! The `metal` engine: a captured launch's chain run on the device, as `metal_join`'s `launch` encodes it
//! (`Pass::chain_at` over the program's kernel, `join_program_pipe`). Each region the lab placed is wrapped where it
//! lies (decision 0353: no copy), a reservation as a buffer over its pages, a mapped block as its own wrap
//! (`Block::device_buffer`); its words are bound to the wraps' device addresses (`Wave::load_on`), a view's home's host
//! word to the host's. A capture is the CPU device's, its words the host's addresses: bound again, it is the device's.
//!
//! On Metal a wrap released leaves the pages it wrapped unreadable to the host (`Shared::wrap_static`), so the wraps
//! are released before the memory they wrap, once the replay is compared (`Loaded`'s fields' order).

use crate::{Mem, Ran};
use rerail_accel_gpu::gpu::join::{Pass, Reach};
use rerail_device::device::Buffer;

/// The lab's regions wrapped for the device, and the codes' directory's wrap (the binary's, held as long as it).
pub struct Wraps {
    pub bufs: Vec<Buffer>,
    /// Each region's device address.
    pub dev: Vec<u64>,
    /// Whether the launch writes each (a reservation of the lab's) or only reads it (a block mapped from its file).
    pub writes: Vec<bool>,
    pub codes: Buffer,
}

impl Wraps {
    /// `mems` wrapped for the process's device: an error where it has none, or where it is the host's
    /// (`RERAIL_GPU=cpu`), as the program's engine is that one.
    pub fn of(mems: &[Mem]) -> Result<Wraps, String> {
        let shared = rerail_device::shared::get().ok_or("the metal engine: no device here (it runs on a Mac with Metal)")?;
        if shared.device.is_cpu() {
            return Err("the metal engine: the device is the host's (RERAIL_GPU=cpu); the program engine is that one".into());
        }
        let (mut bufs, mut dev, mut writes) = (Vec::with_capacity(mems.len()), Vec::with_capacity(mems.len()), Vec::with_capacity(mems.len()));
        for m in mems {
            let b = match m {
                // SAFETY: the reservation outlives the wrap (`Loaded` releases its wraps first), read and written by
                // the device only while the lab waits for it.
                Mem::Reserved { p, len } => unsafe { rerail_device::device::wrap_host_memory(&shared.device, *p, *len) }?,
                // A block mapped from its file: wrapped over its whole pages (its file's tail page mapped whole), as a
                // no-copy wrap is of whole pages.
                // SAFETY: the mapping outlives the wrap, which the device only reads.
                Mem::Mapped(b) => unsafe { rerail_device::device::wrap_host_memory(&shared.device, b.bytes(0, 0).as_ptr().cast_mut(), b.used().max(1).next_multiple_of(rerail_blocks::block::PAGE)) }?,
            };
            dev.push(b.device_address().ok_or("the metal engine: a buffer without a device address")?);
            writes.push(matches!(m, Mem::Reserved { .. }));
            bufs.push(b);
        }
        // A measurement aid: `LAB_WRAPS` names each wrap whose device address is not its memory's.
        if std::env::var_os("LAB_WRAPS").is_some() {
            for (i, (m, d)) in mems.iter().zip(&dev).enumerate() {
                if m.base() as u64 != *d {
                    eprintln!("join-lab: wrap {i}: host {:#x}, device {d:#x}, {} bytes", m.base(), m.len());
                }
            }
        }
        let codes = shared.wrap_static(rerail_tape_format::codes_dir::bytes())?;
        Ok(Wraps { bufs, dev, writes, codes })
    }

    /// The codes' directory's device address.
    pub fn codes_address(&self) -> Result<u64, String> {
        self.codes.device_address().ok_or_else(|| "the codes' directory's wrap has no device address".into())
    }
}

/// The chain of `l` on the device, its binds the wraps of regions `binds`: its dispatches as `launch` encodes them,
/// every region resident, committed alone and waited for. Its seconds are the device's; its dispatches and groups
/// read from its control words once it is done (each dispatch's grid, as the phases before it set them).
pub fn run(l: &crate::Loaded, w: &Wraps, binds: &[usize; 9]) -> Result<Ran, String> {
    // The replay is an evaluation of its own (decision 0601): its one launch charged to it.
    let _evaluation = rerail_accel_gpu::gpu::launch::LaunchScope::enter();
    let pipe = rerail_device_metal::code_rule_dsl::transducer::packed::metal_join::join_program_pipe(l.wave.format)?;
    let pass = Pass::new()?;
    for (b, &write) in w.bufs.iter().zip(&w.writes) {
        pass.use_as(b, if write { Reach::Write } else { Reach::Read });
    }
    pass.use_read_fixed(&[&w.codes]);
    let bound: Vec<(&Buffer, u64)> = binds.iter().map(|&i| (&w.bufs[i], 0)).collect();
    let args: Vec<u8> = l.wave.args.iter().flat_map(|a| a.to_ne_bytes()).collect();
    let ctl = &w.bufs[binds[7]];
    let start = std::time::Instant::now();
    pass.chain_at(&pipe, &bound, &args, ctl, &l.wave.offsets);
    let device = pass.commit_wait()?;
    let wall = start.elapsed().as_secs_f64();
    let mut ran = Ran { seconds: if device > 0.0 { device } else { wall }, ..Ran::default() };
    let at = l.mems[binds[7]].base();
    for &offset in &l.wave.offsets {
        // SAFETY: three words of the control words, the device's work on them done.
        let grid = unsafe { std::ptr::read_volatile((at + offset as usize) as *const [u32; 3]) };
        if !grid.contains(&0) {
            ran.dispatches += 1;
            ran.groups += grid.iter().map(|&g| u64::from(g)).product::<u64>();
        }
    }
    Ok(ran)
}
