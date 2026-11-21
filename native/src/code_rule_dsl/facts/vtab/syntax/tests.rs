use super::*;
use super::candidates::PythonCandidates as _;
use crate::code_rule_dsl::facts::core::{self, Built};
use crate::code_rule_dsl::language::{GrammarDialect, dialect_by_label, tape_tag};
use crate::code_rule_dsl::model::FileInput;
use crate::content_store::Binding;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl")
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
/// The corpus as a batch, document `d` keyed `d`.
pub(in crate::code_rule_dsl) fn documents() -> Vec<Document> {
    corpus()
        .into_iter()
        .enumerate()
        .map(|(d, (path, source))| {
            let facts = facts(&path, &source);
            Document { key: d as u32, core: format!("/w#core:{path}"), digest: core::digest(&facts), path, facts, tape: None, guest: None, module: None, unpackaged: false }
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
    ("op_t", &[Ty::Node, Ty::Sym]),
    ("tape", &[Ty::Node, Ty::Node, Ty::Int, Ty::Int]),
];

/// Every relation holds T's exporter's rows for the same files: node ids map to T's global ids by each document's
/// first id, and atoms decode to T's strings.
#[test]
fn relations_hold_t_exporters_rows() {
    let store = ContentStore::open(&temp("store")).unwrap();
    let db = Connection::open_in_memory().unwrap();
    let modules = Modules::default();
    // T numbers nodes in preorder: an id's low bits map back to its local id (they differ per definition).
    let projected: HashMap<i64, Projected> = documents()
        .into_iter()
        .map(|d| {
            let dialect = Dialect::new(core::facts_tag(&d.facts)).unwrap();
            (i64::from(d.key), Projected::new(d.key, Arc::clone(&d.facts), Some(&dialect), Shape::T, false, None))
        })
        .collect();
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
                        Ty::Node => (first[&(v >> 32)] + i64::from(projected[&(v >> 32)].local_of((v & 0xffff_ffff) as u32).unwrap())).to_string(),
                        Ty::Kind => {
                            assert_eq!(crate::code_rule_dsl::facts::core::kind_tag(v as u32), tag);
                            catalog.kind_name(crate::code_rule_dsl::facts::core::kind_code(v as u32)).unwrap().to_owned()
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
        // T's `op` is `op_t` here (O79): `op` is `core.rerail`'s, every anonymous token.
        let t_rel = if rel == "op_t" { "op" } else { rel };
        let mut stmt = t.prepare(&format!("SELECT * FROM {t_rel}")).unwrap();
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
    // `+` on every column reference hides the constraint from xBestIndex: SQLite scans and filters.
    let column = regex::Regex::new(r"\b(n|p|c|d|kind|par|idx) ").unwrap();
    for (rel, filter) in [
        ("node", format!("n = {n}")),
        ("node", "d = 2".to_owned()),
        ("node", format!("kind = {call}")),
        ("node", format!("n BETWEEN {} AND {}", n, n + 30)),
        ("node", format!("n > {} AND n < {}", (1i64 << 32) - 5, (1i64 << 32) + 3)),
        ("child", format!("p = {n}")),
        ("child", format!("c IN ({n}, {}, 0, 7)", n + 1)),
        ("child", format!("p = {n} AND idx = 1")),
        ("child", "idx = 0".to_owned()),
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
        let hidden = column.replace_all(&filter, "+$1 ");
        let scanned = key(&format!("SELECT * FROM {rel} WHERE {hidden}"));
        assert!(!indexed.is_empty() || filter.contains("0, 7"), "{rel} {filter}: no rows");
        assert_eq!(indexed, scanned, "{rel} {filter}");
    }
}

/// A view's statistics (`measured`, read through its walk) are those its per-row reads give: rows, ndv, sq and sqd
/// of every column of `node`, `child`, `field` and `tape` over several documents. The measure of the cross part's
/// base relations (decision 0458) had read each value by its row, locating it in the batch: 0.9 s of a hogql run.
#[test]
fn a_views_measured_statistics_are_its_per_row_reads() {
    let store = ContentStore::open(&temp("measured")).unwrap();
    for (name, source) in sources(&store, documents().into_iter().take(4).collect()).unwrap() {
        if !["node", "child", "field", "tape"].contains(&name) {
            continue;
        }
        let n = source.len();
        let per_row: Vec<(String, i64, i64, i64, i64)> = source
            .columns()
            .into_iter()
            .enumerate()
            .map(|(c, col)| {
                let mut pairs: Vec<(i64, i64)> = (0..n).map(|r| (source.value(r, c), source.value(r, 0) >> 32)).collect();
                pairs.sort_unstable();
                let mut values: BTreeMap<i64, BTreeMap<i64, i64>> = BTreeMap::new();
                for (v, d) in pairs {
                    *values.entry(v).or_default().entry(d).or_default() += 1;
                }
                let sq = values.values().map(|d| d.values().sum::<i64>().pow(2)).sum();
                let sqd = values.values().flat_map(|d| d.values()).map(|k| k * k).sum();
                (col, n as i64, values.len() as i64, sq, sqd)
            })
            .collect();
        assert_eq!(super::measured(source.as_ref()), per_row, "{name}");
        assert_eq!(super::measured_sorted(source.as_ref()), per_row, "{name}: sorted");
        assert!(per_row.iter().any(|m| m.2 < m.1), "{name}: a column with repeated values, counted");
    }
}

/// A view's walk of the rows whose own column is in a set (`for_rows_keyed`: a semi-join's rows at its exports'
/// nodes, decision 0458) gives the rows, in row order, and the values that reading those rows one at a time gives:
/// for every view that has it, at every third row's key, and a stop where `each` says.
#[test]
fn a_views_keyed_walk_gives_the_rows_a_per_row_read_finds() {
    let store = ContentStore::open(&temp("keyed-walk")).unwrap();
    let mut tried = 0;
    // Each view, and the view as a chunk binds it (`Layered`, no value admitted): the same walk.
    let views = sources(&store, documents().into_iter().take(4).collect()).unwrap();
    let layered = views.iter().map(|(name, s)| (format!("{name} layered"), Arc::new(super::values::Layered::new(name, Arc::clone(s), Default::default())) as Arc<dyn Source>));
    for (name, source) in views.iter().map(|(n, s)| (n.to_string(), Arc::clone(s))).chain(layered).collect::<Vec<_>>() {
        let n = source.len();
        let all: Vec<usize> = (0..source.columns().len()).collect();
        for c in 0..all.len() {
            let keys: rustc_hash::FxHashSet<i64> = (0..n).step_by(3).map(|r| source.value(r, c)).collect();
            let mut walked: Vec<(usize, Vec<i64>)> = Vec::new();
            if !source.for_rows_keyed(c, &keys, &all, &mut |r, row| {
                walked.push((r, row.to_vec()));
                true
            }) {
                continue;
            }
            let per_row: Vec<(usize, Vec<i64>)> = (0..n).filter(|&r| keys.contains(&source.value(r, c))).map(|r| (r, all.iter().map(|&k| source.value(r, k)).collect())).collect();
            assert_eq!(walked, per_row, "{name} column {c}");
            let mut stopped = 0;
            source.for_rows_keyed(c, &keys, &all, &mut |_, _| {
                stopped += 1;
                stopped < 2
            });
            assert_eq!(stopped, 2.min(per_row.len()), "{name} column {c}: a stop");
            tried += 1;
        }
    }
    assert!(tried >= 8, "{tried} views walked by key");
}

/// A view's filter scan, document by document, holds the rows a per-row read finds: over several documents, for
/// filters on each column of `node`, `child`, `field` and `tape`, and on two at once. It is the view's own scan (a
/// merge had left it outside the `Source` impl, where the trait's per-row scan shadowed it).
#[test]
fn a_views_filter_scan_holds_the_rows_a_per_row_read_finds() {
    let store = ContentStore::open(&temp("filter-eq")).unwrap();
    let python: GrammarDialect = dialect_by_label("python").unwrap();
    let call = core::kind_atom(tape_tag(python).unwrap(), python.kinds().kind_id("call"));
    let mut tried = 0;
    for (name, source) in sources(&store, documents().into_iter().take(4).collect()).unwrap() {
        if !["node", "child", "field", "tape"].contains(&name) {
            continue;
        }
        let width = source.columns().len();
        let n = source.len();
        assert!(n > 0, "{name}: no rows");
        // The view's row passes (`for_rows`, `rows_of`) give the rows a per-row read gives: every column, and the
        // last alone (no parent read, so no walk).
        for cols in [(0..width).collect::<Vec<_>>(), vec![width - 1]] {
            let per_row: Vec<i64> = (0..n).flat_map(|r| cols.iter().map(move |&c| (r, c))).map(|(r, c)| source.value(r, c)).collect();
            let mut passed = Vec::new();
            source.for_rows(&cols, &mut |r, row| {
                assert_eq!(r, passed.len() / cols.len(), "{name}: rows in order");
                passed.extend_from_slice(row);
            });
            assert_eq!(passed, per_row, "{name} {cols:?}: for_rows");
            let mut all = Vec::new();
            source.rows_of(&cols, &mut all);
            assert_eq!(all, per_row, "{name} {cols:?}: rows_of");
        }
        // Each row's document (`documents_of`) is its column's value's high bits, for every column.
        for c in 0..width {
            let per_row: Vec<(usize, i64)> = (0..n).map(|r| (r, source.value(r, c))).map(|(r, v)| (r, if v >= 1 << 32 { v >> 32 } else { v })).collect();
            let mut docs = Vec::new();
            source.documents_of(c, &mut |r, d| docs.push((r, d)));
            assert_eq!(docs, per_row, "{name} column {c}: documents_of");
        }
        for (c, eqs) in (0..width).flat_map(|c| {
            let (a, b) = (source.value(n / 3, c), source.value(n - 1, c));
            let pair = if width > 1 { vec![(c, a), ((c + 1) % width, source.value(n / 3, (c + 1) % width))] } else { vec![(c, a)] };
            [(c, vec![(c, a)]), (c, vec![(c, b)]), (c, pair)]
        }).chain([(2, vec![(2, i64::from(call))])].into_iter().filter(|_| name == "node")) {
            let per_row: Vec<u32> = (0..n).filter(|&r| eqs.iter().all(|&(c, v)| source.value(r, c) == v)).map(|r| r as u32).collect();
            assert!(!per_row.is_empty(), "{name} column {c}: a value read from a row matches it");
            assert_eq!(source.filter_eq(&eqs), per_row, "{name} {eqs:?}");
            tried += 1;
        }
    }
    assert!(tried >= 4 * 3, "the four views were tried");
}

/// Q220: a lookup of `node` by kind (`lookup_read`, a join's `kind($c, #k)` read where the view lies) gives every row
/// of that kind, each with the values a per-row read gives, from the batch's index by kind (`Batch::by_kind`), for
/// every kind of four documents, common and rare; and `lookup` gives the same rows.
#[test]
fn a_kind_lookup_reads_the_rows_a_per_row_read_finds() {
    let store = ContentStore::open(&temp("kind-read")).unwrap();
    let (_, node) = sources(&store, documents().into_iter().take(4).collect()).unwrap().into_iter().find(|(name, _)| *name == "node").unwrap();
    let (n, width) = (node.len(), node.columns().len());
    let cols: Vec<usize> = (0..width).collect();
    let mut kinds: BTreeMap<i64, usize> = BTreeMap::new();
    for r in 0..n {
        *kinds.entry(node.value(r, 2)).or_default() += 1;
    }
    assert!(kinds.values().any(|&k| k * 16 >= n) && kinds.values().any(|&k| k == 1), "the documents have common kinds and single ones");
    for &kind in kinds.keys() {
        let per_row: BTreeSet<(usize, Vec<i64>)> = (0..n).filter(|&r| node.value(r, 2) == kind).map(|r| (r, cols.iter().map(|&c| node.value(r, c)).collect())).collect();
        let mut read = BTreeSet::new();
        let mut rows = 0;
        assert!(node.lookup_read(2, kind, &cols, &mut |r, vals| {
            rows += 1;
            read.insert((r, vals.to_vec()));
        }), "kind {kind}: the view reads its lookup");
        assert_eq!(rows, per_row.len(), "kind {kind}: each row once");
        assert_eq!(read, per_row, "kind {kind}");
        let found: Vec<usize> = node.lookup(2, &[kind]).expect("the view looks up by kind");
        assert_eq!(found, per_row.iter().map(|(r, _)| *r).collect::<Vec<_>>(), "kind {kind}: lookup's rows, in order");
    }
}

/// The statistics of a relation held as a view of `node` (decision 0329) are its rows' counts, made without reading
/// them: per column, the distinct values, the squares of their counts and each value's count, as counting every row
/// gives (`n`, the key: each value once, no counts).
#[test]
fn a_node_columns_counts_are_its_rows_counts() {
    let store = ContentStore::open(&temp("node-counts")).unwrap();
    let (_, node) = sources(&store, documents().into_iter().take(4).collect()).unwrap().into_iter().find(|(name, _)| *name == "node").unwrap();
    let batch = node.python_batch().expect("a batch's view");
    assert!(node.len() > 50, "the documents have nodes");
    for column in 0..3 {
        let mut by: BTreeMap<i64, i64> = BTreeMap::new();
        for r in 0..node.len() {
            *by.entry(node.value(r, column)).or_default() += 1;
        }
        let counts: Vec<(i64, i64)> = by.into_iter().collect();
        let (ndv, sq, got) = node_column_counts(&batch, column).expect("node's column");
        assert_eq!(ndv, counts.len() as i64, "column {column}'s distinct values");
        assert_eq!(sq, counts.iter().map(|&(_, n)| n * n).sum::<i64>(), "column {column}'s squares");
        if column == 1 {
            assert!(counts.iter().all(|&(_, n)| n == 1) && got.is_none(), "n is node's key");
        } else {
            assert_eq!(got, Some(counts), "column {column}'s counts");
        }
    }
    assert!(node_column_counts(&batch, 3).is_none());
}

/// A field's children by parent and field (`lookup_eq`, a join's `field($a, #f, $t)` with `$a` bound) are the rows a
/// lookup by parent finds and a filter on the field keeps, in the same order: for every parent and each of its
/// fields, and for a field it lacks.
#[test]
fn a_fields_lookup_by_parent_and_field_holds_what_a_filter_keeps() {
    let store = ContentStore::open(&temp("lookup-eq")).unwrap();
    let (_, field) = sources(&store, documents().into_iter().take(4).collect()).unwrap().into_iter().find(|(name, _)| *name == "field").unwrap();
    let parents: BTreeSet<i64> = (0..field.len()).map(|r| field.value(r, 0)).collect();
    let fields: BTreeSet<i64> = (0..field.len()).map(|r| field.value(r, 1)).collect();
    assert!(parents.len() > 10 && fields.len() > 3, "the documents have fields");
    let mut found = 0;
    for &p in &parents {
        for f in fields.iter().copied().take(6).chain([-7]) {
            let filtered: Vec<usize> = field.lookup(0, &[p]).unwrap().into_iter().filter(|&r| field.value(r, 1) == f).collect();
            assert_eq!(field.lookup_eq(0, p, &[(1, f)]), Some(filtered.clone()), "parent {p}, field {f}");
            found += filtered.len();
        }
    }
    assert!(found > 0, "some parent holds one of the fields tried");
    // By child and field: the child's one row where its field is the one asked, for every child, each field and one
    // no row has, and for a node of no document.
    let children: BTreeSet<i64> = (0..field.len()).map(|r| field.value(r, 2)).collect();
    let mut kept = 0;
    for &c in &children {
        for f in fields.iter().copied().take(6).chain([-7]) {
            let filtered: Vec<usize> = field.lookup(2, &[c]).unwrap().into_iter().filter(|&r| field.value(r, 1) == f).collect();
            assert_eq!(field.lookup_eq(2, c, &[(1, f)]), Some(filtered.clone()), "child {c}, field {f}");
            kept += filtered.len();
        }
    }
    assert!(kept > 0, "some child is one of the fields tried");
    assert_eq!(field.lookup_eq(2, i64::MAX, &[(1, 1)]), Some(Vec::new()));
    // Any other probe is the lookup and filter's.
    assert_eq!(field.lookup_eq(1, 1, &[(2, 1)]), None);
    assert_eq!(field.lookup_eq(0, 1, &[(2, 1)]), None);
}

/// Two cursors on one table, joined on a column the inner one does not index, pair children by position.
#[test]
fn self_joins_pair_children_by_position() {
    let store = ContentStore::open(&temp("self-join")).unwrap();
    let db = Connection::open_in_memory().unwrap();
    let modules = Modules::default();
    for (name, source) in sources(&store, documents().into_iter().take(1).collect()).unwrap() {
        modules.bind(&db, name, source).unwrap();
    }
    let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    let pairs = "FROM child a CROSS JOIN child b WHERE a.p < b.p AND b.p < a.p + 40 AND a.idx = b.idx";
    let joined = count(&format!("SELECT count(*) {pairs}"));
    assert!(joined > 0);
    assert_eq!(joined, count(&format!("SELECT count(*) {}", pairs.replace("a.idx = b.idx", "+a.idx = +b.idx"))));
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
    // Stubs are named as their modules.
    assert_eq!((module("os/__init__.pyi"), package("os/__init__.pyi")), ("os".to_owned(), "os".to_owned()));
    assert_eq!((module("os/path.pyi"), package("os/path.pyi")), ("os.path".to_owned(), "os".to_owned()));
    assert_eq!(module("x/src/a/mod.rs"), "crate.a");
    assert_eq!(module("src/lib.rs"), "crate");
}

/// One document of `source` in `label`'s dialect, keyed `key`, with its tape (W's values read it).
fn document(key: u32, path: &str, label: &str, source: &str) -> Document {
    let file = FileInput::new(path.to_owned(), label.to_owned(), Arc::new(source.to_owned()), None, None, "core").unwrap();
    let (core::Built::Facts(facts), Some(tape)) = core::build_with_tape(&file, true).unwrap() else { panic!("{path} did not parse") };
    let facts = Arc::new(facts);
    Document { key, core: format!("/w#core:{path}"), digest: core::digest(&facts), path: path.to_owned(), facts, tape: Some((tape, Arc::new(file))), guest: None, module: None, unpackaged: false }
}

/// `core.rerail`'s relations of `docs` for a program that includes `core.rerail` and reads them (`text_demand::projection_of`'s shape
/// and reads), each row as text: a node as `kind'text'`, a span as `'text'`, a symbol as itself. The relations made
/// for what a program reads are the batch's (`Extra`'s W's values, source lines and places, `tree`'s `op_at` and
/// `visit_index`), the others `core_rr`'s.
fn core_rows(docs: Vec<Document>) -> HashMap<String, BTreeSet<String>> {
    let store = ContentStore::open(&temp("core")).unwrap();
    let reads = [
        "node", "text", "comment", "op", "op_at", "str", "strprefix", "value", "value_type", "name", "spread", "next_sibling", "syntax_error",
        "src_line", "line_count", "str_part", "first_token", "last_token", "token_before", "token_after", "child", "line", "end_line", "col",
        "visit_index", "ast_depth", "structural_before", "structural_after",
    ];
    let projection = Projection { shape: Shape::Core, reads: Some(reads.iter().map(|r| (*r).to_owned()).collect()), ..Projection::core() };
    let sources = shaped(&store, docs, &projection).unwrap();
    let source = |name: &str| Arc::clone(&sources.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("no {name}")).1);
    let read = |name: &str| -> Vec<Vec<i64>> {
        let s = source(name);
        (0..s.len()).map(|r| (0..s.columns().len()).map(|c| s.value(r, c)).collect()).collect()
    };
    let sym = |v: i64| store.atom_value(v as u32).unwrap().unwrap();
    let texts: HashMap<i64, String> = read("text").into_iter().map(|r| (r[0], sym(r[1]))).collect();
    let kinds: HashMap<i64, String> = read("node")
        .into_iter()
        .map(|r| {
            let (tag, code) = (crate::code_rule_dsl::facts::core::kind_tag(r[2] as u32), crate::code_rule_dsl::facts::core::kind_code(r[2] as u32));
            (r[1], from_tape_tag(tag).unwrap().kinds().kind_name(code).unwrap_or("ERROR").to_owned())
        })
        .collect();
    let loc = |v: i64| match (kinds.get(&v), texts.get(&v)) {
        (Some(k), Some(t)) if !t.contains('\n') => format!("{k}'{t}'"),
        (Some(k), _) => k.clone(),
        (None, Some(t)) => format!("'{t}'"),
        (None, None) => format!("#{v}"),
    };
    // Per relation, which columns are locations (`l`), symbols (`s`) and numbers (`i`).
    let shapes = [
        ("comment", "ls"),
        ("op", "ls"),
        ("structural_before", "ls"),
        ("structural_after", "ls"),
        ("op_at", "lis"),
        ("str", "ls"),
        ("strprefix", "ls"),
        ("value", "ls"),
        ("value_type", "ss"),
        ("name", "ls"),
        ("spread", "l"),
        ("next_sibling", "ll"),
        ("syntax_error", "iis"),
        ("src_line", "iil"),
        ("line_count", "ii"),
        ("str_part", "lisli"),
        ("first_token", "ll"),
        ("last_token", "ll"),
        ("token_before", "ll"),
        ("token_after", "ll"),
        ("child", "lli"),
        ("line", "li"),
        ("end_line", "li"),
        ("col", "li"),
        ("visit_index", "li"),
        ("ast_depth", "li"),
    ];
    shapes
        .iter()
        .map(|(name, shape)| {
            let rows = read(name)
                .into_iter()
                .map(|r| {
                    r.iter()
                        .zip(shape.chars())
                        .map(|(&v, c)| match c {
                            'l' => loc(v),
                            's' => sym(v),
                            _ => v.to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect();
            ((*name).to_owned(), rows)
        })
        .collect()
}

/// `core.rerail`'s base relations over a Python file (decision 0046): comments are nodes and children, `text` is every
/// node's, `op` every token the tape stores (one the grammar implies is in no tape and no relation, RER-131), literals decode, and
/// the source-text, token and place relations agree.
#[test]
fn core_rr_relations_hold_what_core_rr_says() {
    let src = "# top\nimport os.path as p\ndef f(a, b):  # tail\n    \"\"\"d\\x41\"\"\"\n    x = a not in b\n    g(*a, f\"v{a}w\", 0xff)\n    return x\n";
    let rows = core_rows(vec![document(1, "m.py", "python", src), document(2, "bad.py", "python", "x = (\n")]);
    let has = |rel: &str, row: &str| assert!(rows[rel].contains(row), "{rel} has no {row:?}: {:?}", rows[rel]);
    let lacks = |rel: &str, row: &str| assert!(!rows[rel].contains(row), "{rel} has {row:?}");
    has("comment", "comment'# top' top");
    has("comment", "comment'# tail' tail");
    has("child", "module comment'# top' 0");
    has("next_sibling", "comment'# top' import_statement'import os.path as p'");
    has("op", "comparison_operator'a not in b' not in");
    has("op_at", "comparison_operator'a not in b' 0 not in");
    // `def` is kept (`async def` holds it beside a choice); the parentheses and the separators are
    // delimiters the grammar implies: no tape holds them, and no relation.
    has("op", "function_definition def");
    lacks("op", "argument_list'(*a, f\"v{a}w\", 0xff)' ,");
    // The source around a node, between it and its stored neighbours: the separators the tape
    // leaves out, and the parent's closing bracket after its last child.
    has("structural_before", "list_splat'*a' (");
    has("structural_before", "string'f\"v{a}w\"' , ");
    has("structural_before", "integer'0xff' , ");
    has("structural_after", "integer'0xff' )");
    has("str", "string'\"\"\"d\\x41\"\"\"' dA");
    has("value", "string'\"\"\"d\\x41\"\"\"' dA");
    has("strprefix", "string'f\"v{a}w\"' f");
    assert!(!rows["str"].iter().any(|r| r.starts_with("string'f")), "an interpolated literal has no str");
    has("value", "integer'0xff' 255");
    has("value_type", "255 int");
    has("name", "dotted_name'os.path' os.path");
    has("spread", "list_splat'*a'");
    has("str_part", "string'f\"v{a}w\"' 0 v string'f\"v{a}w\"' 0");
    has("str_part", "string'f\"v{a}w\"' 1 w string'f\"v{a}w\"' 0");
    assert!(rows["syntax_error"].iter().any(|r| r.starts_with("2 1 ")), "{:?}", rows["syntax_error"]);
    assert!(!rows["syntax_error"].iter().any(|r| r.starts_with("1 ")));
    has("line_count", "1 7");
    has("src_line", "1 3 'def f(a, b):  # tail'");
    has("line", "comparison_operator'a not in b' 5");
    has("col", "comparison_operator'a not in b' 8");
    has("end_line", "block 7");
    has("visit_index", "module 0");
    has("ast_depth", "module 0");
    // `def` is stored (kept); `return` and the parentheses are left out, so no token holds them.
    has("first_token", "function_definition 'def'");
    has("token_before", "identifier'f' 'def'");
    has("last_token", "return_statement'return x' identifier'x'");
    lacks("first_token", "return_statement'return x' 'return'");
    lacks("token_after", "identifier'f' '('");
}

/// The shape follows the program: one that includes `core.rerail` reads `core.rerail`'s, one that declares its base
/// relations itself (T's, the converter's) T's.
#[test]
fn programs_that_include_core_rr_read_its_shape() {
    let core = std::fs::read_to_string(crate::code_rule_dsl::logic::rr::library().join("core.rerail")).unwrap();
    let with = crate::code_rule_dsl::logic::rr::parse_version(&format!("{core}\n.decl q(n: node)\nq(n) :- comment(n, _).\n.output q\n")).unwrap();
    assert_eq!(crate::code_rule_dsl::logic::plan::text_demand::projection_of(&with, BTreeSet::new()).shape, Shape::Core);
    let without = crate::code_rule_dsl::logic::rr::parse_version(".decl text(n: number, s: symbol)\n.input text\n.decl q(n: number)\nq(n) :- text(n, _).\n.output q\n").unwrap();
    assert_eq!(crate::code_rule_dsl::logic::plan::text_demand::projection_of(&without, BTreeSet::new()).shape, Shape::T);
}

/// End to end through the corpus runner, a program that includes `core.rerail`: its options, `@dot` with an empty head,
/// `@lower` in Unicode, and comments as `core.rerail`'s nodes.
#[test]
fn core_rr_programs_read_options_and_functors() {
    use crate::code_rule_dsl::logic::plan::corpus;
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("core-rr");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "# hello\nx = 1\n").unwrap();
    let core = std::fs::read_to_string(crate::code_rule_dsl::logic::rr::library().join("core.rerail")).unwrap();
    let program = format!(
        r#"{core}
.decl part(a: symbol, b: symbol)
part("", "b"). part("a", "b").
.decl dotted(s: symbol)
dotted(s) :- part(a, b), s = @dot(a, b).
.decl low(s: symbol)
low(s) :- option("name", v), s = @lower(v).
.decl item(i: number, v: symbol)
item(i, v) :- option_item("list", i, v).
.decl said(s: symbol)
said(s) :- comment(_, s).
.output dotted, low, item, said
"#
    );
    let out = dir.join("out");
    let options = serde_json::json!({"name": "ÀB", "list": ["x", "y"]});
    corpus::run_with_options(&[program.as_str()], &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
    assert_eq!(read("dotted"), [["a.b"], ["b"]]);
    assert_eq!(read("low"), [["àb"]]);
    assert_eq!(read("item"), [["0", "x"], ["1", "y"]]);
    assert_eq!(read("said"), [["hello"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `within(n, a)` with both given is a test (t50 step 4, `Source::holds`): it holds exactly where `a` is on `n`'s
/// chain of parents, and makes no row. `lookup_all` (the composite probe that reads the row) agrees.
#[test]
fn within_with_both_given_is_an_interval_test() {
    let store = ContentStore::open(&temp("within-holds")).unwrap();
    let docs: Vec<Document> = documents().into_iter().take(3).collect();
    let (batch, _, _, _) = Batch::build(&store, docs, &Projection::default(), &[], &[], false).unwrap();
    let nodes = *batch.nodes().last().unwrap();
    let place = |r: usize| {
        let doc = batch.nodes().partition_point(|&o| o <= r) - 1;
        (doc, r - batch.nodes()[doc])
    };
    use crate::code_rule_dsl::facts::vtab::Source;
    // A sample of nodes against every node (8,398 nodes: every pair is 70 M tests).
    {
        let within = within::Within::new(Arc::clone(&batch), within::Mode::Tree);
        let format = "held";
        assert!(within.composite(), "both columns at once");
        let stride = 31;
        let (mut held, mut pairs) = (0usize, 0usize);
        for n in (0..nodes).step_by(stride) {
            for a in 0..nodes {
                let ((dn, ln), (da, la)) = (place(n), place(a));
                let p = &batch.docs[dn];
                let mut up = p.parent(ln);
                while up != NONE && up as usize != la {
                    up = p.parent(up as usize);
                }
                let want = dn == da && up != NONE;
                let eq = [(0, batch.id(n)), (1, batch.id(a))];
                assert_eq!(within.holds(&eq), Some(want), "{format:?}: node rows {n} below {a}");
                held += usize::from(want);
                pairs += 1;
            }
        }
        assert!(held > nodes / stride && held < pairs, "{format:?}: some pairs hold, not all ({held} of {pairs})");
        assert!(!within.is_solved() && within.made_rows() == 0, "{format:?}: the test made no row");
        // The composite probe reads the pair's row: one where the test holds.
        for (n, a) in [(nodes - 1, 0), (1, 0), (0, 1)] {
            let eq = [(0, batch.id(n)), (1, batch.id(a))];
            assert_eq!(within.lookup_all(&eq).unwrap().len(), usize::from(within.holds(&eq).unwrap()), "{format:?}: {n} below {a}");
        }
    }
}

/// `within` and `span_within` (decision 0050) hold exactly the pairs that comparing every pair of one document's
/// nodes gives (preorder intervals for the tree, spans for the other): looked up by either column, and scanned.
/// The documents with errors and empty bodies bring nodes of zero width.
#[test]
fn within_holds_exactly_the_contained_pairs() {
    let store = ContentStore::open(&temp("within")).unwrap();
    let mut docs: Vec<Document> = documents().into_iter().take(3).collect();
    for (at, source) in ["x = (1,\nif y:\n", "def f(a, b:\n    return [a for\n", "", "class C:\n"].into_iter().enumerate() {
        let path = format!("broken{at}.py");
        let facts = facts(&path, source);
        docs.push(Document { key: (10 + at) as u32, core: format!("/w#core:{path}"), digest: core::digest(&facts), path, facts, tape: None, guest: None, module: None, unpackaged: false });
    }
    let (batch, _, _, _) = Batch::build(&store, docs, &Projection::default(), &[], &[], false).unwrap();
    let nodes = *batch.nodes().last().unwrap();
    let id = |r: usize| batch.id(r);
    // Borrow the demanded columns once, as a bulk consumer does. Keep lazy initialization
    // out of the exhaustive reference loop; that loop checks the relation, not setup cost.
    let axes: Vec<Vec<u32>> = batch.docs.iter().map(|p| p.places().map(|(_, _, d)| d).collect()).collect();
    // (document, local, size, depth, start, end) of node row r.
    // Read once: the reference loop below visits every pair.
    let rows: Vec<_> = (0..nodes)
        .map(|r| {
            let doc = batch.nodes().partition_point(|&o| o <= r) - 1;
            let (p, local) = (&batch.docs[doc], r - batch.nodes()[doc]);
            let (start, end) = p.facts.span(p.node(local as u32));
            (p.key, local, p.size(local) as usize, axes[doc][local] as usize, start, end)
        })
        .collect();
    let at = |r: usize| rows[r];
    let zero = (0..nodes).filter(|&r| at(r).4 == at(r).5).count();
    assert!(zero > 0, "the broken documents have nodes of zero width");
    let cpu = || { let (u, s) = crate::code_rule_dsl::prof::thread_usage(); u + s };
    let (mut reference_cpu, mut relation_cpu) = (0.0, 0.0);
    // The tree's containment reads the documents' tapes (decision 0293), in every format; spans read the facts.
    use crate::code_rule_dsl::tape::formats::Format;
    assert!(batch.docs.iter().all(|p| p.facts.tape.is_some()), "every document holds its tape");
    for mode in [within::Mode::Tree, within::Mode::Span] {
        let before_reference = cpu();
        let mut expected = BTreeSet::new();
        for a in 0..nodes {
            for b in 0..nodes {
                let ((da, la, _, _, sa, ea), (db, lb, zb, _, sb, eb)) = (at(a), at(b));
                let holds = da == db
                    && match mode {
                        within::Mode::Tree => lb < la && la <= lb + zb,
                        within::Mode::Span => sb <= sa && ea <= eb,
                        within::Mode::Axis(_) => unreachable!("the axes have their own test"),
                    };
                if holds {
                    expected.insert((id(a), id(b)));
                }
            }
        }
        reference_cpu += cpu() - before_reference;
        let formats: &[Format] = if mode == within::Mode::Tree { &Format::ALL } else { &[Format::F15] };
        for &format in formats {
            let within = within::Within::new(Arc::clone(&batch), mode).reading(format);
            let before_relation = cpu();
            let rows = |rows: Vec<usize>| -> BTreeSet<(i64, i64)> { rows.into_iter().map(|r| (within.value(r, 0), within.value(r, 1))).collect() };
            let ids: Vec<i64> = (0..nodes).map(id).collect();
            // By outer first (half the nodes), then by inner, then the scan: rows made by one lookup serve the others.
            let half: BTreeSet<i64> = ids[..nodes / 2].iter().copied().collect();
            assert_eq!(rows(within.lookup(1, &ids[..nodes / 2]).unwrap()), expected.iter().filter(|(_, b)| half.contains(b)).copied().collect());
            assert_eq!(rows(within.lookup(0, &ids).unwrap()), expected);
            assert_eq!(rows(within.lookup(1, &ids).unwrap()), expected);
            assert_eq!(rows((0..within.len()).collect()), expected);
            // Each pair's documents (`documents_of`) are its ids' high bits.
            for c in 0..2 {
                let mut docs = Vec::new();
                within.documents_of(c, &mut |r, d| docs.push((r, d)));
                let read: Vec<(usize, i64)> = (0..within.len()).map(|r| (r, within.value(r, c))).map(|(r, v)| (r, if v >= 1 << 32 { v >> 32 } else { v })).collect();
                assert_eq!(docs, read, "column {c}: documents_of");
            }
            // Each node's containers are made once: the inner side's rows are exactly the pairs.
            assert_eq!(within.lookup(0, &ids).unwrap().len(), expected.len());
            let own = usize::from(mode == within::Mode::Span);
            assert_eq!(within.estimated_len(), (0..nodes).map(|r| at(r).3 + own).sum::<usize>());
            if mode == within::Mode::Tree {
                assert_eq!(within.len(), expected.len(), "the tree's rows are its estimate");
            }
            // Read whole first (the device's keyed read, decision 0277): the tree by the batch's solve, every pair once,
            // and the lookups after it read the same rows; spans have no solve.
            let whole = within::Within::new(Arc::clone(&batch), mode).reading(format);
            assert_eq!(whole.complete_rows(), mode == within::Mode::Tree, "containment is solved for the batch, spans are not");
            if mode == within::Mode::Tree {
                let rows = |rows: Vec<usize>| -> BTreeSet<(i64, i64)> { rows.into_iter().map(|r| (whole.value(r, 0), whole.value(r, 1))).collect() };
                assert_eq!(whole.len(), expected.len());
                assert_eq!(rows((0..whole.len()).collect()), expected);
                assert_eq!(rows(whole.lookup(0, &ids).unwrap()), expected);
                assert_eq!(rows(whole.lookup(1, &ids).unwrap()), expected);
            }
            relation_cpu += cpu() - before_relation;
        }
    }
    eprintln!("CONTAINMENT CHECK: {nodes} nodes, reference {reference_cpu:.6} CPU s, relation and comparisons {relation_cpu:.6} CPU s");
}

/// The kind-set axes (`within::Axis`: `nearest`, `inside`, `has`) hold exactly the pairs a brute-force walk of each
/// node's strict ancestors gives, looked up by either column and scanned, and their statistics count the rows.
#[test]
fn the_kind_set_axes_hold_exactly_their_pairs() {
    use crate::code_rule_dsl::facts::core::kind_atom;
    let store = ContentStore::open(&temp("axes")).unwrap();
    let mut docs: Vec<Document> = documents().into_iter().take(4).collect();
    // A document with ERROR nodes: an ERROR node's kind is ERROR's atom (`Facts::kind`, slot 0), whatever kind its tape
    // entry holds: a member of a set that names ERROR, on the host, the device and the tape's code set alike (0289).
    let source = "def f(:\n    return [1, 2\nclass C:\n    x = (\ny = f(1) + g(2)\n";
    let broken = facts("broken.py", source);
    assert!(broken.errors().iter().any(|&id| broken.flags(id) & 8 != 0), "broken.py has ERROR nodes");
    docs.push(Document { key: 4, core: "/w#core:broken.py".into(), digest: core::digest(&broken), path: "broken.py".into(), facts: broken, tape: None, guest: None, module: None, unpackaged: false });
    let (batch, _, _, _) = Batch::build(&store, docs, &Projection::default(), &[], &[], false).unwrap();
    let nodes = *batch.nodes().last().unwrap();
    let id = |r: usize| batch.id(r);
    // (document, local, size) and kind atom of node row r.
    let at = |r: usize| {
        let (p, local) = batch.locate(View::Node, r);
        (p.key, local, p.size(local) as usize)
    };
    let kind = |r: usize| {
        let (p, local) = batch.locate(View::Node, r);
        let code = crate::code_rule_dsl::facts::core::kind_code(p.facts.kind(p.node(local as u32)));
        i64::from(kind_atom(p.tag, batch.dialects[p.tag].canonical.get(usize::from(code)).copied().unwrap_or(code)))
    };
    // Nested kinds: the root's, and two from inside the documents; and every kind of broken.py's nodes, ERROR's
    // among them.
    let mut set: Vec<i64> = [0, nodes / 3, nodes / 2].map(kind).to_vec();
    set.extend((batch.nodes()[4]..nodes).map(kind));
    set.sort_unstable();
    set.dedup();
    // Each row's place and membership read once: the brute force below is quadratic in the rows.
    let place: Vec<(u32, usize, usize)> = (0..nodes).map(at).collect();
    // An ERROR node (its atom's slot 0) is a member, since the set names ERROR (`Within::member`).
    let error = |k: i64| k % i64::from(crate::code_rule_dsl::facts::core::KIND_STRIDE) == 0;
    assert!(set.iter().any(|&k| error(k)), "the set names ERROR");
    let member: Vec<bool> = (0..nodes).map(&kind).map(|k| set.contains(&k)).collect();
    assert!((0..nodes).any(|r| error(kind(r)) && member[r]), "an ERROR node is a member");
    let below = |n: usize, a: usize| {
        let ((dn, ln, _), (da, la, za)) = (place[n], place[a]);
        dn == da && la < ln && ln <= la + za
    };
    // Each axis's pairs by brute force, once: a node's nearest member ancestor is its deepest (the latest in
    // preorder of the ancestors).
    let expected_of = |axis: within::Axis| -> BTreeSet<(i64, i64)> {
        let mut expected = BTreeSet::new();
        for n in 0..nodes {
            let mut nearest: Option<usize> = None;
            for a in 0..nodes {
                if !below(n, a) {
                    continue;
                }
                match axis {
                    within::Axis::Inside if member[a] => {
                        expected.insert((id(n), id(a)));
                    }
                    within::Axis::Has if member[n] => {
                        expected.insert((id(n), id(a)));
                    }
                    within::Axis::Nearest if member[a] && nearest.is_none_or(|b| place[b].1 < place[a].1) => nearest = Some(a),
                    _ => {}
                }
            }
            if let Some(a) = nearest {
                expected.insert((id(n), id(a)));
            }
        }
        expected
    };
    use crate::code_rule_dsl::typing::file_local::Binding;
    // Each axis made per looked-up node, and with the batch's solve chosen (decision 0143) on the CPU and on Metal;
    // each read off the documents' tapes in every format (decision 0293).
    use crate::code_rule_dsl::tape::formats::Format;
    assert!(batch.docs.iter().all(|p| p.facts.tape.is_some()), "the axes read every document's tape");
    let ways = [(Binding::Cpu, false), (Binding::Cpu, true), (Binding::Metal, true)];
    let axes = [within::Axis::Nearest, within::Axis::Inside, within::Axis::Has];
    let expected_by: Vec<BTreeSet<(i64, i64)>> = axes.iter().map(|&a| expected_of(a)).collect();
    let cases = axes.into_iter().zip(&expected_by).flat_map(|a| ways.map(|w| (a, w))).flat_map(|c| Format::ALL.map(|f| (c, f)));
    for (((axis, expected), (binding, solve)), format) in cases {
        let expected = expected.clone();
        let source = within::Within::axis(Arc::clone(&batch), axis, &set, binding).reading(format);
        if solve {
            source.expect_lookups(0, 1e12);
        }
        assert!(!expected.is_empty());
        let rows = |rows: Vec<usize>| -> BTreeSet<(i64, i64)> { rows.into_iter().map(|r| (source.value(r, 0), source.value(r, 1))).collect() };
        let ids: Vec<i64> = (0..nodes).map(id).collect();
        let half: BTreeSet<i64> = ids[..nodes / 2].iter().copied().collect();
        let case = format!("{axis:?} on {binding:?}, solved {solve}, format {format:?}");
        assert_eq!(rows(source.lookup(1, &ids[..nodes / 2]).unwrap()), expected.iter().filter(|(_, a)| half.contains(a)).copied().collect(), "{case}");
        assert_eq!(rows(source.lookup(0, &ids).unwrap()), expected, "{case}");
        assert_eq!(rows(source.lookup(1, &ids).unwrap()), expected, "{case}");
        assert_eq!(rows((0..source.len()).collect()), expected, "{case}");
        assert_eq!(source.len(), expected.len());
        assert_eq!(source.estimated_len(), expected.len(), "an axis's rows are counted from the tree");
        let stats = source.statistics().unwrap();
        assert!(stats.iter().all(|(_, rows, ndv, _, _)| *rows == expected.len() as i64 && *ndv <= *rows));
    }
}

/// A lookup by kind (or tape node) where the document is bound too stays in that document: over 128 copies of a
/// file, `node WHERE d = ? AND kind = ?` reads one copy's calls, not every copy's (rechecked against `d`).
#[test]
fn lookups_with_a_bound_document_read_that_document_only() {
    let store = ContentStore::open(&temp("doc-lookups")).unwrap();
    let db = Connection::open_in_memory().unwrap();
    let modules = Modules::default();
    let source = "f(1)\ng(2)\nh(f(3))\n";
    let docs: Vec<Document> = (1..=128).map(|k| document(k, &format!("m{k}.py"), "python", source)).collect();
    for (name, source) in sources(&store, docs).unwrap() {
        modules.bind(&db, name, source).unwrap();
    }
    let python: GrammarDialect = dialect_by_label("python").unwrap();
    let call = core::kind_atom(tape_tag(python).unwrap(), python.kinds().kind_id("call"));
    let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    let node = modules.get("node").unwrap();
    let before = node.counters().candidates.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(count(&format!("SELECT count(*) FROM node WHERE d = 7 AND kind = {call}")), 4);
    let read = node.counters().candidates.load(std::sync::atomic::Ordering::Relaxed) - before;
    assert_eq!(read, 4, "one document's calls, not 128 documents'");
    // The same answer with the document only rechecked.
    assert_eq!(count(&format!("SELECT count(*) FROM node WHERE +d = 7 AND kind = {call}")), 4);
    assert_eq!(count(&format!("SELECT count(*) FROM node WHERE d = 999 AND kind = {call}")), 0);
    let core = modules.get("core_node").unwrap();
    let t: i64 = count("SELECT t FROM core_node WHERE d = 3 LIMIT 1 OFFSET 2");
    let before = core.counters().candidates.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(count(&format!("SELECT count(*) FROM core_node WHERE d = 3 AND t = {t}")), 1);
    assert_eq!(core.counters().candidates.load(std::sync::atomic::Ordering::Relaxed) - before, 1);
}


#[test]
#[cfg(feature = "python-syntax-error")]
fn python_grammar_verdict_is_demanded_and_shared_across_projections() {
    let store = ContentStore::open(&temp("python-verdict")).unwrap();
    let doc = document(0, "a.py", "python", "print \"legacy\"\n");
    // A document is made on its batch's first read of it (decision 0494): each batch here reads one relation whole.
    let read = |projection: &Projection, rel: &str| {
        let sources = shaped(&store, vec![doc.clone()], projection).unwrap();
        sources.iter().find(|(n, _)| *n == rel).unwrap_or_else(|| panic!("no {rel}")).1.len()
    };
    let source_only = Projection { reads: Some(["source".into(), "text".into()].into()), ..Projection::core() };
    read(&source_only, "text");
    assert!(doc.facts.python_syntax.get().is_none(), "raw text must not request grammar validation");
    let ast = Projection { reads: Some(["syntax_error".into()].into()), ..Projection::core() };
    read(&ast, "syntax_error");
    let verdict = doc.facts.python_syntax.get().expect("demanded verdict");
    assert!(matches!(verdict, Ok(Some(_))));
    read(&ast, "syntax_error");
    assert!(std::ptr::eq(verdict, doc.facts.python_syntax.get().unwrap()), "the tape owns one shared verdict");
}

/// Where `ast.parse` raises no `SyntaxError` (a format spec's escape it cannot decode raises `UnicodeDecodeError`),
/// W's worker fails its batch: so does a batch reading CPython's verdict or facts of that document.
#[test]
#[cfg(feature = "python-syntax-error")]
fn a_raising_ast_parse_fails_the_batch() {
    let store = ContentStore::open(&temp("python-raises")).unwrap();
    for read in ["syntax_error", "cpython_syntax_error", "literal_json", "dump_sha256"] {
        let doc = document(0, "a.py", "python", "x = f\"{y:\\x}\"\n");
        let projection = Projection { reads: Some([read.into()].into()), ..Projection::core() };
        // The document is made on the first read (decision 0494): the source read reports why it failed.
        let sources = shaped(&store, vec![doc], &projection).unwrap();
        let source = &sources.iter().find(|(n, _)| *n == read).unwrap_or_else(|| panic!("no {read}")).1;
        let _ = source.len();
        let error = source.failure().unwrap_or_else(|| panic!("{read}: the batch fails"));
        assert!(error.contains("a.py: CPython's ast.parse raises UnicodeDecodeError: 'unicodeescape' codec"), "{read}: {error}");
    }
    let doc = document(0, "a.py", "python", "x = f\"{y:\\x}\"\n");
    let text = Projection { reads: Some(["source".into(), "text".into()].into()), ..Projection::core() };
    let sources = shaped(&store, vec![doc], &text).unwrap();
    let source = &sources.iter().find(|(n, _)| *n == "text").unwrap().1;
    let _ = source.len();
    assert!(source.failure().is_none(), "a text read needs no verdict");
}

/// Literal values and positions are one demanded producer; a names-only rule must not build either table.
#[test]
fn unread_python_literal_columns_are_not_materialized() {
    let store = ContentStore::open(&temp("literal-demand")).unwrap();
    let doc = document(0, "a.py", "python", "x = 'plain'\ny = f'left{x}right'\n");
    let names = Projection { reads: Some(["name".into()].into()), ..Projection::core() };
    let rows = shaped(&store, vec![doc.clone()], &names).unwrap();
    for rel in ["str_part", "str_part_start", "str_part_word"] {
        assert_eq!(rows.iter().find(|(name, _)| *name == rel).unwrap().1.len(), 0, "{rel}");
    }
    let literals = Projection { reads: Some(["str_part".into(), "str_part_start".into()].into()), ..Projection::core() };
    let rows = shaped(&store, vec![doc.clone()], &literals).unwrap();
    for rel in ["str_part", "str_part_start"] {
        assert_eq!(rows.iter().find(|(name, _)| *name == rel).unwrap().1.len(), 3, "{rel}");
    }
    assert_eq!(rows.iter().find(|(name, _)| *name == "str_part_word").unwrap().1.len(), 0);
    let words = Projection { reads: Some(["str_part_word".into()].into()), ..Projection::core() };
    let rows = shaped(&store, vec![doc], &words).unwrap();
    assert_eq!(rows.iter().find(|(name, _)| *name == "str_part_word").unwrap().1.len(), 3);
    for rel in ["str_part", "str_part_start"] {
        assert_eq!(rows.iter().find(|(name, _)| *name == rel).unwrap().1.len(), 0, "{rel}");
    }
}


#[test]
fn long_container_text_is_materialized_only_for_the_consumers_that_demand_it() {
    let store = ContentStore::open(&temp("container-text-demand")).unwrap();
    let source = format!("items = [{}]\n", "1,".repeat(1000));
    let doc = document(0, "a.py", "python", &source);
    let prelude = std::fs::read_to_string(crate::code_rule_dsl::logic::rr::library().join("core.rerail")).unwrap();
    for (body, source_needed, line_needed) in [
        ("out(t) :- source(_, sp), kind(n, \"identifier\"), text(n, t).", false, false),
        (".decl copied(sp: span)\ncopied(s) :- source(_, s).\nout(t) :- copied(s), alias = s, text(alias, t).", true, false),
        (".decl copied(sp: span)\ncopied(s) :- src_line(_, _, s).\nout(t) :- copied(s), text(s, t).", false, true),
        (".decl copied(sp: span)\ncopied(s) :- source(_, s).\ncopied(s) :- src_line(_, _, s).\nout(t) :- copied(s), text(s, t).", true, true),
        ("out(\"missing\") :- source(_, sp), !text(sp, _).", true, false),
        ("out(t) :- source(_, sp), match_all(\"(?<hit>items)\", sp), text(hit, t).", false, false),
    ] {
        let program = crate::code_rule_dsl::logic::rr::parse_version(&format!("{prelude}\n.decl out(t: symbol)\n{body}\n.output out\n")).unwrap();
        let projection = crate::code_rule_dsl::logic::plan::text_demand::projection_of(&program, ["source", "src_line", "text", "rx_span"].into_iter().map(str::to_owned).collect());
        let rows = shaped(&store, vec![doc.clone()], &projection).unwrap();
        let relation = |name| &rows.iter().find(|(n, _)| *n == name).unwrap().1;
        let text = relation("text");
        let has_text = |loc| (0..text.len()).any(|row| text.value(row, 0) == loc);
        assert_eq!(has_text(relation("source").value(0, 1)), source_needed, "source: {body}");
        assert_eq!(has_text(relation("src_line").value(0, 2)), line_needed, "line: {body}");
    }
}

#[test]
fn candidate_demands_preserve_document_scope_and_receiver_independence() {
    let store=ContentStore::open(&temp("candidate-scope")).unwrap();
    let source="import requests\nrequests.get('top')\nasync def f():\n requests.get('url')\n factory().open()\n def nested():\n  requests.get('sync')\n";
    let docs=vec![document(1,"a.py","python",source),document(2,"b.py","python",source)];
    let projection=Projection{reads:Some(["doc","node","line"].into_iter().map(str::to_owned).collect()),..Projection::core()};
    let (batch,relations,_,(strings,_))=Batch::build(&store,docs.clone(),&projection,&[],&[],false).unwrap();
    assert!(!strings.iter().any(|s|s=="url" || s=="requests" || s==source),"unrequested AST strings must not be interned");
    for name in ["text","name","str","value","op","op_at"] {
        assert_eq!(relations.iter().find(|(n,_)|*n==name).unwrap().1.len(),0,"{name}");
    }
    let targets=BTreeSet::from(["requests.get".to_owned()]);
    let rows=batch.python_candidates(&[2],&targets,"async_function","open").unwrap();
    let int=|v:&rusqlite::types::Value|if let rusqlite::types::Value::Integer(n)=v{*n}else{panic!("candidate must be typed")};
    assert_eq!(rows.len(),2);
    assert!(rows.iter().all(|r|int(&r[0])>>32==2));
    assert_eq!(rows.iter().filter(|r|int(&r[2])>=0).count(),1);
    assert_eq!(rows.iter().filter(|r|int(&r[3])>=0).count(),1);
    let receivers=batch.python_candidates(&[2],&BTreeSet::new(),"async_function","open").unwrap();
    assert_eq!(receivers.len(),1);assert_eq!(int(&receivers[0][2]),-1);
    assert!(batch.python_candidates(&[],&targets,"async_function","open").unwrap().is_empty());
    let all=batch.python_candidates(&[1,2],&targets,"async_function","open").unwrap();assert_eq!(all.len(),4);
    let (full,_,_,_)=Batch::build(&store,docs,&Projection::core(),&[],&[],false).unwrap();
    assert_eq!(all,full.python_candidates(&[1,2],&targets,"async_function","open").unwrap(),"projection must preserve node identity");
}

#[test]
fn projection_axes_and_token_spans_are_read_without_renumbering() {
    let f = facts("a.py", "f(a + 1)\ng(b)\n");
    let dialect = Dialect::new(tape_tag(GrammarDialect::Python).unwrap()).unwrap();
    let p = Projected::new(7, Arc::clone(&f), Some(&dialect), Shape::Core, false, None);
    // Independent eager reference: the named nodes' tokens in preorder (by start: they do not overlap), then source
    // and lines.
    let mut expected = Vec::new();
    for (_, id) in p.nodes() {
        for c in f.children(id) {
            if !f.named(c) && f.end(c) > f.start(c) { expected.push([f.start(c), f.end(c)]); }
        }
    }
    expected.sort_unstable();
    let tokens = expected.len();
    expected.push([0,f.source.len() as u32]);
    let lines = core_rr::Lines::new(&f.source,true);
    expected.extend((1..=lines.count()).map(|l|lines.span(l)));
    assert_eq!(p.extra.as_ref().unwrap().tokens as usize,tokens);
    assert_eq!(p.locs(),p.len()+expected.len() as u32);
    for (i,&span) in expected.iter().enumerate() { assert_eq!(p.span(p.len()+i as u32),span); }
    for l in 0..(p.len() as usize) {
        let parent=p.parent(l);
        if l==0 { assert_eq!(parent,NONE); }
        else {
            let siblings:Vec<_>=p.children(parent).collect();
            assert_eq!(p.idx(l),siblings.iter().position(|&c|c==l as u32).unwrap() as u32);
        }
        let mut depth=0;let mut up=parent;
        while up!=NONE { depth+=1;up=p.parent(up as usize); }
        assert_eq!(p.depth(l),depth);
        let subtree_end=f.subtree_end(p.node(l as u32));
        assert_eq!(p.size(l),p.nodes().skip(l+1).filter(|&(_,id)|id<subtree_end).count() as u32);
    }
    // The `field` view's rows: the nodes below the root under a field, in order, from the rank index.
    let fielded: Vec<u32> = p.nodes().skip(1).filter(|&(_, x)| f.field(x) != 0).map(|(l, _)| l).collect();
    assert!(!fielded.is_empty());
    assert_eq!((0..p.fielded()).map(|i| p.fielded_local(i)).collect::<Vec<_>>(), fielded);
}

/// `Extra`'s span ids (`granules::ExtraSpan`) never equal a node's or a position's: each kind's slots are neither the
/// positions' (`span_of` reads none of them) nor any slot a document's nodes take, even one folded past the granules'
/// slots; the kinds' slots are distinct; and one span past a kind's capacity has no id (a named error, not a wrap).
#[test]
fn definition_slots_stay_below_the_span_bit() {
    use granules::{ExtraSpan, INDEX, extra_low, span_of};
    let kinds = [ExtraSpan::Line, ExtraSpan::Lf, ExtraSpan::Tape, ExtraSpan::Comment, ExtraSpan::Match];
    let mut extra = BTreeSet::new();
    for kind in kinds {
        assert_eq!(extra_low(kind, kind.capacity()), None, "{kind:?} past its capacity");
        for i in (0..kind.capacity()).step_by(INDEX as usize).chain([kind.capacity() - 1]) {
            let low = extra_low(kind, i).expect("within capacity");
            assert_eq!(span_of(low), None, "{kind:?} {i} is read as a position");
            extra.insert(low >> INDEX_BITS);
        }
    }
    assert_eq!(extra.len(), granules::EXTRA_SLOTS as usize, "each kind's slots are its own");
    if !definition_ids() {
        return;
    }
    let many: String = (0..17_000).map(|i| format!("def f{i}():\n    return {i}\n")).collect();
    let (_, p) = leaf_ids("python", "many.py", &many);
    assert!((0..p.locs()).all(|l| !extra.contains(&((p.global(l) as u32) >> INDEX_BITS))), "a node or position takes an `Extra` slot");
}

/// What core.rerail's span ids come to over real files (the capacities `granules::ExtraSpan` and `granules::POSITIONS`
/// give): over `RERAIL_PARITY_ROOTS` (`fixtures`, the data-layer fixtures' sources, or directories), per Python
/// document, the most positions (tokens, text and lines), physical, `\n` and tape lines, comments, and matches and
/// groups of every regex the data-layer pack's `match_all`, `match_group` and `rx_span` name over the source and each
/// kind of line (an upper bound: one program with every pattern over every span kind).
#[test]
#[ignore]
fn span_ids_census() {
    use granules::ExtraSpan;
    // The data-layer pack's converted programs: pack test data (`npm run fixtures:pull`), not in git.
    let pack = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../packs/data-layer/upstream/lint/authored/packs/data-layer/candidates/datalog");
    assert!(pack.exists(), "{} is missing: pack test data is not in git: run `npm run fixtures:pull`", pack.display());
    let literal = regex::Regex::new(r#"(?:match_all|match_group|rx_span)\("((?:[^"\\]|\\.)*)""#).unwrap();
    let mut patterns: BTreeSet<String> = BTreeSet::new();
    for e in std::fs::read_dir(&pack).unwrap().flatten() {
        let text = std::fs::read_to_string(e.path()).unwrap_or_default();
        patterns.extend(literal.captures_iter(&text).map(|c| c[1].replace("\\\"", "\"").replace("\\\\", "\\")));
    }
    let regexes: Vec<regex::Regex> = patterns.iter().filter_map(|p| regex::Regex::new(p).ok()).collect();
    eprintln!("census: {} patterns, {} compile", patterns.len(), regexes.len());
    let count = |text: &str| -> usize {
        regexes.iter().map(|re| re.captures_iter(text).map(|c| c.iter().skip(1).flatten().count() + 1).sum::<usize>()).sum()
    };
    for corpus in crate::code_rule_dsl::cpython::parity::corpora() {
        // (value, file) maxima: positions, lines, lf lines, tape lines, comments, matches.
        let files = &corpus.files;
        let threads = 16;
        let maxima: Vec<[(usize, String); 6]> = std::thread::scope(|s| {
            (0..threads)
                .map(|t| {
                    let count = &count;
                    s.spawn(move || {
                        let mut most: [(usize, String); 6] = Default::default();
                        for (path, shown) in files.iter().skip(t).step_by(threads) {
                            let Ok(src) = std::fs::read_to_string(path) else { continue };
                            let Ok(file) = FileInput::new(shown.clone(), "python".to_owned(), Arc::new(src.clone()), None, None, "core") else { continue };
                            let Ok(Built::Facts(facts)) = core::build(&file) else { continue };
                            let facts = Arc::new(facts);
                            let dialect = Dialect::new(core::facts_tag(&facts)).unwrap();
                            let p = Projected::new(1, Arc::clone(&facts), Some(&dialect), Shape::Core, false, None);
                            let f = &*facts;
                            let lines = physical_lines(&src, true);
                            let lf = physical_lines(&src, false);
                            let tape: Vec<std::ops::Range<usize>> = (0..f.lines.starts().len()).map(|i| f.lines.start(i)..f.lines.end(i)).collect();
                            let comments = (0..f.len() as u32).filter(|&id| f.named(id) && crate::code_rule_dsl::language::adapter::is_comment_kind(dialect.name(f, id))).count();
                            let matches = count(&src) + [&lines, &lf, &tape].iter().flat_map(|ls| ls.iter()).map(|r| count(&src[r.clone()])).sum::<usize>();
                            let now = [(p.locs() - p.len()) as usize, lines.len() + 1, lf.len(), tape.len(), comments, matches];
                            for (m, v) in most.iter_mut().zip(now) {
                                if v > m.0 {
                                    *m = (v, shown.clone());
                                }
                            }
                        }
                        most
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect()
        });
        let capacity = [granules::POSITIONS, u64::from(ExtraSpan::Line.capacity()), u64::from(ExtraSpan::Lf.capacity()), u64::from(ExtraSpan::Tape.capacity()), u64::from(ExtraSpan::Comment.capacity()), u64::from(ExtraSpan::Match.capacity())];
        for (k, name) in ["positions", "source and physical lines", "\\n lines", "tape lines", "comments", "matches and groups"].into_iter().enumerate() {
            let (v, at) = maxima.iter().map(|m| m[k].clone()).max().unwrap_or_default();
            eprintln!("census {}: {} files: {name}: at most {v} per document ({at}), capacity {}", corpus.name, files.len(), capacity[k]);
        }
    }
}

/// Each node's text and its id, under `granules`' numbering (the default): `(text, id)` pairs.
fn leaf_ids(label: &str, path: &str, source: &str) -> (BTreeSet<(String, i64)>, Projected) {
    let d = document(7, path, label, source);
    let dialect = Dialect::new(core::facts_tag(&d.facts)).unwrap();
    let p = Projected::new(d.key, Arc::clone(&d.facts), Some(&dialect), Shape::Core, false, None);
    let f = &*d.facts;
    let leaves = (0..p.len()).map(|l| (f.text(p.node(l)).to_owned(), p.global(l))).collect();
    (leaves, p)
}

/// Decision 0094 §2, per language: an edit inside one definition renumbers that definition's nodes and no other's,
/// and moving definitions keeps their ids (slots follow keys, not order). Each case: the text before, the text after,
/// and leaves that are in untouched granules (each must keep its id).
#[test]
fn definition_ids_follow_granules_in_every_language() {
    // The scheme is the process's: under `RERAIL_NODE_IDS=preorder` there are no granules to check.
    if !definition_ids() {
        return;
    }
    let cases: &[(&str, &str, &str, &str, &[&str])] = &[
        // An edit in `f`'s body; `C.m`, `g`, the import and the module constant keep theirs.
        ("python", "a.py",
         "import os\nLIMIT = 3\n\ndef f(a):\n    return a + 1\n\n@dataclass\nclass C:\n    size: int = 0\n    def m(self, x):\n        return [x, x * 2]\n\ndef g():\n    return os.path.join('gx', 'gy')\n",
         "import os\nLIMIT = 3\n\ndef f(a):\n    y = 2\n    return a + y\n\n@dataclass\nclass C:\n    size: int = 0\n    def m(self, x):\n        return [x, x * 2]\n\ndef g():\n    return os.path.join('gx', 'gy')\n",
         &["os", "LIMIT", "'gx'", "'gy'", "size", "x", "self"]),
        // Moving `g` above `f`: both keep their ids.
        ("python", "a.py",
         "def f():\n    return 'fa'\n\ndef g():\n    return 'ga'\n",
         "def g():\n    return 'ga'\n\ndef f():\n    return 'fa'\n",
         &["'fa'", "'ga'", "f", "g"]),
        // A TSX component edited in its JSX; the other component, the hook and the type keep theirs.
        ("tsx", "a.tsx",
         "import React from 'react';\ntype Props = { title: string };\nexport const Header = ({ title }: Props) => <h1 className=\"hd\">{title}</h1>;\nexport function Footer() {\n  return <footer>ft</footer>;\n}\nfunction useCount() { return 41; }\n",
         "import React from 'react';\ntype Props = { title: string };\nexport const Header = ({ title }: Props) => <div><h1 className=\"hd\">{title}</h1><p>sub</p></div>;\nexport function Footer() {\n  return <footer>ft</footer>;\n}\nfunction useCount() { return 41; }\n",
         &["'react'", "Props", "ft", "useCount", "41"]),
        // A class method edited; the other method, the field and the free function keep theirs.
        ("typescript", "a.ts",
         "class Store {\n  items: number[] = [];\n  add(n: number) { this.items.push(n); }\n  size() { return this.items.length; }\n}\nfunction helper() { return 'hp'; }\n",
         "class Store {\n  items: number[] = [];\n  add(n: number) { if (n > 0) { this.items.push(n); } }\n  size() { return this.items.length; }\n}\nfunction helper() { return 'hp'; }\n",
         &["'hp'", "length", "helper"]),
        // SQL: one statement edited, the others keep theirs.
        ("sql", "a.sql",
         "CREATE TABLE users (id INT, name TEXT);\nCREATE VIEW active AS SELECT id FROM users WHERE id > 7;\nSELECT name FROM users;\n",
         "CREATE TABLE users (id INT, name TEXT, email TEXT);\nCREATE VIEW active AS SELECT id FROM users WHERE id > 7;\nSELECT name FROM users;\n",
         &["active", "7"]),
        // PHP: a method edited; the other method and the function keep theirs.
        ("php", "a.php",
         "<?php\nclass A {\n  public $v = 1;\n  function one() { return 'po'; }\n  function two() { return 'pt'; }\n}\nfunction free() { return 'pf'; }\n",
         "<?php\nclass A {\n  public $v = 1;\n  function one() { $x = 2; return 'po' . $x; }\n  function two() { return 'pt'; }\n}\nfunction free() { return 'pf'; }\n",
         &["'pt'", "'pf'", "two", "free"]),
        // Rust: a function in an impl edited; the struct, the other method and the free function keep theirs.
        ("rust", "a.rs",
         "struct S { n: u32 }\nimpl S {\n    fn get(&self) -> u32 { self.n }\n    fn put(&mut self, v: u32) { self.n = v; }\n}\nfn main() { let s = S { n: 5 }; }\n",
         "struct S { n: u32 }\nimpl S {\n    fn get(&self) -> u32 { self.n + 1 }\n    fn put(&mut self, v: u32) { self.n = v; }\n}\nfn main() { let s = S { n: 5 }; }\n",
         &["put", "v", "main", "5"]),
    ];
    for (label, path, before, after, kept) in cases {
        if crate::code_rule_dsl::language::dialect_by_label(label).is_none_or(|d| !d.built()) { continue; }
        let (a, pa) = leaf_ids(label, path, before);
        let (b, pb) = leaf_ids(label, path, after);
        let id_of = |set: &BTreeSet<(String, i64)>, t: &str| set.iter().filter(|(x, _)| x == t).map(|(_, i)| *i).collect::<BTreeSet<i64>>();
        for t in *kept {
            let (x, y) = (id_of(&a, t), id_of(&b, t));
            assert!(!x.is_empty(), "{label}: no leaf {t}");
            assert_eq!(x, y, "{label}: {t} was renumbered");
        }
        // Ids are unique, and each maps back to its node.
        for p in [&pa, &pb] {
            let ids: BTreeSet<i64> = (0..p.locs()).map(|l| p.global(l)).collect();
            assert_eq!(ids.len(), p.locs() as usize, "{label}: ids clash");
            assert!((0..p.locs()).all(|l| p.local_of((p.global(l) & 0xffff_ffff) as u32) == Some(l)), "{label}: an id does not map back");
        }
    }
}

/// Overflow never falls back to preorder: more granules than slots fold statements into their parents, and a
/// granule past an index's reach goes on in further slots; ids stay unique and map back.
#[test]
fn definition_ids_fold_and_continue_past_their_fields() {
    // 17,000 module-level statements: more than the granules' slots.
    let many: String = (0..17_000).map(|i| format!("x{i} = {i}\n")).collect();
    // One function of 300,000 nodes: past one slot's indexes.
    let big = format!("def f():\n    return [{}]\n", (0..100_000).map(|i| format!("a{i}.b")).collect::<Vec<_>>().join(", "));
    for source in [many, big] {
        let (_, p) = leaf_ids("python", "big.py", &source);
        let ids: BTreeSet<i64> = (0..p.locs()).map(|l| p.global(l)).collect();
        assert_eq!(ids.len(), p.locs() as usize, "ids clash");
        assert!((0..p.locs()).all(|l| p.local_of((p.global(l) & 0xffff_ffff) as u32) == Some(l)), "an id does not map back");
    }
}

/// `node_order`'s keys (decision 0121): in preorder in every document, and an edit inside one granule moves no other
/// granule's keys, where a preorder index moves every node after the edit. Adding a granule moves those after it, still
/// in preorder.
#[test]
fn node_order_keys_follow_preorder_and_keep_other_granules() {
    if !definition_ids() {
        return;
    }
    // Each leaf's text, id and order key; and whether the keys ascend in preorder.
    let keys = |source: &str| {
        let (_, p) = leaf_ids("python", "a.py", source);
        let ascending = (1..p.len()).all(|l| p.order(l - 1) < p.order(l));
        let f = &*p.facts;
        let leaves: BTreeSet<(String, i64, u32)> = (0..p.len()).filter(|&l| f.children(p.node(l)).is_empty()).map(|l| (f.text(p.node(l)).to_owned(), p.global(l), p.order(l))).collect();
        assert!((0..p.len()).all(|l| p.local_of_order(p.order(l)) == Some(l)), "a key does not map back");
        (ascending, leaves)
    };
    let before = "import os\nLIMIT = 3\n\ndef f(a):\n    return a + 1\n\nclass C:\n    size: int = 0\n    def m(self, x):\n        return [x, x * 2]\n    tail = 'ct'\n\ndef g():\n    return os.path.join('gx', 'gy')\n";
    // `f`'s body grows; `C`'s nested method splits `C`'s nodes into two runs, which stay as they were.
    let edited = before.replace("    return a + 1\n", "    y = [a, a, a]\n    return a + y[0]\n");
    // A statement added before `f`: the ranks after it move.
    let added = before.replace("LIMIT = 3\n", "LIMIT = 3\nOTHER = 4\n");
    let (a0, k0) = keys(before);
    let (a1, k1) = keys(&edited);
    let (a2, _) = keys(&added);
    assert!(a0 && a1 && a2, "keys not in preorder");
    let kept = |k: &BTreeSet<(String, i64, u32)>, text: &str| k.iter().filter(|(t, _, _)| t == text).map(|(_, id, key)| (*id, *key)).collect::<Vec<_>>();
    for text in ["os", "LIMIT", "size", "x", "self", "tail", "path", "join"] {
        assert!(!kept(&k0, text).is_empty(), "{text} not a leaf");
        assert_eq!(kept(&k0, text), kept(&k1, text), "{text} moved: an edit in f moved another granule's id or order key");
    }
}

/// The bit widths from data (decision 0094 §2): over every source file under `DEFIDS_DIRS` (`:`-separated; files by
/// suffix, `node_modules` and virtualenvs left out), per language: files, the most granules in a document and its
/// 99th percentile, the most nodes and tokens in one granule, documents folded or continued, and the numbering's CPU.
#[test]
#[ignore]
fn definition_ids_census() {
    let dirs = std::env::var("DEFIDS_DIRS").expect("DEFIDS_DIRS");
    let label = |p: &Path| match p.extension()?.to_str()? {
        "py" => Some("python"),
        "ts" | "mts" | "cts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "js" | "mjs" | "cjs" | "jsx" => Some("javascript"),
        "sql" => Some("sql"),
        "php" => Some("php"),
        "rs" => Some("rust"),
        "vue" => Some("vue"),
        _ => None,
    };
    let mut files: Vec<(PathBuf, &str)> = Vec::new();
    for dir in dirs.split(':') {
        let mut stack = vec![PathBuf::from(dir)];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if p.is_dir() {
                    if !matches!(name.as_str(), "node_modules" | ".venv" | "venv" | ".git" | "target" | "dist" | "build" | "site-packages" | "tmp") {
                        stack.push(p);
                    }
                } else if let Some(l) = label(&p) {
                    files.push((p, l));
                }
            }
        }
    }
    files.sort();
    #[derive(Default, Clone)]
    struct Row { parse: f64, files: usize, failed: usize, granules: Vec<usize>, biggest: u32, biggest_at: String, most_at: String, folded: usize, continued: usize, nodes: usize, cpu: f64 }
    let threads = 4;
    let rows: Vec<HashMap<&str, Row>> = std::thread::scope(|s| {
        (0..threads)
            .map(|t| {
                let files = &files;
                s.spawn(move || {
                    let mut by: HashMap<&str, Row> = HashMap::new();
                    for (path, l) in files.iter().skip(t).step_by(threads) {
                        let row = by.entry(*l).or_default();
                        row.files += 1;
                        let Ok(src) = std::fs::read_to_string(path) else { row.failed += 1; continue };
                        let Ok(file) = FileInput::new(path.display().to_string(), (*l).to_owned(), Arc::new(src), None, None, "core") else { row.failed += 1; continue };
                        let t_parse = cpu_now();
                        let Ok(Built::Facts(facts)) = core::build(&file) else { row.failed += 1; continue };
                        let facts = Arc::new(facts);
                        let dialect = Dialect::new(core::facts_tag(&facts)).unwrap();
                        let p = Projected::new(1, Arc::clone(&facts), Some(&dialect), Shape::Core, false, None);
                        row.parse += cpu_now() - t_parse;
                        let f = &*facts;
                        let tokens: Vec<u32> = p.nodes().flat_map(|(l, id)| f.children(id).into_iter().filter(|&c| !f.named(c) && f.end(c) > f.start(c)).map(move |_| l)).collect();
                        let t0 = cpu_now();
                        let _ = granules::number(&p, &dialect, &tokens, None);
                        row.cpu += cpu_now() - t0;
                        let (g, cont, biggest, fold) = granules::census(&p, &dialect, &tokens);
                        row.nodes += (p.len() as usize) + tokens.len();
                        if g > row.granules.iter().copied().max().unwrap_or(0) {
                            row.most_at = path.display().to_string();
                        }
                        row.granules.push(g);
                        if biggest > row.biggest {
                            row.biggest = biggest;
                            row.biggest_at = path.display().to_string();
                        }
                        row.folded += usize::from(fold > 0);
                        row.continued += usize::from(cont > 0);
                    }
                    by
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect()
    });
    let mut all: HashMap<&str, Row> = HashMap::new();
    for by in rows {
        for (l, r) in by {
            let a = all.entry(l).or_default();
            a.files += r.files;
            a.failed += r.failed;
            a.granules.extend(r.granules);
            if r.biggest > a.biggest {
                a.biggest = r.biggest;
                a.biggest_at = r.biggest_at;
            }
            a.most_at = if a.most_at.is_empty() { r.most_at } else { a.most_at.clone() };
            a.folded += r.folded;
            a.continued += r.continued;
            a.nodes += r.nodes;
            a.cpu += r.cpu;
            a.parse += r.parse;
        }
    }
    let mut langs: Vec<_> = all.into_iter().collect();
    langs.sort_by_key(|(l, _)| *l);
    for (l, mut r) in langs {
        r.granules.sort_unstable();
        let pct = |q: f64| r.granules.get(((r.granules.len() as f64 - 1.0) * q) as usize).copied().unwrap_or(0);
        eprintln!(
            "census {l}: {} files ({} failed), granules p50 {} p99 {} max {}, biggest granule {} nodes+tokens ({}), folded {}, continued {}, {} nodes+tokens, numbering {:.2} s CPU (parse and project, numbering included, {:.2} s)",
            r.files, r.failed, pct(0.5), pct(0.99), r.granules.last().copied().unwrap_or(0), r.biggest, r.biggest_at, r.folded, r.continued, r.nodes, r.cpu, r.parse
        );
    }
}

/// This thread's CPU seconds.
fn cpu_now() -> f64 {
    rerail_prof::thread_cpu().as_secs_f64()
}

/// A rule reading the text of a node whose kind no body fixes gets a twin reading `@rr_node_text` (decision 0109);
/// a fixed kind, a field of leaves only, a short constant, and the twin itself need none. With the twin, no rule
/// asks for every node's text at any length. A negation gets no twin: of a short constant it needs no longer text,
/// and otherwise every node's text is made at any length.
#[test]
fn long_text_twins_where_no_body_fixes_the_kind() {
    let prelude = std::fs::read_to_string(crate::code_rule_dsl::logic::rr::library().join("core.rerail")).unwrap();
    for (body, twins) in [
        ("out(t) :- node(_, n, _, _, _), text(n, t).", 1),
        ("out(t) :- node(_, n, _, _, _), text(n, t), field(n, \"object\", o), text(o, t).", 3),
        ("out(t) :- kind(n, \"call\"), text(n, t).", 0),
        ("out(t) :- field(_, \"attribute\", n), text(n, t).", 0),
        ("out(\"doc\") :- node(_, n, _, _, _), text(n, \"doc\").", 0),
    ] {
        let program = crate::code_rule_dsl::logic::rr::parse_version(&format!("{prelude}\n.decl out(t: symbol)\n{body}\n.output out\n")).unwrap();
        let (program, made) = crate::code_rule_dsl::logic::plan::corpus::preparation::text::long_text_twins(program, &Vec::new()).unwrap();
        assert_eq!(made, twins, "{body}");
        assert!(crate::code_rule_dsl::logic::plan::text_demand::text_kind_demand(&program, &[]).any.is_empty(), "{body}: every text is read");
        assert_eq!(crate::code_rule_dsl::logic::plan::corpus::preparation::text::long_text_twins(program, &Vec::new()).unwrap().1, 0, "{body}: twinned once");
    }
    for (body, negated_any) in [
        ("out(\"doc\") :- node(_, n, _, _, _), !text(n, \"doc\").", 0),
        ("out(t) :- node(_, n, _, _, _), field(n, \"attribute\", c), text(c, t), !text(n, t).", 1),
    ] {
        let program = crate::code_rule_dsl::logic::rr::parse_version(&format!("{prelude}\n.decl out(t: symbol)\n{body}\n.output out\n")).unwrap();
        let (program, made) = crate::code_rule_dsl::logic::plan::corpus::preparation::text::long_text_twins(program, &Vec::new()).unwrap();
        assert_eq!(made, 0, "{body}: no twin");
        let demand = crate::code_rule_dsl::logic::plan::text_demand::text_kind_demand(&program, &[]);
        assert_eq!((demand.any.len(), demand.negated_any), (0, negated_any), "{body}");
    }
}

/// The nodes given `text` are the ones the rules read (decision 0110): a fixed kind, a node's field, or a matcher
/// capture's kind or field; a node nothing narrows widens it to every node. An `inside`'s `field` is the inner
/// node's place, not the capture's.
#[test]
fn text_only_for_the_nodes_read() {
    let prelude = std::fs::read_to_string(crate::code_rule_dsl::logic::rr::library().join("core.rerail")).unwrap();
    let fun = |rule: &str| format!(r#"{{"language": "python", "syntax": "valid", "rule": {rule}}}"#);
    let some = |kinds: &[&str], fields: &[&str]| Some(TextNodes { kinds: kinds.iter().map(|k| k.to_string()).collect(), fields: fields.iter().map(|k| k.to_string()).collect(), ..TextNodes::default() });
    let valued = |values: &[&str]| Some(TextNodes { values: values.iter().map(|k| k.to_string()).collect(), ..TextNodes::default() });
    for (body, read) in [
        ("out(t) :- kind(n, \"call\"), text(n, t).".to_owned(), some(&["call"], &[])),
        ("out(t) :- field(_, \"name\", n), text(n, t).".to_owned(), some(&[], &["name"])),
        ("out(t) :- node(_, n, _, _, _), text(n, t).".to_owned(), None),
        // Only compared with a constant: the nodes whose text it is (names.rerail's `literal_str`).
        ("out(l) :- kind(l, \"generic_type\"), child(l, v, 0), text(v, \"Literal\").".to_owned(), valued(&["Literal"])),
        // A child of a node of a fixed kind: those children (imports.rerail's `ribind`).
        ("out(t) :- kind(n, \"dotted_name\"), child(n, c, 0), text(c, t).".to_owned(), Some(TextNodes { parents: BTreeSet::from(["dotted_name".to_owned()]), ..TextNodes::default() })),
        // A child of a node in a field: the children of the nodes in it (imports.rerail's `impmod`).
        ("out(m) :- field(_, \"module_name\", mn), child(mn, dn, 1), text(dn, m).".to_owned(), Some(TextNodes { parent_fields: BTreeSet::from(["module_name".to_owned()]), ..TextNodes::default() })),
        // A child of a node nothing narrows: every node.
        ("out(t) :- child(n, c, 0), text(c, t).".to_owned(), None),
        (format!("out(t) :- ast(d, f, {}, NAME: n), text(n, t).", fun(r#"{"all": [{"kind": "function_definition"}, {"has": {"pattern": "$NAME", "field": "name"}}]}"#)), some(&[], &["name"])),
        (format!("out(t) :- ast(d, f, {}, N: n), text(n, t).", fun(r#"{"all": [{"kind": "call"}, {"has": {"all": [{"kind": "identifier"}, {"pattern": "$N"}]}}]}"#)), some(&["identifier"], &[])),
        (format!("out(t) :- ast(d, f, {}, N: n), text(n, t).", fun(r#"{"all": [{"kind": "call"}, {"inside": {"pattern": "$N", "field": "body"}}]}"#)), None),
    ] {
        let program = crate::code_rule_dsl::logic::rr::parse_version(&format!("{prelude}\n.decl out(t: symbol)\n{body}\n.output out\n")).unwrap();
        let (program, matchers) = crate::code_rule_dsl::logic::plan::corpus::preparation::text::matcher_relations(program).unwrap();
        assert_eq!(crate::code_rule_dsl::logic::plan::text_demand::text_kind_demand(&program, &matchers).read, read, "{body}");
    }
}

/// A negated `text` read narrows the nodes given `text` as a positive one does: by the field the node is in
/// (PYI025's `field(a, "alias", al), !text(al, "AbstractSet")`), the constant it is compared with, or its fixed kind.
/// Of a constant within the bound it needs no text past the bound, even of a fixed kind; compared with a variable, its
/// fixed kind's text is made at any length.
#[test]
fn a_negated_text_read_narrows_the_nodes_it_tests() {
    let prelude = std::fs::read_to_string(crate::code_rule_dsl::logic::rr::library().join("core.rerail")).unwrap();
    let set = |names: &[&str]| names.iter().map(|k| k.to_string()).collect::<BTreeSet<_>>();
    for (body, read, long) in [
        ("out(\"AbstractSet\") :- field(_, \"alias\", n), !text(n, \"AbstractSet\").", TextNodes { fields: set(&["alias"]), ..TextNodes::default() }, set(&[])),
        ("out(\"self\") :- node(_, n, _, _, _), !text(n, \"self\").", TextNodes { values: set(&["self"]), ..TextNodes::default() }, set(&[])),
        ("out(\"self\") :- kind(n, \"identifier\"), !text(n, \"self\").", TextNodes { kinds: set(&["identifier"]), ..TextNodes::default() }, set(&[])),
        ("out(t) :- kind(n, \"identifier\"), kind(s, \"string\"), text(s, t), !text(n, t).", TextNodes { kinds: set(&["identifier", "string"]), ..TextNodes::default() }, set(&["identifier", "string"])),
    ] {
        let program = crate::code_rule_dsl::logic::rr::parse_version(&format!("{prelude}\n.decl out(t: symbol)\n{body}\n.output out\n")).unwrap();
        let demand = crate::code_rule_dsl::logic::plan::text_demand::text_kind_demand(&program, &[]);
        assert_eq!(demand.read, Some(read), "{body}: the nodes given text");
        assert_eq!(demand.long, long, "{body}: the kinds whose text is made at any length");
        assert_eq!((demand.any.len(), demand.negated_any), (0, 0), "{body}: no node of any kind");
    }
}

/// A base relation's name (`kind`, `field`, `child`) the program gives rules of its own is a relation like any
/// other: it narrows the nodes whose text is read only by what its rules give. Here they give nothing narrower than
/// `edge`, so every node's text is read. `node`'s projection `kind(n, k) :- node(_, n, k, _, _)` is `kind` itself.
#[test]
fn a_derived_base_name_narrows_no_text() {
    let decls = ".decl edge(a: node, b: node)\n.decl text(n: node, t: symbol)\n.decl node(d: node, n: node, k: symbol, s: number, e: number)\n.input edge, text, node\n\
                 .decl kind(n: node, k: symbol)\n.decl field(p: node, f: symbol, c: node)\n.decl child(p: node, c: node, i: number)\n\
                 .decl out(t: symbol)\n.output out\n";
    for (derives, body) in [
        ("kind(n, \"call\") :- edge(n, _).", "out(t) :- kind(n, \"call\"), text(n, t)."),
        ("field(p, \"name\", c) :- edge(p, c).", "out(t) :- field(_, \"name\", n), text(n, t)."),
        ("field(p, \"attribute\", c) :- edge(p, c).", "out(t) :- field(_, \"attribute\", n), text(n, t)."),
        ("child(p, c, 0) :- edge(p, c).\nkind(n, \"dotted_name\") :- edge(n, _).", "out(t) :- kind(n, \"dotted_name\"), child(n, c, 0), text(c, t)."),
        // `child` alone derived, `kind` the base relation: only the `child` guard keeps `parents_of` from narrowing
        // the child's text to the children of `dotted_name` nodes.
        ("child(p, c, 0) :- edge(p, c).", "out(t) :- kind(n, \"dotted_name\"), child(n, c, 0), text(c, t)."),
        // `node`'s projection and a rule of the program's own: `kind` is the program's.
        ("kind(n, k) :- node(_, n, k, _, _).\nkind(n, \"call\") :- edge(n, _).", "out(t) :- kind(n, \"call\"), text(n, t)."),
    ] {
        let program = crate::code_rule_dsl::logic::rr::parse_version(&format!("{decls}{derives}\n{body}\n")).unwrap();
        let demand = crate::code_rule_dsl::logic::plan::text_demand::text_kind_demand(&program, &[]);
        assert_eq!(demand.read, None, "{derives} / {body}: every node's text");
        assert!(!demand.any.is_empty(), "{derives} / {body}: a text of a node of any kind");
    }
    // `node`'s projection alone (the lowering's, `rr::lower` `projections`) is `kind` itself: it narrows.
    let program = crate::code_rule_dsl::logic::rr::parse_version(&format!("{decls}kind(n, k) :- node(_, n, k, _, _).\nout(t) :- kind(n, \"call\"), text(n, t).\n")).unwrap();
    assert_eq!(crate::code_rule_dsl::logic::plan::text_demand::text_kind_demand(&program, &[]).read, Some(TextNodes { kinds: BTreeSet::from(["call".to_owned()]), ..TextNodes::default() }));
}

/// A negated `text` read demands the rows it tests, as a positive one does: a policy whose only read of a node's text
/// is `!text(v, "self")` finds the same alone as beside a policy that reads those nodes' text. Alone, no identifier's
/// text was made, and the negation held of `self` too.
#[test]
fn a_negated_text_read_finds_the_same_alone_and_beside_a_reader() {
    let dir = tempfile::tempdir().unwrap();
    let at = |name: &str| dir.path().join(name);
    std::fs::create_dir_all(at("root")).unwrap();
    std::fs::write(at("root/a.py"), "self.x: int\nother.y: int\n").unwrap();
    let policy = |id: &str, rule: &str| format!(".policy \"t:{id}\" revision 1\n.language python\n.mode steer\n.scope include \"**/*\"\n{rule}\n");
    std::fs::write(at("negated.rerail"), policy("negated", ".violation Odd(at: node) code \"odd\" \"an attribute of a name other than self\"\nOdd(t) :- kind(t, \"attribute\"), field(t, \"object\", v), kind(v, \"identifier\"), !text(v, \"self\").")).unwrap();
    std::fs::write(at("reader.rerail"), policy("reader", ".violation Named(at: node) code \"named\" \"a name\"\nNamed(v) :- kind(v, \"identifier\"), text(v, \"nothing\").")).unwrap();
    let lines = |policies: &[&str], out: &str| -> Vec<i64> {
        let paths: Vec<PathBuf> = policies.iter().map(|p| at(p)).collect();
        let report = crate::code_rule_dsl::tools::run_policies(&paths, &at("root"), &at(out), &serde_json::Value::Null).unwrap();
        report["diagnostics"].as_array().unwrap().iter().filter(|d| d["code"] == "odd").map(|d| d["line"].as_i64().unwrap()).collect()
    };
    assert_eq!(lines(&["negated.rerail"], "alone"), [2], "alone");
    assert_eq!(lines(&["negated.rerail", "reader.rerail"], "beside"), [2], "beside a reader");
}

/// `node_text` (a node's exact text, at any length) is answered on request (decision 0174): each reader asks for the
/// nodes its other literals bind, less those that read the text; a relation that reads `node_text` is read through
/// its widened copy; a negation asks for the nodes it tests. A reader that binds its node by nothing else keeps it an
/// input, for every node.
/// Made for every node, a module's text, each class's in it and each function's in those were all interned (390 MB of
/// atoms from 69 MB of source).
#[test]
fn node_text_answered_on_request() {
    let decls = ".decl node(d: number, n: node, k: symbol, a: number, b: number)\n.decl child(p: node, c: node, i: number)\n.decl field(p: node, f: symbol, c: node)\n.decl node_text(n: node, s: symbol)\n.input node, child, field, node_text\n.decl out(s: symbol)\n.output out\n.decl named(n: node, s: symbol)\n";
    let requests = |body: &str| -> Option<Vec<String>> {
        let program = crate::code_rule_dsl::logic::rr::parse_version(&format!("{decls}{body}\n")).unwrap();
        if program.input_names().iter().any(|i| i == "node_text") {
            return None;
        }
        let head = |r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>| program.rel_name(r.head().rel());
        let mut out: Vec<String> = program.rules().filter(|&r| head(r).starts_with("rr_node_text_req") || head(r).starts_with("rr_wide_")).map(|r| format!("{}.", program.show_rule(r).replace('$', ""))).collect();
        out.sort();
        Some(out)
    };
    let one = |body: &str, want: &[&str]| assert_eq!(requests(body), Some(want.iter().map(|s| s.to_string()).collect()), "{body}");
    one("out(s) :- node(_, m, \"module\", _, _), node_text(m, s).", &["rr_node_text_req(m) :- node(_, m, \"module\", _, _)."]);
    one("out(s) :- field(_, \"type\", y), node_text(y, s).", &["rr_node_text_req(y) :- field(_, \"type\", y)."]);
    // A literal that reads the text is left out; the rest that binds the node is the request, a variable read once a
    // wildcard (decision 0291).
    one("out(s) :- field(p, \"name\", y), node_text(y, s), node(_, q, \"call\", _, _), field(q, \"function\", f), node_text(f, s), child(p, x, 0).", &[
        "rr_node_text_req(f) :- node(_, q, \"call\", _, _), field(q, \"function\", f).",
        "rr_node_text_req(y) :- field(p, \"name\", y), child(p, _, 0).",
    ]);
    // Decision 0291. A reader that pairs nodes by kind, read only with its first node bound: the request for that node
    // also joins what its reader binds it by (`rr_node_text_req_by_pair_0`). The request for the other loses the
    // witness `v` and the atoms the rest already holds (`node(_, _, k, _, _)` beside `node(_, e, k, _, _)`): it was
    // every same-kind pair.
    one(".decl pair(v: node, e: node)\npair(v, e) :- node(_, e, \"string\", _, _), node(_, v, k, _, _), node(_, e, k, _, _), node_text(v, s), node_text(e, s).\nout(t) :- field(_, \"right\", v), pair(v, _), t = \"x\".", &[
        "rr_node_text_req(e) :- node(_, e, \"string\", _, _).",
        "rr_node_text_req(v) :- node(_, e, \"string\", _, _), node(_, v, k, _, _), node(_, e, k, _, _), rr_node_text_req_by_pair_0(v).",
        "rr_node_text_req_by_pair_0(rr_dn) :- field(_, \"right\", rr_dn).",
    ]);
    // A node bound by a relation that reads node_text: through its widened copy, on the columns it binds without it.
    // `named`'s own request is narrowed by its one reader's ask (decision 0291).
    one("named(n, s) :- node(_, n, \"identifier\", _, _), node_text(n, s).\nout(t) :- named(n, \"x\"), field(n, \"name\", m), node_text(m, t).", &[
        "rr_node_text_req(m) :- rr_wide_node_text_named(n), field(n, \"name\", m).",
        "rr_node_text_req(n) :- node(_, n, \"identifier\", _, _), rr_node_text_req_by_named_0(n).",
        "rr_node_text_req_by_named_0(rr_dn) :- field(rr_dn, \"name\", _).",
        "rr_wide_node_text_named(n) :- node(_, n, \"identifier\", _, _).",
    ]);
    // A negation asks for the nodes it tests.
    one("out(s) :- node(_, n, \"call\", _, _), !node_text(n, \"x\"), s = \"y\".", &["rr_node_text_req(n) :- node(_, n, \"call\", _, _)."]);
    // Every node: eager, as before.
    assert_eq!(requests("out(s) :- node_text(_, s)."), None);
}

/// What an edit changes of `node_order` (measured for the stable order keys): over `ORDER_PAIRS` (lines `step\told
/// file\tnew file`, Python), the rows deleted plus inserted under each key: the preorder index (today's), a granule
/// run's rank and the index inside the granule (no state: the same for a full run), and the floor, the nodes whose
/// ids change (their rows change under any key).
#[test]
#[ignore]
fn node_order_keys_census() {
    let pairs = std::fs::read_to_string(std::env::var("ORDER_PAIRS").expect("ORDER_PAIRS")).unwrap();
    // Each scheme's rows (id, key) of a document.
    let rows = |path: &str| -> [BTreeSet<(i64, i64)>; 3] {
        let src = std::fs::read_to_string(path).unwrap();
        let (_, p) = leaf_ids("python", "a.py", &src);
        let dialect = Dialect::new(core::facts_tag(&p.facts)).unwrap();
        let of = granules::granule_of(&p, &dialect);
        let n = p.len() as usize;
        let mut pre = BTreeSet::new();
        let mut runs = BTreeSet::new();
        let mut ids = BTreeSet::new();
        let mut keys = BTreeSet::new();
        let (mut rank, mut idx) = (0i64, vec![0i64; of.iter().max().map_or(1, |&m| m as usize + 1)]);
        for l in 0..n {
            let id = p.global(l as u32);
            if l > 0 && of[l] != of[l - 1] {
                rank += 1;
            }
            let g = of[l] as usize;
            pre.insert((id, l as i64));
            runs.insert((id, rank << 32 | idx[g]));
            assert_eq!(runs.len(), pre.len());
            keys.insert((id, i64::from(p.order(l as u32))));
            idx[g] += 1;
            ids.insert((id, 0));
        }
        assert_eq!(runs.len(), keys.len());
        [pre, keys, ids]
    };
    let mut total = [[0usize; 3]; 3];
    for line in pairs.lines() {
        let mut f = line.split('\t');
        let (step, old, new) = (f.next().unwrap(), f.next().unwrap(), f.next().unwrap());
        let (a, b) = (rows(old), rows(new));
        let d: Vec<usize> = (0..3).map(|i| a[i].symmetric_difference(&b[i]).count()).collect();
        let s = ["06", "07", "08"].iter().position(|x| *x == step).unwrap();
        for i in 0..3 {
            total[s][i] += d[i];
        }
        println!("{step} {:>6} nodes  preorder {:>6}  runs {:>6}  floor {:>6}  {}", b[0].len(), d[0], d[1], d[2], new.rsplit('/').take(2).collect::<Vec<_>>().join(" < "));
    }
    for (s, t) in total.iter().enumerate() {
        println!("step {}: preorder {}  runs {}  floor {}", ["06", "07", "08"][s], t[0], t[1], t[2]);
    }
}

/// A phase whose exports hold no node projects only the subtrees of the kinds its rules start from (`node_anchors`):
/// a kind reached below another is no anchor of its own; a parent, an ancestor, a child's place among unnamed
/// siblings, a node no kind or descent proves, or an export holding a node, keeps every node.
#[test]
fn a_phase_projects_the_subtrees_its_rules_start_from() {
    let prelude = std::fs::read_to_string(crate::code_rule_dsl::logic::rr::library().join("core.rerail")).unwrap();
    let kinds = |ks: &[&str]| Ok(ks.iter().map(|k| k.to_string()).collect::<BTreeSet<String>>());
    let phase = |body: &str, out: &str| format!("{prelude}\n.phase p\n.export out\n.decl out({out})\n{body}\n.output out\n");
    for (src, want) in [
        (phase("out(t) :- kind(i, \"import_statement\"), field(i, \"name\", n), kind(n, \"dotted_name\"), text(n, t).", "t: symbol"), kinds(&["import_statement"])),
        (phase(".decl imp(i: node, q: symbol)\nimp(i, q) :- kind(i, \"import_statement\"), child(i, n, 0), text(n, q).\nout(q) :- imp(i, q), kind(i, \"import_statement\"), docof(i, _).", "q: symbol"), kinds(&["import_statement"])),
        (phase("out(t) :- kind(i, \"import_statement\"), !child(i, _, 1), text(i, t).", "t: symbol"), kinds(&["import_statement"])),
        (phase("out(t) :- kind(i, \"import_statement\"), child(p, i, _), text(p, t).", "t: symbol"), Err(String::new())),
        (phase("out(t) :- kind(i, \"import_statement\"), within(i, f), text(f, t).", "t: symbol"), Err(String::new())),
        (phase("out(t) :- kind(i, \"import_statement\"), child(_, i, k), k = 0, text(i, t).", "t: symbol"), Err(String::new())),
        (phase("out(t) :- node(_, n, _, _, _), text(n, t).", "t: symbol"), Err(String::new())),
        (phase("out(i) :- kind(i, \"import_statement\").", "i: node"), Err(String::new())),
        (format!("{prelude}\n.decl out(t: symbol)\nout(t) :- kind(i, \"import_statement\"), text(i, t).\n.output out\n"), Err(String::new())),
    ] {
        let (program, matchers) = crate::code_rule_dsl::logic::plan::corpus::preparation::text::matcher_relations(crate::code_rule_dsl::logic::rr::parse_version(&src).unwrap()).unwrap();
        let got = crate::code_rule_dsl::logic::plan::text_demand::node_anchors(&program, &program.settings().unwrap(), &matchers);
        let body = src.rsplit(".decl out").next().unwrap_or("");
        match &want {
            Ok(k) => assert_eq!(got.as_ref().ok(), Some(k), "{body}: {got:?}"),
            Err(_) => assert!(got.is_err(), "{body}: {got:?}"),
        }
    }
}

/// A rule reading the text of a node no kind fixes gets twins reading `rr_long_text_node(n)` (`long_text_twins`): a
/// node's own fact, as `text` is, so the phase still projects only the subtrees its rules start from. Read as a
/// relation of nodes from anywhere, it kept every node: PostHog's `py_from_name` visited all 41 M nodes.
#[test]
fn a_long_text_twin_keeps_its_phases_anchors() {
    let prelude = std::fs::read_to_string(crate::code_rule_dsl::logic::rr::library().join("core.rerail")).unwrap();
    let src = format!("{prelude}\n.phase p\n.export out\n.decl out(t: symbol)\nout(t) :- kind(i, \"import_from_statement\"), field(i, \"name\", n), text(n, t).\n.output out\n");
    let (program, matchers) = crate::code_rule_dsl::logic::plan::corpus::preparation::text::matcher_relations(crate::code_rule_dsl::logic::rr::parse_version(&src).unwrap()).unwrap();
    let (program, made) = crate::code_rule_dsl::logic::plan::corpus::preparation::text::long_text_twins(program, &matchers).unwrap();
    assert!(made > 0, "the rule reads the text of a node no kind fixes");
    let got = crate::code_rule_dsl::logic::plan::text_demand::node_anchors(&program, &program.settings().unwrap(), &matchers);
    assert_eq!(got, Ok(["import_from_statement".to_owned()].into_iter().collect::<BTreeSet<String>>()));
}

/// The definition ids of every node, token and span of a set of files, pinned (decision 0094 §2): a change to how
/// they are computed (which nodes start a granule, their keys, their slots) must leave every id as it was, or the
/// ids kept across edits and runs move.
#[test]
fn definition_ids_are_pinned() {
    if !definition_ids() {
        return;
    }
    use sha2::{Digest, Sha256};
    let ruff = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../packs/python/upstream/ruff/fixtures");
    let mut files: Vec<(&str, String, String)> = [
        ("python", "a.py", "import os\nLIMIT = 3\n\ndef f(a):\n    return a + 1\n\n@dataclass\nclass C:\n    size: int = 0\n    def m(self, x):\n        return [x, x * 2]\n"),
        ("tsx", "a.tsx", "import React from 'react';\ntype Props = { title: string };\nexport const Header = ({ title }: Props) => <h1>{title}</h1>;\nexport function Footer() {\n  return <footer>ft</footer>;\n}\n"),
        ("typescript", "a.ts", "class Store {\n  items: number[] = [];\n  add(n: number) { this.items.push(n); }\n}\nfunction helper() { return 'hp'; }\n"),
        ("sql", "a.sql", "CREATE TABLE users (id INT, name TEXT);\nCREATE VIEW active AS SELECT id FROM users WHERE id > 7;\n"),
        ("php", "a.php", "<?php\nclass A {\n  public $v = 1;\n  function one() { return 'po'; }\n}\nfunction free() { return 'pf'; }\n"),
        ("rust", "a.rs", "struct S { n: u32 }\nimpl S {\n    fn get(&self) -> u32 { self.n }\n}\nfn main() { let s = S { n: 5 }; }\n"),
    ].into_iter().map(|(l, p, s)| (l, p.to_owned(), s.to_owned())).collect();
    for f in ["pyflakes/F601.py", "pyflakes/F632.py", "flake8_simplify/SIM905.py", "flake8_bugbear/B031.py", "flake8_bugbear/B006_B008.py"] {
        let source = std::fs::read_to_string(ruff.join(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
        files.push(("python", f.to_owned(), source));
    }
    let mut h = Sha256::new();
    for (label, path, source) in &files {
        let (_, p) = leaf_ids(label, path, source);
        for l in 0..p.locs() {
            h.update(p.global(l).to_le_bytes());
            assert_eq!(p.local_of((p.global(l) & 0xffff_ffff) as u32), Some(l), "{path}: id of {l} does not map back");
        }
    }
    let got: String = h.finalize()[..16].iter().map(|b| format!("{b:02x}")).collect();
    // Moved once by decision 0353 step 2: a granule's tokens are numbered in preorder of the tokens, not by their
    // nodes' order, so the granule table counts them off a rank index (was f280acef04fbe4f44e98f401f7f7d4f8; the
    // nodes' and spans' ids are unchanged).
    assert_eq!(got, "a181683bc9d837ac82beba26e6ff7057", "the definition ids moved");
}

/// What a kind is to the granules (a definition, a wrapper, a body, a container's owner) is a table by kind code,
/// made once per grammar from the kind names (`Dialect::granule_kind`), not names matched for each node: in every
/// grammar the table says what the names say.
#[test]
fn granule_kinds_are_a_table_by_kind_code() {
    for (label, path, source) in [("python", "a.py", "x = 1\n"), ("javascript", "a.js", "x;\n"), ("typescript", "a.ts", "x;\n"), ("tsx", "a.tsx", "x;\n"),
        ("php", "a.php", "<?php\nx();\n"), ("rust", "a.rs", "fn f() {}\n"), ("sql", "a.sql", "SELECT 1;\n")] {
        let d = document(7, path, label, source);
        let dialect = Dialect::new(core::facts_tag(&d.facts)).unwrap();
        let mut seen = 0;
        for code in 0..dialect.catalog.kind_count() {
            let name = dialect.catalog.kind_name(code as u16).unwrap_or("");
            let want = granules::kind_flags(name);
            seen += usize::from(want != 0);
            assert_eq!(dialect.granule_kind(code as u16), want, "{label}: {name}");
        }
        assert!(seen > 0, "{label}: no kind starts or holds a granule");
    }
}

/// Decision 0353 step 2: the granule table answers every node's and token's id, equal to `DefIds::low`'s, in formats
/// 15, 16 and 16z, in both shapes, for every language with a granule rule and a document folded past the slots;
/// `Table::node` inverts it; the scan gives the same ids in preorder; and no other tape node has one.
#[test]
fn node_ids_in_place_equal_the_numbered_ones() {
    use crate::code_rule_dsl::lines::{LineTable, Terminators};
    use crate::code_rule_dsl::tape::{Grammar, TapeImage, formats::Format};
    use ranks::Item;
    if !definition_ids() {
        return;
    }
    let many: String = (0..17_000).map(|i| format!("def f{i}():\n    return {i}\n")).collect();
    // An anchor first, then the root's own tokens (`;`) far past its subtree: the index reaches them.
    let tail: String = format!("def f():\n    return 1\n{}a = 1; b = 2\n", (0..200).map(|i| format!("v{i} = {i}\n")).collect::<String>());
    let cases: &[(&str, &str)] = &[
        ("python", "if a is not b and c not in d:\n    pass\n"),
        ("python", "import os\nX = 1\n\n# c\n@dec\ndef f(a, b=2):\n    '''doc'''\n    return [x for x in a if x]  # t\n\nclass C(B):\n    y: int = 3\n    def m(self):\n        def inner():\n            pass\n        return inner\n\nif X:\n    print(f(1))\n"),
        ("python", &many),
        ("python", &tail),
        ("typescript", "import { a } from 'b';\nexport const App = () => 1;\nclass K { x = 1; m(): number { return this.x; } }\ninterface I { y: string }\nfunction g(n: number) { if (n) { return n; } }\n"),
        ("tsx", "export function A() { return <div className=\"x\">{[1].map(v => <B key={v} />)}</div>; }\nconst y = 2;\n"),
        ("javascript", "const a = 1;\nexport default class Z extends Y { static s = 1; get g() { return 1 } }\nfunction* h() { yield 1 }\n"),
        ("rust", "use std::fmt;\nconst N: u32 = 1;\nstruct S { a: u32 }\nimpl fmt::Display for S {\n    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, \"{}\", self.a) }\n}\nmod m { pub fn g() -> u32 { 1 } }\nfn main() { let x = S { a: N }; println!(\"{x}\"); }\n"),
        ("php", "<?php\nnamespace N;\nuse A\\B;\nclass C { private $p = 1; public function m() { return $this->p; } }\nfunction g($x) { return $x + 1; }\n"),
        ("sql", "CREATE TABLE t (a int, b text);\nCREATE VIEW v AS SELECT a FROM t WHERE b = 'x';\nSELECT 1;\n"),
    ];
    let mut narrower = 0;
    for &(label, source) in cases {
        let lang = dialect_by_label(label).unwrap_or_else(|| panic!("no dialect {label}"));
        let grammar = Grammar::new(lang).unwrap();
        let source = Arc::new(source.to_owned());
        let image = TapeImage::parse(&grammar, &source, &|| false).unwrap().unwrap().primary;
        for format in Format::ALL {
            let lines = Arc::new(LineTable::new(&source, Terminators::of(lang)));
            let facts = Arc::new(core::from_image(Arc::clone(&source), &grammar, image.clone().in_format(format, source.as_bytes()), lines).unwrap());
            let f = &*facts;
            let dialect = Dialect::new(core::facts_tag(f)).unwrap();
            // Whole, and narrowed to anchors (the root and each outermost anchor's subtree, as `node_anchors` asks).
            let anchors: BTreeSet<String> = ["call", "call_expression", "function_definition", "function_item", "function_declaration", "method_definition", "class_declaration", "member_expression", "binary_expression", "select_statement"].iter().map(|s| (*s).to_owned()).collect();
            for (shape, narrowed) in [(Shape::Core, false), (Shape::T, false), (Shape::Core, true)] {
                let p = if narrowed {
                    Projected::anchored(1, Arc::clone(&facts), Some(&dialect), shape, &anchors)
                } else {
                    Projected::new(1, Arc::clone(&facts), Some(&dialect), shape, false, None)
                };
                // The walk's parents (one pass, no tape parent read) are `parent`'s.
                for (l, _, q) in p.walk() {
                    assert_eq!(q, p.parent(l as usize), "{label} {format:?} {shape:?}: the parent of local {l}");
                }
                // A parent searched from the innermost granule holding the node is the tape's (`Tape::parent_within`).
                if let Some(t) = p.table() {
                    for x in 1..f.len() as u32 {
                        assert_eq!(f.parent_within(x, t.holding(x)), f.parent(x), "{label} {format:?} {shape:?}: tape node {x}'s parent");
                    }
                }
                // A node's place among its parent's children and its depth, read off the tape (decision 0353 step 3),
                // are the walk's.
                let (mut next, mut depth) = (vec![0u32; p.len() as usize], vec![0u32; p.len() as usize]);
                for (l, _, q) in p.walk().skip(1) {
                    assert_eq!(p.idx(l as usize), next[q as usize], "{label} {format:?} {shape:?}: the place of local {l}");
                    next[q as usize] += 1;
                    depth[l as usize] = depth[q as usize] + 1;
                    assert_eq!(p.depth(l as usize), depth[l as usize], "{label} {format:?} {shape:?}: the depth of local {l}");
                }
                // `next_sibling` (a view, decision 0353 step 3): its pairs are each numbered node's numbered direct
                // children, consecutive, as core.rerail's rows were made; each read both ways off the tape.
                let mut pairs = Vec::new();
                for (_, x) in p.nodes() {
                    let mut prev = None;
                    for c in p.children_of_node(x) {
                        pairs.extend(prev.map(|a| (a, c)));
                        prev = Some(c);
                    }
                }
                pairs.sort_unstable();
                let mut read: Vec<(u32, u32)> = p.ranks().iter(Item::Sibling).map(|(_, x)| {
                    let b = p.ranks().rank(Item::Node, x);
                    (p.prev_sibling(b).expect("a sibling before"), b)
                }).collect();
                read.sort_unstable();
                assert_eq!(read, pairs, "{label} {format:?} {shape:?} {narrowed}: next_sibling's pairs");
                let after: HashMap<u32, u32> = pairs.iter().copied().collect();
                for l in 0..p.len() {
                    assert_eq!(p.next_sibling(l), after.get(&l).copied(), "{label} {format:?} {shape:?} {narrowed}: the sibling after local {l}");
                }
                let reference = p.reference(&dialect);
                let low = &reference.low[..];
                let table = p.table().expect("ids by definition");
                let at = format!("{label} {} {shape:?}{}", format.name(), if narrowed { " narrowed" } else { "" });
                if narrowed && p.len() > 1 && p.len() < Projected::new(1, Arc::clone(&facts), Some(&dialect), shape, false, None).len() {
                    narrower += 1;
                }
                // Every item the projection numbers, by tape node: nodes, then (Core) tokens in preorder.
                let mut want: Vec<(u32, u32, Item)> = p.nodes().map(|(l, x)| (x, low[l as usize], Item::Node)).collect();
                if shape == Shape::Core {
                    want.extend(p.token_order().into_iter().enumerate().map(|(i, (x, _))| (x, low[(p.len() as usize) + i], Item::Token)));
                }
                assert_eq!(want.len(), low.len(), "{at}: an id for each node and token");
                let by_node: HashMap<u32, (u32, Item)> = want.iter().map(|&(x, low, item)| (x, (low, item))).collect();
                for x in 0..f.len() as u32 {
                    assert_eq!(table.id(p.ranks(), x), by_node.get(&x).copied(), "{at}: tape node {x} ({})", dialect.name(f, x));
                }
                for &(x, low, item) in &want {
                    assert_eq!(table.node(p.ranks(), low), Some((x, item)), "{at}: the node of {low:#x}");
                }
                let mut scanned = Vec::new();
                table.scan(p.ranks(), |x, low, item| scanned.push((x, low, item)));
                want.sort_unstable();
                assert_eq!(scanned, want, "{at}: the scan");
                // The projection's reads: each node's and token's id, and back; each node's order key, and back.
                let tokens = p.ranks().count(Item::Token);
                for l in 0..p.len() + tokens {
                    assert_eq!(p.global(l), 1 << 32 | i64::from(low[l as usize]), "{at}: the id of local {l}");
                    assert_eq!(p.local_of(low[l as usize]), Some(l), "{at}: the local of {:#x}", low[l as usize]);
                }
                // The passes' ids (one walk each) are the point reads'.
                let walked: Vec<(u32, i64)> = p.node_ids().chain(p.token_ids()).map(|(l, _, n)| (l, n)).collect();
                assert_eq!(walked, (0..p.len() + tokens).map(|l| (l, p.global(l))).collect::<Vec<_>>(), "{at}: node_ids, token_ids");
                assert_eq!(p.loc_ids().collect::<Vec<_>>(), (0..p.locs()).map(|l| (l, p.global(l))).collect::<Vec<_>>(), "{at}: loc_ids");
                for (l, x) in p.nodes() {
                    assert_eq!(table.order(p.ranks(), x), Some(reference.order[l as usize]), "{at}: the order key of tape node {x}");
                    assert_eq!(p.order(l), reference.order[l as usize], "{at}: the order key of local {l}");
                    assert_eq!(p.local_of_order(reference.order[l as usize]), Some(l), "{at}: the node of order key {:#x}", reference.order[l as usize]);
                }
                // The rank index's point reads (written once, `id_reads`) agree with its walk over the set bits.
                let r = p.ranks();
                for item in [Item::Node, Item::Token, Item::Field, Item::Sibling] {
                    let walked: Vec<(u32, u32)> = r.iter(item).collect();
                    assert_eq!(walked.len() as u32, r.count(item), "{at}: {item:?}'s count");
                    let set: HashMap<u32, u32> = walked.iter().map(|&(k, x)| (x, k)).collect();
                    for &(k, x) in &walked {
                        assert_eq!(r.select(item, k), Some(x), "{at}: select {item:?} {k}");
                    }
                    assert_eq!(r.select(item, r.count(item)), None, "{at}: select past {item:?}'s last");
                    let mut before = 0;
                    for x in 0..f.len() as u32 + 64 {
                        assert_eq!(r.rank(item, x), before, "{at}: rank {item:?} {x}");
                        assert_eq!(r.is(item, x), set.contains_key(&x), "{at}: is {item:?} {x}");
                        before += u32::from(set.contains_key(&x));
                    }
                }
                // Lows that are no item's have no number; order keys past the last have no node.
                for bad in [u32::MAX, 0x7fff_ffff] {
                    assert_eq!(table.number_of(r, bad), None, "{at}: the number of {bad:#x}");
                    assert_eq!(table.number_of_order(r, bad), None, "{at}: the node of order key {bad:#x}");
                }
            }
        }
    }
    assert!(narrower >= 6, "anchored projections narrower than the whole: {narrower}");
}

/// An axis's row count is counted from the tree, without its batch's solve: a guest round digests each input by its
/// length, and the solve, every node's chain of member ancestors as pairs, was made for that count alone (780 MB of
/// a PostHog run's 2.5 GB peak). A scan over `0..len` still reads every pair, solved then.
#[test]
fn an_axis_counts_its_rows_without_its_solve() {
    use crate::code_rule_dsl::facts::core::kind_atom;
    use crate::code_rule_dsl::typing::file_local::Binding;
    let store = ContentStore::open(&temp("axes-len")).unwrap();
    let docs: Vec<Document> = documents().into_iter().take(4).collect();
    let (batch, _, _, _) = Batch::build(&store, docs, &Projection::default(), &[], &[], false).unwrap();
    let nodes = *batch.nodes().last().unwrap();
    let kind = |r: usize| {
        let (p, local) = batch.locate(View::Node, r);
        let code = crate::code_rule_dsl::facts::core::kind_code(p.facts.kind(p.node(local as u32)));
        i64::from(kind_atom(p.tag, batch.dialects[p.tag].canonical.get(usize::from(code)).copied().unwrap_or(code)))
    };
    let set: Vec<i64> = [0, nodes / 3, nodes / 2].map(kind).to_vec();
    let sources = [
        ("containment", within::Within::new(Arc::clone(&batch), within::Mode::Tree)),
        ("nearest", within::Within::axis(Arc::clone(&batch), within::Axis::Nearest, &set, Binding::Cpu)),
        ("inside", within::Within::axis(Arc::clone(&batch), within::Axis::Inside, &set, Binding::Cpu)),
    ];
    // The last two documents' nodes, as a guest round's new documents (`retain::owners_delta`).
    let new_from = batch.nodes()[2];
    let new_ids: BTreeSet<i64> = (new_from..nodes).map(|r| batch.id(r)).collect();
    for (name, source) in sources {
        let len = source.len();
        assert!(len > 0, "{name}");
        assert!(!source.is_solved(), "{name}: counted without its solve");
        let owned: BTreeSet<(i64, i64)> = source.rows_owned(0, &|n| new_ids.contains(&n)).expect("a batch's view finds a document's rows").into_iter().map(|r| (source.value(r, 0), source.value(r, 1))).collect();
        assert!(!source.is_solved(), "{name}: the new documents' rows found without its solve");
        let scanned: BTreeSet<(i64, i64)> = (0..len).map(|r| (source.value(r, 0), source.value(r, 1))).collect();
        assert!(!owned.is_empty(), "{name}");
        assert_eq!(owned, scanned.iter().filter(|(n, _)| new_ids.contains(n)).copied().collect(), "{name}: exactly the new documents' rows");
        assert_eq!(scanned.len(), len, "{name}: a scan reads every pair once");
        assert!(source.is_solved(), "{name}: solved by the scan");
        assert_eq!(source.len(), len, "{name}");
    }
}

/// Decision 0353 step 5: a solvable axis's answer is one relation of ids in a block, written once: each row's two
/// ids, as `value` reads them, in its order, every column at its place, which the device reads where it lies
/// (`Source::in_block`), with room for an index on each column. Asked again, it is the same relation. Span
/// containment has no solve, and no such relation.
#[test]
fn a_solved_axis_is_one_relation_of_ids_in_a_block() {
    use crate::code_rule_dsl::facts::core::kind_atom;
    use crate::code_rule_dsl::typing::file_local::Binding;
    let store = ContentStore::open(&temp("axes-ids")).unwrap();
    let docs: Vec<Document> = documents().into_iter().take(4).collect();
    let (batch, _, _, _) = Batch::build(&store, docs, &Projection::default(), &[], &[], false).unwrap();
    let nodes = *batch.nodes().last().unwrap();
    let kind = |r: usize| {
        let (p, local) = batch.locate(View::Node, r);
        let code = crate::code_rule_dsl::facts::core::kind_code(p.facts.kind(p.node(local as u32)));
        i64::from(kind_atom(p.tag, batch.dialects[p.tag].canonical.get(usize::from(code)).copied().unwrap_or(code)))
    };
    let set: Vec<i64> = [0, nodes / 3, nodes / 2].map(kind).to_vec();
    for (name, source) in [
        ("containment", within::Within::new(Arc::clone(&batch), within::Mode::Tree)),
        ("nearest", within::Within::axis(Arc::clone(&batch), within::Axis::Nearest, &set, Binding::Cpu)),
        ("inside", within::Within::axis(Arc::clone(&batch), within::Axis::Inside, &set, Binding::Cpu)),
    ] {
        let ib = source.in_block().unwrap_or_else(|| panic!("{name}: its answer in a block"));
        assert_eq!(ib.rows as usize, source.len(), "{name}: every row");
        assert!(ib.rows > 0, "{name}");
        let rel = ib.relation();
        for r in 0..ib.rows {
            assert_eq!((rel.rows.get(r, 0), rel.rows.get(r, 1)), (source.value(r as usize, 0), source.value(r as usize, 1)), "{name}: row {r}");
        }
        let again = source.in_block().expect("again");
        assert!(Arc::ptr_eq(&again.facts, &ib.facts) && again.at == ib.at, "{name}: written once");
        // An index on each column is built in its block: a reader's probe does not copy it (`DevIndex::moved`).
        assert!(ib.own_index(&[0]).is_some() && ib.own_index(&[1]).is_some(), "{name}: room for an index on each column");
    }
    assert!(within::Within::new(Arc::clone(&batch), within::Mode::Span).in_block().is_none(), "span containment has no solve");
}

/// The numbering reads' cases: a projection of each source, in each format, whole, as T and narrowed, with the tape
/// nodes to read (every one, and 64 past the last).
fn id_read_cases() -> Vec<(String, Projected, u32)> {
    use crate::code_rule_dsl::lines::{LineTable, Terminators};
    use crate::code_rule_dsl::tape::{Grammar, TapeImage, formats::Format};
    let mut out = Vec::new();
    let many: String = (0..17_000).map(|i| format!("def f{i}():\n    return {i}\n")).collect();
    let cases: &[(&str, &str)] = &[
        ("python", "import os\nX = 1\n@dec\ndef f(a, b=2):\n    '''doc'''\n    return [x for x in a if x]\n\nclass C(B):\n    def m(self):\n        def inner():\n            pass\n        return inner\n"),
        ("python", &many),
        ("typescript", "import { a } from 'b';\nexport const App = () => 1;\nclass K { x = 1; m(): number { return this.x; } }\nfunction g(n: number) { if (n) { return n; } }\n"),
    ];
    let anchors: BTreeSet<String> = ["call", "call_expression", "function_definition", "method_definition", "class_declaration"].iter().map(|s| (*s).to_owned()).collect();
    for &(label, source) in cases {
        let lang = dialect_by_label(label).unwrap();
        let grammar = Grammar::new(lang).unwrap();
        let source = Arc::new(source.to_owned());
        let image = TapeImage::parse(&grammar, &source, &|| false).unwrap().unwrap().primary;
        for format in Format::ALL {
            let lines = Arc::new(LineTable::new(&source, Terminators::of(lang)));
            let facts = Arc::new(core::from_image(Arc::clone(&source), &grammar, image.clone().in_format(format, source.as_bytes()), lines).unwrap());
            let dialect = Dialect::new(core::facts_tag(&facts)).unwrap();
            for (shape, narrowed) in [(Shape::Core, false), (Shape::T, false), (Shape::Core, true)] {
                let p = if narrowed { Projected::anchored(1, Arc::clone(&facts), Some(&dialect), shape, &anchors) } else { Projected::new(1, Arc::clone(&facts), Some(&dialect), shape, false, None) };
                let at = format!("{label} {} {shape:?}{}", format.name(), if narrowed { " narrowed" } else { "" });
                out.push((at, p, facts.len() as u32 + 64));
            }
        }
    }
    out
}

/// Tape node `x`'s answers by the host's numbering reads (`Numbering::is`, …), as the probes lay them out: its bits,
/// ranks and selects, the low bits of the node and the token numbered `x`, its order key, and back: the low's item's
/// number (`u32::MAX` for none), then the item, and the order key's node.
fn host_id_reads(h: &rerail_clif::ids::Numbering, x: u32) -> [u32; 12] {
    // SAFETY: the projection's live descriptor.
    unsafe {
        let (low0, order) = (h.low_of(0, x), h.order_of(x));
        let (item, k) = h.number_of(low0).unwrap_or((u32::MAX, u32::MAX));
        [u32::from(h.is(0, x)), h.rank(0, x), h.rank(1, x), h.select(0, x), h.select(1, x), low0, h.low_of(1, x), order, k, item, h.number_of_order(order), u32::from(h.is(2, x))]
    }
}

/// Decision 0536: the numbering's reads written directly in Rust (`Numbering::is`, …) answer what its reads in CLIF
/// answer, by the JIT: for every tape node, the kernels' probes (`the_kernels_id_reads_are_the_hosts`) and every
/// item's bit, rank and select, and lows and order keys that are no item's. In all three formats, whole and narrowed.
#[test]
fn the_jits_id_reads_are_the_hosts() {
    if !definition_ids() {
        return;
    }
    let jit = rerail_clif::jit::Reads::new();
    type Two = extern "C" fn(*const u8, u32, u32) -> u32;
    type One = extern "C" fn(*const u8, u32) -> u32;
    type Wide = extern "C" fn(*const u8, u32) -> u64;
    // SAFETY: each read's signature: the descriptor, then its arguments.
    let two = |name: &str| -> Two { unsafe { std::mem::transmute(jit.function(name)) } };
    // SAFETY: as `two`.
    let one = |name: &str| -> One { unsafe { std::mem::transmute(jit.function(name)) } };
    let (is, rank, select, low_of) = (two("num_is"), two("num_rank"), two("num_select"), two("num_low_of"));
    let (order_of, number_of_order) = (one("num_order_of"), one("num_number_of_order"));
    // SAFETY: as `two`.
    let number_of: Wide = unsafe { std::mem::transmute(jit.function("num_number_of")) };
    let wide = |v: Option<(u32, u32)>| v.map_or(u64::MAX, |(i, k)| u64::from(i) << 32 | u64::from(k));
    let mut tried = 0;
    for (at, p, n) in id_read_cases() {
        let r = p.ranks();
        let (h, d) = (r.desc(), r.numbering());
        for x in 0..n {
            let (low0, order) = (low_of(h, 0, x), order_of(h, x));
            let k = number_of(h, low0);
            let (item, k) = if k == u64::MAX { (u32::MAX, u32::MAX) } else { ((k >> 32) as u32, k as u32) };
            let jits = [is(h, 0, x), rank(h, 0, x), rank(h, 1, x), select(h, 0, x), select(h, 1, x), low0, low_of(h, 1, x), order, k, item, number_of_order(h, order), is(h, 2, x)];
            assert_eq!(host_id_reads(d, x), jits, "{at}: tape node {x}");
            let (bad_low, bad_key) = (x << 7 | 0xffff_0000, x << 9);
            // SAFETY: the projection's live descriptor.
            unsafe {
                for item in 0..4 {
                    assert_eq!((u32::from(d.is(item, x)), d.rank(item, x), d.select(item, x)), (is(h, item, x), rank(h, item, x), select(h, item, x)), "{at}: item {item}, tape node {x}");
                }
                assert_eq!(wide(d.number_of(bad_low)), number_of(h, bad_low), "{at}: the number of low {bad_low:#x}");
                assert_eq!(d.number_of_order(bad_key), number_of_order(h, bad_key), "{at}: the node of order key {bad_key:#x}");
            }
        }
        tried += 1;
    }
    assert_eq!(tried, 3 * 3 * 3, "every case, format and shape");
}

/// Decision 0353 step 2: the numbering's reads as UKL (`<ukl/reads/ids.h>`, written once in CLIF), run on the device
/// over a projection's descriptor where it lies in its block, answer what the host's reads (written directly) answer: for
/// every tape node, its bit and ranks, the selects, the ids' low bits of nodes and tokens and back, and the order keys
/// and back. In all three formats, whole and narrowed, a document of more nodes than a granule's slot holds. The
/// kernel is UKL, on the process's device (decision 0440 step C).
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn the_kernels_id_reads_are_the_hosts() {
    use rerail_gpu::device::ComputeQueue;
    use rerail_gpu::kernels::{alloc, launch, read};
    if !definition_ids() {
        return;
    }
    let Some(shared) = rerail_gpu::shared::get() else { return };
    const WIDE: usize = 12;
    /// The probe's parameters: the descriptor's offset in its block, the nodes.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct IdsP {
        d: u64,
        n: u32,
        pad: u32,
    }
    unsafe impl rerail_gpu::Pod for IdsP {}
    const KERNEL: &str = r#"
typedef struct IdsP { uint64_t d; uint32_t n; uint32_t pad; } IdsP;
UKL_REFLECT_STRUCT(IdsP, d, n, pad)
DEFINE_KERNEL2(syntax_tests, ids_probe, ROBUFFER, uint8_t, b, "[block]", ARGS, IdsP, p, , RWBUFFER, uint32_t, out, "[n * 12]",
               GID0, uint32_t, x, )
{
    if (x >= p.n) return;
    uint64_t d = p.d;
    UKL_GLOBAL uint32_t* o = out + x * 12u;
    o[0] = clif_num_is(b, d, 0u, x);
    o[1] = clif_num_rank(b, d, 0u, x);
    o[2] = clif_num_rank(b, d, 1u, x);
    o[3] = clif_num_select(b, d, 0u, x);
    o[4] = clif_num_select(b, d, 1u, x);
    o[5] = clif_num_low_of(b, d, 0u, x);
    o[6] = clif_num_low_of(b, d, 1u, x);
    o[7] = clif_num_order_of(b, d, x);
    uint64_t k = clif_num_number_of(b, d, o[5]);
    o[8] = (uint32_t)k; o[9] = (uint32_t)(k >> 32);
    o[10] = clif_num_number_of_order(b, d, o[7]);
    o[11] = clif_num_is(b, d, 2u, x);
}
"#;
    let source = format!("#include <ukl/backend.h>\n#include <ukl/reads/ids.h>\n{KERNEL}");
    let program = rerail_gpu::kernels::program("ids_probe.uglykl", Box::leak(source.into_boxed_str())).unwrap_or_else(|e| panic!("the id reads compile: {e}"));
    let probe = program.function("ids_probe").unwrap();
    let mut tried = 0;
    for (at, p, n) in id_read_cases() {
        let r = p.ranks();
        let block = r.block();
        let d = block.offset_of(r.desc()) as u64;
        let out = alloc::<u32>(n as usize * WIDE).unwrap();
        let mut q = ComputeQueue::new(&shared.queue);
        launch(&q, probe)
            // The projection's block, read where it lies.
            .buffer("b", &block.device_buffer(&shared.device).unwrap())
            .value("p", &IdsP { d, n, pad: 0 })
            .buffer("out", &out)
            .run(&[(u64::from(n) * 16).div_ceil(64)], &[64])
            .unwrap_or_else(|e| panic!("{e}"));
        q.flush().await_().unwrap();
        let got: Vec<u32> = read(&out, n as usize * WIDE);
        for x in 0..n {
            let want = host_id_reads(r.numbering(), x);
            assert_eq!(&got[x as usize * WIDE..][..WIDE], &want, "{at}: tape node {x}");
        }
        tried += 1;
    }
    assert_eq!(tried, 3 * 3 * 3, "every case, format and shape");
}

/// Decision 0353 step 4c: the `node`, `child`, `field`, `core_node` and `line` views' rows read through the views' reads (written once in CLIF), by the JIT on
/// the host and as MSL on the device, over the document's entry, its numbering where it lies in its block and its
/// tape where it is read in place, are the host's rows (`Viewed::value_of`): every numbered node's document, id, kind,
/// start and end; each child's parent, id and place; each fielded node's parent, field and id; each node's tape node;
/// each node's line, by its lines where they lie (step 4d). In all three formats, whole and narrowed.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn the_view_reads_are_the_hosts_rows() {
    use crate::code_rule_dsl::lines::{LineTable, Terminators};
    use crate::code_rule_dsl::tape::{Grammar, TapeImage, formats::Format};
    use ranks::Item;
    use rerail_clif::view::TapeView;
    use rerail_gpu::device::ComputeQueue;
    use rerail_gpu::kernels::{alloc, buffer_of, launch, read};
    const WIDE: usize = 16;
    /// The probe's parameters: the entry's offset in its block, the tape's and the lines' device addresses, the
    /// projection's nodes and fields.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ViewP {
        d: u64,
        tt: u64,
        ll: u64,
        n: u32,
        nf: u32,
    }
    unsafe impl rerail_gpu::Pod for ViewP {}
    const KERNEL: &str = r#"
// The tape and the lines are reached by their device addresses (a word each in the parameters, the buffers made
// resident): no binding.
typedef struct ViewP { uint64_t d; uint64_t tt; uint64_t ll; uint32_t n; uint32_t nf; } ViewP;
UKL_REFLECT_STRUCT(ViewP, d, tt, ll, n, nf)
DEFINE_KERNEL2(syntax_tests, view_probe, ROBUFFER, uint8_t, b, "[block]", ARGS, ViewP, p, , ROBUFFER, uint32_t, tv, "[29]",
               RWBUFFER, int64_t, out, "[n * 16]", GID0, uint32_t, g, )
{
    // A thread a (local, slot): each read is called once with its column a value, so the compiler makes one copy of
    // each (a column a constant, unrolled, had it make sixteen, each with 16z's decoder: minutes of compile on an M3).
    uint32_t l = g / 16u, k = g % 16u;
    if (l >= p.n) return;
    uint64_t d = p.d;
    uint32_t n = p.n, nf = p.nf;
    UKL_GLOBAL const uint8_t* tt = (UKL_GLOBAL const uint8_t*)p.tt;
    UKL_GLOBAL const uint8_t* ll = (UKL_GLOBAL const uint8_t*)p.ll;
    uint32_t v[29];
    for (uint32_t q = 0u; q < 29u; q++) v[q] = tv[q];
    int64_t r = 0;
    if (k < 5u) r = (int64_t)clif_view_node(b, d, tt, v, ll, l, k);
    else if (k < 8u) r = l + 1u < n ? (int64_t)clif_view_child(b, d, tt, v, ll, l, k - 5u) : 0;
    else if (k < 11u) r = l < nf ? (int64_t)clif_view_field(b, d, tt, v, ll, l, k - 8u) : 0;
    else if (k < 14u) r = (int64_t)clif_view_core(b, d, tt, v, ll, l, k - 11u);
    else r = (int64_t)clif_view_line(b, d, tt, v, ll, l, k - 14u);
    out[l * 16u + k] = r;
}
"#;
    let jit = rerail_clif::jit::Reads::new();
    // SAFETY: `view_node`'s signature: the entry, the tape's base, its view, the lines' base, a local, a column.
    let read_of = |name: &str| -> extern "C" fn(*const u8, *const u8, *const TapeView, *const u8, u32, u32) -> i64 { unsafe { std::mem::transmute(jit.function(name)) } };
    let (node, child, field, core_node, line) = (read_of("view_node"), read_of("view_child"), read_of("view_field"), read_of("view_core"), read_of("view_line"));
    // The process's device, where its buffers have device addresses (OpenCL 1.2's have none: 0440 step C).
    let probe = rerail_gpu::shared::get().filter(|_| alloc::<u8>(1).ok().and_then(|b| b.device_address()).is_some()).map(|shared| {
        let source = format!("#include <ukl/backend.h>\n#include <ukl/reads/view.h>\n{KERNEL}");
        let program = rerail_gpu::kernels::program("view_probe.uglykl", Box::leak(source.into_boxed_str())).unwrap_or_else(|e| panic!("the view reads compile: {e}"));
        (shared, program.function("view_probe").unwrap())
    });
    let cases: &[(&str, &str)] = &[
        ("python", "import os\nX = 1\n@dec\ndef f(a, b=2):\n    '''doc'''\n    return [x for x in a if x]\n\nclass C(B):\n    def m(self):\n        def inner():\n            pass\n        return inner\n"),
        ("typescript", "import { a } from 'b';\nexport const App = () => 1;\nclass K { x = 1; m(): number { return this.x; } }\nfunction g(n: number) { if (n) { return n; } }\n"),
    ];
    let anchors: BTreeSet<String> = ["call", "call_expression", "function_definition", "method_definition", "class_declaration"].iter().map(|s| (*s).to_owned()).collect();
    let mut tried = 0;
    for &(label, source) in cases {
        let lang = dialect_by_label(label).unwrap();
        let grammar = Grammar::new(lang).unwrap();
        let source = Arc::new(source.to_owned());
        let image = TapeImage::parse(&grammar, &source, &|| false).unwrap().unwrap().primary;
        for format in Format::ALL {
            let lines = Arc::new(LineTable::new(&source, Terminators::of(lang)));
            let words: Vec<u32> = lines.words().to_vec();
            let facts = Arc::new(core::from_image(Arc::clone(&source), &grammar, image.clone().in_format(format, source.as_bytes()), lines).unwrap());
            let dialect = Dialect::new(core::facts_tag(&facts)).unwrap();
            let (tape, view) = facts.tape.as_ref().unwrap().in_place().unwrap();
            // A 16z tape's reads decode its blocks into the reader's slots (0353 step 4b).
            let reader = rerail_clif::view::Reader::new(&view, tape.as_ptr());
            let v = reader.view();
            for (shape, narrowed) in [(Shape::Core, false), (Shape::T, false), (Shape::Core, true)] {
                let p = if narrowed { Projected::anchored(7, Arc::clone(&facts), Some(&dialect), shape, &anchors) } else { Projected::new(7, Arc::clone(&facts), Some(&dialect), shape, false, None) };
                let at = format!("{label} {} {shape:?}{}", format.name(), if narrowed { " narrowed" } else { "" });
                let block = p.ranks().block();
                let entry = view_docs::ViewEntry::new(block, &p, &dialect);
                let n = p.len();
                let nf = p.ranks().count(Item::Field);
                let parent = |l: u32| match p.parent(l as usize) {
                    NONE => -1,
                    q => p.global(q),
                };
                // Each local's rows: its `node` row, then `child` row `l` (of local `l + 1`), `field` row `l`, `core_node` row.
                let want: Vec<i64> = (0..n)
                    .flat_map(|l| {
                        let x = p.ranks().select(Item::Node, l).unwrap();
                        let c = l + 1;
                        let child = if c < n { [parent(c), p.global(c), i64::from(p.idx(c as usize))] } else { [0; 3] };
                        let field = if l < nf {
                            let fx = p.ranks().select(Item::Field, l).unwrap();
                            let fl = p.ranks().rank(Item::Node, fx);
                            [parent(fl), i64::from(field_atom(p.tag, facts.field(fx))), p.global(fl)]
                        } else {
                            [0; 3]
                        };
                        let mut row = vec![i64::from(p.key), p.global(l), i64::from(dialect.kind_of_atom(facts.kind(x))), i64::from(facts.start(x)), i64::from(facts.end(x))];
                        row.extend(child);
                        row.extend(field);
                        row.extend([i64::from(p.key), i64::from(x), p.global(l)]);
                        row.extend([p.global(l), i64::from(facts.start_line(x))]);
                        row
                    })
                    .collect();
                let e = entry.desc();
                let t = tape.as_ptr();
                let ll = words.as_ptr().cast::<u8>();
                let jitted: Vec<i64> = (0..n)
                    .flat_map(|l| {
                        let mut row: Vec<i64> = (0..5).map(|c| node(e, t, v, ll, l, c)).collect();
                        row.extend((0..3).map(|c| if l + 1 < n { child(e, t, v, ll, l, c) } else { 0 }));
                        row.extend((0..3).map(|c| if l < nf { field(e, t, v, ll, l, c) } else { 0 }));
                        row.extend((0..3).map(|c| core_node(e, t, v, ll, l, c)));
                        row.extend((0..2).map(|c| line(e, t, v, ll, l, c)));
                        row
                    })
                    .collect();
                assert_eq!(jitted, want, "{at}: the JIT's rows");
                if let Some((shared, probe)) = probe {
                    let d = block.offset_of(entry.desc()) as u64;
                    // The tape where the device reads it: a test's buffer of its bytes, a word of room past them.
                    let mut bytes = tape.to_vec();
                    bytes.resize(tape.len() + 16, 0);
                    let tt = buffer_of(&bytes).unwrap();
                    let out = alloc::<i64>(n.max(1) as usize * WIDE).unwrap();
                    // Its lines where the device reads them: a test's buffer of their words.
                    let lb = buffer_of(&words).unwrap();
                    let (tt_at, lb_at) = (tt.device_address().unwrap(), lb.device_address().unwrap());
                    // A 16z tape's blocks through a cache every thread shares (0353 step 4b): one of a block a
                    // generation, which turns all the time and may find no room (a failure counted: the device
                    // refuses the batch), and one with room.
                    for room in [1u32, 4, 64] {
                    let mut view = view;
                    let spec = rerail_clif::view::cache::Spec::new(3, room, view.columns[3] as usize).with_stats();
                    let cache = alloc::<u8>(spec.bytes()).unwrap();
                    if view.format == rerail_clif::view::F16Z_FORMAT {
                        // SAFETY: the buffer's bytes, shared with the host, not yet read by the device.
                        spec.init(unsafe { std::slice::from_raw_parts_mut(cache.contents().cast::<u8>(), spec.bytes()) }).unwrap();
                        let rel = cache.device_address().unwrap().wrapping_sub(tt_at) as i64;
                        (view.side[0], view.side[1], view.columns[3], view.columns[4]) = (rel as u32, (rel >> 32) as u32, spec.entry_bytes() as u32, 1);
                    }
                    // SAFETY: a `TapeView` is u32s alone, `#[repr(C)]`.
                    let view_words = unsafe { std::slice::from_raw_parts((&raw const view).cast::<u32>(), std::mem::size_of::<TapeView>() / 4) };
                    let tv = buffer_of(view_words).unwrap();
                    let mut q = ComputeQueue::new(&shared.queue);
                    launch(&q, probe)
                        .buffer("b", &block.device_buffer(&shared.device).unwrap())
                        .value("p", &ViewP { d, tt: tt_at, ll: lb_at, n, nf })
                        .buffer("tv", &tv)
                        .buffer("out", &out)
                        .uses(&tt)
                        .uses(&lb)
                        .uses(&cache)
                        .run(&[(u64::from(n) * 16).div_ceil(64)], &[64])
                        .unwrap_or_else(|e| panic!("{e}"));
                    q.flush().await_().unwrap();
                    let got: Vec<i64> = read(&out, want.len());
                    // SAFETY: the cache's header, its command completed.
                    let word = |at: i32| unsafe { *cache.contents().cast::<u8>().add(at as usize).cast::<u32>() };
                    if view.format == rerail_clif::view::F16Z_FORMAT {
                        // SAFETY: the cache's header, its command completed.
                        let counts = unsafe { rerail_clif::view::cache::Counts::read(cache.contents().cast::<u8>()) };
                        eprintln!("{at}, {room} a generation: the device's cache: {counts:?}, hit ratio {:.3}", counts.hit_ratio());
                    }
                    let failed = word(rerail_clif::view::cache::FAILED);
                    assert!(room == 1 || failed == 0, "{at}: the device's cache with room failed {failed} times");
                    if failed == 0 {
                        assert_eq!(got, want, "{at}, {room} a generation: the device's rows");
                    }
                    }
                }
                tried += 1;
            }
        }
    }
    assert_eq!(tried, 2 * 3 * 3, "every case, format and shape");
}

/// Decision 0494, R1 and R3: a batch read whole makes one group, whose relations hold the rows the eager batch's do;
/// one whose documents a probe of each made holds the same rows once scanned, over several groups.
#[test]
fn a_batch_made_by_group_holds_the_eager_batchs_rows() {
    let store = ContentStore::open(&temp("by-group")).unwrap();
    let docs = || -> Vec<Document> {
        (1..=4).map(|k| document(k, &format!("m{k}.py"), "python", &format!("def f{k}(x):\n    # c{k}\n    return g(x, {k}) + \"s{k}\"\n"))).collect()
    };
    let reads = ["node", "text", "comment", "op", "str", "name", "src_line", "line_count", "child", "line", "col", "visit_index", "ast_depth", "doc", "path_part", "file_name"];
    let projection = Projection { shape: Shape::Core, reads: Some(reads.iter().map(|r| (*r).to_owned()).collect()), ..Projection::core() };
    let rows = |s: &dyn Source| -> Vec<Vec<i64>> {
        let mut rows: Vec<Vec<i64>> = (0..s.len()).map(|r| (0..s.columns().len()).map(|c| s.value(r, c)).collect()).collect();
        rows.sort();
        rows
    };
    let (_, eager, _, _) = Batch::build(&store, docs(), &projection, &[], &[], false).unwrap();
    let eager: HashMap<&str, Vec<Vec<i64>>> = eager.iter().map(|(n, r)| (*n, rows(r))).collect();
    // Read whole from the first read: one group.
    let whole = Batch::new(&store, docs(), &projection, &[], &[], false, None, &HashMap::new()).unwrap();
    for (name, _, s) in made_sources(&whole) {
        assert_eq!(rows(&*s), eager[name], "{name}, read whole");
    }
    assert_eq!(whole.groups(), 1, "a batch read whole makes one group");
    // A probe of each document's key, then a scan: three groups.
    let probed = Batch::new(&store, docs(), &projection, &[], &[], false, None, &HashMap::new()).unwrap();
    let sources = made_sources(&probed);
    let doc = Arc::clone(&sources.iter().find(|(n, _, _)| *n == "doc").unwrap().2);
    for k in [3, 1] {
        let found = doc.lookup(0, &[k]).unwrap();
        assert_eq!(found.iter().map(|&r| doc.value(r, 0)).collect::<Vec<_>>(), vec![k], "document {k}'s row");
    }
    assert_eq!(probed.groups(), 2, "each probe made its document's group");
    for (name, _, s) in &sources {
        assert_eq!(rows(&**s), eager[name], "{name}, made by probes then a scan");
    }
    assert_eq!(probed.groups(), 3, "the scan made the rest as one group");
}

/// Decision 0494 (Q396): reads of a batch side by side make each document once. Each thread reads a relation whole;
/// a read finding documents another is making waits for them, so every group made is kept, none made again and set
/// aside (the base relations counted on the pool made the whole batch once each).
#[test]
fn reads_side_by_side_make_each_document_once() {
    let store = ContentStore::open(&temp("side-by-side")).unwrap();
    let docs: Vec<Document> = (1..=48).map(|k| document(k, &format!("m{k}.py"), "python", &format!("def f{k}(x):\n    # c{k}\n    return g(x, {k}) + \"s{k}\"\n"))).collect();
    let reads = ["node", "text", "comment", "op", "str", "name", "line", "doc"];
    let projection = Projection { shape: Shape::Core, reads: Some(reads.iter().map(|r| (*r).to_owned()).collect()), ..Projection::core() };
    let batch = Batch::new(&store, docs, &projection, &[], &[], false, None, &HashMap::new()).unwrap();
    let sources = made_sources(&batch);
    let start = std::sync::Barrier::new(sources.len());
    let read: Vec<usize> = std::thread::scope(|scope| {
        let threads: Vec<_> = sources
            .iter()
            .map(|(_, _, s)| {
                let start = &start;
                scope.spawn(move || {
                    start.wait();
                    (0..s.len()).filter(|&r| s.value(r, 0) >= 0).count()
                })
            })
            .collect();
        threads.into_iter().map(|t| t.join().unwrap()).collect()
    });
    assert!(read.iter().sum::<usize>() > 0);
    assert!(batch.groups() >= 1);
    assert_eq!(batch.makes(), batch.groups(), "no group made again and set aside");
}

/// Decision 0494, R2: a probe of a relation by a node makes that node's document alone, its projection and its rows.
#[test]
fn a_probe_makes_the_document_it_names_alone() {
    let store = ContentStore::open(&temp("probe-one")).unwrap();
    let docs: Vec<Document> = (1..=5).map(|k| document(k, &format!("m{k}.py"), "python", &format!("x{k} = y({k})\n"))).collect();
    let projection = Projection { shape: Shape::Core, reads: Some(["text".to_owned(), "op".to_owned()].into()), ..Projection::core() };
    let batch = Batch::new(&store, docs, &projection, &[], &[], false, None, &HashMap::new()).unwrap();
    let sources = made_sources(&batch);
    assert_eq!(batch.docs.made_count(), 0, "nothing is made before a read");
    let n = batch.docs[2].global(1);
    let text = Arc::clone(&sources.iter().find(|(n, _, _)| *n == "text").unwrap().2);
    let found = text.lookup(0, &[n]).unwrap();
    assert!(!found.is_empty(), "the node's text");
    assert!(found.iter().all(|&r| text.value(r, 0) == n));
    assert_eq!(batch.docs.made_count(), 1, "only the probed document is made");
    assert_eq!(batch.groups(), 1);
    assert!(text.failure().is_none());
}

/// Decision 0494, R5: a batch given its documents' views' counts and its relations' rows knows every view's and
/// relation's `len` with no document made; a probe then makes its document alone. A made document whose counts differ
/// from those given fails, named, and so does a relation whose made rows are not the count given.
#[test]
fn a_batch_given_counts_knows_its_row_space_making_nothing() {
    let store = ContentStore::open(&temp("given-counts")).unwrap();
    let docs = || -> Vec<Document> {
        (1..=4).map(|k| document(k, &format!("m{k}.py"), "python", &format!("def f{k}(x, y={k}):\n    return g(x)\n\nclass C{k}:\n    pass\n"))).collect()
    };
    let reads = ["node", "child", "field", "next_sibling", "text", "op", "line", "doc"];
    let projection = Projection { shape: Shape::Core, reads: Some(reads.iter().map(|r| (*r).to_owned()).collect()), ..Projection::core() };
    let lens = |batch: &Arc<Batch>| -> BTreeMap<String, usize> {
        let mut out: BTreeMap<String, usize> = made_sources(batch).into_iter().map(|(n, _, s)| (n.to_owned(), s.len())).collect();
        out.extend(views(batch).into_iter().map(|(n, s)| (n.to_owned(), s.len())));
        out
    };
    // The counts, as a batch made eagerly has them.
    let eager = Batch::new(&store, docs(), &projection, &[], &[], false, None, &HashMap::new()).unwrap();
    let want = lens(&eager);
    let counts: HashMap<u32, made::ViewCounts> = eager.view_counts().into_iter().collect();
    // Every relation's rows, as the run gives them (their kept statistics'), the families' among them.
    let totals: HashMap<String, usize> = want.iter().map(|(n, &l)| (n.clone(), l)).collect();
    assert_eq!(counts.len(), 4);
    // Given them: every `len`, nothing made.
    let lazy = Batch::new(&store, docs(), &projection, &[], &[], false, None, &counts).unwrap();
    lazy.give_totals(&totals);
    assert!(lazy.lazy(), "a batch given every document's counts is made by its reads");
    assert_eq!(lens(&lazy), want, "the lens, from the counts");
    assert_eq!(lazy.docs.made_count(), 0, "no document made for the row space or a len");
    let text = Arc::clone(&made_sources(&lazy).into_iter().find(|(n, _, _)| *n == "text").unwrap().2);
    let n = lazy.docs[2].global(1);
    assert!(!text.lookup(0, &[n]).unwrap().is_empty());
    assert_eq!(lazy.docs.made_count(), 1, "the probe made its document alone");
    assert!(text.failure().is_none(), "{:?}", text.failure());
    // A document whose counts are not those given: it fails as it is made, named.
    let mut wrong = counts.clone();
    wrong.get_mut(&2).unwrap()[0] += 1;
    let off = Batch::new(&store, docs(), &projection, &[], &[], false, None, &wrong).unwrap();
    let text = Arc::clone(&made_sources(&off).into_iter().find(|(n, _, _)| *n == "text").unwrap().2);
    let _ = text.lookup(0, &[off.docs[1].global(1)]);
    let failed = text.failure().expect("a document's counts differ");
    assert!(failed.contains("m2.py") && failed.contains("0494"), "{failed}");
    // A relation whose rows, every document made, are not the count given: named.
    let mut wrong = totals.clone();
    *wrong.get_mut("text").unwrap() += 1;
    let off = Batch::new(&store, docs(), &projection, &[], &[], false, None, &counts).unwrap();
    off.give_totals(&wrong);
    let text = Arc::clone(&made_sources(&off).into_iter().find(|(n, _, _)| *n == "text").unwrap().2);
    let mut seen = 0;
    text.for_rows(&[0], &mut |_, _| seen += 1);
    let failed = text.failure().expect("a relation's rows differ from its count");
    assert!(failed.contains("text") && failed.contains("0494"), "{failed}");
}

/// A tape value admitted into the batch (decision 0431 §8) runs no `ast` matcher: the files the batch was started
/// with are matched once, and are not matched again (a value alone is no file, `Grower::admit`). Each round of guests
/// ran the wave over every file of the chunk again, 17 s of 131 s of CPU on `all` once the reference graph's rules
/// read string annotations (decision 0483), before the rounds went with the embed slice (0431 step 7).
#[test]
fn a_value_admitted_into_the_batch_matches_no_file_again() {
    struct Seen(std::sync::Mutex<Vec<String>>);
    impl AstWave for Seen {
        fn len(&self) -> usize {
            1
        }
        fn matched(
            &self,
            tapes: &[(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)],
            _: &[Option<&Projected>],
            _: crate::code_rule_dsl::facts::nearest::Binding,
            _: &crate::content_store::atoms::Atoms,
        ) -> StoreResult<Vec<(String, Relation)>> {
            self.0.lock().unwrap().extend(tapes.iter().map(|(_, _, path)| path.clone()));
            // A node of each document matched: its key (the test's path `m{key}.py`) above its local id.
            let mut rows = Relation::new(&["n"]);
            for (_, _, path) in tapes {
                let key: i64 = path.trim_start_matches('m').trim_end_matches(".py").parse().unwrap();
                rows.push(&[key << 32 | 1]);
            }
            Ok(vec![("rr_ast_1_1".to_owned(), rows)])
        }
    }
    let store = ContentStore::open(&temp("grown-matchers")).unwrap();
    let wave = Arc::new(Seen(std::sync::Mutex::new(Vec::new())));
    let matchers = Matchers::new(Arc::clone(&wave) as Arc<dyn AstWave>);
    let tests = Arc::new(crate::code_rule_dsl::logic::strings::Vocabulary::default());
    let binding = crate::code_rule_dsl::facts::nearest::Binding::Cpu;
    let projection = Projection::core();
    let file = |k: u32| document(k, &format!("m{k}.py"), "python", &format!("def f{k}(x):\n    return x\n"));
    let read = |sources: &[(String, Arc<dyn Source>)]| {
        let (_, matched) = sources.iter().find(|(n, _)| n == "rr_ast_1_1").expect("the matchers' relation is bound");
        (0..matched.len()).for_each(|r| _ = std::hint::black_box(matched.value(r, 0)));
    };
    let (sources, mut grower) = Grower::start(&store, vec![file(1), file(2)], &[], &matchers, &tests, binding, &projection, &HashMap::new()).unwrap();
    read(&sources);
    let admitted = grower.admit(&store, file(3), &[], &tests, binding, &projection).unwrap();
    assert!(!admitted.iter().any(|(n, _)| n == "rr_ast_1_1"), "a value's sources hold no matchers' rows of their own");
    let mut seen = wave.0.lock().unwrap().clone();
    seen.sort();
    assert_eq!(seen, ["m1.py", "m2.py"], "the started files matched once, and none again when a value is admitted");
}

/// Decision 0353, step 3 on the CPU (step 1): a batch's views' directory is made with or without a device, and
/// self-relative. Read from its offset in the batch's block, at the host's bases of its homes, each document's row
/// names that document's entry, its tape's bytes and its lines' words where they lie. The device's address of it is
/// given only where the device reads every home.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn the_views_directory_is_the_hosts_too() {
    use rerail_clif::view_reads::{ViewDir, ViewDoc, ViewHome, ViewRow};
    let store = ContentStore::open(&temp("views-dir-host")).unwrap();
    let docs: Vec<Document> = (1..=3).map(|k| document(k, &format!("m{k}.py"), "python", &format!("def f{k}(x):\n    return g(x, {k})\n"))).collect();
    let projection = Projection { shape: Shape::Core, reads: Some(["node".to_owned(), "child".to_owned()].into()), ..Projection::core() };
    let batch = Batch::new(&store, docs, &projection, &[], &[], false, None, &HashMap::new()).unwrap();
    let chunk = batch.chunk();
    let views = device_chunk::views_in_place(&batch, chunk).expect("a directory, device or not");
    let block = &chunk.block;
    // SAFETY: the directory, its rows and its homes, in the batch's block, which the batch holds.
    let dir = unsafe { &*block.ptr::<ViewDir>(views.at) };
    assert_eq!(dir.at as usize, views.at, "the directory's own offset");
    assert_eq!(dir.ndocs as usize, batch.docs.len());
    let rows = unsafe { std::slice::from_raw_parts(block.ptr::<ViewRow>(dir.rows as usize), dir.ndocs as usize) };
    let homes = unsafe { std::slice::from_raw_parts(block.ptr::<ViewHome>(dir.homes as usize), dir.nhomes as usize) };
    for (row, p) in rows.iter().zip(batch.docs.iter()) {
        let entry = unsafe { &*block.ptr::<ViewDoc>(row.entry as usize) };
        assert_eq!(entry.key, p.key, "the row's entry is its document's");
        let (bytes, view) = p.facts.tape.as_ref().unwrap().in_place().unwrap();
        assert_eq!(homes[row.homes[0] as usize].host + row.tape, bytes.as_ptr() as u64, "document {}'s tape, where it lies", p.key);
        assert_eq!((row.view.format, row.view.nodes), (view.format, view.nodes));
        assert_eq!(homes[row.homes[1] as usize].host + row.lines, p.facts.lines.words().as_ptr() as u64, "document {}'s lines", p.key);
    }
    let all_on_device = crate::code_rule_dsl::tape::placed::device().is_some() && homes.iter().all(|h| h.gpu != 0);
    assert_eq!(views.dir.is_some(), all_on_device, "the device's address only where it reads every home");
}

/// Decision 0353, step 3 on the CPU (step 2): the directory's searches written directly in Rust (`ViewDir::doc_of_row`,
/// `doc_of_key`) answer what their CLIF answers, by the JIT, for every row of every view, every key and a key of
/// none, with a memo of the right document, of a wrong one, and of none. Documents of no rows in a view (a module's
/// docstring alone) are among them.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn the_directorys_searches_are_the_jits() {
    use rerail_clif::view_reads::{NO_DOC, VIEWS, ViewDir};
    let store = ContentStore::open(&temp("views-dir-search")).unwrap();
    let mut docs: Vec<Document> = (1..=5).map(|k| document(k * 3, &format!("m{k}.py"), "python", &format!("def f{k}(x):\n    return g(x, {k})\n"))).collect();
    docs.push(document(40, "m40.py", "python", "\"\"\"only a docstring\"\"\"\n"));
    let projection = Projection { shape: Shape::Core, reads: Some(["node".to_owned(), "child".to_owned(), "field".to_owned()].into()), ..Projection::core() };
    let batch = Batch::new(&store, docs, &projection, &[], &[], false, None, &HashMap::new()).unwrap();
    let chunk = batch.chunk();
    let views = device_chunk::views_in_place(&batch, chunk).expect("a directory");
    let jit = rerail_clif::jit::Reads::new();
    type ByRow = extern "C" fn(*const u8, u32, u32, u32) -> u32;
    type ByKey = extern "C" fn(*const u8, u32, u32) -> u32;
    // SAFETY: each read's signature: the descriptor, then its arguments.
    let (by_row, by_key): (ByRow, ByKey) = unsafe { (std::mem::transmute(jit.function("dir_doc_of_row")), std::mem::transmute(jit.function("dir_doc_of_key"))) };
    let d = chunk.block.ptr::<u8>(views.at).cast_const();
    // SAFETY: the directory in the batch's block, which the batch holds.
    let dir = unsafe { &*d.cast::<ViewDir>() };
    let rows = unsafe { dir.rows() };
    let n = dir.ndocs;
    assert_eq!(n, 6);
    let mut tried = 0;
    for k in 0..VIEWS.len() as u32 {
        let last = rows.last().map_or(0, |r| r.rows[k as usize]) + 4;
        for r in 0..last {
            for memo in [0, n / 2, n - 1, n, NO_DOC] {
                // SAFETY: as `dir`.
                let host = unsafe { dir.doc_of_row(k, r, memo) };
                assert_eq!(by_row(d, k, r, memo), host, "view {k}, row {r}, memo {memo}");
                let next = rows.get(host as usize + 1).map_or(u32::MAX, |e| e.rows[k as usize]);
                assert!(rows[host as usize].rows[k as usize] <= r && r < next || host + 1 == n, "view {k}, row {r}: document {host}");
                tried += 1;
            }
        }
    }
    for key in 0..45 {
        for memo in [0, 2, n, NO_DOC] {
            // SAFETY: as `dir`.
            let host = unsafe { dir.doc_of_key(key, memo) };
            assert_eq!(by_key(d, key, memo), host, "key {key}, memo {memo}");
            let expect = batch.docs.iter().position(|p| p.key == key).map_or(NO_DOC, |i| i as u32);
            assert_eq!(host, expect, "key {key}");
            tried += 1;
        }
    }
    assert!(tried > 100);
}

/// Decision 0353, step 3 on the CPU (step 2): the directory's searches in the kernels (`clif_dir_doc_of_row`,
/// `clif_dir_doc_of_key`, the UKL of their CLIF), over the directory where it lies in its batch's block, answer what
/// the host's answer: every row of every view, every key and keys of none, with each memo. The kernel is UKL, on the
/// process's device (decision 0440 step C).
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn the_kernels_directory_searches_are_the_hosts() {
    use rerail_clif::view_reads::{NO_DOC, VIEWS, ViewDir};
    use rerail_gpu::device::ComputeQueue;
    use rerail_gpu::kernels::{alloc, launch, read};
    let Some(shared) = rerail_gpu::shared::get() else { return };
    /// A probe's arguments, each search's: the directory's offset in its block, then (view or key, row, memo) by
    /// thread.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct DirP {
        d: u64,
        n: u32,
        pad: u32,
    }
    unsafe impl rerail_gpu::Pod for DirP {}
    const KERNEL: &str = r#"
typedef struct DirP { uint64_t d; uint32_t n; uint32_t pad; } DirP;
UKL_REFLECT_STRUCT(DirP, d, n, pad)
DEFINE_KERNEL2(syntax_tests, dir_probe, ROBUFFER, uint8_t, b, "[block]", ARGS, DirP, p, , ROBUFFER, uint32_t, ask, "[n * 4]",
               RWBUFFER, uint32_t, out, "[n]", GID0, uint32_t, x, )
{
    if (x >= p.n) return;
    UKL_GLOBAL const uint32_t* a = ask + x * 4u;
    out[x] = a[0] == 0u ? clif_dir_doc_of_row(b, p.d, a[1], a[2], a[3]) : clif_dir_doc_of_key(b, p.d, a[1], a[3]);
}
"#;
    let store = ContentStore::open(&temp("views-dir-kernel")).unwrap();
    let mut docs: Vec<Document> = (1..=5).map(|k| document(k * 3, &format!("m{k}.py"), "python", &format!("def f{k}(x):\n    return g(x, {k})\n"))).collect();
    docs.push(document(40, "m40.py", "python", "\"\"\"only a docstring\"\"\"\n"));
    let projection = Projection { shape: Shape::Core, reads: Some(["node".to_owned(), "child".to_owned(), "field".to_owned()].into()), ..Projection::core() };
    let batch = Batch::new(&store, docs, &projection, &[], &[], false, None, &HashMap::new()).unwrap();
    let chunk = batch.chunk();
    let views = device_chunk::views_in_place(&batch, chunk).expect("a directory");
    // SAFETY: the directory in the batch's block, which the batch holds.
    let dir = unsafe { &*chunk.block.ptr::<ViewDir>(views.at) };
    let rows = unsafe { dir.rows() };
    let n = dir.ndocs;
    let mut asks: Vec<[u32; 4]> = Vec::new();
    for k in 0..VIEWS.len() as u32 {
        for r in 0..rows.last().map_or(0, |e| e.rows[k as usize]) + 4 {
            for memo in [0, n / 2, n - 1, n, NO_DOC] {
                asks.push([0, k, r, memo]);
            }
        }
    }
    for key in 0..45 {
        for memo in [0, 2, n, NO_DOC] {
            asks.push([1, key, 0, memo]);
        }
    }
    // SAFETY: as `dir`.
    let want: Vec<u32> = asks.iter().map(|&[op, a, r, memo]| unsafe { if op == 0 { dir.doc_of_row(a, r, memo) } else { dir.doc_of_key(a, memo) } }).collect();
    let source = format!("#include <ukl/backend.h>\n#include <ukl/reads/relation.h>\n{KERNEL}");
    let program = rerail_gpu::kernels::program("dir_probe.uglykl", Box::leak(source.into_boxed_str())).unwrap_or_else(|e| panic!("the directory's searches compile: {e}"));
    let probe = program.function("dir_probe").unwrap();
    let m = asks.len() as u32;
    let flat: Vec<u32> = asks.iter().flatten().copied().collect();
    let ask = rerail_gpu::kernels::buffer_of(&flat).unwrap();
    let out = alloc::<u32>(asks.len()).unwrap();
    let mut q = ComputeQueue::new(&shared.queue);
    launch(&q, probe)
        .buffer("b", &chunk.block.device_buffer(&shared.device).unwrap())
        .value("p", &DirP { d: views.at as u64, n: m, pad: 0 })
        .buffer("ask", &ask)
        .buffer("out", &out)
        .run(&[(u64::from(m)).div_ceil(64)], &[64])
        .unwrap_or_else(|e| panic!("{e}"));
    q.flush().await_().unwrap();
    let got: Vec<u32> = read(&out, asks.len());
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(g, w, "ask {:?}", asks[i]);
    }
    assert!(m > 100);
}
