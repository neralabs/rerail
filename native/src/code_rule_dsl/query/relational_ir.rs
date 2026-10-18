use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum TerminalKind {
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

const ALL_TERMINAL_KINDS: [TerminalKind; 11] = [
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

#[derive(Clone, Debug, Eq, PartialEq)]
enum RelationNode {
    Seed,
    Producer(TerminalSpec),
    Apply {
        operation: SqlOperation,
        inputs: Vec<RelationNode>,
    },
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
        }
    }

    fn collect_operations(&self, operations: &mut Vec<SqlOperation>) {
        if let Self::Apply { operation, inputs } = self {
            for input in inputs {
                input.collect_operations(operations);
            }
            if !operations.contains(operation) {
                operations.push(*operation);
            }
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
