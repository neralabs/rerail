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
mod json;
mod rr;
mod rust;
mod scope;
mod sql;
mod syntax;
// The language pack interface the core asks (generated from the traits above: tools/cratify/packgen.py).
pub(crate) mod pack;
mod value;
mod vue;

pub(crate) use super::with_adapter;
pub(crate) use arguments::{ArgumentFeature, ArgumentRules};
pub(crate) use calls::{CallFeature, CallRules};
pub(crate) use format_args::{FormatArgFeature, FormatArgRules};
// Read through `with_adapter!` only, whose arm for a family left out of the build is `unreachable!`.
#[cfg_attr(not(feature = "lang-json"), allow(unused_imports))]
pub(crate) use json::Json;
// Read through `with_adapter!` only, whose arm for a family left out of the build is `unreachable!`.
#[cfg_attr(not(feature = "lang-rr"), allow(unused_imports))]
pub(crate) use rr::Rr;
// Read through `with_adapter!` only, whose arm for a family left out of the build is `unreachable!`.
#[cfg_attr(not(feature = "lang-rust"), allow(unused_imports))]
pub(crate) use rust::Rust;
pub(crate) use scope::{ScopeFeature, ScopeRules};
// Read through `with_adapter!` only, whose arm for a family left out of the build is `unreachable!`.
#[cfg_attr(not(feature = "lang-sql"), allow(unused_imports))]
pub(crate) use sql::Sql;
pub(crate) use syntax::{QueryWhen, SyntaxFeature, SyntaxRules};
pub(crate) use value::{ValueFeature, ValueRules};
// Read through `with_adapter!` only, whose arm for a family left out of the build is `unreachable!`.
#[cfg_attr(not(feature = "lang-vue"), allow(unused_imports))]
pub(crate) use vue::Vue;

use super::{
    Family, GrammarDialect, PackageType, SourceType, is_declaration_file, language_not_supported,
};
use crate::code_rule_dsl::identifier::{ecma_name, python_name};
use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{
    file_biome_suppressions, file_eslint_directives, file_noqa, file_nosec, file_oxlint_directives,
    file_phpcs, file_phpstan, file_psalm,
};
use crate::code_rule_dsl::model::{ViolationSpec, python_module_path_on_disk};
use crate::code_rule_dsl::suppress::{
    BiomeSuppressions, EslintDirectives, NoqaIndex, NosecIndex, OxlintDirectives, PhpcsIndex,
    PhpstanIndex, PsalmIndex, Suppress, parent_statement, root,
};
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_core::{AstGrep, Doc, Node};
use ast_grep_language::LanguageExt;
use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;
use std::sync::LazyLock;

/// Python's family: Ruff's semantics.
pub(crate) struct Python;
/// TypeScript, TSX and JavaScript: ESLint's semantics.
pub(crate) struct Ecma;
/// PHP's family.
pub(crate) struct Php;
/// Blade templates' own tree (`blade::forte`). A template's semantics are those of the PHP it
/// embeds (`blade::view`), a guest document read with PHP's family (`embed`): a clause reads
/// them with `ast.embedded` or `embedded`. The concerns name PHP's, which the compile refuses on
/// the template's own tree (`compile::refuse_unavailable_semantics`).
pub(crate) struct Blade;
/// The adapter of a capability a family does not read (registry `refused` or `not-supported-yet`).
pub(crate) struct Refused;

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
    /// Whether the family's files read their nearest `package.json`: its `type` decides an ECMA
    /// file's module system (`packageType`, `SourceType::of_file`).
    const PACKAGE_MANIFESTS: bool;
    /// Whether an ast query's `file` may ask for the file's JSX runtime (`jsxRuntime`: its
    /// pragmas, imports and manifest's dependencies).
    const JSX_RUNTIMES: bool;
    /// Whether an ast query's `file` may ask for declaration files (`declaration`,
    /// typescript-eslint's `.d.ts` predicate). Python has stubs instead and refuses it.
    const DECLARATION_FILES: bool;
    /// Whether the linter the family's ports follow strips a leading byte order mark from the
    /// text its rules read (ESLint's `SourceCode`), so a `file.content` regex reads a file from
    /// past it.
    const STRIPS_BOM: bool;
    /// The kinds of the family's comment nodes, where `rerail-ignore` directives are read
    /// (`suppress::rerail`).
    const COMMENT_KINDS: &'static [&'static str];
    /// The leaf kinds that may stand before a file's first code without being code (a shebang
    /// line, PHP's `<?php`): a `rerail-ignore` comment before them all covers the whole file.
    const PREAMBLE_KINDS: &'static [&'static str];
    /// The kinds of a body that starts at its first statement, a comment before that statement
    /// standing beside it rather than in it (Python's `block`): a leading `rerail-ignore` comment
    /// there covers that statement, not the whole body.
    const BODY_KINDS: &'static [&'static str];
    /// The comment kinds a token or sibling walk passes over (`is_comment_kind`).
    const WALK_COMMENT_KINDS: &'static [&'static str] = Self::COMMENT_KINDS;
    /// Whether an ast query may ask where a file's imports lead (`importTarget`, eslint-plugin-n's).
    const IMPORT_TARGETS: bool = false;
    /// Whether the family's files are parsed by Rerail's port of Forte rather than tree-sitter
    /// (`parsed_by_forte`, the registry's `parser`).
    const PARSED_BY_FORTE: bool = false;
    type Scope: ScopeRules;
    type Value: ValueRules;
    type Arguments: ArgumentRules;
    type Calls: CallRules;
    type FormatArgs: FormatArgRules;
    /// An identifier's text as the family compares names (Python: NFKC; ECMAScript: escapes
    /// decoded; others: as written).
    fn name(text: Cow<'_, str>) -> Cow<'_, str>;
    /// The bytes tree-sitter parses for a file of the family, as long as `source` and with its
    /// character boundaries and line ends: `source` itself, but for PHP's view of a template
    /// (`php_view`). A tree of them reads `source`.
    fn parse_input(source: &str) -> Cow<'_, str>;
    /// A file's module, from package markers on disk, in the shape hosts send as `module`
    /// (Python only).
    fn module_path(file: &Path) -> Option<serde_json::Value>;
    /// Whether the family's inline suppression comments suppress `violation` (whose `suppress`
    /// is `suppress`) anchored over the bytes `anchored` of `source` (most linters read only its
    /// start), `node` placing the file and `witness` its byte span.
    fn suppresses<D: Doc>(
        violation: &ViolationSpec,
        suppress: &Suppress,
        node: &Node<'_, D>,
        source: &str,
        lines: &LineTable,
        anchored: Range<usize>,
        witness: (usize, usize),
    ) -> bool;
    /// The module system of the file at `path` whose nearest `package.json` has `package` as
    /// its type (`None`: no manifest), for a query that leaves `sourceType` to the file; `None`
    /// where the file's syntax decides (`has_module_syntax`). A family without module systems
    /// reads every file as a module, the `sourceType` its queries default to.
    fn source_type(_path: &str, _package: Option<PackageType>) -> Option<SourceType> {
        Some(SourceType::Module)
    }
}

impl Adapter for Python {
    const FAMILY: Family = Family::Python;
    const LABEL: &'static str = "python";
    const TERMINATORS: Terminators = Terminators::Newlines;
    const PACKAGES: bool = true;
    const PACKAGE_MANIFESTS: bool = false;
    const JSX_RUNTIMES: bool = false;
    const DECLARATION_FILES: bool = false;
    const STRIPS_BOM: bool = false;
    const COMMENT_KINDS: &'static [&'static str] = &["comment"];
    const PREAMBLE_KINDS: &'static [&'static str] = &[];
    const BODY_KINDS: &'static [&'static str] = &["block"];
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Self;
    type FormatArgs = Self;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        python_name(text)
    }

    fn parse_input(source: &str) -> Cow<'_, str> {
        Cow::Borrowed(source)
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
        anchored: Range<usize>,
        witness: (usize, usize),
    ) -> bool {
        let at = anchored.start;
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
    const PACKAGE_MANIFESTS: bool = true;
    const JSX_RUNTIMES: bool = true;
    const DECLARATION_FILES: bool = true;
    const STRIPS_BOM: bool = true;
    /// `html_comment`: a script's `<!--` and line-leading `-->` comments.
    const COMMENT_KINDS: &'static [&'static str] = &["comment", "html_comment"];
    const IMPORT_TARGETS: bool = true;
    const PREAMBLE_KINDS: &'static [&'static str] = &["hash_bang_line"];
    const BODY_KINDS: &'static [&'static str] = &[];
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Refused;
    type FormatArgs = Refused;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        ecma_name(text)
    }

    fn parse_input(source: &str) -> Cow<'_, str> {
        Cow::Borrowed(source)
    }

    fn module_path(_: &Path) -> Option<serde_json::Value> {
        None
    }

    /// A project may run ESLint, oxlint (which also reads `eslint-` directives) or Biome: any
    /// linter the violation names may suppress it. Oxlint and Biome ask about the anchored span:
    /// an oxlint block covers a problem that overlaps it, a `biome-ignore-start` range one it
    /// holds, a `biome-ignore` one that starts in what it covers.
    fn suppresses<D: Doc>(
        _: &ViolationSpec,
        suppress: &Suppress,
        node: &Node<'_, D>,
        source: &str,
        lines: &LineTable,
        anchored: Range<usize>,
        _: (usize, usize),
    ) -> bool {
        suppress.eslint.as_ref().is_some_and(|rules| {
            let directives = file_eslint_directives(|| EslintDirectives::read(node, source, lines));
            directives.suppresses(rules, anchored.start, lines)
        }) || suppress.oxlint.as_ref().is_some_and(|rules| {
            let directives = file_oxlint_directives(|| OxlintDirectives::read(&root(node), source));
            directives.suppresses(rules, anchored.clone())
        }) || suppress.biome.as_ref().is_some_and(|rules| {
            let suppressions =
                file_biome_suppressions(|| BiomeSuppressions::read(&root(node), source));
            suppressions.suppresses(rules, anchored.clone())
        })
    }

    /// `.cjs` and `.cts` (`.d.cts` declarations included) are CommonJS; `.mjs`, `.mts`, `.tsx`
    /// and other declaration files modules. A `.js`, `.jsx` or `.ts` file follows its package's
    /// `type`, as Node reads `.ts` files the same way as `.js` ones (type stripping), and is a
    /// module without a manifest (ESLint's default config, which upstream fixtures assume); where
    /// the manifest sets no valid `type`, Node's syntax detection decides.
    fn source_type(path: &str, package: Option<PackageType>) -> Option<SourceType> {
        let name = path.rsplit('/').next().unwrap_or(path);
        match name.rsplit_once('.').map(|(_, extension)| extension) {
            Some("cjs" | "cts") => Some(SourceType::CommonJs),
            Some("ts") if is_declaration_file(name) => Some(SourceType::Module),
            Some("js" | "jsx" | "ts") => match package {
                None | Some(PackageType::Module) => Some(SourceType::Module),
                Some(PackageType::CommonJs) => Some(SourceType::CommonJs),
                Some(PackageType::Unset) => None,
            },
            _ => Some(SourceType::Module),
        }
    }
}

impl Ecma {
    /// The module system of a file by its extension and its nearest `package.json`'s `type` alone
    /// (`SourceType::by_package_type`).
    pub(crate) fn source_type_by_package_type(
        path: &str,
        package: Option<PackageType>,
    ) -> SourceType {
        let name = path.rsplit('/').next().unwrap_or(path);
        match name.rsplit_once('.').map(|(_, extension)| extension) {
            Some("mjs" | "mts") => SourceType::Module,
            Some("cjs" | "cts") => SourceType::CommonJs,
            _ if package == Some(PackageType::Module) => SourceType::Module,
            _ => SourceType::CommonJs,
        }
    }
}

impl Adapter for Php {
    const FAMILY: Family = Family::Php;
    const LABEL: &'static str = "php";
    const TERMINATORS: Terminators = Terminators::Newlines;
    const PACKAGES: bool = false;
    const PACKAGE_MANIFESTS: bool = false;
    const JSX_RUNTIMES: bool = false;
    const DECLARATION_FILES: bool = true;
    const STRIPS_BOM: bool = false;
    const COMMENT_KINDS: &'static [&'static str] = &["comment"];
    const PREAMBLE_KINDS: &'static [&'static str] = &["php_tag"];
    const BODY_KINDS: &'static [&'static str] = &[];
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Self;
    type FormatArgs = Self;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        text
    }

    fn parse_input(source: &str) -> Cow<'_, str> {
        crate::code_rule_dsl::php_view::view(source)
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
        anchored: Range<usize>,
        _: (usize, usize),
    ) -> bool {
        let at = anchored.start;
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
    const PACKAGE_MANIFESTS: bool = false;
    const JSX_RUNTIMES: bool = false;
    const DECLARATION_FILES: bool = true;
    const STRIPS_BOM: bool = false;
    /// A template's `<!-- -->` (`comment`) and `{{-- --}}` (`blade_comment`); its embedded PHP's
    /// are read with PHP's family.
    const COMMENT_KINDS: &'static [&'static str] = &["comment", "blade_comment"];
    /// A `{{-- --}}` counts as a token of the template.
    const WALK_COMMENT_KINDS: &'static [&'static str] = &["comment"];
    const PARSED_BY_FORTE: bool = true;
    /// A template's `php_tag` is a whole `<?php … ?>` region, code rather than a preamble.
    const PREAMBLE_KINDS: &'static [&'static str] = &[];
    const BODY_KINDS: &'static [&'static str] = &[];
    type Scope = Php;
    type Value = Php;
    type Arguments = Php;
    type Calls = Self;
    type FormatArgs = Php;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        text
    }

    /// Forte parses a template (`blade::forte`), never tree-sitter.
    fn parse_input(source: &str) -> Cow<'_, str> {
        Cow::Borrowed(source)
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
        _: Range<usize>,
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

/// The module system of the file at `path` in the dialect labelled `label`
/// (`Adapter::source_type`); a module for a label no dialect has.
pub(crate) fn source_type(
    label: &str,
    path: &str,
    package: Option<PackageType>,
) -> Option<SourceType> {
    match super::dialect_by_label(label) {
        Some(dialect) => {
            with_adapter!(Family::of(dialect), A => A::source_type(path, package))
        }
        None => Some(SourceType::Module),
    }
}

/// Whether files of the dialect labelled `label` take their module from package markers
/// (`Adapter::PACKAGES`); `false` for a label no dialect has.
pub(crate) fn packages(label: &str) -> bool {
    super::dialect_by_label(label)
        .is_some_and(|dialect| with_adapter!(Family::of(dialect), A => A::PACKAGES))
}

/// Every family's `kinds`, each once: the walks ask per node they visit, so the union is gathered
/// once rather than per call.
fn every_family_kind(kinds: impl Fn(Family) -> &'static [&'static str]) -> Vec<&'static str> {
    let mut union = Vec::new();
    for kind in Family::ALL.into_iter().flat_map(kinds) {
        if !union.contains(kind) {
            union.push(*kind);
        }
    }
    union
}

static WALK_COMMENT_KINDS: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| every_family_kind(|family| with_adapter!(family, A => A::WALK_COMMENT_KINDS)));

static PAREN_KINDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    every_family_kind(
        |family| with_adapter!(family, A => <<A as Adapter>::Scope as ScopeRules>::PAREN_KINDS),
    )
});

/// Whether `kind` is a comment that a token or sibling walk passes over, in any family's grammar
/// (`Adapter::WALK_COMMENT_KINDS`). The matchers that walk (`compare`, `layout`) hold no family, and
/// a kind's name means a comment in every grammar that has it.
pub(crate) fn is_comment_kind(kind: &str) -> bool {
    WALK_COMMENT_KINDS.contains(&kind)
}

/// Whether `kind` is a parenthesized expression in any family's grammar
/// (`ScopeRules::PAREN_KINDS`), for the matchers that hold no family.
pub(crate) fn is_paren_kind(kind: &str) -> bool {
    PAREN_KINDS.contains(&kind)
}

/// Whether a file of `dialect` is parsed by Forte: its family's `Adapter::PARSED_BY_FORTE`.
pub(crate) fn parsed_by_forte(dialect: GrammarDialect) -> bool {
    with_adapter!(Family::of(dialect), A => A::PARSED_BY_FORTE)
}

/// Whether an ast query in `dialect` may use `importTarget`: its family's `Adapter::IMPORT_TARGETS`.
pub(crate) fn has_import_targets(dialect: GrammarDialect) -> bool {
    with_adapter!(Family::of(dialect), A => A::IMPORT_TARGETS)
}

/// What tree-sitter parses for a file of `dialect`: its family's `Adapter::parse_input`.
pub(crate) fn parse_input(dialect: GrammarDialect, source: &str) -> Cow<'_, str> {
    with_adapter!(Family::of(dialect), A => A::parse_input(source))
}

/// `dialect`'s ast-grep tree of `parse_input`, reading `source`'s text.
pub(crate) fn ast_grep(dialect: GrammarDialect, source: &str) -> AstGrep<StrDoc<GrammarDialect>> {
    match parse_input(dialect, source) {
        Cow::Borrowed(_) => dialect.ast_grep(source),
        Cow::Owned(input) => {
            let mut doc = StrDoc::new(&input, dialect);
            // The input has `source`'s length and character boundaries, so every node's range
            // reads `source`.
            doc.src = source.to_owned();
            AstGrep::doc(doc)
        }
    }
}

/// Whether files of the dialect labelled `label` read their nearest `package.json`
/// (`Adapter::PACKAGE_MANIFESTS`); `false` for a label no dialect has.
pub(crate) fn package_manifests(label: &str) -> bool {
    super::dialect_by_label(label)
        .is_some_and(|dialect| with_adapter!(Family::of(dialect), A => A::PACKAGE_MANIFESTS))
}

/// Whether files of the dialect labelled `label` are read past a leading byte order mark
/// (`Adapter::STRIPS_BOM`); `false` for a label no dialect has.
pub(crate) fn strips_bom(label: &str) -> bool {
    super::dialect_by_label(label)
        .is_some_and(|dialect| with_adapter!(Family::of(dialect), A => A::STRIPS_BOM))
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
