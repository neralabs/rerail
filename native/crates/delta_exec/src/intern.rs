//! `intern` on the host (decision 0686, B5; *0686: Operations* §10): its signature, its twin registered as its
//! reference, and the call that runs it over a changed document. Its layout and its twin are
//! `rerail_delta_twins::intern`.

use crate::dir::Directory;
use crate::files::{BlobId, BlobRows, Placed};
use crate::op::{Across, Found, Independence, Inside, OpId, Output, ParamsLayout, Phase, Registry, Signature, Tiling, Unit, Version, run_cpu};
use crate::sym::Atoms;
use rerail_blocks::atoms::{count_in, name_in};
use rerail_blocks::block::kind;
use rerail_blocks::gens::{BODY_AT, Table};
use rerail_blocks::region::pin_block;
use rerail_delta_twins::Params;
use rerail_delta_twins::intern::{self as twin, InternP};

/// *0686: Operations* §10's shape: a hash build (shape 18: an item a node, each name probed in the atoms' hash table,
/// found ids written in place), then a scan for new ids (shape 10: in node order, after the hash phase).
static INTERN_PHASE: [Phase; 2] = [
    Phase { name: "hash", found: Found::Fixed, unit: Unit::Threadgroup, inside: Inside::Nothing, across: Across::Nothing, output: Output::InPlace, bounds: &[] },
    Phase { name: "scan", found: Found::Fixed, unit: Unit::Threadgroup, inside: Inside::Nothing, across: Across::Ordered, output: Output::ByScan, bounds: &[] },
];

/// `intern`: read the document's source and tape (their blocks, which may be one) · rw the atoms · rw the document's
/// symbols block.
pub static INTERN: Signature = Signature::of(
    &crate::op::interfaces::INTERN,
    ParamsLayout::of::<InternP>(),
    &INTERN_PHASE,
    Independence::Not("one writer of the atoms"),
    intern_facts,
);

/// The written slots' facts by meaning: the atoms' strings by id (by their bytes), and the document's nodes' symbols.
pub fn intern_facts(_: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    match slot {
        twin::ATOMS => (1..1 + count_in(bytes)).map(|id| {
            let mut r = vec![i64::from(id)];
            r.extend(name_in(bytes, id).unwrap_or("").bytes().map(i64::from));
            r
        }).collect(),
        _ => {
            let (p, m) = twin::doc_syms(bytes);
            let mut rows = vec![vec![-1, i64::from(p), i64::from(m)]];
            rows.extend((0..twin::count(bytes)).map(|n| vec![n as i64, i64::from(twin::text(bytes, n)), i64::from(twin::value(bytes, n))]));
            rows
        }
    }
}

pub struct InternOps {
    pub intern: OpId,
}

impl InternOps {
    pub fn declare(reg: &mut Registry) -> Result<InternOps, String> {
        let intern = reg.declare(&INTERN)?;
        reg.register_reference(Version::twin(intern, "intern twin", twin::INTERN, &INTERN_PHASE))?;
        Ok(InternOps { intern })
    }
}

/// Interns the document whose source and tape lie at `src` and `tape`, answered at `path` as
/// `module`: its symbols block (node → symbol), made here where the directory keeps what lasts, written by `intern`
/// (which adds its new names to `atoms`), frozen and housed as a blob.
#[allow(clippy::too_many_arguments)]
pub fn intern(reg: &Registry, ops: &InternOps, dir: &mut Directory, blobs: &mut BlobRows, atoms: Atoms, src: Placed, tape: Placed, path: &str, module: &str) -> Result<BlobId, String> {
    intern_at(reg, ops, dir, blobs, atoms, src, tape, path, module, Tiling::WHOLE)
}

/// `intern` with its hash phase's items run in tiles of `tiling` (its scan is one item, in node order).
#[allow(clippy::too_many_arguments)]
pub fn intern_at(reg: &Registry, ops: &InternOps, dir: &mut Directory, blobs: &mut BlobRows, atoms: Atoms, src: Placed, tape: Placed, path: &str, module: &str, tiling: Tiling) -> Result<BlobId, String> {
    let n = crate::files::nodes(dir, src, tape)?;
    let body = twin::syms_bytes(n, path, module);
    // Kept where the directory keeps what lasts: a pure answer, kept by key (D4) while the store's atoms keep their ids.
    let id = dir.make_placed(dir.lasting(), kind::FACTS, body, 0)?;
    let made = (|| {
        {
            let b = dir.block(id).expect("just made");
            let mut w = pin_block(&b, "intern", true)?;
            twin::init_syms(w.bytes_mut().expect("a write pin"), n, path, module);
        }
        let mut p = [0u8; InternP::SIZE];
        InternP { src_at: src.at as u64, src_len: src.len as u64, tape_at: tape.at as u64, tape_len: tape.len as u64 }.write(&mut p);
        run_cpu(reg, dir, ops.intern, &[src.block, tape.block, atoms.0, id], &p, tiling)?;
        dir.freeze(id)?;
        blobs.house(reg, dir, Placed { block: id, at: BODY_AT, len: body })
    })();
    match made {
        Ok((s, new)) => {
            if !new {
                dir.release(id);
            }
            Ok(s)
        }
        Err(e) => {
            dir.release(id);
            Err(e)
        }
    }
}

#[cfg(test)]
#[path = "intern_tests.rs"]
mod tests;
