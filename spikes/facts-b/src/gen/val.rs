//! Values inside the dataflow: SQLite's storage classes as small `Copy` records (text interned per
//! process), convertible to and from the IR executors' `sql::value::V`.
use rerail_facts::sql::value::{self as sv, V};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;

/// A stored value. `Real` holds the `f64` bits; `Text` an interned symbol. Derived order and
/// equality are representational (what the dataflow keys on); SQL comparison goes through `V`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub enum Val {
    #[default]
    Null,
    Int(i64),
    Real(u64),
    Text(u32),
}

pub type Row = Vec<Val>;

#[derive(Default)]
struct Syms {
    strs: Vec<Rc<str>>,
    map: FxHashMap<Rc<str>, u32>,
}

thread_local! {
    static SYMS: RefCell<Syms> = RefCell::new(Syms::default());
}

pub fn sym(s: &str) -> u32 {
    SYMS.with(|y| {
        let mut y = y.borrow_mut();
        if let Some(&i) = y.map.get(s) {
            return i;
        }
        let r: Rc<str> = Rc::from(s);
        let i = y.strs.len() as u32;
        y.strs.push(r.clone());
        y.map.insert(r, i);
        i
    })
}

pub fn text(i: u32) -> Rc<str> {
    SYMS.with(|y| y.borrow().strs[i as usize].clone())
}

/// Interned strings and their bytes (arena estimate: string bytes + Rc header + map entry).
pub fn syms_footprint() -> (usize, usize) {
    SYMS.with(|y| {
        let y = y.borrow();
        let bytes: usize = y.strs.iter().map(|s| s.len() + 16 + 8 + 24).sum();
        (y.strs.len(), bytes)
    })
}

pub fn to_v(v: Val) -> V {
    match v {
        Val::Null => V::Null,
        Val::Int(i) => V::Int(i),
        Val::Real(b) => V::Real(f64::from_bits(b)),
        Val::Text(s) => V::Text(text(s)),
    }
}

pub fn from_v(v: &V) -> Val {
    match v {
        V::Null => Val::Null,
        V::Int(i) => Val::Int(*i),
        V::Real(f) => Val::Real(f.to_bits()),
        V::Text(s) => Val::Text(sym(s)),
    }
}

pub fn row_to_v(r: &[Val]) -> Vec<V> {
    r.iter().map(|v| to_v(*v)).collect()
}

/// Equality key of a value under SQL `=` after comparison affinity: NULL has none (never equal),
/// an integral REAL equals the INTEGER (`value::key`).
pub fn eq_key(v: &V) -> Option<Val> {
    sv::key(v).map(|k| match k {
        sv::Key::Int(i) => Val::Int(i),
        sv::Key::Real(b) => Val::Real(b),
        sv::Key::Text(s) => Val::Text(sym(&s)),
    })
}

/// Grouping key (DISTINCT, UNION, GROUP BY, PARTITION BY): NULLs equal each other.
pub fn group_key(v: &V) -> Val {
    eq_key(v).unwrap_or(Val::Null)
}
