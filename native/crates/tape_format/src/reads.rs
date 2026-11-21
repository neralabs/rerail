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
    /// at a time.
    #[cold]
    #[inline(never)]
    fn select(t: &[u8], v: &TapeView, ones: bool, j: u32) -> u32 {
        let (zs, os) = (field(t, v, 2), field(t, v, 3));
        let skip = if ones { zs.wrapping_mul(8) } else { 0 };
        let sample = part(v, skip.wrapping_add((j >> SAMPLE_SHIFT) * 8));
        let (mut w, mut before) = (u32_at(t, sample, 0), u32_at(t, sample, 4));
        let high = part(v, zs.wrapping_add(os).wrapping_mul(8));
        loop {
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

    /// `c + size`, past `c` however `size` wraps, and no further than the tape's nodes (decision 0463).
    #[inline(always)]
    fn hop(v: &TapeView, c: u32, size: u32) -> u32 {
        c.wrapping_add(size).max(c.wrapping_add(1)).min(v.nodes)
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

/// The reads of a 16z tape through the rolling cache its reads share (decision 0353 step 4b): a node's block found
/// in the cache, or decoded into it as a format 16 tape of one block (`tape_z_block`, `z_decode`), then read by format
/// 16's reads in the entry (the CLIF's `in_block`). Written directly, the CLIF's steps over the same bytes, so a host
/// thread and a device share one cache; held to the JIT's by `f16::tests::the_rust_16z_reads_are_the_jits`.
///
/// The cache is named apart from the view (`Z::cache`): the view's own (`side[0..2]`, the cache less the tape) is
/// the device's where the device reads it, and the host's reads find the cache at its host address.
pub mod f16z {
    use super::{NONE, TapeView, f16, lz4, u8_at, u32_at, width};
    use rerail_clif::view::{F16_K as K, F16Z_BLOCK as Z_BLOCK, F16Z_PIECES as PIECES, cache, z};

    /// A 16z tape as its reads take it: the bytes its view's offsets count from, its view, and the cache its blocks
    /// are decoded into (a live `cache::Spec` of at least the view's entry size, `columns[3]`).
    #[derive(Clone, Copy)]
    pub struct Z<'a> {
        pub t: &'a [u8],
        pub v: &'a TapeView,
        pub cache: *const u8,
    }

    /// The block's key in the cache: the tape's id (`side[2..4]`) above the block (`tape_z_block`'s).
    #[inline(always)]
    fn key(v: &TapeView, block: u32) -> u64 {
        (u64::from(v.side[3]) << 32 | u64::from(v.side[2])) << 22 | u64::from(block)
    }

    impl Z<'_> {
        /// Block `block`'s entry in the cache, decoded into it on a miss; the dead entry (a failure counted) for a
        /// block past the tape's last, or where the cache keeps none (`tape_z_block`).
        pub fn block(&self, block: u32) -> *const u8 {
            let v = self.v;
            let blocks = v.nodes.wrapping_add(Z_BLOCK - 1) >> Z_BLOCK.trailing_zeros();
            if block >= blocks {
                // SAFETY: the reader's cache (`Z`'s).
                return unsafe { cache::failed(self.cache) };
            }
            let size = v.columns[3];
            // SAFETY: the reader's cache; the fill writes the entry's `size` bytes alone.
            unsafe { cache::get(self.cache, key(v, block), size, &|e| decode(self.t, v, block, e, size)) }
        }

        /// `body` over node `i`'s block's format 16 tape and view in the cache, and the node's place there: where
        /// threads share the cache (`columns[4]`), the entry checked complete and the block's before the read and
        /// unchanged after it, read again a bounded number of times, else the dead entry read (a failure counted).
        #[inline]
        pub fn read<R>(&self, i: u32, body: impl Fn(&[u8], &TapeView, u32) -> R) -> R {
            let block = i >> Z_BLOCK.trailing_zeros();
            let local = i & (Z_BLOCK - 1);
            let size = self.v.columns[3] as usize;
            let shared = self.v.columns[4] != 0;
            let of = |e: *const u8| {
                // SAFETY: an entry of the cache (or its dead entry), `size` bytes and the cache's slack: its tape after
                // its header and view, its view at `SLOT_VIEW`.
                unsafe { (std::slice::from_raw_parts(e.add(z::SLOT_BYTES as usize), size - z::SLOT_BYTES as usize), &*e.add(z::SLOT_VIEW as usize).cast::<TapeView>()) }
            };
            let mut tries = 0;
            loop {
                let e = self.block(block);
                // SAFETY: an entry of the reader's cache.
                let before = unsafe { cache::ready(e) };
                let (e, recheck) = if !shared {
                    (e, false)
                } else {
                    std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
                    // SAFETY: as `before`.
                    let ours = unsafe { cache::key(e) } == key(self.v, block);
                    if ours && before & cache::WRITING == 0 && before != 0 {
                        (e, true)
                    } else if tries >= cache::TRIES {
                        // SAFETY: the reader's cache.
                        (unsafe { cache::failed(self.cache) }.cast_const(), false)
                    } else {
                        tries += 1;
                        continue;
                    }
                };
                let (t, v) = of(e);
                let value = body(t, v, local);
                if !recheck {
                    return value;
                }
                std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
                // SAFETY: as `before`.
                if unsafe { cache::ready(e) } == before {
                    return value;
                }
                if tries >= cache::TRIES {
                    // SAFETY: the reader's cache.
                    let (t, v) = of(unsafe { cache::failed(self.cache) });
                    return body(t, v, local);
                }
                tries += 1;
            }
        }

        pub fn slot(&self, i: u32) -> u64 {
            self.read(i, |t, v, j| f16::slot(t, v, j))
        }
        pub fn size(&self, i: u32) -> u32 {
            self.read(i, |t, v, j| f16::size(t, v, j))
        }
        pub fn number(&self, i: u32) -> u32 {
            self.read(i, |t, v, j| f16::number(t, v, j))
        }
        pub fn start(&self, i: u32) -> u32 {
            self.read(i, |t, v, j| f16::start(t, v, j))
        }
        pub fn end(&self, i: u32) -> u32 {
            self.read(i, |t, v, j| f16::end(t, v, j))
        }
        /// As format 16's (`parent16` with `tape_slot`), each slot read in its block: the walk steps back over
        /// blocks' starts.
        pub fn parent(&self, i: u32) -> u32 {
            let mut j = i;
            while j != 0 {
                let s = self.slot(j - 1);
                if s & 1 == 0 {
                    return j - 1;
                }
                let step = (s >> 2) as u32;
                if step.wrapping_sub(1) >= j {
                    return NONE;
                }
                let back = j - step;
                if s & 2 != 0 {
                    return back;
                }
                j = back;
            }
            NONE
        }
    }

    /// Block `block` of the 16z tape `t` (its view `v`) decoded into the cache entry `e` of `size` bytes, as a format
    /// 16 tape of one block (`z_decode`): each piece decoded where the block's view places it (its column, then its
    /// side list's part), its checkpoints copied, each side list's one part's starts after them, and its view.
    /// Whether it fits the entry, else nothing written.
    fn decode(t: &[u8], v: &TapeView, block: u32, e: *mut u8, size: u32) -> bool {
        let nodes = v.nodes;
        let nb = nodes.wrapping_sub(block << Z_BLOCK.trailing_zeros()).min(Z_BLOCK);
        let blocks = nodes.wrapping_add(Z_BLOCK - 1) >> Z_BLOCK.trailing_zeros();
        // Each side list's part of the block: from its start to the next's (list `q`'s starts at `4 * q * (blocks +
        // 1)`), held to its list's order.
        let parts = v.columns[1];
        let count = blocks.wrapping_add(1);
        let side: [u32; 4] = std::array::from_fn(|q| {
            let at = (count.wrapping_mul(q as u32).wrapping_add(block)).wrapping_mul(4);
            let from = u32_at(t, parts, at);
            u32_at(t, parts, at.wrapping_add(4)).max(from).wrapping_sub(from)
        });
        // The columns' places: column `q` holds `nb * w[q]` bytes, its side part after it, then 16 bytes of room.
        let w = [1, width(v, 0), width(v, 1), width(v, 2)];
        let cw = width(v, 3);
        let mut c = [0u32; 5];
        let mut ends = [0u32; 4];
        for q in 0..4 {
            let end = nb.wrapping_mul(c[q]).wrapping_add(nb.wrapping_mul(w[q]));
            ends[q] = end;
            let end = end.wrapping_add(side[q]).wrapping_add(16);
            c[q + 1] = end.wrapping_add(nb - 1) / nb;
        }
        let ckpt_at = nb.wrapping_mul(c[4]);
        let ckpt_len = (nb.wrapping_add(K - 1) >> K.trailing_zeros()).wrapping_mul(cw);
        let parts_at = ckpt_at.wrapping_add(ckpt_len).wrapping_add(16 + 7) & !7;
        let end = parts_at.wrapping_add(32 + 8);
        if u64::from(end) + z::SLOT_BYTES as u64 + z::SLACK as u64 > u64::from(size) {
            return false;
        }
        // SAFETY: the entry taken, this fill's alone: `size` bytes.
        let entry = unsafe { std::slice::from_raw_parts_mut(e, size as usize) };
        let (head, dst) = entry.split_at_mut(z::SLOT_BYTES as usize);
        // The block's view: its nodes, format 16's, its columns' places, its side parts, the tape's widths and choices.
        let view = &mut head[z::SLOT_VIEW as usize..];
        let put = |view: &mut [u8], at: usize, value: u32| view[at..at + 4].copy_from_slice(&value.to_le_bytes());
        use std::mem::offset_of;
        for (value, at) in [
            (nb, offset_of!(TapeView, nodes)),
            (16, offset_of!(TapeView, format)),
            (v.widths, offset_of!(TapeView, widths)),
            (v.cut, offset_of!(TapeView, cut)),
            (v.per, offset_of!(TapeView, per)),
            (v.source_len, offset_of!(TapeView, source_len)),
            (0, offset_of!(TapeView, stride)),
        ] {
            put(view, at, value);
        }
        for (k, &value) in c.iter().enumerate() {
            put(view, offset_of!(TapeView, columns) + 4 * k, value);
        }
        for q in 0..4 {
            put(view, offset_of!(TapeView, side) + 4 * q, ends[q]);
            put(view, offset_of!(TapeView, parts) + 4 * q, parts_at.wrapping_add(8 * q as u32));
            let p = parts_at as usize + 8 * q;
            dst[p..p + 4].copy_from_slice(&0u32.to_le_bytes());
            dst[p + 4..p + 8].copy_from_slice(&side[q].to_le_bytes());
        }
        // Its checkpoints, raw where the tape stores them (`Z / K` a block, `cw` bytes each).
        let from = v.columns[2] as usize + (block.wrapping_mul(Z_BLOCK / K).wrapping_mul(cw)) as usize;
        let (at, len) = (ckpt_at as usize, ckpt_len as usize);
        dst[at..at + len].copy_from_slice(&t[from..from + len]);
        // Its pieces, one a turn: the piece index's start, length and kind (a repeat's, the piece it names).
        let pieces = blocks.wrapping_mul(PIECES);
        let starts = v.columns[0];
        let lens = starts.wrapping_add(pieces.wrapping_mul(4));
        let kinds = lens.wrapping_add(pieces.wrapping_mul(4));
        let payload = v.stride;
        for q in 0..PIECES as usize {
            let j = block.wrapping_mul(PIECES).wrapping_add(q as u32);
            let kind = u8_at(t, kinds, j) as u8;
            // A repeat names a piece of the tape that is none (checked where the tape is read in): else itself.
            let k = if kind == z::REPEAT {
                let named = u32_at(t, payload, u32_at(t, starts, j.wrapping_mul(4)));
                if named < pieces { named } else { j }
            } else {
                j
            };
            let kind = u8_at(t, kinds, k) as u8;
            let start = u32_at(t, starts, k.wrapping_mul(4));
            let len = u32_at(t, lens, k.wrapping_mul(4));
            let src = payload.wrapping_add(start) as usize;
            // Where it decodes: its column (`nb * C[q]`) to its side part's end.
            let (to, to_end) = (nb.wrapping_mul(c[q]) as usize, ends[q].wrapping_add(side[q]) as usize);
            let want = to_end.saturating_sub(to);
            if kind == z::LZ4 {
                lz4(t, src, src + len as usize, &mut dst[to..], want);
            } else {
                let n = (len as usize).min(want);
                dst[to..to + n].copy_from_slice(&t[src..src + n]);
            }
        }
        true
    }
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
