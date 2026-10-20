//! SQL-demanded packed batch relation.
//!
//! Registration stores immutable verified programs, tapes, limits, and a sparse task grid. SQLite
//! calls `xFilter` with the batch token. The first call executes the packed backend and stores its
//! normalized relation. No host callback or semantic matcher can be registered here.

use super::super::language::GrammarDialect;
use super::super::matcher::Reading;
use super::super::packed::{
    Error as PackedError, ExecutionBackend, Program, execute_with_backend_packed_result_limits,
};
use super::super::tape::TapeDoc;
use super::super::value::Lang;
use ast_grep_core::AstGrep;
use num_traits::FromPrimitive as _;
use rusqlite::ffi;
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
    VTabCursor,
};
use rusqlite::{Connection, Result as SqlResult};
use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const MODULE_NAME: &CStr = c"rerail_matches";
const BATCH_COLUMN: c_int = 0;
const USE_BATCH: c_int = 1;

/// Everything `xFilter` needs to execute one sparse packed batch. Construction verifies programs
/// before registration; this value contains no callback and cannot enter the semantic matcher.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum ProducerKind {
    Terminal,
    Pattern,
    Navigation,
    ValueFact,
    SemanticFact,
    ParseFact,
}

/// A bounded parsed-document request. It owns grammar and host selection only. Nested user rules
/// remain relational SQL operands and are not stored in this descriptor.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct ParseFactDescriptor {
    pub(in crate::code_rule_dsl) grammar: super::super::literal::Grammar,
    pub(in crate::code_rule_dsl) host_kinds: Arc<std::collections::BTreeSet<u16>>,
    pub(in crate::code_rule_dsl) reading: super::super::scope::Reading,
}

impl std::fmt::Debug for ParseFactDescriptor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParseFactDescriptor")
            .finish_non_exhaustive()
    }
}

/// A bounded semantic-fact request. The descriptor emits only predicate-local rows. Nested user
/// rules remain relational operands and are never executed by this producer.
#[derive(Clone)]
pub(in crate::code_rule_dsl) enum SemanticFactSource {
    Resolves(super::super::matcher::Resolves),
    Binding {
        test: Box<super::super::matcher::BindingTest>,
        request: super::semantic_facts::BindingRequest,
    },
    Binds {
        test: Box<super::super::matcher::Binds>,
        request: super::semantic_facts::BindsRequest,
    },
    Arguments(super::super::matcher::Arguments),
    FormatArgs(super::super::matcher::FormatArgs),
    Layout(super::super::matcher::Layout),
    Calls(super::super::calls::CallsTest),
}

impl SemanticFactSource {
    pub(in crate::code_rule_dsl) fn has_nested_user_rule(&self) -> bool {
        match self {
            Self::Binding { test, .. } => {
                test.site.is_some()
                    || test.value.is_some()
                    || test.annotation.is_some()
                    || test.occurrence.is_some()
            }
            Self::Binds { test, .. } => {
                test.site.is_some()
                    || test.value.is_some()
                    || test.annotation.is_some()
                    || test.occurrence.is_some()
                    || test.redefines.as_ref().is_some_and(|redefines| {
                        redefines.site.is_some()
                            || redefines.value.is_some()
                            || redefines.annotation.is_some()
                    })
            }
            Self::FormatArgs(test) => test.unused.is_some() || test.missing.is_some(),
            Self::Resolves(_) | Self::Arguments(_) | Self::Layout(_) | Self::Calls(_) => false,
        }
    }
}

#[derive(Clone)]
pub(in crate::code_rule_dsl) struct SemanticFactDescriptor {
    pub(in crate::code_rule_dsl) source: SemanticFactSource,
    pub(in crate::code_rule_dsl) input_labels: Vec<String>,
    pub(in crate::code_rule_dsl) call_through: Option<super::super::calls::Through>,
}

impl std::fmt::Debug for SemanticFactDescriptor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SemanticFactDescriptor")
            .finish_non_exhaustive()
    }
}

/// A bounded value-reading request. It contains no predicate and does not decide a rule. The
/// producer emits one typed fact for each demanded node; SQL applies value predicates to facts.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct ValueFactDescriptor {
    pub(in crate::code_rule_dsl) language: GrammarDialect,
    pub(in crate::code_rule_dsl) follow: bool,
    pub(in crate::code_rule_dsl) expression_kinds: Vec<u16>,
    pub(in crate::code_rule_dsl) reading: Reading,
}

impl std::fmt::Debug for ValueFactDescriptor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValueFactDescriptor")
            .field("language", &self.language)
            .field("follow", &self.follow)
            .field("expression_kinds", &self.expression_kinds)
            .finish_non_exhaustive()
    }
}

struct ValueFact {
    verdict: Option<bool>,
    text: Option<String>,
}

impl ValueFact {
    fn known(text: String) -> Self {
        Self {
            verdict: Some(true),
            text: Some(text),
        }
    }

    fn missing() -> Self {
        Self {
            verdict: Some(false),
            text: None,
        }
    }

    fn unknown() -> Self {
        Self {
            verdict: None,
            text: None,
        }
    }
}

impl ValueFactDescriptor {
    fn canonical(&self, value: &super::super::value::Value) -> Option<String> {
        use super::super::value::Value;
        Some(match value {
            Value::Unknown => return None,
            Value::None => "null".to_owned(),
            Value::Undefined => "undefined".to_owned(),
            Value::Ellipsis => "ellipsis".to_owned(),
            Value::Bool(value) if self.reading.lang() == Lang::Python => {
                format!("int:{}", i64::from(*value))
            }
            Value::Bool(value) => format!("bool:{}", i64::from(*value)),
            Value::Int(value) => format!("int:{value}"),
            Value::Float(value)
                if self.reading.lang() == Lang::Python
                    && value.is_finite()
                    && value.fract() == 0.0 =>
            {
                format!(
                    "int:{}",
                    num_bigint::BigInt::from_f64(*value)
                        .expect("a finite integral float converts to an integer")
                )
            }
            Value::Float(value) => format!("float:{:016x}", value.to_bits()),
            Value::BigInt(value) => format!("bigint:{value}"),
            Value::Str(value) => format!("string:{value}"),
            Value::Utf16(value) => format!(
                "utf16:{}",
                value
                    .iter()
                    .map(|unit| format!("{unit:04x}"))
                    .collect::<String>()
            ),
            Value::PhpStr(value) | Value::Bytes(value) => format!(
                "bytes:{}",
                value
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ),
            // Phase two predicates accept scalar literals only. A known non-scalar fact is still
            // distinct from every accepted scalar and must not become unknown.
            Value::Complex(..)
            | Value::Tuple(_)
            | Value::Array(_)
            | Value::RegExp(_)
            | Value::PhpArray(_)
            | Value::Container { .. } => "non-scalar".to_owned(),
        })
    }

    fn read(&self, node: &ast_grep_core::Node<'_, TapeDoc>) -> ValueFact {
        if !self.reading.has_value(node) {
            return ValueFact::missing();
        }
        let value = self.reading.value(node);
        match self.canonical(&value) {
            Some(value) => ValueFact::known(value),
            None => ValueFact::unknown(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct DocumentMapSpan {
    pub(in crate::code_rule_dsl) source_document: usize,
    pub(in crate::code_rule_dsl) generated_document: usize,
    pub(in crate::code_rule_dsl) source_start: usize,
    pub(in crate::code_rule_dsl) source_end: usize,
    pub(in crate::code_rule_dsl) generated_start: usize,
    pub(in crate::code_rule_dsl) generated_end: usize,
}

pub(in crate::code_rule_dsl) struct PackedDemand {
    backend: ExecutionBackend,
    programs: Vec<Program>,
    producer_kinds: Vec<ProducerKind>,
    value_facts: Vec<Option<ValueFactDescriptor>>,
    semantic_facts: Vec<Option<SemanticFactDescriptor>>,
    parse_facts: Vec<Option<ParseFactDescriptor>>,
    document_maps: Vec<DocumentMapSpan>,
    files: Vec<TapeDoc>,
    anchors: Vec<Vec<Vec<u32>>>,
    task_grid: Vec<Vec<bool>>,
    result_limits: Vec<u32>,
    failure_injections: HashMap<usize, String>,
}

impl PackedDemand {
    pub(in crate::code_rule_dsl) fn new(
        backend: ExecutionBackend,
        programs: Vec<Program>,
        files: Vec<TapeDoc>,
        anchors: Vec<Vec<Vec<u32>>>,
        task_grid: Vec<Vec<bool>>,
        result_limits: Vec<u32>,
    ) -> Result<Self, PackedError> {
        if anchors.len() != files.len()
            || task_grid.len() != files.len()
            || anchors.iter().any(|file| file.len() != programs.len())
            || task_grid.iter().any(|file| file.len() != programs.len())
            || result_limits.len() != programs.len()
        {
            return Err(PackedError::InvalidAnchor);
        }
        let producer_kinds = vec![ProducerKind::Terminal; programs.len()];
        let value_facts = vec![None; programs.len()];
        let semantic_facts = vec![None; programs.len()];
        let parse_facts = vec![None; programs.len()];
        Ok(Self {
            backend,
            programs,
            producer_kinds,
            value_facts,
            semantic_facts,
            parse_facts,
            document_maps: Vec::new(),
            files,
            anchors,
            task_grid,
            result_limits,
            failure_injections: HashMap::new(),
        })
    }

    pub(in crate::code_rule_dsl) fn with_producer_kinds(
        mut self,
        producer_kinds: Vec<ProducerKind>,
    ) -> Result<Self, PackedError> {
        if producer_kinds.len() != self.programs.len()
            || producer_kinds.iter().enumerate().any(|(producer, kind)| {
                (*kind == ProducerKind::Pattern && !self.programs[producer].is_ast_pattern())
                    || (*kind == ProducerKind::Navigation
                        && (!self.programs[producer].is_navigation_descriptor()
                            || self
                                .task_grid
                                .iter()
                                .zip(&self.anchors)
                                .any(|(tasks, anchors)| {
                                    tasks[producer] && anchors[producer].as_slice() != [0]
                                })))
            })
        {
            return Err(PackedError::InvalidAnchor);
        }
        self.producer_kinds = producer_kinds;
        Ok(self)
    }

    pub(in crate::code_rule_dsl) fn with_value_facts(
        mut self,
        value_facts: Vec<Option<ValueFactDescriptor>>,
    ) -> Result<Self, PackedError> {
        if value_facts.len() != self.programs.len()
            || value_facts
                .iter()
                .enumerate()
                .any(|(producer, descriptor)| {
                    descriptor.is_some()
                        != (self.producer_kinds[producer] == ProducerKind::ValueFact)
                })
        {
            return Err(PackedError::InvalidAnchor);
        }
        self.value_facts = value_facts;
        Ok(self)
    }

    pub(in crate::code_rule_dsl) fn with_semantic_facts(
        mut self,
        semantic_facts: Vec<Option<SemanticFactDescriptor>>,
    ) -> Result<Self, PackedError> {
        if semantic_facts.len() != self.programs.len()
            || semantic_facts
                .iter()
                .enumerate()
                .any(|(producer, descriptor)| {
                    descriptor.is_some()
                        != (self.producer_kinds[producer] == ProducerKind::SemanticFact)
                })
        {
            return Err(PackedError::InvalidAnchor);
        }
        self.semantic_facts = semantic_facts;
        Ok(self)
    }

    pub(in crate::code_rule_dsl) fn with_parse_facts(
        mut self,
        parse_facts: Vec<Option<ParseFactDescriptor>>,
    ) -> Result<Self, PackedError> {
        if parse_facts.len() != self.programs.len()
            || parse_facts
                .iter()
                .enumerate()
                .any(|(producer, descriptor)| {
                    descriptor.is_some()
                        != (self.producer_kinds[producer] == ProducerKind::ParseFact)
                })
        {
            return Err(PackedError::InvalidAnchor);
        }
        self.parse_facts = parse_facts;
        Ok(self)
    }

    pub(in crate::code_rule_dsl) fn with_document_maps(
        mut self,
        document_maps: Vec<DocumentMapSpan>,
        required: bool,
    ) -> Result<Self, PackedError> {
        if required && document_maps.is_empty() {
            return Err(PackedError::InvalidAnchor);
        }
        if document_maps.iter().any(|span| {
            span.source_document >= self.files.len()
                || span.generated_document >= self.files.len()
                || span.source_start >= span.source_end
                || span.generated_start >= span.generated_end
                || span.source_end > self.files[span.source_document].transducer_source().len()
        }) || document_maps.iter().enumerate().any(|(index, span)| {
            document_maps[..index].iter().any(|earlier| {
                (span.source_document == earlier.source_document
                    && span.source_start < earlier.source_end
                    && earlier.source_start < span.source_end)
                    || (span.generated_document == earlier.generated_document
                        && span.generated_start < earlier.generated_end
                        && earlier.generated_start < span.generated_end)
            })
        }) {
            return Err(PackedError::InvalidAnchor);
        }
        self.document_maps = document_maps;
        Ok(self)
    }

    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn with_failure_injection(
        mut self,
        producer: usize,
        code: String,
    ) -> Result<Self, PackedError> {
        if producer >= self.programs.len()
            || self.failure_injections.insert(producer, code).is_some()
        {
            return Err(PackedError::InvalidAnchor);
        }
        Ok(self)
    }

    fn execute(self, token: i64) -> Vec<PackedRow> {
        let execution = match execute_with_backend_packed_result_limits(
            self.backend,
            &self.programs,
            &self.files,
            &self.anchors,
            Some(&self.result_limits),
        ) {
            Ok(execution) => execution,
            Err(error) => {
                let message = format!("{error:?}");
                let mut rows = vec![PackedRow::batch_status(token, false, Some(message.clone()))];
                for (file, tasks) in self.task_grid.iter().enumerate() {
                    for (producer, demanded) in tasks.iter().copied().enumerate() {
                        if demanded {
                            rows.push(PackedRow::producer_status(
                                token,
                                file,
                                producer,
                                false,
                                None,
                                Some(message.clone()),
                            ));
                        }
                    }
                }
                return rows;
            }
        };
        let mut rows: Vec<PackedRow> = Vec::new();
        let mut batch_error = None;
        for file in 0..execution.file_count() {
            for program in 0..execution.program_count() {
                if !self.task_grid[file][program] {
                    continue;
                }
                if let Some(code) = self.failure_injections.get(&program) {
                    batch_error.get_or_insert_with(|| code.clone());
                    rows.push(PackedRow::producer_status(
                        token,
                        file,
                        program,
                        false,
                        None,
                        Some(code.clone()),
                    ));
                    continue;
                }
                if self.producer_kinds[program] == ProducerKind::ParseFact {
                    let descriptor = self.parse_facts[program]
                        .as_ref()
                        .expect("a verified parse producer has its descriptor");
                    let parsed = AstGrep::doc(self.files[file].clone());
                    let _scope = super::super::matcher::FileScopeSlot::install(
                        Vec::new(),
                        None,
                        !parsed.root().get_doc().syntax_error_ranges().is_empty(),
                    );
                    let parse_row_start = rows.len();
                    let mut failure = None;
                    super::super::matcher::begin_bounded_producer(
                        self.result_limits[program] as usize,
                    );
                    super::super::matcher::with_file_memo(&parsed, || {
                        let root = parsed.root();
                        let tape = root.get_doc();
                        for (task_index, ordinal) in
                            self.anchors[file][program].iter().copied().enumerate()
                        {
                            let host = parsed.adopt(tape.node(ordinal));
                            let parsed_tree = super::super::matcher::parse_fact_tree(
                                &host,
                                descriptor.grammar,
                                &descriptor.host_kinds,
                                descriptor.reading,
                            );
                            match parsed_tree {
                                Some(super::super::literal::Parsed::Tree(tree)) => {
                                    let document = AstGrep::doc(super::super::literal::SubDoc {
                                        tree: Arc::clone(&tree),
                                    });
                                    for (position, candidate) in document.root().dfs().enumerate() {
                                        if rows.len() - parse_row_start
                                            >= self.result_limits[program] as usize
                                        {
                                            failure = Some("parse-fact-capacity".to_owned());
                                            break;
                                        }
                                        let Some(range) = tree.host_range(candidate.node_id())
                                        else {
                                            failure = Some("parse-invalid-map".to_owned());
                                            break;
                                        };
                                        rows.push(PackedRow::parse_fact(
                                            token,
                                            file,
                                            program,
                                            task_index,
                                            ordinal,
                                            position,
                                            candidate.kind_id(),
                                            range,
                                            candidate.text().into_owned(),
                                            candidate.children().next().is_none(),
                                            candidate.is_named(),
                                        ));
                                    }
                                }
                                Some(super::super::literal::Parsed::NotString) => {}
                                Some(super::super::literal::Parsed::Unknown) | None => {
                                    failure = Some(
                                        if super::super::matcher::bounded_producer_exceeded() {
                                            "parse-fact-capacity"
                                        } else {
                                            "parse-incomplete"
                                        }
                                        .to_owned(),
                                    );
                                }
                            }
                            if failure.is_some() {
                                break;
                            }
                        }
                    });
                    if let Some(code) = failure {
                        rows.truncate(parse_row_start);
                        batch_error.get_or_insert_with(|| code.clone());
                        rows.push(PackedRow::producer_status(
                            token,
                            file,
                            program,
                            false,
                            None,
                            Some(code),
                        ));
                    } else {
                        rows.push(PackedRow::producer_status(
                            token,
                            file,
                            program,
                            true,
                            Some(true),
                            None,
                        ));
                    }
                    continue;
                }
                if self.producer_kinds[program] == ProducerKind::SemanticFact {
                    use super::semantic_facts::{Target, slot};
                    let descriptor = self.semantic_facts[program]
                        .as_ref()
                        .expect("a verified semantic producer has its descriptor");
                    let parsed = AstGrep::doc(self.files[file].clone());
                    let _scope = super::super::matcher::FileScopeSlot::install(
                        Vec::new(),
                        None,
                        !parsed.root().get_doc().syntax_error_ranges().is_empty(),
                    );
                    let semantic_row_start = rows.len();
                    let semantic_capacity_failed = std::cell::Cell::new(false);
                    let call_summary = match &descriptor.source {
                        SemanticFactSource::Calls(test) => Some(
                            super::super::calls::summarize(
                                parsed.root(),
                                test.language,
                                "",
                                None,
                                !parsed.root().get_doc().syntax_error_ranges().is_empty(),
                                self.files[file].transducer_source().len(),
                                None,
                            )
                            .0,
                        ),
                        _ => None,
                    };
                    super::super::matcher::with_file_memo(&parsed, || {
                        let root = parsed.root();
                        let tape = root.get_doc();
                        for (task_index, ordinal) in
                            self.anchors[file][program].iter().copied().enumerate()
                        {
                            let node = parsed.adopt(tape.node(ordinal));
                            let capture_sets = if descriptor.input_labels.is_empty() {
                                vec![Vec::new()]
                            } else {
                                let matched = rows
                                    .iter()
                                    .filter(|row| {
                                        row.row_kind == RowKind::TerminalMatch
                                            && row.file_index == Some(file as i64)
                                            && row.node == Some(i64::from(ordinal))
                                    })
                                    .filter_map(|row| Some((row.producer_id?, row.match_index?)))
                                    .collect::<Vec<_>>();
                                descriptor.input_labels.iter().fold(
                                    vec![Vec::new()],
                                    |environments, label| {
                                        let alternatives = matched
                                            .iter()
                                            .map(|&(producer, matched)| {
                                                rows.iter()
                                                    .filter(|row| {
                                                        row.row_kind == RowKind::Capture
                                                            && row.file_index == Some(file as i64)
                                                            && row.producer_id == Some(producer)
                                                            && row.match_index == Some(matched)
                                                            && row.capture_label.as_ref()
                                                                == Some(label)
                                                    })
                                                    .cloned()
                                                    .collect::<Vec<_>>()
                                            })
                                            .filter(|captures| !captures.is_empty())
                                            .collect::<Vec<_>>();
                                        environments
                                            .into_iter()
                                            .flat_map(|environment| {
                                                alternatives.iter().map(move |alternative| {
                                                    let mut combined = environment.clone();
                                                    combined.extend(alternative.iter().cloned());
                                                    combined
                                                })
                                            })
                                            .collect()
                                    },
                                )
                            };
                            for (environment_index, captures) in
                                capture_sets.into_iter().enumerate()
                            {
                                let environment_key = (environment_index as i64) << 32;
                                let mut environment = ast_grep_core::meta_var::MetaVarEnv::new();
                                for label in &descriptor.input_labels {
                                    let found = captures
                                        .iter()
                                        .filter(|capture| {
                                            capture.capture_label.as_ref() == Some(label)
                                        })
                                        .filter_map(|capture| {
                                            let node = u32::try_from(capture.node?).ok()?;
                                            Some(parsed.adopt(tape.node(node)))
                                        })
                                        .collect::<Vec<_>>();
                                    if captures.iter().any(|capture| {
                                        capture.capture_label.as_ref() == Some(label)
                                            && capture.capture_kind == Some(2)
                                    }) {
                                        environment.insert_multi(label, found);
                                    } else if let Some(found) = found.into_iter().next() {
                                        let _ = environment.insert(label, found);
                                    }
                                }
                                let mut facts = super::super::matcher::producer_facts(
                                    &descriptor.source,
                                    node.clone(),
                                    &environment,
                                );
                                if let (
                                    SemanticFactSource::Calls(test),
                                    Some(summary),
                                    Some(through),
                                ) = (&descriptor.source, &call_summary, descriptor.call_through)
                                {
                                    use super::super::calls::summary::Callee;
                                    use super::super::language::Family;
                                    use super::super::language::adapter::{
                                        Adapter, CallRules, with_adapter,
                                    };
                                    facts.rows.clear();
                                    let callee = with_adapter!(Family::of(test.language), A => <<A as Adapter>::Calls as CallRules>::callee_at(
                                        &node,
                                        summary,
                                        || super::super::matcher::calls_tables(&node, test.language),
                                    ));
                                    match callee {
                                        Some(Some(Callee::Name(name))) => {
                                            let mut pending = summary
                                                .defs
                                                .iter()
                                                .enumerate()
                                                .filter(|(_, definition)| {
                                                    summary.name(definition.name) == name
                                                })
                                                .map(|(index, _)| index)
                                                .collect::<Vec<_>>();
                                            let mut reached = std::collections::BTreeSet::new();
                                            while let Some(index) = pending.pop() {
                                                if !reached.insert(index) {
                                                    continue;
                                                }
                                                let definition = &summary.defs[index];
                                                if !through.allows(definition.flags) {
                                                    continue;
                                                }
                                                for (_, target) in summary
                                                    .edges
                                                    .iter()
                                                    .filter(|(caller, _)| *caller == index as u32)
                                                {
                                                    let target = summary.name(*target);
                                                    pending.extend(
                                                        summary
                                                            .defs
                                                            .iter()
                                                            .enumerate()
                                                            .filter(|(_, candidate)| {
                                                                summary.name(candidate.name)
                                                                    == target
                                                            })
                                                            .map(|(index, _)| index),
                                                    );
                                                }
                                            }
                                            if reached.is_empty() {
                                                continue;
                                            }
                                            facts.gate(1, Target::None);
                                            for index in reached {
                                                let definition = &summary.defs[index];
                                                let target = root.dfs().find(|candidate| {
                                                    candidate.range().start
                                                        == definition.start as usize
                                                        && candidate.range().end
                                                            == definition.end as usize
                                                });
                                                if let Some(target) = target {
                                                    facts.push(
                                                        slot::CALL_TARGET,
                                                        0,
                                                        1,
                                                        Target::Node(target),
                                                    );
                                                }
                                            }
                                        }
                                        Some(Some(Callee::Open)) | None => {
                                            facts.gate(-1, Target::None);
                                        }
                                        Some(None) => {}
                                    }
                                }
                                let has_gate =
                                    facts.rows.iter().any(|fact| fact.slot == slot::GATE);
                                for fact in facts.rows {
                                    let (related, kind) = match fact.target {
                                        Target::None => (None, None),
                                        Target::Tree(tree, table) => {
                                            let _ = table;
                                            (None, Some(tree.root_kind()))
                                        }
                                        Target::Node(target) => (
                                            u32::try_from(target.node_id()).ok(),
                                            Some(target.kind_id()),
                                        ),
                                        Target::TreeNode(tree, node_id) => {
                                            (None, tree.node_kind(node_id))
                                        }
                                    };
                                    let output_label = (fact.slot == slot::ARGUMENT).then(|| {
                                        let SemanticFactSource::Arguments(arguments) =
                                            &descriptor.source
                                        else {
                                            unreachable!(
                                                "argument rows have an argument descriptor"
                                            )
                                        };
                                        arguments.bind[fact.key as usize].0.clone()
                                    });
                                    let fact_key = environment_key | fact.key;
                                    let mut row = PackedRow::semantic_fact(
                                        token,
                                        file,
                                        program,
                                        task_index,
                                        ordinal,
                                        fact.slot,
                                        fact_key,
                                        fact.pos,
                                        fact.flag,
                                        related,
                                        kind,
                                        output_label,
                                    );
                                    if let Some(related) = related {
                                        let span = tape.transducer_span(related);
                                        row.source_text = tape
                                            .transducer_source()
                                            .get(span)
                                            .map(|text| String::from_utf8_lossy(text).into_owned());
                                        row.structural_key =
                                            Some(tape.transducer_structural_key(related));
                                        row.named_leaf =
                                            Some(i64::from(tape.transducer_is_named_leaf(related)));
                                        row.capture_named = Some(i64::from(
                                            tape.transducer_flags(related)
                                                & super::super::packed::NAMED
                                                != 0,
                                        ));
                                    }
                                    if rows.len() - semantic_row_start
                                        >= self.result_limits[program] as usize
                                    {
                                        semantic_capacity_failed.set(true);
                                        break;
                                    }
                                    rows.push(row);
                                }
                                if has_gate {
                                    for capture in &captures {
                                        let Some(related) =
                                            capture.node.and_then(|node| u32::try_from(node).ok())
                                        else {
                                            continue;
                                        };
                                        if rows.len() - semantic_row_start
                                            >= self.result_limits[program] as usize
                                        {
                                            semantic_capacity_failed.set(true);
                                            break;
                                        }
                                        let mut row = PackedRow::semantic_fact(
                                            token,
                                            file,
                                            program,
                                            task_index,
                                            ordinal,
                                            slot::INPUT_BINDING,
                                            environment_key,
                                            capture.capture_occurrence.unwrap_or(0),
                                            1,
                                            Some(related),
                                            Some(tape.transducer_kind(related)),
                                            capture.capture_label.clone(),
                                        );
                                        row.source_text = capture.source_text.clone();
                                        row.structural_key = capture.structural_key.clone();
                                        row.named_leaf = capture.named_leaf;
                                        row.capture_named = capture.capture_named;
                                        row.capture_kind = capture.capture_kind;
                                        row.equality_occurrence = capture.equality_occurrence;
                                        rows.push(row);
                                    }
                                }
                                if semantic_capacity_failed.get() {
                                    break;
                                }
                            }
                            if semantic_capacity_failed.get() {
                                break;
                            }
                        }
                    });
                    if semantic_capacity_failed.get() {
                        rows.truncate(semantic_row_start);
                        let code = "semantic-fact-capacity".to_owned();
                        batch_error.get_or_insert_with(|| code.clone());
                        rows.push(PackedRow::producer_status(
                            token,
                            file,
                            program,
                            false,
                            None,
                            Some(code),
                        ));
                    } else {
                        rows.push(PackedRow::producer_status(
                            token,
                            file,
                            program,
                            true,
                            Some(true),
                            None,
                        ));
                    }
                    continue;
                }
                if self.producer_kinds[program] == ProducerKind::ValueFact {
                    let descriptor = self.value_facts[program]
                        .as_ref()
                        .expect("a verified value producer has its descriptor");
                    let parsed = AstGrep::doc(self.files[file].clone());
                    let _scope = super::super::matcher::FileScopeSlot::install(
                        Vec::new(),
                        None,
                        !parsed.root().get_doc().syntax_error_ranges().is_empty(),
                    );
                    super::super::matcher::with_file_memo(&parsed, || {
                        let root = parsed.root();
                        let tape = root.get_doc();
                        for (task_index, node) in
                            self.anchors[file][program].iter().copied().enumerate()
                        {
                            let node_value = parsed.adopt(tape.node(node));
                            let value = descriptor.read(&node_value);
                            rows.push(PackedRow::value_fact(
                                token,
                                file,
                                program,
                                task_index,
                                node,
                                value.verdict,
                                value.text,
                            ));
                        }
                    });
                    rows.push(PackedRow::producer_status(
                        token,
                        file,
                        program,
                        true,
                        Some(true),
                        None,
                    ));
                    continue;
                }
                if self.producer_kinds[program] == ProducerKind::Pattern {
                    let tape = &self.files[file];
                    let mut match_index = 0_usize;
                    let mut failed = None;
                    for (task_index, root) in
                        self.anchors[file][program].iter().copied().enumerate()
                    {
                        if match_index >= self.result_limits[program] as usize {
                            break;
                        }
                        match self.programs[program].execute_ast_pattern(tape, root) {
                            Ok(Some(captures)) => {
                                rows.push(PackedRow::terminal_match(TerminalMatchData {
                                    batch: token,
                                    file,
                                    producer: program,
                                    match_index,
                                    task_index: task_index as u32,
                                    start_ordinal: root,
                                    node: root,
                                    span: tape.transducer_span(root),
                                    output_label: Some("ast-grep-pattern".to_owned()),
                                }));
                                let mut equality_occurrences = HashMap::<&str, usize>::new();
                                for (capture_index, capture) in captures.iter().enumerate() {
                                    let label = &capture.label;
                                    let node = capture.node;
                                    let span = tape.transducer_span(node);
                                    let sequence_start = captures
                                        .iter()
                                        .position(|candidate| candidate.label == *label)
                                        .expect("the current capture has a sequence");
                                    let sequence_count = captures
                                        .iter()
                                        .filter(|candidate| candidate.label == *label)
                                        .count();
                                    let capture_named = tape.transducer_flags(node)
                                        & super::super::packed::NAMED
                                        != 0;
                                    let equality_occurrence = if capture_named {
                                        let occurrence =
                                            equality_occurrences.entry(label.as_str()).or_default();
                                        let value = *occurrence;
                                        *occurrence += 1;
                                        value as i64
                                    } else {
                                        -1
                                    };
                                    rows.push(PackedRow::capture(CaptureData {
                                        batch: token,
                                        file,
                                        producer: program,
                                        match_index,
                                        capture_index,
                                        occurrence: capture.occurrence,
                                        sequence_start,
                                        sequence_count,
                                        node,
                                        capture_label: Some(label.clone()),
                                        text_value: None,
                                        source_text: tape
                                            .transducer_source()
                                            .get(span.clone())
                                            .map(|text| String::from_utf8_lossy(text).into_owned()),
                                        structural_key: tape.transducer_structural_key(node),
                                        named_leaf: tape.transducer_is_named_leaf(node),
                                        capture_kind: if capture.multi { 2 } else { 1 },
                                        capture_named,
                                        equality_occurrence,
                                        start_byte: span.start,
                                        end_byte: span.end,
                                    }));
                                }
                                match_index += 1;
                            }
                            Ok(None) => {}
                            Err(error) => {
                                failed = Some(format!("{error:?}"));
                                break;
                            }
                        }
                    }
                    if let Some(message) = failed {
                        batch_error.get_or_insert_with(|| message.clone());
                        rows.push(PackedRow::producer_status(
                            token,
                            file,
                            program,
                            false,
                            None,
                            Some(message),
                        ));
                    } else {
                        rows.push(PackedRow::producer_status(
                            token,
                            file,
                            program,
                            true,
                            Some(match_index != 0),
                            None,
                        ));
                    }
                    continue;
                }
                if self.producer_kinds[program] == ProducerKind::Navigation {
                    let tape = &self.files[file];
                    let mut edge_index = 0_usize;
                    let mut failed = false;
                    for parent in 0..tape.transducer_len() as u32 {
                        for (child_order, child) in tape.transducer_children(parent).enumerate() {
                            if edge_index >= 1 << 26 {
                                failed = true;
                                break;
                            }
                            rows.push(PackedRow::navigation(
                                token,
                                file,
                                program,
                                edge_index,
                                parent,
                                child,
                                child_order,
                                tape.transducer_field(child),
                                tape.transducer_flags(child) & 1 != 0,
                            ));
                            edge_index += 1;
                        }
                        if failed {
                            break;
                        }
                    }
                    if failed {
                        let message = format!("{:?}", PackedError::WorkLimit);
                        batch_error.get_or_insert_with(|| message.clone());
                        rows.push(PackedRow::producer_status(
                            token,
                            file,
                            program,
                            false,
                            None,
                            Some(message),
                        ));
                    } else {
                        rows.push(PackedRow::producer_status(
                            token,
                            file,
                            program,
                            true,
                            Some(edge_index != 0),
                            None,
                        ));
                    }
                    continue;
                }
                let (results, error) = match execution.program_results(file, program) {
                    Ok(result) => result,
                    Err(error) => {
                        let message = format!("{error:?}");
                        batch_error.get_or_insert_with(|| message.clone());
                        rows.push(PackedRow::producer_status(
                            token,
                            file,
                            program,
                            false,
                            None,
                            Some(message),
                        ));
                        continue;
                    }
                };
                if let Some(error) = error {
                    let message = format!("{error:?}");
                    batch_error.get_or_insert_with(|| message.clone());
                    rows.push(PackedRow::producer_status(
                        token,
                        file,
                        program,
                        false,
                        None,
                        Some(message),
                    ));
                    continue;
                }
                rows.push(PackedRow::producer_status(
                    token,
                    file,
                    program,
                    true,
                    Some(!results.is_empty()),
                    None,
                ));
                for (match_index, result) in results.into_iter().enumerate() {
                    rows.push(PackedRow::terminal_match(TerminalMatchData {
                        batch: token,
                        file,
                        producer: program,
                        match_index,
                        task_index: result.task_ordinal(),
                        start_ordinal: result.local_ordinal(),
                        node: result.start_node(),
                        span: self.files[file].transducer_span(result.start_node()),
                        output_label: self.programs[program]
                            .label_name(result.label())
                            .map(str::to_owned),
                    }));
                    let captures = result.captures().collect::<Vec<_>>();
                    for (capture_index, capture) in captures.iter().copied().enumerate() {
                        let label = capture.label();
                        let occurrence = captures[..capture_index]
                            .iter()
                            .filter(|candidate| candidate.label() == label)
                            .count();
                        let sequence_start = captures
                            .iter()
                            .position(|candidate| candidate.label() == label)
                            .expect("the current capture has a sequence");
                        let sequence_count = captures
                            .iter()
                            .filter(|candidate| candidate.label() == label)
                            .count();
                        let span = capture.span();
                        let source_text = self.files[file]
                            .transducer_source()
                            .get(span.clone())
                            .map(|text| String::from_utf8_lossy(text).into_owned());
                        rows.push(PackedRow::capture(CaptureData {
                            batch: token,
                            file,
                            producer: program,
                            match_index,
                            capture_index,
                            occurrence,
                            sequence_start,
                            sequence_count,
                            node: capture.node(),
                            capture_label: self.programs[program]
                                .label_name(label)
                                .map(str::to_owned),
                            text_value: capture
                                .transformed_text()
                                .map(|text| String::from_utf8_lossy(text).into_owned()),
                            source_text,
                            structural_key: self.files[file]
                                .transducer_structural_key(capture.node()),
                            named_leaf: self.files[file].transducer_is_named_leaf(capture.node()),
                            capture_kind: 1,
                            capture_named: self.files[file].transducer_flags(capture.node())
                                & super::super::packed::NAMED
                                != 0,
                            equality_occurrence: 0,
                            start_byte: span.start,
                            end_byte: span.end,
                        }));
                    }
                }
            }
        }
        rows.extend(
            self.document_maps
                .iter()
                .enumerate()
                .map(|(index, span)| PackedRow::document_map(token, index, *span)),
        );
        rows.insert(
            0,
            PackedRow::batch_status(token, batch_error.is_none(), batch_error),
        );
        rows
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i64)]
pub(in crate::code_rule_dsl) enum RowKind {
    BatchStatus = 1,
    ProducerStatus = 2,
    TerminalMatch = 3,
    Capture = 4,
    Navigation = 5,
    Fact = 6,
    DocumentMap = 7,
}

const ALL_ROW_KINDS: [RowKind; 7] = [
    RowKind::BatchStatus,
    RowKind::ProducerStatus,
    RowKind::TerminalMatch,
    RowKind::Capture,
    RowKind::Navigation,
    RowKind::Fact,
    RowKind::DocumentMap,
];

struct TerminalMatchData {
    batch: i64,
    file: usize,
    producer: usize,
    match_index: usize,
    task_index: u32,
    start_ordinal: u32,
    node: u32,
    span: std::ops::Range<usize>,
    output_label: Option<String>,
}

struct CaptureData {
    batch: i64,
    file: usize,
    producer: usize,
    match_index: usize,
    capture_index: usize,
    occurrence: usize,
    sequence_start: usize,
    sequence_count: usize,
    node: u32,
    capture_label: Option<String>,
    text_value: Option<String>,
    source_text: Option<String>,
    structural_key: String,
    named_leaf: bool,
    capture_kind: i64,
    capture_named: bool,
    equality_occurrence: i64,
    start_byte: usize,
    end_byte: usize,
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct TestProducerStatusRow {
    pub(in crate::code_rule_dsl) producer: i64,
    pub(in crate::code_rule_dsl) file: i64,
    pub(in crate::code_rule_dsl) complete: i64,
    pub(in crate::code_rule_dsl) verdict: Option<i64>,
    pub(in crate::code_rule_dsl) work_used: Option<i64>,
    pub(in crate::code_rule_dsl) error: Option<String>,
}

#[cfg(test)]
pub(in crate::code_rule_dsl) struct TestFactRow {
    pub(in crate::code_rule_dsl) producer: i64,
    pub(in crate::code_rule_dsl) node: i64,
    pub(in crate::code_rule_dsl) related: Option<i64>,
    pub(in crate::code_rule_dsl) slot: i64,
    pub(in crate::code_rule_dsl) key: i64,
    pub(in crate::code_rule_dsl) position: i64,
    pub(in crate::code_rule_dsl) flag: Option<i64>,
    pub(in crate::code_rule_dsl) target_kind: Option<i64>,
    pub(in crate::code_rule_dsl) label: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct PackedRow {
    batch: i64,
    row_kind: RowKind,
    producer_id: Option<i64>,
    file_index: Option<i64>,
    task_index: Option<i64>,
    match_index: Option<i64>,
    capture_index: Option<i64>,
    capture_occurrence: Option<i64>,
    capture_sequence_start: Option<i64>,
    capture_sequence_count: Option<i64>,
    node: Option<i64>,
    related_node: Option<i64>,
    axis_kind: Option<i64>,
    axis_order: Option<i64>,
    fact_kind: Option<i64>,
    document_id: Option<i64>,
    mapped_document_id: Option<i64>,
    start_ordinal: Option<i64>,
    output_label: Option<String>,
    capture_label: Option<String>,
    text_value: Option<String>,
    start_byte: Option<i64>,
    end_byte: Option<i64>,
    verdict: Option<i64>,
    complete: i64,
    work_used: Option<i64>,
    error_code: Option<String>,
    field_id: Option<i64>,
    related_named: Option<i64>,
    source_text: Option<String>,
    structural_key: Option<String>,
    named_leaf: Option<i64>,
    capture_kind: Option<i64>,
    capture_named: Option<i64>,
    equality_occurrence: Option<i64>,
}

impl PackedRow {
    fn blank(batch: i64, row_kind: RowKind) -> Self {
        debug_assert!(ALL_ROW_KINDS.contains(&row_kind));
        Self {
            batch,
            row_kind,
            producer_id: None,
            file_index: None,
            task_index: None,
            match_index: None,
            capture_index: None,
            capture_occurrence: None,
            capture_sequence_start: None,
            capture_sequence_count: None,
            node: None,
            related_node: None,
            axis_kind: None,
            axis_order: None,
            fact_kind: None,
            document_id: None,
            mapped_document_id: None,
            start_ordinal: None,
            output_label: None,
            capture_label: None,
            text_value: None,
            start_byte: None,
            end_byte: None,
            verdict: None,
            complete: 1,
            work_used: None,
            error_code: None,
            field_id: None,
            related_named: None,
            source_text: None,
            structural_key: None,
            named_leaf: None,
            capture_kind: None,
            capture_named: None,
            equality_occurrence: None,
        }
    }

    fn batch_status(batch: i64, complete: bool, error: Option<String>) -> Self {
        let mut row = Self::blank(batch, RowKind::BatchStatus);
        row.complete = i64::from(complete);
        row.error_code = error;
        row
    }

    fn producer_status(
        batch: i64,
        file: usize,
        producer: usize,
        complete: bool,
        verdict: Option<bool>,
        error: Option<String>,
    ) -> Self {
        let mut row = Self::blank(batch, RowKind::ProducerStatus);
        row.file_index = Some(file as i64);
        row.producer_id = Some(producer as i64);
        row.complete = i64::from(complete);
        row.verdict = verdict.map(i64::from);
        row.error_code = error;
        row
    }

    #[allow(clippy::too_many_arguments)]
    fn navigation(
        batch: i64,
        file: usize,
        producer: usize,
        edge_index: usize,
        parent: u32,
        child: u32,
        child_order: usize,
        field: u16,
        named: bool,
    ) -> Self {
        let mut row = Self::blank(batch, RowKind::Navigation);
        row.file_index = Some(file as i64);
        row.producer_id = Some(producer as i64);
        row.task_index = Some(0);
        row.match_index = Some(edge_index as i64);
        row.node = Some(parent as i64);
        row.related_node = Some(child as i64);
        row.axis_kind = Some(1);
        row.axis_order = Some(child_order as i64);
        row.field_id = Some(i64::from(field));
        row.related_named = Some(i64::from(named));
        row
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_fact(
        batch: i64,
        file: usize,
        producer: usize,
        task_index: usize,
        host: u32,
        position: usize,
        kind: u16,
        range: std::ops::Range<usize>,
        source_text: String,
        leaf: bool,
        named: bool,
    ) -> Self {
        let mut row = Self::blank(batch, RowKind::Fact);
        row.file_index = Some(file as i64);
        row.producer_id = Some(producer as i64);
        row.task_index = Some(task_index as i64);
        row.match_index = Some(position as i64);
        row.capture_index = Some(position as i64);
        row.node = Some(host as i64);
        row.axis_order = Some(position as i64);
        row.fact_kind = Some(18);
        row.capture_kind = Some(i64::from(kind));
        row.start_byte = Some(range.start as i64);
        row.end_byte = Some(range.end as i64);
        row.verdict = Some(1);
        row.source_text = Some(source_text);
        row.named_leaf = Some(i64::from(leaf));
        row.capture_named = Some(i64::from(named));
        row
    }

    fn document_map(batch: i64, index: usize, span: DocumentMapSpan) -> Self {
        let mut row = Self::blank(batch, RowKind::DocumentMap);
        row.file_index = Some(span.generated_document as i64);
        row.match_index = Some(index as i64);
        row.node = Some(span.source_start as i64);
        row.related_node = Some(span.source_end as i64);
        row.document_id = Some(span.generated_document as i64);
        row.mapped_document_id = Some(span.source_document as i64);
        row.start_byte = Some(span.generated_start as i64);
        row.end_byte = Some(span.generated_end as i64);
        row
    }

    fn value_fact(
        batch: i64,
        file: usize,
        producer: usize,
        task_index: usize,
        node: u32,
        verdict: Option<bool>,
        text: Option<String>,
    ) -> Self {
        let mut row = Self::blank(batch, RowKind::Fact);
        row.file_index = Some(file as i64);
        row.producer_id = Some(producer as i64);
        row.task_index = Some(task_index as i64);
        row.match_index = Some(task_index as i64);
        row.node = Some(node as i64);
        row.fact_kind = Some(1);
        row.verdict = verdict.map(i64::from);
        row.text_value = text;
        row
    }

    #[allow(clippy::too_many_arguments)]
    fn semantic_fact(
        batch: i64,
        file: usize,
        producer: usize,
        task_index: usize,
        root: u32,
        slot: u16,
        key: i64,
        position: i64,
        flag: i64,
        related_node: Option<u32>,
        related_kind: Option<u16>,
        output_label: Option<String>,
    ) -> Self {
        let mut row = Self::blank(batch, RowKind::Fact);
        row.file_index = Some(file as i64);
        row.producer_id = Some(producer as i64);
        row.task_index = Some(task_index as i64);
        row.match_index = Some(key);
        row.capture_index = Some(position);
        row.node = Some(root as i64);
        row.related_node = related_node.map(i64::from);
        row.fact_kind = Some(i64::from(slot));
        row.verdict = Some(flag);
        row.capture_kind = related_kind.map(i64::from);
        row.output_label = output_label;
        row
    }

    fn terminal_match(data: TerminalMatchData) -> Self {
        let mut row = Self::blank(data.batch, RowKind::TerminalMatch);
        row.file_index = Some(data.file as i64);
        row.producer_id = Some(data.producer as i64);
        row.task_index = Some(data.task_index as i64);
        row.match_index = Some(data.match_index as i64);
        row.start_ordinal = Some(data.start_ordinal as i64);
        row.node = Some(data.node as i64);
        row.start_byte = Some(data.span.start as i64);
        row.end_byte = Some(data.span.end as i64);
        row.output_label = data.output_label;
        row
    }

    fn capture(data: CaptureData) -> Self {
        let mut row = Self::blank(data.batch, RowKind::Capture);
        row.file_index = Some(data.file as i64);
        row.producer_id = Some(data.producer as i64);
        row.match_index = Some(data.match_index as i64);
        row.capture_index = Some(data.capture_index as i64);
        row.capture_occurrence = Some(data.occurrence as i64);
        row.capture_sequence_start = Some(data.sequence_start as i64);
        row.capture_sequence_count = Some(data.sequence_count as i64);
        row.node = Some(data.node as i64);
        row.capture_label = data.capture_label;
        row.text_value = data.text_value;
        row.source_text = data.source_text;
        row.structural_key = Some(data.structural_key);
        row.named_leaf = Some(i64::from(data.named_leaf));
        row.capture_kind = Some(data.capture_kind);
        row.capture_named = Some(i64::from(data.capture_named));
        row.equality_occurrence = Some(data.equality_occurrence);
        row.start_byte = Some(data.start_byte as i64);
        row.end_byte = Some(data.end_byte as i64);
        row
    }

    #[cfg(test)]
    pub(in crate::code_rule_dsl) const fn row_kind(&self) -> RowKind {
        self.row_kind
    }

    pub(in crate::code_rule_dsl) fn is_complete(&self) -> bool {
        self.complete != 0
    }

    pub(in crate::code_rule_dsl) fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }

    pub(in crate::code_rule_dsl) fn from_sql_row(row: &rusqlite::Row<'_>) -> SqlResult<Self> {
        let row_kind = match row.get::<_, i64>(1)? {
            1 => RowKind::BatchStatus,
            2 => RowKind::ProducerStatus,
            3 => RowKind::TerminalMatch,
            4 => RowKind::Capture,
            5 => RowKind::Navigation,
            6 => RowKind::Fact,
            7 => RowKind::DocumentMap,
            value => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Integer,
                    format!("unknown rerail_matches row kind {value}").into(),
                ));
            }
        };
        Ok(Self {
            batch: row.get(0)?,
            row_kind,
            producer_id: row.get(2)?,
            file_index: row.get(3)?,
            task_index: row.get(4)?,
            match_index: row.get(5)?,
            capture_index: row.get(6)?,
            capture_occurrence: row.get(7)?,
            capture_sequence_start: row.get(8)?,
            capture_sequence_count: row.get(9)?,
            node: row.get(10)?,
            related_node: row.get(11)?,
            axis_kind: row.get(12)?,
            axis_order: row.get(13)?,
            fact_kind: row.get(14)?,
            document_id: row.get(15)?,
            mapped_document_id: row.get(16)?,
            start_ordinal: row.get(17)?,
            output_label: row.get(18)?,
            capture_label: row.get(19)?,
            text_value: row.get(20)?,
            start_byte: row.get(21)?,
            end_byte: row.get(22)?,
            verdict: row.get(23)?,
            complete: row.get(24)?,
            work_used: row.get(25)?,
            error_code: row.get(26)?,
            field_id: row.get(27)?,
            related_named: row.get(28)?,
            source_text: row.get(29)?,
            structural_key: row.get(30)?,
            named_leaf: row.get(31)?,
            capture_kind: row.get(32)?,
            capture_named: row.get(33)?,
            equality_occurrence: row.get(34)?,
        })
    }
}

fn execute_without_unwind(token: i64, execute: impl FnOnce() -> Vec<PackedRow>) -> Vec<PackedRow> {
    catch_unwind(AssertUnwindSafe(execute)).unwrap_or_else(|_| {
        vec![PackedRow::batch_status(
            token,
            false,
            Some("packed producer panicked".to_owned()),
        )]
    })
}

struct MatchBatch {
    token: i64,
    demand: Mutex<Option<PackedDemand>>,
    rows: OnceLock<Arc<Vec<PackedRow>>>,
    executions: AtomicUsize,
    filters: AtomicUsize,
}

impl MatchBatch {
    fn rows(&self) -> SqlResult<Arc<Vec<PackedRow>>> {
        Ok(Arc::clone(self.rows.get_or_init(|| {
            self.executions.fetch_add(1, Ordering::Relaxed);
            let demand = self
                .demand
                .lock()
                .expect("packed demand mutex poisoned")
                .take()
                .expect("a demanded packed batch executes once");
            Arc::new(execute_without_unwind(self.token, || {
                demand.execute(self.token)
            }))
        })))
    }
}

#[derive(Default)]
pub(in crate::code_rule_dsl) struct MatchRegistry {
    next: AtomicI64,
    batches: Mutex<HashMap<i64, Arc<MatchBatch>>>,
}

impl MatchRegistry {
    pub(in crate::code_rule_dsl) fn register(&self, demand: PackedDemand) -> i64 {
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.batches
            .lock()
            .expect("match registry mutex poisoned")
            .insert(
                token,
                Arc::new(MatchBatch {
                    token,
                    demand: Mutex::new(Some(demand)),
                    rows: OnceLock::new(),
                    executions: AtomicUsize::new(0),
                    filters: AtomicUsize::new(0),
                }),
            );
        token
    }

    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn query_producer_status_rows(
        &self,
        token: i64,
    ) -> Vec<TestProducerStatusRow> {
        let Some(batch) = self.batch(token) else {
            return Vec::new();
        };
        let Some(rows) = batch.rows.get() else {
            return Vec::new();
        };
        rows.iter()
            .filter(|row| row.row_kind == RowKind::ProducerStatus)
            .map(|row| TestProducerStatusRow {
                producer: row.producer_id.expect("a producer status has a producer"),
                file: row.file_index.expect("a producer status has a file"),
                complete: row.complete,
                verdict: row.verdict,
                work_used: row.work_used,
                error: row.error_code.clone(),
            })
            .collect()
    }

    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn query_fact_rows(&self, token: i64) -> Vec<TestFactRow> {
        let Some(batch) = self.batch(token) else {
            return Vec::new();
        };
        let Some(rows) = batch.rows.get() else {
            return Vec::new();
        };
        rows.iter()
            .filter(|row| row.row_kind == RowKind::Fact)
            .map(|row| TestFactRow {
                producer: row.producer_id.expect("a fact has a producer"),
                node: row.node.expect("a fact has a source node"),
                related: row.related_node,
                slot: row.fact_kind.expect("a fact has a slot"),
                key: row.match_index.expect("a fact has a key"),
                position: row.capture_index.unwrap_or(0),
                flag: row.verdict,
                target_kind: row.capture_kind,
                label: row.output_label.clone(),
            })
            .collect()
    }

    pub(in crate::code_rule_dsl) fn query_execution_counters(
        &self,
        token: i64,
    ) -> Option<(usize, usize)> {
        self.batch(token).map(|batch| {
            (
                batch.executions.load(Ordering::Relaxed),
                batch.filters.load(Ordering::Relaxed),
            )
        })
    }

    pub(in crate::code_rule_dsl) fn remove_batch(&self, token: i64) {
        self.batches
            .lock()
            .expect("match registry mutex poisoned")
            .remove(&token);
    }

    fn batch(&self, token: i64) -> Option<Arc<MatchBatch>> {
        self.batches
            .lock()
            .expect("match registry mutex poisoned")
            .get(&token)
            .cloned()
    }
}

pub(in crate::code_rule_dsl) fn register_module(
    connection: &Connection,
    registry: Arc<MatchRegistry>,
) -> SqlResult<()> {
    const MODULE: Module<MatchTable> = Module::eponymous_only_module();
    connection.create_module(MODULE_NAME, &MODULE, Some(registry))
}

#[repr(C)]
struct MatchTable {
    base: ffi::sqlite3_vtab,
    registry: Arc<MatchRegistry>,
}

unsafe impl<'vtab> VTab<'vtab> for MatchTable {
    type Aux = Arc<MatchRegistry>;
    type Cursor = MatchCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<MatchRegistry>>,
        _module_name: &[u8],
        _database_name: &[u8],
        _table_name: &[u8],
        _args: &[&[u8]],
    ) -> SqlResult<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(batch INTEGER HIDDEN,row_kind INTEGER NOT NULL,producer_id INTEGER,file_index INTEGER,task_index INTEGER,match_index INTEGER,capture_index INTEGER,capture_occurrence INTEGER,capture_sequence_start INTEGER,capture_sequence_count INTEGER,node INTEGER,related_node INTEGER,axis_kind INTEGER,axis_order INTEGER,fact_kind INTEGER,document_id INTEGER,mapped_document_id INTEGER,start_ordinal INTEGER,output_label TEXT,capture_label TEXT,text_value TEXT,start_byte INTEGER,end_byte INTEGER,verdict INTEGER,complete INTEGER NOT NULL,work_used INTEGER,error_code TEXT,field_id INTEGER,related_named INTEGER,source_text TEXT,structural_key TEXT,named_leaf INTEGER,capture_kind INTEGER,capture_named INTEGER,equality_occurrence INTEGER)"),
            Self {
                base: ffi::sqlite3_vtab::default(),
                registry: Arc::clone(aux.ok_or_else(|| {
                    rusqlite::Error::ModuleError("rerail_matches registry is missing".to_owned())
                })?),
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> SqlResult<bool> {
        for (index, constraint) in info.constraints().enumerate() {
            if constraint.column() == BATCH_COLUMN
                && constraint.is_usable()
                && constraint.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                let mut usage = info.constraint_usage(index);
                usage.set_argv_index(1);
                usage.set_omit(true);
                info.set_idx_num(USE_BATCH);
                info.set_estimated_cost(100_000.0);
                info.set_estimated_rows(100_000);
                return Ok(true);
            }
        }
        info.set_idx_num(0);
        info.set_estimated_cost(f64::MAX);
        info.set_estimated_rows(i64::MAX);
        Ok(true)
    }

    fn open(&mut self) -> SqlResult<Self::Cursor> {
        Ok(MatchCursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            registry: Arc::clone(&self.registry),
            rows: Arc::new(Vec::new()),
            position: 0,
            phantom: PhantomData,
        })
    }
}

#[repr(C)]
struct MatchCursor<'vtab> {
    base: ffi::sqlite3_vtab_cursor,
    registry: Arc<MatchRegistry>,
    rows: Arc<Vec<PackedRow>>,
    position: usize,
    phantom: PhantomData<&'vtab MatchTable>,
}

unsafe impl VTabCursor for MatchCursor<'_> {
    fn filter(
        &mut self,
        idx_num: c_int,
        _idx_str: Option<&str>,
        args: &Filters<'_>,
    ) -> SqlResult<()> {
        if idx_num != USE_BATCH {
            return Err(rusqlite::Error::ModuleError(
                "rerail_matches requires one batch equality constraint".to_owned(),
            ));
        }
        let token = args.get::<i64>(0)?;
        let batch = self.registry.batch(token).ok_or_else(|| {
            rusqlite::Error::ModuleError("rerail_matches batch is missing".to_owned())
        })?;
        batch.filters.fetch_add(1, Ordering::Relaxed);
        self.rows = batch.rows()?;
        self.position = 0;
        Ok(())
    }

    fn next(&mut self) -> SqlResult<()> {
        self.position += 1;
        Ok(())
    }

    fn eof(&self) -> bool {
        self.position >= self.rows.len()
    }

    fn column(&self, context: &mut Context, column: c_int) -> SqlResult<()> {
        let row = &self.rows[self.position];
        match column {
            0 => context.set_result(&row.batch),
            1 => context.set_result(&(row.row_kind as i64)),
            2 => context.set_result(&row.producer_id),
            3 => context.set_result(&row.file_index),
            4 => context.set_result(&row.task_index),
            5 => context.set_result(&row.match_index),
            6 => context.set_result(&row.capture_index),
            7 => context.set_result(&row.capture_occurrence),
            8 => context.set_result(&row.capture_sequence_start),
            9 => context.set_result(&row.capture_sequence_count),
            10 => context.set_result(&row.node),
            11 => context.set_result(&row.related_node),
            12 => context.set_result(&row.axis_kind),
            13 => context.set_result(&row.axis_order),
            14 => context.set_result(&row.fact_kind),
            15 => context.set_result(&row.document_id),
            16 => context.set_result(&row.mapped_document_id),
            17 => context.set_result(&row.start_ordinal),
            18 => context.set_result(&row.output_label.as_deref()),
            19 => context.set_result(&row.capture_label.as_deref()),
            20 => context.set_result(&row.text_value.as_deref()),
            21 => context.set_result(&row.start_byte),
            22 => context.set_result(&row.end_byte),
            23 => context.set_result(&row.verdict),
            24 => context.set_result(&row.complete),
            25 => context.set_result(&row.work_used),
            26 => context.set_result(&row.error_code.as_deref()),
            27 => context.set_result(&row.field_id),
            28 => context.set_result(&row.related_named),
            29 => context.set_result(&row.source_text.as_deref()),
            30 => context.set_result(&row.structural_key.as_deref()),
            31 => context.set_result(&row.named_leaf),
            32 => context.set_result(&row.capture_kind),
            33 => context.set_result(&row.capture_named),
            34 => context.set_result(&row.equality_occurrence),
            _ => Err(rusqlite::Error::InvalidColumnIndex(column as usize)),
        }
    }

    fn rowid(&self) -> SqlResult<i64> {
        Ok(self.position as i64 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::{MatchRegistry, PackedDemand, ProducerKind, RowKind, register_module};
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::packed::{ExecutionBackend, Program};
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use rusqlite::Connection;
    use std::sync::Arc;

    fn empty_demand() -> PackedDemand {
        PackedDemand::new(
            ExecutionBackend::CpuJit,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("valid empty demand")
    }

    #[test]
    fn demanded_navigation_producer_emits_typed_parent_child_edges() {
        let source = Arc::new("f(a, b)\n".to_owned());
        let grammar = Grammar::new(GrammarDialect::Python).expect("Python grammar");
        let image = TapeImage::parse(&grammar, &source, &|| false)
            .expect("parser completed")
            .expect("tape image");
        let tape = TapeDoc::restore(Arc::clone(&source), grammar, &image, &|| false)
            .expect("restored tape");
        let node_count = tape.transducer_len();
        let demand = PackedDemand::new(
            ExecutionBackend::CpuJit,
            vec![Program::compile_navigation().expect("verified navigation producer")],
            vec![tape],
            vec![vec![vec![0]]],
            vec![vec![true]],
            vec![u32::MAX],
        )
        .and_then(|demand| demand.with_producer_kinds(vec![ProducerKind::Navigation]))
        .expect("typed navigation demand");
        let rows = demand.execute(7);
        let navigation = rows
            .iter()
            .filter(|row| row.row_kind == RowKind::Navigation)
            .collect::<Vec<_>>();
        assert_eq!(navigation.len(), node_count - 1, "{rows:?}");
        assert!(navigation.iter().all(|row| {
            row.node.is_some()
                && row.related_node.is_some()
                && row.axis_order.is_some()
                && row.field_id.is_some()
                && row.related_named.is_some()
        }));
        assert!(rows.iter().any(|row| {
            row.row_kind == RowKind::ProducerStatus
                && row.producer_id == Some(0)
                && row.complete == 1
        }));
    }

    #[test]
    fn expanded_relation_has_one_typed_schema() {
        let connection = Connection::open_in_memory().expect("SQLite connection");
        register_module(&connection, Arc::new(MatchRegistry::default()))
            .expect("virtual table module");
        let columns = connection
            .prepare("SELECT name FROM pragma_table_xinfo('rerail_matches') ORDER BY cid")
            .expect("schema query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("schema rows")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("schema values");
        assert_eq!(
            columns,
            [
                "batch",
                "row_kind",
                "producer_id",
                "file_index",
                "task_index",
                "match_index",
                "capture_index",
                "capture_occurrence",
                "capture_sequence_start",
                "capture_sequence_count",
                "node",
                "related_node",
                "axis_kind",
                "axis_order",
                "fact_kind",
                "document_id",
                "mapped_document_id",
                "start_ordinal",
                "output_label",
                "capture_label",
                "text_value",
                "start_byte",
                "end_byte",
                "verdict",
                "complete",
                "work_used",
                "error_code",
                "field_id",
                "related_named",
                "source_text",
                "structural_key",
                "named_leaf",
                "capture_kind",
                "capture_named",
                "equality_occurrence",
            ]
        );
    }

    #[test]
    fn empty_batch_is_inert_until_sql_demands_one_explicit_batch_status() {
        let connection = Connection::open_in_memory().expect("SQLite connection");
        let registry = Arc::new(MatchRegistry::default());
        register_module(&connection, Arc::clone(&registry)).expect("virtual table module");
        let token = registry.register(empty_demand());
        assert_eq!(registry.query_execution_counters(token), Some((0, 0)));

        let rows = connection
            .prepare(
                "SELECT row_kind, producer_id, file_index, complete, error_code \
                 FROM rerail_matches WHERE batch = ?1 ORDER BY rowid",
            )
            .expect("expanded relation query")
            .query_map([token], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .expect("batch rows")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("batch relation");

        assert_eq!(rows, vec![(1, None, None, 1, None)]);
        let repeated = connection
            .query_row(
                "SELECT count(*) FROM rerail_matches WHERE batch = ?1",
                [token],
                |row| row.get::<_, i64>(0),
            )
            .expect("second cursor reads the materialized relation");
        assert_eq!(repeated, 1);
        assert_eq!(registry.query_execution_counters(token), Some((1, 2)));
    }
}
