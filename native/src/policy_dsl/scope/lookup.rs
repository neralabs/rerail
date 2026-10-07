//! How `resolves.binding` reads a reference's binding beyond `ScopeTable::binding`.

use super::{Binding, NodeRef, Reference, ScopeTable};

/// Readings of a reference's binding that some Ruff checks use instead of `resolve_name`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Lookup {
    /// Ruff's `lookup_symbol`: in a scope that declares the name `global`, the read stops at that
    /// scope's `global` binding, which carries no value, instead of continuing in the module.
    /// PHP's reads always stop there.
    pub(crate) stop_at_global: bool,
    /// Ruff's `only_binding`: the binding shadows no earlier binding of its name in its scope.
    /// TypeScript and PHP: the name has that one declaration (binding) in its scope.
    pub(crate) only: bool,
}

impl ScopeTable {
    /// `ScopeTable::binding` under `lookup`: `None` where `only` does not hold.
    pub(crate) fn binding_by(
        &self,
        reference: &Reference,
        name: &str,
        lookup: Lookup,
    ) -> Option<(&Binding, Option<NodeRef>)> {
        if lookup == Lookup::default() {
            return self.binding(reference, name);
        }
        match crate::policy_dsl::language::Family::of(self.language) {
            crate::policy_dsl::language::Family::Python => {
                let found = self.python_index_by(reference, name, lookup)?;
                let binding = self.binding_at(found, name);
                Some((binding, binding.site.value))
            }
            // A read after `global $x` reads that statement's binding: PHP's flow stops there.
            // Blade's reads are its embedded PHP's (`blade::view`).
            crate::policy_dsl::language::Family::Php
            | crate::policy_dsl::language::Family::Blade => {
                let (scope, _) = self.php_index(reference)?;
                let single = self.scopes[scope as usize]
                    .bindings
                    .get(name)
                    .is_some_and(|bindings| bindings.len() == 1);
                self.binding(reference, name)
                    .filter(|_| !lookup.only || single)
            }
            crate::policy_dsl::language::Family::Ecma => {
                let (scope, _) = self.resolve_lexical_scope(reference, name)?;
                let single = self.scopes[scope as usize]
                    .bindings
                    .get(name)
                    .is_some_and(|bindings| bindings.len() == 1);
                self.binding(reference, name).filter(|_| single)
            }
        }
    }

    /// Python: the binding `ScopeTable::binding_by` reads, as its scope and its index among that
    /// scope's bindings of the name. A deletion, Ruff's `global` placeholder and a binding that
    /// fails `only` give none.
    pub(crate) fn python_index_by(
        &self,
        reference: &Reference,
        name: &str,
        lookup: Lookup,
    ) -> Option<(u32, usize)> {
        let (scope, index) = self.resolve_python_scope(reference, name, lookup.stop_at_global)?;
        let binding = self.binding_at((scope, index), name);
        if !binding.shadows()
            || binding.is_deletion()
            || (lookup.only && !self.first(scope, name, binding))
        {
            return None;
        }
        Some((scope, index))
    }

    /// Whether `binding` was added to `scope` before every other binding of `name` there, in
    /// Ruff's visiting order: the scope's own bindings in document order, then those made from
    /// function bodies Ruff visits later (`global` placeholders, `nonlocal` writes).
    fn first(&self, scope: u32, name: &str, binding: &Binding) -> bool {
        let Some(bindings) = self.scopes[scope as usize].bindings.get(name) else {
            return false;
        };
        let order = |index: usize, binding: &Binding| (binding.deferred, index);
        let Some(own) = bindings
            .iter()
            .position(|candidate| std::ptr::eq(candidate, binding))
        else {
            return false;
        };
        let own = order(own, binding);
        bindings
            .iter()
            .enumerate()
            .all(|(index, other)| order(index, other) >= own)
    }
}
