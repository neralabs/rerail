//! Forte's `Parser\Concerns\ProcessesDirectives`: a directive token (with its arguments) becomes
//! a standalone directive, opens a block (a paired directive, a condition, or a directive the
//! registry learned pairs from the template), opens a branch of an open block, or closes one;
//! blocks left open close where an enclosing one ends.

use super::super::ParseError;
use super::super::directives::helper::{check_directive_args, extract_directive_name_from_token};
use super::super::directives::token_index::{DirectiveTokenIndex, NameSet};
use super::super::node::{NodeKind, StructureRole};
use super::super::token::TokenType;
use super::{ConditionFrame, DirectiveFrame, Family, Names, SwitchFrame, TreeBuilder};
use std::borrow::Cow;

/// What `openPairedDirective`'s `$frameMeta` carries: the frame's names, when the registry's
/// are not the ones.
pub(super) struct FrameMeta<'a> {
    pub(super) terminators: Names<'a>,
    pub(super) branches: Names<'a>,
    pub(super) openers: Names<'a>,
}

/// The kind of frame `closeUnmatchedStructuresAtDepth` closes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Structure {
    Directive,
    Condition,
    Switch,
}

/// Whether a matching open directive frame takes the directive as a branch or a close.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Match {
    Branch,
    Close,
}

impl<'a> TreeBuilder<'a, '_> {
    /// `processDirective`.
    pub(super) fn process_directive(&mut self) -> Result<(), ParseError> {
        let start_pos = self.pos;
        let directive_token = self.tokens[self.pos];
        let source: &'a [u8] = self.source;
        let directive_name = extract_directive_name_from_token(&directive_token, source);
        let mut token_count = 1;
        let check_idx = self.pos + 1;
        let args_info = check_directive_args(self.tokens, source, check_idx, self.tokens.len());
        token_count += args_info.consumed;
        let args_content = args_info.args_content;
        if self.is_switch_related_directive(&directive_name) {
            return self.process_switch_directive(
                directive_name,
                start_pos,
                token_count,
                args_content,
            );
        }
        if self.is_condition_related_directive(&directive_name) {
            return self.process_condition_directive(
                directive_name,
                start_pos,
                token_count,
                args_content,
            );
        }
        if self.directives.is_conditional_pair(&directive_name) {
            return self.process_conditional_pairing_directive(
                directive_name,
                start_pos,
                token_count,
                args_content,
            );
        }
        if self.is_branch_of_open_directive(&directive_name) {
            return self.open_directive_branch(
                &directive_name,
                start_pos,
                token_count,
                args_content,
            );
        }
        if self.is_closing_directive(&directive_name) {
            return self.close_directive(&directive_name, start_pos, token_count, args_content);
        }
        if self.directives.is_paired(&directive_name) {
            self.recover_repeated_unterminated_paired_directive(&directive_name, start_pos)?;
            return self.open_paired_directive(
                directive_name,
                start_pos,
                token_count,
                args_content,
                None,
            );
        }
        if self.try_open_discovered_directive(
            &directive_name,
            start_pos,
            token_count,
            args_content,
        )? {
            return Ok(());
        }
        self.create_standalone_directive(&directive_name, start_pos, token_count, args_content);
        Ok(())
    }

    /// `recoverRepeatedUnterminatedPairedDirective`: when an open block of the same family has
    /// a terminator glued to a word since it opened (`foo@endforeach`, lexed as text), the
    /// structures opened inside it close here.
    fn recover_repeated_unterminated_paired_directive(
        &mut self,
        directive_name: &[u8],
        start_pos: usize,
    ) -> Result<(), ParseError> {
        let Some(directive) = self.directives.get_directive(directive_name) else {
            return Ok(());
        };
        if directive.terminators.is_empty() || self.open_directives.is_empty() {
            return Ok(());
        }
        let openers = [directive_name];
        for index in (0..self.open_directives.len()).rev() {
            self.budget.spend(1)?;
            let frame = &self.open_directives[index];
            if !directive_families_intersect(&frame.openers, openers.as_slice()) {
                continue;
            }
            let previous_start = self.nodes[frame.block_idx].token_start;
            let content_start = usize::try_from(previous_start)
                .ok()
                .and_then(|idx| self.tokens.get(idx))
                .map(|token| token.end as usize);
            let content_end = self.tokens.get(start_pos).map(|token| token.start as usize);
            let (Some(content_start), Some(content_end)) = (content_start, content_end) else {
                return Ok(());
            };
            if content_end <= content_start {
                return Ok(());
            }
            let content =
                super::super::scan::substr(self.source, content_start, content_end - content_start);
            self.budget.spend(1 + content.len() as u64 / 64)?;
            if !has_glued_terminator(content, &frame.terminators) {
                return Ok(());
            }
            let element_stack_base = frame.element_stack_base;
            self.close_unmatched_structures_at_depth(element_stack_base, start_pos)?;
            return Ok(());
        }
        Ok(())
    }

    /// `tryOpenDiscoveredDirective`: opens a block for a directive the registry does not know
    /// when the template pairs it (`@x … @endx`) or branches it (`@x … @elsex … @endx`).
    fn try_open_discovered_directive(
        &mut self,
        directive_name: &[u8],
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
    ) -> Result<bool, ParseError> {
        let Some(family) = self.resolve_discovered_directive_family(directive_name) else {
            return Ok(false);
        };
        let directive_start = start_pos + token_count;
        let search_end =
            self.get_discovered_directive_search_end(&family.openers, directive_start)?;
        if !family.has_advisory_condition {
            if !self.has_directive_in_search_range(&family.end_name, directive_start, search_end)? {
                return Ok(false);
            }
            let (index, budget) = self.index_and_budget();
            let terminator_idx = index.find_matching_terminator_for_openers(
                family.openers.as_slice(),
                directive_start,
                [&*family.end_name].as_slice(),
                search_end,
                0,
                budget,
            )?;
            if terminator_idx.is_none() {
                return Ok(false);
            }
            self.open_paired_directive(
                Cow::Owned(directive_name.to_vec()),
                start_pos,
                token_count,
                args_content,
                Some(FrameMeta {
                    terminators: Names::Many(vec![family.end_name.clone()]),
                    branches: Names::Many(Vec::new()),
                    openers: Names::Many(family.openers),
                }),
            )?;
            return Ok(true);
        }
        let generic_condition_terminators = self.directives.get_condition_terminators();
        let terminators: Vec<Box<[u8]>> =
            if generic_condition_terminators.contains(&family.end_name) {
                generic_condition_terminators.to_vec()
            } else {
                std::iter::once(family.end_name.clone())
                    .chain(generic_condition_terminators.iter().cloned())
                    .collect()
            };
        let branches: Vec<Box<[u8]>> = vec![
            family.else_name.clone(),
            b"else".as_slice().into(),
            b"elseif".as_slice().into(),
        ];
        let mut has_possible_boundary =
            self.has_directive_in_search_range(&family.end_name, directive_start, search_end)?
                || self.has_directive_in_search_range(
                    &family.else_name,
                    directive_start,
                    search_end,
                )?;
        if !has_possible_boundary {
            for name in generic_condition_terminators {
                if self.has_directive_in_search_range(name, directive_start, search_end)? {
                    has_possible_boundary = true;
                    break;
                }
            }
        }
        if !has_possible_boundary {
            return Ok(false);
        }
        let (index, budget) = self.index_and_budget();
        let analysis = index.analyze_unknown_directive_family(
            family.openers.as_slice(),
            directive_start,
            terminators.as_slice(),
            branches.as_slice(),
            search_end,
            budget,
        )?;
        let (Some(_), Some(terminator_name)) = (analysis.terminator_idx, analysis.terminator_name)
        else {
            return Ok(false);
        };
        let opens_as_condition_like =
            analysis.branch_idx.is_some() || terminator_name != &*family.end_name;
        let meta = if opens_as_condition_like {
            FrameMeta {
                terminators: Names::Many(terminators),
                branches: Names::Many(branches),
                openers: Names::Many(family.openers),
            }
        } else {
            FrameMeta {
                terminators: Names::Many(vec![family.end_name.clone()]),
                branches: Names::Many(Vec::new()),
                openers: Names::Many(family.openers),
            }
        };
        self.open_paired_directive(
            Cow::Owned(directive_name.to_vec()),
            start_pos,
            token_count,
            args_content,
            Some(meta),
        )?;
        Ok(true)
    }

    /// `resolveDiscoveredDirectiveFamily`: the names of an unknown directive's family when the
    /// registry learned it pairs or branches (`@x`/`@endx`, `@x`/`@elsex`; `@unlessx` joins
    /// `@x`).
    fn resolve_discovered_directive_family(&mut self, directive_name: &[u8]) -> Option<Family> {
        if let Some(family) = self.discovered_directive_family_cache.get(directive_name) {
            return family.clone();
        }
        let family = self.discover_directive_family(directive_name);
        self.discovered_directive_family_cache
            .insert(directive_name.into(), family.clone());
        family
    }

    /// The uncached part of `resolveDiscoveredDirectiveFamily`.
    fn discover_directive_family(&self, directive_name: &[u8]) -> Option<Family> {
        let directives = self.directives;
        if directives.has_explicit_directive(directive_name) {
            return None;
        }
        let concat = |prefix: &[u8], name: &[u8]| -> Box<[u8]> {
            [prefix, name].concat().into_boxed_slice()
        };
        let mut base_name: &[u8] = directive_name;
        let mut unless_name = concat(b"unless", directive_name);
        let mut is_unless_variant = false;
        if directive_name.starts_with(b"unless") && directive_name.len() > 6 {
            let candidate_base = &directive_name[6..];
            if candidate_base.is_empty() || directives.has_explicit_directive(candidate_base) {
                return None;
            }
            base_name = candidate_base;
            unless_name = directive_name.into();
            is_unless_variant = true;
        }
        let else_name = concat(b"else", base_name);
        let end_name = concat(b"end", base_name);
        let has_advisory_pair =
            directives.has_advisory_pair(base_name) || directives.has_seen_directive(&end_name);
        let has_advisory_condition = directives.has_advisory_condition(base_name)
            || directives.has_seen_directive(&else_name);
        if !has_advisory_pair && !has_advisory_condition {
            return None;
        }
        let openers = if is_unless_variant || directives.has_seen_directive(&unless_name) {
            vec![base_name.into(), unless_name]
        } else {
            vec![directive_name.into()]
        };
        Some(Family {
            else_name,
            end_name,
            openers,
            has_advisory_condition,
        })
    }

    /// The directive index, built on first use, and the budget, borrowed together.
    fn index_and_budget(&mut self) -> (&mut DirectiveTokenIndex, &mut super::super::Budget) {
        let (tokens, source) = (self.tokens, self.source);
        let index = self
            .directive_index
            .get_or_insert_with(|| DirectiveTokenIndex::new(tokens, source));
        (index, &mut *self.budget)
    }

    /// `hasDirectiveInSearchRange`.
    fn has_directive_in_search_range(
        &mut self,
        directive_name: &[u8],
        min_idx: usize,
        max_idx_exclusive: Option<usize>,
    ) -> Result<bool, ParseError> {
        let (index, budget) = self.index_and_budget();
        Ok(match max_idx_exclusive {
            None => index.exists_after(directive_name, min_idx, budget)?,
            Some(max) => index.exists_between(directive_name, min_idx, max, budget)?,
        })
    }

    /// `getDiscoveredDirectiveSearchEnd`: the nearest of the tag's end (inside a tag), the
    /// containing element's closing tag, an open block's next boundary and an open switch's.
    fn get_discovered_directive_search_end(
        &mut self,
        opener_names: &[Box<[u8]>],
        start_idx: usize,
    ) -> Result<Option<usize>, ParseError> {
        let mut search_end = self.attribute_region_end;
        search_end = take_earlier_boundary(search_end, self.find_open_element_boundary(start_idx)?);
        search_end = take_earlier_boundary(
            search_end,
            self.find_open_directive_boundary(opener_names, start_idx)?,
        );
        Ok(take_earlier_boundary(
            search_end,
            self.find_open_switch_boundary(start_idx)?,
        ))
    }

    /// `findOpenElementBoundary`: the closing tag of the innermost open element with a name,
    /// counting nested elements of the same name.
    fn find_open_element_boundary(
        &mut self,
        start_idx: usize,
    ) -> Result<Option<usize>, ParseError> {
        let mut containing_tag_name = None;
        for i in (0..self.open_elements.len()).rev() {
            let element_idx = self.open_elements[i];
            if self.nodes.get(element_idx).map(|node| node.kind) != Some(NodeKind::Element) {
                continue;
            }
            containing_tag_name = self.tag_names.get(element_idx);
            if containing_tag_name.is_some() {
                break;
            }
        }
        let Some(containing_tag_name) = containing_tag_name else {
            return Ok(None);
        };
        let tokens = self.tokens;
        let source = self.source;
        let total = self.token_total;
        // The names compared are lowercase; the tag name tokens' are lowered by the comparison.
        let text = |idx: usize| {
            super::super::directives::token_index::token_text(
                source,
                tokens[idx].start,
                tokens[idx].end,
            )
        };
        let mut containing_tag_depth = 0usize;
        for i in start_idx..total {
            self.budget.spend(1)?;
            if tokens[i].kind != TokenType::LessThan {
                continue;
            }
            if i + 2 >= total {
                continue;
            }
            if tokens[i + 1].kind == TokenType::Slash {
                if tokens[i + 2].kind != TokenType::TagName {
                    continue;
                }
                if !text(i + 2).eq_ignore_ascii_case(containing_tag_name) {
                    continue;
                }
                if containing_tag_depth == 0 {
                    return Ok(Some(i));
                }
                containing_tag_depth -= 1;
                continue;
            }
            if tokens[i + 1].kind != TokenType::TagName {
                continue;
            }
            if !text(i + 1).eq_ignore_ascii_case(containing_tag_name) {
                continue;
            }
            if !is_self_closing_element_at(tokens, i, total, self.budget)? {
                containing_tag_depth += 1;
            }
        }
        Ok(None)
    }

    /// `findOpenDirectiveBoundary`: the nearest next boundary (branch or terminator at nesting
    /// zero) of any open block.
    fn find_open_directive_boundary(
        &mut self,
        opener_names: &(impl NameSet + ?Sized),
        start_idx: usize,
    ) -> Result<Option<usize>, ParseError> {
        if self.open_directives.is_empty() {
            return Ok(None);
        }
        let (tokens, source) = (self.tokens, self.source);
        let index = self
            .directive_index
            .get_or_insert_with(|| DirectiveTokenIndex::new(tokens, source));
        let mut boundary = None;
        for frame in &self.open_directives {
            if frame.terminators.is_empty() && frame.branches.is_empty() {
                continue;
            }
            let initial_nesting =
                directive_families_intersect(&frame.openers, opener_names) as usize;
            let boundary_idx = if !frame.branches.is_empty() {
                index.find_matching_boundary_for_openers(
                    &frame.openers,
                    start_idx,
                    &frame.terminators,
                    &frame.branches,
                    None,
                    initial_nesting,
                    self.budget,
                )?
            } else {
                index.find_matching_terminator_for_openers(
                    &frame.openers,
                    start_idx,
                    &frame.terminators,
                    None,
                    initial_nesting,
                    self.budget,
                )?
            };
            if boundary_idx.is_some() {
                boundary = take_earlier_boundary(boundary, boundary_idx);
            }
        }
        Ok(boundary)
    }

    /// `findOpenSwitchBoundary`: the nearest next case or `@endswitch` of any open switch.
    fn find_open_switch_boundary(&mut self, start_idx: usize) -> Result<Option<usize>, ParseError> {
        if self.open_switches.is_empty() {
            return Ok(None);
        }
        let directives = self.directives;
        let (tokens, source) = (self.tokens, self.source);
        let index = self
            .directive_index
            .get_or_insert_with(|| DirectiveTokenIndex::new(tokens, source));
        let mut boundary = None;
        for frame in &self.open_switches {
            let mut boundaries = directives.get_switch_branches(&frame.name);
            boundaries.push(b"endswitch");
            let boundary_idx = index.find_matching_terminator(
                &frame.name,
                start_idx,
                boundaries.as_slice(),
                None,
                0,
                self.budget,
            )?;
            if boundary_idx.is_some() {
                boundary = take_earlier_boundary(boundary, boundary_idx);
            }
        }
        Ok(boundary)
    }

    /// `isClosingDirective`: a terminator of an open block.
    fn is_closing_directive(&self, directive_name: &[u8]) -> bool {
        self.open_directives
            .iter()
            .rev()
            .any(|frame| frame.terminators.contains(directive_name))
    }

    /// `openPairedDirective`: a block node, its opening directive as its first child, and a
    /// frame; both nodes stay open.
    pub(super) fn open_paired_directive(
        &mut self,
        directive_name: Cow<'a, [u8]>,
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
        frame_meta: Option<FrameMeta<'a>>,
    ) -> Result<(), ParseError> {
        let block_node = Self::create_directive_node(
            NodeKind::DirectiveBlock,
            0,
            start_pos,
            token_count,
            &directive_name,
            args_content,
            None,
        );
        let block_idx = self.add_child(block_node);
        let element_stack_base = self.open_elements.len();
        self.check_element_depth()?;
        self.open_elements.push(block_idx);
        let start_directive_node = Self::create_directive_node(
            NodeKind::Directive,
            block_idx,
            start_pos,
            token_count,
            &directive_name,
            args_content,
            Some(StructureRole::Opening),
        );
        let start_directive_idx = self.add_child(start_directive_node);
        let directive = self.directives.get_directive(&directive_name);
        let (terminators, branches, openers) = match frame_meta {
            Some(meta) => (meta.terminators, meta.branches, meta.openers),
            None => (
                Names::Registry(directive.map_or(&[][..], |directive| &directive.terminators)),
                Names::Registry(
                    directive.map_or(&[][..], |directive| &directive.condition_like_branches),
                ),
                Names::One(directive_name),
            ),
        };
        self.check_directive_depth()?;
        self.open_directives.push(DirectiveFrame {
            block_idx,
            start_directive_idx,
            element_stack_base,
            terminators,
            branches,
            openers,
        });
        self.check_element_depth()?;
        self.open_elements.push(start_directive_idx);
        self.pos += token_count;
        Ok(())
    }

    /// `isBranchOfOpenDirective`.
    pub(super) fn is_branch_of_open_directive(&self, directive_name: &[u8]) -> bool {
        self.open_directives
            .iter()
            .rev()
            .any(|frame| frame.branches.contains(directive_name))
    }

    /// `openDirectiveBranch`: an intermediate directive of the innermost block it branches,
    /// which stays open in place of the previous branch.
    pub(super) fn open_directive_branch(
        &mut self,
        directive_name: &[u8],
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
    ) -> Result<(), ParseError> {
        let Some(matched_idx) = self
            .open_directives
            .iter()
            .rposition(|frame| frame.branches.contains(directive_name))
        else {
            self.create_standalone_directive(directive_name, start_pos, token_count, args_content);
            return Ok(());
        };
        let target_depth = self.open_directives[matched_idx].element_stack_base + 1;
        self.close_unmatched_structures_at_depth(target_depth, start_pos)?;
        let Some(frame) = self.open_directives.get(matched_idx) else {
            return Ok(());
        };
        let (block_idx, start_directive_idx, element_stack_base) = (
            frame.block_idx,
            frame.start_directive_idx,
            frame.element_stack_base,
        );
        self.pop_if_top(start_directive_idx);
        self.pop_elements_to_depth(element_stack_base + 1);
        let branch_node = Self::create_directive_node(
            NodeKind::Directive,
            block_idx,
            start_pos,
            token_count,
            directive_name,
            args_content,
            Some(StructureRole::Intermediate),
        );
        let branch_idx = self.add_child(branch_node);
        self.check_element_depth()?;
        self.open_elements.push(branch_idx);
        self.open_directives[matched_idx].start_directive_idx = branch_idx;
        self.pos += token_count;
        Ok(())
    }

    /// `closeDirective`: the closing directive of the innermost block it terminates; blocks
    /// opened inside that one close unmatched.
    pub(super) fn close_directive(
        &mut self,
        directive_name: &[u8],
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
    ) -> Result<(), ParseError> {
        let Some(matched_idx) = self
            .open_directives
            .iter()
            .rposition(|frame| frame.terminators.contains(directive_name))
        else {
            self.create_standalone_directive(directive_name, start_pos, token_count, args_content);
            return Ok(());
        };
        let target_depth = self.open_directives[matched_idx].element_stack_base + 2;
        self.close_unmatched_structures_at_depth(target_depth, start_pos)?;
        while self.open_directives.len() > matched_idx + 1 {
            if let Some(frame) = self.open_directives.pop() {
                self.close_unmatched_directive_frame(&frame, start_pos);
            }
        }
        let Some(frame) = self.open_directives.pop() else {
            self.pos += token_count;
            return Ok(());
        };
        let block_idx = frame.block_idx;
        self.pop_elements_to_depth(frame.element_stack_base + 2);
        self.pop_if_top(frame.start_directive_idx);
        let end_directive_node = Self::create_directive_node(
            NodeKind::Directive,
            block_idx,
            start_pos,
            token_count,
            directive_name,
            args_content,
            Some(StructureRole::Closing),
        );
        self.add_child(end_directive_node);
        self.pop_if_top(block_idx);
        let end_pos = (start_pos + token_count) as i32;
        let block = &mut self.nodes[block_idx];
        block.token_count = end_pos - block.token_start;
        self.pos += token_count;
        Ok(())
    }

    /// `closeUnmatchedDirectiveFrame`: the block ends at `end_pos` at least.
    fn close_unmatched_directive_frame(&mut self, frame: &DirectiveFrame<'_>, end_pos: usize) {
        let block_idx = frame.block_idx;
        self.pop_elements_to_depth(frame.element_stack_base + 2);
        self.pop_if_top(frame.start_directive_idx);
        self.pop_if_top(block_idx);
        let block = &mut self.nodes[block_idx];
        block.token_count = block.token_count.max(end_pos as i32 - block.token_start);
    }

    /// `closeUnmatchedStructuresAtDepth`: closes, innermost first, the open blocks, conditions
    /// and switches based at `target_depth` or deeper.
    pub(super) fn close_unmatched_structures_at_depth(
        &mut self,
        target_depth: usize,
        end_pos: usize,
    ) -> Result<(), ParseError> {
        loop {
            self.budget.spend(1)?;
            let mut candidate_kind = None;
            let mut candidate_base: i64 = -1;
            let mut candidate_start: i64 = -1;
            let mut consider = |kind: Structure, base: usize, start: i64| {
                let base = base as i64;
                if base < target_depth as i64 {
                    return;
                }
                if base > candidate_base || (base == candidate_base && start > candidate_start) {
                    candidate_kind = Some(kind);
                    candidate_base = base;
                    candidate_start = start;
                }
            };
            let start_of = |idx: usize| {
                self.nodes
                    .get(idx)
                    .map_or(-1, |node| node.token_start as i64)
            };
            if let Some(frame) = self.open_directives.last() {
                consider(
                    Structure::Directive,
                    frame.element_stack_base,
                    start_of(frame.block_idx),
                );
            }
            if let Some(frame) = self.open_conditions.last() {
                consider(
                    Structure::Condition,
                    frame.element_stack_base,
                    start_of(frame.block_idx),
                );
            }
            if let Some(frame) = self.open_switches.last() {
                consider(
                    Structure::Switch,
                    frame.element_stack_base,
                    start_of(frame.block_idx),
                );
            }
            match candidate_kind {
                None => return Ok(()),
                Some(Structure::Directive) => {
                    if let Some(frame) = self.open_directives.pop() {
                        self.close_unmatched_directive_frame(&frame, end_pos);
                    }
                }
                Some(Structure::Condition) => {
                    if let Some(frame) = self.open_conditions.pop() {
                        self.close_unmatched_condition_frame(&frame, end_pos);
                    }
                }
                Some(Structure::Switch) => {
                    if let Some(frame) = self.open_switches.pop() {
                        self.close_unmatched_switch_frame(&frame, end_pos);
                    }
                }
            }
        }
    }

    /// `closeUnmatchedSwitchFrame`.
    fn close_unmatched_switch_frame(&mut self, frame: &SwitchFrame<'_>, end_pos: usize) {
        let block_idx = frame.block_idx;
        self.pop_elements_to_depth(frame.element_stack_base + 2);
        if let Some(current_case_idx) = frame.current_case_idx {
            self.pop_if_top(current_case_idx);
        }
        self.pop_if_top(frame.switch_directive_idx);
        self.pop_if_top(block_idx);
        let block = &mut self.nodes[block_idx];
        block.token_count = block.token_count.max(end_pos as i32 - block.token_start);
    }

    /// `createStandaloneDirective`.
    pub(super) fn create_standalone_directive(
        &mut self,
        directive_name: &[u8],
        start_pos: usize,
        token_count: usize,
        args_content: Option<&[u8]>,
    ) {
        let node = Self::create_directive_node(
            NodeKind::Directive,
            0,
            start_pos,
            token_count,
            directive_name,
            args_content,
            Some(StructureRole::None),
        );
        self.add_child(node);
        self.pos += token_count;
    }

    /// `closeRemainingDirectives`: open switches, conditions and blocks end at the last token.
    pub(super) fn close_remaining_directives(&mut self) {
        let end_pos = self.tokens.len() as i32;
        while let Some(frame) = self.open_switches.pop() {
            if let Some(current_case_idx) = frame.current_case_idx {
                self.pop_if_top(current_case_idx);
            }
            self.pop_if_top(frame.switch_directive_idx);
            self.pop_if_top(frame.block_idx);
            let block = &mut self.nodes[frame.block_idx];
            block.token_count = end_pos - block.token_start;
        }
        while let Some(frame) = self.open_conditions.pop() {
            self.pop_if_top(frame.current_branch_idx);
            self.pop_if_top(frame.block_idx);
            let block = &mut self.nodes[frame.block_idx];
            block.token_count = end_pos - block.token_start;
        }
        while let Some(frame) = self.open_directives.pop() {
            self.pop_if_top(frame.start_directive_idx);
            self.pop_if_top(frame.block_idx);
            let block = &mut self.nodes[frame.block_idx];
            block.token_count = end_pos - block.token_start;
        }
    }

    /// `isConditionRelatedDirective`: a condition, a branch of the innermost open condition,
    /// or a condition terminator.
    fn is_condition_related_directive(&self, directive_name: &[u8]) -> bool {
        let directives = self.directives;
        if directives.is_condition(directive_name) {
            return true;
        }
        if let Some(current_condition) = self.open_conditions.last() {
            let branches = directives.get_branches(&current_condition.name);
            if branches.iter().any(|branch| &**branch == directive_name) {
                return true;
            }
        }
        directives
            .get_condition_terminators()
            .iter()
            .any(|terminator| &**terminator == directive_name)
    }

    /// `processConditionDirective`.
    fn process_condition_directive(
        &mut self,
        directive_name: Cow<'a, [u8]>,
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
    ) -> Result<(), ParseError> {
        let directives = self.directives;
        let directive = directives.get_directive(&directive_name);
        let opens = directive.is_some_and(|directive| directive.role == StructureRole::Opening);
        // An argument-bearing @empty can open its own condition inside @forelse.
        if args_content.is_some()
            && self.is_branch_of_open_directive(&directive_name)
            && directives.is_condition(&directive_name)
            && opens
            && self.has_own_condition_terminator_before_outer_boundary(
                &directive_name,
                start_pos + token_count,
            )?
        {
            return self.open_condition(directive_name, start_pos, token_count, args_content);
        }
        if self.try_handle_condition_directive_with_open_directive(
            &directive_name,
            start_pos,
            token_count,
            args_content,
        )? {
            return Ok(());
        }
        let terminators = directives.get_condition_terminators();
        let is_final_terminator = terminators
            .iter()
            .any(|terminator| **terminator == *directive_name);
        if is_final_terminator && !self.open_conditions.is_empty() {
            return self.close_condition(&directive_name, start_pos, token_count, args_content);
        }
        if let Some(current_condition) = self.open_conditions.last() {
            let branches = directives.get_branches(&current_condition.name);
            if branches.iter().any(|branch| **branch == *directive_name) {
                return self.open_condition_branch(
                    &directive_name,
                    start_pos,
                    token_count,
                    args_content,
                );
            }
        }
        if directives.is_condition(&directive_name) && opens {
            return self.open_condition(directive_name, start_pos, token_count, args_content);
        }
        self.create_standalone_directive(&directive_name, start_pos, token_count, args_content);
        Ok(())
    }

    /// `hasOwnConditionTerminatorBeforeOuterBoundary`: the directive's own condition
    /// terminator comes before the open blocks' next boundary.
    fn has_own_condition_terminator_before_outer_boundary(
        &mut self,
        directive_name: &[u8],
        search_start: usize,
    ) -> Result<bool, ParseError> {
        let directives = self.directives;
        let Some(directive) = directives.get_directive(directive_name) else {
            return Ok(false);
        };
        let condition_terminators = directives.get_condition_terminators();
        let own: Vec<&[u8]> = directive
            .terminators
            .iter()
            .filter(|terminator| condition_terminators.contains(terminator))
            .map(|terminator| &**terminator)
            .collect();
        if own.is_empty() {
            return Ok(false);
        }
        let search_end =
            self.find_open_directive_boundary([directive_name].as_slice(), search_start)?;
        let (index, budget) = self.index_and_budget();
        Ok(index
            .find_matching_terminator(
                directive_name,
                search_start,
                own.as_slice(),
                search_end,
                0,
                budget,
            )?
            .is_some())
    }

    /// `tryHandleConditionDirectiveWithOpenDirective`: a condition directive an open block
    /// takes as its branch or close, unless an open condition opened since takes it.
    fn try_handle_condition_directive_with_open_directive(
        &mut self,
        directive_name: &[u8],
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
    ) -> Result<bool, ParseError> {
        let Some((directive_frame_start, kind)) =
            self.get_matching_open_directive_frame(directive_name)
        else {
            return Ok(false);
        };
        if let Some(condition_frame_start) = self.get_matching_open_condition_frame(directive_name)
            && condition_frame_start >= directive_frame_start
        {
            return Ok(false);
        }
        match kind {
            Match::Branch => {
                self.open_directive_branch(directive_name, start_pos, token_count, args_content)?
            }
            Match::Close => {
                self.close_directive(directive_name, start_pos, token_count, args_content)?
            }
        }
        Ok(true)
    }

    /// `getMatchingOpenDirectiveFrame`, with `getDirectiveFrameTokenStart` of the frame: the
    /// innermost open block that takes the directive as a branch or a close.
    fn get_matching_open_directive_frame(&self, directive_name: &[u8]) -> Option<(i64, Match)> {
        for frame in self.open_directives.iter().rev() {
            let kind = if frame.branches.contains(directive_name) {
                Match::Branch
            } else if frame.terminators.contains(directive_name) {
                Match::Close
            } else {
                continue;
            };
            let start = self
                .nodes
                .get(frame.start_directive_idx)
                .map_or(-1, |node| node.token_start as i64);
            return Some((start, kind));
        }
        None
    }

    /// `getMatchingOpenConditionFrame`, with `getConditionFrameTokenStart` of the frame: the
    /// innermost open condition, when it takes the directive as a branch or a close.
    fn get_matching_open_condition_frame(&self, directive_name: &[u8]) -> Option<i64> {
        let frame = self.open_conditions.last()?;
        let directives = self.directives;
        let matches = directives
            .get_condition_terminators()
            .iter()
            .any(|terminator| &**terminator == directive_name)
            || directives
                .get_branches(&frame.name)
                .iter()
                .any(|branch| &**branch == directive_name);
        if !matches {
            return None;
        }
        Some(
            self.nodes
                .get(frame.current_branch_idx)
                .map_or(-1, |node| node.token_start as i64),
        )
    }

    /// `openCondition`: a block node, its opening directive, and a condition frame.
    fn open_condition(
        &mut self,
        directive_name: Cow<'a, [u8]>,
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
    ) -> Result<(), ParseError> {
        let block_node = Self::create_directive_node(
            NodeKind::DirectiveBlock,
            0,
            start_pos,
            token_count,
            &directive_name,
            args_content,
            None,
        );
        let block_idx = self.add_child(block_node);
        let element_stack_base = self.open_elements.len();
        self.check_element_depth()?;
        self.open_elements.push(block_idx);
        let branch_node = Self::create_directive_node(
            NodeKind::Directive,
            0,
            start_pos,
            token_count,
            &directive_name,
            args_content,
            Some(StructureRole::Opening),
        );
        let branch_idx = self.add_child(branch_node);
        self.check_element_depth()?;
        self.open_elements.push(branch_idx);
        self.check_condition_depth()?;
        self.open_conditions.push(ConditionFrame {
            block_idx,
            current_branch_idx: branch_idx,
            name: directive_name,
            element_stack_base,
        });
        self.pos += token_count;
        Ok(())
    }

    /// `openConditionBranch`: an intermediate directive of the innermost open condition.
    fn open_condition_branch(
        &mut self,
        directive_name: &[u8],
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
    ) -> Result<(), ParseError> {
        let Some(condition_idx) = self.open_conditions.len().checked_sub(1) else {
            self.create_standalone_directive(directive_name, start_pos, token_count, args_content);
            return Ok(());
        };
        let element_stack_base = self.open_conditions[condition_idx].element_stack_base;
        self.close_unmatched_structures_at_depth(element_stack_base + 1, start_pos)?;
        let Some(frame) = self.open_conditions.get(condition_idx) else {
            return Ok(());
        };
        let (current_branch_idx, element_stack_base) =
            (frame.current_branch_idx, frame.element_stack_base);
        self.pop_if_top(current_branch_idx);
        self.pop_elements_to_depth(element_stack_base + 1);
        let branch_node = Self::create_directive_node(
            NodeKind::Directive,
            0,
            start_pos,
            token_count,
            directive_name,
            args_content,
            Some(StructureRole::Intermediate),
        );
        let branch_idx = self.add_child(branch_node);
        self.check_element_depth()?;
        self.open_elements.push(branch_idx);
        self.open_conditions[condition_idx].current_branch_idx = branch_idx;
        self.pos += token_count;
        Ok(())
    }

    /// `closeCondition`: the closing directive of the innermost open condition that owns it
    /// (else the innermost); conditions opened inside that one close unmatched.
    fn close_condition(
        &mut self,
        directive_name: &[u8],
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
    ) -> Result<(), ParseError> {
        if self.open_conditions.is_empty() {
            self.create_standalone_directive(directive_name, start_pos, token_count, args_content);
            return Ok(());
        }
        let matched_idx = self
            .open_conditions
            .iter()
            .rposition(|frame| self.condition_frame_accepts_close(frame, directive_name))
            // Preserve legacy mixed condition terminators when no open frame owns the close
            // exactly.
            .unwrap_or(self.open_conditions.len() - 1);
        let target_depth = self.open_conditions[matched_idx].element_stack_base + 1;
        self.close_unmatched_structures_at_depth(target_depth, start_pos)?;
        while self.open_conditions.len() > matched_idx + 1 {
            if let Some(frame) = self.open_conditions.pop() {
                self.close_unmatched_condition_frame(&frame, start_pos);
            }
        }
        let Some(frame) = self.open_conditions.pop() else {
            self.pos += token_count;
            return Ok(());
        };
        self.pop_elements_to_depth(frame.element_stack_base + 1);
        self.pop_if_top(frame.current_branch_idx);
        let terminator_node = Self::create_directive_node(
            NodeKind::Directive,
            0,
            start_pos,
            token_count,
            directive_name,
            args_content,
            Some(StructureRole::Closing),
        );
        self.add_child(terminator_node);
        self.pop_if_top(frame.block_idx);
        let end_pos = (start_pos + token_count) as i32;
        let block = &mut self.nodes[frame.block_idx];
        block.token_count = end_pos - block.token_start;
        self.pos += token_count;
        Ok(())
    }

    /// `conditionFrameAcceptsClose`: the directive is one of the condition's own terminators
    /// and a condition terminator.
    fn condition_frame_accepts_close(
        &self,
        frame: &ConditionFrame<'_>,
        directive_name: &[u8],
    ) -> bool {
        let directives = self.directives;
        let Some(directive) = directives.get_directive(&frame.name) else {
            return false;
        };
        let condition_terminators = directives.get_condition_terminators();
        directive.terminators.iter().any(|terminator| {
            terminator.eq_ignore_ascii_case(directive_name)
                && condition_terminators.iter().any(|condition_terminator| {
                    condition_terminator.as_ref() == terminator.to_ascii_lowercase().as_slice()
                })
        })
    }

    /// `closeUnmatchedConditionFrame`.
    fn close_unmatched_condition_frame(&mut self, frame: &ConditionFrame<'_>, end_pos: usize) {
        let block_idx = frame.block_idx;
        self.pop_elements_to_depth(frame.element_stack_base + 1);
        self.pop_if_top(frame.current_branch_idx);
        self.pop_if_top(block_idx);
        let block = &mut self.nodes[block_idx];
        block.token_count = block.token_count.max(end_pos as i32 - block.token_start);
    }
}

/// `directiveFamiliesIntersect`: the families share their first name, or one's second name is
/// the other's first or second.
fn directive_families_intersect(
    left: &(impl NameSet + ?Sized),
    right: &(impl NameSet + ?Sized),
) -> bool {
    let (left_first, left_second) = first_two(left);
    let (right_first, right_second) = first_two(right);
    if left_first == right_first {
        return true;
    }
    left_second == right_first
        || right_second == left_first
        || (left_second.is_some() && left_second == right_second)
}

/// The first two names of a list (`$names[0] ?? null`, `$names[1] ?? null`), as owned bytes
/// only when a list must be walked to reach them.
fn first_two(names: &(impl NameSet + ?Sized)) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let mut first = None;
    let mut second = None;
    let mut i = 0;
    names.for_each_name(&mut |name| {
        match i {
            0 => first = Some(name.to_vec()),
            1 => second = Some(name.to_vec()),
            _ => {}
        }
        i += 1;
    });
    (first, second)
}

/// `takeEarlierBoundary`.
fn take_earlier_boundary(current: Option<usize>, candidate: Option<usize>) -> Option<usize> {
    match (current, candidate) {
        (current, None) => current,
        (None, candidate) => candidate,
        (Some(current), Some(candidate)) => Some(current.min(candidate)),
    }
}

/// `isSelfClosingElementAt`: the tag at `start_idx` ends with `/>` before another tag starts.
fn is_self_closing_element_at(
    tokens: &[super::super::token::Token],
    start_idx: usize,
    limit_idx: usize,
    budget: &mut super::super::Budget,
) -> Result<bool, ParseError> {
    for i in start_idx + 1..limit_idx {
        budget.spend(1)?;
        match tokens[i].kind {
            TokenType::GreaterThan | TokenType::SyntheticClose => {
                return Ok(tokens[i - 1].kind == TokenType::Slash);
            }
            TokenType::LessThan => return Ok(false),
            _ => {}
        }
    }
    Ok(false)
}

/// `preg_match('/(?<=[A-Za-z0-9_])@(?:t1|t2|…)(?![A-Za-z0-9_])/i', $content) === 1` for the
/// terminators (quoted, so literal): some `@` right after a word byte is followed by one of the
/// terminators, ASCII case-insensitively, and then by no word byte. An empty list is the
/// empty alternative.
fn has_glued_terminator(content: &[u8], terminators: &Names<'_>) -> bool {
    let is_word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let matches_at = |after: usize, terminator: &[u8]| {
        let end = after + terminator.len();
        end <= content.len()
            && content[after..end].eq_ignore_ascii_case(terminator)
            && !content.get(end).is_some_and(|byte| is_word(*byte))
    };
    for at in 1..content.len() {
        if content[at] != b'@' || !is_word(content[at - 1]) {
            continue;
        }
        if terminators.is_empty() {
            if matches_at(at + 1, b"") {
                return true;
            }
            continue;
        }
        let mut found = false;
        terminators.for_each_name(&mut |terminator| {
            found = found || matches_at(at + 1, terminator);
        });
        if found {
            return true;
        }
    }
    false
}
