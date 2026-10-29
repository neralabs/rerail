//! The languages Rerail reads. Every question about a language asks here: which grammar dialects
//! a code-rule language spans, which dialect a file label selects, a dialect's tape tag, its node
//! kinds and fields (`KindCatalog`, which rules and tapes share), which semantic family serves a
//! grammar, and which capabilities a language supports. The language registry
//! (`languages/*.json`, `docs/language-packs.md`) is their one source: `build.rs` validates it
//! and generates `GrammarDialect`'s production variants, `DIALECTS`, `CodeRuleLanguage`,
//! `PLANNED_LANGUAGES`, `Family`, `Family::of`, `Family::honours` and `with_adapter!` from it
//! (`language_registry.rs` in `OUT_DIR`). The host routes files by the same registry's suffixes
//! (`SOURCE_LANGUAGES` in code-rule-languages.ts); the repository index receives them.
//!
//! Three layers:
//! - A **dialect** is one tree-sitter grammar (`GrammarDialect`): one variant per supported
//!   registry dialect (its `variant`), plus the test-only grammars listed here. Files are parsed
//!   by exactly one dialect.
//! - A **code-rule language** is what a rule's `language` names. It spans the dialects its rules run
//!   on; a sub-rule one dialect cannot express is `Never` there (`compile::dialect`). A
//!   `typescript` rule also runs on JavaScript files; a `javascript` rule runs on them alone.
//! - A **family** is the semantic implementation a dialect is read with: its scope table, value
//!   decoder, call arguments, suppression comments and line terminators. TypeScript, TSX and
//!   JavaScript share the ECMAScript family.
//!
//! Adding a language: add its registry entry (`planned` first), and, for a new family, its adapter
//! (`adapter`): one type implementing `Adapter`, whose associated types name the family's
//! implementation of each concern, or `Refused`. `with_adapter!`, generated from the registry's
//! families, dispatches every family question to it (the build fails until it exists). A rule that
//! names a planned language, or a capability its language has `not-supported-yet`, is refused as
//! `language-not-supported`; a `refused` capability is refused as invalid, never answered by
//! another family's code.

pub(crate) mod adapter;

use crate::code_rule_dsl::blade::forte::tree::{self as blade_tree, BladeLang};
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::package_manifest::{DEFAULT_NODE_RANGE, Manifest};
use crate::code_rule_dsl::semver::Range;
use ast_grep_core::{
    language::Language,
    matcher::{Pattern, PatternBuilder, PatternError},
    tree_sitter::{StrDoc, TSLanguage},
};
use ast_grep_language::{LanguageExt, SupportLang};
use std::borrow::Cow;
use std::sync::LazyLock;

impl std::fmt::Display for GrammarDialect {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Language for GrammarDialect {
    fn kind_to_id(&self, kind: &str) -> u16 {
        self.kind_id(kind)
    }

    fn field_to_id(&self, field: &str) -> Option<u16> {
        self.field_id(field)
    }

    fn expando_char(&self) -> char {
        match self.support() {
            Some(lang) => lang.expando_char(),
            None => BladeLang.expando_char(),
        }
    }

    fn pre_process_pattern<'q>(&self, query: &'q str) -> Cow<'q, str> {
        match self.support() {
            Some(language) => language.pre_process_pattern(query),
            None => BladeLang.pre_process_pattern(query),
        }
    }

    /// Blade's patterns are Forte trees (`blade::forte::tree`), whose kinds are numbered as Blade's
    /// tapes number them; every other dialect's are tree-sitter trees.
    fn build_pattern(&self, builder: &PatternBuilder) -> Result<Pattern, PatternError> {
        match self {
            Self::Blade => BladeLang.build_pattern(builder),
            _ => builder.build(|source| StrDoc::try_new(source, *self)),
        }
    }
}

impl LanguageExt for GrammarDialect {
    /// The grammar the registry names: one `build.rs` generates from a vendored grammar (Python,
    /// TypeScript, TSX and JavaScript, which carry patches: `grammars/tree-sitter-{python,
    /// typescript,javascript}/PATCHES.md`), else ast-grep's. ast-grep is built without its Python,
    /// TypeScript and JavaScript parsers, so `SupportLang` only lends these dialects its pattern
    /// syntax. Test-only dialects are ast-grep's. Blade has none: Rerail's port of Forte parses it
    /// (`blade::forte`).
    fn get_ts_language(&self) -> TSLanguage {
        self.ts_language()
            .unwrap_or_else(|| panic!("{self} is not parsed with tree-sitter"))
    }
}

impl GrammarDialect {
    /// The dialect's tree-sitter grammar; `None` for one Rerail parses itself (Blade).
    fn ts_language(self) -> Option<TSLanguage> {
        registry_grammar(self).or_else(|| self.support().map(|lang| lang.get_ts_language()))
    }
}

/// Where a dialect's kinds come from: a tree-sitter grammar's symbol table, or the kinds of
/// Blade's tree (`blade::forte::tree`, Rerail's port of Forte).
#[derive(Clone, Copy)]
enum OwnKinds {
    Grammar(&'static TSLanguage),
    Blade,
}

/// The kind id every grammar gives ERROR (tree-sitter's `ts_builtin_sym_error`); Blade's tree
/// shares it (`tape` records a template's lexer errors as ERROR leaves).
const ERROR_ID: u16 = u16::MAX;

/// The node kinds and fields of one grammar: a dialect's tree-sitter grammar, or for Blade its
/// tree's (`blade::forte::tree::KIND_NAMES` and `FIELD_NAMES`, ids from 1, ERROR aside). A tape
/// holds one grammar: a file's embedded program is a document of its own (`embed`), its kinds its
/// own grammar's. Rules and tapes read kinds and fields only here.
#[derive(Clone, Copy)]
pub(crate) struct KindCatalog {
    own: OwnKinds,
}

impl KindCatalog {
    /// The tree-sitter grammar the dialect's files are parsed with; `None` for Blade's.
    pub(crate) fn grammar(self) -> Option<&'static TSLanguage> {
        match self.own {
            OwnKinds::Grammar(grammar) => Some(grammar),
            OwnKinds::Blade => None,
        }
    }

    pub(crate) fn kind_count(self) -> usize {
        match self.own {
            OwnKinds::Grammar(grammar) => grammar.node_kind_count(),
            OwnKinds::Blade => blade_tree::KIND_NAMES.len(),
        }
    }

    /// The id of the named kind called `kind` (0 where there is none).
    pub(crate) fn kind_id(self, kind: &str) -> u16 {
        match self.own {
            OwnKinds::Grammar(grammar) => grammar.id_for_node_kind(kind, true),
            OwnKinds::Blade if kind == "ERROR" => ERROR_ID,
            OwnKinds::Blade => blade_tree::KIND_NAMES[..usize::from(blade_tree::FIRST_ANONYMOUS)]
                .iter()
                .skip(1)
                .position(|name| *name == kind)
                .map_or(0, |index| index as u16 + 1),
        }
    }

    /// The ids a rule's `kind` names: its own, except that `comment` in a Blade template names its
    /// comments of every kind (`<!-- -->`, `{{-- --}}`, `<!--[if ...]>` and `<!...>`).
    pub(crate) fn kind_ids(self, kind: &str) -> Vec<u16> {
        match self.own {
            OwnKinds::Blade if kind == "comment" => [
                "comment",
                "blade_comment",
                "conditional_comment",
                "bogus_comment",
            ]
            .into_iter()
            .map(|kind| self.kind_id(kind))
            .collect(),
            _ => vec![self.kind_id(kind)],
        }
    }

    pub(crate) fn kind_name(self, id: u16) -> Option<&'static str> {
        match self.own {
            OwnKinds::Grammar(grammar) => grammar.node_kind_for_id(id),
            OwnKinds::Blade if id == ERROR_ID => Some("ERROR"),
            OwnKinds::Blade => blade_tree::KIND_NAMES
                .get(usize::from(id))
                .copied()
                .filter(|name| !name.is_empty()),
        }
    }

    pub(crate) fn kind_is_named(self, id: u16) -> bool {
        match self.own {
            OwnKinds::Grammar(grammar) => grammar.node_kind_is_named(id),
            OwnKinds::Blade => id == ERROR_ID || (id != 0 && id < blade_tree::FIRST_ANONYMOUS),
        }
    }

    pub(crate) fn kind_is_visible(self, id: u16) -> bool {
        match self.own {
            OwnKinds::Grammar(grammar) => grammar.node_kind_is_visible(id),
            OwnKinds::Blade => id != 0 && usize::from(id) < blade_tree::KIND_NAMES.len(),
        }
    }

    pub(crate) fn kind_is_supertype(self, id: u16) -> bool {
        match self.own {
            OwnKinds::Grammar(grammar) => grammar.node_kind_is_supertype(id),
            OwnKinds::Blade => false,
        }
    }

    pub(crate) fn field_count(self) -> usize {
        match self.own {
            OwnKinds::Grammar(grammar) => grammar.field_count(),
            OwnKinds::Blade => blade_tree::FIELD_NAMES.len() - 1,
        }
    }

    /// The id of the field called `name`.
    pub(crate) fn field_id(self, name: &str) -> Option<u16> {
        match self.own {
            OwnKinds::Grammar(grammar) => grammar.field_id_for_name(name).map(|id| id.get()),
            OwnKinds::Blade => blade_tree::FIELD_NAMES
                .iter()
                .skip(1)
                .position(|field| *field == name)
                .map(|index| index as u16 + 1),
        }
    }

    pub(crate) fn field_name(self, id: u16) -> Option<&'static str> {
        match self.own {
            OwnKinds::Grammar(grammar) => grammar.field_name_for_id(id),
            OwnKinds::Blade => (id != 0)
                .then(|| blade_tree::FIELD_NAMES.get(usize::from(id)).copied())
                .flatten(),
        }
    }
}

impl GrammarDialect {
    /// The dialect's tree-sitter grammar, loaded once; `None` for Blade's.
    fn grammar(self) -> Option<&'static TSLanguage> {
        static GRAMMARS: std::sync::LazyLock<
            std::collections::HashMap<GrammarDialect, TSLanguage>,
        > = std::sync::LazyLock::new(|| {
            GrammarDialect::ALL
                .iter()
                .filter_map(|&dialect| Some((dialect, dialect.ts_language()?)))
                .collect()
        });
        GRAMMARS.get(&self)
    }

    /// The dialect's kinds and fields: Blade's tree's (Forte, the registry's `parser` `forte`),
    /// else its tree-sitter grammar's.
    pub(crate) fn kinds(self) -> KindCatalog {
        KindCatalog {
            own: match self.grammar() {
                Some(grammar) => OwnKinds::Grammar(grammar),
                None => OwnKinds::Blade,
            },
        }
    }

    pub(crate) fn kind_count(self) -> usize {
        self.kinds().kind_count()
    }

    pub(crate) fn kind_id(self, kind: &str) -> u16 {
        self.kinds().kind_id(kind)
    }

    pub(crate) fn kind_name(self, id: u16) -> Option<&'static str> {
        self.kinds().kind_name(id)
    }

    pub(crate) fn kind_is_named(self, id: u16) -> bool {
        self.kinds().kind_is_named(id)
    }

    pub(crate) fn kind_is_supertype(self, id: u16) -> bool {
        self.kinds().kind_is_supertype(id)
    }

    /// Every dialect, test-only grammars included.
    pub(crate) fn all() -> &'static [Self] {
        Self::ALL
    }

    pub(crate) fn field_id(self, name: &str) -> Option<u16> {
        self.kinds().field_id(name)
    }
}

/// A rule's `pattern`: its source, or a context and the kind of the node it selects there.
pub(crate) enum PatternSource<'a> {
    Plain(&'a str),
    Contextual { context: &'a str, selector: &'a str },
}

/// A rule's pattern compiled for `language`, the grammar of the document its clause reads (an
/// embedded clause's is its program's: `ast.embedded`).
pub(crate) fn rule_pattern(
    source: PatternSource<'_>,
    language: GrammarDialect,
) -> Result<Pattern, PatternError> {
    match source {
        PatternSource::Plain(text) => Pattern::try_new(text, language),
        PatternSource::Contextual { context, selector } => {
            Pattern::contextual(context, selector, language)
        }
    }
}

/// One grammar dialect: the file label hosts and the store use, its file suffixes (the host's
/// routing, `label_of_path`) and its persisted tape tag (never reused). The rows are the
/// registry's supported dialects (`DIALECTS`, generated).
struct Dialect {
    lang: GrammarDialect,
    label: &'static str,
    suffixes: &'static [&'static str],
    tape_tag: u8,
}

// `grammar_dialect!`, `DIALECTS`, `registry_grammar`, `CodeRuleLanguage` (`ALL`, `name`,
// `dialects`, `family`, `capability`), `PLANNED_LANGUAGES`, `Family` (`ALL`, `of`, `honours`) and
// `with_adapter!`.
include!(concat!(env!("OUT_DIR"), "/language_registry.rs"));

// `GrammarDialect`: the registry's supported dialects, then grammars only tests parse (tape round
// trips over other shapes, the transducer's grammar census), each with the ast-grep language that
// lends it pattern syntax. No file or code rule selects a test-only grammar.
grammar_dialect! {
    Java => SupportLang::Java,
    CSharp => SupportLang::CSharp,
    Cpp => SupportLang::Cpp,
    C => SupportLang::C,
    Bash => SupportLang::Bash,
    Ruby => SupportLang::Ruby,
    TestRust => SupportLang::Rust,
    Kotlin => SupportLang::Kotlin,
    Scala => SupportLang::Scala,
    Swift => SupportLang::Swift,
    Html => SupportLang::Html,
    Css => SupportLang::Css,
    Lua => SupportLang::Lua,
    Json => SupportLang::Json,
    Go => SupportLang::Go,
}

impl Via {
    /// The family of the files that embed a program through this embedding.
    pub(crate) fn host_family(self) -> Family {
        GrammarDialect::ALL
            .iter()
            .find(|dialect| dialect.embeds().iter().any(|&(via, _)| via == self))
            .map(|&dialect| Family::of(dialect))
            .expect("an embedding has a host dialect")
    }
}

/// The family `Family::of` reads a test-only grammar with.
#[cfg(test)]
const TEST_DIALECT_FAMILY: Family = Family::Ecma;

/// Grammars only tests parse (tape round trips over other shapes); no file or code rule selects them.
#[cfg(test)]
const TEST_TAPE_TAGS: &[(GrammarDialect, u8)] = &[
    (GrammarDialect::Scala, 4),
    (GrammarDialect::Go, 5),
    (GrammarDialect::Html, 250),
];

/// The embeddings of the test-only grammars (`GrammarDialect::embeds`): an HTML file's scripts and
/// event handlers, one TypeScript, TSX or JavaScript program (`embed::TestScript`), so tests
/// encode, restore and read an embedded ECMAScript program as an embedding language's would be.
#[cfg(test)]
const fn test_embeds(dialect: GrammarDialect) -> &'static [(Via, GrammarDialect)] {
    match dialect {
        GrammarDialect::Html => &[
            (Via::TestScript, GrammarDialect::TypeScript),
            (Via::TestScript, GrammarDialect::Tsx),
            (Via::TestScript, GrammarDialect::JavaScript),
        ],
        _ => &[],
    }
}

fn dialect(lang: GrammarDialect) -> Option<&'static Dialect> {
    DIALECTS.iter().find(|dialect| dialect.lang == lang)
}

/// The dialect a file label names (`FileInput.language`, the store's file language).
pub(crate) fn dialect_by_label(label: &str) -> Option<GrammarDialect> {
    DIALECTS
        .iter()
        .find(|dialect| dialect.label == label)
        .map(|dialect| dialect.lang)
}

/// A dialect's file label.
pub(crate) fn label(lang: GrammarDialect) -> Option<&'static str> {
    dialect(lang).map(|dialect| dialect.label)
}

/// The file label a path's suffix selects, as the host's table does for dialect files: the
/// longest matching suffix.
pub(crate) fn label_of_path(path: &str) -> Option<&'static str> {
    let path = path.to_ascii_lowercase();
    DIALECTS
        .iter()
        .flat_map(|dialect| dialect.suffixes.iter().map(move |suffix| (suffix, dialect)))
        .filter(|(suffix, _)| path.ends_with(**suffix))
        .max_by_key(|(suffix, _)| suffix.len())
        .map(|(_, dialect)| dialect.label)
}

/// A dialect's persisted tape tag.
pub(crate) fn tape_tag(lang: GrammarDialect) -> Option<u8> {
    let tag = dialect(lang).map(|dialect| dialect.tape_tag);
    #[cfg(test)]
    let tag = tag.or_else(|| {
        TEST_TAPE_TAGS
            .iter()
            .find(|(test, _)| *test == lang)
            .map(|(_, tag)| *tag)
    });
    tag
}

/// The dialect a persisted tape tag names.
pub(crate) fn from_tape_tag(tag: u8) -> Option<GrammarDialect> {
    let lang = DIALECTS
        .iter()
        .find(|dialect| dialect.tape_tag == tag)
        .map(|dialect| dialect.lang);
    #[cfg(test)]
    let lang = lang.or_else(|| {
        TEST_TAPE_TAGS
            .iter()
            .find(|(_, test)| *test == tag)
            .map(|(lang, _)| *lang)
    });
    lang
}

/// The stable code of a compile error for a rule whose language, or a capability it reads, the
/// registry marks not supported yet. Hosts keep such a rule stored but inactive.
pub(crate) const LANGUAGE_NOT_SUPPORTED: &str = "language-not-supported";

/// A `language-not-supported` compile error: its message starts with the code.
pub(crate) fn language_not_supported(message: impl std::fmt::Display) -> napi::Error {
    napi::Error::new(
        napi::Status::InvalidArg,
        format!("{LANGUAGE_NOT_SUPPORTED}: {message}"),
    )
}

impl CodeRuleLanguage {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.name() == name)
    }

    /// The language a rule's `language` at `path` names. A language the registry plans is
    /// `language-not-supported`; any other unknown name is `unknown`'s error.
    pub(crate) fn read(
        name: &str,
        path: &str,
        unknown: impl FnOnce() -> napi::Error,
    ) -> napi::Result<Self> {
        Self::parse(name).ok_or_else(|| {
            if PLANNED_LANGUAGES.contains(&name) {
                language_not_supported(format!("{path}: `{name}` is not supported yet"))
            } else {
                unknown()
            }
        })
    }
}

/// Whether a language reads a capability, as its registry entry declares.
/// The generated `CodeRuleLanguage::capability` constructs the variants the registry uses; one no
/// supported language declares today is still handled wherever a rule is compiled.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Support {
    Supported,
    /// The language has no such thing, or no plan for it: a rule using it is invalid.
    Refused,
    /// Planned: a rule using it is `language-not-supported`, kept but inactive.
    NotSupportedYet,
}

/// A semantic capability a language may support (the registry's `capabilities` keys).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Semantic {
    /// Scope table, bindings and usage: `resolves`, `binds`.
    Scope,
    /// Value decoding and comparison: `value`, `compare`.
    Value,
    /// A call's arguments: `arguments`.
    Arguments,
    /// The workspace call graph: `calls`.
    Calls,
    /// Format strings' arguments: `formatArgs`.
    FormatArgs,
}

/// The suppressions a family honours (the registry's `suppressions`), one kind per linter
/// (`suppress`'s table). PHP honours three linters' (a PHP project may run any of them); Blade
/// templates none: no linter reads a template's `{{-- --}}` comments for directives.
pub(crate) use crate::code_rule_dsl::suppress::Suppression;

/// The ECMAScript source form that structural `when` gates and scope construction read.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum SourceType {
    /// Top-level declarations join the global scope, and top-level `this` is the global object.
    Script,
    /// A module scope; top-level `this` is `undefined`.
    Module,
    /// A function-like module scope with CommonJS globals.
    CommonJs,
}

impl SourceType {
    /// The module system of the file at `path` in the dialect labelled `label`, whose nearest
    /// `package.json` has `package` as its type (`None`: no manifest), for a query that leaves
    /// `sourceType` to the file: its family's `Adapter::source_type`. `None` where Node reads the
    /// syntax instead (`has_module_syntax`).
    pub(crate) fn of_file(label: &str, path: &str, package: Option<PackageType>) -> Option<Self> {
        adapter::source_type(label, path, package)
    }
}

/// The `type` of an ECMA file's nearest `package.json` (its `package`'s `type`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum PackageType {
    /// `"type": "module"`.
    Module,
    /// `"type": "commonjs"`.
    CommonJs,
    /// No valid `type`: none, another value, or a manifest that cannot be read.
    Unset,
}

impl PackageType {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "module" => Some(Self::Module),
            "commonjs" => Some(Self::CommonJs),
            "unset" => Some(Self::Unset),
            _ => None,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::CommonJs => "commonjs",
            Self::Unset => "unset",
        }
    }
}

/// The CommonJS module wrapper's parameters, which a module may not redeclare lexically.
const COMMONJS_WRAPPER_NAMES: [&str; 5] =
    ["require", "module", "exports", "__dirname", "__filename"];

/// Node's module syntax detection (22.7 and later) for a file no `type` decides: a top-level
/// static `import` or `export` declaration that type stripping keeps, `import.meta` anywhere, a
/// top-level `await` (outside functions and class bodies, `for await` included), or a top-level
/// `const`, `let` or `class` declaring one of the CommonJS wrapper's names, each of which only
/// parses as a module.
pub(crate) fn has_module_syntax<D: ast_grep_core::Doc>(root: &ast_grep_core::Node<'_, D>) -> bool {
    root.children()
        .any(|child| is_module_declaration(&child) || redeclares_commonjs_wrapper(&child))
        || (root.text().contains("meta")
            && root
                .dfs()
                .any(|node| node.kind() == "meta_property" && node.text().starts_with("import")))
        || (root.text().contains("await") && has_top_level_await(root))
}

/// Whether a top-level statement is an `import` or `export` declaration that Node's type
/// stripping leaves in place. It erases `import type`, `export type`, and exported interfaces,
/// type aliases and `declare` declarations. `import x = require(…)` and `export =` are
/// TypeScript's CommonJS forms, which Node refuses to strip.
fn is_module_declaration<D: ast_grep_core::Doc>(statement: &ast_grep_core::Node<'_, D>) -> bool {
    match statement.kind().as_ref() {
        "import_statement" => !statement
            .children()
            .any(|child| matches!(child.kind().as_ref(), "type" | "import_require_clause")),
        "export_statement" => {
            !statement
                .children()
                .any(|child| matches!(child.kind().as_ref(), "type" | "="))
                && !statement.field("declaration").is_some_and(|declaration| {
                    matches!(
                        declaration.kind().as_ref(),
                        "interface_declaration" | "type_alias_declaration" | "ambient_declaration"
                    )
                })
        }
        _ => false,
    }
}

/// Whether `node` holds an `await` that is not inside a function or a class body. A class
/// member's computed name and decorators run where the class is defined, so they are read; its
/// body, parameters and field initializer are not. Walks an explicit stack, so a deeply nested
/// expression cannot exhaust the native stack.
fn has_top_level_await<D: ast_grep_core::Doc>(node: &ast_grep_core::Node<'_, D>) -> bool {
    let mut pending = node.children().collect::<Vec<_>>();
    while let Some(child) = pending.pop() {
        match child.kind().as_ref() {
            "await_expression" => return true,
            "for_in_statement" if child.children().any(|token| token.kind() == "await") => {
                return true;
            }
            "function_declaration"
            | "generator_function_declaration"
            | "function_expression"
            | "function"
            | "generator_function"
            | "arrow_function" => {}
            "method_definition" => pending.extend(evaluated_where_defined(&child)),
            "class_body" => {
                for member in child.children() {
                    if matches!(
                        member.kind().as_ref(),
                        "method_definition" | "field_definition" | "public_field_definition"
                    ) {
                        pending.extend(evaluated_where_defined(&member));
                    } else if member.kind() == "decorator" {
                        pending.push(member);
                    }
                }
            }
            _ => pending.extend(child.children()),
        }
    }
    false
}

/// The parts of a method or class field that run where it is defined: its name (a computed one's
/// expression) and its decorators.
fn evaluated_where_defined<'r, D: ast_grep_core::Doc>(
    member: &ast_grep_core::Node<'r, D>,
) -> Vec<ast_grep_core::Node<'r, D>> {
    member
        .field("name")
        .or_else(|| member.field("property"))
        .into_iter()
        .chain(
            member
                .children()
                .filter(|child| child.kind() == "decorator"),
        )
        .collect()
}

/// Whether a top-level statement is a `const`, `let` or `class` declaration binding a CommonJS
/// wrapper name (destructured names included). A TypeScript `abstract class` is a class once
/// types are stripped; a `declare` one is erased (`ambient_declaration`), so it binds nothing.
fn redeclares_commonjs_wrapper<D: ast_grep_core::Doc>(
    statement: &ast_grep_core::Node<'_, D>,
) -> bool {
    match statement.kind().as_ref() {
        "lexical_declaration" => statement
            .children()
            .filter(|declarator| declarator.kind() == "variable_declarator")
            .filter_map(|declarator| declarator.field("name"))
            .any(|name| binds_commonjs_wrapper(&name)),
        "class_declaration" | "abstract_class_declaration" => statement
            .field("name")
            .is_some_and(|name| COMMONJS_WRAPPER_NAMES.contains(&name.text().as_ref())),
        _ => false,
    }
}

/// Whether a binding pattern binds a CommonJS wrapper name: its identifiers in binding position,
/// not a default value's or a renamed property's key. Walks an explicit stack.
fn binds_commonjs_wrapper<D: ast_grep_core::Doc>(pattern: &ast_grep_core::Node<'_, D>) -> bool {
    let mut pending = vec![pattern.clone()];
    while let Some(pattern) = pending.pop() {
        match pattern.kind().as_ref() {
            "identifier" | "shorthand_property_identifier_pattern"
                if COMMONJS_WRAPPER_NAMES.contains(&pattern.text().as_ref()) =>
            {
                return true;
            }
            "assignment_pattern" | "object_assignment_pattern" => {
                pending.extend(pattern.field("left"));
            }
            "pair_pattern" => pending.extend(pattern.field("value")),
            "object_pattern" | "array_pattern" | "rest_pattern" => {
                pending.extend(pattern.children());
            }
            _ => {}
        }
    }
    false
}

/// The JSX runtime of an ECMA file (`file.jsxRuntime`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JsxRuntime {
    React,
    /// Another JSX library's.
    Other,
    Unknown,
}

impl JsxRuntime {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "react" => Some(Self::React),
            "other" => Some(Self::Other),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

/// The JSX libraries whose `h` or `createElement` a file that imports one renders its JSX with.
const JSX_FACTORY_SOURCES: [&str; 7] = [
    "@stencil/core",
    "preact",
    "vue",
    "solid-js",
    "solid-js/h",
    "hyperapp",
    "mithril",
];

/// `@jsxImportSource` packages that are React's JSX runtime or wrap it (Emotion's and Theme UI's
/// `css`/`sx` props); a subpath (`react/jsx-runtime`) counts as its package.
const REACT_JSX_IMPORT_SOURCES: [&str; 4] =
    ["react", "@emotion/react", "theme-ui", "@theme-ui/core"];
/// `@jsxImportSource` packages of JSX libraries other than React; a subpath counts as its package.
const OTHER_JSX_IMPORT_SOURCES: [&str; 9] = [
    "preact",
    "solid-js",
    "vue",
    "@stencil/core",
    "hono",
    "@builder.io/qwik",
    "inferno",
    "mithril",
    "hyperapp",
];

static JSX_IMPORT_SOURCE_PRAGMA: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"@jsxImportSource\s+(\S+)").expect("the pragma pattern compiles")
});
static JSX_PRAGMA: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"@jsx\s+(\S+)").expect("the pragma pattern compiles"));

/// Whether `source` is one of `packages` or a subpath of one.
fn names_package(packages: &[&str], source: &str) -> bool {
    packages.iter().any(|package| {
        source
            .strip_prefix(package)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

/// The JSX runtime a JSX import source (a `@jsxImportSource` pragma, a `jsxImportSource`
/// compiler option) names: React or a React wrapper, another JSX library, or neither (`None`).
pub(crate) fn jsx_import_source_runtime(source: &str) -> Option<JsxRuntime> {
    if names_package(&REACT_JSX_IMPORT_SOURCES, source) {
        Some(JsxRuntime::React)
    } else if names_package(&OTHER_JSX_IMPORT_SOURCES, source) {
        Some(JsxRuntime::Other)
    } else {
        None
    }
}

/// The JSX runtime an ECMA file's own text names: a `@jsxImportSource` pragma naming React or a
/// React wrapper (React's) or another JSX library (`Other`), else a `@jsx` pragma naming `React` or
/// `React.…` (React's), each from the first comment that has one, read in the comment's text
/// without its `//`, `/*` and `*/` (Babel's `comment.value`), as eslint-plugin-react reads its
/// pragma; else a top-level static import of `h` or `createElement` from another JSX library.
/// A pragma naming anything else is no evidence either way (Emotion's `@jsx jsx` renders React), so
/// `None` where the text names no runtime, and the file's manifest decides
/// (`Manifest::jsx_runtime`).
pub(crate) fn jsx_runtime_in_source<D: ast_grep_core::Doc>(
    root: &ast_grep_core::Node<'_, D>,
) -> Option<JsxRuntime> {
    if root.text().contains("@jsx") {
        let comments = root
            .dfs()
            .filter(|node| node.kind() == "comment")
            .map(|node| {
                let text = node.text();
                let value = match text.strip_prefix("//") {
                    Some(line) => line,
                    None => text
                        .strip_prefix("/*")
                        .map_or(&*text, |block| block.strip_suffix("*/").unwrap_or(block)),
                };
                value.to_owned()
            })
            .collect::<Vec<_>>();
        let pragma = |pattern: &regex::Regex| {
            comments
                .iter()
                .find_map(|comment| Some(pattern.captures(comment)?.get(1)?.as_str().to_owned()))
        };
        if let Some(source) = pragma(&JSX_IMPORT_SOURCE_PRAGMA) {
            if let Some(runtime) = jsx_import_source_runtime(&source) {
                return Some(runtime);
            }
        } else if pragma(&JSX_PRAGMA)
            .is_some_and(|factory| factory == "React" || factory.starts_with("React."))
        {
            return Some(JsxRuntime::React);
        }
    }
    let unquoted = |text: &str| text.trim_matches(['"', '\'']).to_owned();
    root.children()
        .filter(|child| child.kind() == "import_statement")
        .any(|statement| {
            statement.field("source").is_some_and(|source| {
                JSX_FACTORY_SOURCES.contains(&unquoted(&source.text()).as_str())
            }) && statement.dfs().any(|node| {
                node.kind() == "import_specifier"
                    && node.field("name").is_some_and(|name| {
                        matches!(unquoted(&name.text()).as_str(), "h" | "createElement")
                    })
            })
        })
        .then_some(JsxRuntime::Other)
}

/// An ECMA query's `file.package`: conditions on the file's nearest `package.json`. `bin`,
/// `dependsOn` and `path` need one, so a file without one fails them, `bin: false` included;
/// `nodeVersion` reads eslint-plugin-n's default range there.
#[derive(Debug)]
pub(crate) struct PackageCondition {
    /// Whether the file is one of the manifest's `bin` targets (`Manifest::is_bin`).
    pub(crate) bin: Option<bool>,
    /// The range the file's configured Node range must be a subset of.
    pub(crate) node_version: Option<Range>,
    /// Names one of which the manifest must list as a dependency of any kind.
    pub(crate) depends_on: Option<Vec<String>>,
    /// Over the file's path relative to the manifest's folder.
    pub(crate) path: Option<TextCondition>,
    /// Whether the file is in its Next app's server module graph (`Manifest::in_server_graph`).
    pub(crate) server_graph: Option<bool>,
}

impl PackageCondition {
    fn holds(&self, path: &str, manifest: Option<&Manifest>) -> bool {
        self.bin
            .is_none_or(|bin| manifest.is_some_and(|manifest| manifest.is_bin(path) == bin))
            && self.node_version.as_ref().is_none_or(|range| {
                manifest.map_or_else(
                    || DEFAULT_NODE_RANGE.is_subset_of(range),
                    |manifest| manifest.node_range_is_subset_of(range),
                )
            })
            && self
                .depends_on
                .as_ref()
                .is_none_or(|names| manifest.is_some_and(|manifest| manifest.depends_on(names)))
            && self.path.as_ref().is_none_or(|condition| {
                manifest
                    .and_then(|manifest| manifest.relative_path(path))
                    .is_some_and(|relative| condition.holds(relative))
            })
            && self.server_graph.is_none_or(|member| {
                manifest.and_then(|manifest| manifest.in_server_graph(path)) == Some(member)
            })
    }
}

/// An ast query's `file` condition: every key set must hold, or the query does not apply to the
/// file.
#[derive(Debug)]
pub(crate) struct FileCondition {
    /// Over the path the host sends.
    pub(crate) path: Option<TextCondition>,
    /// TypeScript: whether the path names a declaration file.
    pub(crate) declaration: Option<bool>,
    /// Python: over the dotted module path.
    pub(crate) module: Option<TextCondition>,
    /// Python: over the module path's last segment.
    pub(crate) module_name: Option<TextCondition>,
    /// Python: over the first segment, only in a package.
    pub(crate) package: Option<TextCondition>,
    /// Python: whether the host supplied a module path.
    pub(crate) in_package: Option<bool>,
    /// TypeScript: the file's nearest `package.json` (`file.package`).
    pub(crate) manifest: Option<PackageCondition>,
    /// TypeScript: the JSX runtimes the file's may be one of. Its text can decide it, so it is
    /// checked once the file is read (`run_clause`), not by `holds`.
    pub(crate) jsx_runtime: Option<Vec<JsxRuntime>>,
}

/// A text condition: `regex` (unanchored, as ast-grep's) must match, `notRegex` must not.
#[derive(Debug)]
pub(crate) struct TextCondition {
    pub(crate) regex: Option<regex::Regex>,
    pub(crate) not_regex: Option<regex::Regex>,
}

impl TextCondition {
    fn holds(&self, text: &str) -> bool {
        self.regex.as_ref().is_none_or(|regex| regex.is_match(text))
            && self
                .not_regex
                .as_ref()
                .is_none_or(|regex| !regex.is_match(text))
    }
}

impl FileCondition {
    /// Whether a file at `path` with this module path and nearest `package.json` satisfies the
    /// condition, `jsx_runtime` aside. A module path's text condition needs a module path
    /// (`package` one in a package), `notRegex` too.
    pub(crate) fn holds(
        &self,
        path: &str,
        module: Option<&ModulePath>,
        manifest: Option<&Manifest>,
    ) -> bool {
        self.path
            .as_ref()
            .is_none_or(|condition| condition.holds(path))
            && self
                .declaration
                .is_none_or(|declaration| is_declaration_file(path) == declaration)
            && self.module.as_ref().is_none_or(|condition| {
                module.is_some_and(|module| condition.holds(&module.text()))
            })
            && self
                .module_name
                .as_ref()
                .is_none_or(|condition| module.is_some_and(|module| condition.holds(module.last())))
            && self.package.as_ref().is_none_or(|condition| {
                module.is_some_and(|module| module.in_package() && condition.holds(module.first()))
            })
            && self
                .in_package
                .is_none_or(|in_package| module.is_some_and(ModulePath::in_package) == in_package)
            && self
                .manifest
                .as_ref()
                .is_none_or(|condition| condition.holds(path, manifest))
    }
}

/// typescript-eslint 8.70.1's declaration-file predicate.
pub(crate) fn is_declaration_file(path: &str) -> bool {
    let path = path.to_lowercase();
    [".d.ts", ".d.mts", ".d.cts"]
        .iter()
        .any(|extension| path.ends_with(extension))
        || (path.ends_with(".ts") && path.find(".d.").is_some_and(|at| at + 6 <= path.len()))
}

/// Ruff's source-type predicate for Python stub files.
pub(crate) fn is_stub_file(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|extension| extension == "pyi")
}

/// Native symbol ids of every named kind called one of `names`.
pub(crate) fn kind_ids(
    language: GrammarDialect,
    names: &[&str],
) -> std::collections::BTreeSet<u16> {
    (0..language.kind_count() as u16)
        .filter(|&id| {
            language.kind_is_named(id)
                && language
                    .kind_name(id)
                    .is_some_and(|name| names.contains(&name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dialect_round_trips_its_label_suffixes_and_tape_tag() {
        for dialect in DIALECTS {
            assert_eq!(dialect_by_label(dialect.label), Some(dialect.lang));
            assert_eq!(from_tape_tag(dialect.tape_tag), Some(dialect.lang));
            for suffix in dialect.suffixes {
                assert_eq!(label_of_path(&format!("a/b{suffix}")), Some(dialect.label));
            }
        }
        let mut tags = DIALECTS
            .iter()
            .map(|dialect| dialect.tape_tag)
            .chain(TEST_TAPE_TAGS.iter().map(|(_, tag)| *tag))
            .collect::<Vec<_>>();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), DIALECTS.len() + TEST_TAPE_TAGS.len());
    }

    #[test]
    fn every_code_rule_dialect_is_a_file_dialect_of_its_family() {
        for language in CodeRuleLanguage::ALL {
            assert_eq!(CodeRuleLanguage::parse(language.name()), Some(language));
            for &lang in language.dialects() {
                assert!(label(lang).is_some(), "{lang:?}");
                assert_eq!(Family::of(lang), language.family());
            }
        }
    }

    /// Blade's kinds and fields are its tree's own (`blade::forte::tree`), as the tape encodes
    /// them: PHP's are its embedded program's, a document of their own. ERROR is a named kind of
    /// Blade's too (a template's lexer errors).
    #[test]
    fn blade_names_its_trees_kinds_and_fields_only() {
        let blade = GrammarDialect::Blade.kinds();
        assert!(blade.grammar().is_none());
        assert_eq!(blade.kind_count(), blade_tree::KIND_NAMES.len());
        for (id, name) in blade_tree::KIND_NAMES.iter().enumerate().skip(1) {
            let id = u16::try_from(id).unwrap();
            assert_eq!(blade.kind_name(id), Some(*name), "{id}");
            assert_eq!(blade.kind_is_named(id), id < blade_tree::FIRST_ANONYMOUS);
            if id < blade_tree::FIRST_ANONYMOUS {
                assert_eq!(blade.kind_id(name), id, "{name}");
            }
        }
        assert_eq!(blade.kind_name(u16::MAX), Some("ERROR"));
        assert_eq!(blade.kind_id("ERROR"), u16::MAX);
        assert!(blade.kind_is_named(u16::MAX));
        for (id, name) in blade_tree::FIELD_NAMES.iter().enumerate().skip(1) {
            let id = u16::try_from(id).unwrap();
            assert_eq!(blade.field_id(name), Some(id), "{name}");
            assert_eq!(blade.field_name(id), Some(*name));
        }
        assert_eq!(blade.field_count(), blade_tree::FIELD_NAMES.len() - 1);
        assert_eq!(blade.kind_id("variable_name"), 0);
        assert_eq!(blade.field_id("condition"), None);
    }

    #[test]
    fn a_planned_language_is_not_supported_yet_and_an_unknown_one_is_invalid() {
        let unknown = || napi::Error::new(napi::Status::InvalidArg, "unknown".to_owned());
        for planned in PLANNED_LANGUAGES {
            assert!(CodeRuleLanguage::parse(planned).is_none(), "{planned}");
            let error = CodeRuleLanguage::read(planned, "/language", unknown).unwrap_err();
            assert_eq!(
                error.reason,
                format!("language-not-supported: /language: `{planned}` is not supported yet")
            );
        }
        assert_eq!(TEST_PLANNED_LANGUAGES, ["exemplar"]);
        assert!(PLANNED_LANGUAGES.contains(&"exemplar"));
        assert_eq!(
            CodeRuleLanguage::read("exemplar", "/language", unknown)
                .unwrap_err()
                .reason,
            "language-not-supported: /language: `exemplar` is not supported yet"
        );
        assert_eq!(
            CodeRuleLanguage::read("ruby", "/language", unknown)
                .unwrap_err()
                .reason,
            "unknown"
        );
        assert_eq!(
            CodeRuleLanguage::read("php", "/language", unknown).unwrap(),
            CodeRuleLanguage::Php
        );
    }

    /// The host reads the registry through `src/engine/code-rule/code-rule-language-registry.generated.ts`, which a script
    /// generates; the engine reads it through `build.rs`. Both must read the same registry.
    #[test]
    fn the_host_registry_module_matches_the_engine_tables() {
        let module =
            include_str!("../../../src/engine/code-rule/code-rule-language-registry.generated.ts");
        let start = module.find("/* registry */").expect("registry start") + 14;
        let end = module.find("/* end registry */").expect("registry end");
        let entries: Vec<serde_json::Value> =
            serde_json::from_str(&module[start..end]).expect("registry JSON");
        let text = |value: &serde_json::Value| value.as_str().expect("string").to_owned();
        let supported = entries
            .iter()
            .filter(|entry| entry["status"] == "supported")
            .collect::<Vec<_>>();
        let planned = entries
            .iter()
            .filter(|entry| entry["status"] == "planned")
            .map(|entry| text(&entry["id"]))
            .collect::<Vec<_>>();
        // Tests also plan the languages only tests see; the host module lists the registry's own.
        assert_eq!(
            PLANNED_LANGUAGES
                .iter()
                .filter(|id| !TEST_PLANNED_LANGUAGES.contains(id))
                .copied()
                .collect::<Vec<_>>(),
            planned
        );
        assert_eq!(
            supported
                .iter()
                .map(|entry| text(&entry["id"]))
                .collect::<Vec<_>>(),
            CodeRuleLanguage::ALL.map(CodeRuleLanguage::name)
        );
        let rows = supported
            .iter()
            .flat_map(|entry| entry["dialects"].as_array().expect("dialects"))
            .map(|dialect| {
                let suffixes = dialect["suffixes"]
                    .as_array()
                    .expect("suffixes")
                    .iter()
                    .map(text)
                    .collect::<Vec<_>>();
                (
                    text(&dialect["label"]),
                    suffixes,
                    dialect["tapeTag"].as_u64(),
                )
            })
            .collect::<Vec<_>>();
        let native = DIALECTS
            .iter()
            .map(|dialect| {
                let suffixes = dialect.suffixes.iter().map(|s| (*s).to_owned()).collect();
                (
                    dialect.label.to_owned(),
                    suffixes,
                    Some(u64::from(dialect.tape_tag)),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(rows, native);
        let semantics = [
            ("scope", Semantic::Scope),
            ("value", Semantic::Value),
            ("arguments", Semantic::Arguments),
            ("calls", Semantic::Calls),
            ("formatArgs", Semantic::FormatArgs),
        ];
        // Every kind's `suppress` key is its registry name.
        let suppressions = Suppression::ALL.iter().map(|&kind| (kind.key(), kind));
        for (entry, language) in supported.iter().zip(CodeRuleLanguage::ALL) {
            let runs_on = entry["runsOn"].as_array().map_or_else(
                || {
                    entry["dialects"]
                        .as_array()
                        .expect("dialects")
                        .iter()
                        .map(|dialect| text(&dialect["label"]))
                        .collect::<Vec<_>>()
                },
                |labels| labels.iter().map(text).collect(),
            );
            let dialects = language
                .dialects()
                .iter()
                .map(|&lang| label(lang).expect("label").to_owned())
                .collect::<Vec<_>>();
            assert_eq!(runs_on, dialects, "{language:?}");
            let capabilities = entry["capabilities"].as_object().expect("capabilities");
            assert_eq!(capabilities.len(), semantics.len(), "{language:?}");
            for (name, semantic) in semantics {
                let support = match language.capability(semantic) {
                    Support::Supported => "supported",
                    Support::Refused => "refused",
                    Support::NotSupportedYet => "not-supported-yet",
                };
                assert_eq!(capabilities[name], support, "{language:?} {name}");
            }
            let listed = entry["suppressions"]
                .as_array()
                .expect("suppressions")
                .iter()
                .map(text)
                .collect::<Vec<_>>();
            for (name, suppression) in suppressions.clone() {
                assert_eq!(
                    language.family().honours(suppression),
                    listed.iter().any(|listed| listed == name),
                    "{language:?} {name}"
                );
            }
        }
    }

    #[test]
    fn every_dialect_grammar_fits_the_tape() {
        for dialect in DIALECTS {
            assert!(
                crate::code_rule_dsl::tape::Grammar::new(dialect.lang).is_ok(),
                "{:?}",
                dialect.lang
            );
        }
    }

    /// TypeScript parses a postfix `!` inside the member access chain, so it asserts its operand,
    /// never a binary, unary or argumentless `new` expression before it (the vendored grammar's
    /// patch, `grammars/tree-sitter-typescript/PATCHES.md`; tree-sitter-typescript#299).
    #[test]
    fn non_null_assertion_binds_to_its_operand() {
        let source = "a - b!;\nif (n < x!.y) {}\nc && n < q!.end;\n-u!;\nnew A!();\n\
                      new B()!.c;\nd!!.e;\n";
        let expected = [
            ("b!", "binary_expression"),
            ("x!", "member_expression"),
            ("q!", "member_expression"),
            ("u!", "unary_expression"),
            ("A!", "new_expression"),
            ("new B()!", "member_expression"),
            ("d!!", "member_expression"),
            ("d!", "non_null_expression"),
        ];
        for dialect in [GrammarDialect::TypeScript, GrammarDialect::Tsx] {
            let root = dialect.ast_grep(source);
            assert!(!root.root().get_inner_node().has_error(), "{dialect:?}");
            let assertions = root
                .root()
                .dfs()
                .filter(|node| node.kind() == "non_null_expression")
                .map(|node| {
                    let parent = node.parent().expect("an assertion has a parent");
                    (node.text().into_owned(), parent.kind().into_owned())
                })
                .collect::<Vec<_>>();
            let expected = expected
                .iter()
                .map(|&(text, parent)| (text.to_owned(), parent.to_owned()))
                .collect::<Vec<_>>();
            assert_eq!(assertions, expected, "{dialect:?}");
        }
    }

    /// TypeScript parses type arguments after a member expression only, so a prefix or binary
    /// operator before a generic call or an instantiation applies to it, never becomes part of its
    /// callee; where comparisons could also read the input, a generic call is read before `(`
    /// (Patch 2 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn type_arguments_follow_a_primary_expression() {
        let source = "!q<H>('x');\nawait f<T>();\n-b<T>(c);\ntypeof d<T>(c);\nvoid e<T>(c);\n\
                      a + g<T>(c);\nx = a < b > (c);\ny = a < b || c > (d);\n!h.i<T>(c)!.j;\n\
                      k + l<T>;\n";
        let expected = [
            ("q<H>('x')", "unary_expression"),
            ("f<T>()", "await_expression"),
            ("b<T>(c)", "unary_expression"),
            ("d<T>(c)", "unary_expression"),
            ("e<T>(c)", "unary_expression"),
            ("g<T>(c)", "binary_expression"),
            ("a < b > (c)", "assignment_expression"),
            ("h.i<T>(c)", "non_null_expression"),
            ("l<T>", "binary_expression"),
        ];
        for dialect in [GrammarDialect::TypeScript, GrammarDialect::Tsx] {
            let root = dialect.ast_grep(source);
            assert!(!root.root().get_inner_node().has_error(), "{dialect:?}");
            let generic = root
                .root()
                .dfs()
                .filter(|node| {
                    node.kind() == "instantiation_expression"
                        || (node.kind() == "call_expression"
                            && node.field("type_arguments").is_some())
                })
                .map(|node| {
                    let parent = node.parent().expect("an expression has a parent");
                    (node.text().into_owned(), parent.kind().into_owned())
                })
                .collect::<Vec<_>>();
            let expected = expected
                .iter()
                .map(|&(text, parent)| (text.to_owned(), parent.to_owned()))
                .collect::<Vec<_>>();
            assert_eq!(generic, expected, "{dialect:?}");
        }
    }

    /// A bare `&` in a JSX attribute string is text, as TypeScript reads it; only a complete
    /// character reference such as `&amp;` is an `html_character_reference` (Patch 3 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`). JSX parses only in the TSX dialect.
    #[test]
    fn a_bare_ampersand_in_a_jsx_string_is_text() {
        let source = "<img src=\"/a?w=8&h=8\" />;\n\
                      <script src=\"https://example.com/a.js?a=1&b=2\" />;\n\
                      <a href=\"https://fonts.googleapis.com/css2?family=Inter&display=swap\" />;\n\
                      <a title='Q&' alt=\"x&#;y\" />;\n\
                      <img src=\"a&amp;b&#38;c&#x26;\" />;\n";
        let root = GrammarDialect::Tsx.ast_grep(source);
        assert!(!root.root().get_inner_node().has_error());
        let strings = root
            .root()
            .dfs()
            .filter(|node| node.kind() == "string")
            .map(|node| {
                node.children()
                    .filter(|child| child.is_named())
                    .map(|child| format!("{}:{}", child.kind(), child.text()))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            strings,
            [
                "string_fragment:/a?w=8 string_fragment:& string_fragment:h=8",
                "string_fragment:https://example.com/a.js?a=1 string_fragment:& string_fragment:b=2",
                "string_fragment:https://fonts.googleapis.com/css2?family=Inter \
                 string_fragment:& string_fragment:display=swap",
                "string_fragment:Q string_fragment:&",
                "string_fragment:x string_fragment:& string_fragment:#;y",
                "string_fragment:a html_character_reference:&amp; string_fragment:b \
                 html_character_reference:&#38; string_fragment:c html_character_reference:&#x26;",
            ]
        );
    }

    /// A type parameter may carry the variance annotations `in`, `out` or `in out`, after `const`,
    /// and `out` alone still names a parameter (Patch 4 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn variance_annotations_parse() {
        let source = "interface A<in T> {}\ntype B<out T> = T;\nclass C<in out T> {}\n\
                      class D<const in T> {}\ntype F<out T> = () => T;\ntype G<out> = out;\n\
                      function f<out>(x: out) {}\ntype H<in out> = out;\n";
        for dialect in [GrammarDialect::TypeScript, GrammarDialect::Tsx] {
            let root = dialect.ast_grep(source);
            assert!(!root.root().get_inner_node().has_error(), "{dialect:?}");
            let names = root
                .root()
                .dfs()
                .filter(|node| node.kind() == "type_parameter")
                .map(|node| {
                    let name = node.field("name").expect("a type parameter has a name");
                    format!("{}:{}", name.kind(), name.text())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                names,
                [
                    "type_identifier:T",
                    "type_identifier:T",
                    "type_identifier:T",
                    "type_identifier:T",
                    "type_identifier:T",
                    "type_identifier:out",
                    "type_identifier:out",
                    "type_identifier:out",
                ],
                "{dialect:?}"
            );
            // TS1029: `in` must precede `out`.
            let reversed = dialect.ast_grep("interface X<out in T> {}");
            assert!(reversed.root().get_inner_node().has_error(), "{dialect:?}");
        }
    }

    /// A function type's parameter may be named by a type keyword, as TypeScript reads
    /// `(any) => void`; `(string)` alone stays a parenthesized type, and `void` and `null`, which
    /// cannot name a parameter, stay errors (Patch 5 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn keyword_named_function_type_parameters_parse() {
        // The first parameter's function and parameter kinds, with its pattern's kind and text.
        let first_parameter = |dialect: GrammarDialect, source: &str| {
            let root = dialect.ast_grep(source);
            assert!(
                !root.root().get_inner_node().has_error(),
                "{dialect:?}: {source}"
            );
            let parameters = root
                .root()
                .dfs()
                .find(|node| node.kind() == "formal_parameters")
                .unwrap_or_else(|| panic!("{dialect:?}: {source} has parameters"));
            let function = parameters.parent().expect("parameters have a parent");
            let parameter = parameters
                .children()
                .find(|child| child.is_named())
                .unwrap_or_else(|| panic!("{dialect:?}: {source} has a parameter"));
            let pattern = parameter
                .field("pattern")
                .expect("a parameter has a pattern");
            format!(
                "{} {} {}:{}",
                function.kind(),
                parameter.kind(),
                pattern.kind(),
                pattern.text()
            )
        };
        let keywords = [
            "any", "unknown", "number", "string", "boolean", "never", "object", "symbol", "bigint",
        ];
        for dialect in [GrammarDialect::TypeScript, GrammarDialect::Tsx] {
            for keyword in keywords {
                assert_eq!(
                    first_parameter(dialect, &format!("type T = ({keyword}) => void;")),
                    format!("function_type required_parameter identifier:{keyword}"),
                    "{dialect:?}"
                );
            }
            // As the value side's `(undefined) => 1` reads it.
            assert_eq!(
                first_parameter(dialect, "type T = (undefined) => void;"),
                "function_type required_parameter undefined:undefined",
                "{dialect:?}"
            );
            for (source, expected) in [
                (
                    "let r: Record<number, (any) => void>;",
                    "function_type required_parameter identifier:any",
                ),
                (
                    "type T = (unknown, x) => void;",
                    "function_type required_parameter identifier:unknown",
                ),
                (
                    "type T = (never, x) => void;",
                    "function_type required_parameter identifier:never",
                ),
                (
                    "interface I { n: (string) => void }",
                    "function_type required_parameter identifier:string",
                ),
                (
                    "type T = (foo) => void;",
                    "function_type required_parameter identifier:foo",
                ),
                (
                    "type T = (any, x) => void;",
                    "function_type required_parameter identifier:any",
                ),
                (
                    "type T = (any?) => void;",
                    "function_type optional_parameter identifier:any",
                ),
                (
                    "type T = (a: any) => void;",
                    "function_type required_parameter identifier:a",
                ),
                (
                    "const f = (any) => 1;",
                    "arrow_function required_parameter identifier:any",
                ),
                (
                    "type T = new (any) => X;",
                    "constructor_type required_parameter identifier:any",
                ),
            ] {
                assert_eq!(first_parameter(dialect, source), expected, "{dialect:?}");
            }
            let parenthesized = dialect.ast_grep("type T = (string);");
            let value = parenthesized
                .root()
                .dfs()
                .find(|node| node.kind() == "type_alias_declaration")
                .and_then(|node| node.field("value"))
                .expect("an alias has a value");
            assert_eq!(value.kind(), "parenthesized_type", "{dialect:?}");
            assert_eq!(
                value
                    .child(1)
                    .map(|child| child.kind().into_owned())
                    .as_deref(),
                Some("predefined_type"),
                "{dialect:?}"
            );
            for source in ["type T = (void) => void;", "type T = (null) => void;"] {
                let root = dialect.ast_grep(source);
                assert!(
                    root.root().get_inner_node().has_error(),
                    "{dialect:?}: {source}"
                );
            }
        }
    }

    /// After a type, a member named `in` or `instanceof` on the next line ends the member before it,
    /// as one named by an identifier that starts with `in` (`in_point`, `in2`) now does everywhere,
    /// while `a⏎instanceof B` stays one binary expression (Patch 6 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn members_named_in_or_instanceof_after_a_newline_parse() {
        let source = "type O = {\n  a: typeof b\n  instanceof: typeof c\n  in?: string\n};\n\
                      interface I {\n  a: number\n  in: string\n  in_point: number\n  in2: number\n  \
                      in$x: number\n  instanceof_x: number\n  instanceof?(): void\n}\n\
                      class C {\n  a: number\n  in: string\n}\n";
        for dialect in [GrammarDialect::TypeScript, GrammarDialect::Tsx] {
            let root = dialect.ast_grep(source);
            assert!(!root.root().get_inner_node().has_error(), "{dialect:?}");
            let members = root
                .root()
                .dfs()
                .filter(|node| {
                    matches!(
                        node.kind().as_ref(),
                        "property_signature" | "method_signature" | "public_field_definition"
                    )
                })
                .map(|node| {
                    node.field("name")
                        .expect("a member has a name")
                        .text()
                        .into_owned()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                members,
                [
                    "a",
                    "instanceof",
                    "in",
                    "a",
                    "in",
                    "in_point",
                    "in2",
                    "in$x",
                    "instanceof_x",
                    "instanceof",
                    "a",
                    "in",
                ],
                "{dialect:?}"
            );
            for expression in ["a\ninstanceof B", "a\ninstanceof (B)", "a\nin b"] {
                let root = dialect.ast_grep(format!("{expression};\n"));
                assert!(!root.root().get_inner_node().has_error(), "{dialect:?}");
                let binaries = root
                    .root()
                    .dfs()
                    .filter(|node| node.kind() == "binary_expression")
                    .map(|node| node.text().into_owned())
                    .collect::<Vec<_>>();
                assert_eq!(binaries, [expression], "{dialect:?}");
            }
            // TypeScript reads `b in` as the start of a binary expression here.
            let initialized = dialect.ast_grep("class C {\n  a = b\n  in: number\n}\n");
            assert!(
                initialized.root().get_inner_node().has_error(),
                "{dialect:?}"
            );
        }
    }

    /// A bare `&` in JSX text is text, as TypeScript reads it; only a complete character
    /// reference such as `&amp;` is an `html_character_reference` (Patch 7 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`). JSX parses only in the TSX dialect.
    #[test]
    fn a_bare_ampersand_in_jsx_text_is_text() {
        let source = "<span>Help & Support</span>;\n<h2>{a} & {b}</h2>;\n\
                      <p>a &amp; b &#38; c &#x26;</p>;\n<p>&</p>;\n<p>a&b</p>;\n<p>&nbsp</p>;\n\
                      <p>a &b; c &#; d &#x; e</p>;\n";
        let root = GrammarDialect::Tsx.ast_grep(source);
        assert!(!root.root().get_inner_node().has_error());
        let children = root
            .root()
            .dfs()
            .filter(|node| node.kind() == "jsx_element")
            .map(|node| {
                node.children()
                    .filter(|child| {
                        !matches!(
                            child.kind().as_ref(),
                            "jsx_opening_element" | "jsx_closing_element"
                        )
                    })
                    .map(|child| format!("{}:{}", child.kind(), child.text()))
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            children,
            [
                "jsx_text:Help & Support",
                "jsx_expression:{a} | jsx_text: &  | jsx_expression:{b}",
                "jsx_text:a  | html_character_reference:&amp; | jsx_text: b  | \
                 html_character_reference:&#38; | jsx_text: c  | html_character_reference:&#x26;",
                "jsx_text:&",
                "jsx_text:a&b",
                "jsx_text:&nbsp",
                "jsx_text:a  | html_character_reference:&b; | jsx_text: c &#; d &#x; e",
            ]
        );
    }

    /// The JavaScript dialect's tree of `source`, which must parse without an error or a missing
    /// token (`grammars/tree-sitter-javascript/PATCHES.md`).
    fn javascript(source: &str) -> ast_grep_core::AstGrep<StrDoc<GrammarDialect>> {
        let root = GrammarDialect::JavaScript.ast_grep(source);
        let tree = root.root().get_inner_node();
        assert!(!tree.has_error(), "{source}\n{}", tree.to_sexp());
        root
    }

    /// The text of each named child of every node of kind `kind`.
    fn named_children_text(
        root: &ast_grep_core::AstGrep<StrDoc<GrammarDialect>>,
        kind: &str,
    ) -> Vec<String> {
        root.root()
            .dfs()
            .filter(|node| node.kind() == kind)
            .flat_map(|node| node.children().collect::<Vec<_>>())
            .filter(|child| child.is_named())
            .map(|child| child.text().into_owned())
            .collect()
    }

    /// A bare `&` in a JSX attribute string of a `.js` file is text; only a complete character
    /// reference is an `html_character_reference` (Patch 1 in
    /// `grammars/tree-sitter-javascript/PATCHES.md`, as TSX's Patch 3).
    #[test]
    fn a_bare_ampersand_in_a_javascript_jsx_string_is_text() {
        let root = javascript(
            "<a href=\"https://fonts.googleapis.com/css2?family=Krona+One&display=swap\" />;\n\
             <img src='/a?w=8&h=8' title=\"Q&\" alt=\"a&amp;b\" />;\n",
        );
        let strings = root
            .root()
            .dfs()
            .filter(|node| node.kind() == "string")
            .map(|node| {
                node.children()
                    .filter(|child| child.is_named())
                    .map(|child| format!("{}:{}", child.kind(), child.text()))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            strings,
            [
                "string_fragment:https://fonts.googleapis.com/css2?family=Krona+One \
                 string_fragment:& string_fragment:display=swap",
                "string_fragment:/a?w=8 string_fragment:& string_fragment:h=8",
                "string_fragment:Q string_fragment:&",
                "string_fragment:a html_character_reference:&amp; string_fragment:b",
            ]
        );
    }

    /// A bare `&` in the JSX text of a `.js` file is text; `&amp;` stays a reference (Patch 2 in
    /// `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn a_bare_ampersand_in_javascript_jsx_text_is_text() {
        let root = javascript(
            "<span>Help & Support</span>;\n<h2>{a} & {b}</h2>;\n<p>a &amp; b &#; c</p>;\n",
        );
        let children = root
            .root()
            .dfs()
            .filter(|node| node.kind() == "jsx_element")
            .map(|node| {
                node.children()
                    .filter(|child| {
                        !matches!(
                            child.kind().as_ref(),
                            "jsx_opening_element" | "jsx_closing_element"
                        )
                    })
                    .map(|child| format!("{}:{}", child.kind(), child.text()))
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            children,
            [
                "jsx_text:Help & Support",
                "jsx_expression:{a} | jsx_text: &  | jsx_expression:{b}",
                "jsx_text:a  | html_character_reference:&amp; | jsx_text: b &#; c",
            ]
        );
    }

    /// A re-export takes import attributes, as an import does (Patch 3 in
    /// `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn javascript_re_exports_take_import_attributes() {
        let root = javascript(
            "export { default as x } from './x.json' with { type: 'json' };\n\
             export * from 'y' with { type: 'json' }\n\
             export * as z from 'z' with { type: 'json' };\n",
        );
        assert_eq!(
            named_children_text(&root, "export_statement")
                .into_iter()
                .filter(|text| text.starts_with("with"))
                .collect::<Vec<_>>(),
            ["with { type: 'json' }"; 3]
        );
    }

    /// A `{` that starts a statement or an arrow function's body opens a block holding a labeled
    /// statement, never an object, as ECMAScript reads it; elsewhere it is an object (Patch 4 in
    /// `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn a_brace_at_javascript_statement_start_opens_a_block() {
        let root = javascript(
            "if (foo) { bar: 'baz' }\nf = () => { bar: 1 };\n{ bar: 1 }\n\
             g = () => ({ bar: 1 });\nx = { bar: 1 };\n",
        );
        let braces = root
            .root()
            .dfs()
            .filter(|node| matches!(node.kind().as_ref(), "statement_block" | "object"))
            .map(|node| format!("{}:{}", node.kind(), node.text()))
            .collect::<Vec<_>>();
        assert_eq!(
            braces,
            [
                "statement_block:{ bar: 'baz' }",
                "statement_block:{ bar: 1 }",
                "statement_block:{ bar: 1 }",
                "object:{ bar: 1 }",
                "object:{ bar: 1 }",
            ]
        );
    }

    /// `using` and `await using` declarations head a `for` statement, as `const` does (Patch 5 in
    /// `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn using_declarations_head_a_javascript_for_statement() {
        let root = javascript(
            "for (await using r = getResource(); ;) {}\nfor (using s = f(); i < 1; i++) {}\n",
        );
        let initializers = root
            .root()
            .dfs()
            .filter_map(|node| node.field("initializer"))
            .map(|node| format!("{}:{}", node.kind(), node.text()))
            .collect::<Vec<_>>();
        assert_eq!(
            initializers,
            [
                "using_declaration:await using r = getResource();",
                "using_declaration:using s = f();",
            ]
        );
    }

    /// A destructuring assignment's target may be parenthesized, comments and all (Patch 6 in
    /// `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn parenthesized_javascript_destructuring_targets_parse() {
        let root = javascript(
            "({foo: (/**/foo)} = {});\n({foo: (foo) = 1} = {});\n[(a), (b.c), ...(e)] = d;\n",
        );
        let targets = root
            .root()
            .dfs()
            .filter(|node| node.kind() == "assignment_expression")
            .map(|node| node.field("left").expect("an assignment has a left side"))
            .map(|left| {
                let parenthesized = left
                    .dfs()
                    .filter(|node| node.kind() == "parenthesized_expression")
                    .map(|node| node.text().into_owned())
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("{}:{parenthesized}", left.kind())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            targets,
            [
                "object_pattern:(/**/foo)",
                "object_pattern:(foo)",
                "array_pattern:(a) (b.c) (e)",
            ]
        );
    }

    /// Outside generators `yield` may label a statement (Patch 7 in
    /// `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn yield_labels_a_javascript_statement() {
        let root = javascript("var yield = 5;\nyield: while (foo) {\n  break yield;\n}\n");
        let labels = root
            .root()
            .dfs()
            .filter(|node| node.kind() == "statement_identifier")
            .map(|node| node.text().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(labels, ["yield", "yield"]);
    }

    /// `yield` is an identifier where no yield expression parses or where one would be an
    /// operand; where both read, the generator's yield expression stays (Patch 10 in
    /// `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn yield_is_a_javascript_name_where_no_yield_expression_parses() {
        let root = javascript(
            "yield = 2;\nyield += 1;\nyield++;\n[yield] = a;\nfor (yield of xs);\nyield.x;\n\
             f(yield);\nyield + 1;\nfunction* g() { yield; yield x; yield* h(); yield (a); }\n",
        );
        let yields = root
            .root()
            .dfs()
            .filter(|node| node.text().starts_with("yield"))
            .filter(|node| matches!(node.kind().as_ref(), "identifier" | "yield_expression"))
            .map(|node| format!("{}:{}", node.kind(), node.text()))
            .collect::<Vec<_>>();
        assert_eq!(
            yields,
            [
                "identifier:yield",
                "identifier:yield",
                "identifier:yield",
                "identifier:yield",
                "identifier:yield",
                "identifier:yield",
                "yield_expression:yield",
                "yield_expression:yield + 1",
                "yield_expression:yield",
                "yield_expression:yield x",
                "yield_expression:yield* h()",
                "yield_expression:yield (a)",
            ]
        );
    }

    /// A line break before `[`, `(`, `*`, `+` or `-` ends the statement where that token cannot
    /// continue it, as after a `var` declarator or a class field's name, and so does one before a
    /// member named `instanceof`; after an expression the line goes on (Patch 8 in
    /// `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn javascript_inserts_a_semicolon_before_a_token_that_cannot_continue() {
        let root = javascript(
            "var a\n+b > 0 ? c : d\nvar e\n[f] = g\nvar h\n(function () {})()\n\
             class C {\n  i\n  [j]\n  k\n  *l() {}\n  #m\n  instanceof() {}\n}\nn\n[o]\np\n+q\n",
        );
        assert_eq!(
            named_children_text(&root, "program"),
            [
                "var a",
                "+b > 0 ? c : d",
                "var e",
                "[f] = g",
                "var h",
                "(function () {})()",
                "class C {\n  i\n  [j]\n  k\n  *l() {}\n  #m\n  instanceof() {}\n}",
                "n\n[o]",
                "p\n+q",
            ]
        );
        assert_eq!(
            named_children_text(&root, "class_body"),
            ["i", "[j]", "k", "*l() {}", "#m", "instanceof() {}"]
        );
    }

    /// An arrow function is no operand: a line break after its block body ends the statement
    /// unless the next line goes on with `,` or a ternary's `:`, while a function expression's
    /// body does not (Patch 9 in `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn a_javascript_arrow_block_body_ends_the_statement_at_a_line_break() {
        let root = javascript(
            "var foo = () => { return bar }\n[1, 2, 3].map(foo)\nx = () => {}\n(1)\n\
             class C {\n  a = () => {}\n  *b() {}\n}\nf(() => {}\n, 1)\nc ? () => {}\n: d\n\
             e = function () {}\n[1]\n",
        );
        assert_eq!(
            named_children_text(&root, "program"),
            [
                "var foo = () => { return bar }",
                "[1, 2, 3].map(foo)",
                "x = () => {}",
                "(1)",
                "class C {\n  a = () => {}\n  *b() {}\n}",
                "f(() => {}\n, 1)",
                "c ? () => {}\n: d",
                "e = function () {}\n[1]",
            ]
        );
        assert_eq!(
            named_children_text(&root, "class_body"),
            ["a = () => {}", "*b() {}"]
        );
    }

    /// The TypeScript and TSX dialects' trees of `source`, each of which must parse without an
    /// error or a missing token (`grammars/tree-sitter-typescript/PATCHES.md`).
    fn typescript_and_tsx(source: &str) -> [ast_grep_core::AstGrep<StrDoc<GrammarDialect>>; 2] {
        [GrammarDialect::TypeScript, GrammarDialect::Tsx].map(|dialect| {
            let root = dialect.ast_grep(source);
            let tree = root.root().get_inner_node();
            assert!(
                !tree.has_error(),
                "{dialect:?}: {source}\n{}",
                tree.to_sexp()
            );
            root
        })
    }

    /// A re-export takes import attributes, as an import does, also a type-only one and with the
    /// deprecated `assert` (Patch 8 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn typescript_re_exports_take_import_attributes() {
        for root in typescript_and_tsx(
            "export { default as x } from './x.json' with { type: 'json' };\n\
             export * from 'y' with { type: 'json' }\n\
             export * as z from 'z' with { type: 'json' };\n\
             export type { A } from './a.json' with { type: 'json' };\n\
             export { b } from './b.json' assert { type: 'json' };\n",
        ) {
            assert_eq!(
                named_children_text(&root, "export_statement")
                    .into_iter()
                    .filter(|text| text.starts_with("with") || text.starts_with("assert"))
                    .collect::<Vec<_>>(),
                [
                    "with { type: 'json' }",
                    "with { type: 'json' }",
                    "with { type: 'json' }",
                    "with { type: 'json' }",
                    "assert { type: 'json' }",
                ]
            );
        }
    }

    /// A `{` that starts a statement or an arrow function's body opens a block holding a labeled
    /// statement, never an object; in an expression it is an object and in a type an object type
    /// (Patch 9 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn a_brace_at_typescript_statement_start_opens_a_block() {
        for root in typescript_and_tsx(
            "if (foo) { bar: 'baz' }\nf = (): void => { bar: 1 };\n{ bar: 1 }\n\
             g = (): { bar: number } => ({ bar: 1 });\nx = { bar: 1 } satisfies T;\n",
        ) {
            let braces = root
                .root()
                .dfs()
                .filter(|node| {
                    matches!(
                        node.kind().as_ref(),
                        "statement_block" | "object" | "object_type"
                    )
                })
                .map(|node| format!("{}:{}", node.kind(), node.text()))
                .collect::<Vec<_>>();
            assert_eq!(
                braces,
                [
                    "statement_block:{ bar: 'baz' }",
                    "statement_block:{ bar: 1 }",
                    "statement_block:{ bar: 1 }",
                    "object_type:{ bar: number }",
                    "object:{ bar: 1 }",
                    "object:{ bar: 1 }",
                ]
            );
        }
    }

    /// Outside generators `yield` may label a statement (Patch 10 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn yield_labels_a_typescript_statement() {
        for root in typescript_and_tsx("var yield = 5;\nyield: while (foo) {\n  break yield;\n}\n")
        {
            let labels = root
                .root()
                .dfs()
                .filter(|node| node.kind() == "statement_identifier")
                .map(|node| node.text().into_owned())
                .collect::<Vec<_>>();
            assert_eq!(labels, ["yield", "yield"]);
        }
    }

    /// A destructuring assignment's target may be parenthesized, comments, assertions and all
    /// (Patch 11 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn parenthesized_typescript_destructuring_targets_parse() {
        for root in typescript_and_tsx(
            "({foo: (/**/foo)} = {});\n({foo: (foo as any) = 1} = {});\n\
             [(a), (b.c!), ...(e)] = d;\n",
        ) {
            let targets = root
                .root()
                .dfs()
                .filter(|node| node.kind() == "assignment_expression")
                .map(|node| node.field("left").expect("an assignment has a left side"))
                .map(|left| {
                    let parenthesized = left
                        .dfs()
                        .filter(|node| node.kind() == "parenthesized_expression")
                        .map(|node| node.text().into_owned())
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("{}:{parenthesized}", left.kind())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                targets,
                [
                    "object_pattern:(/**/foo)",
                    "object_pattern:(foo as any)",
                    "array_pattern:(a) (b.c!) (e)",
                ]
            );
        }
    }

    /// A line break before `*`, `+` or `-` ends the statement where no binary operator can come
    /// next, as after a `var` declarator, a type or a class field's name, and so does one before
    /// a member named `instanceof`; after an expression the line goes on (Patch 12 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn typescript_inserts_a_semicolon_before_a_token_that_cannot_continue() {
        for root in typescript_and_tsx(
            "var a\n+b > 0 ? c : d\nlet t: T\n-1\n\
             class C {\n  i\n  *l() {}\n  j: string\n  *k() {}\n  #m\n  instanceof() {}\n}\n\
             interface I {\n  a: number\n  instanceof(): void\n}\n\
             n\n*o\np\n+q\nr\ninstanceof (S)\n",
        ) {
            assert_eq!(
                named_children_text(&root, "program"),
                [
                    "var a",
                    "+b > 0 ? c : d",
                    "let t: T",
                    "-1",
                    "class C {\n  i\n  *l() {}\n  j: string\n  *k() {}\n  #m\n  instanceof() {}\n}",
                    "interface I {\n  a: number\n  instanceof(): void\n}",
                    "n\n*o",
                    "p\n+q",
                    "r\ninstanceof (S)",
                ]
            );
            assert_eq!(
                named_children_text(&root, "class_body"),
                [
                    "i",
                    "*l() {}",
                    "j: string",
                    "*k() {}",
                    "#m",
                    "instanceof() {}"
                ]
            );
            assert_eq!(
                named_children_text(&root, "interface_body"),
                ["a: number", "instanceof(): void"]
            );
        }
    }

    /// An arrow function is no operand: a line break after its block body ends the statement
    /// unless the next line goes on with `,` or a ternary's `:`, while a function expression's
    /// body does not (Patch 13 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn a_typescript_arrow_block_body_ends_the_statement_at_a_line_break() {
        for root in typescript_and_tsx(
            "const foo = (): number[] => { return bar }\n[1, 2, 3].map(foo)\nx = () => {}\n(1)\n\
             class C {\n  a = () => {}\n  *b() {}\n}\nf(() => {}\n, 1)\nc ? () => {}\n: d\n\
             e = function () {}\n[1]\n",
        ) {
            assert_eq!(
                named_children_text(&root, "program"),
                [
                    "const foo = (): number[] => { return bar }",
                    "[1, 2, 3].map(foo)",
                    "x = () => {}",
                    "(1)",
                    "class C {\n  a = () => {}\n  *b() {}\n}",
                    "f(() => {}\n, 1)",
                    "c ? () => {}\n: d",
                    "e = function () {}\n[1]",
                ]
            );
            assert_eq!(
                named_children_text(&root, "class_body"),
                ["a = () => {}", "*b() {}"]
            );
        }
    }

    /// `yield` is an identifier where no yield expression parses or where one would be an
    /// operand; where both read, the generator's yield expression stays (Patch 14 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn yield_is_a_typescript_name_where_no_yield_expression_parses() {
        for root in typescript_and_tsx(
            "yield = 2;\nyield += 1;\nyield++;\n[yield] = a;\nfor (yield of xs);\nyield.x;\n\
             f(yield);\nyield + 1;\nfunction* g() { yield; yield x; yield* h(); yield (a); }\n",
        ) {
            let yields = root
                .root()
                .dfs()
                .filter(|node| node.text().starts_with("yield"))
                .filter(|node| matches!(node.kind().as_ref(), "identifier" | "yield_expression"))
                .map(|node| format!("{}:{}", node.kind(), node.text()))
                .collect::<Vec<_>>();
            assert_eq!(
                yields,
                [
                    "identifier:yield",
                    "identifier:yield",
                    "identifier:yield",
                    "identifier:yield",
                    "identifier:yield",
                    "identifier:yield",
                    "yield_expression:yield",
                    "yield_expression:yield + 1",
                    "yield_expression:yield",
                    "yield_expression:yield x",
                    "yield_expression:yield* h()",
                    "yield_expression:yield (a)",
                ]
            );
        }
    }

    /// In a generator, `yield++⏎x` is `yield (++x)`, as ECMAScript reads it: a `yield` with an
    /// operand outweighs the `yield` name that `yield++;` alone still is (Patch 10 in
    /// `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn a_javascript_generator_yields_a_prefix_update_after_yield() {
        let root = javascript("function* g(x) { yield++\nx; yield--\nx; }\nyield++;\nyield--;\n");
        let updates = root
            .root()
            .dfs()
            .filter(|node| node.kind() == "update_expression")
            .map(|node| {
                let parent = node.parent().expect("an update has a parent");
                format!("{}>{}", parent.kind(), node.text())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            updates,
            [
                "yield_expression>++\nx",
                "yield_expression>--\nx",
                "expression_statement>yield++",
                "expression_statement>yield--",
            ]
        );
    }

    /// The scanner reads every ECMAScript whitespace the grammar's extras skip, such as U+3000,
    /// U+00A0 and U+2028, so an automatic semicolon goes in after it (Patch 11 in
    /// `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn javascript_reads_unicode_whitespace_before_an_automatic_semicolon() {
        let root = javascript(
            "`\u{3000}template`\u{3000}\na\u{a0}= 1\u{3000}\nb = 2\u{2028}c = 3\u{feff}\n",
        );
        assert_eq!(
            named_children_text(&root, "program"),
            ["`\u{3000}template`", "a\u{a0}= 1", "b = 2", "c = 3"]
        );
    }

    /// A JSX name may be a reserved word or start with `this`, as Babel and TypeScript read it
    /// (Patch 12 in `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn javascript_jsx_names_may_be_reserved_words_or_this() {
        let root = javascript(
            "<a default=\"x\" class=\"y\" for=\"z\" if />;\n<a.class />;\n\
             <this.props.tag></this.props.tag>;\n",
        );
        let names = root
            .root()
            .dfs()
            .filter(|node| {
                matches!(
                    node.kind().as_ref(),
                    "property_identifier" | "identifier" | "this"
                )
            })
            .map(|node| format!("{}:{}", node.kind(), node.text()))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "identifier:a",
                "property_identifier:default",
                "property_identifier:class",
                "property_identifier:for",
                "property_identifier:if",
                "identifier:a",
                "property_identifier:class",
                "this:this",
                "property_identifier:props",
                "property_identifier:tag",
                "this:this",
                "property_identifier:props",
                "property_identifier:tag",
            ]
        );
    }

    /// A comment may stand between a JSX closing tag's `<` and `/`, as Babel and espree read it
    /// (Patch 13 in `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn a_comment_may_follow_a_javascript_jsx_closing_tag_lt() {
        let root = javascript("<></* c *//>;\n<a>\n  </* d *//a>;\n<a>x</a>;\n");
        let closings = root
            .root()
            .dfs()
            .filter(|node| node.kind() == "jsx_closing_element")
            .map(|node| {
                node.children()
                    .map(|child| child.kind().into_owned())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            closings,
            [
                "< comment / >",
                "< comment / identifier >",
                "</ identifier >"
            ]
        );
    }

    /// In a generator, `yield++⏎x` is `yield (++x)`, as ECMAScript reads it, while `yield++;`
    /// alone is the `yield` name's update (Patch 14 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn a_typescript_generator_yields_a_prefix_update_after_yield() {
        for root in typescript_and_tsx(
            "function* g(x: number) { yield++\nx; yield--\nx; }\nyield++;\nyield--;\n",
        ) {
            let updates = root
                .root()
                .dfs()
                .filter(|node| node.kind() == "update_expression")
                .map(|node| {
                    let parent = node.parent().expect("an update has a parent");
                    format!("{}>{}", parent.kind(), node.text())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                updates,
                [
                    "yield_expression>++\nx",
                    "yield_expression>--\nx",
                    "expression_statement>yield++",
                    "expression_statement>yield--",
                ]
            );
        }
    }

    /// An import type is a primary type: an array type, an indexed access or type arguments may
    /// follow it (Patch 15 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn typescript_import_types_are_primary_types() {
        for root in typescript_and_tsx(
            "let a: import('x').T[];\nlet b: readonly import('x').T[];\n\
             let c: import('x').T['a']['b'][number];\nlet d: import('x').I<T>;\n\
             let e: typeof import('x');\n",
        ) {
            let types = root
                .root()
                .dfs()
                .filter(|node| node.kind() == "type_annotation")
                .map(|node| {
                    let ty = node.child(1).expect("an annotation has a type");
                    format!("{}:{}", ty.kind(), ty.text())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                types,
                [
                    "array_type:import('x').T[]",
                    "readonly_type:readonly import('x').T[]",
                    "lookup_type:import('x').T['a']['b'][number]",
                    "generic_type:import('x').I<T>",
                    "type_query:typeof import('x')",
                ]
            );
        }
    }

    /// A tuple member may be named by a type keyword (Patch 16 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn typescript_tuple_members_may_be_named_by_type_keywords() {
        for root in typescript_and_tsx(
            "const s: [spoken: string, symbol: string][] = [];\n\
             type T = [string: number, any?: boolean, ...unknown: number[]];\ntype U = [any?, string];\n",
        ) {
            let members = root
                .root()
                .dfs()
                .filter(|node| {
                    node.parent()
                        .is_some_and(|parent| parent.kind() == "tuple_type")
                })
                .filter(|node| node.is_named())
                .map(|node| format!("{}:{}", node.kind(), node.text()))
                .collect::<Vec<_>>();
            assert_eq!(
                members,
                [
                    "required_parameter:spoken: string",
                    "required_parameter:symbol: string",
                    "required_parameter:string: number",
                    "optional_parameter:any?: boolean",
                    "required_parameter:...unknown: number[]",
                    "optional_type:any?",
                    "predefined_type:string",
                ]
            );
        }
    }

    /// A default export's overloads and ambient declaration may be anonymous function signatures
    /// (Patch 17 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn typescript_default_exports_may_be_anonymous_function_signatures() {
        for root in typescript_and_tsx(
            "export default function (): void;\nexport default function (x: string): string;\n\
             export default function (x?: string): string | void { return x; }\n\
             declare module 'm' {\n  export default function (\n    port?: number,\n  ): Promise<void>;\n}\n",
        ) {
            let exported = root
                .root()
                .dfs()
                .filter(|node| node.kind() == "export_statement")
                .map(|node| {
                    let value = node
                        .field("declaration")
                        .or_else(|| node.field("value"))
                        .expect("a default export has a declaration or a value");
                    format!("{}:{}", value.kind(), value.field("name").is_some())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                exported,
                [
                    "function_signature:false",
                    "function_signature:false",
                    "function_expression:false",
                    "function_signature:false",
                ]
            );
        }
    }

    /// `accessor` follows `static`, `abstract` and `override`, and still names a member or a
    /// variable (Patch 18 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn typescript_accessor_follows_other_modifiers() {
        for root in typescript_and_tsx(
            "abstract class A {\n  static accessor a = 1;\n  private abstract accessor b: number;\n  \
             override accessor c = () => {};\n  public static override accessor d = 2;\n  \
             accessor e = 3;\n  accessor = 4;\n  static accessor;\n  accessor() {}\n}\n\
             const o = { accessor, r: 1 };\naccessor.dispose();\n",
        ) {
            let members = root
                .root()
                .dfs()
                .filter(|node| {
                    matches!(
                        node.kind().as_ref(),
                        "public_field_definition" | "method_definition"
                    )
                })
                .map(|node| {
                    // The modifiers: the member's text before its name.
                    let name = node.field("name").expect("a member has a name");
                    let text = node.text();
                    let modifiers = text[..name.range().start - node.range().start].trim_end();
                    format!("{modifiers}|{}", name.text())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                members,
                [
                    "static accessor|a",
                    "private abstract accessor|b",
                    "override accessor|c",
                    "public static override accessor|d",
                    "accessor|e",
                    "|accessor",
                    "static|accessor",
                    "|accessor",
                ]
            );
            assert_eq!(
                root.root()
                    .dfs()
                    .filter(|node| node.text() == "accessor")
                    .filter(|node| {
                        matches!(
                            node.kind().as_ref(),
                            "identifier" | "shorthand_property_identifier"
                        )
                    })
                    .count(),
                2
            );
        }
    }

    /// A type-only re-export of a whole module (Patch 19 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn typescript_type_only_star_re_exports_parse() {
        for root in typescript_and_tsx(
            "export type * from 'hello';\nexport type * as Bar from 'module';\n\
             export type * from './x.json' with { type: 'json' };\n",
        ) {
            let exports = root
                .root()
                .dfs()
                .filter(|node| node.kind() == "export_statement")
                .map(|node| {
                    node.children()
                        .map(|child| child.kind().into_owned())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>();
            assert_eq!(
                exports,
                [
                    "export type * from string ;",
                    "export type namespace_export from string ;",
                    "export type * from string import_attribute ;",
                ]
            );
        }
    }

    /// The scanner reads every ECMAScript whitespace and line terminator the grammar's extras
    /// skip, such as U+3000, U+00A0 and U+2028, so an automatic semicolon goes in after it
    /// (Patch 20 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn typescript_reads_unicode_whitespace_before_an_automatic_semicolon() {
        for root in typescript_and_tsx(
            "`\u{3000}template`\u{3000}\na\u{a0}= 1\u{3000}\nb = 2\u{2028}c = 3 // d\u{2028}e = 4\u{feff}\n\
             f = g\u{3000}? h : i\n",
        ) {
            assert_eq!(
                named_children_text(&root, "program"),
                [
                    "`\u{3000}template`",
                    "a\u{a0}= 1",
                    "b = 2",
                    "c = 3 // d",
                    "e = 4",
                    "f = g\u{3000}? h : i",
                ]
            );
        }
    }

    /// A comment may stand between a TSX closing tag's `<` and `/` (Patch 21 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn a_comment_may_follow_a_tsx_closing_tag_lt() {
        let root = GrammarDialect::Tsx.ast_grep("<></* c *//>;\n<a>\n  </* d *//a>;\n<a>x</a>;\n");
        assert!(!root.root().get_inner_node().has_error());
        let closings = root
            .root()
            .dfs()
            .filter(|node| node.kind() == "jsx_closing_element")
            .map(|node| {
                node.children()
                    .map(|child| child.kind().into_owned())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            closings,
            [
                "< comment / >",
                "< comment / identifier >",
                "</ identifier >"
            ]
        );
    }

    /// `abstract` may name a member or a variable, and still starts an abstract class or member
    /// (Patch 22 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn abstract_may_name_a_typescript_member() {
        for root in typescript_and_tsx(
            "function f(): { abstract: Set<string>; concrete: number } { return x; }\n\
             interface I { abstract?(): void }\nclass C { abstract = 1; abstract() {} }\n\
             let abstract = 2;\nabstract class A { abstract m(): void; abstract x: number }\n",
        ) {
            let names = root
                .root()
                .dfs()
                .filter(|node| node.text() == "abstract" && node.is_named())
                .map(|node| node.kind().into_owned())
                .collect::<Vec<_>>();
            assert_eq!(
                names,
                [
                    "property_identifier",
                    "property_identifier",
                    "property_identifier",
                    "property_identifier",
                    "identifier",
                ]
            );
            assert_eq!(
                root.root()
                    .dfs()
                    .filter(|node| {
                        matches!(
                            node.kind().as_ref(),
                            "abstract_class_declaration" | "abstract_method_signature"
                        )
                    })
                    .count(),
                2
            );
        }
    }

    /// A lone CR ends a line, for an automatic semicolon and a `//` comment, as every ECMAScript
    /// line terminator does; a U+00A0 on a JSX line of its own stays text, whatever the locale
    /// (Patch 11 in `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn javascript_reads_a_lone_cr_and_jsx_text_by_ecmascript_line_terminators() {
        let root = javascript("a = 1\rb = 2 // c\rd = 3;\n<a>\n\u{a0}</a>;\n");
        assert_eq!(
            named_children_text(&root, "program"),
            ["a = 1", "b = 2 // c", "d = 3;", "<a>\n\u{a0}</a>;"]
        );
        assert_eq!(
            named_children_text(&root, "jsx_element"),
            ["<a>", "\n\u{a0}", "</a>"]
        );
    }

    /// `<this />` is an element named `this`, as Babel reads it (Patch 12 in
    /// `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn this_names_a_javascript_jsx_element() {
        let root = javascript("<this />;\nx = <this a=\"1\">t</this>;\n");
        let names = root
            .root()
            .dfs()
            .filter_map(|node| node.field("name"))
            .map(|name| format!("{}:{}", name.kind(), name.text()))
            .collect::<Vec<_>>();
        assert_eq!(names, ["this:this", "this:this", "this:this"]);
    }

    /// An import type with no qualifier takes type arguments too (Patch 15 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn a_bare_typescript_import_type_takes_type_arguments() {
        for root in typescript_and_tsx("type A<T> = import('x')<T>;\ntype B = import('x')<T>[];\n")
        {
            let types = root
                .root()
                .dfs()
                .filter_map(|node| node.field("value"))
                .map(|value| format!("{}:{}", value.kind(), value.text()))
                .collect::<Vec<_>>();
            assert_eq!(
                types,
                ["generic_type:import('x')<T>", "array_type:import('x')<T>[]"]
            );
        }
    }

    /// A tuple member may be named by a keyword a type starts with, such as `void` or `null`,
    /// while `[void?]` stays an optional type (Patch 16 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn typescript_tuple_members_may_be_named_by_keywords_that_start_a_type() {
        for root in typescript_and_tsx(
            "type A = [void: number, null?: string, this: T, typeof?: U, infer: V, import: W];\n\
             type B = [void?, null?, typeof x, keyof T, unique symbol, import('x').T];\n",
        ) {
            let members = root
                .root()
                .dfs()
                .filter(|node| {
                    node.parent()
                        .is_some_and(|parent| parent.kind() == "tuple_type")
                })
                .filter(|node| node.is_named())
                .map(|node| {
                    let name = node.field("name").map(|name| format!("{}:", name.text()));
                    format!("{}:{}", node.kind(), name.unwrap_or_default())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                members,
                [
                    "required_parameter:void:",
                    "optional_parameter:null:",
                    "required_parameter:this:",
                    "optional_parameter:typeof:",
                    "required_parameter:infer:",
                    "required_parameter:import:",
                    "optional_type:",
                    "optional_type:",
                    "type_query:",
                    "index_type_query:",
                    "predefined_type:",
                    "member_expression:",
                ]
            );
        }
    }

    /// A lone CR ends a line, for an automatic semicolon and a `//` comment, and a U+00A0 on a
    /// TSX line of its own stays text, whatever the locale (Patch 20 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn typescript_reads_a_lone_cr_and_tsx_text_by_ecmascript_line_terminators() {
        for root in typescript_and_tsx("a = 1\rb = 2 // c\rd = 3;\r\ne = 4\n") {
            assert_eq!(
                named_children_text(&root, "program"),
                ["a = 1", "b = 2 // c", "d = 3;", "e = 4"]
            );
        }
        let root = GrammarDialect::Tsx.ast_grep("<a>\n\u{a0}</a>;\n<b>\n  \n</b>;\n");
        assert!(!root.root().get_inner_node().has_error());
        assert_eq!(
            named_children_text(&root, "jsx_element"),
            ["<a>", "\n\u{a0}", "</a>", "<b>", "</b>"]
        );
    }

    /// The patches keep TypeScript's syntax errors: a function type's parameter cannot be named
    /// `void`, a comment's `/` closes no TSX tag, and `accessor` before a line break is a field
    /// name, not a modifier (Patches 5, 18 and 21 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn typescript_patches_keep_typescript_syntax_errors() {
        for dialect in [GrammarDialect::TypeScript, GrammarDialect::Tsx] {
            let root = dialect.ast_grep("type Bad = (void) => void;\n");
            assert!(root.root().get_inner_node().has_error(), "{dialect:?}");
            let root = dialect.ast_grep("class C { accessor\nx = 1 }\n");
            assert!(!root.root().get_inner_node().has_error(), "{dialect:?}");
            assert_eq!(
                named_children_text(&root, "class_body"),
                ["accessor", "x = 1"],
                "{dialect:?}"
            );
        }
        for source in ["<a></* x */a>;\n", "<a></* x\n"] {
            let root = GrammarDialect::Tsx.ast_grep(source);
            assert!(root.root().get_inner_node().has_error(), "{source}");
        }
    }

    /// A block comment holding a line break ends the line, as ECMAScript reads it, while one on
    /// a single line does not (Patch 14 in `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn a_multi_line_javascript_comment_ends_the_line() {
        let root =
            javascript("a = 1 /*\n*/ b = 2\nx /*\n*/ ++y\nc /* one line */ [d]\ne /*\n*/ (f)\n");
        assert_eq!(
            named_children_text(&root, "program"),
            [
                "a = 1",
                "/*\n*/",
                "b = 2",
                "x",
                "/*\n*/",
                "++y",
                "c /* one line */ [d]",
                "e /*\n*/ (f)",
            ]
        );
    }

    /// A line break right after `return` or `yield` ends the statement before any operand, as
    /// ECMAScript's restricted productions read it; a token that cannot start an operand goes on
    /// (Patch 15 in `grammars/tree-sitter-javascript/PATCHES.md`).
    #[test]
    fn javascript_return_and_yield_end_at_a_line_break() {
        let root = javascript(
            "function f() {\n  return\n  [a]\n  return\n  (b)\n  return /*\n  */ -1\n  return (\n    c\n  )\n}\n\
             function* g() {\n  yield\n  (d)\n  x = yield\n  , e\n}\n",
        );
        assert_eq!(
            named_children_text(&root, "statement_block"),
            [
                "return",
                "[a]",
                "return",
                "(b)",
                "return",
                "/*\n  */",
                "-1",
                "return (\n    c\n  )",
                "yield",
                "(d)",
                "x = yield\n  , e",
            ]
        );
    }

    /// A `<` after a line break starts the next type member, as TypeScript reads it: a call or
    /// construct signature with type parameters follows another member with no separator, while a
    /// comparison or a generic call after an expression goes on (Patch 23 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn a_typescript_call_signature_with_type_parameters_follows_a_member_at_a_line_break() {
        let source = "interface ParseBody {\n  (request: Request): Promise<T>\n  \
                      <T extends BodyData>(request: Request): Promise<T>\n  a: Foo\n  \
                      <U>(x: U): void\n  m(): void\n  <V>(): V\n  b: string\n  new <W>(): W\n}\n\
                      type O = { (): void\n<T>(): T; c: Foo<\nT> }\n";
        for root in typescript_and_tsx(source) {
            assert_eq!(
                named_children_text(&root, "interface_body"),
                [
                    "(request: Request): Promise<T>",
                    "<T extends BodyData>(request: Request): Promise<T>",
                    "a: Foo",
                    "<U>(x: U): void",
                    "m(): void",
                    "<V>(): V",
                    "b: string",
                    "new <W>(): W",
                ]
            );
            assert_eq!(
                named_children_text(&root, "object_type"),
                ["(): void", "<T>(): T", "c: Foo<\nT>"]
            );
            let root = typescript_and_tsx("a\n< b;\nfoo\n<T>(x);\n")
                .into_iter()
                .next()
                .expect("the TypeScript tree");
            assert_eq!(
                named_children_text(&root, "program"),
                ["a\n< b;", "foo\n<T>(x);"]
            );
        }
        // In TSX `<T>y` would be a JSX element, as in `tsc`.
        let root = GrammarDialect::TypeScript.ast_grep("let x: Foo\n<T>y;\n");
        assert!(!root.root().get_inner_node().has_error());
        assert_eq!(
            named_children_text(&root, "program"),
            ["let x: Foo", "<T>y;"]
        );
    }

    /// A line break right after `return` or `yield` ends the statement before an operand, and a
    /// block comment holding a line break counts as one, as TypeScript reads them (Patch 24 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn typescript_return_and_yield_end_at_a_line_break() {
        let source = "function* g(): Generator<number> {\n  yield\n  (d)\n  yield\n  [e]\n  \
                      yield /*\n  */ f\n  x = yield\n  , h\n}\nfunction k(): void {\n  return /*\n  */ -1\n  \
                      return\n  /a/g\n}\na = 1 /*\n*/ b = 2\nc = d /*\n*/ (e)\nlabel: for (;;) { break /*\n*/ label }\n";
        for root in typescript_and_tsx(source) {
            assert_eq!(
                named_children_text(&root, "statement_block"),
                [
                    "yield",
                    "(d)",
                    "yield",
                    "[e]",
                    "yield",
                    "/*\n  */",
                    "f",
                    "x = yield\n  , h",
                    "return",
                    "/*\n  */",
                    "-1",
                    "return",
                    "/a/g",
                    "break",
                    "/*\n*/",
                    "label",
                ]
            );
            assert_eq!(
                named_children_text(&root, "program")[2..5],
                ["a = 1", "/*\n*/", "b = 2"]
            );
            assert_eq!(named_children_text(&root, "program")[5], "c = d /*\n*/ (e)");
        }
    }

    /// A class may declare a static index signature, as TypeScript 4.3 allows; an interface may
    /// not (Patch 25 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn a_typescript_class_declares_a_static_index_signature() {
        for root in typescript_and_tsx(
            "class C { static [k: string]: number; static readonly [n: number]: string; [s: string]: T; }\n",
        ) {
            let signatures = root
                .root()
                .dfs()
                .filter(|node| node.kind() == "index_signature")
                .map(|node| {
                    let name = node.field("name").map(|name| name.text().into_owned());
                    format!("{}:{}", name.unwrap_or_default(), node.text())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                signatures,
                [
                    "k:static [k: string]: number",
                    "n:static readonly [n: number]: string",
                    "s:[s: string]: T",
                ]
            );
        }
        for dialect in [GrammarDialect::TypeScript, GrammarDialect::Tsx] {
            let root = dialect.ast_grep("interface I { static [k: string]: number; }\n");
            assert!(root.root().get_inner_node().has_error(), "{dialect:?}");
        }
    }

    /// `global { … }` without `declare` is a global augmentation, an `internal_module` named
    /// `global`; `declare global { … }` stays an `ambient_declaration`, and `global` stays a name
    /// elsewhere, member names included (Patch 26 in `grammars/tree-sitter-typescript/PATCHES.md`).
    #[test]
    fn a_typescript_global_augmentation_needs_no_declare() {
        for root in typescript_and_tsx(
            "declare module 'x' { global { interface String {} } }\nglobal { interface Array<T> {} }\n\
             declare global { interface Window {} }\nglobal.process = 1;\nconst g = global;\n\
             function f(global: number) {}\nglobal: for (;;) { break global; }\n\
             o.global;\nx = { global: 1 };\nclass K { global() {} }\n",
        ) {
            let modules = root
                .root()
                .dfs()
                .filter(|node| node.kind() == "internal_module")
                .map(|node| {
                    let name = node.field("name").expect("a module has a name");
                    format!("{}:{}", name.kind(), name.text())
                })
                .collect::<Vec<_>>();
            assert_eq!(modules, ["identifier:global", "identifier:global"]);
            let ambient = root
                .root()
                .dfs()
                .filter(|node| node.kind() == "ambient_declaration")
                .count();
            assert_eq!(ambient, 2);
            let names = root
                .root()
                .dfs()
                .filter(|node| node.text() == "global" && node.is_named())
                .map(|node| {
                    let parent = node.parent().expect("a name has a parent");
                    format!("{}<{}", node.kind(), parent.kind())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                names,
                [
                    "identifier<internal_module",
                    "identifier<internal_module",
                    "identifier<member_expression",
                    "identifier<variable_declarator",
                    "identifier<required_parameter",
                    "statement_identifier<labeled_statement",
                    "statement_identifier<break_statement",
                    "property_identifier<member_expression",
                    "property_identifier<pair",
                    "property_identifier<method_definition",
                ]
            );
        }
        // `global` followed by `{` on the next line is an expression statement and a block, not an
        // augmentation: TypeScript reads one there, but the grammar would need an external scanner.
        for root in typescript_and_tsx("declare module 'x' { global\n{ } }\n") {
            assert!(!root.root().get_inner_node().has_error());
            assert!(
                !root
                    .root()
                    .dfs()
                    .any(|node| node.kind() == "internal_module")
            );
            let body = root
                .root()
                .dfs()
                .find(|node| node.kind() == "module")
                .and_then(|module| module.field("body"))
                .expect("the module has a body");
            let statements = body
                .children()
                .filter(|node| node.is_named())
                .map(|node| node.kind().into_owned())
                .collect::<Vec<_>>();
            assert_eq!(statements, ["expression_statement", "statement_block"]);
        }
    }

    /// A tagged template may take type arguments, as TypeScript reads it: a `call_expression` with
    /// `type_arguments` and a `template_string` as its `arguments`, whether the type is a name or an
    /// object type with optional or `;`-separated members, and `a < b > \`c\`` is one too (Patch 27 in
    /// `grammars/tree-sitter-typescript/PATCHES.md`). A tagged template without type arguments and
    /// `new A<T>` keep their readings.
    #[test]
    fn a_typescript_tagged_template_takes_type_arguments() {
        for root in typescript_and_tsx(
            "const A = styled.div<{ open?: boolean; size: number }>`\n  top: ${({ open }) => (open ? 0 : 1)}px;\n`;\n\
             const B = styled(C)<{ x: boolean }>`top: 0;`;\nconst D = f<T>`x`;\nx = a < b > `c`;\n\
             const E = css`top: 0;`;\nconst F = new G<T>;\n",
        ) {
            assert!(!root.root().get_inner_node().has_error());
            let tagged = root
                .root()
                .dfs()
                .filter(|node| {
                    node.kind() == "call_expression"
                        && node
                            .field("arguments")
                            .is_some_and(|arguments| arguments.kind() == "template_string")
                })
                .map(|node| {
                    let function = node.field("function").expect("a call has a function");
                    let type_arguments = node
                        .field("type_arguments")
                        .map(|types| types.text().into_owned());
                    format!("{}{}", function.text(), type_arguments.unwrap_or_default())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                tagged,
                [
                    "styled.div<{ open?: boolean; size: number }>",
                    "styled(C)<{ x: boolean }>",
                    "f<T>",
                    "a< b >",
                    "css"
                ]
            );
            assert!(
                root.root()
                    .dfs()
                    .any(|node| node.kind() == "new_expression")
            );
            assert!(
                !root
                    .root()
                    .dfs()
                    .any(|node| node.kind() == "binary_expression")
            );
        }
    }
}
