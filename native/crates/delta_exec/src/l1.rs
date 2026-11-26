//! L1, the boot operation (decision 0686, B6; B§4, B§5, B§9; the rebuild plan's target): a delta operator of one
//! tape, no program, one production (K-form's eight relations) and facts in that are program globals (the symbols).
//!
//! Its ports, B§4's exactly:
//!
//! - **tape** (B4): a row a document, `(d, tape block, source at, source length, tape at, tape length, nodes)`, where
//!   the file layer placed its source and the parser wrote its tape; a changed file is its old row gone and its new
//!   row added, an unchanged one the identity;
//! - **no program**: L1 reads no K-form;
//! - **facts in** (B5): a row a document, `(d, symbols block)`, its node → symbol facts that `intern` wrote; the
//!   atoms are the one program global, read in place;
//! - **output** (B2): P2's bistate in the program trace, a trace a K-form relation (`kform::shape`).
//!
//! Its strategy is file grain: a document added is read whole by `k0_pass` into the outputs' δ runs, a document gone
//! is read whole again from its kept tape (a blob, never versioned) with the opposite sign, and `consolidate` nets
//! the two, so the δ is the diff against the rows it kept. An unchanged tape is L4's identity: no operation runs, and
//! L1 keeps no cache. Its Rust beyond `k0_pass` is the wiring of these ports.

use crate::contract::{At, CacheW, Cx, DeltaOp, In, Out, Port, Ports, Work};
use crate::dir::BlockId;
use crate::files::DocBlob;
use crate::k0::{K0Doc, K0Ops, k0_pass};
use crate::kform::{self, RELATIONS};
use crate::sym::Atoms;
use rerail_blocks::bistate::{Bistate, ColumnIndex, Read, RowId};
use rerail_blocks::layer::{ColType, Shape};
use rerail_blocks::relation::View;

/// The tape port's place among L1's inputs, and the symbols port's.
pub const TAPE: usize = 0;
pub const SYMS: usize = 1;

/// The tape port's rows: `(d, tape block, source at, source length, tape at, tape length, nodes)`, by document.
pub fn tape_shape() -> Shape {
    let c = vec![ColType::I32, ColType::I32, ColType::I64, ColType::I64, ColType::I64, ColType::I64, ColType::I64];
    Shape::run(c, (0..7).collect(), Some(0))
}

/// The symbols port's rows: `(d, symbols block)`, by document.
pub fn syms_shape() -> Shape {
    Shape::run(vec![ColType::I32, ColType::I32], vec![0, 1], Some(0))
}

/// Document `doc`'s row in the tape port, as module `d`.
pub fn tape_row(d: i64, doc: &DocBlob) -> Vec<i64> {
    let nodes = crate::tape::reads(&doc.text).len();
    vec![d, i64::from(doc.block.0), doc.src.0 as i64, doc.src.1 as i64, doc.tape.0 as i64, doc.tape.1 as i64, nodes as i64]
}

/// Module `d`'s row in the symbols port.
pub fn syms_row(d: i64, syms: BlockId) -> Vec<i64> {
    vec![d, i64::from(syms.0)]
}

/// L1: `k0_pass` over its tape port's changed documents.
pub struct L1 {
    pub ops: K0Ops,
    pub atoms: Atoms,
}

fn cell(v: &View<'_>, id: RowId, c: u16) -> i64 {
    Read::get(v, id, ColumnIndex::new(c))
}

/// The document of tape row `id` of `tape`, with its symbols block from `syms` (the row of its module).
fn doc_at(tape: &View<'_>, id: RowId, syms: &View<'_>) -> Result<(i64, K0Doc), String> {
    let c = |k| cell(tape, id, k) as usize;
    let d = cell(tape, id, 0);
    let s = (0..Read::end(syms).get() as u32).map(RowId::new).find(|&s| Read::live(syms, s) && cell(syms, s, 0) == d);
    let s = s.ok_or_else(|| format!("module {d}: a tape with no symbols in the facts-in port"))?;
    let doc = K0Doc { tape: BlockId(c(1) as u32), src: (c(2), c(3)), at: (c(4), c(5)), nodes: c(6), syms: BlockId(cell(syms, s, 1) as u32) };
    Ok((d, doc))
}

fn live<'v>(v: &'v View<'_>) -> impl Iterator<Item = RowId> + 'v {
    (0..Read::end(v).get() as u32).map(RowId::new).filter(|&id| Read::live(v, id))
}

impl L1 {
    /// `k0_pass` over one document with sign `diff`, into the outputs' runs.
    fn pass(&self, cx: &mut Cx<'_>, (d, doc): (i64, K0Doc), diff: i32, out: &Out, work: &mut Work) -> Result<(), String> {
        let runs: Vec<BlockId> = (0..RELATIONS.len()).map(|k| out.block(k)).collect();
        work.rows_read += 1;
        work.rows_written += k0_pass(cx.reg, &self.ops, cx.dir, self.atoms.0, doc, d, diff, &runs).map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl DeltaOp for L1 {
    fn ports(&self) -> Ports {
        Ports {
            ins: vec![Port { name: "tape", shape: tape_shape() }, Port { name: "symbols", shape: syms_shape() }],
            outs: RELATIONS.iter().enumerate().map(|(i, (name, _))| Port { name, shape: kform::shape(i) }).collect(),
        }
    }

    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_>, _: Option<&Bistate<View<'_>>>, _: &mut CacheW, out: &mut Out, _: &mut Out) -> Result<Work, String> {
        let mut work = Work::default();
        if ins.is_identity() {
            return Ok(work);
        }
        let (tape, syms) = (&ins.ports[TAPE], &ins.ports[SYMS]);
        if let (Some(tb), Some(sb)) = (tape.before(), syms.before()) {
            for id in tape.gone() {
                self.pass(cx, doc_at(tb, id, sb)?, -1, out, &mut work)?;
            }
        }
        for id in tape.added() {
            self.pass(cx, doc_at(tape.after(), id, syms.after())?, 1, out, &mut work)?;
        }
        Ok(work)
    }

    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_>, out: &mut Out) -> Result<(), String> {
        let (tape, syms) = (&ins.ports[TAPE], &ins.ports[SYMS]);
        let mut work = Work::default();
        for id in live(tape) {
            self.pass(cx, doc_at(tape, id, syms)?, 1, out, &mut work)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "l1_tests.rs"]
mod tests;
