//! The residency of the directory's slabs (P§11.4: sources and tapes "leased; never versioned", pure answers "aged
//! out by lease"). Each slab is held on a lease, renewed by every use (`hold`, `touch`): it lapses `DROP_AFTER` after
//! the slab's last use (`rerail_accel::budget::DROP_AFTER`, one definition with `rerail_slabs`), and the slab is then
//! aged out (`age_out`), by a thread of the residency's own at the earliest lapse, which ends with the residency. Past
//! the budget (`rerail_accel::budget::resident`, the residency's share of the Metal budget), the least recently used
//! slab no reader holds is retired early, to make room. A slab aged out or retired goes when its last reader lets go;
//! its documents are loaded again when asked, from the store's objects, which hold them (`files::World`): the content
//! hit outlives the lease.
//!
//! Its state is shared with that thread, so it is behind a lock, as `rerail_slabs`'s is. It holds slabs, not claims:
//! a block's writes stay its one writer's (`dir`).

use crate::dir::BlockId;
use rerail_blocks::block::Block;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// A slab held: its id, the residency's hold on it, its last use by the clock (the least recently used is retired
/// first) and when (its lease runs from it).
struct Held {
    id: BlockId,
    slab: Arc<Block>,
    clock: u64,
    used: Instant,
}

#[derive(Default)]
struct State {
    slabs: Vec<Held>,
    clock: u64,
    /// The lease (`None`: `DROP_AFTER`; a test's otherwise).
    lease: Option<Duration>,
    /// Whether the thread that ages slabs out runs.
    ager: bool,
    /// The slabs retired past the budget, and those aged out.
    retired: u64,
    aged: u64,
}

impl State {
    fn lease(&self) -> Duration {
        self.lease.unwrap_or(rerail_accel::budget::DROP_AFTER)
    }

    /// Ages out every slab whose lease lapsed by `now`: the count.
    fn age_out(&mut self, now: Instant) -> u64 {
        let lease = self.lease();
        let before = self.slabs.len();
        self.slabs.retain(|h| now < h.used + lease);
        let n = (before - self.slabs.len()) as u64;
        self.aged += n;
        n
    }
}

type Shared = Arc<(Mutex<State>, Condvar)>;

fn lock(r: &Shared) -> MutexGuard<'_, State> {
    r.0.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The slabs resident, their budget and their leases.
#[derive(Default)]
pub struct Residency {
    shared: Shared,
    /// The budget (0: `rerail_accel::budget::resident`): a test's.
    budget: usize,
}

impl Residency {
    /// The residency with a budget of `budget` bytes and leases of `lease` (a test's), holding what it holds.
    pub fn limited(self, budget: usize, lease: Duration) -> Residency {
        lock(&self.shared).lease = Some(lease);
        self.shared.1.notify_all();
        Residency { budget, ..self }
    }

    /// Holds slab `id` on a lease from now: the residency's, not the directory's; aged out, or retired, it goes once
    /// no reader holds it.
    pub fn hold(&self, id: BlockId, slab: Arc<Block>) {
        let mut r = lock(&self.shared);
        r.clock += 1;
        let clock = r.clock;
        r.slabs.push(Held { id, slab, clock, used: Instant::now() });
        let start = !r.ager;
        r.ager = true;
        drop(r);
        self.shared.1.notify_all();
        if start {
            self.ager();
        }
    }

    /// The thread that ages slabs out: woken at the earliest lapse, or every second to see whether the residency is
    /// gone.
    fn ager(&self) {
        let weak = Arc::downgrade(&self.shared);
        let _ = std::thread::Builder::new().name("rerail-delta-lease".into()).spawn(move || {
            while let Some(r) = weak.upgrade() {
                let mut st = lock(&r);
                let now = Instant::now();
                st.age_out(now);
                let lease = st.lease();
                let next = st.slabs.iter().map(|h| h.used + lease).min();
                let wait = next.map_or(Duration::from_secs(1), |t| t.saturating_duration_since(now)).min(Duration::from_secs(1));
                drop(r.1.wait_timeout(st, wait).unwrap_or_else(PoisonError::into_inner));
            }
        });
    }

    /// Room for a slab of `bytes` within the budget (`State::room` in `rerail_slabs`): while what is held and it pass
    /// the budget, the least recently used slab no reader holds is retired. Refused when none can be.
    pub fn room_for(&self, bytes: usize) -> Result<(), String> {
        let limit = if self.budget == 0 { rerail_accel::budget::resident() } else { self.budget };
        let mut r = lock(&self.shared);
        loop {
            let held: usize = r.slabs.iter().map(|s| s.slab.reserved()).sum();
            if held + bytes <= limit {
                return Ok(());
            }
            // Held by none but the residency: a launch or a reference reading it keeps it.
            let Some(at) = r.slabs.iter().enumerate().filter(|(_, s)| Arc::strong_count(&s.slab) == 1).min_by_key(|(_, s)| s.clock).map(|(at, _)| at) else {
                return Err(format!("no room for a slab of {bytes} bytes: {held} held of {limit}, none to retire"));
            };
            r.slabs.remove(at);
            r.retired += 1;
        }
    }

    /// Slab `id` used now (a document in it asked for): its lease renewed, and the last the residency retires.
    pub fn touch(&self, id: BlockId) {
        let mut r = lock(&self.shared);
        r.clock += 1;
        let clock = r.clock;
        if let Some(s) = r.slabs.iter_mut().find(|s| s.id == id) {
            s.clock = clock;
            s.used = Instant::now();
        }
    }

    /// Ages out every slab whose lease lapsed by `now` (what the residency's thread does at each lapse): the count.
    pub fn age_out(&self, now: Instant) -> u64 {
        lock(&self.shared).age_out(now)
    }

    /// Its counts: slabs retired past the budget, and aged out.
    pub fn counts(&self) -> (u64, u64) {
        let r = lock(&self.shared);
        (r.retired, r.aged)
    }
}
