#![expect(
    dead_code,
    reason = "the SQL AST is exhaustive; individual physical plans construct only their required variants"
)]

use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) enum SqlAstNodeKind {
    RelationScan,
    DemandScan,
    Select,
    Join,
    SemiJoin,
    AntiJoin,
    OrderedReduce,
    Aggregate,
    Project,
    Union,
    Materialize,
    OutputMerge,
    Group,
    Window,
    Recursive,
    Reference,
}

impl SqlAstNodeKind {
    pub(in crate::code_rule_dsl) const ALL: [Self; 16] = [
        Self::RelationScan,
        Self::DemandScan,
        Self::Select,
        Self::Join,
        Self::SemiJoin,
        Self::AntiJoin,
        Self::OrderedReduce,
        Self::Aggregate,
        Self::Project,
        Self::Union,
        Self::Materialize,
        Self::OutputMerge,
        Self::Group,
        Self::Window,
        Self::Recursive,
        Self::Reference,
    ];
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct SqlRelationRef(u32);

impl SqlRelationRef {
    pub(in crate::code_rule_dsl) const fn new(value: u32) -> Self {
        Self(value)
    }

    fn rendered_name(self) -> String {
        format!("r{}", self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum PhysicalRelation {
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
    Clause(u32),
    ClauseOutput(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum SqlParameter {
    Batch,
    Files,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum SqlValue {
    Null,
    Integer(i64),
    Text(String),
    Boolean(bool),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct SqlColumn {
    pub(in crate::code_rule_dsl) relation: Option<SqlRelationRef>,
    pub(in crate::code_rule_dsl) name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum SqlBinaryOperator {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    And,
    Or,
    Add,
    Subtract,
    Multiply,
    Divide,
    Concatenate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum SqlUnaryOperator {
    Not,
    IsNull,
    IsNotNull,
    Negate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum SqlExpression {
    All,
    Column(SqlColumn),
    Parameter(SqlParameter),
    Value(SqlValue),
    Binary {
        operator: SqlBinaryOperator,
        left: Box<SqlExpression>,
        right: Box<SqlExpression>,
    },
    Unary {
        operator: SqlUnaryOperator,
        input: Box<SqlExpression>,
    },
    Function {
        name: String,
        arguments: Vec<SqlExpression>,
    },
    In {
        input: Box<SqlExpression>,
        values: Vec<SqlExpression>,
    },
    Case {
        branches: Vec<(SqlExpression, SqlExpression)>,
        fallback: Box<SqlExpression>,
    },
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

impl SqlColumn {
    fn render(&self) -> String {
        self.relation.as_ref().map_or_else(
            || quote_identifier(&self.name),
            |relation| {
                format!(
                    "{}.{}",
                    quote_identifier(&relation.rendered_name()),
                    quote_identifier(&self.name)
                )
            },
        )
    }
}

impl SqlExpression {
    fn render(&self) -> String {
        match self {
            Self::All => "*".to_owned(),
            Self::Column(column) => column.render(),
            Self::Parameter(parameter) => match parameter {
                SqlParameter::Batch => ":batch".to_owned(),
                SqlParameter::Files => ":files".to_owned(),
            },
            Self::Value(value) => match value {
                SqlValue::Null => "NULL".to_owned(),
                SqlValue::Integer(value) => value.to_string(),
                SqlValue::Text(value) => format!("'{}'", value.replace('\'', "''")),
                SqlValue::Boolean(value) => i64::from(*value).to_string(),
            },
            Self::Binary {
                operator,
                left,
                right,
            } => {
                let operator = match operator {
                    SqlBinaryOperator::Equal => "=",
                    SqlBinaryOperator::NotEqual => "<>",
                    SqlBinaryOperator::Less => "<",
                    SqlBinaryOperator::LessOrEqual => "<=",
                    SqlBinaryOperator::Greater => ">",
                    SqlBinaryOperator::GreaterOrEqual => ">=",
                    SqlBinaryOperator::And => "AND",
                    SqlBinaryOperator::Or => "OR",
                    SqlBinaryOperator::Add => "+",
                    SqlBinaryOperator::Subtract => "-",
                    SqlBinaryOperator::Multiply => "*",
                    SqlBinaryOperator::Divide => "/",
                    SqlBinaryOperator::Concatenate => "||",
                };
                format!("({} {operator} {})", left.render(), right.render())
            }
            Self::Unary { operator, input } => match operator {
                SqlUnaryOperator::Not => format!("(NOT {})", input.render()),
                SqlUnaryOperator::IsNull => format!("({} IS NULL)", input.render()),
                SqlUnaryOperator::IsNotNull => format!("({} IS NOT NULL)", input.render()),
                SqlUnaryOperator::Negate => format!("(-{})", input.render()),
            },
            Self::Function { name, arguments } => format!(
                "{}({})",
                quote_identifier(name),
                arguments
                    .iter()
                    .map(Self::render)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::In { input, values } => format!(
                "({} IN ({}))",
                input.render(),
                values
                    .iter()
                    .map(Self::render)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Case { branches, fallback } => {
                let branches = branches
                    .iter()
                    .map(|(condition, result)| {
                        format!("WHEN {} THEN {}", condition.render(), result.render())
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("CASE {branches} ELSE {} END", fallback.render())
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct SqlProjection {
    pub(in crate::code_rule_dsl) expression: SqlExpression,
    pub(in crate::code_rule_dsl) alias: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct SqlOrderTerm {
    pub(in crate::code_rule_dsl) expression: SqlExpression,
    pub(in crate::code_rule_dsl) descending: bool,
}

impl SqlProjection {
    fn render(&self) -> String {
        self.alias.as_ref().map_or_else(
            || self.expression.render(),
            |alias| {
                format!(
                    "{} AS {}",
                    self.expression.render(),
                    quote_identifier(alias)
                )
            },
        )
    }
}

impl SqlOrderTerm {
    fn render(&self) -> String {
        format!(
            "{} {}",
            self.expression.render(),
            if self.descending { "DESC" } else { "ASC" }
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum SqlAggregateFunction {
    Count,
    Minimum,
    Maximum,
    Sum,
    JsonGroupArray,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct SqlAggregate {
    pub(in crate::code_rule_dsl) function: SqlAggregateFunction,
    pub(in crate::code_rule_dsl) expression: Option<SqlExpression>,
    pub(in crate::code_rule_dsl) distinct: bool,
    pub(in crate::code_rule_dsl) alias: String,
}

impl SqlAggregate {
    fn render(&self) -> String {
        let function = match self.function {
            SqlAggregateFunction::Count => "count",
            SqlAggregateFunction::Minimum => "min",
            SqlAggregateFunction::Maximum => "max",
            SqlAggregateFunction::Sum => "sum",
            SqlAggregateFunction::JsonGroupArray => "json_group_array",
        };
        let argument = self
            .expression
            .as_ref()
            .map_or_else(|| "*".to_owned(), SqlExpression::render);
        format!(
            "{}({}{argument}) AS {}",
            quote_identifier(function),
            if self.distinct { "DISTINCT " } else { "" },
            quote_identifier(&self.alias)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum SqlAstNode {
    RelationScan {
        source: PhysicalRelation,
        projection: Vec<SqlProjection>,
        predicates: Vec<SqlExpression>,
        ordering: Vec<SqlOrderTerm>,
    },
    DemandScan {
        source: PhysicalRelation,
        producer_ids: Vec<u64>,
        projection: Vec<SqlProjection>,
        predicates: Vec<SqlExpression>,
    },
    Select {
        input: SqlRelationRef,
        predicate: SqlExpression,
    },
    Join {
        left: SqlRelationRef,
        right: SqlRelationRef,
        predicate: SqlExpression,
        left_outer: bool,
    },
    SemiJoin {
        input: SqlRelationRef,
        evidence: SqlRelationRef,
        predicate: SqlExpression,
    },
    AntiJoin {
        input: SqlRelationRef,
        excluded: SqlRelationRef,
        predicate: SqlExpression,
    },
    OrderedReduce {
        input: SqlRelationRef,
        partition_by: Vec<SqlExpression>,
        order_by: Vec<SqlOrderTerm>,
        limit: u64,
    },
    Aggregate {
        input: SqlRelationRef,
        group_by: Vec<SqlExpression>,
        aggregates: Vec<SqlAggregate>,
        having: Option<SqlExpression>,
    },
    Project {
        input: SqlRelationRef,
        projection: Vec<SqlProjection>,
        distinct: bool,
    },
    Union {
        inputs: Vec<SqlRelationRef>,
        distinct: bool,
    },
    Materialize {
        input: SqlRelationRef,
    },
    OutputMerge {
        inputs: Vec<SqlRelationRef>,
        order_by: Vec<SqlOrderTerm>,
    },
    Group {
        input: SqlRelationRef,
        partition_by: Vec<SqlExpression>,
    },
    Window {
        input: SqlRelationRef,
        partition_by: Vec<SqlExpression>,
        order_by: Vec<SqlOrderTerm>,
        output: SqlProjection,
    },
    Recursive {
        seed: SqlRelationRef,
        step: SqlRelationRef,
        union_all: bool,
        depth_column: SqlColumn,
        maximum_depth: u32,
    },
    Reference {
        relation: SqlRelationRef,
    },
}

impl PhysicalRelation {
    fn rendered_name(&self) -> String {
        match self {
            Self::BatchStatus => "rerail_batch_status".to_owned(),
            Self::ProducerStatus => "rerail_producer_status".to_owned(),
            Self::TerminalMatch => "rerail_terminal_match".to_owned(),
            Self::Capture => "rerail_capture".to_owned(),
            Self::Navigation => "rerail_navigation".to_owned(),
            Self::SemanticFact => "rerail_semantic_fact".to_owned(),
            Self::ValueFact => "rerail_value_fact".to_owned(),
            Self::ParseFact => "rerail_parse_fact".to_owned(),
            Self::TransformFact => "rerail_transform_fact".to_owned(),
            Self::DocumentMap => "rerail_document_map".to_owned(),
            Self::Clause(ordinal) => format!("clause_{ordinal}"),
            Self::ClauseOutput(ordinal) => format!("clause_{ordinal}_output"),
        }
    }
}

fn render_projection(projection: &[SqlProjection]) -> String {
    if projection.is_empty() {
        "*".to_owned()
    } else {
        projection
            .iter()
            .map(SqlProjection::render)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn render_predicates(predicates: &[SqlExpression]) -> String {
    if predicates.is_empty() {
        String::new()
    } else {
        format!(
            " WHERE {}",
            predicates
                .iter()
                .map(SqlExpression::render)
                .collect::<Vec<_>>()
                .join(" AND ")
        )
    }
}

fn render_ordering(ordering: &[SqlOrderTerm]) -> String {
    if ordering.is_empty() {
        String::new()
    } else {
        format!(
            " ORDER BY {}",
            ordering
                .iter()
                .map(SqlOrderTerm::render)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl SqlAstNode {
    pub(in crate::code_rule_dsl) const fn kind(&self) -> SqlAstNodeKind {
        match self {
            Self::RelationScan { .. } => SqlAstNodeKind::RelationScan,
            Self::DemandScan { .. } => SqlAstNodeKind::DemandScan,
            Self::Select { .. } => SqlAstNodeKind::Select,
            Self::Join { .. } => SqlAstNodeKind::Join,
            Self::SemiJoin { .. } => SqlAstNodeKind::SemiJoin,
            Self::AntiJoin { .. } => SqlAstNodeKind::AntiJoin,
            Self::OrderedReduce { .. } => SqlAstNodeKind::OrderedReduce,
            Self::Aggregate { .. } => SqlAstNodeKind::Aggregate,
            Self::Project { .. } => SqlAstNodeKind::Project,
            Self::Union { .. } => SqlAstNodeKind::Union,
            Self::Materialize { .. } => SqlAstNodeKind::Materialize,
            Self::OutputMerge { .. } => SqlAstNodeKind::OutputMerge,
            Self::Group { .. } => SqlAstNodeKind::Group,
            Self::Window { .. } => SqlAstNodeKind::Window,
            Self::Recursive { .. } => SqlAstNodeKind::Recursive,
            Self::Reference { .. } => SqlAstNodeKind::Reference,
        }
    }

    pub(in crate::code_rule_dsl) fn inputs(&self) -> Vec<SqlRelationRef> {
        match self {
            Self::RelationScan { .. } | Self::DemandScan { .. } => Vec::new(),
            Self::Select { input, .. }
            | Self::OrderedReduce { input, .. }
            | Self::Aggregate { input, .. }
            | Self::Project { input, .. }
            | Self::Materialize { input }
            | Self::Group { input, .. }
            | Self::Window { input, .. } => vec![*input],
            Self::Join { left, right, .. } => vec![*left, *right],
            Self::SemiJoin {
                input, evidence, ..
            } => vec![*input, *evidence],
            Self::AntiJoin {
                input, excluded, ..
            } => vec![*input, *excluded],
            Self::Union { inputs, .. } | Self::OutputMerge { inputs, .. } => inputs.clone(),
            Self::Recursive { seed, step, .. } => vec![*seed, *step],
            Self::Reference { relation } => vec![*relation],
        }
    }

    pub(super) fn render(&self) -> String {
        let reference = |relation: SqlRelationRef| quote_identifier(&relation.rendered_name());
        match self {
            Self::RelationScan {
                source,
                projection,
                predicates,
                ordering,
            } => format!(
                "SELECT {} FROM {}{}{}",
                render_projection(projection),
                quote_identifier(&source.rendered_name()),
                render_predicates(predicates),
                render_ordering(ordering)
            ),
            Self::DemandScan {
                source,
                producer_ids,
                projection,
                predicates,
            } => {
                let mut predicates = predicates.clone();
                let producer = SqlExpression::Column(SqlColumn {
                    relation: None,
                    name: "producer_id".to_owned(),
                });
                let demand = if producer_ids.is_empty() {
                    SqlExpression::Value(SqlValue::Boolean(false))
                } else {
                    SqlExpression::In {
                        input: Box::new(producer),
                        values: producer_ids
                            .iter()
                            .map(|producer_id| {
                                SqlExpression::Value(SqlValue::Integer(
                                    i64::try_from(*producer_id)
                                        .expect("validated producer identity fits SQLite INTEGER"),
                                ))
                            })
                            .collect(),
                    }
                };
                predicates.push(demand);
                format!(
                    "SELECT {} FROM {}{}",
                    render_projection(projection),
                    quote_identifier(&source.rendered_name()),
                    render_predicates(&predicates)
                )
            }
            Self::Select { input, predicate } => format!(
                "SELECT * FROM {} WHERE {}",
                reference(*input),
                predicate.render()
            ),
            Self::Join {
                left,
                right,
                predicate,
                left_outer,
            } => format!(
                "SELECT {}.*, {}.* FROM {} {} JOIN {} ON {}",
                reference(*left),
                reference(*right),
                reference(*left),
                if *left_outer { "LEFT" } else { "INNER" },
                reference(*right),
                predicate.render()
            ),
            Self::SemiJoin {
                input,
                evidence,
                predicate,
            } => format!(
                "SELECT * FROM {} WHERE EXISTS (SELECT 1 FROM {} WHERE {})",
                reference(*input),
                reference(*evidence),
                predicate.render()
            ),
            Self::AntiJoin {
                input,
                excluded,
                predicate,
            } => format!(
                "SELECT * FROM {} WHERE NOT EXISTS (SELECT 1 FROM {} WHERE {})",
                reference(*input),
                reference(*excluded),
                predicate.render()
            ),
            Self::OrderedReduce {
                input,
                partition_by,
                order_by,
                limit,
            } => {
                let partition = if partition_by.is_empty() {
                    String::new()
                } else {
                    format!(
                        "PARTITION BY {}",
                        partition_by
                            .iter()
                            .map(SqlExpression::render)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                let ordering = if order_by.is_empty() {
                    String::new()
                } else {
                    format!(
                        "{}ORDER BY {}",
                        if partition.is_empty() { "" } else { " " },
                        order_by
                            .iter()
                            .map(SqlOrderTerm::render)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                format!(
                    "SELECT * FROM (SELECT *, row_number() OVER ({partition}{ordering}) AS \"__rank\" FROM {}) WHERE \"__rank\" <= {limit}",
                    reference(*input)
                )
            }
            Self::Aggregate {
                input,
                group_by,
                aggregates,
                having,
            } => {
                let groups = group_by
                    .iter()
                    .map(SqlExpression::render)
                    .collect::<Vec<_>>();
                let mut selection = groups.clone();
                selection.extend(aggregates.iter().map(SqlAggregate::render));
                let group_clause = if groups.is_empty() {
                    String::new()
                } else {
                    format!(" GROUP BY {}", groups.join(", "))
                };
                let having_clause = having.as_ref().map_or_else(String::new, |predicate| {
                    format!(" HAVING {}", predicate.render())
                });
                format!(
                    "SELECT {} FROM {}{group_clause}{having_clause}",
                    selection.join(", "),
                    reference(*input)
                )
            }
            Self::Project {
                input,
                projection,
                distinct,
            } => format!(
                "SELECT {}{} FROM {}",
                if *distinct { "DISTINCT " } else { "" },
                render_projection(projection),
                reference(*input)
            ),
            Self::Union { inputs, distinct } => inputs
                .iter()
                .map(|input| format!("SELECT * FROM {}", reference(*input)))
                .collect::<Vec<_>>()
                .join(if *distinct { " UNION " } else { " UNION ALL " }),
            Self::Materialize { input } | Self::Reference { relation: input } => {
                format!("SELECT * FROM {}", reference(*input))
            }
            Self::OutputMerge { inputs, order_by } => format!(
                "{}{}",
                inputs
                    .iter()
                    .map(|input| format!("SELECT * FROM {}", reference(*input)))
                    .collect::<Vec<_>>()
                    .join(" UNION ALL "),
                render_ordering(order_by)
            ),
            Self::Group {
                input,
                partition_by,
            } => format!(
                "SELECT * FROM {}{}",
                reference(*input),
                if partition_by.is_empty() {
                    String::new()
                } else {
                    format!(
                        " GROUP BY {}",
                        partition_by
                            .iter()
                            .map(SqlExpression::render)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            ),
            Self::Window {
                input,
                partition_by,
                order_by,
                output,
            } => {
                let mut specification = Vec::new();
                if !partition_by.is_empty() {
                    specification.push(format!(
                        "PARTITION BY {}",
                        partition_by
                            .iter()
                            .map(SqlExpression::render)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !order_by.is_empty() {
                    specification.push(format!(
                        "ORDER BY {}",
                        order_by
                            .iter()
                            .map(SqlOrderTerm::render)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                format!(
                    "SELECT *, {} OVER ({}){} FROM {}",
                    output.expression.render(),
                    specification.join(" "),
                    output.alias.as_ref().map_or_else(String::new, |alias| {
                        format!(" AS {}", quote_identifier(alias))
                    }),
                    reference(*input)
                )
            }
            Self::Recursive {
                seed,
                step,
                union_all,
                depth_column,
                maximum_depth,
            } => format!(
                "SELECT * FROM {} {} SELECT * FROM {} WHERE {} <= {maximum_depth}",
                reference(*seed),
                if *union_all { "UNION ALL" } else { "UNION" },
                reference(*step),
                depth_column.render()
            ),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum SqlAstError {
    DuplicateRelation(SqlRelationRef),
    MissingRelation {
        relation: SqlRelationRef,
        input: SqlRelationRef,
    },
    Cycle(Vec<SqlRelationRef>),
    MissingOutput(SqlRelationRef),
    NoOutputs,
    ProducerIdOutOfRange(u64),
    InvalidNode {
        relation: SqlRelationRef,
        reason: &'static str,
    },
}

#[derive(Default)]
pub(in crate::code_rule_dsl) struct SqlAst {
    nodes: BTreeMap<SqlRelationRef, SqlAstNode>,
    outputs: Vec<SqlRelationRef>,
}

impl SqlAst {
    pub(in crate::code_rule_dsl) fn new() -> Self {
        Self::default()
    }

    pub(in crate::code_rule_dsl) fn insert(
        &mut self,
        relation: SqlRelationRef,
        node: SqlAstNode,
    ) -> Result<(), SqlAstError> {
        if self.nodes.insert(relation, node).is_some() {
            return Err(SqlAstError::DuplicateRelation(relation));
        }
        Ok(())
    }

    pub(in crate::code_rule_dsl) fn set_outputs(&mut self, outputs: Vec<SqlRelationRef>) {
        self.outputs = outputs;
    }

    pub(in crate::code_rule_dsl) fn node(
        &self,
        relation: SqlRelationRef,
    ) -> Option<&SqlAstNode> {
        self.nodes.get(&relation)
    }

    pub(in crate::code_rule_dsl) fn validate(&self) -> Result<(), SqlAstError> {
        if self.outputs.is_empty() {
            return Err(SqlAstError::NoOutputs);
        }
        for output in &self.outputs {
            if !self.nodes.contains_key(output) {
                return Err(SqlAstError::MissingOutput(*output));
            }
        }
        for (relation, node) in &self.nodes {
            if let SqlAstNode::DemandScan { producer_ids, .. } = node {
                if let Some(producer_id) = producer_ids
                    .iter()
                    .copied()
                    .find(|producer_id| i64::try_from(*producer_id).is_err())
                {
                    return Err(SqlAstError::ProducerIdOutOfRange(producer_id));
                }
                if producer_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
                    return Err(SqlAstError::InvalidNode {
                        relation: *relation,
                        reason: "producer demands must be sorted and unique",
                    });
                }
            }
            let invalid = match node {
                SqlAstNode::Union { inputs, .. } | SqlAstNode::OutputMerge { inputs, .. }
                    if inputs.is_empty() =>
                {
                    Some("set nodes require at least one input")
                }
                SqlAstNode::Aggregate {
                    group_by,
                    aggregates,
                    ..
                } if group_by.is_empty() && aggregates.is_empty() => {
                    Some("aggregate nodes require a group or aggregate")
                }
                SqlAstNode::OrderedReduce { limit: 0, .. } => {
                    Some("ordered reductions require a positive limit")
                }
                SqlAstNode::Recursive {
                    maximum_depth: 0, ..
                } => Some("recursive nodes require a positive depth limit"),
                _ => None,
            };
            if let Some(reason) = invalid {
                return Err(SqlAstError::InvalidNode {
                    relation: *relation,
                    reason,
                });
            }
            for input in node.inputs() {
                if !self.nodes.contains_key(&input) {
                    return Err(SqlAstError::MissingRelation {
                        relation: *relation,
                        input,
                    });
                }
            }
        }
        let mut complete = BTreeSet::new();
        let mut active = Vec::new();
        let mut positions = BTreeMap::new();
        fn visit(
            relation: SqlRelationRef,
            nodes: &BTreeMap<SqlRelationRef, SqlAstNode>,
            complete: &mut BTreeSet<SqlRelationRef>,
            active: &mut Vec<SqlRelationRef>,
            positions: &mut BTreeMap<SqlRelationRef, usize>,
        ) -> Result<(), SqlAstError> {
            if complete.contains(&relation) {
                return Ok(());
            }
            if let Some(position) = positions.get(&relation).copied() {
                let mut cycle = active[position..].to_vec();
                cycle.push(relation);
                return Err(SqlAstError::Cycle(cycle));
            }
            positions.insert(relation, active.len());
            active.push(relation);
            for input in nodes[&relation].inputs() {
                visit(input, nodes, complete, active, positions)?;
            }
            active.pop();
            positions.remove(&relation);
            complete.insert(relation);
            Ok(())
        }
        for relation in self.nodes.keys().copied() {
            visit(
                relation,
                &self.nodes,
                &mut complete,
                &mut active,
                &mut positions,
            )?;
        }
        Ok(())
    }

    fn ordered_relations(&self) -> Result<Vec<SqlRelationRef>, SqlAstError> {
        self.validate()?;
        fn visit(
            relation: SqlRelationRef,
            nodes: &BTreeMap<SqlRelationRef, SqlAstNode>,
            visited: &mut BTreeSet<SqlRelationRef>,
            order: &mut Vec<SqlRelationRef>,
        ) {
            if !visited.insert(relation) {
                return;
            }
            for input in nodes[&relation].inputs() {
                visit(input, nodes, visited, order);
            }
            order.push(relation);
        }
        let mut visited = BTreeSet::new();
        let mut order = Vec::with_capacity(self.nodes.len());
        for output in &self.outputs {
            visit(*output, &self.nodes, &mut visited, &mut order);
        }
        for relation in self.nodes.keys().copied() {
            visit(relation, &self.nodes, &mut visited, &mut order);
        }
        Ok(order)
    }

    fn render_parts(&self) -> Result<(bool, String, String), SqlAstError> {
        let order = self.ordered_relations()?;
        let recursive = self
            .nodes
            .values()
            .any(|node| matches!(node, SqlAstNode::Recursive { .. }));
        let ctes = order
            .into_iter()
            .map(|relation| {
                let node = &self.nodes[&relation];
                format!(
                    "{} AS{} (\n  {}\n)",
                    quote_identifier(&relation.rendered_name()),
                    if matches!(node, SqlAstNode::Materialize { .. }) {
                        " MATERIALIZED"
                    } else {
                        ""
                    },
                    node.render()
                )
            })
            .collect::<Vec<_>>()
            .join(",\n");
        let outputs = self
            .outputs
            .iter()
            .map(|output| {
                format!(
                    "SELECT * FROM {}",
                    quote_identifier(&output.rendered_name())
                )
            })
            .collect::<Vec<_>>()
            .join(" UNION ALL ");
        Ok((recursive, ctes, outputs))
    }

    pub(in crate::code_rule_dsl) fn render(&self) -> Result<String, SqlAstError> {
        let (recursive, ctes, outputs) = self.render_parts()?;
        Ok(format!(
            "WITH{} {ctes}\n{outputs}",
            if recursive { " RECURSIVE" } else { "" }
        ))
    }
}

pub(in crate::code_rule_dsl) struct ClauseMergeAst {
    ast: SqlAst,
    sources: Vec<PhysicalRelation>,
    outputs: bool,
}

impl ClauseMergeAst {
    pub(in crate::code_rule_dsl) fn new(
        ordinals: &[usize],
        outputs: bool,
    ) -> Result<Self, String> {
        if ordinals.is_empty() {
            return Err("a clause merge requires at least one input".to_owned());
        }
        let mut ast = SqlAst::new();
        let mut inputs = Vec::with_capacity(ordinals.len());
        let mut sources = Vec::with_capacity(ordinals.len());
        for (index, ordinal) in ordinals.iter().copied().enumerate() {
            let relation = SqlRelationRef::new(
                u32::try_from(index + 1).map_err(|_| "clause merge input count exceeds u32")?,
            );
            let ordinal = u32::try_from(ordinal).map_err(|_| "clause ordinal exceeds u32")?;
            let source = if outputs {
                PhysicalRelation::ClauseOutput(ordinal)
            } else {
                PhysicalRelation::Clause(ordinal)
            };
            ast.insert(
                relation,
                SqlAstNode::RelationScan {
                    source: source.clone(),
                    projection: Vec::new(),
                    predicates: Vec::new(),
                    ordering: Vec::new(),
                },
            )
            .map_err(|error| format!("invalid clause merge input: {error:?}"))?;
            inputs.push(relation);
            sources.push(source);
        }
        let merged = SqlRelationRef::new(
            u32::try_from(ordinals.len() + 1)
                .map_err(|_| "clause merge relation count exceeds u32")?,
        );
        let order_by = if outputs {
            ["clause_ordinal", "file_index", "stable_order"]
                .into_iter()
                .map(|name| SqlOrderTerm {
                    expression: SqlExpression::Column(SqlColumn {
                        relation: None,
                        name: name.to_owned(),
                    }),
                    descending: false,
                })
                .collect()
        } else {
            [
                "clause_ordinal",
                "file_index",
                "node",
                "choice_order",
                "producer_id",
                "match_index",
                "capture_index",
            ]
            .into_iter()
            .map(|name| SqlOrderTerm {
                expression: SqlExpression::Column(SqlColumn {
                    relation: None,
                    name: name.to_owned(),
                }),
                descending: false,
            })
            .collect()
        };
        ast.insert(merged, SqlAstNode::OutputMerge { inputs, order_by })
            .map_err(|error| format!("invalid clause output merge: {error:?}"))?;
        ast.set_outputs(vec![merged]);
        ast.validate()
            .map_err(|error| format!("invalid clause merge AST: {error:?}"))?;
        Ok(Self {
            ast,
            sources,
            outputs,
        })
    }

    pub(in crate::code_rule_dsl) fn render(&self) -> Result<(String, String), String> {
        self.ast
            .validate()
            .map_err(|error| format!("invalid clause merge AST: {error:?}"))?;
        let inputs = self
            .sources
            .iter()
            .map(|source| format!("SELECT * FROM {}", source.rendered_name()))
            .collect::<Vec<_>>()
            .join(" UNION ALL ");
        let inputs = if self.sources.len() == 1 {
            inputs
        } else {
            format!("SELECT * FROM ({inputs})")
        };
        let ordering = if self.outputs {
            "clause_ordinal, file_index, stable_order"
        } else {
            "clause_ordinal, file_index, node, choice_order, producer_id, match_index, capture_index"
        };
        Ok((String::new(), format!("{inputs} ORDER BY {ordering}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str) -> SqlExpression {
        SqlExpression::Column(SqlColumn {
            relation: None,
            name: name.to_owned(),
        })
    }

    fn truth() -> SqlExpression {
        SqlExpression::Value(SqlValue::Boolean(true))
    }

    #[test]
    fn every_typed_node_renders_from_explicit_edges() {
        let mut ast = SqlAst::new();
        let relation = |value| SqlRelationRef::new(value);
        ast.insert(
            relation(1),
            SqlAstNode::RelationScan {
                source: PhysicalRelation::TerminalMatch,
                projection: Vec::new(),
                predicates: vec![truth()],
                ordering: Vec::new(),
            },
        )
        .unwrap();
        ast.insert(
            relation(2),
            SqlAstNode::DemandScan {
                source: PhysicalRelation::Capture,
                producer_ids: vec![3, 7],
                projection: Vec::new(),
                predicates: Vec::new(),
            },
        )
        .unwrap();
        ast.insert(
            relation(3),
            SqlAstNode::Select {
                input: relation(1),
                predicate: truth(),
            },
        )
        .unwrap();
        ast.insert(
            relation(4),
            SqlAstNode::Join {
                left: relation(1),
                right: relation(2),
                predicate: truth(),
                left_outer: false,
            },
        )
        .unwrap();
        ast.insert(
            relation(5),
            SqlAstNode::SemiJoin {
                input: relation(1),
                evidence: relation(2),
                predicate: truth(),
            },
        )
        .unwrap();
        ast.insert(
            relation(6),
            SqlAstNode::AntiJoin {
                input: relation(1),
                excluded: relation(2),
                predicate: truth(),
            },
        )
        .unwrap();
        ast.insert(
            relation(7),
            SqlAstNode::OrderedReduce {
                input: relation(1),
                partition_by: vec![column("file_index")],
                order_by: vec![SqlOrderTerm {
                    expression: column("node"),
                    descending: false,
                }],
                limit: 1,
            },
        )
        .unwrap();
        ast.insert(
            relation(8),
            SqlAstNode::Aggregate {
                input: relation(1),
                group_by: vec![column("file_index")],
                aggregates: vec![SqlAggregate {
                    function: SqlAggregateFunction::Count,
                    expression: None,
                    distinct: false,
                    alias: "matches".to_owned(),
                }],
                having: Some(truth()),
            },
        )
        .unwrap();
        ast.insert(
            relation(9),
            SqlAstNode::Project {
                input: relation(1),
                projection: vec![SqlProjection {
                    expression: column("node"),
                    alias: Some("projected_node".to_owned()),
                }],
                distinct: true,
            },
        )
        .unwrap();
        ast.insert(
            relation(10),
            SqlAstNode::Union {
                inputs: vec![relation(1), relation(2)],
                distinct: false,
            },
        )
        .unwrap();
        ast.insert(relation(11), SqlAstNode::Materialize { input: relation(1) })
            .unwrap();
        ast.insert(
            relation(12),
            SqlAstNode::OutputMerge {
                inputs: vec![relation(3), relation(5)],
                order_by: vec![SqlOrderTerm {
                    expression: column("node"),
                    descending: false,
                }],
            },
        )
        .unwrap();
        ast.insert(
            relation(13),
            SqlAstNode::Group {
                input: relation(1),
                partition_by: vec![column("file_index")],
            },
        )
        .unwrap();
        ast.insert(
            relation(14),
            SqlAstNode::Window {
                input: relation(1),
                partition_by: vec![column("file_index")],
                order_by: vec![SqlOrderTerm {
                    expression: column("node"),
                    descending: false,
                }],
                output: SqlProjection {
                    expression: SqlExpression::Function {
                        name: "row_number".to_owned(),
                        arguments: Vec::new(),
                    },
                    alias: Some("position".to_owned()),
                },
            },
        )
        .unwrap();
        ast.insert(
            relation(15),
            SqlAstNode::Recursive {
                seed: relation(1),
                step: relation(3),
                union_all: true,
                depth_column: SqlColumn {
                    relation: None,
                    name: "depth".to_owned(),
                },
                maximum_depth: 32,
            },
        )
        .unwrap();
        ast.insert(
            relation(16),
            SqlAstNode::Reference {
                relation: relation(12),
            },
        )
        .unwrap();
        ast.set_outputs(vec![relation(16)]);

        let kinds = (1..=16)
            .map(|ordinal| ast.node(relation(ordinal)).unwrap().kind())
            .collect::<BTreeSet<_>>();
        assert_eq!(kinds, SqlAstNodeKind::ALL.into_iter().collect());
        let sql = ast.render().unwrap();
        for fragment in [
            "WITH RECURSIVE",
            "EXISTS",
            "NOT EXISTS",
            "\"row_number\"()",
            "GROUP BY",
            "UNION ALL",
            "AS MATERIALIZED",
        ] {
            assert!(sql.contains(fragment), "{fragment}: {sql}");
        }
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE rerail_terminal_match(node INTEGER, producer_id INTEGER, file_index INTEGER, depth INTEGER);\n                 CREATE TABLE rerail_capture(node INTEGER, producer_id INTEGER, file_index INTEGER, depth INTEGER);",
            )
            .unwrap();
        connection.prepare(&sql).expect("complete typed SQL AST");
    }

    #[test]
    fn rendered_ast_executes_against_an_ordinary_table_binding() {
        let scan = SqlRelationRef::new(1);
        let projected = SqlRelationRef::new(2);
        let mut ast = SqlAst::new();
        ast.insert(
            scan,
            SqlAstNode::RelationScan {
                source: PhysicalRelation::TerminalMatch,
                projection: vec![SqlProjection {
                    expression: column("node"),
                    alias: None,
                }],
                predicates: Vec::new(),
                ordering: Vec::new(),
            },
        )
        .unwrap();
        ast.insert(
            projected,
            SqlAstNode::Project {
                input: scan,
                projection: vec![SqlProjection {
                    expression: column("node"),
                    alias: Some("result_node".to_owned()),
                }],
                distinct: true,
            },
        )
        .unwrap();
        ast.set_outputs(vec![projected]);
        let sql = ast.render().unwrap();

        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE rerail_terminal_match(node INTEGER NOT NULL);\n                 INSERT INTO rerail_terminal_match(node) VALUES (7), (7);",
            )
            .unwrap();
        let rows = connection
            .prepare(&sql)
            .unwrap()
            .query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows, [7]);
    }

    #[test]
    fn clause_output_merge_uses_the_typed_ast_for_multi_clause_statements() {
        let (ctes, output) = ClauseMergeAst::new(&[0, 1], false)
            .unwrap()
            .render()
            .unwrap();
        let extra_ctes = if ctes.is_empty() {
            String::new()
        } else {
            format!(",\n{ctes}")
        };
        let sql = format!(
            "WITH clause_0 AS (SELECT 1 AS clause_ordinal, 1 AS file_index, 8 AS node, '' AS choice_order, 0 AS producer_id, 0 AS match_index, 0 AS capture_index),\nclause_1 AS (SELECT 0 AS clause_ordinal, 1 AS file_index, 9 AS node, '' AS choice_order, 0 AS producer_id, 0 AS match_index, 0 AS capture_index){extra_ctes}\n{output}"
        );
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        let rows = connection
            .prepare(&sql)
            .unwrap()
            .query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows, [0, 1]);
    }

    #[test]
    fn dependencies_are_explicit_and_cycles_fail_before_rendering() {
        let scan = SqlRelationRef::new(1);
        let selected = SqlRelationRef::new(2);
        let mut ast = SqlAst::new();
        ast.insert(
            scan,
            SqlAstNode::RelationScan {
                source: PhysicalRelation::TerminalMatch,
                projection: Vec::new(),
                predicates: Vec::new(),
                ordering: Vec::new(),
            },
        )
        .unwrap();
        ast.insert(selected, SqlAstNode::Reference { relation: scan })
            .unwrap();
        ast.set_outputs(vec![selected]);
        assert_eq!(ast.node(selected).unwrap().inputs(), [scan]);
        assert_eq!(ast.validate(), Ok(()));

        let mut cycle = SqlAst::new();
        cycle
            .insert(scan, SqlAstNode::Reference { relation: selected })
            .unwrap();
        cycle
            .insert(selected, SqlAstNode::Reference { relation: scan })
            .unwrap();
        cycle.set_outputs(vec![selected]);
        assert_eq!(
            cycle.validate(),
            Err(SqlAstError::Cycle(vec![scan, selected, scan]))
        );
    }
}
