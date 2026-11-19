//! The preprocessor's evaluator over tapes (decision 0436), plain MSL passes (`pp_eval.metal`).
//!
//! A unit is its files: each a source's bytes and its preprocessor tape (`rerail_c_pp::tape`,
//! a v9np image), which the device reads in place. The host's part is the loading: it copies
//! the files into one buffer and lists where each is (`Unit::load`). It then encodes the passes
//! and waits; every size a pass needs is made on the device, and no pass's result comes back to
//! the host to decide the next.
//!
//! Built so far: the unit's nodes (a count a file, their prefix sum, and from it the next
//! dispatch's size), the names (each identifier leaf's key), the directives (each file's P0
//! rows: a file pass, a prefix sum and a file pass) and their names (`evaluate`).

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
    /// The unit's nodes: each tape's header says its own.
    pub node_bound: usize,
    /// At least the unit's directives: a directive is a node over its `#`, so a file has at most
    /// half its nodes.
    pub dir_bound: usize,
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
        // A tape's node count, from its header (`rerail_tape_format`: bytes 24..28).
        let nodes = |t: &[u8]| t.get(24..28).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
        let node_bound = files.iter().map(|f| nodes(f.1)).sum::<usize>().max(1);
        let dir_bound = files.iter().map(|f| nodes(f.1) / 2).sum::<usize>().max(1);
        Ok(Unit { data, files: MappedBuffer::from_slice(&device, &table)?, table, node_bound, dir_bound })
    }
}

/// The passes' results, read back for tests.
pub struct Evaluated {
    /// Each file's first node in the unit, the unit's nodes, and each node's key (0 where it is
    /// no identifier).
    pub bases: Vec<u32>,
    pub total: u32,
    pub keys: Vec<u64>,
    /// Each node as the device's reader gives it (key, start, end, size), with `nodes`.
    pub nodes: Vec<[u32; 4]>,
    /// Each file's first directive row, the rows, each row's name key (0: none), and each
    /// file's groups.
    pub dir_bases: Vec<u32>,
    pub rows: Vec<PpDir>,
    pub dir_keys: Vec<u64>,
    pub groups: Vec<u32>,
    /// Each file's include guard's key (0: none).
    pub guards: Vec<u64>,
}

/// A directive's row (`PpDir` in the MSL): its node in its file's tape, its kind, the text tokens
/// before it in the file, the group it is read in and the group after it, and its chain
/// (`u32::MAX`: none).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PpDir {
    pub node: u32,
    pub kind: u32,
    pub text_before: u32,
    pub group: u32,
    pub after: u32,
    pub chain: u32,
}

/// The passes, in one command buffer:
/// - the unit's nodes: `pp_counts`, `pp_scan`;
/// - the names: `pp_names` (and `pp_nodes` with `nodes`), one thread a node, their number the
///   scan's;
/// - the directives: `pp_dir_counts` (one thread a file walks its tape), `pp_scan`, `pp_dirs`
///   (the same walk, each file's rows from its first);
/// - the directives' names: `pp_dir_keys`, one thread a row, their number the second scan's;
/// - each file's include guard: `pp_guards`, one thread a file.
pub fn evaluate(unit: &Unit, nodes: bool) -> Result<Evaluated, Error> {
    let device = crate::device()?;
    let n = unit.table.len();
    let per_file = || MappedBuffer::zeroed::<u32>(&device, n.max(1));
    let one = |k: usize| MappedBuffer::zeroed::<u32>(&device, k);
    let (counts, bases, dir_counts, dir_bases, groups, guarded) = (per_file()?, per_file()?, per_file()?, per_file()?, per_file()?, per_file()?);
    let guards = MappedBuffer::zeroed::<u64>(&device, n.max(1))?;
    let (bad, total, args, dir_total, dir_args) = (one(1)?, one(1)?, one(4)?, one(1)?, one(4)?);
    let keys = MappedBuffer::zeroed::<u64>(&device, unit.node_bound)?;
    let out = MappedBuffer::zeroed::<[u32; 4]>(&device, if nodes { unit.node_bound } else { 1 })?;
    let rows = MappedBuffer::zeroed::<PpDir>(&device, unit.dir_bound)?;
    let dir_keys = MappedBuffer::zeroed::<u64>(&device, unit.dir_bound)?;
    let names = ["pp_counts", "pp_scan", "pp_names", "pp_nodes", "pp_dir_counts", "pp_dirs", "pp_dir_keys", "pp_guards"];
    let pipes = names.iter().map(|n| pipeline_named(SOURCE, n)).collect::<Result<Vec<_>, _>>()?;
    let pipe = |name: &str| &pipes[names.iter().position(|&n| n == name).expect("a pass of the list")];
    let n32 = n as u32;
    let files = MTLSize::new(n.max(1) as u64, 1, 1);
    let _scope = crate::launch::LaunchScope::enter();
    let queue = crate::pool::queue(&device)?;
    let command = queue.new_command_buffer();
    command.set_label("Rerail pp evaluator");
    // A pass of one thread a file, its count of files at `at`.
    let by_file = |name: &str, buffers: &[&metal::BufferRef], at: u64| {
        let e = command.new_compute_command_encoder();
        e.set_compute_pipeline_state(pipe(name));
        e.checked_set_buffers(command, buffers);
        e.set_bytes(at, 4, std::ptr::from_ref(&n32).cast());
        e.checked_dispatch_threads(command, files, MTLSize::new(64, 1, 1));
        e.end_encoding();
    };
    // A scan of the files' counts: each file's first, the total, the next dispatch's size.
    let scan = |counts: &MappedBuffer, bases: &MappedBuffer, total: &MappedBuffer, args: &MappedBuffer| {
        let e = command.new_compute_command_encoder();
        e.set_compute_pipeline_state(pipe("pp_scan"));
        e.checked_set_buffers(command, &[&counts.buffer, &bases.buffer, &total.buffer, &args.buffer]);
        e.set_bytes(4, 4, std::ptr::from_ref(&n32).cast());
        e.checked_dispatch_thread_groups(command, MTLSize::new(1, 1, 1), MTLSize::new(1024, 1, 1));
        e.end_encoding();
    };
    // A pass of one thread an element, sized by `args` (groups of 256).
    let indirect = |name: &str, buffers: &[&metal::BufferRef], args: &MappedBuffer| {
        let e = command.new_compute_command_encoder();
        e.set_compute_pipeline_state(pipe(name));
        e.checked_set_buffers(command, buffers);
        e.set_bytes(buffers.len() as u64, 4, std::ptr::from_ref(&n32).cast());
        e.checked_dispatch_thread_groups_indirect(command, &args.buffer, 0, MTLSize::new(256, 1, 1));
        e.end_encoding();
    };
    let (u_files, u_data) = (&unit.files.buffer, &unit.data.buffer);
    by_file("pp_counts", &[u_files, u_data, &counts.buffer, &bad.buffer], 4);
    scan(&counts, &bases, &total, &args);
    indirect("pp_names", &[u_files, u_data, &bases.buffer, &total.buffer, &keys.buffer], &args);
    if nodes {
        indirect("pp_nodes", &[u_files, u_data, &bases.buffer, &total.buffer, &out.buffer], &args);
    }
    by_file("pp_dir_counts", &[u_files, u_data, &dir_counts.buffer, &bad.buffer], 4);
    scan(&dir_counts, &dir_bases, &dir_total, &dir_args);
    by_file("pp_dirs", &[u_files, u_data, &dir_bases.buffer, &rows.buffer, &groups.buffer, &guarded.buffer, &bad.buffer], 7);
    indirect("pp_dir_keys", &[u_files, u_data, &bases.buffer, &keys.buffer, &dir_bases.buffer, &dir_total.buffer, &rows.buffer, &dir_keys.buffer], &dir_args);
    by_file("pp_guards", &[&guarded.buffer, &dir_bases.buffer, &dir_keys.buffer, &guards.buffer], 4);
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
        let dir_total = *dir_total.host().cast::<u32>() as usize;
        let read = |b: &MappedBuffer| std::slice::from_raw_parts(b.host().cast::<u32>(), n).to_vec();
        Ok(Evaluated {
            bases: read(&bases),
            total,
            keys: std::slice::from_raw_parts(keys.host().cast::<u64>(), total as usize).to_vec(),
            nodes: if nodes { std::slice::from_raw_parts(out.host().cast::<[u32; 4]>(), total as usize).to_vec() } else { Vec::new() },
            dir_bases: read(&dir_bases),
            rows: std::slice::from_raw_parts(rows.host().cast::<PpDir>(), dir_total).to_vec(),
            dir_keys: std::slice::from_raw_parts(dir_keys.host().cast::<u64>(), dir_total).to_vec(),
            groups: read(&groups),
            guards: std::slice::from_raw_parts(guards.host().cast::<u64>(), n).to_vec(),
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
