use crate::code_rule_dsl::model::CompiledCodeRule;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TerminalKind {
    Kind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SqlShape {
    PackedMaterialized,
    StatusJoin,
    ScanTerminal,
    Projection,
    OrderBy,
}

#[derive(Debug)]
pub(super) struct TerminalPlan {
    id: usize,
    kind: TerminalKind,
    language: &'static str,
    source: String,
}

impl TerminalPlan {
    pub(super) const fn id(&self) -> usize {
        self.id
    }

    pub(super) const fn kind(&self) -> TerminalKind {
        self.kind
    }

    pub(super) const fn language(&self) -> &'static str {
        self.language
    }

    pub(super) fn source(&self) -> &str {
        &self.source
    }
}

#[derive(Debug)]
pub(super) struct SqlCatalogPlan {
    terminals: Vec<TerminalPlan>,
    shape: Vec<SqlShape>,
    sql: String,
    clause_terminal_ids: Vec<Vec<usize>>,
}

impl SqlCatalogPlan {
    pub(super) fn compile(_code_rules: &[Arc<CompiledCodeRule>]) -> Result<Self, LoweringError> {
        Err(LoweringError {
            unsupported: vec!["relational SQL lowering is not implemented".to_owned()],
        })
    }

    pub(super) fn terminals(&self) -> &[TerminalPlan] {
        &self.terminals
    }

    pub(super) fn shape(&self) -> &[SqlShape] {
        &self.shape
    }

    pub(super) fn sql(&self) -> &str {
        &self.sql
    }

    pub(super) fn clause_count(&self) -> usize {
        self.clause_terminal_ids.len()
    }

    pub(super) fn clause_terminal_ids(&self) -> &[Vec<usize>] {
        &self.clause_terminal_ids
    }

    pub(super) fn unsupported(&self) -> &[String] {
        &[]
    }
}

#[derive(Debug)]
pub(super) struct LoweringError {
    unsupported: Vec<String>,
}

impl LoweringError {
    pub(super) fn unsupported(&self) -> &[String] {
        &self.unsupported
    }
}
