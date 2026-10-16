//! Literal sub-grammars (a Rerail extension): a string literal's decoded text parsed as a tree of
//! its own, which `parse` matches with a rule of the grammar's kinds. The tree is a separate
//! document with its own environment; its nodes map back to the source through the literal's
//! offset map, so captures exported to the host anchor where the text came from.

pub(crate) mod annotation;
pub(crate) mod brace;
pub(crate) mod eslint_directive;
pub(crate) mod offsets;
pub(crate) mod percent;
pub(crate) mod php_string;
pub(crate) mod phpdoc;
pub(crate) mod printf;
pub(crate) mod scanf;
pub(crate) mod semantic;

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::value::Value;
use ast_grep_core::Position;
use ast_grep_core::language::Language;
use ast_grep_core::matcher::{Pattern, PatternBuilder, PatternError};
use ast_grep_core::source::{Content, Doc, Edit, SgNode};
use ast_grep_core::tree_sitter::StrDoc;
use offsets::Decoded;
use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A sub-grammar `parse` can apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Grammar {
    /// printf-style formats (`percent`).
    Percent,
    /// `str.format` templates (`brace`).
    Brace,
    /// String type definitions read as Python expressions (`annotation`); the scope table
    /// parses them.
    Annotation,
    /// ESLint's directive comments (`eslintDirective`), in TypeScript.
    EslintDirective,
    /// PHP's printf-family formats (`printfFormat`).
    Printf,
    /// PHP's `sscanf`/`fscanf` formats (`scanfFormat`).
    Scanf,
    /// PHP doc comments as PHPStan's phpdoc-parser reads them (`phpDoc`).
    PhpDoc,
    /// Test-only: every decoded character is a `char` node under a `text` root.
    #[cfg(test)]
    Chars,
}

/// What a grammar parses: a node of one family's host language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LiteralHost {
    /// Python `string` and `concatenated_string` nodes, decoded as `str`.
    PythonString,
    /// TypeScript comments.
    EcmaComment,
    /// PHP string literals (`string`, `encapsed_string`, `heredoc`, `nowdoc`), decoded as PHP
    /// reads them.
    PhpString,
    /// PHP doc comments (`comment` nodes PHP lexes as `T_DOC_COMMENT`).
    PhpComment,
}

impl Grammar {
    /// Every grammar, in the order error messages list them.
    pub(crate) const ALL: &'static [Grammar] = &[
        Self::Percent,
        Self::Brace,
        Self::Annotation,
        Self::EslintDirective,
        Self::Printf,
        Self::Scanf,
        Self::PhpDoc,
        #[cfg(test)]
        Self::Chars,
    ];

    /// The language of this grammar's trees.
    pub(crate) fn lang(self) -> SubLang {
        SubLang(self)
    }

    /// The grammar a `parse.grammar` name selects.
    pub(crate) fn named(name: &str) -> Option<Self> {
        match name {
            "percentFormat" => Some(Self::Percent),
            "braceFormat" => Some(Self::Brace),
            "annotation" => Some(Self::Annotation),
            "eslintDirective" => Some(Self::EslintDirective),
            "printfFormat" => Some(Self::Printf),
            "scanfFormat" => Some(Self::Scanf),
            "phpDoc" => Some(Self::PhpDoc),
            #[cfg(test)]
            "testChars" => Some(Self::Chars),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Percent => "percentFormat",
            Self::Brace => "braceFormat",
            Self::Annotation => "annotation",
            Self::EslintDirective => "eslintDirective",
            Self::Printf => "printfFormat",
            Self::Scanf => "scanfFormat",
            Self::PhpDoc => "phpDoc",
            #[cfg(test)]
            Self::Chars => "testChars",
        }
    }

    /// The kinds a tree of this grammar has, in id order from 1.
    pub(crate) fn kinds(self) -> &'static [&'static str] {
        match self {
            Self::Percent => percent::KINDS,
            Self::Brace => brace::KINDS,
            // Python's own kinds, which the grammar checks when it compiles the rule.
            Self::Annotation => &[],
            Self::EslintDirective => eslint_directive::KINDS,
            Self::Printf => printf::KINDS,
            Self::Scanf => scanf::KINDS,
            Self::PhpDoc => phpdoc::KINDS,
            #[cfg(test)]
            Self::Chars => &["text", "char", "error"],
        }
    }

    /// The host nodes the grammar parses.
    pub(crate) fn literal_host(self) -> LiteralHost {
        match self {
            Self::Percent | Self::Brace | Self::Annotation => LiteralHost::PythonString,
            Self::EslintDirective => LiteralHost::EcmaComment,
            Self::Printf | Self::Scanf => LiteralHost::PhpString,
            Self::PhpDoc => LiteralHost::PhpComment,
            #[cfg(test)]
            Self::Chars => LiteralHost::PythonString,
        }
    }

    /// The language of the host the grammar parses, whose values its trees hold.
    pub(crate) fn host_language(self) -> GrammarDialect {
        match self.literal_host() {
            LiteralHost::PythonString => GrammarDialect::Python,
            LiteralHost::EcmaComment => GrammarDialect::TypeScript,
            LiteralHost::PhpString | LiteralHost::PhpComment => GrammarDialect::Php,
        }
    }

    /// What the grammar parses, for messages.
    pub(crate) fn host(self) -> &'static str {
        match self.literal_host() {
            LiteralHost::PythonString => "Python literals",
            LiteralHost::EcmaComment => "TypeScript comments",
            LiteralHost::PhpString => "PHP strings",
            LiteralHost::PhpComment => "PHP doc comments",
        }
    }

    /// Whether a tree depends on the literal alone, so a syntax error elsewhere cannot change it:
    /// whether a string is a type definition depends on the names the file binds.
    pub(crate) fn reads_only_the_node(self) -> bool {
        self != Self::Annotation
    }

    fn kind_id(self, kind: &str) -> u16 {
        if self == Self::Annotation {
            return GrammarDialect::Python.kind_to_id(kind);
        }
        self.kinds()
            .iter()
            .position(|known| *known == kind)
            .map_or(0, |index| index as u16 + 1)
    }

    fn kind_name(self, id: u16) -> &'static str {
        if self == Self::Annotation {
            return annotation::kind_name(id);
        }
        self.kinds()
            .get(usize::from(id).wrapping_sub(1))
            .copied()
            .unwrap_or("")
    }

    /// Parses decoded text into a tree root.
    fn parse(self, text: &str) -> PNode {
        match self {
            Self::Percent => percent::parse(text),
            Self::Brace => brace::parse(text),
            Self::Printf => printf::parse(text),
            Self::Scanf => scanf::parse(text),
            // Past the nesting bound an error root (`parse_php_comment` reads it as unknown).
            Self::PhpDoc => phpdoc::parse(text).unwrap_or_else(|| {
                let mut root = PNode::new(self, "php_doc", 0, text.len());
                root.error = true;
                root
            }),
            // A comment that is no directive has no tree (`parse_comment`).
            Self::EslintDirective => eslint_directive::tree(text)
                .unwrap_or_else(|| PNode::new(self, "directive", 0, text.len())),
            // The scope table parses annotations (`annotation::parse`).
            Self::Annotation => PNode {
                kind: annotation::ERROR_KIND,
                start: 0,
                end: text.len(),
                named: true,
                error: true,
                extra: false,
                field: 0,
                value: None,
                children: Vec::new(),
            },
            #[cfg(test)]
            Self::Chars => {
                let mut root = PNode::new(self, "text", 0, text.len());
                for (at, c) in text.char_indices() {
                    root.children.push(
                        PNode::new(self, "char", at, at + c.len_utf8())
                            .with_value(Value::Str(c.to_string())),
                    );
                }
                root
            }
        }
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
    fn of_node(&self, id: usize, _: Range<usize>) -> Option<Range<usize>> {
        let rec = self.nodes.get(self.index_of(id)?)?;
        Some(self.map_range(rec.start as usize..rec.end as usize))
    }

    fn of_range(&self, range: Range<usize>) -> Option<Range<usize>> {
        Some(self.map_range(self.decoded(range)?))
    }

    /// Only an exact literal's bytes are copied: elsewhere decoding rewrote them.
    fn copied(&self, range: Range<usize>) -> Option<Range<usize>> {
        let range = self.decoded(range)?;
        matches!(self.map, HostMap::Exact(_)).then(|| self.map_range(range))
    }
}

/// Node ids of sub-trees have the top bit set; tapes use small indices, tree-sitter addresses.
/// Blade trees (`blade::forte::tree`) draw their ids from the same counter.
pub(crate) const ID_FLAG: usize = 1 << 63;
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
        let tree = NEXT_TREE.fetch_add(1, Ordering::Relaxed) & ((1 << 39) - 1);
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
}

/// A sub-tree's text. Node ranges are offsets into it plus `offset`, which an annotation's tree
/// sets so its identifiers have ranges no host reference has (see `scope::python`).
#[derive(Clone)]
pub(crate) struct SubSource {
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
pub(crate) struct SubLang(Grammar);

impl Language for SubLang {
    fn kind_to_id(&self, kind: &str) -> u16 {
        self.0.kind_id(kind)
    }
    fn field_to_id(&self, field: &str) -> Option<u16> {
        (self.0 == Grammar::Annotation)
            .then(|| GrammarDialect::Python.field_to_id(field))
            .flatten()
    }
    fn build_pattern(&self, builder: &PatternBuilder) -> Result<Pattern, PatternError> {
        // Patterns are Python's, and only annotations hold Python.
        builder.build(|src| StrDoc::try_new(src, GrammarDialect::Python))
    }
}

/// A sub-tree as an ast-grep document.
#[derive(Clone)]
pub(crate) struct SubDoc {
    pub(crate) tree: Arc<SubTree>,
}

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
pub(crate) struct SubNode<'r> {
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

/// Parses a TypeScript comment ESLint reads as a directive (`eslintDirective`): its text, mapped
/// to the source by a shift. Spends one step of `spend` per byte and tree node; `None` when the
/// budget ran out.
pub(crate) fn parse_comment<D: Doc>(
    node: &ast_grep_core::Node<'_, D>,
    spend: &mut dyn FnMut(usize) -> bool,
) -> Option<Parsed> {
    let text = node.text();
    if !spend(text.len() + 1) {
        return None;
    }
    let Some(root) = eslint_directive::tree(&text) else {
        return Some(Parsed::NotString);
    };
    if !spend(root.size()) {
        return None;
    }
    Some(Parsed::Tree(Arc::new(SubTree::new(
        Grammar::EslintDirective,
        text.into_owned(),
        0,
        root,
        HostMap::Exact(node.range().start as isize),
    ))))
}

/// Parses a PHP doc comment (`phpDoc`): its text, mapped to the source by a shift. Spends one step
/// of `spend` per byte and tree node; `None` when the budget ran out.
pub(crate) fn parse_php_comment<D: Doc>(
    node: &ast_grep_core::Node<'_, D>,
    spend: &mut dyn FnMut(usize) -> bool,
) -> Option<Parsed> {
    let text = node.text();
    if !phpdoc::is_doc_comment(&text) {
        return Some(Parsed::NotString);
    }
    if !spend(text.len() + 1) {
        return None;
    }
    let Some(root) = phpdoc::parse(&text) else {
        return Some(Parsed::Unknown);
    };
    if !spend(root.size()) {
        return None;
    }
    Some(Parsed::Tree(Arc::new(SubTree::new(
        Grammar::PhpDoc,
        text.into_owned(),
        0,
        root,
        HostMap::Exact(node.range().start as isize),
    ))))
}

/// Parses a Python string node with a text grammar (not `annotation`, which the scope table
/// parses). Spends one step of `spend` per tree node; `None` when the budget ran out.
pub(crate) fn parse_string<D: Doc>(
    node: &ast_grep_core::Node<'_, D>,
    grammar: Grammar,
    spend: &mut dyn FnMut(usize) -> bool,
) -> Option<Parsed> {
    let decoded = match offsets::decode_python(node) {
        offsets::Decode::Known(decoded) => decoded,
        offsets::Decode::Unknown => return Some(Parsed::Unknown),
        offsets::Decode::NotString => return Some(Parsed::NotString),
    };
    if !spend(decoded.text.len() + 1) {
        return None;
    }
    let root = grammar.parse(&decoded.text);
    if !spend(root.size()) {
        return None;
    }
    let text = decoded.text.clone();
    Some(Parsed::Tree(Arc::new(SubTree::new(
        grammar,
        text,
        0,
        root,
        HostMap::Spans(decoded),
    ))))
}

/// Parses a PHP string literal with a text grammar (`printfFormat`, `scanfFormat`). Spends one
/// step of `spend` per byte and tree node; `None` when the budget ran out.
pub(crate) fn parse_php_string<D: Doc>(
    node: &ast_grep_core::Node<'_, D>,
    grammar: Grammar,
    spend: &mut dyn FnMut(usize) -> bool,
) -> Option<Parsed> {
    let (text, map) = match php_string::decode(node) {
        php_string::PhpLiteral::Known(text, map) => (text, map),
        php_string::PhpLiteral::Unknown => return Some(Parsed::Unknown),
        php_string::PhpLiteral::NotString => return Some(Parsed::NotString),
    };
    parse_php_text(grammar, text, map, spend)
}

/// Parses the string PHP gives an expression that is no string literal (`value/php.rs`: a
/// concatenation, a variable followed to its binding) with a text grammar, every node mapped to
/// `whole`, the expression's range. Bytes that are not UTF-8 and a value past 64 KiB are unknown.
/// Spends as `parse_php_string` does.
pub(crate) fn parse_php_value(
    bytes: Vec<u8>,
    whole: Range<usize>,
    grammar: Grammar,
    spend: &mut dyn FnMut(usize) -> bool,
) -> Option<Parsed> {
    let Some(text) = php_string::text(bytes) else {
        return Some(Parsed::Unknown);
    };
    parse_php_text(grammar, text, HostMap::Whole(whole), spend)
}

fn parse_php_text(
    grammar: Grammar,
    text: String,
    map: HostMap,
    spend: &mut dyn FnMut(usize) -> bool,
) -> Option<Parsed> {
    if !spend(text.len() + 1) {
        return None;
    }
    let root = grammar.parse(&text);
    if !spend(root.size()) {
        return None;
    }
    Some(Parsed::Tree(Arc::new(SubTree::new(
        grammar, text, 0, root, map,
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast_grep_core::AstGrep;
    use ast_grep_core::tree_sitter::LanguageExt;

    fn tree(source: &str) -> Arc<SubTree> {
        let root = GrammarDialect::Python.ast_grep(source);
        let node = root
            .root()
            .dfs()
            .find(|node| node.kind() == "string")
            .expect("a string");
        match parse_string(&node, Grammar::Chars, &mut |_| true) {
            Some(Parsed::Tree(tree)) => tree,
            _ => panic!("parses"),
        }
    }

    #[test]
    fn sub_trees_are_documents_with_mapped_nodes() {
        let source = "x = 'a\\x62c'";
        let tree = tree(source);
        let root = AstGrep::doc(SubDoc { tree: tree.clone() });
        let root = root.root();
        assert_eq!(root.kind(), "text");
        assert_eq!(root.text(), "abc");
        let chars: Vec<_> = root.children().collect();
        assert_eq!(chars.len(), 3);
        assert_eq!(chars[1].text(), "b");
        assert_eq!(
            chars[1].prev().map(|node| node.text().into_owned()),
            Some("a".into())
        );
        assert_eq!(
            chars[1].next().map(|node| node.text().into_owned()),
            Some("c".into())
        );
        assert_eq!(
            chars[2].parent().map(|node| node.kind().into_owned()),
            Some("text".into())
        );
        let host = crate::code_rule_dsl::origin::Origin::of_node(
            &*tree,
            chars[1].node_id(),
            chars[1].range(),
        )
        .expect("mapped");
        assert_eq!(&source[host], "\\x62");
        assert_eq!(
            tree.value(chars[2].node_id()),
            Some(&Value::Str("c".into()))
        );
        assert!(sub_tree(&chars[0]).is_some());
        assert!(chars[0].node_id() & ID_FLAG != 0);
        assert_eq!(
            chars.iter().map(|node| node.range()).collect::<Vec<_>>(),
            [0..1, 1..2, 2..3]
        );
    }
}
