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
    fn collect_terminals(&self, terminals: &mut Vec<TerminalKind>) {
        match self {
            Self::Seed => {}
            Self::Producer(spec) => {
                if !terminals.contains(&spec.kind) {
                    terminals.push(spec.kind);
                }
            }
            Self::Apply { inputs, .. } => {
                for input in inputs {
                    input.collect_terminals(terminals);
                }
            }
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
            Self::Seed | Self::Producer(_) => {}
        }
    }

    #[cfg(test)]
    fn insert_before(&mut self, target: SqlOperation, inserted: &[SqlOperation]) -> bool {
        let Self::Apply { operation, inputs } = self else {
            return false;
        };
        if *operation == target {
            let original = std::mem::take(inputs);
            let mut replacement = RelationNode::Apply {
                operation: inserted[0],
                inputs: original,
            };
            for operation in &inserted[1..] {
                replacement = RelationNode::Apply {
                    operation: *operation,
                    inputs: vec![replacement],
                };
            }
            *inputs = vec![replacement];
            return true;
        }
        inputs
            .iter_mut()
            .any(|input| input.insert_before(target, inserted))
    }
}

const ROW_COLUMNS: &str = "batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named";

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

    fn render_terminal(&mut self, inputs: &[RelationNode]) -> Result<String, String> {
        let [RelationNode::Producer(spec)] = inputs else {
            return Err("a terminal scan must have one producer operand".to_owned());
        };
        let producer = spec.producer_id()?;
        let name = self.next_name("term");
        self.ctes.push(format!(
            "{name} AS (\n  SELECT match.* FROM terminal_match match\n  INNER JOIN producer_status status USING (producer_id, file_index)\n  WHERE match.producer_id = {producer} AND status.complete = 1\n)"
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
        let relation_name = self.peek_name("relation");
        let candidates = format!("{relation_name}_candidates");
        let branches = relations
            .iter()
            .enumerate()
            .map(|(branch, relation)| {
                format!("SELECT {branch} AS branch_order, branch.* FROM {relation} branch")
            })
            .collect::<Vec<_>>()
            .join("\n  UNION ALL\n  ");
        self.ctes
            .push(format!("{candidates} AS (\n  {branches}\n)"));
        let name = self.next_name("relation");
        debug_assert_eq!(name, relation_name);
        self.ctes.push(format!(
            "{name} AS (\n  SELECT candidate.* FROM {candidates} candidate\n  WHERE NOT EXISTS (\n    SELECT 1 FROM {candidates} earlier\n    WHERE earlier.file_index = candidate.file_index AND earlier.node = candidate.node\n      AND earlier.branch_order < candidate.branch_order\n  )\n)"
        ));
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
            "{name} AS (\n  SELECT universe.* FROM {universe} universe\n  WHERE NOT EXISTS (\n    SELECT 1 FROM {excluded} excluded\n    WHERE excluded.file_index = universe.file_index AND excluded.node = universe.node\n  )\n)"
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
        let related = self.render(related)?;
        let stop = stop.map(|stop| self.render(stop)).transpose()?;
        let name = self.next_name("relation");
        let edges = "navigation edge";
        let field = spec
            .field
            .map_or("1".to_owned(), |field| format!("edge.field_id = {field}"));
        let recursive_stop_guard = stop.as_ref().map_or(String::new(), |stop| {
            if spec.axis == NavigationAxis::Descendant && !spec.field_stop_children_only {
                String::new()
            } else {
                format!(
                    "\n    AND NOT EXISTS (SELECT 1 FROM {stop} stopped WHERE stopped.file_index = walk.file_index AND stopped.node = walk.candidate)"
                )
            }
        });
        let candidate_stop_limit = stop.as_ref().map_or(String::new(), |stop| {
            if spec.axis == NavigationAxis::Descendant && !spec.field_stop_children_only {
                format!(
                    "\n      AND NOT EXISTS (SELECT 1 FROM {name}_walk earlier JOIN {stop} stopped ON stopped.file_index = earlier.file_index AND stopped.node = earlier.candidate WHERE earlier.file_index = walk.file_index AND earlier.source_node = walk.source_node AND earlier.candidate < walk.candidate)"
                )
            } else {
                String::new()
            }
        });
        let sql = match spec.axis {
            NavigationAxis::Descendant | NavigationAxis::Ancestor => {
                let (seed_join, step_join) = if spec.axis == NavigationAxis::Descendant {
                    ("edge.node = source.node", "edge.node = walk.candidate")
                } else {
                    (
                        "edge.related_node = source.node",
                        "edge.related_node = walk.candidate",
                    )
                };
                let candidate = if spec.axis == NavigationAxis::Descendant {
                    "edge.related_node"
                } else {
                    "edge.node"
                };
                let recursive = spec.stop != NavigationStop::Neighbor;
                let recursive_field =
                    if spec.axis == NavigationAxis::Descendant && spec.field.is_some() {
                        "1".to_owned()
                    } else {
                        field.clone()
                    };
                let children_only =
                    spec.field_stop_children_only && spec.stop == NavigationStop::Rule;
                let recurse = if recursive {
                    format!(
                        "\n  UNION ALL\n  SELECT walk.file_index, walk.source_node, {candidate}, walk.axis_order + 1, {recursive_field}\n  FROM {name}_walk walk\n  JOIN {edges} ON edge.producer_id = {} AND edge.file_index = walk.file_index AND {step_join}\n  WHERE {}{}",
                        spec.producer,
                        if children_only {
                            "walk.axis_order = 1"
                        } else {
                            "1"
                        },
                        recursive_stop_guard
                    )
                } else {
                    String::new()
                };
                format!(
                    "{name}_walk(file_index, source_node, candidate, axis_order, eligible) AS (\n  SELECT source.file_index, source.node, {candidate}, 1, {field}\n  FROM {source} source\n  JOIN {edges} ON edge.producer_id = {} AND edge.file_index = source.file_index AND {seed_join}{recurse}\n),\n{name} AS (\n  SELECT source.* FROM {source} source\n  WHERE EXISTS (\n    SELECT 1 FROM {name}_walk walk\n    JOIN {related} related ON related.file_index = walk.file_index AND related.node = walk.candidate\n    WHERE walk.source_node = source.node AND walk.file_index = source.file_index AND walk.eligible{candidate_stop_limit}\n    ORDER BY walk.axis_order, walk.candidate LIMIT 1\n  )\n)",
                    spec.producer
                )
            }
            NavigationAxis::FollowingSibling | NavigationAxis::PrecedingSibling => {
                let forward = spec.axis == NavigationAxis::FollowingSibling;
                let comparison = if forward { ">" } else { "<" };
                let order = if forward { "ASC" } else { "DESC" };
                let neighbor = if spec.stop == NavigationStop::Neighbor {
                    if forward {
                        "AND candidate.axis_order = origin.axis_order + 1"
                    } else {
                        "AND candidate.axis_order = origin.axis_order - 1"
                    }
                } else {
                    ""
                };
                let stopped = stop.as_ref().map_or(String::new(), |stop| format!(
                    "\n      AND NOT EXISTS (\n        SELECT 1 FROM navigation between_edge\n        JOIN {stop} stopped ON stopped.file_index = between_edge.file_index AND stopped.node = between_edge.related_node\n        WHERE between_edge.producer_id = {} AND between_edge.file_index = source.file_index\n          AND between_edge.node = origin.node AND between_edge.axis_order {comparison} origin.axis_order\n          AND between_edge.axis_order {opposite} candidate.axis_order\n      )",
                    spec.producer,
                    opposite = if forward { "<" } else { ">" }
                ));
                format!(
                    "{name} AS (\n  SELECT source.* FROM {source} source\n  WHERE EXISTS (\n    SELECT 1 FROM navigation origin\n    JOIN navigation candidate ON candidate.producer_id = {} AND candidate.file_index = origin.file_index AND candidate.node = origin.node\n    JOIN {related} related ON related.file_index = candidate.file_index AND related.node = candidate.related_node\n    WHERE origin.producer_id = {} AND origin.file_index = source.file_index AND origin.related_node = source.node\n      AND candidate.axis_order {comparison} origin.axis_order {neighbor}{stopped}\n    ORDER BY candidate.axis_order {order} LIMIT 1\n  )\n)",
                    spec.producer, spec.producer
                )
            }
        };
        self.ctes.push(sql);
        Ok(name)
    }

    fn render_nth_child(
        &mut self,
        spec: NthChildSpec,
        source: &RelationNode,
        of_rule: Option<&RelationNode>,
    ) -> Result<String, String> {
        let source = self.render(source)?;
        let of_rule = of_rule.map(|rule| self.render(rule)).transpose()?;
        let counted = of_rule.as_ref().map_or(String::new(), |rule| {
            format!("\n      JOIN {rule} counted_rule ON counted_rule.file_index = counted.file_index AND counted_rule.node = counted.related_node")
        });
        let source_rule = of_rule.as_ref().map_or(String::new(), |rule| {
            format!("\n  JOIN {rule} source_rule ON source_rule.file_index = source.file_index AND source_rule.node = source.node")
        });
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
        let name = self.next_name("relation");
        self.ctes.push(format!(
            "{name}_positions AS (\n  SELECT source.file_index, source.node, (\n    SELECT count(*) FROM navigation counted{counted}\n    WHERE counted.producer_id = {} AND counted.file_index = edge.file_index AND counted.node = edge.node\n      AND counted.related_named = 1 AND counted.axis_order {direction} edge.axis_order\n  ) AS position\n  FROM {source} source{source_rule}\n  JOIN navigation edge ON edge.producer_id = {} AND edge.file_index = source.file_index AND edge.related_node = source.node\n  WHERE edge.related_named = 1\n),\n{name} AS (\n  SELECT source.* FROM {source} source\n  JOIN {name}_positions position ON position.file_index = source.file_index AND position.node = source.node\n  WHERE {position}\n)",
            spec.producer, spec.producer
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

    #[cfg(test)]
    pub(super) fn terminal(&mut self, terminal: TerminalKind) {
        self.terminal_spec(terminal, format!("{terminal:?}"));
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
        let mut combined = Self {
            roots: vec![RelationNode::Apply { operation, inputs }],
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

    #[cfg(test)]
    pub(super) fn insert_operations_before(
        &mut self,
        target: SqlOperation,
        inserted: &[SqlOperation],
    ) -> Result<(), String> {
        if inserted.is_empty() {
            return Ok(());
        }
        if !self
            .roots
            .iter_mut()
            .any(|root| root.insert_before(target, inserted))
        {
            return Err(format!("relational IR has no {target:?} operation"));
        }
        self.refresh_explanation();
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn terminals(&self) -> &[TerminalKind] {
        &self.terminal_kinds
    }

    pub(super) fn contains_terminal_spec(&self, kind: TerminalKind, semantic_key: &str) -> bool {
        fn contains(node: &RelationNode, kind: TerminalKind, semantic_key: &str) -> bool {
            match node {
                RelationNode::Seed => false,
                RelationNode::Producer(spec) => {
                    spec.kind == kind && spec.semantic_key == semantic_key
                }
                RelationNode::Apply { inputs, .. } => inputs
                    .iter()
                    .any(|input| contains(input, kind, semantic_key)),
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
        Ok((renderer.ctes.join(",\n"), result))
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
