//! Branded indexes (decision 0678): a row's id, a column's index and a word of liveness bits are each their own
//! type, so one is never passed where another is meant. Each is its integer underneath (`repr(transparent)`), so a
//! read costs what the integer's did.

// The ids: the floor's one definition (`rerail_delta_core::ids`).
pub use rerail_delta_core::ids::*;
