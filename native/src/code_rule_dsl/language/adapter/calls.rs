//! The workspace call graph (`calls`): `Semantic::Calls`.

use super::{Php, Python, Refused};

/// A family's calls.
pub(crate) trait CallRules {
    /// Whether the family's calls join the call graph: its languages' registry capability,
    /// which the adapter kit checks.
    #[cfg_attr(not(test), allow(dead_code))]
    const SUPPORTED: bool;
}

impl CallRules for Python {
    const SUPPORTED: bool = true;
}

impl CallRules for Php {
    const SUPPORTED: bool = true;
}

impl CallRules for Refused {
    const SUPPORTED: bool = false;
}
