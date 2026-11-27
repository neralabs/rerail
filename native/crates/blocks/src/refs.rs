//! References inside a block: each one relative to its own address (Boost's `offset_ptr`, MLDB's `MappedPtr`).
//! The floor's one definition (`rerail_delta_core::refs`), re-exported.

pub use rerail_delta_core::refs::{Ptr, Span};
