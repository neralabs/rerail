//! Macro expansion on the device (decision 0448), `pp_expand.metal`. Built so far: the setup's
//! definitions (S1, S2, S4), read from the tapes in place.

use crate::Error;
use crate::pp_blocks::{Enc, Grid, Scan};
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
