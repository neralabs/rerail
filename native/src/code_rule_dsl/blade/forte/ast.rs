//! What Forte's AST API reads off the flat tree, for the properties the oracle records
//! (`forte_tree.php`'s `p`), ported method by method from:
//!
//! - `Ast/Document/Concerns/ManagesAst.php`: `createElementWrapper`, `extractTagName`,
//!   `collectTokenText`, `getTokenRangeContent`, `getNodeTokenContent`,
//!   `findOpeningTagEndPosition`, `getSourceSlice`.
//! - `Ast/Node.php`: `startOffset`, `endOffset`, `getDocumentContent`, `children`
//!   (`INTERNAL_KIND_SET`).
//! - `Ast/Elements/ElementNode.php`: `tagName`, `tagNameText`, `isVoid` (`VoidElements`);
//!   `Ast/Elements/Concerns/ManagesElementClosing.php`: `isSelfClosing`, `isPaired`,
//!   `getClosingNameIndex`; `Ast/Elements/ElementNameNode.php` + `Concerns/ManagesTagNameParts`:
//!   `__toString`/`name` (an opening name's token text, echoes of dynamic names included).
//! - `Components/ComponentManager.php`: `isComponent`, `getMatchingPrefix`, `parseComponentName`,
//!   `extractBracketContent` (default prefixes `x-`, `livewire:`, `flux:`);
//!   `Ast/Components/ComponentNode.php`: `getMetadata` (`ComponentMetadata`).
//! - `Ast/Elements/Attributes.php`: `ensureLoaded`, `isStandaloneAttributeKind`;
//!   `Ast/Elements/Attribute.php`: `type`, `rawName`, `nameText`, `valueText`, `quote`,
//!   `getNameNodeIndex`, `getValueNodeIndex`; `Ast/Elements/AttributeName.php`: `__toString`,
//!   `stripPrefix`; `Ast/Elements/AttributeType.php`.
//! - `Ast/DirectiveBlockNode.php`: `startDirective`, `endDirective`,
//!   `getIntermediateDirectives`; `Ast/DirectiveNode.php`: `role`, `isIntermediate`;
//!   `Ast/Concerns/HasDirectiveName.php`: `nameText`.
//! - `Ast/EchoNode.php`: `content`, `stripBraces`; `Ast/PhpBlockNode.php` +
//!   `Ast/Concerns/HasDelimitedContent.php`: `content`; `Ast/PhpTagNode.php`: `phpType`,
//!   `isShortEcho`, `content`.
//!
//! What the PHP returns as a substring of the template is a range of it here; what it builds is
//! a `Cow`. Forte's rewrite state (synthetic content and metadata) is empty for a parsed
//! document, so its lookups (`getSyntheticContent`, `getSyntheticMeta`) are left out, as are its
//! extension node kinds and token types (none registered). Child walks are bounded by the node
//! count, so a malformed link cycle ends instead of spinning.

use super::node::{FlatNode, NONE, NodeKind, StructureRole};
use super::token::Token;
#[cfg(test)]
use super::token::TokenType;
use std::borrow::Cow;
use std::ops::Range;

/// The class `Document` wraps an element in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ElementClass {
    Element,
    Component,
    Slot,
}

/// `ComponentMetadata`: the fields the oracle records.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the oracle's AST reads, which only the tests compare"
    )
)]
pub(crate) struct Component<'a> {
    /// `type`.
    pub(crate) kind: Cow<'a, [u8]>,
    pub(crate) prefix: Cow<'a, [u8]>,
    /// `componentName`.
    pub(crate) name: Cow<'a, [u8]>,
    pub(crate) slot_name: Option<Cow<'a, [u8]>>,
}

/// What `ElementNode` reads.
#[cfg(test)]
pub(crate) struct Element<'a> {
    pub(crate) class: ElementClass,
    /// `tagNameText()`.
    pub(crate) tag: Cow<'a, [u8]>,
    /// `isVoid()`.
    pub(crate) void: bool,
    /// `isSelfClosing()`.
    pub(crate) self_closing: bool,
    /// `isPaired()`.
    pub(crate) paired: bool,
    /// `ComponentNode::getMetadata()`, for components and slots.
    pub(crate) component: Option<Component<'a>>,
}

/// What `Attribute` reads.
#[cfg(test)]
pub(crate) struct Attribute<'a> {
    pub(crate) index: usize,
    /// `type()`.
    pub(crate) kind: &'static str,
    pub(crate) raw_name: Cow<'a, [u8]>,
    pub(crate) name: Cow<'a, [u8]>,
    pub(crate) value: Option<Cow<'a, [u8]>>,
    pub(crate) quote: Option<u8>,
}

/// What `DirectiveBlockNode` reads.
pub(crate) struct Block {
    pub(crate) start: Option<usize>,
    pub(crate) end: Option<usize>,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the oracle's AST reads, which only the tests compare"
        )
    )]
    pub(crate) mid: Vec<usize>,
}

/// `PhpTagNode::phpType()`.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PhpTagType {
    Php,
    Echo,
}

/// `ComponentManager`'s default prefixes, in its order.
const COMPONENT_PREFIXES: [&[u8]; 3] = [b"x-", b"livewire:", b"flux:"];

/// `VoidElements::$voidElements`.
#[cfg(test)]
const VOID_ELEMENTS: [&[u8]; 14] = [
    b"area", b"base", b"br", b"col", b"embed", b"hr", b"img", b"input", b"link", b"meta", b"param",
    b"source", b"track", b"wbr",
];

/// `AttributeType`.
mod attribute_type {
    #[cfg(test)]
    pub(super) const BOUND: &str = "bound";
    #[cfg(test)]
    pub(super) const ESCAPED: &str = "escaped";
    #[cfg(test)]
    pub(super) const SHORTHAND: &str = "shorthand";
    #[cfg(test)]
    pub(super) const STATIC: &str = "static";
    /// `Attribute::type()` of a Blade construct in attribute position.
    #[cfg(test)]
    pub(super) const STANDALONE: &str = "standalone";
}

/// A parsed template, read as Forte's AST API reads its `Document`.
pub(crate) struct Ast<'a> {
    source: &'a [u8],
    tokens: &'a [Token],
    nodes: &'a [FlatNode],
}

/// `substr($source, $offset, $length)` as a range of `source`, with PHP 8's clamping of negative
/// and out-of-range arguments.
fn php_substr(len: usize, offset: i64, length: Option<i64>) -> Range<usize> {
    let len_i = len as i64;
    let start = if offset < 0 {
        (len_i + offset).max(0)
    } else {
        offset.min(len_i)
    };
    let end = match length {
        None => len_i,
        Some(length) if length < 0 => len_i + length,
        Some(length) => start.saturating_add(length).min(len_i),
    };
    if end <= start {
        return start as usize..start as usize;
    }
    start as usize..end as usize
}

/// `str_starts_with` then strip: the rest of `range` after `prefix`, if `range` starts with it.
#[cfg(test)]
fn strip_start(source: &[u8], range: Range<usize>, prefix: &[u8]) -> Range<usize> {
    if source[range.clone()].starts_with(prefix) {
        range.start + prefix.len()..range.end
    } else {
        range
    }
}

/// `str_ends_with` then `substr(..., 0, -strlen($suffix))`.
#[cfg(test)]
fn strip_end(source: &[u8], range: Range<usize>, suffix: &[u8]) -> Range<usize> {
    if source[range.clone()].ends_with(suffix) {
        range.start..range.end - suffix.len()
    } else {
        range
    }
}

/// `ltrim($text, $chars)`.
#[cfg(test)]
fn ltrim_chars<'b>(text: &'b [u8], chars: &[u8]) -> &'b [u8] {
    let start = text
        .iter()
        .position(|b| !chars.contains(b))
        .unwrap_or(text.len());
    &text[start..]
}

/// `Str::startsWith($name, $prefixes)` over `ComponentManager::$prefixes`.
fn is_component(name: &[u8]) -> bool {
    COMPONENT_PREFIXES
        .iter()
        .any(|prefix| !prefix.is_empty() && name.starts_with(prefix))
}

/// `ComponentManager::getMatchingPrefix`.
fn get_matching_prefix(name: &[u8]) -> Option<&'static [u8]> {
    COMPONENT_PREFIXES
        .iter()
        .copied()
        .find(|prefix| name.starts_with(prefix))
}

/// `ComponentManager::parseComponentName`: the metadata fields the oracle records, and `isSlot`.
fn parse_component_name(name: Cow<'_, [u8]>) -> Option<(Component<'_>, bool)> {
    let prefix = get_matching_prefix(&name)?;
    let component_name: Cow<'_, [u8]> = match name {
        Cow::Borrowed(name) => Cow::Borrowed(&name[prefix.len()..]),
        Cow::Owned(name) => Cow::Owned(name[prefix.len()..].to_vec()),
    };
    // `rtrim($prefix, ':-')`, then `x` reads as `blade`.
    let end = prefix
        .iter()
        .rposition(|b| *b != b':' && *b != b'-')
        .map_or(0, |at| at + 1);
    let kind: &'static [u8] = if &prefix[..end] == b"x" {
        b"blade"
    } else {
        &prefix[..end]
    };
    let is_slot = component_name.starts_with(b"slot");
    let mut slot_name = None;
    if is_slot {
        if component_name.starts_with(b"slot:") {
            slot_name = Some(sub_cow(&component_name, 5..component_name.len()));
        } else if component_name.starts_with(b"slot[") {
            slot_name = extract_bracket_content(&component_name, 5)
                .map(|range| sub_cow(&component_name, range));
        }
        // `isDynamicSlot`, `isMultiValueSlot` and `baseSlotName` are not recorded.
    }
    Some((
        Component {
            kind: Cow::Borrowed(kind),
            prefix: Cow::Borrowed(prefix),
            name: component_name,
            slot_name,
        },
        is_slot,
    ))
}

/// A part of a `Cow`, owned as the whole is.
fn sub_cow<'b>(whole: &Cow<'b, [u8]>, range: Range<usize>) -> Cow<'b, [u8]> {
    match whole {
        Cow::Borrowed(whole) => Cow::Borrowed(&whole[range]),
        Cow::Owned(whole) => Cow::Owned(whole[range].to_vec()),
    }
}

/// `ComponentManager::extractBracketContent`.
fn extract_bracket_content(component_name: &[u8], start: usize) -> Option<Range<usize>> {
    let mut depth = 0usize;
    let mut end = start;
    for (i, char) in component_name.iter().enumerate().skip(start) {
        if *char == b'[' {
            depth += 1;
        } else if *char == b']' {
            if depth == 0 {
                end = i;
                break;
            }
            depth -= 1;
        }
    }
    (end > start).then_some(start..end)
}

/// `Node::isInternalKind` (`INTERNAL_KIND_SET`).
fn is_internal_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::ElementName
            | NodeKind::ClosingElementName
            | NodeKind::Attribute
            | NodeKind::JsxAttribute
            | NodeKind::AttributeName
            | NodeKind::AttributeValue
            | NodeKind::AttributeWhitespace
    )
}

/// `Attributes::isStandaloneAttributeKind`.
#[cfg(test)]
fn is_standalone_attribute_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Echo
            | NodeKind::RawEcho
            | NodeKind::TripleEcho
            | NodeKind::Directive
            | NodeKind::DirectiveBlock
            | NodeKind::PhpTag
            | NodeKind::PhpBlock
            | NodeKind::JsxAttribute
    )
}

impl<'a> Ast<'a> {
    pub(crate) fn new(source: &'a [u8], tokens: &'a [Token], nodes: &'a [FlatNode]) -> Self {
        Self {
            source,
            tokens,
            nodes,
        }
    }

    #[cfg(test)]
    pub(crate) fn source(&self) -> &'a [u8] {
        self.source
    }

    #[cfg(test)]
    pub(crate) fn nodes(&self) -> &'a [FlatNode] {
        self.nodes
    }

    fn node(&self, index: i32) -> Option<&'a FlatNode> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.nodes.get(index))
    }

    fn token(&self, index: i64) -> Option<&'a Token> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.tokens.get(index))
    }

    /// A node's children by `firstChild`/`nextSibling`, every kind, bounded by the node count.
    fn child_links(&self, index: usize) -> impl Iterator<Item = (usize, &'a FlatNode)> + 'a {
        let nodes = self.nodes;
        let mut next = nodes.get(index).map_or(NONE, |node| node.first_child);
        let mut left = nodes.len();
        std::iter::from_fn(move || {
            if left == 0 {
                return None;
            }
            left -= 1;
            let at = usize::try_from(next).ok()?;
            let node = nodes.get(at)?;
            next = node.next_sibling;
            Some((at, node))
        })
    }

    /// `Node::children` (skipping `INTERNAL_KIND_SET`), as node indices.
    fn children(&self, index: usize) -> impl Iterator<Item = (usize, &'a FlatNode)> + 'a {
        self.child_links(index)
            .filter(|(_, node)| !is_internal_kind(node.kind))
    }

    /// `getSourceSlice($start, $end)`: `substr($source, $start, $end - $start)`.
    #[cfg(test)]
    fn get_source_slice(&self, start: i64, end: i64) -> Range<usize> {
        php_substr(self.source.len(), start, Some(end - start))
    }

    /// `getTokenRangeContent`.
    #[cfg(test)]
    fn get_token_range_content(&self, token_start: i32, token_count: i32) -> Range<usize> {
        if token_count == 0 || token_start < 0 || token_start as usize >= self.tokens.len() {
            return 0..0;
        }
        let end_index =
            (i64::from(token_start) + i64::from(token_count) - 1).min(self.tokens.len() as i64 - 1);
        let (Some(start_token), Some(end_token)) =
            (self.token(i64::from(token_start)), self.token(end_index))
        else {
            return 0..0;
        };
        self.get_source_slice(i64::from(start_token.start), i64::from(end_token.end))
    }

    /// `getNodeTokenContent`.
    #[cfg(test)]
    fn get_node_token_content(&self, index: usize) -> Range<usize> {
        match self.nodes.get(index) {
            Some(flat) => self.get_token_range_content(flat.token_start, flat.token_count),
            None => 0..0,
        }
    }

    /// `Node::startOffset` (-1 for a node without tokens; `getToken` throws past the end, which
    /// no tree Forte builds reaches).
    #[cfg(test)]
    fn start_offset(&self, flat: &FlatNode) -> i64 {
        if flat.token_start < 0 {
            return -1;
        }
        self.token(i64::from(flat.token_start))
            .map_or(-1, |token| i64::from(token.start))
    }

    /// `Node::endOffset`.
    #[cfg(test)]
    fn end_offset(&self, flat: &FlatNode) -> i64 {
        if flat.token_start < 0 || flat.token_count <= 0 {
            return -1;
        }
        let token_end = i64::from(flat.token_start) + i64::from(flat.token_count) - 1;
        self.token(token_end)
            .map_or(-1, |token| i64::from(token.end))
    }

    /// `Node::getDocumentContent`.
    #[cfg(test)]
    fn get_document_content(&self, index: usize) -> Range<usize> {
        match self.nodes.get(index) {
            Some(flat) => self.get_source_slice(self.start_offset(flat), self.end_offset(flat)),
            None => 0..0,
        }
    }

    /// The text of a range of tokens, concatenated (`collectTokenText`): borrowed while the
    /// tokens are adjacent.
    fn collect_token_text(&self, start: i64, count: i64) -> Cow<'a, [u8]> {
        let mut joined: Option<Range<usize>> = None;
        let mut owned: Option<Vec<u8>> = None;
        let mut i = 0;
        while i < count && start + i < self.tokens.len() as i64 {
            let Some(token) = self.token(start + i) else {
                break;
            };
            let text = php_substr(
                self.source.len(),
                i64::from(token.start),
                Some(i64::from(token.end) - i64::from(token.start)),
            );
            i += 1;
            if text.is_empty() {
                continue;
            }
            if let Some(owned) = owned.as_mut() {
                owned.extend_from_slice(&self.source[text]);
            } else if let Some(range) = joined.as_mut() {
                if range.end == text.start {
                    range.end = text.end;
                } else {
                    let mut concatenated = self.source[range.clone()].to_vec();
                    concatenated.extend_from_slice(&self.source[text]);
                    owned = Some(concatenated);
                }
            } else {
                joined = Some(text);
            }
        }
        match (owned, joined) {
            (Some(owned), _) => Cow::Owned(owned),
            (None, Some(range)) => Cow::Borrowed(&self.source[range]),
            (None, None) => Cow::Borrowed(&[]),
        }
    }

    /// `ManagesAst::extractTagName`: the name `createElementWrapper` classifies an element by.
    fn extract_tag_name(&self, flat: &FlatNode) -> Cow<'a, [u8]> {
        let tag_name_count = flat.data;
        if tag_name_count > 0 {
            return self
                .collect_token_text(i64::from(flat.token_start) + 1, i64::from(tag_name_count));
        }
        if let Some(child) = self.node(flat.first_child)
            && child.kind == NodeKind::ElementName
        {
            return self
                .collect_token_text(i64::from(child.token_start), i64::from(child.token_count));
        }
        let token_start = i64::from(flat.token_start) + 1;
        if token_start < self.tokens.len() as i64
            && let Some(token) = self.token(token_start)
        {
            let range = php_substr(
                self.source.len(),
                i64::from(token.start),
                Some(i64::from(token.end) - i64::from(token.start)),
            );
            return Cow::Borrowed(&self.source[range]);
        }
        Cow::Borrowed(&[])
    }

    /// `ManagesAst::createElementWrapper`: the class an element is wrapped in.
    fn create_element_wrapper(&self, flat: &FlatNode) -> ElementClass {
        let tag_name = self.extract_tag_name(flat);
        if is_component(&tag_name) {
            if let Some((_, true)) = parse_component_name(tag_name) {
                return ElementClass::Slot;
            }
            return ElementClass::Component;
        }
        ElementClass::Element
    }

    /// `ElementNode::tagNameText` (`tagName()`'s `ElementNameNode::name()`: its token text). An
    /// element without an `ElementName` child, where Forte throws, reads as the empty name.
    #[cfg(test)]
    fn tag_name_text(&self, index: usize) -> Cow<'a, [u8]> {
        match self
            .child_links(index)
            .find(|(_, child)| child.kind == NodeKind::ElementName)
        {
            Some((child, _)) => Cow::Borrowed(&self.source[self.get_node_token_content(child)]),
            None => Cow::Borrowed(&[]),
        }
    }

    /// `ManagesElementClosing::getClosingNameIndex`.
    #[cfg(test)]
    fn get_closing_name_index(&self, flat: &FlatNode) -> Option<usize> {
        if flat.last_child == NONE {
            return None;
        }
        let last = self.node(flat.last_child)?;
        (last.kind == NodeKind::ClosingElementName).then_some(flat.last_child as usize)
    }

    /// The class `Document::getNode` wraps an element node in.
    pub(crate) fn element_class(&self, index: usize) -> ElementClass {
        self.nodes.get(index).map_or(ElementClass::Element, |flat| {
            self.create_element_wrapper(flat)
        })
    }

    /// What `ElementNode` (or `ComponentNode`/`SlotNode`) reads for an element node.
    #[cfg(test)]
    pub(crate) fn element(&self, index: usize) -> Element<'a> {
        let Some(flat) = self.nodes.get(index) else {
            return Element {
                class: ElementClass::Element,
                tag: Cow::Borrowed(&[]),
                void: false,
                self_closing: false,
                paired: false,
                component: None,
            };
        };
        let class = self.create_element_wrapper(flat);
        let tag = self.tag_name_text(index);
        let lower = tag.to_ascii_lowercase();
        let void = VOID_ELEMENTS.contains(&lower.as_slice());
        let component = match class {
            ElementClass::Element => None,
            // `ComponentNode::getMetadata`.
            ElementClass::Component | ElementClass::Slot => {
                Some(match parse_component_name(tag.clone()) {
                    Some((metadata, _)) => metadata,
                    None => Component {
                        kind: Cow::Borrowed(b"blade"),
                        prefix: Cow::Borrowed(b"x-"),
                        name: tag.clone(),
                        slot_name: None,
                    },
                })
            }
        };
        Element {
            class,
            tag,
            void,
            self_closing: flat.data == 1,
            paired: self.get_closing_name_index(flat).is_some(),
            component,
        }
    }

    /// `ManagesAst::findOpeningTagEndPosition`.
    #[cfg(test)]
    fn find_opening_tag_end_position(&self, element_index: usize) -> i64 {
        let Some(element) = self.nodes.get(element_index) else {
            return self.source.len() as i64;
        };
        if element.token_start < 0 {
            return -1;
        }
        let start = element.token_start as usize;
        let end = (start as i64 + i64::from(element.token_count)).max(0) as usize;
        let mut depth = 0i64;
        for token in self
            .tokens
            .get(start..end.min(self.tokens.len()).max(start))
            .unwrap_or(&[])
        {
            use TokenType::*;
            match token.kind {
                EchoStart | RawEchoStart | TripleEchoStart | PhpTagStart | PhpBlockStart => {
                    depth += 1
                }
                EchoEnd | RawEchoEnd | TripleEchoEnd | PhpTagEnd | PhpBlockEnd => depth -= 1,
                _ => {}
            }
            if depth == 0 && (token.kind == GreaterThan || token.kind == DeclEnd) {
                return i64::from(token.end);
            }
            if depth == 0 && token.kind == SyntheticClose {
                return i64::from(token.start);
            }
        }
        self.source.len() as i64
    }

    /// `ElementNode::attributes()->all()` (`Attributes::ensureLoaded`), each attribute read as
    /// the oracle reads it.
    #[cfg(test)]
    pub(crate) fn attributes(&self, element: usize) -> Vec<Attribute<'a>> {
        let mut items = Vec::new();
        let opening_tag_end_pos = self.find_opening_tag_end_position(element);
        for (child_index, child) in self.child_links(element) {
            match child.kind {
                NodeKind::ElementName => continue,
                NodeKind::ClosingElementName => break,
                _ => {}
            }
            if child.token_start < 0 {
                break;
            }
            match self.token(i64::from(child.token_start)) {
                Some(token) if i64::from(token.start) >= opening_tag_end_pos => break,
                Some(_) => {}
                // PHP reads `null['start']` as null, and `null >= $pos` compares as booleans.
                None if opening_tag_end_pos == 0 => break,
                None => {}
            }
            // `AttributeWhitespace` only sets the attribute's leading whitespace, which the
            // oracle does not read; extension attribute kinds are left out (none registered).
            if child.kind == NodeKind::Attribute {
                items.push(self.attribute(child_index, false));
            } else if is_standalone_attribute_kind(child.kind) {
                items.push(self.attribute(child_index, true));
            }
        }
        items
    }

    /// An `Attribute`'s reads: `type`, `rawName`, `nameText`, `valueText`, `quote`.
    #[cfg(test)]
    fn attribute(&self, index: usize, is_blade_construct: bool) -> Attribute<'a> {
        let name_idx = self.get_name_node_index(index);
        let kind = self.attribute_type(name_idx, is_blade_construct);
        // `rawName` and `nameText` are empty for a Blade construct and for a missing name.
        let raw_name: &'a [u8] = match name_idx {
            Some(name_idx) if !is_blade_construct => {
                &self.source[self.get_node_token_content(name_idx)]
            }
            _ => &[],
        };
        let value_idx = name_idx.and_then(|name_idx| self.get_value_node_index(name_idx));
        Attribute {
            index,
            kind,
            raw_name: Cow::Borrowed(raw_name),
            // `AttributeName::__toString`: the raw name without its prefix.
            name: Cow::Borrowed(strip_prefix(kind, raw_name)),
            value: value_idx
                .map(|value_idx| Cow::Borrowed(&self.source[self.value_text(value_idx)])),
            quote: value_idx.and_then(|value_idx| self.quote(value_idx)),
        }
    }

    /// `Attribute::getNameNodeIndex`: the first child, when it is an `AttributeName`.
    #[cfg(test)]
    fn get_name_node_index(&self, index: usize) -> Option<usize> {
        let flat = self.nodes.get(index)?;
        let child = self.node(flat.first_child)?;
        (child.kind == NodeKind::AttributeName).then_some(flat.first_child as usize)
    }

    /// `Attribute::getValueNodeIndex`: the name's next sibling, when it is an `AttributeValue`.
    #[cfg(test)]
    fn get_value_node_index(&self, name_idx: usize) -> Option<usize> {
        let name = self.nodes.get(name_idx)?;
        let value = self.node(name.next_sibling)?;
        (value.kind == NodeKind::AttributeValue).then_some(name.next_sibling as usize)
    }

    /// `Attribute::type`.
    #[cfg(test)]
    fn attribute_type(&self, name_idx: Option<usize>, is_blade_construct: bool) -> &'static str {
        if is_blade_construct {
            return attribute_type::STANDALONE;
        }
        let Some(name) = name_idx.and_then(|name_idx| self.nodes.get(name_idx)) else {
            return attribute_type::STATIC;
        };
        if name.token_count == 0 {
            return attribute_type::STATIC;
        }
        // Extension token types are left out (none registered).
        match self
            .token(i64::from(name.token_start))
            .map(|token| token.kind)
        {
            Some(TokenType::BoundAttribute) => attribute_type::BOUND,
            Some(TokenType::EscapedAttribute) => attribute_type::ESCAPED,
            Some(TokenType::ShorthandAttribute) => attribute_type::SHORTHAND,
            _ => attribute_type::STATIC,
        }
    }

    /// `Attribute::valueText` of an attribute with a value node: its tokens without the quotes.
    #[cfg(test)]
    fn value_text(&self, value_idx: usize) -> Range<usize> {
        let Some(value) = self.nodes.get(value_idx) else {
            return 0..0;
        };
        let token_total = self.tokens.len() as i64;
        let mut token_start = i64::from(value.token_start);
        let mut token_end = token_start + i64::from(value.token_count);
        if token_start < 0 || token_start >= token_total {
            return 0..0;
        }
        if token_start < token_end
            && self
                .token(token_start)
                .is_some_and(|token| token.kind == TokenType::Quote)
        {
            token_start += 1;
        }
        if token_end > token_start
            && token_end - 1 < token_total
            && self
                .token(token_end - 1)
                .is_some_and(|token| token.kind == TokenType::Quote)
        {
            token_end -= 1;
        }
        if token_start >= token_end || token_start >= token_total || token_end > token_total {
            return 0..0;
        }
        let (Some(start_token), Some(end_token)) =
            (self.token(token_start), self.token(token_end - 1))
        else {
            return 0..0;
        };
        php_substr(
            self.source.len(),
            i64::from(start_token.start),
            Some(i64::from(end_token.end) - i64::from(start_token.start)),
        )
    }

    /// `Attribute::quote` of an attribute with a value node: the opening quote's byte.
    #[cfg(test)]
    fn quote(&self, value_idx: usize) -> Option<u8> {
        let value = self.nodes.get(value_idx)?;
        if value.token_count == 0 {
            return None;
        }
        let first = self.token(i64::from(value.token_start))?;
        if first.kind != TokenType::Quote {
            return None;
        }
        self.source.get(first.start as usize).copied()
    }

    /// `HasDirectiveName::nameText`.
    fn name_text(flat: &FlatNode) -> &[u8] {
        flat.directive.as_ref().map_or(&[][..], |info| &info.name)
    }

    /// `DirectiveBlockNode::startDirective`, `endDirective` and `getIntermediateDirectives`.
    pub(crate) fn block(&self, index: usize) -> Block {
        let Some(flat) = self.nodes.get(index) else {
            return Block {
                start: None,
                end: None,
                mid: Vec::new(),
            };
        };
        let name = Self::name_text(flat);
        let start = self
            .children(index)
            .find(|(_, child)| child.kind == NodeKind::Directive && Self::name_text(child) == name)
            .map(|(at, _)| at);
        let mut count = 0usize;
        let mut last = None;
        let mut mid = Vec::new();
        for (at, child) in self.children(index) {
            count += 1;
            last = Some((at, child));
            // `DirectiveNode::isIntermediate`: `role` defaults to `None`.
            if child.kind == NodeKind::Directive
                && child.directive.as_ref().and_then(|info| info.role)
                    == Some(StructureRole::Intermediate)
            {
                mid.push(at);
            }
        }
        let end = match last {
            Some((at, child)) if count >= 2 && child.kind == NodeKind::Directive => Some(at),
            _ => None,
        };
        Block { start, end, mid }
    }

    /// `EchoNode::content()`, as a range of the template.
    #[cfg(test)]
    pub(crate) fn echo_content(&self, index: usize) -> Range<usize> {
        let raw = self.get_document_content(index);
        // `stripBraces`.
        let (open, close): (&[u8], &[u8]) = match self.nodes.get(index).map(|flat| flat.kind) {
            Some(NodeKind::Echo) => (b"{{", b"}}"),
            Some(NodeKind::RawEcho) => (b"{!!", b"!!}"),
            Some(NodeKind::TripleEcho) => (b"{{{", b"}}}"),
            _ => return raw,
        };
        let content = strip_start(self.source, raw, open);
        strip_end(self.source, content, close)
    }

    /// `PhpBlockNode::content()` (`HasDelimitedContent`), as a range of the template.
    #[cfg(test)]
    pub(crate) fn php_block_content(&self, index: usize) -> Range<usize> {
        let raw = self.get_document_content(index);
        let raw = strip_start(self.source, raw, b"@php");
        strip_end(self.source, raw, b"@endphp")
    }

    /// `PhpTagNode::phpType()` and `content()`, as a range of the template.
    #[cfg(test)]
    pub(crate) fn php_tag(&self, index: usize) -> (PhpTagType, Range<usize>) {
        let raw = self.get_document_content(index);
        let text = &self.source[raw.clone()];
        let kind = if text.starts_with(b"<?=") {
            PhpTagType::Echo
        } else {
            PhpTagType::Php
        };
        let content = if text.starts_with(b"<?php") {
            raw.start + 5..raw.end
        } else if text.starts_with(b"<?=") {
            raw.start + 3..raw.end
        } else if text.starts_with(b"<?") {
            raw.start + 2..raw.end
        } else {
            raw
        };
        (kind, strip_end(self.source, content, b"?>"))
    }
}

/// `AttributeName::stripPrefix`.
#[cfg(test)]
fn strip_prefix<'b>(kind: &str, content: &'b [u8]) -> &'b [u8] {
    match kind {
        attribute_type::BOUND => ltrim_chars(content, b":"),
        attribute_type::ESCAPED if content.starts_with(b"::") => &content[2..],
        attribute_type::ESCAPED => ltrim_chars(content, b":"),
        attribute_type::SHORTHAND if content.starts_with(b":$") => &content[2..],
        attribute_type::SHORTHAND => ltrim_chars(content, b":$"),
        _ => content,
    }
}
