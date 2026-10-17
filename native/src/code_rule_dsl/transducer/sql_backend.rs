//! SQL-demanded batch relation.
//!
//! Registration stores one inert executor. SQLite calls `xFilter` with its token. The first call
//! executes the complete batch and stores normalized rows. Later scans read the same rows.

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
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const MODULE_NAME: &CStr = c"rerail_matches";
const BATCH_COLUMN: c_int = 0;
const USE_BATCH: c_int = 1;

pub(in crate::code_rule_dsl) struct DrivenRow {
    pub(in crate::code_rule_dsl) file: usize,
    pub(in crate::code_rule_dsl) program: usize,
    pub(in crate::code_rule_dsl) match_index: Option<usize>,
    pub(in crate::code_rule_dsl) complete: bool,
    pub(in crate::code_rule_dsl) error: Option<String>,
}

#[derive(Clone)]
struct MatchRow {
    batch: i64,
    file: i64,
    program: i64,
    match_index: Option<i64>,
    complete: i64,
    error_code: Option<String>,
}

type DemandExecutor = Box<dyn FnOnce() -> Vec<DrivenRow> + Send>;

struct MatchBatch {
    token: i64,
    demand: Mutex<Option<DemandExecutor>>,
    rows: OnceLock<Arc<Vec<MatchRow>>>,
    executions: AtomicUsize,
    filters: AtomicUsize,
}

impl MatchBatch {
    fn rows(&self) -> Arc<Vec<MatchRow>> {
        Arc::clone(self.rows.get_or_init(|| {
            self.executions.fetch_add(1, Ordering::Relaxed);
            let execute = self
                .demand
                .lock()
                .expect("demand executor mutex poisoned")
                .take()
                .expect("a demanded batch executes once");
            Arc::new(
                execute()
                    .into_iter()
                    .map(|row| MatchRow {
                        batch: self.token,
                        file: row.file as i64,
                        program: row.program as i64,
                        match_index: row.match_index.map(|index| index as i64),
                        complete: i64::from(row.complete),
                        error_code: row.error,
                    })
                    .collect(),
            )
        }))
    }
}

#[derive(Default)]
pub(in crate::code_rule_dsl) struct MatchRegistry {
    next: AtomicI64,
    batches: Mutex<HashMap<i64, Arc<MatchBatch>>>,
}

impl MatchRegistry {
    pub(in crate::code_rule_dsl) fn register_demand(&self, execute: DemandExecutor) -> i64 {
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.batches
            .lock()
            .expect("match registry mutex poisoned")
            .insert(
                token,
                Arc::new(MatchBatch {
                    token,
                    demand: Mutex::new(Some(execute)),
                    rows: OnceLock::new(),
                    executions: AtomicUsize::new(0),
                    filters: AtomicUsize::new(0),
                }),
            );
        token
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
            Cow::Borrowed(c"CREATE TABLE x(batch HIDDEN,file_index,program_index,match_index,start_ordinal,output_label,capture_index,capture_label,capture_node,start_byte,end_byte,complete,error_code)"),
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
    rows: Arc<Vec<MatchRow>>,
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
        self.rows = batch.rows();
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
            1 => context.set_result(&row.file),
            2 => context.set_result(&row.program),
            3 => context.set_result(&row.match_index),
            4..=10 => context.set_result(&Option::<i64>::None),
            11 => context.set_result(&row.complete),
            12 => context.set_result(&row.error_code.as_deref()),
            _ => Err(rusqlite::Error::InvalidColumnIndex(column as usize)),
        }
    }

    fn rowid(&self) -> SqlResult<i64> {
        Ok(self.position as i64 + 1)
    }
}
