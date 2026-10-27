//! Test observations follow each reservation until its final Metal owner retires.
use std::cell::RefCell;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::Duration;

#[derive(Default)]
struct State {
    bytes: Mutex<usize>,
    retired: Condvar,
}

thread_local! {
    static ACTIVE: RefCell<Vec<Weak<State>>> = const { RefCell::new(Vec::new()) };
}

pub(in super::super) struct BufferObservation {
    state: Arc<State>,
    _owner_thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl BufferObservation {
    pub(in super::super) fn enter() -> Self {
        let state = Arc::new(State::default());
        ACTIVE.with_borrow_mut(|active| active.push(Arc::downgrade(&state)));
        Self {
            state,
            _owner_thread: std::marker::PhantomData,
        }
    }

    pub(in super::super) fn live_bytes(&self) -> usize {
        *self.state.bytes.lock().unwrap()
    }

    /// Metal completion can precede driver resource retirement. Observe actual
    /// ownership release, with a bounded failure if the operation leaks scratch.
    pub(in super::super) fn wait_for_live_bytes(&self, expected: usize) -> usize {
        let (bytes, _) = self
            .state
            .retired
            .wait_timeout_while(
                self.state.bytes.lock().unwrap(),
                Duration::from_secs(1),
                |bytes| *bytes != expected,
            )
            .unwrap();
        *bytes
    }
}

impl Drop for BufferObservation {
    fn drop(&mut self) {
        let own = Arc::downgrade(&self.state);
        ACTIVE.with_borrow_mut(|active| active.retain(|entry| !entry.ptr_eq(&own)));
    }
}

pub(super) struct ReservationObservation {
    state: Arc<State>,
    bytes: usize,
}

pub(super) fn reserve(bytes: usize) -> Vec<ReservationObservation> {
    ACTIVE.with_borrow(|active| {
        active
            .iter()
            .filter_map(Weak::upgrade)
            .map(|state| {
                let mut live = state.bytes.lock().unwrap();
                *live = live
                    .checked_add(bytes)
                    .expect("observed allocation overflow");
                drop(live);
                ReservationObservation { state, bytes }
            })
            .collect()
    })
}

impl Drop for ReservationObservation {
    fn drop(&mut self) {
        let mut live = self.state.bytes.lock().unwrap();
        *live = live
            .checked_sub(self.bytes)
            .expect("observed reservation retired twice");
        self.state.retired.notify_all();
    }
}
