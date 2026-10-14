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
mod rust;
mod scope;
mod sql;
mod syntax;
mod value;

pub(crate) use super::with_adapter;
pub(crate) use arguments::{ArgumentFeature, ArgumentRules};
pub(crate) use calls::{CallFeature, CallRules};
pub(crate) use format_args::{FormatArgFeature, FormatArgRules};
pub(crate) use rust::Rust;
pub(crate) use scope::{ScopeFeature, ScopeRules};
pub(crate) use sql::Sql;
pub(crate) use syntax::{SyntaxFeature, SyntaxRules};
pub(crate) use value::{ValueFeature, ValueRules};

use super::{Family, GrammarDialect, language_not_supported};
use crate::code_rule_dsl::identifier::python_name;
use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{
    file_eslint_directives, file_noqa, file_nosec, file_phpcs, file_phpstan, file_psalm,
};
use crate::code_rule_dsl::model::{ViolationSpec, python_module_path_on_disk};
use crate::code_rule_dsl::suppress::{
    EslintDirectives, NoqaIndex, NosecIndex, PhpcsIndex, PhpstanIndex, PsalmIndex, Suppress,
    parent_statement, root,
};
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::path::Path;

/// Python's family: Ruff's semantics.
pub(crate) struct Python;
/// TypeScript, TSX and JavaScript: ESLint's semantics.
pub(crate) struct Ecma;
/// PHP's family.
pub(crate) struct Php;
/// Blade templates: their semantics are those of the PHP the template embeds (`blade::view`).
/// Each concern impl delegates to PHP's, apart from what reads the template itself: scope reads
/// the compiled view (`php::visit_blade_program`), calls come from the tape, and refusals name
/// Blade.
pub(crate) struct Blade;
/// The adapter of a capability a family does not read (registry `refused` or `not-supported-yet`).
pub(crate) struct Refused;

/// A part of a document a family's semantics read: the root of its tree, and the dialect that
/// tree is read as (the file's own, or the one its embedder built an embedded program in:
/// `tape::EmbeddedProgram`).
pub(crate) struct Region<'r, D: Doc> {
    pub(crate) root: Node<'r, D>,
    pub(crate) dialect: GrammarDialect,
}

/// The parts of the document of `dialect` rooted at `root` that its family's semantics read: the
/// root itself, but in a dialect that embeds others (`GrammarDialect::embeds`) the programs its
/// tape embeds (`embed`), each read as the dialect its embedder built it in, which a tape records
/// (`tape::EmbeddedProgram`): a Blade template's compiled view, PHP; none where it embeds nothing
/// (or is no tape). At most one program per registry `via`. Consumers read every region; call
/// summaries read the first (`calls::summarize`), so a family whose documents may hold several
/// refuses calls.
pub(crate) fn regions<'r, D: Doc>(
    root: &Node<'r, D>,
    dialect: GrammarDialect,
) -> impl Iterator<Item = Region<'r, D>> + use<'r, D> {
    let doc: &'r dyn std::any::Any = root.get_doc();
    let programs = doc
        .downcast_ref::<TapeDoc>()
        .map_or(&[][..], TapeDoc::programs);
    let embedding = dialect.kinds().has_embedded();
    let whole = (!embedding).then(|| Region {
        root: root.clone(),
        dialect,
    });
    // The programs are the root's last children, in tape order.
    let host = root.clone();
    let count = if embedding { host.children().len() } else { 0 };
    let embedded = (count.saturating_sub(programs.len())..count).filter_map(move |nth| {
        let child = host.child(nth)?;
        let program = programs
            .iter()
            .find(|program| program.root as usize == child.node_id())?;
        Some(Region {
            root: child,
            dialect: program.dialect,
        })
    });
    whole.into_iter().chain(embedded)
}

/// A registry family's semantics.
pub(crate) trait Adapter: 'static {
    /// The family dispatch names it by; the adapter kit checks the two agree.
    #[cfg_attr(not(test), allow(dead_code))]
    const FAMILY: Family;
    /// The family's name in diagnostics.
    const LABEL: &'static str;
    /// The line terminators of the linter the family's ports follow.
    const TERMINATORS: Terminators;
    /// Whether the family names a file's module from the package markers around it (Python's
    /// `__init__.py`): a file without a host module gets the markers' module.
    const PACKAGES: bool;
    /// Whether an ast query's `file` may ask for declaration files (`declaration`,
    /// typescript-eslint's `.d.ts` predicate). Python has stubs instead and refuses it.
    const DECLARATION_FILES: bool;
    type Scope: ScopeRules;
    type Value: ValueRules;
    type Arguments: ArgumentRules;
    type Calls: CallRules;
    type FormatArgs: FormatArgRules;
    /// An identifier's text as the family compares names (Python: NFKC; others: as written).
    fn name(text: Cow<'_, str>) -> Cow<'_, str>;
    /// A file's module, from package markers on disk, in the shape hosts send as `module`
    /// (Python only).
    fn module_path(file: &Path) -> Option<serde_json::Value>;
    /// Whether the family's inline suppression comments suppress `violation` (whose `suppress`
    /// is `suppress`) at byte `at` of `source`, `node` placing the file and `witness` its byte
    /// span.
    fn suppresses<D: Doc>(
        violation: &ViolationSpec,
        suppress: &Suppress,
        node: &Node<'_, D>,
        source: &str,
        lines: &LineTable,
        at: usize,
        witness: (usize, usize),
    ) -> bool;
}

impl Adapter for Python {
    const FAMILY: Family = Family::Python;
    const LABEL: &'static str = "python";
    const TERMINATORS: Terminators = Terminators::Newlines;
    const PACKAGES: bool = true;
    const DECLARATION_FILES: bool = false;
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Self;
    type FormatArgs = Self;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        python_name(text)
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
        witness: (usize, usize),
    ) -> bool {
        // A project may run Ruff and Bandit together: either linter the violation names may
        // suppress it. Bandit reads `# nosec` on the reported line and on the lines of the node it
        // visited, here the witness.
        suppress.noqa.as_ref().is_some_and(|noqa| {
            let index = file_noqa(|| NoqaIndex::read(&root(node), source, lines));
            let parent = noqa.parent.then(|| parent_statement(node, at)).flatten();
            index.suppresses(&noqa.codes, at, parent)
        }) || suppress.nosec.as_ref().is_some_and(|codes| {
            let index = file_nosec(|| NosecIndex::read(&root(node), source, lines));
            index.suppresses(codes, at, witness, lines)
        })
    }
}

impl Adapter for Ecma {
    const FAMILY: Family = Family::Ecma;
    const LABEL: &'static str = "typescript";
    const TERMINATORS: Terminators = Terminators::Eslint;
    const PACKAGES: bool = false;
    const DECLARATION_FILES: bool = true;
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Refused;
    type FormatArgs = Refused;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        text
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
        _: (usize, usize),
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
    const PACKAGES: bool = false;
    const DECLARATION_FILES: bool = true;
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Self;
    type FormatArgs = Self;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        text
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
        _: (usize, usize),
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
    const PACKAGES: bool = false;
    const DECLARATION_FILES: bool = true;
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Self;
    type FormatArgs = Self;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        text
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
        _: (usize, usize),
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

/// Whether files of the dialect labelled `label` take their module from package markers
/// (`Adapter::PACKAGES`); `false` for a label no dialect has.
pub(crate) fn packages(label: &str) -> bool {
    super::dialect_by_label(label)
        .is_some_and(|dialect| with_adapter!(Family::of(dialect), A => A::PACKAGES))
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
