//! The reach index's form found in a program (decision 0119): the relations held as each start's closure, not as rows.
//!
//! A relation `r` of two columns is a reach when one of its rules is `r(x, z) :- r(x, y), edge(y, z)` (three distinct
//! variables, `edge` not reading `r`) and no other rule reads `r`. Its other rules become the rules of
//! `rr_reach_seed_<r>`, and `r` is made natively (`Native::Reach`) from the seeds and the edges; its one rule left,
//! `r(x, z) :- rr_reach_seed_<r>(x, y), edge(y, z)`, orders it after both. A relation `s` whose one rule is
//! `s(x, q) :- r(x, e), map(e, q)` over a reach `r` is made through it (`Native::ReachThrough`), its rule kept as the
//! order.
//!
//! `.decl r(…) reach` asks for the form: a relation so declared that does not have it is an error, so a rule edited
//! out of the shape is found, not run as rows. `.decl r(…) no_reach` keeps `r` as rows. `RERAIL_REACH_INDEX=off` finds
//! none but those declared `reach`.
//!
//! The passes are `plan::corpus::preparation::demand`'s: `ReachForm` finds the form, and `UndoInCycles` makes rows
//! again of a closure a later rewrite put in a cycle.

/// The prefix of a reach's seed relation.
pub const SEED: &str = "rr_reach_seed_";
/// The qualifier asking for the form.
pub const QUALIFIER: &str = "reach";
