//! One ledger for a session's owner, workers, failed executions, and recovery SQL.
use parking_lot::Mutex;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

pub mod observer;
// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)

#[derive(Default)]
struct State {
    used: usize,
    reserved: usize,
    limit: Option<usize>,
    stopped: bool,
    recovery: usize,
    listeners: Vec<std::sync::Weak<Mutex<Vec<Receipt>>>>,
}
#[derive(Default)]
pub struct Ledger {
    state: Mutex<State>,
    measure_vm_steps: AtomicBool,
    vm_steps: AtomicU64,
}
thread_local! {static CURRENT:RefCell<Option<Arc<Ledger>>>=const{RefCell::new(None)};}
pub fn current() -> Option<Arc<Ledger>> {
    CURRENT.with(|v| v.borrow().clone())
}
fn available(s: &State) -> usize {
    s.limit.map_or(usize::MAX, |limit| {
        limit.saturating_sub(
            s.used
                .saturating_add(s.reserved)
                .saturating_add(s.recovery)
                .saturating_add(1),
        )
    })
}
#[derive(Clone)]
pub struct Receipt {
    pub sql: String,
    pub counters: [u64; 4],
}
impl Ledger {
    pub fn measure(&self) -> Measurement<'_> {
        self.measure_vm_steps.store(true, Relaxed);
        Measurement(self)
    }
    pub fn counters(&self) -> (usize, u64) {
        (self.state.lock().used, self.vm_steps.load(Relaxed))
    }
    pub fn credits(&self) -> Option<usize> {
        let s = self.state.lock();
        s.limit.map(|_| if s.stopped { 0 } else { available(&s) })
    }
    pub fn receipts(&self) -> Arc<Mutex<Vec<Receipt>>> {
        let rows = Arc::new(Mutex::new(Vec::new()));
        let mut state = self.state.lock();
        state.listeners.retain(|w| w.strong_count() > 0);
        state.listeners.push(Arc::downgrade(&rows));
        rows
    }
    fn listening(&self) -> bool {
        self.state.lock().listeners.iter().any(|w| w.strong_count() > 0)
    }
    fn record(&self, row: Receipt) {
        let listeners = self
            .state
            .lock()
            .listeners
            .iter()
            .filter_map(|w| w.upgrade())
            .collect::<Vec<_>>();
        for listener in listeners {
            listener.lock().push(row.clone());
        }
    }
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    pub fn configure(&self, limit: Option<usize>) -> Result<(), String> {
        let mut s = self.state.lock();
        if limit.is_some_and(|n| n < s.used + s.reserved) {
            return Err(format!(
                "SQL session already used {} submissions and reserved {}; cannot set a smaller allowance",
                s.used, s.reserved
            ));
        }
        s.limit = limit;
        s.stopped = false;
        Ok(())
    }
    pub fn exhausted(&self) -> bool {
        self.state.lock().stopped
    }
    pub fn available(&self) -> usize {
        let s = self.state.lock();
        if s.stopped { 0 } else { available(&s) }
    }
    /// Preflight owner operations that cannot roll back their native schema changes.
    pub fn require_owner(&self, statements: impl FnOnce() -> usize) -> Result<(), String> {
        let s = self.state.lock();
        if s.limit.is_none() {
            return Ok(());
        }
        let count = statements();
        let remaining = if s.stopped { 0 } else { available(&s) };
        if count > remaining {
            return Err(format!(
                "SQL session submission allowance: owner operation needs up to {count} statements; {remaining} credits remain"
            ));
        }
        Ok(())
    }
    pub fn snapshot(&self) -> serde_json::Value {
        let s = self.state.lock();
        serde_json::json!({"limit":s.limit,"executed":s.used,"reserved":s.reserved,
            "recovery_reserve":s.recovery,"available":s.limit.map(|_|if s.stopped{0}else{available(&s)}),"exhausted":s.stopped,
            "policy":"Session lifetime; owner and workers share credits; failed executions and recovery count; diagnostic EXPLAIN does not execute work"})
    }
    pub fn enter(self: &Arc<Self>, name: &str) -> Result<Scope, String> {
        let metadata = name == "explain";
        let recovery =
            if matches!(name, "query" | "query_plan" | "verification snapshot") || metadata {
                0
            } else {
                8
            };
        let previous = {
            let mut s = self.state.lock();
            let previous = s.recovery;
            s.recovery = s.recovery.max(recovery);
            if !metadata && (s.stopped || available(&s) == 0) {
                s.recovery = previous;
                return Err(
                    "SQL session submission allowance exhausted before operation admission".into(),
                );
            }
            previous
        };
        let prior = CURRENT.with(|v| v.replace(Some(self.clone())));
        Ok(Scope {
            ledger: self.clone(),
            previous,
            prior,
        })
    }
    pub fn reserve(self: &Arc<Self>, count: usize) -> Result<Arc<Reservation>, String> {
        let mut s = self.state.lock();
        if s.stopped || count > available(&s) {
            return Err(format!(
                "SQL work plan needs {count} submissions; {} session credits remain",
                available(&s)
            ));
        }
        s.reserved = s
            .reserved
            .checked_add(count)
            .ok_or("SQL reservation overflow")?;
        Ok(Arc::new(Reservation {
            ledger: self.clone(),
            remaining: Mutex::new(count),
        }))
    }
    pub fn charge(&self, recovery: bool, reserved: bool) -> bool {
        let mut s = self.state.lock();
        if reserved {
            s.reserved = s.reserved.saturating_sub(1);
        }
        s.used = s.used.saturating_add(1);
        let Some(limit) = s.limit else {
            return true;
        };
        if reserved || recovery && s.stopped {
            return s.used + s.reserved <= limit;
        }
        if !s.stopped && s.used + s.reserved + s.recovery < limit {
            true
        } else {
            s.stopped = true;
            false
        }
    }
}
pub struct Measurement<'a>(&'a Ledger);
impl Drop for Measurement<'_> {
    fn drop(&mut self) {
        self.0.measure_vm_steps.store(false, Relaxed);
    }
}
pub struct Scope {
    ledger: Arc<Ledger>,
    previous: usize,
    prior: Option<Arc<Ledger>>,
}
impl Drop for Scope {
    fn drop(&mut self) {
        self.ledger.state.lock().recovery = self.previous;
        CURRENT.with(|v| v.replace(self.prior.take()));
    }
}
pub struct Reservation {
    ledger: Arc<Ledger>,
    remaining: Mutex<usize>,
}
impl Reservation {
    pub fn ledger(&self) -> Arc<Ledger> {
        self.ledger.clone()
    }

    pub fn charge(&self) -> bool {
        let mut count = self.remaining.lock();
        if *count == 0 {
            return self.ledger.charge(false, false);
        }
        *count -= 1;
        self.ledger.charge(false, true)
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let count = *self.remaining.get_mut();
        self.ledger.state.lock().reserved -= count;
    }
}
