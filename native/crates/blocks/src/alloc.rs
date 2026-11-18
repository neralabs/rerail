//! Where every block comes from (decision 0342): a caller states what it needs, and the one policy installed
//! (`install`; `rerail_accel::blocks`, which sees the Metal budget and the host blocks it made) chooses the memory.
//!
//! ```text
//! alloc::block(&Need { kind: FACTS, bytes: 64 MB, writes: Writes::Churn, device: Device::May })
//!     -> the policy: a pool slab if the Metal budget has it spare, else anonymous memory; a free list (Churn)
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

/// What a caller needs of a block; never which memory.
#[derive(Clone, Copy, Debug)]
pub struct Need {
    pub kind: u32,
    pub bytes: usize,
    pub writes: Writes,
    pub device: Device,
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
    block(&Need { kind, bytes, writes: Writes::Once, device: Device::No }).expect("a host block")
}

/// A block of anonymous memory for `need` (free-listed when it churns); refused when it must be the device's.
pub fn plain(need: &Need) -> Result<Block, String> {
    if need.device == Device::Must {
        return Err(format!("a device block of {} bytes with no device pool", need.bytes));
    }
    Ok(shaped(Block::anon(need.kind, need.bytes), need))
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
        let need = |writes, device| Need { kind: kind::FACTS, bytes: 1 << 20, writes, device };
        let once = plain(&need(Writes::Once, Device::May)).unwrap();
        assert!(!once.in_slab() && !once.has_free_list());
        let churn = plain(&need(Writes::Churn, Device::No)).unwrap();
        assert!(!churn.in_slab() && churn.has_free_list());
        assert!(plain(&need(Writes::Once, Device::Must)).is_err());
    }
}
