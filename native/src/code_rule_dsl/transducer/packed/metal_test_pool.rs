//! The device pool in a test build (O83).
//!
//! In production the pool is the process's: every evaluation's buffers count against its 256 MiB,
//! and a plan is sized from what is free. Tests run in parallel threads, so under one process-wide
//! ledger a test would size its plan from what the other tests left, and fail or measure their
//! buffers as its own.
//!
//! So in a test build each thread has its own ledger. A test sees the pool, its peak and the driver
//! as it would alone (the driver figure is the process's, so it is left out: the thread's ledger
//! stands in for it).
//!
//! What all tests hold at once is bounded by a weighted lock over bytes: `TEST_DEVICE_BYTES`, the
//! engine's RAM budget. A reservation that would pass it waits until other tests release theirs,
//! so small tests run side by side and a large one waits for room. The wait is bounded (a test
//! may hold buffers while it waits on another thread that needs some), and after it the
//! reservation goes ahead: the bound is on memory, never on correctness.
use std::sync::atomic::AtomicUsize;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// What the tests' Metal buffers may hold at once: the engine's 1 GB RAM budget.
const TEST_DEVICE_BYTES: usize = 1 << 30;
/// The longest a reservation waits for room.
const ADMISSION_WAIT: Duration = Duration::from_secs(30);

struct Ledgers {
    pool: &'static AtomicUsize,
    peak: &'static AtomicUsize,
}

thread_local! {
    // Leaked: a reservation can outlive its thread (a cached buffer, or one dropped elsewhere),
    // and it releases into the ledger it was charged to. Two words per test thread.
    static LEDGERS: Ledgers = Ledgers {
        pool: Box::leak(Box::new(AtomicUsize::new(0))),
        peak: Box::leak(Box::new(AtomicUsize::new(0))),
    };
}

static HELD: Mutex<usize> = Mutex::new(0);
static RELEASED: Condvar = Condvar::new();

/// This thread's pool ledger.
pub(super) fn pool() -> &'static AtomicUsize {
    LEDGERS.with(|ledgers| ledgers.pool)
}

/// The most this thread's pool ledger has held.
pub(super) fn pool_peak() -> &'static AtomicUsize {
    LEDGERS.with(|ledgers| ledgers.peak)
}

/// Left out in a test build: the driver's figure is the process's, every test's buffers.
pub(super) fn driver_bytes(_device: &metal::DeviceRef) -> u64 {
    0
}

/// Waits (bounded) until `bytes` fit beside what the other tests hold, then counts them.
pub(super) fn admit(bytes: usize) {
    let held = HELD.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut held, _) = RELEASED
        .wait_timeout_while(held, ADMISSION_WAIT, |held| {
            *held != 0 && held.saturating_add(bytes) > TEST_DEVICE_BYTES
        })
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *held = held.saturating_add(bytes);
}

/// Returns `bytes` to the tests' total.
pub(super) fn release(bytes: usize) {
    let mut held = HELD.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *held = held.saturating_sub(bytes);
    drop(held);
    RELEASED.notify_all();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn each_thread_has_its_own_pool_ledger() {
        let here = pool() as *const AtomicUsize as usize;
        let there = std::thread::spawn(|| pool() as *const AtomicUsize as usize).join().unwrap();
        assert_ne!(here, there);
        assert_eq!(pool().load(Ordering::Acquire), 0, "a fresh test thread starts empty");
    }

    #[test]
    fn a_reservation_waits_for_room_and_goes_ahead_when_it_is_released() {
        // Each thread admits half the bound; the third waits until one releases.
        let half = TEST_DEVICE_BYTES / 2;
        admit(half);
        let started = std::time::Instant::now();
        let waiter = std::thread::spawn(move || {
            admit(half);
            admit(half);
            let waited = started.elapsed();
            release(half);
            release(half);
            waited
        });
        std::thread::sleep(Duration::from_millis(200));
        release(half);
        assert!(waiter.join().unwrap() >= Duration::from_millis(150));
    }
}
