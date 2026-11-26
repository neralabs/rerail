//! The tile join on the device (decision 0095 §4): two relations grouped by their key 0's chunk, compared by brute
//! force within a chunk. The GPU lane's `join` kernel takes it for joins grouped by document (decision 0100 §1);
//! the engine's row programs read the same layout (`Resident`).
//!
//! **The layout.** The rows are grouped by their lead column's chunk, its high part (`v >> 16`): a group carries its
//! chunk once, and each row its own 16 low bits. Every other column is kept as narrow as its values let it, exactly:
//! `Low16` (every value in its row's chunk), `I32`, or `I64`. A group is cut into tiles of 256 rows.

use super::join::{Held, Pass, Pipe, Scan, pipeline};
use rerail_device::device::Buffer;
use std::sync::Arc;

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
    pub group_list: rerail_blocks::vector::Vector<rerail_accel::rows::Run>,
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
fn packed<T: Copy>(values: impl ExactSizeIterator<Item = T>) -> Result<Held, String> {
    Held::of(values)
}

impl Resident {
    /// `columns` (each `rows` long) on the device, grouped by column 0's chunk. The host's part: a radix sort of
    /// the lead's chunks (`rows_sort`), then per column one gather in that order, its form from the gathered
    /// values (each in its row's chunk; its span), and one pass writing it into its buffer.
    pub fn new(columns: &[&[i64]], rows: usize) -> Result<Self, String> {
        super::join::device()?;
        let lead = columns.first().ok_or("a relation of no columns")?;
        let chunks: rerail_blocks::vector::Vector<i64> = lead[..rows].iter().map(|&v| v >> 16).collect();
        let sorted = rerail_accel::rows::sort(&chunks);
        let group_list = rerail_accel::rows::runs(&sorted.keys);
        // Tiles: a group's rows 256 at a time.
        let (mut highs, mut tiles) = (rerail_blocks::vector::Vector::with_capacity(group_list.len()), rerail_blocks::vector::Vector::<[u32; 4]>::new());
        for (g, &rerail_accel::rows::Run { key: high, start, count }) in group_list.iter().enumerate() {
            highs.push(high);
            for s in (start..start + count).step_by(TILE) {
                tiles.push([g as u32, s, (start + count - s).min(TILE as u32), 0]);
            }
        }
        let perm = sorted.perm;
        let mut forms = Vec::with_capacity(columns.len());
        let mut buffers = Vec::with_capacity(columns.len());
        let mut bytes = 0;
        let mut v: rerail_blocks::vector::Vector<i64> = rerail_blocks::vector::Vector::with_capacity(rows);
        for c in columns {
            v.clear();
            v.extend(perm.iter().map(|&r| c[r as usize]));
            let low16 = v.iter().zip(&sorted.keys).all(|(&x, &h)| x >> 16 == h);
            let (min, max) = rerail_accel::rows::span(&v).unwrap_or((0, 0));
            let (form, buffer) = if low16 {
                bytes += 2 * rows;
                (Form::Low16, packed(v.iter().map(|&x| x as u16))?)
            } else if min >= i64::from(i32::MIN) && max <= i64::from(i32::MAX) {
                bytes += 4 * rows;
                (Form::I32, packed(v.iter().map(|&x| x as i32))?)
            } else {
                bytes += 8 * rows;
                (Form::I64, packed(v.iter().copied())?)
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
            tiles: packed(tiles.into_iter())?,
            tile_count,
            highs: packed(highs.into_iter())?,
            columns: buffers,
            perm: packed(perm.into_iter())?,
        })
    }
}

/// A join's pairs (`tile_join`): each pair's host rows, left and right, and the GPU time.
pub struct Joined {
    pub pairs: rerail_blocks::vector::Vector<[u32; 2]>,
    pub gpu_seconds: f64,
    /// The pairs of rows the kernel compared: the brute force's work.
    pub compared: u64,
}

/// The equi-join of two resident relations on key columns, by brute force within a document (decision 0095 §4,
/// `tile`): both relations grouped by their key 0 (each one's column 0, a node column in the 16-bit layout on both
/// sides), so a pair whose key 0 is equal is in one group, a document's rows. A threadgroup takes 256 left rows of
/// a group; the group's right rows come through threadgroup memory 256 at a time (each key 16 bits when both sides
/// hold it as `Low16`, else 64), and each thread compares its row with each. Two passes, count and scatter, with a
/// scan of the threads' counts between: the pairs in order, sized exactly. For a plain equi-join the sort and probe
/// (`metal_sort`) does only the pairs' work; the tile is for pairs a key cannot find (containment, order).
pub fn tile_join(left: &Resident, lkeys: &[usize], right: &Resident, rkeys: &[usize]) -> Result<Joined, String> {
    // An evaluation of the launch budget (`super::launch`), or the caller's when it is in one: both passes.
    let _evaluation = super::launch::LaunchScope::enter();
    let pass = Pass::new()?;
    let job = TileJob::count(&pass, left, lkeys, right, rkeys)?;
    let mut gpu = pass.commit_wait()?;
    let pass = Pass::new()?;
    let job = job.scatter(&pass, left, lkeys, right, rkeys)?;
    if job.total > 0 {
        gpu += pass.commit_wait()?;
    }
    Ok(Joined { pairs: job.pairs(), gpu_seconds: gpu, compared: job.compared })
}

/// A tile join encoded into a caller's passes (several in one submission each, decision 0100 §5): its count
/// pass first, then, once that submission completed and the pairs are counted, its scatter.
pub struct TileJob {
    tiles: usize,
    threads: usize,
    tiles_buf: Held,
    highs_buf: Held,
    _counts: Held,
    offsets: Held,
    scatter_pipe: Arc<Pipe>,
    out: Option<Held>,
    /// What the pairs' sort reads and writes, until its submission completed.
    keep: Vec<Held>,
    /// The pairs (after the count's submission completed and `scatter`).
    pub total: usize,
    /// The pairs of rows the kernel compares: the brute force's work.
    pub compared: u64,
}

impl TileJob {
    /// The count pass of the join of `left` and `right` on their key columns, encoded into `pass`.
    pub fn count(pass: &Pass, left: &Resident, lkeys: &[usize], right: &Resident, rkeys: &[usize]) -> Result<TileJob, String> {
    super::join::device()?;
    if lkeys.is_empty() || lkeys.len() != rkeys.len() || lkeys.len() > 3 || lkeys[0] != 0 || rkeys[0] != 0 || left.forms[0] != Form::Low16 || right.forms[0] != Form::Low16 {
        return Err("a tile join takes one to three keys, each side's key 0 its column 0".into());
    }
    // The join's tiles: each left tile of a chunk the right also has, with the right's rows of it.
    let (lg, rg) = (&left.group_list, &right.group_list);
    let mut tiles: rerail_blocks::vector::Vector<[u32; 4]> = rerail_blocks::vector::Vector::new();
    let mut highs: rerail_blocks::vector::Vector<i64> = rerail_blocks::vector::Vector::new();
    let mut compared = 0u64;
    let mut r = 0;
    for &rerail_accel::rows::Run { key: high, start, count } in lg {
        while r < rg.len() && rg[r].key < high {
            r += 1;
        }
        if r < rg.len() && rg[r].key == high {
            compared += u64::from(count) * u64::from(rg[r].count);
            for s in (start..start + count).step_by(TILE) {
                tiles.push([s, (start + count - s).min(TILE as u32), rg[r].start, rg[r].count]);
                highs.push(high);
            }
        }
    }
    let ty = |f: Form| match f {
        Form::Low16 => "uint16_t",
        Form::I32 => "int32_t",
        Form::I64 => "int64_t",
    };
    let full = |side: char, g: usize, f: Form, row: &str| match f {
        Form::Low16 => format!("((high << 16) | (int64_t){side}{g}[{row}])"),
        Form::I32 => format!("(int64_t){side}{g}[{row}]"),
        _ => format!("{side}{g}[{row}]"),
    };
    let k = lkeys.len();
    let rows: String = lkeys
        .iter()
        .enumerate()
        .map(|(i, &c)| format!("               ROBUFFER, {}, l{i}, \"[left rows]\",\n", ty(left.forms[c])))
        .chain(rkeys.iter().enumerate().map(|(i, &c)| format!("               ROBUFFER, {}, r{i}, \"[right rows]\",\n", ty(right.forms[c]))))
        .collect();
    // Stage the right keys; compare. Each key 16 bits when both sides hold it in the chunk's terms
    // (`Low16`), else 64.
    let narrow: Vec<bool> = (0..k).map(|i| left.forms[lkeys[i]] == Form::Low16 && right.forms[rkeys[i]] == Form::Low16).collect();
    let (mut stage, mut shared_rows, mut mine) = (String::new(), String::new(), String::new());
    let mut test = String::from("true");
    for i in 0..k {
        let (lf, rf) = (left.forms[lkeys[i]], right.forms[rkeys[i]]);
        let ty = if narrow[i] { "uint16_t" } else { "int64_t" };
        shared_rows.push_str(&format!("               SHARED, {ty}, s{i}, 256,\n"));
        let (r, l) = if narrow[i] { (format!("r{i}[rs + j]"), format!("l{i}[lrow]")) } else { (full('r', i, rf, "rs + j"), full('l', i, lf, "lrow")) };
        stage.push_str(&format!("    if (j < rc) {{ s{i}[t] = {r}; }}\n"));
        mine.push_str(&format!("  {ty} mine{i} = live ? ({ty}){l} : ({ty})0;\n"));
        test.push_str(&format!(" && s{i}[q] == mine{i}"));
    }
    let lp: String = (0..k)
        .map(|i| format!(", UKL_GLOBAL const {}* l{i}", ty(left.forms[lkeys[i]])))
        .chain((0..k).map(|i| format!(", UKL_GLOBAL const {}* r{i}", ty(right.forms[rkeys[i]]))))
        .collect();
    let la: String = (0..k).map(|i| format!(", l{i}")).chain((0..k).map(|i| format!(", r{i}"))).collect();
    let tgp: String = (0..k).map(|i| format!(", UKL_LOCAL {}* s{i}", if narrow[i] { "uint16_t" } else { "int64_t" })).collect();
    let tga: String = (0..k).map(|i| format!(", s{i}")).collect();
    let common = super::join::COMMON;
    let src = format!(
        r#"{common}
// A tile join's tile: its left rows (first, count) and its group's right rows.
typedef struct JoinTile {{ uint32_t ls; uint32_t lc; uint32_t rs; uint32_t rc; }} JoinTile;

// A thread's left row against its group's right rows, staged 256 at a time: counted, or written at its offset.
UKL_INLINE void tile_pairs(uint32_t tg, uint32_t t, UKL_GLOBAL const JoinTile* tiles, UKL_GLOBAL const int64_t* highs{lp},
                           UKL_GLOBAL const uint32_t* offsets, UKL_GLOBAL const uint32_t* lperm, UKL_GLOBAL const uint32_t* rperm,
                           UKL_GLOBAL uint32_t* out, UKL_GLOBAL uint32_t* counts, bool write{tgp})
{{
  JoinTile tile = tiles[tg];
  int64_t high = highs[tg];
  bool live = t < tile.lc;
  uint32_t lrow = tile.ls + ukl_min(t, tile.lc - 1u);
{mine}  uint32_t n = 0u;
  uint32_t at = write ? offsets[tg * 256u + t] : 0u;
  for (uint32_t base = 0u; base < tile.rc; base += 256u) {{
    uint32_t j = base + t;
    uint32_t rc = tile.rc;
    uint32_t rs = tile.rs;
{stage}    ukl_tg_barrier();
    uint32_t m = ukl_min(256u, rc - base);
    for (uint32_t q = 0u; q < m; q++) {{
      if (live && {test}) {{
        if (write) {{ out[2u * (at + n)] = lperm[lrow]; out[2u * (at + n) + 1u] = rperm[rs + base + q]; }}
        n++;
      }}
    }}
    ukl_tg_barrier();
  }}
  if (!write) counts[tg * 256u + t] = n;
}}

DEFINE_KERNEL2(join, tile_count,
               ROBUFFER, JoinTile, tiles, "[groups]",
               ROBUFFER, int64_t, highs, "[groups]",
{rows}               RWBUFFER, uint32_t, counts, "[256 * groups]",
{shared_rows}               TGID0, uint32_t, tg, ,
               LID0, uint32_t, t, )
{{
  tile_pairs(tg, t, tiles, highs{la}, counts, counts, counts, counts, counts, false{tga});
}}

DEFINE_KERNEL2(join, tile_scatter,
               ROBUFFER, JoinTile, tiles, "[groups]",
               ROBUFFER, int64_t, highs, "[groups]",
{rows}               ROBUFFER, uint32_t, offsets, "[256 * groups + 1]",
               ROBUFFER, uint32_t, lperm, "[left rows]",
               ROBUFFER, uint32_t, rperm, "[right rows]",
               RWBUFFER, uint32_t, out, "[2 * pairs]",
{shared_rows}               TGID0, uint32_t, tg, ,
               LID0, uint32_t, t, )
{{
  tile_pairs(tg, t, tiles, highs{la}, offsets, lperm, rperm, out, out, true{tga});
}}

"#
    );
    let variant: String = (0..k).map(|i| format!("{:?}{:?}{}", left.forms[lkeys[i]], right.forms[rkeys[i]], narrow[i])).collect();
    let pipe = |name: &str| pipeline(&format!("tile {variant}"), name, || src.clone());
    let (count, scatter) = (pipe("tile_count")?, pipe("tile_scatter")?);
    let threads = tiles.len() * TILE;
    let tiles_buf = Held::of(tiles.iter().copied())?;
    let highs_buf = Held::of(highs.iter().copied())?;
    let counts = Held::new(4 * threads)?;
    let offsets = Held::new(4 * (threads + 1))?;
    // The counts' scan, the library's: its levels the job's until the count's submission completed.
    let scan = Scan::owned(threads)?;
    // SAFETY: the offsets' last word, the total, is read after completion; with no tiles nothing writes it.
    unsafe { *offsets.buffer.contents().cast::<u32>().add(threads) = 0 };
    let mut inputs: Vec<&Buffer> = vec![&tiles_buf.buffer, &highs_buf.buffer];
    inputs.extend(lkeys.iter().map(|&c| &left.columns[c].buffer));
    inputs.extend(rkeys.iter().map(|&c| &right.columns[c].buffer));
    if !tiles.is_empty() {
        let mut with = inputs;
        with.push(&counts.buffer);
        pass.run(&count, &with, &[], &[], tiles.len());
        scan.encode_into(pass, &counts.buffer, &offsets.buffer)?;
    }
    // Every buffer the passes read is the job's until its submissions completed.
    Ok(TileJob { tiles: tiles.len(), threads, tiles_buf, highs_buf, _counts: counts, offsets, scatter_pipe: scatter, out: None, keep: scan.into_held(), total: 0, compared })
    }

    /// The scatter, encoded into `pass`, once the count's submission completed (the same sides and keys).
    pub fn scatter(mut self, pass: &Pass, left: &Resident, lkeys: &[usize], right: &Resident, rkeys: &[usize]) -> Result<TileJob, String> {
        // SAFETY: `threads + 1` offsets, written by the scan (or zeroed), read after completion.
        self.total = unsafe { *self.offsets.buffer.contents().cast::<u32>().add(self.threads) } as usize;
        let out = Held::new(8 * self.total)?;
        if self.total > 0 {
            let mut with: Vec<&Buffer> = vec![&self.tiles_buf.buffer, &self.highs_buf.buffer];
            with.extend(lkeys.iter().map(|&c| &left.columns[c].buffer));
            with.extend(rkeys.iter().map(|&c| &right.columns[c].buffer));
            with.extend([&self.offsets.buffer, &left.perm.buffer, &right.perm.buffer, &out.buffer]);
            pass.run(&self.scatter_pipe, &with, &[], &[], self.tiles);
        }
        self.out = Some(out);
        Ok(self)
    }

    /// The scattered pairs sorted into the kernel's order (left row, then right row) on the device, encoded into
    /// `pass` after `scatter`: `left_rows` and `right_rows` are the sides' host rows, which bound the sort's key.
    pub fn sort(mut self, pass: &Pass, left_rows: usize, right_rows: usize) -> Result<TileJob, String> {
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
    /// submission completed.
    pub fn pairs(&self) -> rerail_blocks::vector::Vector<[u32; 2]> {
        let Some(out) = &self.out else { return rerail_blocks::vector::Vector::new() };
        // SAFETY: `total` pairs, written by the scatter.
        let flat = unsafe { std::slice::from_raw_parts(out.buffer.contents().cast::<[u32; 2]>(), self.total) };
        flat.iter().copied().collect()
    }
}

