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
//! value is computed. The digest is 128 bits: two SipHash-1-3 hashers with independent random keys.

use std::any::Any;
use std::cell::RefCell;
use std::hash::{BuildHasher, Hasher, RandomState};
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

/// Derived values kept on this thread while held.
pub(in crate::code_rule_dsl) struct Keep;

impl Keep {
    pub(in crate::code_rule_dsl) fn start() -> Self {
        MEMO.with(|m| *m.borrow_mut() = Some(Memo { keys: (RandomState::new(), RandomState::new()), values: FxHashMap::default() }));
        Keep
    }
}

impl Drop for Keep {
    fn drop(&mut self) {
        MEMO.with(|m| *m.borrow_mut() = None);
    }
}

/// The digest of a value's inputs, written one by one. Each piece is length- or tag-delimited, so two different
/// input lists never write the same bytes.
pub(in crate::code_rule_dsl) struct Inputs {
    a: std::hash::DefaultHasher,
    b: std::hash::DefaultHasher,
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
}

/// The value `name` over the inputs `inputs` writes: kept from an earlier request with the same digest, or `make`'s.
/// Counted per name as a hit or a miss in the profile's decisions ("derived values").
pub(in crate::code_rule_dsl) fn value<V: Clone + 'static>(name: &'static str, inputs: impl FnOnce(&mut Inputs), make: impl FnOnce() -> V) -> V {
    let digest = MEMO.with(|m| {
        let m = m.borrow();
        let m = m.as_ref()?;
        let mut d = Inputs { a: m.keys.0.build_hasher(), b: m.keys.1.build_hasher(), bytes: 0 };
        d.str(name);
        inputs(&mut d);
        super::profile::work(super::profile::Unit::Byte, d.bytes);
        Some(d.finish())
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
