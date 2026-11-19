//! The one block policy (decision 0342): every block of the process is made here (`rerail_blocks::alloc::block`
//! calls it once `install`ed), so one place sees the memory they hold and chooses each one's.
//!
//! - `Device::Must`: a slab of the GPU pool within the Metal budget (`gpu::shared::lease_block_within`: the pool's
//!   eviction order makes room); an exact block in a join's budget is a pooled buffer of its size, leased as the
//!   join's buffers are (`gpu::shared::lease_block_for_join`: a device run's own relation, decision 0325 step 5).
//! - `Device::May`: a slab only when the budget holds it with nothing evicted and half of it left to the device's
//!   own buffers (`lease_block_spare`); else host
//!   memory, which the device reads in place through a no-copy buffer.
//! - `Device::No`: host memory: anonymous, resident only where written.
//!
//! Host blocks are counted (`host_held`) beside the pool's leases (`gpu::shared::held_bytes`): the profile reads
//! both, and a policy that weighs the host's pressure reads the same ledger.

use rerail_blocks::Block;
use rerail_blocks::alloc::{Device, Need};

/// Installs this policy as the process's (`rerail_blocks::alloc::install`).
pub fn install() {
    rerail_blocks::alloc::install(policy);
}

/// A block for `need` (module docs).
pub fn policy(need: &Need) -> Result<Block, String> {
    match need.device {
        Device::Must => leased(need).map(|b| rerail_blocks::alloc::shaped(b, need)),
        Device::May => Ok(spare(need).map_or_else(|| host(need), |b| rerail_blocks::alloc::shaped(b, need))),
        Device::No => Ok(host(need)),
    }
}

/// A device block for `need`, leased from the pool in its budget.
#[cfg(target_os = "macos")]
fn leased(need: &Need) -> Result<Block, String> {
    use rerail_blocks::alloc::{Budget, Size};
    match (need.size, need.budget) {
        (Size::Slab, Budget::Metal) => crate::gpu::shared::lease_block_within(need.kind, need.bytes),
        (Size::Exact, Budget::Join) => crate::gpu::shared::lease_block_for_join(need.kind, need.bytes),
        (size, budget) => Err(format!("a device block of {} bytes, {size:?} in the {budget:?} budget: no such lease", need.bytes)),
    }
}

/// A device block for `need`: none without a device.
#[cfg(not(target_os = "macos"))]
fn leased(need: &Need) -> Result<Block, String> {
    Err(format!("a device block of {} bytes with no device", need.bytes))
}

/// A device block for `need` where the device has room to spare, else `None` (a host block, then). Half the Metal
/// budget stays the device's own buffers': taking the last room had the residency drop everything it held 28 times
/// a run (decision 0325 step 3, `lease_block_spare`'s `keep`).
#[cfg(target_os = "macos")]
fn spare(need: &Need) -> Option<Block> {
    crate::gpu::shared::lease_block_spare(need.kind, need.bytes, crate::budget::metal() / 2).ok()
}

/// No device: no block of one to spare.
#[cfg(not(target_os = "macos"))]
fn spare(_: &Need) -> Option<Block> {
    None
}

/// The host blocks made, while they live (their extents; a dropped block's is gone).
static HOST: std::sync::Mutex<Vec<std::sync::Weak<rerail_blocks::block::Extent>>> = std::sync::Mutex::new(Vec::new());

/// A host block for `need`, counted.
fn host(need: &Need) -> Block {
    let b = rerail_blocks::alloc::shaped(Block::anon(need.kind, rerail_blocks::alloc::bytes_of(need)), need);
    let mut live = HOST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    live.retain(|e| e.strong_count() > 0);
    live.push(b.meter());
    b
}

/// The bytes zeroed as blocks' memory was handed out again (`rerail_blocks::block::zeroed_bytes`).
pub fn zeroed_bytes() -> u64 {
    rerail_blocks::block::zeroed_bytes()
}

/// The host blocks live now: bytes used in them, and of those the bytes their free lists hold.
pub fn host_held() -> (usize, usize) {
    let mut live = HOST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    live.retain(|e| e.strong_count() > 0);
    live.iter().filter_map(std::sync::Weak::upgrade).fold((0, 0), |(u, f), e| {
        (u + e.top.load(std::sync::atomic::Ordering::Relaxed), f + e.free.load(std::sync::atomic::Ordering::Relaxed))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rerail_blocks::alloc::{Budget, Size, Writes};
    use rerail_blocks::block::kind;

    fn need(writes: Writes, device: Device) -> Need {
        Need { kind: kind::FACTS, bytes: 8 << 20, writes, device, ..Need::SLAB }
    }

    /// A block no device reads is host memory, counted while it lives and not after; one that churns keeps a
    /// free list, whose bytes the ledger names.
    #[test]
    fn a_host_block_is_counted_while_it_lives() {
        let (before, _) = host_held();
        let b = policy(&need(Writes::Churn, Device::No)).unwrap();
        assert!(!b.in_slab() && b.has_free_list());
        let at = b.try_alloc_chunk(1 << 20, 1 << 14).unwrap();
        b.free_chunk(at, 1 << 20);
        let (used, free) = host_held();
        assert!(used >= before + (1 << 20), "counted: {used} after {before}");
        assert!(free >= 1 << 20, "its free list's bytes: {free}");
        drop(b);
        assert!(host_held().0 < used, "not counted once dropped");
    }

    /// A block the device must read is a slab of the pool; one it may read is a slab when the budget has room
    /// spare. Without a device, the first is refused and the second is host memory.
    #[test]
    fn the_device_gets_a_slab_when_it_must_or_when_one_is_spare() {
        let must = policy(&need(Writes::Once, Device::Must));
        let may = policy(&need(Writes::Churn, Device::May)).unwrap();
        assert!(may.has_free_list());
        #[cfg(target_os = "macos")]
        if crate::gpu::shared::get().is_some() {
            assert!(must.unwrap().in_slab(), "a device block is a slab");
            assert!(may.in_slab(), "a slab when one is spare");
            return;
        }
        assert!(must.is_err());
        assert!(!may.in_slab());
    }

    /// A device run's own relation (Q205): an exact block counted in a join's budget is a pooled buffer of its
    /// size, not a slab's least; an exact block no device reads is host memory of its size.
    #[test]
    fn an_exact_device_block_in_a_joins_budget_is_its_size() {
        let exact = |device| Need { kind: kind::FACTS, bytes: 40 << 10, writes: Writes::Once, device, size: Size::Exact, budget: Budget::Join };
        let host = policy(&exact(Device::No)).unwrap();
        assert!(!host.in_slab());
        let page = rerail_blocks::block::PAGE;
        assert_eq!(host.reserved(), (40usize << 10).max(rerail_blocks::alloc::EXACT_MIN).div_ceil(page) * page, "its need, in whole pages");
        #[cfg(target_os = "macos")]
        if crate::gpu::shared::get().is_some() {
            let b = policy(&exact(Device::Must)).unwrap();
            assert!(b.in_slab(), "a pooled buffer");
            assert!(b.reserved() < crate::gpu::shared::SLAB_MIN, "of its size ({} bytes), not a slab's least", b.reserved());
            return;
        }
        assert!(policy(&exact(Device::Must)).is_err());
    }
}
