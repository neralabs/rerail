use super::relational_ir::{RelationalIr, SqlOperation};
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
    ir: RelationalIr,
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
                    builder.lower_matcher(&matcher.matcher, matcher.language, &at, &mut terminals);
                    builder.clause_terminal_ids.push(terminals);
                }
            }
        }
        if !builder.unsupported.is_empty() {
            return Err(LoweringError {
                unsupported: builder.unsupported,
            });
        }
        Ok(builder.finish())
    }

    pub(super) fn terminals(&self) -> &[TerminalPlan] {
        &self.terminals
    }

    #[cfg(test)]
    pub(super) fn shape(&self) -> &[SqlShape] {
        self.ir.operations()
    }

    pub(super) fn verify(&self) -> Result<(), String> {
        self.ir.validate()?;
        if self
            .terminals
            .iter()
            .any(|terminal| !self.ir.terminals().contains(&terminal.kind))
        {
            return Err("SQL catalog terminal is absent from relational IR producers".to_owned());
        }
        let normalized = self.ir.normalized_sql();
        for prohibited in ["tq_", "rerail_nodes", "rerail_facts", "GrammarMatcher"] {
            if normalized.contains(prohibited) {
                return Err(format!(
                    "relational IR contains prohibited SQL `{prohibited}`"
                ));
            }
        }
        if self.sql.trim().is_empty() {
            return Err("SQL catalog plan has no statement".to_owned());
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
    Kind(GrammarDialect, u16),
    Pattern(GrammarDialect, String),
    Regex(GrammarDialect, String),
}

#[derive(Default)]
struct Builder {
    terminals: Vec<TerminalPlan>,
    terminal_ids: HashMap<TerminalIdentity, usize>,
    ir: RelationalIr,
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
                    self.ir.terminal(TerminalKind::Kind);
                    self.ir.operation(SqlOperation::ScanTerminal);
                }
            }
            AstMatcher::All(items) => {
                let operation = format!("{at}/all");
                self.unsupported
                    .push(format!("{operation}: ordered conjunction"));
                for (index, item) in items.iter().enumerate() {
                    self.lower_matcher(item, language, &format!("{operation}/{index}"), terminals);
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
                    self.ir.terminal(TerminalKind::Regex);
                    self.ir.operation(SqlOperation::ScanTerminal);
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
                    self.ir.terminal(TerminalKind::Pattern);
                    self.ir.operation(SqlOperation::ScanTerminal);
                    self.ir.operation(SqlOperation::ScanCapture);
                }
                Err(error) => self
                    .unsupported
                    .push(format!("{at}: pattern terminal is not lowered: {error:?}")),
            },
            AstMatcher::Any(items) => {
                let operation = format!("{at}/any");
                self.unsupported
                    .push(format!("{operation}: ordered alternatives"));
                for (index, item) in items.iter().enumerate() {
                    self.lower_matcher(item, language, &format!("{operation}/{index}"), terminals);
                }
            }
            AstMatcher::Not(item) => {
                let operation = format!("{at}/not");
                self.unsupported.push(format!("{operation}: negation"));
                self.lower_matcher(item, language, &format!("{operation}/rule"), terminals);
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
                );
                for (name, constraint) in &core.constraints {
                    self.lower_matcher(
                        constraint,
                        language,
                        &format!("{operation}/{name}"),
                        terminals,
                    );
                }
            }
            AstMatcher::Never => {}
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

    fn finish(self) -> SqlCatalogPlan {
        let mut clauses = String::new();
        for (ordinal, terminals) in self.clause_terminal_ids.iter().enumerate() {
            let terminal = terminals
                .first()
                .expect("a successfully lowered clause has a terminal");
            writeln!(
                clauses,
                ", clause_{ordinal} AS (\n  SELECT m.* FROM terminal_match m\n  INNER JOIN producer_status s USING (producer_id, file_index)\n  WHERE m.producer_id = {terminal} AND s.complete = 1\n)"
            )
            .expect("write SQL");
        }
        let sql = format!(
            "WITH packed AS MATERIALIZED (\n  SELECT * FROM rerail_matches WHERE batch = :batch\n),\nproducer_status AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 1\n),\nterminal_match AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 2\n)\n{clauses}\nSELECT * FROM ({})\nORDER BY file_index, producer_id, match_index, capture_index",
            (0..self.clause_terminal_ids.len())
                .map(|ordinal| format!("SELECT * FROM clause_{ordinal}"))
                .collect::<Vec<_>>()
                .join(" UNION ALL ")
        );
        let mut ir = self.ir;
        ir.operation(SqlOperation::Projection);
        ir.operation(SqlOperation::OrderBy);
        SqlCatalogPlan {
            terminals: self.terminals,
            ir,
            sql,
            #[cfg(test)]
            clause_terminal_ids: self.clause_terminal_ids,
        }
    }
}
