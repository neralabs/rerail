//! Forte's `Parser\Directives\DirectiveTokenIndex`: the template's directive tokens by position
//! and by name, which the tree builder searches for terminators, branches and boundaries.
//!
//! Names are interned: each distinct lowercase name gets an id, and the name sets a search takes
//! (`getNameSet`) are marks on those ids, set before the search and cleared after it. A name the
//! template never uses has no id and can match nothing, as it matches nothing in Forte's sets.
//! Every iteration of a search spends a step.

use super::super::token::{Token, TokenType};
use super::super::{Budget, Exhausted};
use super::helper::lower_cow;
use std::collections::HashMap;

/// A name set's mark on an id: the name opens, branches or terminates.
const OPENER: u8 = 1;
const BRANCH: u8 = 2;
const TERMINATOR: u8 = 4;

/// `DirectiveTokenIndex`.
pub(crate) struct DirectiveTokenIndex {
    /// Name to id.
    ids: HashMap<Box<[u8]>, u32>,
    /// Id to name.
    names: Vec<Box<[u8]>>,
    /// `$byName`, by id.
    by_name: Vec<Vec<usize>>,
    /// `$allPositions`.
    all_positions: Vec<usize>,
    /// `$allNames`, as ids.
    all_names: Vec<u32>,
    /// The current search's name sets.
    marks: Vec<u8>,
}

/// A list of names a search takes (`getNameSet`'s argument).
pub(crate) trait NameSet {
    fn for_each_name(&self, f: &mut dyn FnMut(&[u8]));
}

impl<T: AsRef<[u8]>> NameSet for [T] {
    fn for_each_name(&self, f: &mut dyn FnMut(&[u8])) {
        for name in self {
            f(name.as_ref());
        }
    }
}

/// What `analyzeUnknownDirectiveFamily` finds.
pub(crate) struct Analysis<'i> {
    pub(crate) terminator_idx: Option<usize>,
    pub(crate) terminator_name: Option<&'i [u8]>,
    pub(crate) branch_idx: Option<usize>,
}

impl DirectiveTokenIndex {
    /// `__construct` and `build`: every directive token's position under its name (lowercase,
    /// without the `@`, as `extractNameFromOffsets` takes it).
    pub(crate) fn new(tokens: &[Token], source: &[u8]) -> Self {
        let mut index = Self {
            ids: HashMap::new(),
            names: Vec::new(),
            by_name: Vec::new(),
            all_positions: Vec::new(),
            all_names: Vec::new(),
            marks: Vec::new(),
        };
        for (idx, token) in tokens.iter().enumerate() {
            if token.kind != TokenType::Directive {
                continue;
            }
            let name = extract_name_from_offsets(token.start, token.end, source);
            let id = match index.ids.get(&*name) {
                Some(id) => *id,
                None => {
                    let id = index.names.len() as u32;
                    let name: Box<[u8]> = name.into_owned().into_boxed_slice();
                    index.names.push(name.clone());
                    index.ids.insert(name, id);
                    index.by_name.push(Vec::new());
                    index.marks.push(0);
                    id
                }
            };
            index.all_positions.push(idx);
            index.all_names.push(id);
            index.by_name[id as usize].push(idx);
        }
        index
    }

    /// The id of a name (`normalize`d), if the template uses it.
    fn id(&self, name: &[u8]) -> Option<u32> {
        self.ids.get(&*lower_cow(name)).copied()
    }

    /// `existsAfter`.
    pub(crate) fn exists_after(
        &self,
        name: &[u8],
        min_idx: usize,
        budget: &mut Budget,
    ) -> Result<bool, Exhausted> {
        let Some(id) = self.id(name) else {
            return Ok(false);
        };
        let positions = &self.by_name[id as usize];
        match positions.last() {
            None => return Ok(false),
            Some(last) if *last < min_idx => return Ok(false),
            Some(_) => {}
        }
        Ok(find_first_index_gte(positions, min_idx, budget)?.is_some())
    }

    /// `existsBetween`.
    pub(crate) fn exists_between(
        &self,
        name: &[u8],
        min_idx: usize,
        max_idx_exclusive: usize,
        budget: &mut Budget,
    ) -> Result<bool, Exhausted> {
        let Some(id) = self.id(name) else {
            return Ok(false);
        };
        let positions = &self.by_name[id as usize];
        match positions.last() {
            None => return Ok(false),
            Some(last) if *last < min_idx => return Ok(false),
            Some(_) => {}
        }
        let Some(first_idx) = find_first_index_gte(positions, min_idx, budget)? else {
            return Ok(false);
        };
        Ok(positions[first_idx] < max_idx_exclusive)
    }

    /// `findMatchingTerminator`.
    pub(crate) fn find_matching_terminator(
        &mut self,
        directive_name: &[u8],
        start_idx: usize,
        terminators: &(impl NameSet + ?Sized),
        max_idx_exclusive: Option<usize>,
        initial_nesting: usize,
        budget: &mut Budget,
    ) -> Result<Option<usize>, Exhausted> {
        self.find_matching_terminator_for_openers(
            [directive_name].as_slice(),
            start_idx,
            terminators,
            max_idx_exclusive,
            initial_nesting,
            budget,
        )
    }

    /// `findMatchingTerminatorForOpeners`.
    pub(crate) fn find_matching_terminator_for_openers(
        &mut self,
        opener_names: &(impl NameSet + ?Sized),
        start_idx: usize,
        terminators: &(impl NameSet + ?Sized),
        max_idx_exclusive: Option<usize>,
        initial_nesting: usize,
        budget: &mut Budget,
    ) -> Result<Option<usize>, Exhausted> {
        let Some(first_idx) = find_first_index_gte(&self.all_positions, start_idx, budget)? else {
            return Ok(None);
        };
        self.mark(opener_names, OPENER);
        self.mark(terminators, TERMINATOR);
        let mut found = Ok(None);
        let mut nesting = initial_nesting;
        for i in first_idx..self.all_positions.len() {
            if let Err(exhausted) = budget.spend(1) {
                found = Err(exhausted);
                break;
            }
            if max_idx_exclusive.is_some_and(|max| self.all_positions[i] >= max) {
                break;
            }
            let marks = self.marks[self.all_names[i] as usize];
            if marks & OPENER != 0 {
                nesting += 1;
                continue;
            }
            if marks & TERMINATOR == 0 {
                continue;
            }
            if nesting == 0 {
                found = Ok(Some(self.all_positions[i]));
                break;
            }
            nesting -= 1;
        }
        self.clear(opener_names);
        self.clear(terminators);
        found
    }

    /// `findMatchingBoundaryForOpeners`: the first branch or terminator at nesting zero.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn find_matching_boundary_for_openers(
        &mut self,
        opener_names: &(impl NameSet + ?Sized),
        start_idx: usize,
        terminator_names: &(impl NameSet + ?Sized),
        branch_names: &(impl NameSet + ?Sized),
        max_idx_exclusive: Option<usize>,
        initial_nesting: usize,
        budget: &mut Budget,
    ) -> Result<Option<usize>, Exhausted> {
        let Some(first_idx) = find_first_index_gte(&self.all_positions, start_idx, budget)? else {
            return Ok(None);
        };
        self.mark(opener_names, OPENER);
        self.mark(terminator_names, TERMINATOR);
        self.mark(branch_names, BRANCH);
        let mut found = Ok(None);
        let mut nesting = initial_nesting;
        for i in first_idx..self.all_positions.len() {
            if let Err(exhausted) = budget.spend(1) {
                found = Err(exhausted);
                break;
            }
            if max_idx_exclusive.is_some_and(|max| self.all_positions[i] >= max) {
                break;
            }
            let marks = self.marks[self.all_names[i] as usize];
            if marks & OPENER != 0 {
                nesting += 1;
                continue;
            }
            if marks & BRANCH != 0 {
                if nesting == 0 {
                    found = Ok(Some(self.all_positions[i]));
                    break;
                }
                continue;
            }
            if marks & TERMINATOR == 0 {
                continue;
            }
            if nesting == 0 {
                found = Ok(Some(self.all_positions[i]));
                break;
            }
            nesting -= 1;
        }
        self.clear(opener_names);
        self.clear(terminator_names);
        self.clear(branch_names);
        found
    }

    /// `analyzeUnknownDirectiveFamily`: the first terminator at nesting zero, with the first
    /// branch at nesting zero before it.
    pub(crate) fn analyze_unknown_directive_family(
        &mut self,
        opener_names: &(impl NameSet + ?Sized),
        start_idx: usize,
        terminator_names: &(impl NameSet + ?Sized),
        branch_names: &(impl NameSet + ?Sized),
        max_idx_exclusive: Option<usize>,
        budget: &mut Budget,
    ) -> Result<Analysis<'_>, Exhausted> {
        let none = Analysis {
            terminator_idx: None,
            terminator_name: None,
            branch_idx: None,
        };
        let Some(first_idx) = find_first_index_gte(&self.all_positions, start_idx, budget)? else {
            return Ok(none);
        };
        self.mark(opener_names, OPENER);
        self.mark(terminator_names, TERMINATOR);
        self.mark(branch_names, BRANCH);
        let mut found: Result<Option<(usize, u32, Option<usize>)>, Exhausted> = Ok(None);
        let mut nesting = 0usize;
        let mut branch_idx = None;
        for i in first_idx..self.all_positions.len() {
            if let Err(exhausted) = budget.spend(1) {
                found = Err(exhausted);
                break;
            }
            if max_idx_exclusive.is_some_and(|max| self.all_positions[i] >= max) {
                break;
            }
            let id = self.all_names[i];
            let marks = self.marks[id as usize];
            if marks & OPENER != 0 {
                nesting += 1;
                continue;
            }
            if marks & TERMINATOR != 0 {
                if nesting == 0 {
                    found = Ok(Some((self.all_positions[i], id, branch_idx)));
                    break;
                }
                nesting -= 1;
                continue;
            }
            if branch_idx.is_none() && nesting == 0 && marks & BRANCH != 0 {
                branch_idx = Some(self.all_positions[i]);
            }
        }
        self.clear(opener_names);
        self.clear(terminator_names);
        self.clear(branch_names);
        Ok(match found? {
            None => none,
            Some((terminator_idx, id, branch_idx)) => Analysis {
                terminator_idx: Some(terminator_idx),
                terminator_name: Some(&self.names[id as usize]),
                branch_idx,
            },
        })
    }

    /// Marks the ids of `names` with `mark` (`getNameSet`).
    fn mark(&mut self, names: &(impl NameSet + ?Sized), mark: u8) {
        names.for_each_name(&mut |name| {
            if let Some(id) = self.id(name) {
                self.marks[id as usize] |= mark;
            }
        });
    }

    /// Clears the marks of `names`.
    fn clear(&mut self, names: &(impl NameSet + ?Sized)) {
        names.for_each_name(&mut |name| {
            if let Some(id) = self.id(name) {
                self.marks[id as usize] = 0;
            }
        });
    }
}

/// `extractNameFromOffsets`: the token's text without a leading `@`, lowercase.
fn extract_name_from_offsets(start: u32, end: u32, source: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let text = token_text(source, start, end);
    let text = text.strip_prefix(b"@").unwrap_or(text);
    lower_cow(text)
}

/// `substr($source, $start, $end - $start)`.
pub(crate) fn token_text(source: &[u8], start: u32, end: u32) -> &[u8] {
    let start = start as usize;
    let end = end as usize;
    if start >= source.len() || end <= start {
        return &[];
    }
    &source[start..end.min(source.len())]
}

/// `findFirstIndexGte`: binary search for the first value at least `target`.
fn find_first_index_gte(
    arr: &[usize],
    target: usize,
    budget: &mut Budget,
) -> Result<Option<usize>, Exhausted> {
    if arr.is_empty() {
        return Ok(None);
    }
    let mut lo: isize = 0;
    let mut hi: isize = arr.len() as isize - 1;
    let mut result = None;
    while lo <= hi {
        budget.spend(1)?;
        let mid = (lo + hi) / 2;
        if arr[mid as usize] >= target {
            result = Some(mid as usize);
            hi = mid - 1;
        } else {
            lo = mid + 1;
        }
    }
    Ok(result)
}
