//! Where every block comes from (decision 0342): a caller states what it needs, and the one policy installed
//! (`install`; `rerail_accel::blocks`, which sees the Metal budget and the host blocks it made) chooses the memory.
//!
//! ```text
//! alloc::block(&Need { kind: FACTS, bytes: 64 MB, writes: Writes::Churn, device: Device::May, ..Need::SLAB })
//!     -> the policy: a pool slab if the Metal budget has it spare, else anonymous memory; a free list (Churn)
//! alloc::block(&Need { kind: FACTS, bytes: 40 KB, writes: Writes::Once, device: Device::Must, size: Size::Exact, budget: Budget::Join })
//!     -> the policy: a pooled buffer of the relation's size, bounded as a device join's buffers are
//! ```
//!
//! Without a policy (this crate's own tests, a tool that never sets up the pool) a block is anonymous memory, and
//! one that must be the device's is refused.

use crate::block::Block;
use std::sync::OnceLock;

/// How a block's structures are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Writes {
    /// Built front to back, then frozen: bump-allocated.
    Once,
    /// Open structures made and dropped while it lives (a chunk's relations): a free list (`Block::with_free_list`).
    Churn,
}

/// Whether the device reads the block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    /// Never: host memory.
    No,
    /// Perhaps (the run is bound to the device): a device slab when it costs nothing else, else host memory the
    /// device reads through a no-copy buffer.
    May,
    /// Always: a device slab, made room for.
    Must,
}

/// How big a block is, past the bytes needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// A slab's least size or more (the pool's `SLAB_MIN`): a block many structures share.
    Slab,
    /// The bytes needed (at least `EXACT_MIN`): one structure alone in it, given back with it (a device run's own
    /// relation, decision 0325 step 5). Shared 16 MB slabs, kept by any relation in them, refused small views their
    /// room.
    Exact,
}

/// The least an exact block is: four pages.
pub const EXACT_MIN: usize = 4 * crate::block::PAGE;

/// The budget a device block is counted in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Budget {
    /// The Metal budget, whose eviction order makes room.
    Metal,
    /// A device join's: bounded inside the join's budget where it runs within one (a refusal runs the join on the
    /// CPU), as the join's own buffers are.
    Join,
}

/// What a caller needs of a block; never which memory.
#[derive(Clone, Copy, Debug)]
pub struct Need {
    pub kind: u32,
    pub bytes: usize,
    pub writes: Writes,
    pub device: Device,
    pub size: Size,
    /// Read for a device block only.
    pub budget: Budget,
}

impl Need {
    /// The usual size and budget, to fill a need's other fields from (`..Need::SLAB`).
    pub const SLAB: Need = Need { kind: 0, bytes: 0, writes: Writes::Once, device: Device::No, size: Size::Slab, budget: Budget::Metal };
}

/// The policy: a block for a need.
pub type Policy = fn(&Need) -> Result<Block, String>;

static POLICY: OnceLock<Policy> = OnceLock::new();

/// Installs the process's policy; the first one installed stays.
pub fn install(policy: Policy) {
    let _ = POLICY.set(policy);
}

/// A block for `need`, by the policy installed (else `plain`).
pub fn block(need: &Need) -> Result<Block, String> {
    match POLICY.get() {
        Some(p) => p(need),
        None => plain(need),
    }
}

/// A block built once and read on the host (a builder's: a memo entry, a packed relation, a program, facts), from
/// the policy.
pub fn host(kind: u32, bytes: usize) -> Block {
    block(&Need { kind, bytes, writes: Writes::Once, device: Device::No, ..Need::SLAB }).expect("a host block")
}

/// A block of anonymous memory for `need` (free-listed when it churns); refused when it must be the device's.
pub fn plain(need: &Need) -> Result<Block, String> {
    if need.device == Device::Must {
        return Err(format!("a device block of {} bytes with no device pool", need.bytes));
    }
    Ok(shaped(Block::anon(need.kind, bytes_of(need)), need))
}

/// The bytes a host block for `need` has: what it needs, at least `EXACT_MIN` when exact.
pub fn bytes_of(need: &Need) -> usize {
    match need.size {
        Size::Slab => need.bytes,
        Size::Exact => need.bytes.max(EXACT_MIN),
    }
}

/// `b` with what `need.writes` asks: a free list when it churns.
pub fn shaped(b: Block, need: &Need) -> Block {
    match need.writes {
        Writes::Once => b,
        Writes::Churn => b.with_free_list(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::kind;

    /// With no policy, a block is anonymous; one that churns keeps a free list; one the device must read is
    /// refused.
    #[test]
    fn with_no_policy_a_block_is_anonymous_and_shaped_by_its_writes() {
        let need = |writes, device| Need { kind: kind::FACTS, bytes: 1 << 20, writes, device, ..Need::SLAB };
        let once = plain(&need(Writes::Once, Device::May)).unwrap();
        assert!(!once.in_slab() && !once.has_free_list());
        let churn = plain(&need(Writes::Churn, Device::No)).unwrap();
        assert!(!churn.in_slab() && churn.has_free_list());
        assert!(plain(&need(Writes::Once, Device::Must)).is_err());
        // An exact block is its need, at least four pages.
        let small = Need { kind: kind::FACTS, bytes: 100, writes: Writes::Once, device: Device::No, size: Size::Exact, budget: Budget::Join };
        assert_eq!(plain(&small).unwrap().reserved(), EXACT_MIN);
    }
}
