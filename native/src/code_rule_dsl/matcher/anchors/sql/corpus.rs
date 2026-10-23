//! Workspace-owned positional index. A file is published once per source and grammar;
//! one batched preparation phase publishes compact candidate sets before any matching.
use super::super::AnchorPlan;
use super::{candidates::Roots, store::Store};
use crate::code_rule_dsl::tape::TapeDoc;
use parking_lot::Mutex;
use rusqlite::Result;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::{cell::RefCell, sync::Arc};

#[derive(Default)]
pub(in crate::code_rule_dsl) struct Corpus {
    state: Mutex<Option<State>>,
}

#[derive(Clone)]
pub(super) struct File {
    pub(super) fid: i64,
    pub(super) lang: u32,
    pub(super) digest: [u8; 32],
    pub(super) nodes: usize,
    pub(super) roots: HashMap<usize, Roots>,
    pub(super) bytes: usize,
    structural: bool,
}

struct State {
    files: HashMap<String, Arc<File>>,
    batch_queries: usize,
    data_statements: usize,
    plans: HashMap<usize, Arc<AnchorPlan>>,
    templates: HashMap<(u32, Vec<usize>), Arc<super::lower::Statement>>,
    languages: HashSet<u32>,
    #[cfg(test)]
    idle: Vec<Store>,
    // Drops after query connections, so the database is closed before unlinking it.
    writer: Store,
}

impl Corpus {
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
                batch_queries: 0,
                data_statements: 0,
                plans: HashMap::new(),
                templates: HashMap::new(),
                languages: HashSet::new(),
                #[cfg(test)]
                idle: Vec::new(),
                writer: Store::temporary()?,
            });
        }
        let state = state.as_mut().expect("initialized corpus");
        let lang = u32::from(
            crate::code_rule_dsl::language::tape_tag(*ast_grep_core::Doc::get_lang(doc))
                .expect("tape language"),
        );
        if let Some(file) = state
            .files
            .get(path)
            .filter(|file| file.lang == lang && &file.digest == doc.index_source_digest())
        {
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
            state.files.get(path).map(|file| file.fid),
            state.languages.contains(&lang),
            interrupted,
        )?;
        state.languages.insert(lang);
        state.files.insert(
            path.to_owned(),
            Arc::new(File {
                fid,
                lang,
                digest: *doc.index_source_digest(),
                nodes: doc.transducer_len(),
                roots: HashMap::new(),
                bytes: 0,
                structural: false,
            }),
        );
        Ok(fid)
    }

    pub(super) fn prepared_file(
        &self,
        path: &str,
        lang: u32,
        digest: &[u8; 32],
    ) -> Option<Arc<File>> {
        self.state
            .lock()
            .as_ref()?
            .files
            .get(path)
            .filter(|file| file.lang == lang && &file.digest == digest)
            .cloned()
    }

    /// The only production candidate-query entry point: plans and files are batched together.
    pub(in crate::code_rule_dsl) fn prepare(
        &self,
        plans: &BTreeMap<u32, Vec<Arc<AnchorPlan>>>,
    ) -> Result<()> {
        let mut guard = self.state.lock();
        let Some(state) = guard.as_mut() else {
            return Ok(());
        };
        for (&lang, plans) in plans {
            for plan in plans {
                state
                    .plans
                    .entry(Arc::as_ptr(plan) as usize)
                    .or_insert_with(|| Arc::clone(plan));
            }
            // Discover covering demands across the full plan frontier before executing any
            // bounded group. A sparse early group must not force a later universal re-decode.
            let mut covering = std::collections::BTreeSet::new();
            for group in std::iter::once(plans.as_slice()) {
                let ids: Vec<_> = group.iter().map(|p| Arc::as_ptr(p) as usize).collect();
                let missing: Vec<_> = state
                    .files
                    .values()
                    .filter(|f| {
                        f.lang == lang
                            && !f.structural
                            && ids.iter().any(|id| !f.roots.contains_key(id))
                    })
                    .map(|f| f.fid)
                    .collect();
                if missing.is_empty() {
                    continue;
                }
                let refs: Vec<_> = group.iter().map(Arc::as_ref).collect();
                if Self::template(state, lang, &ids, &refs).universal {
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
            for plans in std::iter::once(plans.as_slice()) {
                let ids: Vec<_> = plans.iter().map(|p| Arc::as_ptr(p) as usize).collect();
                let refs: Vec<_> = plans.iter().map(Arc::as_ref).collect();
                let result = Self::select_group(state, lang, &ids, &refs, true);
                #[cfg(feature = "code-rule-profile")]
                if result.is_err() {
                    crate::code_rule_dsl::profile::record_anchor_batch_failure();
                }
                result?;
            }
        }
        Ok(())
    }

    fn template(
        state: &mut State,
        lang: u32,
        ids: &[usize],
        plans: &[&AnchorPlan],
    ) -> Arc<super::lower::Statement> {
        let key = (lang, ids.to_vec());
        if let Some(template) = state.templates.get(&key) {
            return Arc::clone(template);
        }
        #[cfg(feature = "code-rule-profile")]
        let mut lower =
            crate::code_rule_dsl::profile::trace::Span::start("anchor-lower", "template");
        let statement = Arc::new(super::lower::batch_files(plans, lang, &[]));
        #[cfg(feature="code-rule-profile")]
        lower.detail(||serde_json::json!({"plans":plans.len(),"files":0,"template":true,"sqlBytes":statement.queries.iter().map(String::len).sum::<usize>()+statement.stages.iter().map(|s|s.sql.len()).sum::<usize>(),"materializations":statement.stages.len(),"resultQueries":statement.queries.len()}));
        if ids.iter().all(|id| state.plans.contains_key(id)) {
            state.templates.insert(key, Arc::clone(&statement));
        }
        statement
    }

    fn select_group(
        state: &mut State,
        lang: u32,
        ids: &[usize],
        plans: &[&AnchorPlan],
        covered: bool,
    ) -> Result<()> {
        let files: Vec<_> = state
            .files
            .iter()
            .filter(|(_, file)| {
                file.lang == lang && ids.iter().any(|id| !file.roots.contains_key(id))
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
            return Self::select_group(state, lang, &ids, &plans, covered);
        }
        // Keep bind counts bounded, without issuing a query for each file or rule.
        for files in files.chunks(16_384) {
            let fids: Vec<_> = files.iter().map(|(_, file)| file.fid).collect();
            let nodes: HashMap<_, _> = files
                .iter()
                .map(|(_, file)| (file.fid, file.nodes))
                .collect();
            let statement = Self::template(state, lang, ids, plans);
            let stages = statement
                .stages
                .iter()
                .filter(|s| !(covered && s.covering))
                .count();
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
        Self::select_group(state, lang, &ids, plans, false)
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
        let mut state = self.state.lock();
        if let Some(state) = state.as_mut()
            && let Some(file) = state.files.get(path)
        {
            state.writer.remove_known(file.fid)?;
            state.files.remove(path);
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
        let store = match state.idle.pop() {
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
                state.idle.push(store);
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
