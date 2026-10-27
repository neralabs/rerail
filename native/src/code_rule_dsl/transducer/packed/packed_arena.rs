use std::marker::PhantomData;
use std::sync::atomic::{AtomicU32, Ordering};

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ArenaOffset<Tag> {
    value: u32,
    marker: PhantomData<fn() -> Tag>,
}

impl<Tag> ArenaOffset<Tag> {
    const fn new(value: u32) -> Self {
        Self {
            value,
            marker: PhantomData,
        }
    }

    pub(super) const fn get(&self) -> u32 {
        self.value
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ArenaRange<Tag> {
    pub(super) start: ArenaOffset<Tag>,
    pub(super) capacity: u32,
}

pub(super) enum PendingTag {}
pub(super) enum CaptureTag {}
pub(super) enum QueryStateTag {}
pub(super) enum ResultTag {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SharedReservationError {
    EvaluationCapacity,
    GlobalPlan,
}

pub(super) struct EvaluationQuota<Tag> {
    limit: u32,
    used: AtomicU32,
    marker: PhantomData<fn() -> Tag>,
}

impl<Tag> EvaluationQuota<Tag> {
    pub(super) const fn new(limit: u32) -> Self {
        Self {
            limit,
            used: AtomicU32::new(0),
            marker: PhantomData,
        }
    }

    fn reserve(&self, count: u32) -> Result<(), SharedReservationError> {
        reserve_atomic(&self.used, self.limit, count)
            .map(|_| ())
            .map_err(|_| SharedReservationError::EvaluationCapacity)
    }

    pub(super) fn used(&self) -> u32 {
        self.used.load(Ordering::Relaxed)
    }
}

pub(super) struct GlobalArenaCursor<Tag> {
    range: ArenaRange<Tag>,
    used: AtomicU32,
}

impl<Tag> GlobalArenaCursor<Tag> {
    pub(super) const fn new(range: ArenaRange<Tag>) -> Self {
        Self {
            range,
            used: AtomicU32::new(0),
        }
    }

    pub(super) fn reserve(
        &self,
        quota: &EvaluationQuota<Tag>,
        count: u32,
    ) -> Result<ArenaOffset<Tag>, SharedReservationError> {
        quota.reserve(count)?;
        self.reserve_physical(count)
    }

    fn reserve_physical(&self, count: u32) -> Result<ArenaOffset<Tag>, SharedReservationError> {
        let relative = reserve_atomic(&self.used, self.range.capacity, count)
            .map_err(|_| SharedReservationError::GlobalPlan)?;
        self.range
            .start
            .get()
            .checked_add(relative)
            .map(ArenaOffset::new)
            .ok_or(SharedReservationError::GlobalPlan)
    }

    pub(super) fn used(&self) -> u32 {
        self.used.load(Ordering::Relaxed)
    }
}

pub(super) struct LazyArenaBlock<Tag> {
    limit: u32,
    state: AtomicU32,
    marker: PhantomData<fn() -> Tag>,
}

impl<Tag> LazyArenaBlock<Tag> {
    pub(super) const fn new(limit: u32) -> Self {
        Self {
            limit,
            state: AtomicU32::new(0),
            marker: PhantomData,
        }
    }

    pub(super) fn ensure(
        &self,
        global: &GlobalArenaCursor<Tag>,
    ) -> Result<ArenaRange<Tag>, SharedReservationError> {
        loop {
            match self.state.load(Ordering::Relaxed) {
                0 => {
                    if self
                        .state
                        .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
                        .is_err()
                    {
                        continue;
                    }
                    match global.reserve_physical(self.limit) {
                        Ok(start) if start.get() <= u32::MAX - 2 => {
                            self.state.store(start.get() + 2, Ordering::Relaxed);
                            return Ok(ArenaRange {
                                start,
                                capacity: self.limit,
                            });
                        }
                        Ok(_) | Err(_) => {
                            self.state.store(u32::MAX, Ordering::Relaxed);
                            return Err(SharedReservationError::GlobalPlan);
                        }
                    }
                }
                1 => std::hint::spin_loop(),
                u32::MAX => return Err(SharedReservationError::GlobalPlan),
                ready => {
                    return Ok(ArenaRange {
                        start: ArenaOffset::new(ready - 2),
                        capacity: self.limit,
                    });
                }
            }
        }
    }
}

pub(super) struct TaskArenaChunk<Tag> {
    next: u32,
    end: u32,
    marker: PhantomData<fn() -> Tag>,
}

impl<Tag> Default for TaskArenaChunk<Tag> {
    fn default() -> Self {
        Self {
            next: 0,
            end: 0,
            marker: PhantomData,
        }
    }
}

impl<Tag> TaskArenaChunk<Tag> {
    pub(super) fn reserve_one(
        &mut self,
        quota: &EvaluationQuota<Tag>,
        global: &GlobalArenaCursor<Tag>,
        chunk_size: u32,
    ) -> Result<ArenaOffset<Tag>, SharedReservationError> {
        if chunk_size == 0 {
            return Err(SharedReservationError::GlobalPlan);
        }
        quota.reserve(1)?;
        if self.next == self.end {
            let start = global.reserve_physical(chunk_size)?.get();
            self.next = start;
            self.end = start
                .checked_add(chunk_size)
                .ok_or(SharedReservationError::GlobalPlan)?;
        }
        let offset = ArenaOffset::new(self.next);
        self.next += 1;
        Ok(offset)
    }
}

fn reserve_atomic(cursor: &AtomicU32, capacity: u32, count: u32) -> Result<u32, ()> {
    let mut old = cursor.load(Ordering::Relaxed);
    loop {
        if old > capacity || count > capacity - old {
            return Err(());
        }
        let new = old + count;
        match cursor.compare_exchange_weak(old, new, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return Ok(old),
            Err(current) => old = current,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_reservation_enforces_evaluation_quota_before_global_capacity() {
        let global = GlobalArenaCursor::new(ArenaRange::<ResultTag> {
            start: ArenaOffset::new(10),
            capacity: 8,
        });
        let first = EvaluationQuota::new(2);
        let second = EvaluationQuota::new(3);
        assert_eq!(global.reserve(&first, 2).unwrap().get(), 10);
        assert!(matches!(
            global.reserve(&first, 1),
            Err(SharedReservationError::EvaluationCapacity)
        ));
        assert_eq!(global.reserve(&second, 3).unwrap().get(), 12);
        assert_eq!(first.used(), 2);
        assert_eq!(second.used(), 3);
        assert_eq!(global.used(), 5);
    }

    #[test]
    fn shared_reservation_reports_impossible_global_plan() {
        let global = GlobalArenaCursor::new(ArenaRange::<CaptureTag> {
            start: ArenaOffset::new(0),
            capacity: 1,
        });
        let quota = EvaluationQuota::new(2);
        assert_eq!(global.reserve(&quota, 1).unwrap().get(), 0);
        assert!(matches!(
            global.reserve(&quota, 1),
            Err(SharedReservationError::GlobalPlan)
        ));
        assert_eq!(quota.used(), 2);
    }

    #[test]
    fn task_chunks_keep_physical_records_compact_and_quota_exact() {
        let global = GlobalArenaCursor::new(ArenaRange::<PendingTag> {
            start: ArenaOffset::new(100),
            capacity: 8,
        });
        let quota = EvaluationQuota::new(5);
        let mut task = TaskArenaChunk::default();
        let offsets = (0..5)
            .map(|_| task.reserve_one(&quota, &global, 4).unwrap().get())
            .collect::<Vec<_>>();
        assert_eq!(offsets, [100, 101, 102, 103, 104]);
        assert_eq!(quota.used(), 5);
        assert_eq!(global.used(), 8);
    }

    #[test]
    fn lazy_block_is_allocated_once_for_concurrent_evaluation_tasks() {
        use std::sync::Arc;

        let global = Arc::new(GlobalArenaCursor::new(ArenaRange::<QueryStateTag> {
            start: ArenaOffset::new(20),
            capacity: 64,
        }));
        let block = Arc::new(LazyArenaBlock::new(32));
        let threads = (0..8)
            .map(|_| {
                let global = Arc::clone(&global);
                let block = Arc::clone(&block);
                std::thread::spawn(move || block.ensure(&global).unwrap())
            })
            .collect::<Vec<_>>();
        for thread in threads {
            let range = thread.join().unwrap();
            assert_eq!(range.start.get(), 20);
            assert_eq!(range.capacity, 32);
        }
        assert_eq!(global.used(), 32);
    }

    #[test]
    fn concurrent_tasks_share_one_global_typed_arena_without_overlap() {
        use std::sync::Arc;

        let global = Arc::new(GlobalArenaCursor::new(ArenaRange::<ResultTag> {
            start: ArenaOffset::new(0),
            capacity: 4_000,
        }));
        let quotas = (0..4)
            .map(|_| Arc::new(EvaluationQuota::new(1_000)))
            .collect::<Vec<_>>();
        let mut threads = Vec::new();
        for quota in quotas {
            let global = Arc::clone(&global);
            threads.push(std::thread::spawn(move || {
                (0..1_000)
                    .map(|_| global.reserve(&quota, 1).unwrap().get())
                    .collect::<Vec<_>>()
            }));
        }
        let mut offsets = threads
            .into_iter()
            .flat_map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        offsets.sort_unstable();
        assert_eq!(offsets, (0..4_000).collect::<Vec<_>>());
        assert_eq!(global.used(), 4_000);
    }
}
