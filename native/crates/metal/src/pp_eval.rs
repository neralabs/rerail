//! The preprocessor's evaluator over tapes (decision 0436), plain MSL passes (`pp_eval.metal`).
//!
//! A unit is its files: each a source's bytes and its preprocessor tape (`rerail_c_pp::tape`,
//! a v9np image), which the device reads in place. The host's part is the loading: it copies
//! the files into one buffer and lists where each is (`Unit::load`). It then encodes the passes
//! and waits; every size a pass needs is made on the device, and no pass's result comes back to
//! the host to decide the next.
//!
//! Built so far: the unit's nodes (a count a file, their prefix sum, and from it the next
//! dispatch's size) and the names (`names`: each identifier leaf's key).

use crate::Error;
use crate::launch::{CheckedComputeEncoder, commit_checked, wait_checked};
use crate::runtime::{MappedBuffer, pipeline_named};
use metal::MTLSize;

/// The passes' source.
pub const SOURCE: &str = include_str!("pp_eval.metal");

/// A file of the unit (`PpFile` in the MSL): its source's bytes and its tape's, at byte offsets
/// in the unit's data.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PpFile {
    pub src: u32,
    pub src_len: u32,
    pub tape: u32,
    pub tape_len: u32,
}

/// A unit's files on the device.
pub struct Unit {
    pub data: MappedBuffer,
    pub files: MappedBuffer,
    pub table: Vec<PpFile>,
    /// At least the unit's nodes: a tape's code column is two bytes a node.
    pub node_bound: usize,
}

/// Bytes past the last file, so that a word read at a source's last byte stays in the buffer.
const SLACK: usize = 16;

impl Unit {
    /// `files` (each its source and its tape) copied into one buffer, each at 16 bytes.
    pub fn load(files: &[(&[u8], &[u8])]) -> Result<Unit, Error> {
        let device = crate::device()?;
        let pad = |x: usize| (x + 15) & !15;
        let mut table = Vec::with_capacity(files.len());
        let mut at = 0;
        for (src, tape) in files {
            let s = at;
            at = pad(at + src.len());
            let t = at;
            at = pad(at + tape.len());
            table.push(PpFile { src: s as u32, src_len: src.len() as u32, tape: t as u32, tape_len: tape.len() as u32 });
        }
        let data = MappedBuffer::zeroed::<u8>(&device, at + SLACK)?;
        for ((src, tape), f) in files.iter().zip(&table) {
            // SAFETY: the buffer holds `at` bytes, every file's placed inside them, apart.
            unsafe {
                std::ptr::copy_nonoverlapping(src.as_ptr(), data.host().add(f.src as usize), src.len());
                std::ptr::copy_nonoverlapping(tape.as_ptr(), data.host().add(f.tape as usize), tape.len());
            }
        }
        let node_bound = files.iter().map(|f| f.1.len() / 2).sum::<usize>().max(1);
        Ok(Unit { data, files: MappedBuffer::from_slice(&device, &table)?, table, node_bound })
    }
}

/// The names pass's results, read back for tests: each file's first node in the unit, the
/// unit's nodes, and each node's key (0 where it is no identifier).
pub struct Names {
    pub bases: Vec<u32>,
    pub total: u32,
    pub keys: Vec<u64>,
    /// Each node as the device's reader gives it (key, start, end, size), with `nodes`.
    pub nodes: Vec<[u32; 4]>,
}

/// The unit's nodes and their names: `pp_counts`, `pp_scan`, then `pp_names` (and `pp_nodes`
/// with `nodes`) one thread a node, their number the scan's.
pub fn names(unit: &Unit, nodes: bool) -> Result<Names, Error> {
    let device = crate::device()?;
    let n = unit.table.len();
    let counts = MappedBuffer::zeroed::<u32>(&device, n.max(1))?;
    let bases = MappedBuffer::zeroed::<u32>(&device, n.max(1))?;
    let bad = MappedBuffer::zeroed::<u32>(&device, 1)?;
    let total = MappedBuffer::zeroed::<u32>(&device, 1)?;
    let args = MappedBuffer::zeroed::<u32>(&device, 4)?;
    let keys = MappedBuffer::zeroed::<u64>(&device, unit.node_bound)?;
    let out = MappedBuffer::zeroed::<[u32; 4]>(&device, if nodes { unit.node_bound } else { 1 })?;
    let pipe = |name: &str| pipeline_named(SOURCE, name);
    let (p_counts, p_scan, p_names, p_nodes) = (pipe("pp_counts")?, pipe("pp_scan")?, pipe("pp_names")?, pipe("pp_nodes")?);
    let n32 = n as u32;
    let _scope = crate::launch::LaunchScope::enter();
    let queue = crate::pool::queue(&device)?;
    let command = queue.new_command_buffer();
    command.set_label("Rerail pp evaluator");
    let e = command.new_compute_command_encoder();
    e.set_compute_pipeline_state(&p_counts);
    e.checked_set_buffers(command, &[&unit.files.buffer, &unit.data.buffer, &counts.buffer, &bad.buffer]);
    e.set_bytes(4, 4, std::ptr::from_ref(&n32).cast());
    e.checked_dispatch_threads(command, MTLSize::new(n.max(1) as u64, 1, 1), MTLSize::new(64, 1, 1));
    e.end_encoding();
    let e = command.new_compute_command_encoder();
    e.set_compute_pipeline_state(&p_scan);
    e.checked_set_buffers(command, &[&counts.buffer, &bases.buffer, &total.buffer, &args.buffer]);
    e.set_bytes(4, 4, std::ptr::from_ref(&n32).cast());
    e.checked_dispatch_thread_groups(command, MTLSize::new(1, 1, 1), MTLSize::new(1024, 1, 1));
    e.end_encoding();
    for (p, o) in [(&p_names, &keys), (&p_nodes, &out)] {
        if std::ptr::eq(p, &p_nodes) && !nodes {
            continue;
        }
        let e = command.new_compute_command_encoder();
        e.set_compute_pipeline_state(p);
        e.checked_set_buffers(command, &[&unit.files.buffer, &unit.data.buffer, &bases.buffer, &total.buffer, &o.buffer]);
        e.set_bytes(5, 4, std::ptr::from_ref(&n32).cast());
        e.checked_dispatch_thread_groups_indirect(command, &args.buffer, 0, MTLSize::new(256, 1, 1));
        e.end_encoding();
    }
    if let Err(refused) = commit_checked(command) {
        crate::launch::release_refused(command);
        return Err(refused);
    }
    wait_checked(command)?;
    // SAFETY: the device finished writing every buffer; each holds the counts read here.
    unsafe {
        if *bad.host().cast::<u32>() != 0 {
            return Err(Error::BackendFailure);
        }
        let total = *total.host().cast::<u32>();
        let read = |b: &MappedBuffer, k: usize| std::slice::from_raw_parts(b.host().cast::<u32>(), k).to_vec();
        Ok(Names {
            bases: read(&bases, n),
            total,
            keys: std::slice::from_raw_parts(keys.host().cast::<u64>(), total as usize).to_vec(),
            nodes: if nodes { std::slice::from_raw_parts(out.host().cast::<[u32; 4]>(), total as usize).to_vec() } else { Vec::new() },
        })
    }
}

/// A name's key, as `pp_eval.metal`'s `name_key` makes it: its bytes 8 at a time.
pub fn name_key(bytes: &[u8]) -> u64 {
    let mut h = 0x9e37_79b9_7f4a_7c15u64 ^ bytes.len() as u64;
    for chunk in bytes.chunks(8) {
        let mut w = [0u8; 8];
        w[..chunk.len()].copy_from_slice(chunk);
        h = (h ^ u64::from_le_bytes(w)).wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 32;
    }
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^ (h >> 29)
}
