use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum TerminalKind {
    Universe,
    Pattern,
    Kind,
    Regex,
    Text,
    Value,
    Navigation,
    Equality,
    Fact,
    Parse,
    CallSeed,
    Transform,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct TerminalSpec {
    kind: TerminalKind,
    semantic_key: String,
}

impl TerminalSpec {
    fn new(kind: TerminalKind, semantic_key: impl Into<String>) -> Self {
        Self {
            kind,
            semantic_key: semantic_key.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum SqlOperation {
    PackedMaterialized,
    StatusJoin,
    ScanTerminal,
    ScanCapture,
    ScanNavigation,
    ScanFact,
    InnerJoin,
    LeftJoin,
    OrderedUnion,
    AntiJoin,
    Exists,
    RecursiveCte,
    GroupBy,
    Having,
    WindowRank,
    MinPosition,
    Distinct,
    CaseTruth,
    Bindings,
    MapDocument,
    Projection,
    OrderBy,
    WhereFilter,
    FileFilter,
    CrossFileJoin,
}

const ALL_TERMINAL_KINDS: [TerminalKind; 12] = [
    TerminalKind::Universe,
    TerminalKind::Pattern,
    TerminalKind::Kind,
    TerminalKind::Regex,
    TerminalKind::Text,
    TerminalKind::Value,
    TerminalKind::Navigation,
    TerminalKind::Equality,
    TerminalKind::Fact,
    TerminalKind::Parse,
    TerminalKind::CallSeed,
    TerminalKind::Transform,
];

const ALL_SQL_OPERATIONS: [SqlOperation; 25] = [
    SqlOperation::PackedMaterialized,
    SqlOperation::StatusJoin,
    SqlOperation::ScanTerminal,
    SqlOperation::ScanCapture,
    SqlOperation::ScanNavigation,
    SqlOperation::ScanFact,
    SqlOperation::InnerJoin,
    SqlOperation::LeftJoin,
    SqlOperation::OrderedUnion,
    SqlOperation::AntiJoin,
    SqlOperation::Exists,
    SqlOperation::RecursiveCte,
    SqlOperation::GroupBy,
    SqlOperation::Having,
    SqlOperation::WindowRank,
    SqlOperation::MinPosition,
    SqlOperation::Distinct,
    SqlOperation::CaseTruth,
    SqlOperation::Bindings,
    SqlOperation::MapDocument,
    SqlOperation::Projection,
    SqlOperation::OrderBy,
    SqlOperation::WhereFilter,
    SqlOperation::FileFilter,
    SqlOperation::CrossFileJoin,
];

impl SqlOperation {
    pub(super) const fn fragment(self) -> &'static str {
        match self {
            Self::PackedMaterialized => "WITH packed AS MATERIALIZED",
            Self::StatusJoin => "JOIN producer_status",
            Self::ScanTerminal => "FROM terminal_match",
            Self::ScanCapture => "JOIN capture",
            Self::ScanNavigation => "JOIN navigation",
            Self::ScanFact => "JOIN fact",
            Self::InnerJoin => "INNER JOIN",
            Self::LeftJoin => "LEFT JOIN",
            Self::OrderedUnion => "UNION ALL /* branch_order */",
            Self::AntiJoin => "NOT EXISTS",
            Self::Exists => "EXISTS",
            Self::RecursiveCte => "WITH RECURSIVE",
            Self::GroupBy => "GROUP BY",
            Self::Having => "HAVING",
            Self::WindowRank => "row_number() OVER (ORDER BY",
            Self::MinPosition => "min(axis_order)",
            Self::Distinct => "DISTINCT",
            Self::CaseTruth => "CASE verdict",
            Self::Bindings => "JOIN environment_binding",
            Self::MapDocument => "JOIN document_map",
            Self::Projection => "SELECT /* violation projection */",
            Self::OrderBy => "ORDER BY",
            Self::WhereFilter => "WHERE",
            Self::FileFilter => "JOIN batch_file",
            Self::CrossFileJoin => "JOIN /* cross-file relation */",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NavigationAxis {
    Ancestor,
    Descendant,
    FollowingSibling,
    PrecedingSibling,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NavigationStop {
    Neighbor,
    End,
    Rule,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NavigationSpec {
    pub(super) axis: NavigationAxis,
    pub(super) stop: NavigationStop,
    pub(super) field: Option<u16>,
    pub(super) field_stop_children_only: bool,
    pub(super) producer: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NthChildSpec {
    pub(super) step: i32,
    pub(super) offset: i32,
    pub(super) reverse: bool,
    pub(super) producer: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CountSpec {
    pub(super) navigation: NavigationSpec,
    pub(super) minimum: usize,
    pub(super) maximum: Option<usize>,
    pub(super) binding: Option<(String, i64)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RelationNode {
    Seed,
    Producer(TerminalSpec),
    Apply {
        operation: SqlOperation,
        inputs: Vec<RelationNode>,
    },
    Navigate {
        spec: NavigationSpec,
        source: Box<RelationNode>,
        related: Box<RelationNode>,
        stop: Option<Box<RelationNode>>,
    },
    NthChild {
        spec: NthChildSpec,
        source: Box<RelationNode>,
        of_rule: Option<Box<RelationNode>>,
    },
    Constrain {
        source: Box<RelationNode>,
        constraints: Vec<(String, RelationNode)>,
    },
    Count {
        spec: CountSpec,
        source: Box<RelationNode>,
        related: Box<RelationNode>,
        stop: Option<Box<RelationNode>>,
    },
    PrefixChoice {
        source: Box<RelationNode>,
        branch: usize,
    },
    MergeChoices(Vec<RelationNode>),
    Rendered(String),
}

impl TerminalSpec {
    fn producer_id(&self) -> Result<usize, String> {
        self.semantic_key
            .strip_prefix("terminal:")
            .ok_or_else(|| "production terminal has no producer identity".to_owned())?
            .parse()
            .map_err(|_| "production terminal has an invalid producer identity".to_owned())
    }
}

impl RelationNode {
    fn apply_after(self, input: Self) -> Self {
        let join = |left, right| Self::Apply {
            operation: SqlOperation::InnerJoin,
            inputs: vec![left, right],
        };
        match self {
            Self::Count {
                spec,
                source,
                related,
                stop,
            } => Self::Count {
                spec,
                source: Box::new(join(input, *source)),
                related,
                stop,
            },
            Self::Navigate {
                spec,
                source,
                related,
                stop,
            } => Self::Navigate {
                spec,
                source: Box::new(join(input, *source)),
                related,
                stop,
            },
            Self::NthChild {
                spec,
                source,
                of_rule,
            } => Self::NthChild {
                spec,
                source: Box::new(join(input, *source)),
                of_rule,
            },
            Self::Apply {
                operation: SqlOperation::InnerJoin,
                inputs,
            } => inputs
                .into_iter()
                .fold(input, |current, nested| nested.apply_after(current)),
            Self::Apply {
                operation: SqlOperation::OrderedUnion,
                inputs,
            } => Self::MergeChoices(
                inputs
                    .into_iter()
                    .enumerate()
                    .map(|(branch, nested)| {
                        nested.apply_after(Self::PrefixChoice {
                            source: Box::new(input.clone()),
                            branch,
                        })
                    })
                    .collect(),
            ),
            Self::Apply {
                operation: SqlOperation::AntiJoin,
                inputs,
            } => {
                let [universe, excluded] = <Vec<Self> as TryInto<[Self; 2]>>::try_into(inputs)
                    .expect("a verified negation has universe and excluded operands");
                Self::Apply {
                    operation: SqlOperation::AntiJoin,
                    inputs: vec![join(input.clone(), universe), excluded.apply_after(input)],
                }
            }
            Self::Apply { operation, inputs }
                if matches!(
                    operation,
                    SqlOperation::CaseTruth
                        | SqlOperation::WindowRank
                        | SqlOperation::Projection
                        | SqlOperation::OrderBy
                ) =>
            {
                Self::Apply {
                    operation,
                    inputs: inputs
                        .into_iter()
                        .map(|nested| nested.apply_after(input.clone()))
                        .collect(),
                }
            }
            Self::Rendered(_) => input,
            operation => join(input, operation),
        }
    }

    fn can_bind(&self) -> bool {
        match self {
            Self::Producer(spec) => spec.kind == TerminalKind::Pattern,
            Self::Apply { inputs, .. } => inputs.iter().any(Self::can_bind),
            Self::Navigate {
                source, related, ..
            } => source.can_bind() || related.can_bind(),
            Self::NthChild {
                source, of_rule, ..
            } => source.can_bind() || of_rule.as_deref().is_some_and(Self::can_bind),
            Self::Constrain { .. } | Self::Count { .. } => true,
            Self::PrefixChoice { source, .. } => source.can_bind(),
            Self::MergeChoices(inputs) => inputs.iter().any(Self::can_bind),
            Self::Seed | Self::Rendered(_) => false,
        }
    }

    fn collect_terminals(&self, terminals: &mut Vec<TerminalKind>) {
        match self {
            Self::Seed | Self::Rendered(_) => {}
            Self::Producer(spec) => {
                if !terminals.contains(&spec.kind) {
                    terminals.push(spec.kind);
                }
            }
            Self::Apply { inputs, .. } | Self::MergeChoices(inputs) => {
                for input in inputs {
                    input.collect_terminals(terminals);
                }
            }
            Self::PrefixChoice { source, .. } => source.collect_terminals(terminals),
            Self::Navigate {
                spec,
                source,
                related,
                stop,
            } => {
                if !terminals.contains(&TerminalKind::Navigation) {
                    terminals.push(TerminalKind::Navigation);
                }
                let _ = spec;
                source.collect_terminals(terminals);
                related.collect_terminals(terminals);
                if let Some(stop) = stop {
                    stop.collect_terminals(terminals);
                }
            }
            Self::NthChild {
                source, of_rule, ..
            } => {
                if !terminals.contains(&TerminalKind::Navigation) {
                    terminals.push(TerminalKind::Navigation);
                }
                source.collect_terminals(terminals);
                if let Some(of_rule) = of_rule {
                    of_rule.collect_terminals(terminals);
                }
            }
            Self::Constrain {
                source,
                constraints,
            } => {
                source.collect_terminals(terminals);
                for (_, constraint) in constraints {
                    constraint.collect_terminals(terminals);
                }
            }
            Self::Count {
                source,
                related,
                stop,
                ..
            } => {
                if !terminals.contains(&TerminalKind::Navigation) {
                    terminals.push(TerminalKind::Navigation);
                }
                source.collect_terminals(terminals);
                related.collect_terminals(terminals);
                if let Some(stop) = stop {
                    stop.collect_terminals(terminals);
                }
            }
        }
    }

    fn collect_operations(&self, operations: &mut Vec<SqlOperation>) {
        match self {
            Self::Apply { operation, inputs } => {
                for input in inputs {
                    input.collect_operations(operations);
                }
                if !operations.contains(operation) {
                    operations.push(*operation);
                }
            }
            Self::Navigate {
                source,
                related,
                stop,
                ..
            } => {
                source.collect_operations(operations);
                related.collect_operations(operations);
                if let Some(stop) = stop {
                    stop.collect_operations(operations);
                }
                for operation in [SqlOperation::ScanNavigation, SqlOperation::InnerJoin] {
                    if !operations.contains(&operation) {
                        operations.push(operation);
                    }
                }
            }
            Self::NthChild {
                source, of_rule, ..
            } => {
                source.collect_operations(operations);
                if let Some(of_rule) = of_rule {
                    of_rule.collect_operations(operations);
                }
                for operation in [SqlOperation::ScanNavigation, SqlOperation::WindowRank] {
                    if !operations.contains(&operation) {
                        operations.push(operation);
                    }
                }
            }
            Self::Constrain {
                source,
                constraints,
            } => {
                source.collect_operations(operations);
                for (_, constraint) in constraints {
                    constraint.collect_operations(operations);
                }
                for operation in [SqlOperation::Bindings, SqlOperation::InnerJoin] {
                    if !operations.contains(&operation) {
                        operations.push(operation);
                    }
                }
            }
            Self::PrefixChoice { source, .. } => source.collect_operations(operations),
            Self::MergeChoices(inputs) => {
                for input in inputs {
                    input.collect_operations(operations);
                }
                if !operations.contains(&SqlOperation::OrderedUnion) {
                    operations.push(SqlOperation::OrderedUnion);
                }
            }
            Self::Count {
                source,
                related,
                stop,
                ..
            } => {
                source.collect_operations(operations);
                related.collect_operations(operations);
                if let Some(stop) = stop {
                    stop.collect_operations(operations);
                }
                for operation in [
                    SqlOperation::ScanNavigation,
                    SqlOperation::GroupBy,
                    SqlOperation::Having,
                ] {
                    if !operations.contains(&operation) {
                        operations.push(operation);
                    }
                }
            }
            Self::Seed | Self::Producer(_) | Self::Rendered(_) => {}
        }
    }
}

const ROW_COLUMNS: &str = "batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence";

struct SqlRenderer {
    clause: usize,
    next_relation: usize,
    ctes: Vec<String>,
}

impl SqlRenderer {
    fn new(clause: usize) -> Self {
        Self {
            clause,
            next_relation: 0,
            ctes: Vec::new(),
        }
    }

    fn render(&mut self, node: &RelationNode) -> Result<String, String> {
        let RelationNode::Apply { operation, inputs } = node else {
            return match node {
                RelationNode::Navigate {
                    spec,
                    source,
                    related,
                    stop,
                } => self.render_navigation(*spec, source, related, stop.as_deref()),
                RelationNode::NthChild {
                    spec,
                    source,
                    of_rule,
                } => self.render_nth_child(*spec, source, of_rule.as_deref()),
                RelationNode::Constrain {
                    source,
                    constraints,
                } => self.render_constraints(source, constraints),
                RelationNode::Count {
                    spec,
                    source,
                    related,
                    stop,
                } => self.render_count(spec, source, related, stop.as_deref()),
                RelationNode::PrefixChoice { source, branch } => {
                    let source = self.render(source)?;
                    let name = self.next_name("choice");
                    let columns = qualified_row_columns("source");
                    self.ctes.push(format!(
                        "{name} AS (\n  SELECT {columns}, source.bindings, source.choice_order || printf('%08d/', {branch}) AS choice_order FROM {source} source\n)"
                    ));
                    Ok(name)
                }
                RelationNode::Rendered(name) => Ok(name.clone()),
                RelationNode::MergeChoices(inputs) => {
                    let inputs = inputs
                        .iter()
                        .map(|input| self.render(input))
                        .collect::<Result<Vec<_>, _>>()?;
                    let name = self.next_name("relation");
                    self.ctes.push(format!(
                        "{name} AS (\n  {}\n)",
                        inputs
                            .iter()
                            .map(|input| format!("SELECT * FROM {input}"))
                            .collect::<Vec<_>>()
                            .join("\n  UNION ALL\n  ")
                    ));
                    Ok(name)
                }
                _ => Err("a clause root must be an executable relation".to_owned()),
            };
        };
        match operation {
            SqlOperation::ScanTerminal => self.render_terminal(inputs),
            SqlOperation::ScanCapture
            | SqlOperation::CaseTruth
            | SqlOperation::WindowRank
            | SqlOperation::Projection
            | SqlOperation::OrderBy => self.render_transparent(inputs),
            SqlOperation::InnerJoin => self.render_all(inputs),
            SqlOperation::OrderedUnion => self.render_any(inputs),
            SqlOperation::AntiJoin => self.render_not(inputs),
            unsupported => Err(format!(
                "SQL rendering is not implemented for {unsupported:?}"
            )),
        }
    }

    fn sql_text(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    fn capture_equality(left: &str, right: &str) -> String {
        format!(
            "({left}.node = {right}.node OR (({left}.named_leaf = 1 OR {right}.named_leaf = 1) AND {left}.source_text = {right}.source_text) OR ({left}.named_leaf = 0 AND {right}.named_leaf = 0 AND {left}.structural_key = {right}.structural_key))"
        )
    }

    fn json_capture_equality(left: &str, right: &str) -> String {
        format!(
            "(json_extract({left}.value, '$.node') = json_extract({right}.value, '$.node') OR ((json_extract({left}.value, '$.leaf') = 1 OR json_extract({right}.value, '$.leaf') = 1) AND json_extract({left}.value, '$.text') = json_extract({right}.value, '$.text')) OR (json_extract({left}.value, '$.leaf') = 0 AND json_extract({right}.value, '$.leaf') = 0 AND json_extract({left}.value, '$.structure') = json_extract({right}.value, '$.structure')))"
        )
    }

    fn binding_compatibility(left: &str, right: &str) -> String {
        format!(
            "NOT EXISTS (SELECT 1 FROM json_each({left}.bindings) left_binding JOIN json_each({right}.bindings) right_binding ON json_extract(right_binding.value, '$.label') = json_extract(left_binding.value, '$.label') WHERE json_extract(left_binding.value, '$.kind') <> json_extract(right_binding.value, '$.kind') OR (json_extract(left_binding.value, '$.kind') = 1 AND NOT {equal}) OR (json_extract(left_binding.value, '$.kind') = 2 AND json_extract(left_binding.value, '$.named') = 1 AND json_extract(right_binding.value, '$.named') = 1 AND json_extract(left_binding.value, '$.equality_occurrence') = json_extract(right_binding.value, '$.equality_occurrence') AND NOT {equal}) OR (json_extract(left_binding.value, '$.kind') = 2 AND (SELECT count(*) FROM json_each({left}.bindings) item WHERE json_extract(item.value, '$.label') = json_extract(left_binding.value, '$.label') AND json_extract(item.value, '$.named') = 1) <> (SELECT count(*) FROM json_each({right}.bindings) item WHERE json_extract(item.value, '$.label') = json_extract(right_binding.value, '$.label') AND json_extract(item.value, '$.named') = 1)))",
            equal = Self::json_capture_equality("left_binding", "right_binding")
        )
    }

    fn merged_bindings(inputs: &[String]) -> String {
        let values = inputs
            .iter()
            .enumerate()
            .map(|(input_order, input)| {
                format!(
                    "SELECT {input_order} AS input_order, CAST(key AS INTEGER) AS capture_order, value FROM json_each({input}.bindings)"
                )
            })
            .collect::<Vec<_>>()
            .join(" UNION ALL ");
        format!(
            "COALESCE((SELECT json_group_array(json(value)) FROM (SELECT value FROM ({values}) GROUP BY value ORDER BY min(input_order), min(capture_order))), json('[]'))"
        )
    }

    fn render_terminal(&mut self, inputs: &[RelationNode]) -> Result<String, String> {
        let [RelationNode::Producer(spec)] = inputs else {
            return Err("a terminal scan must have one producer operand".to_owned());
        };
        let producer = spec.producer_id()?;
        let name = self.next_name("term");
        let columns = qualified_row_columns("match");
        self.ctes.push(format!(
            "{name} AS (\n  SELECT {columns}, COALESCE((\n    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))\n    FROM capture captured\n    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index\n      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'\n    ORDER BY captured.capture_index\n  ), json('[]')) AS bindings, '' AS choice_order\n  FROM terminal_match match\n  INNER JOIN producer_status status USING (producer_id, file_index)\n  WHERE match.producer_id = {producer} AND status.complete = 1\n    AND NOT EXISTS (\n      SELECT 1 FROM capture first\n      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index\n        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label\n        AND repeated.capture_index > first.capture_index\n      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index\n        AND first.match_index = match.match_index AND first.capture_label <> 'match'\n        AND first.capture_kind = 1 AND repeated.capture_kind = 1\n        AND NOT {}\n    )\n)",
            Self::capture_equality("first", "repeated")
        ));
        Ok(name)
    }

    fn render_transparent(&mut self, inputs: &[RelationNode]) -> Result<String, String> {
        let [input] = inputs else {
            return Err("a unary relational operation must have one operand".to_owned());
        };
        self.render(input)
    }

    fn render_all(&mut self, inputs: &[RelationNode]) -> Result<String, String> {
        if inputs.len() < 2 {
            return Err("ordered conjunction requires at least two operands".to_owned());
        }
        let relations = inputs
            .iter()
            .map(|input| self.render(input))
            .collect::<Result<Vec<_>, _>>()?;
        let name = self.next_name("relation");
        if !inputs.iter().any(RelationNode::can_bind) {
            let mut sql = format!(
                "{name} AS (\n  SELECT left_input.* FROM {} left_input",
                relations[0]
            );
            for (index, relation) in relations.iter().enumerate().skip(1) {
                let alias = if index == 1 {
                    "right_input".to_owned()
                } else {
                    format!("input_{index}")
                };
                sql.push_str(&format!(
                    "\n  INNER JOIN {relation} {alias}\n    ON {alias}.file_index = left_input.file_index AND {alias}.node = left_input.node"
                ));
            }
            sql.push_str("\n)");
            self.ctes.push(sql);
            return Ok(name);
        }
        let columns = qualified_row_columns("left_input");
        let merged = Self::merged_bindings(
            &relations
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    if index == 0 {
                        "left_input".to_owned()
                    } else if index == 1 {
                        "right_input".to_owned()
                    } else {
                        format!("input_{index}")
                    }
                })
                .collect::<Vec<_>>(),
        );
        let choices = relations
            .iter()
            .enumerate()
            .map(|(index, _)| {
                if index == 0 {
                    "left_input.choice_order".to_owned()
                } else if index == 1 {
                    "right_input.choice_order".to_owned()
                } else {
                    format!("input_{index}.choice_order")
                }
            })
            .collect::<Vec<_>>()
            .join(" || ");
        let mut sql = format!(
            "{name} AS (\n  SELECT {columns}, {merged} AS bindings, {choices} AS choice_order FROM {} left_input",
            relations[0]
        );
        for (index, relation) in relations.iter().enumerate().skip(1) {
            let alias = if index == 1 {
                "right_input".to_owned()
            } else {
                format!("input_{index}")
            };
            let compatible = (0..index)
                .map(|prior| {
                    let prior = if prior == 0 {
                        "left_input".to_owned()
                    } else if prior == 1 {
                        "right_input".to_owned()
                    } else {
                        format!("input_{prior}")
                    };
                    Self::binding_compatibility(&prior, &alias)
                })
                .collect::<Vec<_>>()
                .join(" AND ");
            sql.push_str(&format!(
                "\n  INNER JOIN {relation} {alias}\n    ON {alias}.file_index = left_input.file_index AND {alias}.node = left_input.node\n    AND {compatible}"
            ));
        }
        sql.push_str("\n)");
        self.ctes.push(sql);
        Ok(name)
    }

    fn render_any(&mut self, inputs: &[RelationNode]) -> Result<String, String> {
        if inputs.len() < 2 {
            return Err("ordered alternatives require at least two operands".to_owned());
        }
        let relations = inputs
            .iter()
            .map(|input| self.render(input))
            .collect::<Result<Vec<_>, _>>()?;
        let name = self.next_name("relation");
        let branches = relations
            .iter()
            .enumerate()
            .map(|(branch, relation)| {
                let columns = qualified_row_columns("branch");
                format!(
                    "SELECT {columns}, branch.bindings, printf('%08d/', {branch}) || branch.choice_order AS choice_order FROM {relation} branch"
                )
            })
            .collect::<Vec<_>>()
            .join("\n  UNION ALL\n  ");
        self.ctes.push(format!("{name} AS (\n  {branches}\n)"));
        Ok(name)
    }

    fn render_not(&mut self, inputs: &[RelationNode]) -> Result<String, String> {
        let [universe, excluded] = inputs else {
            return Err("negation requires universe and excluded operands".to_owned());
        };
        let universe = self.render(universe)?;
        let excluded = self.render(excluded)?;
        let name = self.next_name("relation");
        self.ctes.push(format!(
            "{name} AS (\n  SELECT universe.* FROM {universe} universe\n  WHERE NOT EXISTS (\n    SELECT 1 FROM {excluded} excluded\n    WHERE excluded.file_index = universe.file_index AND excluded.node = universe.node\n      AND {}\n  )\n)",
            Self::binding_compatibility("universe", "excluded")
        ));
        Ok(name)
    }

    fn render_navigation(
        &mut self,
        spec: NavigationSpec,
        source: &RelationNode,
        related: &RelationNode,
        stop: Option<&RelationNode>,
    ) -> Result<String, String> {
        let source = self.render(source)?;
        let name = self.next_name("relation");
        let walk = format!("{name}_walk");
        let field = spec
            .field
            .map_or("1".to_owned(), |field| format!("edge.field_id = {field}"));
        match spec.axis {
            NavigationAxis::Descendant | NavigationAxis::Ancestor => {
                let (seed_join, step_join, candidate) = if spec.axis == NavigationAxis::Descendant {
                    (
                        "edge.node = source.node",
                        "edge.node = walk.candidate",
                        "edge.related_node",
                    )
                } else {
                    (
                        "edge.related_node = source.node",
                        "edge.related_node = walk.candidate",
                        "edge.node",
                    )
                };
                let recursive_field =
                    if spec.axis == NavigationAxis::Descendant && spec.field.is_some() {
                        "1".to_owned()
                    } else {
                        field.clone()
                    };
                let recurse = if spec.stop == NavigationStop::Neighbor {
                    String::new()
                } else {
                    let guard =
                        if spec.field_stop_children_only && spec.stop == NavigationStop::Rule {
                            "\n  WHERE walk.axis_order = 1"
                        } else {
                            ""
                        };
                    format!(
                        "\n  UNION ALL\n  SELECT walk.file_index, walk.source_node, {candidate}, walk.axis_order + 1, {recursive_field}, walk.path || printf('/%d', {candidate}), walk.source_bindings\n  FROM {walk} walk\n  JOIN navigation edge ON edge.producer_id = {} AND edge.file_index = walk.file_index AND {step_join}{guard}",
                        spec.producer
                    )
                };
                self.ctes.push(format!(
                    "{walk}(file_index, source_node, candidate, axis_order, eligible, path, source_bindings) AS (\n  SELECT source.file_index, source.node, {candidate}, 1, {field}, printf('/%d', {candidate}), source.bindings\n  FROM {source} source\n  JOIN navigation edge ON edge.producer_id = {} AND edge.file_index = source.file_index AND {seed_join}{recurse}\n)",
                    spec.producer
                ));
            }
            NavigationAxis::FollowingSibling | NavigationAxis::PrecedingSibling => {
                let forward = spec.axis == NavigationAxis::FollowingSibling;
                let comparison = if forward { ">" } else { "<" };
                let neighbor = if spec.stop == NavigationStop::Neighbor {
                    if forward {
                        "AND candidate.axis_order = origin.axis_order + 1"
                    } else {
                        "AND candidate.axis_order = origin.axis_order - 1"
                    }
                } else {
                    ""
                };
                self.ctes.push(format!(
                    "{walk}(file_index, source_node, candidate, axis_order, eligible, path, source_bindings) AS (\n  SELECT source.file_index, source.node, candidate.related_node, abs(candidate.axis_order - origin.axis_order), 1, printf('/%d', candidate.related_node), source.bindings\n  FROM {source} source\n  JOIN navigation origin ON origin.producer_id = {} AND origin.file_index = source.file_index AND origin.related_node = source.node\n  JOIN navigation candidate ON candidate.producer_id = {} AND candidate.file_index = origin.file_index AND candidate.node = origin.node\n  WHERE candidate.axis_order {comparison} origin.axis_order {neighbor}\n)",
                    spec.producer, spec.producer
                ));
            }
        }
        let context = format!("{name}_context");
        let context_columns = qualified_row_columns_with_node("source", "walk.candidate");
        self.ctes.push(format!(
            "{context} AS (\n  SELECT {context_columns}, source.bindings, source.choice_order\n  FROM {walk} walk\n  JOIN {source} source ON source.file_index = walk.file_index AND source.node = walk.source_node\n)"
        ));
        let related = related
            .clone()
            .apply_after(RelationNode::Rendered(context.clone()));
        let related = self.render(&related)?;
        let stop = stop
            .map(|stop| {
                self.render(
                    &stop
                        .clone()
                        .apply_after(RelationNode::Rendered(context.clone())),
                )
            })
            .transpose()?;
        let candidate_stop_limit = stop.as_ref().map_or(String::new(), |stop| match spec.axis {
            NavigationAxis::Descendant => format!(
                "\n    AND NOT EXISTS (SELECT 1 FROM {walk} earlier JOIN {stop} stopped ON stopped.file_index = earlier.file_index AND stopped.node = earlier.candidate AND stopped.related_node = earlier.source_node WHERE earlier.file_index = walk.file_index AND earlier.source_node = walk.source_node AND earlier.path <> walk.path AND substr(walk.path, 1, length(earlier.path) + 1) = earlier.path || '/' AND {})",
                Self::binding_compatibility("related", "stopped")
            ),
            NavigationAxis::Ancestor => format!(
                "\n    AND NOT EXISTS (SELECT 1 FROM {walk} earlier JOIN {stop} stopped ON stopped.file_index = earlier.file_index AND stopped.node = earlier.candidate AND stopped.related_node = earlier.source_node WHERE earlier.file_index = walk.file_index AND earlier.source_node = walk.source_node AND earlier.axis_order < walk.axis_order AND {})",
                Self::binding_compatibility("related", "stopped")
            ),
            NavigationAxis::FollowingSibling | NavigationAxis::PrecedingSibling => {
                let between = "earlier.axis_order < walk.axis_order";
                format!(
                    "\n    AND NOT EXISTS (SELECT 1 FROM {walk} earlier JOIN {stop} stopped ON stopped.file_index = earlier.file_index AND stopped.node = earlier.candidate AND stopped.related_node = earlier.source_node WHERE earlier.file_index = walk.file_index AND earlier.source_node = walk.source_node AND {between} AND {})",
                    Self::binding_compatibility("related", "stopped")
                )
            }
        });
        let order = if spec.axis == NavigationAxis::Descendant {
            "walk.candidate ASC"
        } else {
            "walk.axis_order ASC"
        };
        let accepted = format!("{name}_accepted");
        self.ctes.push(format!(
            "{accepted} AS (\n  SELECT walk.file_index, walk.source_node, related.bindings, related.choice_order, row_number() OVER (PARTITION BY walk.file_index, walk.source_node, walk.source_bindings ORDER BY {order}, walk.candidate, related.choice_order, related.producer_id, related.match_index) AS accepted_order\n  FROM {walk} walk\n  JOIN {related} related ON related.file_index = walk.file_index AND related.node = walk.candidate AND related.related_node = walk.source_node\n  WHERE walk.eligible{candidate_stop_limit}\n)"
        ));
        let columns = qualified_row_columns("source");
        self.ctes.push(format!(
            "{name} AS (\n  SELECT {columns}, accepted.bindings, accepted.choice_order FROM {source} source\n  JOIN {accepted} accepted ON accepted.file_index = source.file_index AND accepted.source_node = source.node AND accepted.accepted_order = 1\n)"
        ));
        Ok(name)
    }

    fn render_constraints(
        &mut self,
        source: &RelationNode,
        constraints: &[(String, RelationNode)],
    ) -> Result<String, String> {
        let mut current = self.render(source)?;
        for (capture, constraint) in constraints {
            let constraint = self.render(constraint)?;
            let capture = Self::sql_text(capture);
            let name = self.next_name("constraint");
            let columns = qualified_row_columns("source");
            let merged = Self::merged_bindings(&["source".to_owned(), "checked".to_owned()]);
            let compatible = Self::binding_compatibility("source", "checked");
            self.ctes.push(format!(
                "{name} AS (\n  SELECT {columns}, {merged} AS bindings, source.choice_order || checked.choice_order AS choice_order\n  FROM {current} source\n  JOIN json_each(source.bindings) bound\n    ON json_extract(bound.value, '$.label') = {capture} AND json_extract(bound.value, '$.kind') = 1 AND json_extract(bound.value, '$.occurrence') = 0\n  JOIN {constraint} checked\n    ON checked.file_index = source.file_index AND checked.node = json_extract(bound.value, '$.node')\n    AND {compatible}\n  UNION ALL\n  SELECT {columns}, source.bindings, source.choice_order\n  FROM {current} source\n  WHERE NOT EXISTS (SELECT 1 FROM json_each(source.bindings) bound WHERE json_extract(bound.value, '$.label') = {capture} AND json_extract(bound.value, '$.kind') = 1)\n)"
            ));
            current = name;
        }
        Ok(current)
    }

    fn render_count(
        &mut self,
        spec: &CountSpec,
        source: &RelationNode,
        related: &RelationNode,
        stop: Option<&RelationNode>,
    ) -> Result<String, String> {
        let source = self.render(source)?;
        let name = self.next_name("count");
        let candidates = format!("{name}_candidates");
        let producer = spec.navigation.producer;
        match spec.navigation.axis {
            NavigationAxis::Descendant | NavigationAxis::Ancestor => {
                let descendant = spec.navigation.axis == NavigationAxis::Descendant;
                let seed_join = if descendant {
                    "edge.node = source.node"
                } else {
                    "edge.related_node = source.node"
                };
                let step_join = if descendant {
                    "edge.node = walk.candidate"
                } else {
                    "edge.related_node = walk.candidate"
                };
                let candidate = if descendant {
                    "edge.related_node"
                } else {
                    "edge.node"
                };
                let recurse = if spec.navigation.stop == NavigationStop::Neighbor {
                    String::new()
                } else {
                    format!(
                        "\n  UNION ALL\n  SELECT walk.file_index, walk.source_node, {candidate}, walk.axis_order + 1, walk.path || printf('/%d', {candidate})\n  FROM {candidates} walk\n  JOIN navigation edge ON edge.producer_id = {producer} AND edge.file_index = walk.file_index AND {step_join}"
                    )
                };
                self.ctes.push(format!(
                    "{candidates}(file_index, source_node, candidate, axis_order, path) AS (\n  SELECT source.file_index, source.node, {candidate}, 1, printf('/%d', {candidate})\n  FROM {source} source\n  JOIN navigation edge ON edge.producer_id = {producer} AND edge.file_index = source.file_index AND {seed_join}{recurse}\n)"
                ));
            }
            NavigationAxis::FollowingSibling | NavigationAxis::PrecedingSibling => {
                let forward = spec.navigation.axis == NavigationAxis::FollowingSibling;
                let comparison = if forward { ">" } else { "<" };
                let neighbor = if spec.navigation.stop == NavigationStop::Neighbor {
                    if forward {
                        "AND candidate.axis_order = origin.axis_order + 1"
                    } else {
                        "AND candidate.axis_order = origin.axis_order - 1"
                    }
                } else {
                    ""
                };
                self.ctes.push(format!(
                    "{candidates} AS (\n  SELECT source.file_index, source.node AS source_node, candidate.related_node AS candidate, abs(candidate.axis_order - origin.axis_order) AS axis_order, printf('/%d', candidate.related_node) AS path\n  FROM {source} source\n  JOIN navigation origin ON origin.producer_id = {producer} AND origin.file_index = source.file_index AND origin.related_node = source.node\n  JOIN navigation candidate ON candidate.producer_id = {producer} AND candidate.file_index = origin.file_index AND candidate.node = origin.node\n  WHERE candidate.axis_order {comparison} origin.axis_order {neighbor}\n)"
                ));
            }
        }
        let context = format!("{name}_context");
        let context_columns = qualified_row_columns_with_node("source", "candidate.candidate");
        self.ctes.push(format!(
            "{context} AS (\n  SELECT {context_columns}, source.bindings, source.choice_order\n  FROM {candidates} candidate\n  JOIN {source} source ON source.file_index = candidate.file_index AND source.node = candidate.source_node\n)"
        ));
        let related = related
            .clone()
            .apply_after(RelationNode::Rendered(context.clone()));
        let related = self.render(&related)?;
        let stop = stop
            .map(|stop| {
                self.render(
                    &stop
                        .clone()
                        .apply_after(RelationNode::Rendered(context.clone())),
                )
            })
            .transpose()?;
        let stop_limit = stop.as_ref().map_or(String::new(), |stop| match spec.navigation.axis {
            NavigationAxis::Descendant => format!(
                " AND NOT EXISTS (SELECT 1 FROM {candidates} earlier JOIN {stop} stopped ON stopped.file_index = earlier.file_index AND stopped.node = earlier.candidate AND stopped.related_node = earlier.source_node WHERE earlier.file_index = candidate.file_index AND earlier.source_node = candidate.source_node AND earlier.path <> candidate.path AND substr(candidate.path, 1, length(earlier.path) + 1) = earlier.path || '/' AND {})",
                Self::binding_compatibility("source", "stopped")
            ),
            NavigationAxis::FollowingSibling | NavigationAxis::PrecedingSibling => {
                let comparison = if spec.navigation.axis == NavigationAxis::FollowingSibling {
                    "<"
                } else {
                    ">"
                };
                format!(
                    " AND NOT EXISTS (SELECT 1 FROM {candidates} earlier JOIN {stop} stopped ON stopped.file_index = earlier.file_index AND stopped.node = earlier.candidate AND stopped.related_node = earlier.source_node WHERE earlier.file_index = candidate.file_index AND earlier.source_node = candidate.source_node AND earlier.candidate {comparison} candidate.candidate AND {})",
                    Self::binding_compatibility("source", "stopped")
                )
            }
            NavigationAxis::Ancestor => format!(
                " AND NOT EXISTS (SELECT 1 FROM {candidates} earlier JOIN {stop} stopped ON stopped.file_index = earlier.file_index AND stopped.node = earlier.candidate AND stopped.related_node = earlier.source_node WHERE earlier.file_index = candidate.file_index AND earlier.source_node = candidate.source_node AND earlier.axis_order < candidate.axis_order AND {})",
                Self::binding_compatibility("source", "stopped")
            ),
        });
        let counts = format!("{name}_values");
        self.ctes.push(format!(
            "{counts} AS (\n  SELECT source.*, (\n    SELECT count(DISTINCT candidate.candidate) FROM {candidates} candidate\n    JOIN {related} related ON related.file_index = candidate.file_index AND related.node = candidate.candidate AND related.related_node = source.node AND {}\n    WHERE candidate.file_index = source.file_index AND candidate.source_node = source.node{stop_limit}\n  ) AS found\n  FROM {source} source\n)",
            Self::binding_compatibility("source", "related")
        ));
        let maximum = spec.maximum.map_or(String::new(), |maximum| {
            format!(" AND counted.found <= {maximum}")
        });
        let columns = qualified_row_columns("counted");
        let bindings = spec.binding.as_ref().map_or_else(
            || "counted.bindings".to_owned(),
            |(binding, offset)| {
                let binding = Self::sql_text(binding);
                format!(
                    "COALESCE((SELECT json_group_array(json(value)) FROM (SELECT value FROM json_each(counted.bindings) UNION ALL SELECT json_object('label', {binding}, 'occurrence', 0, 'node', NULL, 'text', CAST(counted.found + ({offset}) AS TEXT), 'structure', NULL, 'leaf', 0, 'kind', 1, 'named', 0, 'equality_occurrence', 0))), json('[]'))"
                )
            },
        );
        self.ctes.push(format!(
            "{name} AS (\n  SELECT {columns}, {bindings} AS bindings, counted.choice_order FROM {counts} counted\n  WHERE counted.found >= {}{maximum}\n)",
            spec.minimum
        ));
        Ok(name)
    }

    fn render_nth_child(
        &mut self,
        spec: NthChildSpec,
        source: &RelationNode,
        of_rule: Option<&RelationNode>,
    ) -> Result<String, String> {
        let source = self.render(source)?;
        let name = self.next_name("relation");
        let direction = if spec.reverse { ">=" } else { "<=" };
        let position = if spec.step == 0 {
            format!("position = {}", spec.offset)
        } else if spec.step > 0 {
            format!(
                "position - {} >= 0 AND (position - {}) % {} = 0",
                spec.offset, spec.offset, spec.step
            )
        } else {
            format!(
                "position - {} <= 0 AND (position - {}) % {} = 0",
                spec.offset, spec.offset, spec.step
            )
        };
        let (counted_join, source_join, output_bindings, output_choice) = if let Some(of_rule) =
            of_rule
        {
            let siblings = format!("{name}_siblings");
            let columns = qualified_row_columns_with_node("source", "candidate.related_node");
            self.ctes.push(format!(
                "{siblings} AS (\n  SELECT {columns}, source.bindings, source.choice_order\n  FROM {source} source\n  JOIN navigation edge ON edge.producer_id = {} AND edge.file_index = source.file_index AND edge.related_node = source.node\n  JOIN navigation candidate ON candidate.producer_id = {} AND candidate.file_index = edge.file_index AND candidate.node = edge.node AND candidate.related_named = 1\n)",
                spec.producer, spec.producer
            ));
            let matches = of_rule
                .clone()
                .apply_after(RelationNode::Rendered(siblings));
            let matches = self.render(&matches)?;
            (
                format!(
                    "\n      JOIN {matches} counted_rule ON counted_rule.file_index = counted.file_index AND counted_rule.node = counted.related_node AND counted_rule.related_node = source.node AND {}",
                    Self::binding_compatibility("source", "counted_rule")
                ),
                format!(
                    "\n  JOIN {matches} source_rule ON source_rule.file_index = source.file_index AND source_rule.node = source.node AND source_rule.related_node = source.node AND {}",
                    Self::binding_compatibility("source", "source_rule")
                ),
                Self::merged_bindings(&["source".to_owned(), "source_rule".to_owned()]),
                "source.choice_order || source_rule.choice_order".to_owned(),
            )
        } else {
            (
                String::new(),
                String::new(),
                "source.bindings".to_owned(),
                "source.choice_order".to_owned(),
            )
        };
        self.ctes.push(format!(
            "{name}_positions AS (\n  SELECT source.file_index, source.node, source.bindings, {output_bindings} AS output_bindings, {output_choice} AS output_choice, (\n    SELECT count(DISTINCT counted.related_node) FROM navigation counted{counted_join}\n    WHERE counted.producer_id = {} AND counted.file_index = edge.file_index AND counted.node = edge.node\n      AND counted.related_named = 1 AND counted.axis_order {direction} edge.axis_order\n  ) AS position\n  FROM {source} source{source_join}\n  JOIN navigation edge ON edge.producer_id = {} AND edge.file_index = source.file_index AND edge.related_node = source.node\n  WHERE edge.related_named = 1\n),\n{name} AS (\n  SELECT {} , position.output_bindings AS bindings, position.output_choice AS choice_order FROM {source} source\n  JOIN {name}_positions position ON position.file_index = source.file_index AND position.node = source.node AND position.bindings = source.bindings\n  WHERE {position}\n)",
            spec.producer,
            spec.producer,
            qualified_row_columns("source")
        ));
        Ok(name)
    }

    fn peek_name(&self, family: &str) -> String {
        format!("clause_{}_{family}_{}", self.clause, self.next_relation)
    }

    fn next_name(&mut self, family: &str) -> String {
        let name = self.peek_name(family);
        self.next_relation += 1;
        name
    }
}

#[cfg(test)]
pub(super) fn normalized_binding_compatibility(left: &str, right: &str) -> String {
    SqlRenderer::binding_compatibility(left, right)
}

fn qualified_row_columns_with_node(alias: &str, node: &str) -> String {
    ROW_COLUMNS
        .split(", ")
        .map(|column| {
            if column == "node" {
                format!("{node} AS node")
            } else if column == "related_node" {
                format!("{alias}.node AS related_node")
            } else {
                format!("{alias}.{column}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn qualified_row_columns(alias: &str) -> String {
    ROW_COLUMNS
        .split(", ")
        .map(|column| format!("{alias}.{column}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RelationalIr {
    roots: Vec<RelationNode>,
    terminal_kinds: Vec<TerminalKind>,
    operations: Vec<SqlOperation>,
}

impl Default for RelationalIr {
    fn default() -> Self {
        Self::empty()
    }
}

impl RelationalIr {
    pub(super) fn empty() -> Self {
        Self {
            roots: Vec::new(),
            terminal_kinds: Vec::new(),
            operations: vec![SqlOperation::PackedMaterialized, SqlOperation::StatusJoin],
        }
    }

    pub(super) fn terminal_spec(
        &mut self,
        terminal: TerminalKind,
        semantic_key: impl Into<String>,
    ) {
        self.roots.push(RelationNode::Producer(TerminalSpec::new(
            terminal,
            semantic_key,
        )));
        self.refresh_explanation();
    }

    fn one_root(self) -> Result<RelationNode, String> {
        let [root] = <Vec<RelationNode> as TryInto<[RelationNode; 1]>>::try_into(self.roots)
            .map_err(|_| "a relational operand must have exactly one root".to_owned())?;
        Ok(root)
    }

    pub(super) fn navigate(
        spec: NavigationSpec,
        source: Self,
        related: Self,
        stop: Option<Self>,
    ) -> Result<Self, String> {
        let node = RelationNode::Navigate {
            spec,
            source: Box::new(source.one_root()?),
            related: Box::new(related.one_root()?),
            stop: stop.map(Self::one_root).transpose()?.map(Box::new),
        };
        let mut ir = Self {
            roots: vec![node],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        ir.refresh_explanation();
        Ok(ir)
    }

    pub(super) fn nth_child(
        spec: NthChildSpec,
        source: Self,
        of_rule: Option<Self>,
    ) -> Result<Self, String> {
        let node = RelationNode::NthChild {
            spec,
            source: Box::new(source.one_root()?),
            of_rule: of_rule.map(Self::one_root).transpose()?.map(Box::new),
        };
        let mut ir = Self {
            roots: vec![node],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        ir.refresh_explanation();
        Ok(ir)
    }

    pub(super) fn constrain(
        source: Self,
        constraints: Vec<(String, Self)>,
    ) -> Result<Self, String> {
        let constraints = constraints
            .into_iter()
            .map(|(name, constraint)| Ok((name, constraint.one_root()?)))
            .collect::<Result<Vec<_>, String>>()?;
        let node = RelationNode::Constrain {
            source: Box::new(source.one_root()?),
            constraints,
        };
        let mut ir = Self {
            roots: vec![node],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        ir.refresh_explanation();
        Ok(ir)
    }

    pub(super) fn count(
        spec: CountSpec,
        source: Self,
        related: Self,
        stop: Option<Self>,
    ) -> Result<Self, String> {
        let node = RelationNode::Count {
            spec,
            source: Box::new(source.one_root()?),
            related: Box::new(related.one_root()?),
            stop: stop.map(Self::one_root).transpose()?.map(Box::new),
        };
        let mut ir = Self {
            roots: vec![node],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        ir.refresh_explanation();
        Ok(ir)
    }

    pub(super) fn combine(operation: SqlOperation, parts: Vec<Self>) -> Result<Self, String> {
        if parts.is_empty() {
            return Err(format!(
                "{operation:?} requires at least one relational operand"
            ));
        }
        let mut inputs = Vec::with_capacity(parts.len());
        for part in parts {
            if part.roots.len() != 1 {
                return Err("a relational operand must have exactly one root".to_owned());
            }
            inputs.extend(part.roots);
        }
        let root = if operation == SqlOperation::InnerJoin {
            let mut inputs = inputs.into_iter();
            let mut accumulated = inputs
                .next()
                .expect("the nonempty operand check supplies the first relation");
            for input in inputs {
                accumulated = input.apply_after(accumulated);
            }
            accumulated
        } else {
            RelationNode::Apply { operation, inputs }
        };
        let mut combined = Self {
            roots: vec![root],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        combined.refresh_explanation();
        Ok(combined)
    }

    pub(super) fn operation(&mut self, operation: SqlOperation) {
        assert!(
            !matches!(
                operation,
                SqlOperation::PackedMaterialized | SqlOperation::StatusJoin
            ),
            "packed scan and status join are statement prelude operations"
        );
        let inputs = if self.roots.is_empty() {
            vec![RelationNode::Seed]
        } else {
            std::mem::take(&mut self.roots)
        };
        self.roots.push(RelationNode::Apply { operation, inputs });
        self.refresh_explanation();
    }

    pub(super) fn contains_terminal_spec(&self, kind: TerminalKind, semantic_key: &str) -> bool {
        fn contains(node: &RelationNode, kind: TerminalKind, semantic_key: &str) -> bool {
            match node {
                RelationNode::Seed | RelationNode::Rendered(_) => false,
                RelationNode::Producer(spec) => {
                    spec.kind == kind && spec.semantic_key == semantic_key
                }
                RelationNode::Apply { inputs, .. } | RelationNode::MergeChoices(inputs) => inputs
                    .iter()
                    .any(|input| contains(input, kind, semantic_key)),
                RelationNode::PrefixChoice { source, .. } => contains(source, kind, semantic_key),
                RelationNode::Navigate {
                    spec,
                    source,
                    related,
                    stop,
                } => {
                    (kind == TerminalKind::Navigation
                        && semantic_key == format!("terminal:{}", spec.producer))
                        || contains(source, kind, semantic_key)
                        || contains(related, kind, semantic_key)
                        || stop
                            .as_deref()
                            .is_some_and(|stop| contains(stop, kind, semantic_key))
                }
                RelationNode::NthChild {
                    spec,
                    source,
                    of_rule,
                } => {
                    (kind == TerminalKind::Navigation
                        && semantic_key == format!("terminal:{}", spec.producer))
                        || contains(source, kind, semantic_key)
                        || of_rule
                            .as_deref()
                            .is_some_and(|rule| contains(rule, kind, semantic_key))
                }
                RelationNode::Constrain {
                    source,
                    constraints,
                } => {
                    contains(source, kind, semantic_key)
                        || constraints
                            .iter()
                            .any(|(_, rule)| contains(rule, kind, semantic_key))
                }
                RelationNode::Count {
                    spec,
                    source,
                    related,
                    stop,
                } => {
                    (kind == TerminalKind::Navigation
                        && semantic_key == format!("terminal:{}", spec.navigation.producer))
                        || contains(source, kind, semantic_key)
                        || contains(related, kind, semantic_key)
                        || stop
                            .as_deref()
                            .is_some_and(|rule| contains(rule, kind, semantic_key))
                }
            }
        }

        self.roots
            .iter()
            .any(|root| contains(root, kind, semantic_key))
    }

    #[cfg(test)]
    pub(super) fn operations(&self) -> &[SqlOperation] {
        &self.operations
    }

    pub(super) fn render_clause_relations(
        &self,
        clause_ordinal: usize,
    ) -> Result<(String, String), String> {
        if self.roots.len() != 1 {
            return Err("a clause must have exactly one relational root".to_owned());
        }
        let mut renderer = SqlRenderer::new(clause_ordinal);
        let result = renderer.render(&self.roots[0])?;
        let selected = renderer.next_name("selected");
        renderer.ctes.push(format!(
            "{selected} AS (\n  SELECT candidate.* FROM {result} candidate\n  WHERE NOT EXISTS (\n    SELECT 1 FROM {result} earlier\n    WHERE earlier.file_index = candidate.file_index AND earlier.node = candidate.node\n      AND (earlier.choice_order < candidate.choice_order\n        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id < candidate.producer_id)\n        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id = candidate.producer_id AND earlier.match_index < candidate.match_index))\n  )\n)"
        ));
        Ok((renderer.ctes.join(",\n"), selected))
    }

    pub(super) fn normalized_sql(&self) -> String {
        self.operations
            .iter()
            .map(|operation| operation.fragment())
            .collect::<Vec<_>>()
            .join(" -> ")
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        if self.operations.first() != Some(&SqlOperation::PackedMaterialized) {
            return Err("relational IR must start with a materialized packed scan".to_owned());
        }
        if self.operations.get(1) != Some(&SqlOperation::StatusJoin) {
            return Err("relational IR must join producer status before evaluation".to_owned());
        }
        if ALL_TERMINAL_KINDS.len() != ALL_TERMINAL_KINDS.iter().collect::<BTreeSet<_>>().len() {
            return Err("relational IR terminal catalog contains duplicate kinds".to_owned());
        }
        if ALL_SQL_OPERATIONS.len() != ALL_SQL_OPERATIONS.iter().collect::<BTreeSet<_>>().len() {
            return Err("relational IR SQL catalog contains duplicate operations".to_owned());
        }
        let mut terminals = Vec::new();
        let mut operations = vec![SqlOperation::PackedMaterialized, SqlOperation::StatusJoin];
        for root in &self.roots {
            root.collect_terminals(&mut terminals);
            root.collect_operations(&mut operations);
        }
        if terminals != self.terminal_kinds || operations != self.operations {
            return Err("relational IR explanation does not match its expression graph".to_owned());
        }
        Ok(())
    }

    fn refresh_explanation(&mut self) {
        let mut terminals = Vec::new();
        let mut operations = vec![SqlOperation::PackedMaterialized, SqlOperation::StatusJoin];
        for root in &self.roots {
            root.collect_terminals(&mut terminals);
            root.collect_operations(&mut operations);
        }
        self.terminal_kinds = terminals;
        self.operations = operations;
    }
}
