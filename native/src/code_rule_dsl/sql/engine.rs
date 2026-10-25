//! The SQL execution boundary: bounded publication plans over typed native matcher relations.
//! A session owns prepared statements. Changing files replaces batch slots, never compiled SQL.
use super::{
    exec::{
        self, Batch, BatchSlot, Captured, Census, ClauseRun, CompilePhases, FileContext, Program,
        RunFile, SqlError, SqlMatch, Stats,
    },
    fragments::{
        self, Active, DemandStore, Fragment,
        capture_cell::{Cell, decode_list},
        span_capture,
    },
    plan::Unsupported,
    timing::{Started, timed},
};
use crate::code_rule_dsl::{
    matcher::{self, GrammarMatcher},
    tape::TapeDoc,
};
use rusqlite::{Connection, vtab::Module};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::CString,
    sync::{Arc, LazyLock, Mutex},
};

mod planning;
mod statistics;
mod structural;
#[cfg(test)]
mod tests;

const VERSION: u32 = 3;
const ARMS: usize = 256;
const CACHE_LIMIT: usize = 1024;
type CacheKey = ([u8; 32], u32, Option<String>);
static CACHE: LazyLock<Mutex<BTreeMap<CacheKey, Arc<planning::Template>>>> =
    LazyLock::new(Mutex::default);

#[derive(Clone)]
struct Clause {
    grammar: GrammarMatcher,
    template: Arc<planning::Template>,
    each: Option<String>,
    allowance: Option<(usize, usize)>,
}
#[derive(Default)]
pub(in crate::code_rule_dsl) struct ProgramCompiler {
    clauses: Vec<Clause>,
    phases: CompilePhases,
    hits: usize,
    statistics: statistics::PlannerStats,
    statistics_error: Option<String>,
}
impl ProgramCompiler {
    pub(in crate::code_rule_dsl) fn clause(
        &mut self,
        grammar: &GrammarMatcher,
        each: Option<&str>,
    ) -> Result<usize, Unsupported> {
        let key = (grammar.source_digest, VERSION, each.map(str::to_owned));
        let mut cache = CACHE.lock().unwrap();
        let template = if let Some(template) = cache.get(&key) {
            self.hits += 1;
            template.clone()
        } else {
            let template = timed(&mut self.phases.lower, || {
                Arc::new(planning::Template::compile(grammar, each))
            });
            if cache.len() >= CACHE_LIMIT {
                cache.pop_first();
            }
            cache.insert(key, template.clone());
            template
        };
        drop(cache);
        let id = self.clauses.len();
        self.clauses.push(Clause {
            grammar: template.owner.clone(),
            template,
            each: each.map(str::to_owned),
            allowance: None,
        });
        Ok(id)
    }
    pub(in crate::code_rule_dsl) fn statistics_unavailable(&mut self, reason: String) {
        self.statistics_error = Some(reason);
    }
    pub(in crate::code_rule_dsl) fn read_statistics(
        &mut self,
        store: Arc<crate::content_store::ContentStore>,
    ) -> Result<(), SqlError> {
        let mut kinds = BTreeSet::new();
        for c in &self.clauses {
            kinds.extend(
                c.grammar
                    .anchors
                    .root_kinds()
                    .into_iter()
                    .flatten()
                    .copied(),
            );
            if let Some(leaf) = c
                .template
                .predicate
                .as_ref()
                .and_then(|p| p.rare_descendant())
            {
                kinds.extend(leaf.iter().copied());
                kinds.extend(
                    c.grammar
                        .anchors
                        .root_kinds()
                        .into_iter()
                        .flatten()
                        .copied(),
                );
            }
            if let Some(parent) = c
                .template
                .predicate
                .as_ref()
                .and_then(structural::Predicate::parent_seed)
            {
                kinds.extend(parent.seed_kinds().unwrap().iter().copied());
            }
        }
        if !kinds.is_empty() {
            self.statistics =
                statistics::PlannerStats::read(store, &kinds.into_iter().collect::<Vec<_>>())?;
        }
        Ok(())
    }
    pub(in crate::code_rule_dsl) fn clause_with_limits(
        &mut self,
        grammar: &GrammarMatcher,
        each: Option<&str>,
        limits: &crate::code_rule_dsl::model::Limits,
    ) -> Result<usize, Unsupported> {
        let id = self.clause(grammar, each)?;
        self.clauses[id].allowance = Some((limits.max_visited_nodes, limits.max_visits_per_node));
        Ok(id)
    }
    pub(in crate::code_rule_dsl) fn finish_with_batch(
        mut self,
        _: usize,
    ) -> (Compiled, CompilePhases) {
        let started = Started::now();
        let queries = planning::queries(&self.clauses, &self.statistics);
        let registry = planning::manifest(&self.clauses);
        self.phases.finish.add(started.elapsed());
        (
            Compiled {
                clauses: Arc::new(self.clauses),
                queries: Arc::new(queries),
                hits: self.hits,
                registry,
                statistics: self.statistics,
                statistics_error: self.statistics_error,
            },
            self.phases,
        )
    }
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn finish(self) -> (Compiled, CompilePhases) {
        self.finish_with_batch(64)
    }
}
#[cfg(test)]
pub(in crate::code_rule_dsl) fn compile_enumerating(
    grammars: &[&GrammarMatcher],
    each: Option<&str>,
) -> Result<Compiled, SqlError> {
    let mut compiler = ProgramCompiler::default();
    for g in grammars {
        compiler.clause(g, each).map_err(SqlError::Unsupported)?;
    }
    Ok(compiler.finish_with_batch(64).0)
}
pub(in crate::code_rule_dsl) struct Compiled {
    clauses: Arc<Vec<Clause>>,
    queries: Arc<Vec<String>>,
    hits: usize,
    registry: serde_json::Value,
    statistics: statistics::PlannerStats,
    statistics_error: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::code_rule_dsl) enum Unknown {
    Limit(matcher::UtilLimit),
    Deadline,
}
#[derive(Default, Clone, Debug)]
pub(in crate::code_rule_dsl) struct FileOutcome {
    pub matches: Vec<SqlMatch>,
    pub unknown: Option<Unknown>,
}
impl std::ops::Deref for FileOutcome {
    type Target = [SqlMatch];
    fn deref(&self) -> &Self::Target {
        &self.matches
    }
}
impl FileOutcome {
    pub fn as_slice(&self) -> &[SqlMatch] {
        &self.matches
    }
}
impl Compiled {
    pub(in crate::code_rule_dsl) fn session(&self) -> Result<Session, SqlError> {
        Session::new(self)
    }
    pub(in crate::code_rule_dsl) fn executions_for(&self, runs: &[ClauseRun]) -> usize {
        runs.iter()
            .filter(|r| !r.files.is_empty())
            .map(|r| r.statement / ARMS)
            .collect::<BTreeSet<_>>()
            .len()
    }
    pub(in crate::code_rule_dsl) fn census(&self) -> Census {
        let bytes = self.queries.iter().map(String::len).sum();
        Census {
            steps: self.queries.len(),
            texts: self.queries.len(),
            bytes,
            stored: bytes,
            shapes: self.clauses.len(),
            ..Default::default()
        }
    }
    pub(in crate::code_rule_dsl) fn relational_plan(&self) -> serde_json::Value {
        serde_json::json!({"compiler":"typed-matcher-fragments", "compilerVersion":VERSION,"cacheHits":self.hits,"clauses":self.clauses.len(),"sqlStatements":self.queries.len(),"maxMatchingStatementsPerPartition":self.queries.len(),"sqlBytes":self.queries.iter().map(String::len).sum::<usize>(),"nativeComposition":true,"registry":self.registry,"statisticsError":self.statistics_error,"statistics":self.statistics.kinds,"descendantAnchors":planning::decisions(&self.clauses,&self.statistics)})
    }
    pub(in crate::code_rule_dsl) fn run_batches(
        &self,
        files: Vec<RunFile>,
        _: usize,
    ) -> Result<Vec<FileOutcome>, SqlError> {
        // Complete job domain; SQL splits by query limits, never by individual files.
        let mut session = self.session()?;
        let mut runs = (0..self.clauses.len())
            .map(|statement| ClauseRun {
                statement,
                files: vec![],
            })
            .collect::<Vec<_>>();
        for (file, (_, clause, _)) in files.iter().enumerate() {
            if let Some((id, roots)) = clause {
                runs[*id].files.push((file, roots.clone()));
            }
        }
        let mut output = vec![FileOutcome::default(); files.len()];
        let result = session.run(
            files.into_iter().map(|(doc, _, ctx)| (doc, ctx)).collect(),
            &runs,
        )?;
        for (run, found) in runs.iter().zip(result) {
            for ((file, _), rows) in run.files.iter().zip(found) {
                output[*file] = rows;
            }
        }
        Ok(output)
    }
}

pub(in crate::code_rule_dsl) struct Session {
    // The observer must drop before SQLite closes its connection.
    #[cfg(any(test, feature = "unbounded-work"))]
    _profile: Option<Box<super::profiling::Profiler>>,
    db: Connection,
    clauses: Arc<Vec<Clause>>,
    queries: Arc<Vec<String>>,
    active: Vec<Active>,
    slot: Arc<BatchSlot>,
    demands: Arc<DemandStore>,
    stats: Stats,
    statistics: statistics::PlannerStats,
    pub(in crate::code_rule_dsl) last_profile: serde_json::Value,
    batch_metrics: Vec<Arc<Mutex<fragments::batch::Metrics>>>,
}
impl Session {
    fn new(compiled: &Compiled) -> Result<Self, SqlError> {
        if compiled.queries.len() > 25 {
            return Err(SqlError::Sql(
                "matching plan exceeds 100 executions across four worker partitions".into(),
            ));
        }
        let start = Started::now();
        let db = Connection::open_in_memory()?;
        #[cfg(any(test, feature = "unbounded-work"))]
        // SAFETY: the observer is dropped before its live connection.
        let profile = unsafe { super::profiling::Profiler::configured(&db) }?;
        let slot = Arc::new(BatchSlot::default());
        const NODES: Module<'static, super::nodes::NodeTable> = Module::eponymous_only_module();
        db.create_module(c"node", &NODES, Some(slot.clone()))?;
        let demands = Arc::new(DemandStore::default());
        demands.register_group(&db)?;
        let mut active = vec![];
        for (i, clause) in compiled.clauses.iter().enumerate() {
            let state: Active = Arc::default();
            if clause.template.predicate.is_some() {
                fragments::candidates::register(
                    &db,
                    &CString::new(format!("candidates_{i}")).unwrap(),
                    state.clone(),
                    clause.grammar.anchors.root_kinds().and_then(|k| {
                        compiled
                            .statistics
                            .rows(&k.iter().copied().collect::<Vec<_>>())
                    }),
                )?;
                fragments::candidates::register_files(
                    &db,
                    &CString::new(format!("candidate_files_{i}")).unwrap(),
                    state.clone(),
                )?;
                demands.register(&db, &format!("demand_{i}"), vec![])?;
            }
            active.push(state);
        }
        let mut batch_metrics = vec![];
        for (chunk, clauses) in compiled.clauses.chunks(ARMS).enumerate() {
            let metrics = Arc::default();
            let members = clauses
                .iter()
                .enumerate()
                .filter(|(_, c)| c.template.native.is_some())
                .map(|(i, c)| {
                    let id = chunk * ARMS + i;
                    fragments::batch::Member {
                        id,
                        active: active[id].clone(),
                        prefix: c.template.prefix.clone(),
                    }
                })
                .collect();
            fragments::batch::register(
                &db,
                &CString::new(format!("matcher_batch_{chunk}")).unwrap(),
                members,
                demands.clone(),
                Arc::clone(&metrics),
                clauses
                    .iter()
                    .map(|c| c.template.names.len())
                    .max()
                    .unwrap_or(0),
            )?;
            batch_metrics.push(metrics);
        }
        db.set_prepared_statement_cache_capacity(compiled.queries.len().max(1));
        for sql in compiled.queries.iter() {
            db.prepare_cached(sql)?;
        }
        let stats = Stats {
            sessions: 1,
            texts: compiled.queries.len(),
            setup: start.elapsed(),
            ..Default::default()
        };
        Ok(Self {
            #[cfg(any(test, feature = "unbounded-work"))]
            _profile: profile,
            db,
            clauses: compiled.clauses.clone(),
            queries: compiled.queries.clone(),
            active,
            slot,
            demands,
            stats,
            statistics: compiled.statistics.clone(),
            last_profile: serde_json::Value::Null,
            batch_metrics,
        })
    }
    pub(in crate::code_rule_dsl) fn stats(&self) -> Stats {
        self.stats
    }
    pub(in crate::code_rule_dsl) fn prepared_statements(&self) -> usize {
        self.queries.len()
    }
    pub(in crate::code_rule_dsl) fn run(
        &mut self,
        docs: Vec<(TapeDoc, FileContext)>,
        runs: &[ClauseRun],
    ) -> Result<Vec<Vec<FileOutcome>>, SqlError> {
        let statistics_start = Started::now();
        let fresh = self.statistics.refresh()?;
        let statistics_cpu = statistics_start.elapsed().cpu;
        for metrics in &self.batch_metrics {
            *metrics.lock().unwrap() = Default::default();
        }
        // Only a changed decision invalidates prepared SQL. Numeric drift alone does not.
        if planning::decisions(&self.clauses, &fresh)
            != planning::decisions(&self.clauses, &self.statistics)
        {
            self.queries = Arc::new(planning::queries(&self.clauses, &fresh));
            self.db.flush_prepared_statement_cache();
            let prepare = Started::now();
            for sql in self.queries.iter() {
                self.db.prepare_cached(sql)?;
            }
            self.stats.setup.add(prepare.elapsed());
        }
        self.statistics = fresh;
        #[cfg(any(test, feature = "unbounded-work"))]
        let memory_before_match =
            serde_json::from_str::<serde_json::Value>(&crate::native_memory_profile())
                .unwrap_or_default();
        #[cfg(any(test, feature = "unbounded-work"))]
        let navigation_bytes = docs
            .iter()
            .map(|(doc, _)| doc.navigation_bytes())
            .sum::<usize>();
        let started = Started::now();
        let batch = Batch::of(docs, Arc::new(Program { terminals: vec![] }));
        self.stats.node_facts.add(started.elapsed());
        let mut positions = BTreeMap::new();
        let mut output = runs
            .iter()
            .map(|r| vec![FileOutcome::default(); r.files.len()])
            .collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        for (run, r) in runs.iter().enumerate() {
            if r.statement >= self.clauses.len() || !seen.insert(r.statement) {
                return Err(SqlError::Sql("invalid or duplicate clause run".into()));
            }
            for (pos, (file, _)) in r.files.iter().enumerate() {
                if *file >= batch.files.len()
                    || positions.insert((r.statement, *file), (run, pos)).is_some()
                {
                    return Err(SqlError::Sql("invalid or duplicate file demand".into()));
                }
            }
        }
        *self.slot.current.write() = Some(batch.clone());
        let _scopes = super::terminals::BatchScopes::begin();
        let mut boundaries = vec![];
        for (id, clause) in self.clauses.iter().enumerate() {
            let scope = runs
                .iter()
                .find(|r| r.statement == id)
                .map(|r| r.files.clone())
                .unwrap_or_default();
            let mut grammar = clause.grammar.clone();
            if let Some(native) = &clause.template.native {
                grammar.matcher = native.clone();
            }
            let f = Arc::new(Fragment {
                grammar,
                names: clause.template.names.clone(),
                requests: vec![],
                work: usize::MAX,
                each: clause.each.clone(),
                metrics: Arc::default(),
                scope: Some(scope),
                allowance: clause.allowance,
            });
            boundaries.push(f.metrics.clone());
            *self.active[id].write().unwrap() = Some((batch.clone(), f));
        }
        let execution = Started::now();
        let mut statements = vec![];
        let result = (|| -> Result<(), SqlError> {
            for (chunk, sql) in self.queries.iter().enumerate() {
                if !runs
                    .iter()
                    .any(|r| r.statement / ARMS == chunk && !r.files.is_empty())
                {
                    continue;
                }
                let timer = Started::now();
                let mut stmt = self.db.prepare_cached(sql)?;
                stmt.reset_status(rusqlite::StatementStatus::VmStep);
                let mut row_count = 0;
                {
                    let mut rows = stmt.query([])?;
                    while let Some(row) = rows.next()? {
                        row_count += 1;
                        let id = row.get::<_, u32>(0)? as usize;
                        let file = row.get::<_, u32>(1)? as usize;
                        let &(run, pos) = positions.get(&(id, file)).ok_or_else(|| {
                            SqlError::Sql("fragment escaped its requested scope".into())
                        })?;
                        let target = &mut output[run][pos];
                        let truth = row.get::<_, i64>(3)?;
                        if truth != 1 {
                            target.unknown = Some(if truth == -5 {
                                Unknown::Deadline
                            } else if let Some(limit) = fragments::failure::decode(truth) {
                                Unknown::Limit(limit)
                            } else {
                                return Err(SqlError::Sql(format!(
                                    "invalid fragment completion tag {truth}"
                                )));
                            });
                            target.matches.clear();
                            continue;
                        }
                        if target.unknown.is_some() {
                            continue;
                        }
                        let template = &self.clauses[id].template;
                        let mut captures = vec![];
                        for (column, name) in template.names.iter().enumerate() {
                            let capture = match row.get::<_, Cell>(5 + column)? {
                                Cell::Null => continue,
                                Cell::Integer(v) if template.counts.contains(name) => {
                                    Captured::Count(v)
                                }
                                Cell::Integer(v) => {
                                    Captured::Node(u32::try_from(v).map_err(|_| {
                                        SqlError::Sql("invalid capture node".into())
                                    })?)
                                }
                                Cell::List(bytes) => {
                                    let nodes = decode_list(
                                        &bytes,
                                        file as u32,
                                        batch.files[file].facts.len(),
                                    )?;
                                    Captured::List(
                                        nodes.first().zip(nodes.last()).map(|(a, b)| (*a, *b)),
                                    )
                                }
                                Cell::Span(bytes) => {
                                    let span = span_capture::decode(&bytes)?;
                                    if span.file != file as u32 {
                                        return Err(SqlError::Sql(
                                            "capture belongs to another file".into(),
                                        ));
                                    }
                                    Captured::Text(span.text.unwrap_or_default(), span.range)
                                }
                            };
                            captures.push((name.clone(), capture));
                        }
                        target.matches.push(SqlMatch {
                            root: row.get(2)?,
                            captures,
                        });
                    }
                }
                statements.push(serde_json::json!({"sqlBytes":sql.len(),"rows":row_count,"vm":stmt.get_status(rusqlite::StatementStatus::VmStep),"cpuNs":timer.elapsed().cpu,"wallNs":timer.elapsed().wall}));
                self.stats.statements += 1;
                self.stats.rows += row_count;
                self.stats.select.add(timer.elapsed());
                #[cfg(any(test, feature = "unbounded-work"))]
                exec::record(sql, timer.elapsed().wall, row_count);
            }
            Ok(())
        })();
        self.stats.time.add(execution.elapsed());
        self.last_profile = serde_json::json!({"statements":statements,"boundaries":boundaries.iter().map(|m|m.lock().unwrap().clone()).collect::<Vec<_>>(),"nativeBatches":self.batch_metrics.iter().map(|m|serde_json::to_value(&*m.lock().unwrap()).unwrap()).collect::<Vec<_>>(),"unknownFiles":output.iter().flatten().filter(|o|o.unknown.is_some()).count()});
        self.last_profile["planning"] = serde_json::json!({"statisticsCpuNs":statistics_cpu,"source":if self.statistics.source.is_some(){"SQL kind vocabulary"}else{"unavailable"},"kindCounts":self.statistics.kinds,"anchorShapes":planning::decisions(&self.clauses,&self.statistics)});
        #[cfg(any(test, feature = "unbounded-work"))]
        {
            self.last_profile["memory"] = serde_json::json!({
                "scope": "allocator snapshots are process-global; navigation bytes belong to this partition",
                "navigationBytes": navigation_bytes,
                "beforeMatch": memory_before_match,
                "afterMatch": serde_json::from_str::<serde_json::Value>(&crate::native_memory_profile()).unwrap_or_default(),
            });
        }
        drop(_scopes);
        #[cfg(any(test, feature = "unbounded-work"))]
        {
            self.last_profile["memory"]["afterScopeRelease"] =
                serde_json::from_str(&crate::native_memory_profile()).unwrap_or_default();
        }
        // A failed statement cannot leave a stale batch or demand handle usable by the next run.
        for active in &self.active {
            active.write().unwrap().take();
        }
        self.slot.current.write().take();
        self.demands.clear();
        result?;
        // Publication order is local to a file. A global UNION merge keeps every producer
        // cursor alive together. Preserve enumeration order for equal roots with a stable sort.
        for file in output.iter_mut().flatten() {
            file.matches.sort_by_key(|m| m.root);
        }
        Ok(output)
    }
}

#[cfg(test)]
impl Compiled {
    pub fn statements(&self) -> impl Iterator<Item = &String> {
        self.queries.iter()
    }
    pub fn planned_statements(&self) -> Vec<String> {
        self.queries.as_ref().clone()
    }
    pub fn loop_statements(&self) -> Vec<usize> {
        vec![]
    }
    pub fn statement_count(&self) -> (usize, usize) {
        (0, self.queries.len())
    }
}
impl IntoIterator for FileOutcome {
    type Item = SqlMatch;
    type IntoIter = std::vec::IntoIter<SqlMatch>;
    fn into_iter(self) -> Self::IntoIter {
        self.matches.into_iter()
    }
}
#[cfg(test)]
impl Session {
    pub fn plans(&mut self) -> Result<Vec<(String, exec::QueryPlan)>, SqlError> {
        self.queries
            .iter()
            .map(|sql| {
                let rows = self
                    .db
                    .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(3)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((sql.clone(), rows))
            })
            .collect()
    }
}
