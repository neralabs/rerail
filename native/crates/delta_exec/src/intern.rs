//! `intern` on the host (decision 0686, B5; *0686: Operations* §10): its signature, its twin registered as its
//! reference, and the call that runs it over a changed document. Its layout and its twin are
//! `rerail_delta_twins::intern`.

use crate::dir::{BlockId, Directory};
use crate::files::{BlobId, BlobRows, Placed};
use crate::op::{Across, Found, Independence, Inside, Output, ParamsLayout, Phase, Registry, Runner, Signature, Tiling, Unit, Version};
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

/// The UKL version's shape (`ukl/intern.uglykl`): the hash phase a lane a node; the scan one threadgroup, its items
/// in rounds of the group's threads, in order.
pub(crate) static INTERN_UKL: [Phase; 2] = [
    Phase { unit: Unit::Lane, ..INTERN_PHASE[0] },
    Phase { inside: Inside::GroupMemory, ..INTERN_PHASE[1] },
];

/// `intern`: read the document's source and tape (their blocks, which may be one) · rw the atoms · rw the document's
/// symbols block · read the language's table (`intern::write_language`).
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

/// The module's operations declared, each from its UKL interface, and their twins registered as references: the
/// runner's registry (`Runner::new`), the one place they are registered.
pub(crate) fn register(reg: &mut Registry) -> Result<(), String> {
    let intern = reg.declare(&INTERN)?;
    reg.register_reference(Version::twin(intern, "intern twin", twin::INTERN, &INTERN_PHASE))?;
    Ok(())
}

/// Interns the document whose source and tape lie at `src` and `tape`, answered at `path` as
/// `module`: its symbols block (node → symbol), made here where the directory keeps what lasts, written by `intern`
/// (which adds its new names to `atoms`), frozen and housed as a blob.
#[allow(clippy::too_many_arguments)]
pub fn intern(ops: &Runner, dir: &mut Directory, blobs: &mut BlobRows, atoms: Atoms, src: Placed, tape: Placed, path: &str, module: &str) -> Result<BlobId, String> {
    intern_at(ops, dir, blobs, atoms, src, tape, path, module, Tiling::WHOLE)
}

/// `intern` with its hash phase's items run in tiles of `tiling` (its scan is one item, in node order).
#[allow(clippy::too_many_arguments)]
pub fn intern_at(ops: &Runner, dir: &mut Directory, blobs: &mut BlobRows, atoms: Atoms, src: Placed, tape: Placed, path: &str, module: &str, tiling: Tiling) -> Result<BlobId, String> {
    let (id, slots, p, body) = launch(dir, atoms, src, tape, path, module)?;
    let made = (|| {
        ops.run(dir, &INTERN, &slots, &p, tiling)?;
        dir.freeze(id)?;
        blobs.house(ops, dir, Placed { block: id, at: BODY_AT, len: body })
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

/// `intern`'s launch over the document at `src` and `tape`: its symbols block made (kept where the directory keeps
/// what lasts: a pure answer, kept by key, D4, while the store's atoms keep their ids), its header written (the tape's
/// view among it); its slots, its params and the symbols block's body bytes.
pub(crate) fn launch(dir: &mut Directory, atoms: Atoms, src: Placed, tape: Placed, path: &str, module: &str) -> Result<(BlockId, [BlockId; 5], [u8; InternP::SIZE], usize), String> {
    let n = crate::files::nodes(dir, src, tape)?;
    let view = {
        let l = dir.read("intern's tape", &[tape.block])?;
        let bytes = &l.block(0)[tape.at..tape.at + tape.len];
        let v = rerail_rr_syntax::tape_format::f16::placement(bytes)?.view(rerail_rr_syntax::tape::lang(), 0)?;
        // SAFETY: a view is `VIEW_WORDS` u32s (`TapeView`, `repr(C)`), its size checked by the transmute.
        unsafe { core::mem::transmute::<_, [u32; twin::VIEW_WORDS]>(v) }
    };
    let lang = language(dir)?;
    let body = twin::syms_bytes(n, path, module);
    let id = dir.make_placed(dir.lasting(), kind::FACTS, body, 0)?;
    let b = dir.block(id).expect("just made");
    let mut w = pin_block(&b, "intern", true)?;
    twin::init_syms(w.bytes_mut().expect("a write pin"), n, &view, path, module);
    let mut p = [0u8; InternP::SIZE];
    InternP { src_at: src.at as u64, src_len: src.len as u64, tape_at: tape.at as u64, tape_len: tape.len as u64 }.write(&mut p);
    Ok((id, [src.block, tape.block, atoms.0, id, lang], p, body))
}

/// The `.rerail` language's table for `intern`'s kernels (`intern::write_language`): made once a directory, frozen.
pub(crate) fn language(dir: &mut Directory) -> Result<BlockId, String> {
    if let Some(id) = dir.language {
        return Ok(id);
    }
    let id = dir.make(kind::CONTROL, twin::language_bytes());
    {
        let b = dir.block(id).expect("just made");
        let mut w = pin_block(&b, "the language's table", true)?;
        twin::write_language(w.bytes_mut().expect("a write pin"));
    }
    dir.freeze(id)?;
    dir.language = Some(id);
    Ok(id)
}

#[cfg(test)]
#[path = "intern_tests.rs"]
mod tests;
