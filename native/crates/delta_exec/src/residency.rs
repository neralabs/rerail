//! The residency of the directory's slabs (`rerail_slabs`'s policy, as it is): each slab held with the clock of its
//! last use; the least recently used, of those no reader holds, retired only while what is held and a new slab pass
//! the budget (`rerail_accel::budget::resident`, the residency's share of the Metal budget, one definition with
//! `rerail_slabs`); and all of them dropped `DROP_AFTER` after the last execution (`executed`), by a thread of their
//! own, which ends with the residency. A retired slab's memory goes when its last reader lets go; its documents are
//! loaded again when asked (the store's objects hold them, `files::World`).
//!
//! Its state is shared with that thread, so it is behind a lock, as `rerail_slabs`'s is. It holds slabs, not claims:
//! a block's writes stay its one writer's (`dir`).

use crate::dir::BlockId;
use rerail_blocks::block::Block;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

#[derive(Default)]
struct State {
    slabs: Vec<(BlockId, Arc<Block>, u64)>,
    clock: u64,
    deadline: Option<Instant>,
    /// The slabs retired, and the drops.
    retired: u64,
    drops: u64,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

fn lock(r: &Shared) -> MutexGuard<'_, State> {
    r.0.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The slabs resident, their budget and their drop.
#[derive(Default)]
pub struct Residency {
    shared: Shared,
    /// The budget (0: `rerail_accel::budget::resident`), and how long after an execution it drops all (`None`:
    /// `DROP_AFTER`): a test's.
    budget: usize,
    drop_after: Option<Duration>,
    dropper: bool,
}

impl Residency {
    /// The residency with a budget of `budget` bytes, which drops all `after` an execution (a test's), holding what it
    /// holds.
    pub fn limited(self, budget: usize, after: Duration) -> Residency {
        Residency { budget, drop_after: Some(after), ..self }
    }

    /// Holds slab `id`, used now: the residency's, not the directory's; retired, or dropped, it goes once no reader
    /// holds it.
    pub fn hold(&self, id: BlockId, slab: Arc<Block>) {
        let mut r = lock(&self.shared);
        r.clock += 1;
        let clock = r.clock;
        r.slabs.push((id, slab, clock));
    }

    /// Room for a slab of `bytes` within the budget (`State::room` in `rerail_slabs`): while what is held and it pass
    /// the budget, the least recently used slab no reader holds is retired. Refused when none can be.
    pub fn room_for(&self, bytes: usize) -> Result<(), String> {
        let limit = if self.budget == 0 { rerail_accel::budget::resident() } else { self.budget };
        let mut r = lock(&self.shared);
        loop {
            let held: usize = r.slabs.iter().map(|s| s.1.reserved()).sum();
            if held + bytes <= limit {
                return Ok(());
            }
            // Held by none but the residency: a launch or a reference reading it keeps it.
            let Some(at) = r.slabs.iter().enumerate().filter(|(_, s)| Arc::strong_count(&s.1) == 1).min_by_key(|(_, s)| s.2).map(|(at, _)| at) else {
                return Err(format!("no room for a slab of {bytes} bytes: {held} held of {limit}, none to retire"));
            };
            r.slabs.remove(at);
            r.retired += 1;
        }
    }

    /// Slab `id` used now (a document in it asked for): the last the residency retires.
    pub fn touch(&self, id: BlockId) {
        let mut r = lock(&self.shared);
        r.clock += 1;
        let clock = r.clock;
        if let Some(s) = r.slabs.iter_mut().find(|s| s.0 == id) {
            s.2 = clock;
        }
    }

    /// A full execution ended (`rerail_slabs::executed`): every slab is dropped `DROP_AFTER` from now, unless another
    /// ends first (which moves the deadline), by the residency's drop thread.
    pub fn executed(&mut self) {
        let after = self.drop_after.unwrap_or(rerail_accel::budget::DROP_AFTER);
        lock(&self.shared).deadline = Some(Instant::now() + after);
        self.shared.1.notify_all();
        if self.dropper {
            return;
        }
        self.dropper = true;
        let weak = Arc::downgrade(&self.shared);
        let _ = std::thread::Builder::new().name("rerail-delta-drop".into()).spawn(move || {
            // Woken at the deadline, or every second to see whether the residency is gone.
            while let Some(r) = weak.upgrade() {
                let mut st = lock(&r);
                let now = Instant::now();
                match st.deadline {
                    Some(d) if now >= d => {
                        st.slabs.clear();
                        st.deadline = None;
                        st.drops += 1;
                    }
                    d => {
                        let wait = d.map_or(Duration::from_secs(1), |d| (d - now).min(Duration::from_secs(1)));
                        drop(r.1.wait_timeout(st, wait).unwrap_or_else(PoisonError::into_inner));
                    }
                }
            }
        });
    }

    /// Its counts: slabs retired, and drops.
    pub fn counts(&self) -> (u64, u64) {
        let r = lock(&self.shared);
        (r.retired, r.drops)
    }
}
