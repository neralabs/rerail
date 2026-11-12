//! Literal sub-grammars (a Rerail extension): a string literal's decoded text parsed as a tree of
//! its own, which `parse` matches with a rule of the grammar's kinds. The tree is a separate
//! document with its own environment; its nodes map back to the source through the literal's
//! offset map, so captures exported to the host anchor where the text came from.

pub(crate) mod annotation;
pub(crate) mod brace;
pub(crate) mod eslint_directive;
pub(crate) mod identifier_words;
pub(crate) mod offsets;
pub(crate) mod percent;
pub(crate) mod php_string;
pub(crate) mod phpdoc;
pub(crate) mod printf;
pub(crate) mod python_fstring;
pub(crate) mod python_string;
pub(crate) mod rust_format;
pub(crate) mod scanf;
pub(crate) mod semantic;

use crate::code_rule_dsl::embed::segments::FileRanges;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::value::Value;
use ast_grep_core::Position;
use ast_grep_core::language::Language;
use ast_grep_core::matcher::{Pattern, PatternBuilder, PatternError};
use ast_grep_core::source::{Content, Doc, Edit, SgNode};

use offsets::Decoded;
use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A sub-grammar `parse` can apply, as the crate that reads its host's literals declares it: the engine registers
/// each as it loads (`install`), in the order error messages list them.
pub(crate) trait LiteralGrammar: Send + Sync + 'static {
    /// Its `parse.grammar` name.
    fn name(&self) -> &'static str;
    /// The kinds a tree of it has, in id order from 1.
    fn kinds(&self) -> &'static [&'static str];
    /// The language of the host it parses, whose values its trees hold; that language's family parses the
    /// host's literals (`SyntaxRules::parse_literal`).
    fn host_language(&self) -> GrammarDialect;
    /// What it parses, for messages.
    fn host(&self) -> &'static str;
    /// The host node kinds it parses.
    fn host_kinds(&self) -> &'static [&'static str];
    /// Whether it parses its host's comments, not its strings.
    fn reads_comments(&self) -> bool {
        false
    }
    /// Whether its trees are its host language's own syntax, parsed by the scope table (Python's string
    /// annotations): no `host`, and a rule over them may use the host's keys besides.
    fn of_host_syntax(&self) -> bool {
        false
    }
    /// Whether an item names a global `resolves` reads (ESLint's directives).
    fn item_resolves(&self) -> bool {
        false
    }
    /// Whether it parses an interpolated literal's parts (`interpolation`: Python's f-strings).
    fn interpolates(&self) -> bool {
        false
    }
    /// Whether a tree depends on the literal alone, so a syntax error elsewhere cannot change it.
    fn reads_only_the_node(&self) -> bool {
        true
    }
    /// A kind's id where its kinds are not `kinds` (a host syntax's).
    fn own_kind_id(&self, kind: &str) -> Option<u16> {
        let _ = kind;
        None
    }
    /// A kind id's name where its kinds are not `kinds`.
    fn own_kind_name(&self, id: u16) -> Option<&'static str> {
        let _ = id;
        None
    }
    /// Parses decoded text into a tree root, its nodes `grammar`'s (this grammar's handle).
    fn parse(&self, grammar: Grammar, text: &str) -> PNode;
}

/// A sub-grammar `parse` can apply (`LiteralGrammar`); equal by name.
#[derive(Clone, Copy)]
pub(crate) struct Grammar(&'static dyn LiteralGrammar);

static GRAMMARS: std::sync::OnceLock<Vec<Grammar>> = std::sync::OnceLock::new();

/// The engine's grammars, in the order error messages list them; the first call holds.
pub(crate) fn install(grammars: Vec<Grammar>) {
    let _ = GRAMMARS.set(grammars);
}

impl PartialEq for Grammar {
    fn eq(&self, other: &Self) -> bool {
        self.name() == other.name()
    }
}

impl Eq for Grammar {}

impl std::hash::Hash for Grammar {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name().hash(state);
    }
}

impl std::fmt::Debug for Grammar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl Grammar {
    /// `grammar`'s handle.
    pub(crate) const fn new(grammar: &'static dyn LiteralGrammar) -> Self {
        Self(grammar)
    }

    /// Every grammar the engine registered, in the order error messages list them.
    pub(crate) fn all() -> &'static [Grammar] {
        GRAMMARS.get().map_or(&[], Vec::as_slice)
    }

    /// The language of this grammar's trees.
    pub(crate) fn lang(self) -> SubLang {
        SubLang(self)
    }

    /// The grammar a `parse.grammar` name selects.
    pub(crate) fn named(name: &str) -> Option<Self> {
        Self::all().iter().copied().find(|grammar| grammar.name() == name)
    }

    /// The registered grammar `name` names, which a part of the engine reads by name (`formatArgs`' styles).
    pub(crate) fn expect(name: &str) -> Self {
        Self::named(name).unwrap_or_else(|| panic!("no grammar `{name}` registered"))
    }

    pub(crate) fn name(self) -> &'static str {
        self.0.name()
    }

    /// The kinds a tree of this grammar has, in id order from 1.
    pub(crate) fn kinds(self) -> &'static [&'static str] {
        self.0.kinds()
    }

    /// The language of the host the grammar parses, whose values its trees hold.
    pub(crate) fn host_language(self) -> GrammarDialect {
        self.0.host_language()
    }

    /// What the grammar parses, for messages.
    pub(crate) fn host(self) -> &'static str {
        self.0.host()
    }

    /// The host node kinds the grammar parses.
    pub(crate) fn host_kinds(self) -> &'static [&'static str] {
        self.0.host_kinds()
    }

    pub(crate) fn reads_comments(self) -> bool {
        self.0.reads_comments()
    }

    pub(crate) fn of_host_syntax(self) -> bool {
        self.0.of_host_syntax()
    }

    pub(crate) fn item_resolves(self) -> bool {
        self.0.item_resolves()
    }

    pub(crate) fn interpolates(self) -> bool {
        self.0.interpolates()
    }

    /// Whether a tree depends on the literal alone, so a syntax error elsewhere cannot change it:
    /// whether a string is a type definition depends on the names the file binds.
    pub(crate) fn reads_only_the_node(self) -> bool {
        self.0.reads_only_the_node()
    }

    fn kind_id(self, kind: &str) -> u16 {
        if let Some(id) = self.0.own_kind_id(kind) {
            return id;
        }
        self.kinds()
            .iter()
            .position(|known| *known == kind)
            .map_or(0, |index| index as u16 + 1)
    }

    fn kind_name(self, id: u16) -> &'static str {
        if let Some(name) = self.0.own_kind_name(id) {
            return name;
        }
        self.kinds()
            .get(usize::from(id).wrapping_sub(1))
            .copied()
            .unwrap_or("")
    }

    /// Parses decoded text into a tree root.
    pub(crate) fn parse(self, text: &str) -> PNode {
        self.0.parse(self, text)
    }
}

/// Test-only: every decoded character is a `char` node under a `text` root (`testChars`, of Python strings).
#[cfg(test)]
pub(crate) struct Chars;

#[cfg(test)]
pub(crate) const CHARS: Grammar = Grammar::new(&Chars);

#[cfg(test)]
impl LiteralGrammar for Chars {
    fn name(&self) -> &'static str {
        "testChars"
    }
    fn kinds(&self) -> &'static [&'static str] {
        &["text", "char", "error"]
    }
    fn host_language(&self) -> GrammarDialect {
        GrammarDialect::Python
    }
    fn host(&self) -> &'static str {
        "Python literals"
    }
    fn host_kinds(&self) -> &'static [&'static str] {
        &["string", "concatenated_string"]
    }
    fn parse(&self, grammar: Grammar, text: &str) -> PNode {
        let mut root = PNode::new(grammar, "text", 0, text.len());
        for (at, c) in text.char_indices() {
            root.children.push(
                PNode::new(grammar, "char", at, at + c.len_utf8())
                    .with_value(Value::Str(c.to_string())),
            );
        }
        root
    }
}

/// A node as a grammar's parser builds it; `SubTree::new` flattens it.
#[derive(Clone, Debug)]
pub(crate) struct PNode {
    pub(crate) kind: u16,
    /// Byte range in the decoded text.
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) named: bool,
    pub(crate) error: bool,
    pub(crate) extra: bool,
    pub(crate) field: u16,
    pub(crate) value: Option<Value>,
    pub(crate) children: Vec<PNode>,
}

impl PNode {
    pub(crate) fn new(grammar: Grammar, kind: &str, start: usize, end: usize) -> Self {
        let kind = grammar.kind_id(kind);
        debug_assert!(kind != 0, "unknown kind");
        Self {
            kind,
            start,
            end,
            named: true,
            error: false,
            extra: false,
            field: 0,
            value: None,
            children: Vec::new(),
        }
    }

    pub(crate) fn with_value(mut self, value: Value) -> Self {
        self.value = Some(value);
        self
    }

    /// Nodes in this subtree.
    pub(crate) fn size(&self) -> usize {
        let mut size = 0;
        let mut stack = vec![self];
        while let Some(node) = stack.pop() {
            size += 1;
            stack.extend(&node.children);
        }
        size
    }
}

const NONE: u32 = u32::MAX;

#[derive(Clone, Debug)]
struct Rec {
    kind: u16,
    field: u16,
    parent: u32,
    first: u32,
    next: u32,
    prev: u32,
    children: u32,
    start: u32,
    end: u32,
    named: bool,
    error: bool,
    extra: bool,
}

/// How the tree's nodes map to source bytes.
pub(crate) enum HostMap {
    /// Through the literal's per-character spans.
    Spans(Decoded),
    /// Every node covers this range (Ruff's complex string annotations).
    Whole(Range<usize>),
    /// Decoded offset `o` is source byte `o + delta` (Ruff's simple string annotations).
    Exact(isize),
}

/// A parsed literal: an immutable tree over its decoded text.
pub(crate) struct SubTree {
    lang: SubLang,
    source: SubSource,
    nodes: Vec<Rec>,
    values: Vec<Option<Value>>,
    map: HostMap,
    /// Added to a node's index to make its `node_id`: unique among live trees and far from the
    /// ids of host documents, so memos keyed by node id never confuse the two.
    base: usize,
}

/// A parsed tree's bytes are the literal's decoded text: a node stands for the source bytes of its
/// characters (`HostMap`), in the document the literal was parsed from.
impl crate::code_rule_dsl::origin::Origin for SubTree {
    /// A literal's characters stand in source order, so a node stands for one source range.
    fn of_node(&self, id: usize, _: Range<usize>) -> Option<FileRanges> {
        let rec = self.nodes.get(self.index_of(id)?)?;
        Some(FileRanges::one(
            self.map_range(rec.start as usize..rec.end as usize),
        ))
    }

    fn of_range(&self, range: Range<usize>) -> Option<FileRanges> {
        Some(FileRanges::one(self.map_range(self.decoded(range)?)))
    }

    fn bounds(&self, id: usize, range: Range<usize>) -> Option<Range<usize>> {
        Some(self.of_node(id, range)?.primary())
    }

    /// Only an exact literal's bytes are copied: elsewhere decoding rewrote them.
    fn copied(&self, range: Range<usize>) -> Option<Range<usize>> {
        let range = self.decoded(range)?;
        matches!(self.map, HostMap::Exact(_)).then(|| self.map_range(range))
    }
}

/// Node ids of sub-trees have the top bit set; tapes use small indices, tree-sitter addresses.
/// Blade trees (`blade::forte::tree`) draw their ids from the same counter.
/// On wasm32 (the browser build) the top bit is bit 31, which leaves 7 bits of tree counter above a tree's 24
/// bits of nodes: enough for an explorer's few files, not for a corpus (wasm/README.md).
pub(crate) const ID_FLAG: usize = 1 << (usize::BITS - 1);
/// The tree counter's bits between `ID_FLAG` and a tree's 24 bits of nodes: 39 on 64-bit targets, 7 on wasm32.
pub(crate) const TREE_MASK: usize = (1 << (usize::BITS - 25)) - 1;
pub(crate) static NEXT_TREE: AtomicUsize = AtomicUsize::new(0);

impl SubTree {
    /// Flattens `root` over `text`. `offset` shifts every node range (see `SubSource`).
    pub(crate) fn new(
        grammar: Grammar,
        text: String,
        offset: usize,
        root: PNode,
        map: HostMap,
    ) -> Self {
        let mut nodes: Vec<Rec> = Vec::new();
        let mut values = Vec::new();
        // The last child linked so far, per node.
        let mut last: Vec<u32> = Vec::new();
        let mut stack: Vec<(PNode, u32)> = vec![(root, NONE)];
        while let Some((node, parent)) = stack.pop() {
            let index = nodes.len() as u32;
            let prev = if parent == NONE {
                NONE
            } else {
                let parent_rec = &mut nodes[parent as usize];
                parent_rec.children += 1;
                let prev = last[parent as usize];
                if prev == NONE {
                    parent_rec.first = index;
                } else {
                    nodes[prev as usize].next = index;
                }
                last[parent as usize] = index;
                prev
            };
            last.push(NONE);
            nodes.push(Rec {
                kind: node.kind,
                field: node.field,
                parent,
                first: NONE,
                next: NONE,
                prev,
                children: 0,
                start: node.start as u32,
                end: node.end as u32,
                named: node.named,
                error: node.error,
                extra: node.extra,
            });
            values.push(node.value);
            for child in node.children.into_iter().rev() {
                stack.push((child, index));
            }
        }
        let tree = NEXT_TREE.fetch_add(1, Ordering::Relaxed) & TREE_MASK;
        Self {
            lang: SubLang(grammar),
            source: SubSource { text, offset },
            nodes,
            values,
            map,
            base: ID_FLAG | (tree << 24),
        }
    }

    fn index_of(&self, node_id: usize) -> Option<usize> {
        (node_id & !((1 << 24) - 1) == self.base)
            .then_some(node_id & ((1 << 24) - 1))
            .filter(|index| *index < self.nodes.len())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    /// The grammar kind of the parsed root.
    pub(crate) fn root_kind(&self) -> u16 {
        self.nodes[0].kind
    }

    #[cfg_attr(not(test), allow(dead_code))]
    /// The grammar kind of the node with this id.
    pub(crate) fn node_kind(&self, node_id: usize) -> Option<u16> {
        self.nodes
            .get(self.index_of(node_id)?)
            .map(|node| node.kind)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    /// The source range of the node with this id.
    pub(crate) fn host_range(&self, node_id: usize) -> Option<Range<usize>> {
        let rec = self.nodes.get(self.index_of(node_id)?)?;
        Some(self.map_range(rec.start as usize..rec.end as usize))
    }

    /// A range of this tree's nodes (offset included) as decoded bytes.
    fn decoded(&self, range: Range<usize>) -> Option<Range<usize>> {
        let offset = self.source.offset;
        let start = range.start.checked_sub(offset)?;
        let end = range.end.checked_sub(offset)?;
        (end <= self.source.text.len()).then_some(start..end)
    }

    /// The source range of decoded bytes `range`.
    fn map_range(&self, range: Range<usize>) -> Range<usize> {
        match &self.map {
            HostMap::Spans(decoded) => decoded.host_range(range),
            HostMap::Whole(whole) => whole.clone(),
            HostMap::Exact(delta) => {
                let shift = |at: usize| (at as isize + delta).max(0) as usize;
                shift(range.start)..shift(range.end)
            }
        }
    }

    /// Nodes in the tree.
    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// The decoded text.
    pub(crate) fn text(&self) -> &str {
        &self.source.text
    }

    /// Where node ranges start: the tree's offset (see `SubSource`).
    pub(crate) fn offset(&self) -> usize {
        self.source.offset
    }

    /// Whether the tree is one error node: a literal its grammar refuses.
    pub(crate) fn is_error(&self) -> bool {
        self.nodes.first().is_some_and(|root| root.error)
    }

    /// The value the grammar gives the node with this id.
    pub(crate) fn value(&self, node_id: usize) -> Option<&Value> {
        self.values.get(self.index_of(node_id)?)?.as_ref()
    }

    /// The root's value.
    pub(crate) fn root_value(&self) -> Option<&Value> {
        self.values.first()?.as_ref()
    }

    /// The root's children in order, each its kind's name and its value.
    pub(crate) fn root_children(
        &self,
    ) -> impl Iterator<Item = (&'static str, Option<&Value>)> + '_ {
        let grammar = self.lang.0;
        let mut at = self.nodes.first().map_or(NONE, |root| root.first);
        std::iter::from_fn(move || {
            let i = (at != NONE).then_some(at as usize)?;
            at = self.nodes[i].next;
            Some((
                grammar.kind_name(self.nodes[i].kind),
                self.values[i].as_ref(),
            ))
        })
    }

    /// Every node in preorder (the tree's own order: a parent before its children, siblings in order), each with its
    /// parent's index (`None` at the root) and its range in `text()` (the offset left out).
    pub(crate) fn preorder(&self) -> impl Iterator<Item = PreNode> + '_ {
        self.nodes.iter().map(|rec| PreNode {
            kind: rec.kind,
            field: rec.field,
            parent: (rec.parent != NONE).then_some(rec.parent),
            start: rec.start,
            end: rec.end,
            named: rec.named,
            extra: rec.extra,
            error: rec.error,
        })
    }
}

/// One node of a `SubTree` (`SubTree::preorder`): its grammar kind and field ids (tree-sitter's, for an annotation:
/// ERROR is `annotation::ERROR_KIND`), its parent's index, its range in the tree's text and its flags.
#[derive(Clone, Copy, Debug)]
pub struct PreNode {
    pub(crate) kind: u16,
    pub(crate) field: u16,
    pub(crate) parent: Option<u32>,
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) named: bool,
    pub(crate) extra: bool,
    pub(crate) error: bool,
}

/// A sub-tree's text. Node ranges are offsets into it plus `offset`, which an annotation's tree
/// sets so its identifiers have ranges no host reference has (see `scope::python`).
#[derive(Clone)]
pub struct SubSource {
    text: String,
    offset: usize,
}

impl Content for SubSource {
    type Underlying = u8;
    fn get_range(&self, range: Range<usize>) -> &[u8] {
        let start = range.start.saturating_sub(self.offset).min(self.text.len());
        let end = range
            .end
            .saturating_sub(self.offset)
            .clamp(start, self.text.len());
        &self.text.as_bytes()[start..end]
    }
    fn decode_str(src: &str) -> Cow<'_, [u8]> {
        Cow::Borrowed(src.as_bytes())
    }
    fn encode_bytes(bytes: &[u8]) -> Cow<'_, str> {
        String::from_utf8_lossy(bytes)
    }
    fn get_char_column(&self, _column: usize, offset: usize) -> usize {
        let end = offset.saturating_sub(self.offset).min(self.text.len());
        let line = self.text[..end].rfind('\n').map_or(0, |at| at + 1);
        self.text[line..end].chars().count()
    }
}

/// The language of a sub-tree: its grammar's kinds; an annotation's are Python's.
#[derive(Clone, Copy)]
pub struct SubLang(Grammar);

impl Language for SubLang {
    fn kind_to_id(&self, kind: &str) -> u16 {
        self.0.kind_id(kind)
    }
    fn field_to_id(&self, field: &str) -> Option<u16> {
        // A host syntax's trees have its fields (Python's annotations).
        self.0
            .of_host_syntax()
            .then(|| self.0.host_language().field_to_id(field))
            .flatten()
    }
    fn build_pattern(&self, builder: &PatternBuilder) -> Result<Pattern, PatternError> {
        // Patterns are the host's syntax, which only a host-syntax grammar's trees hold (Python's annotations):
        // written as its trees are, with no token the grammar implies (RER-131).
        ast_grep_core::Language::build_pattern(&self.0.host_language(), builder)
    }
}

/// A sub-tree as an ast-grep document.
#[derive(Clone)]
pub(crate) struct SubDoc {
    pub(crate) tree: Arc<SubTree>,
}

impl SubDoc {}

impl Doc for SubDoc {
    type Source = SubSource;
    type Lang = SubLang;
    type Node<'r> = SubNode<'r>;
    fn get_lang(&self) -> &SubLang {
        &self.tree.lang
    }
    fn get_source(&self) -> &SubSource {
        &self.tree.source
    }
    fn do_edit(&mut self, _edit: &Edit<SubSource>) -> Result<(), String> {
        Err("parsed literals are immutable".into())
    }
    fn root_node(&self) -> SubNode<'_> {
        SubNode {
            tree: &self.tree,
            index: 0,
        }
    }
    fn get_node_text<'a>(&'a self, node: &SubNode<'a>) -> Cow<'a, str> {
        let rec = node.rec();
        Cow::Borrowed(&self.tree.source.text[rec.start as usize..rec.end as usize])
    }
}

#[derive(Clone, Copy)]
pub struct SubNode<'r> {
    tree: &'r SubTree,
    index: u32,
}

impl SubNode<'_> {
    fn rec(&self) -> &Rec {
        &self.tree.nodes[self.index as usize]
    }
    fn at(&self, index: u32) -> Option<Self> {
        (index != NONE).then_some(Self { index, ..*self })
    }
    fn position(&self, at: u32) -> Position {
        let text = &self.tree.source.text[..at as usize];
        let row = text.matches('\n').count();
        let column = text.len() - text.rfind('\n').map_or(0, |line| line + 1);
        Position::new(row, column, at as usize + self.tree.source.offset)
    }
}

struct Children<'r> {
    node: SubNode<'r>,
    next: u32,
    remaining: usize,
}

impl<'r> Iterator for Children<'r> {
    type Item = SubNode<'r>;
    fn next(&mut self) -> Option<SubNode<'r>> {
        let node = self.node.at(self.next)?;
        self.next = node.rec().next;
        self.remaining -= 1;
        Some(node)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for Children<'_> {}

impl<'r> SgNode<'r> for SubNode<'r> {
    fn parent(&self) -> Option<Self> {
        self.at(self.rec().parent)
    }
    fn ancestors(&self, _root: Self) -> impl Iterator<Item = Self> {
        let mut ancestor = self.parent();
        std::iter::from_fn(move || {
            let next = ancestor?;
            ancestor = next.parent();
            Some(next)
        })
    }
    fn children(&self) -> impl ExactSizeIterator<Item = Self> {
        Children {
            node: *self,
            next: self.rec().first,
            remaining: self.rec().children as usize,
        }
    }
    fn next(&self) -> Option<Self> {
        self.at(self.rec().next)
    }
    fn prev(&self) -> Option<Self> {
        self.at(self.rec().prev)
    }
    fn next_all(&self) -> impl Iterator<Item = Self> {
        let mut next = SgNode::next(self);
        std::iter::from_fn(move || {
            let node = next?;
            next = SgNode::next(&node);
            Some(node)
        })
    }
    fn prev_all(&self) -> impl Iterator<Item = Self> {
        let mut prev = SgNode::prev(self);
        std::iter::from_fn(move || {
            let node = prev?;
            prev = SgNode::prev(&node);
            Some(node)
        })
    }
    fn kind(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.tree.lang.0.kind_name(self.rec().kind))
    }
    fn kind_id(&self) -> u16 {
        self.rec().kind
    }
    fn node_id(&self) -> usize {
        self.tree.base | self.index as usize
    }
    fn range(&self) -> Range<usize> {
        let offset = self.tree.source.offset;
        self.rec().start as usize + offset..self.rec().end as usize + offset
    }
    fn start_pos(&self) -> Position {
        self.position(self.rec().start)
    }
    fn end_pos(&self) -> Position {
        self.position(self.rec().end)
    }
    fn is_named(&self) -> bool {
        self.rec().named
    }
    fn is_named_leaf(&self) -> bool {
        self.children().all(|child| !child.is_named())
    }
    fn is_leaf(&self) -> bool {
        self.rec().children == 0
    }
    fn is_error(&self) -> bool {
        self.rec().error
    }
    fn is_extra(&self) -> bool {
        self.rec().extra
    }
    fn field(&self, name: &str) -> Option<Self> {
        let id = self.tree.lang.field_to_id(name)?;
        self.child_by_field_id(id)
    }
    fn field_children(&self, field_id: Option<u16>) -> impl Iterator<Item = Self> {
        self.children()
            .filter(move |child| field_id.is_some_and(|id| id != 0 && child.rec().field == id))
    }
    fn child_by_field_id(&self, field_id: u16) -> Option<Self> {
        (field_id != 0)
            .then(|| self.children().find(|child| child.rec().field == field_id))
            .flatten()
    }
}

/// The sub-tree a node belongs to, when its document is one.
pub(crate) fn sub_tree<'r, D: Doc>(node: &ast_grep_core::Node<'r, D>) -> Option<&'r Arc<SubTree>> {
    let doc: &dyn std::any::Any = node.get_doc();
    doc.downcast_ref::<SubDoc>().map(|doc| &doc.tree)
}

/// What `parse` reads from a host node.
pub(crate) enum Parsed {
    Tree(Arc<SubTree>),
    /// A string whose value is not known (see `offsets::Decode::Unknown`).
    Unknown,
    /// Not a string this grammar reads.
    NotString,
}








#[cfg(test)]
mod tests;
