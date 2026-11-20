//! Named-only tapes (RER-131): which anonymous tokens the tape stores.
//!
//! A tree-sitter node's visible children follow its rule in `grammar.json`. For each visible kind
//! this module builds the rule's automaton over the children (named children labelled by kind and
//! field, anonymous tokens by kind and field; hidden rules inlined) and determinizes it. Between
//! two named children, and after the last, the tokens are a *gap*.
//!
//! A gap's tokens are **implied** where they are the only shortest token word from the gap's
//! state to its next named child (or to the end): the kind and the named children say what they
//! are. The tape's writer leaves implied tokens out as it walks tree-sitter's tree
//! ([`TokenGrammar::stored_children`]); every other token is stored, a leaf under its parent, as
//! tree-sitter has it. So `f(a, b)` stores none of its punctuation, `f(a, b,)` stores its trailing
//! `,` and `)`, and `a + b` stores its `+` (as short as `-`).
//!
//! Two classes of token, by text alone ([`is_delimiter`]):
//! - a **delimiter** (a bracket, a separator, a terminator, a quote) is left out wherever its gap
//!   is implied: where it is stored, the shape alone does not say it is there (`f(a,)`);
//! - every other token (a keyword, an operator, `:` `.` `::` `..` `=>`) is left out only where its
//!   kind implies it in every gap it can stand in ([`kept`]): `return` in Python's
//!   `return_statement`, `=` in a keyword argument. Where some gap holds it beside a choice
//!   (`async def`, `export default function f`), it is stored in every gap. So such a token is
//!   either always stored under its kind or never, and a rule that reads it reads it exactly; one
//!   its kind always implies is the kind's question (`kind(n, "return_statement")`).
//!
//! No reader rebuilds an implied token as a node. A rule that asks about one asks the grammar
//! instead: [`TokenGrammar::gaps`] walks a parent's stored children through its automaton and says
//! which word each gap holds, from the kinds and fields alone (the gap test, t31). A node
//! the automaton cannot walk (an ERROR, a contextual alias, a grammar the tree disagrees with)
//! keeps all of its tokens. Nothing here is chosen by hand: a new grammar version is a new
//! automaton, never a reclassification.

use crate::code_rule_dsl::language::GrammarDialect;
use rustc_hash::FxHashMap;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use tree_sitter::{Language, Node as TsNode, TreeCursor};

use super::token_automata::{self, Automaton, Label, Sym};
pub(in crate::code_rule_dsl) use super::token_automata::is_delimiter;

/// Temporary buffers for one tree walk. Ancestor masks stay on the encoder's
/// stack; a completed mask can serve the next parent. Gaps never outlive a call.
#[derive(Default)]
pub(super) struct FilterScratch {
    masks: Vec<Vec<bool>>,
    gap: Vec<(usize, Label)>,
    missing: Vec<usize>,
}

impl FilterScratch {
    pub(super) fn recycle(&mut self, mut mask: Vec<bool>) {
        mask.clear();
        self.masks.push(mask);
    }
}

/// Every visible kind's automaton for one grammar.
pub struct TokenGrammar {
    names: FxHashMap<String, Sym>,
    texts: Vec<String>,
    kinds: FxHashMap<Sym, Kind>,
    /// Per tree-sitter field id: its symbol (0 where the grammar has none).
    field_sym: Vec<Sym>,
    /// Symbols and filtering need by tree-sitter kind id, computed once per grammar.
    kind_sym: Vec<Option<Sym>>,
    pub(in crate::code_rule_dsl) unsupported: BTreeSet<String>,
    /// The grammar's extras (comments and the like): in no automaton, anywhere among children.
    extras: BTreeSet<String>,
    /// Every token text and field name, sorted, leaked once per dialect ([`Self::intern`]).
    interned: Vec<&'static str>,
}

/// A kind's automaton, read from the bytes the build wrote when a walk first needs it (`TokenGrammar::automaton`):
/// a grammar holds hundreds of kinds, and reading them all at once was most of what compiling a rule with a pattern
/// cost a process (a pattern is a tape, `tape::pattern_doc`), where a few are walked.
struct Kind {
    /// Where the automaton lies in the build's bytes (`token_automata::index`); empty for one built from a
    /// `grammar.json` here, which is held read.
    at: &'static [u8],
    /// The automaton, and whether it leaves any token out (else the kind's tapes store every child).
    read: OnceLock<Option<(Automaton, bool)>>,
}

impl Kind {
    fn held(automaton: Automaton) -> Self {
        let filters = automaton.dropped().next().is_some();
        Kind { at: &[], read: OnceLock::from(Some((automaton, filters))) }
    }

    fn read(&self) -> Option<&(Automaton, bool)> {
        self.read
            .get_or_init(|| {
                let automaton = token_automata::decode_automaton(self.at)?;
                let filters = automaton.dropped().next().is_some();
                Some((automaton, filters))
            })
            .as_ref()
    }
}

/// A parent's gaps ([`TokenGrammar::gaps`]): per stored child, the implied word just before it
/// (empty where the gap is empty, stored, or before a token or an extra), and the word after the
/// last child. Each token is its text and its field (`""` for none).
#[derive(Debug, Default)]
pub struct Gaps<'g> {
    pub before: Vec<Vec<(&'g str, &'g str)>>,
    pub end: Vec<(&'g str, &'g str)>,
}


impl TokenGrammar {
    /// Every visible kind's automaton from `json` (a `grammar.json`), its names resolved in
    /// `language`.
    pub fn from_json(json: &str, language: &Language) -> Result<Self, String> {
        token_automata::build(json).map(|built| {
            let token_automata::Built { texts, kinds, unsupported, extras } = built;
            Self::from_kinds(texts, kinds.into_iter().map(|(sym, automaton)| (sym, Kind::held(automaton))).collect(), unsupported, extras, language)
        })
    }

    /// The automata the build wrote (`token_automata::index`), each read when first walked, its names resolved in
    /// `language`.
    fn from_index(index: token_automata::Index<'static>, language: &Language) -> Self {
        let token_automata::Index { texts, kinds, unsupported, extras } = index;
        let kinds = kinds.into_iter().map(|(sym, at)| (sym, Kind { at, read: OnceLock::new() })).collect();
        Self::from_kinds(texts, kinds, unsupported, extras, language)
    }

    fn from_kinds(texts: Vec<String>, kinds: FxHashMap<Sym, Kind>, unsupported: BTreeSet<String>, extras: BTreeSet<String>, language: &Language) -> Self {
        let names: FxHashMap<String, Sym> = texts.iter().enumerate().map(|(sym, text)| (text.clone(), sym as Sym)).collect();
        let field_sym = (0..=language.field_count() as u16)
            .map(|id| language.field_name_for_id(id).and_then(|name| names.get(name).copied()).unwrap_or(0))
            .collect();
        let kind_sym = (0..language.node_kind_count() as u16)
            .map(|id| language.node_kind_for_id(id).and_then(|name| names.get(name).copied())).collect();
        let mut interned: Vec<&'static str> = texts.iter().map(|text| &*Box::leak(text.clone().into_boxed_str())).collect();
        interned.push("");
        interned.sort_unstable();
        interned.dedup();
        Self { names, texts, kinds, field_sym, kind_sym, unsupported, extras, interned }
    }

    pub fn kind_count(&self) -> usize {
        self.kinds.len()
    }

    /// The automaton of the kind `sym`, read on first use (`Kind`).
    fn automaton(&self, sym: Sym) -> Option<&Automaton> {
        self.kinds.get(&sym)?.read().map(|(automaton, _)| automaton)
    }

    /// Every kind's automaton, each read now if it was not yet.
    fn automata(&self) -> impl Iterator<Item = (&Sym, &Automaton)> + '_ {
        self.kinds.iter().filter_map(|(sym, kind)| kind.read().map(|(automaton, _)| (sym, automaton)))
    }

    #[cfg(any(test, feature = "test-hooks"))]
    fn text(&self, sym: Sym) -> &str {
        &self.texts[sym as usize]
    }

    /// Which of `node`'s children the tape stores, in order: `false` for an implied token (see the
    /// module). `None` where it stores them all: a node with no automaton, or one whose children
    /// the automaton cannot walk. A MISSING child walks the automaton as the child it stands for
    /// and is always stored: it is a syntax error.
    pub fn stored_children(&self, node: TsNode<'_>) -> Option<Vec<bool>> {
        self.stored_children_in(node, &mut node.walk(), &mut FilterScratch::default())
    }

    pub(super) fn stored_children_in<'tree>(&self, node: TsNode<'tree>, cursor: &mut TreeCursor<'tree>, scratch: &mut FilterScratch) -> Option<Vec<bool>> {
        if node.is_error() || !node.is_named() || node.child_count() == 0 {
            return None;
        }
        let sym = self.kind_sym.get(node.kind_id() as usize).copied().flatten()?;
        let automaton = match self.kinds.get(&sym)?.read()? {
            (automaton, true) => automaton,
            (_, false) => return None,
        };
        cursor.reset(node);
        if !cursor.goto_first_child() {
            return None;
        }
        let mut stored = scratch.masks.pop().unwrap_or_default();
        stored.reserve(node.child_count() as usize);
        scratch.gap.clear();
        scratch.missing.clear();
        let walked = (|| {
            let mut state = 0u32;
            loop {
                let child = cursor.node();
                let position = stored.len();
                stored.push(true);
                if !child.is_extra() {
                    if child.is_error() {
                        return None;
                    }
                    if child.is_missing() {
                        scratch.missing.push(position);
                    }
                    let name = self.kind_sym.get(child.kind_id() as usize).copied().flatten()?;
                    let field = cursor.field_id().map_or(0, |id| self.field_sym.get(usize::from(id.get())).copied().unwrap_or(0));
                    if child.is_named() {
                        state = gap_end(automaton, state, &scratch.gap, Some(Label { token: false, name, field }), &mut stored)?;
                        scratch.gap.clear();
                    } else {
                        if child.child_count() > 0 {
                            return None;
                        }
                        scratch.gap.push((position, Label { token: true, name, field }));
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            gap_end(automaton, state, &scratch.gap, None, &mut stored)?;
            for &position in &scratch.missing {
                stored[position] = true;
            }
            Some(())
        })();
        if walked.is_some() {
            Some(stored)
        } else {
            scratch.recycle(stored);
            None
        }
    }
}

impl TokenGrammar {
    /// Whether `kind` is one of the grammar's extras (a comment and the like).
    pub fn is_extra(&self, kind: &str) -> bool {
        self.extras.contains(kind)
    }

    /// The field names `kind`'s automaton labels its children with.
    pub(in crate::code_rule_dsl) fn fields_of(&self, kind: &str) -> impl Iterator<Item = &str> {
        let fields = self.names.get(kind).and_then(|&sym| self.automaton(sym)).map_or(&[][..], |automaton| &automaton.fields[..]);
        fields.iter().map(|&field| self.texts[field as usize].as_str())
    }

    /// The gaps of a `kind` node whose stored children are `children` (named, kind, field; `""`
    /// for no field), in order, as the tape holds them: walked through the kind's automaton, a gap
    /// with no stored token holds the implied word to the next named child (or to the end).
    /// `None` where the kind has no automaton or the automaton refuses the children.
    pub(in crate::code_rule_dsl) fn gaps<'c>(&self, kind: &str, children: impl IntoIterator<Item = (bool, &'c str, &'c str)>) -> Option<Gaps<'_>> {
        let automaton = self.automaton(*self.names.get(kind)?)?;
        let word = |labels: &[Label]| labels.iter().map(|label| (self.texts[label.name as usize].as_str(), self.texts[label.field as usize].as_str())).collect::<Vec<_>>();
        let walk = |mut state: u32, labels: &[Label]| -> Option<u32> {
            for label in labels {
                state = *automaton.moves[state as usize].get(label)?;
            }
            Some(state)
        };
        let kept_of = |implied: &[Label]| implied.iter().filter(|label| automaton.kept.contains(label)).copied().collect::<Vec<_>>();
        // An implied word whose kept tokens are stored: the tokens the tape left out.
        let dropped = |implied: &[Label]| implied.iter().filter(|label| !automaton.kept.contains(label)).copied().collect::<Vec<_>>();
        let mut gaps = Gaps::default();
        let (mut state, mut pending) = (0u32, Vec::<Label>::new());
        for (named, child, field) in children {
            gaps.before.push(Vec::new());
            if self.extras.contains(child) {
                continue;
            }
            let label = Label { token: !named, name: *self.names.get(child)?, field: if field.is_empty() { 0 } else { *self.names.get(field)? } };
            if !named {
                pending.push(label);
                continue;
            }
            if pending.is_empty() {
                // An empty gap: the implied word, which is empty where the child follows directly.
                match automaton.implied.to.get(&(state, label)) {
                    Some(implied) => {
                        *gaps.before.last_mut()? = word(implied);
                        state = walk(state, implied)?;
                    }
                    None if automaton.moves[state as usize].contains_key(&label) => {}
                    None => return None,
                }
            } else {
                // A stored gap, which reaches the child; or an implied one whose kept tokens
                // ([`kept`]) are stored and the rest left out (`for (k in o)`: `for` is kept, `(`
                // is not), where the stored tokens alone walk only part of the way.
                match walk(state, &pending).filter(|to| automaton.moves[*to as usize].contains_key(&label)) {
                    Some(to) => state = to,
                    None => {
                        let implied = automaton.implied.to.get(&(state, label)).filter(|implied| kept_of(implied) == pending)?;
                        *gaps.before.last_mut()? = word(&dropped(implied));
                        state = walk(state, implied)?;
                    }
                }
                pending.clear();
            }
            state = *automaton.moves[state as usize].get(&label)?;
        }
        let end = automaton.implied.end[state as usize].as_deref();
        let stored_end = walk(state, &pending).is_some_and(|to| automaton.accepting[to as usize]);
        if pending.is_empty() || !stored_end && end.is_some_and(|implied| kept_of(implied) == pending) {
            gaps.end = end.map(|implied| word(&dropped(implied))).unwrap_or_default();
        }
        Some(gaps)
    }
}

/// Closes a gap from `state` before `next` (a named child; `None` at the end): marks its tokens
/// not stored where they are the implied word. The state after `next`; `None` where the automaton
/// refuses the gap or `next`.
fn gap_end(automaton: &Automaton, state: u32, gap: &[(usize, Label)], next: Option<Label>, stored: &mut [bool]) -> Option<u32> {
    let mut at = state;
    for (_, label) in gap {
        at = *automaton.moves[at as usize].get(label)?;
    }
    let after = match next {
        Some(label) => *automaton.moves[at as usize].get(&label)?,
        None if automaton.accepting[at as usize] => at,
        None => return None,
    };
    // No token can be omitted from an empty gap. The transition above still
    // validates the next child (or acceptance); no implied-word lookup is needed.
    if gap.is_empty() {
        return Some(after);
    }
    let implied = match next {
        Some(label) => automaton.implied.to.get(&(state, label)),
        None => automaton.implied.end[state as usize].as_ref(),
    };
    if implied.is_some_and(|word| word.len() == gap.len() && word.iter().zip(gap).all(|(a, (_, b))| a == b)) {
        for (position, label) in gap {
            if !automaton.kept.contains(label) {
                stored[*position] = false;
            }
        }
    }
    Some(after)
}

/// What the automata say about where tokens are implied, for the check of rules that test
/// tokens (`compile::token_tests`).
impl TokenGrammar {
    /// The automata of the kinds whose tapes leave tokens out (the filtered kinds), as text, for a
    /// lean parser's tables (`rerail-sql-lean`'s `gen_kinds.py`): a parser that writes the tape
    /// itself decides each gap as `stored_children` does. Lines, tab-separated; a label is
    /// `t` or `n` (token or named), its name and its field (`""`: none), and a word is labels
    /// joined by `\x1f`, a label's parts by `\x1e`:
    ///
    /// - `K kind`: the automaton that follows is this kind's;
    /// - `S state accepting`; `M state label to`: a move;
    /// - `I state label word`: the implied word from `state` to the named child `label`;
    /// - `E state word`: the implied word from `state` to the end;
    /// - `P label`: a kept token (`kept`).
    pub fn automata_text(&self) -> String {
        let label = |l: &Label| format!("{}\x1e{}\x1e{}", if l.token { "t" } else { "n" }, self.texts[l.name as usize], self.texts[l.field as usize]);
        let word = |w: &[Label]| w.iter().map(label).collect::<Vec<_>>().join("\x1f");
        let mut kinds: Vec<_> = self.automata().filter(|(_, automaton)| automaton.dropped().next().is_some()).collect();
        kinds.sort_by_key(|(sym, _)| self.texts[**sym as usize].clone());
        let mut out = String::new();
        for (sym, a) in kinds {
            out.push_str(&format!("K\t{}\n", self.texts[*sym as usize]));
            for (state, moves) in a.moves.iter().enumerate() {
                out.push_str(&format!("S\t{state}\t{}\n", u8::from(a.accepting[state])));
                let mut moves: Vec<_> = moves.iter().map(|(l, to)| (label(l), to)).collect();
                moves.sort();
                for (l, to) in moves {
                    out.push_str(&format!("M\t{state}\t{l}\t{to}\n"));
                }
            }
            let mut to: Vec<_> = a.implied.to.iter().map(|((state, l), w)| (*state, label(l), word(w))).collect();
            to.sort();
            for (state, l, w) in to {
                out.push_str(&format!("I\t{state}\t{l}\t{w}\n"));
            }
            for (state, w) in a.implied.end.iter().enumerate() {
                if let Some(w) = w {
                    out.push_str(&format!("E\t{state}\t{}\n", word(w)));
                }
            }
            let mut kept: Vec<_> = a.kept.iter().map(label).collect();
            kept.sort();
            for l in kept {
                out.push_str(&format!("P\t{l}\n"));
            }
        }
        out
    }

    /// What the automata say, by name ([`token_automata::Census`]): per kind, the (token, field) pairs a tape of it
    /// may leave out (those some implied word of its automaton holds, less the kept ones; `""` is no field), each
    /// kind's parents, and every token text some automaton moves on.
    pub(in crate::code_rule_dsl) fn census(&self) -> token_automata::Census {
        token_automata::census(&self.texts, self.automata(), &self.extras)
    }

    /// Per kind: the (token, field) pairs a tape of it may leave out ([`Self::census`]).
    pub fn implied_tokens(&self) -> BTreeMap<String, BTreeSet<(String, String)>> {
        self.census().implied
    }

    /// Per kind: the kinds whose automata have it as a named child.
    pub fn parents(&self) -> BTreeMap<String, BTreeSet<String>> {
        self.census().parents
    }

    /// Every token text some automaton moves on.
    pub fn token_texts(&self) -> BTreeSet<String> {
        self.census().tokens
    }
}

// `built(lang)`: the automata of a dialect's grammar, built with the build (`token_automata::GRAMMARS`), and
// `census(lang)`, what they say by name; `None` for a dialect whose tapes keep all their tokens, or a grammar that has
// none.
include!(concat!(env!("OUT_DIR"), "/token_automata.rs"));

/// The `grammar.json` a dialect's parser was generated from, built into the engine (the vendored
/// grammars, and byte-identical copies of the Rust and PHP crates' under `grammars/token-grammars`);
/// `None` for a dialect whose tapes keep all their tokens.
pub fn grammar_json(lang: GrammarDialect) -> Option<&'static str> {
    // Relative to this file (`native/grammars`), so that no checkout's directory is part of the crate.
    macro_rules! grammar {
        ($path:literal) => {
            include_str!(concat!("../../../grammars/", $path, "/grammar.json"))
        };
    }
    Some(match lang {
        GrammarDialect::Python => grammar!("tree-sitter-python/src"),
        GrammarDialect::JavaScript => grammar!("tree-sitter-javascript/src"),
        GrammarDialect::TypeScript => grammar!("tree-sitter-typescript/typescript/src"),
        GrammarDialect::Tsx => grammar!("tree-sitter-typescript/tsx/src"),
        GrammarDialect::Sql => grammar!("tree-sitter-sql/src"),
        GrammarDialect::Vue => grammar!("tree-sitter-vue/src"),
        GrammarDialect::Markdown => grammar!("tree-sitter-markdown/markdown/src"),
        GrammarDialect::MarkdownInline => grammar!("tree-sitter-markdown/markdown-inline/src"),
        GrammarDialect::Rust => grammar!("token-grammars/rust"),
        GrammarDialect::Php => grammar!("token-grammars/php"),
        GrammarDialect::Json => grammar!("token-grammars/json"),
        GrammarDialect::Java => grammar!("token-grammars/java"),
        GrammarDialect::CSharp => grammar!("token-grammars/csharp"),
        GrammarDialect::Html => grammar!("token-grammars/html"),
        GrammarDialect::Css => grammar!("token-grammars/css"),
        GrammarDialect::Kotlin => grammar!("token-grammars/kotlin"),
        GrammarDialect::Swift => grammar!("token-grammars/swift"),
        GrammarDialect::Scala => grammar!("token-grammars/scala"),
        GrammarDialect::Go => grammar!("token-grammars/go"),
        GrammarDialect::C => grammar!("token-grammars/c"),
        GrammarDialect::Cpp => grammar!("token-grammars/cpp"),
        _ => return None,
    })
}

impl TokenGrammar {
    /// A dialect's automata, for its tape grammar's tree-sitter language; `None` where it has none.
    pub fn of_dialect(lang: GrammarDialect) -> Option<Arc<Self>> {
        Self::by_dialect(lang)?.clone()
    }

    fn by_dialect(lang: GrammarDialect) -> Option<&'static Option<Arc<Self>>> {
        // Built once per dialect, as `of` builds it; a lookup after that, for the token queries.
        static BY_DIALECT: [OnceLock<Option<Arc<TokenGrammar>>>; 256] = [const { OnceLock::new() }; 256];
        Some(BY_DIALECT.get(lang as usize)?.get_or_init(|| {
            let grammar = super::Grammar::new(lang).ok()?;
            Self::of(lang, grammar.kinds.grammar()?)
        }))
    }

    /// The dialect's grammar as `of_dialect` keeps it, for the life of the process (it is never dropped).
    pub fn of_dialect_static(lang: GrammarDialect) -> Option<&'static Self> {
        Self::by_dialect(lang)?.as_deref()
    }

    /// `text` (a token text or field name of this grammar) for the life of the process: the
    /// grammar is built once per dialect and never dropped.
    pub(in crate::code_rule_dsl) fn intern(&self, text: &str) -> &'static str {
        match self.interned.binary_search_by(|known| (*known).cmp(text)) {
            Ok(at) => self.interned[at],
            Err(_) => "",
        }
    }

    /// A dialect's automata, built once per process from its `grammar.json`; `None` where there
    /// is none (a dialect with its own parser, or no grammar on disk).
    pub fn of(lang: GrammarDialect, language: &Language) -> Option<Arc<Self>> {
        static KEPT: LazyLock<Mutex<FxHashMap<GrammarDialect, Option<Arc<TokenGrammar>>>>> = LazyLock::new(Default::default);
        let mut kept = KEPT.lock().ok()?;
        kept.entry(lang)
            .or_insert_with(|| built(lang).and_then(token_automata::index).map(|index| Arc::new(Self::from_index(index, language))))
            .clone()
    }
}

/// `parent`'s stored children and its gaps; `None` where its kind has no automaton.
///
/// A gap test asks this of a node's parent for every node it tests (each of a module's statements walks all of
/// them), so nothing is copied out: a tape's kinds are its grammar's names, and the fields are the automaton's.
pub fn stored_gaps<'t, 'g, D: ast_grep_core::Doc>(grammar: &'g TokenGrammar, parent: &ast_grep_core::Node<'t, D>) -> Option<(Vec<ast_grep_core::Node<'t, D>>, Gaps<'g>)> {
    let kind = parent.kind();
    let children: Vec<ast_grep_core::Node<'t, D>> = parent.children().collect();
    let mut field_at: Vec<&str> = vec![""; children.len()];
    for field in grammar.fields_of(&kind) {
        let mut from = 0;
        for child in parent.field_children(field) {
            // A field's children come in the children's order.
            let id = child.node_id();
            if let Some(k) = children[from..].iter().position(|c| c.node_id() == id) {
                field_at[from + k] = field;
                from += k + 1;
            }
        }
    }
    let kinds: Vec<(bool, std::borrow::Cow<'_, str>)> = children.iter().map(|child| (child.is_named(), child.kind())).collect();
    let gaps = grammar.gaps(&kind, kinds.iter().zip(&field_at).map(|((named, kind), field)| (*named, kind.as_ref(), *field)))?;
    Some((children, gaps))
}
