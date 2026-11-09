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
use std::rc::Rc;

use rustc_hash::FxHashMap;

/// At most this many values are kept; past it the memo starts again (a run's distinct specs number in the
/// thousands).
const MOST: usize = 1 << 16;

struct Memo {
    keys: (RandomState, RandomState),
    values: FxHashMap<(&'static str, u128), Rc<dyn Any>>,
}

thread_local! {
    static MEMO: RefCell<Option<Memo>> = const { RefCell::new(None) };
}

/// Derived values kept on this thread while held; for a session's run (`across`, the session's generation), the
/// values its earlier runs kept, put back at the end (decision 0090 step 5).
pub(in crate::code_rule_dsl) struct Keep(Option<u64>);

thread_local! {
    /// A session's derived values between its runs, with the session's generation.
    static SESSION_MEMO: RefCell<Option<(u64, Memo)>> = const { RefCell::new(None) };
}

impl Keep {
    pub(in crate::code_rule_dsl) fn start(across: Option<u64>) -> Self {
        let kept = across.and_then(|g| SESSION_MEMO.with(|s| s.borrow_mut().take().filter(|(k, _)| *k == g).map(|(_, m)| m)));
        MEMO.with(|m| *m.borrow_mut() = Some(kept.unwrap_or_else(|| Memo { keys: (RandomState::default(), RandomState::default()), values: FxHashMap::default() })));
        Keep(across)
    }
}

impl Drop for Keep {
    fn drop(&mut self) {
        let memo = MEMO.with(|m| m.borrow_mut().take());
        if let (Some(g), Some(memo)) = (self.0, memo) {
            SESSION_MEMO.with(|s| *s.borrow_mut() = Some((g, memo)));
        }
    }
}

/// The digest of a value's inputs, written one by one. Each piece is length- or tag-delimited, so two different
/// input lists never write the same bytes.
pub(in crate::code_rule_dsl) struct Inputs {
    a: foldhash::quality::FoldHasher<'static>,
    b: foldhash::quality::FoldHasher<'static>,
    /// Bytes written: the inputs' size, charged as `Byte` work.
    bytes: u64,
}

impl Inputs {
    pub(in crate::code_rule_dsl) fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }

    pub(in crate::code_rule_dsl) fn bytes(&mut self, s: &[u8]) {
        self.u64(s.len() as u64);
        self.a.write(s);
        self.b.write(s);
        self.bytes += s.len() as u64;
    }

    pub(in crate::code_rule_dsl) fn u64(&mut self, n: u64) {
        self.a.write_u64(n);
        self.b.write_u64(n);
        self.bytes += 8;
    }

    fn finish(&self) -> u128 {
        (u128::from(self.a.finish()) << 64) | u128::from(self.b.finish())
    }

    /// The digest of what `inputs` writes, under the hashers `keys`, charged as `Byte` work.
    fn over(keys: &(RandomState, RandomState), inputs: impl FnOnce(&mut Inputs)) -> u128 {
        let mut d = Inputs { a: keys.0.build_hasher(), b: keys.1.build_hasher(), bytes: 0 };
        inputs(&mut d);
        super::profile::work(super::profile::Unit::Byte, d.bytes);
        d.finish()
    }
}

/// The digest of what `inputs` writes, under keys fixed for the process: a key that outlives one run (a session's
/// kept plans, `plan::PlanKey`). Two different input lists collide with odds of about 2^-128.
pub(in crate::code_rule_dsl) fn digest(inputs: impl FnOnce(&mut Inputs)) -> u128 {
    static KEYS: std::sync::OnceLock<(RandomState, RandomState)> = std::sync::OnceLock::new();
    Inputs::over(KEYS.get_or_init(|| (RandomState::default(), RandomState::default())), inputs)
}

/// The value `name` over the inputs `inputs` writes: kept from an earlier request with the same digest, or `make`'s.
/// Counted per name as a hit or a miss in the profile's decisions ("derived values").
pub(in crate::code_rule_dsl) fn value<V: Clone + 'static>(name: &'static str, inputs: impl FnOnce(&mut Inputs), make: impl FnOnce() -> V) -> V {
    let digest = MEMO.with(|m| {
        let m = m.borrow();
        let m = m.as_ref()?;
        Some(Inputs::over(&m.keys, |d| {
            d.str(name);
            inputs(d);
        }))
    });
    let Some(digest) = digest else {
        return make();
    };
    let kept = MEMO.with(|m| m.borrow().as_ref().and_then(|m| m.values.get(&(name, digest)).cloned()));
    if let Some(v) = kept.and_then(|v| v.downcast_ref::<V>().cloned()) {
        super::profile::decision("derived values", || format!("{name}: kept"));
        return v;
    }
    super::profile::decision("derived values", || format!("{name}: made"));
    let v = make();
    MEMO.with(|m| {
        if let Some(m) = m.borrow_mut().as_mut() {
            if m.values.len() >= MOST {
                m.values.clear();
            }
            m.values.insert((name, digest), Rc::new(v.clone()));
        }
    });
    v
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let _keep = Keep::start(None);
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
