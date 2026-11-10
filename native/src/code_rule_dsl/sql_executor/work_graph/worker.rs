//! Each worker owns SQLite and binds immutable native roots without schema SQL.
use super::super::super::facts::vtab::Modules;
use super::*;
use rusqlite::hooks::{AuthAction, Authorization};

#[derive(Debug)]
pub(in crate::code_rule_dsl::sql_executor) struct Rows {
    pub width: usize,
    pub rows: Vec<Vec<Option<i64>>>,
    indexes: Vec<std::sync::OnceLock<(Vec<usize>, usize)>>,
}
impl Rows {
    pub fn new(width: usize, rows: Vec<Vec<Option<i64>>>) -> Self {
        Self {
            width,
            rows,
            indexes: (0..width).map(|_| std::sync::OnceLock::new()).collect(),
        }
    }
    fn index(&self, c: usize) -> &(Vec<usize>, usize) {
        self.indexes[c].get_or_init(|| {
            let _profile = super::super::super::sql_profile::index_build(self.rows.len());
            let mut ids = (0..self.rows.len())
                .filter(|&r| self.rows[r][c].is_some())
                .collect::<Vec<_>>();
            ids.sort_unstable_by_key(|&r| self.rows[r][c]);
            let distinct = usize::from(!ids.is_empty())
                + ids
                    .windows(2)
                    .filter(|pair| self.rows[pair[0]][c] != self.rows[pair[1]][c])
                    .count();
            (ids, distinct)
        })
    }
}
impl Source for Rows {
    fn columns(&self) -> Vec<String> {
        (0..self.width).map(|c| format!("c{c}")).collect()
    }
    fn len(&self) -> usize {
        self.rows.len()
    }
    fn indexed(&self, _: usize) -> bool {
        true
    }
    fn estimated_matches(&self, c: usize, batch: bool) -> usize {
        let (ids, distinct) = self.index(c);
        ids.len()
            .div_ceil((*distinct).max(1))
            .saturating_mul(if batch { 8 } else { 1 })
    }
    fn lookup(&self, c: usize, values: &[i64]) -> Option<Vec<usize>> {
        let (ids, _) = self.index(c);
        Some(
            values
                .iter()
                .flat_map(|&v| {
                    let lo = ids.partition_point(|&r| self.rows[r][c] < Some(v));
                    let hi = ids.partition_point(|&r| self.rows[r][c] <= Some(v));
                    ids[lo..hi].iter().copied()
                })
                .collect(),
        )
    }
    fn value(&self, r: usize, c: usize) -> i64 {
        self.rows[r][c].unwrap_or(0)
    }
    fn cell(&self, r: usize, c: usize) -> SqlValue {
        self.rows[r][c].map_or(SqlValue::Null, SqlValue::Integer)
    }
    fn revision(&self) -> Option<u64> {
        Some(0)
    }
}
pub(super) struct Results(pub BTreeMap<String, Arc<Rows>>);

pub(super) struct Job {
    pub id: usize,
    pub graph: u64,
    pub sql: String,
    pub outputs: Vec<(String, usize)>,
    pub bindings: BTreeMap<String, Arc<dyn Source>>,
    pub shared: Arc<Shared>,
    pub explain: Value,
    pub units: f64,
    pub scheduler: Arc<Scheduler>,
}
// Field order is intentional: the callback must drop before its connection.
struct Database {
    profiler: Option<Box<super::super::super::sql_profile::Profiler>>,
    _admission: Option<Box<super::super::super::sql_admission::observer::Observer>>,
    connection: rusqlite::Connection,
    _modules: Modules,
    accounting: Option<Arc<Accounting>>,
}
impl Drop for Database {
    fn drop(&mut self) {
        if let (Some(profiler), Some(accounting)) = (&self.profiler, &self.accounting) {
            for (counter, total) in accounting.0.iter().zip(profiler.totals()) {
                counter.fetch_add(total, Ordering::Relaxed);
            }
        }
    }
}
impl Database {
    fn open(job: &Job) -> std::result::Result<Self, String> {
        let connection = rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?;
        // SAFETY: the callback is dropped before its owned connection, including errors.
        let admission=job.shared.reservation.as_ref().map(|r|unsafe {
            super::super::super::sql_admission::observer::Observer::install(&connection,r.ledger(),Some(r.clone()))
        });
        let mut profiler =
            unsafe { super::super::super::sql_profile::Profiler::configured(&connection) }
                .map_err(|e| e.to_string())?;
        if let Some(p)=&mut profiler {if let Some(a)=&admission {p.observe(a.shared.clone());}}
        super::super::reductions::register(&connection).map_err(|e|e.to_string())?;
        let modules = Modules::default();
        // Establish RAII ownership before any fallible binding or hook registration.
        let mut db = Self {
            profiler,
            _admission:admission,
            connection,
            _modules: modules,
            accounting: job.shared.accounting.clone(),
        };
        if let Some(p) = &db.profiler {
            p.context(json!({"scope":"sql-extension","operation":"query_plan",
            "executionPhase":"relationalEvaluation","stage":"SQL read-only group","executor":"sql",
            "sqlWorkGraph":job.graph,"sqlGroup":job.id,"sqlWorkGroup":job.explain}));
        }
        for (name, source) in &job.bindings {
            db._modules
                .bind(&db.connection, name, source.clone())
                .map_err(|e| e.to_string())?;
        }
        let inputs = job
            .bindings
            .keys()
            .map(|n| n.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        db.connection
            .authorizer(Some(
                move |ctx: rusqlite::hooks::AuthContext<'_>| match ctx.action {
                    AuthAction::Select | AuthAction::Recursive => Authorization::Allow,
                    AuthAction::Read { table_name, .. }
                        if inputs.contains(&table_name.to_ascii_lowercase()) =>
                    {
                        Authorization::Allow
                    }
                    AuthAction::Function { function_name } if pure_builtin(function_name) => {
                        Authorization::Allow
                    }
                    _ => Authorization::Deny,
                },
            ))
            .map_err(|e| e.to_string())?;
        let shared = job.shared.clone();
        let interval = if shared.max_steps.is_some() { 1 } else { 1000 };
        db.connection
            .progress_handler(
                interval,
                Some(move || {
                    if super::super::super::sql_profile::explaining() {
                        return false;
                    }
                    let exhausted = shared.max_steps.is_some_and(|max| {
                        shared
                            .steps
                            .fetch_add(1, Ordering::Relaxed)
                            .saturating_add(1)
                            >= max
                    });
                    if exhausted {
                        shared.cancelled.store(true, Ordering::Release);
                    }
                    exhausted || shared.interrupted()
                }),
            )
            .map_err(|e| e.to_string())?;
        Ok(db)
    }
}
pub(in crate::code_rule_dsl::sql_executor) fn pure_builtin(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "rr_nonnegative_count" | "rr_update_sum" | "rr_sum_low" | "rr_sum_high" | "rr_sum_value" | "abs"
            | "avg"
            | "cast"
            | "coalesce"
            | "count"
            | "dense_rank"
            | "first_value"
            | "glob"
            | "group_concat"
            | "ifnull"
            | "iif"
            | "instr"
            | "lag"
            | "last_value"
            | "lead"
            | "length"
            | "like"
            | "lower"
            | "ltrim"
            | "max"
            | "min"
            | "nth_value"
            | "ntile"
            | "nullif"
            | "printf"
            | "quote"
            | "rank"
            | "replace"
            | "round"
            | "row_number"
            | "rtrim"
            | "sign"
            | "substr"
            | "substring"
            | "sum"
            | "total"
            | "trim"
            | "typeof"
            | "unicode"
            | "upper"
    )
}
impl Program for Job {
    fn name(&self) -> &'static str {
        "SQL read-only group"
    }
    fn units(&self) -> f64 {
        self.units
    }
    fn per_unit(&self, lane: Lane) -> Option<f64> {
        (lane == Lane::Sql).then_some(1e-6)
    }
    fn run(&self, _: Lane) -> std::result::Result<ProgramOutput, String> {
        let result = self
            .execute()
            .map_err(|e| format!("SQL group {}: {e}", self.id))
            .and_then(|rows| {
                let rows = Results(rows);
                self.scheduler.finished(self.id, &rows)?;
                Ok(Arc::new(rows) as ProgramOutput)
            });
        if let Err(message) = &result {
            self.scheduler.failed(message.clone());
        }
        result
    }
}
impl Job {
    fn execute(&self) -> std::result::Result<BTreeMap<String, Arc<Rows>>, String> {
        if self.shared.interrupted() {
            return Err("SQL work graph cancelled or timed out".into());
        }
        let db = Database::open(self)?;
        let result = (|| {
            let mut statement = db
                .connection
                .prepare(&self.sql)
                .map_err(|e| e.to_string())?;
            if !statement.readonly() || statement.parameter_count() != 0 {
                return Err("SQL work nodes require parameter-free read-only queries".into());
            }
            let mut rows = self
                .outputs
                .iter()
                .map(|(_, width)| Rows::new(*width, vec![]))
                .collect::<Vec<_>>();
            self.shared.submit()?;
            let read = (|| {
                let mut cursor = statement.query([]).map_err(|e| e.to_string())?;
                while let Some(row) = cursor.next().map_err(|e| e.to_string())? {
                    let tag = usize::try_from(row.get::<_, i64>(0).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                    let output = rows.get_mut(tag).ok_or("invalid SQL work output tag")?;
                    let cells = (1..=output.width)
                        .map(|c| row.get::<_, Option<i64>>(c))
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(|e| e.to_string())?;
                    output.rows.push(cells);
                }
                Ok::<_, String>(())
            })();
            if self.shared.max_steps.is_none() {
                self.shared.steps.fetch_add(
                    statement
                        .get_status(rusqlite::StatementStatus::VmStep)
                        .max(0) as u64,
                    Ordering::Relaxed,
                );
            }
            read?;
            Ok(self
                .outputs
                .iter()
                .zip(rows)
                .map(|((name, _), rows)| (name.clone(), Arc::new(rows)))
                .collect())
        })();
        if let Some(p) = &db.profiler {
            p.checkpoint(&db.connection).map_err(|e| e.to_string())?;
        }
        result
    }
}
