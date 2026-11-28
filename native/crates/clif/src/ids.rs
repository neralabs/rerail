//! A projection's numbering as its reads are told where it lies (decision 0353 step 2): the rank index of the tape
//! nodes it numbers, and its granule table. Each column is a run in the block the descriptor is in, `off` bytes from
//! the run's own address (`rerail_blocks::refs::Span`'s layout). One definition: the host's `Ranks` and
//! `granules::Table` fill it, its reads read it (`Numbering::is`, …, on the host; `id_reads` in CLIF for the JIT and
//! the kernels), and its MSL is written from it.
//!
//! The rank index is groups of 64 tape nodes, `WORDS` u64s each: one word of bits per item (a numbered node, a token,
//! a node under a field, a node after a sibling), then the nodes before the group (low 32 bits) and the tokens (high
//! 32 bits), then the fielded nodes (low) and the nodes after a sibling (high) before it. A select sample gives the group of every `SAMPLE`-th item.
//!
//! The granule table is per granule (in preorder of their starts): its start and its subtree's end as ranks of each
//! item (`rank_starts`, `rank_ends`; nodes, then tokens), its parent, its own nodes and tokens as prefix sums, its
//! slots, its child granules (`kids`: the granule, and its parent's own nodes and tokens before it) and the run rank
//! of each stretch between them (`runs`), and the slots and runs by key (`by_slot`, `run_at`).

/// The items a rank index marks: numbered nodes, tokens, nodes under a field, nodes after a numbered sibling, spread
/// nodes, the token order's (the lexical tokens, the places with a token, one before, one after), `op`'s (a
/// numbered node's anonymous children), the nodes with a numbered tape parent (and the root: `structural_before`'s,
/// `rr_token_named`'s), an anchored projection's anchors, and the nodes with text (`Item`'s numbers).
pub const ITEMS: u32 = 13;
/// A group's words: one of bits an item, then the counts before it, two to a word.
pub const WORDS: u32 = ITEMS + ITEMS.div_ceil(2);
/// Items a select sample stands for.
pub const SAMPLE: u32 = 64;
/// An id's low 32 bits: its granule's slot, then its index in it (`INDEX_BITS`).
pub const SLOT_BITS: u32 = 14;
pub const INDEX_BITS: u32 = 32 - SLOT_BITS;
/// The top slots, from the last down, that hold a document's spans past its nodes and tokens (`span_low`).
pub const SPAN_SLOTS: u32 = 16;
/// An order key's low bits: its index in its granule; above them, its run's rank.
pub const ORDER_INDEX_BITS: u32 = 18;
/// The items a granule jump stands for (`Numbering::jumps`).
pub const JUMP: u32 = 64;

/// `len` items, `off` bytes from the run's own address (`rerail_blocks::refs::Span`'s layout).
#[repr(C)]
#[derive(Debug, Default)]
pub struct Run {
    pub off: i64,
    pub len: u64,
}

impl Run {
    /// Points the run at `len` items at `target`.
    ///
    /// # Safety
    /// `this` is the run's address, and the items stay where they are as long as the run is read.
    pub unsafe fn set<T>(this: *mut Run, target: *const T, len: usize) {
        unsafe {
            (*this).off = target as i64 - this as i64;
            (*this).len = len as u64;
        }
    }

    /// The items, as a slice.
    ///
    /// # Safety
    /// The run was set (`set`) to `T`s that are still there.
    pub unsafe fn as_slice<T>(&self) -> &[T] {
        if self.len == 0 {
            return &[];
        }
        unsafe { std::slice::from_raw_parts((self as *const Run as *const u8).offset(self.off as isize) as *const T, self.len as usize) }
    }
}

/// A projection's numbering (`Numbering` in the kernels).
#[repr(C)]
#[derive(Debug, Default)]
pub struct Numbering {
    /// The first group's number (its first tape node over 64).
    pub first: u32,
    /// The granules (0: no table; ids are in preorder).
    pub granules: u32,
    /// Each item's count.
    pub count: [u32; ITEMS as usize],
    /// The order keys are preorder indexes: the runs or a granule's nodes outgrow the key's bits.
    pub preorder_keys: u32,
    /// The rank index: `[u64; WORDS]` a group, and each item's samples (u32 groups).
    pub groups: Run,
    pub samples: [Run; ITEMS as usize],
    /// Per granule (u32s): its start and end on the tape, its parent (`u32::MAX` for the root's).
    pub starts: Run,
    pub ends: Run,
    pub parents: Run,
    /// Own nodes and tokens of granules `..g`: prefix sums (u32s), one longer than the granules.
    pub nodes: Run,
    pub tokens: Run,
    /// Granule `g`'s slots are `slots[at[g]..at[g + 1]]` (u32s).
    pub at: Run,
    pub slots: Run,
    /// `[slot, granule, continuation]`, by slot.
    pub by_slot: Run,
    /// Granule `g`'s child granules are `kids[first_kid[g]..first_kid[g + 1]]`: `[granule, nodes before, tokens
    /// before]`.
    pub first_kid: Run,
    pub kids: Run,
    /// Granule `g`'s stretch `s` has run rank `runs[first_kid[g] + g + s]`; each run's `[granule, stretch]` by rank.
    pub runs: Run,
    pub run_at: Run,
    /// Each granule's start and end as ranks: `[nodes, tokens]`.
    pub rank_starts: [Run; 2],
    pub rank_ends: [Run; 2],
    /// By item, the last granule starting at or before item number `JUMP * b`, for each `b` (u32s): a number's
    /// granule is searched between two of them.
    pub jumps: [Run; 2],
}
const _: () = assert!(std::mem::size_of::<Numbering>() == 64 + 16 * 32);

/// No number, no node, no granule: a read's answer for none (the CLIF's `NONE`).
pub const NONE: u32 = u32::MAX;

/// The first of `by_slot`'s entries at or past `slot`, found among the `W` entries about `guess` (the entries below the
/// slot there counted, no branch on them; the window's edge entries say whether it lies inside, else the table is
/// bisected): a measurement's, against `slot_entry_stepped`. A window of 16 or 32 took 2.5 ns a low where the steps took
/// 2.25 (`what_numbering_a_documents_lows_costs_one_by_one_and_sorted`), and in idc's trace `local_of` kept its share.
#[cfg(feature = "test-hooks")]
fn slot_entry<const W: usize>(by_slot: &[[u32; 3]], slot: u32, guess: u32) -> u32 {
    let n = by_slot.len() as u32;
    let lo = guess.saturating_sub(W as u32 / 2).min(n.saturating_sub(W as u32));
    if let Some(w) = by_slot[lo as usize..].first_chunk::<W>() {
        let below = w.iter().fold(0u32, |c, e| c + u32::from(e[0] < slot));
        let hi = lo + W as u32;
        let inside = ((lo == 0) | (by_slot[lo.saturating_sub(1) as usize][0] < slot)) & ((hi == n) | (by_slot[hi.min(n - 1) as usize][0] >= slot));
        if inside {
            return lo + below;
        }
    }
    bisect(0, n, |i| by_slot[i as usize][0] < slot)
}

/// The first of `by_slot`'s entries at or past `slot`, stepped to from `guess` one entry at a time, forward while the
/// entry is below the slot, then back while the one before is not; a run of clashes past eight steps is bisected. The
/// slots are hashes, about evenly spread: the entry was within 4 of the guess for 70% of 20 M nodes' lows in 8,890
/// Python files, a mean of 3.45 steps.
#[inline]
fn slot_entry_stepped(by_slot: &[[u32; 3]], slot: u32, guess: u32) -> u32 {
    let n = by_slot.len() as u32;
    let (mut at, mut steps) = (guess, 0);
    while at < n && steps < 8 && by_slot[at as usize][0] < slot {
        (at, steps) = (at + 1, steps + 1);
    }
    while at != 0 && steps < 8 && by_slot[at as usize - 1][0] >= slot {
        (at, steps) = (at - 1, steps + 1);
    }
    if steps == 8 { bisect(0, n, |i| by_slot[i as usize][0] < slot) } else { at }
}

/// The position of `m`'s set bit `k` (`k` below its popcount): the byte by the bytes' running popcounts, then the bit
/// within it (the CLIF's `select64`).
#[inline]
pub fn select64(m: u64, k: u32) -> u32 {
    const ONES: u64 = 0x0101_0101_0101_0101;
    const HIGH: u64 = 0x8080_8080_8080_8080;
    let x = m - ((m >> 1) & 0x5555_5555_5555_5555);
    let x = (x & 0x3333_3333_3333_3333) + ((x >> 2) & 0x3333_3333_3333_3333);
    let x = (x + (x >> 4)) & 0x0f0f_0f0f_0f0f_0f0f;
    // Byte b of `sums`: the set bits of bytes 0..=b.
    let sums = x.wrapping_mul(ONES);
    // The bytes whose running count is at most `k` come before the bit's byte: one high bit for each.
    let byte = ((u64::from(k).wrapping_mul(ONES) | HIGH).wrapping_sub(sums) & HIGH).count_ones();
    // The set bits of the bytes before it: `sums` a byte up, read at the byte (0 for the first), no branch: as one, taken
    // whenever the bit lay in the first byte, it was a fifth of `select`'s samples.
    let before = ((sums << 8) >> (8 * byte)) & 0xff;
    let bits = (m >> (8 * byte)) & 0xff;
    // The bit within the byte, from a table: clearing the byte's lower set bits in a loop of 0 to 7 turns, its exit
    // mispredicted, was 54% of `select`'s samples in an idc run (1,044 of 1,930).
    8 * byte + u32::from(SELECT_IN_BYTE[bits as usize][(u64::from(k) - before) as usize])
}

/// `SELECT_IN_BYTE[b][j]`: the position of byte `b`'s set bit `j` (8 where it has none).
const SELECT_IN_BYTE: [[u8; 8]; 256] = {
    let mut t = [[8u8; 8]; 256];
    let mut b = 0;
    while b < 256 {
        let (mut j, mut i) = (0, 0);
        while i < 8 {
            if b >> i & 1 != 0 {
                t[b][j] = i as u8;
                j += 1;
            }
            i += 1;
        }
        b += 1;
    }
    t
};

/// The first `i` in `lo..hi` for which `below(i)` is false, `below` true before it and false from it (`hi` where
/// none is; the CLIF's `bisect_in`).
#[inline]
/// The first `i` in `0..n` for which `below(i)` is false (`bisect` from 0).
pub(crate) fn bisect_n(n: u32, below: impl Fn(u32) -> bool) -> u32 {
    bisect(0, n, below)
}

fn bisect(mut lo: u32, mut hi: u32, below: impl Fn(u32) -> bool) -> u32 {
    while lo < hi {
        let middle = lo.wrapping_add(hi) >> 1;
        if below(middle) {
            lo = middle + 1;
        } else {
            hi = middle;
        }
    }
    lo
}

/// The numbering's reads (decision 0536): written directly, the same reads as the CLIF's (`id_reads`, the JIT's
/// and the kernels' MSL), held equal to them by the facts crate's tests. An item is a numbered node (0), a token
/// (1), a node under a field (2) or a node after a sibling (3); a number is an item's rank among its kind.
///
/// # Safety
/// Every read: the descriptor's runs were set (`Run::set`) to items that are still there.
impl Numbering {
    /// Run `run`'s items.
    #[inline(always)]
    unsafe fn of<T>(run: &Run) -> &[T] {
        // SAFETY: the caller's (the reads').
        unsafe { run.as_slice() }
    }

    /// Group `g`'s words.
    #[inline(always)]
    unsafe fn group(&self, g: u32) -> &[u64; WORDS as usize] {
        // SAFETY: the caller's.
        let groups = unsafe { Self::of::<[u64; WORDS as usize]>(&self.groups) };
        &groups[g as usize]
    }

    /// `item`'s count before group `g`: word `ITEMS + item / 2`'s low or high half (nodes, tokens; fielded, siblings;
    /// spread).
    #[inline(always)]
    unsafe fn before(&self, g: u32, item: u32) -> u32 {
        // SAFETY: the caller's.
        let word = unsafe { self.group(g) }[(ITEMS + item / 2) as usize];
        (word >> (32 * (item & 1))) as u32
    }

    /// Granule `g`'s u32 in the column `run`.
    #[inline(always)]
    unsafe fn u32_at(run: &Run, g: u32) -> u32 {
        // SAFETY: the caller's.
        let column = unsafe { Self::of::<u32>(run) };
        column[g as usize]
    }

    /// Whether tape node `x` is a numbered `item` (`num_is`).
    ///
    /// # Safety
    /// As every read's (the impl's).
    #[inline]
    pub unsafe fn is(&self, item: u32, x: u32) -> bool {
        let g = (x >> 6).wrapping_sub(self.first);
        if x >> 6 < self.first || g >= self.groups.len as u32 {
            return false;
        }
        // SAFETY: the caller's; `g` is a group.
        let bits = unsafe { self.group(g) }[item as usize];
        bits >> (x & 63) & 1 != 0
    }

    /// The numbered `item`s before tape node `x` (`num_rank`).
    ///
    /// # Safety
    /// As every read's.
    #[inline]
    pub unsafe fn rank(&self, item: u32, x: u32) -> u32 {
        if x >> 6 < self.first {
            return 0;
        }
        let g = (x >> 6) - self.first;
        if g >= self.groups.len as u32 {
            return self.count[item as usize];
        }
        // SAFETY: the caller's; `g` is a group.
        let (before, bits) = unsafe { (self.before(g, item), self.group(g)[item as usize]) };
        before + (bits & ((1u64 << (x & 63)) - 1)).count_ones()
    }

    /// The tape node of `item` number `r` (`NONE` past the last): its sample's group, forward (`num_select`).
    ///
    /// # Safety
    /// As every read's.
    #[inline]
    pub unsafe fn select(&self, item: u32, r: u32) -> u32 {
        if r >= self.count[item as usize] {
            return NONE;
        }
        // SAFETY: the caller's; an item's sample of every `SAMPLE`th number, and `r` a number.
        let mut g = unsafe { Self::u32_at(&self.samples[item as usize], r >> 6) };
        // A node's number lies in its sample's group or the next, about half each (20 M node numbers of 8,890 Python
        // files: 49.9% and 49.9%, 0.2% further): that step taken without a branch, so the loop's first test holds.
        // As a branch, its exit mispredicted half the time (half of `select`'s samples, with the in-byte loop gone).
        if g + 1 < self.groups.len as u32 {
            // SAFETY: the caller's; `g + 1` a group.
            g += u32::from(unsafe { self.before(g + 1, item) } <= r);
        }
        loop {
            // SAFETY: the caller's; number `r` lies in a group from its sample's on.
            let (m, before) = unsafe { (self.group(g)[item as usize], self.before(g, item)) };
            let k = r.wrapping_sub(before);
            if k < m.count_ones() {
                return (self.first + g) * 64 + select64(m, k);
            }
            g += 1;
        }
    }

    /// The groups `select(item, r)` steps past its sample's to reach number `r` (a measurement's read: the walk's length
    /// is the layout's, not the caller's).
    ///
    /// # Safety
    /// As every read's; `r` below the item's count.
    #[cfg(feature = "test-hooks")]
    pub unsafe fn select_walk(&self, item: u32, r: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe {
            let g0 = Self::u32_at(&self.samples[item as usize], r >> 6);
            let mut g = g0;
            while r.wrapping_sub(self.before(g, item)) >= self.group(g)[item as usize].count_ones() {
                g += 1;
            }
            g - g0
        }
    }

    /// The sum of the `by_slot` entries `lows` find, by way `how` (0 stepped, `number_of`'s, 1 a window of 16, 2 of 32): a
    /// measurement's, to time the ways against each other.
    ///
    /// # Safety
    /// As every read's.
    #[cfg(feature = "test-hooks")]
    pub unsafe fn slot_entries(&self, lows: &[u32], how: u8) -> u64 {
        // SAFETY: the caller's.
        let by_slot = unsafe { Self::of::<[u32; 3]>(&self.by_slot) };
        let n = by_slot.len() as u32;
        lows.iter().fold(0u64, |sum, &low| {
            let slot = low >> INDEX_BITS;
            let guess = (((u64::from(slot) * u64::from(n)) >> SLOT_BITS) as u32).min(n.saturating_sub(1));
            let at = match how {
                0 => slot_entry_stepped(by_slot, slot, guess),
                1 => slot_entry::<16>(by_slot, slot, guess),
                _ => slot_entry::<32>(by_slot, slot, guess),
            };
            sum + u64::from(at)
        })
    }

    /// How far `number_of(low)`'s entry lies from its guess, in entries (signed: past it positive), for a low whose
    /// slot has an entry (a measurement's read).
    ///
    /// # Safety
    /// As every read's.
    #[cfg(feature = "test-hooks")]
    pub unsafe fn number_of_miss(&self, low: u32) -> i64 {
        let slot = low >> INDEX_BITS;
        // SAFETY: the caller's.
        let by_slot = unsafe { Self::of::<[u32; 3]>(&self.by_slot) };
        let n = by_slot.len() as u32;
        let guess = (((u64::from(slot) * u64::from(n)) >> SLOT_BITS) as u32).min(n.saturating_sub(1));
        let at = bisect(0, n, |i| by_slot[i as usize][0] < slot);
        i64::from(at) - i64::from(guess)
    }

    /// The `by_slot` entries `number_of(low)` steps over from its guess, `STEPS + 1` where it bisects instead (a
    /// measurement's read).
    ///
    /// # Safety
    /// As every read's.
    #[cfg(feature = "test-hooks")]
    pub unsafe fn number_of_steps(&self, low: u32) -> u32 {
        const STEPS: u32 = 8;
        let slot = low >> INDEX_BITS;
        // SAFETY: the caller's.
        let by_slot = unsafe { Self::of::<[u32; 3]>(&self.by_slot) };
        let n = by_slot.len() as u32;
        let mut at = (((u64::from(slot) * u64::from(n)) >> SLOT_BITS) as u32).min(n.saturating_sub(1));
        let mut steps = 0;
        while at < n && steps < STEPS && by_slot[at as usize][0] < slot {
            (at, steps) = (at + 1, steps + 1);
        }
        while at != 0 && steps < STEPS && by_slot[at as usize - 1][0] >= slot {
            (at, steps) = (at - 1, steps + 1);
        }
        if steps == STEPS { STEPS + 1 } else { steps }
    }

    /// The innermost granule holding `item` number `k` (0 or 1), and the last granule starting at or before it, as
    /// `(g, j)`: a bisection of the starts' ranks between the jumps either side of `k`, then up the parents
    /// (`num_granule`).
    ///
    /// # Safety
    /// As every read's.
    #[inline]
    pub unsafe fn granule(&self, item: u32, k: u32) -> Option<(u32, u32)> {
        const _: () = assert!(JUMP.is_power_of_two());
        // SAFETY: the caller's.
        let (starts, ends, jumps, parents) = unsafe {
            let i = item as usize;
            (Self::of::<u32>(&self.rank_starts[i]), Self::of::<u32>(&self.rank_ends[i]), Self::of::<u32>(&self.jumps[i]), Self::of::<u32>(&self.parents))
        };
        let q = (k >> JUMP.trailing_zeros()) as usize;
        let lo = jumps.get(q).copied().unwrap_or(0);
        let hi = jumps.get(q + 1).map_or(self.granules, |&j| (j + 1).min(self.granules));
        let after = bisect(lo, hi, |i| starts[i as usize] <= k);
        let j = after.checked_sub(1)?;
        let mut g = j;
        while k >= ends[g as usize] {
            g = parents[g as usize];
            if g == NONE {
                return None;
            }
        }
        Some((g, j))
    }

    /// `granule`, from `hint`: the last granule starting at or before the number read before. Where it is still
    /// that for `k` (probes in key order read numbers near each other), no bisection; else `granule`'s, and the hint
    /// is its. The same answer as `granule` whatever the hint (it is checked, not trusted).
    ///
    /// # Safety
    /// As every read's.
    #[inline]
    pub unsafe fn granule_near(&self, item: u32, k: u32, hint: &mut u32) -> Option<(u32, u32)> {
        // SAFETY: the caller's.
        let (starts, ends, parents) = unsafe { (Self::of::<u32>(&self.rank_starts[item as usize]), Self::of::<u32>(&self.rank_ends[item as usize]), Self::of::<u32>(&self.parents)) };
        let j = *hint;
        let n = self.granules;
        if j >= n || starts[j as usize] > k || (j + 1 < n && starts[j as usize + 1] <= k) {
            // SAFETY: the caller's.
            let (g, j) = unsafe { self.granule(item, k) }?;
            *hint = j;
            return Some((g, j));
        }
        let mut g = j;
        while k >= ends[g as usize] {
            g = parents[g as usize];
            if g == NONE {
                return None;
            }
        }
        Some((g, j))
    }

    /// `low_of`, its granule found from `hint` (`granule_near`).
    ///
    /// # Safety
    /// As every read's.
    #[inline]
    pub unsafe fn low_of_near(&self, item: u32, k: u32, hint: &mut u32) -> u32 {
        // SAFETY: the caller's.
        unsafe {
            let Some((g, j)) = self.granule_near(item, k, hint) else { return NONE };
            let index = self.index_in(item, k, g, j);
            let slot = Self::u32_at(&self.slots, Self::u32_at(&self.at, g).wrapping_add(index >> INDEX_BITS));
            slot << INDEX_BITS | index & ((1 << INDEX_BITS) - 1)
        }
    }

    /// `item` number `k`'s index in granule `g` (`j` the last granule starting at or before it).
    #[inline(always)]
    unsafe fn index_in(&self, item: u32, k: u32, g: u32, j: u32) -> u32 {
        // SAFETY: the caller's.
        let (starts, nodes, tokens) = unsafe { (Self::of::<u32>(&self.rank_starts[item as usize]), Self::of::<u32>(&self.nodes), Self::of::<u32>(&self.tokens)) };
        let (g, j) = (g as usize, j as usize);
        let own = k.wrapping_sub(starts[g]);
        if item == 0 {
            own.wrapping_sub(nodes[j + 1].wrapping_sub(nodes[g + 1]))
        } else {
            nodes[g + 1].wrapping_sub(nodes[g]).wrapping_add(own).wrapping_sub(tokens[j + 1].wrapping_sub(tokens[g + 1]))
        }
    }

    /// The low 32 bits of `item` number `k`'s id (a node, 0, or a token, 1), `NONE` for none (`num_low_of`): its
    /// granule's slot (`slots[at[g] + index >> INDEX_BITS]`), then its index's low bits.
    ///
    /// # Safety
    /// As every read's.
    #[inline]
    pub unsafe fn low_of(&self, item: u32, k: u32) -> u32 {
        // SAFETY: the caller's.
        unsafe {
            let Some((g, j)) = self.granule(item, k) else { return NONE };
            let index = self.index_in(item, k, g, j);
            let slot = Self::u32_at(&self.slots, Self::u32_at(&self.at, g).wrapping_add(index >> INDEX_BITS));
            slot << INDEX_BITS | index & ((1 << INDEX_BITS) - 1)
        }
    }

    /// Granule `g`'s child granule entries: `[granule, nodes before, tokens before]`.
    #[inline(always)]
    unsafe fn kids(&self, g: u32) -> &[[u32; 3]] {
        // SAFETY: the caller's.
        let (first, kids) = unsafe { (Self::of::<u32>(&self.first_kid), Self::of::<[u32; 3]>(&self.kids)) };
        &kids[first[g as usize] as usize..first[g as usize + 1] as usize]
    }

    /// The number of granule `g`'s own `item` `k` (its `k`-th, in order), `NONE` when `stretch` is some and the item
    /// does not lie in that stretch of `g` (after `stretch` of its child granules; `num_number_in`).
    ///
    /// # Safety
    /// As every read's.
    #[inline]
    pub unsafe fn number_in(&self, g: u32, item: u32, k: u32, stretch: Option<u32>) -> u32 {
        // SAFETY: the caller's.
        let kids = unsafe { self.kids(g) };
        let w = item as usize + 1;
        let i = bisect(0, kids.len() as u32, |t| kids[t as usize][w] <= k);
        if stretch.is_some_and(|s| s != i) {
            return NONE;
        }
        // SAFETY: the caller's.
        unsafe {
            match i.checked_sub(1) {
                None => Self::u32_at(&self.rank_starts[item as usize], g).wrapping_add(k),
                Some(e) => {
                    let [kid, ..] = kids[e as usize];
                    Self::u32_at(&self.rank_ends[item as usize], kid).wrapping_add(k).wrapping_sub(kids[e as usize][w])
                }
            }
        }
    }

    /// The item whose id's low 32 bits are `low`, as `(item, number)` (`num_number_of`): the slot's entry, then its
    /// index among the granule's nodes or tokens. The slots are hashes, about evenly spread: the entry is near
    /// `slot`'s share of them, a few steps from a guess there; a run of clashes past eight steps is bisected.
    ///
    /// # Safety
    /// As every read's.
    #[inline]
    pub unsafe fn number_of(&self, low: u32) -> Option<(u32, u32)> {
        let slot = low >> INDEX_BITS;
        // SAFETY: the caller's.
        let (by_slot, nodes, tokens) = unsafe { (Self::of::<[u32; 3]>(&self.by_slot), Self::of::<u32>(&self.nodes), Self::of::<u32>(&self.tokens)) };
        let n = by_slot.len() as u32;
        // The guess: `slot`'s share of the entries, the last one at most.
        let guess = (((u64::from(slot) * u64::from(n)) >> SLOT_BITS) as u32).min(n.saturating_sub(1));
        let at = slot_entry_stepped(by_slot, slot, guess);
        let &[s, g, c] = by_slot.get(at as usize)?;
        if s != slot {
            return None;
        }
        let index = c << INDEX_BITS | low & ((1 << INDEX_BITS) - 1);
        let g1 = g as usize + 1;
        let own = nodes[g1].wrapping_sub(nodes[g as usize]);
        let (item, k, all) = if index < own { (0, index, own) } else { (1, index - own, tokens[g1].wrapping_sub(tokens[g as usize])) };
        // SAFETY: the caller's.
        (k < all).then(|| (item, unsafe { self.number_in(g, item, k, None) }))
    }

    /// `number_of` of each of `lows`, ascending (a document's probes in a batch: the bulk read of t57's view reads),
    /// into `out`: `item << 32 | number` each, `u64::MAX` for none. One pass: the slot table's entry found by moving on
    /// from the last low's (a bisection of the rest past `STEPS` entries, as `number_of`'s guess falls back), and a
    /// granule's child granules searched by moving on from the last number's while the granule and item are the same.
    ///
    /// # Safety
    /// As every read's; `lows` ascending, `out` as long.
    pub unsafe fn number_of_sorted(&self, lows: &[u32], out: &mut [u64]) {
        const STEPS: u32 = 8;
        debug_assert!(lows.is_sorted() && out.len() == lows.len());
        // SAFETY: the caller's.
        let (by_slot, nodes, tokens) = unsafe { (Self::of::<[u32; 3]>(&self.by_slot), Self::of::<u32>(&self.nodes), Self::of::<u32>(&self.tokens)) };
        let n = by_slot.len() as u32;
        let mut at = 0u32;
        // The last granule and item searched, and the child granules before the last number (`number_in`'s `i`).
        let (mut last_g, mut last_item, mut i) = (u32::MAX, u32::MAX, 0u32);
        for (o, &low) in out.iter_mut().zip(lows) {
            *o = u64::MAX;
            let slot = low >> INDEX_BITS;
            let mut steps = 0;
            while at < n && steps < STEPS && by_slot[at as usize][0] < slot {
                (at, steps) = (at + 1, steps + 1);
            }
            if steps == STEPS {
                at = bisect(at, n, |e| by_slot[e as usize][0] < slot);
            }
            let Some(&[s, g, c]) = by_slot.get(at as usize) else { continue };
            if s != slot {
                continue;
            }
            let index = c << INDEX_BITS | low & ((1 << INDEX_BITS) - 1);
            let g1 = g as usize + 1;
            let own = nodes[g1].wrapping_sub(nodes[g as usize]);
            let (item, k, all) = if index < own { (0, index, own) } else { (1, index - own, tokens[g1].wrapping_sub(tokens[g as usize])) };
            if k >= all {
                continue;
            }
            // SAFETY: the caller's.
            let kids = unsafe { self.kids(g) };
            let w = item as usize + 1;
            if g != last_g || item != last_item || (i > 0 && kids[i as usize - 1][w] > k) {
                (last_g, last_item) = (g, item);
                i = bisect(0, kids.len() as u32, |t| kids[t as usize][w] <= k);
            } else {
                while (i as usize) < kids.len() && kids[i as usize][w] <= k {
                    i += 1;
                }
            }
            // SAFETY: the caller's.
            let number = unsafe {
                match i.checked_sub(1) {
                    None => Self::u32_at(&self.rank_starts[item as usize], g).wrapping_add(k),
                    Some(e) => Self::u32_at(&self.rank_ends[item as usize], kids[e as usize][0]).wrapping_add(k).wrapping_sub(kids[e as usize][w]),
                }
            };
            *o = u64::from(item) << 32 | u64::from(number);
        }
    }

    /// Node number `l`'s order key: its stretch's run rank, then its index in its granule (its number itself where
    /// the keys are preorder indexes); `NONE` for none (`num_order_of`).
    ///
    /// # Safety
    /// As every read's.
    #[inline]
    pub unsafe fn order_of(&self, l: u32) -> u32 {
        if self.preorder_keys != 0 {
            return l;
        }
        // SAFETY: the caller's.
        unsafe {
            let Some((g, j)) = self.granule(0, l) else { return NONE };
            let index = self.index_in(0, l, g, j);
            let kids = self.kids(g);
            let starts = Self::of::<u32>(&self.rank_starts[0]);
            let s = bisect(0, kids.len() as u32, |t| starts[kids[t as usize][0] as usize] < l);
            let first = Self::u32_at(&self.first_kid, g);
            Self::u32_at(&self.runs, first.wrapping_add(g).wrapping_add(s)) << ORDER_INDEX_BITS | index
        }
    }

    /// The node number whose order key is `key` (`order_of`'s inverse), `NONE` for none (`num_number_of_order`).
    ///
    /// # Safety
    /// As every read's.
    #[inline]
    pub unsafe fn number_of_order(&self, key: u32) -> u32 {
        if self.preorder_keys != 0 {
            return if key < self.count[0] { key } else { NONE };
        }
        // SAFETY: the caller's.
        unsafe {
            let (run_at, nodes) = (Self::of::<[u32; 2]>(&self.run_at), Self::of::<u32>(&self.nodes));
            let Some(&[g, s]) = run_at.get((key >> ORDER_INDEX_BITS) as usize) else { return NONE };
            let k = key & ((1 << ORDER_INDEX_BITS) - 1);
            if k < nodes[g as usize + 1].wrapping_sub(nodes[g as usize]) { self.number_in(g, 0, k, Some(s)) } else { NONE }
        }
    }
}

/// A field's byte offset in the descriptor.
pub mod at {
    use super::{Numbering, Run};
    use std::mem::offset_of;
    pub const FIRST: i32 = offset_of!(Numbering, first) as i32;
    pub const GRANULES: i32 = offset_of!(Numbering, granules) as i32;
    pub const COUNT: i32 = offset_of!(Numbering, count) as i32;
    pub const PREORDER_KEYS: i32 = offset_of!(Numbering, preorder_keys) as i32;
    pub const GROUPS: i32 = offset_of!(Numbering, groups) as i32;
    pub const SAMPLES: i32 = offset_of!(Numbering, samples) as i32;
    pub const STARTS: i32 = offset_of!(Numbering, starts) as i32;
    pub const ENDS: i32 = offset_of!(Numbering, ends) as i32;
    pub const PARENTS: i32 = offset_of!(Numbering, parents) as i32;
    pub const NODES: i32 = offset_of!(Numbering, nodes) as i32;
    pub const TOKENS: i32 = offset_of!(Numbering, tokens) as i32;
    pub const AT: i32 = offset_of!(Numbering, at) as i32;
    pub const SLOTS: i32 = offset_of!(Numbering, slots) as i32;
    pub const BY_SLOT: i32 = offset_of!(Numbering, by_slot) as i32;
    pub const FIRST_KID: i32 = offset_of!(Numbering, first_kid) as i32;
    pub const KIDS: i32 = offset_of!(Numbering, kids) as i32;
    pub const RUNS: i32 = offset_of!(Numbering, runs) as i32;
    pub const RUN_AT: i32 = offset_of!(Numbering, run_at) as i32;
    pub const RANK_STARTS: i32 = offset_of!(Numbering, rank_starts) as i32;
    pub const RANK_ENDS: i32 = offset_of!(Numbering, rank_ends) as i32;
    pub const JUMPS: i32 = offset_of!(Numbering, jumps) as i32;
    /// A run's size, and its fields'.
    pub const RUN: i32 = std::mem::size_of::<Run>() as i32;
    pub const RUN_OFF: i32 = offset_of!(Run, off) as i32;
    pub const RUN_LEN: i32 = offset_of!(Run, len) as i32;
}

/// The descriptor in UKL: `struct Run`, `struct Numbering`, each field's offset and the size asserted against this
/// definition's, so the kernels cannot read a layout of their own.
pub fn ukl() -> String {
    use std::mem::offset_of;
    let (count, samples) = (format!("count[{ITEMS}]"), format!("samples[{ITEMS}]"));
    let fields: [(&str, &str, usize); 21] = [
        ("uint32_t", "first", offset_of!(Numbering, first)),
        ("uint32_t", "granules", offset_of!(Numbering, granules)),
        ("uint32_t", &count, offset_of!(Numbering, count)),
        ("uint32_t", "preorder_keys", offset_of!(Numbering, preorder_keys)),
        ("Run", "groups", offset_of!(Numbering, groups)),
        ("Run", &samples, offset_of!(Numbering, samples)),
        ("Run", "starts", offset_of!(Numbering, starts)),
        ("Run", "ends", offset_of!(Numbering, ends)),
        ("Run", "parents", offset_of!(Numbering, parents)),
        ("Run", "nodes", offset_of!(Numbering, nodes)),
        ("Run", "tokens", offset_of!(Numbering, tokens)),
        ("Run", "at", offset_of!(Numbering, at)),
        ("Run", "slots", offset_of!(Numbering, slots)),
        ("Run", "by_slot", offset_of!(Numbering, by_slot)),
        ("Run", "first_kid", offset_of!(Numbering, first_kid)),
        ("Run", "kids", offset_of!(Numbering, kids)),
        ("Run", "runs", offset_of!(Numbering, runs)),
        ("Run", "run_at", offset_of!(Numbering, run_at)),
        ("Run", "rank_starts[2]", offset_of!(Numbering, rank_starts)),
        ("Run", "rank_ends[2]", offset_of!(Numbering, rank_ends)),
        ("Run", "jumps[2]", offset_of!(Numbering, jumps)),
    ];
    let mut s = String::from("#include <ukl/backend.h>\ntypedef struct Run {\n    int64_t off;\n    uint64_t len;\n} Run;\nUKL_STATIC_ASSERT(sizeof(Run) == 16, \"Run: size\");\n");
    s += "typedef struct Numbering {\n";
    for (ty, name, _) in fields.iter() {
        s += &format!("    {ty} {name};\n");
    }
    s += "} Numbering;\n";
    s += &format!("UKL_STATIC_ASSERT(sizeof(Numbering) == {}, \"Numbering: size\");\n", std::mem::size_of::<Numbering>());
    for (_, name, at) in fields.iter() {
        let name = name.split('[').next().unwrap_or(name);
        s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(Numbering, {name}) == {at}, \"Numbering.{name}\");\n");
    }
    s
}

#[cfg(test)]
mod select_tests {
    /// `select64` finds set bit `k` of a word as a scan of its bits does: every set bit of words of every density,
    /// each byte's every pattern among them.
    #[test]
    fn a_words_set_bit_k_is_where_a_scan_finds_it() {
        let mut words: Vec<u64> = (0..256u64).flat_map(|b| (0..8).map(move |at| b << (8 * at))).collect();
        let mut r = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..20_000 {
            r ^= r << 13;
            r ^= r >> 7;
            r ^= r << 17;
            // Sparse, even and dense words.
            words.extend([r, r & r.rotate_left(17), r | r.rotate_left(29), u64::MAX, 1 << (r % 64)]);
        }
        for &m in &words {
            let set: Vec<u32> = (0..64).filter(|&i| m >> i & 1 != 0).collect();
            for (k, &want) in set.iter().enumerate() {
                assert_eq!(super::select64(m, k as u32), want, "{m:#x} bit {k}");
            }
        }
    }
}
