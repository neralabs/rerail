//! SQL-demanded packed batch relation.
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
    task_grid: Vec<Vec<bool>>,
    result_limits: Vec<u32>,
}

impl PackedDemand {
    pub(in crate::code_rule_dsl) fn new(
        backend: ExecutionBackend,
        programs: Vec<Program>,
        files: Vec<TapeDoc>,
        anchors: Vec<Vec<Vec<u32>>>,
        task_grid: Vec<Vec<bool>>,
        result_limits: Vec<u32>,
    ) -> Result<Self, PackedError> {
        if anchors.len() != files.len()
            || task_grid.len() != files.len()
            || anchors.iter().any(|file| file.len() != programs.len())
            || task_grid.iter().any(|file| file.len() != programs.len())
            || result_limits.len() != programs.len()
        {
            return Err(PackedError::InvalidAnchor);
        }
        Ok(Self {
            backend,
            programs,
            files,
            anchors,
            task_grid,
            result_limits,
        })
    }

    fn execute(self, token: i64) -> Vec<PackedRow> {
        let execution = match execute_with_backend_packed_result_limits(
            self.backend,
            &self.programs,
            &self.files,
            &self.anchors,
            Some(&self.result_limits),
        ) {
            Ok(execution) => execution,
            Err(error) => {
                let message = format!("{error:?}");
                let mut rows = vec![PackedRow::batch_status(token, false, Some(message.clone()))];
                for (file, tasks) in self.task_grid.iter().enumerate() {
                    for (producer, demanded) in tasks.iter().copied().enumerate() {
                        if demanded {
                            rows.push(PackedRow::producer_status(
                                token,
                                file,
                                producer,
                                false,
                                None,
                                Some(message.clone()),
                            ));
                        }
                    }
                }
                return rows;
            }
        };
        let mut rows = Vec::new();
        let mut batch_error = None;
        for file in 0..execution.file_count() {
            for program in 0..execution.program_count() {
                if !self.task_grid[file][program] {
                    continue;
                }
                let (results, error) = match execution.program_results(file, program) {
                    Ok(result) => result,
                    Err(error) => {
                        let message = format!("{error:?}");
                        batch_error.get_or_insert_with(|| message.clone());
                        rows.push(PackedRow::producer_status(
                            token,
                            file,
                            program,
                            false,
                            None,
                            Some(message),
                        ));
                        continue;
                    }
                };
                if let Some(error) = error {
                    let message = format!("{error:?}");
                    batch_error.get_or_insert_with(|| message.clone());
                    rows.push(PackedRow::producer_status(
                        token,
                        file,
                        program,
                        false,
                        None,
                        Some(message),
                    ));
                    continue;
                }
                rows.push(PackedRow::producer_status(
                    token,
                    file,
                    program,
                    true,
                    Some(!results.is_empty()),
                    None,
                ));
                for (match_index, result) in results.into_iter().enumerate() {
                    rows.push(PackedRow::terminal_match(TerminalMatchData {
                        batch: token,
                        file,
                        producer: program,
                        match_index,
                        task_index: result.task_ordinal(),
                        start_ordinal: result.local_ordinal(),
                        node: result.start_node(),
                        output_label: self.programs[program]
                            .label_name(result.label())
                            .map(str::to_owned),
                    }));
                    let captures = result.captures().collect::<Vec<_>>();
                    for (capture_index, capture) in captures.iter().copied().enumerate() {
                        let label = capture.label();
                        let occurrence = captures[..capture_index]
                            .iter()
                            .filter(|candidate| candidate.label() == label)
                            .count();
                        let sequence_start = captures
                            .iter()
                            .position(|candidate| candidate.label() == label)
                            .expect("the current capture has a sequence");
                        let sequence_count = captures
                            .iter()
                            .filter(|candidate| candidate.label() == label)
                            .count();
                        let span = capture.span();
                        rows.push(PackedRow::capture(CaptureData {
                            batch: token,
                            file,
                            producer: program,
                            match_index,
                            capture_index,
                            occurrence,
                            sequence_start,
                            sequence_count,
                            node: capture.node(),
                            capture_label: self.programs[program]
                                .label_name(label)
                                .map(str::to_owned),
                            text_value: capture
                                .transformed_text()
                                .map(|text| String::from_utf8_lossy(text).into_owned()),
                            start_byte: span.start,
                            end_byte: span.end,
                        }));
                    }
                }
            }
        }
        rows.insert(
            0,
            PackedRow::batch_status(token, batch_error.is_none(), batch_error),
        );
        rows
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i64)]
pub(in crate::code_rule_dsl) enum RowKind {
    BatchStatus = 1,
    ProducerStatus = 2,
    TerminalMatch = 3,
    Capture = 4,
    Navigation = 5,
    Fact = 6,
    DocumentMap = 7,
}

const ALL_ROW_KINDS: [RowKind; 7] = [
    RowKind::BatchStatus,
    RowKind::ProducerStatus,
    RowKind::TerminalMatch,
    RowKind::Capture,
    RowKind::Navigation,
    RowKind::Fact,
    RowKind::DocumentMap,
];

struct TerminalMatchData {
    batch: i64,
    file: usize,
    producer: usize,
    match_index: usize,
    task_index: u32,
    start_ordinal: u32,
    node: u32,
    output_label: Option<String>,
}

struct CaptureData {
    batch: i64,
    file: usize,
    producer: usize,
    match_index: usize,
    capture_index: usize,
    occurrence: usize,
    sequence_start: usize,
    sequence_count: usize,
    node: u32,
    capture_label: Option<String>,
    text_value: Option<String>,
    start_byte: usize,
    end_byte: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct PackedRow {
    batch: i64,
    row_kind: RowKind,
    producer_id: Option<i64>,
    file_index: Option<i64>,
    task_index: Option<i64>,
    match_index: Option<i64>,
    capture_index: Option<i64>,
    capture_occurrence: Option<i64>,
    capture_sequence_start: Option<i64>,
    capture_sequence_count: Option<i64>,
    node: Option<i64>,
    related_node: Option<i64>,
    axis_kind: Option<i64>,
    axis_order: Option<i64>,
    fact_kind: Option<i64>,
    document_id: Option<i64>,
    mapped_document_id: Option<i64>,
    start_ordinal: Option<i64>,
    output_label: Option<String>,
    capture_label: Option<String>,
    text_value: Option<String>,
    start_byte: Option<i64>,
    end_byte: Option<i64>,
    verdict: Option<i64>,
    complete: i64,
    work_used: Option<i64>,
    error_code: Option<String>,
}

impl PackedRow {
    fn blank(batch: i64, row_kind: RowKind) -> Self {
        debug_assert!(ALL_ROW_KINDS.contains(&row_kind));
        Self {
            batch,
            row_kind,
            producer_id: None,
            file_index: None,
            task_index: None,
            match_index: None,
            capture_index: None,
            capture_occurrence: None,
            capture_sequence_start: None,
            capture_sequence_count: None,
            node: None,
            related_node: None,
            axis_kind: None,
            axis_order: None,
            fact_kind: None,
            document_id: None,
            mapped_document_id: None,
            start_ordinal: None,
            output_label: None,
            capture_label: None,
            text_value: None,
            start_byte: None,
            end_byte: None,
            verdict: None,
            complete: 1,
            work_used: None,
            error_code: None,
        }
    }

    fn batch_status(batch: i64, complete: bool, error: Option<String>) -> Self {
        let mut row = Self::blank(batch, RowKind::BatchStatus);
        row.complete = i64::from(complete);
        row.error_code = error;
        row
    }

    fn producer_status(
        batch: i64,
        file: usize,
        producer: usize,
        complete: bool,
        verdict: Option<bool>,
        error: Option<String>,
    ) -> Self {
        let mut row = Self::blank(batch, RowKind::ProducerStatus);
        row.file_index = Some(file as i64);
        row.producer_id = Some(producer as i64);
        row.complete = i64::from(complete);
        row.verdict = verdict.map(i64::from);
        row.error_code = error;
        row
    }

    fn terminal_match(data: TerminalMatchData) -> Self {
        let mut row = Self::blank(data.batch, RowKind::TerminalMatch);
        row.file_index = Some(data.file as i64);
        row.producer_id = Some(data.producer as i64);
        row.task_index = Some(data.task_index as i64);
        row.match_index = Some(data.match_index as i64);
        row.start_ordinal = Some(data.start_ordinal as i64);
        row.node = Some(data.node as i64);
        row.output_label = data.output_label;
        row
    }

    fn capture(data: CaptureData) -> Self {
        let mut row = Self::blank(data.batch, RowKind::Capture);
        row.file_index = Some(data.file as i64);
        row.producer_id = Some(data.producer as i64);
        row.match_index = Some(data.match_index as i64);
        row.capture_index = Some(data.capture_index as i64);
        row.capture_occurrence = Some(data.occurrence as i64);
        row.capture_sequence_start = Some(data.sequence_start as i64);
        row.capture_sequence_count = Some(data.sequence_count as i64);
        row.node = Some(data.node as i64);
        row.capture_label = data.capture_label;
        row.text_value = data.text_value;
        row.start_byte = Some(data.start_byte as i64);
        row.end_byte = Some(data.end_byte as i64);
        row
    }

    #[cfg(test)]
    pub(in crate::code_rule_dsl) const fn row_kind(&self) -> RowKind {
        self.row_kind
    }

    pub(in crate::code_rule_dsl) fn is_complete(&self) -> bool {
        self.complete != 0
    }

    pub(in crate::code_rule_dsl) fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }

    pub(in crate::code_rule_dsl) fn from_sql_row(row: &rusqlite::Row<'_>) -> SqlResult<Self> {
        let row_kind = match row.get::<_, i64>(1)? {
            1 => RowKind::BatchStatus,
            2 => RowKind::ProducerStatus,
            3 => RowKind::TerminalMatch,
            4 => RowKind::Capture,
            5 => RowKind::Navigation,
            6 => RowKind::Fact,
            7 => RowKind::DocumentMap,
            value => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Integer,
                    format!("unknown rerail_matches row kind {value}").into(),
                ));
            }
        };
        Ok(Self {
            batch: row.get(0)?,
            row_kind,
            producer_id: row.get(2)?,
            file_index: row.get(3)?,
            task_index: row.get(4)?,
            match_index: row.get(5)?,
            capture_index: row.get(6)?,
            capture_occurrence: row.get(7)?,
            capture_sequence_start: row.get(8)?,
            capture_sequence_count: row.get(9)?,
            node: row.get(10)?,
            related_node: row.get(11)?,
            axis_kind: row.get(12)?,
            axis_order: row.get(13)?,
            fact_kind: row.get(14)?,
            document_id: row.get(15)?,
            mapped_document_id: row.get(16)?,
            start_ordinal: row.get(17)?,
            output_label: row.get(18)?,
            capture_label: row.get(19)?,
            text_value: row.get(20)?,
            start_byte: row.get(21)?,
            end_byte: row.get(22)?,
            verdict: row.get(23)?,
            complete: row.get(24)?,
            work_used: row.get(25)?,
            error_code: row.get(26)?,
        })
    }
}

struct MatchBatch {
    token: i64,
    demand: Mutex<Option<PackedDemand>>,
    rows: OnceLock<Arc<Vec<PackedRow>>>,
    executions: AtomicUsize,
    filters: AtomicUsize,
}

impl MatchBatch {
    fn rows(&self) -> SqlResult<Arc<Vec<PackedRow>>> {
        Ok(Arc::clone(self.rows.get_or_init(|| {
            self.executions.fetch_add(1, Ordering::Relaxed);
            let demand = self
                .demand
                .lock()
                .expect("packed demand mutex poisoned")
                .take()
                .expect("a demanded packed batch executes once");
            Arc::new(demand.execute(self.token))
        })))
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
            Cow::Borrowed(c"CREATE TABLE x(batch INTEGER HIDDEN,row_kind INTEGER NOT NULL,producer_id INTEGER,file_index INTEGER,task_index INTEGER,match_index INTEGER,capture_index INTEGER,capture_occurrence INTEGER,capture_sequence_start INTEGER,capture_sequence_count INTEGER,node INTEGER,related_node INTEGER,axis_kind INTEGER,axis_order INTEGER,fact_kind INTEGER,document_id INTEGER,mapped_document_id INTEGER,start_ordinal INTEGER,output_label TEXT,capture_label TEXT,text_value TEXT,start_byte INTEGER,end_byte INTEGER,verdict INTEGER,complete INTEGER NOT NULL,work_used INTEGER,error_code TEXT)"),
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
    rows: Arc<Vec<PackedRow>>,
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
            1 => context.set_result(&(row.row_kind as i64)),
            2 => context.set_result(&row.producer_id),
            3 => context.set_result(&row.file_index),
            4 => context.set_result(&row.task_index),
            5 => context.set_result(&row.match_index),
            6 => context.set_result(&row.capture_index),
            7 => context.set_result(&row.capture_occurrence),
            8 => context.set_result(&row.capture_sequence_start),
            9 => context.set_result(&row.capture_sequence_count),
            10 => context.set_result(&row.node),
            11 => context.set_result(&row.related_node),
            12 => context.set_result(&row.axis_kind),
            13 => context.set_result(&row.axis_order),
            14 => context.set_result(&row.fact_kind),
            15 => context.set_result(&row.document_id),
            16 => context.set_result(&row.mapped_document_id),
            17 => context.set_result(&row.start_ordinal),
            18 => context.set_result(&row.output_label.as_deref()),
            19 => context.set_result(&row.capture_label.as_deref()),
            20 => context.set_result(&row.text_value.as_deref()),
            21 => context.set_result(&row.start_byte),
            22 => context.set_result(&row.end_byte),
            23 => context.set_result(&row.verdict),
            24 => context.set_result(&row.complete),
            25 => context.set_result(&row.work_used),
            26 => context.set_result(&row.error_code.as_deref()),
            _ => Err(rusqlite::Error::InvalidColumnIndex(column as usize)),
        }
    }

    fn rowid(&self) -> SqlResult<i64> {
        Ok(self.position as i64 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::{MatchRegistry, PackedDemand, register_module};
    use crate::code_rule_dsl::packed::ExecutionBackend;
    use rusqlite::Connection;
    use std::sync::Arc;

    fn empty_demand() -> PackedDemand {
        PackedDemand::new(
            ExecutionBackend::CpuJit,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("valid empty demand")
    }

    #[test]
    fn expanded_relation_has_one_typed_schema() {
        let connection = Connection::open_in_memory().expect("SQLite connection");
        register_module(&connection, Arc::new(MatchRegistry::default()))
            .expect("virtual table module");
        let columns = connection
            .prepare("SELECT name FROM pragma_table_xinfo('rerail_matches') ORDER BY cid")
            .expect("schema query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("schema rows")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("schema values");
        assert_eq!(
            columns,
            [
                "batch",
                "row_kind",
                "producer_id",
                "file_index",
                "task_index",
                "match_index",
                "capture_index",
                "capture_occurrence",
                "capture_sequence_start",
                "capture_sequence_count",
                "node",
                "related_node",
                "axis_kind",
                "axis_order",
                "fact_kind",
                "document_id",
                "mapped_document_id",
                "start_ordinal",
                "output_label",
                "capture_label",
                "text_value",
                "start_byte",
                "end_byte",
                "verdict",
                "complete",
                "work_used",
                "error_code",
            ]
        );
    }

    #[test]
    fn empty_batch_is_inert_until_sql_demands_one_explicit_batch_status() {
        let connection = Connection::open_in_memory().expect("SQLite connection");
        let registry = Arc::new(MatchRegistry::default());
        register_module(&connection, Arc::clone(&registry)).expect("virtual table module");
        let token = registry.register(empty_demand());
        assert_eq!(registry.query_execution_counters(token), Some((0, 0)));

        let rows = connection
            .prepare(
                "SELECT row_kind, producer_id, file_index, complete, error_code \
                 FROM rerail_matches WHERE batch = ?1 ORDER BY rowid",
            )
            .expect("expanded relation query")
            .query_map([token], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .expect("batch rows")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("batch relation");

        assert_eq!(rows, vec![(1, None, None, 1, None)]);
        let repeated = connection
            .query_row(
                "SELECT count(*) FROM rerail_matches WHERE batch = ?1",
                [token],
                |row| row.get::<_, i64>(0),
            )
            .expect("second cursor reads the materialized relation");
        assert_eq!(repeated, 1);
        assert_eq!(registry.query_execution_counters(token), Some((1, 2)));
    }
}
