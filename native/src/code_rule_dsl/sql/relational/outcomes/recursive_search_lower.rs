//! Semantic base programs with a pure, ordered recursive continuation.
use super::{Lower, Op, Plan, Rc};
use crate::code_rule_dsl::{
    matcher::{self, AstMatcher, StopBy, UtilRef},
    sql::relational::{ir::Set, recursive_chain::pure, recursive_downward::Downward},
};

pub(in crate::code_rule_dsl::sql::relational::outcomes) struct RecursiveSearch {
    pub base: Plan,
    pub guard: Set,
    pub child_kinds: Option<Vec<u16>>,
}
impl Lower {
    pub(super) fn recursive_search(&mut self, util: &UtilRef) -> Result<Option<Op>, String> {
        let key = (util.table.as_ptr() as usize, util.index);
        let scoped = (key.0, key.1, self.namespace.clone());
        if let Some((id, component)) = self.recursive_searches.get(&scoped) {
            return Ok(Some(Op::RecursiveSearch {
                id: *id,
                component: Rc::clone(component),
            }));
        }
        // Native uses a different cache contract for value-reading recursion.
        // Such programs must not silently use a non-memo schedule.
        if matcher::reads_values(&AstMatcher::Util(util.clone()), &mut Default::default()) {
            return Ok(None);
        }
        util.with_target(|body| {
            let AstMatcher::Any(items) = body else {
                return Ok(None);
            };
            if items.len() < 2 {
                return Ok(None);
            }
            let (guards, last) = match items.last().unwrap() {
                AstMatcher::All(terms) if !terms.is_empty() => {
                    (&terms[..terms.len() - 1], terms.last().unwrap())
                }
                other => (&[][..], other),
            };
            let AstMatcher::Has(child, StopBy::Neighbor) = last else {
                return Ok(None);
            };
            let AstMatcher::Util(next) = child.rule() else {
                return Ok(None);
            };
            if (next.table.as_ptr() as usize, next.index) != key
                || guards.iter().any(|g| pure(g).is_err())
            {
                return Ok(None);
            }
            let guard = Set::Intersect(
                guards
                    .iter()
                    .map(|g| self.pure.set(g))
                    .collect::<Result<_, _>>()?,
            );
            let structural = Downward {
                base: Set::Empty,
                guard: guard.clone(),
                child_kinds: None,
            };
            if structural.guard_kinds().is_none() {
                return Ok(None);
            }
            if !self.active.insert(key) {
                return Err("recursive search: cyclic base dependency".into());
            }
            let base = if items.len() == 2 {
                self.plan(&items[0])
            } else {
                self.plan(&AstMatcher::Any(items[..items.len() - 1].to_vec()))
            };
            self.active.remove(&key);
            let base = base?;
            if base.uses_memo() {
                return Err("recursive search: memoized base needs a depth contract".into());
            }
            let child_kinds = if self.three_valued {
                None
            } else {
                matcher::walk_candidate_filter(&AstMatcher::Util(util.clone())).map(|(k, _)| k)
            };
            let id = self.recursive_searches.len();
            let component = Rc::new(RecursiveSearch {
                base,
                guard,
                child_kinds,
            });
            self.recursive_searches
                .insert(scoped, (id, Rc::clone(&component)));
            Ok(Some(Op::RecursiveSearch { id, component }))
        })
        .unwrap_or(Ok(None))
    }
}
