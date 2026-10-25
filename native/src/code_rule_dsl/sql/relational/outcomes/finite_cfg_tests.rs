use super::*;
use crate::code_rule_dsl::sql::relational::outcomes::{
    Slots,
    effects::Effects,
    finite_lower::{Node, Program},
};
fn node(op: Instruction) -> Node {
    Node {
        op,
        effects: Effects::default(),
        mentions: Slots::new(),
        writes: Slots::new(),
        node_writes: Slots::new(),
    }
}
fn fixture(all: bool) -> Program {
    Program {
        source: None,
        share_search: false,
        code: vec![
            node(Instruction::Eval(0)),
            node(Instruction::Eval(1)),
            node(Instruction::Eval(2)),
            node(Instruction::Boolean {
                all,
                children: vec![0, 1, 2],
            }),
            node(Instruction::Not(3)),
        ],
        functions: vec![],
        regions: vec![],
        entry: 4,
        mentions: Slots::new(),
        writes: Slots::new(),
        node_writes: Slots::new(),
    }
}
fn execute(cfg: &Cfg, values: &[i8]) -> i8 {
    let mut t = cfg.entry;
    for _ in 0..256 {
        match t {
            Target::Return(v) => return v,
            Target::Site(i) => {
                let s = &cfg.sites[i];
                let truth = values[s.source];
                t = s.routes.0[if truth < 0 { 3 } else { truth as usize }];
                if let Target::Return(-1) = t {
                    return truth;
                }
            }
        }
    }
    panic!("acyclic CFG did not terminate")
}
#[test]
fn boolean_cfg_preserves_unknown_short_circuit_and_not() {
    for all in [false, true] {
        let p = fixture(all);
        let cfg = compile(&p).unwrap();
        assert!(
            cfg.sites
                .iter()
                .all(|s| matches!(p.code[s.source].op, Instruction::Eval(_)))
        );
        for a in [-2, 0, 1, 2] {
            for b in [-2, 0, 1, 2] {
                for c in [-2, 0, 1, 2] {
                    let values = [a, b, c];
                    let mut unknown = false;
                    let mut answer = if all { 1 } else { 0 };
                    for v in values {
                        if v < 0 {
                            answer = v;
                            break;
                        }
                        if (all && v == 0) || (!all && v == 1) {
                            answer = v;
                            break;
                        }
                        unknown |= v == 2;
                        if unknown {
                            answer = 2;
                        }
                    }
                    let expected = match answer {
                        0 => 1,
                        1 => 0,
                        v => v,
                    };
                    assert_eq!(
                        execute(&cfg, &values),
                        expected,
                        "all={all} values={values:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn cached_failure_follower_boundary_is_not_flattened() {
    let mut p = fixture(false);
    p.code[1].effects.uses_memo = true;
    let cfg = compile(&p).unwrap();
    assert!(cfg.sites.iter().any(|s| s.source == 3));
}
#[test]
fn captured_program_retains_original_scheduler() {
    let mut p = fixture(true);
    p.code[0].writes.insert(0);
    assert!(compile(&p).is_none());
}

#[test]
fn static_cfg_native_value_unknown_order_and_finite_work() {
    use crate::code_rule_dsl::sql::relational::outcomes::publication_tests::{
        check_work_compiler, policy_utils,
    };
    use serde_json::json;
    let rules = [
        json!({"kind":"arguments","has":{"stopBy":"neighbor","all":[{"matches":"u"},{"not":{"not":{"matches":"u"}}},{"any":[{"regex":"^1$"},{"regex":"^2$"}]}]}}),
    ];
    let policy = policy_utils(&rules, json!({"u":{"testUnknown":"^x$","value":{"gt":0}}}));
    for work in [0, 1, 2, 3, 4, 8, 32, i64::MAX] {
        check_work_compiler(
            &policy,
            &rules,
            &["f(x,1,2);f(0,2);f(1);"],
            None,
            work,
            |grammars, work| {
                let compiled = crate::code_rule_dsl::sql::relational::outcomes::compile_job_finite(
                    grammars, work,
                )?;
                if let Ok(path) = std::env::var("RERAIL_FINITE_CFG_DIAGNOSTIC") {
                    std::fs::write(format!("{path}.{work}.sql"), &compiled.sql).unwrap();
                }
                assert!(
                    compiled
                        .relations
                        .to_string()
                        .contains("direct predicate projection")
                );
                eprintln!("static CFG native gate work={work}");
                Ok(compiled)
            },
        );
    }
}

#[test]
fn binding_occurrence_is_remapped_as_a_boundary_entry() {
    use crate::code_rule_dsl::sql::relational::{
        ir::Set,
        outcomes::{Op, Plan, finite_cfg_emit},
    };
    let mut p = fixture(true);
    p.code.push(node(Instruction::Binding {
        core: Box::new(Plan {
            op: Op::Predicate(Set::AllNodes),
            effects: Effects::default(),
            mentions: Slots::new(),
            writes: Slots::new(),
            node_writes: Slots::new(),
            walk_filter: None,
        }),
        children: vec![(7, 3)],
        occurrence: Some(4),
    }));
    p.entry = 5;
    let cfg = compile(&p).unwrap();
    let adapted = finite_cfg_emit::adapt(&p, &cfg).unwrap();
    let Target::Site(expected) = cfg.entries[&4] else {
        panic!("atomic occurrence boundary")
    };
    let Instruction::Binding {
        occurrence,
        children,
        ..
    } = &adapted.code[adapted.entry].op
    else {
        panic!("binding entry")
    };
    assert_eq!(*occurrence, Some(expected));
    let Target::Site(child) = cfg.entries[&3] else {
        panic!("atomic child boundary")
    };
    assert_eq!(children, &[(7, child)]);
}
