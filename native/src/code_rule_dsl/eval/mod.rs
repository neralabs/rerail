pub(super) mod evidence;
mod output;
mod query;

use super::calls;
use super::json::{invalid, json_error, required_str};
use super::language::adapter::package_manifests;
use super::model::{
    ClauseKind, CompiledClause, CompiledCodeRule, Evaluation, FactQuery, FileInput, Metrics,
    Snapshot, Terms, package_marker_directory, python_module_in,
};
use super::module_graph;
use super::package_manifest::{Manifest, manifest_directory, manifest_in};
use napi::Result;
pub(super) use output::{
    coverage, evaluation_json, failure, overflow_evaluation, result_order, violation_from_witness,
    violation_ids,
};
pub(super) use query::{
    WitnessError, evaluate_query, file_language, grammar_matcher, witness_from_match,
    witnesses_from_match,
};
#[cfg(test)]
pub(super) use query::{reset_shared_group_lookups, shared_group_lookups};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// A repository snapshot's files. A Python file without a host `module` is named, as the workspace
/// names it, by the package markers (`__init__.py`) the snapshot holds (`python_module_in`), and
/// an ECMA file without a host `package` reads the nearest `package.json` entry the snapshot holds
/// (`DerivedPackages`), so a stateless evaluation of a file set and a workspace holding the same
/// files read the same modules and manifests. With `server_graph`, a derived manifest naming
/// `next` carries the server graph the snapshot's files give. Also returns what the derivation
/// read, so an editor's edit derives again only what it changes (`DerivedPackages::edited`).
pub(super) fn parse_snapshot(
    source: &str,
    server_graph: bool,
) -> Result<(Snapshot, DerivedPackages)> {
    let value: Value = serde_json::from_str(source).map_err(json_error)?;
    let revision = value.get("revision").and_then(Value::as_u64).unwrap_or(0) as u32;
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("snapshot.files must be an array"))?;
    let packages = files
        .iter()
        .filter_map(|file| package_marker_directory(file.get("path")?.as_str()?))
        .collect::<HashSet<_>>();
    let mut derived = Vec::new();
    let files = files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let path = required_str(file, "path", "snapshot.files")?;
            let language = required_str(file, "language", "snapshot.files")?;
            let at = format!("snapshot.files/{index}");
            // A `module` of another type is refused like a malformed one.
            let module = (crate::code_rule_dsl::language::adapter::packages(language)
                && file.get("module").is_none()
                && !packages.is_empty())
            .then(|| python_module_in(path, |directory| packages.contains(directory)))
            .flatten();
            let package = match file.get("package") {
                Some(package) => Some(Arc::new(Manifest::from_wire(package, &at)?)),
                None => {
                    if package_manifests(language) {
                        derived.push(index);
                    }
                    None
                }
            };
            FileInput::new(
                path.to_owned(),
                language.to_owned(),
                required_str(file, "source", "snapshot.files")?
                    .to_owned()
                    .into(),
                file.get("module").or(module.as_ref()),
                package,
                &at,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let mut snapshot = Snapshot { revision, files };
    let derived = DerivedPackages::new(&mut snapshot, derived, server_graph);
    Ok((snapshot, derived))
}

#[cfg(test)]
thread_local! {
    /// Package manifests `DerivedPackages` derived on this thread.
    pub(super) static DERIVED_MANIFESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The `package` a snapshot gives each of its ECMA files without a host one: the nearest
/// `package.json` entry it holds, read as on disk (`Manifest::parse`, `manifest_in`), with the
/// `jsxImportSource` of the package's `tsconfig.json` or `jsconfig.json` entry, and with its
/// package's server graph over the snapshot's files where `server_graph` asks for it
/// (`module_graph::in_file_set`). It keeps what it read, so an editor's edit derives again only
/// the packages the edit changes (`edited`), as a fresh snapshot would derive them.
pub(super) struct DerivedPackages {
    /// The indices of the files whose `package` the snapshot derives.
    files: Vec<usize>,
    /// Whether a derived manifest naming `next` carries its package's server graph.
    server_graph: bool,
    /// Each `package.json` entry as parsed, with its package's configuration, by directory.
    packages: HashMap<String, (Manifest, module_graph::Config)>,
    /// Each package's manifest as its files carry it, by directory.
    manifests: HashMap<String, Arc<Manifest>>,
    /// What server graphs read of the snapshot's files below a package naming `next`, by path.
    summaries: HashMap<String, Option<module_graph::Summary>>,
}

impl DerivedPackages {
    /// Derives the `package` of the snapshot's files at `files`; none where it has none to derive.
    fn new(snapshot: &mut Snapshot, files: Vec<usize>, server_graph: bool) -> Self {
        let mut derived = Self {
            files,
            server_graph,
            packages: HashMap::new(),
            manifests: HashMap::new(),
            summaries: HashMap::new(),
        };
        if derived.files.is_empty() {
            return derived;
        }
        let texts = config_texts(snapshot);
        let read = |path: &str| texts.get(path).map(|text| (*text).to_owned());
        for file in &snapshot.files {
            if let Some(directory) = manifest_directory(&file.path) {
                let manifest = Manifest::parse(directory.to_owned(), file.source.as_bytes());
                let config = module_graph::Config::read(directory, &read);
                derived
                    .packages
                    .insert(directory.to_owned(), (manifest, config));
            }
        }
        let directories = derived.packages.keys().cloned().collect();
        derived.derive(snapshot, directories);
        derived
    }

    /// Derives again what an edit of the file at `index` changes: its package's manifest, where
    /// it is a `package.json` entry; the packages whose configuration a project configuration
    /// changes (its own, or one it `extends`); and, where a server graph reads the source, its
    /// package's graph, only where the edit changes the source's summary.
    pub(super) fn edited(&mut self, snapshot: &mut Snapshot, index: usize) {
        if self.files.is_empty() {
            return;
        }
        let file = &snapshot.files[index];
        let mut changed = BTreeSet::new();
        if let Some(directory) = manifest_directory(&file.path) {
            let parsed = Manifest::parse(directory.to_owned(), file.source.as_bytes());
            if let Some((manifest, _)) = self.packages.get_mut(directory) {
                *manifest = parsed;
            }
            changed.insert(directory.to_owned());
        }
        if module_graph::is_module_config(&file.path) {
            let texts = config_texts(snapshot);
            let read = |path: &str| texts.get(path).map(|text| (*text).to_owned());
            for (directory, (_, config)) in &mut self.packages {
                let read_again = module_graph::Config::read(directory, &read);
                if read_again != *config {
                    *config = read_again;
                    changed.insert(directory.clone());
                }
            }
        }
        // Only sources below a package naming `next` have a summary, and only the nearest
        // package's graph reads one (`module_graph::from_summaries`).
        if let Some(summary) = self.summaries.get_mut(&file.path) {
            let again = module_graph::summarize_bounded(&file.path, &file.source);
            if again != *summary {
                *summary = again;
                changed.extend(
                    manifest_in(&file.path, |directory| self.manifests.get(directory))
                        .filter(|manifest| manifest.has_server_graph())
                        .map(|manifest| manifest.folder().to_owned()),
                );
            }
        }
        if !changed.is_empty() {
            self.derive(snapshot, changed);
        }
    }

    /// Derives the manifests of the packages in `directories` again, then gives each derived file
    /// its package's.
    fn derive(&mut self, snapshot: &mut Snapshot, directories: BTreeSet<String>) {
        let Self {
            files,
            server_graph,
            packages,
            manifests,
            summaries,
        } = self;
        for directory in directories {
            #[cfg(test)]
            DERIVED_MANIFESTS.set(DERIVED_MANIFESTS.get() + 1);
            let (parsed, config) = &packages[&directory];
            let manifest = parsed
                .clone()
                .with_jsx_import_source(config.jsx_import_source());
            let manifest = if *server_graph && manifest.has_server_graph() {
                let members = module_graph::in_file_set(
                    &directory,
                    snapshot
                        .files
                        .iter()
                        .map(|file| (file.path.as_str(), file.source.as_str())),
                    summaries,
                    &|directory| packages.contains_key(directory),
                    config,
                );
                manifest.with_server_graph(members)
            } else {
                manifest
            };
            manifests.insert(directory, Arc::new(manifest));
        }
        for &index in files.iter() {
            let file = &mut snapshot.files[index];
            file.package = manifest_in(&file.path, |directory| manifests.get(directory));
        }
    }
}

/// The snapshot's project configuration entries' texts, by path.
fn config_texts(snapshot: &Snapshot) -> HashMap<&str, &str> {
    snapshot
        .files
        .iter()
        .filter(|file| module_graph::is_module_config(&file.path))
        .map(|file| (file.path.as_str(), file.source.as_str()))
        .collect()
}

/// Evaluates `code_rule`, its grammar matchers matched natively (`matcher::run_clause`).
pub(super) fn evaluate_code_rule(
    code_rule: &CompiledCodeRule,
    snapshot: &Snapshot,
    mode: &str,
) -> Result<Evaluation> {
    evaluate_clauses(code_rule, snapshot, mode, &code_rule.clauses)
}

pub(super) fn evaluate_clauses(
    code_rule: &CompiledCodeRule,
    snapshot: &Snapshot,
    mode: &str,
    clauses: &[CompiledClause],
) -> Result<Evaluation> {
    if mode != "decision" && mode != "report" {
        return Err(invalid(format!("unsupported evaluation mode `{mode}`")));
    }
    let limits = &code_rule.capabilities.limits;
    // The budget is what this code rule will read: the files in its scope, not the whole snapshot.
    let (in_scope_files, in_scope_bytes) = snapshot
        .files
        .iter()
        .filter(|file| code_rule.scope.matches(&file.path))
        .fold((0usize, 0usize), |(files, bytes), file| {
            (files + 1, bytes + file.source.len())
        });
    // Each reported clause has a coverage entry; a synthetic leaf is part of the clause it serves.
    let covered: Vec<(Option<String>, String)> = clauses
        .iter()
        .filter(|clause| !clause.synthetic)
        .map(|clause| (None, clause.id.clone()))
        .collect();
    if in_scope_files > limits.max_files || in_scope_bytes > limits.max_source_bytes {
        return Ok(Evaluation {
            clauses: covered,
            ..overflow_evaluation("runtime", "repository input budget exceeded")
        });
    }
    let mut evaluation = Evaluation {
        violations: Vec::new(),
        failures: Vec::new(),
        metrics: Metrics {
            files: in_scope_files,
            ..Metrics::default()
        },
        clauses: covered,
    };
    let _calls = stateless_calls(code_rule, snapshot, clauses, &mut evaluation.metrics)?;
    for clause in clauses {
        match &clause.kind {
            ClauseKind::Forbid(query) => {
                let mut partial = BTreeMap::new();
                let mut util_limit = None;
                let matches = evaluate_query(
                    query,
                    snapshot,
                    &BTreeMap::new(),
                    &code_rule.scope,
                    &mut evaluation.metrics,
                    limits,
                    mode == "decision",
                    &mut partial,
                    Some(&clause.violation),
                    &mut util_limit,
                )?;
                if let Some((path, limit)) = util_limit {
                    evaluation.failures.push(failure(
                        &clause.id,
                        "budget-exceeded",
                        limit.message(),
                        Some(&path),
                    ));
                    continue;
                }
                let matches = match matches {
                    Ok(matches) => matches,
                    Err(incomplete) => {
                        // A file the parser did not read reports how much of it the errors cover.
                        evaluation.failures.push(failure(
                            &clause.id,
                            "source-incomplete",
                            incomplete
                                .unread
                                .as_deref()
                                .unwrap_or("query source is incomplete"),
                            Some(&incomplete.path),
                        ));
                        continue;
                    }
                };
                for (path, message) in &partial {
                    evaluation.failures.push(failure(
                        &clause.id,
                        "source-partial",
                        message,
                        Some(path),
                    ));
                }
                if evaluation.metrics.witnesses > limits.max_witnesses {
                    evaluation.failures.push(failure(
                        &clause.id,
                        crate::code_rule_dsl::model::WITNESS_LIMIT,
                        "witness budget exceeded",
                        None,
                    ));
                    continue;
                }
                for witness in matches {
                    evaluation
                        .violations
                        .push(violation_from_witness(code_rule, clause, witness));
                    if mode == "decision" {
                        break;
                    }
                }
            }
            ClauseKind::RequireEach { subject, evidence } => {
                let subject_rows = stateless_rows(
                    code_rule,
                    clause,
                    (subject, evidence::Side::Subject),
                    snapshot,
                    &mut evaluation.metrics,
                )?;
                let evidence_rows = stateless_rows(
                    code_rule,
                    clause,
                    (evidence, evidence::Side::Evidence),
                    snapshot,
                    &mut evaluation.metrics,
                )?;
                merge(
                    &mut evaluation,
                    evidence::require_each(
                        code_rule,
                        clause,
                        (subject, subject_rows),
                        (evidence, &evidence_rows),
                    ),
                );
            }
            ClauseKind::Budget { query, .. } => {
                let rows = stateless_rows(
                    code_rule,
                    clause,
                    (query, evidence::Side::Counted),
                    snapshot,
                    &mut evaluation.metrics,
                )?;
                merge(
                    &mut evaluation,
                    evidence::budget(code_rule, clause, query, rows),
                );
            }
            ClauseKind::Llm => evaluation.failures.push(failure(
                &clause.id,
                "missing-capability",
                "LLM capability is unavailable",
                None,
            )),
        }
    }
    if evaluation.metrics.witnesses > limits.max_witnesses
        && !evaluation.failures.iter().any(|failure| {
            failure.get("code").and_then(Value::as_str)
                == Some(crate::code_rule_dsl::model::WITNESS_LIMIT)
        })
    {
        evaluation.failures.push(failure(
            "runtime",
            crate::code_rule_dsl::model::WITNESS_LIMIT,
            "witness budget exceeded",
            None,
        ));
    }
    Ok(evaluation)
}

fn merge(evaluation: &mut Evaluation, mut other: Evaluation) {
    evaluation.violations.append(&mut other.violations);
    evaluation.failures.append(&mut other.failures);
    evaluation.metrics.witnesses += other.metrics.witnesses;
}

/// An evidence relation's rows in a stateless snapshot: each file of the code rule's scope and the
/// relation's domain evaluated alone, so that one file's failure makes only its rows unknown.
fn stateless_rows(
    code_rule: &CompiledCodeRule,
    clause: &CompiledClause,
    (relation, side): (&FactQuery, evidence::Side),
    snapshot: &Snapshot,
    metrics: &mut Metrics,
) -> Result<evidence::Rows> {
    let ast = relation.ast.as_ref();
    let violation = evidence::producer_violation(clause, relation, side);
    let limits = &code_rule.capabilities.limits;
    let mut rows = evidence::Rows::default();
    // Rows in path order, as the workspace's evidence index keeps them, whatever the snapshot's
    // file order: a group's anchor is its first row.
    let mut files = snapshot
        .files
        .iter()
        .filter(|file| {
            code_rule.scope.matches(&file.path)
                && ast
                    .domain
                    .as_ref()
                    .is_none_or(|domain| domain.matches(&file.path))
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    for file in files {
        let single = Snapshot {
            revision: snapshot.revision,
            files: vec![file.clone()],
        };
        let mut partial = BTreeMap::new();
        let mut util_limit = None;
        let mut file_metrics = Metrics::default();
        let witnesses = evaluate_query(
            &ast.query,
            &single,
            &BTreeMap::new(),
            &code_rule.scope,
            &mut file_metrics,
            limits,
            false,
            &mut partial,
            Some(&violation),
            &mut util_limit,
        )?;
        metrics.visited_nodes += file_metrics.visited_nodes;
        match witnesses {
            Ok(witnesses)
                if util_limit.is_none()
                    && partial.is_empty()
                    && file_metrics.witnesses <= limits.max_witnesses =>
            {
                rows.witnesses.extend(witnesses);
            }
            _ => {
                rows.incomplete.insert(file.path.clone());
            }
        }
    }
    Ok(rows)
}

/// The closure the `calls` terms of `clauses` read, over the snapshot's Python and PHP files in
/// the code rule's scope, installed for their evaluation. Each file's seeds are its own evaluation of
/// each term's seed query, so one file's failure makes only its definitions unknown. Seed work
/// spends the evaluation's work budget (`metrics.visited_nodes`); seed witnesses are no violation,
/// so each seed evaluation has its own witness budget, as a workspace's seed producer has per
/// file, and spends none of the clauses'.
fn stateless_calls(
    code_rule: &CompiledCodeRule,
    snapshot: &Snapshot,
    clauses: &[CompiledClause],
    metrics: &mut Metrics,
) -> Result<Option<calls::EvaluationSlot>> {
    let mut terms = Terms::default();
    for clause in clauses {
        if let ClauseKind::Forbid(query) = &clause.kind {
            for term in query.calls() {
                terms.add(&term);
            }
        }
    }
    if terms.is_empty() {
        return Ok(None);
    }
    let limits = &code_rule.capabilities.limits;
    let mut summaries = Vec::new();
    let mut seeds = std::collections::HashMap::<Arc<str>, calls::FileSeeds>::new();
    for file in &snapshot.files {
        let Some(language) = crate::code_rule_dsl::language::dialect_by_label(&file.language)
        else {
            continue;
        };
        if !calls::summarizes(language) || !code_rule.scope.matches(&file.path) {
            continue;
        }
        let path: Arc<str> = Arc::from(file.path.as_str());
        let module = file.module.as_deref();
        // A Blade file's calls are its tape's embedded program's (`calls::taped`).
        let (summary, _) = if calls::taped(language) {
            let (doc, _) = query::file_tape(language, file)?;
            let has_errors = !doc.syntax_error_ranges().is_empty();
            let root = ast_grep_core::AstGrep::doc(doc);
            let bytes = file.source.len();
            calls::summarize(
                root.root(),
                language,
                &file.path,
                module,
                has_errors,
                bytes,
                None,
            )
        } else {
            let root = crate::code_rule_dsl::language::adapter::ast_grep(language, &file.source);
            let has_errors = root.root().get_doc().tree.root_node().has_error();
            let bytes = file.source.len();
            calls::summarize(
                root.root(),
                language,
                &file.path,
                module,
                has_errors,
                bytes,
                None,
            )
        };
        let summary = Arc::new(summary);
        summaries.push((Arc::clone(&path), Arc::clone(&summary)));
        let single = Snapshot {
            revision: snapshot.revision,
            files: vec![file.clone()],
        };
        let file_seeds = seeds.entry(path).or_default();
        // Terms differing only in `through` share one seed query.
        let mut evaluated = std::collections::HashMap::<Arc<str>, Option<Vec<u32>>>::new();
        for (id, term) in terms.terms.iter().enumerate() {
            let id = id as calls::TermId;
            if let Some(starts) = evaluated.get(&term.seed_key) {
                match starts {
                    Some(starts) => starts
                        .iter()
                        .for_each(|start| file_seeds.matched(*start, id)),
                    None => file_seeds.unknown.push(id),
                }
                continue;
            }
            let (mut partial, mut util_limit) = (BTreeMap::new(), None);
            let mut seed_metrics = Metrics {
                visited_nodes: metrics.visited_nodes,
                ..Metrics::default()
            };
            let witnesses = evaluate_query(
                &term.seed,
                &single,
                &BTreeMap::new(),
                &code_rule.scope,
                &mut seed_metrics,
                limits,
                false,
                &mut partial,
                None,
                &mut util_limit,
            )?;
            metrics.visited_nodes = seed_metrics.visited_nodes;
            let starts = match witnesses {
                // Past the witness budget the witnesses stop short: the file's seeds are unknown.
                Ok(witnesses)
                    if util_limit.is_none()
                        && partial.is_empty()
                        && seed_metrics.witnesses <= limits.max_witnesses =>
                {
                    let starts = calls::seed_starts(
                        witnesses.iter().map(|witness| &witness.span),
                        term.seed_scope,
                        Some(summary.as_ref()),
                    )
                    .unwrap_or_default();
                    for start in &starts {
                        file_seeds.matched(*start, id);
                    }
                    Some(starts)
                }
                _ => {
                    file_seeds.unknown.push(id);
                    None
                }
            };
            evaluated.insert(Arc::clone(&term.seed_key), starts);
        }
    }
    let view = calls::CallsView::build(terms.questions(), summaries.clone(), &seeds);
    Ok(Some(calls::EvaluationSlot::install(
        calls::EvaluationCalls {
            view: Arc::new(view),
            summaries: summaries
                .into_iter()
                .map(|(path, summary)| (path.to_string(), summary))
                .collect(),
        },
    )))
}

#[cfg(test)]
mod tests;
