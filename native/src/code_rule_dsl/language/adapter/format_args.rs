//! Format strings' arguments (`formatArgs`): `Semantic::FormatArgs`.

use super::{Php, Python, Refused};

/// A family's format strings.
pub(crate) trait FormatArgRules {
    /// Whether the family reads format strings' arguments: its languages' registry capability,
    /// which the adapter kit checks.
    #[cfg_attr(not(test), allow(dead_code))]
    const SUPPORTED: bool;
}

impl FormatArgRules for Python {
    const SUPPORTED: bool = true;
}

impl FormatArgRules for Php {
    const SUPPORTED: bool = true;
}

impl FormatArgRules for Refused {
    const SUPPORTED: bool = false;
}
