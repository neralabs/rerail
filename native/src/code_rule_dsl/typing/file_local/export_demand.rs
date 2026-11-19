//! Export demand: an export holds only the rows its cross-file readers can use.
//!
//! A file-local relation that a cross-file rule reads is an export. The chunks make it in full and hand all of it
//! on, and the per-file memo keeps it. Its reader often joins it with a relation that names only a few of its rows:
//!
//! ```text
//! fl_later(n, c1) :- within(c1, n), rr_nearest(c1, s), s = n.          -- every (scope, descendant) pair
//! later(n, N)     :- demand_later(n, N), fl_later(n, c1), binds(c1, N). -- the scopes demand_later names
//! ```
//!
//! `fl_later` is 278 k rows on 20 files; the reader uses the scopes `demand_later` names. `demand_later` is
//! cross-file (its `N` is another file's), but its `n` is derived from this file's rows: each of its rules, cut to
//! the literals in `n`'s document, is a file-local rule for a superset of its `n` column:
//!
//! ```text
//! demand_later(s1, N) :- demand_alt(n, N), agg13(n, s1).   -- cross-file
//! dm_demand_later_0(s1) :- dm_demand_alt_0(n), agg13(n, s1). -- file-local: every s1 it can name, and more
//! ```
//!
//! The export is then handed on cut to its readers' demand (`xd_fl_later(n, c1) :- fl_later(n, c1),
//! dm_demand_later_0(n)`), and its readers read that. The cut keeps every row a reader joins: each filter is implied
//! by a literal of the reader.
//!
//! Why the superset fires in the right chunk: a demand's column holds a location (a node or a document). Its rule
//! keeps only the literals joined to that location through same-document columns (`same_doc`), so each lies in the
//! location's document, which is one chunk's. Literals of other documents, negations and aggregates are dropped:
//! dropping a literal only widens the superset. A cross-file literal on a kept location is its own superset,
//! `dm_<rel>_<column>`, made the same way (recursively; a cycle is an ordinary file-local recursion). A rule whose
//! location the kept literals do not bind makes the superset everything (`top`): it filters nothing, and is left out.
//!
//! An export is left whole when a reader reads it negatively, in an aggregate, natively, or with nothing to cut it
//! by; when it is an output, a lattice or a gate's; and when the cut relations do not split as file-local.

mod on_version;
pub use on_version::DemandExports;

/// The most supersets made: past it, a demand is `top` (the readers are cut by what was made).
const MAX_KEYS: usize = 2000;

/// Whether the cut is on: it is, unless `RERAIL_EXPORT_DEMAND=off` (for comparison). The prepared program's key
/// follows the program, so a changed choice is another file-local part and a memo miss.
pub(crate) fn on() -> bool {
    std::env::var("RERAIL_EXPORT_DEMAND").map_or(true, |v| v != "off")
}

/// The rounds a cut is made again in, each without what did not split as file-local.
const ROUNDS: usize = 8;
