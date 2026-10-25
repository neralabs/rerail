//! Batched AstMatcher binding. SQL sees completed typed outcomes, never instructions.
pub(in crate::code_rule_dsl::sql) use self::capture_cell::{Cell, encode_list};
pub(in crate::code_rule_dsl::sql) mod batch;
pub(in crate::code_rule_dsl::sql) mod candidates;
pub(in crate::code_rule_dsl::sql) mod capture_cell;
pub(in crate::code_rule_dsl::sql) mod capture_env;
mod demand;
pub(in crate::code_rule_dsl::sql) mod failure;
pub(in crate::code_rule_dsl::sql) mod span_capture;
pub(in crate::code_rule_dsl::sql) use crate::code_rule_dsl::{
    matcher::{self, GrammarMatcher, Stopped},
    sql::{exec::Batch, plan::SlotKind, terminals::with_scopes, timing::Started},
    tape::TapeDoc,
};
use ast_grep_core::{meta_var::MetaVarEnv, source::Content};
pub(in crate::code_rule_dsl::sql) use demand::DemandStore;
pub(in crate::code_rule_dsl::sql) use rusqlite::{
    Connection, Error, Result,
    fallible_iterator::FallibleIterator,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor, sqlite3_vtab, sqlite3_vtab_cursor,
    },
};
pub(in crate::code_rule_dsl::sql) use std::{
    borrow::Cow,
    collections::BTreeMap,
    ffi::{CStr, CString, c_int},
    marker::PhantomData,
    sync::{Arc, Mutex},
};

#[derive(Default, Clone, serde::Serialize)]
pub(in crate::code_rule_dsl::sql) struct Metrics {
    pub filters: usize,
    pub roots: usize,
    pub peak_request_rows: usize,
    pub rows: usize,
    pub completed_roots: usize,
    pub logical_unknown_roots: usize,
    pub native_cpu_ns: u64,
    pub native_wall_ns: u64,
    pub remaining_work: BTreeMap<u32, usize>,
}

/// Capture inputs are immutable batch data. The key includes request identity, not just the root.
#[derive(Clone)]
pub(in crate::code_rule_dsl::sql) struct Request {
    pub file: u32,
    pub node: u32,
    pub inputs: Vec<(String, SlotKind, Cell)>,
}

pub(in crate::code_rule_dsl::sql) struct Fragment {
    pub grammar: GrammarMatcher,
    pub names: Vec<String>,
    pub requests: Vec<Request>,
    pub work: usize,
    pub each: Option<String>,
    pub metrics: Arc<Mutex<Metrics>>,
    pub scope: Option<Vec<(usize, Option<Arc<[u32]>>)>>,
    pub allowance: Option<(usize, usize)>,
}

impl Fragment {
    pub fn new(grammar: &GrammarMatcher, batch: &Batch, work: usize) -> Self {
        let mut analysis = matcher::CaptureAnalysis::default();
        let mut names = analysis.binds(&grammar.matcher);
        names.extend(analysis.counts(&grammar.matcher));
        let mut requests = vec![];
        for (file, entry) in batch.files.iter().enumerate() {
            if entry.dialect != grammar.language
                || !grammar.applies
                || grammar.file.as_ref().is_some_and(|c| {
                    entry
                        .context
                        .path
                        .as_ref()
                        .is_none_or(|p| !c.holds(p, entry.context.module.as_deref()))
                })
            {
                continue;
            }
            for node in 0..entry.facts.len() {
                if grammar
                    .anchors
                    .root_kinds()
                    .is_none_or(|k| k.contains(&entry.facts.kind(node)))
                {
                    requests.push(Request {
                        file: file as u32,
                        node,
                        inputs: vec![],
                    });
                }
            }
        }
        Self {
            grammar: grammar.clone(),
            names: names.into_iter().collect(),
            requests,
            work,
            each: None,
            metrics: Arc::default(),
            scope: None,
            allowance: None,
        }
    }

    pub(in crate::code_rule_dsl::sql) fn requests_for(
        &self,
        batch: &Batch,
        scope: &[(usize, Option<Arc<[u32]>>)],
    ) -> Result<Vec<Request>> {
        let mut requests = vec![];
        for (file, selected) in scope {
            let entry = batch
                .files
                .get(*file)
                .ok_or_else(|| invalid("invalid scope file"))?;
            if entry.dialect != self.grammar.language
                || !self.grammar.applies
                || self.grammar.file.as_ref().is_some_and(|c| {
                    entry
                        .context
                        .path
                        .as_ref()
                        .is_none_or(|p| !c.holds(p, entry.context.module.as_deref()))
                })
            {
                continue;
            }
            let kinds = self.grammar.anchors.root_kinds();
            let roots = match selected {
                Some(roots) => roots.to_vec(),
                None => kinds.map_or_else(
                    || (0..entry.facts.len()).collect(),
                    |k| entry.facts.kind_ids(&k.iter().copied().collect::<Vec<_>>()),
                ),
            };
            for node in roots {
                if node >= entry.facts.len() {
                    return Err(invalid("scope root outside document"));
                }
                if kinds.is_none_or(|k| k.contains(&entry.facts.kind(node))) {
                    requests.push(Request {
                        file: *file as u32,
                        node,
                        inputs: vec![],
                    });
                }
            }
        }
        Ok(requests)
    }

    fn evaluate(&self, batch: &Batch, requests: &[Request], keys: &[usize]) -> Result<Vec<Row>> {
        let mut groups: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for &key in keys {
            let request = requests
                .get(key)
                .ok_or_else(|| invalid("invalid request key"))?;
            groups.entry(request.file).or_default().push(key);
        }
        let mut output = vec![];
        for (file, mut keys) in groups {
            keys.sort_by_key(|&key| (requests[key].node, key));
            output.extend(self.evaluate_file(batch, requests, file, keys.into_iter(), None)?);
        }
        Ok(output)
    }

    // The file-first batch already owns canonical request order, so it needs no regroup or sort.
    fn evaluate_file(
        &self,
        batch: &Batch,
        requests: &[Request],
        file: u32,
        keys: impl Iterator<Item = usize>,
        remaining: Option<usize>,
    ) -> Result<Vec<Row>> {
        let mut keys = keys.peekable();
        if keys.peek().is_none() {
            return Ok(vec![]);
        }
        let started = Started::now();
        let mut output = vec![];
        let mut metrics = self.metrics.lock().unwrap();
        metrics.peak_request_rows = metrics.peak_request_rows.max(requests.len());
        let entry = batch
            .files
            .get(file as usize)
            .ok_or_else(|| invalid("invalid document"))?;
        let mut provisional = vec![];
        let mut stopped = None;
        let mut left = remaining.unwrap_or_else(|| {
            self.allowance.map_or(self.work, |(base, per_node)| {
                base.max(per_node.saturating_mul(entry.facts.len() as usize))
            })
        });
        let _budget = entry.context.budget.as_ref().map(|b| b.clone().resume());
        let (result, _) = crate::code_rule_dsl::budget::with_allowance(u64::MAX, || {
            with_scopes(file as usize, entry, || {
                matcher::reuse_file_memo(&entry.root, || -> Result<()> {
                    matcher::begin_clause(&self.grammar, left);
                    // Always close clause state, including a transport error.
                    let result = (|| -> Result<()> {
                        for key in keys {
                            let r = &requests[key];
                            if r.node >= entry.facts.len() {
                                return Err(invalid("root outside document"));
                            }
                            let bindings = r
                                .inputs
                                .iter()
                                .map(|(n, k, _)| (n.clone(), *k))
                                .collect::<Vec<_>>();
                            let values = r
                                .inputs
                                .iter()
                                .map(|(_, _, v)| v.clone())
                                .collect::<Vec<_>>();
                            let env = if r.inputs.is_empty() {
                                MetaVarEnv::new()
                            } else {
                                capture_env::environment(
                                    file,
                                    &entry.facts,
                                    |id| matcher::node_at(&entry.root, id),
                                    &bindings,
                                    &values,
                                )?
                            };
                            let inputs = env;
                            let mut env = Cow::Borrowed(&inputs);
                            metrics.roots += 1;
                            match matcher::fragment_at(
                                &self.grammar,
                                matcher::node_at(&entry.root, r.node),
                                &mut env,
                            ) {
                                Ok(1) if self.each.is_some() => {
                                    left = matcher::end_clause().0;
                                    let mut transport = None;
                                    let mut ordinal = 0;
                                    let result = matcher::enumerate_bound(
                                        &entry.root,
                                        &self.grammar,
                                        r.node,
                                        env.into_owned(),
                                        &inputs,
                                        &mut left,
                                        self.each.as_ref().unwrap(),
                                        |env, _| match capture_values(file, &env, &self.names) {
                                            Ok(values) => {
                                                provisional.push(Row {
                                                    request: key as i64,
                                                    file,
                                                    node: r.node,
                                                    truth: 1,
                                                    ordinal,
                                                    values,
                                                });
                                                ordinal += 1;
                                                Ok(true)
                                            }
                                            Err(error) => {
                                                transport = Some(error);
                                                Ok(false)
                                            }
                                        },
                                    );
                                    matcher::begin_clause(&self.grammar, left);
                                    if let Some(error) = transport {
                                        return Err(error);
                                    }
                                    if let Err(reason) = result {
                                        stopped = Some(reason);
                                        break;
                                    }
                                    metrics.completed_roots += 1;
                                }
                                Ok(0) => {
                                    metrics.completed_roots += 1;
                                }
                                Ok(truth) => {
                                    metrics.completed_roots += 1;
                                    metrics.logical_unknown_roots += usize::from(truth == 2);
                                    provisional.push(Row {
                                        request: key as i64,
                                        file,
                                        node: r.node,
                                        truth,
                                        ordinal: 0,
                                        values: if truth == 1 {
                                            capture_values(file, &env, &self.names)?
                                        } else {
                                            vec![Cell::Null; self.names.len()]
                                        },
                                    });
                                }
                                Err(reason) => {
                                    stopped = Some(reason);
                                    break;
                                }
                            }
                        }
                        Ok(())
                    })();
                    left = matcher::end_clause().0;
                    result
                })
            })
        });
        result?;
        metrics.remaining_work.insert(file, left);
        match stopped {
            None => output.extend(provisional),
            Some(Stopped::Limit(limit)) => output.push(Row {
                request: -1,
                file,
                node: 0,
                ordinal: 0,
                truth: failure::encode(&format!("{limit:?}"))
                    .ok_or_else(|| invalid("unknown native failure"))?,
                values: vec![Cell::Null; self.names.len()],
            }),
            Some(Stopped::Deadline) => output.push(Row {
                request: -1,
                file,
                node: 0,
                truth: -5,
                ordinal: 0,
                values: vec![Cell::Null; self.names.len()],
            }),
        }
        let elapsed = started.elapsed();
        metrics.rows += output.len();
        metrics.native_cpu_ns += elapsed.cpu;
        metrics.native_wall_ns += elapsed.wall;
        Ok(output)
    }
}

fn capture_values(file: u32, env: &MetaVarEnv<'_, TapeDoc>, names: &[String]) -> Result<Vec<Cell>> {
    names
        .iter()
        .map(|name| {
            if let Some(nodes) = env.get_labels(name) {
                return Ok(Cell::List(encode_list(
                    file,
                    &nodes.iter().map(|n| n.node_id() as u32).collect::<Vec<_>>(),
                )?));
            }
            if let Some(node) = env.get_match(name) {
                return Ok(Cell::Integer(node.node_id() as i64));
            }
            if let Some(range) = matcher::exported_span(env, name) {
                return Ok(Cell::Span(span_capture::encode(&span_capture::Span {
                    file,
                    node: None,
                    text: matcher::exported_text(env, name),
                    range,
                })?));
            }
            if let Some(bytes) = env.get_transformed(name) {
                return Ok(Cell::Integer(
                    <TapeDoc as ast_grep_core::Doc>::Source::encode_bytes(bytes)
                        .parse()
                        .map_err(|_| invalid("unsupported transformed capture"))?,
                ));
            }
            Ok(Cell::Null)
        })
        .collect()
}

#[derive(Clone)]
struct Row {
    request: i64,
    file: u32,
    node: u32,
    truth: i64,
    ordinal: i64,
    values: Vec<Cell>,
}
pub(in crate::code_rule_dsl::sql) fn invalid(message: &str) -> Error {
    Error::ModuleError(format!("matcher fragment: {message}"))
}

pub(in crate::code_rule_dsl::sql) type Active =
    Arc<std::sync::RwLock<Option<(Arc<Batch>, Arc<Fragment>)>>>;

pub(in crate::code_rule_dsl::sql) fn register(
    db: &Connection,
    name: &CStr,
    batch: Arc<Batch>,
    fragment: Arc<Fragment>,
) -> Result<()> {
    let names = fragment.names.clone();
    register_slot(
        db,
        name,
        Arc::new(std::sync::RwLock::new(Some((batch, fragment)))),
        names,
        Arc::default(),
    )
}

pub(in crate::code_rule_dsl::sql) fn register_slot(
    db: &Connection,
    name: &CStr,
    active: Active,
    names: Vec<String>,
    demands: Arc<DemandStore>,
) -> Result<()> {
    register_slot_estimated(db, name, active, names, demands, None)
}
pub(in crate::code_rule_dsl::sql) fn register_slot_estimated(
    db: &Connection,
    name: &CStr,
    active: Active,
    names: Vec<String>,
    demands: Arc<DemandStore>,
    estimate: Option<u64>,
) -> Result<()> {
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(name, &MODULE, Some((active, names, demands, estimate)))
}

#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    active: Active,
    width: usize,
    demands: Arc<DemandStore>,
    estimate: Option<u64>,
}
// SAFETY: SQLite's base is first; cursors retain the batch slot and demand store.
unsafe impl<'v> VTab<'v> for Table {
    type Aux = (Active, Vec<String>, Arc<DemandStore>, Option<u64>);
    type Cursor = Cursor<'v>;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Self::Aux>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let (active, names, demands, estimate) = aux.ok_or_else(|| invalid("missing binding"))?;
        let columns = (0..names.len())
            .map(|i| format!(",c{i}"))
            .collect::<String>();
        let schema = CString::new(format!("CREATE TABLE x(file INTEGER,node INTEGER,truth INTEGER,ordinal INTEGER{columns},request INTEGER HIDDEN,demand INTEGER HIDDEN)")).map_err(|_| invalid("invalid schema"))?;
        Ok((
            Cow::Owned(schema),
            Self {
                base: sqlite3_vtab::default(),
                active: active.clone(),
                width: names.len(),
                demands: demands.clone(),
                estimate: *estimate,
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let request = 4 + self.width as c_int;
        let constraints = info
            .constraints()
            .enumerate()
            .filter(|(_, c)| {
                c.is_usable() && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            })
            .map(|(i, c)| (i, c.column()))
            .collect::<Vec<_>>();
        if let Some(&(i, _)) = constraints.iter().find(|(_, c)| *c == request + 1) {
            info.constraint_usage(i).set_argv_index(1);
            info.constraint_usage(i).set_omit(true);
            info.set_idx_num(2);
        } else if let Some(&(i, _)) = constraints.iter().find(|(_, c)| *c == request) {
            if !info.is_in_constraint(i)? || !info.set_in_constraint(i, true)? {
                return Ok(false);
            }
            info.constraint_usage(i).set_argv_index(1);
            info.constraint_usage(i).set_omit(true);
            info.set_idx_num(1);
        } else {
            info.set_idx_num(0);
        }
        let count = self
            .estimate
            .map(|v| v.min(i64::MAX as u64) as i64)
            .or_else(|| {
                self.active
                    .read()
                    .unwrap()
                    .as_ref()
                    .filter(|(_, f)| f.scope.is_none())
                    .map(|(_, f)| f.requests.len() as i64)
            })
            .unwrap_or(i64::MAX);
        info.set_estimated_rows(count);
        info.set_estimated_cost(count.max(1) as f64);
        Ok(true)
    }
    fn open(&'v mut self) -> Result<Self::Cursor> {
        Ok(Cursor {
            base: sqlite3_vtab_cursor::default(),
            active: self.active.clone(),
            demands: self.demands.clone(),
            rows: vec![],
            position: 0,
            width: self.width,
            phantom: PhantomData,
        })
    }
}
#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    active: Active,
    demands: Arc<DemandStore>,
    rows: Vec<Row>,
    position: usize,
    width: usize,
    phantom: PhantomData<&'v Table>,
}
// SAFETY: SQLite's cursor base is first and all output is owned by this cursor.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, index: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let (batch, fragment) = self
            .active
            .read()
            .unwrap()
            .clone()
            .ok_or_else(|| invalid("no active batch"))?;
        fragment.metrics.lock().unwrap().filters += 1;
        self.rows = if index == 0 && fragment.scope.is_some() {
            // One SQLite invocation owns the whole batch, but unbound requests need no
            // whole-job materialization. Build and release each file's requests in Rust.
            let mut output = Vec::new();
            let mut offset = 0;
            for file in fragment.scope.as_ref().unwrap() {
                let requests = fragment.requests_for(&batch, std::slice::from_ref(file))?;
                let mut rows = fragment.evaluate(
                    &batch,
                    &requests,
                    &(0..requests.len()).collect::<Vec<_>>(),
                )?;
                for row in &mut rows {
                    if row.request >= 0 {
                        row.request += offset;
                    }
                }
                offset += requests.len() as i64;
                output.extend(rows);
            }
            output
        } else if index == 2 {
            let demand = self.demands.take(args.get::<i64>(0)?)?;
            let mut rows = fragment.evaluate(
                &batch,
                &demand.requests,
                &(0..demand.requests.len()).collect::<Vec<_>>(),
            )?;
            for row in &mut rows {
                if row.request >= 0 {
                    row.request = demand.identities[row.request as usize];
                }
            }
            rows
        } else {
            let scoped;
            let requests = if let Some(scope) = &fragment.scope {
                scoped = fragment.requests_for(&batch, scope)?;
                scoped.as_slice()
            } else {
                &fragment.requests
            };
            let keys = if index == 0 {
                (0..requests.len()).collect()
            } else {
                let mut values = args.in_values(0)?;
                let mut keys = vec![];
                while let Some(v) = values.next()? {
                    keys.push(
                        usize::try_from(v.as_i64()?).map_err(|_| invalid("negative demand key"))?,
                    );
                }
                keys.sort_unstable();
                keys.dedup();
                keys
            };
            fragment.evaluate(&batch, requests, &keys)?
        };
        self.position = 0;
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.position += 1;
        if self.position == self.rows.len() {
            // SQLite can keep an exhausted UNION arm open until the statement finishes.
            self.rows = Vec::new();
        }
        Ok(())
    }
    fn eof(&self) -> bool {
        self.position >= self.rows.len()
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        let row = &self.rows[self.position];
        match col {
            0 => ctx.set_result(&i64::from(row.file)),
            1 => ctx.set_result(&i64::from(row.node)),
            2 => ctx.set_result(&row.truth),
            3 => ctx.set_result(&row.ordinal),
            n if n == 4 + self.width as c_int => ctx.set_result(&row.request),
            n if n == 5 + self.width as c_int => ctx.set_result(&rusqlite::types::Null),
            n => ctx.set_result(&row.values[(n - 4) as usize]),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.position as i64)
    }
}
