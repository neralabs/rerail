//! Value decoding and comparison (`value`, `compare`): `Semantic::Value`.

use super::{Ecma, Php, Python, Refused};

/// A family's values.
pub(crate) trait ValueRules {
    /// Whether the family decodes values: its languages' registry capability,
    /// which the adapter kit checks.
    #[cfg_attr(not(test), allow(dead_code))]
    const SUPPORTED: bool;
}

impl ValueRules for Python {
    const SUPPORTED: bool = true;
}

impl ValueRules for Ecma {
    const SUPPORTED: bool = true;
}

impl ValueRules for Php {
    const SUPPORTED: bool = true;
}

impl ValueRules for Refused {
    const SUPPORTED: bool = false;
}
