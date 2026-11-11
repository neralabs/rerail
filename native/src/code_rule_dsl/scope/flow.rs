//! Variable flow: what reaches each read of a variable and each binding, as a family whose scope has a flow
//! analysis builds it (PHP's, `scope::php::Flow`). The shared readers (`ScopeTable::flow`, `matcher::binds`,
//! `matcher::resolve`) ask it through `VariableFlow`; the family's own code reads its own type.

use crate::code_rule_dsl::value::NodeRef;

/// A family's flow over one file.
pub(crate) trait VariableFlow: std::any::Any + Send + Sync {
    /// The read starting at byte `start`.
    fn read(&self, start: usize) -> Option<&Read>;
    /// What reaches the binding whose name node starts at `start`.
    fn reach(&self, start: u32) -> Option<&Reach>;
    /// Whether the binding at `start` reaches its scope's normal end; `None` where that is not known.
    fn latest(&self, scope: u32, start: u32) -> Option<bool>;
    /// Whether the binding at `start` writes a variable of an outer scope; `None` where that is not known.
    fn outer(&self, start: u32) -> Option<bool>;
    /// The flow as its family's own type.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Whether a variable is defined where it is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Defined {
    /// On every path.
    Yes,
    /// On no path.
    No,
    /// On some paths.
    Maybe,
    /// PHP's dynamic symbol table, a reference or an unknown callee may decide it.
    Unknown,
}

/// How a read may meet an undefined variable without an ordinary warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Quiet {
    /// An ordinary read: an undefined variable warns, and PHPStan reports it.
    No,
    /// `isset(...)`, `empty(...)`, the left side of `??` and `??=`, a subscript or reference that
    /// creates the variable (`$a[] = 1`, `&$a`): silent, and PHPStan reports nothing.
    Silent,
    /// An argument a callee may take by reference: whether PHP warns, and whether PHPStan reports
    /// an undefined one, depends on the callee (PHPStan's stubs tell an out-parameter such as
    /// `preg_match`'s `$matches` from an in-out one such as `array_push`'s).
    ByReference,
}

/// What reaches one read of a variable.
#[derive(Clone, Debug)]
pub(crate) struct Read {
    /// The variable, `$` included.
    pub(crate) name: Box<str>,
    /// The node read: the variable, or the string of a `compact` argument.
    pub(crate) node: NodeRef,
    /// The scope the read is made in.
    pub(crate) scope: u32,
    /// The bindings some path reaches the read with: their scope and index among that scope's
    /// bindings of the name.
    pub(crate) bindings: Box<[(u32, usize)]>,
    pub(crate) defined: Defined,
    /// The variable may hold a value none of `bindings` gave it: a reference to it was taken, it
    /// is `global` or `static`, or a callee may have taken it by reference.
    pub(crate) aliased: bool,
    pub(crate) quiet: Quiet,
    /// A superglobal (`$_GET`, `$GLOBALS`, ...), which every scope sees.
    pub(crate) superglobal: bool,
    /// A read that changes the value rather than reads it for another use: the binding the
    /// change makes carries its use back to the bindings this read reaches.
    pub(crate) modify: Option<Modify>,
}

/// What reaches a binding where it is made: the bindings of its variable some path reaches the
/// binding's site with (the ones it overwrites), whether some path reaches it with the variable
/// undefined, and whether PHP's dynamic symbol table may have bound the variable on some path, so
/// that `bindings` may be incomplete.
#[derive(Clone, Debug, Default)]
pub(crate) struct Reach {
    pub(crate) bindings: Box<[(u32, usize)]>,
    /// Read by the oracle tests, which record where a binding meets its variable undefined.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) undefined: bool,
    pub(crate) unknown: bool,
}

/// How a read changes its variable's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Modify {
    /// The base of a subscript assignment (`$a[] = 1`, `$a[k] .= v;`).
    Element,
    /// A compound assignment or increment whose value is discarded (`$a .= v;`, `$i++;`): as
    /// Psalm reads it, a value only such an update of the variable reads is used only if the
    /// updated value is.
    Update,
}
