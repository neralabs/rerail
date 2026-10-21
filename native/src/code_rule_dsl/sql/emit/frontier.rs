//! Progressive evaluation over stable candidate keys. Each phase evaluates a growing prefix,
//! including the earlier candidates. The last phase therefore owns a coherent set of frames
//! for deferred recursive lineage. No dependency can refer to an overwritten phase's keys.
//!
//! Prefixes grow independently per parent; a decided parent's prefix stops growing. A growing
//! parent replays less than twice its domain before exhaustion, skipping the penultimate phase
//! when it would nearly exhaust the domain. Closed prefixes can replay while other parents grow.
//! Recursive demand and pattern caches survive phases. A changed recursive answer
//! reruns the enclosing member with new frontiers, including its earlier failed candidates.

use super::{Emitter, FRONTIER, Live, Reads, UNTIL, Writes};
use crate::code_rule_dsl::sql::plan::{OfRule, Op, Walk};

pub(super) struct Frontier {
    all: String,
    candidates: String,
}

/// Set-only tests already run in batches. Schedule bodies that produce recursive or semantic
/// demands, including demands nested under arbitrary compiler operators.
fn demands(op: &Op) -> bool {
    match op {
        Op::Recur { .. } | Op::Fact { .. } => true,
        Op::All(items) | Op::Any(items) | Op::Seq(items) => items.iter().any(demands),
        Op::AllOpen(items) => items.iter().any(|term| demands(&term.op)),
        Op::AnyOpen(items) => items.iter().any(|(op, _)| demands(op)),
        Op::Not(op) | Op::Unknowable { op, .. } => demands(op),
        Op::Relation { sub, .. }
        | Op::Search { sub, .. }
        | Op::Count { sub, .. }
        | Op::Every { sub, .. }
        | Op::Follow { sub, .. } => demands(sub),
        Op::Call { body, .. } | Op::Parse { body, .. } => demands(body),
        Op::Constrained { rule, constraints } => {
            demands(rule) || constraints.iter().any(|(_, op)| demands(op))
        }
        Op::Host { rule, .. } => demands(rule),
        Op::NthCommit { nth, commit } => demands(nth) || demands(commit),
        Op::NthChild {
            of_rule: Some(OfRule::Frame(op)),
            ..
        } => demands(op),
        _ => false,
    }
}

impl Emitter<'_> {
    /// Open a phase loop. Single-candidate walks need no scheduler.
    pub(super) fn frontier(
        &mut self,
        walk: Walk,
        sub: &Op,
        all: &str,
        held: &Live,
    ) -> (String, Option<Frontier>) {
        if walk.single() || !demands(sub) {
            return (all.to_owned(), None);
        }
        let candidates = self.within(held, |this| {
            let name = this.name("prefix");
            this.create(
                &name,
                &format!(
                    "k INTEGER PRIMARY KEY, file, node{}, pk, ord{}",
                    this.physical_list(),
                    this.extra_list()
                ),
            );
            name
        });
        self.same_space(all, &candidates);
        self.looping += 1;
        self.push(
            FRONTIER.to_owned(),
            Reads::join(&[all]).writes(Writes::Insert(candidates.clone())),
        );
        (
            candidates.clone(),
            Some(Frontier {
                all: all.to_owned(),
                candidates,
            }),
        )
    }

    /// Close parents with a decisive first event. Unknown stops may close a strict search only
    /// after every earlier candidate was evaluated. Non-strict searches require a true match.
    pub(super) fn frontier_search(
        &mut self,
        frontier: Option<Frontier>,
        candidates: &str,
        matched: &str,
        strict: bool,
        opened: Option<&str>,
    ) {
        let Some(frontier) = frontier else { return };
        debug_assert_eq!(candidates, frontier.candidates);
        self.frontier_end(
            frontier,
            matched,
            opened,
            if strict { "strict" } else { "search" },
            0,
            None,
        );
    }

    /// An exact count exhausts its domain. A predicate can stop once its true lower bound
    /// proves a minimum-only success or an over-maximum failure, even with unknown candidates.
    pub(super) fn frontier_count(
        &mut self,
        frontier: Option<Frontier>,
        matched: &str,
        min: usize,
        max: Option<usize>,
    ) {
        let Some(frontier) = frontier else { return };
        self.frontier_end(frontier, matched, None, "count", min, max);
    }

    fn frontier_end(
        &mut self,
        frontier: Frontier,
        matched: &str,
        opened: Option<&str>,
        mode: &str,
        min: usize,
        max: Option<usize>,
    ) {
        let Frontier { all, candidates } = frontier;
        let mut inputs = vec![all.as_str(), candidates.as_str(), matched];
        inputs.extend(opened);
        self.push(
            format!(
                "{UNTIL}{FRONTIER}{all} {candidates} {matched} {} {mode} {} {min} {}",
                opened.unwrap_or("-"),
                usize::from(self.three),
                max.map_or_else(|| "-".to_owned(), |n| n.to_string())
            ),
            Reads::join(&inputs).writes(Writes::None),
        );
        self.looping -= 1;
    }
}
