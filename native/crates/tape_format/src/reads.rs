//! The tape's reads in Rust, written directly (decision 0534): what this crate's readers (`F16`, `F16Z`, `F15`,
//! `StoredPresence`) read a node with. The reads in CLIF (`rerail_clif::reads`, decision 0310) are the JIT's and
//! the kernels'; these read the same bytes the same way, and a test holds every read of every node equal to the
//! JIT's (`f16::tests::the_rust_reads_are_the_jits`).
//!
//! A read takes the bytes the view's offsets count from, the view, and a document-local node. Every load is a
//! slice's: a load past the bytes is a corrupt tape, and panics. Every value read is held where a reader's next
//! step needs it held (decision 0463): a size is at least 1, a span lies in the source, a parent's step moves back.

use rerail_clif::view::TapeView;

/// No node.
pub const NONE: u32 = u32::MAX;

#[cold]
#[inline(never)]
fn past() -> ! {
    panic!("a read past the tape's bytes: a corrupt tape")
}

/// The `N` bytes at `base + at`.
#[inline(always)]
fn bytes<const N: usize>(t: &[u8], base: u32, at: u32) -> [u8; N] {
    let from = base as usize + at as usize;
    match t.get(from..from + N) {
        Some(b) => b.try_into().unwrap_or_else(|_| past()),
        None => past(),
    }
}

#[inline(always)]
fn u8_at(t: &[u8], base: u32, at: u32) -> u32 {
    u32::from(bytes::<1>(t, base, at)[0])
}

#[inline(always)]
fn u16_at(t: &[u8], base: u32, at: u32) -> u32 {
    u32::from(u16::from_le_bytes(bytes(t, base, at)))
}

#[inline(always)]
fn u32_at(t: &[u8], base: u32, at: u32) -> u32 {
    u32::from_le_bytes(bytes(t, base, at))
}

/// Eight bytes, little-endian: format 16's arrays have 8 bytes of room past their last value.
#[inline(always)]
fn word_at(t: &[u8], base: u32, at: u32) -> u64 {
    u64::from_le_bytes(bytes(t, base, at))
}

/// The low `w` bytes' mask (all ones at 8 or more).
#[inline(always)]
fn mask(w: u32) -> u64 {
    if w >= 8 { u64::MAX } else { (1u64 << (w * 8)) - 1 }
}

/// The width at nibble `k` of the view's widths.
#[inline(always)]
fn width(v: &TapeView, k: u32) -> u32 {
    (v.widths >> (4 * k)) & 15
}

/// The reads of a format 16 tape.
pub mod f16 {
    use super::{NONE, TapeView, mask, u8_at, u16_at, u32_at, width, word_at};
    use rerail_clif::view::{F16_BLOCK as BLOCK, F16_K as K, F16_PART as PART, F16_SUB as SUB};

    /// Column `r` (0 codes, 1 slots, 2 gaps, 3 lengths, 4 checkpoints) of block `b`: blocks are `PART` nodes at the
    /// view's stride, each its columns end to end at its rows (the last block's are fewer), column `r` at `rows *
    /// columns[r]` (decision 0434).
    #[inline(always)]
    fn column(v: &TapeView, r: usize, b: u32) -> u32 {
        let base = b.wrapping_mul(v.stride).wrapping_add(v.columns[0]);
        if r == 0 {
            return base;
        }
        let rows = v.nodes.wrapping_sub(b * PART).min(PART);
        base.wrapping_add(rows.wrapping_mul(v.columns[r]))
    }

    /// Value `i` of array `a` (0 slots, 1 gaps, 2 lengths): one word's load, or (all ones) its side list's.
    #[inline(always)]
    fn get(t: &[u8], v: &TapeView, a: u32, i: u32) -> u64 {
        let w = width(v, a);
        let array = column(v, 1 + a as usize, i / PART);
        let m = mask(w);
        let value = word_at(t, array, (i % PART).wrapping_mul(w)) & m;
        if value == m { side(t, v, a as usize + 1, i) } else { value }
    }

    /// Node `i`'s value in side list `k` (0 codes' low parts, 1 slots, 2 gaps, 3 lengths), the node listed: its
    /// part, its block's run of offsets, a search of them for its low byte. Every offset is held to the part's end
    /// (decision 0463).
    #[cold]
    #[inline(never)]
    fn side(t: &[u8], v: &TapeView, k: usize, i: u32) -> u64 {
        let (list, parts) = (v.side[k], v.parts[k]);
        let p = i / PART;
        let start = u32_at(t, parts, p * 4);
        let limit = u32_at(t, parts, p * 4 + 4);
        let blocks = v.nodes.wrapping_add(BLOCK - 1) / BLOCK;
        let nsub = SUB.min(blocks.wrapping_sub(p * SUB));
        let j = (i / BLOCK) % SUB;
        let mut lo = if j == 0 { 0 } else { u16_at(t, list, start.wrapping_add((j - 1) * 2).min(limit)) };
        let mut hi = u16_at(t, list, start.wrapping_add(j * 2).min(limit));
        let offsets = start.wrapping_add(nsub.wrapping_mul(2));
        let low = i % BLOCK;
        while lo < hi {
            let middle = lo.wrapping_add(hi) >> 1;
            if u8_at(t, list, offsets.wrapping_add(middle).min(limit)) < low {
                lo = middle + 1;
            } else {
                hi = middle;
            }
        }
        // The list's values follow the part's offsets.
        let vw = (v.widths >> (16 + 4 * k)) & 15;
        let count = u16_at(t, list, start.wrapping_add(nsub.wrapping_sub(1).wrapping_mul(2)).min(limit));
        let at = offsets.wrapping_add(count).wrapping_add(lo.wrapping_mul(vw)).min(limit);
        word_at(t, list, at) & mask(vw)
    }

    /// Checkpoint `c`: the start of node `c * K`, in the checkpoints of its block.
    #[inline(always)]
    fn checkpoint(t: &[u8], v: &TapeView, c: u32) -> u64 {
        let cw = width(v, 3);
        let at = column(v, 4, c / (PART / K));
        word_at(t, at, (c % (PART / K)).wrapping_mul(cw)) & mask(cw)
    }

    /// The node's slot: an inner node's subtree size, shifted up by one; a leaf's step back to its previous sibling
    /// or (bit 1) its parent, shifted up by two, with bit 0 set.
    #[inline(always)]
    pub fn slot(t: &[u8], v: &TapeView, i: u32) -> u64 {
        get(t, v, 0, i)
    }

    /// The node's subtree's size: at least 1, whatever the slot holds, so every hop by sizes moves on (0463).
    #[inline(always)]
    pub fn size(t: &[u8], v: &TapeView, i: u32) -> u32 {
        let s = get(t, v, 0, i);
        if s & 1 != 0 { 1 } else { ((s >> 1) as u32).max(1) }
    }

    /// The node's code's number in its language's table: its byte, or past the cutoff, the byte's run and its low
    /// part from the side list.
    #[inline(always)]
    pub fn number(t: &[u8], v: &TapeView, i: u32) -> u32 {
        let byte = u8_at(t, column(v, 0, i / PART), i % PART);
        if byte < v.cut {
            return byte;
        }
        let low = side(t, v, 0, i) as u32;
        v.cut.wrapping_add((byte - v.cut).wrapping_mul(v.per)).wrapping_add(low)
    }

    /// The node's start as the tape states it: its checkpoint, then the gaps of the nodes after it, to its own.
    pub fn start_stated(t: &[u8], v: &TapeView, i: u32) -> u32 {
        let c = i / K;
        let from = checkpoint(t, v, c);
        let first = c * K;
        let r = i - first;
        if r == 0 {
            return from as u32;
        }
        // At a byte's width, the gaps of nodes `first + 1 ..= i` are bytes 1 to `r` of the word at the checkpoint's
        // node: summed by folding the word, unless one is escaped (all ones: its side list's).
        if width(v, 1) == 1 {
            let x = word_at(t, column(v, 2, first / PART), first % PART);
            let m = (u64::MAX >> ((K - 1 - r) * 8)) & !0xff;
            let x = x & m;
            if all_ones_bytes(x) & m == 0 {
                return from.wrapping_add(byte_sum(x)) as u32;
            }
        }
        // Else (a wider gap, or an escaped one) gap by gap: at most `K - 1` values.
        let mut sum = from;
        for j in first + 1..=i {
            sum = sum.wrapping_add(get(t, v, 1, j));
        }
        sum as u32
    }

    /// The high bit of each byte of `x` that is all ones.
    #[inline(always)]
    fn all_ones_bytes(x: u64) -> u64 {
        const LOW7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
        let n = !x;
        !(((n & LOW7).wrapping_add(LOW7)) | n) & 0x8080_8080_8080_8080
    }

    /// The sum of `x`'s eight bytes.
    #[inline(always)]
    fn byte_sum(x: u64) -> u64 {
        let x = (x & 0x00ff_00ff_00ff_00ff) + ((x >> 8) & 0x00ff_00ff_00ff_00ff);
        let x = (x & 0x0000_ffff_0000_ffff) + ((x >> 16) & 0x0000_ffff_0000_ffff);
        (x & 0xffff_ffff) + (x >> 32)
    }

    /// The node's start, held to the source (decision 0463).
    #[inline(always)]
    pub fn start(t: &[u8], v: &TapeView, i: u32) -> u32 {
        start_stated(t, v, i).min(v.source_len)
    }

    /// The node's end, held between its start and the source's end (decision 0463).
    #[inline(always)]
    pub fn end(t: &[u8], v: &TapeView, i: u32) -> u32 {
        let start = start(t, v, i);
        let end = start_stated(t, v, i).wrapping_add(length(t, v, i));
        end.max(start).min(v.source_len)
    }

    /// The node's length in bytes.
    #[inline(always)]
    pub fn length(t: &[u8], v: &TapeView, i: u32) -> u32 {
        get(t, v, 2, i) as u32
    }

    /// The node's start given the node before's (any value for the first node): a gap's add, or its checkpoint.
    #[inline(always)]
    pub fn start_next(t: &[u8], v: &TapeView, i: u32, prev: u32) -> u32 {
        if !i.is_multiple_of(K) { prev.wrapping_add(get(t, v, 1, i) as u32) } else { checkpoint(t, v, i / K) as u32 }
    }

    /// The node's parent, or `NONE` at the root: the node before is the parent (an inner node), or a leaf naming
    /// the previous sibling or, every `K`-th sibling, the parent. A step of 1 to `j` back, else the slot is no
    /// tape's and the walk ends (decision 0463).
    pub fn parent(t: &[u8], v: &TapeView, i: u32) -> u32 {
        let mut j = i;
        while j != 0 {
            let s = get(t, v, 0, j - 1);
            if s & 1 == 0 {
                return j - 1;
            }
            let step = (s >> 2) as u32;
            if step.wrapping_sub(1) >= j {
                return NONE;
            }
            j -= step;
            if s & 2 != 0 {
                return j;
            }
        }
        NONE
    }

    // ---- lines: the newline index (`crate::newlines`) ------------------------------------------------------------

    /// The index's first fields' bytes (`newlines::HEAD`), and every 256th zero and one of its high bits sampled.
    const HEAD: u32 = 20;
    const SAMPLE_SHIFT: u32 = 8;

    /// The index's u32 field `k` (0 the newlines' count, 1 the low parts' width, 2 and 3 the zero and one samples'
    /// counts, 4 the high bits' words).
    #[inline(always)]
    fn field(t: &[u8], v: &TapeView, k: u32) -> u32 {
        u32_at(t, v.lines, 4 * k)
    }

    /// Where the index's part `offset` bytes past its first fields lies.
    #[inline(always)]
    fn part(v: &TapeView, offset: u32) -> u32 {
        v.lines.wrapping_add(offset).wrapping_add(HEAD)
    }

    /// Newline `i`'s low part (`l` bits).
    fn low(t: &[u8], v: &TapeView, i: u32, l: u32) -> u64 {
        let before = field(t, v, 2).wrapping_add(field(t, v, 3)).wrapping_add(field(t, v, 4)).wrapping_mul(8);
        let bit = u64::from(i) * u64::from(l);
        let word = word_at(t, part(v, before), (bit >> 3) as u32) >> (bit & 7);
        word & 1u64.wrapping_shl(l).wrapping_sub(1)
    }

    /// The position of the `r`-th set bit of `x` (one is there).
    #[inline(always)]
    fn select_in_word(mut x: u64, mut r: u32) -> u32 {
        let mut pos = 0;
        for s in [32, 16, 8, 4, 2, 1] {
            let c = (x & ((1u64 << s) - 1)).count_ones();
            if r >= c {
                r -= c;
                x >>= s;
                pos += s;
            }
        }
        pos
    }

    /// The position in the index's high bits of its `j`-th one (`ones`) or zero: from the sample before it, a word
    /// at a time, within the index's words. Past them (a `j` the index does not hold: a line past the last), the
    /// position past its last word: unbounded, a word of none was a step of zero, and past the index lies the rest of
    /// the slab, zeroed (decision 0463).
    #[cold]
    #[inline(never)]
    fn select(t: &[u8], v: &TapeView, ones: bool, j: u32) -> u32 {
        let (zs, os) = (field(t, v, 2), field(t, v, 3));
        let skip = if ones { zs.wrapping_mul(8) } else { 0 };
        let sample = part(v, skip.wrapping_add((j >> SAMPLE_SHIFT) * 8));
        let (mut w, mut before) = (u32_at(t, sample, 0), u32_at(t, sample, 4));
        let high = part(v, zs.wrapping_add(os).wrapping_mul(8));
        let words = field(t, v, 4);
        loop {
            if w >= words {
                return words.wrapping_mul(64);
            }
            let word = word_at(t, high, w.wrapping_mul(8));
            let x = if ones { word } else { !word };
            let through = before.wrapping_add(x.count_ones());
            if through > j {
                return w.wrapping_mul(64).wrapping_add(select_in_word(x, j.wrapping_sub(before)));
            }
            w = w.wrapping_add(1);
            before = through;
        }
    }

    /// The line byte `x` lies in (from 0): the newlines before it, by the index (the zeros before and after its
    /// bucket, then a search of the bucket's low parts).
    pub fn line(t: &[u8], v: &TapeView, x: u32) -> u32 {
        if field(t, v, 0) == 0 {
            return 0;
        }
        let l = field(t, v, 1);
        let h = x.wrapping_shr(l);
        let xl = u64::from(x & 1u32.wrapping_shl(l).wrapping_sub(1));
        let mut lo = if h == 0 { 0 } else { select(t, v, false, h - 1).wrapping_sub(h - 1) };
        let mut hi = select(t, v, false, h).wrapping_sub(h);
        while lo < hi {
            let middle = lo.wrapping_add(hi) >> 1;
            if low(t, v, middle, l) < xl {
                lo = middle + 1;
            } else {
                hi = middle;
            }
        }
        lo
    }

    /// Line `k`'s first byte: 0 for the first; else one past newline `k - 1`, from its bucket (its one's position)
    /// and its low part.
    pub fn line_start(t: &[u8], v: &TapeView, k: u32) -> u32 {
        if k == 0 {
            return 0;
        }
        let i = k - 1;
        let bucket = select(t, v, true, i).wrapping_sub(i);
        let l = field(t, v, 1);
        (bucket.wrapping_shl(l) | low(t, v, i, l) as u32).wrapping_add(1)
    }

    pub use super::has_kind;
}

/// The reads of a format 15 tape.
pub mod f15 {
    use super::{NONE, TapeView, u16_at, u32_at, width};
    use rerail_clif::view::{F15_STRIDE_SHIFT, F15_TAIL_FROM_START};

    /// Row `i`'s cell of a column of width 2 or 4.
    #[inline(always)]
    fn cell(t: &[u8], column: u32, w: u32, i: u32) -> u32 {
        if w == 2 { u16_at(t, column, i.wrapping_mul(2)) } else { u32_at(t, column, i.wrapping_mul(4)) }
    }

    /// The node's subtree's size, at least 1 (decision 0463).
    #[inline(always)]
    pub fn size(t: &[u8], v: &TapeView, i: u32) -> u32 {
        cell(t, v.columns[1], width(v, 0), i).max(1)
    }

    /// The node's code's number in the tape's own table.
    #[inline(always)]
    pub fn number(t: &[u8], v: &TapeView, i: u32) -> u32 {
        u16_at(t, v.columns[0], i.wrapping_mul(2))
    }

    /// The node's start as the tape states it: its stride's checkpoint and its offset.
    #[inline(always)]
    pub fn start_stated(t: &[u8], v: &TapeView, i: u32) -> u32 {
        let checkpoint = u32_at(t, v.columns[2], (i >> F15_STRIDE_SHIFT) * 4);
        checkpoint.wrapping_add(cell(t, v.columns[3], width(v, 1), i))
    }

    /// The node's end as the tape states it: a tail from the node's own start, or from its last descendant's.
    #[inline(always)]
    pub fn end_stated(t: &[u8], v: &TapeView, i: u32) -> u32 {
        let from = if v.widths & F15_TAIL_FROM_START != 0 { i } else { i.wrapping_add(size(t, v, i)).wrapping_sub(1) };
        start_stated(t, v, from).wrapping_add(cell(t, v.columns[4], width(v, 2), i))
    }

    /// The node's start, held to the source (decision 0463).
    #[inline(always)]
    pub fn start(t: &[u8], v: &TapeView, i: u32) -> u32 {
        start_stated(t, v, i).min(v.source_len)
    }

    /// The node's end, held between its start and the source's end (decision 0463).
    #[inline(always)]
    pub fn end(t: &[u8], v: &TapeView, i: u32) -> u32 {
        end_stated(t, v, i).max(start(t, v, i)).min(v.source_len)
    }

    /// `c + size`, no further than the tape's nodes, and always past `c` however `size` wraps (decision 0463): from
    /// the view's last node (a short count, a node asked past it) a hop clamped to the nodes was a step of zero, and a
    /// walk stopped only past its end never stopped.
    #[inline(always)]
    fn hop(v: &TapeView, c: u32, size: u32) -> u32 {
        let past = c.wrapping_add(1);
        c.wrapping_add(size).max(past).min(v.nodes).max(past)
    }

    /// The node's parent, or `NONE` at the root: the nearest node before whose subtree holds the node, back over
    /// at most 64 nodes, else a descent from the root over the siblings' subtrees.
    pub fn parent(t: &[u8], v: &TapeView, i: u32) -> u32 {
        if i == 0 {
            return NONE;
        }
        let mut j = i;
        for _ in 0..64 {
            if j == 0 {
                break;
            }
            j -= 1;
            if hop(v, j, size(t, v, j)) > i {
                return j;
            }
        }
        let mut at = 0;
        loop {
            let mut c = at + 1;
            loop {
                let after = hop(v, c, size(t, v, c));
                if after > i {
                    break;
                }
                c = after;
            }
            if c == i {
                return at;
            }
            at = c;
        }
    }

    pub use super::has_kind;
}

/// Whether the tape's block `block` (its nodes `PART * block ..`; `NONE`: the whole document) has a node of kind
/// `kind`, by its presence (decision 0434 §4, step 6): 1 where the tape has none to tell by (format 15, or one
/// stored without it), and the document's answer where its blocks' presences are not held.
pub fn has_kind(t: &[u8], v: &TapeView, block: u32, kind: u32) -> u32 {
    let at = v.presence;
    if at == 0 {
        return 1;
    }
    let kinds = u32_at(t, at, 0);
    let word = kind >> 6;
    if word >= kinds {
        return 0;
    }
    let (fields, count) = (u32_at(t, at, 4), u32_at(t, at, 8));
    // The block's presence follows the document's; `NONE` + 1 is the document's (0).
    let next = block.wrapping_add(1);
    let index = if next < count { next } else { 0 };
    let k = index.wrapping_mul(kinds.wrapping_add(fields)).wrapping_add(word).wrapping_shl(3).wrapping_add(16);
    ((word_at(t, at, k) >> (kind & 63)) & 1) as u32
}

/// LZ4's block format, `t[from..to]`, decoded into `dst[..len]`: the bytes decoded, `len` for a whole block. The one
/// decoder (decision 0353 step 4b): the CLIF's `tape_lz4` (`z_lz4`, the JIT's and the kernels'), written directly, the
/// same steps and the same stops. A length's extension bytes are read one a step while the block has them; a
/// sequence whose literals pass `len` or `to`, whose match has no offset, reaches before `dst` or passes `len`, ends
/// the decode where the sequence began writing.
pub fn lz4(t: &[u8], from: usize, to: usize, dst: &mut [u8], len: usize) -> usize {
    let (to, len) = (to.min(t.len()), len.min(dst.len()));
    // A length of `nibble`: 15 is followed by bytes added to it, the last under 255.
    let length = |nibble: u8, s: &mut usize| -> usize {
        let mut n = u32::from(nibble);
        if n == 15 {
            while *s < to {
                let b = t[*s];
                *s += 1;
                n = n.wrapping_add(u32::from(b));
                if b != 255 {
                    break;
                }
            }
        }
        n as usize
    };
    let (mut s, mut d) = (from, 0);
    while s < to {
        let token = t[s];
        s += 1;
        let lit = length(token >> 4, &mut s);
        if d + lit > len || s + lit > to {
            return d;
        }
        dst[d..d + lit].copy_from_slice(&t[s..s + lit]);
        (s, d) = (s + lit, d + lit);
        // A match's two offset bytes, else the block's last literals ended it.
        if s + 2 > to {
            return d;
        }
        let off = usize::from(u16::from_le_bytes([t[s], t[s + 1]]));
        s += 2;
        let m = length(token & 15, &mut s) + 4;
        if off == 0 || off > d || d + m > len {
            return d;
        }
        if off >= m {
            dst.copy_within(d - off..d - off + m, d);
        } else {
            // A match that overlaps its own output repeats the `off` bytes before it.
            for k in d..d + m {
                dst[k] = dst[k - off];
            }
        }
        d += m;
    }
    d
}
