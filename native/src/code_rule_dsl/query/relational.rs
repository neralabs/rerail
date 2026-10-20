use super::relational_ir::{
    ComparisonSpec, CountSpec, NavigationAxis, NavigationSpec, NavigationStop, NthChildSpec,
    ParseFactSpec, RecursiveUtilitySpec, RecursiveUtilityTransition, RelationalIr,
    SemanticFactSpec, SemanticRuleSpec, SqlOperation, UtilityFrameSpec, UtilitySpec, ValueLiteral,
    ValuePredicateSpec, qualified_failure_columns, qualified_row_columns,
};
use crate::code_rule_dsl::language::{GrammarDialect, label};
use crate::code_rule_dsl::matcher::{
    AstMatcher, MAX_UTIL_DEPTH, Semantic, StopBy, UtilCall, UtilRef,
};
use crate::code_rule_dsl::model::{ClauseKind, CompiledCodeRule, Query};
use crate::code_rule_dsl::packed::Program;
use crate::code_rule_dsl::transducer::sql_backend::{
    ParseFactDescriptor, ProducerKind, SemanticFactDescriptor, SemanticFactSource,
    ValueFactDescriptor,
};
use crate::code_rule_dsl::value::Value;
use ast_grep_core::matcher::Matcher;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
    value_fact: Option<ValueFactDescriptor>,
    semantic_fact: Option<SemanticFactDescriptor>,
    parse_fact: Option<ParseFactDescriptor>,
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
                let _ = calls;
                for matcher in matchers {
                    let language = label(matcher.language).unwrap_or("unknown");
                    let at = format!("{clause_at}/{language}/rule");
                    let mut terminals = Vec::new();
                    builder.current_clause = (rule_index, clause_index);
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

    pub(super) fn requires_document_maps(&self) -> bool {
        self.clause_irs
            .iter()
            .any(|ir| ir.operations().contains(&SqlOperation::MapDocument))
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
    Navigation(GrammarDialect),
    Kind(GrammarDialect, u16),
    Pattern(GrammarDialect, String),
    Regex(GrammarDialect, String),
    Value(GrammarDialect, bool, Vec<u16>, String),
    Fact(GrammarDialect, String),
    Parse(GrammarDialect, String),
}

type UtilityGraph = BTreeMap<usize, (UtilRef, AstMatcher, Vec<usize>)>;

#[derive(Default)]
struct Builder {
    terminals: Vec<TerminalPlan>,
    terminal_ids: HashMap<TerminalIdentity, usize>,
    clause_irs: Vec<RelationalIr>,
    clause_terminal_ids: Vec<Vec<usize>>,
    unsupported: Vec<String>,
    current_clause: (usize, usize),
    current_result_limit: u32,
    current_max_work: usize,
    utility_stack: Vec<usize>,
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
                call_through = Some(term.through);
            }
            Semantic::Resolves(_) | Semantic::Arguments(_) | Semantic::Layout(_) => {}
            Semantic::Parse(_) => unreachable!("parse returned before semantic fact lowering"),
            Semantic::Value(_) | Semantic::Host(_) | Semantic::Capture(_) => {
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
            Semantic::Value(_) | Semantic::Parse(_) | Semantic::Host(_) | Semantic::Capture(_) => {
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
                if comparison.transform.is_some() || !comparison.commute.is_empty() {
                    self.unsupported.push(format!(
                        "{at}: transformed or commuted comparison is not lowered"
                    ));
                    return;
                }
                *ir = RelationalIr::comparison(ComparisonSpec {
                    left: comparison.left.clone(),
                    op: comparison.op,
                    right: comparison.right.clone(),
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
                let Some(expected) = test.eq.as_ref() else {
                    self.unsupported
                        .push(format!("{at}: value predicate has no SQL comparison"));
                    return;
                };
                if unsupported_value_shape {
                    self.unsupported.push(format!(
                        "{at}: compound value predicate is not lowered in phase two"
                    ));
                    return;
                }
                let expected = match expected {
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
                };
                let descriptor = ValueFactDescriptor {
                    language: test.reading.language,
                    follow: test.reading.follow,
                    expression_kinds: test.reading.expressions.iter().copied().collect(),
                    reading: test.reading.clone(),
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
        let key = util.key();
        let Some(component) = util.component_name() else {
            self.unsupported
                .push(format!("{at}: utility component has no stable identity"));
            return;
        };
        if self.utility_stack.is_empty()
            && let Some(graph) = Self::recursive_component(&util)
        {
            self.lower_recursive_utility(
                &util, call, &component, graph, language, at, terminals, ir,
            );
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
        let Some(rule) = util.with_target(Clone::clone) else {
            self.unsupported.push(format!(
                "{at}: utility component `{component}` is unavailable"
            ));
            return;
        };
        let depth = u32::try_from(self.utility_stack.len() + 1).unwrap_or(u32::MAX);
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
        let universe = self.universe_terminal(language);
        if !terminals.contains(&universe) {
            terminals.push(universe);
        }
        let utility = UtilitySpec {
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
            active: &mut BTreeSet<usize>,
            completed: &mut BTreeMap<usize, u32>,
        ) -> Option<u32> {
            let key = util.key();
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
        fn collect(util: UtilRef, graph: &mut UtilityGraph) {
            let key = util.key();
            if graph.contains_key(&key) {
                return;
            }
            let Some(rule) = util.with_target(Clone::clone) else {
                return;
            };
            let mut references = Vec::new();
            utility_references(&rule, &mut references);
            let dependencies = references.iter().map(UtilRef::key).collect::<Vec<_>>();
            graph.insert(key, (util, rule, dependencies));
            for reference in references {
                collect(reference, graph);
            }
        }
        fn reaches(
            graph: &UtilityGraph,
            from: usize,
            target: usize,
            seen: &mut BTreeSet<usize>,
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
        collect(root.clone(), &mut graph);
        let root_key = root.key();
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
        if call.is_some() {
            self.unsupported.push(format!(
                "{at}: parameterized recursive utility component `{component}` is not lowered"
            ));
            return;
        }
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
        self.utility_stack.extend(keys.iter().copied());
        let errors_before = self.unsupported.len();
        let mut bases = Vec::new();
        let mut transitions = Vec::new();
        for (_, key) in ordered.iter() {
            let (_, rule, _) = &graph[key];
            let alternatives = match rule {
                AstMatcher::Any(items) => items.iter().collect::<Vec<_>>(),
                other => vec![other],
            };
            let mut component_bases = Vec::new();
            for alternative in alternatives {
                let recursive = match alternative {
                    AstMatcher::Has(related, stop) => match related.rule() {
                        AstMatcher::Util(target) if keys.contains(&target.key()) => Some((
                            target.key(),
                            matches!(stop, StopBy::Neighbor),
                            matches!(stop, StopBy::Neighbor | StopBy::End),
                        )),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some((target, neighbor, supported)) = recursive {
                    if !supported {
                        self.unsupported.push(format!(
                            "{at}: recursive utility stop rule is not a bounded SQL transition"
                        ));
                    } else {
                        transitions.push(RecursiveUtilityTransition {
                            caller: ids[key],
                            target: ids[&target],
                            neighbor,
                        });
                    }
                    continue;
                }
                let mut references = Vec::new();
                utility_references(alternative, &mut references);
                if references
                    .iter()
                    .any(|reference| keys.contains(&reference.key()))
                {
                    self.unsupported.push(format!(
                        "{at}: recursive utility reference must cross a `has` relation"
                    ));
                    continue;
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
        self.utility_stack.clear();
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
        let spec = RecursiveUtilitySpec {
            component: component.to_owned(),
            root: ids[&root.key()],
            universe_producer: universe,
            navigation_producer: navigation,
            max_depth: MAX_UTIL_DEPTH,
            max_work: self.current_max_work,
            transitions,
        };
        match RelationalIr::recursive_utility(spec, bases) {
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
            Program::compile_kind(kind).expect("a kind terminal is a verified program"),
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
        });
        self.terminal_ids.insert(identity, id);
        id
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
            value_fact: None,
            semantic_fact: None,
            parse_fact: None,
        });
        self.terminal_ids.insert(identity, id);
        id
    }

    fn finish(self) -> Result<SqlCatalogPlan, LoweringError> {
        let mut clause_irs = self.clause_irs;
        let recursive = clause_irs.iter().any(RelationalIr::requires_recursive_cte);
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
            let failure_projection = qualified_failure_columns("failed");
            let producers = self.clause_terminal_ids[ordinal]
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>();
            let failure_union = if producers.is_empty() {
                String::new()
            } else {
                format!(
                    "\n  UNION ALL\n  SELECT {failure_projection}, json('[]') AS bindings, '' AS choice_order, {ordinal} AS clause_ordinal\n  FROM producer_status failed\n  WHERE failed.producer_id IN ({}) AND failed.complete <> 1",
                    producers.join(", ")
                )
            };
            writeln!(
                clauses,
                ",\n{relations},\nclause_{ordinal} AS (\n  SELECT {projection}, result.bindings, result.choice_order, {ordinal} AS clause_ordinal FROM {result} result\n  CROSS JOIN batch_status batch\n  WHERE batch.complete = 1\n    AND NOT EXISTS (\n      SELECT 1 FROM producer_status undecided\n      WHERE undecided.complete <> 1 OR undecided.verdict IS NULL\n    ){failure_union}\n)"
            )
            .expect("write SQL");
        }
        let with = if recursive { "WITH RECURSIVE" } else { "WITH" };
        let clause_union = (0..clause_irs.len())
            .map(|ordinal| format!("SELECT * FROM clause_{ordinal}"))
            .collect::<Vec<_>>()
            .join(" UNION ALL ");
        let clause_union = if clause_irs.len() == 1 {
            clause_union
        } else {
            format!("SELECT * FROM ({clause_union})")
        };
        let sql = format!(
            "{with} packed AS MATERIALIZED (\n  SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE batch = :batch\n),\nbatch_status AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 1\n),\nproducer_status AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 2\n),\nterminal_match AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 3\n),\ncapture AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 4\n),\nnavigation AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 5\n),\nfact AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 6\n),\ndocument_map AS MATERIALIZED (\n  SELECT * FROM packed WHERE row_kind = 7\n)\n{clauses}\n{clause_union}\nORDER BY clause_ordinal, file_index, node, choice_order, producer_id, match_index, capture_index"
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
