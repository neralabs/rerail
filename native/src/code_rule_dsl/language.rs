//! The languages Rerail reads. Every question about a language asks here: which grammar dialects
//! a code-rule language spans, which dialect a file label selects, a dialect's tape tag, its node
//! kinds and fields (`KindCatalog`, which rules and tapes share), which semantic family serves a
//! grammar, and which capabilities a language supports. The language registry
//! (`languages/*.json`, `docs/language-packs.md`) is their one source: `build.rs` validates it
//! and generates `DIALECTS`, `CodeRuleLanguage`, `PLANNED_LANGUAGES`, `Family::of`,
//! `Family::honours` and `with_adapter!` from it (`language_registry.rs` in `OUT_DIR`). The host
//! routes files by the same registry's suffixes (`SOURCE_LANGUAGES` in code-rule-languages.ts);
//! the repository index receives them.
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
//! Adding a language: add its registry entry (`planned` first), its `GrammarDialect` variant and,
//! for a new family, its adapter (`adapter`): one type implementing `Adapter`, whose associated
//! types name the family's implementation of each concern, or `Refused`. `with_adapter!`,
//! generated from the registry's families, dispatches every family question to it (the build
//! fails until it exists). A rule that names a planned language, or a capability its language
//! has `not-supported-yet`, is refused as `language-not-supported`; a `refused` capability is
//! refused as invalid, never answered by another family's code.

pub(crate) mod adapter;

use crate::code_rule_dsl::blade::forte::tree::{self as blade_tree, BladeLang};
use crate::code_rule_dsl::module_path::ModulePath;
use ast_grep_core::{
    language::Language,
    matcher::{Pattern, PatternBuilder, PatternError, PatternNode},
    tree_sitter::{StrDoc, TSLanguage},
};
use ast_grep_language::{LanguageExt, SupportLang};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::sync::{Arc, LazyLock};

/// Rerail's grammar dialect. `ast_grep_language::SupportLang` is closed, while Blade (parsed by
/// Rerail's port of Forte, `blade::forte`) is part of Rerail's supported source surface.
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
    php: GrammarDialect::Php
        .grammar()
        .expect("PHP is parsed with tree-sitter"),
    offset: u16::try_from(blade_tree::KIND_NAMES.len()).expect("Blade's kinds fit a u16"),
});

/// Blade's fields the PHP grammar has none of, numbered after PHP's (`KindCatalog`).
static BLADE_OWN_FIELDS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| {
    let php = BLADE_EMBEDDED.php;
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

/// The node kinds and fields a dialect's tapes carry and its rules name: its grammar's, except
/// Blade's. Blade's own kinds are its tree's (`blade::forte::tree::KIND_NAMES`, ids from 1, ERROR
/// aside). A Blade tape also holds the PHP its template embeds (`blade::view`), so Blade numbers
/// its own kinds first and PHP's after them (PHP's id plus Blade's kind count). A name both give a
/// named kind (`attribute`, `comment`, `php_tag`, `text`) is Blade's. Blade's fields are PHP's,
/// then those of its tree PHP has none of (`closing_name`, `expression`, ...): a field both name
/// (`name`, `value`, `arguments`) has one id. Rules and tapes read kinds and fields only here.
#[derive(Clone, Copy)]
pub(crate) struct KindCatalog {
    own: OwnKinds,
    embedded: Option<&'static Embedded>,
}

impl KindCatalog {
    /// The tree-sitter grammar the dialect's files are parsed with; `None` for Blade's.
    pub(crate) fn grammar(self) -> Option<&'static TSLanguage> {
        match self.own {
            OwnKinds::Grammar(grammar) => Some(grammar),
            OwnKinds::Blade => None,
        }
    }

    /// The PHP the dialect's tapes embed: Blade's.
    pub(crate) fn embedded(self) -> Option<&'static Embedded> {
        self.embedded
    }

    /// Whether a kind id is the embedded PHP's (a Blade tape's program), not the dialect's own.
    pub(crate) fn is_embedded_kind(self, id: u16) -> bool {
        self.embedded.is_some_and(|embedded| embedded.owns(id))
    }

    /// The tree-sitter grammar a kind id is a symbol of, and its id there; `None` for one of
    /// Blade's own kinds.
    fn kind_home(self, id: u16) -> Option<(&'static TSLanguage, u16)> {
        match (self.embedded, self.own) {
            (Some(embedded), _) if embedded.owns(id) => Some((embedded.php, id - embedded.offset)),
            (_, OwnKinds::Grammar(grammar)) => Some((grammar, id)),
            // ERROR is every grammar's: in a Blade tape, only the embedded PHP's.
            (Some(embedded), OwnKinds::Blade) if id == u16::MAX => Some((embedded.php, id)),
            (_, OwnKinds::Blade) => None,
        }
    }

    fn own_kind_count(self) -> usize {
        match self.own {
            OwnKinds::Grammar(grammar) => grammar.node_kind_count(),
            OwnKinds::Blade => blade_tree::KIND_NAMES.len(),
        }
    }

    pub(crate) fn kind_count(self) -> usize {
        self.own_kind_count()
            + self
                .embedded
                .map_or(0, |embedded| embedded.php.node_kind_count())
    }

    /// The id of the named kind called `kind` (0 where there is none).
    pub(crate) fn kind_id(self, kind: &str) -> u16 {
        let own = match self.own {
            OwnKinds::Grammar(grammar) => grammar.id_for_node_kind(kind, true),
            OwnKinds::Blade => blade_tree::KIND_NAMES[..usize::from(blade_tree::FIRST_ANONYMOUS)]
                .iter()
                .skip(1)
                .position(|name| *name == kind)
                .map_or(0, |index| index as u16 + 1),
        };
        match self.embedded {
            Some(embedded) if own == 0 => match embedded.php.id_for_node_kind(kind, true) {
                0 => 0,
                id => embedded.shift(id),
            },
            _ => own,
        }
    }

    /// The kinds besides `kind_id("comment")` a rule's `kind: comment` names: in a Blade tape, the
    /// template's other comments (`{{-- --}}`, `<!--[if ...]>`, `<!...>`) and its embedded PHP's
    /// `comment`, which the template's own `comment` (`<!-- -->`) shadows in `kind_id`.
    pub(crate) fn other_comment_kinds(self) -> Vec<u16> {
        let Some(embedded) = self.embedded else {
            return Vec::new();
        };
        let mut kinds = ["blade_comment", "conditional_comment", "bogus_comment"]
            .into_iter()
            .map(|kind| self.kind_id(kind))
            .filter(|&id| id != 0)
            .collect::<Vec<_>>();
        match embedded.php.id_for_node_kind("comment", true) {
            0 => {}
            id => kinds.push(embedded.shift(id)),
        }
        kinds
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

    /// The grammar whose fields the dialect's are, Blade's own after PHP's.
    fn fields(self) -> (&'static TSLanguage, &'static [&'static str]) {
        match (self.own, self.embedded) {
            (OwnKinds::Grammar(grammar), None) => (grammar, &[]),
            (_, Some(embedded)) => (embedded.php, BLADE_OWN_FIELDS.as_slice()),
            (OwnKinds::Blade, None) => unreachable!("Blade's tapes embed PHP"),
        }
    }

    pub(crate) fn field_count(self) -> usize {
        let (grammar, own) = self.fields();
        grammar.field_count() + own.len()
    }

    pub(crate) fn field_id(self, name: &str) -> Option<u16> {
        let (grammar, own) = self.fields();
        grammar
            .field_id_for_name(name)
            .map(|id| id.get())
            .or_else(|| {
                own.iter()
                    .position(|own| *own == name)
                    .map(|index| (grammar.field_count() + index + 1) as u16)
            })
    }

    pub(crate) fn field_name(self, id: u16) -> Option<&'static str> {
        let (grammar, own) = self.fields();
        match usize::from(id).checked_sub(grammar.field_count() + 1) {
            Some(index) => own.get(index).copied(),
            None => grammar.field_name_for_id(id),
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

    /// The dialect's kinds and fields.
    pub(crate) fn kinds(self) -> KindCatalog {
        match self {
            Self::Blade => KindCatalog {
                own: OwnKinds::Blade,
                embedded: Some(&*BLADE_EMBEDDED),
            },
            _ => KindCatalog {
                own: OwnKinds::Grammar(
                    self.grammar()
                        .expect("a dialect other than Blade is parsed with tree-sitter"),
                ),
                embedded: None,
            },
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

type PatternKey = [u8; 32];
type CompiledPattern = (Arc<Pattern>, GrammarDialect);
type PatternPrograms = FxHashMap<PatternKey, CompiledPattern>;
static PATTERNS: LazyLock<Mutex<PatternPrograms>> =
    LazyLock::new(|| Mutex::new(FxHashMap::default()));
const MAX_PATTERNS: usize = 4_096;

/// A rule's pattern compiled for `language`, with the dialect its source was parsed as. A Blade
/// pattern that holds no Blade construct (Blade reads it as text alone, `sprintf($F, $$$A)`) is
/// PHP: it is parsed with PHP's grammar and matches the template's embedded program
/// (`blade::view`), its kinds numbered as Blade's tapes number PHP's.
pub(crate) fn rule_pattern(
    source: PatternSource<'_>,
    language: GrammarDialect,
) -> Result<(Arc<Pattern>, GrammarDialect), PatternError> {
    let mut digest = Sha256::new();
    digest.update([language as u8]);
    let text = match &source {
        PatternSource::Plain(context) => {
            digest.update([0]);
            digest.update((context.len() as u64).to_le_bytes());
            digest.update(context.as_bytes());
            *context
        }
        PatternSource::Contextual { context, selector } => {
            digest.update([1]);
            digest.update((context.len() as u64).to_le_bytes());
            digest.update(context.as_bytes());
            digest.update((selector.len() as u64).to_le_bytes());
            digest.update(selector.as_bytes());
            *context
        }
    };
    let key: PatternKey = digest.finalize().into();
    if let Some(compiled) = PATTERNS.lock().get(&key).cloned() {
        return Ok(compiled);
    }
    let php = adapter::with_adapter!(
        Family::of(language),
        A => <A as adapter::SyntaxRules>::embeds_pattern(text)
    );
    let (mut pattern, dialect) = if !php {
        let pattern = match source {
            PatternSource::Plain(text) => Pattern::try_new(text, language)?,
            PatternSource::Contextual { context, selector } => {
                Pattern::contextual(context, selector, language)?
            }
        };
        (pattern, language)
    } else {
        let dialect = GrammarDialect::Php;
        let pattern = match source {
            PatternSource::Plain(text) => Pattern::try_new(text, dialect)?,
            PatternSource::Contextual { context, selector } => {
                // The selected node's own kind restricts what it matches. Build the pattern from
                // that node before its PHP kind numbers move into Blade's combined catalog.
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
        (pattern, dialect)
    };
    if php {
        shift_kinds(&mut pattern.node, &BLADE_EMBEDDED);
    }
    let compiled = (Arc::new(pattern), dialect);
    let mut patterns = PATTERNS.lock();
    if let Some(existing) = patterns.get(&key) {
        return Ok(existing.clone());
    }
    if patterns.len() == MAX_PATTERNS {
        patterns.clear();
    }
    patterns.insert(key, compiled.clone());
    Ok(compiled)
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

// `DIALECTS`, `registry_grammar`, `CodeRuleLanguage` (`ALL`, `name`, `dialects`, `family`,
// `capability`), `PLANNED_LANGUAGES`, `Family::of`, `Family::honours` and `with_adapter!`.
include!(concat!(env!("OUT_DIR"), "/language_registry.rs"));

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

/// The semantic implementation a dialect is read with.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Family {
    Python,
    /// TypeScript, TSX and JavaScript.
    Ecma,
    Php,
    /// Its semantics are those of the PHP its template embeds (`blade::view`), as the Blade
    /// compiler oracle (`fixtures/blade`) checks.
    Blade,
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

/// Inline suppression comments a family honours (the registry's `suppressions`). PHP has three
/// linters' (a PHP project may run any of them); Blade templates have none: no linter reads a
/// template's `{{-- --}}` comments for directives.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Suppression {
    /// Ruff's `# noqa`.
    Noqa,
    /// Bandit's `# nosec`.
    Nosec,
    /// ESLint's `eslint-disable` comments.
    Eslint,
    /// PHPStan's `@phpstan-ignore` comments.
    Phpstan,
    /// Psalm's `@psalm-suppress` tags.
    Psalm,
    /// PHP_CodeSniffer's `phpcs:` annotations.
    Phpcs,
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
        let suppressions = [
            ("noqa", Suppression::Noqa),
            ("nosec", Suppression::Nosec),
            ("eslint", Suppression::Eslint),
            ("phpstan", Suppression::Phpstan),
            ("psalm", Suppression::Psalm),
            ("phpcs", Suppression::Phpcs),
        ];
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
            for (name, suppression) in suppressions {
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
