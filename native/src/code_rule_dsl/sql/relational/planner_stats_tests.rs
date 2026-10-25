use super::*;
use crate::code_rule_dsl::sql::relational::{
    emit,
    ir::{Edge, Program, Relation, Set},
    physical,
};
use crate::code_rule_dsl::{
    language::GrammarDialect,
    matcher::node_at,
    sql::{exec, tests::differential::tape},
};
use ast_grep_core::AstGrep;
use rusqlite::{Connection, StatementStatus};

fn rows(db: &Connection, sql: &str) -> (Vec<u32>, i32) {
    let mut statement = db.prepare(sql).unwrap();
    let rows = statement
        .query_map([], |r| r.get(2))
        .unwrap()
        .collect::<Result<Vec<u32>, _>>()
        .unwrap();
    (rows, statement.get_status(StatementStatus::VmStep))
}

#[test]
fn unknown_statistics_are_not_zero() {
    let unknown = PlannerStats::default();
    assert_eq!(unknown.kind_rows(&[123]), None);
    assert_eq!(unknown.direct_edges(1, 2), None);
}

#[test]
fn skewed_batch_changes_pure_anchor_with_exact_native_rows() {
    let mut evidence = Vec::new();
    for calls_are_rare in [true, false] {
        let source = if calls_are_rare {
            format!("{}\nf(1)\n", "1\n".repeat(512))
        } else {
            format!("{}\nf(1)\n", "f()\n".repeat(512))
        };
        let doc = tape(GrammarDialect::Python, &source);
        let root = AstGrep::doc(doc.clone());
        let call = (0..doc.transducer_len() as u32)
            .map(|id| node_at(&root, id))
            .find(|n| n.kind() == "call")
            .unwrap()
            .kind_id();
        let integer = (0..doc.transducer_len() as u32)
            .map(|id| node_at(&root, id))
            .find(|n| n.kind() == "integer")
            .unwrap()
            .kind_id();
        let args = (0..doc.transducer_len() as u32)
            .map(|id| node_at(&root, id))
            .find(|n| n.kind() == "argument_list")
            .unwrap()
            .kind_id();
        // Root call -> arguments -> integer. Changing corpus skew changes the rarest endpoint.
        let mut program = Program::default();
        program.relations.push(Relation {
            source: "pure structural path".into(),
            domain: None,
            keys: None,
            expression: Set::Intersect(vec![
                Set::Kinds(vec![call]),
                Set::Preimage(
                    Edge::Child,
                    Box::new(Set::Intersect(vec![
                        Set::Kinds(vec![args]),
                        Set::Preimage(Edge::Child, Box::new(Set::Kinds(vec![integer]))),
                    ])),
                ),
            ]),
        });
        let plan = physical::plan(&program).unwrap();
        let expected: Vec<_> = (0..doc.transducer_len() as u32)
            .filter(|&id| {
                let n = node_at(&root, id);
                n.kind_id() == call
                    && n.children().any(|a| {
                        a.kind_id() == args && a.children().any(|c| c.kind_id() == integer)
                    })
            })
            .collect();
        exec::with_nodes(vec![doc], |db, batch| {
            let stats = PlannerStats::from_batch(batch, GrammarDialect::Python);
            assert_eq!(stats.kind_rows(&[u16::MAX]), Some(0));
            assert!(stats.direct_edges(call, args).unwrap() > 0);
            let (chosen, decisions) = emit::emit_with_stats(&program, &plan, &stats).unwrap();
            assert_eq!(decisions.len(), 1);
            assert_eq!(decisions[0].variable, if calls_are_rare { 0 } else { 2 });
            let (actual, chosen_vm) = rows(db, &chosen);
            assert_eq!(actual, expected);
            // Force the opposite atomic posting estimate; only the physical anchor changes.
            let mut opposite = stats.clone();
            opposite
                .kinds
                .insert(call, if calls_are_rare { 1_000_000 } else { 0 });
            opposite
                .kinds
                .insert(integer, if calls_are_rare { 0 } else { 1_000_000 });
            opposite.kinds.insert(args, 1_000_000);
            let (alternative, _) = emit::emit_with_stats(&program, &plan, &opposite).unwrap();
            let (other, alternative_vm) = rows(db, &alternative);
            assert_eq!(other, expected);
            evidence.push(serde_json::json!({"callsRare":calls_are_rare,"chosenVariable":decisions[0].variable,"estimatedRows":decisions[0].estimated_rows,"vm":chosen_vm,"forcedAlternativeVm":alternative_vm}));
            assert!(chosen_vm < alternative_vm, "{evidence:?}");
            let (unknown_sql, unknown_decisions) =
                emit::emit_with_stats(&program, &plan, &PlannerStats::default()).unwrap();
            assert!(unknown_decisions.is_empty());
            assert_eq!(unknown_sql, emit::emit(&program, &plan).unwrap());
        });
    }
    if let Ok(path) = std::env::var("RERAIL_PLANNER_STATS_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
}

#[test]
fn language_snapshot_excludes_colliding_kind_ids() {
    let python = tape(GrammarDialect::Python, "f(1)\n");
    let typescript = tape(GrammarDialect::TypeScript, &"const x = f(1);\n".repeat(100));
    exec::with_nodes(vec![python.clone()], |_, batch| {
        let alone = PlannerStats::from_batch(batch, GrammarDialect::Python);
        exec::with_nodes(vec![python, typescript], |_, mixed| {
            let filtered = PlannerStats::from_batch(mixed, GrammarDialect::Python);
            let other = PlannerStats::from_batch(mixed, GrammarDialect::TypeScript);
            assert_eq!(alone.kinds, filtered.kinds);
            assert_eq!(alone.edges, filtered.edges);
            assert!(filtered.for_dialect(GrammarDialect::TypeScript).is_none());
            assert!(filtered.for_dialect(GrammarDialect::Python).is_some());
            assert!(
                filtered
                    .kinds
                    .keys()
                    .any(|kind| other.kinds.contains_key(kind)),
                "test requires actual colliding local kind IDs"
            );
            assert_ne!(filtered.kinds, other.kinds);
        });
    });
}

#[test]
fn pure_relation_reuse_selects_inline_or_materialized_with_vm_controls() {
    let mut evidence = Vec::new();
    for size in [8, 128, 1024] {
        let doc = tape(GrammarDialect::Python, &"f(1)\n".repeat(size));
        let root = AstGrep::doc(doc.clone());
        let integer = (0..doc.transducer_len() as u32)
            .map(|id| node_at(&root, id))
            .find(|n| n.kind() == "integer")
            .unwrap()
            .kind_id();
        for repeated in [false, true] {
            let body = Set::Intersect(vec![Set::Kinds(vec![integer]), Set::Nonempty]);
            let consumer = Set::Intersect(vec![Set::Reference(1), Set::Kinds(vec![integer])]);
            let expression = if repeated {
                Set::Union(vec![
                    consumer.clone(),
                    Set::Intersect(vec![Set::Reference(1), Set::Nonempty]),
                ])
            } else {
                consumer
            };
            let program = Program {
                relations: vec![
                    Relation {
                        source: "consumer".into(),
                        expression,
                        domain: None,
                        keys: None,
                    },
                    Relation {
                        source: "producer".into(),
                        expression: body,
                        domain: None,
                        keys: None,
                    },
                ],
                ..Program::default()
            };
            let plan = physical::plan(&program).unwrap();
            exec::with_nodes(vec![doc.clone()], |db, batch| {
                let stats = PlannerStats::from_batch(batch, GrammarDialect::Python);
                let choices = materialization_decisions(&program, &plan, &stats);
                assert_eq!(choices[1].materialized, repeated);
                assert_eq!(choices[1].consumers, if repeated { 2 } else { 1 });
                let (sql, _) = emit::emit_with_stats(&program, &plan, &stats).unwrap();
                assert_eq!(sql.contains("r1(file,node) AS MATERIALIZED"), repeated);
                let (actual, vm) = rows(db, &sql);
                assert_eq!(actual.len(), size);
                let mut forced = stats.clone();
                forced.force_materialization = Some(!repeated);
                let (alternative, _) = emit::emit_with_stats(&program, &plan, &forced).unwrap();
                let (other, alternative_vm) = rows(db, &alternative);
                assert_eq!(actual, other);
                evidence.push(serde_json::json!({"size":size,"uses":choices[1].consumers,"materialized":repeated,"vm":vm,"forcedAlternativeVm":alternative_vm}));
            });
        }
    }
    if let Ok(path) = std::env::var("RERAIL_PLANNER_STATS_EVIDENCE") {
        std::fs::write(
            format!("{path}.reuse.json"),
            serde_json::to_vec_pretty(&evidence).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn incident_child_fanout_reverses_with_corpus_skew() {
    let mut evidence = Vec::new();
    for expressions_are_common in [true, false] {
        let source = if expressions_are_common {
            format!("{}def f():\n    pass\n", "x = 1\n".repeat(512))
        } else {
            format!(
                "{}x = 1\n",
                (0..512)
                    .map(|i| format!("def f{i}():\n    pass\n"))
                    .collect::<String>()
            )
        };
        let doc = tape(GrammarDialect::Python, &source);
        let root = AstGrep::doc(doc.clone());
        let kind = |name: &str| {
            (0..doc.transducer_len() as u32)
                .map(|id| node_at(&root, id))
                .find(|n| n.kind() == name)
                .unwrap()
                .kind_id()
        };
        let module = kind("module");
        let expression = kind("expression_statement");
        let function = kind("function_definition");
        let set = Set::Intersect(vec![
            Set::Kinds(vec![module]),
            Set::Preimage(Edge::Child, Box::new(Set::Kinds(vec![expression]))),
            Set::Preimage(Edge::Child, Box::new(Set::Kinds(vec![function]))),
        ]);
        let program = Program {
            relations: vec![Relation {
                source: "two independent pure child constraints".into(),
                expression: set,
                domain: None,
                keys: None,
            }],
            ..Program::default()
        };
        let plan = physical::plan(&program).unwrap();
        let expected: Vec<_> = (0..doc.transducer_len() as u32)
            .filter(|&id| {
                let n = node_at(&root, id);
                n.kind_id() == module
                    && n.children().any(|c| c.kind_id() == expression)
                    && n.children().any(|c| c.kind_id() == function)
            })
            .collect();
        exec::with_nodes(vec![doc], |db, batch| {
            let stats = PlannerStats::from_batch(batch, GrammarDialect::Python);
            let (sql, anchors, edges) =
                emit::emit_with_stats_detailed(&program, &plan, &stats).unwrap();
            assert_eq!(anchors[0].variable, 0);
            assert_eq!(edges[0].target, if expressions_are_common { 2 } else { 1 });
            assert_eq!(edges[0].observed_edges, 1);
            let (actual, vm) = rows(db, &sql);
            assert_eq!(actual, expected);
            let mut opposite = stats.clone();
            opposite.edges.insert(
                (module, expression),
                if expressions_are_common { 0 } else { 1_000_000 },
            );
            opposite.edges.insert(
                (module, function),
                if expressions_are_common { 1_000_000 } else { 0 },
            );
            let (alternative, _, choices) =
                emit::emit_with_stats_detailed(&program, &plan, &opposite).unwrap();
            assert_ne!(choices[0].target, edges[0].target);
            let (other, alternative_vm) = rows(db, &alternative);
            assert_eq!(other, expected);
            assert!(vm < alternative_vm, "vm={vm},alternative={alternative_vm}");
            let unknown = PlannerStats::default();
            let (unknown_sql, _, unknown_edges) =
                emit::emit_with_stats_detailed(&program, &plan, &unknown).unwrap();
            assert!(unknown_edges.is_empty());
            assert_eq!(unknown_sql, emit::emit(&program, &plan).unwrap());
            evidence.push(serde_json::json!({"expressionsCommon":expressions_are_common,"selectedTarget":edges[0].target,"observedEdges":edges[0].observed_edges,"knownRows":edges[0].known_rows,"vm":vm,"forcedAlternativeVm":alternative_vm}));
        });
    }
    if let Ok(path) = std::env::var("RERAIL_PLANNER_STATS_EVIDENCE") {
        std::fs::write(
            format!("{path}.edges.json"),
            serde_json::to_vec_pretty(&evidence).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn unsupported_or_unbounded_incident_edges_keep_source_order() {
    let mut stats = PlannerStats {
        complete: true,
        ..PlannerStats::default()
    };
    stats.kinds.insert(1, 10);
    let arm = Arm {
        nodes: 2,
        edges: vec![(0, Edge::Next, 1)],
        predicates: vec![Predicate::Kinds(0, vec![1]), Predicate::Kinds(1, vec![1])],
        references: vec![],
    };
    assert!(stats.incident_edge(&arm, &[true, false]).is_none());
    let arm = Arm {
        edges: vec![(0, Edge::Child, 1)],
        predicates: vec![Predicate::Kinds(0, vec![1])],
        ..arm
    };
    assert!(stats.incident_edge(&arm, &[true, false]).is_none());
}
