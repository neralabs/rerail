//! Row programs on the device (decision 0095 §2, §3, §6): `scan` as `count` and `scatter` over a
//! relation held in the stratified 16-bit layout, in UKL (decision 0440 step C) on any backend.
//!
//! **The layout.** The rows are grouped by their lead column's chunk, its high part (`v >> 16`):
//! a group carries its chunk once, and each row its own 16 low bits. Every other column is kept as
//! narrow as its values let it, exactly:
//! - `Low16`: every value is in its row's chunk, so a row holds its low 16 bits;
//! - `I32`: every value fits 32 bits;
//! - `I64`: the rest.
//!
//! A group is cut into tiles of 256 rows, one threadgroup each.
//!
//! **The kernels**, generated from the row program (one generator, not one per case):
//! - `rows_count`: each thread a row, its predicate a straight-line expression of the program's
//!   operations; the tile's passing rows summed by a SIMD group sum into its count;
//! - the counts' exclusive prefix sum: the UKL library's scan (`rerail_accel::gpu::join::Scan`);
//! - `rows_scatter`: the predicate again, each passing row written at its tile's offset plus its
//!   rank (a SIMD group's exclusive sum). The output is in order, with no atomic append.
//!
//! **16-bit comparisons.** Two `Low16` columns of a row share its chunk, so they compare as
//! `uint16_t`s. A `Low16` column against a parameter compares the chunk first (one value per
//! threadgroup) and the low bits only when the chunks are equal. `@doc(x)` of a `Low16` column is
//! the chunk's: `high >> 16`, the same for the whole tile.
//!
//! Threadgroup memory: eight words. Every loop is bounded by its input.

use super::Error;
use super::block::{BlockOp, BlockProgram, CmpOp, Rhs};
use super::metal_sort::Held;
pub use rerail_accel::gpu::tile::{Form, Resident};
#[cfg(any(test, feature = "test-hooks"))]
pub use rerail_accel::gpu::tile::{Joined, tile_join};
use rerail_accel::gpu::join::{Pass, Scan, pipeline};
use rerail_device::device::Buffer;
use std::fmt::Write as _;

/// A row program's input column: a gathered column, or the document of one (`x >> 32`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Col(usize),
    Doc(usize),
}


/// A scan's result: the host rows that pass, in order, and the submission's device time.
pub struct Scanned {
    pub rows: Vec<u32>,
    pub gpu_seconds: f64,
}

/// The UKL of a row program's predicate over `inputs` (program column `k` is `inputs[k]`), for
/// the columns' forms: the body of `pass`, one `bool` per operation.
pub(in crate::code_rule_dsl) fn predicate(program: &BlockProgram, inputs: &[Input], forms: &[Form]) -> Result<String, Error> {
    let shape = program.rows.ok_or(Error::UnsupportedBackend)?;
    if usize::from(shape.columns) != inputs.len() || shape.sets != 0 || program.extracts.len() != 1 {
        return Err(Error::UnsupportedBackend);
    }
    let low16 = |input: Input| matches!(input, Input::Col(g) if forms[g] == Form::Low16);
    // A gathered column's full value.
    let full = |g: usize| match forms[g] {
        Form::Low16 => format!("((high << 16) | (int64_t)c{g}[row])"),
        Form::I32 => format!("(int64_t)c{g}[row]"),
        _ => format!("c{g}[row]"),
    };
    let value = |input: Input| match input {
        Input::Col(g) => full(g),
        Input::Doc(g) if forms[g] == Form::Low16 => "(high >> 16)".to_owned(),
        Input::Doc(g) => format!("({} >> 32)", full(g)),
    };
    let lane = |v: String, width: u8| match width {
        16 => format!("(int64_t)(int16_t)({v})"),
        32 => format!("(int64_t)(int32_t)({v})"),
        _ => v,
    };
    let sym = |op: CmpOp| match op {
        CmpOp::Eq => "==",
        CmpOp::Ne => "!=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
    };
    let mut body = String::new();
    for (x, op) in program.ops.iter().enumerate() {
        let e = match *op {
            BlockOp::Cmp { col, op, rhs, width } => {
                let a = inputs[usize::from(col)];
                match rhs {
                    // Two columns of a row's chunk: their low 16 bits.
                    Rhs::Col(c) if width == 64 && low16(a) && low16(inputs[usize::from(c)]) => {
                        let (Input::Col(ga), Input::Col(gb)) = (a, inputs[usize::from(c)]) else { unreachable!("low16 columns") };
                        format!("c{ga}[row] {} c{gb}[row]", sym(op))
                    }
                    // A chunk's column against a parameter: the chunk, then the low bits.
                    Rhs::Param(p) if width == 64 && low16(a) => {
                        let Input::Col(g) = a else { unreachable!("a low16 column") };
                        let p = 3 * usize::from(p);
                        let (h, l) = (format!("params[{}]", p + 1), format!("(uint16_t)params[{}]", p + 2));
                        let lo = format!("c{g}[row]");
                        match op {
                            CmpOp::Eq => format!("(high == {h} && {lo} == {l})"),
                            CmpOp::Ne => format!("(high != {h} || {lo} != {l})"),
                            CmpOp::Lt => format!("(high < {h} || (high == {h} && {lo} < {l}))"),
                            CmpOp::Le => format!("(high < {h} || (high == {h} && {lo} <= {l}))"),
                        }
                    }
                    Rhs::Col(c) => format!("{} {} {}", lane(value(a), width), sym(op), lane(value(inputs[usize::from(c)]), width)),
                    Rhs::Param(p) => format!("{} {} {}", lane(value(a), width), sym(op), lane(format!("params[{}]", 3 * usize::from(p)), width)),
                }
            }
            BlockOp::Const(0) => "false".to_owned(),
            BlockOp::Const(u64::MAX) => "true".to_owned(),
            BlockOp::Not(a) => format!("!v{a}"),
            BlockOp::And(a, b) => format!("v{a} && v{b}"),
            BlockOp::Or(a, b) => format!("v{a} || v{b}"),
            BlockOp::Xor(a, b) => format!("v{a} != v{b}"),
            BlockOp::AndNot(a, b) => format!("v{a} && !v{b}"),
            // A pattern by row number, a set: not on the device yet.
            _ => return Err(Error::UnsupportedBackend),
        };
        let _ = writeln!(body, "    bool v{x} = {e};");
    }
    let _ = writeln!(body, "    return v{};", program.extracts[0]);
    Ok(body)
}

/// The two kernels of a row program over a resident relation's forms (the scan between them is the library's).
fn source(body: &str, forms: &[Form]) -> String {
    let ty = |f: Form| match f {
        Form::Low16 => "uint16_t",
        Form::I32 => "int32_t",
        Form::I64 => "int64_t",
    };
    let cols: String = forms.iter().enumerate().map(|(g, &f)| format!(", UKL_GLOBAL const {}* c{g}", ty(f))).collect();
    let args: String = (0..forms.len()).map(|g| format!(", c{g}")).collect();
    let rows: String = forms.iter().enumerate().map(|(g, &f)| format!("               ROBUFFER, {}, c{g}, \"[rows]\",\n", ty(f))).collect();
    let common = rerail_accel::gpu::join::COMMON;
    format!(
        r#"{common}
typedef struct Tile {{ uint32_t group; uint32_t start; uint32_t count; uint32_t pad; }} Tile;

UKL_INLINE bool pass(uint32_t row, int64_t high, UKL_GLOBAL const int64_t* params{cols}) {{
{body}}}

// Each tile's passing rows.
DEFINE_KERNEL2(rows, rows_count,
               ROBUFFER, Tile, tiles, "[tiles]",
               ROBUFFER, int64_t, highs, "[groups]",
               ROBUFFER, int64_t, params, "[3 * params + 1]",
{rows}               RWBUFFER, uint32_t, counts, "[tiles]",
               SHARED, uint32_t, part, JOIN_SGS,
               TGID0, uint32_t, tg, ,
               LID0, uint32_t, t, ,
               LANE, uint32_t, lane, )
{{
  Tile tile = tiles[tg];
  bool p = t < tile.count && pass(tile.start + t, highs[tile.group], params{args});
  uint32_t total = 0u;
  join_tg_prefix((uint32_t)p, t, lane, part, &total __UKL_ARGS);
  if (t == 0u) counts[tg] = total;
}}

// Each tile's passing rows at its offset, in order.
DEFINE_KERNEL2(rows, rows_scatter,
               ROBUFFER, Tile, tiles, "[tiles]",
               ROBUFFER, int64_t, highs, "[groups]",
               ROBUFFER, int64_t, params, "[3 * params + 1]",
{rows}               ROBUFFER, uint32_t, offsets, "[tiles + 1]",
               ROBUFFER, uint32_t, perm, "[rows]",
               RWBUFFER, uint32_t, out, "[rows]",
               SHARED, uint32_t, part, JOIN_SGS,
               TGID0, uint32_t, tg, ,
               LID0, uint32_t, t, ,
               LANE, uint32_t, lane, )
{{
  Tile tile = tiles[tg];
  bool p = t < tile.count && pass(tile.start + t, highs[tile.group], params{args});
  uint32_t total = 0u;
  uint32_t rank = join_tg_prefix((uint32_t)p, t, lane, part, &total __UKL_ARGS);
  if (p) out[offsets[tg] + rank] = perm[tile.start + t];
}}
"#
    )
}

/// Runs a row program over a resident relation: count, scan and scatter in one submission.
/// `params` are the program's parameters; each goes to the device as itself, its chunk and its
/// low bits.
pub fn scan(program: &BlockProgram, inputs: &[Input], resident: &Resident, params: &[i64]) -> Result<Scanned, Error> {
    super::metal_sort::ready()?;
    // An evaluation of the launch budget, or the caller's when it is in one: the pass's launches are charged to it.
    let _scope = super::metal_backend::launch::LaunchScope::enter();
    let body = predicate(program, inputs, &resident.forms)?;
    let src = source(&body, &resident.forms);
    // One variant per program's source: the predicate is the program's.
    let count = pipeline(&src, "rows_count", || src.clone()).map_err(device_err)?;
    let scatter = pipeline(&src, "rows_scatter", || src.clone()).map_err(device_err)?;
    let wide: Vec<i64> = params.iter().flat_map(|&p| [p, p >> 16, p & 0xffff]).chain([0]).collect();
    let params = Held::of(wide.iter().copied()).map_err(device_err)?;
    let tiles = resident.tile_count;
    let counts = Held::new(4 * tiles).map_err(device_err)?;
    let offsets = Held::new(4 * (tiles + 1)).map_err(device_err)?;
    // This thread's scratch: the submission is awaited here.
    let prefix = Scan::new(tiles).map_err(device_err)?;
    // The outputs are at most the rows.
    let out = Held::new(4 * resident.rows).map_err(device_err)?;
    let pass = Pass::new().map_err(device_err)?;
    let mut inputs: Vec<&Buffer> = vec![&resident.tiles.buffer, &resident.highs.buffer, &params.buffer];
    inputs.extend(resident.columns.iter().map(|b| &b.buffer));
    if tiles > 0 {
        let mut with_counts = inputs.clone();
        with_counts.push(&counts.buffer);
        pass.run(&count, &with_counts, &[], &[], tiles);
        prefix.encode_into(&pass, &counts.buffer, &offsets.buffer).map_err(device_err)?;
        let mut with_out = inputs;
        with_out.extend([&offsets.buffer, &resident.perm.buffer, &out.buffer]);
        pass.run(&scatter, &with_out, &[], &[], tiles);
    } else {
        // SAFETY: the total's word, read after the submission; with no tiles nothing writes it.
        unsafe { *offsets.buffer.contents().cast::<u32>() = 0 };
    }
    let (gpu, ok) = pass.commit().map_err(Error::from)?.wait_ok();
    if !ok {
        return Err(Error::BackendFailure);
    }
    // SAFETY: the buffers hold `tiles + 1` and `rows` words, written by the kernels and read
    // after the command buffer completed.
    let total = unsafe { *offsets.buffer.contents().cast::<u32>().add(tiles) } as usize;
    let rows = unsafe { std::slice::from_raw_parts(out.buffer.contents().cast::<u32>(), total) }.to_vec();
    Ok(Scanned { rows, gpu_seconds: gpu })
}

/// A failure of the one device manager (`rerail_accel::gpu`), in the backend's terms.
fn device_err(_: String) -> Error {
    Error::BackendFailure
}
