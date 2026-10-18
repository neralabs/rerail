//! SQL-demanded packed batch relation.
#![allow(
    dead_code,
    reason = "the packed lowering layer calls this boundary; the deleted matcher callback must not keep it artificially live"
)]
//!
//! Registration stores immutable verified programs, tapes, limits, and a sparse task grid. SQLite
//! calls `xFilter` with the batch token. The first call executes the packed backend and stores its
//! normalized relation. No host callback or semantic matcher can be registered here.

use super::super::packed::{
    Error as PackedError, ExecutionBackend, Program, execute_with_backend_packed_result_limits,
};
use super::super::tape::TapeDoc;
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

/// Everything `xFilter` needs to execute one sparse packed batch. Construction verifies programs
/// before registration; this value contains no callback and cannot enter the semantic matcher.
pub(in crate::code_rule_dsl) struct PackedDemand {
    backend: ExecutionBackend,
    programs: Vec<Program>,
    files: Vec<TapeDoc>,
    anchors: Vec<Vec<Vec<u32>>>,
    result_limits: Vec<u32>,
}

impl PackedDemand {
    pub(in crate::code_rule_dsl) fn new(
        backend: ExecutionBackend,
        programs: Vec<Program>,
        files: Vec<TapeDoc>,
        anchors: Vec<Vec<Vec<u32>>>,
        result_limits: Vec<u32>,
    ) -> Result<Self, PackedError> {
        if anchors.len() != files.len()
            || anchors.iter().any(|file| file.len() != programs.len())
            || result_limits.len() != programs.len()
        {
            return Err(PackedError::InvalidAnchor);
        }
        Ok(Self {
            backend,
            programs,
            files,
            anchors,
            result_limits,
        })
    }

    fn execute(self, token: i64) -> Result<Vec<MatchRow>, PackedError> {
        let execution = execute_with_backend_packed_result_limits(
            self.backend,
            &self.programs,
            &self.files,
            &self.anchors,
            Some(&self.result_limits),
        )?;
        let mut rows = Vec::new();
        for file in 0..execution.file_count() {
            for program in 0..execution.program_count() {
                let (results, error) = execution.program_results(file, program)?;
                if let Some(error) = error {
                    rows.push(MatchRow::status(token, file, program, false, Some(error)));
                    continue;
                }
                if results.is_empty() {
                    rows.push(MatchRow::status(token, file, program, true, None));
                    continue;
                }
                for (match_index, result) in results.into_iter().enumerate() {
                    let captures = result.captures().collect::<Vec<_>>();
                    if captures.is_empty() {
                        rows.push(MatchRow {
                            batch: token,
                            file: file as i64,
                            program: program as i64,
                            match_index: Some(match_index as i64),
                            start_ordinal: Some(result.task_ordinal() as i64),
                            output_label: self.programs[program]
                                .label_name(result.label())
                                .map(str::to_owned),
                            capture_index: None,
                            capture_label: None,
                            capture_node: None,
                            start_byte: None,
                            end_byte: None,
                            complete: 1,
                            error_code: None,
                        });
                    } else {
                        for (capture_index, capture) in captures.into_iter().enumerate() {
                            let span = capture.span();
                            rows.push(MatchRow {
                                batch: token,
                                file: file as i64,
                                program: program as i64,
                                match_index: Some(match_index as i64),
                                start_ordinal: Some(result.task_ordinal() as i64),
                                output_label: self.programs[program]
                                    .label_name(result.label())
                                    .map(str::to_owned),
                                capture_index: Some(capture_index as i64),
                                capture_label: self.programs[program]
                                    .label_name(capture.label())
                                    .map(str::to_owned),
                                capture_node: Some(capture.node() as i64),
                                start_byte: Some(span.start as i64),
                                end_byte: Some(span.end as i64),
                                complete: 1,
                                error_code: None,
                            });
                        }
                    }
                }
            }
        }
        Ok(rows)
    }
}

#[derive(Clone)]
struct MatchRow {
    batch: i64,
    file: i64,
    program: i64,
    match_index: Option<i64>,
    start_ordinal: Option<i64>,
    output_label: Option<String>,
    capture_index: Option<i64>,
    capture_label: Option<String>,
    capture_node: Option<i64>,
    start_byte: Option<i64>,
    end_byte: Option<i64>,
    complete: i64,
    error_code: Option<String>,
}

impl MatchRow {
    fn status(
        batch: i64,
        file: usize,
        program: usize,
        complete: bool,
        error: Option<PackedError>,
    ) -> Self {
        Self {
            batch,
            file: file as i64,
            program: program as i64,
            match_index: None,
            start_ordinal: None,
            output_label: None,
            capture_index: None,
            capture_label: None,
            capture_node: None,
            start_byte: None,
            end_byte: None,
            complete: i64::from(complete),
            error_code: error.map(|error| format!("{error:?}")),
        }
    }
}

struct MatchBatch {
    token: i64,
    demand: Mutex<Option<PackedDemand>>,
    rows: OnceLock<Result<Arc<Vec<MatchRow>>, String>>,
    executions: AtomicUsize,
    filters: AtomicUsize,
}

impl MatchBatch {
    fn rows(&self) -> SqlResult<Arc<Vec<MatchRow>>> {
        self.rows
            .get_or_init(|| {
                self.executions.fetch_add(1, Ordering::Relaxed);
                let demand = self
                    .demand
                    .lock()
                    .expect("packed demand mutex poisoned")
                    .take()
                    .expect("a demanded packed batch executes once");
                demand
                    .execute(self.token)
                    .map(Arc::new)
                    .map_err(|error| format!("packed batch failed: {error:?}"))
            })
            .as_ref()
            .map(Arc::clone)
            .map_err(|error| rusqlite::Error::ModuleError(error.clone()))
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
            1 => context.set_result(&row.file),
            2 => context.set_result(&row.program),
            3 => context.set_result(&row.match_index),
            4 => context.set_result(&row.start_ordinal),
            5 => context.set_result(&row.output_label.as_deref()),
            6 => context.set_result(&row.capture_index),
            7 => context.set_result(&row.capture_label.as_deref()),
            8 => context.set_result(&row.capture_node),
            9 => context.set_result(&row.start_byte),
            10 => context.set_result(&row.end_byte),
            11 => context.set_result(&row.complete),
            12 => context.set_result(&row.error_code.as_deref()),
            _ => Err(rusqlite::Error::InvalidColumnIndex(column as usize)),
        }
    }

    fn rowid(&self) -> SqlResult<i64> {
        Ok(self.position as i64 + 1)
    }
}
