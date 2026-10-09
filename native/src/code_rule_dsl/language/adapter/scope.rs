//! Scope tables, bindings and usage (`resolves`, `binds`): `Semantic::Scope`.

use super::{Blade, Ecma, Php, Python, Refused};

/// A family's scope semantics.
pub(crate) trait ScopeRules {
    /// Whether the family builds scope tables: its languages' registry capability,
    /// which the adapter kit checks.
    #[cfg_attr(not(test), allow(dead_code))]
    const SUPPORTED: bool;
}

impl ScopeRules for Python {
    const SUPPORTED: bool = true;
}

impl ScopeRules for Ecma {
    const SUPPORTED: bool = true;
}

impl ScopeRules for Php {
    const SUPPORTED: bool = true;
}

impl ScopeRules for Blade {
    const SUPPORTED: bool = true;
}

impl ScopeRules for Refused {
    const SUPPORTED: bool = false;
}
