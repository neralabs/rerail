//! Field relationships start from sparse first-child postings and follow stored parent links.
use super::*;

impl<'tree, D> FileAnchorIndex<'tree, D>
where
    D: Doc<Source = String, Lang = GrammarDialect>,
    D::Node<'tree>: AnchorNode<'tree>,
{
    pub(super) fn field_selection<'index>(
        &'index self,
        scope: Selection<'index>,
        inner: &AnchorPlan,
        direction: Direction,
        recursive: bool,
        field: u16,
        memo: &mut UtilitySelections<'index>,
    ) -> Result<Selection<'index>, ()> {
        let posting = self.fields.get(&field).map_or(&[][..], Vec::as_slice);
        if posting.is_empty() {
            return Ok(Selection::empty());
        }
        let inner_scope = match direction {
            Direction::Has if !recursive => {
                let children = Selection::Roots(Cow::Borrowed(posting));
                if matches!(scope, Selection::All) {
                    children
                } else {
                    self.filter(children, |child| {
                        self.entries[child]
                            .parent
                            .is_some_and(|parent| scope.contains(parent.get() - 1))
                    })?
                }
            }
            Direction::Inside if !recursive && !matches!(scope, Selection::All) => {
                let mut parents = Vec::new();
                for child in scope.ordinals(self.len()) {
                    check_budget()?;
                    if posting.binary_search(&child).is_ok()
                        && let Some(parent) = self.entries[child].parent
                    {
                        parents.push(parent.get() - 1);
                    }
                }
                parents.sort_unstable();
                parents.dedup();
                Selection::Roots(Cow::Owned(parents))
            }
            _ => Selection::All,
        };
        let matches = self.select_complete(inner, inner_scope, memo)?;
        if matches.is_empty() {
            return Ok(Selection::empty());
        }
        match direction {
            Direction::Has => {
                let mut roots = Vec::new();
                // Remember edges, not parents: different children can enter the same parent
                // through different fields. A failed edge must not hide a later valid one.
                let mut seen = vec![0_u64; self.len().div_ceil(64)];
                for start in matches.ordinals(self.len()) {
                    let mut child = start;
                    while let Some(parent) = self.entries[child].parent {
                        check_budget()?;
                        let (word, bit) = (child / 64, 1_u64 << (child % 64));
                        if seen[word] & bit != 0 {
                            break;
                        }
                        seen[word] |= bit;
                        let parent = parent.get() - 1;
                        if posting.binary_search(&child).is_ok() && scope.contains(parent) {
                            roots.push(parent);
                        }
                        if !recursive {
                            break;
                        }
                        child = parent;
                    }
                }
                roots.sort_unstable();
                roots.dedup();
                Ok(Selection::Roots(Cow::Owned(roots)))
            }
            Direction::Inside => {
                let children = self.filter(Selection::Roots(Cow::Borrowed(posting)), |child| {
                    self.entries[child]
                        .parent
                        .is_some_and(|parent| matches.contains(parent.get() - 1))
                })?;
                if !recursive {
                    return combine(scope, children, true);
                }
                let mut ranges: Vec<(usize, usize)> = Vec::new();
                for child in children.ordinals(self.len()) {
                    check_budget()?;
                    let end = self.entries[child].subtree_end;
                    if let Some(previous) = ranges.last_mut()
                        && child < previous.1
                    {
                        previous.1 = previous.1.max(end);
                    } else {
                        ranges.push((child, end));
                    }
                }
                self.filter(scope, |ordinal| {
                    let offset = ranges.partition_point(|(start, _)| *start <= ordinal);
                    offset != 0 && ordinal < ranges[offset - 1].1
                })
            }
            Direction::Precedes | Direction::Follows => {
                unreachable!("fields belong to parent edges")
            }
        }
    }
}
