//! The preprocessor's evaluator over tapes (decision 0436), plain MSL passes (`pp_eval.metal`).
//!
//! A unit is its files: each a source's bytes and its preprocessor tape (`rerail_c_pp::tape`,
//! format 16, decision 0468 §1), documents of the slab residency. The evaluator reads them where
//! they are: the first document's slab bound, the others made resident and reached from it by
//! the slabs' addresses (0353 step 4, 0471), their views listed (`Unit::of`). Nothing is copied. Writing a tape into its room is its loader's (the store's load, 0353 step 4a;
//! a test's stand-in for it): `placement` says where the parts go and makes the view. The host
//! then encodes the passes and waits; every size a pass needs is made on the device, and no
//! pass's result comes back to the host to decide the next.
//!
//! Built so far: the unit's nodes (a count a file, their prefix sum, and from it the next
//! dispatch's size), the names (each identifier leaf's key), the directives (each file's P0
//! rows: a file pass, a prefix sum and a file pass) and their names (`evaluate`).

use crate::Error;
use crate::pp_blocks::{Arena, Bound, Buf, Enc, Grid, Scan, TG};

/// The passes' source, preprocessed for Metal (`crate::pp_blocks::source`).
pub fn source() -> Result<&'static str, Error> {
    crate::pp_blocks::source()
}

pub use crate::slabs::DocView;

/// A unit's files on the device: its documents, where the residency placed them, and their views.
pub struct Unit {
    /// The documents, held for the unit's life. The first one's slab is the one the kernels bind.
    pub documents: Vec<Arc<Resident>>,
    /// The unit's other slabs, which its passes make resident (`Enc::resident`).
    pub slabs: Vec<Arc<crate::slabs::Slab>>,
    /// Their views, in the unit's file order (`PpFile` in the MSL): placed in each run's arena
    /// (`Front::files`).
    pub views: Vec<DocView>,
    /// The unit's nodes: each view's.
    pub node_bound: usize,
    /// At least the unit's directives: a directive is a node over its `#`, so a file has at most
    /// half its nodes.
    pub dir_bound: usize,
    /// Each file's nodes.
    pub file_nodes: Vec<u32>,
}

use crate::slabs::{Placed, Placement, Resident, Table, TableAt};
use std::sync::Arc;

/// The bytes a preprocessor document takes in its slab: its tape, then its source (each 16-byte
/// aligned, the source with the margin a word read past its end needs).
fn document_bytes(tape: usize, source: usize) -> usize {
    crate::slabs::aligned(tape) + crate::slabs::aligned(source + crate::slabs::SOURCE_MARGIN)
}

/// A preprocessor document's placement in the residency (`slabs::insert`): its digest (the
/// tape's and its source's together, as a resident tape's identity is), its room, and its
/// language's table (`DeviceCode`s of `lang`'s, a code a kind and field, held by the slab once,
/// key `tag | revision << 8`: tag 0 is no language's, so the preprocessor tape's alone). `write`
/// is the loader's: it fills the room's tape and source (`tape_len` and `source_len` bytes) as a
/// load does; the view is made from the bytes there.
pub fn placement<'a>(
    tape_len: usize,
    source_len: usize,
    digest: [u8; 32],
    lang: &'static rerail_tape_format::f16::Lang,
    write: impl FnOnce(&mut [u8], &mut [u8]) + 'a,
) -> Placement<'a> {
    let codes = lang.table().len();
    let table = Table { key: u32::from(rerail_tape_format::f16::Lang::revision(lang)) << 8, len: 24 * codes };
    let bytes = document_bytes(tape_len, source_len);
    let place = move |slab: &Arc<crate::slabs::Slab>, offset: usize, table_at: Option<TableAt>| -> Result<Placed, Error> {
        let base = slab.host();
        let source_at = offset + crate::slabs::aligned(tape_len);
        // SAFETY: `insert` gave `offset..offset + bytes` of the slab to this document alone.
        let (tape, source) = unsafe { (std::slice::from_raw_parts_mut(base.add(offset), tape_len), std::slice::from_raw_parts_mut(base.add(source_at), source_len)) };
        write(tape, source);
        let p = rerail_tape_format::f16::placement(tape).map_err(|_| Error::BackendFailure)?;
        let placed = p.view(lang, 0).map_err(|_| Error::BackendFailure)?;
        let table_at = table_at.ok_or(Error::BackendFailure)?;
        let new_table = (!table_at.held).then(|| {
            let mut t = vec![0u8; 24 * codes];
            for (n, e) in lang.table().entries().iter().enumerate() {
                // `DeviceCode`: kind, field (u16s), flags, class count, then room for its classes.
                t[24 * n..24 * n + 2].copy_from_slice(&e.kind.to_le_bytes()); // copy: not block data
                t[24 * n + 2..24 * n + 4].copy_from_slice(&e.field.to_le_bytes()); // copy: not block data
            }
            t
        });
        let count = |n: usize| u32::try_from(n).map_err(|_| Error::BackendCapacity);
        let view = DocView {
            at: count(offset)?,
            nodes: count(p.n)?,
            format: 16,
            widths: placed.widths,
            columns: placed.columns,
            stride: placed.stride,
            side: placed.side,
            parts: placed.parts,
            cut: placed.cut,
            per: placed.per,
            lines: placed.lines,
            line_count: placed.line_count,
            codes: crate::slabs::codes_offset(offset, table_at.at),
            source: count(source_at - offset)?,
            source_len: count(source_len)?,
            code_count: count(codes)?,
            ..DocView::default()
        };
        let document = Resident {
            digest,
            slab: Arc::clone(slab),
            view,
            at: offset,
            len: bytes,
            bytes: [crate::slabs::aligned(tape_len), crate::slabs::aligned(source_len + crate::slabs::SOURCE_MARGIN), 0],
            tape_length: tape_len,
            table: Some(table),
        };
        Ok(Placed { document, table: new_table })
    };
    (digest, bytes, Some(table), Box::new(place))
}

impl Unit {
    /// The unit of `documents` (in its file order), read where they lie: each view's offset counts
    /// from the first document's slab, by the slabs' addresses (0471). A device with no buffer
    /// addresses reads a unit in one slab only.
    pub fn of(documents: Vec<Arc<Resident>>) -> Result<Unit, Error> {
        let block = documents.first().map(|d| Arc::clone(&d.slab));
        let base = block.as_ref().map(|b| b.device_buffer().device_address());
        let mut slabs: Vec<Arc<crate::slabs::Slab>> = Vec::new();
        let mut views = Vec::with_capacity(documents.len());
        for d in &documents {
            let mut view = d.view;
            if block.as_ref().is_some_and(|b| b.id != d.slab.id) {
                let (Some(Some(base)), Some(at)) = (base, d.slab.device_buffer().device_address()) else {
                    return Err(Error::BackendCapacity);
                };
                view.set_offset((at as i64).wrapping_sub(base as i64).wrapping_add(i64::from(d.view.at)));
                if !slabs.iter().any(|s| s.id == d.slab.id) {
                    slabs.push(Arc::clone(&d.slab));
                }
            }
            views.push(view);
        }
        let file_nodes: Vec<u32> = views.iter().map(|v| v.nodes).collect();
        let node_bound = file_nodes.iter().map(|&n| n as usize).sum::<usize>().max(1);
        let dir_bound = file_nodes.iter().map(|&n| n as usize / 2).sum::<usize>().max(1);
        Ok(Unit { views, documents, slabs, node_bound, dir_bound, file_nodes })
    }

    /// The unit's files.
    pub fn len(&self) -> usize {
        self.views.len()
    }

    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    /// The first document's slab, as the kernels bind it (`None`: no documents).
    pub fn data(&self) -> Option<&crate::lane::Buffer> {
        self.documents.first().map(|d| d.slab.device_buffer())
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
    /// The unit's views (`PpFile`s), in the arena.
    pub files: Buf,
    pub n_files: Buf,
    counts: Buf,
    pub bases: Buf,
    dir_counts: Buf,
    pub dir_bases: Buf,
    groups: Buf,
    guarded: Buf,
    guards: Buf,
    bad: Buf,
    args: Buf,
    dir_args: Buf,
    pub keys: Buf,
    out: Buf,
    pub rows: Buf,
    pub dir_keys: Buf,
    scan: Scan,
    dir_scan: Scan,
}

impl Front {
    pub fn new(arena: &Arena, unit: &Unit, nodes: bool) -> Result<Front, Error> {
        let n = unit.len();
        let per_file = || arena.zeroed::<u32>(n);
        let one = |k: usize| arena.zeroed::<u32>(k);
        Ok(Front {
            n,
            nodes,
            files: arena.from_slice(&unit.views)?,
            n_files: arena.from_slice(&[n as u32])?,
            counts: per_file()?,
            bases: per_file()?,
            dir_counts: per_file()?,
            dir_bases: per_file()?,
            groups: per_file()?,
            guarded: per_file()?,
            guards: arena.zeroed::<u64>(n)?,
            bad: one(1)?,
            args: one(4)?,
            dir_args: one(4)?,
            keys: arena.zeroed::<u64>(unit.node_bound)?,
            out: arena.zeroed::<[u32; 4]>(if nodes { unit.node_bound } else { 1 })?,
            rows: arena.zeroed::<PpDir>(unit.dir_bound)?,
            dir_keys: arena.zeroed::<u64>(unit.dir_bound)?,
            scan: Scan::new(arena, n)?,
            dir_scan: Scan::new(arena, n)?,
        })
    }

    /// The slab of the unit's documents, as a pass binds it (the views' store where the unit
    /// has none: no pass reads it then).
    pub fn data<'a>(&'a self, unit: &'a Unit) -> &'a dyn Bound {
        match unit.data() {
            Some(b) => b,
            None => &self.files,
        }
    }

    /// The unit's nodes in all (after `encode`).
    pub fn total(&self) -> &Buf {
        &self.scan.total
    }

    /// The indirect arguments of a pass a node each (after `encode`).
    pub fn node_args(&self) -> &Buf {
        &self.args
    }

    /// The unit's directive rows in all (after `encode`).
    pub fn dir_total(&self) -> &Buf {
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
        let (u_files, u_data): (&dyn Bound, &dyn Bound) = (&self.files, self.data(unit));
        enc.run("pp_counts", None, &[u_files, u_data, &self.counts, &self.bad], &[n32], files)?;
        self.scan.encode(enc, &self.n_files, 0, &self.counts, &self.bases)?;
        enc.run("pb_args", None, &[&self.scan.total, &self.args], &[0], Grid::Groups(1))?;
        let by_node = Grid::Indirect(&self.args, 0);
        enc.run("pp_names", None, &[u_files, u_data, &self.bases, &self.scan.total, &self.keys], &[n32], by_node)?;
        if self.nodes {
            enc.run("pp_nodes", None, &[u_files, u_data, &self.bases, &self.scan.total, &self.out], &[n32], by_node)?;
        }
        enc.run("pp_dir_counts", None, &[u_files, u_data, &self.dir_counts, &self.bad], &[n32], files)?;
        self.dir_scan.encode(enc, &self.n_files, 0, &self.dir_counts, &self.dir_bases)?;
        enc.run("pb_args", None, &[&self.dir_scan.total, &self.dir_args], &[0], Grid::Groups(1))?;
        enc.run("pp_dirs", None, &[u_files, u_data, &self.dir_bases, &self.rows, &self.groups, &self.guarded, &self.bad], &[n32], files)?;
        let by_row = Grid::Indirect(&self.dir_args, 0);
        let dir_total = &self.dir_scan.total;
        enc.run("pp_dir_keys", None, &[u_files, u_data, &self.bases, &self.keys, &self.dir_bases, dir_total, &self.rows, &self.dir_keys], &[n32], by_row)?;
        enc.run("pp_guards", None, &[&self.guarded, &self.dir_bases, &self.dir_keys, &self.guards], &[n32], files)
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
            let read = |b: &Buf| std::slice::from_raw_parts(b.host().cast::<u32>(), n).to_vec();
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
    run_reading(&[], f)
}

/// `run` over `unit`: each pass makes the unit's other slabs resident.
pub fn run_over(unit: &Unit, f: impl FnOnce(&Enc<'_>) -> Result<(), Error>) -> Result<usize, Error> {
    run_reading(&unit.slabs, f)
}

fn run_reading(resident: &[Arc<crate::slabs::Slab>], f: impl FnOnce(&Enc<'_>) -> Result<(), Error>) -> Result<usize, Error> {
    crate::lane::device()?;
    let scope = crate::launch::LaunchScope::enter();
    let enc = Enc::new("Rerail pp evaluator", source()?, resident);
    // An encoding refused part way runs none of it.
    f(&enc)?;
    enc.into_command().run()?;
    // R9 of decision 0448: every pass in this one command, so the host reads nothing between
    // passes (it reads only after the whole command completed). An encoding that put a dispatch
    // in another command is refused.
    let records = scope.snapshot();
    if records.iter().any(|r| r.command != records[0].command) {
        return Err(Error::BackendFailure);
    }
    Ok(records.len())
}

/// The front passes over `unit`, read back (`nodes`: each node as the device's reader gives it).
pub fn evaluate(unit: &Unit, nodes: bool) -> Result<Evaluated, Error> {
    let arena = Arena::new();
    let front = Front::new(&arena, unit, nodes)?;
    arena.lease()?;
    run_over(unit, |enc| front.encode(enc, unit))?;
    front.read()
}

/// A name's key, as `pp_eval.metal`'s `name_key` makes it: its bytes 8 at a time, line splices
/// left out (its phase-2 spelling).
pub fn name_key(bytes: &[u8]) -> u64 {
    let spelled = phase2(bytes);
    let mut h = 0x9e37_79b9_7f4a_7c15u64 ^ spelled.len() as u64;
    for chunk in spelled.chunks(8) {
        let mut w = [0u8; 8];
        w[..chunk.len()].copy_from_slice(chunk); // copy: not block data (a word of a name being hashed)
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
