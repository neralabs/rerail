//! The preprocessor's evaluator over tapes (decision 0436), plain MSL passes (`pp_eval.metal`).
//!
//! A unit is its files: each a source's bytes and its preprocessor tape (`rerail_c_pp::tape`,
//! format 16, decision 0460 §1), documents of the slab residency. The evaluator reads them where
//! they are: the unit's documents gathered into one slab (`slabs::gather`), their views listed
//! (`Unit::of`). Writing a tape into its room is its loader's (the store's load, 0353 step 4a;
//! a test's stand-in for it): `placement` says where the parts go and makes the view. The host
//! then encodes the passes and waits; every size a pass needs is made on the device, and no
//! pass's result comes back to the host to decide the next.
//!
//! Built so far: the unit's nodes (a count a file, their prefix sum, and from it the next
//! dispatch's size), the names (each identifier leaf's key), the directives (each file's P0
//! rows: a file pass, a prefix sum and a file pass) and their names (`evaluate`).

use crate::Error;
use crate::launch::{commit_checked, wait_checked};
use crate::pp_blocks::{Arena, Bound, Buf, Enc, Grid, Lay, Scan};

/// The passes' source, preprocessed for Metal (`crate::pp_blocks::source`).
pub fn source() -> Result<&'static str, Error> {
    crate::pp_blocks::source()
}

pub use crate::slabs::DocView;

/// A unit's files on the device: its documents, in one slab, and their views.
pub struct Unit {
    /// The documents, gathered into one slab: the lease the kernels read through.
    pub documents: Vec<Arc<Resident>>,
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
    /// The unit of `documents` (in its file order): gathered into one slab, which the kernels
    /// read, and their views listed.
    pub fn of(documents: Vec<Arc<Resident>>) -> Result<Unit, Error> {
        let documents = crate::slabs::gather(documents)?;
        let views: Vec<DocView> = documents.iter().map(|d| d.view).collect();
        let file_nodes: Vec<u32> = views.iter().map(|v| v.nodes).collect();
        let node_bound = file_nodes.iter().map(|&n| n as usize).sum::<usize>().max(1);
        let dir_bound = file_nodes.iter().map(|&n| n as usize / 2).sum::<usize>().max(1);
        Ok(Unit { views, documents, node_bound, dir_bound, file_nodes })
    }

    /// The unit's files.
    pub fn len(&self) -> usize {
        self.views.len()
    }

    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    /// The slab the unit's documents are in, as the kernels bind it (`None`: no documents).
    pub fn data(&self) -> Option<&metal::BufferRef> {
        self.documents.first().map(|d| &**d.slab.buffer())
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

/// The front passes' stores, and the grid's own: its control words (`GRID`: the barrier's and
/// the expansion's stage), the groups' sums its scans share (`SUMS`), and the run's layout.
pub struct Front {
    n: usize,
    nodes: bool,
    /// The unit's views (`PpFile`s), in the arena.
    pub files: Buf,
    pub grid: Buf,
    sums: Buf,
    pub lay: Buf,
    counts: Buf,
    pub bases: Buf,
    dir_counts: Buf,
    pub dir_bases: Buf,
    groups: Buf,
    guarded: Buf,
    guards: Buf,
    bad: Buf,
    pub keys: Buf,
    out: Buf,
    pub rows: Buf,
    pub dir_keys: Buf,
    scan: Scan,
    dir_scan: Scan,
}

/// The stages a run of `pp_setup` ends after (`LAST`): the front, the definitions, the
/// visibility, or the whole setup (the expansion follows).
pub const TO_FRONT: u32 = 1;
pub const TO_DEFS: u32 = 2;
pub const TO_VIS: u32 = 3;
pub const TO_ALL: u32 = 4;

impl Front {
    /// `groups`: the grid's groups (the scans' sums are two words a group).
    pub fn new(arena: &Arena, unit: &Unit, nodes: bool, groups: u64) -> Result<Front, Error> {
        let n = unit.len();
        let per_file = || arena.zeroed::<u32>(n);
        Ok(Front {
            n,
            nodes,
            files: arena.from_slice(&unit.views)?,
            grid: arena.zeroed::<u32>(8)?,
            sums: arena.zeroed::<u32>(2 * groups as usize + 2)?,
            lay: arena.zeroed::<u32>(crate::pp_blocks::SLOTS.len())?,
            counts: per_file()?,
            bases: per_file()?,
            dir_counts: per_file()?,
            dir_bases: per_file()?,
            groups: per_file()?,
            guarded: per_file()?,
            guards: arena.zeroed::<u64>(n)?,
            bad: arena.zeroed::<u32>(1)?,
            keys: arena.zeroed::<u64>(unit.node_bound)?,
            out: arena.zeroed::<[u32; 4]>(if nodes { unit.node_bound } else { 1 })?,
            rows: arena.zeroed::<PpDir>(unit.dir_bound)?,
            dir_keys: arena.zeroed::<u64>(unit.dir_bound)?,
            scan: Scan::new(arena)?,
            dir_scan: Scan::new(arena)?,
        })
    }

    /// The front's slots of the layout, and the run's words: the stage it ends after.
    pub fn lay(&self, l: &mut Lay, last: u32) {
        l.at("GRID", &self.grid).at("SUMS", &self.sums).at("FILES", &self.files).at("COUNTS", &self.counts).at("BASES", &self.bases);
        l.at("TOTAL", &self.scan.total).at("DIR_COUNTS", &self.dir_counts).at("DIR_BASES", &self.dir_bases).at("DIR_TOTAL", &self.dir_scan.total);
        l.at("DIR_GROUPS", &self.groups).at("GUARDED", &self.guarded).at("GUARDS", &self.guards).at("BAD", &self.bad).at("KEYS", &self.keys);
        l.at("NODES_OUT", &self.out).at("ROWS", &self.rows).at("DIR_KEYS", &self.dir_keys);
        l.word("N_FILES", self.n as u32).word("NODES_FLAG", u32::from(self.nodes)).word("LAST", last);
    }

    /// The slab of the unit's documents, as a pass binds it (the views' store where the unit
    /// has none: no pass reads it then).
    pub fn data<'a>(&'a self, unit: &'a Unit) -> &'a dyn Bound {
        match unit.data() {
            Some(b) => b,
            None => &self.files,
        }
    }

    /// The unit's nodes in all (after a run).
    pub fn total(&self) -> &Buf {
        &self.scan.total
    }

    /// The unit's directive rows in all (after a run).
    pub fn dir_total(&self) -> &Buf {
        &self.dir_scan.total
    }

    /// Whether the run's grid failed a barrier (after it).
    pub fn barrier_failed(&self) -> bool {
        // SAFETY: the device finished writing the grid's words.
        unsafe { *self.grid.host().cast::<u32>().add(2) != 0 }
    }

    /// `pp_setup` over the arena, `groups` groups.
    pub fn setup(&self, enc: &Enc<'_>, arena: &Arena, unit: &Unit, groups: u64) -> Result<(), Error> {
        enc.run("pp_setup", None, &[arena, self.data(unit), &self.lay], &[], Grid::Groups(groups))
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
    run_timed(f).map(|(launches, _)| launches)
}

/// `run`, and the command buffer's time on the GPU, in seconds.
pub fn run_timed(f: impl FnOnce(&Enc<'_>) -> Result<(), Error>) -> Result<(usize, f64), Error> {
    let device = crate::device()?;
    let scope = crate::launch::LaunchScope::enter();
    let queue = crate::pool::queue(&device)?;
    let command = queue.new_command_buffer();
    command.set_label("Rerail pp evaluator");
    let encoded = f(&Enc { command, source: source()? });
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
    Ok((records.len(), crate::launch::gpu_seconds(command)))
}

/// The front passes over `unit`, read back (`nodes`: each node as the device's reader gives it).
pub fn evaluate(unit: &Unit, nodes: bool) -> Result<Evaluated, Error> {
    let mut groups = crate::pp_blocks::GROUPS;
    loop {
        let arena = Arena::new();
        let front = Front::new(&arena, unit, nodes, groups)?;
        arena.lease()?;
        let mut l = Lay::new();
        front.lay(&mut l, TO_FRONT);
        if on_grid(&arena, unit, &front, &l, groups)?.is_none() {
            groups = fewer(groups)?;
            continue;
        }
        return front.read();
    }
}

/// Runs the persistent grid over the arena at `groups` groups, laid out as `l`, in one command
/// buffer: `pp_setup` to the stage `l` names, and for the whole setup (`TO_ALL`) the expansion
/// too: `px_expand` twice and `pp_check`. The launches it made and its GPU time in seconds, or
/// `None` where a barrier failed (the grid's groups were not resident together).
pub fn on_grid(arena: &Arena, unit: &Unit, front: &Front, l: &Lay, groups: u64) -> Result<Option<(usize, f64)>, Error> {
    l.write(&front.lay);
    let all = l.get("LAST") == TO_ALL;
    let ran = run_timed(|enc| {
        front.setup(enc, arena, unit, groups)?;
        if all {
            for _ in 0..2 {
                enc.run("px_expand", None, &[arena, front.data(unit), &front.lay], &[], Grid::Groups(groups))?;
            }
            enc.run("pp_check", None, &[arena, &front.lay], &[], Grid::Groups(1))?;
        }
        Ok(())
    })?;
    Ok(if front.barrier_failed() { None } else { Some(ran) })
}

/// The groups of the next run after a barrier failed: half, and none past one.
pub fn fewer(groups: u64) -> Result<u64, Error> {
    if groups <= 1 { Err(Error::BackendFailure) } else { Ok(groups / 2) }
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
