//! Experimental batched AstMatcher binding. SQL sees completed typed outcomes, never instructions.
use super::{
    capture_cell::{Cell, encode_list},
    capture_env, span_capture,
};
use crate::code_rule_dsl::{
    matcher::{self, GrammarMatcher, Stopped},
    sql::{exec::Batch, plan::SlotKind, terminals::with_scopes, timing::Started},
    tape::TapeDoc,
};
use ast_grep_core::{meta_var::MetaVarEnv, source::Content};
use rusqlite::{
    Connection, Error, Result,
    fallible_iterator::FallibleIterator,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor, sqlite3_vtab, sqlite3_vtab_cursor,
    },
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    ffi::{CStr, CString, c_int},
    marker::PhantomData,
    sync::{Arc, Mutex},
};

#[path = "fragment_tests.rs"]
mod tests;

#[derive(Default, Clone, serde::Serialize)]
pub(super) struct Metrics {
    pub filters: usize,
    pub roots: usize,
    pub rows: usize,
    pub completed_roots: usize,
    pub native_cpu_ns: u64,
    pub native_wall_ns: u64,
    pub remaining_work: BTreeMap<u32, usize>,
}

/// Capture inputs are immutable batch data. The key includes request identity, not just the root.
pub(super) struct Request {
    pub file: u32,
    pub node: u32,
    pub inputs: Vec<(String, SlotKind, Cell)>,
}

pub(super) struct Fragment {
    pub grammar: GrammarMatcher,
    pub names: Vec<String>,
    pub requests: Vec<Request>,
    pub work: usize,
    pub each: Option<String>,
    pub metrics: Arc<Mutex<Metrics>>,
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
        }
    }

    fn evaluate(&self, batch: &Batch, keys: &[usize]) -> Result<Vec<Row>> {
        let started = Started::now();
        let mut groups: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for &key in keys {
            let request = self
                .requests
                .get(key)
                .ok_or_else(|| invalid("invalid request key"))?;
            groups.entry(request.file).or_default().push(key);
        }
        let mut output = vec![];
        let mut metrics = self.metrics.lock().unwrap();
        metrics.filters += 1;
        for (file, keys) in groups {
            let entry = batch
                .files
                .get(file as usize)
                .ok_or_else(|| invalid("invalid document"))?;
            // Request order is canonical root order; do not deduplicate distinct capture inputs.
            let mut keys = keys;
            keys.sort_by_key(|&key| (self.requests[key].node, key));
            let mut provisional = vec![];
            let mut stopped = None;
            let mut left = self.work;
            with_scopes(file as usize, entry, || {
                matcher::with_file_memo(&entry.root, || -> Result<()> {
                    matcher::begin_clause(&self.grammar, left);
                    // Always close clause state, including a transport error.
                    let result = (|| -> Result<()> {
                        for key in keys {
                            let r = &self.requests[key];
                            if r.node >= entry.facts.len() {
                                return Err(invalid("root outside document"));
                            }
                            if self.each.is_some() && !r.inputs.is_empty() {
                                return Err(invalid(
                                    "enumeration with bound inputs needs an exclusion contract",
                                ));
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
                            let env = capture_env::environment(
                                file,
                                &entry.facts,
                                |id| matcher::node_at(&entry.root, id),
                                &bindings,
                                &values,
                            )?;
                            let mut env = Cow::Owned(env);
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
                                    let result = matcher::enumerate(
                                        &entry.root,
                                        &self.grammar,
                                        r.node,
                                        env.into_owned(),
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
            })?;
            metrics.remaining_work.insert(file, left);
            match stopped {
                None => output.extend(provisional),
                Some(Stopped::Limit(limit)) => output.push(Row {
                    request: -1,
                    file,
                    node: 0,
                    ordinal: 0,
                    truth: super::failure::encode(&format!("{limit:?}"))
                        .ok_or_else(|| invalid("unknown native failure"))?,
                    values: vec![Cell::Null; self.names.len()],
                }),
                Some(Stopped::Deadline) => return Err(invalid("native deadline")),
            }
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

struct Row {
    request: i64,
    file: u32,
    node: u32,
    truth: i64,
    ordinal: i64,
    values: Vec<Cell>,
}
fn invalid(message: &str) -> Error {
    Error::ModuleError(format!("matcher fragment: {message}"))
}

pub(super) fn register(
    db: &Connection,
    name: &CStr,
    batch: Arc<Batch>,
    fragment: Arc<Fragment>,
) -> Result<()> {
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(name, &MODULE, Some((batch, fragment)))
}

#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    batch: Arc<Batch>,
    fragment: Arc<Fragment>,
}
// SAFETY: SQLite's base is first; cursors retain the owned batch and compiled matcher.
unsafe impl<'v> VTab<'v> for Table {
    type Aux = (Arc<Batch>, Arc<Fragment>);
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
        let (batch, fragment) = aux.ok_or_else(|| invalid("missing batch"))?;
        let columns = (0..fragment.names.len())
            .map(|i| format!(",c{i}"))
            .collect::<String>();
        let schema = CString::new(format!("CREATE TABLE x(file INTEGER,node INTEGER,truth INTEGER,ordinal INTEGER{columns},request INTEGER HIDDEN)"))
            .map_err(|_| invalid("invalid schema"))?;
        Ok((
            Cow::Owned(schema),
            Self {
                base: sqlite3_vtab::default(),
                batch: batch.clone(),
                fragment: fragment.clone(),
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let request = 4 + self.fragment.names.len() as c_int;
        let index = info
            .constraints()
            .enumerate()
            .find(|(_, c)| {
                c.is_usable()
                    && c.column() == request
                    && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            })
            .map(|(i, _)| i);
        if let Some(index) = index {
            // Scalar correlated invocation is forbidden at this boundary.
            if !info.is_in_constraint(index)? || !info.set_in_constraint(index, true)? {
                return Ok(false);
            }
            info.constraint_usage(index).set_argv_index(1);
            info.constraint_usage(index).set_omit(true);
            info.set_idx_num(1);
        } else {
            info.set_idx_num(0);
        }
        info.set_estimated_rows(self.fragment.requests.len() as i64);
        info.set_estimated_cost(self.fragment.requests.len().max(1) as f64);
        Ok(true)
    }
    fn open(&'v mut self) -> Result<Self::Cursor> {
        Ok(Cursor {
            base: sqlite3_vtab_cursor::default(),
            batch: self.batch.clone(),
            fragment: self.fragment.clone(),
            rows: vec![],
            position: 0,
            phantom: PhantomData,
        })
    }
}
#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    batch: Arc<Batch>,
    fragment: Arc<Fragment>,
    rows: Vec<Row>,
    position: usize,
    phantom: PhantomData<&'v Table>,
}
// SAFETY: SQLite's cursor base is first and all output is owned by this cursor.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, index: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let mut keys = if index == 0 {
            (0..self.fragment.requests.len()).collect()
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
        // Keep explicit empty demand empty; it must not become a full scan.
        self.rows = self.fragment.evaluate(&self.batch, &keys)?;
        keys.clear();
        self.position = 0;
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.position += 1;
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
            n if n == 4 + self.fragment.names.len() as c_int => ctx.set_result(&row.request),
            n => ctx.set_result(&row.values[(n - 4) as usize]),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.position as i64)
    }
}
