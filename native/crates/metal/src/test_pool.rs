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
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// What the tests' Metal buffers may hold at once: the engine's 1 GB RAM budget.
const TEST_DEVICE_BYTES: usize = 1 << 30;
/// The longest a reservation waits for room.
const ADMISSION_WAIT: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Ledgers {
    pool: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}

thread_local! {
    // A reservation can outlive its thread (a cached buffer, or one dropped elsewhere): it holds the ledger it was
    // charged to, and releases into it.
    static LEDGERS: Ledgers = Ledgers::default();
}

static HELD: Mutex<usize> = Mutex::new(0);
static RELEASED: Condvar = Condvar::new();

/// This thread's pool ledger; on a thread whose locals are being torn down (a lease dropped by another local's
/// drop as the thread exits), an empty one: the thread holds nothing more to count.
pub fn pool() -> Arc<AtomicUsize> {
    LEDGERS.try_with(|ledgers| Arc::clone(&ledgers.pool)).unwrap_or_default()
}

/// The most this thread's pool ledger has held.
pub fn pool_peak() -> Arc<AtomicUsize> {
    LEDGERS.try_with(|ledgers| Arc::clone(&ledgers.peak)).unwrap_or_default()
}

thread_local! {
    /// A test of the driver's figure itself (`measure_driver`): this thread reads the device's.
    static MEASURED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// A test's working set for the device (`set_working_set`).
    static WORKING_SET: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Left out in a test build: the driver's figure is the process's, every test's buffers. A test of what the driver's
/// figure decides reads it (`measure_driver`).
#[cfg(target_os = "macos")]
pub fn driver_bytes(device: &metal::DeviceRef) -> u64 {
    if MEASURED.get() { device.current_allocated_size() } else { 0 }
}

/// This thread reads the driver's figure (`driver_bytes`), as a production build does.
pub fn measure_driver() {
    MEASURED.set(true);
}

/// This thread's working set for the device, in place of the OS's (`runtime::working_set`).
pub fn set_working_set(bytes: Option<u64>) {
    WORKING_SET.set(bytes);
}

pub fn working_set() -> Option<u64> {
    WORKING_SET.get()
}

/// Waits (bounded) until `bytes` fit beside what the other tests hold, then counts them.
pub fn admit(bytes: usize) {
    let held = HELD.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut held, _) = RELEASED
        .wait_timeout_while(held, ADMISSION_WAIT, |held| {
            *held != 0 && held.saturating_add(bytes) > TEST_DEVICE_BYTES
        })
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *held = held.saturating_add(bytes);
}

/// Returns `bytes` to the tests' total.
pub fn release(bytes: usize) {
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
        let here = Arc::as_ptr(&pool()) as usize;
        let there = std::thread::spawn(|| Arc::as_ptr(&pool()) as usize).join().unwrap();
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
