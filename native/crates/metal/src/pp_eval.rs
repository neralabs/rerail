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
use crate::launch::{commit_checked, wait_checked};
use crate::pp_blocks::{Enc, Grid, Scan, TG};
use crate::runtime::MappedBuffer;

/// The passes' source: the tape reader and the front passes, the blocks, and the expansion.
pub const SOURCE: &str = crate::pp_blocks::SOURCE;

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
    /// Each file's nodes (its tape's header).
    pub file_nodes: Vec<u32>,
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
        let file_nodes = files.iter().map(|f| nodes(f.1) as u32).collect();
        Ok(Unit { data, files: MappedBuffer::from_slice(&device, &table)?, table, node_bound, dir_bound, file_nodes })
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

/// The passes' buffers on the device, made before any is encoded.
pub struct Front {
    n: usize,
    nodes: bool,
    pub n_files: MappedBuffer,
    counts: MappedBuffer,
    pub bases: MappedBuffer,
    dir_counts: MappedBuffer,
    pub dir_bases: MappedBuffer,
    groups: MappedBuffer,
    guarded: MappedBuffer,
    guards: MappedBuffer,
    bad: MappedBuffer,
    args: MappedBuffer,
    dir_args: MappedBuffer,
    pub keys: MappedBuffer,
    out: MappedBuffer,
    pub rows: MappedBuffer,
    pub dir_keys: MappedBuffer,
    scan: Scan,
    dir_scan: Scan,
}

impl Front {
    pub fn new(unit: &Unit, nodes: bool) -> Result<Front, Error> {
        let device = crate::device()?;
        let n = unit.table.len();
        let per_file = || MappedBuffer::zeroed::<u32>(&device, n.max(1));
        let one = |k: usize| MappedBuffer::zeroed::<u32>(&device, k);
        Ok(Front {
            n,
            nodes,
            n_files: MappedBuffer::from_slice(&device, &[n as u32])?,
            counts: per_file()?,
            bases: per_file()?,
            dir_counts: per_file()?,
            dir_bases: per_file()?,
            groups: per_file()?,
            guarded: per_file()?,
            guards: MappedBuffer::zeroed::<u64>(&device, n.max(1))?,
            bad: one(1)?,
            args: one(4)?,
            dir_args: one(4)?,
            keys: MappedBuffer::zeroed::<u64>(&device, unit.node_bound)?,
            out: MappedBuffer::zeroed::<[u32; 4]>(&device, if nodes { unit.node_bound } else { 1 })?,
            rows: MappedBuffer::zeroed::<PpDir>(&device, unit.dir_bound)?,
            dir_keys: MappedBuffer::zeroed::<u64>(&device, unit.dir_bound)?,
            scan: Scan::new(n)?,
            dir_scan: Scan::new(n)?,
        })
    }

    /// The unit's nodes in all (after `encode`).
    pub fn total(&self) -> &MappedBuffer {
        &self.scan.total
    }

    /// The indirect arguments of a pass a node each (after `encode`).
    pub fn node_args(&self) -> &MappedBuffer {
        &self.args
    }

    /// The unit's directive rows in all (after `encode`).
    pub fn dir_total(&self) -> &MappedBuffer {
        &self.dir_scan.total
    }

    /// The passes, in order:
    /// - the unit's nodes: `pp_counts` (one thread a file), a scan;
    /// - the names: `pp_names` (and `pp_nodes` where asked), one thread a node, their number the
    ///   scan's;
    /// - the directives: `pp_dir_counts` (one thread a file walks its tape), a scan, `pp_dirs`
    ///   (the same walk, each file's rows from its first);
    /// - the directives' names: `pp_dir_keys`, one thread a row, their number the second scan's;
    /// - each file's include guard: `pp_guards`, one thread a file.
    pub fn encode(&self, enc: &Enc<'_>, unit: &Unit) -> Result<(), Error> {
        let n32 = self.n as u32;
        let files = Grid::Groups((self.n as u64).div_ceil(TG));
        let (u_files, u_data) = (&unit.files.buffer as &metal::BufferRef, &unit.data.buffer as &metal::BufferRef);
        enc.run("pp_counts", None, &[u_files, u_data, &self.counts.buffer, &self.bad.buffer], &[n32], files)?;
        self.scan.encode(enc, &self.n_files.buffer, 0, &self.counts.buffer, &self.bases.buffer)?;
        enc.run("pb_args", None, &[&self.scan.total.buffer, &self.args.buffer], &[0], Grid::Groups(1))?;
        let by_node = Grid::Indirect(&self.args.buffer, 0);
        enc.run("pp_names", None, &[u_files, u_data, &self.bases.buffer, &self.scan.total.buffer, &self.keys.buffer], &[n32], by_node)?;
        if self.nodes {
            enc.run("pp_nodes", None, &[u_files, u_data, &self.bases.buffer, &self.scan.total.buffer, &self.out.buffer], &[n32], by_node)?;
        }
        enc.run("pp_dir_counts", None, &[u_files, u_data, &self.dir_counts.buffer, &self.bad.buffer], &[n32], files)?;
        self.dir_scan.encode(enc, &self.n_files.buffer, 0, &self.dir_counts.buffer, &self.dir_bases.buffer)?;
        enc.run("pb_args", None, &[&self.dir_scan.total.buffer, &self.dir_args.buffer], &[0], Grid::Groups(1))?;
        enc.run("pp_dirs", None, &[u_files, u_data, &self.dir_bases.buffer, &self.rows.buffer, &self.groups.buffer, &self.guarded.buffer, &self.bad.buffer], &[n32], files)?;
        let by_row = Grid::Indirect(&self.dir_args.buffer, 0);
        let dir_total = &self.dir_scan.total.buffer;
        enc.run("pp_dir_keys", None, &[u_files, u_data, &self.bases.buffer, &self.keys.buffer, &self.dir_bases.buffer, dir_total, &self.rows.buffer, &self.dir_keys.buffer], &[n32], by_row)?;
        enc.run("pp_guards", None, &[&self.guarded.buffer, &self.dir_bases.buffer, &self.dir_keys.buffer, &self.guards.buffer], &[n32], files)
    }

    /// The results, read back for tests (after the command buffer is done).
    pub fn read(&self) -> Result<Evaluated, Error> {
        let n = self.n;
        // SAFETY: the device finished writing every buffer; each holds the counts read here.
        unsafe {
            if *self.bad.host().cast::<u32>() != 0 {
                return Err(Error::BackendFailure);
            }
            let total = *self.scan.total.host().cast::<u32>();
            let dir_total = *self.dir_scan.total.host().cast::<u32>() as usize;
            let read = |b: &MappedBuffer| std::slice::from_raw_parts(b.host().cast::<u32>(), n).to_vec();
            Ok(Evaluated {
                bases: read(&self.bases),
                total,
                keys: std::slice::from_raw_parts(self.keys.host().cast::<u64>(), total as usize).to_vec(),
                nodes: if self.nodes { std::slice::from_raw_parts(self.out.host().cast::<[u32; 4]>(), total as usize).to_vec() } else { Vec::new() },
                dir_bases: read(&self.dir_bases),
                rows: std::slice::from_raw_parts(self.rows.host().cast::<PpDir>(), dir_total).to_vec(),
                dir_keys: std::slice::from_raw_parts(self.dir_keys.host().cast::<u64>(), dir_total).to_vec(),
                groups: read(&self.groups),
                guards: std::slice::from_raw_parts(self.guards.host().cast::<u64>(), n).to_vec(),
            })
        }
    }
}

/// Encodes `f`'s passes in one command buffer, commits it and waits: the launches it made.
pub fn run(f: impl FnOnce(&Enc<'_>) -> Result<(), Error>) -> Result<usize, Error> {
    let device = crate::device()?;
    let scope = crate::launch::LaunchScope::enter();
    let queue = crate::pool::queue(&device)?;
    let command = queue.new_command_buffer();
    command.set_label("Rerail pp evaluator");
    let encoded = f(&Enc { command, source: SOURCE });
    if let Err(e) = encoded {
        command.commit();
        command.wait_until_completed();
        return Err(e);
    }
    if let Err(refused) = commit_checked(command) {
        crate::launch::release_refused(command);
        return Err(refused);
    }
    wait_checked(command)?;
    // R9 of decision 0448: every pass in this one command buffer, so the host reads nothing
    // between passes (it reads only after the whole buffer completed). An encoding that put a
    // dispatch in another buffer is refused.
    let records = scope.snapshot();
    if records.iter().any(|r| r.command != records[0].command) {
        return Err(Error::BackendFailure);
    }
    Ok(records.len())
}

/// The front passes over `unit`, read back (`nodes`: each node as the device's reader gives it).
pub fn evaluate(unit: &Unit, nodes: bool) -> Result<Evaluated, Error> {
    let front = Front::new(unit, nodes)?;
    run(|enc| front.encode(enc, unit))?;
    front.read()
}

/// A name's key, as `pp_eval.metal`'s `name_key` makes it: its bytes 8 at a time, line splices
/// left out (its phase-2 spelling).
pub fn name_key(bytes: &[u8]) -> u64 {
    let spelled = phase2(bytes);
    let mut h = 0x9e37_79b9_7f4a_7c15u64 ^ spelled.len() as u64;
    for chunk in spelled.chunks(8) {
        let mut w = [0u8; 8];
        w[..chunk.len()].copy_from_slice(chunk);
        h = (h ^ u64::from_le_bytes(w)).wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 32;
    }
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^ (h >> 29)
}

/// `bytes` with their line splices left out (a `\`, blanks, then a newline), as phase 2 reads them.
pub fn phase2(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if !bytes.contains(&b'\\') {
        return std::borrow::Cow::Borrowed(bytes);
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let mut k = i + 1;
            while matches!(bytes.get(k), Some(b' ' | b'\t' | 0x0b | 0x0c)) {
                k += 1;
            }
            let e = match bytes.get(k) {
                Some(b'\n') => Some(k + 1),
                Some(b'\r') if bytes.get(k + 1) == Some(&b'\n') => Some(k + 2),
                _ => None,
            };
            if let Some(e) = e {
                i = e;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    std::borrow::Cow::Owned(out)
}
