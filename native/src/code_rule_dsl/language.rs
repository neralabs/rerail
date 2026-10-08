//! The languages Rerail reads, in one table. Every question about a language asks here: which
//! grammar dialects a code-rule language spans, which dialect a file label selects, a dialect's tape
//! tag, its node kinds and fields (`KindCatalog`, which rules and tapes share), and which semantic
//! family serves a grammar. The host owns which file extensions select a label
//! (`SOURCE_LANGUAGES` in code-rule-enforcer.ts); the repository index receives them.
//!
//! Three layers:
//! - A **dialect** is one tree-sitter grammar (`GrammarDialect`): Python, TypeScript, TSX,
//!   JavaScript (JSX included), PHP. Files are parsed by exactly one dialect.
//! - A **code-rule language** is what a rule's `language` names. It spans the dialects its rules run
//!   on; a sub-rule one dialect cannot express is `Never` there (`compile::dialect`). A
//!   `typescript` rule also runs on JavaScript files; a `javascript` rule runs on them alone.
//! - A **family** is the semantic implementation a dialect is read with: its scope table, value
//!   decoder, call arguments, suppression comments and line terminators. TypeScript, TSX and
//!   JavaScript share the ECMAScript family.
//!
//! Adding a language: add its dialect row (file label, tape tag, extensions), its code-rule language
//! (name and dialects), its family, and the family's arm wherever a family is matched; what a
//! family does not implement is refused when a code rule compiles (`Family::implements`), never
//! answered by another family's code.

use crate::code_rule_dsl::module_path::ModulePath;
use ast_grep_core::{
    language::Language,
    matcher::{Pattern, PatternBuilder, PatternError, PatternNode},
    tree_sitter::{StrDoc, TSLanguage},
};
use ast_grep_language::{LanguageExt, SupportLang};
use std::borrow::Cow;

/// Rerail's grammar dialect. `ast_grep_language::SupportLang` is closed, while Blade's vendored
/// grammar is part of Rerail's supported source surface.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum GrammarDialect {
    Python,
    TypeScript,
    Tsx,
    JavaScript,
    Php,
    Blade,
    #[cfg(test)]
    Java,
    #[cfg(test)]
    CSharp,
    #[cfg(test)]
    Cpp,
    #[cfg(test)]
    C,
    #[cfg(test)]
    Bash,
    #[cfg(test)]
    Ruby,
    #[cfg(test)]
    Rust,
    #[cfg(test)]
    Kotlin,
    #[cfg(test)]
    Scala,
    #[cfg(test)]
    Swift,
    #[cfg(test)]
    Html,
    #[cfg(test)]
    Css,
    #[cfg(test)]
    Lua,
    #[cfg(test)]
    Json,
    #[cfg(test)]
    Go,
}

impl GrammarDialect {
    /// Every dialect, test-only grammars included.
    const ALL: &'static [Self] = &[
        Self::Python,
        Self::TypeScript,
        Self::Tsx,
        Self::JavaScript,
        Self::Php,
        Self::Blade,
        #[cfg(test)]
        Self::Java,
        #[cfg(test)]
        Self::CSharp,
        #[cfg(test)]
        Self::Cpp,
        #[cfg(test)]
        Self::C,
        #[cfg(test)]
        Self::Bash,
        #[cfg(test)]
        Self::Ruby,
        #[cfg(test)]
        Self::Rust,
        #[cfg(test)]
        Self::Kotlin,
        #[cfg(test)]
        Self::Scala,
        #[cfg(test)]
        Self::Swift,
        #[cfg(test)]
        Self::Html,
        #[cfg(test)]
        Self::Css,
        #[cfg(test)]
        Self::Lua,
        #[cfg(test)]
        Self::Json,
        #[cfg(test)]
        Self::Go,
    ];

    const fn support(self) -> Option<SupportLang> {
        match self {
            Self::Python => Some(SupportLang::Python),
            Self::TypeScript => Some(SupportLang::TypeScript),
            Self::Tsx => Some(SupportLang::Tsx),
            Self::JavaScript => Some(SupportLang::JavaScript),
            Self::Php => Some(SupportLang::Php),
            Self::Blade => None,
            #[cfg(test)]
            Self::Java => Some(SupportLang::Java),
            #[cfg(test)]
            Self::CSharp => Some(SupportLang::CSharp),
            #[cfg(test)]
            Self::Cpp => Some(SupportLang::Cpp),
            #[cfg(test)]
            Self::C => Some(SupportLang::C),
            #[cfg(test)]
            Self::Bash => Some(SupportLang::Bash),
            #[cfg(test)]
            Self::Ruby => Some(SupportLang::Ruby),
            #[cfg(test)]
            Self::Rust => Some(SupportLang::Rust),
            #[cfg(test)]
            Self::Kotlin => Some(SupportLang::Kotlin),
            #[cfg(test)]
            Self::Scala => Some(SupportLang::Scala),
            #[cfg(test)]
            Self::Swift => Some(SupportLang::Swift),
            #[cfg(test)]
            Self::Html => Some(SupportLang::Html),
            #[cfg(test)]
            Self::Css => Some(SupportLang::Css),
            #[cfg(test)]
            Self::Lua => Some(SupportLang::Lua),
            #[cfg(test)]
            Self::Json => Some(SupportLang::Json),
            #[cfg(test)]
            Self::Go => Some(SupportLang::Go),
        }
    }
}

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
        self.support().map_or('µ', |lang| lang.expando_char())
    }

    fn pre_process_pattern<'q>(&self, query: &'q str) -> Cow<'q, str> {
        if let Some(language) = self.support() {
            return language.pre_process_pattern(query);
        }
        let mut processed = String::with_capacity(query.len());
        let mut dollars = 0;
        for character in query.chars() {
            if character == '$' {
                dollars += 1;
                continue;
            }
            let expando = matches!(character, 'A'..='Z' | '_') || dollars == 3;
            processed.extend(std::iter::repeat_n(
                if expando { 'µ' } else { '$' },
                dollars,
            ));
            dollars = 0;
            processed.push(character);
        }
        processed.extend(std::iter::repeat_n(
            if dollars == 3 { 'µ' } else { '$' },
            dollars,
        ));
        Cow::Owned(processed)
    }

    fn build_pattern(&self, builder: &PatternBuilder) -> Result<Pattern, PatternError> {
        builder.build(|source| StrDoc::try_new(source, *self))
    }
}

impl LanguageExt for GrammarDialect {
    /// The grammar `build.rs` generates from a vendored grammar (Blade; Python, TypeScript and
    /// TSX, which carry patches: `grammars/tree-sitter-{python,typescript}/PATCHES.md`), else
    /// ast-grep's. ast-grep is built without its Python and TypeScript parsers, so `SupportLang`
    /// only lends these dialects its pattern syntax.
    fn get_ts_language(&self) -> TSLanguage {
        unsafe extern "C" {
            fn tree_sitter_blade() -> *const ();
            fn tree_sitter_python() -> *const ();
            fn tree_sitter_typescript() -> *const ();
            fn tree_sitter_tsx() -> *const ();
        }
        let vendored = match self {
            Self::Blade => tree_sitter_blade,
            Self::Python => tree_sitter_python,
            Self::TypeScript => tree_sitter_typescript,
            Self::Tsx => tree_sitter_tsx,
            _ => {
                return self
                    .support()
                    .expect("a dialect without a vendored grammar is ast-grep's")
                    .get_ts_language();
            }
        };
        unsafe { tree_sitter_language::LanguageFn::from_raw(vendored) }.into()
    }
}

/// The PHP a Blade tape embeds (`blade::view`) and where its kind ids start in Blade's numbering.
pub(crate) struct Embedded {
    pub(crate) php: &'static TSLanguage,
    pub(crate) offset: u16,
}

impl Embedded {
    /// Whether a kind id in the dialect's numbering is PHP's. ERROR is every grammar's.
    fn owns(&self, id: u16) -> bool {
        id >= self.offset && id != u16::MAX
    }

    /// A PHP kind id in the dialect's numbering. ERROR is every grammar's.
    fn shift(&self, id: u16) -> u16 {
        match id {
            u16::MAX => id,
            _ => id + self.offset,
        }
    }
}

static BLADE_EMBEDDED: std::sync::LazyLock<Embedded> = std::sync::LazyLock::new(|| Embedded {
    php: GrammarDialect::Php.grammar(),
    offset: u16::try_from(GrammarDialect::Blade.grammar().node_kind_count())
        .expect("Blade's kinds fit a u16"),
});

/// The node kinds and fields a dialect's tapes carry and its rules name: its grammar's, except
/// Blade's. A Blade tape also holds the PHP its template embeds (`blade::view`), so Blade numbers
/// its own kinds first and PHP's after them (PHP's id plus Blade's kind count). A name both
/// grammars give a named kind (`attribute`, `comment`, `php_tag`, `php_end_tag`) is Blade's.
/// Blade's grammar has no fields; its fields are PHP's. Rules and tapes read kinds and fields
/// only here.
#[derive(Clone, Copy)]
pub(crate) struct KindCatalog {
    own: &'static TSLanguage,
    embedded: Option<&'static Embedded>,
}

impl KindCatalog {
    /// The grammar the dialect's files are parsed with.
    pub(crate) fn grammar(self) -> &'static TSLanguage {
        self.own
    }

    /// The PHP the dialect's tapes embed: Blade's.
    pub(crate) fn embedded(self) -> Option<&'static Embedded> {
        self.embedded
    }

    /// The grammar a kind id belongs to, and the id in it.
    fn kind_home(self, id: u16) -> (&'static TSLanguage, u16) {
        match self.embedded {
            Some(embedded) if embedded.owns(id) => (embedded.php, id - embedded.offset),
            _ => (self.own, id),
        }
    }

    /// Whether a kind id is the embedded PHP's (a Blade tape's program), not the dialect's own.
    pub(crate) fn is_embedded_kind(self, id: u16) -> bool {
        self.embedded.is_some_and(|embedded| embedded.owns(id))
    }

    pub(crate) fn kind_count(self) -> usize {
        self.own.node_kind_count()
            + self
                .embedded
                .map_or(0, |embedded| embedded.php.node_kind_count())
    }

    /// The id of the named kind called `kind` (0 where there is none).
    pub(crate) fn kind_id(self, kind: &str) -> u16 {
        let own = self.own.id_for_node_kind(kind, true);
        match self.embedded {
            Some(embedded) if own == 0 => match embedded.php.id_for_node_kind(kind, true) {
                0 => 0,
                id => embedded.shift(id),
            },
            _ => own,
        }
    }

    pub(crate) fn kind_name(self, id: u16) -> Option<&'static str> {
        let (grammar, id) = self.kind_home(id);
        grammar.node_kind_for_id(id)
    }

    pub(crate) fn kind_is_named(self, id: u16) -> bool {
        let (grammar, id) = self.kind_home(id);
        grammar.node_kind_is_named(id)
    }

    pub(crate) fn kind_is_visible(self, id: u16) -> bool {
        let (grammar, id) = self.kind_home(id);
        grammar.node_kind_is_visible(id)
    }

    pub(crate) fn kind_is_supertype(self, id: u16) -> bool {
        let (grammar, id) = self.kind_home(id);
        grammar.node_kind_is_supertype(id)
    }

    /// The grammar whose fields the dialect's are: PHP's for Blade, which has none.
    fn fields(self) -> &'static TSLanguage {
        self.embedded.map_or(self.own, |embedded| embedded.php)
    }

    pub(crate) fn field_count(self) -> usize {
        self.fields().field_count()
    }

    pub(crate) fn field_id(self, name: &str) -> Option<u16> {
        self.fields().field_id_for_name(name).map(|id| id.get())
    }

    pub(crate) fn field_name(self, id: u16) -> Option<&'static str> {
        self.fields().field_name_for_id(id)
    }
}

impl GrammarDialect {
    /// The dialect's grammar, loaded once.
    fn grammar(self) -> &'static TSLanguage {
        static GRAMMARS: std::sync::LazyLock<
            std::collections::HashMap<GrammarDialect, TSLanguage>,
        > = std::sync::LazyLock::new(|| {
            GrammarDialect::ALL
                .iter()
                .map(|&dialect| (dialect, dialect.get_ts_language()))
                .collect()
        });
        &GRAMMARS[&self]
    }

    /// The dialect's kinds and fields.
    pub(crate) fn kinds(self) -> KindCatalog {
        KindCatalog {
            own: self.grammar(),
            embedded: (self == Self::Blade).then(|| &*BLADE_EMBEDDED),
        }
    }

    pub(crate) fn is_embedded_kind(self, id: u16) -> bool {
        self.kinds().is_embedded_kind(id)
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

    pub(crate) fn field_id(self, name: &str) -> Option<u16> {
        self.kinds().field_id(name)
    }
}

/// A rule's `pattern`: its source, or a context and the kind of the node it selects there.
pub(crate) enum PatternSource<'a> {
    Plain(&'a str),
    Contextual { context: &'a str, selector: &'a str },
}

/// A rule's pattern compiled for `language`, with the dialect its source was parsed as. A Blade
/// pattern that holds no Blade construct (Blade reads it as text alone, `sprintf($F, $$$A)`) is
/// PHP: it is parsed with PHP's grammar and matches the template's embedded program
/// (`blade::view`), its kinds numbered as Blade's tapes number PHP's.
pub(crate) fn rule_pattern(
    source: PatternSource<'_>,
    language: GrammarDialect,
) -> Result<(Pattern, GrammarDialect), PatternError> {
    let php = match (Family::of(language), &source) {
        (Family::Blade, PatternSource::Plain(text)) => blade_text(text),
        (Family::Blade, PatternSource::Contextual { context, .. }) => blade_text(context),
        (Family::Python | Family::Ecma | Family::Php, _) => false,
    };
    if !php {
        let pattern = match source {
            PatternSource::Plain(text) => Pattern::try_new(text, language)?,
            PatternSource::Contextual { context, selector } => {
                Pattern::contextual(context, selector, language)?
            }
        };
        return Ok((pattern, language));
    }
    let dialect = GrammarDialect::Php;
    let mut pattern = match source {
        PatternSource::Plain(text) => Pattern::try_new(text, dialect)?,
        PatternSource::Contextual { context, selector } => {
            // The selected node's own kind restricts what it matches; `Pattern::contextual`
            // records it in PHP's numbering, so the pattern is built from the node itself.
            let root = dialect.ast_grep(dialect.pre_process_pattern(context));
            let kind = ast_grep_core::matcher::KindMatcher::try_new(selector, dialect)?;
            let Some(node) = root.root().find(&kind) else {
                return Err(PatternError::NoSelectorInContext {
                    context: context.to_owned(),
                    selector: selector.to_owned(),
                });
            };
            let pattern = Pattern::from(node.get_node().clone());
            if matches!(pattern.node, PatternNode::MetaVar { .. }) {
                return Err(PatternError::Parse(format!(
                    "the selector `{selector}` selects a metavariable, which matches the selected kind only in a pattern of the template's own grammar; select the node around it"
                )));
            }
            pattern
        }
    };
    shift_kinds(&mut pattern.node, &BLADE_EMBEDDED);
    Ok((pattern, dialect))
}

/// Whether Blade reads `source` as text alone, with no directive, echo, `@php`, component, HTML
/// element or comment: a rule's pattern that is PHP (`rule_pattern`).
fn blade_text(source: &str) -> bool {
    use ast_grep_core::Language;
    use ast_grep_language::LanguageExt;
    let blade = GrammarDialect::Blade;
    let root = blade.ast_grep(blade.pre_process_pattern(source));
    root.root()
        .dfs()
        .all(|node| !node.is_named() || matches!(&*node.kind(), "document" | "text" | "ERROR"))
}

/// Renumbers a PHP pattern's kinds as a Blade tape numbers PHP's.
fn shift_kinds(node: &mut PatternNode, embedded: &Embedded) {
    match node {
        PatternNode::Terminal { kind_id, .. } => *kind_id = embedded.shift(*kind_id),
        PatternNode::Internal { kind_id, children } => {
            *kind_id = embedded.shift(*kind_id);
            for child in children {
                shift_kinds(child, embedded);
            }
        }
        PatternNode::MetaVar { .. } => {}
    }
}

/// One grammar dialect: the file label hosts and the store use, and its persisted tape tag (never
/// reused). Its extensions are the host's; tests read them through `label_of_path`.
struct Dialect {
    lang: GrammarDialect,
    label: &'static str,
    #[cfg(test)]
    extensions: &'static [&'static str],
    tape_tag: u8,
}

const DIALECTS: &[Dialect] = &[
    Dialect {
        lang: GrammarDialect::Python,
        label: "python",
        #[cfg(test)]
        extensions: &["py", "pyi"],
        tape_tag: 1,
    },
    Dialect {
        lang: GrammarDialect::TypeScript,
        label: "typescript",
        #[cfg(test)]
        extensions: &["ts", "mts", "cts"],
        tape_tag: 2,
    },
    Dialect {
        lang: GrammarDialect::Tsx,
        label: "tsx",
        #[cfg(test)]
        extensions: &["tsx"],
        tape_tag: 3,
    },
    Dialect {
        lang: GrammarDialect::JavaScript,
        label: "javascript",
        #[cfg(test)]
        extensions: &["js", "jsx", "mjs", "cjs"],
        tape_tag: 6,
    },
    Dialect {
        lang: GrammarDialect::Php,
        label: "php",
        #[cfg(test)]
        extensions: &["php"],
        tape_tag: 7,
    },
    Dialect {
        lang: GrammarDialect::Blade,
        label: "blade",
        #[cfg(test)]
        extensions: &["blade.php"],
        tape_tag: 8,
    },
];

/// Grammars only tests parse (tape round trips over other shapes); no file or code rule selects them.
#[cfg(test)]
const TEST_TAPE_TAGS: &[(GrammarDialect, u8)] =
    &[(GrammarDialect::Scala, 4), (GrammarDialect::Go, 5)];

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

/// The file label a path's extension selects, as the host's table does for dialect files.
#[cfg(test)]
pub(crate) fn label_of_path(path: &str) -> Option<&'static str> {
    let path = path.to_ascii_lowercase();
    DIALECTS
        .iter()
        .filter(|dialect| {
            dialect
                .extensions
                .iter()
                .any(|extension| path.ends_with(&format!(".{extension}")))
        })
        .max_by_key(|dialect| {
            dialect
                .extensions
                .iter()
                .filter(|extension| path.ends_with(&format!(".{extension}")))
                .map(|extension| extension.len())
                .max()
                .expect("filtered dialect has a matching extension")
        })
        .map(|dialect| dialect.label)
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

/// A code-rule language: the `language` of a query, a define or a transform.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CodeRuleLanguage {
    Python,
    TypeScript,
    JavaScript,
    Php,
    Blade,
}

impl CodeRuleLanguage {
    pub(crate) const ALL: [Self; 5] = [
        Self::Python,
        Self::TypeScript,
        Self::JavaScript,
        Self::Php,
        Self::Blade,
    ];

    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.name() == name)
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Php => "php",
            Self::Blade => "blade",
        }
    }

    /// The dialects its rules run on, the first being the one its own text (transform templates,
    /// define clauses) is read in.
    pub(crate) const fn dialects(self) -> &'static [GrammarDialect] {
        match self {
            Self::Python => &[GrammarDialect::Python],
            Self::TypeScript => &[
                GrammarDialect::TypeScript,
                GrammarDialect::Tsx,
                GrammarDialect::JavaScript,
            ],
            Self::JavaScript => &[GrammarDialect::JavaScript],
            Self::Php => &[GrammarDialect::Php],
            Self::Blade => &[GrammarDialect::Blade],
        }
    }

    pub(crate) const fn primary(self) -> GrammarDialect {
        self.dialects()[0]
    }

    pub(crate) const fn family(self) -> Family {
        match self {
            Self::Python => Family::Python,
            Self::TypeScript | Self::JavaScript => Family::Ecma,
            Self::Php => Family::Php,
            Self::Blade => Family::Blade,
        }
    }
}

/// The semantic implementation a dialect is read with.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Family {
    Python,
    /// TypeScript, TSX and JavaScript.
    Ecma,
    Php,
    Blade,
}

/// A semantic capability a family may implement.
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

/// Inline suppression comments a family honours.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Suppression {
    /// Ruff's `# noqa`.
    Noqa,
    /// ESLint's `eslint-disable` comments.
    Eslint,
    /// PHPStan's `@phpstan-ignore` comments.
    Phpstan,
    /// Psalm's `@psalm-suppress` tags.
    Psalm,
    /// PHP_CodeSniffer's `phpcs:` annotations.
    Phpcs,
}

impl Family {
    pub(crate) fn of(lang: GrammarDialect) -> Self {
        match lang {
            GrammarDialect::Python => Self::Python,
            GrammarDialect::TypeScript | GrammarDialect::Tsx | GrammarDialect::JavaScript => {
                Self::Ecma
            }
            GrammarDialect::Php => Self::Php,
            GrammarDialect::Blade => Self::Blade,
            #[cfg(test)]
            _ => Self::Ecma,
        }
    }

    /// Whether the family implements `semantic`; a code rule using it otherwise does not compile.
    pub(crate) const fn implements(self, semantic: Semantic) -> bool {
        match (self, semantic) {
            (Self::Python, _) => true,
            (Self::Ecma, Semantic::Scope | Semantic::Value | Semantic::Arguments) => true,
            (Self::Ecma, Semantic::Calls | Semantic::FormatArgs) => false,
            (
                Self::Php,
                Semantic::Scope
                | Semantic::Value
                | Semantic::Arguments
                | Semantic::FormatArgs
                | Semantic::Calls,
            ) => true,
            // Blade's arms read its embedded PHP program (`blade::view`) as PHP, as the Blade
            // compiler oracle (`rerail-fixtures/blade`) checks.
            (
                Self::Blade,
                Semantic::Scope
                | Semantic::Value
                | Semantic::Arguments
                | Semantic::FormatArgs
                | Semantic::Calls,
            ) => true,
        }
    }

    /// Whether the family's values, arguments and variables are PHP's: PHP's, and Blade's, whose
    /// semantics are those of the PHP its template embeds (`blade::view`).
    pub(crate) const fn reads_php(self) -> bool {
        match self {
            Self::Php | Self::Blade => true,
            Self::Python | Self::Ecma => false,
        }
    }

    /// Whether the family's files carry `suppression`'s comments. PHP has three linters' (a PHP
    /// project may run any of them); Blade templates have none: no linter reads a template's
    /// `{{-- --}}` comments for directives.
    pub(crate) const fn honours(self, suppression: Suppression) -> bool {
        match (self, suppression) {
            (Self::Python, Suppression::Noqa) | (Self::Ecma, Suppression::Eslint) => true,
            (Self::Php, Suppression::Phpstan | Suppression::Psalm | Suppression::Phpcs) => true,
            (Self::Python, _) | (Self::Ecma, _) | (Self::Php, _) | (Self::Blade, _) => false,
        }
    }
}

/// The ECMAScript source form that structural `when` gates and scope construction read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SourceType {
    /// Top-level declarations join the global scope, and top-level `this` is the global object.
    Script,
    /// A module scope; top-level `this` is `undefined`.
    Module,
    /// A function-like module scope with CommonJS globals.
    CommonJs,
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
    /// Whether a file at `path` with this module path satisfies the condition. A module path's
    /// text condition needs a module path (`package` one in a package), `notRegex` too.
    pub(crate) fn holds(&self, path: &str, module: Option<&ModulePath>) -> bool {
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
    fn every_dialect_round_trips_its_label_extensions_and_tape_tag() {
        for dialect in DIALECTS {
            assert_eq!(dialect_by_label(dialect.label), Some(dialect.lang));
            assert_eq!(from_tape_tag(dialect.tape_tag), Some(dialect.lang));
            for extension in dialect.extensions {
                assert_eq!(
                    label_of_path(&format!("a/b.{extension}")),
                    Some(dialect.label)
                );
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
}
