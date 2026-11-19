//! Macro expansion on the device (decision 0448), `pp_expand.metal`. Built so far: the setup's
//! definitions (S1, S2, S4), read from the tapes in place.

use crate::Error;
use crate::pp_blocks::{Enc, Grid, Scan, SegOp, SegScan, Sort, TG};
use metal::BufferRef;
use crate::pp_eval::{Front, Unit, name_key};
use crate::runtime::MappedBuffer;

/// A definition (`PpDef` in the MSL): the unit's nodes of its name, its `params` node (0: none)
/// and its body's first leaf; the leaves of each; its flags; its first parameter slot and item.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PpDef {
    pub name: u32,
    pub params: u32,
    pub n_params: u32,
    pub body: u32,
    pub n_body: u32,
    pub flags: u32,
    pub pfirst: u32,
    pub ifirst: u32,
}

/// `PpDef::flags`.
pub const D_DEFINE: u32 = 1;
pub const D_FNLIKE: u32 = 2;
pub const D_VARIADIC: u32 = 4;
pub const D_BAD: u32 = 8;
/// An item's parameter where it is none, and its flags above it.
pub const NO_PARAM: u32 = 0xffff;
pub const I_PASTE: u32 = 1 << 16;
pub const I_SPACE: u32 = 1 << 17;

/// The setup's definitions on the device: one a directive row, each parameter list's leaves'
/// keys (a parameter's at its leaf, 0 elsewhere), and each body leaf's item.
pub struct Defs {
    pub defs: MappedBuffer,
    pcount: MappedBuffer,
    bcount: MappedBuffer,
    pfirst: MappedBuffer,
    ifirst: MappedBuffer,
    pscan: Scan,
    bscan: Scan,
    row_args: MappedBuffer,
    pargs: MappedBuffer,
    bargs: MappedBuffer,
    pub pkeys: MappedBuffer,
    pub items: MappedBuffer,
    /// Set where a store's room ran out (decision 0448 §2): the unit runs again at the next rung.
    pub room: MappedBuffer,
    cap: u32,
}

/// The rungs of the setup's stores: rung `k` holds `nodes >> (RUNGS - 1 - k)` leaves, so the last
/// holds every node of the unit, the most there can be.
pub const RUNGS: u32 = 4;

impl Defs {
    /// Room for `unit`'s rows, and at rung `rung` for their parameters' and bodies' leaves: each
    /// a node, so the last rung's, the unit's nodes, bounds them.
    pub fn new(unit: &Unit, rung: u32) -> Result<Defs, Error> {
        let device = crate::device()?;
        let rows = unit.dir_bound;
        let nodes = (unit.node_bound >> (RUNGS - 1 - rung.min(RUNGS - 1))).max(256);
        let words = |k: usize| MappedBuffer::zeroed::<u32>(&device, k.max(1));
        Ok(Defs {
            defs: MappedBuffer::zeroed::<PpDef>(&device, rows)?,
            pcount: words(rows)?,
            bcount: words(rows)?,
            pfirst: words(rows)?,
            ifirst: words(rows)?,
            pscan: Scan::new(rows)?,
            bscan: Scan::new(rows)?,
            row_args: words(4)?,
            pargs: words(4)?,
            bargs: words(4)?,
            pkeys: MappedBuffer::zeroed::<u64>(&device, nodes)?,
            items: words(nodes)?,
            room: words(1)?,
            cap: nodes as u32,
        })
    }

    /// S1 (`pd_defs`, one thread a row), the scans of the counts, S2 (`pd_params`, one thread a
    /// parameter list's leaf) and S4 (`pd_items`, one thread a body leaf).
    pub fn encode(&self, enc: &Enc<'_>, unit: &Unit, front: &Front) -> Result<(), Error> {
        let n32 = unit.table.len() as u32;
        let (u_files, u_data) = (&unit.files.buffer as &metal::BufferRef, &unit.data.buffer as &metal::BufferRef);
        let dir_total = &front.dir_total().buffer;
        let row_args = &self.row_args;
        enc.run("pb_args", None, &[dir_total, &row_args.buffer], &[0], Grid::Groups(1))?;
        enc.run(
            "pd_defs",
            None,
            &[u_files, u_data, &front.bases.buffer, &front.dir_bases.buffer, dir_total, &front.rows.buffer, &self.defs.buffer, &self.pcount.buffer, &self.bcount.buffer],
            &[n32],
            Grid::Indirect(&row_args.buffer, 0),
        )?;
        self.pscan.encode(enc, dir_total, 0, &self.pcount.buffer, &self.pfirst.buffer)?;
        self.bscan.encode(enc, dir_total, 0, &self.bcount.buffer, &self.ifirst.buffer)?;
        enc.run("pb_args", None, &[&self.pscan.total.buffer, &self.pargs.buffer], &[0], Grid::Groups(1))?;
        enc.run("pb_args", None, &[&self.bscan.total.buffer, &self.bargs.buffer], &[0], Grid::Groups(1))?;
        let va = name_key(b"__VA_ARGS__");
        enc.run(
            "pd_params",
            None,
            &[u_files, u_data, &front.bases.buffer, &front.keys.buffer, &front.dir_bases.buffer, dir_total, &self.defs.buffer, &self.pfirst.buffer, &self.pscan.total.buffer, &self.pkeys.buffer, &self.room.buffer],
            &[n32, va as u32, (va >> 32) as u32, self.cap],
            Grid::Indirect(&self.pargs.buffer, 0),
        )?;
        enc.run(
            "pd_items",
            None,
            &[u_files, u_data, &front.bases.buffer, &front.keys.buffer, &front.dir_bases.buffer, dir_total, &self.defs.buffer, &self.ifirst.buffer, &self.bscan.total.buffer, &self.pkeys.buffer, &self.items.buffer, &self.room.buffer],
            &[n32, self.cap],
            Grid::Indirect(&self.bargs.buffer, 0),
        )?;
        Ok(())
    }
}

/// The setup's definitions, read back for tests.
pub struct DefsRead {
    pub defs: Vec<PpDef>,
    pub pkeys: Vec<u64>,
    pub items: Vec<u32>,
}

/// The front passes and the definitions over `unit`, in one command buffer, read back: from
/// rung 0, and at the next where a store's room ran out.
pub fn definitions(unit: &Unit) -> Result<(crate::pp_eval::Evaluated, DefsRead), Error> {
    definitions_from(unit, 0)
}

/// `definitions` from rung `rung`.
pub fn definitions_from(unit: &Unit, rung: u32) -> Result<(crate::pp_eval::Evaluated, DefsRead), Error> {
    let front = Front::new(unit, false)?;
    let defs = Defs::new(unit, rung)?;
    crate::pp_eval::run(|enc| {
        front.encode(enc, unit)?;
        defs.encode(enc, unit, &front)
    })?;
    // SAFETY: the device finished writing `room`.
    if unsafe { *defs.room.host().cast::<u32>() } != 0 {
        if rung + 1 >= RUNGS {
            return Err(Error::BackendCapacity);
        }
        drop((front, defs));
        return definitions_from(unit, rung + 1);
    }
    let ev = front.read()?;
    let rows = ev.rows.len();
    // SAFETY: the device finished; each buffer holds at least the counts read.
    let (pt, bt) = unsafe { (*defs.pscan.total.host().cast::<u32>() as usize, *defs.bscan.total.host().cast::<u32>() as usize) };
    let read = unsafe {
        DefsRead {
            defs: std::slice::from_raw_parts(defs.defs.host().cast::<PpDef>(), rows).to_vec(),
            pkeys: std::slice::from_raw_parts(defs.pkeys.host().cast::<u64>(), pt).to_vec(),
            items: std::slice::from_raw_parts(defs.items.host().cast::<u32>(), bt).to_vec(),
        }
    };
    Ok((ev, read))
}

/// An instance of a file in the unit's layout (`PpInst` in the MSL): its file, its first
/// directive and its first stretch in `Layout`'s arrays.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PpInst {
    pub file: u32,
    pub dir_first: u32,
    pub gap_first: u32,
}

/// The unit's include tree laid out: P1's answer, the test input of 0448 until P1's includes
/// and conditionals run on the device. By instance, in the unit's order; by directive and by
/// stretch (a file's `k`-th, before its directive `k`; the last after them all) of each
/// instance, its slot and whether it is active.
pub struct Layout {
    pub insts: Vec<PpInst>,
    pub dir_slots: Vec<u32>,
    pub dir_active: Vec<u32>,
    pub gap_slots: Vec<u32>,
    pub gap_active: Vec<u32>,
    /// The built-in macros' names' keys: rows at slot 0.
    pub builtins: Vec<u64>,
    /// The dialect's options as the paste relexer reads them (`relex_options`).
    pub relex_options: u32,
    /// Each file's `__FILE__`: the string literal of its presumed path (its `"` and `\` escaped),
    /// by the unit's file index; then `__DATE__`'s, `__TIME__`'s and `__TIMESTAMP__`'s.
    pub file_literals: Vec<Vec<u8>>,
}

/// What a lookup answers where it is no definition's row (`pp_expand.metal`'s `L_*`).
pub const L_NOT_NAME: u32 = 0xffff_ffff;
pub const L_NONE: u32 = 0xffff_fffe;
pub const L_BUILTIN: u32 = 0xffff_fffd;
/// The built-ins the device expands to one token, and those that are plain names in text
/// (`__has_include`, `__has_include_next`, `__has_embed`); a definition's row is below `L_IDENT`.
pub const L_FILE: u32 = 0xffff_fffc;
pub const L_LINE: u32 = 0xffff_fffb;
pub const L_DATE: u32 = 0xffff_fffa;
pub const L_TIME: u32 = 0xffff_fff9;
pub const L_TIMESTAMP: u32 = 0xffff_fff8;
pub const L_IDENT: u32 = 0xffff_fff7;

/// A built-in's lookup value, by its name.
fn builtin_value(key: u64) -> u32 {
    let named: [(&[u8], u32); 8] = [
        (b"__LINE__", L_LINE),
        (b"__FILE__", L_FILE),
        (b"__DATE__", L_DATE),
        (b"__TIME__", L_TIME),
        (b"__TIMESTAMP__", L_TIMESTAMP),
        (b"__has_include", L_IDENT),
        (b"__has_include_next", L_IDENT),
        (b"__has_embed", L_IDENT),
    ];
    named.iter().find(|(n, _)| name_key(n) == key).map_or(L_BUILTIN, |&(_, v)| v)
}

/// S6–S8 and the lookups (R4): the text map, the visibility rows and their sort, the active
/// stretches, and each stretch token's meaning where it stands.
pub struct Vis {
    // The layout, on the device.
    insts: MappedBuffer,
    n_insts: MappedBuffer,
    dslot: MappedBuffer,
    dactive: MappedBuffer,
    n_dirs: MappedBuffer,
    gslot: MappedBuffer,
    gactive: MappedBuffer,
    pub(crate) n_gaps: MappedBuffer,
    n_builtin: u32,
    nd: usize,
    ng: usize,
    // The text map: a word a node of the unit.
    owner_in: MappedBuffer,
    heads: MappedBuffer,
    owner: MappedBuffer,
    owner_scan: SegScan,
    tflag: MappedBuffer,
    trank: MappedBuffer,
    tscan: Scan,
    pub(crate) text_nodes: MappedBuffer,
    // The rows: a directive of the layout each, after the built-ins'.
    rflag: MappedBuffer,
    rat: MappedBuffer,
    rscan: Scan,
    pub(crate) sort_keys: MappedBuffer,
    pub(crate) name_keys: MappedBuffer,
    pub(crate) values: MappedBuffer,
    pub(crate) n_rows: MappedBuffer,
    pub(crate) sort: Sort,
    // The stretches: a word each, and a lookup a text token.
    gcounts: MappedBuffer,
    pub(crate) gfirsts: MappedBuffer,
    /// The stretches in the unit's order: their keys (slots), sorted with their places, and
    /// each one's slot in that order.
    gkeys: MappedBuffer,
    gsort: Sort,
    pub(crate) sslot: MappedBuffer,
    pub(crate) gat: MappedBuffer,
    pub(crate) gscan: Scan,
    pub(crate) targs: MappedBuffer,
    pub lookups: MappedBuffer,
    pub bad: MappedBuffer,
}

/// The top level's tokens at most: each instance's file's nodes.
pub fn top_bound(unit: &Unit, lay: &Layout) -> usize {
    lay.insts.iter().map(|i| unit.file_nodes[i.file as usize] as usize).sum::<usize>().max(1)
}

const NO_INST: [PpInst; 1] = [PpInst { file: 0, dir_first: 0, gap_first: 0 }];

impl Vis {
    pub fn new(unit: &Unit, lay: &Layout) -> Result<Vis, Error> {
        let device = crate::device()?;
        let words = |v: &[u32]| MappedBuffer::from_slice(&device, if v.is_empty() { &[0] } else { v });
        let zeroed = |k: usize| MappedBuffer::zeroed::<u32>(&device, k.max(1));
        let nodes = unit.node_bound;
        let top = top_bound(unit, lay);
        let (nd, ng, nb) = (lay.dir_slots.len(), lay.gap_slots.len(), lay.builtins.len());
        let rows = nd + nb;
        // The built-ins' rows, at slot 0: the configuration's, written before the passes.
        let mut sk = vec![0u64; rows.max(1)];
        let mut nk = vec![0u64; rows.max(1)];
        let mut vals = vec![0u32; rows.max(1)];
        for (k, &key) in lay.builtins.iter().enumerate() {
            sk[k] = key & 0xffff_ffff_0000_0000;
            nk[k] = key;
            vals[k] = builtin_value(key);
        }
        Ok(Vis {
            insts: MappedBuffer::from_slice(&device, if lay.insts.is_empty() { &NO_INST } else { &lay.insts })?,
            n_insts: words(&[lay.insts.len() as u32])?,
            dslot: words(&lay.dir_slots)?,
            dactive: words(&lay.dir_active)?,
            n_dirs: words(&[nd as u32])?,
            gslot: words(&lay.gap_slots)?,
            gactive: words(&lay.gap_active)?,
            n_gaps: words(&[ng as u32])?,
            n_builtin: nb as u32,
            nd,
            ng,
            owner_in: zeroed(nodes)?,
            heads: zeroed(nodes)?,
            owner: zeroed(nodes)?,
            owner_scan: SegScan::new(nodes)?,
            tflag: zeroed(nodes)?,
            trank: zeroed(nodes)?,
            tscan: Scan::new(nodes)?,
            text_nodes: zeroed(nodes)?,
            rflag: zeroed(nd)?,
            rat: zeroed(nd)?,
            rscan: Scan::new(nd)?,
            sort_keys: MappedBuffer::from_slice(&device, &sk)?,
            name_keys: MappedBuffer::from_slice(&device, &nk)?,
            values: MappedBuffer::from_slice(&device, &vals)?,
            n_rows: zeroed(1)?,
            sort: Sort::new(rows)?,
            gcounts: zeroed(ng)?,
            gfirsts: zeroed(ng)?,
            gkeys: MappedBuffer::zeroed::<u64>(&device, ng.max(1))?,
            gsort: Sort::new(ng)?,
            sslot: zeroed(ng)?,
            gat: zeroed(ng)?,
            gscan: Scan::new(ng)?,
            targs: zeroed(4)?,
            lookups: zeroed(top)?,
            bad: zeroed(1)?,
        })
    }

    /// The passes, each one thread an element:
    /// - the text map: `pe_owner_in` and a max scan (a node each), `pe_text_flag`, a scan,
    ///   `pe_text_nodes`;
    /// - the rows: `pe_row_flag` (a directive each), a scan, `pe_rows`, `pe_rows_total`, one
    ///   stable sort by (the key's upper half, slot);
    /// - the stretches: `pe_gap_counts` (a stretch each), a scan;
    /// - `pe_lookups`, a stretch token each, sized by that scan.
    pub fn encode(&self, enc: &Enc<'_>, unit: &Unit, front: &Front, defs: &Defs) -> Result<(), Error> {
        let n32 = unit.table.len() as u32;
        let (u_files, u_data) = (&unit.files.buffer as &BufferRef, &unit.data.buffer as &BufferRef);
        let total = &front.total().buffer;
        let dir_total = &front.dir_total().buffer;
        let by_node = Grid::Indirect(&front.node_args().buffer, 0);
        enc.run("pe_owner_in", None, &[u_files, u_data, &front.bases.buffer, total, &self.owner_in.buffer], &[n32], by_node)?;
        self.owner_scan.encode(enc, SegOp::Max, total, 0, &self.owner_in.buffer, &self.heads.buffer, &self.owner.buffer)?;
        enc.run("pe_text_flag", None, &[u_files, u_data, &front.bases.buffer, total, &self.owner.buffer, &self.tflag.buffer], &[n32], by_node)?;
        self.tscan.encode(enc, total, 0, &self.tflag.buffer, &self.trank.buffer)?;
        enc.run("pe_text_nodes", None, &[&self.tflag.buffer, &self.trank.buffer, total, &self.text_nodes.buffer], &[], by_node)?;
        let by_dir = Grid::Groups((self.nd as u64).div_ceil(TG));
        enc.run(
            "pe_row_flag",
            None,
            &[&self.insts.buffer, &self.n_insts.buffer, &self.dactive.buffer, &self.n_dirs.buffer, &front.dir_bases.buffer, &front.rows.buffer, &defs.defs.buffer, &front.dir_keys.buffer, &self.rflag.buffer],
            &[],
            by_dir,
        )?;
        self.rscan.encode(enc, &self.n_dirs.buffer, 0, &self.rflag.buffer, &self.rat.buffer)?;
        enc.run(
            "pe_rows",
            None,
            &[
                &self.insts.buffer,
                &self.n_insts.buffer,
                &self.dslot.buffer,
                &self.n_dirs.buffer,
                &front.dir_bases.buffer,
                &front.rows.buffer,
                &front.dir_keys.buffer,
                &self.rflag.buffer,
                &self.rat.buffer,
                &self.sort_keys.buffer,
                &self.name_keys.buffer,
                &self.values.buffer,
            ],
            &[self.n_builtin],
            by_dir,
        )?;
        enc.run("pe_rows_total", None, &[&self.rscan.total.buffer, &self.n_rows.buffer], &[self.n_builtin], Grid::Groups(1))?;
        self.sort.encode(enc, &self.n_rows.buffer, 0, &self.sort_keys.buffer, 64)?;
        let by_gap = Grid::Groups((self.ng as u64).div_ceil(TG));
        enc.run("pe_gap_keys", None, &[&self.gslot.buffer, &self.n_gaps.buffer, &self.gkeys.buffer], &[], by_gap)?;
        // 32 bits sort in an even number of passes: the keys end in place, their places in `vals_a`.
        debug_assert!(Sort::in_input(32));
        self.gsort.encode(enc, &self.n_gaps.buffer, 0, &self.gkeys.buffer, 32)?;
        enc.run(
            "pe_gap_counts",
            None,
            &[
                &self.insts.buffer,
                &self.n_insts.buffer,
                &self.gactive.buffer,
                &self.n_gaps.buffer,
                &front.dir_bases.buffer,
                dir_total,
                &front.rows.buffer,
                &front.bases.buffer,
                total,
                &self.trank.buffer,
                &self.tscan.total.buffer,
                &self.gcounts.buffer,
                &self.gfirsts.buffer,
                &self.gsort.vals_a.buffer,
                &self.gkeys.buffer,
                &self.sslot.buffer,
            ],
            &[n32],
            by_gap,
        )?;
        self.gscan.encode(enc, &self.n_gaps.buffer, 0, &self.gcounts.buffer, &self.gat.buffer)?;
        enc.run("pb_args", None, &[&self.gscan.total.buffer, &self.targs.buffer], &[0], Grid::Groups(1))?;
        // 64 bits sort in an even number of passes: the keys end in place, their rows in `vals_a`.
        debug_assert!(Sort::in_input(64));
        enc.run(
            "pe_lookups",
            None,
            &[
                u_files,
                u_data,
                &front.bases.buffer,
                &front.keys.buffer,
                &self.text_nodes.buffer,
                &self.gat.buffer,
                &self.n_gaps.buffer,
                &self.gfirsts.buffer,
                &self.sslot.buffer,
                &self.gscan.total.buffer,
                &self.sort_keys.buffer,
                &self.sort.vals_a.buffer,
                &self.n_rows.buffer,
                &self.name_keys.buffer,
                &self.values.buffer,
                &self.lookups.buffer,
                &self.bad.buffer,
            ],
            &[n32],
            Grid::Indirect(&self.targs.buffer, 0),
        )
    }

    /// The lookups, read back for tests (after the command buffer is done).
    pub fn read(&self) -> Result<Vec<u32>, Error> {
        // SAFETY: the device finished; `lookups` holds the scan's total of words.
        unsafe {
            if *self.bad.host().cast::<u32>() != 0 {
                return Err(Error::BackendCapacity);
            }
            let n = *self.gscan.total.host().cast::<u32>() as usize;
            Ok(std::slice::from_raw_parts(self.lookups.host().cast::<u32>(), n).to_vec())
        }
    }
}

/// A list (`PxList` in the MSL): 24 words; an argument (`PxArg`): 3.
const LIST_WORDS: usize = 24;
const ARG_WORDS: usize = 3;

/// Why a unit is refused (`pp_expand.metal`'s `X_*`). `X_ROOM` reruns it at the next rung.
pub const X_BUILTIN: u32 = 1;
pub const X_PAST_END: u32 = 4;
pub const X_PASTE: u32 = 8;
pub const X_DEPTH: u32 = 16;
pub const X_ROOM: u32 = 32;
pub const X_QUEUE: u32 = 64;
pub const X_HASH: u32 = 128;
pub const X_ACROSS: u32 = 256;
pub const X_VA_OPT: u32 = 512;
pub const X_RELEX: u32 = 1024;
pub const X_SPLICE: u32 = 2048;

/// The expansion's rungs: rung `k`'s arena holds `2^(k+1)` tokens a top token.
pub const EXP_RUNGS: u32 = 5;
/// The queue's persistent dispatch: worker groups, and its launches a phase (the first does the
/// work; the others start where every worker exited with items left, and end at once if none).
const WORKER_GROUPS: u64 = 64;
const WORK_LAUNCHES: usize = 3;
/// The close table's keys: a biased depth (16 bits) above a position (24 bits).
const CLOSE_BITS: u32 = 40;
/// The line index's blocks (`pp_expand.metal`'s `LINE_BLOCK`), and a string literal's tape kind.
const LINE_BLOCK: usize = 4096;
const K_LITERAL: u32 = 30;

/// The expansion's stores (0448 §2), placed before the command buffer, at a rung:
/// - by top token (`top`): its stretch head, depth change and depth, its `)` flag and place, the
///   close table and its sort, its call's end, the reach's max scan, its state, the outermost
///   flags and their scan, its piece of the output and that piece's place;
/// - the arena: tokens (a ref, flags, a child and a depth each), lists, arguments, the queue (two
///   items a list);
/// - the output (refs), and the control words (the arena's tops, the refusals).
pub struct Exp {
    list_cap: u32,
    out_cap: u32,
    q_cap: u32,
    words: MappedBuffer,
    /// The scratch tokens (`#`'s and `##`'s): their bytes, and each one's (start, end, kind).
    /// `st_cap` (start, end, kind) words, then `sb_cap` bytes and slack.
    scratch: MappedBuffer,
    st_cap: usize,
    theads: MappedBuffer,
    delta: MappedBuffer,
    depth: MappedBuffer,
    depth_scan: SegScan,
    close: MappedBuffer,
    close_at: MappedBuffer,
    close_scan: Scan,
    close_keys: MappedBuffer,
    close_sort: Sort,
    cend: MappedBuffer,
    heads: MappedBuffer,
    reach: MappedBuffer,
    reach_scan: SegScan,
    state: MappedBuffer,
    oflag: MappedBuffer,
    oid: MappedBuffer,
    oscan: Scan,
    v: MappedBuffer,
    obase: MappedBuffer,
    /// The top level's rollback: the exit-bearing calls flagged, their scan, their places in
    /// order, and the live claims' running end.
    xflag: MappedBuffer,
    xat: MappedBuffer,
    xscan: Scan,
    xpos: MappedBuffer,
    xcover: MappedBuffer,
    vscan: Scan,
    lists: MappedBuffer,
    /// The tokens' columns, `tok_cap` words each: the ref, the flags, the child, the depth and the
    /// origin (the list whose substitution wrote it).
    tokens: MappedBuffer,
    /// The line index: each 4 KB block's newlines, and their scan.
    line_counts: MappedBuffer,
    lines: MappedBuffer,
    line_scan: Scan,
    line_words: [u32; 2],
    n_blocks: MappedBuffer,
    args: MappedBuffer,
    slots: MappedBuffer,
    qctl: MappedBuffer,
    pub ctl: MappedBuffer,
    pub out: MappedBuffer,
}

impl Exp {
    pub fn new(unit: &Unit, lay: &Layout, rung: u32) -> Result<Exp, Error> {
        let device = crate::device()?;
        let zeroed = |k: usize| MappedBuffer::zeroed::<u32>(&device, k.max(1));
        let top = top_bound(unit, lay);
        // Each rung doubles the whole arena, so a short text with a large expansion fits too.
        let tok = (top + 4096) << (rung + 1);
        let lists = top / 2 + tok / 4 + 256;
        let args = tok / 2 + 256;
        let out = top + tok;
        let q = 2 * lists + 64;
        if tok >= u32::MAX as usize / 2 || q >= u32::MAX as usize / 2 {
            return Err(Error::BackendCapacity);
        }
        let va_opt = name_key(b"__VA_OPT__");
        // The scratch: bytes (64 KB at rung 0, doubling) and tokens, in one buffer: the tokens (an
        // even number, so the bytes after them are 8-byte aligned), then the bytes. The first of
        // each are the files' `__FILE__` literals, placed here.
        let nf = unit.table.len() + 3;
        let lit_bytes: usize = (0..nf).map(|f| lay.file_literals.get(f).map_or(2, Vec::len)).sum();
        let (sb, st) = (((64usize << 10) << rung) + lit_bytes, (tok / 4 + 256 + nf).next_multiple_of(2));
        let mut scratch = vec![0u8; st * 12 + sb + 16];
        let mut at = 0usize;
        for f in 0..nf {
            let lit: &[u8] = lay.file_literals.get(f).map_or(b"\"\"", Vec::as_slice);
            scratch[st * 12 + at..st * 12 + at + lit.len()].copy_from_slice(lit);
            let tok3 = [at as u32, (at + lit.len()) as u32, K_LITERAL];
            for (k, w) in tok3.iter().enumerate() {
                scratch[f * 12 + 4 * k..f * 12 + 4 * k + 4].copy_from_slice(&w.to_le_bytes());
            }
            at += lit.len();
        }
        // The control words: the scratch's tops past the literals.
        let mut ctl = [0u32; 8];
        ctl[4] = lit_bytes as u32;
        ctl[5] = nf as u32;
        // The line index over the unit's bytes.
        let words_n = unit.data.byte_length().div_ceil(8);
        let blocks = words_n.div_ceil(LINE_BLOCK / 8).max(1);
        let words = [unit.table.len() as u32, tok as u32, lists as u32, args as u32, out as u32, q as u32, va_opt as u32, (va_opt >> 32) as u32, sb as u32, st as u32, lay.relex_options];
        Ok(Exp {
            list_cap: lists as u32,
            out_cap: out as u32,
            q_cap: q as u32,
            words: MappedBuffer::from_slice(&device, &words)?,
            // Slack past the last byte: a name's key reads 8 bytes at a time.
            scratch: MappedBuffer::from_slice(&device, &scratch)?,
            st_cap: st,
            theads: zeroed(top)?,
            delta: zeroed(top)?,
            depth: zeroed(top)?,
            depth_scan: SegScan::new(top)?,
            close: zeroed(top)?,
            close_at: zeroed(top)?,
            close_scan: Scan::new(top)?,
            close_keys: MappedBuffer::zeroed::<u64>(&device, top)?,
            close_sort: Sort::new(top)?,
            cend: zeroed(top)?,
            heads: zeroed(top)?,
            reach: zeroed(top)?,
            reach_scan: SegScan::new(top)?,
            state: zeroed(top)?,
            oflag: zeroed(top)?,
            oid: zeroed(top)?,
            oscan: Scan::new(top)?,
            v: zeroed(top)?,
            obase: zeroed(top)?,
            xflag: zeroed(top)?,
            xat: zeroed(top)?,
            xscan: Scan::new(top)?,
            xpos: zeroed(top)?,
            xcover: zeroed(top)?,
            vscan: Scan::new(top)?,
            lists: zeroed(lists * LIST_WORDS)?,
            tokens: zeroed(tok * 5)?,
            line_counts: zeroed(blocks)?,
            lines: zeroed(blocks)?,
            line_scan: Scan::new(blocks)?,
            line_words: [words_n as u32, blocks as u32],
            n_blocks: MappedBuffer::from_slice(&device, &[blocks as u32])?,
            args: zeroed(args * ARG_WORDS)?,
            slots: zeroed(q)?,
            qctl: zeroed(4)?,
            ctl: MappedBuffer::from_slice(&device, &ctl)?,
            out: zeroed(out)?,
        })
    }

    /// The top level's grid passes (one thread a top token, sized by the lookups' count): its
    /// depth (a segmented scan by stretch), the close table (a scan, one 40-bit sort), the calls,
    /// the outermost (a max scan of the reach, a scan); the queue's first phase (CALL, SCAN,
    /// SUBST, flattening PLACE items, to every list's final); the top level's places (a scan of
    /// the pieces); the queue's second phase (PLACE items to the output), and the check.
    pub fn encode(&self, enc: &Enc<'_>, unit: &Unit, front: &Front, defs: &Defs, vis: &Vis) -> Result<(), Error> {
        let n32 = unit.table.len() as u32;
        let (u_files, u_data) = (&unit.files.buffer as &BufferRef, &unit.data.buffer as &BufferRef);
        let n_toks = &vis.gscan.total.buffer;
        let by_top = Grid::Indirect(&vis.targs.buffer, 0);
        let top_in = [u_files, u_data, &front.bases.buffer, &vis.text_nodes.buffer, &vis.gat.buffer, &vis.n_gaps.buffer, &vis.gfirsts.buffer, n_toks];
        let mut parens = top_in.to_vec();
        parens.extend([&self.theads.buffer as &BufferRef, &self.delta.buffer, &self.close.buffer]);
        enc.run("px_top_parens", None, &parens, &[n32], by_top)?;
        // The line index (`__LINE__`): each block's newlines, one thread a block, and their scan.
        let [nw, nb] = self.line_words;
        enc.run("px_line_blocks", None, &[u_data, &self.line_counts.buffer], &[nw, nb], Grid::Groups((nb as u64).div_ceil(TG)))?;
        self.line_scan.encode(enc, &self.n_blocks.buffer, 0, &self.line_counts.buffer, &self.lines.buffer)?;
        self.depth_scan.encode(enc, SegOp::Add, n_toks, 0, &self.delta.buffer, &self.theads.buffer, &self.depth.buffer)?;
        self.close_scan.encode(enc, n_toks, 0, &self.close.buffer, &self.close_at.buffer)?;
        enc.run("px_close_keys", None, &[&self.depth.buffer, &self.close.buffer, &self.close_at.buffer, n_toks, &self.close_keys.buffer, &self.ctl.buffer], &[], by_top)?;
        self.close_sort.encode(enc, &self.close_scan.total.buffer, 0, &self.close_keys.buffer, CLOSE_BITS)?;
        let closes = if Sort::in_input(CLOSE_BITS) { &self.close_keys.buffer as &BufferRef } else { &self.close_sort.keys_b.buffer };
        enc.run(
            "px_top_calls",
            None,
            &[
                u_files,
                u_data,
                &front.bases.buffer,
                &defs.defs.buffer,
                &vis.lookups.buffer,
                &vis.text_nodes.buffer,
                &vis.gat.buffer,
                &vis.n_gaps.buffer,
                &vis.gfirsts.buffer,
                n_toks,
                &self.depth.buffer,
                closes,
                &self.close_scan.total.buffer,
                &self.cend.buffer,
                &self.ctl.buffer,
            ],
            &[n32],
            by_top,
        )?;
        self.reach_scan.encode(enc, SegOp::Max, n_toks, 0, &self.cend.buffer, &self.heads.buffer, &self.reach.buffer)?;
        enc.run("px_top_outer", None, &[&self.cend.buffer, &self.reach.buffer, n_toks, &self.state.buffer, &self.oflag.buffer], &[], by_top)?;
        self.oscan.encode(enc, n_toks, 0, &self.oflag.buffer, &self.oid.buffer)?;
        enc.run(
            "px_top_lists",
            None,
            &[&vis.lookups.buffer, &self.cend.buffer, &self.state.buffer, &self.oid.buffer, n_toks, &vis.sslot.buffer, &vis.gat.buffer, &vis.n_gaps.buffer, &self.lists.buffer, &self.slots.buffer, &self.ctl.buffer, &defs.defs.buffer],
            &[self.list_cap],
            by_top,
        )?;
        enc.run("px_seed", None, &[&self.oscan.total.buffer, &self.qctl.buffer, &self.ctl.buffer], &[self.list_cap], Grid::Groups(1))?;
        self.work(enc, unit, front, defs, vis, closes)?;
        // The top level's rollback: the exit-bearing calls in order, decided by one SIMD group.
        enc.run("px_top_xflag", None, &[&self.state.buffer, &self.oid.buffer, &self.lists.buffer, n_toks, &self.xflag.buffer], &[], by_top)?;
        self.xscan.encode(enc, n_toks, 0, &self.xflag.buffer, &self.xat.buffer)?;
        enc.run("px_top_xlist", None, &[&self.xflag.buffer, &self.xat.buffer, n_toks, &self.xpos.buffer], &[], by_top)?;
        let n_x = &self.xscan.total.buffer;
        enc.run("px_top_cover", None, &[&self.xpos.buffer, n_x, &self.oid.buffer, &self.lists.buffer, &self.xcover.buffer, &self.ctl.buffer], &[], Grid::Groups(1))?;
        enc.run("px_top_sizes", None, &[&self.state.buffer, &self.oid.buffer, &self.lists.buffer, n_toks, &self.v.buffer, &self.xpos.buffer, n_x, &self.xcover.buffer], &[], by_top)?;
        self.vscan.encode(enc, n_toks, 0, &self.v.buffer, &self.obase.buffer)?;
        enc.run(
            "px_top_place",
            None,
            &[
                &self.state.buffer,
                &self.oid.buffer,
                &self.lists.buffer,
                n_toks,
                &self.obase.buffer,
                &vis.text_nodes.buffer,
                &vis.gat.buffer,
                &vis.n_gaps.buffer,
                &vis.gfirsts.buffer,
                &self.out.buffer,
                &self.slots.buffer,
                &self.qctl.buffer,
                &self.ctl.buffer,
            ],
            &[self.out_cap, self.q_cap],
            by_top,
        )?;
        self.work(enc, unit, front, defs, vis, closes)?;
        enc.run("px_check", None, &[&self.qctl.buffer, &self.vscan.total.buffer, &self.ctl.buffer], &[self.out_cap], Grid::Groups(1))
    }

    /// The queue's persistent dispatch, `WORK_LAUNCHES` times.
    fn work(&self, enc: &Enc<'_>, unit: &Unit, front: &Front, defs: &Defs, vis: &Vis, closes: &BufferRef) -> Result<(), Error> {
        for _ in 0..WORK_LAUNCHES {
            enc.run(
                "px_work",
                None,
                &[
                    &unit.files.buffer,
                    &unit.data.buffer,
                    &front.bases.buffer,
                    &front.keys.buffer,
                    &defs.defs.buffer,
                    &defs.items.buffer,
                    &vis.sort_keys.buffer,
                    &vis.sort.vals_a.buffer,
                    &vis.n_rows.buffer,
                    &vis.name_keys.buffer,
                    &vis.values.buffer,
                    &vis.text_nodes.buffer,
                    &vis.gat.buffer,
                    &vis.n_gaps.buffer,
                    &vis.gfirsts.buffer,
                    &vis.gscan.total.buffer,
                    &self.depth.buffer,
                    &self.lists.buffer,
                    &self.tokens.buffer,
                    &self.args.buffer,
                    &self.out.buffer,
                    &self.slots.buffer,
                    &self.qctl.buffer,
                    &self.ctl.buffer,
                    &self.words.buffer,
                    &self.scratch.buffer,
                    closes,
                    &self.close_scan.total.buffer,
                    &self.lines.buffer,
                ],
                &[],
                Grid::Groups(WORKER_GROUPS),
            )?;
        }
        Ok(())
    }

    /// The scratch tokens' bytes and (start, end, kind) (after the command buffer).
    pub fn scratch(&self) -> (Vec<u8>, Vec<[u32; 3]>) {
        // SAFETY: the device finished; the tops are within the capacities where no room ran out.
        unsafe {
            let ctl = self.ctl.host().cast::<u32>();
            let (nb, nt) = (*ctl.add(4) as usize, *ctl.add(5) as usize);
            let bytes = self.scratch.host().add(self.st_cap * 12);
            let nb = nb.min(self.scratch.byte_length() - self.st_cap * 12);
            let nt = nt.min(self.st_cap);
            (std::slice::from_raw_parts(bytes, nb).to_vec(), std::slice::from_raw_parts(self.scratch.host().cast::<[u32; 3]>(), nt).to_vec())
        }
    }

    /// The refusals, and the output's refs where there are none (after the command buffer).
    /// Each store's use and room (`ROOM_STORES`'s order): its top (past the room where it ran
    /// out: a top only grows) and its capacity.
    pub fn room(&self) -> [(u32, u32); 5] {
        // SAFETY: the device finished; `ctl` and `words` are the control and capacity words.
        unsafe {
            let ctl = self.ctl.host().cast::<u32>();
            let w = self.words.host().cast::<u32>();
            [(0, 1), (1, 2), (3, 3), (4, 8), (5, 9)].map(|(t, c)| (*ctl.add(t), *w.add(c)))
        }
    }

    pub fn read(&self) -> (u32, Vec<u32>) {
        // SAFETY: the device finished; `out` holds at least `n_out` words where no room ran out.
        unsafe {
            let refused = *self.ctl.host().cast::<u32>().add(2);
            if refused != 0 {
                return (refused, Vec::new());
            }
            let n = *self.vscan.total.host().cast::<u32>() as usize;
            (0, std::slice::from_raw_parts(self.out.host().cast::<u32>(), n).to_vec())
        }
    }
}

/// A unit expanded on the device: its output's tokens (refs to its tapes' nodes, in order), or
/// why it is refused (`X_*` bits), and the rung it ran at.
pub struct Expanded {
    pub ev: crate::pp_eval::Evaluated,
    pub refused: u32,
    pub out: Vec<u32>,
    pub rung: u32,
    /// The command buffer's launches (at most `KERNEL_LAUNCH_LIMIT`).
    pub launches: usize,
    /// The scratch tokens (a ref with `SCRATCH` set): their bytes, and each one's (start, end,
    /// kind).
    pub scratch: Vec<u8>,
    pub stoks: Vec<[u32; 3]>,
    /// The arena's use and room at the rung it ran at, store by store (`ROOM_STORES`).
    pub room: [(u32, u32); 5],
}

/// The arena's stores, as `Expanded::room` lists them.
pub const ROOM_STORES: [&str; 5] = ["tokens", "lists", "arguments", "scratch bytes", "scratch tokens"];

/// A ref to a scratch token, not a tape node.
pub const SCRATCH: u32 = 0x8000_0000;

/// The front passes, the definitions, the lookups and the expansion over `unit` laid out as
/// `lay`, in one command buffer; from rung `rung`, and at the next where a store's room ran out.
pub fn expand_from(unit: &Unit, lay: &Layout, rung: u32) -> Result<Expanded, Error> {
    let (mut def_rung, mut rung) = (0, rung);
    loop {
        let front = Front::new(unit, false)?;
        let defs = Defs::new(unit, def_rung)?;
        let vis = Vis::new(unit, lay)?;
        let exp = Exp::new(unit, lay, rung)?;
        let launches = crate::pp_eval::run(|enc| {
            front.encode(enc, unit)?;
            defs.encode(enc, unit, &front)?;
            vis.encode(enc, unit, &front, &defs)?;
            exp.encode(enc, unit, &front, &defs, &vis)
        })?;
        // SAFETY: the device finished writing `room`.
        if unsafe { *defs.room.host().cast::<u32>() } != 0 {
            def_rung += 1;
            if def_rung >= RUNGS {
                return Err(Error::BackendCapacity);
            }
            continue;
        }
        vis.read()?;
        let (refused, out) = exp.read();
        if refused & X_ROOM != 0 && refused & !X_ROOM == 0 && rung + 1 < EXP_RUNGS {
            rung += 1;
            continue;
        }
        let (scratch, stoks) = exp.scratch();
        return Ok(Expanded { ev: front.read()?, refused, out, rung, launches, scratch, stoks, room: exp.room() });
    }
}

/// `expand_from` rung 0.
pub fn expand(unit: &Unit, lay: &Layout) -> Result<Expanded, Error> {
    expand_from(unit, lay, 0)
}

/// The front passes, the definitions and the lookups over `unit` laid out as `lay`, in one
/// command buffer: each active stretch token's meaning (`L_*`, or the directive row of its
/// definition), in the layout's order.
pub fn lookups(unit: &Unit, lay: &Layout) -> Result<(crate::pp_eval::Evaluated, Vec<u32>), Error> {
    let mut rung = 0;
    loop {
        let front = Front::new(unit, false)?;
        let defs = Defs::new(unit, rung)?;
        let vis = Vis::new(unit, lay)?;
        crate::pp_eval::run(|enc| {
            front.encode(enc, unit)?;
            defs.encode(enc, unit, &front)?;
            vis.encode(enc, unit, &front, &defs)
        })?;
        // SAFETY: the device finished writing `room`.
        if unsafe { *defs.room.host().cast::<u32>() } != 0 {
            rung += 1;
            if rung >= RUNGS {
                return Err(Error::BackendCapacity);
            }
            continue;
        }
        return Ok((front.read()?, vis.read()?));
    }
}

/// The dialect's options as the relexer reads them (`pp_expand.metal`'s `O_*`): C++, `::`, digit
/// separators, raw strings.
pub fn relex_options(cpp: bool, colons: bool, digit_separators: bool, raw_strings: bool) -> u32 {
    u32::from(cpp) | u32::from(colons) << 1 | u32::from(digit_separators) << 2 | u32::from(raw_strings) << 3
}

/// The relexer's answer where a spelling is no one token, and where the device refuses it.
pub const RELEX_NONE: u32 = 0;
pub const RELEX_REFUSED: u32 = 0xffff;

/// For tests: the relexer's answer for each spelling (a tape kind, `RELEX_NONE` or
/// `RELEX_REFUSED`), under `options`.
pub fn relex(texts: &[Vec<u8>], options: u32) -> Result<Vec<u32>, Error> {
    let device = crate::device()?;
    let mut bytes = Vec::new();
    let mut offsets = vec![0u32];
    for t in texts {
        bytes.extend_from_slice(t);
        offsets.push(bytes.len() as u32);
    }
    bytes.extend_from_slice(&[0; 16]);
    let (b, o) = (MappedBuffer::from_slice(&device, &bytes)?, MappedBuffer::from_slice(&device, &offsets)?);
    let out = MappedBuffer::zeroed::<u32>(&device, texts.len().max(1))?;
    let n = texts.len() as u32;
    crate::pp_eval::run(|enc| enc.run("pd_relex_test", None, &[&b.buffer, &o.buffer, &out.buffer], &[n, options], Grid::Groups(u64::from(n).div_ceil(crate::pp_blocks::TG))))?;
    // SAFETY: the device finished writing `out`.
    Ok(unsafe { std::slice::from_raw_parts(out.host().cast::<u32>(), texts.len()).to_vec() })
}
