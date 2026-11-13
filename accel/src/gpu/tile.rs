//! The tile join on the device (decision 0095b §4): two relations grouped by their key 0's chunk, compared by brute
//! force within a chunk. The GPU lane's `join` kernel takes it for joins grouped by document (decision 0100 §1);
//! the engine's row programs read the same layout (`Resident`).
//!
//! **The layout.** The rows are grouped by their lead column's chunk, its high part (`v >> 16`): a group carries its
//! chunk once, and each row its own 16 low bits. Every other column is kept as narrow as its values let it, exactly:
//! `Low16` (every value in its row's chunk), `I32`, or `I64`. A group is cut into tiles of 256 rows.

use super::join::{Held, Pass, compile, queue};
use super::shared;

/// How a gathered column is held on the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Low16,
    I32,
    I64,
}

/// A relation's columns on the device, grouped by the lead column's chunk.
pub struct Resident {
    /// Each group's chunk and its rows (first, count), in order.
    pub group_list: Vec<(i64, u32, u32)>,
    pub rows: usize,
    pub groups: usize,
    pub forms: Vec<Form>,
    /// The bytes the columns take on the device.
    pub bytes: usize,
    pub tiles: Held,
    pub tile_count: usize,
    pub highs: Held,
    pub columns: Vec<Held>,
    /// Each device row's place in the host's rows.
    pub perm: Held,
}

const TILE: usize = 256;

/// `values` (as `T`, each exactly) written into a pooled buffer.
fn packed<T: Copy>(_device: &metal::Device, values: impl ExactSizeIterator<Item = T>) -> Result<Held, String> {
    Held::of(values)
}

impl Resident {
    /// `columns` (each `rows` long) on the device, grouped by column 0's chunk. The host's part: a radix sort of
    /// the lead's chunks (`rows_sort`), then per column one gather in that order, its form from the gathered
    /// values (each in its row's chunk; its span), and one pass writing it into its buffer.
    pub fn new(columns: &[&[i64]], rows: usize) -> Result<Self, String> {
        let device = &shared::get().ok_or("no Metal device")?.device;
        let lead = columns.first().ok_or("a relation of no columns")?;
        let chunks: Vec<i64> = lead[..rows].iter().map(|&v| v >> 16).collect();
        let sorted = crate::rows::sort(&chunks);
        let group_list = crate::rows::runs(&sorted.keys);
        // Tiles: a group's rows 256 at a time.
        let (mut highs, mut tiles) = (Vec::with_capacity(group_list.len()), Vec::<[u32; 4]>::new());
        for (g, &(high, start, count)) in group_list.iter().enumerate() {
            highs.push(high);
            for s in (start..start + count).step_by(TILE) {
                tiles.push([g as u32, s, (start + count - s).min(TILE as u32), 0]);
            }
        }
        let perm = sorted.perm;
        let mut forms = Vec::with_capacity(columns.len());
        let mut buffers = Vec::with_capacity(columns.len());
        let mut bytes = 0;
        let mut v: Vec<i64> = Vec::with_capacity(rows);
        for c in columns {
            v.clear();
            v.extend(perm.iter().map(|&r| c[r as usize]));
            let low16 = v.iter().zip(&sorted.keys).all(|(&x, &h)| x >> 16 == h);
            let (min, max) = crate::rows::span(&v).unwrap_or((0, 0));
            let (form, buffer) = if low16 {
                bytes += 2 * rows;
                (Form::Low16, packed(device, v.iter().map(|&x| x as u16))?)
            } else if min >= i64::from(i32::MIN) && max <= i64::from(i32::MAX) {
                bytes += 4 * rows;
                (Form::I32, packed(device, v.iter().map(|&x| x as i32))?)
            } else {
                bytes += 8 * rows;
                (Form::I64, packed(device, v.iter().copied())?)
            };
            forms.push(form);
            buffers.push(buffer);
        }
        bytes += 8 * highs.len() + 16 * tiles.len();
        let tile_count = tiles.len();
        Ok(Resident {
            rows,
            groups: group_list.len(),
            group_list,
            forms,
            bytes,
            tiles: packed(device, tiles.into_iter())?,
            tile_count,
            highs: packed(device, highs.into_iter())?,
            columns: buffers,
            perm: packed(device, perm.into_iter())?,
        })
    }
}

/// A join's pairs (`tile_join`): each pair's host rows, left and right, and the GPU time.
pub struct Joined {
    pub pairs: Vec<(u32, u32)>,
    pub gpu_seconds: f64,
    /// The pairs of rows the kernel compared: the brute force's work.
    pub compared: u64,
}

/// The equi-join of two resident relations on key columns, by brute force within a document (decision 0095b §4,
/// `tile`): both relations grouped by their key 0 (each one's column 0, a node column in the 16-bit layout on both
/// sides), so a pair whose key 0 is equal is in one group, a document's rows. A threadgroup takes 256 left rows of
/// a group; the group's right rows come through threadgroup memory 256 at a time (each key 16 bits when both sides
/// hold it as `Low16`, else 64), and each thread compares its row with each. Two passes, count and scatter, with a
/// scan of the threads' counts between: the pairs in order, sized exactly. For a plain equi-join the sort and probe
/// (`metal_sort`) does only the pairs' work; the tile is for pairs a key cannot find (containment, order).
pub fn tile_join(left: &Resident, lkeys: &[usize], right: &Resident, rkeys: &[usize]) -> Result<Joined, String> {
    let queue = queue()?;
    let command = queue.new_command_buffer();
    let pass = Pass::new(command);
    let job = TileJob::count(&pass, left, lkeys, right, rkeys)?;
    pass.end();
    let mut gpu = super::join::commit_wait(command)?;
    let command = queue.new_command_buffer();
    let pass = Pass::new(command);
    let job = job.scatter(&pass, left, lkeys, right, rkeys)?;
    pass.end();
    if job.total > 0 {
        gpu += super::join::commit_wait(command)?;
    }
    Ok(Joined { pairs: job.pairs(), gpu_seconds: gpu, compared: job.compared })
}

/// A tile join encoded into a caller's passes (several in one command buffer each, decision 0100 §5): its count
/// pass first, then, once that command buffer completed and the pairs are counted, its scatter.
pub struct TileJob {
    tiles: usize,
    threads: usize,
    tiles_buf: Held,
    highs_buf: Held,
    unused: Held,
    _counts: Held,
    offsets: Held,
    /// The scan's count: read by the device, so held as long as the job (a dropped lease is the pool's to hand out).
    _n: Held,
    scatter_pipe: metal::ComputePipelineState,
    out: Option<Held>,
    /// What the pairs' sort reads and writes, until its command buffer completed.
    keep: Vec<Held>,
    /// The pairs (after the count's command buffer completed and `scatter`).
    pub total: usize,
    /// The pairs of rows the kernel compares: the brute force's work.
    pub compared: u64,
}

impl TileJob {
    /// The count pass of the join of `left` and `right` on their key columns, encoded into `pass`.
    pub fn count(pass: &Pass<'_>, left: &Resident, lkeys: &[usize], right: &Resident, rkeys: &[usize]) -> Result<TileJob, String> {
    shared::get().ok_or("no Metal device")?;
    if lkeys.is_empty() || lkeys.len() != rkeys.len() || lkeys.len() > 3 || lkeys[0] != 0 || rkeys[0] != 0 || left.forms[0] != Form::Low16 || right.forms[0] != Form::Low16 {
        return Err("a tile join takes one to three keys, each side's key 0 its column 0".into());
    }
    // The join's tiles: each left tile of a chunk the right also has, with the right's rows of it.
    let (lg, rg) = (&left.group_list, &right.group_list);
    let mut tiles: Vec<[u32; 4]> = Vec::new();
    let mut highs: Vec<i64> = Vec::new();
    let mut compared = 0u64;
    let mut r = 0;
    for &(high, start, count) in lg {
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
        _ => format!("{side}{g}[{row}]"),
    };
    let k = lkeys.len();
    let lcols: String = lkeys.iter().enumerate().map(|(i, &c)| format!(", device const {}* l{i} [[buffer({})]]", ty(left.forms[c]), 3 + i)).collect();
    let rcols: String = rkeys.iter().enumerate().map(|(i, &c)| format!(", device const {}* r{i} [[buffer({})]]", ty(right.forms[c]), 3 + k + i)).collect();
    let n = 3 + 2 * k;
    // Stage the right keys; compare. Each key 16 bits when both sides hold it in the chunk's terms
    // (`Low16`), else 64.
    let narrow: Vec<bool> = (0..k).map(|i| left.forms[lkeys[i]] == Form::Low16 && right.forms[rkeys[i]] == Form::Low16).collect();
    let (mut stage, mut decl, mut mine) = (String::new(), String::new(), String::new());
    let mut test = String::from("true");
    for i in 0..k {
        let (lf, rf) = (left.forms[lkeys[i]], right.forms[rkeys[i]]);
        let ty = if narrow[i] { "ushort" } else { "long" };
        decl.push_str(&format!("    threadgroup {ty} s{i}[256];\n"));
        let (r, l) = if narrow[i] { (format!("r{i}[rs + j]"), format!("l{i}[lrow]")) } else { (full('r', i, rf, "rs + j"), full('l', i, lf, "lrow")) };
        stage.push_str(&format!("        if (j < rc) {{ s{i}[t] = {r}; }}\n"));
        mine.push_str(&format!("    {ty} mine{i} = live ? {l} : 0;\n"));
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
        tgp = (0..k).map(|i| format!(", threadgroup {}* s{i}", if narrow[i] { "ushort" } else { "long" })).collect::<String>(),
        tga = (0..k).map(|i| format!(", s{i}")).collect::<String>(),
    );
    let variant: String = (0..k).map(|i| format!("{:?}{:?}{}", left.forms[lkeys[i]], right.forms[rkeys[i]], narrow[i])).collect();
    let pipe = |name: &str| shared::pipeline(&format!("tile {variant} {name}"), || compile(&src, name));
    let (count, prefix, scatter) = (pipe("tile_count")?, pipe("rows_scan")?, pipe("tile_scatter")?);
    let threads = tiles.len() * TILE;
    let tiles_buf = Held::of(tiles.iter().copied())?;
    let highs_buf = Held::of(highs.iter().copied())?;
    let unused = Held::new(4)?;
    let counts = Held::new(4 * threads)?;
    let offsets = Held::new(4 * (threads + 1))?;
    // SAFETY: the offsets' last word, the total, is read after completion; with no tiles nothing writes it.
    unsafe { *offsets.buffer.contents().cast::<u32>().add(threads) = 0 };
    let nbuf = Held::of([threads as u32].into_iter())?;
    let mut inputs: Vec<&metal::BufferRef> = vec![&tiles_buf.buffer, &highs_buf.buffer, &unused.buffer];
    inputs.extend(lkeys.iter().map(|&c| &*left.columns[c].buffer));
    inputs.extend(rkeys.iter().map(|&c| &*right.columns[c].buffer));
    if !tiles.is_empty() {
        let mut with = inputs;
        with.push(&counts.buffer);
        pass.run(&count, &with, &[], &[], tiles.len());
        pass.run(&prefix, &[&counts.buffer, &offsets.buffer, &nbuf.buffer], &[], &[], 1);
    }
    // Every buffer the passes read is the job's until its command buffers completed.
    Ok(TileJob { tiles: tiles.len(), threads, tiles_buf, highs_buf, unused, _counts: counts, offsets, _n: nbuf, scatter_pipe: scatter, out: None, keep: Vec::new(), total: 0, compared })
    }

    /// The scatter, encoded into `pass`, once the count's command buffer completed (the same sides and keys).
    pub fn scatter(mut self, pass: &Pass<'_>, left: &Resident, lkeys: &[usize], right: &Resident, rkeys: &[usize]) -> Result<TileJob, String> {
        // SAFETY: `threads + 1` offsets, written by the scan (or zeroed), read after completion.
        self.total = unsafe { *self.offsets.buffer.contents().cast::<u32>().add(self.threads) } as usize;
        let out = Held::new(8 * self.total)?;
        if self.total > 0 {
            let mut with: Vec<&metal::BufferRef> = vec![&self.tiles_buf.buffer, &self.highs_buf.buffer, &self.unused.buffer];
            with.extend(lkeys.iter().map(|&c| &*left.columns[c].buffer));
            with.extend(rkeys.iter().map(|&c| &*right.columns[c].buffer));
            with.extend([&*self.offsets.buffer, &*left.perm.buffer, &*right.perm.buffer, &*out.buffer]);
            pass.run(&self.scatter_pipe, &with, &[], &[], self.tiles);
        }
        self.out = Some(out);
        Ok(self)
    }

    /// The scattered pairs sorted into the kernel's order (left row, then right row) on the device, encoded into
    /// `pass` after `scatter`: `left_rows` and `right_rows` are the sides' host rows, which bound the sort's key.
    pub fn sort(mut self, pass: &Pass<'_>, left_rows: usize, right_rows: usize) -> Result<TileJob, String> {
        let bits = |n: usize| usize::BITS - n.saturating_sub(1).leading_zeros();
        if let Some(out) = self.out.take() {
            let sorted = Held::new(8 * self.total)?;
            super::join::encode_sorted_pairs(pass, &out, self.total, bits(left_rows), bits(right_rows), &sorted, &mut self.keep)?;
            self.keep.push(out);
            self.out = Some(sorted);
        }
        Ok(self)
    }

    /// The pairs, (left host row, right host row), in tile order (in row order after `sort`): after the scatter's
    /// command buffer completed.
    pub fn pairs(&self) -> Vec<(u32, u32)> {
        let Some(out) = &self.out else { return Vec::new() };
        // SAFETY: `2 * total` words, written by the scatter.
        let flat = unsafe { std::slice::from_raw_parts(out.buffer.contents().cast::<u32>(), 2 * self.total) };
        flat.chunks_exact(2).map(|p| (p[0], p[1])).collect()
    }
}

/// Compiles the tile pairs' sort ahead of its first use (the tile kernels themselves are made per key forms).
pub fn prewarm() {
    let _ = super::join::prewarm_pair_sort();
}
