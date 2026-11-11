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
/// The corpus as a batch, document `d` keyed `d`.
pub(in crate::code_rule_dsl) fn documents() -> Vec<Document> {
    corpus()
        .into_iter()
        .enumerate()
        .map(|(d, (path, source))| {
            let facts = facts(&path, &source);
            Document { key: d as u32, core: format!("/w#core:{path}"), digest: core::digest(&facts), path, facts, guest: None, module: None, tape: None }
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
            (i64::from(d.key), Projected::new(d.key, Arc::clone(&d.facts), &dialect, Shape::T, false, None))
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
        // T's `op` is `op_t` here (O79): `op` is `core.rr`'s, every anonymous token.
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
        // `+` on every column reference hides the constraint from xBestIndex: SQLite scans and filters.
        let hidden = regex::Regex::new(r"\b(n|p|c|d|kind|par|idx) ").unwrap().replace_all(&filter, "+$1 ");
        let scanned = key(&format!("SELECT * FROM {rel} WHERE {hidden}"));
        assert!(!indexed.is_empty() || filter.contains("0, 7"), "{rel} {filter}: no rows");
        assert_eq!(indexed, scanned, "{rel} {filter}");
    }
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

/// One document of `source` in `label`'s dialect, keyed `key`.
fn document(key: u32, path: &str, label: &str, source: &str) -> Document {
    let file = FileInput::new(path.to_owned(), label.to_owned(), Arc::new(source.to_owned()), None, None, "core").unwrap();
    let Built::Facts(facts) = core::build(&file).unwrap() else { panic!("{path} did not parse") };
    let facts = Arc::new(facts);
    Document { key, core: format!("/w#core:{path}"), digest: core::digest(&facts), path: path.to_owned(), facts, guest: None, module: None, tape: None }
}

/// `core.rr`'s relations of `docs`, each row as text: a node as `kind'text'`, a span as `'text'`, a symbol as itself.
fn core_rows(docs: Vec<Document>) -> HashMap<String, BTreeSet<String>> {
    let store = ContentStore::open(&temp("core")).unwrap();
    let sources = shaped(&store, docs, &Projection::core()).unwrap();
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

/// `core.rr`'s base relations over a Python file (decision 0046): comments are nodes and children, `text` is every
/// node's, `op` every anonymous token's, literals decode, and the source-text, token and place relations agree.
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
    has("op", "function_definition def");
    has("op", "argument_list'(*a, f\"v{a}w\", 0xff)' ,");
    lacks("op_at", "argument_list'(*a, f\"v{a}w\", 0xff)' 0 ,");
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
    has("first_token", "function_definition 'def'");
    has("last_token", "return_statement'return x' identifier'x'");
    has("token_after", "identifier'f' '('");
    has("token_before", "identifier'f' 'def'");
}

/// The shape follows the program: one that includes `core.rr` reads `core.rr`'s, one that declares its base
/// relations itself (T's, the converter's) T's.
#[test]
fn programs_that_include_core_rr_read_its_shape() {
    let core = std::fs::read_to_string(crate::code_rule_dsl::logic::rr::library().join("core.rr")).unwrap();
    let with = crate::code_rule_dsl::logic::rr::parse(&format!("{core}\n.decl q(n: node)\nq(n) :- comment(n, _).\n.output q\n")).unwrap();
    assert_eq!(Projection::of(&with, BTreeSet::new()).shape, Shape::Core);
    let without = crate::code_rule_dsl::logic::rr::parse(".decl text(n: number, s: symbol)\n.input text\n.decl q(n: number)\nq(n) :- text(n, _).\n.output q\n").unwrap();
    assert_eq!(Projection::of(&without, BTreeSet::new()).shape, Shape::T);
}

/// End to end through the corpus runner, a program that includes `core.rr`: its options, `@dot` with an empty head,
/// `@lower` in Unicode, and comments as `core.rr`'s nodes.
#[test]
fn core_rr_programs_read_options_and_functors() {
    use crate::code_rule_dsl::logic::plan::corpus;
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("core-rr");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "# hello\nx = 1\n").unwrap();
    let core = std::fs::read_to_string(crate::code_rule_dsl::logic::rr::library().join("core.rr")).unwrap();
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
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(read("dotted"), "a.b\nb\n");
    assert_eq!(read("low"), "àb\n");
    assert_eq!(read("item"), "0\tx\n1\ty\n");
    assert_eq!(read("said"), "hello\n");
    let _ = std::fs::remove_dir_all(&dir);
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
        docs.push(Document { key: (10 + at) as u32, core: format!("/w#core:{path}"), digest: core::digest(&facts), path, facts, guest: None, module: None, tape: None });
    }
    let (batch, _, _) = Batch::build(&store, docs, &Projection::default(), false).unwrap();
    let nodes = *batch.nodes.last().unwrap();
    let id = |r: usize| batch.id(r);
    // Borrow the demanded columns once, as a bulk consumer does. Keep lazy initialization
    // out of the exhaustive reference loop; that loop checks the relation, not setup cost.
    let axes: Vec<(&[u32], &[u32])> = batch.docs.iter().map(|p| (p.sizes(), p.depths())).collect();
    // (document, local, size, depth, start, end) of node row r.
    let at = |r: usize| {
        let doc = batch.nodes.partition_point(|&o| o <= r) - 1;
        let (p, local) = (&batch.docs[doc], r - batch.nodes[doc]);
        let core = p.core[local] as usize;
        (p.key, local, axes[doc].0[local] as usize, axes[doc].1[local] as usize, p.facts.start[core], p.facts.end[core])
    };
    let zero = (0..nodes).filter(|&r| at(r).4 == at(r).5).count();
    assert!(zero > 0, "the broken documents have nodes of zero width");
    let cpu = || { let (u, s) = crate::code_rule_dsl::logic::plan::corpus::thread_usage(); u + s };
    let (mut reference_cpu, mut relation_cpu) = (0.0, 0.0);
    for mode in [within::Mode::Tree, within::Mode::Span] {
        let before_reference = cpu();
        let within = within::Within::new(Arc::clone(&batch), mode);
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
        let before_relation = cpu();
        let rows = |rows: Vec<usize>| -> BTreeSet<(i64, i64)> { rows.into_iter().map(|r| (within.value(r, 0), within.value(r, 1))).collect() };
        let ids: Vec<i64> = (0..nodes).map(id).collect();
        // By outer first (half the nodes), then by inner, then the scan: rows made by one lookup serve the others.
        let half: BTreeSet<i64> = ids[..nodes / 2].iter().copied().collect();
        assert_eq!(rows(within.lookup(1, &ids[..nodes / 2]).unwrap()), expected.iter().filter(|(_, b)| half.contains(b)).copied().collect());
        assert_eq!(rows(within.lookup(0, &ids).unwrap()), expected);
        assert_eq!(rows(within.lookup(1, &ids).unwrap()), expected);
        assert_eq!(rows((0..within.len()).collect()), expected);
        // Each node's containers are made once: the inner side's rows are exactly the pairs.
        assert_eq!(within.lookup(0, &ids).unwrap().len(), expected.len());
        let own = usize::from(mode == within::Mode::Span);
        assert_eq!(within.estimated_len(), (0..nodes).map(|r| at(r).3 + own).sum::<usize>());
        if mode == within::Mode::Tree {
            assert_eq!(within.len(), expected.len(), "the tree's rows are its estimate");
        }
        relation_cpu += cpu() - before_relation;
    }
    eprintln!("CONTAINMENT CHECK: {nodes} nodes, reference {reference_cpu:.6} CPU s, relation and comparisons {relation_cpu:.6} CPU s");
}

/// The kind-set axes (`within::Axis`: `nearest`, `inside`, `has`) hold exactly the pairs a brute-force walk of each
/// node's strict ancestors gives, looked up by either column and scanned, and their statistics count the rows.
#[test]
fn the_kind_set_axes_hold_exactly_their_pairs() {
    use crate::code_rule_dsl::facts::core::kind_atom;
    let store = ContentStore::open(&temp("axes")).unwrap();
    let docs: Vec<Document> = documents().into_iter().take(4).collect();
    let (batch, _, _) = Batch::build(&store, docs, &Projection::default(), false).unwrap();
    let nodes = *batch.nodes.last().unwrap();
    let id = |r: usize| batch.id(r);
    // (document, local, size) and kind atom of node row r.
    let at = |r: usize| {
        let (p, local) = batch.locate(View::Node, r);
        (p.key, local, p.size(local) as usize)
    };
    let kind = |r: usize| {
        let (p, local) = batch.locate(View::Node, r);
        let code = crate::code_rule_dsl::facts::core::kind_code(p.facts.kind[p.core[local] as usize]);
        i64::from(kind_atom(p.tag, batch.dialects[&p.tag].canonical.get(usize::from(code)).copied().unwrap_or(code)))
    };
    // Nested kinds: the root's, and two from inside the documents.
    let set: Vec<i64> = [0, nodes / 3, nodes / 2].map(kind).to_vec();
    let member = |r: usize| set.contains(&kind(r));
    let below = |n: usize, a: usize| {
        let ((dn, ln, _), (da, la, za)) = (at(n), at(a));
        dn == da && la < ln && ln <= la + za
    };
    use crate::code_rule_dsl::typing::file_local::Binding;
    // Each axis made per looked-up node, and with the batch's solve chosen (decision 0105) on the CPU and on Metal.
    let ways = [(Binding::Cpu, false), (Binding::Cpu, true), (Binding::Metal, true)];
    for (axis, (binding, solve)) in [within::Axis::Nearest, within::Axis::Inside, within::Axis::Has].into_iter().flat_map(|a| ways.map(|w| (a, w))) {
        let source = within::Within::axis(Arc::clone(&batch), axis, &set, binding);
        if solve {
            source.expect_lookups(0, 1e12);
        }
        let mut expected = BTreeSet::new();
        for n in 0..nodes {
            for a in 0..nodes {
                let holds = below(n, a)
                    && match axis {
                        within::Axis::Inside => member(a),
                        within::Axis::Has => member(n),
                        within::Axis::Nearest => member(a) && !(0..nodes).any(|c| c != a && member(c) && below(n, c) && below(c, a)),
                    };
                if holds {
                    expected.insert((id(n), id(a)));
                }
            }
        }
        assert!(!expected.is_empty());
        let rows = |rows: Vec<usize>| -> BTreeSet<(i64, i64)> { rows.into_iter().map(|r| (source.value(r, 0), source.value(r, 1))).collect() };
        let ids: Vec<i64> = (0..nodes).map(id).collect();
        let half: BTreeSet<i64> = ids[..nodes / 2].iter().copied().collect();
        assert_eq!(rows(source.lookup(1, &ids[..nodes / 2]).unwrap()), expected.iter().filter(|(_, a)| half.contains(a)).copied().collect());
        assert_eq!(rows(source.lookup(0, &ids).unwrap()), expected);
        assert_eq!(rows(source.lookup(1, &ids).unwrap()), expected);
        assert_eq!(rows((0..source.len()).collect()), expected);
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
fn python_grammar_verdict_is_demanded_and_shared_across_projections() {
    let store = ContentStore::open(&temp("python-verdict")).unwrap();
    let doc = document(0, "a.py", "python", "print \"legacy\"\n");
    let source_only = Projection { reads: Some(["source".into(), "text".into()].into()), ..Projection::core() };
    shaped(&store, vec![doc.clone()], &source_only).unwrap();
    assert!(doc.facts.python_syntax.get().is_none(), "raw text must not request grammar validation");
    let ast = Projection { reads: Some(["syntax_error".into()].into()), ..Projection::core() };
    shaped(&store, vec![doc.clone()], &ast).unwrap();
    let verdict = doc.facts.python_syntax.get().expect("demanded verdict");
    assert!(verdict.is_some());
    shaped(&store, vec![doc.clone()], &ast).unwrap();
    assert!(std::ptr::eq(verdict, doc.facts.python_syntax.get().unwrap()), "the tape owns one shared verdict");
}

#[test]
fn python_exception_rows_share_the_demanded_verdict_cache() {
    let store=ContentStore::open(&temp("python-exception")).unwrap();
    let bad=document(3,"bad.py","python","def broken(:\n");
    let good=document(4,"good.py","python","pass\n");
    let projection=Projection {reads:Some(["syntax_exception".into()].into()),..Projection::core()};
    let rows=shaped(&store,vec![bad.clone(),good.clone()],&projection).unwrap();
    let values=&rows.iter().find(|(name,_)|*name=="syntax_exception").unwrap().1;
    assert_eq!(values.len(),1);
    assert_eq!(values.value(0,0),3);
    assert_eq!(store.atom_value(values.value(0,1) as u32).unwrap().unwrap(),"SyntaxError");
    let location=store.atom_value(values.value(0,3) as u32).unwrap().unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&location).unwrap(),serde_json::json!([1,12,"def broken(:\n",1,13]));
    let cached=bad.facts.python_syntax.get().unwrap();
    assert!(cached.is_some());assert!(good.facts.python_syntax.get().unwrap().is_none());
    let verdict=Projection {reads:Some(["syntax_error".into()].into()),..Projection::core()};
    shaped(&store,vec![bad.clone()],&verdict).unwrap();
    assert!(std::ptr::eq(cached,bad.facts.python_syntax.get().unwrap()));
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
    let prelude = std::fs::read_to_string(crate::code_rule_dsl::logic::rr::library().join("core.rr")).unwrap();
    for (body, source_needed, line_needed) in [
        ("out(t) :- source(_, sp), kind(n, \"identifier\"), text(n, t).", false, false),
        (".decl copied(sp: span)\ncopied(s) :- source(_, s).\nout(t) :- copied(s), alias = s, text(alias, t).", true, false),
        (".decl copied(sp: span)\ncopied(s) :- src_line(_, _, s).\nout(t) :- copied(s), text(s, t).", false, true),
        (".decl copied(sp: span)\ncopied(s) :- source(_, s).\ncopied(s) :- src_line(_, _, s).\nout(t) :- copied(s), text(s, t).", true, true),
        ("out(\"missing\") :- source(_, sp), !text(sp, _).", true, false),
        ("out(t) :- source(_, sp), match_all(\"(?<hit>items)\", sp), text(hit, t).", false, false),
    ] {
        let program = crate::code_rule_dsl::logic::rr::parse(&format!("{prelude}\n.decl out(t: symbol)\n{body}\n.output out\n")).unwrap();
        let projection = Projection::of(&program, ["source", "src_line", "text", "rx_span"].into_iter().map(str::to_owned).collect());
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
    let (batch,relations,(strings,_))=Batch::build(&store,docs.clone(),&projection,false).unwrap();
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
    let (full,_,_)=Batch::build(&store,docs,&Projection::core(),false).unwrap();
    assert_eq!(all,full.python_candidates(&[1,2],&targets,"async_function","open").unwrap(),"projection must preserve node identity");
}

#[test]
fn projection_axes_and_token_spans_are_demanded_without_renumbering() {
    let f = facts("a.py", "f(a + 1)\ng(b)\n");
    let dialect = Dialect::new(tape_tag(GrammarDialect::Python).unwrap()).unwrap();
    let p = Projected::new(7, Arc::clone(&f), &dialect, Shape::Core, false, None);
    assert_eq!(p.core.capacity(),p.core.len());
    assert!(p.idx.get().is_none() && p.size.get().is_none() && p.depth.get().is_none() && p.fielded.get().is_none());
    assert!(p.extra.as_ref().unwrap().token_spans.get().is_none());
    // Independent eager reference: tokens grouped by the named parent's preorder, then source and lines.
    let mut expected = Vec::new();
    for &id in &p.core {
        for &c in f.children(id) {
            if !f.named(c) && f.end[c as usize] > f.start[c as usize] { expected.push([f.start[c as usize], f.end[c as usize]]); }
        }
    }
    let tokens = expected.len();
    expected.push([0,f.source.len() as u32]);
    let lines = core_rr::Lines::new(&f.source,true);
    expected.extend((1..=lines.count()).map(|l|lines.span(l)));
    assert_eq!(p.extra.as_ref().unwrap().tokens as usize,tokens);
    assert_eq!(p.locs(),p.len()+expected.len() as u32);
    // Counting and source/line access never allocate token spans.
    for i in tokens..expected.len() { assert_eq!(p.span(p.len()+i as u32),expected[i]); }
    assert!(p.extra.as_ref().unwrap().token_spans.get().is_none());
    for (i,&span) in expected.iter().enumerate() { assert_eq!(p.span(p.len()+i as u32),span); }
    for l in 0..p.core.len() {
        let parent=p.parent(l);
        if l==0 { assert_eq!(parent,NONE); }
        else {
            let siblings:Vec<_>=p.children(parent).collect();
            assert_eq!(p.idx(l),siblings.iter().position(|&c|c==l as u32).unwrap() as u32);
        }
        let mut depth=0;let mut up=parent;
        while up!=NONE { depth+=1;up=p.parent(up as usize); }
        assert_eq!(p.depth(l),depth);
        let subtree_end=f.subtree_end[p.core[l] as usize];
        assert_eq!(p.size(l),p.core[l+1..].iter().filter(|&&id|id<subtree_end).count() as u32);
    }
    assert!(p.fielded.get().is_none(),"reading other axes must not build field rows");
}

/// Each node's text and its id, under `granules`' numbering (the default): `(text, id)` pairs.
fn leaf_ids(label: &str, path: &str, source: &str) -> (BTreeSet<(String, i64)>, Projected) {
    let d = document(7, path, label, source);
    let dialect = Dialect::new(core::facts_tag(&d.facts)).unwrap();
    let p = Projected::new(d.key, Arc::clone(&d.facts), &dialect, Shape::Core, false, None);
    let f = &*d.facts;
    let leaves = (0..p.len()).map(|l| (f.text(p.core[l as usize]).to_owned(), p.global(l))).collect();
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
                        let p = Projected::new(1, Arc::clone(&facts), &dialect, Shape::Core, false, None);
                        row.parse += cpu_now() - t_parse;
                        let f = &*facts;
                        let tokens: Vec<u32> = p.core.iter().enumerate().flat_map(|(l, &id)| f.children(id).iter().filter(|&&c| !f.named(c) && f.end[c as usize] > f.start[c as usize]).map(move |_| l as u32)).collect();
                        let t0 = cpu_now();
                        let _ = granules::number(&p, &dialect, &tokens);
                        row.cpu += cpu_now() - t0;
                        let (g, cont, biggest, fold) = granules::census(&p, &dialect, &tokens);
                        row.nodes += p.core.len() + tokens.len();
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
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
}

/// A rule reading the text of a node whose kind no body fixes gets a twin reading `@rr_node_text` (decision 0109);
/// a fixed kind, a field of leaves only, a short constant, and the twin itself need none. With the twin, no rule
/// asks for every node's text at any length.
#[test]
fn long_text_twins_where_no_body_fixes_the_kind() {
    let prelude = std::fs::read_to_string(crate::code_rule_dsl::logic::rr::library().join("core.rr")).unwrap();
    for (body, twins) in [
        ("out(t) :- node(_, n, _, _, _), text(n, t).", 1),
        ("out(t) :- node(_, n, _, _, _), text(n, t), field(n, \"object\", o), text(o, t).", 3),
        ("out(t) :- kind(n, \"call\"), text(n, t).", 0),
        ("out(t) :- field(_, \"attribute\", n), text(n, t).", 0),
        ("out(\"doc\") :- node(_, n, _, _, _), text(n, \"doc\").", 0),
    ] {
        let mut program = crate::code_rule_dsl::logic::rr::parse(&format!("{prelude}\n.decl out(t: symbol)\n{body}\n.output out\n")).unwrap();
        assert_eq!(long_text_twins(&mut program, &[]), twins, "{body}");
        assert!(text_kind_demand(&program, &[]).1.is_empty(), "{body}: every text is read");
        assert_eq!(long_text_twins(&mut program, &[]), 0, "{body}: twinned once");
    }
}
