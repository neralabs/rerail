//! Only capture-independent, node-local binding filters are atomic predicates.
use super::super::{Plan, Slots, effects::Effects};
use super::Lower;
use crate::code_rule_dsl::matcher::{AstMatcher, Semantic};

impl Lower {
    pub(super) fn binds(&mut self, matcher: &AstMatcher) -> Result<Plan, String> {
        let AstMatcher::Semantic(Semantic::Binds(test)) = matcher else {
            unreachable!("binding predicate lowering requires binds")
        };
        if test.sub_rules().next().is_some() || test.redefines.is_some() {
            return Err(
                "outcome binds: binding sub-rules and redefinitions require SQL operators".into(),
            );
        }
        if test
            .filter
            .bindings
            .as_ref()
            .is_some_and(|count| count.within.is_some())
        {
            return Err(
                "outcome binds: captured binding regions require typed input columns".into(),
            );
        }
        // Binds::at always reads both the scope table and its usage table. Their
        // clause-owned fill charges must not be replayed as utility answer work.
        let op = self.atomic(matcher);
        Ok(Plan {
            op,
            effects: Effects {
                reads_value: true,
                fallible: true,
                cache_fills: true,
                ..Effects::default()
            },
            mentions: Slots::new(),
            writes: Slots::new(),
            node_writes: Slots::new(),
            walk_filter: None,
        })
    }
}
