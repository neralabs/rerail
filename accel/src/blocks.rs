//! The one block policy (decision 0342): every block of the process is made here (`rerail_blocks::alloc::block`
//! calls it once `install`ed), so one place sees the memory they hold and chooses each one's.
//!
//! - `Device::Must`: a slab of the GPU pool within the Metal budget (`gpu::shared::lease_block_within`: the pool's
//!   eviction order makes room); an exact block in a join's budget is a pooled buffer of its size, leased as the
//!   join's buffers are (`gpu::shared::lease_block_for_join`: a device run's own relation, decision 0325 step 5).
//! - `Device::May`: host memory, resident only where written, which the device reads in place through a no-copy
//!   wrap of its used extent (`Block::device_buffer`). Never a pool slab: a slab's whole length held the Metal budget
//!   while a few MB of it were written (decision 0435 §1).
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
        Device::Must => device(need),
        // Host memory the device reads through a wrap of its used extent (decision 0435 §1).
        Device::May | Device::No => Ok(host(need)),
    }
}

/// A device block for `need`: a lease of the GPU pool's.
#[cfg(gpu_lane)]
fn device(need: &Need) -> Result<Block, String> {
    use rerail_blocks::alloc::{Budget, Size};
    match (need.size, need.budget) {
        (Size::Slab, Budget::Metal) => crate::gpu::shared::lease_block_within(need.kind, need.bytes),
        (Size::Exact, Budget::Join) => crate::gpu::shared::lease_block_for_join(need.kind, need.bytes),
        (size, budget) => Err(format!("a device block of {} bytes, {size:?} in the {budget:?} budget: no such lease", need.bytes)),
    }
    .map(|b| rerail_blocks::alloc::shaped(b, need))
}

/// No GPU lane: no device block.
#[cfg(not(gpu_lane))]
fn device(need: &Need) -> Result<Block, String> {
    Err(format!("a device block of {} bytes with no device", need.bytes))
}

/// The host blocks made, while they live (their extents; a dropped block's is gone): the machine's memory account
/// (decision 0442).
#[derive(Default)]
struct Host(std::sync::Mutex<Vec<std::sync::Weak<rerail_blocks::block::Extent>>>);

/// A host block for `need`, counted.
fn host(need: &Need) -> Block {
    let b = rerail_blocks::alloc::shaped(Block::anon(need.kind, rerail_blocks::alloc::bytes_of(need)), need);
    let host = rerail_context::machine::<Host>();
    let mut live = host.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    live.retain(|e| e.strong_count() > 0);
    live.push(b.meter());
    b
}

/// The bytes zeroed as blocks' memory was handed out again (`rerail_blocks::block::zeroed_bytes`).
pub fn zeroed_bytes() -> u64 {
    rerail_blocks::block::zeroed_bytes()
}

/// The bytes the host blocks' free lists keep resident: their free chunks less those whose pages went to the OS
/// (`Extent::given`).
pub fn host_free_resident() -> usize {
    let host = rerail_context::machine::<Host>();
    let mut live = host.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    live.retain(|e| e.strong_count() > 0);
    live.iter().filter_map(std::sync::Weak::upgrade).map(|e| {
        let free = e.free.load(std::sync::atomic::Ordering::Relaxed);
        free.saturating_sub(e.given.load(std::sync::atomic::Ordering::Relaxed))
    }).sum()
}

/// The host blocks live now: bytes used in them, and of those the bytes their free lists hold.
pub fn host_held() -> (usize, usize) {
    let host = rerail_context::machine::<Host>();
    let mut live = host.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
    /// free list, whose bytes the ledger names. On a machine of its own (decision 0442): the tests' machine counts
    /// every test's blocks, made while this one runs.
    #[test]
    fn a_host_block_is_counted_while_it_lives() {
        let _own = rerail_context::within(rerail_context::Shared::new());
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

    /// A block the device must read is a slab of the pool. One it may read is host memory, even when the Metal budget
    /// has room (decision 0435 R1), and the device reads it through a wrap of its used extent: at least 1 MB, at most
    /// twice what it holds. Without a device, the first is refused.
    #[test]
    fn a_block_the_device_may_read_is_host_memory_wrapped_by_its_used_extent() {
        // Its device leases are counted in the pool's budget, which the pool's tests count.
        #[cfg(gpu_lane)]
        let _leases = crate::gpu::shared::big_leases();
        let must = policy(&need(Writes::Once, Device::Must));
        let may = policy(&need(Writes::Churn, Device::May)).unwrap();
        assert!(may.has_free_list());
        assert!(!may.in_slab(), "host memory, though the budget has room");
        // The process's device, whatever its backend (decision 0440 steps C and E).
        #[cfg(gpu_lane)]
        if let Some(layer) = crate::gpu::shared::layer() {
            assert!(must.unwrap().in_slab(), "a device block is a slab");
            let at = may.try_alloc_chunk(3 << 20, 1 << 14).unwrap();
            assert!(at > 0);
            let used = may.used();
            let wrap = may.device_buffer(layer.device).unwrap().length() as usize;
            assert!(wrap >= used && wrap <= (2 * used).max(1 << 20).min(may.reserved()), "a wrap of {wrap} bytes for {used} used");
            return;
        }
        assert!(must.is_err());
    }

    /// A device run's own relation (Q205): an exact block counted in a join's budget is a pooled buffer of its
    /// size, not a slab's least; an exact block no device reads is host memory of its size.
    #[test]
    fn an_exact_device_block_in_a_joins_budget_is_its_size() {
        // Its device leases are counted in the pool's budget, which the pool's tests count.
        #[cfg(gpu_lane)]
        let _leases = crate::gpu::shared::big_leases();
        let exact = |device| Need { kind: kind::FACTS, bytes: 40 << 10, writes: Writes::Once, device, size: Size::Exact, budget: Budget::Join };
        let host = policy(&exact(Device::No)).unwrap();
        assert!(!host.in_slab());
        let page = rerail_blocks::block::PAGE;
        assert_eq!(host.reserved(), (40usize << 10).max(rerail_blocks::alloc::EXACT_MIN).div_ceil(page) * page, "its need, in whole pages");
        #[cfg(gpu_lane)]
        if crate::gpu::shared::layer().is_some() {
            let b = policy(&exact(Device::Must)).unwrap();
            assert!(b.in_slab(), "a pooled buffer");
            assert!(b.reserved() < crate::gpu::shared::SLAB_MIN, "of its size ({} bytes), not a slab's least", b.reserved());
            return;
        }
        assert!(policy(&exact(Device::Must)).is_err());
    }
}
