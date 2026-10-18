use super::relational_ir::{RelationalIr, SqlOperation, qualified_row_columns};
use crate::code_rule_dsl::language::{GrammarDialect, label};
use crate::code_rule_dsl::matcher::AstMatcher;
use crate::code_rule_dsl::model::{ClauseKind, CompiledCodeRule, Query};
use crate::code_rule_dsl::packed::Program;
use ast_grep_core::matcher::Matcher;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

#[cfg(not(test))]
use super::relational_ir::TerminalKind;
#[cfg(test)]
pub(super) use super::relational_ir::{SqlOperation as SqlShape, TerminalKind};

pub(super) struct TerminalPlan {
    kind: TerminalKind,
    language: GrammarDialect,
    program: Program,
    clauses: Vec<(usize, usize)>,
    result_limit: u32,
}

impl TerminalPlan {
    #[cfg(test)]
    pub(super) const fn kind(&self) -> TerminalKind {
        self.kind
    }

    #[cfg(test)]
    pub(super) fn language(&self) -> &'static str {
        label(self.language).expect("a compiled matcher has a registered dialect")
    }

    pub(super) fn program(&self) -> &Program {
        &self.program
    }

    pub(super) const fn dialect(&self) -> GrammarDialect {
        self.language
    }

    pub(super) fn clauses(&self) -> &[(usize, usize)] {
        &self.clauses
    }

    pub(super) const fn result_limit(&self) -> u32 {
        self.result_limit
    }
}

pub(super) struct SqlCatalogPlan {
    terminals: Vec<TerminalPlan>,
    clause_irs: Vec<RelationalIr>,
    sql: String,
    #[cfg(test)]
    clause_terminal_ids: Vec<Vec<usize>>,
}

impl std::fmt::Debug for SqlCatalogPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SqlCatalogPlan")
            .field("terminals", &self.terminals.len())
            .field("sql", &self.sql)
            .finish()
    }
}

impl SqlCatalogPlan {
    pub(super) fn compile(code_rules: &[Arc<CompiledCodeRule>]) -> Result<Self, LoweringError> {
        let mut builder = Builder::default();
        for (rule_index, code_rule) in code_rules.iter().enumerate() {
            for (clause_index, clause) in code_rule.clauses.iter().enumerate() {
                let clause_at = format!("{}/{}", code_rule.id, clause.id);
                let ClauseKind::Forbid(Query::Ast { matchers, calls }) = &clause.kind else {
                    builder
                        .unsupported
                        .push(format!("{clause_at}: clause relation is not lowered"));
                    continue;
                };
                if !calls.is_empty() {
                    builder
                        .unsupported
                        .push(format!("{clause_at}: cross-file calls are not lowered"));
                }
                for matcher in matchers {
                    let language = label(matcher.language).unwrap_or("unknown");
                    let at = format!("{clause_at}/{language}/rule");
                    let mut terminals = Vec::new();
                    builder.current_clause = (rule_index, clause_index);
                    builder.current_result_limit =
                        u32::try_from(code_rule.capabilities.limits.max_witnesses)
                            .unwrap_or(u32::MAX);
                    let mut ir = RelationalIr::empty();
                    builder.lower_matcher(
                        &matcher.matcher,
                        matcher.language,
                        &at,
                        &mut terminals,
                        &mut ir,
                    );
                    builder.clause_terminal_ids.push(terminals);
                    builder.clause_irs.push(ir);
                }
            }
        }
        if !builder.unsupported.is_empty() {
            return Err(LoweringError {
                unsupported: builder.unsupported,
            });
        }
        let plan = builder.finish()?;
        plan.verify().map_err(|error| LoweringError {
            unsupported: vec![format!("generated relational plan: {error}")],
        })?;
        Ok(plan)
    }

    pub(super) fn terminals(&self) -> &[TerminalPlan] {
        &self.terminals
    }

    #[cfg(test)]
    pub(super) fn shape(&self) -> &[SqlShape] {
        self.clause_irs
            .first()
            .expect("a test SQL plan has one clause")
            .operations()
    }

    pub(super) fn verify(&self) -> Result<(), String> {
        for ir in &self.clause_irs {
            ir.validate()?;
        }
        if self.terminals.iter().enumerate().any(|(id, terminal)| {
            let semantic_key = format!("terminal:{id}");
            !self
                .clause_irs
                .iter()
                .any(|ir| ir.contains_terminal_spec(terminal.kind, &semantic_key))
        }) {
            return Err("SQL catalog terminal is absent from relational IR producers".to_owned());
        }
        for normalized in self.clause_irs.iter().map(RelationalIr::normalized_sql) {
            for prohibited in ["tq_", "rerail_nodes", "rerail_facts", "GrammarMatcher"] {
                if normalized.contains(prohibited) {
                    return Err(format!(
                        "relational IR contains prohibited SQL `{prohibited}`"
                    ));
                }
            }
        }
        if self.sql.trim().is_empty() {
            return Err("SQL catalog plan has no statement".to_owned());
        }
        for prohibited in ["tq_", "rerail_nodes", "rerail_facts", "GrammarMatcher"] {
            if self.sql.contains(prohibited) {
                return Err(format!("generated SQL contains prohibited `{prohibited}`"));
            }
        }
        if self.sql.matches("rerail_matches").count() != 1 {
            return Err("generated SQL must contain one packed relation scan".to_owned());
        }
        Ok(())
    }

    pub(super) fn sql(&self) -> &str {
        &self.sql
    }

    #[cfg(test)]
    pub(super) fn clause_count(&self) -> usize {
        self.clause_terminal_ids.len()
    }

    #[cfg(test)]
    pub(super) fn clause_terminal_ids(&self) -> &[Vec<usize>] {
        &self.clause_terminal_ids
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum TerminalIdentity {
    Universe(GrammarDialect),
    Kind(GrammarDialect, u16),
    Pattern(GrammarDialect, String),
    Regex(GrammarDialect, String),
}

#[derive(Default)]
struct Builder {
    terminals: Vec<TerminalPlan>,
    terminal_ids: HashMap<TerminalIdentity, usize>,
    clause_irs: Vec<RelationalIr>,
    clause_terminal_ids: Vec<Vec<usize>>,
    unsupported: Vec<String>,
    current_clause: (usize, usize),
    current_result_limit: u32,
}

impl Builder {
    fn lower_matcher(
        &mut self,
        matcher: &AstMatcher,
        language: GrammarDialect,
        at: &str,
        terminals: &mut Vec<usize>,
        ir: &mut RelationalIr,
    ) {
        match matcher {
            AstMatcher::Kind(kind) => {
                let Some(kinds) = kind.potential_kinds() else {
                    self.unsupported
                        .push(format!("{at}: dynamic kind terminal"));
                    return;
                };
                for kind in kinds.iter() {
                    let Ok(kind) = u16::try_from(kind) else {
                        self.unsupported.push(format!("{at}: kind id exceeds u16"));
                        continue;
                    };
                    let id = self.kind_terminal(language, kind);
                    if !terminals.contains(&id) {
                        terminals.push(id);
                    }
                    ir.terminal_spec(TerminalKind::Kind, format!("terminal:{id}"));
                    ir.operation(SqlOperation::ScanTerminal);
                }
            }
            AstMatcher::All(items) => {
                let operation = format!("{at}/all");
                let mut parts = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    let mut part = RelationalIr::empty();
                    self.lower_matcher(
                        item,
                        language,
                        &format!("{operation}/{index}"),
                        terminals,
                        &mut part,
                    );
                    parts.push(part);
                }
                match RelationalIr::combine(SqlOperation::InnerJoin, parts) {
                    Ok(mut combined) => {
                        combined.operation(SqlOperation::CaseTruth);
                        *ir = combined;
                    }
                    Err(error) => self.unsupported.push(format!("{operation}: {error}")),
                }
            }
            AstMatcher::Regex(regex) => match Program::compile_regex(regex) {
                Ok(program) => {
                    let id = self.intern_terminal(
                        TerminalIdentity::Regex(language, regex.as_str().to_owned()),
                        TerminalKind::Regex,
                        language,
                        program,
                    );
                    if !terminals.contains(&id) {
                        terminals.push(id);
                    }
                    ir.terminal_spec(TerminalKind::Regex, format!("terminal:{id}"));
                    ir.operation(SqlOperation::ScanTerminal);
                }
                Err(error) => self
                    .unsupported
                    .push(format!("{at}: regex terminal is not lowered: {error:?}")),
            },
            AstMatcher::Has(item, _) => {
                let operation = format!("{at}/has");
                self.unsupported
                    .push(format!("{operation}: descendant relation"));
                self.lower_matcher(
                    item.rule(),
                    language,
                    &format!("{operation}/rule"),
                    terminals,
                    ir,
                );
            }
            AstMatcher::Pattern(pattern) => match Program::compile_ast_grep_literal(pattern) {
                Ok(program) => {
                    let identity = TerminalIdentity::Pattern(
                        language,
                        program.canonical_digest(label(language).unwrap_or("unknown"), b""),
                    );
                    let id =
                        self.intern_terminal(identity, TerminalKind::Pattern, language, program);
                    if !terminals.contains(&id) {
                        terminals.push(id);
                    }
                    ir.terminal_spec(TerminalKind::Pattern, format!("terminal:{id}"));
                    ir.operation(SqlOperation::ScanTerminal);
                    ir.operation(SqlOperation::ScanCapture);
                }
                Err(error) => self
                    .unsupported
                    .push(format!("{at}: pattern terminal is not lowered: {error:?}")),
            },
            AstMatcher::Any(items) => {
                let operation = format!("{at}/any");
                let mut parts = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    let mut part = RelationalIr::empty();
                    self.lower_matcher(
                        item,
                        language,
                        &format!("{operation}/{index}"),
                        terminals,
                        &mut part,
                    );
                    parts.push(part);
                }
                match RelationalIr::combine(SqlOperation::OrderedUnion, parts) {
                    Ok(mut combined) => {
                        combined.operation(SqlOperation::WindowRank);
                        combined.operation(SqlOperation::CaseTruth);
                        *ir = combined;
                    }
                    Err(error) => self.unsupported.push(format!("{operation}: {error}")),
                }
            }
            AstMatcher::Not(item) => {
                let operation = format!("{at}/not");
                let universe_id = self.universe_terminal(language);
                if !terminals.contains(&universe_id) {
                    terminals.push(universe_id);
                }
                let mut universe = RelationalIr::empty();
                universe.terminal_spec(TerminalKind::Universe, format!("terminal:{universe_id}"));
                universe.operation(SqlOperation::ScanTerminal);
                let mut excluded = RelationalIr::empty();
                self.lower_matcher(
                    item,
                    language,
                    &format!("{operation}/rule"),
                    terminals,
                    &mut excluded,
                );
                match RelationalIr::combine(SqlOperation::AntiJoin, vec![universe, excluded]) {
                    Ok(mut combined) => {
                        combined.operation(SqlOperation::CaseTruth);
                        *ir = combined;
                    }
                    Err(error) => self.unsupported.push(format!("{operation}: {error}")),
                }
            }
            AstMatcher::Inside(item, _) => {
                let operation = format!("{at}/inside");
                self.unsupported
                    .push(format!("{operation}: ancestor relation"));
                self.lower_matcher(
                    item.rule(),
                    language,
                    &format!("{operation}/rule"),
                    terminals,
                    ir,
                );
            }
            AstMatcher::Precedes(item, _) => {
                let operation = format!("{at}/precedes");
                self.unsupported
                    .push(format!("{operation}: following-sibling relation"));
                self.lower_matcher(
                    item.rule(),
                    language,
                    &format!("{operation}/rule"),
                    terminals,
                    ir,
                );
            }
            AstMatcher::Follows(item, _) => {
                let operation = format!("{at}/follows");
                self.unsupported
                    .push(format!("{operation}: preceding-sibling relation"));
                self.lower_matcher(
                    item.rule(),
                    language,
                    &format!("{operation}/rule"),
                    terminals,
                    ir,
                );
            }
            AstMatcher::Field(relation) => {
                let operation = format!("{at}/field");
                self.unsupported
                    .push(format!("{operation}: field relation"));
                self.lower_matcher(
                    &relation.rule,
                    language,
                    &format!("{operation}/rule"),
                    terminals,
                    ir,
                );
            }
            AstMatcher::Constrained(core) => {
                let operation = format!("{at}/constraints");
                self.unsupported
                    .push(format!("{operation}: capture constraints"));
                self.lower_matcher(
                    &core.rule,
                    language,
                    &format!("{operation}/rule"),
                    terminals,
                    ir,
                );
                for (name, constraint) in &core.constraints {
                    self.lower_matcher(
                        constraint,
                        language,
                        &format!("{operation}/{name}"),
                        terminals,
                        ir,
                    );
                }
            }
            AstMatcher::Never => self
                .unsupported
                .push(format!("{at}: never matcher is not lowered")),
            AstMatcher::NthChild(_)
            | AstMatcher::Util(_)
            | AstMatcher::Call(_)
            | AstMatcher::Compare(_)
            | AstMatcher::Count(_)
            | AstMatcher::Semantic(_) => self
                .unsupported
                .push(format!("{at}: matcher operation is not lowered")),
            #[cfg(test)]
            AstMatcher::TestUnknown(_) => self
                .unsupported
                .push(format!("{at}: test-only unknown operation")),
        }
    }

    fn universe_terminal(&mut self, language: GrammarDialect) -> usize {
        self.intern_terminal(
            TerminalIdentity::Universe(language),
            TerminalKind::Universe,
            language,
            Program::compile_universe().expect("the universe terminal is a verified program"),
        )
    }

    fn kind_terminal(&mut self, language: GrammarDialect, kind: u16) -> usize {
        self.intern_terminal(
            TerminalIdentity::Kind(language, kind),
            TerminalKind::Kind,
            language,
            Program::compile_kind(kind).expect("a kind terminal is a verified program"),
        )
    }

    fn intern_terminal(
        &mut self,
        identity: TerminalIdentity,
        kind: TerminalKind,
        language: GrammarDialect,
        program: Program,
    ) -> usize {
        if let Some(id) = self.terminal_ids.get(&identity).copied() {
            let terminal = &mut self.terminals[id];
            if !terminal.clauses.contains(&self.current_clause) {
                terminal.clauses.push(self.current_clause);
            }
            terminal.result_limit = terminal.result_limit.max(self.current_result_limit);
            return id;
        }
        let id = self.terminals.len();
        self.terminals.push(TerminalPlan {
            kind,
            language,
            program,
            clauses: vec![self.current_clause],
            result_limit: self.current_result_limit,
        });
        self.terminal_ids.insert(identity, id);
        id
    }

    fn finish(self) -> Result<SqlCatalogPlan, LoweringError> {
        let mut clause_irs = self.clause_irs;
        let mut clauses = String::new();
        for (ordinal, ir) in clause_irs.iter_mut().enumerate() {
            ir.operation(SqlOperation::Projection);
            ir.operation(SqlOperation::OrderBy);
            let (relations, result) =
                ir.render_clause_relations(ordinal)
                    .map_err(|error| LoweringError {
                        unsupported: vec![format!("clause {ordinal}: {error}")],
                    })?;
            let projection = qualified_row_columns("result");
            writeln!(
                clauses,
                ",\n{relations},\nclause_{ordinal} AS (\n  SELECT {projection} FROM {result} result\n  CROSS JOIN batch_status batch\n  WHERE batch.complete = 1\n    AND NOT EXISTS (\n      SELECT 1 FROM producer_status undecided\n      WHERE undecided.complete <> 1 OR undecided.verdict IS NULL\n    )\n)"
            )
            .expect("write SQL");
        }
        let sql = format!(
            "WITH packed AS MATERIALIZED (\n  SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code FROM rerail_matches WHERE batch = :batch\n),\nbatch_status AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 1\n),\nproducer_status AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 2\n),\nterminal_match AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 3\n),\ncapture AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 4\n),\nnavigation AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 5\n),\nfact AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 6\n),\ndocument_map AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 7\n)\n{clauses}\nSELECT * FROM ({})\nORDER BY file_index, producer_id, match_index, capture_index",
            (0..clause_irs.len())
                .map(|ordinal| format!("SELECT * FROM clause_{ordinal}"))
                .collect::<Vec<_>>()
                .join(" UNION ALL ")
        );
        Ok(SqlCatalogPlan {
            terminals: self.terminals,
            clause_irs,
            sql,
            #[cfg(test)]
            clause_terminal_ids: self.clause_terminal_ids,
        })
    }
}
