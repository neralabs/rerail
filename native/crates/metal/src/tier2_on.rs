//! Tier 2 on tier 1's buffers as the Metal backend left them: the compiled words (the compiler
//! crate's `native_cpu_stream`) read a file's bytes, records (positions and codes), mask words
//! and totals in the device's shared memory, where tier 1 wrote them.

use crate::block_program::serial::Resident;
use rerail_compiler::native_cpu_stream::{Columns, Parser, Tier1View};
use rerail_compiler::stream::Refused;

/// File `i`'s node table from tier 1's buffers as the device left them (`Resident`): tier 2
/// reads the file's bytes, positions, mask words and totals in the device's shared memory,
/// where tier 1 wrote them. Or the word that refused it.
pub fn columns_on(parser: &Parser, run: &Resident, i: usize) -> Result<Columns, Refused> {
    let f = run.files[i];
    let source = &run.sources[i];
    let (pos, tot, out) = (run.positions.host().cast::<u32>(), run.tot.host().cast::<u32>(), run.out.host().cast::<u64>());
    // SAFETY: the run's buffers hold each file's counts from `tot_base`, its positions from
    // `pos_base` (`capacity` a stream), its output words from `out_base` (`blocks` an output),
    // with slack past the last of each for tier 2's reads; the device finished writing them.
    let count = |c: usize| unsafe { *tot.add(f.tot_base + c) } as usize;
    let starts = match parser.tier2.starts {
        Some(e) => (pos.wrapping_add(f.pos_base + e * f.capacity), count(e)),
        None => (pos, 0),
    };
    let outputs = out.wrapping_add(f.out_base);
    // The records' other columns: the last bytes, and the codes after the extracts'.
    let lasts = match parser.tier2.lasts {
        Some(e) => (pos.wrapping_add(f.pos_base + e * f.capacity), count(e)),
        None => (pos, 0),
    };
    let codes = pos.wrapping_add(f.pos_base + parser.tier1.extracts.len() * f.capacity);
    let view = Tier1View {
        starts: starts.0,
        nstarts: starts.1.min(lasts.1),
        lasts: lasts.0,
        codes,
        lasts_stride: 8,
        outputs,
        output_stride: 8 * f.blocks,
        totals: tot.wrapping_add(f.tot_base + parser.tier1.extracts.len()).cast(),
        totals_stride: 4,
        totals_mask: u64::from(u32::MAX),
    };
    let mut tape = parser.tier2.tape(starts.1);
    // SAFETY: as above; the source's bytes have `SOURCE_MARGIN` readable bytes after them in
    // its slab; the tape is sized for the stream.
    let r = unsafe { parser.tier2.call(source.slab.buffer.host().add(source.offset), source.len, view, &mut tape) };
    if r != 0 {
        return Err(Refused { pc: r as usize - 1, why: "the compiled words refuse" });
    }
    Ok(tape.columns())
}

