use crate::code_rule_dsl::language::{GrammarDialect, label};
use crate::code_rule_dsl::matcher::AstMatcher;
use crate::code_rule_dsl::model::{ClauseKind, CompiledCodeRule, Query};
use crate::code_rule_dsl::packed::Program;
use ast_grep_core::matcher::Matcher;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum TerminalKind {
    Kind,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SqlShape {
    PackedMaterialized,
    StatusJoin,
    ScanTerminal,
    Projection,
    OrderBy,
}

pub(super) struct TerminalPlan {
    #[cfg(test)]
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
    #[cfg(test)]
    shape: Vec<SqlShape>,
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
        &self.shape
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

#[derive(Default)]
struct Builder {
    terminals: Vec<TerminalPlan>,
    terminal_ids: HashMap<(GrammarDialect, u16), usize>,
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
            AstMatcher::Regex(_) => self.unsupported.push(format!("{at}: regex terminal")),
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
            AstMatcher::Pattern(_) => self.unsupported.push(format!("{at}: pattern terminal")),
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
        if let Some(id) = self.terminal_ids.get(&(language, kind)).copied() {
            let terminal = &mut self.terminals[id];
            if !terminal.clauses.contains(&self.current_clause) {
                terminal.clauses.push(self.current_clause);
            }
            terminal.result_limit = terminal.result_limit.max(self.current_result_limit);
            return id;
        }
        let id = self.terminals.len();
        self.terminals.push(TerminalPlan {
            #[cfg(test)]
            kind: TerminalKind::Kind,
            language,
            program: Program::compile_kind(kind).expect("a kind terminal is a verified program"),
            clauses: vec![self.current_clause],
            result_limit: self.current_result_limit,
        });
        self.terminal_ids.insert((language, kind), id);
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
        SqlCatalogPlan {
            terminals: self.terminals,
            #[cfg(test)]
            shape: vec![
                SqlShape::PackedMaterialized,
                SqlShape::StatusJoin,
                SqlShape::ScanTerminal,
                SqlShape::Projection,
                SqlShape::OrderBy,
            ],
            sql,
            #[cfg(test)]
            clause_terminal_ids: self.clause_terminal_ids,
        }
    }
}
