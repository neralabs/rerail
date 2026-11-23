//! Tier 2 on tier 1's buffers as the Metal backend left them (`rerail_metal_programs::block_program`): the compiled words
//! (`native_cpu_stream`) read a file's bytes, records (positions and codes), mask words and totals in the device's
//! shared memory, where tier 1 wrote them.

use rerail_metal_programs::block_program::serial::Resident;
use crate::native_cpu_stream::{CompiledStream, Parser, Tier1View};
use rerail_compiler::stream::Refused;

/// File `i`'s tape (the engine's bytes) from tier 1's buffers as the device left them
/// (`Resident`): tier 2 reads the file's bytes, positions, mask words and totals in the device's
/// shared memory, where tier 1 wrote them. Or the word that refused it. A file whose tape does not
/// fit the narrow widths is read again at wider ones (`Parser::again`).
pub fn image_on(parser: &Parser, run: &Resident, i: usize) -> Result<rerail_tape_format::Vector<u8>, Refused> {
    image_with(parser, &parser.tier2, run, i).or_else(|narrow| parser.again(narrow, |t2| image_with(parser, t2, run, i)))
}

/// `image_on`, by tier 2 `t2`.
fn image_with(parser: &Parser, t2: &CompiledStream, run: &Resident, i: usize) -> Result<rerail_tape_format::Vector<u8>, Refused> {
    let f = run.files[i];
    let source = &run.sources[i];
    let (pos, tot, out) = (run.positions.host().cast::<u32>(), run.tot.host().cast::<u32>(), run.out.host().cast::<u64>());
    // SAFETY: the run's buffers hold each file's counts from `tot_base`, its positions from
    // `pos_base` (`capacity` a stream), its output words from `out_base` (`blocks` an output),
    // with slack past the last of each for tier 2's reads; the device finished writing them.
    let count = |c: usize| unsafe { *tot.add(f.tot_base + c) } as usize;
    // The records where the device's layout has them (`records::RecordLayout`).
    let extracts = parser.tier1.extracts.len();
    let cols = [t2.starts.unwrap_or(0) as u64, t2.lasts.unwrap_or(0) as u64, extracts as u64];
    let field = |k: usize| pos.wrapping_add(f.pos_base + run.layout.index(k, 0, cols, f.capacity as u64) as usize);
    use rerail_compiler::records::{FIELD_CODE, FIELD_LAST, FIELD_START};
    let starts = match t2.starts {
        Some(e) => (field(FIELD_START), count(e)),
        None => (pos, 0),
    };
    let outputs = out.wrapping_add(f.out_base);
    let lasts = match t2.lasts {
        Some(e) => (field(FIELD_LAST), count(e)),
        None => (pos, 0),
    };
    let codes = field(FIELD_CODE);
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
        record_stride: run.layout.stride() as usize,
        more: 0,
        feed: 0,
    };
    let mut tape = t2.tape(starts.1);
    // The code counts on the program's padding (`BlockNative::pad`): the slab's margin is its.
    assert!(parser.tier1.pad <= rerail_metal::slabs::SOURCE_MARGIN, "a program padded past the slab's margin");
    // SAFETY: as above; the source's bytes have `SOURCE_MARGIN` zero bytes after them in its
    // slab, the program's padding at least; the tape is sized for the stream.
    let r = unsafe { t2.call(source.slab.host().add(source.offset), source.len, view, &mut tape) };
    if r != 0 {
        return Err(Refused { pc: r as usize - 1, why: "the compiled words refuse", at: None });
    }
    Ok(tape.image())
}

