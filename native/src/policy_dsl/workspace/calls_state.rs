//! Cross-file `calls` in a workspace. Each Python and PHP file's call summary and each term's seeds are
//! recorded as its files are evaluated; the closure is rebuilt from them when either changed, and
//! the checks that read it (consumers) run after it, on every file whose stamp (what its calls
//! read of the closure) changed. A consumer's stored result is kept under its file context and
//! stamp, so any workspace whose closure gives the same stamp reuses it.

use super::evaluate::encode_tape;
use super::ingest::candidate_dispatch;
use super::persisted::file_context;
use super::{CheckId, FileState};
use crate::WORK_POOL;
use crate::content_store::{CandidateCheck, HashKey, LocalResultPublication, StoredLocalResult};
use crate::policy_dsl::budget::matcher_budget_exhausted;
use crate::policy_dsl::calls::summary::Summary;
use crate::policy_dsl::calls::{self, CallsView, FileSeeds, TermId};
use crate::policy_dsl::eval::{file_language, grammar_matcher};
use crate::policy_dsl::json::invalid;
use crate::policy_dsl::language::{Family, GrammarDialect};
use crate::policy_dsl::lines::{LineTable, Terminators};
use crate::policy_dsl::model::{CheckKind, CompiledPolicy, FileInput, Metrics, Query, Terms};
use crate::policy_dsl::tape;
use ast_grep_core::AstGrep as Root;
use ast_grep_language::LanguageExt;
use napi::Result;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Cursor;
use std::sync::atomic::AtomicBool;
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::sync::{Arc, LazyLock};

/// A consumer file whose results must follow the graph: its new stamp, and the terms silent in
/// it (no call reaches or may reach their seeds).
pub(super) struct Restamp {
    pub(super) path: String,
    pub(super) stamp: HashKey,
    pub(super) silent: Vec<TermId>,
}

/// Checks holding `calls` terms, with the terms each reads.
pub(super) type Consumers = BTreeMap<CheckId, Vec<TermId>>;

/// The stamp a file's consumer results are under, with the terms silent in the file there.
type Stamped = (HashKey, Arc<[TermId]>);

#[derive(Default)]
pub(super) struct CallsState {
    terms: Terms,
    /// Per term, the synthetic check running its seed query: one per distinct seed query.
    producers: Vec<CheckId>,
    /// The synthetic check of each seed query.
    seed_producers: HashMap<Arc<str>, CheckId>,
    /// The checks whose rules hold `calls` terms, evaluated after the graph, with their terms.
    consumers: Consumers,
    /// Distinct sorted term lists, shared by the files whose consumers read them.
    term_lists: HashMap<Vec<TermId>, Arc<[TermId]>>,
    /// The consumers that read `calls` only positively: they match nothing in a quiet file.
    positive: BTreeSet<CheckId>,
    /// Per file, a hash of what the closure reads of it: its definitions, calls, aliases and star
    /// imports by name, and its seeds by definition.
    fingerprints: HashMap<String, u64>,
    summaries: BTreeMap<String, Arc<Summary>>,
    seeds: HashMap<Arc<str>, FileSeeds>,
    /// The files a consumer applies to: the stamp their consumer results are under, with the terms
    /// silent in the file under it, and the terms their consumers read. The silent terms answer
    /// the quiet consumers, which are not stored, after eviction drops the graph.
    stamps: BTreeMap<String, (Option<Stamped>, Arc<[TermId]>)>,
    view: Option<Arc<CallsView>>,
    /// The files whose summary or seeds changed since `view` was built.
    dirty: BTreeSet<Arc<str>>,
    /// The files whose summary was replaced by one the closure reads the same (an edit of a
    /// comment, or of a definition's body that calls the same): `view` swaps it in unlinked.
    resummarized: BTreeSet<Arc<str>>,
}

impl CallsState {
    pub(super) fn active(&self) -> bool {
        !self.terms.is_empty()
    }

    /// The consumers, with the terms each reads.
    pub(super) fn consumers(&self) -> &Consumers {
        &self.consumers
    }

    /// The terms `checks` read, as one shared sorted list.
    fn terms_of<'c>(&mut self, checks: impl IntoIterator<Item = &'c CheckId>) -> Arc<[TermId]> {
        let mut terms = checks
            .into_iter()
            .filter_map(|id| self.consumers.get(id))
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        terms.sort_unstable();
        terms.dedup();
        self.term_list(terms)
    }

    /// `terms`, sorted and distinct, as the list shared by every file with the same.
    fn term_list(&mut self, terms: Vec<TermId>) -> Arc<[TermId]> {
        Arc::clone(
            self.term_lists
                .entry(terms)
                .or_insert_with_key(|terms| terms.as_slice().into()),
        )
    }

    /// Registers the `calls` terms of policy `index`'s checks: a check holding one is a consumer,
    /// and each term not yet known gets a synthetic check running its seed query.
    pub(super) fn register(&mut self, index: usize, policy: &mut CompiledPolicy) -> Result<()> {
        for check_index in 0..policy.checks.len() {
            let check = &policy.checks[check_index];
            let CheckKind::Forbid(query) = &check.kind else {
                continue;
            };
            let terms = query.calls();
            if check.synthetic || terms.is_empty() {
                continue;
            }
            if let Query::Ast { matchers, .. } = query
                && matchers.iter().all(|matcher| {
                    crate::policy_dsl::matcher::calls_positive_only(&matcher.matcher)
                })
            {
                self.positive.insert((index, check_index));
            }
            let template = check.clone();
            let mut ids = Vec::with_capacity(terms.len());
            for term in terms {
                let Some(id) = self.terms.add(&term) else {
                    ids.extend(self.terms.id(&term.key));
                    continue;
                };
                ids.push(id);
                self.view = None;
                if let Some(&producer) = self.seed_producers.get(&term.seed_key) {
                    self.producers.push(producer);
                    continue;
                }
                let producer_id = (index, policy.checks.len());
                self.seed_producers
                    .insert(Arc::clone(&term.seed_key), producer_id);
                self.producers.push(producer_id);
                let mut producer = template.clone();
                producer.kind = CheckKind::Forbid(term.seed.clone());
                producer.synthetic = true;
                // A seed only answers which definitions match: nothing is reported or suppressed.
                producer.finding.each = None;
                producer.finding.node_regexes.clear();
                producer.finding.suppress = None;
                producer.finding.group_transform = None;
                producer.finding.message_template = None;
                producer.finding.fix = None;
                policy.checks.push(producer);
            }
            self.consumers.insert((index, check_index), ids);
        }
        Ok(())
    }

    /// The consumers of `input` whose result is known empty without evaluating them: each reads
    /// `calls` only positively, the file is within its policy's input limits, and every term it
    /// reads is silent in the file (`silent`; else those silent under the stamp its consumer
    /// results are under, or under the current graph when it has none): no call of it reaches or
    /// may reach their seeds.
    pub(super) fn quiet(
        &self,
        input: &FileInput,
        policies: &[Arc<CompiledPolicy>],
        silent: Option<&[TermId]>,
    ) -> Vec<CheckId> {
        let Some(summary) = self.summaries.get(&input.path) else {
            return Vec::new();
        };
        let consumers = self.consumers_for(input, policies);
        let stamped = self
            .stamps
            .get(&input.path)
            .and_then(|(stamped, _)| stamped.as_ref());
        let computed;
        let silent = match (silent, stamped, &self.view) {
            (Some(silent), _, _) => silent,
            (None, Some((_, silent)), _) => silent,
            (None, None, Some(view)) => {
                let terms = consumers
                    .iter()
                    .filter_map(|id| self.consumers.get(id))
                    .flatten()
                    .copied()
                    .collect::<BTreeSet<_>>();
                computed = view.silent(summary, &terms.into_iter().collect::<Vec<_>>());
                &computed
            }
            (None, None, None) => return Vec::new(),
        };
        consumers
            .into_iter()
            .filter(|id| {
                let limits = &policies[id.0].capabilities.limits;
                limits.max_files != 0
                    && summary.bytes as usize <= limits.max_source_bytes
                    && self.positive.contains(id)
                    && self.consumers[id]
                        .iter()
                        .all(|term| silent.binary_search(term).is_ok())
            })
            .collect()
    }

    /// The consumers among `file`'s AST candidates, which `defer` removes from its dispatch.
    pub(super) fn defer(
        &self,
        file: &mut FileState,
        policies: &[Arc<CompiledPolicy>],
    ) -> Vec<CheckId> {
        let (deferred, kept): (Vec<_>, Vec<_>) = file
            .candidates
            .regex
            .iter()
            .chain(&file.candidates.ast)
            .copied()
            .partition(|id| self.consumers.contains_key(id));
        if !deferred.is_empty() {
            file.candidates = Arc::new(candidate_dispatch(
                file_language(&file.input),
                kept,
                policies,
                &file.input,
            ));
        }
        deferred
    }

    pub(super) fn is_producer(&self, id: &CheckId) -> bool {
        self.producers.contains(id)
    }

    /// The consumers that apply to `input`: in their policy's scope, with a grammar for it.
    pub(super) fn consumers_for(
        &self,
        input: &FileInput,
        policies: &[Arc<CompiledPolicy>],
    ) -> Vec<CheckId> {
        self.consumers
            .keys()
            .copied()
            .filter(|&(policy, check)| {
                policies[policy].scope.matches(&input.path)
                    && matches!(
                        &policies[policy].checks[check].kind,
                        CheckKind::Forbid(Query::Ast { matchers, .. }) if grammar_matcher(matchers, input).is_some()
                    )
            })
            .collect()
    }

    /// Records `file`'s summary, and its seeds from the producers it was evaluated for. A file with
    /// syntax errors, or where a producer failed, has unknown seeds for it. A file consumers
    /// apply to (`consumes`) has no consumer results yet for this evaluation. The graph is
    /// stale only where what it reads of the file changed: an edit that moves no definition in
    /// or out of the graph, and changes no call or seed, leaves it current.
    pub(super) fn record(&mut self, path: &str, file: &FileState, consumes: Option<&[CheckId]>) {
        let mut seeds = FileSeeds::default();
        let summary = file
            .crossfile
            .summary
            .as_ref()
            .filter(|(revision, _)| *revision == file.source_revision)
            .map(|(_, summary)| summary.as_ref());
        for (term, id) in self.producers.iter().enumerate() {
            let scope = self.terms.terms[term].seed_scope;
            let term = term as TermId;
            let witnesses = file.witnesses.get(id);
            let starts = witnesses.and_then(|witnesses| {
                calls::seed_starts(
                    witnesses.iter().map(|witness| &witness.span),
                    scope,
                    summary,
                )
            });
            if file.failures.contains_key(id)
                || (witnesses.is_some() && (file.syntax.is_some() || starts.is_none()))
            {
                seeds.unknown.push(term);
            } else if let Some(starts) = starts {
                for start in starts {
                    seeds.matched(start, term);
                }
            }
        }
        if let Some((revision, summary)) = &file.crossfile.summary
            && *revision == file.source_revision
        {
            let fingerprint = graph_fingerprint(summary, &seeds);
            if self.fingerprints.get(path) != Some(&fingerprint) {
                self.fingerprints.insert(path.to_owned(), fingerprint);
                self.dirty.insert(Arc::from(path));
            } else if self
                .summaries
                .get(path)
                .is_some_and(|known| !Arc::ptr_eq(known, summary))
            {
                self.resummarized.insert(Arc::from(path));
            }
            self.summaries.insert(path.to_owned(), Arc::clone(summary));
            if seeds == FileSeeds::default() {
                self.seeds.remove(path);
            } else {
                self.seeds.insert(Arc::from(path), seeds);
            }
        }
        match consumes {
            Some(checks) => {
                let terms = self.terms_of(checks);
                self.stamps.insert(path.to_owned(), (None, terms));
            }
            None => {
                self.stamps.remove(path);
            }
        }
    }

    /// Drops the call graph and closure of an inactive workspace: the next refresh links them
    /// again from the summaries, which stay (every worktree of a codebase shares them).
    pub(super) fn evict(&mut self) {
        self.view = None;
        self.dirty.clear();
        self.resummarized.clear();
    }

    /// Forgets a removed file.
    pub(super) fn forget(&mut self, path: &str) {
        if self.summaries.remove(path).is_some() {
            self.dirty.insert(Arc::from(path));
        }
        self.seeds.remove(path);
        self.fingerprints.remove(path);
        self.resummarized.remove(path);
        self.stamps.remove(path);
    }

    /// The call graph over every summary and seed, updated where one changed, and the consumer
    /// files whose stamp under it differs from their results'. Unless the graph was built anew,
    /// only the files without results and those an update names can differ.
    pub(super) fn refresh(&mut self) -> Option<(Arc<CallsView>, Vec<Restamp>)> {
        if !self.active() {
            return None;
        }
        // `None`: every file; else the files an update names.
        let affected = match &mut self.view {
            None => {
                let summaries = self
                    .summaries
                    .iter()
                    .map(|(path, summary)| (Arc::from(path.as_str()), Arc::clone(summary)))
                    .collect();
                self.view = Some(Arc::new(CallsView::build(
                    self.terms.questions(),
                    summaries,
                    &self.seeds,
                )));
                self.dirty.clear();
                self.resummarized.clear();
                None
            }
            Some(view) => {
                // Evaluations are done with the view: it is changed in place, not copied.
                for path in std::mem::take(&mut self.resummarized) {
                    if let Some(summary) = self.summaries.get(path.as_ref())
                        && !self.dirty.contains(&path)
                    {
                        Arc::make_mut(view).resummarize(&path, Arc::clone(summary));
                    }
                }
                if self.dirty.is_empty() {
                    Some(BTreeSet::new())
                } else {
                    let changes = std::mem::take(&mut self.dirty)
                        .into_iter()
                        .map(|path| {
                            let summary = self.summaries.get(path.as_ref()).cloned();
                            (path, summary)
                        })
                        .collect::<Vec<_>>();
                    Some(Arc::make_mut(view).update(changes, &self.seeds))
                }
            }
        };
        let view = Arc::clone(self.view.as_ref().expect("the graph is built"));
        let candidates = self
            .stamps
            .iter()
            .filter(|(path, (known, _))| {
                known.is_none()
                    || affected
                        .as_ref()
                        .is_none_or(|affected| affected.contains(path.as_str()))
            })
            .filter_map(|(path, (known, terms))| {
                let known = known.as_ref().map(|(stamp, _)| *stamp);
                Some((path, known, terms, self.summaries.get(path)?))
            })
            .collect::<Vec<_>>();
        let stamp = |(path, known, terms, summary): (
            &String,
            Option<HashKey>,
            &Arc<[TermId]>,
            &Arc<Summary>,
        )| {
            let stamp = view.stamp(summary, terms);
            (known != Some(stamp)).then(|| Restamp {
                path: path.clone(),
                stamp,
                silent: view.silent(summary, terms),
            })
        };
        let changed: Vec<Restamp> = if candidates.len() < 256 {
            candidates.into_iter().filter_map(stamp).collect()
        } else {
            WORK_POOL.install(|| candidates.into_par_iter().filter_map(stamp).collect())
        };
        Some((view, changed))
    }

    pub(super) fn summary(&self, path: &str) -> Option<&Arc<Summary>> {
        self.summaries.get(path)
    }

    /// The stamp `path`'s consumer results are under.
    pub(super) fn stamp(&self, path: &str) -> Option<HashKey> {
        self.stamps
            .get(path)
            .and_then(|(stamped, _)| stamped.as_ref())
            .map(|(stamp, _)| *stamp)
    }

    /// Puts `path`'s consumer results under `stamp`, with the terms `silent` in it there.
    pub(super) fn set_stamp(&mut self, path: &str, stamp: HashKey, silent: Vec<TermId>) {
        if !self.stamps.contains_key(path) {
            return;
        }
        let silent = self.term_list(silent);
        if let Some((known, _)) = self.stamps.get_mut(path) {
            *known = Some((stamp, silent));
        }
    }
}

/// The stored result identity a file's call summary is kept under, beside its check results:
/// keyed by the file's source and context, and by the extractor version through its name.
fn summary_candidate() -> CandidateCheck {
    CandidateCheck {
        policy: HashKey::sha256(b"rerail-calls-summary"),
        check: format!(
            "calls-summary-v{}",
            crate::policy_dsl::calls::summary::EXTRACTOR_VERSION
        ),
    }
}
/// A hash of what the closure reads of one file: not where its definitions start (seeds are read
/// by definition).
fn graph_fingerprint(summary: &Summary, seeds: &FileSeeds) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    summary
        .module
        .map(|module| summary.name(module))
        .hash(&mut hasher);
    summary.open.hash(&mut hasher);
    for def in &summary.defs {
        summary.name(def.name).hash(&mut hasher);
        def.flags.hash(&mut hasher);
        seeds.matched.get(&def.start).hash(&mut hasher);
    }
    seeds.unknown.hash(&mut hasher);
    for (owner, name) in &summary.edges {
        (owner, summary.name(*name)).hash(&mut hasher);
    }
    for (alias, target) in &summary.aliases {
        (summary.name(*alias), summary.name(*target)).hash(&mut hasher);
    }
    for star in &summary.stars {
        summary.name(*star).hash(&mut hasher);
    }
    for (key, target, kind) in &summary.supers {
        (summary.name(*key), summary.name(*target), kind).hash(&mut hasher);
    }
    for target in &summary.targets {
        summary.name(*target).hash(&mut hasher);
    }
    hasher.finish()
}

/// Takes `file`'s call summary from its stored results, if they hold one of this extractor.
pub(super) fn restore_summary(file: &mut FileState, persisted: &[StoredLocalResult]) {
    if !file_language(&file.input).is_some_and(calls::summarizes)
        || file.crossfile.summary.is_some()
    {
        return;
    }
    let candidate = summary_candidate();
    let Some(result) = persisted
        .iter()
        .find(|result| result.candidate == candidate)
    else {
        return;
    };
    let summary =
        shared_summary(file).or_else(|| Some(share_summary(file, Summary::decode(&result.bytes)?)));
    if let Some(summary) = summary {
        file.crossfile.summary = Some((file.source_revision, summary));
        file.crossfile.summary_stored = true;
    }
}

/// Summaries by file version (source, file context), for as long as some workspace holds one:
/// every worktree of a codebase shares one summary per unchanged file.
#[derive(Default)]
struct SharedSummaries {
    summaries: HashMap<(HashKey, HashKey), std::sync::Weak<Summary>>,
    /// Entries left by the last sweep of dropped summaries.
    swept: usize,
}

static SHARED_SUMMARIES: LazyLock<parking_lot::Mutex<SharedSummaries>> =
    LazyLock::new(Default::default);

fn version_key(file: &FileState) -> (HashKey, HashKey) {
    (
        HashKey::from_sha256_digest(file.source_digest),
        file_context(&file.input),
    )
}

/// `file`'s summary as some workspace holds it.
fn shared_summary(file: &FileState) -> Option<Arc<Summary>> {
    SHARED_SUMMARIES
        .lock()
        .summaries
        .get(&version_key(file))
        .and_then(std::sync::Weak::upgrade)
}

/// Shares `summary` as `file`'s, unless another workspace did meanwhile: then theirs.
fn share_summary(file: &FileState, summary: Summary) -> Arc<Summary> {
    let key = version_key(file);
    let mut shared = SHARED_SUMMARIES.lock();
    if let Some(known) = shared
        .summaries
        .get(&key)
        .and_then(std::sync::Weak::upgrade)
    {
        return known;
    }
    let summary = Arc::new(summary);
    shared.summaries.insert(key, Arc::downgrade(&summary));
    if shared.summaries.len() > 2 * shared.swept.max(1 << 10) {
        shared
            .summaries
            .retain(|_, summary| summary.strong_count() > 0);
        shared.swept = shared.summaries.len();
    }
    summary
}

/// `file`'s call summary as a stored result, unless the store holds it already.
pub(super) fn summary_publication(file: &FileState) -> Result<Option<LocalResultPublication>> {
    let Some((_, summary)) = file.crossfile.summary.as_ref().filter(|(revision, _)| {
        *revision == file.source_revision && !file.crossfile.summary_stored
    }) else {
        return Ok(None);
    };
    let bytes = summary.encode();
    let frame = zstd::stream::encode_all(Cursor::new(&bytes), 3)
        .map_err(|error| invalid(error.to_string()))?;
    Ok(Some(LocalResultPublication {
        candidate: summary_candidate(),
        context: file_context(&file.input),
        raw_length: bytes.len(),
        frame,
    }))
}

/// Summaries extracted (not restored from the store), in any workspace of the process.
#[cfg(test)]
pub(super) static SUMMARIES_EXTRACTED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// The key consumer results are stored under: the file context and the stamp they were
/// evaluated under.
pub(super) fn calls_context(context: HashKey, stamp: HashKey) -> HashKey {
    let mut bytes = b"rerail-calls-context-v1\0".to_vec();
    for key in [context, stamp] {
        bytes.extend_from_slice(&key.sha256_bytes().expect("contexts and stamps are sha-256"));
    }
    HashKey::sha256(&bytes)
}

/// Extracts `file`'s call summary unless this source revision has one: from the tape it was
/// evaluated with, else from a parse now. A file that cannot be parsed in time has an open
/// summary of its module alone.
pub(super) fn summarize_file(
    file: &mut FileState,
    grammars: &mut BTreeMap<String, Arc<tape::Grammar>>,
    closed: &AtomicBool,
    metrics: &mut Metrics,
    keep_table: bool,
) -> Result<()> {
    let Some(language) = file_language(&file.input).filter(|language| calls::summarizes(*language))
    else {
        return Ok(());
    };
    if file
        .crossfile
        .summary
        .as_ref()
        .is_some_and(|(revision, _)| *revision == file.source_revision)
    {
        return Ok(());
    }
    if let Some(summary) = shared_summary(file) {
        file.crossfile.summary = Some((file.source_revision, summary));
        return Ok(());
    }
    let grammar = match grammars.get(&file.input.language) {
        Some(grammar) => Arc::clone(grammar),
        None => {
            let grammar = tape::Grammar::new(language).map_err(invalid)?;
            grammars.insert(file.input.language.clone(), Arc::clone(&grammar));
            grammar
        }
    };
    let taped = file
        .tape
        .as_ref()
        .is_some_and(|(revision, _)| *revision == file.source_revision)
        || encode_tape(file, language, &grammar, closed, metrics)?.is_ok();
    let module = file.input.module.clone();
    let opaque = |file: &FileState| Summary {
        open: true,
        bytes: u32::try_from(file.input.source.len()).unwrap_or(u32::MAX),
        // Python names the module, whose members may hide in it; a PHP file names nothing.
        ..match Family::of(language) {
            Family::Python => Summary::extract(
                &GrammarDialect::Python.ast_grep("").root(),
                None,
                file.input.module.as_deref(),
                true,
            ),
            Family::Php | Family::Ecma | Family::Blade => Summary::default(),
        }
    };
    // An opaque summary may be a transient failure's: it is this workspace's alone.
    let (summary, shared) = match file.tape.as_ref().filter(|_| taped) {
        None => (opaque(file), false),
        Some((_, image)) => {
            let lines = Arc::new(LineTable::new(
                &file.input.source,
                Terminators::of(language),
            ));
            match tape::TapeDoc::restore_with_lines(
                Arc::clone(&file.input.source),
                grammar,
                image,
                lines,
                &matcher_budget_exhausted,
            ) {
                Err(_) => (opaque(file), false),
                Ok(doc) => {
                    let errors = !doc.syntax_error_ranges().is_empty();
                    let root = Root::doc(doc);
                    // The table the file's evaluation built, where it reads as `calls` does.
                    let reading = calls::reading(&file.input.path);
                    let known = file
                        .scopes
                        .as_ref()
                        .filter(|(revision, _)| *revision == file.source_revision)
                        .and_then(|(_, tables)| {
                            tables
                                .iter()
                                .find(|table| {
                                    table.language == language && table.reading == reading
                                })
                                .cloned()
                        });
                    let (summary, table) = calls::summarize(
                        root.root(),
                        language,
                        &file.input.path,
                        module.as_deref(),
                        errors,
                        file.input.source.len(),
                        known.as_deref(),
                    );
                    // The table `calls` terms read at call sites: the file's consumers' pass
                    // reuses it.
                    if let Some(table) = table.filter(|_| keep_table) {
                        let tables = match &mut file.scopes {
                            Some((revision, tables)) if *revision == file.source_revision => tables,
                            scopes => &mut scopes.insert((file.source_revision, Vec::new())).1,
                        };
                        if !tables.iter().any(|known| {
                            known.language == table.language && known.reading == table.reading
                        }) {
                            tables.push(Arc::new(table));
                        }
                    }
                    (summary, true)
                }
            }
        }
    };
    let summary = if shared {
        share_summary(file, summary)
    } else {
        Arc::new(summary)
    };
    file.crossfile.summary = Some((file.source_revision, summary));
    #[cfg(test)]
    SUMMARIES_EXTRACTED.fetch_add(1, Ordering::Relaxed);
    Ok(())
}
