    use super::{
        HardPartition, MatchRegistry, PackedDemand, register_module,
        stable_partition_ranges,
    };
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::transducer::packed::{ExecutionBackend, Program};
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use rusqlite::Connection;
    use std::sync::Arc;

    fn empty_demand() -> PackedDemand {
        PackedDemand::new(
            ExecutionBackend::CpuJit,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("valid empty demand")
    }

    fn python_tape(source: &str) -> TapeDoc {
        let source = Arc::new(source.to_owned());
        let grammar = Grammar::new(GrammarDialect::Python).expect("Python grammar");
        let image = TapeImage::parse(&grammar, &source, &|| false)
            .expect("parser completed")
            .expect("tape image");
        TapeDoc::restore(source, grammar, &image, &|| false).expect("restored tape")
    }

    fn multi_file_program_demand(backend: ExecutionBackend) -> PackedDemand {
        PackedDemand::new(
            backend,
            vec![
                Program::compile_universe().expect("first verified universe"),
                Program::compile_universe().expect("second verified universe"),
                Program::compile_universe().expect("failed verified universe"),
            ],
            vec![python_tape("one\n"), python_tape("two\n")],
            vec![
                vec![vec![0], vec![0], vec![0]],
                vec![vec![0], Vec::new(), Vec::new()],
            ],
            vec![vec![true, true, true], vec![true, true, false]],
            vec![u32::MAX, u32::MAX, u32::MAX],
        )
        .and_then(|demand| demand.with_failure_injection(2, "capacity-failure".to_owned()))
        .expect("valid multi-file, multi-program demand")
    }

    #[test]
    fn hard_capacity_partitions_have_stable_task_ranges_and_cause() {
        let demand = multi_file_program_demand(ExecutionBackend::CpuJit);
        let ranges = stable_partition_ranges(2, |range| range.len() <= 1);
        assert_eq!(ranges, [0..1, 1..2]);
        assert_eq!(
            demand.partition_records(&ranges),
            [
                HardPartition {
                    cause: "packed-workspace-capacity",
                    limit: u32::MAX as usize,
                    task_range: 0..3,
                },
                HardPartition {
                    cause: "packed-workspace-capacity",
                    limit: u32::MAX as usize,
                    task_range: 3..5,
                },
            ]
        );
    }

    #[test]
    fn sparse_task_grid_rejects_roots_for_an_excluded_file_producer_pair() {
        let source = Arc::new("value\n".to_owned());
        let grammar = Grammar::new(GrammarDialect::Python).expect("Python grammar");
        let image = TapeImage::parse(&grammar, &source, &|| false)
            .expect("parser completed")
            .expect("tape image");
        let tape = TapeDoc::restore(Arc::clone(&source), grammar, &image, &|| false)
            .expect("restored tape");
        let result = PackedDemand::new(
            ExecutionBackend::CpuJit,
            vec![Program::compile_universe().expect("verified universe")],
            vec![tape],
            vec![vec![vec![0]]],
            vec![vec![false]],
            vec![u32::MAX],
        );
        assert!(matches!(
            result,
            Err(crate::code_rule_dsl::transducer::packed::Error::InvalidAnchor)
        ));
    }

    /// A connection to a new content store: the relations run where their statistics live.
    fn store_connection() -> Connection {
        let path = crate::code_rule_dsl::fixtures::temp();
        drop(crate::code_rule_dsl::fixtures::open(&path));
        Connection::open(path).expect("content store connection")
    }

    #[test]
    fn one_filter_executes_one_immutable_multi_file_program_batch() {
        let connection = store_connection();
        let registry = Arc::new(MatchRegistry::default());
        register_module(&connection, Arc::clone(&registry)).expect("virtual table module");
        let token = registry.register(multi_file_program_demand(ExecutionBackend::CpuJit));
        assert_eq!(
            registry.query_execution_report(token),
            Some(super::PackedExecutionCounters {
                registrations: 1,
                ..super::PackedExecutionCounters::default()
            })
        );
        assert!(registry.query_producer_status_rows(token).is_empty());

        let row_count = connection
            .query_row(
                "SELECT count(*) FROM rerail_matches WHERE batch = ?1",
                [token],
                |row| row.get::<_, i64>(0),
            )
            .expect("first relation scan");
        assert!(row_count > 5);
        let statuses = registry.query_producer_status_rows(token);
        assert_eq!(statuses.len(), 5, "every demanded producer has one status");
        assert_eq!(
            statuses
                .iter()
                .filter(|status| status.complete == 1)
                .count(),
            4
        );
        assert!(statuses.iter().any(|status| {
            status.complete == 0 && status.error.as_deref() == Some("capacity-failure")
        }));
        assert_eq!(
            registry.query_execution_report(token),
            Some(super::PackedExecutionCounters {
                registrations: 1,
                virtual_table_scans: 1,
                filters: 1,
                demand_executions: 1,
                backend_invocations: 1,
                hard_partitions: 0,
                partitions: Vec::new(),
                demanded_producers: 5,
                terminal_tasks: 5,
                command_buffers: 0,
                packed_dispatches: 0,
                anchor_dispatches: 0,
                matcher_calls: 0,
                host_callbacks: 0,
            })
        );

        let repeated = connection
            .query_row(
                "SELECT count(*) FROM rerail_matches WHERE batch = ?1",
                [token],
                |row| row.get::<_, i64>(0),
            )
            .expect("second cursor reads stored rows");
        assert_eq!(repeated, row_count);
        let report = registry
            .query_execution_report(token)
            .expect("registered execution report");
        assert_eq!(report.virtual_table_scans, 2);
        assert_eq!(report.filters, 2);
        assert_eq!(report.demand_executions, 1);
        assert_eq!(report.backend_invocations, 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn metal_uses_one_command_buffer_and_one_packed_dispatch() {
        use crate::code_rule_dsl::transducer::packed::{
            metal_execution_metrics, reset_metal_execution_metrics,
        };

        reset_metal_execution_metrics();
        let connection = store_connection();
        let registry = Arc::new(MatchRegistry::default());
        register_module(&connection, Arc::clone(&registry)).expect("virtual table module");
        let token = registry.register(multi_file_program_demand(ExecutionBackend::MetalJit));
        connection
            .query_row(
                "SELECT count(*) FROM rerail_matches WHERE batch = ?1",
                [token],
                |row| row.get::<_, i64>(0),
            )
            .expect("Metal relation scan");

        let report = registry
            .query_execution_report(token)
            .expect("registered execution report");
        assert_eq!(report.backend_invocations, 1);
        assert_eq!(report.command_buffers, 1);
        assert_eq!(report.packed_dispatches, 1);
        assert_eq!(report.anchor_dispatches, 0);
        assert_eq!(metal_execution_metrics(), (1, 0));
    }

    #[test]
    fn expanded_relation_has_one_typed_schema() {
        let connection = store_connection();
        register_module(&connection, Arc::new(MatchRegistry::default()))
            .expect("virtual table module");
        let columns = connection
            .prepare("SELECT name FROM pragma_table_xinfo('rerail_matches') ORDER BY cid")
            .expect("schema query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("schema rows")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("schema values");
        assert_eq!(
            columns,
            [
                "batch",
                "row_kind",
                "producer_id",
                "file_index",
                "task_index",
                "match_index",
                "capture_index",
                "capture_occurrence",
                "capture_sequence_start",
                "capture_sequence_count",
                "node",
                "related_node",
                "axis_kind",
                "axis_order",
                "fact_kind",
                "document_id",
                "mapped_document_id",
                "start_ordinal",
                "output_label",
                "capture_label",
                "text_value",
                "start_byte",
                "end_byte",
                "verdict",
                "complete",
                "work_used",
                "error_code",
                "field_id",
                "related_named",
                "source_text",
                "structural_key",
                "named_leaf",
                "capture_kind",
                "capture_named",
                "equality_occurrence",
            ]
        );
    }

    #[test]
    fn empty_batch_is_inert_until_sql_demands_one_explicit_batch_status() {
        let connection = store_connection();
        let registry = Arc::new(MatchRegistry::default());
        register_module(&connection, Arc::clone(&registry)).expect("virtual table module");
        let token = registry.register(empty_demand());
        assert_eq!(registry.query_execution_counters(token), Some((0, 0)));

        let rows = connection
            .prepare(
                "SELECT row_kind, producer_id, file_index, complete, error_code \
                 FROM rerail_matches WHERE batch = ?1 ORDER BY rowid",
            )
            .expect("expanded relation query")
            .query_map([token], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .expect("batch rows")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("batch relation");

        assert_eq!(rows, vec![(1, None, None, 1, None)]);
        let repeated = connection
            .query_row(
                "SELECT count(*) FROM rerail_matches WHERE batch = ?1",
                [token],
                |row| row.get::<_, i64>(0),
            )
            .expect("second cursor reads the materialized relation");
        assert_eq!(repeated, 1);
        assert_eq!(registry.query_execution_counters(token), Some((1, 2)));
    }
