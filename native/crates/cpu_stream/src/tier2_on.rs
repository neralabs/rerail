//! Tier 2 on tier 1's buffers as the Metal backend left them (`rerail_metal_programs::block_program`): the compiled words
//! (`native_cpu_stream`) read a file's bytes, records (positions and codes), mask words and totals in the device's
//! shared memory, where tier 1 wrote them.

use rerail_metal_programs::block_program::serial::Resident;
use crate::native_cpu_stream::{CompiledStream, Parser, Tier1View};

/// File `i`'s tape in format 16 (16z where `z`) from tier 1's buffers as the device left them (`Resident`), by tier 2
/// (`Parser::tier2`), each node numbered in `lang` (`numbers`), its lines those
/// `terminators` end; or why it is not one (a word refused the file).
pub fn image16_on(parser: &Parser, run: &Resident, i: usize, terminators: rerail_tape_format::newlines::Terminators, lang: &rerail_tape_format::f16::Lang, numbers: &crate::native_cpu_stream::Numbers, z: bool) -> Result<rerail_tape_format::f16::Written, String> {
    // The device's records are its layout's (`RecordLayout::choose`: columns, where the CPU's are rows): the tier 2
    // that reads that stride (0696).
    let t2 = parser.tier2_at(run.layout.stride()).ok_or("no tier 2 that reads the device's records")?;
    let (view, n) = tier1_view(parser, t2, run, i);
    let source = &run.sources[i];
    // The code counts on the program's padding (`BlockNative::pad`): the slab's margin is its.
    assert!(parser.tier1.pad <= rerail_metal::slabs::SOURCE_MARGIN, "a program padded past the slab's margin");
    // SAFETY: the source's bytes are `len` bytes at its offset in its slab, which the device finished with.
    let bytes = unsafe { std::slice::from_raw_parts(source.slab.host().add(source.offset), source.len) };
    // SAFETY: as `tier1_view`'s; the source's bytes have `SOURCE_MARGIN` zero bytes after them in its slab, the
    // program's padding at least; the tape is sized for the stream.
    t2.image16_by(bytes, crate::native_cpu_stream::Out16 { lang, numbers, terminators, z }, t2.tokens(n, source.len), |tape| unsafe { t2.call(bytes.as_ptr(), bytes.len(), view, tape) })
}

/// File `i`'s tier 1 records, mask words and totals as tier 2 `t2` reads them in the device's buffers, and its records'
/// count.
fn tier1_view(parser: &Parser, t2: &CompiledStream, run: &Resident, i: usize) -> (Tier1View, usize) {
    let f = run.files[i];
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
    (view, starts.1)
}
