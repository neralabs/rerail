//! Isolated SQL kernels. No alternate production execution path is installed.
mod batch;
mod shapes;
#[cfg(test)]
mod tests;

use rusqlite::{Connection, StatementStatus};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

#[derive(Default, serde::Serialize)]
pub(super) struct Meter {
    prepare_cpu_ms: f64,
    execute_cpu_ms: f64,
    sql_bytes: usize,
    statements: usize,
    preparations: usize,
    vm_steps: u64,
    fullscan_steps: u64,
    sorts: u64,
    autoindex_rows: u64,
    #[serde(skip)]
    last_query: Option<String>,
}
pub(super) fn cpu_ms() -> f64 {
    // SAFETY: getrusage initializes the supplied live structure on supported hosts.
    unsafe {
        let mut r: libc::rusage = std::mem::zeroed();
        assert_eq!(libc::getrusage(libc::RUSAGE_SELF, &mut r), 0);
        ((r.ru_utime.tv_sec + r.ru_stime.tv_sec) as f64 * 1000.0)
            + (r.ru_utime.tv_usec + r.ru_stime.tv_usec) as f64 / 1000.0
    }
}
pub(super) fn peak_bytes() -> u64 {
    // SAFETY: as in cpu_ms.
    unsafe {
        let mut r: libc::rusage = std::mem::zeroed();
        assert_eq!(libc::getrusage(libc::RUSAGE_SELF, &mut r), 0);
        let scale = if cfg!(target_os = "macos") { 1 } else { 1024 };
        r.ru_maxrss as u64 * scale
    }
}
impl Meter {
    pub(super) fn last_query(&self) -> Option<&str> {
        self.last_query.as_deref()
    }
    pub(super) fn record_repeated(
        &mut self,
        stmt: &rusqlite::Statement<'_>,
        bytes: usize,
        executions: usize,
        prepare_cpu: f64,
        execute_cpu: f64,
    ) {
        self.statements += executions;
        self.preparations += 1;
        self.sql_bytes += bytes;
        self.prepare_cpu_ms += prepare_cpu;
        self.execute_cpu_ms += execute_cpu;
        self.vm_steps += stmt.get_status(StatementStatus::VmStep) as u64;
        self.fullscan_steps += stmt.get_status(StatementStatus::FullscanStep) as u64;
        self.sorts += stmt.get_status(StatementStatus::Sort) as u64;
        self.autoindex_rows += stmt.get_status(StatementStatus::AutoIndex) as u64;
    }
    pub(super) fn statement(
        &mut self,
        db: &Connection,
        sql: &str,
    ) -> rusqlite::Result<Vec<Vec<i64>>> {
        if sql.starts_with("SELECT") || sql.starts_with("WITH") {
            self.last_query = Some(sql.to_owned());
        }
        self.sql_bytes += sql.len();
        self.statements += 1;
        self.preparations += 1;
        let start = cpu_ms();
        let mut stmt = db.prepare(sql)?;
        self.prepare_cpu_ms += cpu_ms() - start;
        let start = cpu_ms();
        let mut output = Vec::new();
        if stmt.column_count() == 0 {
            stmt.execute([])?;
        } else {
            let width = stmt.column_count();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                output.push(
                    (0..width)
                        .map(|i| row.get(i))
                        .collect::<rusqlite::Result<_>>()?,
                );
            }
        }
        self.execute_cpu_ms += cpu_ms() - start;
        self.vm_steps += stmt.get_status(StatementStatus::VmStep) as u64;
        self.fullscan_steps += stmt.get_status(StatementStatus::FullscanStep) as u64;
        self.sorts += stmt.get_status(StatementStatus::Sort) as u64;
        self.autoindex_rows += stmt.get_status(StatementStatus::AutoIndex) as u64;
        Ok(output)
    }
}

pub(super) fn facts(db: &Connection, files: &[i64], meter: &mut Meter) -> rusqlite::Result<()> {
    // Internal integer IDs, never user-provided SQL expressions.
    let ids = files
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    meter.statement(db, "CREATE TEMP TABLE n(fid INTEGER,pre INTEGER,parent INTEGER,end INTEGER,kind INTEGER,field INTEGER,flags INTEGER,PRIMARY KEY(fid,pre)) WITHOUT ROWID")?;
    meter.statement(db, &format!("INSERT INTO n SELECT fid,pre,parent,end,kind,field,flags FROM decoded_properties WHERE fid IN ({ids})"))?;
    meter.statement(db, "CREATE INDEX temp.n_kind ON n(kind,fid,pre)")?;
    meter.statement(db, "CREATE INDEX temp.n_parent ON n(fid,parent,field,pre)")?;
    Ok(())
}
pub(super) fn vocabulary(db: &Connection) -> rusqlite::Result<BTreeMap<String, i64>> {
    let mut stmt = db.prepare("SELECT 'kind:'||name,code FROM kind WHERE lang=(SELECT lang FROM file LIMIT 1) UNION ALL SELECT 'field:'||name,fcode FROM field WHERE lang=(SELECT lang FROM file LIMIT 1)")?;
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}

fn check_tree(db: &Connection) -> Result<Value, Box<dyn std::error::Error>> {
    let mut stmt=db.prepare("SELECT f.fid,f.nodes,f.path,hex(f.digest),f.lang,count(n.pre) FROM file f JOIN n ON n.fid=f.fid GROUP BY f.fid")?;
    let files: Vec<(i64, i64, String, String, i64, i64)> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let lang: i64 = db.query_row("SELECT lang FROM file LIMIT 1", [], |r| r.get(0))?;
    if files.iter().any(|f| f.1 != f.5 || f.4 != lang) {
        return Err("complete facts in one vocabulary required".into());
    }
    let mut stmt = db.prepare("SELECT fid,pre,parent,end FROM n ORDER BY fid,pre")?;
    let mut rows = stmt.query([])?;
    let mut stack: Vec<(i64, i64)> = Vec::new();
    let mut current = -1;
    let mut expected = 0;
    while let Some(row) = rows.next()? {
        let (fid, pre, parent, end): (i64, i64, i64, i64) =
            (row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?);
        if fid != current {
            stack.clear();
            current = fid;
            expected = 0;
        }
        while stack.last().is_some_and(|(_, hi)| *hi <= pre) {
            stack.pop();
        }
        let nodes = files
            .iter()
            .find(|f| f.0 == fid)
            .ok_or("missing tree file")?
            .1;
        if pre != expected
            || end <= pre
            || end > nodes
            || parent != stack.last().map_or(-1, |(p, _)| *p)
            || stack.last().is_some_and(|(_, hi)| end > *hi)
            || (pre == 0 && end != nodes)
        {
            return Err("input is not a complete contiguous preorder tree".into());
        }
        expected += 1;
        stack.push((pre, end));
    }
    Ok(json!(
        files
            .iter()
            .map(|f| json!({"fid":f.0,"nodes":f.1,"path":f.2,"sourceDigest":f.3,"language":f.4}))
            .collect::<Vec<_>>()
    ))
}
pub(super) fn code(v: &BTreeMap<String, i64>, name: &str) -> i64 {
    *v.get(name)
        .unwrap_or_else(|| panic!("missing vocabulary {name}"))
}

pub fn run(
    capture: &Path,
    files: &[i64],
    experiment: &str,
    variant: &str,
    depth: usize,
) -> Result<Value, Box<dyn std::error::Error>> {
    if files.is_empty() || files.iter().any(|fid| *fid <= 0) || depth > 10 {
        return Err("nonempty positive file IDs and depth <= 10 required".into());
    }
    let process_start = cpu_ms();
    let directory = tempfile::tempdir()?;
    let copy = directory.path().join("capture.sqlite");
    std::fs::copy(capture, &copy)?;
    let copy_cpu = cpu_ms() - process_start;
    let store = crate::store::Store::open(&copy)?;
    let db = &store.db;
    let v = vocabulary(db)?;
    let mut setup = Meter::default();
    let common_start = cpu_ms();
    facts(db, files, &mut setup)?;
    let common_cpu = cpu_ms() - common_start;
    let check_start = cpu_ms();
    let source_files = check_tree(db)?;
    if source_files.as_array().unwrap().len() != files.len() {
        return Err("missing or repeated source files".into());
    }
    let input_check_cpu = cpu_ms() - check_start;
    let mut meter = Meter::default();
    let start = cpu_ms();
    let mut batch_metrics = json!(null);
    let mut rows = match experiment {
        "range" => shapes::ranges(db, &v, variant, &mut meter)?,
        "demand" => shapes::demand(db, &v, variant, &mut meter)?,
        "utility" => shapes::utility(db, &v, variant, depth, &mut meter)?,
        "batch" => {
            let (rows, counters) = batch::run(&store, files, variant, &mut meter)?;
            batch_metrics = counters;
            rows
        }
        _ => return Err("unknown experiment".into()),
    };
    let kernel_cpu = cpu_ms() - start;
    rows.sort(); // Exact multiset comparison retains duplicate rows and witness identities.
    let temp_pages: i64 = db.query_row("PRAGMA temp.page_count", [], |r| r.get(0))?;
    let page_size: i64 = db.query_row("PRAGMA temp.page_size", [], |r| r.get(0))?;
    let counters = store.readers.0.lock().counters;
    // Diagnostic preparation is outside the timed kernel and does not run predicates.
    let mut plan = Vec::new();
    if let Some(sql) = &meter.last_query {
        let mut stmt = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            plan.push(json!({"id":row.get::<_,i64>(0)?,"parent":row.get::<_,i64>(1)?,"detail":row.get::<_,String>(3)?}));
        }
    }
    Ok(json!({
        "format":"rerail.composition-kernel/v1", "experiment":experiment,"variant":variant,
        "files":files,"depth":depth,"copyCpuMs":copy_cpu,"commonCpuMs":common_cpu,
        "kernelCpuMs":kernel_cpu,"totalKernelCpuMs":common_cpu+kernel_cpu,
        "processCpuMs":cpu_ms()-process_start,"peakRssBytes":peak_bytes(),
        "inputCheckCpuMs":input_check_cpu,"sourceFiles":source_files,
        "tempBytes":temp_pages*page_size,"common":setup,"sql":meter,
        "native":counters,"batch":batch_metrics,"rows":rows,
        "plan":plan,"query":meter.last_query,
        "sqliteVersion":rusqlite::version(),"waveRowLimit":batch::WAVE_ROWS,
    }))
}
