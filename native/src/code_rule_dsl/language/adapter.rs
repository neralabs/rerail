//! A family's semantics behind one type: the `Adapter` each registry family implements, and the
//! concern traits (`ScopeRules`, `ValueRules`, `ArgumentRules`, `CallRules`, `FormatArgRules`)
//! its associated types name. The family code stays in its modules (`scope/python.rs`,
//! `value/ecma.rs`, `calls/php.rs`, ...); an adapter is the seam that selects it. Callers
//! dispatch with `with_adapter!` (generated from the registry's families, `build_languages.rs`),
//! the one exhaustive family match. In its body the adapter is a concrete type, whose concerns
//! Rust names only as `<A as Adapter>::Value`; a function generic over `A: Adapter`, called
//! there, names them `A::Value`.

mod arguments;
mod calls;
mod format_args;
#[cfg(test)]
mod kit;
mod scope;
mod syntax;
mod value;

pub(crate) use super::with_adapter;
pub(crate) use arguments::{ArgumentFeature, ArgumentRules};
pub(crate) use calls::CallRules;
pub(crate) use format_args::FormatArgRules;
pub(crate) use scope::ScopeRules;
pub(crate) use syntax::{OptionKeys, SyntaxFeature, SyntaxRules};
pub(crate) use value::ValueRules;

use super::{Family, language_not_supported};
use crate::code_rule_dsl::identifier::python_name;
use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{
    file_eslint_directives, file_noqa, file_phpcs, file_phpstan, file_psalm,
};
use crate::code_rule_dsl::model::{ViolationSpec, python_module_path_on_disk};
use crate::code_rule_dsl::suppress::{
    EslintDirectives, NoqaIndex, PhpcsIndex, PhpstanIndex, PsalmIndex, Suppress, parent_statement,
    root,
};
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::path::Path;

/// Python's family: Ruff's semantics.
pub(crate) struct Python;
/// TypeScript, TSX and JavaScript: ESLint's semantics.
pub(crate) struct Ecma;
/// PHP's family.
pub(crate) struct Php;
/// Blade templates: their semantics are those of the PHP the template embeds (`blade::view`),
/// apart from scope, which reads the compiled view (`php::visit_blade_program`).
pub(crate) struct Blade;
/// The adapter of a capability a family does not read (registry `refused` or `not-supported-yet`).
pub(crate) struct Refused;

/// A part of a document a family's semantics read: the root of its tree.
pub(crate) struct Region<'r, D: Doc> {
    pub(crate) root: Node<'r, D>,
}

/// A document's regions: one, or none for a template without embedded PHP.
pub(crate) type Regions<'r, D> = std::option::IntoIter<Region<'r, D>>;

/// A registry family's semantics.
pub(crate) trait Adapter: 'static {
    /// The family dispatch names it by; the adapter kit checks the two agree.
    #[cfg_attr(not(test), allow(dead_code))]
    const FAMILY: Family;
    /// The family's name in diagnostics.
    const LABEL: &'static str;
    /// The line terminators of the linter the family's ports follow.
    const TERMINATORS: Terminators;
    type Scope: ScopeRules;
    type Value: ValueRules;
    type Arguments: ArgumentRules;
    type Calls: CallRules;
    type FormatArgs: FormatArgRules;
    /// An identifier's text as the family compares names (Python: NFKC; others: as written).
    fn name(text: Cow<'_, str>) -> Cow<'_, str>;
    /// The parts of a document this family's semantics read: the root itself, but for an
    /// embedding family; Blade's is its compiled view's `program` (`blade::program`).
    fn regions<'r, D: Doc>(root: &Node<'r, D>) -> Regions<'r, D>;
    /// A file's module, from package markers on disk, in the shape hosts send as `module`
    /// (Python only).
    fn module_path(file: &Path) -> Option<serde_json::Value>;
    /// Whether the family's inline suppression comments suppress `violation` (whose `suppress`
    /// is `suppress`) at byte `at` of `source`, `node` placing the file.
    fn suppresses<D: Doc>(
        violation: &ViolationSpec,
        suppress: &Suppress,
        node: &Node<'_, D>,
        source: &str,
        lines: &LineTable,
        at: usize,
    ) -> bool;
}

impl Adapter for Python {
    const FAMILY: Family = Family::Python;
    const LABEL: &'static str = "python";
    const TERMINATORS: Terminators = Terminators::Newlines;
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Self;
    type FormatArgs = Self;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        python_name(text)
    }

    fn regions<'r, D: Doc>(root: &Node<'r, D>) -> Regions<'r, D> {
        Some(Region { root: root.clone() }).into_iter()
    }

    fn module_path(file: &Path) -> Option<serde_json::Value> {
        python_module_path_on_disk(file)
    }

    fn suppresses<D: Doc>(
        _: &ViolationSpec,
        suppress: &Suppress,
        node: &Node<'_, D>,
        source: &str,
        lines: &LineTable,
        at: usize,
    ) -> bool {
        suppress.noqa.as_ref().is_some_and(|noqa| {
            let index = file_noqa(|| NoqaIndex::read(&root(node), source, lines));
            let parent = noqa.parent.then(|| parent_statement(node, at)).flatten();
            index.suppresses(&noqa.codes, at, parent)
        })
    }
}

impl Adapter for Ecma {
    const FAMILY: Family = Family::Ecma;
    const LABEL: &'static str = "typescript";
    const TERMINATORS: Terminators = Terminators::Eslint;
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Refused;
    type FormatArgs = Refused;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        text
    }

    fn regions<'r, D: Doc>(root: &Node<'r, D>) -> Regions<'r, D> {
        Some(Region { root: root.clone() }).into_iter()
    }

    fn module_path(_: &Path) -> Option<serde_json::Value> {
        None
    }

    fn suppresses<D: Doc>(
        _: &ViolationSpec,
        suppress: &Suppress,
        node: &Node<'_, D>,
        source: &str,
        lines: &LineTable,
        at: usize,
    ) -> bool {
        suppress.eslint.as_ref().is_some_and(|rules| {
            let directives = file_eslint_directives(|| EslintDirectives::read(node, source, lines));
            directives.suppresses(rules, at, lines)
        })
    }
}

impl Adapter for Php {
    const FAMILY: Family = Family::Php;
    const LABEL: &'static str = "php";
    const TERMINATORS: Terminators = Terminators::Newlines;
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Self;
    type FormatArgs = Self;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        text
    }

    fn regions<'r, D: Doc>(root: &Node<'r, D>) -> Regions<'r, D> {
        Some(Region { root: root.clone() }).into_iter()
    }

    fn module_path(_: &Path) -> Option<serde_json::Value> {
        None
    }

    /// A PHP project may run PHPStan, Psalm and PHP_CodeSniffer together: any of the linters a
    /// violation names may suppress it. A PHPStan identifier ignores one error, so the
    /// violation's witnesses on a line spend the identifiers listed for it, as they are met.
    fn suppresses<D: Doc>(
        violation: &ViolationSpec,
        suppress: &Suppress,
        node: &Node<'_, D>,
        source: &str,
        lines: &LineTable,
        at: usize,
    ) -> bool {
        suppress.phpstan.as_ref().is_some_and(|identifiers| {
            let index = file_phpstan(|| PhpstanIndex::read(&root(node), source, lines));
            index.suppresses(&violation.code, identifiers, at, lines)
        }) || suppress.psalm.as_ref().is_some_and(|psalm| {
            let index = file_psalm(|| PsalmIndex::read(&root(node), source));
            index.suppresses(&psalm.names, psalm.suppress_all, at)
        }) || suppress.phpcs.as_ref().is_some_and(|phpcs| {
            let index = file_phpcs(|| PhpcsIndex::read(&root(node), source, lines));
            index.suppresses(&phpcs.codes, phpcs.version, at, lines)
        })
    }
}

impl Adapter for Blade {
    const FAMILY: Family = Family::Blade;
    const LABEL: &'static str = "blade";
    const TERMINATORS: Terminators = Terminators::Newlines;
    type Scope = Self;
    type Value = Php;
    type Arguments = Self;
    type Calls = Php;
    type FormatArgs = Php;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        text
    }

    fn regions<'r, D: Doc>(root: &Node<'r, D>) -> Regions<'r, D> {
        crate::code_rule_dsl::blade::program(root)
            .map(|root| Region { root })
            .into_iter()
    }

    fn module_path(_: &Path) -> Option<serde_json::Value> {
        None
    }

    /// No linter reads a template's `{{-- --}}` comments for directives.
    fn suppresses<D: Doc>(
        _: &ViolationSpec,
        _: &Suppress,
        _: &Node<'_, D>,
        _: &str,
        _: &LineTable,
        _: usize,
    ) -> bool {
        false
    }
}

/// The module a host sends for the file at `file` whose dialect's label is `label`: its
/// family's `Adapter::module_path`; `None` for a label no dialect has.
pub(crate) fn module_path(label: &str, file: &Path) -> Option<serde_json::Value> {
    let dialect = super::dialect_by_label(label)?;
    with_adapter!(Family::of(dialect), A => A::module_path(file))
}

/// Why a family refuses a DSL feature, with the message the refusal reports.
pub(crate) enum Refusal {
    /// The family has no such thing: the rule is invalid.
    Refused(Cow<'static, str>),
    /// Planned: the rule is `language-not-supported`, kept but inactive.
    NotSupportedYet(Cow<'static, str>),
}

/// The compile error of a refusal at `path`: invalid, or `language-not-supported`.
pub(crate) fn gate(path: &str, verdict: Result<(), Refusal>) -> napi::Result<()> {
    verdict.map_err(|refusal| match refusal {
        Refusal::Refused(message) => invalid(format!("{path}: {message}")),
        Refusal::NotSupportedYet(message) => language_not_supported(format!("{path}: {message}")),
    })
}
