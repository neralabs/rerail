//! Embedded programs: the code a file of one dialect holds in another's grammar. Blade's template
//! compiles to PHP (`compiled-view`); a Vue single-file component holds TypeScript or JavaScript
//! (`script-block`).
//!
//! The registry declares an embedding (`embeds: [{ language, via }]`); `build.rs` generates the
//! `Via` of each supported one and `GrammarDialect::embeds` (the embedded dialects per host dialect).
//! Each `Via`'s embedder is registered for it as the engine loads (`install_embedders`; Vue's is
//! `embed/script_block.rs`, `ScriptBlock`); a `Via` none registered for panics when a file reaches it.
//!
//! A file that embeds code is several documents: the **host**, the file's own tree, and one
//! **guest** per `via` its dialect embeds that the file holds a program for. Every document holds
//! one grammar and passes the same tape checks. An embedder builds, from a file's tape and source,
//! at most one program, which is
//! - its text, written as the host's language reads the file (a Blade view's PHP, a Vue
//!   component's scripts and template expressions), and its parse, read from that text in the
//!   grammar of `Program::dialect` (`Parse::read`: the dialect's lean parser's tape, or
//!   tree-sitter's tree where it refuses), one of the dialects the host embeds through that `via` (a Vue
//!   file's scripts are TypeScript, TSX or JavaScript, as its `lang` says). Its nodes span the
//!   text, as any tree's do;
//! - where its text stands in the host source (`SegmentMap`), recorded as the program is written:
//!   copied stretches byte for byte, written ones over the host bytes they stand for. A guest node
//!   reports, suppresses and is compared with the host through the host bytes it stands for: the
//!   guest is an `origin::Origin`, and `origin::InFile` places its nodes in the file;
//! - the host bytes it cannot be read through (`Program::host_errors`): a Blade view reads the
//!   template's bytes, so the template's syntax errors are its; a Vue program is built from what
//!   the component's recovered tree holds, so the component's errors are not.
//!
//! Each guest is a document derived from the host's (`tape::DerivedImage`): its text, segments,
//! host errors and a tape of its own, which the store keeps as a row of its own beside the host's
//! (`file_documents`). A rule reads a guest through `ast.embedded` (the clause runs on it) or the
//! `embedded` relation (from a host node); a guest node reads the host through `host`.

// cratify: `compiled_view` is rerail-lang-php's (it mounts it)
// cratify: `script_block` is rerail-lang-ecma's (it mounts it)
// cratify: `call_argument` is rerail-lang-python's (it mounts it: it reads the scope table)
pub mod host;
pub mod inline_content;
// cratify: `json_object` is rerail-rr's (the rule-file language's, decision 0495)
pub mod json_object;
// cratify: `match_regex` is rerail-rr's (the rule-file language's, decision 0495)
pub mod match_regex;
// cratify: `rule_pattern` is rerail-rr's (the rule-file language's, decision 0495)
pub mod rule_pattern;
pub mod raw_text;
pub mod segments;
#[cfg(any(test, feature = "test-hooks"))]
pub mod test_script;
// cratify: `tests` is rerail-lang-php's (it mounts it)

#[cfg(any(test, feature = "test-hooks"))]
pub use test_script::TestScript;

use crate::code_rule_dsl::language::GrammarDialect;

use host::HostNode;
use segments::SegmentMap;
use std::ops::Range;

/// How a program's text was read: tree-sitter's tree, or a lean parser's tape of it, which is the
/// tape that tree encodes to (decision 0198). Either spans the program's text.
#[allow(
    clippy::large_enum_variant,
    reason = "one a program read, moved once into its document: a box would be an allocation more for each"
)]
pub enum Parse {
    Tree(tree_sitter::Tree),
    /// The whole text's tape: the placeholders the program's segments omit are still in it
    /// (`DerivedImage::encode` leaves them out, as it does a tree's).
    Tape(crate::code_rule_dsl::tape::TapeImage),
}

impl Parse {
    /// A program's text read in `dialect`: by the dialect's own parser where the engine registered one
    /// (`tape::tree_parser`: a lean parser, rerail-lean-tapes') and it reads the text, else by tree-sitter (one
    /// behavior: tree-sitter's tape, or a refusal). `None` where tree-sitter has no grammar for the dialect.
    pub fn read(dialect: GrammarDialect, text: &str, budget: &dyn Fn() -> bool) -> Option<Self> {
        use crate::code_rule_dsl::tape::{Grammar, tree_parser};
        // A literal grammar's dialect (decision 0495 §2): tree-sitter has none, and its registered parser reads it.
        if dialect == GrammarDialect::Regex {
            let grammar = Grammar::embedded(dialect).ok()?;
            return crate::code_rule_dsl::tape::TapeImage::parse(&grammar, text, &budget)?.ok().map(|image| Self::Tape(image.primary));
        }
        let lean = tree_parser(dialect).and_then(|parser| Grammar::embedded(dialect).ok().and_then(|grammar| parser.program(&grammar, text, budget)));
        if let Some(tape) = lean.and_then(Result::ok) {
            return Some(Self::Tape(tape));
        }
        Self::tree_sitter(dialect, text).map(Self::Tree)
    }

    /// tree-sitter's tree of a program's text in `dialect`.
    pub fn tree_sitter(dialect: GrammarDialect, text: &str) -> Option<tree_sitter::Tree> {
        use ast_grep_language::LanguageExt;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&dialect.get_ts_language()).ok()?;
        parser.parse(text, None)
    }

    /// Whether a lean parser read the text.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn is_lean(&self) -> bool {
        matches!(self, Self::Tape(_))
    }
}

/// A program an embedder built from a host file.
pub struct Program {
    /// The dialect its text is parsed with and read as.
    pub dialect: GrammarDialect,
    /// Its text, read.
    pub parse: Parse,
    pub text: String,
    /// Where its text stands in the host source (indexed: `SegmentMap::index`), and its
    /// placeholders the tape leaves out.
    pub segments: SegmentMap,
    /// The host bytes it cannot be read through, sorted: the host's syntax errors it reads.
    pub host_errors: Vec<Range<usize>>,
    /// The host bytes of the node it stands at, where they are not its text's (decision 0470: a Python string's
    /// program stands at the string, around its content); `None`: the innermost host node over its text.
    pub at: Option<Range<usize>>,
}

/// Builds a host file's embedded program: one per registry `via`.
/// The crate that owns an embedding registers its embedder for its `Via` as the engine loads (`install_embedders`).
/// A program it built is scoped as its dialect's family reads any program, unless the embedding gives the
/// program's statements a meaning of its own, which its crate registers too (`scope::install_program_scopes`: a
/// Blade view's markers define the variables its directives do).
pub trait Embedder: Send + Sync {
    /// The program the host whose tape's root is `root`, over `source`, embeds; `None` where it
    /// embeds none. The tape is read in place (`host`), whichever parser wrote it.
    fn build(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Option<Program>;

    /// Every program the host embeds through this `via`: a program per place for an embedding that has several (an
    /// HTML file's scripts and styles, decision 0431 §4), else `build`'s one.
    fn build_all(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Vec<Program> {
        self.build(root, source, budget).into_iter().collect()
    }

    /// Whether each program stands at a place of its own (`build_all`'s several: the innermost host node over its
    /// bytes), else the file's whole (its root, as a Vue component's view or a Blade file's compiled view is).
    fn per_place(&self) -> bool {
        false
    }

    /// Whether it reads the host through the file's scope table (decision 0470): its programs are then
    /// `build_in_doc`'s, from the batch's document over the host's tape, and `build_all`, which has no document, builds
    /// none.
    fn reads_scope(&self) -> bool {
        false
    }

    /// Whether its programs' syntax verdict is their tape's, not their language's own check of a file: they are no
    /// files of their language (a rule's pattern, whose metavariables CPython refuses; decision 0495 §3).
    fn tape_verdict(&self) -> bool {
        false
    }
    /// The programs of the host `doc` (`reads_scope`).
    fn build_in_doc(&self, _doc: &crate::code_rule_dsl::tape::TapeDoc, _budget: &dyn Fn() -> bool) -> Vec<Program> {
        Vec::new()
    }
}

static EMBEDDERS: std::sync::OnceLock<
    Vec<(crate::code_rule_dsl::language::Via, &'static dyn Embedder)>,
> = std::sync::OnceLock::new();

/// The engine's embedders, by `Via`; the first call holds.
pub fn install_embedders(
    embedders: Vec<(crate::code_rule_dsl::language::Via, &'static dyn Embedder)>,
) {
    let _ = EMBEDDERS.set(embedders);
}

/// `via`'s embedder. Every `Via` the build has is registered as the engine loads. The logic engine's guests build
/// the programs through it too (decision 0222).
pub fn embedder(via: crate::code_rule_dsl::language::Via) -> &'static dyn Embedder {
    EMBEDDERS
        .get()
        .and_then(|embedders| embedders.iter().find(|(known, _)| *known == via))
        .map(|(_, embedder)| *embedder)
        .unwrap_or_else(|| panic!("no embedder registered for {via:?}"))
}
