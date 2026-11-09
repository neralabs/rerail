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
use super::metal_backend::MappedBuffer;
use super::metal_backend::launch::{commit_checked, gpu_seconds};
use super::metal_parallel::encode;
use std::fmt::Write as _;

/// A row program's input column: a gathered column, or the document of one (`x >> 32`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Input {
    Col(usize),
    Doc(usize),
}

/// How a gathered column is held on the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Form {
    Low16,
    I32,
    I64,
}

/// A relation's columns on the device, grouped by the lead column's chunk.
pub(in crate::code_rule_dsl) struct Resident {
    pub(in crate::code_rule_dsl) rows: usize,
    pub(in crate::code_rule_dsl) groups: usize,
    pub(in crate::code_rule_dsl) forms: Vec<Form>,
    /// The bytes the columns take on the device, and as `i64`s.
    pub(in crate::code_rule_dsl) bytes: usize,
    tiles: MappedBuffer,
    tile_count: usize,
    highs: MappedBuffer,
    columns: Vec<MappedBuffer>,
    /// Each device row's place in the host's rows.
    perm: MappedBuffer,
}

const TILE: usize = 256;

impl Resident {
    /// `columns` (each `rows` long) on the device, grouped by column 0's chunk.
    pub(in crate::code_rule_dsl) fn new(columns: &[&[i64]], rows: usize) -> Result<Self, Error> {
        let device = super::metal_program_backend::METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
        let lead = columns.first().ok_or(Error::BackendFailure)?;
        let mut perm: Vec<u32> = (0..rows as u32).collect();
        perm.sort_by_key(|&r| lead[r as usize] >> 16);
        // Groups: runs of one chunk; tiles: a group's rows 256 at a time.
        let (mut highs, mut tiles, mut group_of) = (Vec::<i64>::new(), Vec::<[u32; 4]>::new(), vec![0u32; rows]);
        let mut at = 0;
        while at < rows {
            let high = lead[perm[at] as usize] >> 16;
            let end = at + perm[at..].iter().take_while(|&&r| lead[r as usize] >> 16 == high).count();
            let g = highs.len() as u32;
            highs.push(high);
            group_of[at..end].fill(g);
            for start in (at..end).step_by(TILE) {
                tiles.push([g, start as u32, (end - start).min(TILE) as u32, 0]);
            }
            at = end;
        }
        let mut forms = Vec::with_capacity(columns.len());
        let mut buffers = Vec::with_capacity(columns.len());
        let mut bytes = 0;
        for c in columns {
            let value = |i: usize| c[perm[i] as usize];
            let form = if (0..rows).all(|i| value(i) >> 16 == highs[group_of[i] as usize]) {
                Form::Low16
            } else if (0..rows).all(|i| i32::try_from(value(i)).is_ok()) {
                Form::I32
            } else {
                Form::I64
            };
            let buffer = match form {
                Form::Low16 => {
                    let v: Vec<u16> = (0..rows).map(|i| value(i) as u16).collect();
                    bytes += 2 * rows;
                    MappedBuffer::from_slice(device, if v.is_empty() { &[0u16] } else { &v })?
                }
                Form::I32 => {
                    let v: Vec<i32> = (0..rows).map(|i| value(i) as i32).collect();
                    bytes += 4 * rows;
                    MappedBuffer::from_slice(device, if v.is_empty() { &[0i32] } else { &v })?
                }
                Form::I64 => {
                    let v: Vec<i64> = (0..rows).map(value).collect();
                    bytes += 8 * rows;
                    MappedBuffer::from_slice(device, if v.is_empty() { &[0i64] } else { &v })?
                }
            };
            forms.push(form);
            buffers.push(buffer);
        }
        bytes += 8 * highs.len() + 16 * tiles.len();
        let tile_count = tiles.len();
        Ok(Resident {
            rows,
            groups: highs.len(),
            forms,
            bytes,
            tiles: MappedBuffer::from_slice(device, if tiles.is_empty() { &[[0u32; 4]] } else { &tiles })?,
            tile_count,
            highs: MappedBuffer::from_slice(device, if highs.is_empty() { &[0i64] } else { &highs })?,
            columns: buffers,
            perm: MappedBuffer::from_slice(device, if perm.is_empty() { &[0u32] } else { &perm })?,
        })
    }
}

/// A scan's result: the host rows that pass, in order, and the command buffer's GPU time.
pub(in crate::code_rule_dsl) struct Scanned {
    pub(in crate::code_rule_dsl) rows: Vec<u32>,
    pub(in crate::code_rule_dsl) gpu_seconds: f64,
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
        Form::I64 => format!("c{g}[row]"),
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
pub(in crate::code_rule_dsl) fn scan(program: &BlockProgram, inputs: &[Input], resident: &Resident, params: &[i64]) -> Result<Scanned, Error> {
    let device = super::metal_program_backend::METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    // The launch budget every submission is checked against.
    let _scope = super::metal_backend::launch::LaunchScope::enter();
    let body = predicate(program, inputs, &resident.forms)?;
    let src = source(&body, &resident.forms);
    let count = super::metal_backend::pipeline_named(&src, "rows_count")?;
    let prefix = super::metal_backend::pipeline_named(&src, "rows_scan")?;
    let scatter = super::metal_backend::pipeline_named(&src, "rows_scatter")?;
    let wide: Vec<i64> = params.iter().flat_map(|&p| [p, p >> 16, p & 0xffff]).chain([0]).collect();
    let params = MappedBuffer::from_slice(device, &wide)?;
    let tiles = resident.tile_count;
    let counts = MappedBuffer::zeroed::<u32>(device, tiles.max(1))?;
    let offsets = MappedBuffer::zeroed::<u32>(device, tiles + 1)?;
    let n = MappedBuffer::from_slice(device, &[tiles as u32])?;
    // The outputs are at most the rows.
    let out = MappedBuffer::zeroed::<u32>(device, resident.rows.max(1))?;
    let queue = device.new_command_queue();
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

/// A join's pairs (`tile_join`): each pair's host rows, left and right, and the GPU time.
pub(in crate::code_rule_dsl) struct Joined {
    pub(in crate::code_rule_dsl) pairs: Vec<(u32, u32)>,
    pub(in crate::code_rule_dsl) gpu_seconds: f64,
    /// The pairs of rows the kernel compared: the brute force's work.
    pub(in crate::code_rule_dsl) compared: u64,
}

impl Resident {
    /// Each group's chunk and its rows (first, count), in chunk order.
    fn group_rows(&self) -> Vec<(i64, u32, u32)> {
        // SAFETY: the buffers hold `tile_count` tiles and `groups` chunks, written at `new`.
        let tiles = unsafe { std::slice::from_raw_parts(self.tiles.buffer.contents().cast::<[u32; 4]>(), self.tile_count) };
        let highs = unsafe { std::slice::from_raw_parts(self.highs.buffer.contents().cast::<i64>(), self.groups) };
        let mut out: Vec<(i64, u32, u32)> = Vec::with_capacity(self.groups);
        for t in tiles {
            match out.last_mut() {
                Some(last) if last.0 == highs[t[0] as usize] => last.2 += t[2],
                _ => out.push((highs[t[0] as usize], t[1], t[2])),
            }
        }
        out
    }
}

/// The equi-join of two resident relations on key columns, by brute force within a chunk
/// (decision 0095 §4, `tile`): both relations grouped by their key 0 (each one's column 0, a node
/// column in the 16-bit layout on both sides), so a pair whose key 0 is equal is in one chunk. A
/// threadgroup takes 256 left rows of a chunk; the chunk's right rows come through threadgroup
/// memory 256 at a time (their keys: 16 bits for key 0, 64 for each other), and each thread
/// compares its row with each. Two passes, count and scatter, with a scan of the threads' counts
/// between: the pairs in order, sized exactly.
pub(in crate::code_rule_dsl) fn tile_join(left: &Resident, lkeys: &[usize], right: &Resident, rkeys: &[usize]) -> Result<Joined, Error> {
    let device = super::metal_program_backend::METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    if lkeys.is_empty() || lkeys.len() != rkeys.len() || lkeys.len() > 3 || lkeys[0] != 0 || rkeys[0] != 0 || left.forms[0] != Form::Low16 || right.forms[0] != Form::Low16 {
        return Err(Error::UnsupportedBackend);
    }
    let _scope = super::metal_backend::launch::LaunchScope::enter();
    // The join's tiles: each left tile of a chunk the right also has, with the right's rows of it.
    let (lg, rg) = (left.group_rows(), right.group_rows());
    let mut tiles: Vec<[u32; 4]> = Vec::new();
    let mut highs: Vec<i64> = Vec::new();
    let mut compared = 0u64;
    let mut r = 0;
    for &(high, start, count) in &lg {
        while r < rg.len() && rg[r].0 < high {
            r += 1;
        }
        if r < rg.len() && rg[r].0 == high {
            compared += u64::from(count) * u64::from(rg[r].2);
            for s in (start..start + count).step_by(TILE) {
                tiles.push([s, (start + count - s).min(TILE as u32), rg[r].1, rg[r].2]);
                highs.push(high);
            }
        }
    }
    let ty = |f: Form| match f {
        Form::Low16 => "ushort",
        Form::I32 => "int",
        Form::I64 => "long",
    };
    let full = |side: char, g: usize, f: Form, row: &str| match f {
        Form::Low16 => format!("((high << 16) | long({side}{g}[{row}]))"),
        Form::I32 => format!("long({side}{g}[{row}])"),
        Form::I64 => format!("{side}{g}[{row}]"),
    };
    let k = lkeys.len();
    let lcols: String = lkeys.iter().enumerate().map(|(i, &c)| format!(", device const {}* l{i} [[buffer({})]]", ty(left.forms[c]), 3 + i)).collect();
    let rcols: String = rkeys.iter().enumerate().map(|(i, &c)| format!(", device const {}* r{i} [[buffer({})]]", ty(right.forms[c]), 3 + k + i)).collect();
    let n = 3 + 2 * k;
    // Stage the right keys; compare.
    let mut stage = String::from("        if (j < rc) { s0[t] = r0[rs + j]; }\n");
    let mut decl = String::from("    threadgroup ushort s0[256];\n");
    let mut test = String::from("s0[q] == mine0");
    let mut mine = String::from("    ushort mine0 = live ? l0[lrow] : 0;\n");
    for i in 1..k {
        decl.push_str(&format!("    threadgroup long s{i}[256];\n"));
        stage.push_str(&format!("        if (j < rc) {{ s{i}[t] = {}; }}\n", full('r', i, right.forms[rkeys[i]], "rs + j")));
        mine.push_str(&format!("    long mine{i} = live ? {} : 0;\n", full('l', i, left.forms[lkeys[i]], "lrow")));
        test.push_str(&format!(" && s{i}[q] == mine{i}"));
    }
    let src = format!(
        r#"#include <metal_stdlib>
using namespace metal;
struct JoinTile {{ uint ls; uint lc; uint rs; uint rc; }};

static inline void pairs(uint tg, uint t, device const JoinTile* tiles, device const long* highs{lp}, device const uint* offsets, device const uint* lperm, device const uint* rperm, device uint* out, device uint* counts, bool write{tgp}) {{
    JoinTile tile = tiles[tg];
    long high = highs[tg];
    bool live = t < tile.lc;
    uint lrow = tile.ls + min(t, tile.lc - 1);
{mine}    uint n = 0;
    uint at = write ? offsets[tg * 256 + t] : 0;
    for (uint base = 0; base < tile.rc; base += 256) {{
        uint j = base + t;
        uint rc = tile.rc;
        uint rs = tile.rs;
{stage}        threadgroup_barrier(mem_flags::mem_threadgroup);
        uint m = min(256u, rc - base);
        for (uint q = 0; q < m; q++) {{
            if (live && {test}) {{
                if (write) {{ out[2 * (at + n)] = lperm[lrow]; out[2 * (at + n) + 1] = rperm[rs + base + q]; }}
                n++;
            }}
        }}
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }}
    if (!write) counts[tg * 256 + t] = n;
}}

kernel void tile_count(device const JoinTile* tiles [[buffer(0)]], device const long* highs [[buffer(1)]], constant uint* unused [[buffer(2)]]{lcols}{rcols},
                       device uint* counts [[buffer({n})]], uint tg [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]]) {{
{decl}    pairs(tg, t, tiles, highs{la}, counts, counts, counts, counts, counts, false{tga});
}}

kernel void tile_scatter(device const JoinTile* tiles [[buffer(0)]], device const long* highs [[buffer(1)]], constant uint* unused [[buffer(2)]]{lcols}{rcols},
                         device const uint* offsets [[buffer({n})]], device const uint* lperm [[buffer({n1})]], device const uint* rperm [[buffer({n2})]], device uint* out [[buffer({n3})]],
                         uint tg [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]]) {{
{decl}    pairs(tg, t, tiles, highs{la}, offsets, lperm, rperm, out, out, true{tga});
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
"#,
        lp = (0..k).map(|i| format!(", device const {}* l{i}", ty(left.forms[lkeys[i]]))).chain((0..k).map(|i| format!(", device const {}* r{i}", ty(right.forms[rkeys[i]])))).collect::<String>(),
        la = (0..k).map(|i| format!(", l{i}")).chain((0..k).map(|i| format!(", r{i}"))).collect::<String>(),
        n1 = n + 1,
        n2 = n + 2,
        n3 = n + 3,
        tgp = (0..k).map(|i| if i == 0 { ", threadgroup ushort* s0".to_owned() } else { format!(", threadgroup long* s{i}") }).collect::<String>(),
        tga = (0..k).map(|i| format!(", s{i}")).collect::<String>(),
    );
    let count = super::metal_backend::pipeline_named(&src, "tile_count")?;
    let prefix = super::metal_backend::pipeline_named(&src, "rows_scan")?;
    let scatter = super::metal_backend::pipeline_named(&src, "tile_scatter")?;
    let threads = tiles.len() * TILE;
    let tiles_buf = MappedBuffer::from_slice(device, if tiles.is_empty() { &[[0u32; 4]] } else { &tiles })?;
    let highs_buf = MappedBuffer::from_slice(device, if highs.is_empty() { &[0i64] } else { &highs })?;
    let unused = MappedBuffer::zeroed::<u32>(device, 1)?;
    let counts = MappedBuffer::zeroed::<u32>(device, threads.max(1))?;
    let offsets = MappedBuffer::zeroed::<u32>(device, threads + 1)?;
    let nbuf = MappedBuffer::from_slice(device, &[threads as u32])?;
    let mut inputs: Vec<&metal::BufferRef> = vec![&tiles_buf.buffer, &highs_buf.buffer, &unused.buffer];
    inputs.extend(lkeys.iter().map(|&c| &*left.columns[c].buffer));
    inputs.extend(rkeys.iter().map(|&c| &*right.columns[c].buffer));
    let queue = device.new_command_queue();
    // The count pass alone first: the pairs are sized from it.
    let command = queue.new_command_buffer();
    if !tiles.is_empty() {
        let mut with = inputs.clone();
        with.push(&counts.buffer);
        encode(command, &count, "tile_count", &with, &[], tiles.len(), None);
        encode(command, &prefix, "rows_scan", &[&counts.buffer, &offsets.buffer, &nbuf.buffer], &[], 1, None);
    }
    commit_checked(command)?;
    command.wait_until_completed();
    let mut gpu = gpu_seconds(command);
    // SAFETY: `threads + 1` offsets, written by the scan, read after completion.
    let total = unsafe { *offsets.buffer.contents().cast::<u32>().add(threads) } as usize;
    let out = MappedBuffer::zeroed::<u32>(device, (2 * total).max(1))?;
    if total > 0 {
        let command = queue.new_command_buffer();
        let mut with = inputs;
        with.extend([&*offsets.buffer, &*left.perm.buffer, &*right.perm.buffer, &*out.buffer]);
        encode(command, &scatter, "tile_scatter", &with, &[], tiles.len(), None);
        commit_checked(command)?;
        command.wait_until_completed();
        gpu += gpu_seconds(command);
    }
    // SAFETY: `2 * total` words, written by the scatter.
    let flat = unsafe { std::slice::from_raw_parts(out.buffer.contents().cast::<u32>(), 2 * total) };
    let pairs = flat.chunks_exact(2).map(|p| (p[0], p[1])).collect();
    Ok(Joined { pairs, gpu_seconds: gpu, compared })
}
