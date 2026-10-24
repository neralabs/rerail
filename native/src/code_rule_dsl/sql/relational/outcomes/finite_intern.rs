//! Bounded exact instruction interning; every ordered parent edge remains intact.
use super::super::{Op, Plan, WalkFilter};
use super::{Instruction, Node};
fn filter(a: Option<&WalkFilter>, b: Option<&WalkFilter>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.kinds == b.kinds && a.text == b.text,
        _ => false,
    }
}
fn core(a: &Plan, b: &Plan) -> bool {
    let (
        Op::BoundLeaf {
            term: at,
            inputs: ai,
            outputs: ao,
            kinds: ak,
        },
        Op::BoundLeaf {
            term: bt,
            inputs: bi,
            outputs: bo,
            kinds: bk,
        },
    ) = (&a.op, &b.op)
    else {
        return false;
    };
    at == bt
        && ai == bi
        && ao == bo
        && ak == bk
        && a.mentions == b.mentions
        && a.writes == b.writes
        && a.node_writes == b.node_writes
        && a.effects == b.effects
        && filter(a.walk_filter.as_ref(), b.walk_filter.as_ref())
}
fn instruction(a: &Instruction, b: &Instruction) -> bool {
    match (a, b) {
        (Instruction::Eval(a), Instruction::Eval(b))
        | (Instruction::Call(a), Instruction::Call(b))
        | (Instruction::Not(a), Instruction::Not(b)) => a == b,
        (
            Instruction::Boolean {
                all: a,
                children: ac,
            },
            Instruction::Boolean {
                all: b,
                children: bc,
            },
        ) => a == b && ac == bc,
        (
            Instruction::Search {
                axis: a,
                end: ae,
                field: af,
                body: ab,
                stop: as_,
                bounds: ac,
                filter: ap,
            },
            Instruction::Search {
                axis: b,
                end: be,
                field: bf,
                body: bb,
                stop: bs,
                bounds: bc,
                filter: bp,
            },
        ) => {
            std::mem::discriminant(a) == std::mem::discriminant(b)
                && ae == be
                && af == bf
                && ab == bb
                && as_ == bs
                && ac == bc
                && filter(ap.as_ref(), bp.as_ref())
        }
        (
            Instruction::Binding {
                core: a,
                children: ac,
                occurrence: ao,
            },
            Instruction::Binding {
                core: b,
                children: bc,
                occurrence: bo,
            },
        ) => core(a, b) && ac == bc && ao == bo,
        _ => false,
    }
}
pub(super) fn same(node: &Node, plan: &Plan, op: &Instruction) -> bool {
    node.mentions == plan.mentions
        && node.writes == plan.writes
        && node.node_writes == plan.node_writes
        && node.effects == plan.effects
        && instruction(&node.op, op)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::sql::relational::outcomes::{Slots, finite_lower::Builder};
    fn plan(writes: &[usize]) -> Plan {
        let writes: Slots = writes.iter().copied().collect();
        Plan {
            op: Op::Predicate(crate::code_rule_dsl::sql::relational::ir::Set::AllNodes),
            effects: Default::default(),
            mentions: writes.clone(),
            writes: writes.clone(),
            node_writes: writes,
            walk_filter: None,
        }
    }
    #[test]
    fn instruction_interning_keeps_metadata_order_and_native_function_ids() {
        let mut builder = Builder::default();
        let empty = plan(&[]);
        let first = builder.node(
            &empty,
            Instruction::Boolean {
                all: true,
                children: vec![1, 2],
            },
        );
        assert_eq!(
            first,
            builder.node(
                &empty,
                Instruction::Boolean {
                    all: true,
                    children: vec![1, 2]
                }
            )
        );
        assert_ne!(
            first,
            builder.node(
                &empty,
                Instruction::Boolean {
                    all: true,
                    children: vec![2, 1]
                }
            )
        );
        assert_ne!(
            first,
            builder.node(
                &plan(&[0]),
                Instruction::Boolean {
                    all: true,
                    children: vec![1, 2]
                }
            )
        );
        let call = builder.node(&empty, Instruction::Call(1));
        assert_ne!(call, builder.node(&empty, Instruction::Call(2)));
        let not = builder.node(&empty, Instruction::Not(1));
        assert_ne!(not, builder.node(&empty, Instruction::Not(2)));
    }
    #[test]
    fn binding_interning_pins_fact_identity_and_following_target_slots() {
        let mut builder = Builder::default();
        let empty = plan(&[]);
        let core = |term| {
            Box::new(Plan {
                op: Op::BoundLeaf {
                    term,
                    inputs: vec![0],
                    outputs: vec![0],
                    kinds: None,
                },
                ..plan(&[0])
            })
        };
        let first = builder.node(
            &empty,
            Instruction::Binding {
                core: core(1),
                children: vec![(0, 10)],
                occurrence: None,
            },
        );
        assert_eq!(
            first,
            builder.node(
                &empty,
                Instruction::Binding {
                    core: core(1),
                    children: vec![(0, 10)],
                    occurrence: None
                }
            )
        );
        assert_ne!(
            first,
            builder.node(
                &empty,
                Instruction::Binding {
                    core: core(2),
                    children: vec![(0, 10)],
                    occurrence: None
                }
            )
        );
        assert_ne!(
            first,
            builder.node(
                &empty,
                Instruction::Binding {
                    core: core(1),
                    children: vec![(1, 10)],
                    occurrence: None
                }
            )
        );
    }
}
