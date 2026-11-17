//! The one block policy (decision 0342): every block of the process is made here (`rerail_blocks::alloc::block`
//! calls it once `install`ed), so one place sees the memory they hold and chooses each one's.
//!
//! - `Device::Must`: a slab of the GPU pool within the Metal budget (`gpu::shared::lease_block_within`: the pool's
//!   eviction order makes room).
//! - `Device::May`: a slab only when the budget holds it with nothing evicted (`lease_block_spare`); else host
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
    let shaped = |b| rerail_blocks::alloc::shaped(b, need);
    match need.device {
        #[cfg(target_os = "macos")]
        Device::Must => crate::gpu::shared::lease_block_within(need.kind, need.bytes).map(shaped),
        #[cfg(not(target_os = "macos"))]
        Device::Must => Err(format!("a device block of {} bytes with no device", need.bytes)),
        #[cfg(target_os = "macos")]
        Device::May => match crate::gpu::shared::lease_block_spare(need.kind, need.bytes) {
            Ok(b) => Ok(shaped(b)),
            Err(_) => Ok(host(need)),
        },
        #[cfg(not(target_os = "macos"))]
        Device::May => Ok(host(need)),
        Device::No => Ok(host(need)),
    }
}

/// The host blocks made, while they live (their extents; a dropped block's is gone).
static HOST: std::sync::Mutex<Vec<std::sync::Weak<rerail_blocks::block::Extent>>> = std::sync::Mutex::new(Vec::new());

/// A host block for `need`, counted.
fn host(need: &Need) -> Block {
    let b = rerail_blocks::alloc::shaped(Block::anon(need.kind, need.bytes), need);
    let mut live = HOST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    live.retain(|e| e.strong_count() > 0);
    live.push(b.meter());
    b
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
    use rerail_blocks::alloc::Writes;
    use rerail_blocks::block::kind;

    fn need(writes: Writes, device: Device) -> Need {
        Need { kind: kind::FACTS, bytes: 8 << 20, writes, device }
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
}
