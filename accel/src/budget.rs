//! The process's memory budgets (decision 0126): the one place every bounded part of the engine reads its size from.
//!
//! - **Overall** (`--memory-mb`, `RERAIL_MEMORY_MB`; 1 GB by default): what the process keeps resident on the host.
//!   The parts sized from it (the write queue, the working set, what a session keeps of its cross-file part) take
//!   a fixed share of it: `share(part_at_default)` is the part's size at the default budget, scaled.
//! - **Metal** (`--metal-mb`, `RERAIL_METAL_MB`; half the overall budget by default): the device's buffers, on top of
//!   the overall budget. A run with `--metal` may use the two together: 1.5 GB by default.
//!
//! A command line sets them (`set_memory_mb`, `set_metal_mb`) before the run starts; read before, they are the
//! environment's or the defaults. They are read on every use, so a test may set them.

use std::sync::atomic::{AtomicUsize, Ordering};

/// The overall budget by default: 1 GB.
pub const MEMORY_DEFAULT: usize = 1 << 30;

/// The budgets set (`set_memory_mb`, `set_metal_mb`; 0: from the environment, or the defaults): the machine's
/// (decision 0442), which every caller on it shares.
#[derive(Default)]
pub(crate) struct Budgets {
    memory: AtomicUsize,
    metal: AtomicUsize,
}

/// The budgets of the caller's machine: what a lease keeps to go back to its own (`gpu::shared::Pooled`).
pub(crate) fn budgets() -> std::sync::Arc<Budgets> {
    rerail_context::machine::<Budgets>()
}

impl Budgets {
    /// This machine's Metal budget, in bytes (`metal`).
    pub(crate) fn metal(&self) -> usize {
        #[cfg(test)]
        if let bytes @ 1.. = METAL_TEST.with(std::cell::Cell::get) {
            return bytes;
        }
        match self.metal.load(Ordering::Relaxed) {
            0 => env_mb("RERAIL_METAL_MB").unwrap_or_else(|| self.memory() / 2),
            bytes => bytes,
        }
    }

    /// This machine's overall budget, in bytes (`memory`).
    pub(crate) fn memory(&self) -> usize {
        match self.memory.load(Ordering::Relaxed) {
            0 => {
                let bytes = env_mb("RERAIL_MEMORY_MB").unwrap_or(MEMORY_DEFAULT);
                self.memory.store(bytes, Ordering::Relaxed);
                bytes
            }
            bytes => bytes,
        }
    }
}

fn env_mb(name: &str) -> Option<usize> {
    std::env::var(name).ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|&mb| mb > 0).map(|mb| mb << 20)
}

/// The overall budget, in bytes.
pub fn memory() -> usize {
    budgets().memory()
}

#[cfg(test)]
thread_local! {
    /// A test's Metal budget, on its thread alone (tests run side by side; the budgets are the process's).
    pub(crate) static METAL_TEST: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The Metal budget, in bytes: half the overall budget unless set.
pub fn metal() -> usize {
    budgets().metal()
}

/// Sets the overall budget (a command line's `--memory-mb`); 0 is refused.
pub fn set_memory_mb(mb: usize) -> Result<(), String> {
    if mb == 0 {
        return Err("the memory budget must be at least 1 MB".into());
    }
    budgets().memory.store(mb << 20, Ordering::Relaxed);
    Ok(())
}

/// Sets the Metal budget (a command line's `--metal-mb`); 0 is refused.
pub fn set_metal_mb(mb: usize) -> Result<(), String> {
    if mb == 0 {
        return Err("the Metal budget must be at least 1 MB".into());
    }
    budgets().metal.store(mb << 20, Ordering::Relaxed);
    Ok(())
}

/// A part of the overall budget: `at_default` bytes at the default budget, in proportion to the budget set.
pub fn share(at_default: usize) -> usize {
    (at_default as u128 * memory() as u128 / MEMORY_DEFAULT as u128) as usize
}

/// A part of the Metal budget: `at_default` bytes of its default (half the default overall budget), in proportion.
pub fn metal_share(at_default: usize) -> usize {
    (at_default as u128 * metal() as u128 / (MEMORY_DEFAULT / 2) as u128) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test owns the process's budgets (they are process-wide).
    #[test]
    fn budgets_default_set_and_share() {
        if std::env::var_os("RERAIL_MEMORY_MB").is_none() && std::env::var_os("RERAIL_METAL_MB").is_none() {
            assert_eq!(memory(), 1 << 30);
            assert_eq!(metal(), 512 << 20, "Metal defaults to half the overall budget");
            assert_eq!(share(192 << 20), 192 << 20);
        }
        set_memory_mb(2048).unwrap();
        assert_eq!(share(192 << 20), 384 << 20, "a part scales with the overall budget");
        assert_eq!(metal(), 1 << 30, "unset, Metal follows the overall budget");
        METAL_TEST.with(|m| m.set(256 << 20));
        assert_eq!(metal(), 256 << 20);
        assert_eq!(metal_share(64 << 20), 32 << 20);
        METAL_TEST.with(|m| m.set(0));
        assert!(set_memory_mb(0).is_err());
    }
}
