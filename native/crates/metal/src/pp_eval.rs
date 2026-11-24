//! The preprocessor's evaluator over tapes (decision 0436), UKL passes (`pp_eval.uglykl`, decision
//! 0468 §5) launched on the process's device.
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
use crate::lane::Buffer;
use crate::pp_arena::{Arena, Bound, Buf, Enc, Lay, Scan, Sort};

/// The passes' source, UKL (`crate::pp_arena::source`).
pub fn source() -> &'static str {
    crate::pp_arena::source()
}

pub use crate::slabs::DocView;

/// A unit's files on the device: its documents, where the residency placed them, and their views.
pub struct Unit {
    /// The documents, held for the unit's life. The first one's slab is the one the kernels bind.
    pub documents: Vec<Arc<Resident>>,
    /// The unit's other slabs, which its passes make resident (`Enc::over`).
    pub slabs: Vec<Arc<crate::slabs::Slab>>,
    /// Their views, in the unit's file order (`PpFile` in the UKL): placed in each run's arena
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
    let place = move |slab: &Arc<crate::slabs::Slab>, offset: usize, table_at: TableAt| -> Result<Placed, Error> {
        let base = slab.host();
        let source_at = offset + crate::slabs::aligned(tape_len);
        // SAFETY: `insert` gave `offset..offset + bytes` of the slab to this document alone.
        let (tape, source) = unsafe { (std::slice::from_raw_parts_mut(base.add(offset), tape_len), std::slice::from_raw_parts_mut(base.add(source_at), source_len)) };
        write(tape, source);
        let p = rerail_tape_format::f16::placement(tape).map_err(|_| crate::pp_arena::failure(concat!(file!(), ":", line!())))?;
        let placed = p.view(lang, 0).map_err(|_| crate::pp_arena::failure(concat!(file!(), ":", line!())))?;
        let new_table = (!table_at.held).then(|| {
            let mut t = vec![0u8; 24 * codes];
            for (n, e) in lang.table().entries().enumerate() {
                // `DeviceCode`: kind, field (u16s), flags, class count, then room for its classes.
                t[24 * n..24 * n + 2].copy_from_slice(&e.kind.to_le_bytes()); // copy: not block data
                t[24 * n + 2..24 * n + 4].copy_from_slice(&e.field.to_le_bytes()); // copy: not block data
            }
            t
        });
        let count = |n: usize| u32::try_from(n).map_err(|_| crate::pp_arena::capacity(concat!(file!(), ":", line!())));
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
            source: None,
            view,
            at: offset,
            len: bytes,
            bytes: [crate::slabs::aligned(tape_len), crate::slabs::aligned(source_len + crate::slabs::SOURCE_MARGIN), 0],
            tape_length: tape_len,
            table,
        };
        Ok(Placed { document, table: new_table })
    };
    (digest, bytes, table, Box::new(place))
}

impl Unit {
    /// The unit of `documents` (in its file order), read where they lie: each view's offset counts
    /// from the first document's slab, by the slabs' addresses (0471). A device with no buffer
    /// addresses reads a unit in one slab only.
    pub fn of(documents: Vec<Arc<Resident>>) -> Result<Unit, Error> {
        let block = documents.first().map(|d| Arc::clone(&d.slab));
        Unit::from_slab(documents, block)
    }

    /// The unit's documents read from `base`'s first slab, the one a batch's command binds (a batch's unit,
    /// `pp_expand::expand_batch`): refused (`BackendCapacity`) where a device with no buffer addresses has them in
    /// another.
    pub fn rebased(&self, base: &Unit) -> Result<Unit, Error> {
        Unit::from_slab(self.documents.clone(), base.documents.first().map(|d| Arc::clone(&d.slab)))
    }

    /// `of`, each view's offset counting from `block`.
    fn from_slab(documents: Vec<Arc<Resident>>, block: Option<Arc<crate::slabs::Slab>>) -> Result<Unit, Error> {
        let base = block.as_ref().map(|b| b.device_buffer().device_address());
        let mut slabs: Vec<Arc<crate::slabs::Slab>> = Vec::new();
        let mut views = Vec::with_capacity(documents.len());
        for d in &documents {
            let mut view = d.view;
            if block.as_ref().is_some_and(|b| b.id != d.slab.id) {
                let (Some(Some(base)), Some(at)) = (base, d.slab.device_buffer().device_address()) else {
                    return Err(crate::pp_arena::capacity(concat!(file!(), ":", line!())));
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

    /// The batch of `units`: their files in order, each read where it lies (as `of`). `None` where
    /// a device with no buffer addresses has them in more than one slab.
    pub fn batch(units: &[&Unit]) -> Option<Unit> {
        Unit::of(units.iter().flat_map(|u| u.documents.iter().cloned()).collect()).ok()
    }

    /// The unit's files.
    pub fn len(&self) -> usize {
        self.views.len()
    }

    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    /// The first document's slab, as the kernels bind it (`None`: no documents).
    pub fn data(&self) -> Option<&Buffer> {
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

/// A directive's row (`PpDir` in the UKL): its node in its file's tape, its kind, the text tokens
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

/// The front passes' stores, and the program's own: its words (`PROG`: its phase, its tiles, the locals that cross
/// its phases, its trace; `pp_grid.uglykl`'s `PR_*`), the tiles' sums its scans and sorts share (`SUMS`), the run's
/// layout, its tiles a phase at most (`TMAX`), and a queue for a run of it alone (`on_program`).
pub struct Front {
    n: usize,
    nodes: bool,
    /// The unit's views (`PpFile`s), in the arena.
    pub files: Buf,
    pub prog: Buf,
    sums: Buf,
    pub tiles: u32,
    pub queue: Queue,
    pub lay: Buf,
    pub bases: Buf,
    pub dir_bases: Buf,
    // The directives, a word a node: the row, group and arm flags, the depth events, their scans.
    dflag: Buf,
    gflag: Buf,
    aflag: Buf,
    dev: Buf,
    drank: Buf,
    grank: Buf,
    arank: Buf,
    depth: Buf,
    dev_scan: Scan,
    group_scan: Scan,
    arm_scan: Scan,
    // The arms' and the groups' keys (depth, place), sorted; a file's guard marks.
    arm_keys: Buf,
    arm_sort: Sort,
    group_keys: Buf,
    group_sort: Sort,
    g_out: Buf,
    g_arms: Buf,
    // The text map, a word a node: each node's owner (a max scan), its text flag and rank, and
    // each text token's node.
    owner_in: Buf,
    heads: Buf,
    owner: Buf,
    tflag: Buf,
    trank: Buf,
    tscan: Scan,
    pub text_nodes: Buf,
    groups: Buf,
    guarded: Buf,
    guards: Buf,
    bad: Buf,
    pub keys: Buf,
    out: Buf,
    pub rows: Buf,
    pub dir_keys: Buf,
    /// Each directive's pragma act (`A_*`: `once`, `push_macro`, `pop_macro`; decision 0467 §3).
    pub dir_acts: Buf,
    scan: Scan,
    dir_scan: Scan,
}

/// The stages a unit's program ends after (`LAST`): the front, the definitions, or the whole
/// setup with P1's iterations and the text inside them (`TO_VIS`), the check after it (`TO_ALL`).
pub const TO_FRONT: u32 = 1;
pub const TO_DEFS: u32 = 2;
pub const TO_VIS: u32 = 3;
pub const TO_ALL: u32 = 4;

impl Front {
    /// Its tiles a phase at most from its nodes (`pp_arena::tiles_for`; a run's `TMAX` may be more, for its
    /// expansion's tokens).
    pub fn new(arena: &Arena, unit: &Unit, nodes: bool) -> Result<Front, Error> {
        let n = unit.len();
        let per_file = || arena.zeroed::<u32>(n);
        let per_node = || arena.zeroed::<u32>(unit.node_bound);
        // Each file's first node and the nodes in all: the views' counts, which the host holds
        // (decision 0468: no scan on the device for what the layout says).
        let mut bases = Vec::with_capacity(n.max(1));
        let mut total = 0u32;
        for &k in &unit.file_nodes {
            bases.push(total);
            total += k;
        }
        if bases.is_empty() {
            bases.push(0);
        }
        Ok(Front {
            n,
            nodes,
            files: arena.from_slice(&unit.views)?,
            prog: arena.from_slice(&prog_start())?,
            sums: arena.zeroed::<u32>(4 * crate::pp_arena::TILES_MOST as usize + 4)?,
            tiles: crate::pp_arena::tiles_for(unit.node_bound),
            queue: Queue::new(arena, &[crate::pp_arena::TILES_MOST])?,
            lay: arena.zeroed::<u32>(crate::pp_arena::SLOTS.len())?,
            bases: arena.from_slice(&bases)?,
            dir_bases: per_file()?,
            dflag: per_node()?,
            gflag: per_node()?,
            aflag: per_node()?,
            dev: per_node()?,
            drank: per_node()?,
            grank: per_node()?,
            arank: per_node()?,
            depth: per_node()?,
            dev_scan: Scan::new(arena)?,
            group_scan: Scan::new(arena)?,
            arm_scan: Scan::new(arena)?,
            arm_keys: arena.zeroed::<u64>(unit.dir_bound)?,
            arm_sort: Sort::new(arena, unit.dir_bound)?,
            group_keys: arena.zeroed::<u64>(unit.dir_bound)?,
            group_sort: Sort::new(arena, unit.dir_bound)?,
            g_out: per_file()?,
            g_arms: per_file()?,
            owner_in: per_node()?,
            heads: per_node()?,
            owner: per_node()?,
            tflag: per_node()?,
            trank: per_node()?,
            tscan: Scan::new(arena)?,
            text_nodes: per_node()?,
            groups: per_file()?,
            guarded: per_file()?,
            guards: arena.zeroed::<u64>(n)?,
            bad: arena.zeroed::<u32>(1)?,
            keys: arena.zeroed::<u64>(unit.node_bound)?,
            out: arena.zeroed::<[u32; 4]>(if nodes { unit.node_bound } else { 1 })?,
            rows: arena.zeroed::<PpDir>(unit.dir_bound)?,
            dir_keys: arena.zeroed::<u64>(unit.dir_bound)?,
            dir_acts: arena.zeroed::<u32>(unit.dir_bound)?,
            scan: Scan { total: arena.from_slice(&[total])? },
            dir_scan: Scan::new(arena)?,
        })
    }

    /// The front's slots of the layout, and the run's words: the stage it ends after.
    pub fn lay(&self, l: &mut Lay, last: u32) {
        l.at("PROG", &self.prog).at("SUMS", &self.sums).at("FILES", &self.files).at("BASES", &self.bases);
        l.at("TOTAL", &self.scan.total).at("DIR_BASES", &self.dir_bases).at("DIR_TOTAL", &self.dir_scan.total);
        l.at("DFLAG", &self.dflag).at("GFLAG", &self.gflag).at("AFLAG", &self.aflag).at("DEV", &self.dev).at("DRANK", &self.drank);
        l.at("GRANK", &self.grank).at("ARANK", &self.arank).at("NDEPTH", &self.depth).at("DEV_TOTAL", &self.dev_scan.total);
        l.at("N_GROUPS", &self.group_scan.total).at("N_ARMS", &self.arm_scan.total).at("ARM_KEYS", &self.arm_keys).at("GRP_KEYS", &self.group_keys);
        l.at("ASORT_KB", &self.arm_sort.keys_b).at("ASORT_VA", &self.arm_sort.vals_a).at("ASORT_VB", &self.arm_sort.vals_b).at("ASORT_HIST", &self.arm_sort.hist);
        l.at("PSORT_KB", &self.group_sort.keys_b).at("PSORT_VA", &self.group_sort.vals_a).at("PSORT_VB", &self.group_sort.vals_b).at("PSORT_HIST", &self.group_sort.hist);
        l.at("G_OUT", &self.g_out).at("G_ARMS", &self.g_arms);
        l.at("OWNER_IN", &self.owner_in).at("VHEADS", &self.heads).at("OWNER", &self.owner).at("TFLAG", &self.tflag).at("TRANK", &self.trank);
        l.at("TTOTAL", &self.tscan.total).at("TEXT_NODES", &self.text_nodes);
        l.at("DIR_GROUPS", &self.groups).at("GUARDED", &self.guarded).at("GUARDS", &self.guards).at("BAD", &self.bad).at("KEYS", &self.keys);
        l.at("NODES_OUT", &self.out).at("ROWS", &self.rows).at("DIR_KEYS", &self.dir_keys).at("DIR_ACTS", &self.dir_acts);
        l.word("N_FILES", self.n as u32).word("NODES_FLAG", u32::from(self.nodes)).word("LAST", last).word("TMAX", self.tiles);
        if std::env::var_os("RERAIL_PP_TRACE").is_some() {
            l.word("STOP", u32::MAX);
        }
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

    /// The program's words (after a run).
    fn prog_words(&self) -> &[u32] {
        // SAFETY: the device finished writing the program's words; the store holds `PR_WORDS`.
        unsafe { std::slice::from_raw_parts(self.prog.host().cast::<u32>(), PR_WORDS) }
    }

    /// Whether the run's program ended (after it).
    pub fn ended(&self) -> bool {
        self.prog_words()[PR_END] != 0
    }

    /// The phases the run's program entered (after it).
    pub fn phases(&self) -> u32 {
        self.prog_words()[PR_PHASES]
    }

    /// The trace a run at `STOP` = `PP_TRACE_ON` wrote (after it): each phase's source line and device clock (ns).
    pub fn trace(&self) -> Vec<(u32, u64)> {
        self.prog_words()[PR_TRACE..].chunks(3).take_while(|c| c[0] != 0).map(|c| (c[0], u64::from(c[1]) | (u64::from(c[2]) << 32))).collect()
    }

    /// The results, read back for tests (after the command buffer is done).
    pub fn read(&self) -> Result<Evaluated, Error> {
        let n = self.n;
        // SAFETY: the device finished writing every buffer; each holds the counts read here.
        unsafe {
            if *self.bad.host().cast::<u32>() != 0 {
                return Err(crate::pp_arena::failure(concat!(file!(), ":", line!())));
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

/// Encodes `f`'s launches in one command, commits it and waits: the launches it made.
pub fn run<'a>(f: impl FnOnce(&Enc<'a>) -> Result<(), Error>) -> Result<usize, Error> {
    run_timed(f).map(|(launches, _)| launches)
}

/// `run`, and the command's time on the device, in seconds.
pub fn run_timed<'a>(f: impl FnOnce(&Enc<'a>) -> Result<(), Error>) -> Result<(usize, f64), Error> {
    run_timed_over(&[], f)
}

/// `run_timed`, each launch making `resident` resident: a unit's slabs past its first (0471).
pub fn run_timed_over<'a>(resident: &'a [Arc<crate::slabs::Slab>], f: impl FnOnce(&Enc<'a>) -> Result<(), Error>) -> Result<(usize, f64), Error> {
    let scope = crate::launch::LaunchScope::enter();
    let enc = Enc::over("Rerail pp evaluator", resident);
    f(&enc)?;
    let seconds = enc.finish()?;
    // R9 of decision 0448: every pass in this one command, so the host reads nothing between
    // passes (it reads only after the whole command completed). An encoding that put a launch in
    // another command is refused.
    let records = scope.snapshot();
    if records.iter().any(|r| r.command != records[0].command) {
        return Err(crate::pp_arena::failure(concat!(file!(), ":", line!())));
    }
    Ok((records.len(), seconds))
}

/// The front passes over `unit`, read back (`nodes`: each node as the device's reader gives it).
pub fn evaluate(unit: &Unit, nodes: bool) -> Result<Evaluated, Error> {
    preempted_again(|| evaluate_once(unit, nodes))
}

fn evaluate_once(unit: &Unit, nodes: bool) -> Result<Evaluated, Error> {
    let arena = Arena::new();
    let front = Front::new(&arena, unit, nodes)?;
    arena.lease()?;
    let mut l = Lay::new();
    front.lay(&mut l, TO_FRONT);
    on_unit(&arena, unit, &front, &l)?;
    front.read()
}

/// The program's words (`pp_grid.uglykl`'s `PR_*`): its tiles, whether it ended, the phases it entered, and its
/// trace's first word; the words in all.
const PR_T: usize = 1;
const PR_END: usize = 3;
const PR_PHASES: usize = 7;
const PR_TRACE: usize = 80;
const PR_WORDS: usize = PR_TRACE + 3 * TRACE_N;

/// A program's words before it runs: at its first phase, of one tile (the one its queue holds).
fn prog_start() -> Vec<u32> {
    let mut w = vec![0u32; PR_WORDS];
    w[PR_T] = 1;
    w
}

/// A batch's queue (`pp_grid.uglykl`'s `pp_run`, decision 0653): its words (`PQ_*`), each unit's layout's offset in
/// the arena, and the ring's slots (`pp_arena::queue_slots`).
pub struct Queue {
    buf: Buf,
    units: usize,
    slots: usize,
}

/// The queue's words (`pp_grid.uglykl`'s `PQ_*`).
const PQ_END: usize = 1;
const PQ_CAP: usize = 3;
const PQ_CTL: usize = 4;
const PQ_BUDGET: usize = 9;
const PQ_LAYS: usize = 16;
/// An item's tile bits (`PP_TILE_BITS`): an item is a unit's index above them.
const PP_TILE_BITS: u32 = 12;

impl Queue {
    /// A queue for units of `tiles` tiles a phase at most each.
    pub fn new(arena: &Arena, tiles: &[u32]) -> Result<Queue, Error> {
        let slots = crate::pp_arena::queue_slots(tiles);
        Ok(Queue { buf: arena.zeroed::<u32>(PQ_LAYS + tiles.len() + slots)?, units: tiles.len(), slots })
    }

    /// Its words for a run of the units laid out at `lays` (after the arena is leased): each unit's first phase's
    /// one tile queued.
    fn start(&self, lays: &[u32]) {
        assert!(lays.len() <= self.units && lays.len() < 1 << (32 - PP_TILE_BITS));
        let n = lays.len();
        // SAFETY: the store holds `PQ_LAYS + units + slots` words, and no command reads it yet.
        let w = unsafe { std::slice::from_raw_parts_mut(self.buf.host().cast::<u32>(), PQ_LAYS + self.units + self.slots) };
        w.fill(0);
        w[0] = n as u32;
        w[PQ_END] = crate::pp_arena::BUDGET;
        w[PQ_CAP] = self.slots as u32;
        // Head 0, tail and active the units' first tiles, no overflow, the ring's base 0.
        w[PQ_CTL + 1] = n as u32;
        w[PQ_CTL + 2] = n as u32;
        w[PQ_BUDGET] = crate::pp_arena::BUDGET;
        w[PQ_LAYS..PQ_LAYS + n].copy_from_slice(lays); // copy: not block data (the units' layouts' offsets)
        let slots = PQ_LAYS + n;
        for u in 0..n {
            w[slots + u] = ((u as u32) << PP_TILE_BITS) + 1;
        }
    }

    /// Whether tiles are queued (after a command).
    fn left(&self) -> bool {
        // SAFETY: the device finished writing the queue's words.
        let w = unsafe { std::slice::from_raw_parts(self.buf.host().cast::<u32>(), PQ_LAYS) };
        let end = w[PQ_CTL + 4].wrapping_add(self.slots as u32);
        w[PQ_CTL + 1].min(end) > w[PQ_CTL]
    }
}

/// The commands a batch's programs take at most (`on_program`): each `PP_LAUNCHES` launches of its budget.
const COMMANDS: usize = 1 << 12;

/// One unit's program over the arena laid out as `l` (`on_program`, its front's queue).
pub fn on_unit(arena: &Arena, unit: &Unit, front: &Front, l: &Lay) -> Result<(usize, f64), Error> {
    on_program(arena, &front.queue, &[(front, l)], front.data(unit), &unit.slabs)
}

/// A batch's units' programs over the arena (decision 0653): each unit's stores laid out as its `Lay` (written into its
/// front's slots), its setup to the stage its layout names (the text pass is inside its P1 iterations, decision
/// 0485), on one queue (`queue`, for at least these units), in commands of `PP_LAUNCHES` launches of `pp_run`, a
/// `pp_rebase` after each, while a program has tiles queued. `data` is the slab the units' views count from,
/// `resident` their other slabs. The launches of a command, and the commands' GPU time in seconds. R9 of decision
/// 0448 holds in each command. A program that did not end (its tiles past the ring's room, or the commands') is
/// refused `X_QUEUE` where it runs to the whole setup, else a failure.
pub fn on_program<'a>(arena: &'a Arena, queue: &'a Queue, units: &[(&Front, &Lay)], data: &'a dyn crate::pp_arena::Bound, resident: &'a [Arc<crate::slabs::Slab>]) -> Result<(usize, f64), Error> {
    let mut lays = Vec::with_capacity(units.len());
    for (front, l) in units {
        l.write(&front.lay);
        lays.push(u32::try_from(front.lay.offset()).map_err(|_| crate::pp_arena::capacity(concat!(file!(), ":", line!())))?);
    }
    queue.start(&lays);
    let workers = crate::pp_arena::workers();
    let (mut launches, mut gpu, mut commands) = (0, 0f64, 0);
    for _ in 0..COMMANDS {
        commands += 1;
        let ran = run_timed_over(resident, |enc| {
            for _ in 0..crate::pp_arena::PP_LAUNCHES {
                enc.run("pp_run", &[("arena", arena), ("data", data), ("queue", &queue.buf)], None, workers);
                enc.run("pp_rebase", &[("queue", &queue.buf)], None, 1);
            }
            Ok(())
        })?;
        (launches, gpu) = (ran.0, gpu + ran.1);
        if units.iter().all(|(f, _)| f.ended()) || !queue.left() {
            break;
        }
    }
    // `RERAIL_PP_PROGRAM`: the commands the programs took, and each one's phases entered.
    if std::env::var_os("RERAIL_PP_PROGRAM").is_some() {
        let phases: Vec<String> = units.iter().map(|(f, _)| format!("{}{}", f.phases(), if f.ended() { "" } else { " (not ended)" })).collect();
        eprintln!("PROGRAM {} units, {commands} commands, gpu {:.1} us; phases {}", units.len(), gpu * 1e6, phases.join(", "));
    }
    for (front, l) in units {
        if front.ended() {
            continue;
        }
        if l.get("LAST") != TO_ALL {
            return Err(crate::pp_arena::failure(concat!(file!(), ":", line!())));
        }
        // SAFETY: the commands are done; `CTL` holds the refusals' word at 2.
        unsafe { *arena.host_at(l.get("CTL") as usize).cast::<u32>().add(2) |= crate::pp_expand::X_QUEUE };
    }
    if std::env::var_os("RERAIL_PP_TRACE").is_some() && units.len() == 1 && units[0].1.get("STOP") == u32::MAX {
        report_trace(&units[0].0.trace(), gpu);
    }
    Ok((launches, gpu))
}

/// The trace's entries (`pp_grid.uglykl`'s `PP_TRACE_N`).
const TRACE_N: usize = 2048;

/// A trace's report: the phases' time by source line (where the device has a clock), most first, and the lines'
/// text in the evaluator's source.
pub(crate) fn report_trace(t: &[(u32, u64)], gpu: f64) {
    let lines: Vec<&str> = crate::pp_arena::source().lines().collect();
    let text = |l: u32| lines.get(l as usize - 1).map_or("", |s| s.trim()).chars().take(110).collect::<String>();
    let mut by: std::collections::BTreeMap<u32, (u64, u32)> = Default::default();
    for w in t.windows(2) {
        let e = by.entry(w[0].0).or_default();
        e.0 += w[1].1.saturating_sub(w[0].1);
        e.1 += 1;
    }
    let span = t.last().map_or(0, |l| l.1).saturating_sub(t.first().map_or(0, |f| f.1));
    eprintln!("TRACE {} phases, gpu {:.1} us, traced span {:.1} us", t.len(), gpu * 1e6, span as f64 / 1e3);
    let mut v: Vec<_> = by.into_iter().collect();
    v.sort_by_key(|(_, (ns, _))| std::cmp::Reverse(*ns));
    for (line, (ns, n)) in v.iter().take(40) {
        eprintln!("TRACE {:8.1} us {:5.1}% x{n:<4} L{line}: {}", *ns as f64 / 1e3, 100.0 * *ns as f64 / span.max(1) as f64, text(*line));
    }
    if std::env::var_os("RERAIL_PP_TRACE_ALL").is_some() {
        for w in t.windows(2) {
            eprintln!("TRACESEQ L{} {:.2} us", w[0].0, (w[1].1.saturating_sub(w[0].1)) as f64 / 1e3);
        }
    }
}

/// `run`, again where the system stopped it (`BackendPreempted`: macOS's GPU watchdog, the display waiting while
/// other work holds the GPU), at most `PREEMPTED_RUNS` times in all; past them it is a failure. Any other failure is
/// the caller's.
pub fn preempted_again<T>(mut run: impl FnMut() -> Result<T, Error>) -> Result<T, Error> {
    let mut stopped = 0;
    loop {
        match run() {
            Err(Error::BackendPreempted) if stopped + 1 < PREEMPTED_RUNS => stopped += 1,
            Err(Error::BackendPreempted) => return Err(crate::pp_arena::failure(concat!(file!(), ":", line!(), ": stopped by the system each run"))),
            r => return r,
        }
    }
}

/// The runs of a unit the system may stop before it is a failure (`preempted_again`).
pub const PREEMPTED_RUNS: u32 = 4;

/// A name's key, as `pp_eval.uglykl`'s `name_key` makes it: its bytes 8 at a time, line splices
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

/// Two keys made one (`pp_eval.uglykl`'s `key_mix`).
pub fn key_mix(h: u64, w: u64) -> u64 {
    let h = (h ^ w).wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^ (h >> 32)
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
