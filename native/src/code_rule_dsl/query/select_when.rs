//! `select_when` evaluated over its leaves' rows.
//!
//! A `select_when` reports its `select` side's witnesses when its `exists` side has a witness
//! anywhere. Its sides are regex or AST queries (leaves), or `select_when`s themselves. A leaf is
//! read file by file as `LeafRow`s: whether the leaf failed in the file (its witnesses there are
//! unknown), whether the file was read around syntax errors, and how many witnesses the file
//! holds. `answer` decides everything else: which sides are read at all, whether the answer is
//! known (and if not, at which failed row), which witnesses are reported (a witness budget's
//! worth, in row order) and which partially read files the answer relied on.
//!
//! Rows come from the caller (`Leaves`), in the order the caller reads the leaf. Sides are read
//! depth first, `exists` before `select`, and a `select` side only where its `exists` side is
//! known to hold a witness, so leaves are read in `Plan::leaves` order. A leaf read for a decision
//! (an `exists` side, or the whole query in decision mode) ends with its first row that holds a
//! witness. Each row is read at most once, so a caller that evaluates a leaf as its rows are
//! asked for does no work the answer does not need.

use crate::code_rule_dsl::model::Query;

/// One file of a leaf.
#[derive(Clone, Copy, Debug)]
pub(in crate::code_rule_dsl) struct LeafRow {
    /// The leaf could not answer in this file: its witnesses there are unknown.
    pub(in crate::code_rule_dsl) failed: bool,
    /// The file was read around syntax errors; reported `source-partial` if the answer uses it.
    pub(in crate::code_rule_dsl) partial: bool,
    pub(in crate::code_rule_dsl) witnesses: usize,
}

/// The rows of a plan's leaves.
pub(in crate::code_rule_dsl) trait Leaves {
    /// Row `position` of leaf `leaf` (in `Plan::leaves` order), or `None` past its last row.
    /// `Err` ends the answer with that message.
    fn row(&self, leaf: usize, position: usize) -> Result<Option<LeafRow>, String>;
}

/// A leaf of a plan: the query it reads, whether it is read for a decision, and whether its
/// witnesses may be reported (no `exists` side above it) or only decide a condition.
pub(in crate::code_rule_dsl) struct Leaf<L> {
    pub(in crate::code_rule_dsl) id: L,
    pub(in crate::code_rule_dsl) decision: bool,
    pub(in crate::code_rule_dsl) reported: bool,
}

/// A query's shape and its leaves.
pub(in crate::code_rule_dsl) struct Plan<L> {
    pub(in crate::code_rule_dsl) leaves: Vec<Leaf<L>>,
    /// The query's nodes in pre-order: a `select_when`'s `exists` side follows it directly.
    nodes: Vec<Node>,
}

#[derive(Clone, Copy)]
enum Node {
    /// Index into `Plan::leaves`.
    Leaf(usize),
    /// A `select_when`, with the node index of its `select` side.
    SelectWhen { select: usize },
}

/// What a plan answered, where known: the witnesses reported, as `(leaf, row, how many of the
/// row's witnesses)` in order, and the partially read rows it relied on.
pub(in crate::code_rule_dsl) struct Answer {
    pub(in crate::code_rule_dsl) witnesses: Vec<(usize, usize, usize)>,
    pub(in crate::code_rule_dsl) partial: Vec<(usize, usize)>,
}

/// Where a plan's answer is unknown: the failed `(leaf, row)` it could not do without, the first
/// failed row of the leaf that left it unknown.
pub(in crate::code_rule_dsl) type Unknown = (usize, usize);

impl<L> Plan<L> {
    /// `query`'s plan; `decision` when a single witness answers it. `leaf` names each regex or
    /// AST query, given whether its witnesses may be reported.
    pub(in crate::code_rule_dsl) fn new<'q>(
        query: &'q Query,
        decision: bool,
        mut leaf: impl FnMut(&'q Query, bool) -> L,
    ) -> Self {
        let mut plan = Self {
            leaves: Vec::new(),
            nodes: Vec::new(),
        };
        plan.node(query, decision, true, &mut leaf);
        plan
    }

    fn node<'q>(
        &mut self,
        query: &'q Query,
        decision: bool,
        reported: bool,
        leaf: &mut impl FnMut(&'q Query, bool) -> L,
    ) {
        match query {
            Query::SelectWhen { select, exists } => {
                let node = self.nodes.len();
                self.nodes.push(Node::SelectWhen { select: 0 });
                self.node(exists, true, false, leaf);
                self.nodes[node] = Node::SelectWhen {
                    select: self.nodes.len(),
                };
                self.node(select, decision, reported, leaf);
            }
            Query::Regex { .. } | Query::Ast { .. } => {
                self.nodes.push(Node::Leaf(self.leaves.len()));
                self.leaves.push(Leaf {
                    id: leaf(query, reported),
                    decision,
                    reported,
                });
            }
        }
    }

    /// Node `node`'s answer: `Err` at the failed row where unknown, else whether it holds a
    /// witness (known only for a node read for a decision; `false` otherwise). Appends the
    /// witnesses and partial rows of every leaf it reads to `answer`.
    fn evaluate(
        &self,
        node: usize,
        cap: usize,
        leaves: &dyn Leaves,
        answer: &mut Answer,
    ) -> Result<Result<bool, Unknown>, String> {
        match self.nodes[node] {
            Node::Leaf(leaf) => self.read(leaf, cap, leaves, answer),
            Node::SelectWhen { select } => match self.evaluate(node + 1, cap, leaves, answer)? {
                Ok(true) => self.evaluate(select, cap, leaves, answer),
                condition => Ok(condition),
            },
        }
    }

    /// Reads leaf `leaf`. Read for a decision, it ends at its first row holding a witness (one
    /// witness of it reported) and is unknown only where it has none and a row failed (at its
    /// first failed row). Otherwise it is unknown at its first failed row, and reports its rows'
    /// witnesses in order until `cap` of them are taken; the row where the budget runs out gives
    /// only what is left of it.
    fn read(
        &self,
        leaf: usize,
        cap: usize,
        leaves: &dyn Leaves,
        answer: &mut Answer,
    ) -> Result<Result<bool, Unknown>, String> {
        let Leaf {
            decision, reported, ..
        } = self.leaves[leaf];
        let mut failed = None;
        let mut before = 0usize;
        let mut position = 0;
        while let Some(row) = leaves.row(leaf, position)? {
            if row.partial {
                answer.partial.push((leaf, position));
            }
            if decision {
                if row.witnesses > 0 {
                    if reported {
                        answer.witnesses.push((leaf, position, 1));
                    }
                    return Ok(Ok(true));
                }
                if row.failed {
                    failed.get_or_insert(position);
                }
            } else {
                if row.failed {
                    return Ok(Err((leaf, position)));
                }
                if reported && row.witnesses > 0 && before < cap {
                    answer
                        .witnesses
                        .push((leaf, position, row.witnesses.min(cap - before)));
                }
                before = before.saturating_add(row.witnesses);
            }
            position += 1;
        }
        Ok(failed.map_or(Ok(false), |position| Err((leaf, position))))
    }
}

/// Answers `plan` over `leaves`' rows with a witness budget of `cap`: `Err` at the failed row
/// where the answer is unknown. Only a reported leaf not read for a decision counts the budget.
pub(in crate::code_rule_dsl) fn answer<L>(
    plan: &Plan<L>,
    cap: usize,
    leaves: &dyn Leaves,
) -> Result<Result<Answer, Unknown>, String> {
    let mut answer = Answer {
        witnesses: Vec::new(),
        partial: Vec::new(),
    };
    Ok(plan.evaluate(0, cap, leaves, &mut answer)?.map(|_| answer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::model::RegexSource;
    use regex::Regex;
    use std::cell::RefCell;

    fn leaf() -> Query {
        Query::Regex {
            regex: Regex::new("x").unwrap(),
            source: RegexSource::Content,
        }
    }

    fn when(exists: Query, select: Query) -> Query {
        Query::SelectWhen {
            select: Box::new(select),
            exists: Box::new(exists),
        }
    }

    fn row(witnesses: usize) -> LeafRow {
        LeafRow {
            failed: false,
            partial: false,
            witnesses,
        }
    }

    const FAILED: LeafRow = LeafRow {
        failed: true,
        partial: false,
        witnesses: 0,
    };

    const PARTIAL: LeafRow = LeafRow {
        failed: false,
        partial: true,
        witnesses: 0,
    };

    /// Fixed rows per leaf (in plan order), recording how far each leaf was read.
    struct Fixed {
        rows: Vec<Vec<LeafRow>>,
        read: RefCell<Vec<usize>>,
    }

    impl Fixed {
        fn new(rows: Vec<Vec<LeafRow>>) -> Self {
            let read = RefCell::new(vec![0; rows.len()]);
            Self { rows, read }
        }
    }

    impl Leaves for Fixed {
        fn row(&self, leaf: usize, position: usize) -> Result<Option<LeafRow>, String> {
            let read = &mut self.read.borrow_mut()[leaf];
            *read = (*read).max(position + 1).min(self.rows[leaf].len());
            Ok(self.rows[leaf].get(position).copied())
        }
    }

    fn run(query: &Query, decision: bool, cap: usize, leaves: &Fixed) -> Result<Answer, Unknown> {
        let plan = Plan::new(query, decision, |_, reported| reported);
        answer(&plan, cap, leaves).unwrap()
    }

    #[test]
    fn a_condition_without_a_witness_leaves_the_select_side_unread() {
        let leaves = Fixed::new(vec![vec![PARTIAL, row(0)], vec![row(3)]]);
        let answer = run(&when(leaf(), leaf()), false, 10, &leaves).expect("known");
        assert!(answer.witnesses.is_empty());
        assert_eq!(answer.partial, [(0, 0)]);
        assert_eq!(*leaves.read.borrow(), [2, 0]);
    }

    #[test]
    fn an_unknown_condition_is_the_answer_and_leaves_the_select_side_unread() {
        let leaves = Fixed::new(vec![vec![FAILED, PARTIAL, FAILED], vec![row(3)]]);
        // Unknown at the condition's first failed row.
        assert_eq!(
            run(&when(leaf(), leaf()), false, 10, &leaves).err(),
            Some((0, 0))
        );
        assert_eq!(leaves.read.borrow()[1], 0);
    }

    #[test]
    fn a_witness_decides_a_condition_despite_failed_files_before_it() {
        // Read for a decision, the condition's rows end at its witness.
        let leaves = Fixed::new(vec![vec![FAILED, PARTIAL, row(1)], vec![row(2), PARTIAL]]);
        let answer = run(&when(leaf(), leaf()), false, 10, &leaves).expect("known");
        assert_eq!(answer.witnesses, [(1, 0, 2)]);
        assert_eq!(answer.partial, [(0, 1), (1, 1)]);
    }

    #[test]
    fn a_failed_select_file_makes_the_answer_unknown() {
        let leaves = Fixed::new(vec![vec![row(1)], vec![row(2), FAILED, row(1)]]);
        assert_eq!(
            run(&when(leaf(), leaf()), false, 10, &leaves).err(),
            Some((1, 1))
        );
    }

    #[test]
    fn the_witness_budget_takes_rows_in_order() {
        let leaves = Fixed::new(vec![vec![row(2), row(0), row(3), row(5)]]);
        let answer = run(&leaf(), false, 4, &leaves).expect("known");
        assert_eq!(answer.witnesses, [(0, 0, 2), (0, 2, 2)]);
    }

    #[test]
    fn a_decision_leaf_is_not_read_past_its_first_witness() {
        let leaves = Fixed::new(vec![vec![row(0), row(1), PARTIAL], vec![row(2), PARTIAL]]);
        let answer = run(&when(leaf(), leaf()), true, 10, &leaves).expect("known");
        assert_eq!(answer.witnesses, [(1, 0, 1)]);
        assert!(answer.partial.is_empty());
        assert_eq!(*leaves.read.borrow(), [2, 1]);
    }

    #[test]
    fn a_decision_reports_one_witness() {
        let leaves = Fixed::new(vec![vec![row(1)], vec![PARTIAL, row(3)]]);
        let answer = run(&when(leaf(), leaf()), true, 10, &leaves).expect("known");
        assert_eq!(answer.witnesses, [(1, 1, 1)]);
    }

    #[test]
    fn nested_conditions_gate_in_order() {
        // exists: (select_when exists: leaf 0, select: leaf 1); select: leaf 2.
        let query = when(when(leaf(), leaf()), leaf());
        // Leaf 0 holds a witness, leaf 1 none: the outer condition is empty, leaf 2 unread.
        let leaves = Fixed::new(vec![vec![row(1)], vec![PARTIAL], vec![row(4)]]);
        let answer = run(&query, false, 10, &leaves).expect("known");
        assert!(answer.witnesses.is_empty());
        assert_eq!(answer.partial, [(1, 0)]);
        assert_eq!(leaves.read.borrow()[2], 0);
        // Leaf 1 holds one too: leaf 2 is reported.
        let leaves = Fixed::new(vec![vec![row(1)], vec![row(1)], vec![row(4)]]);
        let answer = run(&query, false, 10, &leaves).expect("known");
        assert_eq!(answer.witnesses, [(2, 0, 4)]);
    }
}
