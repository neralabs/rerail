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
use ast_grep_core::{
    language::Language,
    matcher::{Pattern, PatternBuilder, PatternError, PatternNode},
    tree_sitter::{StrDoc, TSLanguage},
};
use ast_grep_language::{LanguageExt, SupportLang};
use std::borrow::Cow;

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
    /// TypeScript and TSX, which carry patches: `grammars/tree-sitter-{python,typescript}/
    /// PATCHES.md`), else ast-grep's. ast-grep is built without its Python and TypeScript parsers,
    /// so `SupportLang` only lends these dialects its pattern syntax. Test-only dialects are
    /// ast-grep's. Blade has none: Rerail's port of Forte parses it (`blade::forte`).
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

/// A grammar a dialect's tapes embed (`GrammarDialect::embeds`, from the registry's `embeds`), and
/// where its kind and field ids start in the host dialect's numbering.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Embedded {
    /// The dialect the embedded program is read as.
    pub(crate) dialect: GrammarDialect,
    pub(crate) grammar: &'static TSLanguage,
    /// Its kind ids are its grammar's plus this.
    pub(crate) kind_offset: u16,
    /// Its field ids are its grammar's plus this.
    pub(crate) field_offset: u16,
}

impl Embedded {
    /// Whether a kind id in the host's numbering is this grammar's. ERROR is every grammar's.
    fn owns(&self, id: u16) -> bool {
        id != u16::MAX
            && id >= self.kind_offset
            && usize::from(id - self.kind_offset) < self.grammar.node_kind_count()
    }

    /// One of the grammar's kind ids in the host's numbering. ERROR is every grammar's.
    fn shift(&self, id: u16) -> u16 {
        match id {
            u16::MAX => id,
            _ => id + self.kind_offset,
        }
    }

    /// The id of the grammar's named kind called `kind`, in the host's numbering.
    fn kind_id(&self, kind: &str) -> Option<u16> {
        match self.grammar.id_for_node_kind(kind, true) {
            0 => None,
            id => Some(self.shift(id)),
        }
    }
}

/// The embedded grammars of a dialect with `own_kinds` kinds and `own_fields` fields of its own,
/// numbered after them in order: each grammar's kinds after the previous ones' (tree-sitter
/// numbers a grammar's kinds from 0), its fields likewise (from 1, 0 being none).
fn number_embedded(
    own_kinds: usize,
    own_fields: usize,
    dialects: &[GrammarDialect],
) -> Vec<Embedded> {
    let mut kinds = own_kinds;
    let mut fields = own_fields;
    dialects
        .iter()
        .map(|&dialect| {
            let grammar = dialect
                .grammar()
                .expect("an embedded dialect is parsed with tree-sitter");
            let embedded = Embedded {
                dialect,
                grammar,
                kind_offset: u16::try_from(kinds).expect("a tape's kinds fit a u16"),
                field_offset: u16::try_from(fields).expect("a tape's fields fit a u16"),
            };
            kinds += grammar.node_kind_count();
            fields += grammar.field_count();
            embedded
        })
        .collect()
}

/// Blade's fields its embedded PHP has none of, numbered after PHP's (`KindCatalog`).
static BLADE_OWN_FIELDS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| {
    let php = GrammarDialect::Php
        .grammar()
        .expect("PHP is parsed with tree-sitter");
    blade_tree::FIELD_NAMES[1..]
        .iter()
        .copied()
        .filter(|name| php.field_id_for_name(name).is_none())
        .collect()
});

/// Where a dialect's own kinds come from: a tree-sitter grammar's symbol table, or the kinds of
/// Blade's tree (`blade::forte::tree`, Rerail's port of Forte).
#[derive(Clone, Copy)]
enum OwnKinds {
    Grammar(&'static TSLanguage),
    Blade,
}

impl OwnKinds {
    fn kind_count(self) -> usize {
        match self {
            Self::Grammar(grammar) => grammar.node_kind_count(),
            Self::Blade => blade_tree::KIND_NAMES.len(),
        }
    }

    /// Fields numbered before the embedded grammars': a tree-sitter grammar's own. Blade's come
    /// after its embedded PHP's (`KindCatalog`).
    fn leading_field_count(self) -> usize {
        match self {
            Self::Grammar(grammar) => grammar.field_count(),
            Self::Blade => 0,
        }
    }

    /// The id of its named kind called `kind` (0 where there is none).
    fn kind_id(self, kind: &str) -> u16 {
        match self {
            Self::Grammar(grammar) => grammar.id_for_node_kind(kind, true),
            Self::Blade => blade_tree::KIND_NAMES[..usize::from(blade_tree::FIRST_ANONYMOUS)]
                .iter()
                .skip(1)
                .position(|name| *name == kind)
                .map_or(0, |index| index as u16 + 1),
        }
    }
}

/// The node kinds and fields a dialect's tapes carry and its rules name: its own, then those of
/// each grammar it embeds (`GrammarDialect::embeds`, in registry order), each grammar's kinds and
/// fields after the previous ones'. A dialect's own kinds are its tree-sitter grammar's, or for
/// Blade its tree's (`blade::forte::tree::KIND_NAMES`, ids from 1, ERROR aside). A Blade tape also
/// holds the PHP its template embeds (`blade::view`), so Blade numbers its own kinds first and
/// PHP's after them (PHP's id plus Blade's kind count). A name the dialect's own kinds give a named
/// kind is its own (Blade's `attribute`, `comment`, `php_tag`, `text`); any other is the first
/// embedded grammar's that has it. A tree-sitter dialect's fields come first; Blade's are PHP's,
/// then those of its tree PHP has none of (`closing_name`, `expression`, ...): a field both name
/// (`name`, `value`, `arguments`) has one id. Rules and tapes read kinds and fields only here.
#[derive(Clone, Copy)]
pub(crate) struct KindCatalog {
    dialect: GrammarDialect,
    own: OwnKinds,
    embedded: &'static [Embedded],
}

impl KindCatalog {
    /// The tree-sitter grammar the dialect's files are parsed with; `None` for Blade's.
    pub(crate) fn grammar(self) -> Option<&'static TSLanguage> {
        match self.own {
            OwnKinds::Grammar(grammar) => Some(grammar),
            OwnKinds::Blade => None,
        }
    }

    /// The embedded grammar `dialect` is read with, if the dialect's tapes embed it.
    pub(crate) fn embedding(self, dialect: GrammarDialect) -> Option<&'static Embedded> {
        self.embedded
            .iter()
            .find(|embedded| embedded.dialect == dialect)
    }

    /// Whether the dialect's tapes embed another grammar's program (Blade's).
    pub(crate) fn has_embedded(self) -> bool {
        !self.embedded.is_empty()
    }

    /// The embedded grammar a kind id belongs to, if not the dialect's own.
    fn embedded_home(self, id: u16) -> Option<&'static Embedded> {
        self.embedded.iter().find(|embedded| embedded.owns(id))
    }

    /// Whether a kind id is an embedded program's (a Blade tape's PHP), not the dialect's own.
    pub(crate) fn is_embedded_kind(self, id: u16) -> bool {
        self.embedded_home(id).is_some()
    }

    /// The dialect a node of kind `id` is read as: the embedded grammar's, or the dialect's own.
    /// ERROR is the dialect's own.
    pub(crate) fn dialect_of_kind(self, id: u16) -> GrammarDialect {
        self.embedded_home(id)
            .map_or(self.dialect, |embedded| embedded.dialect)
    }

    /// The tree-sitter grammar a kind id is a symbol of, and its id there; `None` for one of
    /// Blade's own kinds.
    fn kind_home(self, id: u16) -> Option<(&'static TSLanguage, u16)> {
        if let Some(embedded) = self.embedded_home(id) {
            return Some((embedded.grammar, id - embedded.kind_offset));
        }
        match (self.own, self.embedded.first()) {
            (OwnKinds::Grammar(grammar), _) => Some((grammar, id)),
            // ERROR is every grammar's: in a Blade tape, only the embedded PHP's.
            (OwnKinds::Blade, Some(embedded)) if id == u16::MAX => Some((embedded.grammar, id)),
            (OwnKinds::Blade, _) => None,
        }
    }

    pub(crate) fn kind_count(self) -> usize {
        self.embedded
            .last()
            .map_or(self.own.kind_count(), |embedded| {
                usize::from(embedded.kind_offset) + embedded.grammar.node_kind_count()
            })
    }

    /// The id of the named kind called `kind` (0 where there is none): the dialect's own, else
    /// the first embedded grammar's.
    pub(crate) fn kind_id(self, kind: &str) -> u16 {
        match self.own.kind_id(kind) {
            0 => self
                .embedded
                .iter()
                .find_map(|embedded| embedded.kind_id(kind))
                .unwrap_or(0),
            own => own,
        }
    }

    /// Every id a rule's `kind: <kind>` names: the dialect's own, or else every embedded
    /// grammar's of that name (a file's program is one of them). In a dialect that embeds code,
    /// `comment` names its comments of every kind and the embedded grammars' alike: a Blade
    /// template's `<!-- -->`, `{{-- --}}`, `<!--[if ...]>` and `<!...>` and its PHP's `comment`.
    pub(crate) fn kind_ids(self, kind: &str) -> Vec<u16> {
        let own = self.own.kind_id(kind);
        if kind == "comment" && self.has_embedded() {
            let other_own = match self.own {
                OwnKinds::Blade => &["blade_comment", "conditional_comment", "bogus_comment"][..],
                OwnKinds::Grammar(_) => &[],
            };
            return (own != 0)
                .then_some(own)
                .into_iter()
                .chain(
                    other_own
                        .iter()
                        .map(|kind| self.own.kind_id(kind))
                        .filter(|&id| id != 0),
                )
                .chain(
                    self.embedded
                        .iter()
                        .filter_map(|embedded| embedded.kind_id(kind)),
                )
                .collect();
        }
        if own != 0 {
            return vec![own];
        }
        self.embedded
            .iter()
            .filter_map(|embedded| embedded.kind_id(kind))
            .collect()
    }

    pub(crate) fn kind_name(self, id: u16) -> Option<&'static str> {
        match self.kind_home(id) {
            Some((grammar, id)) => grammar.node_kind_for_id(id),
            None => blade_tree::KIND_NAMES
                .get(usize::from(id))
                .copied()
                .filter(|name| !name.is_empty()),
        }
    }

    pub(crate) fn kind_is_named(self, id: u16) -> bool {
        match self.kind_home(id) {
            Some((grammar, id)) => grammar.node_kind_is_named(id),
            None => id != 0 && id < blade_tree::FIRST_ANONYMOUS,
        }
    }

    pub(crate) fn kind_is_visible(self, id: u16) -> bool {
        match self.kind_home(id) {
            Some((grammar, id)) => grammar.node_kind_is_visible(id),
            None => id != 0 && usize::from(id) < blade_tree::KIND_NAMES.len(),
        }
    }

    pub(crate) fn kind_is_supertype(self, id: u16) -> bool {
        self.kind_home(id)
            .is_some_and(|(grammar, id)| grammar.node_kind_is_supertype(id))
    }

    /// Fields of the dialect's own numbered after the embedded grammars' (Blade's), and where
    /// they start.
    fn trailing_fields(self) -> (usize, &'static [&'static str]) {
        let embedded = self
            .embedded
            .last()
            .map_or(self.own.leading_field_count(), |embedded| {
                usize::from(embedded.field_offset) + embedded.grammar.field_count()
            });
        match self.own {
            OwnKinds::Grammar(_) => (embedded, &[]),
            OwnKinds::Blade => (embedded, BLADE_OWN_FIELDS.as_slice()),
        }
    }

    pub(crate) fn field_count(self) -> usize {
        let (before, trailing) = self.trailing_fields();
        before + trailing.len()
    }

    /// The id of the field called `name`: the dialect's grammar's, else the first embedded
    /// grammar's, else one of Blade's own.
    pub(crate) fn field_id(self, name: &str) -> Option<u16> {
        let own = match self.own {
            OwnKinds::Grammar(grammar) => grammar.field_id_for_name(name).map(|id| id.get()),
            OwnKinds::Blade => None,
        };
        own.or_else(|| {
            self.embedded.iter().find_map(|embedded| {
                embedded
                    .grammar
                    .field_id_for_name(name)
                    .map(|id| id.get() + embedded.field_offset)
            })
        })
        .or_else(|| {
            let (before, trailing) = self.trailing_fields();
            trailing
                .iter()
                .position(|own| *own == name)
                .map(|index| (before + index + 1) as u16)
        })
    }

    pub(crate) fn field_name(self, id: u16) -> Option<&'static str> {
        if let OwnKinds::Grammar(grammar) = self.own
            && usize::from(id) <= grammar.field_count()
        {
            return grammar.field_name_for_id(id);
        }
        let (before, trailing) = self.trailing_fields();
        if let Some(index) = usize::from(id).checked_sub(before + 1) {
            return trailing.get(index).copied();
        }
        self.embedded.iter().find_map(|embedded| {
            let local = id.checked_sub(embedded.field_offset)?;
            (local != 0 && usize::from(local) <= embedded.grammar.field_count())
                .then(|| embedded.grammar.field_name_for_id(local))
                .flatten()
        })
    }

    /// A catalog of `own` embedding `embedded` (numbered by `number_embedded`): what a two-grammar
    /// embedding (Vue's scripts) looks like before a supported dialect has one.
    #[cfg(test)]
    fn with_embedded(own: GrammarDialect, embedded: &'static [Embedded]) -> Self {
        Self {
            dialect: own,
            own: OwnKinds::Grammar(own.grammar().expect("a tree-sitter dialect")),
            embedded,
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

    /// Where the dialect's own kinds come from: Blade's tree (Forte, the registry's `parser`
    /// `forte`), else its tree-sitter grammar.
    fn own_kinds(self) -> OwnKinds {
        match self.grammar() {
            Some(grammar) => OwnKinds::Grammar(grammar),
            None => OwnKinds::Blade,
        }
    }

    /// The dialect's kinds and fields.
    pub(crate) fn kinds(self) -> KindCatalog {
        static EMBEDDED: std::sync::LazyLock<
            std::collections::HashMap<GrammarDialect, Vec<Embedded>>,
        > = std::sync::LazyLock::new(|| {
            GrammarDialect::ALL
                .iter()
                .map(|&dialect| {
                    let mut embeds = Vec::new();
                    for &(_, embedded) in dialect.embeds() {
                        if !embeds.contains(&embedded) {
                            embeds.push(embedded);
                        }
                    }
                    let own = dialect.own_kinds();
                    let numbered =
                        number_embedded(own.kind_count(), own.leading_field_count(), &embeds);
                    (dialect, numbered)
                })
                .collect()
        });
        KindCatalog {
            dialect: self,
            own: self.own_kinds(),
            embedded: &EMBEDDED[&self],
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
    let text = match &source {
        PatternSource::Plain(text) => text,
        PatternSource::Contextual { context, .. } => context,
    };
    let Some(dialect) = adapter::with_adapter!(
        Family::of(language),
        A => <A as adapter::SyntaxRules>::embedded_pattern(text)
    ) else {
        let pattern = match source {
            PatternSource::Plain(text) => Pattern::try_new(text, language)?,
            PatternSource::Contextual { context, selector } => {
                Pattern::contextual(context, selector, language)?
            }
        };
        return Ok((pattern, language));
    };
    let mut pattern = match source {
        PatternSource::Plain(text) => Pattern::try_new(text, dialect)?,
        PatternSource::Contextual { context, selector } => {
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
    let embedded = language
        .kinds()
        .embedding(dialect)
        .expect("embedded pattern dialect is registered by the host");
    shift_kinds(&mut pattern.node, embedded);
    Ok((pattern, dialect))
}

/// Whether Blade reads `source` as text alone, with no directive, echo, `@php`, component, HTML
/// element or comment: a rule's pattern that is PHP (`rule_pattern`).
fn blade_text(source: &str) -> bool {
    use ast_grep_core::source::SgNode;
    let doc = BladeLang.doc(BladeLang.pre_process_pattern(source).as_bytes());
    let mut stack = vec![doc.tree.root()];
    while let Some(node) = stack.pop() {
        if node.is_named() && !matches!(&*node.kind(), "document" | "text") {
            return false;
        }
        stack.extend(node.children());
    }
    true
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
/// reused). Its suffixes are the host's routing; tests read them through `label_of_path`. The
/// rows are the registry's supported dialects (`DIALECTS`, generated).
struct Dialect {
    lang: GrammarDialect,
    label: &'static str,
    #[cfg(test)]
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

/// The family `Family::of` reads a test-only grammar with.
#[cfg(test)]
const TEST_DIALECT_FAMILY: Family = Family::Ecma;

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

/// The file label a path's suffix selects, as the host's table does for dialect files: the
/// longest matching suffix.
#[cfg(test)]
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

    pub(crate) const fn primary(self) -> GrammarDialect {
        self.dialects()[0]
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

    /// Blade's tapes number PHP's kinds after Blade's own (Forte's tree), PHP's fields first and
    /// Blade's own after them: the numbering persisted Blade tapes were written in.
    #[test]
    fn blade_numbers_php_after_its_own_kinds_and_its_own_fields_after_php() {
        let blade = GrammarDialect::Blade.kinds();
        let php = GrammarDialect::Php.grammar().expect("PHP's grammar");
        let offset = u16::try_from(blade_tree::KIND_NAMES.len()).unwrap();
        assert!(blade.grammar().is_none());
        assert_eq!(
            blade.embedding(GrammarDialect::Php).unwrap().kind_offset,
            offset
        );
        for id in 0..u16::try_from(php.node_kind_count()).unwrap() {
            assert!(blade.is_embedded_kind(id + offset), "{id}");
            assert_eq!(blade.dialect_of_kind(id + offset), GrammarDialect::Php);
            assert_eq!(
                blade.kind_name(id + offset),
                php.node_kind_for_id(id),
                "{id}"
            );
        }
        for id in 1..=u16::try_from(php.field_count()).unwrap() {
            assert_eq!(blade.field_name(id), php.field_name_for_id(id), "{id}");
        }
        for (index, name) in BLADE_OWN_FIELDS.iter().enumerate() {
            let id = u16::try_from(php.field_count() + index + 1).unwrap();
            assert_eq!(blade.field_id(name), Some(id));
            assert_eq!(blade.field_name(id), Some(*name));
        }
        assert_eq!(
            blade.field_count(),
            php.field_count() + BLADE_OWN_FIELDS.len()
        );
        assert_eq!(
            blade.kind_id("variable_name"),
            php.id_for_node_kind("variable_name", true) + offset
        );
        let own = |kind| OwnKinds::Blade.kind_id(kind);
        assert_eq!(blade.kind_id("comment"), own("comment"));
        assert_eq!(
            blade.kind_ids("comment"),
            [
                own("comment"),
                own("blade_comment"),
                own("conditional_comment"),
                own("bogus_comment"),
                php.id_for_node_kind("comment", true) + offset,
            ]
        );
        assert_eq!(
            blade.kind_count(),
            usize::from(offset) + php.node_kind_count()
        );
        assert!(!blade.is_embedded_kind(u16::MAX));
        assert!(!GrammarDialect::Php.kinds().has_embedded());
    }

    /// A dialect embedding two grammars (as Vue's scripts will, in TypeScript or JavaScript)
    /// numbers each one's kinds and fields after the previous grammars', and a kind or field id
    /// reads back as its own grammar's.
    #[test]
    fn two_embedded_grammars_number_their_kinds_and_fields_in_turn() {
        static EMBEDDED: std::sync::LazyLock<Vec<Embedded>> = std::sync::LazyLock::new(|| {
            let json = GrammarDialect::Json.grammar().expect("JSON's grammar");
            number_embedded(
                json.node_kind_count(),
                json.field_count(),
                &[GrammarDialect::Go, GrammarDialect::Scala],
            )
        });
        let catalog = KindCatalog::with_embedded(GrammarDialect::Json, &EMBEDDED);
        let grammar = |dialect: GrammarDialect| dialect.grammar().expect("a tree-sitter grammar");
        let (json, go, scala) = (
            grammar(GrammarDialect::Json),
            grammar(GrammarDialect::Go),
            grammar(GrammarDialect::Scala),
        );
        let [first, second] = [&EMBEDDED[0], &EMBEDDED[1]];
        assert_eq!(usize::from(first.kind_offset), json.node_kind_count());
        assert_eq!(
            usize::from(second.kind_offset),
            json.node_kind_count() + go.node_kind_count()
        );
        assert_eq!(usize::from(first.field_offset), json.field_count());
        assert_eq!(
            usize::from(second.field_offset),
            json.field_count() + go.field_count()
        );
        assert_eq!(
            catalog.kind_count(),
            json.node_kind_count() + go.node_kind_count() + scala.node_kind_count()
        );
        for (embedded, grammar) in [(first, go), (second, scala)] {
            for id in 0..u16::try_from(grammar.node_kind_count()).unwrap() {
                let shifted = id + embedded.kind_offset;
                assert_eq!(catalog.dialect_of_kind(shifted), embedded.dialect);
                assert_eq!(catalog.kind_name(shifted), grammar.node_kind_for_id(id));
                assert_eq!(
                    catalog.kind_is_named(shifted),
                    grammar.node_kind_is_named(id)
                );
            }
            for id in 1..=u16::try_from(grammar.field_count()).unwrap() {
                assert_eq!(
                    catalog.field_name(id + embedded.field_offset),
                    grammar.field_name_for_id(id)
                );
            }
        }
        // JSON has no `identifier`: a rule's `kind: identifier` names Go's and Scala's.
        assert_eq!(json.id_for_node_kind("identifier", true), 0);
        assert_eq!(
            catalog.kind_ids("identifier"),
            [
                go.id_for_node_kind("identifier", true) + first.kind_offset,
                scala.id_for_node_kind("identifier", true) + second.kind_offset,
            ]
        );
        assert_eq!(
            catalog.kind_id("identifier"),
            catalog.kind_ids("identifier")[0]
        );
        assert_eq!(catalog.dialect_of_kind(u16::MAX), GrammarDialect::Json);
        assert_eq!(catalog.kind_name(0), json.node_kind_for_id(0));
        assert_eq!(
            catalog
                .embedding(GrammarDialect::Scala)
                .map(|embedded| embedded.kind_offset),
            Some(second.kind_offset)
        );
        assert!(catalog.embedding(GrammarDialect::Python).is_none());
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
        assert_eq!(planned, PLANNED_LANGUAGES);
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
}
