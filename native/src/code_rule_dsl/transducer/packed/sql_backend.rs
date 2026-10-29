//! SQLite table-valued boundary for batched terminal execution.
//!
//! SQLite calls `xFilter` once with a batch token. The cursor executes all file/program pairs in
//! that batch once and exposes match, capture, and completion rows. SQL remains the outer rule
//! engine; no scalar function executes one matcher for each joined row.

use super::{Error, ExecutionBackend, Output, Program, execute_with_backend};
use crate::policy_dsl::tape::TapeDoc;
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

const MODULE_NAME: &CStr = c"tempo_matches";
const BATCH_COLUMN: c_int = 0;

#[derive(Clone)]
struct MatchRow {
    batch: i64,
    file: i64,
    program: i64,
    match_index: Option<i64>,
    start_ordinal: Option<i64>,
    output_label: Option<i64>,
    capture_index: Option<i64>,
    capture_label: Option<i64>,
    capture_node: Option<i64>,
    start_byte: Option<i64>,
    end_byte: Option<i64>,
    complete: i64,
    error_code: Option<String>,
}

#[derive(Clone, Copy)]
enum TerminalBackend {
    Cpu,
    #[cfg(target_os = "macos")]
    Metal,
}

struct MatchBatch {
    token: i64,
    backend: TerminalBackend,
    programs: Vec<Program>,
    files: Vec<TapeDoc>,
    anchors: Option<Vec<Vec<Vec<u32>>>>,
    rows: OnceLock<Result<Arc<Vec<MatchRow>>, String>>,
    executions: AtomicUsize,
}

impl MatchBatch {
    fn rows(&self) -> Result<Arc<Vec<MatchRow>>, String> {
        self.rows
            .get_or_init(|| {
                self.executions.fetch_add(1, Ordering::Relaxed);
                let anchors = self.anchors.clone().unwrap_or_else(|| {
                    self.files
                        .iter()
                        .map(|file| {
                            self.programs
                                .iter()
                                .map(|_| (0..file.transducer_len() as u32).collect())
                                .collect()
                        })
                        .collect()
                });
                let backend = match self.backend {
                    TerminalBackend::Cpu => ExecutionBackend::CpuJit,
                    #[cfg(target_os = "macos")]
                    TerminalBackend::Metal => ExecutionBackend::Metal,
                };
                let result = execute_with_backend(backend, &self.programs, &self.files, &anchors);
                match result {
                    Ok(outputs) => Ok(Arc::new(self.output_rows(outputs))),
                    Err(error) => Ok(Arc::new(self.failure_rows(error))),
                }
            })
            .clone()
    }

    fn output_rows(&self, outputs: Vec<Vec<Vec<Output>>>) -> Vec<MatchRow> {
        let mut rows = Vec::new();
        for (file, programs) in outputs.into_iter().enumerate() {
            for (program, matches) in programs.into_iter().enumerate() {
                if matches.is_empty() {
                    rows.push(self.status_row(file, program, true, None));
                    continue;
                }
                for (match_index, output) in matches.into_iter().enumerate() {
                    if output.captures.is_empty() {
                        rows.push(MatchRow {
                            batch: self.token,
                            file: file as i64,
                            program: program as i64,
                            match_index: Some(match_index as i64),
                            start_ordinal: Some(i64::from(output.start)),
                            output_label: Some(i64::from(output.label)),
                            capture_index: None,
                            capture_label: None,
                            capture_node: None,
                            start_byte: None,
                            end_byte: None,
                            complete: 1,
                            error_code: None,
                        });
                        continue;
                    }
                    for (capture_index, capture) in output.captures.into_iter().enumerate() {
                        rows.push(MatchRow {
                            batch: self.token,
                            file: file as i64,
                            program: program as i64,
                            match_index: Some(match_index as i64),
                            start_ordinal: Some(i64::from(output.start)),
                            output_label: Some(i64::from(output.label)),
                            capture_index: Some(capture_index as i64),
                            capture_label: Some(i64::from(capture.label)),
                            capture_node: Some(i64::from(capture.node)),
                            start_byte: Some(capture.span.start as i64),
                            end_byte: Some(capture.span.end as i64),
                            complete: 1,
                            error_code: None,
                        });
                    }
                }
            }
        }
        rows
    }

    fn failure_rows(&self, error: Error) -> Vec<MatchRow> {
        let code = format!("{error:?}");
        (0..self.files.len())
            .flat_map(|file| {
                let code = code.clone();
                (0..self.programs.len())
                    .map(move |program| self.status_row(file, program, false, Some(code.clone())))
            })
            .collect()
    }

    fn status_row(
        &self,
        file: usize,
        program: usize,
        complete: bool,
        error_code: Option<String>,
    ) -> MatchRow {
        MatchRow {
            batch: self.token,
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
            error_code,
        }
    }
}

#[derive(Default)]
pub(super) struct MatchRegistry {
    next: AtomicI64,
    batches: Mutex<HashMap<i64, Arc<MatchBatch>>>,
}

impl MatchRegistry {
    pub(super) fn register_batch(
        &self,
        programs: &[Program],
        files: &[TapeDoc],
        anchors: Option<&[Vec<Vec<u32>>]>,
    ) -> Result<i64, Error> {
        self.register(programs, files, anchors, TerminalBackend::Cpu)
    }

    /// Resolve the universal FTS index in one bulk SQL lookup, widen unknown pairs to a full tape
    /// scan, and register one terminal batch. This is the bridge from persisted SQL selection to
    /// SQL-owned relational composition through `tempo_matches(batch)`.
    pub(super) fn register_indexed_batch(
        &self,
        store: &crate::content_store::ContentStore,
        tape_ids: &[crate::content_store::TapeId],
        programs: &[Program],
        files: &[TapeDoc],
    ) -> crate::content_store::StoreResult<i64> {
        if tape_ids.len() != files.len() {
            return Err(crate::content_store::StoreError::Invalid(
                "tape identity count does not match terminal files".to_owned(),
            ));
        }
        let loaded = super::load_literal_anchor_batch(store, tape_ids, programs)?;
        let anchors = super::widen_anchor_candidates(programs, files, loaded);
        self.register_batch(programs, files, Some(&anchors))
            .map_err(|error| crate::content_store::StoreError::Invalid(format!("{error:?}")))
    }

    #[cfg(target_os = "macos")]
    pub(super) fn register_metal_batch(
        &self,
        programs: &[Program],
        files: &[TapeDoc],
        anchors: &[Vec<Vec<u32>>],
    ) -> Result<i64, Error> {
        self.register(programs, files, Some(anchors), TerminalBackend::Metal)
    }

    fn register(
        &self,
        programs: &[Program],
        files: &[TapeDoc],
        anchors: Option<&[Vec<Vec<u32>>]>,
        backend: TerminalBackend,
    ) -> Result<i64, Error> {
        if anchors.is_some_and(|anchors| {
            anchors.len() != files.len() || anchors.iter().any(|file| file.len() != programs.len())
        }) {
            return Err(Error::InvalidAnchor);
        }
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let batch = Arc::new(MatchBatch {
            token,
            backend,
            programs: programs.to_vec(),
            files: files.to_vec(),
            anchors: anchors.map(<[Vec<Vec<u32>>]>::to_vec),
            rows: OnceLock::new(),
            executions: AtomicUsize::new(0),
        });
        self.batches
            .lock()
            .expect("match registry mutex poisoned")
            .insert(token, batch);
        Ok(token)
    }

    pub(super) fn execution_count(&self, token: i64) -> Option<usize> {
        self.batch(token)
            .map(|batch| batch.executions.load(Ordering::Relaxed))
    }

    pub(super) fn remove_batch(&self, token: i64) {
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

pub(super) fn register_module(
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
        let registry = Arc::clone(aux.ok_or_else(|| {
            rusqlite::Error::ModuleError("tempo_matches registry is missing".to_owned())
        })?);
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(batch HIDDEN,file_index,program_index,match_index,start_ordinal,output_label,capture_index,capture_label,capture_node,start_byte,end_byte,complete,error_code)"),
            Self {
                base: ffi::sqlite3_vtab::default(),
                registry,
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> SqlResult<bool> {
        let mut found = false;
        for (constraint, mut usage) in info.constraints_and_usages() {
            if constraint.column() == BATCH_COLUMN
                && constraint.is_usable()
                && constraint.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                usage.set_argv_index(1);
                usage.set_omit(true);
                found = true;
                break;
            }
        }
        if found {
            info.set_idx_num(1);
            info.set_estimated_cost(1000.0);
            info.set_estimated_rows(1000);
        } else {
            info.set_idx_num(0);
            info.set_estimated_cost(f64::MAX);
            info.set_estimated_rows(i64::MAX);
        }
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
        if idx_num != 1 {
            return Err(rusqlite::Error::ModuleError(
                "tempo_matches requires one batch equality constraint".to_owned(),
            ));
        }
        let token = args.get::<i64>(0)?;
        let batch = self.registry.batch(token).ok_or_else(|| {
            rusqlite::Error::ModuleError("tempo_matches batch is missing".to_owned())
        })?;
        self.rows = batch.rows().map_err(rusqlite::Error::ModuleError)?;
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
            5 => context.set_result(&row.output_label),
            6 => context.set_result(&row.capture_index),
            7 => context.set_result(&row.capture_label),
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
