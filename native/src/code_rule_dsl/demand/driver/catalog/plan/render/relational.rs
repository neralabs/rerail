use super::plan::{UtilityDefinitionId, utility_definition_id};
use super::relational_ir::{
    ComparisonSpec, CountSpec, NavigationAxis, NavigationSpec, NavigationStop, NthChildSpec,
    ParseFactSpec, RecursiveUtilitySpec, RecursiveUtilityTransition, RelationalIr,
    SemanticFactSpec, SemanticRuleSpec, SharedRelationalIr, SqlMaterialization, SqlOperation,
    TerminalSpec, UtilityCatalog, UtilityFrameSpec, UtilitySpec, ValueLiteral, ValuePredicateSpec,
    qualified_failure_columns, qualified_row_columns,
};
use crate::code_rule_dsl::language::{GrammarDialect, label};
use crate::code_rule_dsl::matcher::{
    AstMatcher, MAX_UTIL_DEPTH, Semantic, StopBy, StringCase, Transform, UtilCall, UtilRef,
};
use crate::code_rule_dsl::model::{
    Anchor, ClauseKind, CompiledClause, CompiledCodeRule, FactQuery, ProjectionSource, Query,
    RegexOptions, RegexSource, RuleMode, TemplatePart, ViolationSpec,
};
use crate::code_rule_dsl::transducer::packed::Program;
use crate::code_rule_dsl::demand::driver::catalog::sql_backend::{
    FileFactDescriptor, ParseFactDescriptor, ProducerKind, SemanticFactDescriptor,
    SemanticFactSource, TransformFactDescriptor, ValueFactDescriptor,
};
use crate::code_rule_dsl::value::Value;
use ast_grep_core::matcher::Matcher;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

#[cfg(test)]
pub(super) use super::relational_ir::SqlOperation as SqlShape;
pub(super) use super::relational_ir::TerminalKind;

#[derive(Clone)]
struct FileApplicability {
    condition: Option<Arc<crate::code_rule_dsl::language::FileCondition>>,
    required: Option<Arc<crate::code_rule_dsl::matcher::RequiredText>>,
}

pub(super) struct TerminalPlan {
    kind: TerminalKind,
    language: GrammarDialect,
    program: Program,
    clauses: Vec<(usize, usize)>,
    result_limit: u32,
    value_fact: Option<ValueFactDescriptor>,
    semantic_fact: Option<SemanticFactDescriptor>,
    parse_fact: Option<ParseFactDescriptor>,
    transform_fact: Option<TransformFactDescriptor>,
    file_fact: Option<FileFactDescriptor>,
    sound_under_errors: bool,
    file_conditions: Vec<FileApplicability>,
}

impl TerminalPlan {
    pub(super) const fn kind(&self) -> TerminalKind {
        self.kind
    }

    #[cfg(test)]
    pub(super) fn language(&self) -> &'static str {
        label(self.language).expect("a compiled matcher has a registered dialect")
    }

    #[cfg(test)]
    pub(super) fn program_digest(&self) -> String {
        self.program.canonical_digest(self.language(), b"")
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

    pub(super) const fn producer_kind(&self) -> ProducerKind {
        match self.kind {
            TerminalKind::Navigation => ProducerKind::Navigation,
            TerminalKind::Pattern => ProducerKind::Pattern,
            TerminalKind::Value => ProducerKind::ValueFact,
            TerminalKind::Fact => ProducerKind::SemanticFact,
            TerminalKind::Parse => ProducerKind::ParseFact,
            TerminalKind::Transform => ProducerKind::TransformFact,
            TerminalKind::File => ProducerKind::FileFact,
            _ => ProducerKind::Terminal,
        }
    }

    pub(super) fn value_fact(&self) -> Option<ValueFactDescriptor> {
        self.value_fact.clone()
    }

    pub(super) fn semantic_fact(&self) -> Option<SemanticFactDescriptor> {
        self.semantic_fact.clone()
    }

    pub(super) fn parse_fact(&self) -> Option<ParseFactDescriptor> {
        self.parse_fact.clone()
    }

    pub(super) fn transform_fact(&self) -> Option<TransformFactDescriptor> {
        self.transform_fact.clone()
    }

    pub(super) fn file_fact(&self) -> Option<FileFactDescriptor> {
        self.file_fact.clone()
    }

    pub(super) const fn sound_under_errors(&self) -> bool {
        self.sound_under_errors
    }

    pub(super) fn applies_to_file(
        &self,
        path: &str,
        source: &str,
        module: Option<&crate::code_rule_dsl::module_path::ModulePath>,
    ) -> bool {
        self.file_conditions.is_empty()
            || self.file_conditions.iter().any(|applicability| {
                applicability
                    .condition
                    .as_ref()
                    .is_none_or(|condition| condition.holds(path, module, None))
                    && applicability.required.as_ref().is_none_or(|required| {
                        required.holds_with(source, &mut |text, literal| text.contains(literal))
                    })
            })
    }
}

pub(super) struct ExecutionSqlPlan {
    pub(super) materializations: Vec<(String, String)>,
    pub(super) query: String,
}

pub(super) struct SqlCatalogPlan {
    terminals: Vec<TerminalPlan>,
    clause_irs: Vec<RelationalIr>,
    clause_terminal_ids: Vec<Vec<usize>>,
    clause_each: Vec<Option<String>>,
    clause_outputs: Vec<OutputPlan>,
    clause_logical_plans: Vec<super::plan::ClauseLogicalPlan>,
    utility_call_counts: BTreeMap<UtilityDefinitionId, usize>,
    #[cfg(test)]
    authored_fixture_sql: Option<String>,
    #[cfg(test)]
    all_execution_sql: std::sync::OnceLock<String>,
    #[cfg(test)]
    all_relational_sql: std::sync::OnceLock<String>,
}

impl std::fmt::Debug for SqlCatalogPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SqlCatalogPlan")
            .field("terminals", &self.terminals.len())
            .field("clauses", &self.clause_irs.len())
            .finish()
    }
}

impl SqlCatalogPlan {
    pub(super) fn compile(code_rules: &[Arc<CompiledCodeRule>]) -> Result<Self, LoweringError> {
        let mut builder = Builder::default();
        for (rule_index, code_rule) in code_rules.iter().enumerate() {
            for (clause_index, clause) in code_rule.clauses.iter().enumerate() {
                let clause_at = format!("{}/{}", code_rule.id, clause.id);
                builder.current_clause = (rule_index, clause_index);
                builder
                    .clause_source_starts
                    .push((builder.clause_outputs.len(), builder.current_clause));
                builder.current_each = clause.violation.each.clone();
                builder.current_result_limit =
                    u32::try_from(code_rule.capabilities.limits.max_witnesses).unwrap_or(u32::MAX);
                builder.current_max_work = code_rule.capabilities.limits.max_visited_nodes;
                if let ClauseKind::Forbid(Query::SelectWhen { .. }) = &clause.kind {
                    let (terminals, ir) = builder.lower_clause_query(
                        match &clause.kind {
                            ClauseKind::Forbid(query) => query,
                            _ => unreachable!(),
                        },
                        &clause_at,
                    );
                    builder.clause_terminal_ids.push(terminals);
                    builder.clause_each.push(builder.current_each.clone());
                    builder.clause_anchors.push(clause.violation.anchor.clone());
                    builder
                        .clause_outputs
                        .push(OutputPlan::new(code_rule, clause, None));
                    builder.clause_irs.push(ir);
                    continue;
                }
                if let ClauseKind::RequireEach { subject, evidence } = &clause.kind {
                    let (mut terminals, subject_ir) = builder
                        .lower_clause_query(&subject.ast.query, &format!("{clause_at}/subject"));
                    let (evidence_terminals, evidence_ir) = builder
                        .lower_clause_query(&evidence.ast.query, &format!("{clause_at}/evidence"));
                    terminals.extend(evidence_terminals);
                    terminals.sort_unstable();
                    terminals.dedup();
                    builder.refuse_projections(&[subject, evidence], &clause_at);
                    let keys = subject
                        .project
                        .iter()
                        .filter_map(|left| {
                            evidence
                                .project
                                .iter()
                                .find(|right| right.field == left.field)
                                .map(|right| {
                                    (
                                        Builder::projection_sql(&left.source, "subject"),
                                        Builder::projection_sql(&right.source, "evidence"),
                                    )
                                })
                        })
                        .collect();
                    let ir = RelationalIr::require_each(subject_ir, evidence_ir, keys)
                        .unwrap_or_else(|error| {
                            builder.unsupported.push(format!("{clause_at}: {error}"));
                            RelationalIr::empty_relation()
                        });
                    builder.clause_terminal_ids.push(terminals);
                    builder.clause_each.push(builder.current_each.clone());
                    builder.clause_anchors.push(clause.violation.anchor.clone());
                    builder
                        .clause_outputs
                        .push(OutputPlan::new(code_rule, clause, None));
                    builder.clause_irs.push(ir);
                    continue;
                }
                if let ClauseKind::Budget {
                    query,
                    group_by,
                    maximum,
                    distinct,
                } = &clause.kind
                {
                    builder.refuse_projections(&[query], &clause_at);
                    let (terminals, source) =
                        builder.lower_clause_query(&query.ast.query, &format!("{clause_at}/query"));
                    let fields = |names: &[String]| {
                        names
                            .iter()
                            .filter_map(|name| {
                                name.split_once('.')
                                    .and_then(|(_, field)| {
                                        query.project.iter().find(|item| item.field == field)
                                    })
                                    .map(|item| Builder::projection_sql(&item.source, "usage"))
                            })
                            .collect()
                    };
                    let ir =
                        RelationalIr::budget(source, fields(group_by), fields(distinct), *maximum)
                            .unwrap_or_else(|error| {
                                builder.unsupported.push(format!("{clause_at}: {error}"));
                                RelationalIr::empty_relation()
                            });
                    builder.clause_terminal_ids.push(terminals);
                    builder.clause_each.push(builder.current_each.clone());
                    builder.clause_anchors.push(clause.violation.anchor.clone());
                    builder
                        .clause_outputs
                        .push(OutputPlan::new(code_rule, clause, None));
                    builder.clause_irs.push(ir);
                    continue;
                }
                if let ClauseKind::Forbid(Query::Regex {
                    regex,
                    source,
                    options,
                }) = &clause.kind
                {
                    builder.refuse_regex_options(options, &clause_at);
                    builder.current_clause = (rule_index, clause_index);
                    builder.current_result_limit =
                        u32::try_from(code_rule.capabilities.limits.max_witnesses)
                            .unwrap_or(u32::MAX);
                    builder.current_max_work = code_rule.capabilities.limits.max_visited_nodes;
                    let id = builder.file_terminal(regex, *source);
                    let mut ir = RelationalIr::empty();
                    ir.terminal_spec(TerminalKind::File, format!("terminal:{id}"));
                    ir.operation(SqlOperation::ScanTerminal);
                    builder.clause_terminal_ids.push(vec![id]);
                    builder.clause_each.push(builder.current_each.clone());
                    builder.clause_anchors.push(clause.violation.anchor.clone());
                    builder
                        .clause_outputs
                        .push(OutputPlan::new(code_rule, clause, None));
                    builder.clause_irs.push(ir);
                    continue;
                }
                let ClauseKind::Forbid(Query::Ast { matchers, calls }) = &clause.kind else {
                    builder
                        .unsupported
                        .push(format!("{clause_at}: clause relation is not lowered"));
                    continue;
                };
                let _ = calls;
                for matcher in matchers {
                    let language = label(matcher.language).unwrap_or("unknown");
                    let at = format!("{clause_at}/{language}/rule");
                    let mut terminals = Vec::new();
                    builder.current_calls = calls
                        .iter()
                        .map(|term| (Arc::clone(&term.key), Arc::clone(term)))
                        .collect();
                    builder.current_result_limit =
                        u32::try_from(code_rule.capabilities.limits.max_witnesses)
                            .unwrap_or(u32::MAX);
                    builder.current_max_work = code_rule.capabilities.limits.max_visited_nodes;
                    let mut ir = RelationalIr::empty();
                    if matcher.applies {
                        builder.lower_matcher(
                            &matcher.matcher,
                            matcher.language,
                            &at,
                            &mut terminals,
                            &mut ir,
                        );
                    } else {
                        ir = RelationalIr::empty_relation();
                    }
                    for producer in &terminals {
                        if builder.terminals[*producer].kind == TerminalKind::Universe {
                            builder.terminals[*producer].sound_under_errors = false;
                        }
                        builder.terminals[*producer]
                            .file_conditions
                            .push(FileApplicability {
                                condition: matcher.file.clone(),
                                required: matcher.text.clone(),
                            });
                    }
                    builder.clause_terminal_ids.push(terminals);
                    builder.clause_each.push(builder.current_each.clone());
                    builder.clause_anchors.push(clause.violation.anchor.clone());
                    builder.clause_outputs.push(OutputPlan::new(
                        code_rule,
                        clause,
                        matcher.transform.clone(),
                    ));
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

    pub(super) fn requires_document_maps(&self) -> bool {
        self.clause_irs
            .iter()
            .any(|ir| ir.operations().contains(&SqlOperation::MapDocument))
    }

    pub(super) fn terminals(&self) -> &[TerminalPlan] {
        &self.terminals
    }

    fn render_clauses_plan(
        &self,
        ordinals: &[usize],
        outputs: bool,
        packetize: bool,
    ) -> Result<ExecutionSqlPlan, String> {
        if ordinals.is_empty() {
            return Err("an exact-scope statement has no clauses".to_owned());
        }
        if ordinals.windows(2).any(|pair| pair[0] >= pair[1])
            || ordinals
                .iter()
                .any(|ordinal| *ordinal >= self.clause_irs.len())
        {
            return Err("exact-scope clause ordinals are invalid or unordered".to_owned());
        }
        let recursive = self.utility_call_counts.len() > 8
            || ordinals
                .iter()
                .any(|ordinal| self.clause_irs[*ordinal].requires_recursive_cte());
        let mut ctes = Vec::new();
        let mut materializations = Vec::new();
        let mut utilities = UtilityCatalog::new(self.utility_call_counts.clone());
        for &ordinal in ordinals {
            let ir = &self.clause_irs[ordinal];
            let (relations, result, clause_materializations) = ir.render_clause_relations(
                ordinal,
                self.clause_each[ordinal].as_deref(),
                &mut utilities,
                packetize,
            )?;
            materializations.extend(clause_materializations);
            let projection = qualified_row_columns("result");
            let failure_projection = qualified_failure_columns("failed");
            let producer_list = self.clause_terminal_ids[ordinal]
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let failure_union = if producer_list.is_empty() {
                String::new()
            } else {
                format!(
                    "\n  UNION ALL\n  SELECT {failure_projection}, json('[]') AS bindings, '' AS choice_order, {ordinal} AS clause_ordinal\n  FROM producer_status failed\n  WHERE failed.producer_id IN ({producer_list}) AND failed.complete <> 1\n    AND failed.producer_id = (SELECT min(same.producer_id) FROM producer_status same WHERE same.file_index = failed.file_index AND same.error_code = failed.error_code AND same.complete <> 1 AND same.producer_id IN ({producer_list}))"
                )
            };
            let dependency_filter = if producer_list.is_empty() {
                "0".to_owned()
            } else {
                format!(
                    "undecided.producer_id IN ({producer_list}) AND (undecided.complete <> 1 OR undecided.verdict IS NULL)"
                )
            };
            ctes.extend(relations);
            ctes.push(format!(
                "clause_{ordinal} AS (\n  SELECT {projection}, result.bindings, result.choice_order, {ordinal} AS clause_ordinal FROM {result} result\n  CROSS JOIN batch_status batch\n  WHERE batch.complete = 1\n    AND NOT EXISTS (\n      SELECT 1 FROM producer_status undecided\n      WHERE {dependency_filter}\n    ){failure_union}\n)"
            ));
        }
        let (utility_ctes, utility_materializations) =
            RelationalIr::render_utility_catalog(&mut utilities, packetize)?;
        let utility_materialization_tables = utility_materializations
            .iter()
            .map(|materialization| materialization.table.clone())
            .collect::<BTreeSet<_>>();
        materializations.splice(0..0, utility_materializations);
        ctes.splice(0..0, utility_ctes.iter().cloned());
        if outputs {
            for &ordinal in ordinals {
                ctes.push(Builder::output_cte(ordinal, &self.clause_outputs[ordinal])?);
            }
        }

        let (merge_ctes, clause_union) =
            super::plan::ClauseMergeAst::new(ordinals, outputs)?.render()?;
        let merge_ctes = if merge_ctes.is_empty() {
            String::new()
        } else {
            format!(",\n{merge_ctes}")
        };
        let with = if recursive { "WITH RECURSIVE" } else { "WITH" };
        let clauses = ctes.join(",\n");
        let demanded_producers = ordinals
            .iter()
            .flat_map(|ordinal| self.clause_terminal_ids[*ordinal].iter().copied())
            .collect::<BTreeSet<_>>();
        let statement_relations = super::plan::StatementRelations::from_fact_families(
            demanded_producers.iter().filter_map(|producer| {
                match self.terminals[*producer].kind() {
                    TerminalKind::Fact | TerminalKind::CallSeed => {
                        Some(super::plan::FactFamilyContract::Semantic)
                    }
                    TerminalKind::Value => Some(super::plan::FactFamilyContract::Value),
                    TerminalKind::Parse => Some(super::plan::FactFamilyContract::Parse),
                    TerminalKind::Transform => Some(super::plan::FactFamilyContract::Transform),
                    _ => None,
                }
            }),
        )
        .with_utility_definitions(ordinals.iter().flat_map(|ordinal| {
            self.clause_logical_plans[*ordinal]
                .utility_definitions()
                .iter()
                .copied()
        }));
        let typed_relations = super::plan::TypedBoundaryAst::new(
            &statement_relations,
            demanded_producers
                .iter()
                .map(|producer| u64::try_from(*producer).expect("producer ordinal fits u64")),
        )
        .render();
        let file_metadata = "file_metadata AS MATERIALIZED (\n  SELECT CAST(key AS INTEGER) AS file_index, json_extract(value, '$.path') AS path, json_extract(value, '$.source') AS source, json_extract(value, '$.lineStarts') AS line_starts FROM json_each(:files)\n)";
        let query = if outputs {
            format!(
                "{with} {typed_relations},\n{file_metadata}\n,\n{clauses}{merge_ctes}\n{clause_union}"
            )
        } else {
            format!("{with} {typed_relations},\n{clauses}{merge_ctes}\n{clause_union}")
        };
        let materializations = materializations
            .into_iter()
            .map(|materialization: SqlMaterialization| {
                let mut packet_ctes = if utility_materialization_tables
                    .contains(&materialization.table)
                {
                    Vec::new()
                } else {
                    utility_ctes.clone()
                };
                packet_ctes.extend(materialization.ctes);
                let sql = format!(
                    "CREATE TEMP TABLE \"{}\" AS WITH RECURSIVE {typed_relations},\n{file_metadata},\n{}\nSELECT * FROM {}",
                    materialization.table,
                    packet_ctes.join(",\n"),
                    materialization.root,
                );
                (materialization.table, sql)
            })
            .collect();
        Ok(ExecutionSqlPlan {
            materializations,
            query,
        })
    }

    fn render_clauses(&self, ordinals: &[usize], outputs: bool) -> Result<String, String> {
        self.render_clauses_plan(ordinals, outputs, false)
            .map(|plan| plan.query)
    }

    pub(super) fn execution_sql_for_clauses(
        &self,
        ordinals: &[usize],
    ) -> Result<ExecutionSqlPlan, String> {
        self.render_clauses_plan(ordinals, true, true)
    }

    #[cfg(test)]
    pub(super) fn shape(&self) -> &[SqlShape] {
        self.clause_irs
            .first()
            .expect("a test SQL plan has one clause")
            .operations()
    }

    pub(super) fn verify(&self) -> Result<(), String> {
        if self.terminals.iter().any(|terminal| {
            terminal
                .semantic_fact
                .as_ref()
                .is_some_and(|descriptor| descriptor.source.has_nested_user_rule())
        }) {
            return Err("semantic fact descriptor contains a nested user rule".to_owned());
        }
        for ir in &self.clause_irs {
            ir.validate()?;
        }
        let relational_terminals = self
            .clause_irs
            .iter()
            .flat_map(RelationalIr::terminal_specs)
            .collect::<BTreeSet<_>>();
        if self.terminals.iter().enumerate().any(|(id, terminal)| {
            !relational_terminals
                .contains(&TerminalSpec::new(terminal.kind, format!("terminal:{id}")))
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
        for ordinal in 0..self.clause_irs.len() {
            let sql = self.render_clauses(&[ordinal], true)?;
            if sql.trim().is_empty() {
                return Err(format!("SQL clause {ordinal} has no statement"));
            }
            for prohibited in ["tq_", "rerail_nodes", "rerail_facts", "GrammarMatcher"] {
                if sql.contains(prohibited) {
                    return Err(format!(
                        "generated SQL clause {ordinal} contains prohibited `{prohibited}`"
                    ));
                }
            }
            super::plan::TypedBoundaryAst::verify_rendered_sources(&sql)
                .map_err(|error| format!("generated SQL clause {ordinal}: {error}"))?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn structural_sql(&self) -> &str {
        self.all_relational_sql.get_or_init(|| {
            let ordinals = (0..self.clause_irs.len()).collect::<Vec<_>>();
            self.render_clauses(&ordinals, false)
                .expect("verified relational SQL renders")
        })
    }

    #[cfg(test)]
    pub(super) fn execution_sql(&self) -> &str {
        self.all_execution_sql.get_or_init(|| {
            if let Some(authored) = &self.authored_fixture_sql {
                return authored.clone();
            }
            let ordinals = (0..self.clause_irs.len()).collect::<Vec<_>>();
            self.render_clauses(&ordinals, true)
                .expect("verified execution SQL renders")
        })
    }

    pub(super) fn clause_count(&self) -> usize {
        self.clause_terminal_ids.len()
    }

    pub(super) fn clause_output_key(&self, ordinal: usize) -> Option<(&str, &str)> {
        self.clause_outputs
            .get(ordinal)
            .map(|output| (output.rule_id.as_str(), output.clause_id.as_str()))
    }

    pub(super) fn clause_logical_plans(&self) -> &[super::plan::ClauseLogicalPlan] {
        &self.clause_logical_plans
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
    Navigation(GrammarDialect),
    Kind(GrammarDialect, u16),
    Pattern(GrammarDialect, String),
    Regex(GrammarDialect, String),
    Value(GrammarDialect, bool, Vec<u16>, String),
    Fact(GrammarDialect, String),
    Parse(GrammarDialect, String),
    Transform(GrammarDialect, String),
    File(RegexSource, String),
}

type UtilityKey = UtilityDefinitionId;
type UtilityGraph = BTreeMap<UtilityKey, (UtilRef, AstMatcher, Vec<UtilityKey>)>;

fn utility_plan_key(util: &UtilRef) -> Option<UtilityKey> {
    util.semantic_key()
        .map(|digest| utility_definition_id(&digest))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecursiveStep {
    axis: NavigationAxis,
    stop: NavigationStop,
    field: Option<u16>,
    field_stop_children_only: bool,
}

#[derive(Clone)]
struct RecursiveTransitionTemplate {
    target: UtilityKey,
    arguments: Vec<(String, String)>,
    steps: Vec<RecursiveStep>,
    stops: Vec<(usize, AstMatcher)>,
    guards: Vec<(usize, AstMatcher)>,
}

#[derive(Eq, PartialEq)]
struct LoweredRecursiveTransition {
    caller: usize,
    target: usize,
    arguments: Vec<(String, String)>,
    steps: Vec<RecursiveStep>,
    stops: Vec<(usize, RelationalIr)>,
    guards: Vec<(usize, RelationalIr)>,
}

#[derive(Clone)]
struct OutputPlan {
    rule_id: String,
    clause_id: String,
    mode: RuleMode,
    violation: ViolationSpec,
    transform: Option<Arc<Transform>>,
}

fn clause_effect_profile(ir: &RelationalIr) -> u16 {
    let mut profile = 0;
    for (operation, bit) in [
        (SqlOperation::OrderedUnion, 1),
        (SqlOperation::WindowRank, 2),
        (SqlOperation::RecursiveCte, 4),
        (SqlOperation::GroupBy, 8),
        (SqlOperation::Having, 16),
        (SqlOperation::MapDocument, 32),
        (SqlOperation::CrossFileJoin, 64),
    ] {
        if ir.operations().contains(&operation) {
            profile |= bit;
        }
    }
    profile
}

impl OutputPlan {
    fn new(
        code_rule: &CompiledCodeRule,
        clause: &CompiledClause,
        transform: Option<Arc<Transform>>,
    ) -> Self {
        Self {
            rule_id: code_rule.id.clone(),
            clause_id: clause.id.clone(),
            mode: code_rule.rule_mode,
            violation: clause.violation.clone(),
            transform,
        }
    }
}

type UtilityBodyKey = (UtilityKey, GrammarDialect, Vec<UtilityKey>);
type UtilityBodyCache = BTreeMap<UtilityBodyKey, (SharedRelationalIr, Vec<usize>)>;

#[derive(Default)]
struct Builder {
    terminals: Vec<TerminalPlan>,
    terminal_ids: HashMap<TerminalIdentity, usize>,
    clause_irs: Vec<RelationalIr>,
    clause_terminal_ids: Vec<Vec<usize>>,
    clause_each: Vec<Option<String>>,
    clause_anchors: Vec<Anchor>,
    clause_outputs: Vec<OutputPlan>,
    clause_source_starts: Vec<(usize, (usize, usize))>,
    unsupported: Vec<String>,
    current_clause: (usize, usize),
    current_result_limit: u32,
    current_max_work: usize,
    current_each: Option<String>,
    utility_stack: Vec<UtilityKey>,
    utility_bodies: UtilityBodyCache,
    recursive_bodies: UtilityBodyCache,
    utility_call_counts: BTreeMap<UtilityDefinitionId, usize>,
    current_calls: HashMap<Arc<str>, Arc<crate::code_rule_dsl::model::CallsTerm>>,
}

fn utility_references(matcher: &AstMatcher, references: &mut Vec<UtilRef>) {
    let stop = |stop: &StopBy, references: &mut Vec<UtilRef>| {
        if let StopBy::Rule(rule) = stop {
            utility_references(rule, references);
        }
    };
    match matcher {
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            for item in items {
                utility_references(item, references);
            }
        }
        AstMatcher::Not(item) => utility_references(item, references),
        AstMatcher::Inside(item, by)
        | AstMatcher::Has(item, by)
        | AstMatcher::Precedes(item, by)
        | AstMatcher::Follows(item, by) => {
            utility_references(item.rule(), references);
            stop(by, references);
        }
        AstMatcher::Field(relation) => {
            utility_references(&relation.rule, references);
            stop(&relation.stop, references);
        }
        AstMatcher::NthChild(nth) => {
            if let Some(rule) = &nth.of_rule {
                utility_references(rule, references);
            }
        }
        AstMatcher::Constrained(core) => {
            utility_references(&core.rule, references);
            for (_, constraint) in &core.constraints {
                utility_references(constraint, references);
            }
        }
        AstMatcher::Count(count) => utility_references(&count.relation, references),
        AstMatcher::Util(util) => references.push(util.clone()),
        AstMatcher::Call(call) => references.push(call.util.clone()),
        AstMatcher::Pattern(_)
        | AstMatcher::Kind(_)
        | AstMatcher::Regex(_)
        | AstMatcher::Compare(_)
        | AstMatcher::Semantic(_)
        | AstMatcher::Never => {}
        #[cfg(test)]
        AstMatcher::TestUnknown(_) => {}
    }
}

impl Builder {
    fn sql_text(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    fn binding_start(row: &str) -> String {
        format!(
            "COALESCE(CAST(json_extract(binding.value, '$.start_byte') AS INTEGER), (SELECT min(captured.start_byte) FROM capture captured WHERE captured.file_index = {row}.file_index AND captured.node = CAST(json_extract(binding.value, '$.node') AS INTEGER)), (SELECT min(related.start_byte) FROM navigation related WHERE related.file_index = {row}.file_index AND related.related_node = CAST(json_extract(binding.value, '$.node') AS INTEGER)))"
        )
    }

    fn binding_end(row: &str) -> String {
        format!(
            "COALESCE(CAST(json_extract(binding.value, '$.end_byte') AS INTEGER), (SELECT max(captured.end_byte) FROM capture captured WHERE captured.file_index = {row}.file_index AND captured.node = CAST(json_extract(binding.value, '$.node') AS INTEGER)), (SELECT max(related.end_byte) FROM navigation related WHERE related.file_index = {row}.file_index AND related.related_node = CAST(json_extract(binding.value, '$.node') AS INTEGER)))"
        )
    }

    fn binding_text(row: &str, name: &str, multi: bool) -> String {
        let name = Self::sql_text(name);
        if multi {
            let start = Self::binding_start(row);
            let end = Self::binding_end(row);
            format!(
                "COALESCE(CAST(substr(CAST(file.source AS BLOB), (SELECT min({start}) FROM json_each({row}.bindings) binding WHERE json_extract(binding.value, '$.label') = {name}) + 1, (SELECT max({end}) FROM json_each({row}.bindings) binding WHERE json_extract(binding.value, '$.label') = {name}) - (SELECT min({start}) FROM json_each({row}.bindings) binding WHERE json_extract(binding.value, '$.label') = {name})) AS TEXT), '')"
            )
        } else {
            format!(
                "COALESCE((SELECT json_extract(binding.value, '$.text') FROM json_each({row}.bindings) binding WHERE json_extract(binding.value, '$.label') = {name} AND CAST(json_extract(binding.value, '$.occurrence') AS INTEGER) = 0), '')"
            )
        }
    }

    fn transformed_text(
        output: &OutputPlan,
        row: &str,
        name: &str,
        multi: bool,
    ) -> Result<String, String> {
        let Some(transform) = &output.transform else {
            return Ok(Self::binding_text(row, name, multi));
        };
        let Some((source, case)) = transform.simple_case(name) else {
            if transform.contains(name) {
                return Err(format!(
                    "output transform {name} is not a simple SQL case conversion"
                ));
            }
            return Ok(Self::binding_text(row, name, multi));
        };
        let source = Self::binding_text(row, source, false);
        match case {
            StringCase::LowerCase => Ok(format!("lower({source})")),
            StringCase::UpperCase => Ok(format!("upper({source})")),
            _ => Err(format!(
                "output transform {name} uses an unsupported SQL case conversion"
            )),
        }
    }

    fn template_expression(
        template: &crate::code_rule_dsl::model::Template,
        output: &OutputPlan,
        row: &str,
        message: bool,
    ) -> Result<String, String> {
        let parts = template
            .parts()
            .map(|part| match part {
                TemplatePart::Text(text) => Ok(Self::sql_text(text)),
                TemplatePart::Var { name, multi } => {
                    let text = Self::transformed_text(output, row, name, multi)?;
                    if message {
                        let first = format!(
                            "CASE WHEN instr({text}, char(10)) > 0 THEN substr({text}, 1, instr({text}, char(10)) - 1) ELSE {text} END"
                        );
                        Ok(format!(
                            "CASE WHEN instr({text}, char(10)) > 0 OR length({text}) > 80 THEN rtrim(substr({first}, 1, 79)) || '…' ELSE {text} END"
                        ))
                    } else {
                        Ok(text)
                    }
                }
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(if parts.is_empty() {
            "''".to_owned()
        } else {
            parts.join(" || ")
        })
    }

    fn line_expression(byte: &str) -> String {
        format!(
            "COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(file.line_starts) point WHERE CAST(point.value AS INTEGER) <= {byte}), 0)"
        )
    }

    fn column_expression(byte: &str) -> String {
        format!(
            "{byte} - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(file.line_starts) point WHERE CAST(point.value AS INTEGER) <= {byte}), 0)"
        )
    }

    fn span_expression(start: &str, end: &str) -> String {
        format!(
            "json_object('endByte', {end}, 'endColumn', {}, 'endLine', {}, 'startByte', {start}, 'startColumn', {}, 'startLine', {})",
            Self::column_expression(end),
            Self::line_expression(end),
            Self::column_expression(start),
            Self::line_expression(start),
        )
    }

    fn output_cte(ordinal: usize, output: &OutputPlan) -> Result<String, String> {
        let row = "result";
        let binding_start = Self::binding_start(row);
        let binding_end = Self::binding_end(row);
        let capture_range = |name: &str| {
            let name = Self::sql_text(name);
            (
                format!(
                    "COALESCE((SELECT min({binding_start}) FROM json_each({row}.bindings) binding WHERE json_extract(binding.value, '$.label') = {name}), {row}.start_byte)"
                ),
                format!(
                    "COALESCE((SELECT max({binding_end}) FROM json_each({row}.bindings) binding WHERE json_extract(binding.value, '$.label') = {name}), {row}.end_byte)"
                ),
            )
        };
        let (start, end, fallback) = match &output.violation.anchor {
            Anchor::Span => (
                format!("{row}.start_byte"),
                format!("{row}.end_byte"),
                "0".to_owned(),
            ),
            Anchor::Start => (
                format!("{row}.start_byte"),
                format!("{row}.start_byte"),
                "0".to_owned(),
            ),
            Anchor::End => (
                format!("{row}.end_byte"),
                format!("{row}.end_byte"),
                "0".to_owned(),
            ),
            Anchor::FileStart => ("0".to_owned(), "0".to_owned(), "0".to_owned()),
            Anchor::Capture(name) => {
                let (start, end) = capture_range(name);
                let name = Self::sql_text(name);
                (
                    start,
                    end,
                    format!(
                        "NOT EXISTS (SELECT 1 FROM json_each({row}.bindings) binding WHERE json_extract(binding.value, '$.label') = {name} AND (json_extract(binding.value, '$.node') IS NOT NULL OR json_extract(binding.value, '$.start_byte') IS NOT NULL))"
                    ),
                )
            }
        };
        let message = output.violation.message_template.as_ref().map_or_else(
            || Ok(Self::sql_text(&output.violation.message)),
            |template| Self::template_expression(template, output, row, true),
        )?;
        let captures = format!(
            "COALESCE((SELECT json_group_object(grouped.label, json(grouped.spans)) FROM (SELECT json_extract(binding.value, '$.label') AS label, json_group_array({}) AS spans FROM json_each({row}.bindings) binding WHERE (json_extract(binding.value, '$.node') IS NOT NULL OR json_extract(binding.value, '$.start_byte') IS NOT NULL) AND json_extract(binding.value, '$.label') NOT LIKE '@caller:%' GROUP BY json_extract(binding.value, '$.label') ORDER BY json_extract(binding.value, '$.label')) grouped), json('{{}}'))",
            Self::span_expression(&binding_start, &binding_end)
        );
        let variables = format!(
            "CASE WHEN {fallback} THEN json_object('anchorFallback', json('true')) ELSE json('{{}}') END"
        );
        let fix = output.violation.fix_template.as_ref().map_or_else(
            || Ok("NULL".to_owned()),
            |template| {
                let (fix_start, fix_end) = output
                    .violation
                    .fix_range
                    .as_deref()
                    .map(capture_range)
                    .unwrap_or_else(|| (format!("{row}.start_byte"), format!("{row}.end_byte")));
                let span = Self::span_expression(&fix_start, &fix_end);
                let text = Self::template_expression(template, output, row, false)?;
                Ok::<_, String>(format!("json_object('span', json({span}), 'text', {text})"))
            },
        )?;
        let suppressed = match output.violation.suppress.as_ref() {
            None => "0".to_owned(),
            Some(suppress)
                if suppress.eslint.is_none()
                    && suppress.nosec.is_none()
                    && suppress.phpstan.is_none()
                    && suppress.psalm.is_none()
                    && suppress.phpcs.is_none() =>
            {
                let noqa = suppress
                    .noqa
                    .as_ref()
                    .ok_or("an empty suppression plan is invalid")?;
                let line = format!(
                    "rtrim(lower(CAST(substr(CAST(file.source AS BLOB), (SELECT max(CAST(point.value AS INTEGER)) FROM json_each(file.line_starts) point WHERE CAST(point.value AS INTEGER) <= {start}) + 1, COALESCE((SELECT min(CAST(point.value AS INTEGER)) FROM json_each(file.line_starts) point WHERE CAST(point.value AS INTEGER) > {start}), length(CAST(file.source AS BLOB))) - (SELECT max(CAST(point.value AS INTEGER)) FROM json_each(file.line_starts) point WHERE CAST(point.value AS INTEGER) <= {start})) AS TEXT)), char(10) || char(13))"
                );
                let marker = format!("instr({line}, '# noqa')");
                let tail = format!("substr({line}, {marker} + 6)");
                let normalized = format!(
                    "replace(replace(replace(replace(substr({tail}, instr({tail}, ':') + 1), ' ', ''), char(9), ''), char(13), ''), char(10), '')"
                );
                let codes = noqa
                    .codes
                    .iter()
                    .map(|code| {
                        format!(
                            "instr(',' || {normalized} || ',', ',' || lower({}) || ',') > 0",
                            Self::sql_text(code)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" OR ");
                format!(
                    "{marker} > 0 AND substr({tail}, 1, 1) IN ('', ':', ' ', char(9)) AND (instr({tail}, ':') = 0 OR ({codes}))"
                )
            }
            Some(_) => {
                return Err(
                    "SQL output suppression supports only the bounded noqa contract".to_owned(),
                );
            }
        };
        let span = Self::span_expression(&start, &end);
        let (each_start, each_end) = output.violation.each.as_deref().map_or_else(
            || ("NULL".to_owned(), "NULL".to_owned()),
            |name| {
                let name = Self::sql_text(name);
                (
                    format!("(SELECT min({binding_start}) FROM json_each({row}.bindings) binding WHERE json_extract(binding.value, '$.label') = {name})"),
                    format!("(SELECT max({binding_end}) FROM json_each({row}.bindings) binding WHERE json_extract(binding.value, '$.label') = {name})"),
                )
            },
        );
        let stable_order = format!(
            "printf('%08d/%08d/%020d/%s/%020d/%020d', {ordinal}, {row}.file_index, {row}.node, {row}.choice_order, {row}.producer_id, {row}.match_index)"
        );
        Ok(format!(
            "clause_{ordinal}_output AS (\n  SELECT 1 AS output_kind, {} AS rule_id, {} AS clause_id, {} AS code, {message} AS message, file.path AS path, {start} AS start_byte, {end} AS end_byte, {} AS mode, {stable_order} AS stable_order, {captures} AS captures, {variables} AS variables, {fix} AS fix, {span} AS span, {row}.file_index, {row}.node, {row}.bindings, {row}.start_byte AS witness_start_byte, {row}.end_byte AS witness_end_byte, {each_start} AS each_start_byte, {each_end} AS each_end_byte, NULL AS error_code, {ordinal} AS clause_ordinal FROM clause_{ordinal} {row} JOIN file_metadata file USING (file_index) WHERE {row}.row_kind <> 8 AND NOT ({suppressed})\n  UNION ALL\n  SELECT 2, {}, {}, NULL, NULL, file.path, NULL, NULL, {}, printf('%08d/%08d/%020d', {ordinal}, {row}.file_index, {row}.producer_id), json('{{}}'), json('{{}}'), NULL, NULL, {row}.file_index, {row}.node, json('[]'), {row}.start_byte, {row}.end_byte, NULL, NULL, {row}.error_code, {ordinal} FROM clause_{ordinal} {row} JOIN file_metadata file USING (file_index) WHERE {row}.row_kind = 8\n)",
            Self::sql_text(&output.rule_id),
            Self::sql_text(&output.clause_id),
            Self::sql_text(&output.violation.code),
            Self::sql_text(output.mode.as_str()),
            Self::sql_text(&output.rule_id),
            Self::sql_text(&output.clause_id),
            Self::sql_text(output.mode.as_str()),
        ))
    }

    fn semantic_rule(
        &mut self,
        slot: u16,
        rule: &AstMatcher,
        language: GrammarDialect,
        at: &str,
        terminals: &mut Vec<usize>,
    ) -> SemanticRuleSpec {
        if let AstMatcher::Kind(kind) = rule
            && let Some(kinds) = kind.potential_kinds()
            && kinds.iter().count() == 1
            && let Some(kind) = kinds
                .iter()
                .next()
                .and_then(|kind| u16::try_from(kind).ok())
        {
            return SemanticRuleSpec {
                slot,
                target_kind: Some(kind),
                relation: None,
                incomplete_position: None,
                deduplicate_source: false,
            };
        }
        let mut relation = RelationalIr::empty();
        self.lower_matcher(rule, language, at, terminals, &mut relation);
        SemanticRuleSpec {
            slot,
            target_kind: None,
            relation: Some(Box::new(relation)),
            incomplete_position: None,
            deduplicate_source: false,
        }
    }

    fn lower_parse(
        &mut self,
        test: &crate::code_rule_dsl::matcher::ParseTest,
        language: GrammarDialect,
        at: &str,
        terminals: &mut Vec<usize>,
        ir: &mut RelationalIr,
    ) {
        fn one_kind(matcher: &AstMatcher) -> Option<u16> {
            let AstMatcher::Kind(kind) = matcher else {
                return None;
            };
            let kinds = kind.potential_kinds()?;
            (kinds.iter().count() == 1)
                .then(|| u16::try_from(kinds.iter().next().expect("one kind")).ok())
                .flatten()
        }
        fn candidate_shape(
            matcher: &AstMatcher,
            target: &mut Option<u16>,
            export: &mut Option<String>,
            host: &mut Option<u16>,
        ) -> bool {
            match matcher {
                AstMatcher::All(items) => items
                    .iter()
                    .all(|item| candidate_shape(item, target, export, host)),
                AstMatcher::Kind(_) => one_kind(matcher)
                    .filter(|kind| target.replace(*kind).is_none())
                    .is_some(),
                AstMatcher::Semantic(Semantic::Capture(label)) => {
                    export.replace(label.clone()).is_none()
                }
                AstMatcher::Semantic(Semantic::Host(test)) => one_kind(test.rule())
                    .filter(|kind| host.replace(*kind).is_none())
                    .is_some(),
                _ => false,
            }
        }
        let AstMatcher::Has(candidate, StopBy::End) = &test.rule else {
            self.unsupported
                .push(format!("{at}: parse requires a bounded descendant rule"));
            return;
        };
        let (mut target, mut export, mut host) = (None, None, None);
        if !candidate_shape(candidate.rule(), &mut target, &mut export, &mut host) {
            self.unsupported.push(format!(
                "{at}: parse descendant rule is not relationally lowered"
            ));
            return;
        }
        if export.is_none() && test.exports.len() == 1 {
            export = test.exports.first().cloned();
        }
        let Some(target_kind) = target else {
            self.unsupported
                .push(format!("{at}: parse descendant has no fixed kind"));
            return;
        };
        if export
            .as_ref()
            .is_some_and(|label| !test.exports.contains(label))
        {
            self.unsupported
                .push(format!("{at}: parse capture is not in the export catalog"));
            return;
        }
        let host_producer = self.universe_terminal(language);
        if !terminals.contains(&host_producer) {
            terminals.push(host_producer);
        }
        let required_host_producer = host.map(|kind| {
            let producer = self.kind_terminal(language, kind);
            if !terminals.contains(&producer) {
                terminals.push(producer);
            }
            producer
        });
        let producer = self.parse_terminal(
            language,
            at,
            ParseFactDescriptor {
                grammar: test.grammar,
                host_kinds: Arc::clone(&test.host_kinds),
                reading: test.reading,
            },
        );
        if !terminals.contains(&producer) {
            terminals.push(producer);
        }
        *ir = RelationalIr::parsed(ParseFactSpec {
            producer,
            host_producer,
            required_host_producer,
            target_kind,
            export,
        });
    }

    fn lower_semantic(
        &mut self,
        semantic: &Semantic,
        language: GrammarDialect,
        at: &str,
        terminals: &mut Vec<usize>,
        ir: &mut RelationalIr,
    ) {
        if let Semantic::Parse(test) = semantic {
            self.lower_parse(test, language, at, terminals, ir);
            return;
        }
        use crate::code_rule_dsl::transducer::semantic_facts::slot;
        let mut rules = Vec::new();
        let mut add = |slot, rule: &Option<AstMatcher>, suffix: &str, this: &mut Self| {
            if let Some(rule) = rule {
                rules.push(this.semantic_rule(
                    slot,
                    rule,
                    language,
                    &format!("{at}/{suffix}"),
                    terminals,
                ));
            }
        };
        let mut call_through = None;
        match semantic {
            Semantic::Binding(test) => {
                add(slot::SITE, &test.site, "site", self);
                add(slot::VALUE, &test.value, "value", self);
                add(slot::ANNOTATION, &test.annotation, "annotation", self);
                add(slot::OCCURRENCE, &test.occurrence, "occurrence", self);
            }
            Semantic::Binds(test) => {
                add(slot::SITE, &test.site, "site", self);
                add(slot::VALUE, &test.value, "value", self);
                add(slot::ANNOTATION, &test.annotation, "annotation", self);
                add(slot::OCCURRENCE, &test.occurrence, "occurrence", self);
                if let Some(redefines) = &test.redefines {
                    let base = if language == GrammarDialect::Python {
                        slot::SHADOWED_SITE
                    } else {
                        slot::OTHER_SITE
                    };
                    add(base, &redefines.site, "redefines/site", self);
                    add(base + 1, &redefines.value, "redefines/value", self);
                    add(
                        base + 2,
                        &redefines.annotation,
                        "redefines/annotation",
                        self,
                    );
                }
            }
            Semantic::FormatArgs(test) => {
                add(slot::UNUSED, &test.unused, "unused", self);
                add(slot::MISSING, &test.missing, "missing", self);
                for rule in &mut rules {
                    rule.incomplete_position = match rule.slot {
                        slot::UNUSED => Some(0),
                        slot::MISSING => Some(1),
                        _ => None,
                    };
                }
            }
            Semantic::Calls(test) => {
                let Some(term) = self.current_calls.get(&test.key).cloned() else {
                    self.unsupported
                        .push(format!("{at}: calls term has no immutable seed"));
                    return;
                };
                let Query::Ast { matchers, .. } = &term.seed else {
                    self.unsupported
                        .push(format!("{at}: calls seed is not an AST relation"));
                    return;
                };
                let Some(seed) = matchers.iter().find(|matcher| matcher.language == language)
                else {
                    self.unsupported
                        .push(format!("{at}: calls seed has no matching dialect"));
                    return;
                };
                let mut relation = RelationalIr::empty();
                self.lower_matcher(
                    &seed.matcher,
                    language,
                    &format!("{at}/calls/seed"),
                    terminals,
                    &mut relation,
                );
                rules.push(SemanticRuleSpec {
                    slot: slot::CALL_TARGET,
                    target_kind: None,
                    relation: Some(Box::new(relation)),
                    incomplete_position: None,
                    deduplicate_source: true,
                });
                // P's `calls` delta gives a term a `Mode`; N's catalog reads its `through`.
                call_through = Some(term.through.through);
            }
            Semantic::Resolves(_) | Semantic::Arguments(_) | Semantic::Layout(_) => {}
            Semantic::Parse(_) => unreachable!("parse returned before semantic fact lowering"),
            // W's `embedded` (embedded documents) and P's `filePath` have no fact family in N's catalog.
            Semantic::Value(_)
            | Semantic::Host(_)
            | Semantic::Capture(_)
            | Semantic::Embedded(_)
            | Semantic::FilePath(_) => {
                self.unsupported.push(format!(
                    "{at}: semantic fact family belongs to a later phase"
                ));
                return;
            }
        }
        use crate::code_rule_dsl::transducer::semantic_facts::{
            BindingRequest, BindsRequest, RedefinesRequest,
        };
        let input_labels = match semantic {
            Semantic::FormatArgs(test) => vec![test.string.clone(), test.args.clone()],
            Semantic::Layout(test) => test.reads().map(str::to_owned).collect(),
            Semantic::Binding(test) => test
                .filter
                .bindings
                .as_ref()
                .and_then(|count| count.within.clone())
                .into_iter()
                .collect(),
            Semantic::Binds(test) => test
                .filter
                .bindings
                .as_ref()
                .and_then(|count| count.within.clone())
                .into_iter()
                .chain(
                    test.redefines
                        .iter()
                        .filter_map(|redefines| redefines.filter.bindings.as_ref())
                        .filter_map(|count| count.within.clone()),
                )
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            _ => Vec::new(),
        };
        let require_all_candidates = matches!(
            semantic,
            Semantic::Binds(test)
                if test.redefines.as_ref().is_some_and(|redefines| redefines.every)
        );
        let argument_labels = match semantic {
            Semantic::Arguments(arguments) => arguments
                .bind
                .iter()
                .map(|(label, _)| label.clone())
                .collect(),
            _ => Vec::new(),
        };
        let source = match semantic {
            Semantic::Resolves(test) => SemanticFactSource::Resolves((**test).clone()),
            Semantic::Binding(test) => {
                let request = BindingRequest {
                    site: test.site.is_some(),
                    value: test.value.is_some(),
                    annotation: test.annotation.is_some(),
                    occurrence: test.occurrence.is_some(),
                };
                let mut test = (**test).clone();
                (test.site, test.value, test.annotation, test.occurrence) =
                    (None, None, None, None);
                SemanticFactSource::Binding {
                    test: Box::new(test),
                    request,
                }
            }
            Semantic::Binds(test) => {
                let request = BindsRequest {
                    binding: BindingRequest {
                        site: test.site.is_some(),
                        value: test.value.is_some(),
                        annotation: test.annotation.is_some(),
                        occurrence: test.occurrence.is_some(),
                    },
                    redefines: test.redefines.as_ref().map(|redefines| RedefinesRequest {
                        site: redefines.site.is_some(),
                        value: redefines.value.is_some(),
                        annotation: redefines.annotation.is_some(),
                    }),
                };
                let mut test = (**test).clone();
                (test.site, test.value, test.annotation, test.occurrence) =
                    (None, None, None, None);
                if let Some(redefines) = test.redefines.as_deref_mut() {
                    (redefines.site, redefines.value, redefines.annotation) = (None, None, None);
                }
                SemanticFactSource::Binds {
                    test: Box::new(test),
                    request,
                }
            }
            Semantic::Arguments(test) => SemanticFactSource::Arguments((**test).clone()),
            Semantic::FormatArgs(test) => {
                let mut test = (**test).clone();
                (test.unused, test.missing) = (None, None);
                SemanticFactSource::FormatArgs(test)
            }
            Semantic::Layout(test) => SemanticFactSource::Layout((**test).clone()),
            Semantic::Calls(test) => SemanticFactSource::Calls((**test).clone()),
            Semantic::Value(_)
            | Semantic::Parse(_)
            | Semantic::Host(_)
            | Semantic::Capture(_)
            | Semantic::Embedded(_)
            | Semantic::FilePath(_) => {
                unreachable!("unsupported semantic families returned before descriptor creation")
            }
        };
        let id = self.fact_terminal(language, at, source, input_labels, call_through);
        terminals.push(id);
        *ir = RelationalIr::semantic(SemanticFactSpec {
            producer: id,
            argument_labels,
            rules,
            require_all_candidates,
        });
    }

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
                    if language == GrammarDialect::Blade {
                        ir.operation(SqlOperation::MapDocument);
                    }
                }
            }
            AstMatcher::All(items) => {
                let operation = format!("{at}/all");
                let errors_before = self.unsupported.len();
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
                if self.unsupported.len() != errors_before {
                    return;
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
            AstMatcher::Has(item, stop) => self.lower_navigation(
                NavigationAxis::Descendant,
                item.rule(),
                stop,
                None,
                false,
                language,
                &format!("{at}/has"),
                terminals,
                ir,
            ),
            AstMatcher::Pattern(pattern) => match Program::compile_ast_grep_pattern(pattern) {
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
                let errors_before = self.unsupported.len();
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
                if self.unsupported.len() != errors_before {
                    return;
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
            AstMatcher::Inside(item, stop) => self.lower_navigation(
                NavigationAxis::Ancestor,
                item.rule(),
                stop,
                None,
                false,
                language,
                &format!("{at}/inside"),
                terminals,
                ir,
            ),
            AstMatcher::Precedes(item, stop) => self.lower_navigation(
                NavigationAxis::FollowingSibling,
                item.rule(),
                stop,
                None,
                false,
                language,
                &format!("{at}/precedes"),
                terminals,
                ir,
            ),
            AstMatcher::Follows(item, stop) => self.lower_navigation(
                NavigationAxis::PrecedingSibling,
                item.rule(),
                stop,
                None,
                false,
                language,
                &format!("{at}/follows"),
                terminals,
                ir,
            ),
            AstMatcher::Field(relation) => self.lower_navigation(
                if relation.inside {
                    NavigationAxis::Ancestor
                } else {
                    NavigationAxis::Descendant
                },
                &relation.rule,
                &relation.stop,
                Some(relation.field),
                !relation.inside,
                language,
                &format!("{at}/field"),
                terminals,
                ir,
            ),
            AstMatcher::Constrained(core) => {
                let operation = format!("{at}/constraints");
                let mut source = RelationalIr::empty();
                self.lower_matcher(
                    &core.rule,
                    language,
                    &format!("{operation}/rule"),
                    terminals,
                    &mut source,
                );
                let mut constraints = Vec::with_capacity(core.constraints.len());
                for (name, constraint) in &core.constraints {
                    let mut lowered = RelationalIr::empty();
                    self.lower_matcher(
                        constraint,
                        language,
                        &format!("{operation}/{name}"),
                        terminals,
                        &mut lowered,
                    );
                    constraints.push((name.clone(), lowered));
                }
                match RelationalIr::constrain(source, constraints) {
                    Ok(lowered) => *ir = lowered,
                    Err(error) => self.unsupported.push(format!("{operation}: {error}")),
                }
            }
            AstMatcher::NthChild(nth) => {
                let universe_id = self.universe_terminal(language);
                let navigation_id = self.navigation_terminal(language);
                for id in [universe_id, navigation_id] {
                    if !terminals.contains(&id) {
                        terminals.push(id);
                    }
                }
                let mut source = RelationalIr::empty();
                source.terminal_spec(TerminalKind::Universe, format!("terminal:{universe_id}"));
                source.operation(SqlOperation::ScanTerminal);
                let of_rule = nth.of_rule.as_ref().map(|rule| {
                    let mut lowered = RelationalIr::empty();
                    self.lower_matcher(
                        rule,
                        language,
                        &format!("{at}/nthChild/ofRule"),
                        terminals,
                        &mut lowered,
                    );
                    lowered
                });
                match RelationalIr::nth_child(
                    NthChildSpec {
                        step: nth.position.step,
                        offset: nth.position.offset,
                        reverse: nth.reverse,
                        producer: navigation_id,
                    },
                    source,
                    of_rule,
                ) {
                    Ok(lowered) => *ir = lowered,
                    Err(error) => self.unsupported.push(format!("{at}/nthChild: {error}")),
                }
            }
            AstMatcher::Count(count) => {
                self.lower_count(count, language, at, terminals, ir);
            }
            AstMatcher::Compare(comparison) => {
                if !comparison.commute.is_empty() {
                    self.unsupported
                        .push(format!("{at}: commuted comparison is not lowered"));
                    return;
                }
                let transformed = comparison.transform.as_ref().map(|transform| {
                    [&comparison.left, &comparison.right]
                        .into_iter()
                        .filter(|name| transform.contains(name))
                        .map(|name| {
                            let mut roots = transform.roots(name).into_iter();
                            let Some(root) = roots.next() else {
                                return Err(format!(
                                    "{at}: transformed comparison operand `{name}` must have one capture source"
                                ));
                            };
                            if roots.next().is_some() {
                                return Err(format!(
                                    "{at}: transformed comparison operand `{name}` must have one capture source"
                                ));
                            }
                            Ok((name.to_string(), root.to_owned()))
                        })
                        .collect::<Result<Vec<_>, String>>()
                });
                let transformed = match transformed.transpose() {
                    Ok(transformed) => transformed.unwrap_or_default(),
                    Err(error) => {
                        self.unsupported.push(error);
                        return;
                    }
                };
                let transform_producer = (!transformed.is_empty()).then(|| {
                    self.transform_terminal(
                        language,
                        at,
                        Arc::clone(comparison.transform.as_ref().expect("transformed operands")),
                        transformed.clone(),
                        terminals.clone(),
                    )
                });
                if let Some(producer) = transform_producer
                    && !terminals.contains(&producer)
                {
                    terminals.push(producer);
                }
                let source = |name: &str| {
                    transformed
                        .iter()
                        .find_map(|(output, source)| (output == name).then(|| source.clone()))
                        .unwrap_or_else(|| name.to_owned())
                };
                *ir = RelationalIr::comparison(ComparisonSpec {
                    left: comparison.left.clone(),
                    left_source: source(&comparison.left),
                    left_transform: transformed
                        .iter()
                        .any(|(output, _)| output == &comparison.left)
                        .then(|| transform_producer.expect("a transformed operand has a producer")),
                    op: comparison.op,
                    right: comparison.right.clone(),
                    right_source: source(&comparison.right),
                    right_transform: transformed
                        .iter()
                        .any(|(output, _)| output == &comparison.right)
                        .then(|| transform_producer.expect("a transformed operand has a producer")),
                    collation: comparison.collation,
                    text: comparison.text,
                });
            }
            AstMatcher::Semantic(Semantic::Value(test)) => {
                let unsupported_value_shape = !test.types.is_empty()
                    || test.constant.is_some()
                    || !test.starts_with.is_empty()
                    || test.regex.is_some()
                    || test.bits_any.is_some()
                    || !test.bounds.is_empty()
                    || test.truthy.is_some();
                let predicate_fact = unsupported_value_shape || test.eq.is_none();
                let expected = if predicate_fact {
                    None
                } else {
                    let expected = test.eq.as_ref().expect("a simple equality has a literal");
                    Some(match expected {
                        Value::None => ValueLiteral::Null,
                        Value::Bool(value) => ValueLiteral::Bool(*value),
                        Value::Int(value) => ValueLiteral::Integer(value.to_string()),
                        Value::Float(value) => ValueLiteral::Float(value.to_string()),
                        Value::Str(value) => ValueLiteral::String(value.clone()),
                        _ => {
                            self.unsupported.push(format!(
                                "{at}: value equality literal is not representable as a SQL fact"
                            ));
                            return;
                        }
                    })
                };
                let descriptor = ValueFactDescriptor {
                    language: test.reading.language,
                    follow: test.reading.follow,
                    expression_kinds: test.reading.expressions.iter().copied().collect(),
                    reading: test.reading.clone(),
                    predicate: predicate_fact.then(|| test.as_ref().clone()),
                };
                let id = self.value_terminal(at, descriptor);
                if !terminals.contains(&id) {
                    terminals.push(id);
                }
                *ir = RelationalIr::value(ValuePredicateSpec {
                    producer: id,
                    language,
                    eq: expected,
                });
            }
            AstMatcher::Never => *ir = RelationalIr::empty_relation(),
            AstMatcher::Util(util) => {
                self.lower_utility(util.clone(), None, language, at, terminals, ir);
            }
            AstMatcher::Call(call) => {
                self.lower_utility(call.util.clone(), Some(call), language, at, terminals, ir);
            }
            AstMatcher::Semantic(semantic) => {
                self.lower_semantic(semantic, language, at, terminals, ir);
            }
            #[cfg(test)]
            AstMatcher::TestUnknown(_) => self
                .unsupported
                .push(format!("{at}: test-only unknown operation")),
        }
    }

    fn lower_utility(
        &mut self,
        util: UtilRef,
        call: Option<&UtilCall>,
        language: GrammarDialect,
        at: &str,
        terminals: &mut Vec<usize>,
        ir: &mut RelationalIr,
    ) {
        let Some(key) = utility_plan_key(&util) else {
            self.unsupported
                .push(format!("{at}: utility component has no semantic identity"));
            return;
        };
        let Some(component) = util.component_name() else {
            self.unsupported
                .push(format!("{at}: utility component has no stable identity"));
            return;
        };
        *self.utility_call_counts.entry(key).or_default() += 1;
        if let Some(graph) = Self::recursive_component(&util)
            && graph
                .keys()
                .all(|component| !self.utility_stack.contains(component))
        {
            let cache_key = (key, language, self.utility_stack.clone());
            if call.is_none()
                && let Some((body, body_terminals)) = self.recursive_bodies.get(&cache_key)
            {
                *ir = body.instantiate();
                let body_terminals = body_terminals.clone();
                for id in body_terminals {
                    self.mark_terminal_required(id);
                    if !terminals.contains(&id) {
                        terminals.push(id);
                    }
                }
                return;
            }
            let errors_before = self.unsupported.len();
            let mut body_terminals = Vec::new();
            self.lower_recursive_utility(
                &util,
                call,
                &component,
                graph,
                language,
                at,
                &mut body_terminals,
                ir,
            );
            for id in &body_terminals {
                self.mark_terminal_required(*id);
                if !terminals.contains(id) {
                    terminals.push(*id);
                }
            }
            if call.is_none() && self.unsupported.len() == errors_before {
                self.recursive_bodies
                    .insert(cache_key, (ir.clone().into_shared(), body_terminals));
            }
            return;
        }
        if self.utility_stack.is_empty()
            && Self::utility_depth(&util).is_some_and(|depth| depth > MAX_UTIL_DEPTH)
        {
            let universe = self.universe_terminal(language);
            if !terminals.contains(&universe) {
                terminals.push(universe);
            }
            let spec = UtilitySpec {
                definition: key,
                component,
                universe_producer: universe,
                depth: MAX_UTIL_DEPTH + 1,
                max_depth: MAX_UTIL_DEPTH,
                max_work: self.current_max_work,
            };
            *ir = RelationalIr::utility(spec, RelationalIr::empty_relation())
                .expect("a static utility-limit relation has one empty body");
            return;
        }
        if self.utility_stack.contains(&key) {
            self.unsupported.push(format!(
                "{at}: recursive utility component `{component}` has an unsupported recursive form"
            ));
            return;
        }
        let depth = u32::try_from(self.utility_stack.len() + 1).unwrap_or(u32::MAX);
        let body = if call.is_none() {
            let cache_key = (key, language, self.utility_stack.clone());
            if let Some((body, body_terminals)) = self.utility_bodies.get(&cache_key) {
                let body = body.instantiate();
                let body_terminals = body_terminals.clone();
                for id in body_terminals {
                    self.mark_terminal_required(id);
                    if !terminals.contains(&id) {
                        terminals.push(id);
                    }
                }
                body
            } else {
                let Some(rule) = util.with_target(Clone::clone) else {
                    self.unsupported.push(format!(
                        "{at}: utility component `{component}` is unavailable"
                    ));
                    return;
                };
                self.utility_stack.push(key);
                let mut body = RelationalIr::empty();
                let mut body_terminals = Vec::new();
                self.lower_matcher(
                    &rule,
                    language,
                    &format!("{at}/utility/{component}"),
                    &mut body_terminals,
                    &mut body,
                );
                let popped = self.utility_stack.pop();
                debug_assert_eq!(popped, Some(key));
                for id in &body_terminals {
                    self.mark_terminal_required(*id);
                    if !terminals.contains(id) {
                        terminals.push(*id);
                    }
                }
                let shared = body.clone().into_shared();
                self.utility_bodies
                    .insert(cache_key, (shared, body_terminals));
                body
            }
        } else {
            let Some(rule) = util.with_target(Clone::clone) else {
                self.unsupported.push(format!(
                    "{at}: utility component `{component}` is unavailable"
                ));
                return;
            };
            self.utility_stack.push(key);
            let mut body = RelationalIr::empty();
            self.lower_matcher(
                &rule,
                language,
                &format!("{at}/utility/{component}"),
                terminals,
                &mut body,
            );
            let popped = self.utility_stack.pop();
            debug_assert_eq!(popped, Some(key));
            body
        };
        let universe = self.universe_terminal(language);
        if !terminals.contains(&universe) {
            terminals.push(universe);
        }
        let utility = UtilitySpec {
            definition: key,
            component,
            universe_producer: universe,
            depth,
            max_depth: MAX_UTIL_DEPTH,
            max_work: self.current_max_work,
        };
        let lowered = match call {
            Some(call) => RelationalIr::utility_frame(
                UtilityFrameSpec {
                    utility,
                    arguments: call.args.clone(),
                },
                body,
            ),
            None => RelationalIr::utility(utility, body),
        };
        match lowered {
            Ok(lowered) => *ir = lowered,
            Err(error) => self.unsupported.push(format!("{at}: {error}")),
        }
    }

    fn utility_depth(root: &UtilRef) -> Option<u32> {
        fn visit(
            util: &UtilRef,
            active: &mut BTreeSet<UtilityKey>,
            completed: &mut BTreeMap<UtilityKey, u32>,
        ) -> Option<u32> {
            let key = utility_plan_key(util)?;
            if let Some(depth) = completed.get(&key) {
                return Some(*depth);
            }
            if !active.insert(key) {
                return None;
            }
            let rule = util.with_target(Clone::clone)?;
            let mut references = Vec::new();
            utility_references(&rule, &mut references);
            let dependency_depth = references
                .iter()
                .map(|reference| visit(reference, active, completed))
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .max()
                .unwrap_or(0);
            active.remove(&key);
            let depth = dependency_depth.saturating_add(1);
            completed.insert(key, depth);
            Some(depth)
        }
        visit(root, &mut BTreeSet::new(), &mut BTreeMap::new())
    }

    fn recursive_component(root: &UtilRef) -> Option<UtilityGraph> {
        fn collect(util: UtilRef, graph: &mut UtilityGraph) -> Option<()> {
            let key = utility_plan_key(&util)?;
            if graph.contains_key(&key) {
                return Some(());
            }
            let rule = util.with_target(Clone::clone)?;
            let mut references = Vec::new();
            utility_references(&rule, &mut references);
            let dependencies = references
                .iter()
                .map(utility_plan_key)
                .collect::<Option<Vec<_>>>()?;
            graph.insert(key, (util, rule, dependencies));
            for reference in references {
                collect(reference, graph)?;
            }
            Some(())
        }
        fn reaches(
            graph: &UtilityGraph,
            from: UtilityKey,
            target: UtilityKey,
            seen: &mut BTreeSet<UtilityKey>,
        ) -> bool {
            from == target
                || (seen.insert(from)
                    && graph.get(&from).is_some_and(|(_, _, dependencies)| {
                        dependencies
                            .iter()
                            .any(|dependency| reaches(graph, *dependency, target, seen))
                    }))
        }
        let mut graph = BTreeMap::new();
        collect(root.clone(), &mut graph)?;
        let root_key = utility_plan_key(root)?;
        let component = graph
            .iter()
            .filter(|(key, _)| reaches(&graph, **key, root_key, &mut BTreeSet::new()))
            .map(|(key, value)| (*key, value.clone()))
            .collect::<BTreeMap<_, _>>();
        (component.len() > 1
            || component
                .get(&root_key)
                .is_some_and(|(_, _, dependencies)| dependencies.contains(&root_key)))
        .then_some(component)
    }

    fn recursive_complement_rule(
        rule: &AstMatcher,
        keys: &BTreeSet<UtilityKey>,
        negated: bool,
    ) -> Result<AstMatcher, String> {
        let nested =
            |rule: &AstMatcher, negated| Self::recursive_complement_rule(rule, keys, negated);
        let wrap = |rule: AstMatcher| {
            if negated {
                AstMatcher::Not(Box::new(rule))
            } else {
                rule
            }
        };
        match rule {
            AstMatcher::All(items) if negated && items.len() > 1 => {
                let Some(guard_index) = items
                    .iter()
                    .position(|item| !Self::recursive_references_component(item, keys))
                else {
                    return Ok(AstMatcher::Any(
                        items
                            .iter()
                            .map(|item| nested(item, true))
                            .collect::<Result<Vec<_>, _>>()?,
                    ));
                };
                let guard = items[guard_index].clone();
                let mut remainder = items.clone();
                remainder.remove(guard_index);
                let remainder = if remainder.len() == 1 {
                    remainder.pop().expect("one remaining conjunction item")
                } else {
                    AstMatcher::All(remainder)
                };
                Ok(AstMatcher::Any(vec![
                    nested(&guard, true)?,
                    AstMatcher::All(vec![guard, nested(&remainder, true)?]),
                ]))
            }
            AstMatcher::Any(items) if negated => {
                let mut guarded = Vec::new();
                let mut seen = BTreeSet::new();
                for item in items {
                    let AstMatcher::All(terms) = item else {
                        guarded.clear();
                        break;
                    };
                    let Some((kind_index, kind_id)) =
                        terms.iter().enumerate().find_map(|(index, term)| {
                            let AstMatcher::Kind(kind) = term else {
                                return None;
                            };
                            let kinds = kind.potential_kinds()?;
                            if kinds.iter().count() != 1 {
                                return None;
                            }
                            let kind =
                                u16::try_from(kinds.iter().next().expect("one kind")).ok()?;
                            Some((index, kind))
                        })
                    else {
                        guarded.clear();
                        break;
                    };
                    if !seen.insert(kind_id) {
                        guarded.clear();
                        break;
                    }
                    let mut remainder = terms.clone();
                    let kind = remainder.remove(kind_index);
                    let remainder = if remainder.len() == 1 {
                        remainder.pop().expect("one guarded term")
                    } else {
                        AstMatcher::All(remainder)
                    };
                    guarded.push((kind, nested(&remainder, true)?));
                }
                if guarded.len() == items.len() && !guarded.is_empty() {
                    let mut alternatives = guarded
                        .iter()
                        .map(|(kind, failed)| AstMatcher::All(vec![kind.clone(), failed.clone()]))
                        .collect::<Vec<_>>();
                    alternatives.push(AstMatcher::All(
                        guarded
                            .into_iter()
                            .map(|(kind, _)| AstMatcher::Not(Box::new(kind)))
                            .collect(),
                    ));
                    Ok(AstMatcher::Any(alternatives))
                } else {
                    let items = items
                        .iter()
                        .map(|item| nested(item, true))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(AstMatcher::All(items))
                }
            }
            AstMatcher::All(items) | AstMatcher::Any(items) => {
                let items = items
                    .iter()
                    .map(|item| nested(item, negated))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(match (matches!(rule, AstMatcher::All(_)), negated) {
                    (true, false) | (false, true) => AstMatcher::All(items),
                    (false, false) | (true, true) => AstMatcher::Any(items),
                })
            }
            AstMatcher::Not(inner) => nested(inner, !negated),
            AstMatcher::Util(reference)
                if utility_plan_key(reference).is_some_and(|key| keys.contains(&key)) =>
            {
                if negated {
                    Ok(AstMatcher::Util(reference.clone()))
                } else {
                    Err("recursive utility has a negative dependency cycle".to_owned())
                }
            }
            AstMatcher::Call(call)
                if utility_plan_key(&call.util).is_some_and(|key| keys.contains(&key)) =>
            {
                if negated {
                    Ok(AstMatcher::Call(call.clone()))
                } else {
                    Err("recursive utility call has a negative dependency cycle".to_owned())
                }
            }
            AstMatcher::Has(related, stop) => {
                let relation = AstMatcher::Has(
                    related.with_rule(nested(related.rule(), false)?),
                    stop.clone(),
                );
                Ok(wrap(relation))
            }
            AstMatcher::Inside(related, stop) => {
                let relation = AstMatcher::Inside(
                    related.with_rule(nested(related.rule(), false)?),
                    stop.clone(),
                );
                Ok(wrap(relation))
            }
            AstMatcher::Precedes(related, stop) => {
                let relation = AstMatcher::Precedes(
                    related.with_rule(nested(related.rule(), false)?),
                    stop.clone(),
                );
                Ok(wrap(relation))
            }
            AstMatcher::Follows(related, stop) => {
                let relation = AstMatcher::Follows(
                    related.with_rule(nested(related.rule(), false)?),
                    stop.clone(),
                );
                Ok(wrap(relation))
            }
            AstMatcher::Field(field)
                if negated && !field.inside && matches!(field.stop, StopBy::Neighbor) =>
            {
                let mut present = (**field).clone();
                present.rule = AstMatcher::Not(Box::new(AstMatcher::Never));
                let mut failed = (**field).clone();
                failed.rule = nested(&field.rule, true)?;
                Ok(AstMatcher::Any(vec![
                    AstMatcher::Not(Box::new(AstMatcher::Field(Box::new(present)))),
                    AstMatcher::Field(Box::new(failed)),
                ]))
            }
            AstMatcher::Field(field) => {
                let mut field = (**field).clone();
                field.rule = nested(&field.rule, false)?;
                Ok(wrap(AstMatcher::Field(Box::new(field))))
            }
            other => Ok(wrap(other.clone())),
        }
    }

    fn recursive_references_component(rule: &AstMatcher, keys: &BTreeSet<UtilityKey>) -> bool {
        let mut references = Vec::new();
        utility_references(rule, &mut references);
        references
            .iter()
            .any(|reference| utility_plan_key(reference).is_some_and(|key| keys.contains(&key)))
    }

    fn recursive_stop(stop: &StopBy) -> (NavigationStop, Option<AstMatcher>) {
        match stop {
            StopBy::Neighbor => (NavigationStop::Neighbor, None),
            StopBy::End => (NavigationStop::End, None),
            StopBy::Rule(rule) => (NavigationStop::Rule, Some((**rule).clone())),
        }
    }

    fn recursive_guard_kind_sets(rule: &AstMatcher, sets: &mut Vec<BTreeSet<u16>>) {
        let kinds = |kind: &ast_grep_core::matcher::KindMatcher| {
            kind.potential_kinds().map(|potential| {
                potential
                    .iter()
                    .filter_map(|kind| u16::try_from(kind).ok())
                    .collect::<BTreeSet<_>>()
            })
        };
        match rule {
            AstMatcher::Kind(kind) => {
                if let Some(kinds) = kinds(kind) {
                    sets.push(kinds);
                }
            }
            AstMatcher::Any(items)
                if items.iter().all(|item| matches!(item, AstMatcher::Kind(_))) =>
            {
                let mut allowed = BTreeSet::new();
                for item in items {
                    let AstMatcher::Kind(kind) = item else {
                        unreachable!("the guarded alternatives are kinds")
                    };
                    if let Some(kinds) = kinds(kind) {
                        allowed.extend(kinds);
                    }
                }
                sets.push(allowed);
            }
            AstMatcher::All(items) => {
                for item in items {
                    Self::recursive_guard_kind_sets(item, sets);
                }
            }
            _ => {}
        }
    }

    fn recursive_root_kinds(rule: &AstMatcher) -> Option<BTreeSet<u16>> {
        match rule {
            AstMatcher::Kind(kind) => kind.potential_kinds().map(|kinds| {
                kinds
                    .iter()
                    .filter_map(|kind| u16::try_from(kind).ok())
                    .collect()
            }),
            AstMatcher::Any(items) => {
                let mut kinds = BTreeSet::new();
                for item in items {
                    kinds.extend(Self::recursive_root_kinds(item)?);
                }
                Some(kinds)
            }
            AstMatcher::All(items) => items.iter().find_map(Self::recursive_root_kinds),
            _ => None,
        }
    }

    fn recursive_alternatives_are_disjoint(rule: &AstMatcher, keys: &BTreeSet<UtilityKey>) -> bool {
        let AstMatcher::Any(items) = rule else {
            return false;
        };
        let recursive = items
            .iter()
            .filter(|item| Self::recursive_references_component(item, keys))
            .collect::<Vec<_>>();
        if recursive.len() < 2 {
            return false;
        }
        let Some(kind_sets) = recursive
            .iter()
            .map(|item| Self::recursive_root_kinds(item))
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        kind_sets.iter().enumerate().all(|(index, left)| {
            kind_sets[index + 1..]
                .iter()
                .all(|right| left.is_disjoint(right))
        })
    }

    fn recursive_transition_templates(
        rule: &AstMatcher,
        keys: &BTreeSet<UtilityKey>,
        discard_incompatible: bool,
    ) -> Result<Option<Vec<RecursiveTransitionTemplate>>, String> {
        let relation =
            |related: &AstMatcher, axis, stop: &StopBy, field, field_stop_children_only| {
                let Some(mut templates) =
                    Self::recursive_transition_templates(related, keys, discard_incompatible)?
                else {
                    return Ok(None);
                };
                let (stop, stop_rule) = Self::recursive_stop(stop);
                for template in &mut templates {
                    template.steps.push(RecursiveStep {
                        axis,
                        stop,
                        field,
                        field_stop_children_only,
                    });
                    if let Some(stop_rule) = &stop_rule {
                        template
                            .stops
                            .push((template.steps.len() - 1, stop_rule.clone()));
                    }
                }
                Ok(Some(templates))
            };
        match rule {
            AstMatcher::Util(target)
                if utility_plan_key(target).is_some_and(|key| keys.contains(&key)) =>
            {
                Ok(Some(vec![RecursiveTransitionTemplate {
                    target: utility_plan_key(target).expect("matched semantic utility key"),
                    arguments: Vec::new(),
                    steps: Vec::new(),
                    stops: Vec::new(),
                    guards: Vec::new(),
                }]))
            }
            AstMatcher::Call(call)
                if utility_plan_key(&call.util).is_some_and(|key| keys.contains(&key)) =>
            {
                Ok(Some(vec![RecursiveTransitionTemplate {
                    target: utility_plan_key(&call.util).expect("matched semantic utility key"),
                    arguments: call.args.clone(),
                    steps: Vec::new(),
                    stops: Vec::new(),
                    guards: Vec::new(),
                }]))
            }
            AstMatcher::Any(items) => {
                let mut templates = Vec::new();
                for item in items {
                    if let Some(mut item) =
                        Self::recursive_transition_templates(item, keys, discard_incompatible)?
                    {
                        templates.append(&mut item);
                    }
                }
                Ok((!templates.is_empty()).then_some(templates))
            }
            AstMatcher::All(items) => {
                let recursive = items
                    .iter()
                    .filter(|item| Self::recursive_references_component(item, keys))
                    .collect::<Vec<_>>();
                if recursive.is_empty() {
                    return Ok(None);
                }
                let mut templates = if recursive.len() == 1 {
                    Self::recursive_transition_templates(recursive[0], keys, discard_incompatible)?
                        .ok_or_else(|| "recursive utility operand has no transition".to_owned())?
                } else {
                    let operand_sets = recursive
                        .iter()
                        .map(|operand| {
                            Self::recursive_transition_templates(
                                operand,
                                keys,
                                discard_incompatible,
                            )?
                            .ok_or_else(|| "recursive utility operand has no transition".to_owned())
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    let mut combinations = vec![Vec::new()];
                    for operand in operand_sets {
                        let mut product = Vec::new();
                        for prefix in &combinations {
                            for template in &operand {
                                let mut combination = prefix.clone();
                                combination.push(template.clone());
                                product.push(combination);
                                if product.len() > 4_096 {
                                    return Err(
                                        "recursive utility transition expansion exceeds its limit"
                                            .to_owned(),
                                    );
                                }
                            }
                        }
                        combinations = product;
                    }
                    let mut output = Vec::new();
                    for mut operands in combinations {
                        let mut merged = operands.remove(0);
                        let mut root_constraints = Vec::new();
                        for operand in std::iter::once(&merged).chain(operands.iter()) {
                            for (_, guard) in operand
                                .guards
                                .iter()
                                .filter(|(level, _)| *level == operand.steps.len())
                            {
                                Self::recursive_guard_kind_sets(guard, &mut root_constraints);
                            }
                        }
                        let contradictory_root = {
                            let mut constraints = root_constraints.iter();
                            constraints.next().is_some_and(|first| {
                                let mut allowed = first.clone();
                                for constraint in constraints {
                                    allowed.retain(|kind| constraint.contains(kind));
                                }
                                allowed.is_empty()
                            })
                        };
                        let mut kinds_by_level = BTreeMap::<usize, Vec<BTreeSet<u16>>>::new();
                        if operands.iter().all(|operand| operand.steps == merged.steps) {
                            for (level, guard) in merged
                                .guards
                                .iter()
                                .chain(operands.iter().flat_map(|operand| operand.guards.iter()))
                            {
                                Self::recursive_guard_kind_sets(
                                    guard,
                                    kinds_by_level.entry(*level).or_default(),
                                );
                            }
                        }
                        let contradictory_level = kinds_by_level.values().any(|constraints| {
                            let mut constraints = constraints.iter();
                            let Some(first) = constraints.next() else {
                                return false;
                            };
                            let mut allowed = first.clone();
                            for constraint in constraints {
                                allowed.retain(|kind| constraint.contains(kind));
                            }
                            allowed.is_empty()
                        });
                        if contradictory_root || contradictory_level {
                            continue;
                        }
                        if operands.iter().any(|operand| {
                            operand.target != merged.target
                                || operand.arguments != merged.arguments
                                || operand.steps != merged.steps
                        }) {
                            if discard_incompatible {
                                continue;
                            }
                            return Err(format!(
                                "recursive utility conjunction has incompatible recursive paths (root kinds {root_constraints:?}, path lengths {:?})",
                                std::iter::once(&merged)
                                    .chain(operands.iter())
                                    .map(|operand| operand.steps.len())
                                    .collect::<Vec<_>>()
                            ));
                        }
                        for operand in operands {
                            merged.stops.extend(operand.stops);
                            merged.guards.extend(operand.guards);
                        }
                        output.push(merged);
                    }
                    output
                };
                let guards = items
                    .iter()
                    .filter(|item| !Self::recursive_references_component(item, keys))
                    .cloned()
                    .collect::<Vec<_>>();
                if !guards.is_empty() {
                    for template in &mut templates {
                        let level = template.steps.len();
                        template
                            .guards
                            .push((level, AstMatcher::All(guards.clone())));
                    }
                }
                Ok(Some(templates))
            }
            AstMatcher::Has(related, stop) => relation(
                related.rule(),
                NavigationAxis::Descendant,
                stop,
                None,
                false,
            ),
            AstMatcher::Inside(related, stop) => {
                relation(related.rule(), NavigationAxis::Ancestor, stop, None, false)
            }
            AstMatcher::Precedes(related, stop) => relation(
                related.rule(),
                NavigationAxis::FollowingSibling,
                stop,
                None,
                false,
            ),
            AstMatcher::Follows(related, stop) => relation(
                related.rule(),
                NavigationAxis::PrecedingSibling,
                stop,
                None,
                false,
            ),
            AstMatcher::Field(field) => relation(
                &field.rule,
                if field.inside {
                    NavigationAxis::Ancestor
                } else {
                    NavigationAxis::Descendant
                },
                &field.stop,
                Some(field.field),
                field.inside,
            ),
            _ if Self::recursive_references_component(rule, keys) => {
                Err("recursive utility reference is not on a supported structural path".to_owned())
            }
            _ => Ok(None),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_recursive_utility(
        &mut self,
        root: &UtilRef,
        call: Option<&UtilCall>,
        component: &str,
        graph: UtilityGraph,
        language: GrammarDialect,
        at: &str,
        terminals: &mut Vec<usize>,
        ir: &mut RelationalIr,
    ) {
        let mut ordered = graph
            .iter()
            .map(|(key, (reference, _, _))| {
                reference
                    .component_name()
                    .map(|name| (name, *key))
                    .ok_or_else(|| format!("{at}: recursive utility has no stable identity"))
            })
            .collect::<Result<Vec<_>, _>>();
        let Ok(ref mut ordered) = ordered else {
            self.unsupported.push(ordered.unwrap_err());
            return;
        };
        ordered.sort();
        let ids = ordered
            .iter()
            .enumerate()
            .map(|(id, (_, key))| (*key, id))
            .collect::<BTreeMap<_, _>>();
        let keys = ids.keys().copied().collect::<BTreeSet<_>>();
        let disjoint = graph
            .iter()
            .map(|(key, (_, rule, _))| {
                (*key, Self::recursive_alternatives_are_disjoint(rule, &keys))
            })
            .collect::<BTreeMap<_, _>>();
        let complemented = graph
            .iter()
            .map(|(key, (_, rule, _))| {
                Ok((*key, Self::recursive_complement_rule(rule, &keys, true)?))
            })
            .collect::<Result<BTreeMap<_, _>, String>>();
        let complement_validation = complemented
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|rules| {
                for (key, rule) in rules {
                    let alternatives = match rule {
                        AstMatcher::Any(items) => items.iter().collect::<Vec<_>>(),
                        other => vec![other],
                    };
                    for alternative in alternatives {
                        if Self::recursive_references_component(alternative, &keys)
                            && Self::recursive_transition_templates(
                                alternative,
                                &keys,
                                disjoint[key],
                            )?
                            .is_none()
                        {
                            return Err(
                                "complemented recursive operand has no transition".to_owned()
                            );
                        }
                    }
                }
                Ok(())
            });
        let complement = complement_validation.is_ok();
        let parent_depth = self.utility_stack.len();
        self.utility_stack.extend(keys.iter().copied());
        let errors_before = self.unsupported.len();
        let mut bases = Vec::new();
        let mut transitions = Vec::new();
        for (_, key) in ordered.iter() {
            let rule = if complement {
                &complemented.as_ref().expect("validated complement")[key]
            } else {
                &graph[key].1
            };
            let alternatives = match rule {
                AstMatcher::Any(items) => items.iter().collect::<Vec<_>>(),
                other => vec![other],
            };
            let mut component_bases = Vec::new();
            for alternative in alternatives {
                match Self::recursive_transition_templates(alternative, &keys, disjoint[key]) {
                    Ok(Some(templates)) => {
                        for template in templates {
                            let mut stops = Vec::new();
                            for (level, stop) in &template.stops {
                                let mut lowered = RelationalIr::empty();
                                self.lower_matcher(
                                    stop,
                                    language,
                                    &format!("{at}/utility/{component}/stopBy"),
                                    terminals,
                                    &mut lowered,
                                );
                                stops.push((*level, lowered));
                            }
                            let mut guards = Vec::new();
                            for (level, guard) in template.guards {
                                let mut lowered = RelationalIr::empty();
                                self.lower_matcher(
                                    &guard,
                                    language,
                                    &format!("{at}/utility/{component}/guard"),
                                    terminals,
                                    &mut lowered,
                                );
                                guards.push((level, lowered));
                            }
                            let transition = LoweredRecursiveTransition {
                                caller: ids[key],
                                target: ids[&template.target],
                                arguments: template.arguments,
                                steps: template.steps,
                                stops,
                                guards,
                            };
                            if !transitions.contains(&transition) {
                                transitions.push(transition);
                            }
                        }
                        continue;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let complement_error = complement_validation
                            .as_ref()
                            .err()
                            .map_or(String::new(), |reason| {
                                format!("; complemented form: {reason}")
                            });
                        self.unsupported.push(format!(
                            "{at}: recursive utility component `{component}`: {error}{complement_error}"
                        ));
                        continue;
                    }
                }
                let mut base = RelationalIr::empty();
                self.lower_matcher(
                    alternative,
                    language,
                    &format!("{at}/utility/{component}/base"),
                    terminals,
                    &mut base,
                );
                component_bases.push(base);
            }
            if !component_bases.is_empty() {
                let base = if component_bases.len() == 1 {
                    component_bases.pop().expect("one recursive utility base")
                } else {
                    match RelationalIr::combine(SqlOperation::OrderedUnion, component_bases) {
                        Ok(base) => base,
                        Err(error) => {
                            self.unsupported.push(format!("{at}: {error}"));
                            continue;
                        }
                    }
                };
                bases.push((ids[key], base));
            }
        }
        self.utility_stack.truncate(parent_depth);
        if self.unsupported.len() != errors_before {
            return;
        }
        let universe = self.universe_terminal(language);
        let navigation = self.navigation_terminal(language);
        for id in [universe, navigation] {
            if !terminals.contains(&id) {
                terminals.push(id);
            }
        }
        let transitions = transitions
            .into_iter()
            .map(|transition| RecursiveUtilityTransition {
                caller: transition.caller,
                target: transition.target,
                arguments: transition.arguments,
                steps: transition
                    .steps
                    .into_iter()
                    .map(|step| NavigationSpec {
                        axis: step.axis,
                        stop: step.stop,
                        field: step.field,
                        field_stop_children_only: step.field_stop_children_only,
                        producer: navigation,
                        enumerate: false,
                    })
                    .collect(),
                stops: transition.stops,
                guards: transition.guards,
            })
            .collect();
        let root_definition =
            utility_plan_key(root).expect("recursive utility has a semantic definition identity");
        let spec = RecursiveUtilitySpec {
            definition: root_definition,
            component: component.to_owned(),
            root: ids[&utility_plan_key(root).expect("recursive utility has a semantic identity")],
            complement,
            universe_producer: universe,
            navigation_producer: navigation,
            max_depth: MAX_UTIL_DEPTH,
            max_work: self.current_max_work,
            transitions,
        };
        let lowered = RelationalIr::recursive_utility(spec, bases).and_then(|recursive| {
            let Some(call) = call else {
                return Ok(recursive);
            };
            RelationalIr::utility_frame(
                UtilityFrameSpec {
                    utility: UtilitySpec {
                        definition: root_definition,
                        component: component.to_owned(),
                        universe_producer: universe,
                        depth: u32::try_from(parent_depth + 1).unwrap_or(u32::MAX),
                        max_depth: MAX_UTIL_DEPTH,
                        max_work: self.current_max_work,
                    },
                    arguments: call.args.clone(),
                },
                recursive,
            )
        });
        match lowered {
            Ok(lowered) => *ir = lowered,
            Err(error) => self.unsupported.push(format!("{at}: {error}")),
        }
    }

    fn lower_count(
        &mut self,
        count: &crate::code_rule_dsl::matcher::Count,
        language: GrammarDialect,
        at: &str,
        terminals: &mut Vec<usize>,
        ir: &mut RelationalIr,
    ) {
        let (axis, related, stop) = match &count.relation {
            AstMatcher::Inside(related, stop) => (NavigationAxis::Ancestor, related.rule(), stop),
            AstMatcher::Has(related, stop) => (NavigationAxis::Descendant, related.rule(), stop),
            AstMatcher::Precedes(related, stop) => {
                (NavigationAxis::FollowingSibling, related.rule(), stop)
            }
            AstMatcher::Follows(related, stop) => {
                (NavigationAxis::PrecedingSibling, related.rule(), stop)
            }
            _ => {
                self.unsupported
                    .push(format!("{at}/count: invalid relation"));
                return;
            }
        };
        let universe_id = self.universe_terminal(language);
        let navigation_id = self.navigation_terminal(language);
        for id in [universe_id, navigation_id] {
            if !terminals.contains(&id) {
                terminals.push(id);
            }
        }
        let mut source = RelationalIr::empty();
        source.terminal_spec(TerminalKind::Universe, format!("terminal:{universe_id}"));
        source.operation(SqlOperation::ScanTerminal);
        let mut related_ir = RelationalIr::empty();
        self.lower_matcher(
            related,
            language,
            &format!("{at}/count/rule"),
            terminals,
            &mut related_ir,
        );
        let (stop_kind, stop_ir) = match stop {
            StopBy::Neighbor => (NavigationStop::Neighbor, None),
            StopBy::End => (NavigationStop::End, None),
            StopBy::Rule(rule) => {
                let mut lowered = RelationalIr::empty();
                self.lower_matcher(
                    rule,
                    language,
                    &format!("{at}/count/stopBy"),
                    terminals,
                    &mut lowered,
                );
                (NavigationStop::Rule, Some(lowered))
            }
        };
        let binding = count
            .binding
            .as_ref()
            .map(|binding| (binding.name.clone(), binding.offset));
        match RelationalIr::count(
            CountSpec {
                navigation: NavigationSpec {
                    axis,
                    stop: stop_kind,
                    field: None,
                    field_stop_children_only: false,
                    producer: navigation_id,
                    enumerate: false,
                },
                minimum: count.min,
                maximum: count.max,
                binding,
            },
            source,
            related_ir,
            stop_ir,
        ) {
            Ok(lowered) => *ir = lowered,
            Err(error) => self.unsupported.push(format!("{at}/count: {error}")),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_navigation(
        &mut self,
        axis: NavigationAxis,
        related: &AstMatcher,
        stop: &StopBy,
        field: Option<u16>,
        field_stop_children_only: bool,
        language: GrammarDialect,
        at: &str,
        terminals: &mut Vec<usize>,
        ir: &mut RelationalIr,
    ) {
        let universe_id = self.universe_terminal(language);
        let navigation_id = self.navigation_terminal(language);
        for id in [universe_id, navigation_id] {
            if !terminals.contains(&id) {
                terminals.push(id);
            }
        }
        let mut source = RelationalIr::empty();
        source.terminal_spec(TerminalKind::Universe, format!("terminal:{universe_id}"));
        source.operation(SqlOperation::ScanTerminal);
        let mut related_ir = RelationalIr::empty();
        self.lower_matcher(
            related,
            language,
            &format!("{at}/rule"),
            terminals,
            &mut related_ir,
        );
        let (stop_kind, stop_ir) = match stop {
            StopBy::Neighbor => (NavigationStop::Neighbor, None),
            StopBy::End => (NavigationStop::End, None),
            StopBy::Rule(rule) => {
                let mut lowered = RelationalIr::empty();
                self.lower_matcher(
                    rule,
                    language,
                    &format!("{at}/stopBy"),
                    terminals,
                    &mut lowered,
                );
                (NavigationStop::Rule, Some(lowered))
            }
        };
        match RelationalIr::navigate(
            NavigationSpec {
                axis,
                stop: stop_kind,
                field,
                field_stop_children_only,
                producer: navigation_id,
                enumerate: self.current_each.is_some(),
            },
            source,
            related_ir,
            stop_ir,
        ) {
            Ok(lowered) => *ir = lowered,
            Err(error) => self.unsupported.push(format!("{at}: {error}")),
        }
    }

    fn navigation_terminal(&mut self, language: GrammarDialect) -> usize {
        self.intern_terminal(
            TerminalIdentity::Navigation(language),
            TerminalKind::Navigation,
            language,
            Program::compile_navigation().expect("the navigation producer is verified"),
        )
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
            if language.kind_is_supertype(kind) {
                Program::compile_supertype(kind)
            } else {
                Program::compile_kind(kind)
            }
            .expect("a kind terminal is a verified program"),
        )
    }

    fn value_terminal(&mut self, at: &str, descriptor: ValueFactDescriptor) -> usize {
        let identity = TerminalIdentity::Value(
            descriptor.language,
            descriptor.follow,
            descriptor.expression_kinds.clone(),
            at.to_owned(),
        );
        if let Some(id) = self.terminal_ids.get(&identity).copied() {
            return id;
        }
        let id = self.terminals.len();
        self.terminals.push(TerminalPlan {
            kind: TerminalKind::Value,
            language: descriptor.language,
            program: Program::compile_universe().expect("the value fact shell is verified"),
            clauses: vec![self.current_clause],
            result_limit: u32::MAX,
            value_fact: Some(descriptor),
            semantic_fact: None,
            parse_fact: None,
            transform_fact: None,
            file_fact: None,
            sound_under_errors: true,
            file_conditions: Vec::new(),
        });
        self.terminal_ids.insert(identity, id);
        id
    }

    /// P's projection forms this renderer does not lower (a capture's position, kind, line or text,
    /// and `extract`): the plan is refused rather than lowered without them.
    fn refuse_projections(&mut self, relations: &[&FactQuery], at: &str) {
        for item in relations.iter().flat_map(|relation| relation.project.iter()) {
            if item.extract.is_some()
                || matches!(
                    item.source,
                    ProjectionSource::CaptureStart(_)
                        | ProjectionSource::CaptureEnd(_)
                        | ProjectionSource::CaptureKind(_)
                        | ProjectionSource::CaptureLine(_)
                        | ProjectionSource::CaptureText(_)
                )
            {
                self.unsupported
                    .push(format!("{at}: projection `{}` is not lowered", item.field));
            }
        }
    }

    /// P's regex options (match, content and line filters, paths) are not lowered: a query with any
    /// is refused rather than lowered without them.
    fn refuse_regex_options(&mut self, options: &RegexOptions, at: &str) {
        if !(options.include.is_empty()
            && options.exclude.is_empty()
            && options.content_include.is_empty()
            && options.content_exclude.is_empty()
            && options.lines.is_none()
            && options.paths.is_none())
        {
            self.unsupported
                .push(format!("{at}: regex options are not lowered"));
        }
    }

    fn projection_sql(source: &ProjectionSource, row: &str) -> String {
        match source {
            ProjectionSource::Capture(label) => format!(
                "(SELECT json_extract(binding.value, '$.text') FROM json_each({row}.bindings) binding WHERE json_extract(binding.value, '$.label') = '{}')",
                label.replace('\'', "''")
            ),
            ProjectionSource::Path
            | ProjectionSource::Directory
            | ProjectionSource::Name
            | ProjectionSource::Stem
            | ProjectionSource::Module
            | ProjectionSource::Package => format!("CAST({row}.file_index AS TEXT)"),
            // Refused by `refuse_projections`.
            ProjectionSource::CaptureStart(_)
            | ProjectionSource::CaptureEnd(_)
            | ProjectionSource::CaptureKind(_)
            | ProjectionSource::CaptureLine(_)
            | ProjectionSource::CaptureText(_) => "NULL".to_owned(),
        }
    }

    fn lower_clause_query(&mut self, query: &Query, at: &str) -> (Vec<usize>, RelationalIr) {
        match query {
            Query::Regex {
                regex,
                source,
                options,
            } => {
                self.refuse_regex_options(options, at);
                let id = self.file_terminal(regex, *source);
                let mut ir = RelationalIr::empty();
                ir.terminal_spec(TerminalKind::File, format!("terminal:{id}"));
                ir.operation(SqlOperation::ScanTerminal);
                (vec![id], ir)
            }
            Query::Ast { matchers, calls } => {
                self.current_calls = calls
                    .iter()
                    .map(|term| (Arc::clone(&term.key), Arc::clone(term)))
                    .collect();
                let mut terminals = Vec::new();
                let mut relations = Vec::new();
                for matcher in matchers.iter().filter(|matcher| matcher.applies) {
                    let mut ir = RelationalIr::empty();
                    let mut matcher_terminals = Vec::new();
                    self.lower_matcher(
                        &matcher.matcher,
                        matcher.language,
                        at,
                        &mut matcher_terminals,
                        &mut ir,
                    );
                    for producer in &matcher_terminals {
                        if self.terminals[*producer].kind == TerminalKind::Universe {
                            self.terminals[*producer].sound_under_errors = false;
                        }
                        self.terminals[*producer]
                            .file_conditions
                            .push(FileApplicability {
                                condition: matcher.file.clone(),
                                required: matcher.text.clone(),
                            });
                    }
                    terminals.extend(matcher_terminals);
                    relations.push(ir);
                }
                let ir = if relations.is_empty() {
                    RelationalIr::empty_relation()
                } else if relations.len() == 1 {
                    relations.pop().expect("one relation")
                } else {
                    RelationalIr::union_all(relations).unwrap_or_else(|error| {
                        self.unsupported.push(format!("{at}: {error}"));
                        RelationalIr::empty_relation()
                    })
                };
                terminals.sort_unstable();
                terminals.dedup();
                (terminals, ir)
            }
            Query::SelectWhen { select, exists } => {
                let (mut terminals, select) =
                    self.lower_clause_query(select, &format!("{at}/select"));
                let (more, exists) = self.lower_clause_query(exists, &format!("{at}/exists"));
                terminals.extend(more);
                terminals.sort_unstable();
                terminals.dedup();
                let ir = RelationalIr::select_when(select, exists).unwrap_or_else(|error| {
                    self.unsupported.push(format!("{at}: {error}"));
                    RelationalIr::empty_relation()
                });
                (terminals, ir)
            }
        }
    }

    fn transform_terminal(
        &mut self,
        language: GrammarDialect,
        at: &str,
        transform: Arc<Transform>,
        outputs: Vec<(String, String)>,
        sources: Vec<usize>,
    ) -> usize {
        let identity = TerminalIdentity::Transform(language, at.to_owned());
        if let Some(id) = self.terminal_ids.get(&identity).copied() {
            return id;
        }
        let id = self.terminals.len();
        self.terminals.push(TerminalPlan {
            kind: TerminalKind::Transform,
            language,
            program: Program::compile_navigation().expect("the transform fact shell is verified"),
            clauses: vec![self.current_clause],
            result_limit: u32::try_from(self.current_max_work).unwrap_or(u32::MAX),
            value_fact: None,
            semantic_fact: None,
            parse_fact: None,
            transform_fact: Some(TransformFactDescriptor {
                transform,
                outputs,
                sources,
            }),
            file_fact: None,
            sound_under_errors: true,
            file_conditions: Vec::new(),
        });
        self.terminal_ids.insert(identity, id);
        id
    }

    fn file_terminal(&mut self, regex: &regex::Regex, source: RegexSource) -> usize {
        let identity = TerminalIdentity::File(source, regex.as_str().to_owned());
        if let Some(id) = self.terminal_ids.get(&identity).copied() {
            return id;
        }
        let id = self.terminals.len();
        self.terminals.push(TerminalPlan {
            kind: TerminalKind::File,
            language: GrammarDialect::Python,
            program: Program::compile_universe().expect("the file fact shell is verified"),
            clauses: vec![self.current_clause],
            result_limit: 1,
            value_fact: None,
            semantic_fact: None,
            parse_fact: None,
            transform_fact: None,
            file_fact: Some(FileFactDescriptor {
                regex: regex.clone(),
                path: source == RegexSource::Path,
            }),
            sound_under_errors: true,
            file_conditions: Vec::new(),
        });
        self.terminal_ids.insert(identity, id);
        id
    }

    fn parse_terminal(
        &mut self,
        language: GrammarDialect,
        at: &str,
        descriptor: ParseFactDescriptor,
    ) -> usize {
        let identity = TerminalIdentity::Parse(language, at.to_owned());
        let id = self.terminals.len();
        self.terminals.push(TerminalPlan {
            kind: TerminalKind::Parse,
            language,
            program: Program::compile_universe().expect("the parse fact shell is verified"),
            clauses: vec![self.current_clause],
            result_limit: u32::try_from(self.current_max_work).unwrap_or(u32::MAX),
            value_fact: None,
            semantic_fact: None,
            parse_fact: Some(descriptor),
            transform_fact: None,
            file_fact: None,
            sound_under_errors: true,
            file_conditions: Vec::new(),
        });
        self.terminal_ids.insert(identity, id);
        id
    }

    fn fact_terminal(
        &mut self,
        language: GrammarDialect,
        at: &str,
        source: SemanticFactSource,
        input_labels: Vec<String>,
        call_through: Option<crate::code_rule_dsl::calls::Through>,
    ) -> usize {
        let identity = TerminalIdentity::Fact(language, at.to_owned());
        let id = self.terminals.len();
        self.terminals.push(TerminalPlan {
            kind: TerminalKind::Fact,
            language,
            program: Program::compile_universe().expect("the semantic fact shell is verified"),
            clauses: vec![self.current_clause],
            result_limit: u32::try_from(self.current_max_work).unwrap_or(u32::MAX),
            value_fact: None,
            semantic_fact: Some(SemanticFactDescriptor {
                source,
                input_labels,
                call_through,
            }),
            parse_fact: None,
            transform_fact: None,
            file_fact: None,
            sound_under_errors: true,
            file_conditions: Vec::new(),
        });
        self.terminal_ids.insert(identity, id);
        id
    }

    fn mark_terminal_required(&mut self, id: usize) {
        let terminal = &mut self.terminals[id];
        if !terminal.clauses.contains(&self.current_clause) {
            terminal.clauses.push(self.current_clause);
        }
        terminal.result_limit = terminal.result_limit.max(self.current_result_limit);
    }

    fn intern_terminal(
        &mut self,
        identity: TerminalIdentity,
        kind: TerminalKind,
        language: GrammarDialect,
        program: Program,
    ) -> usize {
        if let Some(id) = self.terminal_ids.get(&identity).copied() {
            self.mark_terminal_required(id);
            return id;
        }
        let id = self.terminals.len();
        self.terminals.push(TerminalPlan {
            kind,
            language,
            program,
            clauses: vec![self.current_clause],
            result_limit: self.current_result_limit,
            value_fact: None,
            semantic_fact: None,
            parse_fact: None,
            transform_fact: None,
            file_fact: None,
            sound_under_errors: true,
            file_conditions: Vec::new(),
        });
        self.terminal_ids.insert(identity, id);
        id
    }

    #[cfg(test)]
    fn template_is_var(
        template: Option<&crate::code_rule_dsl::model::Template>,
        name: &str,
    ) -> bool {
        let Some(template) = template else {
            return false;
        };
        matches!(
            template.parts().collect::<Vec<_>>().as_slice(),
            [TemplatePart::Var { name: actual, multi: false }] if *actual == name
        )
    }

    #[cfg(test)]
    fn template_is_text_var(
        template: Option<&crate::code_rule_dsl::model::Template>,
        text: &str,
        name: &str,
    ) -> bool {
        let Some(template) = template else {
            return false;
        };
        matches!(
            template.parts().collect::<Vec<_>>().as_slice(),
            [TemplatePart::Text(actual_text), TemplatePart::Var { name: actual_name, multi: false }]
                if *actual_text == text && *actual_name == name
        )
    }

    #[cfg(test)]
    fn template_is_text_var_text(
        template: Option<&crate::code_rule_dsl::model::Template>,
        before: &str,
        name: &str,
        after: &str,
    ) -> bool {
        let Some(template) = template else {
            return false;
        };
        matches!(
            template.parts().collect::<Vec<_>>().as_slice(),
            [TemplatePart::Text(actual_before), TemplatePart::Var { name: actual_name, multi: false }, TemplatePart::Text(actual_after)]
                if *actual_before == before && *actual_name == name && *actual_after == after
        )
    }

    #[cfg(test)]
    fn authored_fixture_shape(&self) -> Option<super::compact_output_sql::Shape> {
        use super::compact_output_sql::Shape;

        let [ir] = self.clause_irs.as_slice() else {
            return None;
        };
        let [output] = self.clause_outputs.as_slice() else {
            return None;
        };
        if self.clause_terminal_ids.len() != 1 {
            return None;
        }
        let kinds = self.clause_terminal_ids[0]
            .iter()
            .map(|id| self.terminals[*id].kind())
            .collect::<Vec<_>>();
        let capture_a = matches!(&output.violation.anchor, Anchor::Capture(name) if name == "A");
        let message_a = Self::template_is_var(output.violation.message_template.as_ref(), "A");
        let violation = &output.violation;
        let bounded_output = violation.node_regexes.is_empty()
            && violation.group_transform.is_none()
            && violation.project.is_empty();
        let basic_pattern = ir.operations()
            == [
                SqlOperation::PackedMaterialized,
                SqlOperation::StatusJoin,
                SqlOperation::ScanTerminal,
                SqlOperation::ScanCapture,
            ];
        if kinds == [TerminalKind::Pattern] && basic_pattern && bounded_output {
            if output.violation.suppress.as_ref().is_some_and(|suppress| {
                suppress.eslint.is_none()
                    && suppress.nosec.is_none()
                    && suppress.phpstan.is_none()
                    && suppress.psalm.is_none()
                    && suppress.phpcs.is_none()
                    && suppress
                        .noqa
                        .as_ref()
                        .is_some_and(|noqa| noqa.codes == ["F401"] && !noqa.parent)
            }) && output.violation.code == "F401"
                && capture_a
                && message_a
                && output.violation.fix_range.is_none()
                && output.violation.fix.is_some()
                && output.violation.each.is_none()
                && output.transform.is_none()
                && Self::template_is_text_var_text(
                    output.violation.fix_template.as_ref(),
                    "log(",
                    "A",
                    ")",
                )
            {
                return Some(Shape::OutputFixAndSuppression);
            }
            if capture_a
                && output.violation.fix_range.as_deref() == Some("A")
                && output.violation.fix.is_some()
                && output.violation.each.is_none()
                && output.violation.suppress.is_none()
                && output.transform.is_none()
                && Self::template_is_text_var(
                    output.violation.message_template.as_ref(),
                    "replace ",
                    "A",
                )
                && Self::template_is_var(output.violation.fix_template.as_ref(), "B")
            {
                return Some(Shape::CaptureFixRange);
            }
            if capture_a
                && output.violation.fix_range.is_none()
                && output.violation.fix.is_none()
                && output.violation.fix_template.is_none()
                && output.violation.each.is_none()
                && output.violation.suppress.is_none()
                && Self::template_is_var(output.violation.message_template.as_ref(), "LOWER")
                && output.transform.as_ref().is_some_and(|transform| {
                    matches!(
                        transform.simple_case("LOWER"),
                        Some(("A", StringCase::LowerCase))
                    )
                })
            {
                return Some(Shape::CaptureTransform);
            }
            if output.violation.anchor == Anchor::Span
                && output.violation.fix_range.is_none()
                && output.violation.fix.is_none()
                && output.violation.fix_template.is_none()
                && output.violation.each.is_none()
                && output.violation.suppress.is_none()
                && output.transform.is_none()
                && Self::template_is_text_var(
                    output.violation.message_template.as_ref(),
                    "found ",
                    "A",
                )
            {
                return Some(Shape::CaptureMessageTemplate);
            }
            if capture_a
                && message_a
                && output.violation.each.is_none()
                && output.violation.fix_range.is_none()
                && output.violation.fix.is_none()
                && output.violation.fix_template.is_none()
                && output.violation.suppress.is_none()
                && output.transform.is_none()
            {
                return Some(Shape::CaptureAnchor);
            }
        }
        if kinds == [TerminalKind::Pattern, TerminalKind::Pattern]
            && bounded_output
            && ir.operations()
                == [
                    SqlOperation::PackedMaterialized,
                    SqlOperation::StatusJoin,
                    SqlOperation::ScanTerminal,
                    SqlOperation::ScanCapture,
                    SqlOperation::OrderedUnion,
                    SqlOperation::WindowRank,
                    SqlOperation::CaseTruth,
                ]
            && capture_a
            && output.violation.message == "optional capture"
            && output.violation.message_template.is_none()
            && output.violation.fix_range.is_none()
            && output.violation.fix.is_none()
            && output.violation.fix_template.is_none()
            && output.violation.each.is_none()
            && output.violation.suppress.is_none()
            && output.transform.is_none()
        {
            return Some(Shape::CaptureAnchorFallback);
        }
        if kinds
            == [
                TerminalKind::Kind,
                TerminalKind::Universe,
                TerminalKind::Navigation,
                TerminalKind::Pattern,
            ]
            && bounded_output
            && capture_a
            && message_a
            && output.violation.fix_range.is_none()
            && output.violation.fix.is_none()
            && output.violation.fix_template.is_none()
            && output.violation.suppress.is_none()
            && output.transform.is_none()
            && output.violation.each.as_deref() == Some("A")
        {
            let distinct = [
                SqlOperation::PackedMaterialized,
                SqlOperation::StatusJoin,
                SqlOperation::ScanTerminal,
                SqlOperation::InnerJoin,
                SqlOperation::ScanCapture,
                SqlOperation::ScanNavigation,
                SqlOperation::CaseTruth,
            ];
            let stable = [
                SqlOperation::PackedMaterialized,
                SqlOperation::StatusJoin,
                SqlOperation::ScanTerminal,
                SqlOperation::InnerJoin,
                SqlOperation::ScanCapture,
                SqlOperation::ScanNavigation,
                SqlOperation::OrderedUnion,
                SqlOperation::WindowRank,
                SqlOperation::CaseTruth,
            ];
            if ir.operations() == distinct {
                return Some(Shape::CaptureEachDistinct);
            }
            if ir.operations() == stable {
                return Some(Shape::OutputStableMultiplicity);
            }
        }
        None
    }

    fn finish(mut self) -> Result<SqlCatalogPlan, LoweringError> {
        for ir in &mut self.clause_irs {
            ir.refresh_explanation();
            ir.operation(SqlOperation::Projection);
            ir.operation(SqlOperation::OrderBy);
        }
        let mut lowering_variants = BTreeMap::new();
        let clause_logical_plans = self
            .clause_outputs
            .iter()
            .enumerate()
            .map(|(ordinal, output)| {
                let source = self
                    .clause_source_starts
                    .iter()
                    .rev()
                    .find(|(start, _)| *start <= ordinal)
                    .map(|(_, source)| *source)
                    .expect("every lowered clause output has a source clause");
                let variant = lowering_variants
                    .entry((output.rule_id.clone(), output.clause_id.clone()))
                    .or_insert(0_u64);
                let lowering_variant = *variant;
                *variant = variant
                    .checked_add(1)
                    .expect("clause lowering variant count fits u64");
                super::plan::ClauseLogicalPlan::new(
                    ordinal,
                    source,
                    &output.rule_id,
                    &output.clause_id,
                    clause_effect_profile(&self.clause_irs[ordinal]),
                    lowering_variant,
                    self.clause_irs[ordinal].utility_definitions(),
                )
            })
            .collect();
        #[cfg(test)]
        let authored_fixture_sql = self.authored_fixture_shape().map(|shape| {
            let output = &self.clause_outputs[0];
            super::compact_output_sql::render(
                shape,
                &output.rule_id,
                &output.clause_id,
                &output.violation.code,
                output.mode.as_str(),
            )
        });
        Ok(SqlCatalogPlan {
            terminals: self.terminals,
            clause_irs: self.clause_irs,
            clause_terminal_ids: self.clause_terminal_ids,
            clause_each: self.clause_each,
            clause_outputs: self.clause_outputs,
            clause_logical_plans,
            utility_call_counts: self.utility_call_counts,
            #[cfg(test)]
            authored_fixture_sql,
            #[cfg(test)]
            all_execution_sql: std::sync::OnceLock::new(),
            #[cfg(test)]
            all_relational_sql: std::sync::OnceLock::new(),
        })
    }
}
