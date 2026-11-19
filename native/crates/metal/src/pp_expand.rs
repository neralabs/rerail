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
}

/// What a lookup answers where it is no definition's row (`pp_expand.metal`'s `L_*`).
pub const L_NOT_NAME: u32 = 0xffff_ffff;
pub const L_NONE: u32 = 0xffff_fffe;
pub const L_BUILTIN: u32 = 0xffff_fffd;

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
    n_gaps: MappedBuffer,
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
    text_nodes: MappedBuffer,
    // The rows: a directive of the layout each, after the built-ins'.
    rflag: MappedBuffer,
    rat: MappedBuffer,
    rscan: Scan,
    sort_keys: MappedBuffer,
    name_keys: MappedBuffer,
    values: MappedBuffer,
    n_rows: MappedBuffer,
    sort: Sort,
    // The stretches: a word each, and a lookup a text token.
    gcounts: MappedBuffer,
    gfirsts: MappedBuffer,
    gat: MappedBuffer,
    gscan: Scan,
    targs: MappedBuffer,
    pub lookups: MappedBuffer,
    pub bad: MappedBuffer,
}

const NO_INST: [PpInst; 1] = [PpInst { file: 0, dir_first: 0, gap_first: 0 }];

impl Vis {
    pub fn new(unit: &Unit, lay: &Layout) -> Result<Vis, Error> {
        let device = crate::device()?;
        let words = |v: &[u32]| MappedBuffer::from_slice(&device, if v.is_empty() { &[0] } else { v });
        let zeroed = |k: usize| MappedBuffer::zeroed::<u32>(&device, k.max(1));
        let nodes = unit.node_bound;
        let (nd, ng, nb) = (lay.dir_slots.len(), lay.gap_slots.len(), lay.builtins.len());
        let rows = nd + nb;
        // The built-ins' rows, at slot 0: the configuration's, written before the passes.
        let mut sk = vec![0u64; rows.max(1)];
        let mut nk = vec![0u64; rows.max(1)];
        let mut vals = vec![0u32; rows.max(1)];
        for (k, &key) in lay.builtins.iter().enumerate() {
            sk[k] = key & 0xffff_ffff_0000_0000;
            nk[k] = key;
            vals[k] = L_BUILTIN;
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
            gat: zeroed(ng)?,
            gscan: Scan::new(ng)?,
            targs: zeroed(4)?,
            lookups: zeroed(nodes)?,
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
                &self.gslot.buffer,
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
