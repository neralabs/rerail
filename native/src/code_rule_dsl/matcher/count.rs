use super::AstMatcher;
use super::relation::descend_until;
use super::util::{THREE_VALUED, UNKNOWN, UTIL_EXCEEDED};
use super::{StopBy, spend_util_step};
use crate::code_rule_dsl::budget;
use ast_grep_core::matcher::Matcher;
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::source::Content;
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::cell::Cell;
use std::ops::ControlFlow;

/// `count`: how many nodes along a relation's walk match its rule, bounded by `min`/`max`
/// (a Rerail extension). The walk is the relation's own, except that `has` keeps descending
/// below a match (every matching descendant counts) and stops only where `stopBy` says. The
/// rule reads the caller's bindings; like `not`, nothing it binds is kept. With `as`, the count
/// itself is bound as text (`CountBinding`).
#[derive(Clone)]
pub struct Count {
    /// `Inside`, `Has`, `Precedes` or `Follows`.
    pub relation: AstMatcher,
    pub min: usize,
    pub max: Option<usize>,
    pub binding: Option<CountBinding>,
}

/// `as` and `offset`: a count that holds binds `offset` plus the exact count, in decimal, as a
/// transformed variable named `name`, which messages and source-text comparisons read. The walk then runs to
/// its end rather than stopping at `min`, and a count it cannot know exactly is unknown.
#[derive(Clone, Debug)]
pub struct CountBinding {
    pub name: String,
    pub offset: i64,
}

impl CountBinding {
    fn bind<D: Doc>(&self, env: &mut Cow<MetaVarEnv<'_, D>>, found: usize) {
        let value = i128::try_from(found).unwrap_or(i128::MAX) + i128::from(self.offset);
        let bytes = <D::Source as Content>::decode_str(&value.to_string()).into_owned();
        env.to_mut()
            .insert_transformation(&MetaVariable::Multiple, &self.name, bytes);
    }
}

impl Count {
    /// Three-valued: a node whose verdict is unknown may or may not count, and an unknown stop
    /// verdict may hide any number of further nodes. When the bounds could go either way the count
    /// fails with `UNKNOWN` set; so does a bound count (`as`) whose exact value is not known.
    pub(super) fn holds<'tree, D: Doc>(
        &self,
        node: &Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> bool {
        let three = THREE_VALUED.get();
        let saved = three && UNKNOWN.replace(false);
        let mut found = 0usize;
        let mut unknown = 0usize;
        // To the end, a rule looking for a sibling with the same key as a capture bound outside
        // it is tested at the siblings it may match only (`Related::same_key_siblings`).
        let listed = match &self.relation {
            AstMatcher::Precedes(rule, StopBy::End) => rule.same_key_siblings(node, env, true),
            AstMatcher::Follows(rule, StopBy::End) => rule.same_key_siblings(node, env, false),
            _ => None,
        };
        // Each tested node spends a step; a spent budget fails the clause for the file.
        let mut test = |rule: &AstMatcher, candidate: Node<'tree, D>| -> ControlFlow<()> {
            if !spend_util_step() || budget::matcher_call_budget_exhausted() {
                return ControlFlow::Break(());
            }
            let mut probe = Cow::Borrowed(env.as_ref());
            if rule.match_node_with_env(candidate, &mut probe).is_some() {
                found += 1;
                // Past `max` the answer is no; at `min` with no `max` it is yes, unless the
                // count is bound, which needs every node counted.
                if self.max.is_some_and(|max| found > max)
                    || (self.max.is_none() && self.binding.is_none() && found >= self.min)
                {
                    return ControlFlow::Break(());
                }
            } else if three && UNKNOWN.replace(false) {
                unknown += 1;
            }
            ControlFlow::Continue(())
        };
        let open = Cell::new(false);
        let walked = match (&self.relation, listed) {
            (AstMatcher::Precedes(rule, _) | AstMatcher::Follows(rule, _), Some(listed)) => {
                listed.into_iter().try_for_each(|n| test(rule, n))
            }
            (AstMatcher::Inside(rule, stop), _) => stop.walk(
                || node.parent(),
                || node.ancestors(),
                |n| test(rule, n),
                &open,
                three,
            ),
            (AstMatcher::Precedes(rule, stop), _) => stop.walk(
                || node.next(),
                || node.next_all(),
                |n| test(rule, n),
                &open,
                three,
            ),
            (AstMatcher::Follows(rule, stop), _) => stop.walk(
                || node.prev(),
                || node.prev_all(),
                |n| test(rule, n),
                &open,
                three,
            ),
            (AstMatcher::Has(rule, stop), _) => match stop {
                StopBy::Neighbor => node.children().try_for_each(|n| test(rule, n)),
                StopBy::End => node.dfs().skip(1).try_for_each(|n| test(rule, n)),
                StopBy::Rule(stop) => descend_until(node, stop, |n| test(rule, n), &open, three),
            },
            _ => {
                if three {
                    UNKNOWN.set(saved);
                }
                return false;
            }
        };
        if three {
            UNKNOWN.set(saved);
        }
        // A bound count breaks off before its end only past `max`; any other break is a spent
        // budget, and a partial count is no value to report.
        if walked.is_break()
            && (UTIL_EXCEEDED.get().is_some()
                || (self.binding.is_some() && self.max.is_none_or(|max| found <= max)))
        {
            return false;
        }
        // The true count lies in `found..=found + unknown`, unbounded above after an unknown stop.
        let high = (!open.get()).then_some(found + unknown);
        let exact = self.binding.is_none() || high == Some(found);
        let holds = exact
            && found >= self.min
            && self
                .max
                .is_none_or(|max| high.is_some_and(|high| high <= max));
        if holds {
            if let Some(binding) = &self.binding {
                binding.bind(env, found);
            }
            return true;
        }
        let fails =
            high.is_some_and(|high| high < self.min) || self.max.is_some_and(|max| found > max);
        if !fails && three {
            UNKNOWN.set(true);
        }
        false
    }
}

#[cfg(test)]
mod tests;
