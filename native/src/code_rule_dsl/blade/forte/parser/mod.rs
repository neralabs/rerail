//! Forte's tree builder (`Parser\TreeBuilder`) and the traits it uses (`Parser\Concerns\*`:
//! one module each), `Parser\ConstructScanner`, `Parser\OptionalTags\*` and
//! `Ast\Elements\VoidElements`, ported method by method. The builder walks the tokens once and
//! appends flat nodes in Forte's order, linking each to its parent, its parent's last child and
//! its previous sibling as Forte does; the indices, links, token spans, `data`, `genericOffset`
//! and directive extras it writes are Forte's.
//!
//! Left out: Forte's extension hooks (`tryExtensions`, attribute extensions, `TreeContext`) and
//! the API only they call (`collectUntil`, `buildFromTokenRange`, `detachedParent`, node
//! metadata, `createExtensionNode`, `addChildNode`, `linkAs*`, `pushOpenElement`): with no
//! extension registered, as `Document::parse` builds, none of it runs.
//!
//! Bounds: every token processed and every iteration of a loop that scans tokens or directives
//! spends a step from the budget. Nothing recurses: where Forte's `createConditionalComment`
//! processes the comment's contents by calling `processToken` in a nested loop, the port keeps
//! the open conditional comments on a stack (`conditional_comments`) and `build_tree`'s one
//! loop runs each one's contents, then its epilogue, as the nested loops would.

mod comments;
mod conditional_pairing;
mod construct_scanner;
mod directives;
mod echoes;
mod element_attributes;
mod element_auto_closing;
mod element_names;
mod elements;
mod markup;
mod optional_tags;
mod php;
mod switch;
mod void_elements;

use super::directives::Directives;
use super::directives::token_index::{DirectiveTokenIndex, NameSet};
use super::node::{DirectiveInfo, FlatNode, NONE, NodeKind, StructureRole};
use super::token::{Token, TokenType};
use super::{Budget, DepthLimits, ParseError};
use comments::ConditionalCommentFrame;
use std::borrow::Cow;
use std::collections::HashMap;

/// `(new TreeBuilder($tokens, $source, $directives))->setDepthLimits(...)->buildCompact()`:
/// the flat nodes.
pub(crate) fn build(
    tokens: &[Token],
    source: &[u8],
    directives: &Directives,
    limits: DepthLimits,
    budget: &mut Budget,
) -> Result<Vec<FlatNode>, ParseError> {
    let mut builder = TreeBuilder::new(tokens, source, directives, limits, budget);
    builder.build_tree()?;
    Ok(builder.nodes)
}

/// A list of names a directive frame holds: the registry's, one name, or names the builder made.
enum Names<'a> {
    Registry(&'a [Box<[u8]>]),
    One(Cow<'a, [u8]>),
    Many(Vec<Box<[u8]>>),
}

impl Names<'_> {
    fn len(&self) -> usize {
        match self {
            Names::Registry(names) => names.len(),
            Names::One(_) => 1,
            Names::Many(names) => names.len(),
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get(&self, i: usize) -> Option<&[u8]> {
        match self {
            Names::Registry(names) => names.get(i).map(|name| &**name),
            Names::One(name) => (i == 0).then_some(&**name),
            Names::Many(names) => names.get(i).map(|name| &**name),
        }
    }

    /// `in_array($name, $names, true)`.
    fn contains(&self, name: &[u8]) -> bool {
        (0..self.len()).any(|i| self.get(i) == Some(name))
    }
}

impl NameSet for Names<'_> {
    fn for_each_name(&self, f: &mut dyn FnMut(&[u8])) {
        for i in 0..self.len() {
            if let Some(name) = self.get(i) {
                f(name);
            }
        }
    }
}

/// An open paired directive (`$openDirectives`' frames). Forte's frame also holds the name,
/// which nothing reads.
struct DirectiveFrame<'a> {
    block_idx: usize,
    start_directive_idx: usize,
    element_stack_base: usize,
    terminators: Names<'a>,
    branches: Names<'a>,
    openers: Names<'a>,
}

/// An open condition (`$openConditions`' frames).
struct ConditionFrame<'a> {
    block_idx: usize,
    current_branch_idx: usize,
    name: Cow<'a, [u8]>,
    element_stack_base: usize,
}

/// An open `@switch` (`$openSwitches`' frames).
struct SwitchFrame<'a> {
    block_idx: usize,
    switch_directive_idx: usize,
    current_case_idx: Option<usize>,
    name: Cow<'a, [u8]>,
    element_stack_base: usize,
}

/// `$discoveredDirectiveFamilyCache`'s entries.
#[derive(Clone)]
struct Family {
    else_name: Box<[u8]>,
    end_name: Box<[u8]>,
    openers: Vec<Box<[u8]>>,
    has_advisory_condition: bool,
}

/// `$tagNames`: the lowercase tag name of each element node that stayed open, by node index.
#[derive(Default)]
struct TagNames {
    spans: Vec<Option<(u32, u32)>>,
    bytes: Vec<u8>,
}

impl TagNames {
    fn get(&self, idx: usize) -> Option<&[u8]> {
        let (start, end) = (*self.spans.get(idx)?)?;
        Some(&self.bytes[start as usize..end as usize])
    }

    fn set(&mut self, idx: usize, name: &[u8]) {
        if self.spans.len() <= idx {
            self.spans.resize(idx + 1, None);
        }
        let start = self.bytes.len() as u32;
        self.bytes.extend_from_slice(name);
        self.spans[idx] = Some((start, self.bytes.len() as u32));
    }
}

/// `TreeBuilder`.
pub(crate) struct TreeBuilder<'a, 'b> {
    nodes: Vec<FlatNode>,
    tokens: &'a [Token],
    source: &'a [u8],
    directives: &'a Directives,
    budget: &'b mut Budget,
    open_elements: Vec<usize>,
    open_directives: Vec<DirectiveFrame<'a>>,
    open_conditions: Vec<ConditionFrame<'a>>,
    open_switches: Vec<SwitchFrame<'a>>,
    pos: usize,
    tag_names: TagNames,
    /// `$tagNameStacks`: per lowercase tag name, the element stack positions of its open
    /// elements. An emptied stack stays (Forte unsets it), which every reader treats alike.
    tag_name_stacks: HashMap<Box<[u8]>, Vec<usize>>,
    directive_index: Option<DirectiveTokenIndex>,
    attribute_region_end: Option<usize>,
    discovered_directive_family_cache: HashMap<Box<[u8]>, Option<Family>>,
    token_total: usize,
    max_element_depth: usize,
    max_directive_depth: usize,
    max_condition_depth: usize,
    /// The conditional comments whose contents are being processed, innermost last
    /// (`createConditionalComment`'s nested loops).
    conditional_comments: Vec<ConditionalCommentFrame>,
    /// A buffer for tag names.
    scratch: Vec<u8>,
}

impl<'a, 'b> TreeBuilder<'a, 'b> {
    /// `__construct` and `setDepthLimits`.
    fn new(
        tokens: &'a [Token],
        source: &'a [u8],
        directives: &'a Directives,
        limits: DepthLimits,
        budget: &'b mut Budget,
    ) -> Self {
        Self {
            nodes: Vec::with_capacity(tokens.len() / 2 + 1),
            tokens,
            source,
            directives,
            budget,
            open_elements: Vec::new(),
            open_directives: Vec::new(),
            open_conditions: Vec::new(),
            open_switches: Vec::new(),
            pos: 0,
            tag_names: TagNames::default(),
            tag_name_stacks: HashMap::new(),
            directive_index: None,
            attribute_region_end: None,
            discovered_directive_family_cache: HashMap::new(),
            token_total: tokens.len(),
            max_element_depth: limits.elements.max(1),
            max_directive_depth: limits.directives.max(1),
            max_condition_depth: limits.conditions.max(1),
            conditional_comments: Vec::new(),
            scratch: Vec::new(),
        }
    }

    /// Spends one step.
    #[inline]
    fn spend(&mut self) -> Result<(), ParseError> {
        self.budget.spend(1)?;
        Ok(())
    }

    /// The type of the token at `idx` (`$this->tokens[$idx]->type`), `None` past the end.
    #[inline]
    fn kind_at(&self, idx: usize) -> Option<TokenType> {
        self.tokens.get(idx).map(|token| token.kind)
    }

    /// `checkElementDepth`.
    fn check_element_depth(&self) -> Result<(), ParseError> {
        if self.open_elements.len() >= self.max_element_depth {
            return Err(ParseError::Depth(format!(
                "Maximum element nesting depth ({}) exceeded.",
                self.max_element_depth
            )));
        }
        Ok(())
    }

    /// `checkDirectiveDepth`.
    fn check_directive_depth(&self) -> Result<(), ParseError> {
        if self.open_directives.len() >= self.max_directive_depth {
            return Err(ParseError::Depth(format!(
                "Maximum directive nesting depth ({}) exceeded.",
                self.max_directive_depth
            )));
        }
        Ok(())
    }

    /// `checkConditionDepth`.
    fn check_condition_depth(&self) -> Result<(), ParseError> {
        if self.open_conditions.len() >= self.max_condition_depth {
            return Err(ParseError::Depth(format!(
                "Maximum condition nesting depth ({}) exceeded.",
                self.max_condition_depth
            )));
        }
        Ok(())
    }

    /// `buildTree`, with the loops of `createConditionalComment` folded in: a conditional
    /// comment's contents run up to its end token, then its epilogue runs.
    fn build_tree(&mut self) -> Result<(), ParseError> {
        self.nodes.push(FlatNode::new(NodeKind::Root, NONE, 0, 0));
        self.open_elements.push(0);
        loop {
            let limit = self
                .conditional_comments
                .last()
                .map_or(self.token_total, |frame| frame.end_pos);
            if self.pos < limit {
                self.process_token()?;
                continue;
            }
            match self.conditional_comments.pop() {
                Some(frame) => self.finish_conditional_comment(frame),
                None => break,
            }
        }
        self.close_remaining_directives();
        self.close_remaining_elements();
        Ok(())
    }

    /// `processToken`. (Forte first offers the token to its extensions; none are registered.)
    fn process_token(&mut self) -> Result<(), ParseError> {
        self.spend()?;
        let Some(token_type) = self.kind_at(self.pos) else {
            self.pos += 1;
            return Ok(());
        };
        use TokenType::*;
        match token_type {
            LessThan => self.process_element_start()?,
            Text | TagName => self.process_text(),
            Whitespace => self.process_whitespace(),
            AtSign => self.process_at_sign(),
            EchoStart => self.process_echo()?,
            RawEchoStart => self.process_raw_echo()?,
            TripleEchoStart => self.process_triple_echo()?,
            Directive => self.process_directive()?,
            VerbatimStart => self.create_verbatim(self.pos)?,
            BladeCommentStart => self.create_blade_comment(self.pos)?,
            CommentStart => self.create_html_comment(self.pos)?,
            BogusComment => self.create_bogus_comment(self.pos),
            ConditionalCommentStart => self.create_conditional_comment(self.pos)?,
            CdataStart => self.create_cdata(self.pos)?,
            DeclStart => self.create_decl(self.pos)?,
            PiStart => self.create_processing_instruction(self.pos)?,
            DoctypeStart => self.create_doctype(self.pos)?,
            PhpBlockStart => self.create_php_block(self.pos)?,
            PhpTagStart => self.create_php_tag(self.pos)?,
            PhpBlockEnd => self.process_orphan_php_block_end(),
            PhpBlock
            | PhpContent
            | PhpTagEnd
            | SyntheticClose
            | EchoEnd
            | EchoContent
            | RawEchoEnd
            | TripleEchoEnd
            | DirectiveArgs
            | VerbatimEnd
            | CommentEnd
            | BladeCommentEnd
            | Doctype
            | DoctypeEnd
            | CdataEnd
            | PiEnd
            | DeclEnd
            | ConditionalCommentEnd
            | GreaterThan
            | TsxGenericType
            | JsxShorthandAttribute
            | JsxAttributeValue
            | AttributeName
            | AttributeValue
            | BoundAttribute
            | EscapedAttribute
            | ShorthandAttribute
            | Slash
            | Equals
            | Quote => self.emit_single_token_text(),
        }
        Ok(())
    }

    /// `createNode`.
    fn create_node(
        kind: NodeKind,
        parent: usize,
        token_start: usize,
        token_count: usize,
    ) -> FlatNode {
        FlatNode::new(kind, parent as i32, token_start as i32, token_count as i32)
    }

    /// `createNode` with the directive extras `name`, `args` (and `role` when given).
    fn create_directive_node(
        kind: NodeKind,
        parent: usize,
        token_start: usize,
        token_count: usize,
        name: &[u8],
        args: Option<&[u8]>,
        role: Option<StructureRole>,
    ) -> FlatNode {
        let mut node = Self::create_node(kind, parent, token_start, token_count);
        node.directive = Some(Box::new(DirectiveInfo {
            name: name.into(),
            args: args.map(Into::into),
            role,
        }));
        node
    }

    /// `createBlockNode`: a node from `start_pos` through the first `end_token_type` token.
    fn create_block_node(
        &mut self,
        start_pos: usize,
        end_token_type: TokenType,
        node_kind: NodeKind,
    ) -> Result<(), ParseError> {
        let mut end_pos = self.pos;
        let mut has_closing = false;
        while end_pos < self.token_total {
            self.spend()?;
            if self.tokens[end_pos].kind == end_token_type {
                has_closing = true;
                end_pos += 1;
                break;
            }
            end_pos += 1;
        }
        let mut node =
            Self::create_node(node_kind, 0, start_pos, end_pos.saturating_sub(start_pos));
        node.data = has_closing as i32;
        self.add_child(node);
        self.pos = end_pos;
        Ok(())
    }

    /// `addChild`: links the node as the current parent's last child.
    fn add_child(&mut self, mut node: FlatNode) -> usize {
        self.implicitly_close_void_elements();
        let parent_idx = self.get_current_parent();
        node.parent = parent_idx as i32;
        let node_idx = self.nodes.len();
        self.nodes.push(node);
        let first_child = self.nodes[parent_idx].first_child;
        if first_child == NONE {
            self.nodes[parent_idx].first_child = node_idx as i32;
        } else {
            let last_child_idx = self.nodes[parent_idx].last_child;
            if let Some(last) = self.node_mut(last_child_idx) {
                last.next_sibling = node_idx as i32;
            }
        }
        self.nodes[parent_idx].last_child = node_idx as i32;
        node_idx
    }

    /// The node at a link, if the link points at one.
    fn node_mut(&mut self, idx: i32) -> Option<&mut FlatNode> {
        usize::try_from(idx)
            .ok()
            .and_then(|idx| self.nodes.get_mut(idx))
    }

    /// `getCurrentParent`: the open element on top, else the root.
    fn get_current_parent(&self) -> usize {
        self.open_elements.last().copied().unwrap_or(0)
    }

    /// `implicitlyCloseVoidElements`: pops void elements off the top of the stack.
    fn implicitly_close_void_elements(&mut self) {
        while self.open_elements.len() > 1 {
            let Some(&top_idx) = self.open_elements.last() else {
                break;
            };
            let Some(tag_name) = self.tag_names.get(top_idx) else {
                break;
            };
            if !void_elements::is_void_element(tag_name) {
                break;
            }
            self.open_elements.pop();
            if let Some(stack) = self.tag_name_stacks.get_mut(tag_name) {
                stack.pop();
            }
        }
    }

    /// `processText`: a text token, merged into the parent's last child when that is the text
    /// node right before it.
    fn process_text(&mut self) {
        let parent_idx = self.get_current_parent();
        let last_child_idx = self.nodes[parent_idx].last_child;
        if let Some(last) = usize::try_from(last_child_idx)
            .ok()
            .and_then(|idx| self.nodes.get(idx))
            && last.kind == NodeKind::Text
        {
            let expected_next_token = last.token_start as i64 + last.token_count as i64;
            // Only merge adjacent text token spans; gaps can appear when malformed input
            // emits skipped structural tokens.
            if expected_next_token == self.pos as i64 {
                if let Some(last) = self.node_mut(last_child_idx) {
                    last.token_count += 1;
                }
                self.pos += 1;
                return;
            }
        }
        let start_pos = self.pos;
        self.pos += 1;
        self.add_child(Self::create_node(NodeKind::Text, 0, start_pos, 1));
    }

    /// `processWhitespace`.
    fn process_whitespace(&mut self) {
        self.process_text();
    }

    /// `processAtSign`.
    fn process_at_sign(&mut self) {
        let start_pos = self.pos;
        self.pos += 1;
        self.add_child(Self::create_node(NodeKind::NonOutput, 0, start_pos, 1));
    }

    /// `cleanupTagNameStack`.
    fn cleanup_tag_name_stack(&mut self, element_idx: Option<usize>) {
        let Some(element_idx) = element_idx else {
            return;
        };
        if let Some(tag_name) = self.tag_names.get(element_idx)
            && let Some(stack) = self.tag_name_stacks.get_mut(tag_name)
        {
            stack.pop();
        }
    }

    /// `popIfTop`.
    fn pop_if_top(&mut self, idx: usize) {
        if self.open_elements.last() == Some(&idx) {
            let popped = self.open_elements.pop();
            self.cleanup_tag_name_stack(popped);
        }
    }

    /// `popElementsToDepth`.
    fn pop_elements_to_depth(&mut self, target_depth: usize) {
        while self.open_elements.len() > target_depth {
            let popped = self.open_elements.pop();
            self.cleanup_tag_name_stack(popped);
        }
    }

    /// `createFirstChild`: a node linked as the parent's first and last child.
    fn create_first_child(
        &mut self,
        parent_idx: usize,
        kind: NodeKind,
        token_start: usize,
        token_count: usize,
    ) -> usize {
        let child_idx = self.nodes.len();
        self.nodes.push(Self::create_node(
            kind,
            parent_idx,
            token_start,
            token_count,
        ));
        self.nodes[parent_idx].first_child = child_idx as i32;
        self.nodes[parent_idx].last_child = child_idx as i32;
        child_idx
    }

    /// `createSiblingChild`: a node linked after `prev_sibling_idx` as the parent's last child.
    fn create_sibling_child(
        &mut self,
        prev_sibling_idx: usize,
        parent_idx: usize,
        kind: NodeKind,
        token_start: usize,
        token_count: usize,
    ) -> usize {
        let child_idx = self.nodes.len();
        self.nodes.push(Self::create_node(
            kind,
            parent_idx,
            token_start,
            token_count,
        ));
        self.nodes[prev_sibling_idx].next_sibling = child_idx as i32;
        self.nodes[parent_idx].last_child = child_idx as i32;
        child_idx
    }

    /// Appends a node linked after `last_child_idx` (or first) under `parent_idx`, as
    /// `buildParts`, `buildDeclAttributeParts` and `linkChildNode` do.
    fn link_child_node(
        &mut self,
        parent_idx: usize,
        last_child_idx: Option<usize>,
        child: FlatNode,
    ) -> usize {
        let child_idx = self.nodes.len();
        self.nodes.push(child);
        match last_child_idx {
            None => self.nodes[parent_idx].first_child = child_idx as i32,
            Some(last) => self.nodes[last].next_sibling = child_idx as i32,
        }
        self.nodes[parent_idx].last_child = child_idx as i32;
        child_idx
    }
}
