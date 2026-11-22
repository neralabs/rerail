//! PHP's reading of `resolves` (`ScopeRules::resolves`): class, function and constant names through the file's
//! names (`scope::php_names`), variables through its flow (`scope::php::Flow`).

use super::*;
use crate::code_rule_dsl::scope::php_names::{self, PhpNames, Resolved};

/// PHP: the node is a class, function or constant name that resolves to a listed name (any
/// resolved name with `import: true`), or (`imported`) the name a `use` clause imports, whose
/// fully qualified name is listed. `self`, `static` and `parent` resolve to none; an
/// unqualified function or constant PHP resolves only at run time is unknown unless this file
/// declares it or PHP defines it (`scope/php_names.rs`).
pub(crate) fn holds<D: Doc>(this: &Resolves, node: &Node<'_, D>) -> bool {
    let node = this.unparenthesized(node.clone());
    if node.kind() == "variable_name" {
        return this.reference_kinds.contains(&node.kind_id()) && variable(this, &node);
    }
    // A `name` holding an expression (`X::{expr}`) is no name.
    if !this.reference_kinds.contains(&node.kind_id())
        || (node.kind() == "name" && node.children().any(|child| child.is_named()))
    {
        return false;
    }
    let Some(names) = file_names(&node, this.language).as_ref().and_then(PhpNames::of) else {
        return false;
    };
    if let Some((kind, name)) = names.import(&node.range()) {
        return php_names::listed(&this.imported, kind, name, this.prefix);
    }
    if this.imports.is_empty() && !this.import_any {
        return false;
    }
    let Some(reference) = names.reference(&node.range()) else {
        return false;
    };
    let listed = |kind, name: &str| {
        this.import_any || php_names::listed(&this.imports, kind, name, this.prefix)
    };
    let verdict = match (&reference.resolved, reference.kind) {
        (Resolved::Name(name), Some(kind)) => Some(listed(kind, name)),
        (Resolved::Special, _) => Some(false),
        // PHP decides at run time; both candidates listed, or neither, decide here.
        (Resolved::Either { namespaced, global }, Some(kind)) => {
            let namespaced = listed(kind, namespaced);
            (namespaced == listed(kind, global)).then_some(namespaced)
        }
        _ => None,
    };
    verdict.unwrap_or_else(|| {
        UNKNOWN.set(true);
        false
    })
}

/// A PHP variable read: `global`, a superglobal (listed, or any with `global: true`);
/// `unresolved`, a read PHPStan reports undefined because no path defines it. A read `isset`,
/// `empty` or `??` guards is never unresolved; one a callee may take by reference, or one some
/// paths define, is unknown.
fn variable<D: Doc>(this: &Resolves, node: &Node<'_, D>) -> bool {
    use crate::code_rule_dsl::scope::php::{Defined, Quiet};
    let Some(table) = file_table(node, this.language, Reading::default()) else {
        return false;
    };
    let Some(read) = table
        .flow
        .as_deref()
        .and_then(|flow| flow.read(node.range().start))
    else {
        // A binding's own name, not a read.
        return false;
    };
    if this.global_any || !this.globals.is_empty() {
        return read.superglobal
            && (this.global_any || this.globals.contains(read.name.as_ref()));
    }
    if this.unresolved.is_none() || read.superglobal {
        return false;
    }
    match (read.quiet, read.defined) {
        (Quiet::Silent, _) | (_, Defined::Yes) => false,
        (Quiet::No, Defined::No) => true,
        _ => {
            UNKNOWN.set(true);
            false
        }
    }
}
