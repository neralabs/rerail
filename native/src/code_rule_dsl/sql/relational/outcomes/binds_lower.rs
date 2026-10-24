//! Child-free binding filters are atomic predicates with explicit optional node-region inputs.
use super::super::{CaptureKind, Op, Plan, Slots, effects::Effects};
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
        if let Some(name) = test
            .filter
            .bindings
            .as_ref()
            .and_then(|count| count.within.as_ref())
        {
            // Policy validation permits single captures only. Parsed spans
            // share that typed column, with an optional AST node shadow.
            let slot = self.slot(name, CaptureKind::Node)?;
            let inputs = vec![slot];
            let bindings = vec![(
                name.clone(),
                crate::code_rule_dsl::sql::plan::SlotKind::Node,
            )];
            let term = self.pure.program.terminals.len();
            self.pure
                .program
                .terminals
                .push(crate::code_rule_dsl::sql::terminals::Terminal::Fact(
                    crate::code_rule_dsl::sql::terminals::FactTerm {
                        reads: bindings.clone(),
                        verdicts: Box::new(super::binding_region::RegionFacts {
                            test: (**test).clone(),
                            bindings,
                            owner: self.scope_owner,
                        }),
                    },
                ));
            return Ok(Plan {
                op: Op::BoundLeaf {
                    term,
                    inputs,
                    outputs: vec![],
                    kinds: Some(test.name_kinds.iter().copied().collect()),
                },
                effects: Effects {
                    reads_value: true,
                    fallible: true,
                    cache_fills: true,
                    ..Effects::default()
                },
                mentions: Slots::from([slot]),
                writes: Slots::new(),
                node_writes: Slots::new(),
                walk_filter: None,
            });
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
