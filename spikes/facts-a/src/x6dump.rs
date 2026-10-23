//! Disposable (x6-rules exploration): `X6_STAGE_DUMP=DIR` makes `eval::finish` write, for every
//! repository clause it runs, each table `rel::TRACE` hands over (inputs, syntax tables, stored
//! stages) as a row multiset: `DIR/<check>__<clause>.json` = `[{name, cols, rows, hash}]`, where
//! `hash` is the wrapping sum of a per-row blake3 digest (128 bits) over the typed values, so two
//! tables with equal hashes and row counts hold the same rows with the same multiplicities.
//! `X6_STAGE_ROWS=<regex>` also writes the rows of the tables whose name matches, one line per row
//! (typed values, tab-separated), sorted, to `DIR/<check>__<clause>/<name>.rows`.
use crate::rel::Traced;
use crate::sql::engine::Table;
use crate::sql::value::V;
use serde_json::json;
use std::io::Write;
use std::path::Path;

fn encode(v: &V, buf: &mut Vec<u8>) {
    match v {
        V::Null => buf.push(0),
        V::Int(i) => {
            buf.push(1);
            buf.extend_from_slice(&i.to_le_bytes());
        }
        V::Real(x) => {
            buf.push(2);
            buf.extend_from_slice(&x.to_bits().to_le_bytes());
        }
        V::Text(s) => {
            buf.push(3);
            buf.extend_from_slice(&(s.len() as u64).to_le_bytes());
            buf.extend_from_slice(s.as_bytes());
        }
    }
}

fn line(v: &V) -> String {
    match v {
        V::Null => "N".into(),
        V::Int(i) => format!("i{i}"),
        V::Real(x) => format!("r{x:?}"),
        V::Text(s) => format!("t{}", serde_json::to_string(&**s).unwrap()),
    }
}

fn table(name: &str, t: &Table, rows_dir: Option<&Path>) -> serde_json::Value {
    let width = t.schema.cols.len();
    let mut sum: u128 = 0;
    let mut buf = Vec::new();
    let mut lines = rows_dir.map(|_| Vec::with_capacity(t.len()));
    for r in 0..t.len() {
        buf.clear();
        let row = t.row(r);
        for v in &row {
            encode(v, &mut buf);
        }
        let d = blake3::hash(&buf);
        sum = sum.wrapping_add(u128::from_le_bytes(d.as_bytes()[..16].try_into().unwrap()));
        if let Some(l) = lines.as_mut() {
            l.push(row.iter().map(line).collect::<Vec<_>>().join("\t"));
        }
    }
    if let (Some(dir), Some(mut l)) = (rows_dir, lines) {
        l.sort_unstable();
        let mut f = std::io::BufWriter::new(std::fs::File::create(dir.join(format!("{name}.rows"))).unwrap());
        for x in l {
            writeln!(f, "{x}").unwrap();
        }
    }
    json!({"name": name, "cols": t.schema.cols, "width": width, "rows": t.len(), "hash": format!("{sum:032x}")})
}

/// Writes one traced run of a repository clause.
pub fn write(dir: &str, label: &str, traced: &Traced) {
    let rows_rx = std::env::var("X6_STAGE_ROWS").ok().map(|r| regex::Regex::new(&r).expect("X6_STAGE_ROWS"));
    let dir = Path::new(dir);
    std::fs::create_dir_all(dir).unwrap();
    let rows_dir = dir.join(label);
    let out: Vec<_> = traced
        .iter()
        .map(|(name, t)| {
            let want = rows_rx.as_ref().is_some_and(|r| r.is_match(name));
            if want {
                std::fs::create_dir_all(&rows_dir).unwrap();
            }
            table(name, t, want.then_some(rows_dir.as_path()))
        })
        .collect();
    std::fs::write(dir.join(format!("{label}.json")), serde_json::to_string_pretty(&out).unwrap()).unwrap();
}

/// Appends one traced per-file run (a `partitionBy: file` clause over one content) as one JSON
/// line `{path, tables: [[name, rows, hash]]}` to `DIR/<check>__<clause>.perfile.<thread>.jsonl`
/// (one file per worker thread, so lines never interleave).
pub fn append(dir: &str, label: &str, path: &str, traced: &Traced) {
    let dir = Path::new(dir);
    std::fs::create_dir_all(dir).unwrap();
    let tables: Vec<_> = traced
        .iter()
        .map(|(name, t)| {
            let s = table(name, t, None);
            json!([name, s["rows"], s["hash"]])
        })
        .collect();
    let thread = format!("{:?}", std::thread::current().id()).chars().filter(char::is_ascii_digit).collect::<String>();
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("{label}.perfile.{}-{thread}.jsonl", std::process::id()))).unwrap();
    writeln!(f, "{}", json!({"path": path, "tables": tables})).unwrap();
}
