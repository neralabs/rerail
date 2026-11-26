//! `intern` on the host (decision 0686, B5; *0686: Operations* §10): its signature, its twin registered as its
//! reference, and the call that runs it over a changed document. Its layout and its twin are
//! `rerail_delta_twins::intern`.

use crate::dir::{BlockId, Directory};
use crate::files::{BlobId, Blobs, Doc, Placed};
use crate::op::{Across, Found, Independence, Inside, Mode, OpId, Output, ParamsLayout, Phase, Registry, Signature, SlotSpec, Tiling, Unit, Version, run_cpu};
use crate::sym::Atoms;
use rerail_blocks::atoms::{count_in, name_in};
use rerail_blocks::block::kind;
use rerail_blocks::gens::{BODY_AT, Table};
use rerail_blocks::region::pin_block;
use rerail_delta_twins::Params;
use rerail_delta_twins::intern::{self as twin, InternP};

/// One phase, one item: the whole document, its new ids in node order (a hash build, then a scan).
static INTERN_PHASE: [Phase; 1] = [Phase { name: "intern", found: Found::Fixed, unit: Unit::Threadgroup, inside: Inside::Nothing, across: Across::Nothing, output: Output::InPlace, bounds: &[] }];

/// `intern`: read the document's tape block · rw the atoms · rw the document's symbols block.
pub static INTERN: Signature = Signature::new(
    "intern",
    &[
        SlotSpec { name: "tape", mode: Mode::Read, kind: kind::TAPE },
        SlotSpec { name: "atoms", mode: Mode::ReadWrite, kind: kind::ATOMS },
        SlotSpec { name: "syms", mode: Mode::ReadWrite, kind: kind::FACTS },
    ],
    ParamsLayout::of::<InternP>(),
    &INTERN_PHASE,
    Independence::Not("one writer of the atoms"),
    intern_facts,
);

/// The written slots' facts by meaning: the atoms' strings by id (by their bytes), and the document's nodes' symbols.
pub fn intern_facts(_: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    match slot {
        1 => (1..1 + count_in(bytes)).map(|id| {
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

/// Interns the document whose source and tape lie at `src` and `tape` (one tape block), answered at `path` as
/// `module`: its symbols block (node → symbol), made here, written by `intern` (which adds its new names to `atoms`),
/// frozen and housed as a blob.
#[allow(clippy::too_many_arguments)]
pub fn intern(reg: &Registry, ops: &InternOps, dir: &mut Directory, blobs: &mut Blobs, atoms: Atoms, src: Placed, tape: Placed, path: &str, module: &str) -> Result<BlobId, String> {
    if src.block != tape.block {
        return Err("a document's source and tape in two blocks".into());
    }
    let n = {
        let l = dir.read("intern", &[tape.block])?;
        crate::files::reads(l.block(0), src, tape).len()
    };
    let body = twin::syms_bytes(n, path, module);
    let id = dir.make(kind::FACTS, body);
    let made = (|| {
        {
            let b = dir.block(id).expect("just made");
            let mut w = pin_block(&b, "intern", true)?;
            twin::init_syms(w.bytes_mut().expect("a write pin"), n, path, module);
        }
        let mut p = vec![0u8; InternP::SIZE];
        InternP { src_at: src.at as u64, src_len: src.len as u64, tape_at: tape.at as u64, tape_len: tape.len as u64 }.write(&mut p);
        run_cpu(reg, dir, ops.intern, &[tape.block, atoms.0, id], &p, Tiling::WHOLE)?;
        dir.freeze(id)?;
        blobs.house(dir, Placed { block: id, at: BODY_AT, len: body })
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

/// Each document's symbols blob, by its path and its tape: an unchanged tape's is the one it had, and `intern` runs
/// only for a changed one.
#[derive(Default)]
pub struct Interned {
    syms: std::collections::HashMap<(String, BlobId), BlobId>,
    /// `intern`'s runs.
    pub runs: u64,
}

impl Interned {
    /// Document `doc`'s symbols blob: kept when its tape was interned at its path, else `intern`'s.
    #[allow(clippy::too_many_arguments)]
    pub fn of(&mut self, reg: &Registry, ops: &InternOps, dir: &mut Directory, blobs: &mut Blobs, atoms: Atoms, doc: &Doc) -> Result<BlobId, String> {
        let key = (doc.path.clone(), doc.after.tape);
        if let Some(&s) = self.syms.get(&key) {
            return Ok(s);
        }
        let get = |id: &BlobId| blobs.get(id).ok_or_else(|| format!("{}: a blob nothing holds", doc.path));
        let (src, tape) = (get(&doc.after.source)?, get(&doc.after.tape)?);
        let s = intern(reg, ops, dir, blobs, atoms, src, tape, &doc.path, &doc.module)?;
        self.runs += 1;
        self.syms.insert(key, s);
        Ok(s)
    }
}

#[cfg(test)]
#[path = "intern_tests.rs"]
mod tests;
