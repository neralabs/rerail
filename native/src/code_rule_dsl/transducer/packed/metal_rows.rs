//! Row programs on Metal (decision 0095 §2, §3, §6): `scan` as `count` and `scatter` over a
//! relation held in the stratified 16-bit layout.
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
//!   operations; the tile's passing rows summed by `simd_sum` into its count;
//! - `rows_scan`: the counts' exclusive prefix sum, one threadgroup, each thread a run of tiles;
//! - `rows_scatter`: the predicate again, each passing row written at its tile's offset plus its
//!   rank (`simd_prefix_exclusive_sum`). The output is in order, with no atomic append.
//!
//! **16-bit comparisons.** Two `Low16` columns of a row share its chunk, so they compare as
//! `ushort`s. A `Low16` column against a parameter compares the chunk first (one value per
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
use super::metal_backend::launch::{commit_checked, gpu_seconds};
use super::metal_parallel::encode;
use std::fmt::Write as _;

/// A row program's input column: a gathered column, or the document of one (`x >> 32`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Col(usize),
    Doc(usize),
}


/// A scan's result: the host rows that pass, in order, and the command buffer's GPU time.
pub struct Scanned {
    pub rows: Vec<u32>,
    pub gpu_seconds: f64,
}

/// The MSL of a row program's predicate over `inputs` (program column `k` is `inputs[k]`), for
/// the columns' forms: the body of `pass`, one `bool` per operation.
pub(in crate::code_rule_dsl) fn predicate(program: &BlockProgram, inputs: &[Input], forms: &[Form]) -> Result<String, Error> {
    let shape = program.rows.ok_or(Error::UnsupportedBackend)?;
    if usize::from(shape.columns) != inputs.len() || shape.sets != 0 || program.extracts.len() != 1 {
        return Err(Error::UnsupportedBackend);
    }
    let low16 = |input: Input| matches!(input, Input::Col(g) if forms[g] == Form::Low16);
    // A gathered column's full value.
    let full = |g: usize| match forms[g] {
        Form::Low16 => format!("((high << 16) | long(c{g}[row]))"),
        Form::I32 => format!("long(c{g}[row])"),
        _ => format!("c{g}[row]"),
    };
    let value = |input: Input| match input {
        Input::Col(g) => full(g),
        Input::Doc(g) if forms[g] == Form::Low16 => "(high >> 16)".to_owned(),
        Input::Doc(g) => format!("({} >> 32)", full(g)),
    };
    let lane = |v: String, width: u8| match width {
        16 => format!("long(short({v}))"),
        32 => format!("long(int({v}))"),
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
                        let (h, l) = (format!("params[{}]", p + 1), format!("ushort(params[{}])", p + 2));
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

/// The three kernels of a row program over a resident relation's forms.
fn source(body: &str, forms: &[Form]) -> String {
    let ty = |f: Form| match f {
        Form::Low16 => "ushort",
        Form::I32 => "int",
        Form::I64 => "long",
    };
    let cols: String = forms.iter().enumerate().map(|(g, &f)| format!(", device const {}* c{g}", ty(f))).collect();
    let args: String = (0..forms.len()).map(|g| format!(", c{g}")).collect();
    let bufs: String = forms.iter().enumerate().map(|(g, &f)| format!(", device const {}* c{g} [[buffer({})]]", ty(f), 3 + g)).collect();
    let n = 3 + forms.len();
    format!(
        r#"#include <metal_stdlib>
using namespace metal;
struct Tile {{ uint group; uint start; uint count; uint pad; }};

static inline bool pass(uint row, long high, constant long* params{cols}) {{
{body}}}

kernel void rows_count(device const Tile* tiles [[buffer(0)]], device const long* highs [[buffer(1)]], constant long* params [[buffer(2)]]{bufs},
                       device uint* counts [[buffer({n})]],
                       uint tg [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]],
                       uint lane [[thread_index_in_simdgroup]], uint sg [[simdgroup_index_in_threadgroup]]) {{
    Tile tile = tiles[tg];
    bool p = t < tile.count && pass(tile.start + t, highs[tile.group], params{args});
    uint n = simd_sum(uint(p));
    threadgroup uint part[8];
    if (lane == 0) part[sg] = n;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (t == 0) {{
        uint s = 0;
        for (uint k = 0; k < 8; k++) s += part[k];
        counts[tg] = s;
    }}
}}

kernel void rows_scan(device const uint* counts [[buffer(0)]], device uint* offsets [[buffer(1)]], constant uint* n [[buffer(2)]],
                      uint t [[thread_index_in_threadgroup]]) {{
    uint per = (n[0] + 255) / 256;
    uint lo = min(n[0], t * per), hi = min(n[0], lo + per);
    uint s = 0;
    for (uint k = lo; k < hi; k++) s += counts[k];
    threadgroup uint part[256];
    part[t] = s;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint base = 0;
    for (uint k = 0; k < t; k++) base += part[k];
    for (uint k = lo; k < hi; k++) {{ offsets[k] = base; base += counts[k]; }}
    if (t == 255) offsets[n[0]] = base;
}}

kernel void rows_scatter(device const Tile* tiles [[buffer(0)]], device const long* highs [[buffer(1)]], constant long* params [[buffer(2)]]{bufs},
                         device const uint* offsets [[buffer({n})]], device const uint* perm [[buffer({n1})]], device uint* out [[buffer({n2})]],
                         uint tg [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]],
                         uint lane [[thread_index_in_simdgroup]], uint sg [[simdgroup_index_in_threadgroup]]) {{
    Tile tile = tiles[tg];
    bool p = t < tile.count && pass(tile.start + t, highs[tile.group], params{args});
    uint rank = simd_prefix_exclusive_sum(uint(p));
    threadgroup uint part[8];
    if (lane == 31) part[sg] = rank + uint(p);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint base = offsets[tg];
    for (uint k = 0; k < sg; k++) base += part[k];
    if (p) out[base + rank] = perm[tile.start + t];
}}
"#,
        n1 = n + 1,
        n2 = n + 2
    )
}

/// Runs a row program over a resident relation: count, scan and scatter in one command buffer.
/// `params` are the program's parameters; each goes to the device as itself, its chunk and its
/// low bits.
pub fn scan(program: &BlockProgram, inputs: &[Input], resident: &Resident, params: &[i64]) -> Result<Scanned, Error> {
    super::metal_program_backend::METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    // The launch budget every submission is checked against.
    let _scope = super::metal_backend::launch::LaunchScope::enter();
    let body = predicate(program, inputs, &resident.forms)?;
    let src = source(&body, &resident.forms);
    let count = super::metal_backend::pipeline_named(&src, "rows_count")?;
    let prefix = super::metal_backend::pipeline_named(&src, "rows_scan")?;
    let scatter = super::metal_backend::pipeline_named(&src, "rows_scatter")?;
    let wide: Vec<i64> = params.iter().flat_map(|&p| [p, p >> 16, p & 0xffff]).chain([0]).collect();
    let params = Held::of(wide.iter().copied()).map_err(device_err)?;
    let tiles = resident.tile_count;
    let counts = Held::new(4 * tiles).map_err(device_err)?;
    let offsets = Held::new(4 * (tiles + 1)).map_err(device_err)?;
    let n = Held::of([tiles as u32].into_iter()).map_err(device_err)?;
    // The outputs are at most the rows.
    let out = Held::new(4 * resident.rows).map_err(device_err)?;
    let queue = super::metal_sort::queue()?;
    let command = queue.new_command_buffer();
    let mut inputs: Vec<&metal::BufferRef> = vec![&resident.tiles.buffer, &resident.highs.buffer, &params.buffer];
    inputs.extend(resident.columns.iter().map(|b| &*b.buffer));
    if tiles > 0 {
        let mut with_counts = inputs.clone();
        with_counts.push(&counts.buffer);
        encode(command, &count, "rows_count", &with_counts, &[], tiles, None);
        encode(command, &prefix, "rows_scan", &[&counts.buffer, &offsets.buffer, &n.buffer], &[], 1, None);
        let mut with_out = inputs;
        with_out.extend([&*offsets.buffer, &*resident.perm.buffer, &*out.buffer]);
        encode(command, &scatter, "rows_scatter", &with_out, &[], tiles, None);
    }
    commit_checked(command)?;
    command.wait_until_completed();
    let gpu = gpu_seconds(command);
    // SAFETY: the buffers hold `tiles + 1` and `rows` words, written by the kernels and read
    // after the command buffer completed.
    let total = unsafe { *offsets.buffer.contents().cast::<u32>().add(tiles) } as usize;
    let rows = unsafe { std::slice::from_raw_parts(out.buffer.contents().cast::<u32>(), total) }.to_vec();
    Ok(Scanned { rows, gpu_seconds: gpu })
}

/// A failure of the one Metal manager (`rerail_accel::gpu`), in the backend's terms.
fn device_err(_: String) -> Error {
    Error::BackendFailure
}
