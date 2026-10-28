use super::super::{FactFamilyContract, StatementRelations, UtilityDefinitionId};
use super::sql_ast::{
    PhysicalRelation, SqlAstNode, SqlBinaryOperator, SqlColumn, SqlExpression, SqlParameter,
    SqlProjection, SqlValue,
};

const BROAD_COLUMNS: [&str; 35] = [
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
];

#[derive(Clone, Copy)]
enum Projection {
    BatchStatus,
    ProducerStatus,
    TerminalMatch,
    Capture,
    Navigation,
    SemanticFact,
    ValueFact,
    ParseFact,
    TransformFact,
    DocumentMap,
}

fn column(name: &str) -> SqlExpression {
    SqlExpression::Column(SqlColumn {
        relation: None,
        name: name.to_owned(),
    })
}

fn projected(expression: SqlExpression, alias: Option<&str>) -> SqlProjection {
    SqlProjection {
        expression,
        alias: alias.map(str::to_owned),
    }
}

fn broad_projection(
    row_kind: i64,
    available: &[&str],
    complete: Option<i64>,
) -> Vec<SqlProjection> {
    BROAD_COLUMNS
        .iter()
        .map(|name| {
            if *name == "row_kind" {
                projected(
                    SqlExpression::Value(SqlValue::Integer(row_kind)),
                    Some(name),
                )
            } else if *name == "complete" && complete.is_some() && !available.contains(name) {
                projected(
                    SqlExpression::Value(SqlValue::Integer(complete.expect("checked complete"))),
                    Some(name),
                )
            } else if available.contains(name) {
                projected(column(name), None)
            } else {
                projected(SqlExpression::Value(SqlValue::Null), Some(name))
            }
        })
        .collect()
}

impl Projection {
    fn items(self) -> Vec<SqlProjection> {
        const PRODUCER: &[&str] = &[
            "batch",
            "producer_id",
            "file_index",
            "verdict",
            "complete",
            "work_used",
            "error_code",
        ];
        const TERMINAL: &[&str] = &[
            "batch",
            "producer_id",
            "file_index",
            "task_index",
            "match_index",
            "node",
            "document_id",
            "mapped_document_id",
            "start_ordinal",
            "output_label",
            "start_byte",
            "end_byte",
            "verdict",
        ];
        const VALUE: &[&str] = &[
            "batch",
            "producer_id",
            "file_index",
            "task_index",
            "match_index",
            "node",
            "fact_kind",
            "text_value",
            "verdict",
        ];
        const PARSE: &[&str] = &[
            "batch",
            "producer_id",
            "file_index",
            "task_index",
            "match_index",
            "capture_index",
            "node",
            "axis_order",
            "fact_kind",
            "start_byte",
            "end_byte",
            "verdict",
            "source_text",
            "named_leaf",
            "capture_kind",
            "capture_named",
        ];
        const TRANSFORM: &[&str] = &[
            "batch",
            "producer_id",
            "file_index",
            "match_index",
            "node",
            "related_node",
            "fact_kind",
            "output_label",
            "capture_label",
            "text_value",
            "verdict",
            "source_text",
        ];
        match self {
            Self::BatchStatus | Self::Capture | Self::Navigation | Self::DocumentMap => vec![
                projected(column("batch"), None),
                projected(SqlExpression::All, None),
            ],
            Self::ProducerStatus => broad_projection(2, PRODUCER, None),
            Self::TerminalMatch => broad_projection(3, TERMINAL, Some(1)),
            Self::SemanticFact => broad_projection(6, &BROAD_COLUMNS, None),
            Self::ValueFact => broad_projection(6, VALUE, Some(1)),
            Self::ParseFact => broad_projection(6, PARSE, Some(1)),
            Self::TransformFact => broad_projection(6, TRANSFORM, Some(1)),
        }
    }
}

struct RelationScanAst {
    source: PhysicalRelation,
    projection: Projection,
    producer_scoped: bool,
}

impl RelationScanAst {
    fn render(&self, producers: &[u64]) -> String {
        let batch = SqlExpression::Binary {
            operator: SqlBinaryOperator::Equal,
            left: Box::new(column("batch")),
            right: Box::new(SqlExpression::Parameter(SqlParameter::Batch)),
        };
        let node = if self.producer_scoped {
            SqlAstNode::DemandScan {
                source: self.source.clone(),
                producer_ids: producers.to_vec(),
                projection: self.projection.items(),
                predicates: vec![batch],
            }
        } else {
            SqlAstNode::RelationScan {
                source: self.source.clone(),
                projection: self.projection.items(),
                predicates: vec![batch],
                ordering: Vec::new(),
            }
        };
        node.render()
    }
}

enum QueryAst {
    Scan(RelationScanAst),
    UnionAll(Vec<RelationScanAst>),
}

impl QueryAst {
    fn render(&self, producers: &[u64]) -> String {
        match self {
            Self::Scan(scan) => scan.render(producers),
            Self::UnionAll(scans) => scans
                .iter()
                .map(|scan| scan.render(producers))
                .collect::<Vec<_>>()
                .join("\n  UNION ALL\n  "),
        }
    }
}

struct CteAst {
    name: &'static str,
    query: QueryAst,
}

impl CteAst {
    fn render(&self, producers: &[u64]) -> String {
        format!(
            "{} AS MATERIALIZED (\n  {}\n)",
            self.name,
            self.query.render(producers)
        )
    }
}

pub(in crate::code_rule_dsl::query) struct TypedBoundaryAst {
    producers: Vec<u64>,
    utility_definitions: Vec<UtilityDefinitionId>,
    ctes: Vec<CteAst>,
}

impl TypedBoundaryAst {
    pub(in crate::code_rule_dsl::query) fn new(
        relations: &StatementRelations,
        producers: impl IntoIterator<Item = u64>,
    ) -> Self {
        let mut producers = producers.into_iter().collect::<Vec<_>>();
        producers.sort_unstable();
        producers.dedup();
        let scan = |source, projection, producer_scoped| {
            QueryAst::Scan(RelationScanAst {
                source,
                projection,
                producer_scoped,
            })
        };
        let fact_scans = relations
            .fact_families()
            .map(|family| {
                let (source, projection) = match family {
                    FactFamilyContract::Semantic => {
                        (PhysicalRelation::SemanticFact, Projection::SemanticFact)
                    }
                    FactFamilyContract::Value => {
                        (PhysicalRelation::ValueFact, Projection::ValueFact)
                    }
                    FactFamilyContract::Parse => {
                        (PhysicalRelation::ParseFact, Projection::ParseFact)
                    }
                    FactFamilyContract::Transform => {
                        (PhysicalRelation::TransformFact, Projection::TransformFact)
                    }
                };
                RelationScanAst {
                    source,
                    projection,
                    producer_scoped: true,
                }
            })
            .collect();
        let utility_definitions = relations.utility_definitions().collect();
        let ctes = vec![
            CteAst {
                name: "batch_status",
                query: scan(
                    PhysicalRelation::BatchStatus,
                    Projection::BatchStatus,
                    false,
                ),
            },
            CteAst {
                name: "producer_status",
                query: scan(
                    PhysicalRelation::ProducerStatus,
                    Projection::ProducerStatus,
                    true,
                ),
            },
            CteAst {
                name: "terminal_match",
                query: scan(
                    PhysicalRelation::TerminalMatch,
                    Projection::TerminalMatch,
                    true,
                ),
            },
            CteAst {
                name: "capture",
                query: scan(PhysicalRelation::Capture, Projection::Capture, true),
            },
            CteAst {
                name: "navigation",
                query: scan(PhysicalRelation::Navigation, Projection::Navigation, true),
            },
            CteAst {
                name: "fact",
                query: QueryAst::UnionAll(fact_scans),
            },
            CteAst {
                name: "document_map",
                query: scan(
                    PhysicalRelation::DocumentMap,
                    Projection::DocumentMap,
                    false,
                ),
            },
        ];
        Self {
            producers,
            utility_definitions,
            ctes,
        }
    }

    pub(in crate::code_rule_dsl::query) fn render(&self) -> String {
        debug_assert!(
            self.utility_definitions
                .windows(2)
                .all(|pair| pair[0] < pair[1]),
            "physical utility definitions are deterministic and deduplicated"
        );
        self.ctes
            .iter()
            .map(|cte| cte.render(&self.producers))
            .collect::<Vec<_>>()
            .join(",\n")
    }

    pub(in crate::code_rule_dsl::query) fn verify_rendered_sources(
        sql: &str,
    ) -> Result<(), String> {
        if sql.contains("rerail_matches") {
            return Err("generated SQL uses the legacy wide packed relation".to_owned());
        }
        for relation in [
            "rerail_batch_status",
            "rerail_producer_status",
            "rerail_terminal_match",
            "rerail_capture",
            "rerail_navigation",
            "rerail_document_map",
        ] {
            if sql.matches(relation).count() != 1 {
                return Err(format!(
                    "generated SQL must scan typed relation {relation} once"
                ));
            }
        }
        let fact_scans = [
            "rerail_semantic_fact",
            "rerail_value_fact",
            "rerail_parse_fact",
            "rerail_transform_fact",
        ]
        .iter()
        .map(|relation| sql.matches(relation).count())
        .sum::<usize>();
        if fact_scans == 0 {
            return Err("generated SQL has no typed fact-family boundary".to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_boundary_ast_renders_only_planned_fact_families() {
        let relations = StatementRelations::from_fact_families([
            FactFamilyContract::Value,
            FactFamilyContract::Transform,
        ]);
        let sql = TypedBoundaryAst::new(&relations, [7, 2, 7]).render();
        assert!(sql.contains(
            "FROM \"rerail_value_fact\" WHERE (\"batch\" = :batch) AND (\"producer_id\" IN (2, 7))"
        ));
        assert!(sql.contains(
            "FROM \"rerail_transform_fact\" WHERE (\"batch\" = :batch) AND (\"producer_id\" IN (2, 7))"
        ));
        assert!(!sql.contains("rerail_semantic_fact"));
        assert!(!sql.contains("rerail_parse_fact"));
    }
}
