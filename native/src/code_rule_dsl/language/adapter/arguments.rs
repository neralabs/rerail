//! A call's arguments (`arguments`): `Semantic::Arguments`.

use super::{Ecma, Php, Python, Refused};

/// A family's call arguments.
pub(crate) trait ArgumentRules {
    /// Whether the family reads call arguments: its languages' registry capability,
    /// which the adapter kit checks.
    #[cfg_attr(not(test), allow(dead_code))]
    const SUPPORTED: bool;
}

impl ArgumentRules for Python {
    const SUPPORTED: bool = true;
}

impl ArgumentRules for Ecma {
    const SUPPORTED: bool = true;
}

impl ArgumentRules for Php {
    const SUPPORTED: bool = true;
}

impl ArgumentRules for Refused {
    const SUPPORTED: bool = false;
}
