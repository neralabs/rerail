//! The relational lab (decision 0095, step 0): a stratum of a real run, captured with everything its native join
//! reads, and run again on its own in a fraction of a second.
//!
//! A pack run is a minute of CPU. A change to a join (an order, a kernel, a backend) is judged by a few strata of it:
//! `up_once`, `p511_restores`, a text test. The lab keeps those strata as files and runs them again without parsing,
//! batching or planning, and checks their rows against the run's:
//!
//! ```text
//! RERAIL_RELATIONAL_LAB=<dir> [RERAIL_RELATIONAL_LAB_ONLY=up_once,p511_restores] [RERAIL_RELATIONAL_LAB_MIN=<work>]
//!     rerail-engine logic <pack> <corpus> <out>          captures <dir>/<stratum>-<k>.sqlite
//! RERAIL_RELATIONAL_LAB_REPLAY=<dir> rerail_engine-<hash> relational_lab --ignored --nocapture
//!     runs each capture again: its rows against the captured ones, its CPU
//! ```
//!
//! A capture holds:
//!
//! - **`meta`**: the stratum's name and its spec, rewritten so every source is a plain table of the capture.
//! - **`s<k>`**: each source the joins read (`r:` a relation held in memory, `t:` a table or a base input read in
//!   place), as rows of integers, symbols as the run's atoms. Which columns hold symbols is not always declared (a
//!   stored table holds them as integers whatever the column's type), so no value is converted.
//! - **`atoms.sqlite`**, one for the directory: the run's atoms' texts by id. The replay interns them in id order into
//!   a new store, whose ids then are the run's (ids are dense and given out in order), and checks that they are.
//! - **`r<k>`**: each relation of the stratum as it was before the run (its seed rows), and **`out<k>`** as it was
//!   after.
//!
//! The selection is by the stratum's planned work (`RERAIL_RELATIONAL_LAB_MIN`, default 10^5 units) or by name. A
//! stratum whose spec the lab does not take yet (SQL probes, gates, requests, a relation kept from an earlier run) is
//! skipped, with its reason on stderr. Captures stop at 2 GB in all.

use super::spec::{ComponentSpec, InputKey, JoinItem, JoinSpec, Line, Source, Start};
use super::{DERIVED, InRel, Interner, input_of, is_dead, scope, symbols};
use rusqlite::{Connection, params_from_iter};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The bytes all captures may take.
const BUDGET: u64 = 2 << 30;
static WRITTEN: AtomicU64 = AtomicU64::new(0);

/// A stratum being captured: where, and its relations (the run's table, the capture's, the column types, and
/// whether it started empty).
pub(in crate::code_rule_dsl) struct Capture {
    path: PathBuf,
    rels: Vec<(String, String, Vec<u8>, bool)>,
}

/// Where stratum `name`, of planned work `work`, is captured, when the lab is on and wants it.
pub(in crate::code_rule_dsl) fn wanted(name: &str, work: f64) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("RERAIL_RELATIONAL_LAB")?);
    if WRITTEN.load(Ordering::Relaxed) >= BUDGET {
        return None;
    }
    if let Ok(only) = std::env::var("RERAIL_RELATIONAL_LAB_ONLY") {
        let short = name.strip_prefix("prog_").unwrap_or(name);
        if !only.split(',').any(|o| o == short || o == name) {
            return None;
        }
    } else {
        let min = std::env::var("RERAIL_RELATIONAL_LAB_MIN").ok().and_then(|m| m.parse().ok()).unwrap_or(1e5);
        if work < min {
            return None;
        }
    }
    let _ = std::fs::create_dir_all(&dir);
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).take(80).collect();
    (0..).map(|k| dir.join(format!("{safe}-{k}.sqlite"))).find(|p| !p.exists())
}

/// Writes `rows` (each `types.len()` values, symbols as the run's atoms) as table `table` of `out`.
fn write_rows(out: &Connection, table: &str, types: &[u8], rows: impl Iterator<Item = Vec<i64>>) -> rusqlite::Result<u64> {
    let cols: Vec<String> = (0..types.len()).map(|c| format!("c{c} INTEGER")).collect();
    out.execute_batch(&format!("CREATE TABLE {table} ({})", if cols.is_empty() { "c_ INTEGER".to_owned() } else { cols.join(", ") }))?;
    if types.is_empty() {
        return Ok(0);
    }
    let marks = vec!["?"; types.len()].join(", ");
    out.execute_batch("BEGIN")?;
    let mut st = out.prepare(&format!("INSERT INTO {table} VALUES ({marks})"))?;
    let mut n = 0;
    for row in rows {
        st.execute(params_from_iter(row))?;
        n += 1;
    }
    drop(st);
    out.execute_batch("COMMIT")?;
    Ok(n)
}

/// A relation's live rows.
fn rows_of(rel: &InRel) -> impl Iterator<Item = Vec<i64>> + '_ {
    (0..rel.len()).filter(|&id| !is_dead(&rel.dead, id as u32)).map(|id| {
        let r = rel.at(id);
        (0..rel.k).map(|c| r.at(c)).collect()
    })
}

/// The rows of the run's table `table` (columns `c0…`, `types`), from the engine where it holds them, else SQL.
fn table_rows(conn: &Connection, table: &str, types: &[u8], syms: &mut Interner) -> super::Result<Vec<Vec<i64>>> {
    if let Some(rel) = DERIVED.lock().unwrap().get(&(scope(conn), table.to_owned())).cloned() {
        return Ok(rows_of(&rel).collect());
    }
    let cols: Vec<String> = (0..types.len()).map(|c| format!("c{c}")).collect();
    let rel = input_of(conn, &InputKey { base: false, sql: table.to_owned(), cols: cols.join(", "), types: String::from_utf8_lossy(types).into_owned() }, syms)?;
    Ok(rows_of(&rel).collect())
}

/// Captures stratum `name`'s sources and seeds before it opens with `spec`, or says why it is not taken.
pub(in crate::code_rule_dsl) fn capture_inputs(conn: &Connection, name: &str, spec: &ComponentSpec, path: &Path) -> Option<Capture> {
    match capture_(conn, name, spec, path) {
        Ok(c) => Some(c),
        Err(why) => {
            eprintln!("relational lab: {name} not captured: {why}");
            let _ = std::fs::remove_file(path);
            None
        }
    }
}

/// `join` with each of its sources that is not the component's (a native relation's, an input's) given by `f`.
fn with_sources(join: &JoinSpec, f: &mut dyn FnMut(&Source) -> Source) -> JoinSpec {
    let items = join
        .items
        .iter()
        .map(|item| {
            let mut item = item.clone();
            if let JoinItem::Atom { src, .. } | JoinItem::Count { src, .. } | JoinItem::Extreme { src, .. } | JoinItem::StrJoin { src, .. } = &mut item
                && matches!(src, Source::Native(_) | Source::Input(_))
            {
                *src = f(src);
            }
            item
        })
        .collect();
    JoinSpec { items }
}

fn capture_(conn: &Connection, name: &str, spec: &ComponentSpec, path: &Path) -> Result<Capture, String> {
    let shared = symbols(conn).map_err(|e| e.to_string())?;
    let mut syms = Interner::for_loading(&shared);
    let out = Connection::open(path).map_err(|e| e.to_string())?;
    out.execute_batch("PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF; CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT)").map_err(|e| e.to_string())?;
    // The joins' sources, in order of first use: each becomes a table of the capture.
    let mut sources: Vec<Source> = Vec::new();
    let mut rels: Vec<(String, String, Vec<u8>, bool)> = Vec::new();
    for line in &spec.lines {
        match line {
            // The run held its rows in memory, not in tables; the replay writes them, to compare them.
            // Where the run put it (`Device`): the replay runs every capture on the CPU, and again on the device.
            Line::NoWrite | Line::Device | Line::Leveled | Line::Lattice { .. } | Line::Seed { .. } | Line::Join { .. } => {}
            Line::Rel { name, types, start } => {
                let empty = match start {
                    Start::Rows => false,
                    Start::Empty => true,
                    _ => return Err("a relation kept or read from a query".to_owned()),
                };
                rels.push((name.clone(), format!("r{}", rels.len()), types.as_bytes().to_vec(), empty));
            }
            Line::Request(_) => return Err("a `request` line".to_owned()),
            Line::Produced(_) => return Err("a `produced` line".to_owned()),
            Line::Probe { .. } => return Err("a `probe` line".to_owned()),
            Line::Gate { .. } => return Err("a `gate` line".to_owned()),
            Line::Levels { .. } => return Err("a `levels` line".to_owned()),
            Line::Edge(_) => return Err("a `rule` line".to_owned()),
        }
        let joins: Vec<&Arc<JoinSpec>> = match line {
            Line::Seed { join, .. } => vec![join],
            Line::Join { join, whole, .. } => std::iter::once(join).chain(whole).collect(),
            _ => Vec::new(),
        };
        for join in joins {
            with_sources(join, &mut |src| {
                if !sources.contains(src) {
                    sources.push(src.clone());
                }
                src.clone()
            });
        }
    }
    // Each source as a table of the capture.
    let mut tables: Vec<Source> = Vec::new();
    let mut bytes = 0u64;
    for (k, src) in sources.iter().enumerate() {
        let (rel, base): (Arc<InRel>, bool) = match src {
            Source::Native(table) => (DERIVED.lock().unwrap().get(&(scope(conn), table.clone())).cloned().ok_or_else(|| format!("no native relation {table}"))?, false),
            Source::Input(key) => (input_of(conn, key, &mut syms).map_err(|e| e.to_string())?, key.base),
            _ => unreachable!("only a native relation or an input is a source"),
        };
        let table = format!("s{k}");
        let n = write_rows(&out, &table, &rel.types, rows_of(&rel)).map_err(|e| e.to_string())?;
        bytes += n * 8 * rel.k as u64;
        let cols: Vec<String> = (0..rel.k).map(|c| format!("c{c}")).collect();
        tables.push(Source::Input(InputKey { base, sql: table, cols: cols.join(", "), types: String::from_utf8_lossy(&rel.types).into_owned() }));
    }
    for (table, lab, types, empty) in &rels {
        // An `empty` relation starts with no rows: nothing wrote its table, and it may have none.
        let rows = if *empty { Vec::new() } else { table_rows(conn, table, types, &mut syms).map_err(|e| e.to_string())? };
        write_rows(&out, lab, types, rows.into_iter()).map_err(|e| e.to_string())?;
    }
    // The spec over the capture's tables: its relations by their capture names, each join's sources replaced.
    let to_capture = |join: &JoinSpec| Arc::new(with_sources(join, &mut |src| tables[sources.iter().position(|s| s == src).expect("a source seen")].clone()));
    let mut r = 0;
    let lines: Vec<Line> = spec
        .lines
        .iter()
        .filter_map(|line| match line {
            Line::NoWrite => None,
            Line::Rel { types, start, .. } => {
                r += 1;
                Some(Line::Rel { name: format!("r{}", r - 1), types: types.clone(), start: start.clone() })
            }
            Line::Seed { head, join } => Some(Line::Seed { head: *head, join: to_capture(join) }),
            Line::Join { head, delta, join, whole } => Some(Line::Join { head: *head, delta: *delta, join: to_capture(join), whole: whole.as_deref().map(to_capture) }),
            _ => None,
        })
        .collect();
    let captured = ComponentSpec { name: spec.name.clone(), lines };
    for (k, v) in [("name", spec.name.as_str()), ("stratum", name), ("spec", &captured.to_text())] {
        out.execute("INSERT INTO meta VALUES (?1, ?2)", [k, v]).map_err(|e| e.to_string())?;
    }
    WRITTEN.fetch_add(bytes, Ordering::Relaxed);
    Ok(Capture { path: path.to_owned(), rels })
}

/// Appends the atoms the run gave out since the last capture to the directory's `atoms.sqlite` (id, text).
fn atoms(conn: &Connection, dir: &Path) -> Result<(), String> {
    static STORED: std::sync::Mutex<usize> = std::sync::Mutex::new(0);
    let mut stored = STORED.lock().unwrap();
    let arena = crate::content_store::atoms::Atoms::for_connection(conn);
    let n = arena.len();
    let out = Connection::open(dir.join("atoms.sqlite")).map_err(|e| e.to_string())?;
    out.execute_batch("PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF; CREATE TABLE IF NOT EXISTS atoms (id INTEGER PRIMARY KEY, text TEXT NOT NULL); BEGIN")
        .map_err(|e| e.to_string())?;
    let from: usize = out.query_row("SELECT coalesce(max(id) + 1, 0) FROM atoms", [], |r| r.get::<_, i64>(0)).map_err(|e| e.to_string())? as usize;
    {
        let mut st = out.prepare("INSERT INTO atoms VALUES (?1, ?2)").map_err(|e| e.to_string())?;
        for id in from.max(*stored)..n {
            let text = arena.with_str(id as crate::content_store::atoms::Sym, str::to_owned).ok_or_else(|| format!("no atom {id}"))?;
            st.execute(rusqlite::params![id as i64, text]).map_err(|e| e.to_string())?;
        }
    }
    out.execute_batch("COMMIT").map_err(|e| e.to_string())?;
    *stored = n;
    Ok(())
}

/// Writes the stratum's relations as the run left them (`out<k>`), and closes the capture.
pub(in crate::code_rule_dsl) fn capture_outputs(conn: &Connection, cap: Capture) {
    let done = (|| -> Result<(), String> {
        let shared = symbols(conn).map_err(|e| e.to_string())?;
        let mut syms = Interner::for_loading(&shared);
        let out = Connection::open(&cap.path).map_err(|e| e.to_string())?;
        for (k, (table, _, types, _)) in cap.rels.iter().enumerate() {
            let rows = table_rows(conn, table, types, &mut syms).map_err(|e| e.to_string())?;
            write_rows(&out, &format!("out{k}"), types, rows.into_iter()).map_err(|e| e.to_string())?;
        }
        atoms(conn, cap.path.parent().unwrap_or(Path::new(".")))
    })();
    if let Err(why) = done {
        eprintln!("relational lab: {}: outputs not captured: {why}", cap.path.display());
        let _ = std::fs::remove_file(&cap.path);
    }
}

// cratify: its tests (`lab/tests.rs`) are rerail-engine's, which mounts them: they run what this crate is below.
