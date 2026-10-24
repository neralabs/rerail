//! Binding targets are atomic facts; their child programs remain SQL-owned.
use super::super::{CaptureKind, Op, Plan, Slots, effects::Effects};
use super::Lower;
use crate::code_rule_dsl::{
    matcher::{
        self, AstMatcher, BindingTest, Semantic,
        sql_binding_facts::{BindingFacts, binding_facts_budget},
        with_sql_clause_owner,
    },
    sql::{
        plan::SlotKind,
        terminals::{FactTerm, Terminal, Verdict, Verdicts, values_of},
    },
    tape::TapeDoc,
};
use ast_grep_core::{Node, meta_var::MetaVarEnv};
use rusqlite::types::Value;

struct Facts {
    test: BindingTest,
    bindings: Vec<(String, SlotKind)>,
    owner: usize,
}
impl Verdicts for Facts {
    fn rows<'t>(
        &self,
        _: Node<'t, TapeDoc>,
        _: &MetaVarEnv<'t, TapeDoc>,
    ) -> Result<Vec<Verdict>, String> {
        Err("binding facts require a finite allowance".into())
    }
    fn budget_rows<'t>(
        &self,
        node: Node<'t, TapeDoc>,
        env: &MetaVarEnv<'t, TapeDoc>,
        work: usize,
    ) -> (Result<Vec<Verdict>, String>, usize) {
        with_sql_clause_owner(self.owner, || {
            let (facts, left) = binding_facts_budget(&self.test, node, env, work);
            let rows = match facts {
                Err(limit) => Err(format!("{limit:?}")),
                Ok(BindingFacts::No) => Ok(vec![]),
                Ok(BindingFacts::Unknown) => Ok(vec![Verdict::unknown()]),
                Ok(BindingFacts::Found {
                    targets,
                    occurrences,
                }) => {
                    let mut values: Vec<_> = targets
                        .into_iter()
                        .map(|id| id.map_or(Value::Null, |id| Value::Integer(i64::from(id))))
                        .collect();
                    values.push(Value::Text(
                        serde_json::to_string(&occurrences).expect("node ids serialize"),
                    ));
                    values.extend(values_of(&self.bindings[4..], env));
                    Ok(vec![Verdict::holds(values)])
                }
            };
            (rows, left)
        })
    }
}
impl Lower {
    pub(super) fn binding(&mut self, matcher: &AstMatcher) -> Result<Plan, String> {
        let AstMatcher::Semantic(Semantic::Binding(test)) = matcher else {
            unreachable!()
        };
        let serial = self.pure.program.terminals.len();
        let mut bindings = Vec::new();
        let mut inputs = Vec::new();
        for index in 0..4 {
            let name = format!("\0binding{serial}_{index}");
            inputs.push(self.slot(&name, CaptureKind::Node)?);
            bindings.push((name, SlotKind::Node));
        }
        if let Some(name) = test
            .filter
            .bindings
            .as_ref()
            .and_then(|count| count.within.as_ref())
        {
            inputs.push(self.slot(name, CaptureKind::Node)?);
            bindings.push((name.clone(), SlotKind::Node));
        }
        let term = self.pure.program.terminals.len();
        self.pure.program.terminals.push(Terminal::Fact(FactTerm {
            reads: bindings.clone(),
            verdicts: Box::new(Facts {
                test: (**test).clone(),
                bindings,
                owner: self.scope_owner,
            }),
        }));
        let mut children = Vec::new();
        for (index, rule) in [&test.site, &test.value, &test.annotation]
            .into_iter()
            .enumerate()
        {
            if let Some(rule) = rule {
                self.following_origin(inputs[index], std::ptr::from_ref(rule) as usize);
                children.push((inputs[index], self.plan(rule)?));
            }
        }
        let occurrence = test
            .occurrence
            .as_ref()
            .map(|rule| self.plan(rule).map(Box::new))
            .transpose()?;
        // A finite syntax tree and an acyclic utility DAG cannot revisit an active rule
        // identity. Recursive components need an explicit FOLLOWING stack contract.
        if children.iter().any(|(_, child)| recursive_following(child))
            || occurrence
                .as_ref()
                .is_some_and(|child| recursive_following(child))
        {
            return Err(
                "outcome binding: recursive following requires a cycle-key contract".into(),
            );
        }
        let internal: Slots = inputs[..4].iter().copied().collect();
        let mut mentions: Slots = inputs[4..].iter().copied().collect();
        let mut writes = Slots::new();
        let mut node_writes = Slots::new();
        let mut effects = Effects {
            reads_value: matcher::reads_values(matcher, &mut Default::default()),
            fallible: true,
            cache_fills: true,
            typed_exports: true,
            ..Effects::default()
        };
        for child in children
            .iter()
            .map(|(_, child)| child)
            .chain(occurrence.iter().map(Box::as_ref))
        {
            mentions.extend(child.mentions.difference(&internal));
            writes.extend(&child.writes);
            node_writes.extend(&child.node_writes);
            effects.uses_memo |= child.uses_memo();
            effects.file_memo |= child.uses_memo() || child.effects.file_memo;
            effects.contains_chain |= child.effects.contains_chain;
            effects.needs_peak |= child.effects.needs_peak;
            effects.walk_depth = effects.walk_depth.max(child.effects.walk_depth + 1);
        }
        let core = Plan {
            op: Op::BoundLeaf {
                term,
                inputs: inputs.clone(),
                outputs: inputs.clone(),
                kinds: None,
            },
            effects: Effects {
                fallible: true,
                cache_fills: true,
                typed_exports: true,
                ..Effects::default()
            },
            mentions: inputs.iter().copied().collect(),
            writes: inputs.iter().copied().collect(),
            node_writes: inputs.iter().copied().collect(),
            walk_filter: None,
        };
        Ok(Plan {
            op: Op::Binding {
                core: Box::new(core),
                children,
                occurrence_slot: inputs[3],
                occurrence,
            },
            effects,
            mentions,
            writes,
            node_writes,
            walk_filter: None,
        })
    }
}

// Pure chain/downward components contain no binding rules. A semantic recursive
// base may do so, and needs a dynamic active-following key before nesting here.
fn recursive_following(plan: &Plan) -> bool {
    any_plan(plan, |plan| match &plan.op {
        Op::RecursiveSearch { component, .. } => {
            any_plan(&component.base, |p| matches!(p.op, Op::Binding { .. }))
        }
        _ => false,
    })
}
fn any_plan(plan: &Plan, predicate: fn(&Plan) -> bool) -> bool {
    if predicate(plan) {
        return true;
    }
    match &plan.op {
        Op::Binding {
            core,
            children,
            occurrence,
            ..
        } => {
            any_plan(core, predicate)
                || children.iter().any(|(_, p)| any_plan(p, predicate))
                || occurrence.as_ref().is_some_and(|p| any_plan(p, predicate))
        }
        Op::FormatArgs {
            unused, missing, ..
        } => {
            unused.as_ref().is_some_and(|p| any_plan(p, predicate))
                || missing.as_ref().is_some_and(|p| any_plan(p, predicate))
        }
        Op::RecursiveSearch { component, .. } => any_plan(&component.base, predicate),
        Op::All(items) | Op::Any(items) => items.iter().any(|p| any_plan(p, predicate)),
        Op::Shared(child)
        | Op::Charge(_, child)
        | Op::MemoPredicate(_, child)
        | Op::CallFrame { body: child, .. } => any_plan(child, predicate),
        Op::Not(child)
        | Op::Parse { child, .. }
        | Op::FieldNeighbor { child, .. }
        | Op::FieldEnd { child, .. } => any_plan(child, predicate),
        Op::Constrained(core, constraints) => {
            any_plan(core, predicate) || constraints.iter().any(|(_, p)| any_plan(p, predicate))
        }
        Op::Search(_, _, child, stop)
        | Op::Count {
            rule: child, stop, ..
        } => any_plan(child, predicate) || stop.as_ref().is_some_and(|p| any_plan(p, predicate)),
        _ => false,
    }
}
