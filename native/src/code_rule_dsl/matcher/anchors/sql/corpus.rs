//! Workspace-owned positional index. A file is published once per source and grammar;
//! one batched preparation phase publishes compact candidate sets before any matching.
use super::super::AnchorPlan;
use super::{candidates::Roots, store::Store};
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::Doc;
use parking_lot::Mutex;
use rusqlite::Result;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::{cell::RefCell, sync::Arc};

#[derive(Default)]
pub(in crate::code_rule_dsl) struct Corpus {
    state: Arc<Mutex<Option<State>>>,
    sql: crate::code_rule_dsl::relational::pool::Backing,
    members: Mutex<HashMap<String, (Key, Arc<File>)>>,
}

type Key = (Arc<str>, u32, [u8; 32]);

#[derive(Clone)]
pub(super) struct File {
    pub(super) fid: i64,
    pub(super) lang: u32,
    pub(super) digest: [u8; 32],
    pub(super) nodes: usize,
    pub(super) roots: HashMap<usize, Roots>,
    pub(super) bytes: usize,
    structural: bool,
    scope: Arc<Vec<Key>>,
    published_bytes: usize,
}

struct State {
    files: HashMap<Key, Arc<File>>,
    pins: HashMap<Key, usize>,
    idle: std::collections::BTreeMap<u64, Key>,
    idle_keys: HashMap<Key, u64>,
    tick: u64,
    idle_bytes: usize,
    batch_queries: usize,
    data_statements: usize,
    plans: HashMap<usize, Arc<AnchorPlan>>,
    templates: HashMap<(u32, Vec<usize>), Template>,
    languages: HashSet<u32>,
    #[cfg(test)]
    connections: Vec<Store>,
    // Drops after query connections, so the database is closed before unlinking it.
    writer: Store,
}

struct Template {
    decisions: Vec<usize>,
    statement: Arc<super::lower::Statement>,
}

#[path = "corpus/owner.rs"]
mod owner;
#[cfg(test)]
#[path = "corpus/owner_tests.rs"]
mod owner_tests;
#[cfg(test)]
#[path = "corpus/planning_tests.rs"]
mod planning_tests;

impl Corpus {
    pub(in crate::code_rule_dsl) fn memory_profile(&self) -> serde_json::Value {
        let guard = self.state.lock();
        let Some(state) = guard.as_ref() else {
            return serde_json::json!({"templates":0,"sqlBytes":0,"graphBytes":0,"candidateBytes":0});
        };
        let sql = state
            .templates
            .values()
            .map(|t| {
                t.statement
                    .stages
                    .iter()
                    .map(|s| s.sql.capacity())
                    .sum::<usize>()
                    + t.statement
                        .queries
                        .iter()
                        .map(String::capacity)
                        .sum::<usize>()
            })
            .sum::<usize>();
        let graph = state
            .templates
            .values()
            .map(|t| {
                t.statement
                    .graph
                    .iter()
                    .map(String::capacity)
                    .sum::<usize>()
            })
            .sum::<usize>();
        serde_json::json!({"templates":state.templates.len(),"sqlBytes":sql,"graphBytes":graph,"candidateBytes":state.files.values().map(|f|f.bytes).sum::<usize>(),"files":state.files.len(),"plans":state.plans.len(),"idleBytesEstimate":state.idle_bytes})
    }

    pub(in crate::code_rule_dsl) fn sql_backing(
        &self,
    ) -> crate::code_rule_dsl::relational::pool::Backing {
        self.sql.clone()
    }

    /// A new logical membership over the same immutable fact owner.
    pub(in crate::code_rule_dsl) fn fork(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            sql: self.sql.clone(),
            members: Mutex::default(),
        }
    }

    #[cfg(feature = "code-rule-profile")]
    pub(in crate::code_rule_dsl) fn record_origins<'a>(
        &self,
        rules: impl IntoIterator<Item = &'a crate::code_rule_dsl::model::CompiledCodeRule>,
    ) {
        if std::env::var_os("RERAIL_SQL_PROFILE_DIR").is_none() {
            return;
        }
        let guard = self.state.lock();
        let Some(state) = guard.as_ref() else {
            return;
        };
        let mut origins = state.writer.origins.lock();
        for (plan, names) in super::plans::origins(rules) {
            origins.entry(plan).or_default().extend(names);
        }
    }

    #[cfg(all(test, feature = "code-rule-profile"))]
    pub(super) fn preparation_counts(&self) -> (u64, usize) {
        let guard = self.state.lock();
        let state = guard.as_ref().unwrap();
        let decoded = state.writer.readers.0.lock().counters.decoded_nodes;
        (decoded, state.templates.len())
    }
    pub(in crate::code_rule_dsl) fn put(
        &self,
        path: &str,
        doc: &TapeDoc,
        interrupted: &impl Fn() -> bool,
    ) -> Result<i64> {
        let mut state = self.state.lock();
        if state.is_none() {
            *state = Some(State {
                files: HashMap::new(),
                pins: HashMap::new(),
                idle: Default::default(),
                idle_keys: HashMap::new(),
                tick: 0,
                idle_bytes: 0,
                batch_queries: 0,
                data_statements: 0,
                plans: HashMap::new(),
                templates: HashMap::new(),
                languages: HashSet::new(),
                #[cfg(test)]
                connections: Vec::new(),
                writer: Store::temporary()?,
            });
        }
        let state = state.as_mut().expect("initialized corpus");
        let lang = u32::from(
            crate::code_rule_dsl::language::tape_tag(*ast_grep_core::Doc::get_lang(doc))
                .expect("tape language"),
        );
        state.collect()?;
        let key = (Arc::from(path), lang, *doc.index_source_digest());
        if let Some(file) = state.files.get(&key).cloned() {
            self.bind(state, key, file.clone());
            #[cfg(feature = "code-rule-profile")]
            crate::code_rule_dsl::profile::record_sql_index_reuse();
            return Ok(file.fid);
        }
        #[cfg(test)]
        crate::code_rule_dsl::test_support::record_walk(
            crate::code_rule_dsl::test_support::Walk::AnchorBuild,
        );
        let fid = state.writer.put_known(
            path,
            doc,
            None,
            state.languages.contains(&lang),
            interrupted,
        )?;
        state.languages.insert(lang);
        let file = Arc::new(File {
            fid,
            lang,
            digest: *doc.index_source_digest(),
            nodes: doc.transducer_len(),
            roots: HashMap::new(),
            bytes: 0,
            structural: false,
            scope: Arc::default(),
            published_bytes: doc
                .get_source()
                .len()
                .saturating_add(doc.transducer_len().saturating_mul(64)),
        });
        state.files.insert(key.clone(), file.clone());
        self.bind(state, key, file);
        Ok(fid)
    }

    pub(super) fn prepared_file(
        &self,
        path: &str,
        lang: u32,
        digest: &[u8; 32],
    ) -> Option<Arc<File>> {
        self.members
            .lock()
            .get(path)
            .map(|(_, file)| file)
            .filter(|file| file.lang == lang && &file.digest == digest)
            .cloned()
    }

    /// Test convenience for a frontier over every published file.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn prepare(
        &self,
        plans: &BTreeMap<u32, Vec<Arc<AnchorPlan>>>,
    ) -> Result<()> {
        let mut guard = self.state.lock();
        let Some(state) = guard.as_mut() else {
            return Ok(());
        };
        let statistics = super::statistics::Statistics::read(
            &state.writer.db,
            plans
                .iter()
                .flat_map(|(&lang, plans)| plans.iter().map(move |p| (lang, p.as_ref()))),
        )?;
        for (&lang, plans) in plans {
            let files = self
                .members
                .lock()
                .values()
                .map(|(key, _)| key.clone())
                .collect();
            Self::prepare_group(state, lang, plans, Some(&files), &statistics)?;
            for (key, file) in self.members.lock().values_mut() {
                *file = Arc::clone(&state.files[key]);
            }
        }
        Ok(())
    }

    /// Batch only admitted plan/file pairs. Equal frontiers share one SQL request.
    pub(in crate::code_rule_dsl) fn prepare_files(
        &self,
        demands: &BTreeMap<String, Vec<Arc<AnchorPlan>>>,
    ) -> Result<()> {
        let mut guard = self.state.lock();
        let Some(state) = guard.as_mut() else {
            return Ok(());
        };
        type Domain = (Vec<Arc<AnchorPlan>>, HashSet<Key>);
        let mut groups = BTreeMap::<(u32, Vec<usize>), Domain>::new();
        let mut members = self.members.lock();
        for (path, plans) in demands {
            let Some((key, file)) = members.get(path) else {
                continue;
            };
            if plans.is_empty() {
                continue;
            }
            let mut plans = plans.clone();
            // Addresses deduplicate live handles and key this cache only. The compiler
            // independently admits fragments and canonically composes physical work.
            plans.sort_unstable_by_key(|p| Arc::as_ptr(p) as usize);
            plans.dedup_by_key(|p| Arc::as_ptr(p) as usize);
            let ids = plans.iter().map(|p| Arc::as_ptr(p) as usize).collect();
            groups
                .entry((file.lang, ids))
                .or_insert_with(|| (plans, HashSet::new()))
                .1
                .insert(key.clone());
        }
        let statistics = super::statistics::Statistics::read(
            &state.writer.db,
            groups
                .iter()
                .flat_map(|((lang, _), (plans, _))| plans.iter().map(move |p| (*lang, p.as_ref()))),
        )?;
        for ((lang, _), (plans, paths)) in groups {
            Self::prepare_group(state, lang, &plans, Some(&paths), &statistics)?;
            for key in &paths {
                if let Some((_, prepared)) = members.get_mut(key.0.as_ref()) {
                    *prepared = Arc::clone(&state.files[key]);
                }
            }
        }
        Ok(())
    }

    fn prepare_group(
        state: &mut State,
        lang: u32,
        plans: &[Arc<AnchorPlan>],
        paths: Option<&HashSet<Key>>,
        statistics: &super::statistics::Statistics,
    ) -> Result<()> {
        // Cached candidates bind the complete admitted file set, not the owner's contents.
        let mut fids = state
            .files
            .iter()
            .filter(|(key, file)| {
                file.lang == lang && paths.is_none_or(|paths| paths.contains(*key))
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        fids.sort_unstable();
        let scope = Arc::new(fids);
        for (key, file) in &mut state.files {
            if file.lang == lang
                && paths.is_none_or(|paths| paths.contains(key))
                && file.scope != scope
            {
                let file = Arc::make_mut(file);
                file.roots.clear();
                file.bytes = 0;
                file.scope = Arc::clone(&scope);
            }
        }
        for plan in plans {
            state
                .plans
                .entry(Arc::as_ptr(plan) as usize)
                .or_insert_with(|| Arc::clone(plan));
        }
        // Discover covering demands across the full plan frontier before executing any
        // bounded group. A sparse early group must not force a later universal re-decode.
        let mut covering = std::collections::BTreeSet::new();
        for group in std::iter::once(plans) {
            let ids: Vec<_> = group.iter().map(|p| Arc::as_ptr(p) as usize).collect();
            let missing: Vec<_> = state
                .files
                .iter()
                .filter(|(path, f)| {
                    paths.is_none_or(|paths| paths.contains(*path))
                        && f.lang == lang
                        && !f.structural
                        && ids.iter().any(|id| !f.roots.contains_key(id))
                })
                .map(|(_, f)| f.fid)
                .collect();
            if missing.is_empty() {
                continue;
            }
            let refs: Vec<_> = group.iter().map(Arc::as_ref).collect();
            if Self::template(state, lang, &ids, &refs, statistics).universal {
                covering.extend(missing);
            }
        }
        let covering: Vec<_> = covering.into_iter().collect();
        for files in covering.chunks(16_384) {
            super::execute::prepare_structural(&state.writer, files)?;
            let ready: HashSet<_> = files.iter().copied().collect();
            for file in state.files.values_mut().filter(|f| ready.contains(&f.fid)) {
                Arc::make_mut(file).structural = true;
            }
            state.data_statements += 6 + files.len().div_ceil(128);
        }
        for plans in std::iter::once(plans) {
            let ids: Vec<_> = plans.iter().map(|p| Arc::as_ptr(p) as usize).collect();
            let refs: Vec<_> = plans.iter().map(Arc::as_ref).collect();
            let result = Self::select_group(state, lang, &ids, &refs, true, paths, statistics);
            #[cfg(feature = "code-rule-profile")]
            if result.is_err() {
                crate::code_rule_dsl::profile::record_anchor_batch_failure();
            }
            result?;
        }
        Ok(())
    }

    fn template(
        state: &mut State,
        lang: u32,
        ids: &[usize],
        plans: &[&AnchorPlan],
        statistics: &super::statistics::Statistics,
    ) -> Arc<super::lower::Statement> {
        #[cfg(feature = "code-rule-profile")]
        let _timer = crate::code_rule_dsl::profile::PhaseTimer::start(
            crate::code_rule_dsl::profile::Phase::Planning,
        );
        let key = (lang, ids.to_vec());
        let decisions = statistics.decisions(lang, plans);
        if let Some(template) = state.templates.get(&key)
            && template.decisions == decisions
        {
            return Arc::clone(&template.statement);
        }
        #[cfg(feature = "code-rule-profile")]
        let mut lower =
            crate::code_rule_dsl::profile::trace::Span::start("anchor-lower", "template");
        let statement = Arc::new(super::lower::batch_files_planned(
            plans,
            lang,
            &[],
            statistics,
        ));
        #[cfg(feature="code-rule-profile")]
        lower.detail(||serde_json::json!({"plans":plans.len(),"files":0,"template":true,"sqlBytes":statement.queries.iter().map(String::len).sum::<usize>()+statement.stages.iter().map(|s|s.sql.len()).sum::<usize>(),"materializations":statement.preparations(false),"resultQueries":statement.queries.len()}));
        if ids.iter().all(|id| state.plans.contains_key(id)) {
            state.templates.insert(
                key,
                Template {
                    decisions,
                    statement: Arc::clone(&statement),
                },
            );
        }
        statement
    }

    fn select_group(
        state: &mut State,
        lang: u32,
        ids: &[usize],
        plans: &[&AnchorPlan],
        covered: bool,
        paths: Option<&HashSet<Key>>,
        statistics: &super::statistics::Statistics,
    ) -> Result<()> {
        let files: Vec<_> = state
            .files
            .iter()
            .filter(|(path, file)| {
                paths.is_none_or(|paths| paths.contains(*path))
                    && file.lang == lang
                    && ids.iter().any(|id| !file.roots.contains_key(id))
            })
            .map(|(path, file)| (path.clone(), Arc::clone(file)))
            .collect();
        let active: Vec<_> = ids
            .iter()
            .enumerate()
            .filter(|(_, id)| files.iter().any(|(_, file)| !file.roots.contains_key(id)))
            .map(|(i, _)| i)
            .collect();
        if !active.is_empty() && active.len() < ids.len() {
            let ids: Vec<_> = active.iter().map(|&i| ids[i]).collect();
            let plans: Vec<_> = active.iter().map(|&i| plans[i]).collect();
            return Self::select_group(state, lang, &ids, &plans, covered, paths, statistics);
        }
        #[cfg(feature = "code-rule-profile")]
        if std::env::var_os("RERAIL_SQL_PROFILE_DIR").is_some() {
            *state.writer.versions.lock() = state
                .files
                .values()
                .map(|file| (file.fid, (file.lang, file.digest)))
                .collect();
        }
        // Keep bind counts bounded, without issuing a query for each file or rule.
        for files in files.chunks(16_384) {
            let fids: Vec<_> = files.iter().map(|(_, file)| file.fid).collect();
            let nodes: HashMap<_, _> = files
                .iter()
                .map(|(_, file)| (file.fid, file.nodes))
                .collect();
            let statement = Self::template(state, lang, ids, plans, statistics);
            let stages = statement.preparations(covered);
            let mut packed = HashMap::new();
            let mut current = None;
            let mut positions = Vec::new();
            #[cfg(feature = "code-rule-profile")]
            let _timer = crate::code_rule_dsl::profile::PhaseTimer::start(
                crate::code_rule_dsl::profile::Phase::Selection,
            );
            // Rows are ordered by query and file. Buffer only one file's ordinals, then
            // compact them; never retain a corpus-sized vector of uncompressed results.
            state.batch_queries += 1;
            state.data_statements +=
                stages + 1 + statement.queries.len() + fids.len().div_ceil(128);
            #[cfg(feature = "code-rule-profile")]
            crate::code_rule_dsl::profile::record_anchor_batch(files.len(), plans.len(), stages);
            #[cfg(feature = "code-rule-profile")]
            crate::code_rule_dsl::profile::record_anchor_statements(
                statement.queries.len().saturating_sub(1),
            );
            let check_budget = || {
                if crate::code_rule_dsl::budget::matcher_budget_exhausted() {
                    Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_INTERRUPT),
                        None,
                    ))
                } else {
                    Ok(())
                }
            };
            check_budget()?;
            super::execute::rows_for(
                &state.writer,
                &statement,
                &fids,
                covered,
                |qid, fid, pre| {
                    let key = (qid, fid);
                    if current != Some(key)
                        && let Some(previous) = current.replace(key)
                    {
                        packed.insert(
                            previous,
                            Roots::pack(std::mem::take(&mut positions), nodes[&previous.1]),
                        );
                    }
                    positions.push(pre as u32);
                },
            )?;
            check_budget()?;
            if let Some(key) = current {
                packed.insert(key, Roots::pack(positions, nodes[&key.1]));
            }
            for (path, file) in files {
                let current = Arc::make_mut(state.files.get_mut(path).expect("batch file"));
                for (qid, &id) in ids.iter().enumerate() {
                    let roots = packed
                        .remove(&(qid, file.fid))
                        .unwrap_or_else(|| Roots::pack(Vec::new(), file.nodes));
                    current.bytes += roots.bytes();
                    if let Some(old) = current.roots.insert(id, roots) {
                        current.bytes -= old.bytes();
                    }
                }
            }
        }
        #[cfg(feature = "code-rule-profile")]
        crate::code_rule_dsl::profile::record_candidate_bytes(
            state.files.values().map(|file| file.bytes).sum(),
        );
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn prepare_borrowed(&self, plans: &[&AnchorPlan], doc: &TapeDoc) -> Result<()> {
        let mut guard = self.state.lock();
        let state = guard.as_mut().expect("published test source");
        let lang = u32::from(
            crate::code_rule_dsl::language::tape_tag(*ast_grep_core::Doc::get_lang(doc)).unwrap(),
        );
        let ids: Vec<_> = plans
            .iter()
            .map(|&plan| plan as *const AnchorPlan as usize)
            .collect();
        let statistics = super::statistics::Statistics::read(
            &state.writer.db,
            plans.iter().map(|p| (lang, *p)),
        )?;
        Self::select_group(state, lang, &ids, plans, false, None, &statistics)?;
        for (key, file) in self.members.lock().values_mut() {
            *file = Arc::clone(&state.files[key]);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn batch_queries(&self) -> usize {
        self.state
            .lock()
            .as_ref()
            .map_or(0, |state| state.batch_queries)
    }

    #[cfg(test)]
    pub(super) fn data_statements(&self) -> usize {
        self.state
            .lock()
            .as_ref()
            .map_or(0, |state| state.data_statements)
    }

    pub(in crate::code_rule_dsl) fn contains(
        &self,
        path: &str,
        lang: u32,
        digest: &[u8; 32],
    ) -> Result<bool> {
        Ok(self.prepared_file(path, lang, digest).is_some())
    }

    pub(in crate::code_rule_dsl) fn remove(&self, path: &str) -> Result<()> {
        let mut guard = self.state.lock();
        if let Some(state) = guard.as_mut()
            && let Some((key, _)) = self.members.lock().remove(path)
        {
            state.unpin(&key);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn census(&self) -> (u64, i64) {
        let guard = self.state.lock();
        guard.as_ref().map_or((0, 0), |state| {
            (
                state.writer.db.total_changes(),
                state
                    .writer
                    .db
                    .query_row("SELECT count(*) FROM file", [], |row| row.get(0))
                    .unwrap(),
            )
        })
    }

    #[cfg(test)]
    pub(super) fn connection(self: &Arc<Self>) -> Result<Connection> {
        let mut guard = self.state.lock();
        let state = guard.as_mut().expect("publish before querying corpus");
        let store = match state.connections.pop() {
            Some(store) => store,
            None => Store::open(std::path::Path::new(
                state.writer.db.path().expect("file-backed corpus"),
            ))?,
        };
        // Keep the file records and the FTS postings in the same snapshot, including across
        // materialization statements. Other workers can publish different files through WAL.
        store.db.execute_batch("BEGIN DEFERRED")?;
        Ok(Connection {
            store: Some(store),
            corpus: Arc::clone(self),
        })
    }
}

#[cfg(test)]
pub(super) struct Connection {
    store: Option<Store>,
    corpus: Arc<Corpus>,
}

#[cfg(test)]
impl std::ops::Deref for Connection {
    type Target = Store;
    fn deref(&self) -> &Store {
        self.store.as_ref().expect("leased connection")
    }
}

#[cfg(test)]
impl Drop for Connection {
    fn drop(&mut self) {
        let store = self.store.take().expect("leased connection");
        store.readers.clear();
        // An interrupted query must not poison the next lease's transaction.
        if store.db.execute_batch("ROLLBACK").is_ok() {
            let mut guard = self.corpus.state.lock();
            if let Some(state) = guard.as_mut() {
                state.connections.push(store);
            }
        }
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<Corpus>>> = const { RefCell::new(None) };
}

pub(in crate::code_rule_dsl) struct CorpusSlot(Option<Arc<Corpus>>);

impl CorpusSlot {
    pub(in crate::code_rule_dsl) fn install(corpus: Arc<Corpus>) -> Self {
        Self(CURRENT.replace(Some(corpus)))
    }

    /// Stateless calls own one index for their complete evaluation, including nested inputs.
    pub(in crate::code_rule_dsl) fn enter() -> Self {
        Self::install(Self::current())
    }

    pub(in crate::code_rule_dsl) fn current() -> Arc<Corpus> {
        CURRENT.with_borrow(|current| current.clone().unwrap_or_default())
    }
}

impl Drop for CorpusSlot {
    fn drop(&mut self) {
        CURRENT.set(self.0.take());
    }
}
