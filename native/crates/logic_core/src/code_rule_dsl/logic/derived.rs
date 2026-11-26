//! Derived values kept in memory for a run: 03 §14's derivations, on the thread that computes them.
//!
//! A derived value is named by what it is (`"join spec"`) and a digest of every input it reads. A later request
//! with the same name and digest gets the kept value; any changed input changes the digest, so a kept value is never
//! stale and nothing has to be invalidated. This is `content_store::derivation`'s rule (names bound to digests,
//! validity by the inputs), for values that live only as long as a corpus run: a chunk's join specs, which the
//! demand split rebuilds for each chunk's program from the same rules and plans.
//!
//! ```text
//! let spec = derived::value("join spec", |d| { d.str(&rule); d.u64(at) /* every input */ }, || build());
//! ```
//!
//! Kept while a `Keep` is held (`corpus::run_with_options`, with the kept plans, decision 0038); outside one every
//! value is computed. The digest is 128 bits: two foldhash hashers with independent random seeds (SipHash was a
//! tenth of a session run's planning).

use std::any::Any;
use std::cell::RefCell;
use std::hash::{BuildHasher, Hasher};

use foldhash::quality::RandomState;
use std::sync::Arc;

use rustc_hash::FxHashMap;

/// At most this many values are kept; past it the memo starts again (a run's distinct specs number in the
/// thousands).
const MOST: usize = 1 << 16;

pub(in crate::code_rule_dsl) struct Memo {
    keys: (RandomState, RandomState),
    values: FxHashMap<(&'static str, u128), Arc<dyn Any + Send + Sync>>,
    /// Where what it keeps is made (`placed`): its own free-listed blocks (decision 0342 amended), not the stack of
    /// whichever thread planned it, which a value kept for the run held under everything made after it.
    room: Arc<dyn rerail_blocks::maps::Room>,
}

/// The memo of a run, shared by the threads its strata run on (decision 0113): a stratum's job enters it (`enter`).
pub(in crate::code_rule_dsl) type Shared = Arc<parking_lot::Mutex<Memo>>;

/// The run's memo, in the run's context: what a job derives is the run's (decision 0442: the context goes with it).
#[derive(Clone, Default)]
struct Memoized(Option<Shared>);

/// The run's memo (`Memoized`).
pub(in crate::code_rule_dsl) fn share() -> Option<Shared> {
    crate::code_rule_dsl::logic::profile::bound::<Memoized>().0
}

/// Derived values kept for the run while held, none at its start: a session's run derives its own, as a new
/// process's does (t33 step 3).
pub struct Keep(#[allow(dead_code)] crate::code_rule_dsl::logic::profile::Bound);

impl Keep {
    pub fn start() -> Self {
        Keep(crate::code_rule_dsl::logic::profile::bind(Memoized(Some(Arc::new(parking_lot::Mutex::new(Memo { keys: (RandomState::default(), RandomState::default()), values: FxHashMap::default(), room: rerail_blocks::maps::FreeRoom::owner() }))))))
    }
}

/// The digest of a value's inputs, written one by one. Each piece is length- or tag-delimited, so two different
/// input lists never write the same bytes.
pub struct Inputs {
    a: foldhash::quality::FoldHasher<'static>,
    b: foldhash::quality::FoldHasher<'static>,
    /// Bytes written: the inputs' size, charged as `Byte` work.
    bytes: u64,
}

impl Inputs {
    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }

    pub(in crate::code_rule_dsl) fn bytes(&mut self, s: &[u8]) {
        self.u64(s.len() as u64);
        self.a.write(s);
        self.b.write(s);
        self.bytes += s.len() as u64;
    }

    pub fn u64(&mut self, n: u64) {
        self.a.write_u64(n);
        self.b.write_u64(n);
        self.bytes += 8;
    }

    fn finish(&self) -> u128 {
        (u128::from(self.a.finish()) << 64) | u128::from(self.b.finish())
    }

    /// The digest of what `inputs` writes, under the hashers `keys`, charged as `Byte` work.
    fn over<S: BuildHasher<Hasher = foldhash::quality::FoldHasher<'static>>>(keys: &(S, S), inputs: impl FnOnce(&mut Inputs)) -> u128 {
        let mut d = Inputs { a: keys.0.build_hasher(), b: keys.1.build_hasher(), bytes: 0 };
        inputs(&mut d);
        super::profile::work(super::profile::Unit::Byte, d.bytes);
        d.finish()
    }
}

/// The digest of what `inputs` writes, under fixed keys: a key that outlives one run and one process (a session's
/// kept plans, and the plans the store keeps, `plan::PlanKey`, decision 0116 step 2). The engine's build is in
/// whatever keeps it, so the hasher's own version cannot change under a kept key. Two different input lists
/// collide with odds of about 2^-128.
pub fn digest(inputs: impl FnOnce(&mut Inputs)) -> u128 {
    const KEYS: (foldhash::quality::FixedState, foldhash::quality::FixedState) = (foldhash::quality::FixedState::with_seed(0x7265_7261_696c_2d61), foldhash::quality::FixedState::with_seed(0x7265_7261_696c_2d62));
    Inputs::over(&KEYS, inputs)
}

/// `f`, what it makes with no room made in the run's memo's room, where the memo keeps it (a kept value's section,
/// `join_spec`'s): `value` gives its maker no room, so a value's temporaries stay on the thread's stack and only what
/// is kept lies in the memo's blocks. Outside a run's memo, `f` as it is.
pub fn placed<R>(f: impl FnOnce() -> R) -> R {
    match share() {
        Some(memo) => {
            let room = Arc::clone(&memo.lock().room);
            rerail_blocks::maps::within(&room, f)
        }
        None => f(),
    }
}

/// The value `name` over the inputs `inputs` writes: kept from an earlier request with the same digest, or `make`'s.
/// Counted per name as a hit or a miss in the profile's decisions ("derived values").
pub fn value<V: Clone + Send + Sync + 'static>(name: &'static str, inputs: impl FnOnce(&mut Inputs), make: impl FnOnce() -> V) -> V {
    let memo = share();
    let digest = memo.as_ref().map(|m| {
        let keys = m.lock().keys.clone();
        Inputs::over(&keys, |d| {
            d.str(name);
            inputs(d);
        })
    });
    let (Some(digest), Some(memo)) = (digest, memo) else {
        return make();
    };
    let kept = memo.lock().values.get(&(name, digest)).cloned();
    if let Some(v) = kept.and_then(|v| v.downcast_ref::<V>().cloned()) {
        super::profile::decision("derived values", || format!("{name}: kept"));
        return v;
    }
    super::profile::decision("derived values", || format!("{name}: made"));
    let v = make();
    let mut m = memo.lock();
    if m.values.len() >= MOST {
        m.values.clear();
    }
    m.values.insert((name, digest), Arc::new(v.clone()));
    drop(m);
    v
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// What the run's memo keeps is made in its room (`placed`, decision 0342 amended), not on the stack of the thread
    /// that planned it; outside a run's memo, where a structure is made as ever.
    #[test]
    fn a_kept_value_is_made_in_the_memos_room() {
        use rerail_blocks::maps::{default_room, place_of};
        let block = |r: &rerail_blocks::maps::Region| place_of(r).map(|(b, _)| Arc::as_ptr(b) as usize);
        let outside = placed(|| default_room().region(64, 8, "a test's"));
        let _keep = Keep::start();
        let (a, b) = (placed(|| default_room().region(64, 8, "a test's")), placed(|| default_room().region(100, 8, "a test's")));
        let elsewhere = default_room().region(64, 8, "a test's");
        assert!(block(&a).is_some() && block(&a) == block(&b), "the memo's block holds both");
        assert_ne!(block(&a), block(&elsewhere), "a structure made outside `placed` is not in the memo's room");
        assert_ne!(block(&a), block(&outside), "outside a memo, `placed` gives no room");
    }

    #[test]
    fn a_value_is_kept_by_its_inputs_and_made_again_when_one_changes() {
        let made = std::cell::Cell::new(0);
        let get = |x: &str| {
            value("test", |d| d.str(x), || {
                made.set(made.get() + 1);
                format!("<{x}>")
            })
        };
        // Outside a run, every request makes the value.
        assert_eq!(get("a"), "<a>");
        assert_eq!(get("a"), "<a>");
        assert_eq!(made.get(), 2);
        let _keep = Keep::start();
        assert_eq!(get("a"), "<a>");
        assert_eq!(get("a"), "<a>");
        assert_eq!(made.get(), 3);
        assert_eq!(get("b"), "<b>");
        assert_eq!(made.get(), 4);
        // Length-delimited: ("ab", "") and ("a", "b") are different inputs.
        let two = |x: &str, y: &str| value("pair", |d| { d.str(x); d.str(y) }, || format!("{x}|{y}"));
        assert_eq!(two("ab", ""), "ab|");
        assert_eq!(two("a", "b"), "a|b");
    }
}
