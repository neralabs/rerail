//! The indexes the join executors read, described once (decision 0706): what a step needs (`Access`), what the
//! planner has for it (`Available`), what a run saw each one save (`Use`), and whether one pays for its build (`pays`).
//!
//! An executor declares, for a step, the `Access`es it would read and its price with and without each. The driver
//! gathers a level's, made unique by `Access`, builds those that pay into their relations' blocks before the level
//! runs, and the level's readers find them there. No reader builds one.
//!
//! Here because the executors (setfix) and the planner both read it; the planner's base sits above setfix.

use std::collections::BTreeMap;

/// What a step reads of a relation, beside its rows.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// An order on these columns (`runs::Index`).
    Order(Vec<u16>),
    /// A membership filter on these columns, sorted (`blocks::relation::FILTER`): a key it rejects is in no row.
    Filter(Vec<u16>),
    /// A node-keyed index of one column (`blocks::relation::NODE_BITS`, decision 0694): exact membership by node.
    NodeBits(u16),
    /// A value index of one column: its rows by value, for a seed scan with a constant (E4).
    Value(u16),
    /// A view's nodes whose column holds a constant, as node bits in its source's block (`Source::build_value_bits`):
    /// a test of a bound node against the constant (`kind(n, K)`, `!kind(n, K)`) is one bit, no row of it read. The
    /// prepare of a filter whose point query is a view's row by node. The source's node column, the constant's column,
    /// the constant.
    ValueBits(u16, u16, i64),
    /// A group summary of a bag: its group columns and aggregate (E3).
    Summary { group: Vec<u16>, agg: String },
    /// An interval order on (start, end) columns (E5's `within`).
    Interval(u16, u16),
}

/// An index of a relation, by the relation's name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Access {
    pub rel: String,
    pub kind: Kind,
}

impl Access {
    /// A membership test of `rel` by `cols`: node bits for one column, a filter on several (sorted, so a step's key
    /// order does not make another filter).
    pub fn membership(rel: &str, cols: impl IntoIterator<Item = u16>) -> Self {
        let mut cols: Vec<u16> = cols.into_iter().collect();
        cols.sort_unstable();
        cols.dedup();
        let kind = match cols.as_slice() {
            [c] => Kind::NodeBits(*c),
            _ => Kind::Filter(cols),
        };
        Self { rel: rel.to_owned(), kind }
    }
}

/// Whether an index is there for the planner to price with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// In its relation's block.
    Built,
    /// To be built before its readers' level: `cost` (ns), shared by `readers`.
    Scheduled { cost: f64, readers: u32 },
}

/// The indexes a search prices with: those not listed are absent, priced at the step's fallback.
#[derive(Clone, Debug, Default)]
pub struct Available(pub BTreeMap<Access, State>);

impl Available {
    /// The ns a step reading `a` pays for it: nothing where it is built, its share where it is scheduled, and its whole
    /// build over `rows` where it is absent (decision 0706 §3).
    pub fn cost(&self, a: &Access, rows: u64) -> f64 {
        self.share(a).unwrap_or_else(|| build_ns(&a.kind, rows))
    }
    pub fn state(&self, a: &Access) -> Option<State> {
        self.0.get(a).copied()
    }
    /// The build cost a reader of `a` pays: none when built, its share when scheduled, `None` when absent.
    pub fn share(&self, a: &Access) -> Option<f64> {
        match self.state(a)? {
            State::Built => Some(0.0),
            State::Scheduled { cost, readers } => Some(cost / f64::from(readers.max(1))),
        }
    }
}

/// What the steps reading an `Access` did over a run's chunks: membership tests made, and those that found a row. For
/// a view's value index (`Kind::Value`): the walks of its column, and the chunks they were in. For
/// a summary (`Kind::Summary`): its probes, and the rows its relation held at each step's count (`found`), so the
/// probes a row is the ratio, whatever a chunk's size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Use {
    pub tests: u64,
    pub found: u64,
}

impl Use {
    pub fn add(&mut self, o: Use) {
        self.tests += o.tests;
        self.found += o.found;
    }
}

/// An exact membership probe of a computed relation, in ns (t57's `probe2-exists` on `all`, 100 to 230 a test).
pub const PROBE_NS: f64 = 100.0;
/// A view's row read by a bound node for a test (its document found, its local id, its row's value read), in ns (0706
/// E1's node reads: `node_hint`, `granule_of_low` and the read, about 150 a row on `all`).
pub const VIEW_READ_NS: f64 = 150.0;
/// A filter's test (a key's hash, one load, one compare), in ns.
pub const FILTER_TEST_NS: f64 = 4.0;
/// A filter's build, in ns a row (the row's key read, hashed, and its word's bits set).
pub const FILTER_BUILD_NS: f64 = 6.0;
/// Node bits' build, in ns a row (the column's values sorted and laid out by document).
pub const NODE_BITS_BUILD_NS: f64 = 40.0;
/// Node bits' test (a document found, its slot's entry, one bit), in ns.
pub const NODE_BITS_TEST_NS: f64 = 12.0;
/// A scalar step's probe of a bag folded as asked (its memo missed: two write locks, its candidates found and read),
/// in ns (decision 0706 E3: the step's 1,474 ns a row in on `all`, its probes nearly all misses).
pub const SUMMARY_FOLD_NS: f64 = 1_000.0;
/// A probe of a summary built whole (a read lock, one hash lookup), in ns.
pub const SUMMARY_HIT_NS: f64 = 60.0;
/// A walk of a view's column for one constant, in ns a row (the `field` view's, on `all`: 8 ms over 183 k rows).
pub const VALUE_WALK_NS: f64 = 44.0;
/// A view's value index built, in ns a row: the column walked once, its rows sorted by value.
pub const VALUE_BUILD_NS: f64 = 60.0;
/// A whole summary's build, in ns a row (the row's group key read, hashed, its group's summary updated).
pub const SUMMARY_BUILD_NS: f64 = 30.0;
/// A derived input's sorted order built (`Csr::runs`), in ns a row: on `all`, 116 ms over 1.32 M rows (decision
/// 0706 E5).
pub const ORDER_BUILD_NS: f64 = 88.0;
/// The share of absent keys a filter lets through (`blocks::relation::FILTER_BITS`, 5 bits a key).
pub const FILTER_PASS: f64 = 0.003;

/// The ns `kind` takes to build over `rows`.
pub fn build_ns(kind: &Kind, rows: u64) -> f64 {
    rows as f64
        * match kind {
            Kind::Filter(_) => FILTER_BUILD_NS,
            Kind::NodeBits(_) => NODE_BITS_BUILD_NS,
            Kind::Summary { .. } => SUMMARY_BUILD_NS,
            Kind::Value(_) => VALUE_BUILD_NS,
            Kind::ValueBits(..) => NODE_BITS_BUILD_NS,
            Kind::Order(_) => ORDER_BUILD_NS,
            _ => f64::INFINITY,
        }
}

/// The ns a membership step's tests save with `kind` against its exact probe alone: a rejected test skips the probe,
/// every test pays the index's own.
pub fn saves_ns(kind: &Kind, u: Use) -> f64 {
    let absent = u.tests.saturating_sub(u.found) as f64;
    match kind {
        // Node bits are exact: every test is the bits' alone.
        Kind::NodeBits(_) => u.tests as f64 * (PROBE_NS - NODE_BITS_TEST_NS),
        // A view's bits are exact too: every test is a bit, not its row's read by node.
        Kind::ValueBits(..) => u.tests as f64 * (VIEW_READ_NS - NODE_BITS_TEST_NS),
        Kind::Filter(_) => absent * (1.0 - FILTER_PASS) * PROBE_NS - u.tests as f64 * FILTER_TEST_NS,
        _ => 0.0,
    }
}

/// Whether `kind` over a relation of `rows` pays for its build, its readers having done `u` (decision 0706 §4): its
/// savings past its cost, as `keeps` decides a relation.
pub fn pays(kind: &Kind, rows: u64, u: Use) -> bool {
    match kind {
        // A summary's use is by the rows its relation held: its probes over those rows against a build over them.
        Kind::Summary { .. } => u.tests as f64 * (SUMMARY_FOLD_NS - SUMMARY_HIT_NS) > build_ns(kind, u.found.max(1)),
        // A value index's use is its walks (`tests`) over the chunks they were in (`found`): the walks a chunk makes
        // against one build a chunk, both over the view's rows.
        Kind::Value(_) => u.tests as f64 * VALUE_WALK_NS > u.found.max(1) as f64 * VALUE_BUILD_NS,
        // An order is its probes' only read: a reader without one makes it, at the same cost. Probed at all, it pays.
        Kind::Order(_) => u.tests > 0,
        _ => saves_ns(kind, u) > build_ns(kind, rows),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decision 0706 §1, §4: a membership is one `Access` whatever its key's order; a filter pays when its readers'
    /// tests mostly miss and outnumber the rows, and not when they mostly hit.
    #[test]
    fn a_views_bits_by_a_constant_pay_by_their_tests_against_the_constants_nodes() {
        let k = Kind::ValueBits(1, 2, 7);
        assert!(pays(&k, 50_000, Use { tests: 1_000_000, found: 10 }), "a million tests of 50 k nodes");
        assert!(!pays(&k, 50_000, Use { tests: 1_000, found: 10 }), "a thousand tests of 50 k nodes");
    }

    #[test]
    fn a_membership_is_one_access_and_pays_by_its_use() {
        assert_eq!(Access::membership("r", [2, 0]), Access::membership("r", [0, 2]));
        assert_eq!(Access::membership("r", [3]).kind, Kind::NodeBits(3));
        let f = Access::membership("r", [0, 1]).kind;
        assert!(pays(&f, 1_000, Use { tests: 7_000_000, found: 30_000 }), "nameref's: 7.3 M tests, 0.03 M found");
        assert!(!pays(&f, 1_000, Use { tests: 7_000, found: 6_900 }), "tests that find rows save nothing");
        assert!(!pays(&f, 10_000_000, Use { tests: 1_000, found: 0 }), "a few tests do not pay a big build");
        let mut a = Available::default();
        let k = Access::membership("r", [0, 1]);
        assert_eq!(a.share(&k), None, "absent");
        a.0.insert(k.clone(), State::Scheduled { cost: 900.0, readers: 3 });
        assert_eq!(a.share(&k), Some(300.0), "a scheduled build shared by its readers");
        a.0.insert(k.clone(), State::Built);
        assert_eq!(a.share(&k), Some(0.0));
    }

    /// Decision 0706 E5, §3: an order pays once probed at all (a reader would make it anyway); the search prices one
    /// absent at its whole build over its input's rows, one scheduled at its build's share, one built at nothing.
    #[test]
    fn an_order_is_priced_by_what_is_available() {
        let k = Access { rel: "r".into(), kind: Kind::Order(vec![0]) };
        assert!(pays(&k.kind, 1_000_000, Use { tests: 1, found: 0 }));
        assert!(!pays(&k.kind, 1_000_000, Use { tests: 0, found: 0 }));
        let mut a = Available::default();
        assert_eq!(a.cost(&k, 1_000), 1_000.0 * ORDER_BUILD_NS, "absent: its whole build");
        a.0.insert(k.clone(), State::Scheduled { cost: 4_000.0, readers: 4 });
        assert_eq!(a.cost(&k, 1_000), 1_000.0, "scheduled: its share");
        a.0.insert(k.clone(), State::Built);
        assert_eq!(a.cost(&k, 1_000), 0.0, "built");
    }

    /// Decision 0706 E3: a summary pays when its probes are not few against its relation's rows, whatever its size.
    #[test]
    fn a_summary_pays_by_its_probes_a_row() {
        let k = Kind::Summary { group: vec![0, 1], agg: String::new() };
        assert!(pays(&k, 1, Use { tests: 330_631, found: 330_607 }), "public_len's: a probe a row");
        assert!(pays(&k, 1, Use { tests: 481_713, found: 21_473 * 40 }), "rf_fn_at's count: 12 probes a row held");
        assert!(!pays(&k, 1, Use { tests: 36, found: 64_799 }), "rf_owner's: 36 probes of one group, whose fold reads what a build would");
    }

    /// Decision 0706 E4: a view's value index pays when a chunk walks its column for more than one constant.
    #[test]
    fn a_value_index_pays_by_its_walks_a_chunk() {
        let k = Kind::Value(1);
        assert!(pays(&k, 0, Use { tests: 8 * 36, found: 36 }), "field's: 8 fields walked a chunk");
        assert!(pays(&k, 0, Use { tests: 2, found: 1 }), "two walks a chunk");
        assert!(!pays(&k, 0, Use { tests: 36, found: 36 }), "one walk a chunk: the build is that walk");
    }
}
