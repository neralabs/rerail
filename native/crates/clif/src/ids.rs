//! A projection's numbering as its reads are told where it lies (decision 0353 step 2): the rank index of the tape
//! nodes it numbers, and its granule table. Each column is a run in the block the descriptor is in, `off` bytes from
//! the run's own address (`rerail_blocks::refs::Span`'s layout). One definition: the host's `Ranks` and
//! `granules::Table` fill it, the reads (`id_reads`) read it in every backend, and its MSL is written from it.
//!
//! The rank index is groups of 64 tape nodes, `WORDS` u64s each: one word of bits per item (a numbered node, a token,
//! a node under a field, a node after a sibling), then the nodes before the group (low 32 bits) and the tokens (high
//! 32 bits), then the fielded nodes (low) and the nodes after a sibling (high) before it. A select sample gives the group of every `SAMPLE`-th item.
//!
//! The granule table is per granule (in preorder of their starts): its start and its subtree's end as ranks of each
//! item (`rank_starts`, `rank_ends`; nodes, then tokens), its parent, its own nodes and tokens as prefix sums, its
//! slots, its child granules (`kids`: the granule, and its parent's own nodes and tokens before it) and the run rank
//! of each stretch between them (`runs`), and the slots and runs by key (`by_slot`, `run_at`).

/// The items a rank index marks: numbered nodes, tokens, nodes under a field, nodes after a numbered sibling
/// (`Item`'s numbers).
pub const ITEMS: u32 = 4;
/// A group's words: one of bits an item, then the counts before it.
pub const WORDS: u32 = ITEMS + 2;
/// Items a select sample stands for.
pub const SAMPLE: u32 = 64;
/// An id's low 32 bits: its granule's slot, then its index in it (`INDEX_BITS`).
pub const SLOT_BITS: u32 = 14;
pub const INDEX_BITS: u32 = 32 - SLOT_BITS;
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
const _: () = assert!(std::mem::size_of::<Numbering>() == 32 + 16 * 23);

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

/// The descriptor's MSL: `struct Run`, `struct Numbering`, each field's offset and the size asserted against this
/// definition's, so the kernels cannot read a layout of their own.
pub fn msl() -> String {
    use std::mem::offset_of;
    let (count, samples) = (format!("count[{ITEMS}]"), format!("samples[{ITEMS}]"));
    let fields: [(&str, &str, usize); 21] = [
        ("uint", "first", offset_of!(Numbering, first)),
        ("uint", "granules", offset_of!(Numbering, granules)),
        ("uint", &count, offset_of!(Numbering, count)),
        ("uint", "preorder_keys", offset_of!(Numbering, preorder_keys)),
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
    let mut s = String::from("struct Run {\n    long off;\n    ulong len;\n};\nstatic_assert(sizeof(Run) == 16, \"Run: size\");\n");
    s += "struct Numbering {\n";
    for (ty, name, _) in fields.iter() {
        s += &format!("    {ty} {name};\n");
    }
    s += "};\n";
    s += &format!("static_assert(sizeof(Numbering) == {}, \"Numbering: size\");\n", std::mem::size_of::<Numbering>());
    for (_, name, at) in fields.iter() {
        let name = name.split('[').next().unwrap_or(name);
        s += &format!("static_assert(__builtin_offsetof(Numbering, {name}) == {at}, \"Numbering.{name}\");\n");
    }
    s
}
