use super::calls::{SeedScope, TermId, Through};
use super::json::invalid;
use super::matcher::{GrammarMatcher, Transform};
use super::module_path::ModulePath;
use crate::code_rule_dsl::language::GrammarDialect;
use ast_grep_core::replacer::TemplateFix;
use globset::GlobSet;
use napi::Result;
use regex::Regex;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct Limits {
    pub(super) max_ast_pattern_bytes: usize,
    pub(super) max_files: usize,
    pub(super) max_regex_pattern_bytes: usize,
    pub(super) max_source_bytes: usize,
    pub(super) max_visited_nodes: usize,
    pub(super) max_witnesses: usize,
}

#[derive(Clone)]
pub(super) struct Capabilities {
    pub(super) raw: Value,
    pub(super) limits: Limits,
}

#[derive(Clone)]
pub(super) struct CodeRuleScope {
    pub(super) include: GlobSet,
    pub(super) exclude: GlobSet,
}

impl CodeRuleScope {
    pub(super) fn matches(&self, path: &str) -> bool {
        self.include.is_match(path) && !self.exclude.is_match(path)
    }
}

#[derive(Clone)]
pub(super) struct ViolationSpec {
    pub(super) code: String,
    pub(super) message: String,
    pub(super) anchor: Anchor,
    /// The message's `$NAME` references; `None` when it is literal text.
    pub(super) message_template: Option<Template>,
    pub(super) fix: Option<Arc<FixPlan>>,
    /// Per dialect, the rule's regexes with named groups that test the witness node itself.
    pub(super) node_regexes: Vec<(GrammarDialect, Vec<Regex>)>,
    /// `each`: an ast query's capture, one violation per distinct binding of it at a root.
    pub(super) each: Option<String>,
    /// `fix.range`: the capture whose range the fix replaces, inside the witness; `None` replaces
    /// the witness.
    pub(super) fix_range: Option<String>,
    /// `suppress`: the other linter's suppression comments this violation honours.
    pub(super) suppress: Option<super::suppress::Suppress>,
    /// The query's `transform` where one of its sources is a regex group of the witness, computed
    /// again with each witness's groups.
    pub(super) group_transform: Option<Arc<Transform>>,
    /// A cross-file relation's projection: each witness carries these fields
    /// (`Witness::captures["$project"]`), read by the clause that joins or counts the relation.
    pub(super) project: Vec<Projection>,
}

/// Where a violation's `span` points. The violation id always derives from the witness span.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Anchor {
    /// The whole witness: the matched node or regex match.
    Span,
    /// Zero width at the witness start.
    Start,
    /// Zero width at the witness end.
    End,
    /// Zero width at the file's first byte, whatever the witness: a file-level violation, where
    /// the root node starts at its first token (Ruff's INP001 is at 0..0).
    FileStart,
    /// A captured metavariable (ast) or named group (regex); a multi-capture spans its first
    /// node to its last. A capture that did not bind for this witness falls back to the
    /// witness span and marks the violation with `variables.anchorFallback`.
    Capture(String),
}

impl Anchor {
    pub(super) fn parse(text: &str, path: &str) -> Result<Self> {
        match text {
            "witness.span" => Ok(Self::Span),
            "witness.start" => Ok(Self::Start),
            "witness.end" => Ok(Self::End),
            "file.start" => Ok(Self::FileStart),
            _ => match text.strip_prefix("capture.") {
                Some(name) if is_capture_name(name) => Ok(Self::Capture(name.to_owned())),
                _ => Err(invalid(format!(
                    "{path}: unknown anchor `{text}`; expected witness.span, witness.start, \
                     witness.end, file.start, or capture.<NAME>"
                ))),
            },
        }
    }
}

fn is_capture_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '_')
}

#[derive(Clone, Debug, PartialEq)]
enum Segment {
    Text(String),
    Var { name: String, multi: bool },
}

/// A message or fix split into literal text and metavariable references.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Template {
    segments: Vec<Segment>,
    /// Whether a reference is braced (`${NAME}`), which ast-grep's own templates do not read.
    braced: bool,
}

/// The prefix of the transform variables that carry a braced ast fix's literal text.
pub(super) const FIX_TEXT_PREFIX: &str = "__RERAIL_FIX_TEXT_";

impl Template {
    /// Splits `$NAME` / `$$$NAME` exactly as ast-grep-core 0.45.3's `create_template` does:
    /// a name is one or more of `A-Z`, `0-9`, `_`; a `$` that starts no name is literal text.
    /// Rerail also reads `${NAME}` and `$$${NAME}`, which end the name at the brace.
    pub(super) fn parse(template: &str) -> Self {
        Self::split(template, false)
    }

    /// A violation message: as `parse`, except that `\$` is a literal `$` (the backslash dropped),
    /// unless it starts `${NAME}` (a backslash before the variable), and a name starting with a
    /// digit, which no pattern or regex group can bind, stays literal text.
    pub(super) fn parse_message(template: &str) -> Self {
        Self::split(template, true)
    }

    fn split(template: &str, message: bool) -> Self {
        let mut segments = Vec::new();
        let mut braced = false;
        let mut literal = 0;
        let mut offset = 0;
        let bindable =
            |name: &str| !(message && name.starts_with(|char: char| char.is_ascii_digit()));
        while let Some(found) = template[literal + offset..].find('$') {
            let at = literal + offset + found;
            if let Some((name, multi, skipped)) =
                braced_var(&template[at..]).filter(|(name, ..)| bindable(name))
            {
                if at > literal {
                    segments.push(Segment::Text(template[literal..at].to_owned()));
                }
                segments.push(Segment::Var { name, multi });
                braced = true;
                literal = at + skipped;
                offset = 0;
                continue;
            }
            if message && template[..at].ends_with('\\') {
                segments.push(Segment::Text(template[literal..at - 1].to_owned()));
                literal = at;
                offset = 1;
                continue;
            }
            let variable = split_meta_var(&template[at..]).filter(|(name, ..)| bindable(name));
            if let Some((name, multi, skipped)) = variable {
                if at > literal {
                    segments.push(Segment::Text(template[literal..at].to_owned()));
                }
                segments.push(Segment::Var { name, multi });
                literal = at + skipped;
                offset = 0;
            } else {
                offset += found + 1;
            }
        }
        if literal < template.len() {
            segments.push(Segment::Text(template[literal..].to_owned()));
        }
        Self { segments, braced }
    }

    /// A braced template as ast-grep's `TemplateFix` reads it, with the text it would read as part
    /// of a name moved into transform variables (`FIX_TEXT_PREFIX`), and those variables' texts:
    /// the name characters right after a variable, and `$`s right before one. `None` for a template
    /// without braces, which ast-grep reads as written.
    pub(super) fn upstream(&self) -> Option<(String, Vec<(String, String)>)> {
        if !self.braced {
            return None;
        }
        let mut text = String::new();
        let mut literals: Vec<(String, String)> = Vec::new();
        let mut literal = |value: &str, text: &mut String| {
            let name = format!("{FIX_TEXT_PREFIX}{}", literals.len());
            text.push('$');
            text.push_str(&name);
            literals.push((name, value.to_owned()));
        };
        let is_name =
            |char: char| char.is_ascii_uppercase() || char.is_ascii_digit() || char == '_';
        for (index, segment) in self.segments.iter().enumerate() {
            match segment {
                Segment::Var { name, multi } => {
                    text.push_str(if *multi { "$$$" } else { "$" });
                    text.push_str(name);
                }
                Segment::Text(value) => {
                    let mut value = value.as_str();
                    let after_var = index > 0;
                    let before_var = index + 1 < self.segments.len();
                    if after_var {
                        let run = value.len() - value.trim_start_matches(is_name).len();
                        if run > 0 {
                            literal(&value[..run], &mut text);
                            value = &value[run..];
                        }
                    }
                    let dollars = value.len() - value.trim_end_matches('$').len();
                    if before_var && dollars > 0 {
                        text.push_str(&value[..value.len() - dollars]);
                        literal(&value[value.len() - dollars..], &mut text);
                    } else {
                        text.push_str(value);
                    }
                }
            }
        }
        Some((text, literals))
    }

    pub(super) fn is_literal(&self) -> bool {
        self.vars().next().is_none()
    }

    /// Each referenced metavariable and whether it is referenced as `$$$NAME`.
    pub(super) fn vars(&self) -> impl Iterator<Item = (&str, bool)> {
        self.segments.iter().filter_map(|segment| match segment {
            Segment::Var { name, multi } => Some((name.as_str(), *multi)),
            Segment::Text(_) => None,
        })
    }

    /// Renders the template; `lookup` returns a capture's text, `None` for an unbound one.
    pub(super) fn render<'a>(&self, lookup: impl Fn(&str, bool) -> Option<Cow<'a, str>>) -> String {
        let mut rendered = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Text(text) => rendered.push_str(text),
                Segment::Var { name, multi } => {
                    if let Some(text) = lookup(name, *multi) {
                        rendered.push_str(&text);
                    }
                }
            }
        }
        rendered
    }
}

/// `${NAME}` or `$$${NAME}` at the start of `source`: the name, whether it is a multi-capture, and
/// the length read.
fn braced_var(source: &str) -> Option<(String, bool, usize)> {
    let (multi, rest) = match source.strip_prefix("$$${") {
        Some(rest) => (true, rest),
        None => (false, source.strip_prefix("${")?),
    };
    let length = rest.find('}')?;
    let name = &rest[..length];
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|char| char.is_ascii_uppercase() || char.is_ascii_digit() || char == '_');
    valid.then(|| {
        (
            name.to_owned(),
            multi,
            source.len() - rest.len() + length + 1,
        )
    })
}

/// ast-grep-core 0.45.3 `split_first_meta_var` with `$` as the metavariable character.
fn split_meta_var(source: &str) -> Option<(String, bool, usize)> {
    let mut dollars = 0;
    let multi = loop {
        dollars += 1;
        if dollars == 3 {
            break true;
        }
        if !source[dollars..].starts_with('$') {
            break false;
        }
    };
    let length = source[dollars..]
        .find(|char: char| !(char.is_ascii_uppercase() || char.is_ascii_digit() || char == '_'))
        .unwrap_or(source.len() - dollars);
    (length > 0).then(|| {
        (
            source[dollars..dollars + length].to_owned(),
            multi,
            dollars + length,
        )
    })
}

/// A clause's suggested fix, compiled once. A fix replaces the witness span (as ast-grep's
/// replacement replaces the matched node) whatever the violation's anchor.
pub(super) enum FixPlan {
    /// ast-grep `TemplateFix` per grammar dialect the rule compiled for, and the literal texts a
    /// braced template moved into transform variables (`Template::upstream`).
    Ast(Vec<(GrammarDialect, TemplateFix)>, Vec<(String, String)>),
    /// Named-group substitution into a `file.content` regex match.
    Regex(Template),
}

/// One projected field of a cross-file relation row.
#[derive(Clone)]
pub(super) struct Projection {
    pub(super) field: String,
    pub(super) source: ProjectionSource,
}

#[derive(Clone)]
pub(super) enum ProjectionSource {
    /// A capture's text (a multi-node capture's nodes joined as in the source).
    Capture(String),
    /// The file's path, its directory, its file name, or its name without extensions.
    Path,
    Directory,
    Name,
    Stem,
    /// The Python module path, or the package holding it (the module path without its last
    /// segment).
    Module,
    Package,
}

#[derive(Clone)]
pub(super) struct FactQuery {
    pub(super) alias: String,
    pub(super) predicate: Option<Value>,
    /// Rows projected from an AST query's matches in every file of the code rule's scope.
    pub(super) ast: Arc<AstRelation>,
}

#[derive(Clone)]
pub(super) struct AstRelation {
    /// A `Query::Ast`.
    pub(super) query: Query,
    pub(super) project: Vec<Projection>,
    /// The files the relation reads, within the code rule's scope; `None` reads all of it.
    pub(super) domain: Option<CodeRuleScope>,
}

#[derive(Clone)]
pub(super) enum RegexSource {
    Content,
    Path,
}

#[derive(Clone)]
pub(super) enum Query {
    Regex {
        regex: Regex,
        source: RegexSource,
    },
    Ast {
        /// One compiled matcher per grammar the rule language spans; a file uses the grammar its label selects.
        matchers: Vec<GrammarMatcher>,
        /// The `calls` terms of the rule and its utilities, each once: their seeds run per file,
        /// their closure is read at call nodes.
        calls: Vec<Arc<CallsTerm>>,
    },
    SelectWhen {
        select: Box<Query>,
        exists: Box<Query>,
    },
}

impl Query {
    /// The `calls` terms the query's rules hold.
    pub(super) fn calls(&self) -> Vec<Arc<CallsTerm>> {
        match self {
            Query::Ast { calls, .. } => calls.clone(),
            Query::SelectWhen { select, exists } => {
                let mut terms = select.calls();
                terms.extend(exists.calls());
                terms
            }
            Query::Regex { .. } => Vec::new(),
        }
    }
}

/// A `calls` term: its inner rule as a seed query over definitions, and what chains pass through.
#[derive(Clone)]
pub(super) struct CallsTerm {
    /// Identifies the term's semantics: equal keys have equal seeds.
    pub(super) key: Arc<str>,
    /// Identifies its seed query: terms differing only in `through` share it.
    pub(super) seed_key: Arc<str>,
    pub(super) through: Through,
    /// Which definitions the seed query's witnesses stand for.
    pub(super) seed_scope: SeedScope,
    /// `{ all: [{ kind: function_definition }, <inner rule>] }` with the query's utilities: its
    /// witnesses are the definitions that match.
    pub(super) seed: Query,
}

/// Every workspace's `calls` terms, by key.
#[derive(Default)]
pub(super) struct Terms {
    pub(super) terms: Vec<Arc<CallsTerm>>,
    ids: HashMap<Arc<str>, TermId>,
}

impl Terms {
    /// Adds `term` unless an equal one is known: its id when new.
    pub(super) fn add(&mut self, term: &Arc<CallsTerm>) -> Option<TermId> {
        if self.ids.contains_key(&term.key) {
            return None;
        }
        let id = self.terms.len() as TermId;
        self.ids.insert(Arc::clone(&term.key), id);
        self.terms.push(Arc::clone(term));
        Some(id)
    }

    pub(super) fn id(&self, key: &str) -> Option<TermId> {
        self.ids.get(key).copied()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Each term's key and mode, by id: what a `CallsView` asks.
    pub(super) fn questions(&self) -> Vec<(Arc<str>, Through)> {
        self.terms
            .iter()
            .map(|term| (Arc::clone(&term.key), term.through))
            .collect()
    }
}

#[derive(Clone)]
pub(super) enum ClauseKind {
    Forbid(Query),
    RequireEach {
        subject: FactQuery,
        evidence: FactQuery,
    },
    Budget {
        query: FactQuery,
        group_by: Vec<String>,
        maximum: usize,
        /// `count.distinct`: count distinct values of these operands instead of rows.
        distinct: Vec<String>,
    },
    Llm,
}

#[derive(Clone)]
pub(super) struct CompiledClause {
    pub(super) id: String,
    pub(super) violation: ViolationSpec,
    pub(super) kind: ClauseKind,
    pub(super) synthetic: bool,
    /// A synthetic leaf under a `select_when`'s `exists` side: one witness answers it, so a file's
    /// evaluation stops at its first witness.
    pub(super) decision: bool,
}

#[derive(Clone, Copy)]
pub(super) enum RuleMode {
    Log,
    Steer,
    Block,
}

impl RuleMode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Log => "log",
            Self::Steer => "steer",
            Self::Block => "block",
        }
    }
}

#[derive(Clone)]
pub(super) struct CompiledCodeRule {
    pub(super) id: String,
    pub(super) rule_mode: RuleMode,
    pub(super) canonical: String,
    pub(super) digest: String,
    pub(super) required_capabilities: String,
    pub(super) scope: CodeRuleScope,
    pub(super) clauses: Vec<CompiledClause>,
    pub(super) capabilities: Capabilities,
}

#[derive(Clone)]
pub(super) struct FileInput {
    pub(super) path: String,
    pub(super) language: String,
    pub(super) source: Arc<String>,
    /// A Python file's module path: the host's (`module`), else the file's stem.
    pub(super) module: Option<Arc<ModulePath>>,
}

impl FileInput {
    /// `module` is the module path a host derived for a Python file (Ruff's package walk): a
    /// dotted string, or a list of names when one keeps a dot (`["json.d", "__init__"]`). Without
    /// it a Python file is named by its stem, as Ruff names a file outside any package; a file set
    /// that holds package markers derives it first (`python_module_in`).
    pub(super) fn new(
        path: String,
        language: String,
        source: Arc<String>,
        module: Option<&Value>,
        at: &str,
    ) -> Result<Self> {
        let module = match module {
            Some(module) => Some(module_path(module).ok_or_else(|| {
                invalid(format!(
                    "{at}/module must be a dotted module path (non-empty names joined by `.`) or a list of names"
                ))
            })?),
            None => crate::code_rule_dsl::language::dialect_by_label(&language).and_then(|dialect| {
                use crate::code_rule_dsl::language::adapter::{Adapter, ScopeRules, with_adapter};
                with_adapter!(crate::code_rule_dsl::language::Family::of(dialect), A => <<A as Adapter>::Scope as ScopeRules>::unpackaged_module(&path))
            }),
        };
        Ok(Self {
            path,
            language,
            source,
            module: module.map(Arc::new),
        })
    }
}

fn module_path(value: &Value) -> Option<ModulePath> {
    match value {
        Value::String(module) => ModulePath::dotted(module),
        Value::Array(names) => ModulePath::names(
            &names
                .iter()
                .map(Value::as_str)
                .collect::<Option<Vec<_>>>()?,
        ),
        _ => None,
    }
}

/// Ruff's module path for a Python file (`detect_package_root`), in the shape hosts send as
/// `module`: the package root is the highest directory reached from the file's own by climbing
/// while each holds an `__init__.py`. The module is the names from that root's parent down, the
/// file's by its stem (`pkg/__init__.py` is `pkg.__init__`): dotted, or a list when a name keeps a
/// dot (`["json.d", "__init__"]`), since N999 reads such names whole. `None` outside a package,
/// where the engine names the module by the file's stem. Every ingestion path derives it here.
pub(crate) fn python_module_path(
    file: &std::path::Path,
    is_package: impl Fn(&std::path::Path) -> bool,
) -> Option<Value> {
    let mut root = None;
    let mut directory = file.parent()?;
    while is_package(directory) {
        root = Some(directory);
        match directory.parent() {
            Some(parent) if parent != directory => directory = parent,
            _ => break,
        }
    }
    let relative = file.strip_prefix(root?.parent()?).ok()?;
    let mut names = relative
        .components()
        .map(|component| component.as_os_str().to_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    let name = names.pop()?;
    let stem = match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name.as_str(),
    };
    names.push(stem.to_owned());
    if names.iter().any(String::is_empty) {
        return None;
    }
    Some(if names.iter().any(|name| name.contains('.')) {
        Value::Array(names.into_iter().map(Value::String).collect())
    } else {
        Value::String(names.join("."))
    })
}

/// `python_module_path` over the files on disk.
pub(crate) fn python_module_path_on_disk(file: &std::path::Path) -> Option<Value> {
    python_module_path(file, |directory| directory.join("__init__.py").exists())
}

/// The directory a file set's path marks as a Python package (`pkg` for `pkg/__init__.py`, `""`
/// for a root `__init__.py`), if any.
pub(crate) fn package_marker_directory(path: &str) -> Option<&str> {
    path.strip_suffix("/__init__.py")
        .or_else(|| (path == "__init__.py").then_some(""))
}

/// `python_module_path` of a relative `path` in a file set (a workspace, a repository snapshot)
/// whose package markers `is_package` reads: the module a host walking the same files on disk
/// would send.
pub(crate) fn python_module_in(path: &str, is_package: impl Fn(&str) -> bool) -> Option<Value> {
    python_module_path(std::path::Path::new(path), |directory| {
        is_package(directory.to_string_lossy().as_ref())
    })
}

#[derive(Clone)]
pub(super) struct Snapshot {
    pub(super) revision: u32,
    pub(super) files: Vec<FileInput>,
}

#[derive(Clone, Default)]

pub(super) struct Metrics {
    pub(super) files: usize,
    pub(super) visited_nodes: usize,
    pub(super) witnesses: usize,
    pub(super) line_index_nanoseconds: u64,
    pub(super) regex_nanoseconds: u64,
    pub(super) ast_parse_nanoseconds: u64,
    pub(super) ast_match_nanoseconds: u64,
    pub(super) ast_matcher_calls: usize,
    pub(super) storage_retries: usize,
    pub(super) storage_retry_wait_nanoseconds: u64,
    pub(super) storage_transaction_nanoseconds: u64,
    pub(super) storage_object_transaction_nanoseconds: u64,
    pub(super) storage_relationship_transaction_nanoseconds: u64,
    pub(super) storage_read_data_statements: usize,
    pub(super) storage_data_statements: usize,
    pub(super) storage_max_data_statements_per_pass: usize,
    pub(super) storage_transaction_statements: usize,
    pub(super) storage_reconciliation_passes: usize,
    pub(super) storage_read_nanoseconds: u64,
    pub(super) storage_manifest_nanoseconds: u64,
    pub(super) storage_write_nanoseconds: u64,
    pub(super) reused_sources: usize,
    pub(super) reused_tapes: usize,
    pub(super) compressed_sources: usize,
    pub(super) compressed_tapes: usize,
    pub(super) restored_tapes: usize,
    pub(super) restored_candidate_files: usize,
    pub(super) avoided_tape_decompressions: usize,
    pub(super) tape_cache_hits: usize,
    pub(super) tape_decompressions: usize,
    pub(super) restored_local_results: usize,
}

impl Metrics {
    pub(super) fn merge(&mut self, other: Self) {
        self.files += other.files;
        self.visited_nodes += other.visited_nodes;
        self.witnesses += other.witnesses;
        self.line_index_nanoseconds += other.line_index_nanoseconds;
        self.regex_nanoseconds += other.regex_nanoseconds;
        self.ast_parse_nanoseconds += other.ast_parse_nanoseconds;
        self.ast_match_nanoseconds += other.ast_match_nanoseconds;
        self.ast_matcher_calls += other.ast_matcher_calls;
        self.storage_retries += other.storage_retries;
        self.storage_retry_wait_nanoseconds += other.storage_retry_wait_nanoseconds;
        self.storage_transaction_nanoseconds += other.storage_transaction_nanoseconds;
        self.storage_object_transaction_nanoseconds += other.storage_object_transaction_nanoseconds;
        self.storage_relationship_transaction_nanoseconds +=
            other.storage_relationship_transaction_nanoseconds;
        self.storage_read_data_statements += other.storage_read_data_statements;
        self.storage_data_statements += other.storage_data_statements;
        self.storage_max_data_statements_per_pass = self
            .storage_max_data_statements_per_pass
            .max(other.storage_max_data_statements_per_pass);
        self.storage_transaction_statements += other.storage_transaction_statements;
        self.storage_reconciliation_passes += other.storage_reconciliation_passes;
        self.storage_read_nanoseconds += other.storage_read_nanoseconds;
        self.storage_manifest_nanoseconds += other.storage_manifest_nanoseconds;
        self.storage_write_nanoseconds += other.storage_write_nanoseconds;
        self.reused_sources += other.reused_sources;
        self.reused_tapes += other.reused_tapes;
        self.compressed_sources += other.compressed_sources;
        self.compressed_tapes += other.compressed_tapes;
        self.restored_tapes += other.restored_tapes;
        self.restored_candidate_files += other.restored_candidate_files;
        self.avoided_tape_decompressions += other.avoided_tape_decompressions;
        self.tape_cache_hits += other.tape_cache_hits;
        self.tape_decompressions += other.tape_decompressions;
        self.restored_local_results += other.restored_local_results;
    }
}

#[derive(Clone, Default)]
pub(super) struct Evaluation {
    pub(super) violations: Vec<Value>,
    pub(super) failures: Vec<Value>,
    pub(super) metrics: Metrics,
    /// The clauses the result covers, as (rule id, clause id): the rule id is given where the
    /// result spans several code rules, as its failures give it. Each has a `coverage` entry.
    pub(super) clauses: Vec<(Option<String>, String)>,
}
