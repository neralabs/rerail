//! Optional join acceleration (the Bloom screens, the failed-probe caches) shares one allowance across the walks and
//! morsels of a machine: the machine's (decision 0442), not the process's.
use rerail_blocks::vector::Vector;
use super::FACTOR_MAX;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

const LIMIT: usize = 4 * 1024 * 1024;

/// The bytes the machine's optional join acceleration holds.
#[derive(Default)]
pub(super) struct Allowance(AtomicUsize);

pub(super) struct Reservation {
    used: Arc<Allowance>,
    bytes: usize,
}

impl Reservation {
    /// `bytes` of the caller's machine's allowance, or none when they would pass it.
    pub(super) fn take(bytes: usize) -> Option<Self> {
        Self::from(crate::code_rule_dsl::logic::profile::machine::<Allowance>(), LIMIT, bytes)
    }

    fn from(used: Arc<Allowance>, limit: usize, bytes: usize) -> Option<Self> {
        let room = limit.checked_sub(bytes)?;
        used.0.try_update(Relaxed, Relaxed, |n| (n <= room).then(|| n + bytes)).ok()?;
        Some(Self { used, bytes })
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.used.0.fetch_sub(self.bytes, Relaxed);
    }
}

/// A collision evicts the old entry. Compare the entire key and the probe generation on every hit.
#[derive(Default)]
pub(super) struct Failures {
    words: Vector<u64>,
    reservation: Option<Reservation>,
    epoch: u64,
    buckets: usize,
    width: usize,
    disabled: bool,
}

impl Failures {
    pub(super) fn begin_probe(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.words.fill(0);
            self.epoch = 1;
        }
    }

    fn bucket(&self, key: &[i64; FACTOR_MAX]) -> usize {
        use std::hash::Hasher;
        let mut hash = rustc_hash::FxHasher::default();
        for &value in key {
            hash.write_i64(value);
        }
        (hash.finish() as usize & (self.buckets - 1)) * self.width
    }

    pub(super) fn get(&self, key: &[i64; FACTOR_MAX]) -> Option<&[u64]> {
        if self.words.is_empty() {
            return None;
        }
        let start = self.bucket(key);
        let cell = &self.words[start..start + self.width];
        (cell[0] == self.epoch
            && cell[1..=FACTOR_MAX]
                .iter()
                .zip(key)
                .all(|(&a, &b)| a == b as u64))
        .then(|| &cell[1 + FACTOR_MAX..])
    }

    pub(super) fn record(
        &mut self,
        key: &[i64; FACTOR_MAX],
        count: usize,
        counts: impl Iterator<Item = u64>,
    ) {
        if self.disabled {
            return;
        }
        if self.words.is_empty() {
            self.width = 1 + FACTOR_MAX + count;
            let capacity = (2048 / self.width).min(256);
            if capacity == 0 {
                self.disabled = true;
                return;
            }
            self.buckets = 1 << capacity.ilog2();
            let len = self.buckets * self.width;
            let Some(permit) = Reservation::take(len * 8) else {
                self.disabled = true;
                return;
            };
            self.words = rerail_blocks::vector![0; len];
            self.reservation = Some(permit);
        }
        let start = self.bucket(key);
        let cell = &mut self.words[start..start + self.width];
        cell[0] = self.epoch;
        for (word, &value) in cell[1..=FACTOR_MAX].iter_mut().zip(key) {
            *word = value as u64;
        }
        for (word, n) in cell[1 + FACTOR_MAX..].iter_mut().zip(counts) {
            *word = n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_reservations_cannot_exceed_the_allowance() {
        let used = Arc::new(Allowance::default());
        let barrier = std::sync::Barrier::new(8);
        let admitted = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    let held = Reservation::from(Arc::clone(&used), 24, 8);
                    if held.is_some() {
                        admitted.fetch_add(1, Relaxed);
                    }
                    barrier.wait();
                    drop(held);
                });
            }
        });
        assert_eq!(admitted.load(Relaxed), 3);
        assert_eq!(used.0.load(Relaxed), 0);
        assert!(Reservation::from(used, 24, usize::MAX).is_none());
    }

    #[test]
    fn full_cache_evicts_safely_and_new_probe_forgets_failures() {
        let mut failures = Failures::default();
        failures.begin_probe();
        for i in 0..4096 {
            let key = [i, -1, i64::MAX, i64::MIN];
            failures.record(&key, 2, [17, i as u64].into_iter());
            assert_eq!(failures.get(&key), Some([17, i as u64].as_slice()));
        }
        assert!(failures.words.len() * 8 <= 16384);
        let hits = (0..4096)
            .filter(|&i| {
                failures.get(&[i, -1, i64::MAX, i64::MIN]).is_some_and(|c| {
                    assert_eq!(c, [17, i as u64]);
                    true
                })
            })
            .count();
        assert!((1..=256).contains(&hits));
        failures.begin_probe();
        assert!(failures.get(&[4095, -1, i64::MAX, i64::MIN]).is_none());
    }
}
