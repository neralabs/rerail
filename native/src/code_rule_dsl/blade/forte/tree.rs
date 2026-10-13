//! Rerail's view of a Forte parse: an immutable tree implementing ast-grep's `Doc`, `SgNode`,
//! `Language` and `Content`, as `literal`'s sub-trees do, so rules and patterns run over Blade
//! templates.
//!
//! # Nodes
//!
//! One node per flat node Forte's tree builder links from its root, in preorder of Forte's links;
//! the root is `document`, spanning the whole template. A node's kind is the AST class
//! `Document::createBuiltInWrapper` wraps it in, snake_case without `Node`; where one class
//! serves several flat kinds, the flat kind's name:
//!
//! | kind | Forte |
//! |---|---|
//! | `document` | `Root` |
//! | `text` | `Text` (`TextNode`): a leaf, its tokens are its text |
//! | `element`, `component`, `slot` | `Element`, by `createElementWrapper`'s class |
//! | `directive`, `directive_block` | `Directive`, `DirectiveBlock` |
//! | `echo`, `raw_echo`, `triple_echo` | `Echo`, `RawEcho`, `TripleEcho` (`EchoNode`) |
//! | `comment`, `conditional_comment`, `bogus_comment`, `blade_comment` | the comment classes |
//! | `php_block`, `php_tag`, `verbatim`, `doctype`, `cdata` | the classes of those names |
//! | `xml_declaration`, `processing_instruction` | `Decl`, `ProcessingInstruction` |
//! | `escape` | `NonOutput` (`EscapeNode`) |
//! | `stray_closing_tag` | `UnpairedClosingTag` (`StrayClosingTagNode`) |
//! | `element_name`, `closing_element_name` | `ElementName`, `ClosingElementName` |
//! | `attribute`, `jsx_attribute`, `attribute_whitespace` | flat kinds of those names |
//! | `attribute_name`, `attribute_value` | flat kinds of those names |
//!
//! # Leaves
//!
//! A node's own tokens that none of its children covers become leaf children, in text order, so
//! the tree is lossless up to whitespace: whitespace tokens, zero-width tokens and the space
//! trimmed off a `php` or `comment_text` leaf are gaps, as tree-sitter leaves them. `text` and
//! `attribute_whitespace` nodes are leaves themselves. By token type:
//!
//! | token | leaf |
//! |---|---|
//! | `<` `>` `/` `=` `"` `'` | anonymous, kind = text (a doctype's `>` too) |
//! | `<!--` `-->` `<!DOCTYPE` `<![CDATA[` `]]>` `<?xml` `?>` `@` | anonymous (`<!DOCTYPE`: any case) |
//! | `{{` `}}` `{!!` `!!}` `{{{` `}}}` `{{--` `--}}` | anonymous |
//! | `@php` `@endphp` `@verbatim` `@endverbatim` | anonymous |
//! | `<?php` `<?=` `<?` | anonymous, by the tag's opening (`<?php` in any case) |
//! | directive (`@name`) | `directive_name` |
//! | directive arguments | anonymous `(` and `)` around `php`, the trimmed text between (none when empty) |
//! | echo content, `@php` content, `<?php` content | `php`, trimmed (none when empty) |
//! | text of a `comment`, `blade_comment`, `bogus_comment`, `conditional_comment`; a bogus comment token | `comment_text`, trimmed |
//! | other text | `text` |
//! | tag name, attribute name, attribute value | `tag_name`, `attribute_name`, `attribute_value` |
//! | `:name`, `::name`, `:$name` | `bound_attribute_name`, `escaped_attribute_name`, `shorthand_attribute_name` |
//! | doctype content | `doctype_value` |
//! | `<!--[if ...]>`, `<![endif]-->` | `conditional_comment_start`, `conditional_comment_end` |
//! | `<?target` | `processing_instruction_start` |
//! | JSX value `{...}`, JSX `{...spread}`, TSX `<T>` | `jsx_attribute_value`, `jsx_shorthand_attribute`, `tsx_generic_type` |
//!
//! # Fields
//!
//! | node | fields |
//! |---|---|
//! | `element`, `component`, `slot` | `name` (`element_name`), `closing_name` (`closing_element_name`) |
//! | `attribute`, `jsx_attribute` | `name` (`attribute_name`), `value` (`attribute_value`) |
//! | `directive` | `name` (`directive_name`), `arguments` (the arguments' `php`) |
//! | `echo`, `raw_echo`, `triple_echo` | `expression` (`php`) |
//! | `php_block`, `php_tag` | `code` (`php`) |
//! | `directive_block` | `start`, `end` (`DirectiveBlockNode::startDirective`, `endDirective`) |
//!
//! # Spans
//!
//! A node spans its own tokens and its children: Forte's directives own their branch content as
//! children past their own tokens, and a block covers all. Forte's flat records can claim tokens
//! that belong to another node: a directive block left open where an element closes counts the
//! element's closing tag and the directive that later closes it among its tokens. So each token
//! belongs to one node only: of the nodes whose token range holds it, the one with the fewest
//! tokens (the deeper on a tie); directive blocks claim none (their children hold their
//! directives). Children are ordered by start. Nodes without tokens or children sit, zero-wide,
//! where their previous sibling ends (their parent's start for a first child). The test
//! `trees_are_well_formed` checks that children lie in their parent, in order, without overlap
//! (so every byte is in at most one leaf), over every recorded tree.
//!
//! None of these nodes is an error node: a template's lexer errors and a parse's failure are
//! byte ranges on the tree (`error_ranges`), as a tape's syntax errors are. A template Forte does
//! not parse (`ParseError`) is a `document` alone.

use super::ast::{Ast, ElementClass};
use super::directives::Directives;
use super::node::{FlatNode, NodeKind};
use super::token::{Token, TokenType};
use super::{Budget, DepthLimits, ParseError, Parsed};
use crate::code_rule_dsl::literal::{ID_FLAG, NEXT_TREE};
use ast_grep_core::Position;
use ast_grep_core::language::Language;
use ast_grep_core::matcher::{Pattern, PatternBuilder, PatternError};
use ast_grep_core::source::{Content, Doc, Edit, SgNode};
use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::Ordering;

macro_rules! kinds {
    ($($kind:ident = $name:literal,)*) => {
        /// A node kind; its id is its discriminant.
        #[repr(u16)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum Kind {
            _Unknown = 0,
            $($kind,)*
        }

        /// Kind names, by id (0: no kind). A Blade tape numbers Blade's kinds by these ids
        /// (`language::KindCatalog`).
        pub(crate) const KIND_NAMES: &[&str] = &["", $($name,)*];
    };
}

kinds! {
    Document = "document",
    Text = "text",
    Element = "element",
    Component = "component",
    Slot = "slot",
    Directive = "directive",
    DirectiveBlock = "directive_block",
    Echo = "echo",
    RawEcho = "raw_echo",
    TripleEcho = "triple_echo",
    Comment = "comment",
    ConditionalComment = "conditional_comment",
    BogusComment = "bogus_comment",
    BladeComment = "blade_comment",
    PhpBlock = "php_block",
    PhpTag = "php_tag",
    Verbatim = "verbatim",
    Doctype = "doctype",
    Cdata = "cdata",
    XmlDeclaration = "xml_declaration",
    ProcessingInstruction = "processing_instruction",
    Escape = "escape",
    StrayClosingTag = "stray_closing_tag",
    ElementName = "element_name",
    ClosingElementName = "closing_element_name",
    Attribute = "attribute",
    JsxAttribute = "jsx_attribute",
    AttributeWhitespace = "attribute_whitespace",
    AttributeName = "attribute_name",
    AttributeValue = "attribute_value",
    // Named leaves.
    Php = "php",
    DirectiveName = "directive_name",
    CommentText = "comment_text",
    TagName = "tag_name",
    DoctypeValue = "doctype_value",
    JsxAttributeValue = "jsx_attribute_value",
    JsxShorthandAttribute = "jsx_shorthand_attribute",
    TsxGenericType = "tsx_generic_type",
    ConditionalCommentStart = "conditional_comment_start",
    ConditionalCommentEnd = "conditional_comment_end",
    BoundAttributeName = "bound_attribute_name",
    EscapedAttributeName = "escaped_attribute_name",
    ShorthandAttributeName = "shorthand_attribute_name",
    ProcessingInstructionStart = "processing_instruction_start",
    // Anonymous leaves (the first is `FIRST_ANONYMOUS`).
    LessThan = "<",
    GreaterThan = ">",
    Slash = "/",
    Equals = "=",
    DoubleQuote = "\"",
    SingleQuote = "'",
    CommentOpen = "<!--",
    CommentClose = "-->",
    DoctypeOpen = "<!DOCTYPE",
    At = "@",
    EchoOpen = "{{",
    EchoClose = "}}",
    RawEchoOpen = "{!!",
    RawEchoClose = "!!}",
    TripleEchoOpen = "{{{",
    TripleEchoClose = "}}}",
    OpenParen = "(",
    CloseParen = ")",
    VerbatimOpen = "@verbatim",
    VerbatimClose = "@endverbatim",
    PhpBlockOpen = "@php",
    PhpBlockClose = "@endphp",
    PhpTagOpen = "<?php",
    PhpEchoTagOpen = "<?=",
    PhpShortTagOpen = "<?",
    QuestionClose = "?>",
    BladeCommentOpen = "{{--",
    BladeCommentClose = "--}}",
    CdataOpen = "<![CDATA[",
    CdataClose = "]]>",
    XmlDeclarationOpen = "<?xml",
}

/// Kinds from this id on are anonymous (punctuation and delimiters).
pub(crate) const FIRST_ANONYMOUS: u16 = Kind::LessThan as u16;

/// Field names, by id (0: no field).
pub(crate) const FIELD_NAMES: &[&str] = &[
    "",
    "name",
    "closing_name",
    "value",
    "arguments",
    "expression",
    "code",
    "start",
    "end",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
enum Field {
    None = 0,
    Name,
    ClosingName,
    Value,
    Arguments,
    Expression,
    Code,
    Start,
    End,
}

const NONE: u32 = u32::MAX;

/// Node ids of a tree are its base plus a preorder index below `SLOT` per tree slot it took
/// from `NEXT_TREE` (the counter `literal`'s sub-trees draw from).
const SLOT_BITS: u32 = 24;

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
}

/// A template's text and line starts.
#[derive(Clone)]
pub(crate) struct BladeSource {
    bytes: Vec<u8>,
    lines: Vec<u32>,
}

impl BladeSource {
    fn new(bytes: Vec<u8>) -> Self {
        let mut lines = vec![0];
        lines.extend(
            bytes
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .map(|(at, _)| at as u32 + 1),
        );
        Self { bytes, lines }
    }

    /// The line holding byte `at` and where it starts.
    fn line_of(&self, at: usize) -> (usize, usize) {
        let line = self
            .lines
            .partition_point(|start| *start as usize <= at)
            .saturating_sub(1);
        (
            line,
            self.lines
                .get(line)
                .map_or(0, |start| (*start as usize).min(at)),
        )
    }

    fn position(&self, at: usize) -> Position {
        let (line, start) = self.line_of(at);
        Position::new(line, at - start, at)
    }
}

impl Content for BladeSource {
    type Underlying = u8;
    fn get_range(&self, range: Range<usize>) -> &[u8] {
        let start = range.start.min(self.bytes.len());
        let end = range.end.clamp(start, self.bytes.len());
        &self.bytes[start..end]
    }
    fn decode_str(src: &str) -> Cow<'_, [u8]> {
        Cow::Borrowed(src.as_bytes())
    }
    fn encode_bytes(bytes: &[u8]) -> Cow<'_, str> {
        String::from_utf8_lossy(bytes)
    }
    fn get_char_column(&self, _column: usize, offset: usize) -> usize {
        let end = offset.min(self.bytes.len());
        let (_, start) = self.line_of(end);
        self.bytes[start..end]
            .iter()
            .filter(|byte| **byte & 0b1100_0000 != 0b1000_0000)
            .count()
    }
}

/// A parsed template as a tree.
pub(crate) struct BladeTree {
    source: BladeSource,
    nodes: Vec<Rec>,
    /// The flat node each node stands for (`NONE` for leaves).
    forte: Vec<u32>,
    errors: Vec<Range<usize>>,
    parse_error: Option<ParseError>,
    base: usize,
}

/// A leaf or a child to place under a node.
#[derive(Clone, Copy)]
enum Item {
    Node { index: u32, field: Field },
    Leaf { kind: Kind, field: Field },
}

/// Whether a flat node is a leaf itself: its tokens make no leaves.
fn is_terminal(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::Text | NodeKind::AttributeWhitespace)
}

/// Whether a flat node claims the tokens in its range (`Spans` in the module doc).
fn claims_tokens(kind: NodeKind) -> bool {
    !matches!(kind, NodeKind::Root | NodeKind::DirectiveBlock)
}

fn is_comment(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Comment
            | NodeKind::BladeComment
            | NodeKind::BogusComment
            | NodeKind::ConditionalComment
    )
}

/// The kind of a flat node (the module doc's table).
fn node_kind(ast: &Ast<'_>, index: usize, kind: NodeKind) -> Kind {
    match kind {
        NodeKind::Root => Kind::Document,
        NodeKind::Element => match ast.element_class(index) {
            ElementClass::Element => Kind::Element,
            ElementClass::Component => Kind::Component,
            ElementClass::Slot => Kind::Slot,
        },
        NodeKind::Text => Kind::Text,
        NodeKind::Echo => Kind::Echo,
        NodeKind::RawEcho => Kind::RawEcho,
        NodeKind::TripleEcho => Kind::TripleEcho,
        NodeKind::Directive => Kind::Directive,
        NodeKind::DirectiveBlock => Kind::DirectiveBlock,
        NodeKind::Verbatim => Kind::Verbatim,
        NodeKind::PhpBlock => Kind::PhpBlock,
        NodeKind::PhpTag => Kind::PhpTag,
        NodeKind::Comment => Kind::Comment,
        NodeKind::BogusComment => Kind::BogusComment,
        NodeKind::ConditionalComment => Kind::ConditionalComment,
        NodeKind::BladeComment => Kind::BladeComment,
        NodeKind::Doctype => Kind::Doctype,
        NodeKind::Cdata => Kind::Cdata,
        NodeKind::Decl => Kind::XmlDeclaration,
        NodeKind::Attribute => Kind::Attribute,
        NodeKind::JsxAttribute => Kind::JsxAttribute,
        NodeKind::UnpairedClosingTag => Kind::StrayClosingTag,
        NodeKind::NonOutput => Kind::Escape,
        NodeKind::ElementName => Kind::ElementName,
        NodeKind::ClosingElementName => Kind::ClosingElementName,
        NodeKind::AttributeWhitespace => Kind::AttributeWhitespace,
        NodeKind::AttributeName => Kind::AttributeName,
        NodeKind::AttributeValue => Kind::AttributeValue,
        NodeKind::ProcessingInstruction => Kind::ProcessingInstruction,
    }
}

/// The field of a leaf of `kind` under a node of `owner`'s kind.
fn leaf_field(owner: NodeKind, kind: Kind) -> Field {
    match (owner, kind) {
        (NodeKind::Directive, Kind::DirectiveName) => Field::Name,
        (NodeKind::Directive, Kind::Php) => Field::Arguments,
        (NodeKind::Echo | NodeKind::RawEcho | NodeKind::TripleEcho, Kind::Php) => Field::Expression,
        (NodeKind::PhpBlock | NodeKind::PhpTag, Kind::Php) => Field::Code,
        _ => Field::None,
    }
}

/// `text` without PHP's `trim` characters, as a range.
fn trimmed(source: &[u8], range: Range<usize>) -> Range<usize> {
    let text = &source[range.clone()];
    let start = text
        .iter()
        .position(|b| !super::scan::is_trim(*b))
        .unwrap_or(text.len());
    let end = text
        .iter()
        .rposition(|b| !super::scan::is_trim(*b))
        .map_or(start, |at| at + 1);
    range.start + start..range.start + end
}

/// The leaves a token of `owner`'s makes (the module doc's table).
fn token_leaves(
    source: &[u8],
    token: &Token,
    owner: NodeKind,
    mut leaf: impl FnMut(Kind, Range<usize>),
) {
    let range = token.start as usize..token.end as usize;
    if range.is_empty() || range.end > source.len() {
        return;
    }
    let text = &source[range.clone()];
    let kind = match token.kind {
        TokenType::Whitespace | TokenType::SyntheticClose => return,
        TokenType::EchoContent | TokenType::PhpBlock | TokenType::PhpContent => {
            let code = trimmed(source, range);
            if !code.is_empty() {
                leaf(Kind::Php, code);
            }
            return;
        }
        TokenType::Text if is_comment(owner) => {
            let comment = trimmed(source, range);
            if !comment.is_empty() {
                leaf(Kind::CommentText, comment);
            }
            return;
        }
        TokenType::DirectiveArgs => {
            let open = text.first() == Some(&b'(');
            let close = text.len() > usize::from(open) && text.last() == Some(&b')');
            let inner = range.start + usize::from(open)..range.end - usize::from(close);
            if open {
                leaf(Kind::OpenParen, range.start..range.start + 1);
            }
            let code = trimmed(source, inner);
            if !code.is_empty() {
                leaf(Kind::Php, code);
            }
            if close {
                leaf(Kind::CloseParen, range.end - 1..range.end);
            }
            return;
        }
        TokenType::Text => Kind::Text,
        TokenType::LessThan => Kind::LessThan,
        TokenType::GreaterThan | TokenType::DoctypeEnd => Kind::GreaterThan,
        TokenType::Slash => Kind::Slash,
        TokenType::Equals => Kind::Equals,
        TokenType::Quote if text == b"'" => Kind::SingleQuote,
        TokenType::Quote => Kind::DoubleQuote,
        TokenType::TagName => Kind::TagName,
        TokenType::AttributeName => Kind::AttributeName,
        TokenType::AttributeValue => Kind::AttributeValue,
        TokenType::CommentStart => Kind::CommentOpen,
        TokenType::CommentEnd => Kind::CommentClose,
        TokenType::DoctypeStart => Kind::DoctypeOpen,
        TokenType::Doctype => Kind::DoctypeValue,
        TokenType::AtSign => Kind::At,
        TokenType::EchoStart => Kind::EchoOpen,
        TokenType::EchoEnd => Kind::EchoClose,
        TokenType::RawEchoStart => Kind::RawEchoOpen,
        TokenType::RawEchoEnd => Kind::RawEchoClose,
        TokenType::TripleEchoStart => Kind::TripleEchoOpen,
        TokenType::TripleEchoEnd => Kind::TripleEchoClose,
        TokenType::Directive => Kind::DirectiveName,
        TokenType::VerbatimStart => Kind::VerbatimOpen,
        TokenType::VerbatimEnd => Kind::VerbatimClose,
        TokenType::JsxAttributeValue => Kind::JsxAttributeValue,
        TokenType::JsxShorthandAttribute => Kind::JsxShorthandAttribute,
        TokenType::TsxGenericType => Kind::TsxGenericType,
        TokenType::BogusComment => Kind::CommentText,
        TokenType::ConditionalCommentStart => Kind::ConditionalCommentStart,
        TokenType::ConditionalCommentEnd => Kind::ConditionalCommentEnd,
        TokenType::PhpBlockStart => Kind::PhpBlockOpen,
        TokenType::PhpBlockEnd => Kind::PhpBlockClose,
        TokenType::PhpTagStart if text.len() >= 5 && text[..5].eq_ignore_ascii_case(b"<?php") => {
            Kind::PhpTagOpen
        }
        TokenType::PhpTagStart if text.starts_with(b"<?=") => Kind::PhpEchoTagOpen,
        TokenType::PhpTagStart => Kind::PhpShortTagOpen,
        TokenType::PhpTagEnd | TokenType::DeclEnd | TokenType::PiEnd => Kind::QuestionClose,
        TokenType::BladeCommentStart => Kind::BladeCommentOpen,
        TokenType::BladeCommentEnd => Kind::BladeCommentClose,
        TokenType::EscapedAttribute => Kind::EscapedAttributeName,
        TokenType::BoundAttribute => Kind::BoundAttributeName,
        TokenType::ShorthandAttribute => Kind::ShorthandAttributeName,
        TokenType::CdataStart => Kind::CdataOpen,
        TokenType::CdataEnd => Kind::CdataClose,
        TokenType::DeclStart => Kind::XmlDeclarationOpen,
        TokenType::PiStart => Kind::ProcessingInstructionStart,
    };
    leaf(kind, range);
}

/// `find` of a union-find over token indices: the first token at or after `at` no node has
/// claimed yet (`next.len() - 1` when none).
fn unclaimed(next: &mut [u32], mut at: usize) -> usize {
    while next[at] as usize != at {
        let up = next[next[at] as usize];
        next[at] = up;
        at = up as usize;
    }
    at
}

impl BladeTree {
    /// The tree of a parse of `source`.
    pub(crate) fn new(source: Vec<u8>, parsed: Result<Parsed, ParseError>) -> Self {
        match parsed {
            Ok(parsed) => Self::build(
                source,
                &parsed.tokens,
                &parsed.nodes,
                parsed.errors.iter().map(|e| e.offset),
            ),
            Err(error) => {
                let len = source.len();
                let mut tree = Self::build(source, &[], &[], std::iter::empty());
                tree.errors.push(0..len);
                tree.parse_error = Some(error);
                tree
            }
        }
    }

    /// The tree over Forte's flat arrays.
    pub(crate) fn build(
        source: Vec<u8>,
        tokens: &[Token],
        flat: &[FlatNode],
        error_offsets: impl Iterator<Item = u32>,
    ) -> Self {
        let len = source.len();
        let errors = error_offsets
            .map(|offset| {
                let at = (offset as usize).min(len);
                at..at
            })
            .collect();
        let mut tree = Self {
            source: BladeSource::new(source),
            nodes: Vec::new(),
            forte: Vec::new(),
            errors,
            parse_error: None,
            base: 0,
        };
        tree.link(tokens, flat);
        let slots = (tree.nodes.len() >> SLOT_BITS) + 1;
        let first = NEXT_TREE.fetch_add(slots, Ordering::Relaxed) & ((1 << 39) - 1);
        tree.base = ID_FLAG | (first << SLOT_BITS);
        tree
    }

    /// Builds the node records (the module doc's rules).
    fn link(&mut self, tokens: &[Token], flat: &[FlatNode]) {
        let source = &self.source.bytes;
        let len = source.len() as u32;
        let token_count = tokens.len();
        let ast = Ast::new(source, tokens, flat);
        let at = |index: i32| {
            usize::try_from(index)
                .ok()
                .filter(|index| *index < flat.len())
        };

        // Forte's nodes reachable from its root, in preorder, with their depths and children.
        let mut order: Vec<u32> = Vec::with_capacity(flat.len());
        let mut depth = vec![0u32; flat.len()];
        let mut seen = vec![false; flat.len()];
        let mut child_list: Vec<u32> = Vec::with_capacity(flat.len());
        let mut stack: Vec<u32> = Vec::new();
        if !flat.is_empty() {
            seen[0] = true;
            stack.push(0);
        }
        let mut lists: Vec<(u32, u32)> = vec![(0, 0); flat.len()];
        while let Some(node) = stack.pop() {
            order.push(node);
            let first = child_list.len();
            let mut child = flat[node as usize].first_child;
            while let Some(index) = at(child) {
                if seen[index] {
                    break;
                }
                seen[index] = true;
                depth[index] = depth[node as usize] + 1;
                child_list.push(index as u32);
                child = flat[index].next_sibling;
            }
            lists[node as usize] = (first as u32, child_list.len() as u32);
            stack.extend(child_list[first..].iter().rev());
        }
        let children_of = |node: u32| {
            let (start, end) = lists[node as usize];
            &child_list[start as usize..end as usize]
        };
        let own = |node: &FlatNode| -> Range<usize> {
            let start = usize::try_from(node.token_start)
                .unwrap_or(token_count)
                .min(token_count);
            let count = usize::try_from(node.token_count).unwrap_or(0);
            start..start.saturating_add(count).min(token_count)
        };

        // Each token's node: of the claiming nodes whose range holds it, the one with the fewest
        // tokens, the deeper on a tie.
        let mut claimants: Vec<u32> = order
            .iter()
            .copied()
            .filter(|node| {
                claims_tokens(flat[*node as usize].kind) && !own(&flat[*node as usize]).is_empty()
            })
            .collect();
        claimants.sort_by_key(|node| {
            (
                own(&flat[*node as usize]).len(),
                std::cmp::Reverse(depth[*node as usize]),
            )
        });
        let mut next: Vec<u32> = (0..=token_count as u32).collect();
        let mut owner = vec![NONE; token_count];
        for node in claimants {
            let range = own(&flat[node as usize]);
            let mut token = unclaimed(&mut next, range.start);
            while token < range.end {
                owner[token] = node;
                next[token] = token as u32 + 1;
                token = unclaimed(&mut next, token + 1);
            }
        }
        // Each node's tokens, in order.
        let mut owned_start = vec![0u32; flat.len() + 1];
        for node in owner.iter().filter(|node| **node != NONE) {
            owned_start[*node as usize + 1] += 1;
        }
        for i in 0..flat.len() {
            owned_start[i + 1] += owned_start[i];
        }
        let mut fill = owned_start.clone();
        let mut owned = vec![0u32; owned_start[flat.len()] as usize];
        for (token, node) in owner.iter().enumerate().filter(|(_, node)| **node != NONE) {
            owned[fill[*node as usize] as usize] = token as u32;
            fill[*node as usize] += 1;
        }
        let owned_by = |node: u32| {
            &owned[owned_start[node as usize] as usize..owned_start[node as usize + 1] as usize]
        };

        // Spans, bottom up: a node's tokens and its children; `None` for neither.
        let mut span: Vec<Option<(u32, u32)>> = vec![None; flat.len()];
        for &node in order.iter().rev() {
            let mut covered: Option<(u32, u32)> = None;
            let mut add = |start: u32, end: u32| {
                covered = Some(covered.map_or((start, end), |(s, e)| (s.min(start), e.max(end))));
            };
            if let (Some(first), Some(last)) = (owned_by(node).first(), owned_by(node).last()) {
                add(
                    tokens[*first as usize].start.min(len),
                    tokens[*last as usize].end.min(len),
                );
            }
            for child in children_of(node) {
                if let Some((start, end)) = span[*child as usize] {
                    add(start, end);
                }
            }
            span[node as usize] = covered;
        }
        if !flat.is_empty() {
            span[0] = Some((0, len));
        }

        // Records, in preorder.
        let mut last: Vec<u32> = Vec::new();
        let mut items: Vec<(u32, u32, Item)> = Vec::new();
        // (item, parent record, start, end)
        let mut pending: Vec<(Item, u32, u32, u32)> = vec![];
        if flat.is_empty() {
            self.nodes.push(Rec {
                kind: Kind::Document as u16,
                field: 0,
                parent: NONE,
                first: NONE,
                next: NONE,
                prev: NONE,
                children: 0,
                start: 0,
                end: len,
            });
            self.forte.push(NONE);
            return;
        }
        pending.push((
            Item::Node {
                index: 0,
                field: Field::None,
            },
            NONE,
            0,
            len,
        ));
        while let Some((item, parent, start, end)) = pending.pop() {
            let index = self.nodes.len() as u32;
            let prev = if parent == NONE {
                NONE
            } else {
                let parent_rec = &mut self.nodes[parent as usize];
                parent_rec.children += 1;
                let prev = last[parent as usize];
                if prev == NONE {
                    parent_rec.first = index;
                } else {
                    self.nodes[prev as usize].next = index;
                }
                last[parent as usize] = index;
                prev
            };
            last.push(NONE);
            let (kind, field, forte) = match item {
                Item::Node { index: node, field } => {
                    let flat_kind = flat[node as usize].kind;
                    (node_kind(&ast, node as usize, flat_kind), field, node)
                }
                Item::Leaf { kind, field } => (kind, field, NONE),
            };
            self.nodes.push(Rec {
                kind: kind as u16,
                field: field as u16,
                parent,
                first: NONE,
                next: NONE,
                prev,
                children: 0,
                start,
                end,
            });
            self.forte.push(forte);
            let Item::Node { index: node, .. } = item else {
                continue;
            };
            let node_flat = &flat[node as usize];

            // Children: Forte's, then the leaves of the node's tokens, ordered by start.
            items.clear();
            let block =
                (node_flat.kind == NodeKind::DirectiveBlock).then(|| ast.block(node as usize));
            let mut previous_end = start;
            for &child in children_of(node) {
                let (child_start, child_end) = match span[child as usize] {
                    Some(placed) => placed,
                    None => {
                        span[child as usize] = Some((previous_end, previous_end));
                        (previous_end, previous_end)
                    }
                };
                previous_end = child_end;
                let child_kind = flat[child as usize].kind;
                let field = match (node_flat.kind, child_kind) {
                    (NodeKind::Element, NodeKind::ElementName) => Field::Name,
                    (NodeKind::Element, NodeKind::ClosingElementName) => Field::ClosingName,
                    (NodeKind::Attribute | NodeKind::JsxAttribute, NodeKind::AttributeName) => {
                        Field::Name
                    }
                    (NodeKind::Attribute | NodeKind::JsxAttribute, NodeKind::AttributeValue) => {
                        Field::Value
                    }
                    (NodeKind::DirectiveBlock, _) => match &block {
                        Some(block) if block.start == Some(child as usize) => Field::Start,
                        Some(block) if block.end == Some(child as usize) => Field::End,
                        _ => Field::None,
                    },
                    _ => Field::None,
                };
                items.push((
                    child_start,
                    child_end,
                    Item::Node {
                        index: child,
                        field,
                    },
                ));
            }
            if !is_terminal(node_flat.kind) {
                for &token in owned_by(node) {
                    token_leaves(
                        source,
                        &tokens[token as usize],
                        node_flat.kind,
                        |kind, range| {
                            let field = leaf_field(node_flat.kind, kind);
                            items.push((
                                range.start as u32,
                                range.end as u32,
                                Item::Leaf { kind, field },
                            ));
                        },
                    );
                }
            }
            items.sort_by_key(|(start, end, _)| (*start, *end));
            pending.extend(
                items
                    .iter()
                    .rev()
                    .map(|(start, end, item)| (*item, index, *start, *end)),
            );
        }
    }

    #[cfg(test)]
    fn index_of(&self, node_id: usize) -> Option<usize> {
        let index = node_id.checked_sub(self.base)?;
        (index < self.nodes.len()).then_some(index)
    }

    /// Nodes in the tree.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// The template's bytes.
    #[cfg(test)]
    pub(crate) fn source(&self) -> &[u8] {
        &self.source.bytes
    }

    /// The `document` node.
    pub(crate) fn root(&self) -> BladeNode<'_> {
        BladeNode {
            tree: self,
            index: 0,
        }
    }

    /// Where the template has lexer errors (zero-wide, at the error's offset), or the whole
    /// template when Forte did not parse it.
    pub(crate) fn error_ranges(&self) -> &[Range<usize>] {
        &self.errors
    }

    /// Why the template has no tree, when it has none.
    #[cfg(test)]
    pub(crate) fn parse_error(&self) -> Option<&ParseError> {
        self.parse_error.as_ref()
    }

    /// The flat node (index into Forte's array) the node with this id stands for.
    #[cfg(test)]
    pub(crate) fn forte_index(&self, node_id: usize) -> Option<usize> {
        let forte = *self.forte.get(self.index_of(node_id)?)?;
        (forte != NONE).then_some(forte as usize)
    }
}

/// Parses a template into a tree with Forte's default directives, accepting every `@name` as a
/// directive (`ParserOptions::acceptAllDirectives`): an application's or a package's directive
/// (`@ray(...)`, `@livewire(...)`, a `Blade::if` pair) is a `directive` node a rule can name, as
/// Blade's tooling reads it, not text. The view reads such a directive as Blade does
/// (`blade::view`): one `BladeCompiler` does not compile is not PHP the template shows.
pub(crate) fn parse_tree(source: &[u8]) -> BladeTree {
    let mut budget = Budget::for_template(source.len());
    let mut directives = Directives::with_defaults();
    directives.set_accept_all(true);
    let parsed = super::parse(source, directives, DepthLimits::default(), &mut budget);
    BladeTree::new(source.to_vec(), parsed)
}

/// The Blade language of Forte's trees; patterns are parsed with `parse_tree`. A metavariable is
/// written `$X` and read as `µX` (its expando character), so a pattern's PHP variables (`$x`) stay
/// PHP: Forte reads `µX` in an echo or in arguments as opaque PHP bytes, the leaf's whole text.
#[derive(Clone, Copy, Default)]
pub(crate) struct BladeLang;

impl BladeLang {
    /// A document over a template, parsed as patterns are.
    pub(crate) fn doc(self, source: &[u8]) -> BladeDoc {
        BladeDoc {
            tree: Arc::new(parse_tree(source)),
            lang: self,
        }
    }
}

impl Language for BladeLang {
    fn kind_to_id(&self, kind: &str) -> u16 {
        KIND_NAMES
            .iter()
            .position(|name| *name == kind)
            .map_or(0, |id| id as u16)
    }
    fn field_to_id(&self, field: &str) -> Option<u16> {
        FIELD_NAMES
            .iter()
            .skip(1)
            .position(|name| *name == field)
            .map(|id| id as u16 + 1)
    }
    fn expando_char(&self) -> char {
        'µ'
    }
    /// `$X`, `$_`, `$$X` and `$$$X` become `µX`, `µ_`, `µµX` and `µµµX`; a `$` before anything
    /// else (`$x`, `$$x`) is PHP's.
    fn pre_process_pattern<'q>(&self, query: &'q str) -> Cow<'q, str> {
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
        builder.build(|src| Ok(self.doc(src.as_bytes())))
    }
}

/// A Blade tree as an ast-grep document.
#[derive(Clone)]
pub(crate) struct BladeDoc {
    pub(crate) tree: Arc<BladeTree>,
    lang: BladeLang,
}

impl BladeDoc {
    #[cfg(test)]
    pub(crate) fn new(tree: BladeTree, lang: BladeLang) -> Self {
        Self {
            tree: Arc::new(tree),
            lang,
        }
    }
}

impl Doc for BladeDoc {
    type Source = BladeSource;
    type Lang = BladeLang;
    type Node<'r> = BladeNode<'r>;
    fn get_lang(&self) -> &BladeLang {
        &self.lang
    }
    fn get_source(&self) -> &BladeSource {
        &self.tree.source
    }
    fn do_edit(&mut self, _edit: &Edit<BladeSource>) -> Result<(), String> {
        Err("Blade trees are immutable".into())
    }
    fn root_node(&self) -> BladeNode<'_> {
        BladeNode {
            tree: &self.tree,
            index: 0,
        }
    }
    fn get_node_text<'a>(&'a self, node: &BladeNode<'a>) -> Cow<'a, str> {
        let rec = node.rec();
        String::from_utf8_lossy(
            self.tree
                .source
                .get_range(rec.start as usize..rec.end as usize),
        )
    }
}

#[derive(Clone, Copy)]
pub(crate) struct BladeNode<'r> {
    tree: &'r BladeTree,
    index: u32,
}

impl<'r> BladeNode<'r> {
    fn rec(&self) -> &Rec {
        &self.tree.nodes[self.index as usize]
    }
    fn at(&self, index: u32) -> Option<Self> {
        (index != NONE).then_some(Self { index, ..*self })
    }
    /// The field this node is its parent's, if any.
    pub(crate) fn field_name(&self) -> Option<&'static str> {
        match self.rec().field {
            0 => None,
            field => FIELD_NAMES.get(usize::from(field)).copied(),
        }
    }
    /// The template's bytes this node spans.
    pub(crate) fn bytes(&self) -> &'r [u8] {
        self.tree
            .source
            .get_range(self.rec().start as usize..self.rec().end as usize)
    }
}

struct Children<'r> {
    node: BladeNode<'r>,
    next: u32,
    remaining: usize,
}

impl<'r> Iterator for Children<'r> {
    type Item = BladeNode<'r>;
    fn next(&mut self) -> Option<BladeNode<'r>> {
        let node = self.node.at(self.next)?;
        self.next = node.rec().next;
        self.remaining = self.remaining.saturating_sub(1);
        Some(node)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for Children<'_> {}

impl<'r> SgNode<'r> for BladeNode<'r> {
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
        Cow::Borrowed(
            KIND_NAMES
                .get(usize::from(self.rec().kind))
                .copied()
                .unwrap_or(""),
        )
    }
    fn kind_id(&self) -> u16 {
        self.rec().kind
    }
    fn node_id(&self) -> usize {
        self.tree.base + self.index as usize
    }
    fn range(&self) -> Range<usize> {
        self.rec().start as usize..self.rec().end as usize
    }
    fn start_pos(&self) -> Position {
        self.tree.source.position(self.rec().start as usize)
    }
    fn end_pos(&self) -> Position {
        self.tree.source.position(self.rec().end as usize)
    }
    fn is_named(&self) -> bool {
        self.rec().kind < FIRST_ANONYMOUS
    }
    fn is_named_leaf(&self) -> bool {
        self.children().all(|child| !child.is_named())
    }
    fn is_leaf(&self) -> bool {
        self.rec().children == 0
    }
    fn field(&self, name: &str) -> Option<Self> {
        let id = BladeLang.field_to_id(name)?;
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

#[cfg(test)]
mod tests;
