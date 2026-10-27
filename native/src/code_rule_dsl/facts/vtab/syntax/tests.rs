use super::*;
use crate::code_rule_dsl::facts::core::{self, Built};
use crate::code_rule_dsl::language::{GrammarDialect, dialect_by_label, tape_tag};
use crate::code_rule_dsl::model::FileInput;
use crate::content_store::Binding;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl")
}
fn temp(what: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("rerail-vtab-{what}-{}-{nonce}.sqlite", std::process::id()))
}

/// CPython 3.13's `fnmatch`, `ftplib`, `glob`, `json/` and `shlex`: the planner fixture's corpus, in its `doc` order.
fn corpus() -> Vec<(String, String)> {
    let raw = zstd::decode_all(std::fs::File::open(root().join("facts/vtab/golden/corpus.json.zst")).unwrap()).unwrap();
    serde_json::from_slice(&raw).unwrap()
}
fn facts(path: &str, source: &str) -> Arc<Facts> {
    let file = FileInput::new(path.to_owned(), "python".to_owned(), Arc::new(source.to_owned()), None, None, "core").unwrap();
    match core::build(&file).unwrap() {
        Built::Facts(f) => Arc::new(f),
        Built::Unparsed => panic!("{path} did not parse"),
    }
}
fn documents() -> Vec<Document> {
    corpus()
        .into_iter()
        .enumerate()
        .map(|(d, (path, source))| {
            let facts = facts(&path, &source);
            Document { key: d as u32, core: format!("/w#core:{path}"), digest: core::digest(&facts), path, facts }
        })
        .collect()
}

/// How a column's integers decode to T's text.
#[derive(Clone, Copy)]
enum Ty {
    Int,
    Node,
    Kind,
    Field,
    Sym,
}
const RELATIONS: &[(&str, &[Ty])] = &[
    ("doc", &[Ty::Int, Ty::Sym, Ty::Sym]),
    ("pkg", &[Ty::Int, Ty::Sym]),
    ("node", &[Ty::Int, Ty::Node, Ty::Kind, Ty::Int, Ty::Int]),
    ("child", &[Ty::Node, Ty::Node, Ty::Int]),
    ("field", &[Ty::Node, Ty::Field, Ty::Node]),
    ("text", &[Ty::Node, Ty::Sym]),
    ("intval", &[Ty::Node, Ty::Int]),
    ("op", &[Ty::Node, Ty::Sym]),
    ("tape", &[Ty::Node, Ty::Node, Ty::Int, Ty::Int]),
];

/// Every relation holds T's exporter's rows for the same files: node ids map to T's global ids by each document's
/// first id, and atoms decode to T's strings.
#[test]
fn relations_hold_t_exporters_rows() {
    let store = ContentStore::open(&temp("store")).unwrap();
    let db = Connection::open_in_memory().unwrap();
    let modules = Modules::default();
    for (name, source) in sources(&store, documents()).unwrap() {
        modules.bind(&db, name, source).unwrap();
    }
    let t = Connection::open_in_memory().unwrap();
    let dump = zstd::decode_all(std::fs::File::open(root().join("logic/plan/golden/facts.sql.zst")).unwrap()).unwrap();
    t.execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    let first: HashMap<i64, i64> =
        t.prepare("SELECT d, min(n) FROM node GROUP BY d").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    let python: GrammarDialect = dialect_by_label("python").unwrap();
    let (tag, catalog) = (tape_tag(python).unwrap(), python.kinds());
    let mut failures = Vec::new();
    for &(rel, types) in RELATIONS {
        let mut got = BTreeSet::new();
        let mut stmt = db.prepare(&format!("SELECT * FROM {rel}")).unwrap();
        let mut rows = stmt.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            let row: Vec<String> = types
                .iter()
                .enumerate()
                .map(|(i, ty)| {
                    let v: i64 = r.get(i).unwrap();
                    match ty {
                        Ty::Int => v.to_string(),
                        Ty::Node if v == -1 => "-1".to_owned(),
                        Ty::Node => (first[&(v >> 32)] + (v & 0xffff_ffff)).to_string(),
                        Ty::Kind => {
                            assert_eq!(v / i64::from(KIND_STRIDE), i64::from(tag));
                            catalog.kind_name((v % i64::from(KIND_STRIDE)) as u16).unwrap().to_owned()
                        }
                        Ty::Field => {
                            assert_eq!(v >> 16, i64::from(tag));
                            catalog.field_name((v & 0xffff) as u16).unwrap().to_owned()
                        }
                        Ty::Sym => store.atom_value(v as u32).unwrap().unwrap(),
                    }
                })
                .collect();
            got.insert(row.join("\t"));
        }
        let mut stmt = t.prepare(&format!("SELECT * FROM {rel}")).unwrap();
        let expected: BTreeSet<String> = stmt
            .query_map([], |r| {
                Ok((0..types.len())
                    .map(|i| match r.get_ref(i).unwrap() {
                        rusqlite::types::ValueRef::Integer(v) => v.to_string(),
                        rusqlite::types::ValueRef::Text(s) => String::from_utf8_lossy(s).into_owned(),
                        other => panic!("{rel}: {other:?}"),
                    })
                    .collect::<Vec<_>>()
                    .join("\t"))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        if got != expected {
            let missing: Vec<_> = expected.difference(&got).take(3).collect();
            let extra: Vec<_> = got.difference(&expected).take(3).collect();
            failures.push(format!(
                "{rel}: {} rows, T {}; missing {missing:?}, extra {extra:?}",
                got.len(),
                expected.len()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Lookups by node, parent, document and kind and ranges over ids return exactly the rows a scan filters to.
#[test]
fn lookups_agree_with_scans() {
    let store = ContentStore::open(&temp("lookups")).unwrap();
    let db = Connection::open_in_memory().unwrap();
    let modules = Modules::default();
    for (name, source) in sources(&store, documents().into_iter().take(3).collect()).unwrap() {
        modules.bind(&db, name, source).unwrap();
    }
    let python: GrammarDialect = dialect_by_label("python").unwrap();
    let call = core::kind_atom(tape_tag(python).unwrap(), python.kinds().kind_id("call"));
    // A node of document 1 with fields, found by scanning (`+` hides the constraints).
    let n: i64 = db
        .query_row(
            "SELECT p FROM field WHERE +p >= 1 << 32 AND +p < 2 << 32 GROUP BY +p ORDER BY count(*) DESC, +p LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for (rel, filter) in [
        ("node", format!("n = {n}")),
        ("node", "d = 2".to_owned()),
        ("node", format!("kind = {call}")),
        ("node", format!("n BETWEEN {} AND {}", n, n + 30)),
        ("node", format!("n > {} AND n < {}", (1i64 << 32) - 5, (1i64 << 32) + 3)),
        ("child", format!("p = {n}")),
        ("child", format!("c IN ({n}, {}, 0, 7)", n + 1)),
        ("field", format!("p = {n}")),
        ("field", format!("c = {}", n + 1)),
        ("tape", "par = -1".to_owned()),
        ("tape", format!("par = {n}")),
        ("tape", format!("n >= {n} AND n <= {}", n + 10)),
    ] {
        let key = |sql: &str| -> Vec<String> {
            let mut stmt = db.prepare(sql).unwrap();
            let width = stmt.column_count();
            let mut out: Vec<String> = stmt
                .query_map([], |r| Ok((0..width).map(|i| r.get::<_, i64>(i).unwrap().to_string()).collect::<Vec<_>>().join(",")))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            out.sort();
            out
        };
        let indexed = key(&format!("SELECT * FROM {rel} WHERE {filter}"));
        // `+` on every column reference hides the constraint from xBestIndex: SQLite scans and filters.
        let hidden = regex::Regex::new(r"\b(n|p|c|d|kind|par) ").unwrap().replace_all(&filter, "+$1 ");
        let scanned = key(&format!("SELECT * FROM {rel} WHERE {hidden}"));
        assert!(!indexed.is_empty() || filter.contains("0, 7"), "{rel} {filter}: no rows");
        assert_eq!(indexed, scanned, "{rel} {filter}");
    }
}

/// A document whose `#core:` name now binds other facts is refused when the batch is bound.
#[test]
fn binding_refuses_stale_documents() {
    let store = ContentStore::open(&temp("stale")).unwrap();
    let db = Connection::open_in_memory().unwrap();
    let modules = Modules::default();
    let docs = || documents().into_iter().take(2).collect::<Vec<_>>();
    let bindings: Vec<Binding> = docs().iter().map(|d| Binding { name: d.core.clone(), digest: d.digest }).collect();
    store.bind_names(&bindings).unwrap();
    bind(&modules, &db, &store, docs()).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM doc", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    store.bind_names(&[Binding { name: bindings[1].name.clone(), digest: [7; 32] }]).unwrap();
    let err = bind(&modules, &db, &store, docs()).unwrap_err();
    assert!(err.to_string().contains("stale core facts for /w#core:ftplib.py"), "{err}");
}

#[test]
fn modules_and_packages_follow_t() {
    assert_eq!((module("json/__init__.py"), package("json/__init__.py")), ("json".to_owned(), "json".to_owned()));
    assert_eq!((module("json/tool.py"), package("json/tool.py")), ("json.tool".to_owned(), "json".to_owned()));
    assert_eq!((module("shlex.py"), package("shlex.py")), ("shlex".to_owned(), String::new()));
    assert_eq!(module("x/src/a/mod.rs"), "crate.a");
    assert_eq!(module("src/lib.rs"), "crate");
}
