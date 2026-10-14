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

/// A grammar a dialect's tapes embed (`GrammarDialect::embeds`, from the registry's `embeds`), and
/// the codes its kinds and fields have in the host dialect's numbering (`number_embedded`).
#[derive(Debug)]
pub(crate) struct Embedded {
    /// The dialect the embedded program is read as.
    pub(crate) dialect: GrammarDialect,
    pub(crate) grammar: &'static TSLanguage,
    /// Each of its kind ids' code.
    kinds: Box<[u16]>,
    /// The codes it adds, from `kinds_from`: the kind id each stands for.
    added_kinds: Box<[u16]>,
    kinds_from: u16,
    /// Each of its field ids' code (the id 0, no field, is 0).
    fields: Box<[u16]>,
    /// The field codes it adds, from `fields_from`: the field id each stands for.
    added_fields: Box<[u16]>,
    fields_from: u16,
}

impl Embedded {
    /// One of the grammar's kind ids in the host's numbering. ERROR, and an id the grammar does
    /// not have, are `u16::MAX`.
    pub(crate) fn kind_code(&self, id: u16) -> u16 {
        self.kinds.get(usize::from(id)).copied().unwrap_or(u16::MAX)
    }

    /// One of the grammar's field ids in the host's numbering (0, no field, is 0).
    pub(crate) fn field_code(&self, id: u16) -> u16 {
        self.fields[usize::from(id)]
    }

    /// The grammar's kind id a code it added stands for.
    fn added_kind(&self, code: u16) -> Option<u16> {
        let index = code.checked_sub(self.kinds_from)?;
        self.added_kinds.get(usize::from(index)).copied()
    }

    /// The grammar's field id a field code it added stands for.
    fn added_field(&self, code: u16) -> Option<u16> {
        let index = code.checked_sub(self.fields_from)?;
        self.added_fields.get(usize::from(index)).copied()
    }

    /// The code of the grammar's named kind called `kind`.
    fn kind_id(&self, kind: &str) -> Option<u16> {
        match self.grammar.id_for_node_kind(kind, true) {
            0 => None,
            id => Some(self.kind_code(id)),
        }
    }

    /// The code after the last one it adds.
    fn kinds_end(&self) -> usize {
        usize::from(self.kinds_from) + self.added_kinds.len()
    }

    /// The last field code it adds, or the one before its first.
    fn fields_last(&self) -> usize {
        usize::from(self.fields_from) + self.added_fields.len() - 1
    }
}

/// The embedded grammars of a dialect with `own_kinds` kinds and `own_fields` fields of its own,
/// numbered after them in order. The first one's kinds follow the dialect's own (each its id plus
/// `own_kinds`: tree-sitter numbers a grammar's kinds from 0), its fields likewise (from 1, 0
/// being none). A later one shares an earlier one's code for a kind of the same name, namedness
/// and visibility, and for a field of the same name, and adds codes for the rest: the grammars a
/// file's program may be written in (a Vue file's TypeScript, TSX or JavaScript) are mostly one
/// language, and a tape's codes are 10 bits for kinds and 6 for fields. A shared code does not
/// tell which grammar a program is in: the tape records each program's (`tape::EmbeddedProgram`).
fn number_embedded(
    own_kinds: usize,
    own_fields: usize,
    dialects: &[GrammarDialect],
) -> Vec<Embedded> {
    type KindKey = (Option<&'static str>, bool, bool);
    let code = |count: usize| u16::try_from(count).expect("a tape's codes fit a u16");
    let mut next_kind = own_kinds;
    let mut next_field = own_fields + 1;
    let mut kind_codes = std::collections::HashMap::<KindKey, u16>::new();
    let mut field_codes = std::collections::HashMap::<&'static str, u16>::new();
    dialects
        .iter()
        .map(|&dialect| {
            let grammar = dialect
                .grammar()
                .expect("an embedded dialect is parsed with tree-sitter");
            let key = |id: u16| -> KindKey {
                (
                    grammar.node_kind_for_id(id),
                    grammar.node_kind_is_named(id),
                    grammar.node_kind_is_visible(id),
                )
            };
            let (kinds_from, fields_from) = (code(next_kind), code(next_field));
            let mut added_kinds = Vec::new();
            let kinds = (0..code(grammar.node_kind_count()))
                .map(|id| {
                    kind_codes.get(&key(id)).copied().unwrap_or_else(|| {
                        added_kinds.push(id);
                        next_kind += 1;
                        code(next_kind - 1)
                    })
                })
                .collect::<Box<[u16]>>();
            let mut added_fields = Vec::new();
            let fields = std::iter::once(0)
                .chain((1..=code(grammar.field_count())).map(|id| {
                    grammar
                        .field_name_for_id(id)
                        .and_then(|name| field_codes.get(name).copied())
                        .unwrap_or_else(|| {
                            added_fields.push(id);
                            next_field += 1;
                            code(next_field - 1)
                        })
                }))
                .collect::<Box<[u16]>>();
            // A later grammar shares this one's codes; this one's own repeats of a name keep
            // their codes (only one of them is a node's kind: tree-sitter's public symbol).
            for (id, &kind) in kinds.iter().enumerate() {
                kind_codes.entry(key(code(id))).or_insert(kind);
            }
            for (id, &field) in fields.iter().enumerate().skip(1) {
                if let Some(name) = grammar.field_name_for_id(code(id)) {
                    field_codes.entry(name).or_insert(field);
                }
            }
            Embedded {
                dialect,
                grammar,
                kinds,
                added_kinds: added_kinds.into(),
                kinds_from,
                fields,
                added_fields: added_fields.into(),
                fields_from,
            }
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

/// `ids` without repeats, in order (embedded grammars that share a code name it once).
fn unique(mut ids: Vec<u16>) -> Vec<u16> {
    let mut seen = Vec::with_capacity(ids.len());
    ids.retain(|id| {
        let new = !seen.contains(id);
        seen.push(*id);
        new
    });
    ids
}

/// The node kinds and fields a dialect's tapes carry and its rules name: its own, then those of
/// each grammar it embeds (`GrammarDialect::embeds`, in registry order, numbered by
/// `number_embedded`): the first's after the dialect's own, a later one's sharing the first's codes
/// where its names agree. A dialect's own kinds are its tree-sitter grammar's, or for
/// Blade its tree's (`blade::forte::tree::KIND_NAMES`, ids from 1, ERROR aside). A Blade tape also
/// holds the PHP its template embeds (`blade::view`), so Blade numbers its own kinds first and
/// PHP's after them (PHP's id plus Blade's kind count). A name the dialect's own kinds give a named
/// kind is its own (Blade's `attribute`, `comment`, `php_tag`, `text`); any other is the embedded
/// grammars'. A tree-sitter dialect's fields come first; Blade's are PHP's,
/// then those of its tree PHP has none of (`closing_name`, `expression`, ...): a field both name
/// (`name`, `value`, `arguments`) has one id. Rules and tapes read kinds and fields only here.
#[derive(Clone, Copy)]
pub(crate) struct KindCatalog {
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

    /// The grammars the dialect's tapes embed, in numbering order: a tape names each program's
    /// dialect by its index here (`tape::EmbeddedProgram`), as kind codes cannot tell it.
    pub(crate) fn embeddings(self) -> &'static [Embedded] {
        self.embedded
    }

    /// The embedded grammar a kind code belongs to, if not the dialect's own: the one that added
    /// it (a code embedded grammars share is the first's), and the kind id in its grammar.
    fn embedded_home(self, id: u16) -> Option<(&'static Embedded, u16)> {
        self.embedded
            .iter()
            .find_map(|embedded| Some((embedded, embedded.added_kind(id)?)))
    }

    /// Whether a kind id is an embedded program's (a Blade tape's PHP), not the dialect's own.
    pub(crate) fn is_embedded_kind(self, id: u16) -> bool {
        self.embedded_home(id).is_some()
    }

    /// The tree-sitter grammar a kind id is a symbol of, and its id there; `None` for one of
    /// Blade's own kinds.
    fn kind_home(self, id: u16) -> Option<(&'static TSLanguage, u16)> {
        if let Some((embedded, local)) = self.embedded_home(id) {
            return Some((embedded.grammar, local));
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
            .map_or(self.own.kind_count(), Embedded::kinds_end)
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
            return unique(
                (own != 0)
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
                    .collect(),
            );
        }
        if own != 0 {
            return vec![own];
        }
        unique(
            self.embedded
                .iter()
                .filter_map(|embedded| embedded.kind_id(kind))
                .collect(),
        )
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
            .map_or(self.own.leading_field_count(), Embedded::fields_last);
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
                    .map(|id| embedded.field_code(id.get()))
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
            embedded
                .grammar
                .field_name_for_id(embedded.added_field(id)?)
        })
    }

    /// A catalog of `own` embedding `embedded` (numbered by `number_embedded`): what a two-grammar
    /// embedding (Vue's scripts) looks like before a supported dialect has one.
    #[cfg(test)]
    fn with_embedded(own: GrammarDialect, embedded: &'static [Embedded]) -> Self {
        Self {
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
        PatternNode::Terminal { kind_id, .. } => *kind_id = embedded.kind_code(*kind_id),
        PatternNode::Internal { kind_id, children } => {
            *kind_id = embedded.kind_code(*kind_id);
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
        let php_numbering = blade.embedding(GrammarDialect::Php).unwrap();
        for id in 0..u16::try_from(php.node_kind_count()).unwrap() {
            assert_eq!(php_numbering.kind_code(id), id + offset, "{id}");
            assert!(blade.is_embedded_kind(id + offset), "{id}");
            assert_eq!(added_by(blade, id + offset), Some(GrammarDialect::Php));
            assert_eq!(
                blade.kind_name(id + offset),
                php.node_kind_for_id(id),
                "{id}"
            );
        }
        assert_eq!(php_numbering.kind_code(u16::MAX), u16::MAX);
        for id in 1..=u16::try_from(php.field_count()).unwrap() {
            assert_eq!(php_numbering.field_code(id), id, "{id}");
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

    /// A dialect embedding two grammars numbers the first's kinds and fields after its own; the
    /// second shares the first's code for a kind or field of the same name (and namedness and
    /// visibility) and adds the rest after the first's; a code reads back as the grammar that
    /// added it.
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
        let offset = u16::try_from(json.node_kind_count()).unwrap();
        for id in 0..u16::try_from(go.node_kind_count()).unwrap() {
            assert_eq!(first.kind_code(id), id + offset, "{id}");
        }
        let json_fields = u16::try_from(json.field_count()).unwrap();
        for id in 1..=u16::try_from(go.field_count()).unwrap() {
            assert_eq!(first.field_code(id), id + json_fields, "{id}");
        }
        assert_eq!(first.kind_code(u16::MAX), u16::MAX);
        assert_eq!(first.field_code(0), 0);
        // Both have `identifier`, a named, visible kind: one code, Go's.
        let identifier = |grammar: &TSLanguage| grammar.id_for_node_kind("identifier", true);
        assert_eq!(json.id_for_node_kind("identifier", true), 0);
        assert_eq!(
            second.kind_code(identifier(scala)),
            first.kind_code(identifier(go))
        );
        // Scala has kinds Go has not: they follow Go's.
        assert!(catalog.kind_count() > go.node_kind_count() + json.node_kind_count());
        assert!(
            catalog.kind_count()
                < go.node_kind_count() + json.node_kind_count() + scala.node_kind_count()
        );
        for (embedded, grammar) in [(first, go), (second, scala)] {
            for id in 0..u16::try_from(grammar.node_kind_count()).unwrap() {
                let code = embedded.kind_code(id);
                assert_eq!(catalog.kind_name(code), grammar.node_kind_for_id(id));
                assert_eq!(catalog.kind_is_named(code), grammar.node_kind_is_named(id));
                assert_eq!(
                    catalog.kind_is_visible(code),
                    grammar.node_kind_is_visible(id)
                );
            }
            for id in 1..=u16::try_from(grammar.field_count()).unwrap() {
                assert_eq!(
                    catalog.field_name(embedded.field_code(id)),
                    grammar.field_name_for_id(id)
                );
            }
        }
        // A kind only Scala has is a code Scala adds; a shared one, the code Go added.
        let only_scala = (0..u16::try_from(scala.node_kind_count()).unwrap())
            .find(|&id| grammar_kind_absent(go, scala, id))
            .expect("Scala has a kind Go has not");
        assert_eq!(
            added_by(catalog, second.kind_code(only_scala)),
            Some(GrammarDialect::Scala)
        );
        assert_eq!(
            added_by(catalog, first.kind_code(identifier(go))),
            Some(GrammarDialect::Go)
        );
        // JSON has no `identifier`: a rule's `kind: identifier` names the one code.
        assert_eq!(
            catalog.kind_ids("identifier"),
            [first.kind_code(identifier(go))]
        );
        assert_eq!(
            catalog.kind_id("identifier"),
            catalog.kind_ids("identifier")[0]
        );
        assert_eq!(added_by(catalog, u16::MAX), None);
        assert_eq!(catalog.kind_name(0), json.node_kind_for_id(0));
        assert!(std::ptr::eq(
            catalog.embedding(GrammarDialect::Scala).unwrap(),
            second
        ));
        assert!(catalog.embedding(GrammarDialect::Python).is_none());
    }

    /// The embedded grammar that added kind code `code` (`number_embedded`): `None` for the
    /// dialect's own codes and ERROR.
    fn added_by(catalog: KindCatalog, code: u16) -> Option<GrammarDialect> {
        catalog
            .embedded_home(code)
            .map(|(embedded, _)| embedded.dialect)
    }

    /// Whether `other`'s kind `id` has no kind of its name, namedness and visibility in `grammar`.
    fn grammar_kind_absent(grammar: &TSLanguage, other: &TSLanguage, id: u16) -> bool {
        (0..u16::try_from(grammar.node_kind_count()).unwrap()).all(|go_id| {
            (
                grammar.node_kind_for_id(go_id),
                grammar.node_kind_is_named(go_id),
                grammar.node_kind_is_visible(go_id),
            ) != (
                other.node_kind_for_id(id),
                other.node_kind_is_named(id),
                other.node_kind_is_visible(id),
            )
        })
    }

    /// TypeScript, TSX and JavaScript numbered one after another would need more kind codes than
    /// a tape's 10 bits and more field codes than its 6; numbered by name they fit, TSX's and
    /// JavaScript's kinds sharing TypeScript's codes, and every code reads back as the grammar's.
    #[test]
    fn typescript_tsx_and_javascript_share_codes_to_fit_a_tape() {
        const KIND_LIMIT: usize = 1 << 10;
        const FIELD_LIMIT: usize = 1 << 6;
        static EMBEDDED: std::sync::LazyLock<Vec<Embedded>> = std::sync::LazyLock::new(|| {
            let json = GrammarDialect::Json.grammar().expect("JSON's grammar");
            number_embedded(
                json.node_kind_count(),
                json.field_count(),
                &[
                    GrammarDialect::TypeScript,
                    GrammarDialect::Tsx,
                    GrammarDialect::JavaScript,
                ],
            )
        });
        let catalog = KindCatalog::with_embedded(GrammarDialect::Json, &EMBEDDED);
        let json = GrammarDialect::Json.grammar().expect("JSON's grammar");
        let grammars = EMBEDDED.iter().map(|embedded| embedded.grammar);
        let naive_kinds: usize =
            json.node_kind_count() + grammars.clone().map(|g| g.node_kind_count()).sum::<usize>();
        let naive_fields: usize =
            json.field_count() + grammars.map(|g| g.field_count()).sum::<usize>();
        assert!(naive_kinds > KIND_LIMIT, "{naive_kinds}");
        assert!(naive_fields >= FIELD_LIMIT, "{naive_fields}");
        assert!(
            catalog.kind_count() <= KIND_LIMIT,
            "{}",
            catalog.kind_count()
        );
        assert!(
            catalog.field_count() < FIELD_LIMIT,
            "{}",
            catalog.field_count()
        );

        let [typescript, tsx, javascript] = [&EMBEDDED[0], &EMBEDDED[1], &EMBEDDED[2]];
        let offset = u16::try_from(json.node_kind_count()).unwrap();
        for id in 0..u16::try_from(typescript.grammar.node_kind_count()).unwrap() {
            assert_eq!(typescript.kind_code(id), id + offset, "{id}");
        }
        for embedded in [tsx, javascript] {
            let mut shared = 0;
            for id in 0..u16::try_from(embedded.grammar.node_kind_count()).unwrap() {
                fn key(grammar: &TSLanguage, id: u16) -> (Option<&str>, bool, bool) {
                    (
                        grammar.node_kind_for_id(id),
                        grammar.node_kind_is_named(id),
                        grammar.node_kind_is_visible(id),
                    )
                }
                let twin = (0..u16::try_from(typescript.grammar.node_kind_count()).unwrap())
                    .find(|&ts| key(typescript.grammar, ts) == key(embedded.grammar, id));
                if let Some(twin) = twin {
                    shared += 1;
                    assert_eq!(embedded.kind_code(id), typescript.kind_code(twin), "{id}");
                }
            }
            assert!(shared > embedded.grammar.node_kind_count() / 2);
        }
        // JSX is TSX's and JavaScript's, not TypeScript's: JavaScript shares TSX's code for it.
        let jsx = |embedded: &Embedded| {
            embedded.kind_code(embedded.grammar.id_for_node_kind("jsx_element", true))
        };
        assert_ne!(jsx(tsx), u16::MAX);
        assert_eq!(jsx(javascript), jsx(tsx));
        assert_eq!(added_by(catalog, jsx(tsx)), Some(GrammarDialect::Tsx));
        assert_eq!(catalog.kind_ids("jsx_element"), [jsx(tsx)]);
        // Every grammar's `program` is TypeScript's code: a code does not tell which grammar a
        // tape's program is in, so the tape records it (`tape::EmbeddedProgram`).
        let program = |embedded: &Embedded| {
            embedded.kind_code(embedded.grammar.id_for_node_kind("program", true))
        };
        assert_eq!(program(javascript), program(typescript));
        assert_eq!(program(tsx), program(typescript));
        assert_eq!(
            added_by(catalog, program(typescript)),
            Some(GrammarDialect::TypeScript)
        );

        for embedded in [typescript, tsx, javascript] {
            let grammar = embedded.grammar;
            for id in 0..u16::try_from(grammar.node_kind_count()).unwrap() {
                let code = embedded.kind_code(id);
                assert!(catalog.is_embedded_kind(code), "{id}");
                assert_eq!(catalog.kind_name(code), grammar.node_kind_for_id(id));
                assert_eq!(catalog.kind_is_named(code), grammar.node_kind_is_named(id));
                assert_eq!(
                    catalog.kind_is_visible(code),
                    grammar.node_kind_is_visible(id)
                );
            }
            for id in 1..=u16::try_from(grammar.field_count()).unwrap() {
                let code = embedded.field_code(id);
                let name = grammar.field_name_for_id(id);
                assert_eq!(catalog.field_name(code), name, "{id}");
                // One id for a name: the host's own where it has the field, else the shared code.
                let named = catalog.field_id(name.unwrap()).unwrap();
                assert_eq!(catalog.field_name(named), name, "{id}");
                if json.field_id_for_name(name.unwrap()).is_none() {
                    assert_eq!(named, code, "{id}");
                }
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
}
