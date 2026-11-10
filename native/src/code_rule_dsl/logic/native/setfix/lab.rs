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

use super::{DERIVED, InRel, Interner, input, is_dead, scope, symbols, unhex};
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

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
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
    let rel = input(conn, &format!("x:{}:{}:{}", hex(table), hex(&cols.join(", ")), String::from_utf8_lossy(types)), syms)?;
    Ok(rows_of(&rel).collect())
}

/// Captures stratum `name`'s sources and seeds before it opens with `spec`, or says why it is not taken.
pub(in crate::code_rule_dsl) fn capture_inputs(conn: &Connection, name: &str, spec: &str, path: &Path) -> Option<Capture> {
    match capture_(conn, name, spec, path) {
        Ok(c) => Some(c),
        Err(why) => {
            eprintln!("relational lab: {name} not captured: {why}");
            let _ = std::fs::remove_file(path);
            None
        }
    }
}

fn capture_(conn: &Connection, name: &str, spec: &str, path: &Path) -> Result<Capture, String> {
    let shared = symbols(conn).map_err(|e| e.to_string())?;
    let mut syms = Interner::for_loading(&shared);
    let out = Connection::open(path).map_err(|e| e.to_string())?;
    out.execute_batch("PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF; CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT)").map_err(|e| e.to_string())?;
    let mut sources: Vec<String> = Vec::new();
    let mut rels: Vec<(String, String, Vec<u8>, bool)> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    // The source tokens of a native join (decoded), rewritten to the capture's tables.
    let rewrite = |join: &str, sources: &mut Vec<String>| -> String {
        join.lines()
            .map(|line| {
                line.split(' ')
                    .map(|tok| {
                        if tok.starts_with("r:") || tok.starts_with("t:") {
                            let k = sources.iter().position(|s| s == tok).unwrap_or_else(|| {
                                sources.push(tok.to_owned());
                                sources.len() - 1
                            });
                            format!("@SRC{k}@")
                        } else {
                            tok.to_owned()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    for line in spec.lines() {
        let toks: Vec<&str> = line.split(' ').collect();
        match toks.first().copied() {
            Some("name") => lines.push(line.to_owned()),
            // The run held its rows in memory, not in tables; the replay writes them, to compare them.
            Some("nowrite") => {}
            Some("rel") => {
                if toks.len() > 3 && toks[3..].iter().any(|t| *t != "empty") {
                    return Err(format!("a relation with {:?} (kept or read from a query)", &toks[3..]));
                }
                let lab = format!("r{}", rels.len());
                rels.push((toks[1].to_owned(), lab.clone(), toks[2].as_bytes().to_vec(), toks.get(3) == Some(&"empty")));
                lines.push(std::iter::once("rel").chain([lab.as_str()]).chain(toks[2..].iter().copied()).collect::<Vec<_>>().join(" "));
            }
            Some("seed") if toks.len() == 3 => {
                let join = unhex(toks[2]).map_err(|e| e.to_string())?;
                lines.push(format!("seed {} @HEX{}@", toks[1], hex(&rewrite(&join, &mut sources))));
            }
            Some("join") if toks.len() >= 4 => {
                let mut l = format!("join {} {}", toks[1], toks[2]);
                for t in &toks[3..] {
                    let join = unhex(t).map_err(|e| e.to_string())?;
                    l += &format!(" @HEX{}@", hex(&rewrite(&join, &mut sources)));
                }
                lines.push(l);
            }
            Some(other) => return Err(format!("a `{other}` line")),
            None => {}
        }
    }
    // Each source as a table of the capture.
    let mut tables: Vec<String> = Vec::new();
    let mut bytes = 0u64;
    for (k, tok) in sources.iter().enumerate() {
        let rel: Arc<InRel> = if let Some(t) = tok.strip_prefix("r:") {
            let table = unhex(t).map_err(|e| e.to_string())?;
            DERIVED.lock().unwrap().get(&(scope(conn), table.clone())).cloned().ok_or_else(|| format!("no native relation {table}"))?
        } else {
            input(conn, &tok[2..], &mut syms).map_err(|e| e.to_string())?
        };
        let table = format!("s{k}");
        let n = write_rows(&out, &table, &rel.types, rows_of(&rel)).map_err(|e| e.to_string())?;
        bytes += n * 8 * rel.k as u64;
        let cols: Vec<String> = (0..rel.k).map(|c| format!("c{c}")).collect();
        let flag = if tok.starts_with("t:b:") { "b" } else { "x" };
        tables.push(format!("t:{flag}:{}:{}:{}", hex(&table), hex(&cols.join(", ")), String::from_utf8_lossy(&rel.types)));
    }
    for (table, lab, types, empty) in &rels {
        // An `empty` relation starts with no rows: nothing wrote its table, and it may have none.
        let rows = if *empty { Vec::new() } else { table_rows(conn, table, types, &mut syms).map_err(|e| e.to_string())? };
        write_rows(&out, lab, types, rows.into_iter()).map_err(|e| e.to_string())?;
    }
    // The spec over the capture's tables: each join's source tokens replaced, then hex again.
    let spec: Vec<String> = lines
        .iter()
        .map(|l| {
            l.split(' ')
                .map(|tok| match tok.strip_prefix("@HEX").and_then(|t| t.strip_suffix('@')) {
                    Some(h) => {
                        let mut join = unhex(h).unwrap_or_default();
                        for (k, t) in tables.iter().enumerate() {
                            join = join.replace(&format!("@SRC{k}@"), t);
                        }
                        hex(&join)
                    }
                    None => tok.to_owned(),
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    let name_line = spec.iter().find_map(|l| l.strip_prefix("name ")).unwrap_or(name).to_owned();
    for (k, v) in [("name", name_line.as_str()), ("stratum", name), ("spec", &spec.join("\n"))] {
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

#[cfg(test)]
mod tests {
    use super::super::thread_cpu;
    use rusqlite::Connection;
    use std::path::Path;

    /// The distinct rows of `schema.table`, each as text, sorted.
    fn rows(db: &Connection, schema: &str, table: &str) -> Vec<String> {
        let n: i64 = db.query_row(&format!("SELECT count(*) FROM {schema}.pragma_table_info('{table}')"), [], |r| r.get(0)).unwrap();
        let n = n as usize;
        let mut st = db.prepare(&format!("SELECT * FROM {schema}.{table}")).unwrap();
        let mut out: Vec<String> = st
            .query_map([], |r| (0..n).map(|c| r.get_ref(c).map(|v| format!("{v:?}"))).collect::<rusqlite::Result<Vec<_>>>().map(|v| v.join("\t")))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        out.sort();
        out.dedup();
        out
    }

    /// Gives `db`'s new store the run's atoms, at the run's ids: the texts of `atoms` interned in id order.
    fn same_atoms(db: &Connection, atoms: &Path) {
        let arena = crate::content_store::atoms::Atoms::for_connection(db);
        let src = Connection::open(atoms).unwrap();
        let mut st = src.prepare("SELECT id, text FROM atoms ORDER BY id").unwrap();
        let texts: Vec<(i64, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        let have = arena.len();
        for (id, text) in &texts {
            let id = *id as usize;
            if id < have {
                let mine = arena.with_str(id as crate::content_store::atoms::Sym, str::to_owned).unwrap();
                assert_eq!(&mine, text, "the new store's atom {id} is not the run's");
            }
        }
        let rest: Vec<&str> = texts.iter().filter(|(id, _)| *id as usize >= have).map(|(_, t)| t.as_str()).collect();
        for (chunk, start) in rest.chunks(4096).zip((have..).step_by(4096)) {
            let ids = arena.intern_all(chunk).unwrap();
            for (k, id) in ids.iter().enumerate() {
                assert_eq!(*id as usize, start + k, "atom {:?} took id {id}, the run's is {}", chunk[k], start + k);
            }
        }
    }

    /// Each capture of `RERAIL_RELATIONAL_LAB_REPLAY` run again on a store of its own: its rows against the run's,
    /// and its CPU.
    #[test]
    #[ignore = "reads captures a run made with RERAIL_RELATIONAL_LAB"]
    fn relational_lab() {
        let dir = std::env::var("RERAIL_RELATIONAL_LAB_REPLAY").expect("RERAIL_RELATIONAL_LAB_REPLAY");
        let mut captures: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "sqlite") && p.file_name().is_some_and(|n| n != "atoms.sqlite")).collect();
        captures.sort();
        let (mut same, mut all) = (0, 0);
        #[cfg(target_os = "macos")]
        let mut device = Device::default();
        println!("{:>9} {:>9} {:>10}  {:<5} capture", "cpu ms", "solve ms", "rows", "rows");
        for path in captures {
            let store = crate::code_rule_dsl::prefilter::tests::temp();
            drop(crate::code_rule_dsl::prefilter::tests::open(&store));
            let db = Connection::open(&store).unwrap();
            crate::code_rule_dsl::logic::native::register(&db).unwrap();
            crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
            same_atoms(&db, &Path::new(&dir).join("atoms.sqlite"));
            db.execute("ATTACH ?1 AS cap", [path.to_string_lossy()]).unwrap();
            let get = |k: &str| -> String { db.query_row("SELECT value FROM cap.meta WHERE key = ?1", [k], |r| r.get(0)).unwrap() };
            let (name, spec) = (get("name"), get("spec"));
            let tables: Vec<String> = {
                let mut st = db.prepare("SELECT name FROM cap.sqlite_master WHERE type = 'table' AND name != 'meta'").unwrap();
                st.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
            };
            // Each table into the store, its texts interned as this store's atoms.
            for t in &tables {
                let info: Vec<(String, String)> = {
                    let mut st = db.prepare(&format!("SELECT name, type FROM cap.pragma_table_info('{t}')")).unwrap();
                    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
                };
                let cols: Vec<String> = info.iter().map(|(c, _)| c.clone()).collect();
                // In `temp`: a write there commits nothing of the store, whose new atoms are pending (0049).
                db.execute_batch(&format!("CREATE TEMP TABLE {t} ({}); INSERT INTO temp.{t} SELECT * FROM cap.{t};", cols.join(", "))).unwrap();
            }
            let outs: Vec<&String> = tables.iter().filter(|t| t.starts_with("out")).collect();
            super::super::open(&db, &spec).unwrap();
            #[cfg(target_os = "macos")]
            device.add(&super::super::scans(&db, &name).unwrap());
            #[cfg(target_os = "macos")]
            device.tiles(&super::super::tiles(&db, &name).unwrap());
            let c0 = thread_cpu();
            let result = super::super::run(&db, &name, 0).unwrap();
            let cpu = (thread_cpu() - c0) * 1e3;
            super::super::close(&db, &name).unwrap();
            let solve: f64 = result.split_whitespace().nth(5).and_then(|s| s.parse().ok()).unwrap_or(f64::NAN);
            let mut ok = true;
            let mut n = 0;
            for out in &outs {
                let k = &out[3..];
                let (got, want) = (rows(&db, "temp", &format!("r{k}")), rows(&db, "cap", out));
                n += want.len();
                if got != want {
                    ok = false;
                    println!("    r{k}: {} rows, the run had {}", got.len(), want.len());
                }
            }
            all += 1;
            same += usize::from(ok);
            println!("{cpu:>9.1} {solve:>9.1} {n:>10}  {:<5} {}", if ok { "same" } else { "DIFF" }, path.file_name().unwrap().to_string_lossy());
            drop(db);
            let _ = std::fs::remove_file(&store);
        }
        println!("{same} of {all} captures give the run's rows");
        #[cfg(target_os = "macos")]
        device.report();
        assert_eq!(same, all);
    }

    /// The captures' fused filters over input relations (`setfix::scans`), each run on Metal over its rows in the
    /// 16-bit layout (`metal_rows`) and checked against the CPU's rows.
    #[cfg(target_os = "macos")]
    #[derive(Default)]
    struct Device { key: (usize, usize, u64, f64, usize, usize), key_host: f64, joins: usize, joins_same: usize, joins_off: usize, pairs: usize, compared: u64, join_cpu: f64, join_gpu: f64, cases: usize, same: usize, unsupported: usize, rows: usize, passed: usize, cpu: f64, gpu: f64, bytes: usize, wide: usize, forms: [usize; 3] }

    #[cfg(target_os = "macos")]
    impl Device {
        fn add(&mut self, cases: &[super::super::ScanCase]) {
            use crate::code_rule_dsl::logic::native::setfix::rowcode::Word;
            use crate::code_rule_dsl::transducer::packed::metal_rows::{Form, Input, Resident, scan};
            for case in cases {
                let n = case.gathered.first().map_or(0, |g| g.1.len());
                let at = |c: usize| case.gathered.iter().position(|g| g.0 == c).expect("gathered");
                let inputs: Vec<Input> = case.inputs.iter().map(|w| match *w { Word::Col(c) => Input::Col(at(c)), Word::Doc(c) => Input::Doc(at(c)) }).collect();
                let cols: Vec<&[i64]> = case.gathered.iter().map(|g| g.1.as_slice()).collect();
                let resident = Resident::new(&cols, n).expect("resident");
                let got = match scan(&case.program, &inputs, &resident, &case.params) {
                    Ok(s) => s,
                    Err(_) => { self.unsupported += 1; continue; }
                };
                let mut rows = got.rows.clone();
                rows.sort_unstable();
                self.cases += 1;
                if rows == case.passed { self.same += 1 } else { println!("    device DIFF {}: {} rows, the CPU {}", case.rule, rows.len(), case.passed.len()) }
                self.rows += n;
                self.passed += case.passed.len();
                self.cpu += case.cpu_seconds;
                self.gpu += got.gpu_seconds;
                self.bytes += resident.bytes;
                self.wide += 8 * n * cols.len();
                for f in &resident.forms { self.forms[match f { Form::Low16 | Form::Code16 | Form::Group => 0, Form::I32 => 1, Form::I64 => 2 }] += 1; }
            }
        }

        fn tiles(&mut self, cases: &[super::super::TileCase]) {
            use crate::code_rule_dsl::transducer::packed::metal_rows::{Codes, Form, Grouping, Joined, Resident, tile_join};
            let check = |got: &Joined, case: &super::super::TileCase, how: &str| -> bool {
                let mut pairs = got.pairs.clone();
                pairs.sort_unstable();
                let mut want = case.pairs.clone();
                want.sort_unstable();
                if pairs != want { println!("    tile DIFF {how} {}: {} pairs, the CPU {}", case.rule, pairs.len(), want.len()) }
                pairs == want
            };
            for case in cases {
                let (nl, nr) = (case.left[0].len(), case.right[0].len());
                self.pairs += case.pairs.len();
                self.join_cpu += case.cpu_seconds;
                // By key run: key 0 the group, each symbol key after it a 16-bit code of one dictionary for both sides.
                let k = case.left.len();
                let h0 = thread_cpu();
                let codes: Vec<Option<Codes>> = (0..k).map(|i| (i > 0 && case.symbols[i]).then(|| Codes::of(&[&case.left[i], &case.right[i]])).flatten()).collect();
                let refs: Vec<Option<&Codes>> = codes.iter().map(Option::as_ref).collect();
                let l: Vec<&[i64]> = case.left.iter().map(Vec::as_slice).collect();
                let r: Vec<&[i64]> = case.right.iter().map(Vec::as_slice).collect();
                let keys: Vec<usize> = (0..k).collect();
                let by_key = Resident::grouped(&l, nl, Grouping::Key, &refs).and_then(|a| Resident::grouped(&r, nr, Grouping::Key, &refs).map(|b| (a, b)));
                let joined = by_key.and_then(|(a, b)| { let got = tile_join(&a, &keys, &b, &keys)?; Ok((got, a.bytes + b.bytes)) });
                // The host's part: the codes, the sort into key runs, the upload and the runs' merge (waiting is no CPU).
                self.key_host += thread_cpu() - h0;
                match joined {
                    Ok((got, bytes)) => {
                        self.key.0 += 1;
                        self.key.1 += usize::from(check(&got, case, "by key"));
                        self.key.2 += got.compared;
                        self.key.3 += got.gpu_seconds;
                        self.key.4 += bytes;
                        self.key.5 += 8 * k * (nl + nr);
                    }
                    Err(_) => self.joins_off += 1,
                }
                // By chunk, when key 0 is a node column of one chunk on both sides: the first key that is.
                let order = (0..k).find_map(|k0| {
                    let mut keys: Vec<usize> = vec![k0];
                    keys.extend((0..k).filter(|&x| x != k0));
                    let l: Vec<&[i64]> = keys.iter().map(|&x| case.left[x].as_slice()).collect();
                    let r: Vec<&[i64]> = keys.iter().map(|&x| case.right[x].as_slice()).collect();
                    let (l, r) = (Resident::new(&l, nl).ok()?, Resident::new(&r, nr).ok()?);
                    // Node ids: `d << 32 | i`, so a key 0 under 2^32 is not one (a number of a chunk of its own).
                    let nodes = case.left[k0].iter().chain(&case.right[k0]).all(|&v| v >> 32 > 0);
                    (l.forms[0] == Form::Low16 && r.forms[0] == Form::Low16 && nodes).then_some((l, r))
                });
                let Some((l, r)) = order else { continue };
                let Ok(got) = tile_join(&l, &keys, &r, &keys) else { continue };
                self.joins += 1;
                self.joins_same += usize::from(check(&got, case, "by chunk"));
                self.compared += got.compared;
                self.join_gpu += got.gpu_seconds;
            }
        }

        fn report(&self) {
            println!(
                "device joins: {} pairs, CPU hash join {:.1} ms; by key run: {} of {} give the CPU's pairs ({} not on the device), {} compared, GPU {:.1} ms with {:.1} ms of host CPU (sort, codes, upload, merge), keys {} kB against {} kB as i64; by chunk: {} of {}, {} compared, GPU {:.1} ms",
                self.pairs, self.join_cpu * 1e3, self.key.1, self.key.0, self.joins_off, self.key.2, self.key.3 * 1e3, self.key_host * 1e3, self.key.4 >> 10, self.key.5 >> 10,
                self.joins_same, self.joins, self.compared, self.join_gpu * 1e3
            );
            assert_eq!(self.joins_same, self.joins, "every chunk tile join gives the CPU's pairs");
            assert_eq!(self.key.1, self.key.0, "every key-run tile join gives the CPU's pairs");
            println!(
                "device scans: {} of {} give the CPU's rows ({} not on the device); {} rows in, {} passed; CPU {:.1} ms, GPU {:.1} ms; columns {} kB as 16-bit layout, {} kB as i64 (low16 {}, i32 {}, i64 {})",
                self.same, self.cases, self.unsupported, self.rows, self.passed, self.cpu * 1e3, self.gpu * 1e3, self.bytes >> 10, self.wide >> 10, self.forms[0], self.forms[1], self.forms[2]
            );
            assert_eq!(self.same, self.cases, "every device scan gives the CPU's rows");
        }
    }
}
