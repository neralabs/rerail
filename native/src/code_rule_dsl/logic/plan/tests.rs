use super::*;
use crate::code_rule_dsl::logic::front::{parse, prune};
use crate::code_rule_dsl::logic::magic::{Options, magic_adorn, magic_sets};
use crate::content_store::ContentStore;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/logic")
}

/// The facts T's exporter and `stats.py` built for nine CPython 3.13 modules (`json/`, `ftplib`, `shlex`,
/// `fnmatch`, `glob`), loaded from their compressed dump into a new file.
fn facts() -> std::path::PathBuf {
    let dump = zstd::decode_all(std::fs::File::open(root().join("plan/golden/facts.sql.zst")).unwrap()).unwrap();
    let path = crate::code_rule_dsl::prefilter::tests::temp();
    Connection::open(&path).unwrap().execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    path
}

/// T's `rdl.py sql` front: prune, demand (adornment for `.demand *`), prune.
fn program(name: &str) -> Program {
    let src = std::fs::read_to_string(root().join(format!("../typing/programs/{name}.rdl"))).unwrap();
    let p = prune(parse(&src).unwrap());
    let options = Options::default();
    prune(if p.demand.iter().any(|d| d == "*") { magic_adorn(p, &options) } else { magic_sets(p, &options) }.unwrap())
}

fn chosen_line(c: &Chosen) -> String {
    let walks: Vec<String> =
        c.walks.iter().map(|(l, m)| format!("[{}, {}]", serde_json::to_string(l).unwrap(), serde_json::to_string(m).unwrap())).collect();
    format!("{}\t{}\t[{}]\t{}", c.head, c.anchor, walks.join(", "), c.work)
}

/// On T's facts, the port chooses T's plans (anchor, walks, estimated work, rule for rule) and derives T's outputs.
#[test]
fn t_programs_plan_and_run_like_t() {
    let facts = facts();
    let mut failures = Vec::new();
    for name in ["workload", "workload_demand", "calls"] {
        let golden = root().join(format!("plan/golden/{name}"));
        let got = run_sql(&program(name), &facts).unwrap_or_else(|e| panic!("{name}: {e}"));
        let lines: Vec<String> = got.chosen.iter().map(chosen_line).collect();
        let expected = std::fs::read_to_string(golden.join("rules.txt")).unwrap();
        let expected: Vec<&str> = expected.lines().collect();
        if let Some(i) = (0..lines.len().max(expected.len())).find(|&i| lines.get(i).map(String::as_str) != expected.get(i).copied()) {
            failures.push(format!("{name} plan {i}:\n  got      {:?}\n  expected {:?}", lines.get(i), expected.get(i)));
        }
        let mut files: Vec<String> = std::fs::read_dir(&golden)
            .unwrap()
            .filter_map(|e| e.unwrap().file_name().to_str()?.strip_suffix(".csv").map(str::to_owned))
            .collect();
        files.sort();
        assert_eq!(got.outputs.keys().cloned().collect::<Vec<_>>(), files, "{name}");
        assert!(!lines.is_empty() && got.outputs.values().any(|r| !r.is_empty()), "{name}");
        for (rel, rows) in &got.outputs {
            let expected = std::fs::read_to_string(golden.join(format!("{rel}.csv"))).unwrap();
            let expected: Vec<&str> = expected.lines().collect();
            if rows.iter().map(String::as_str).collect::<Vec<_>>() != expected {
                failures.push(format!("{name} {rel}: {} rows, expected {}", rows.len(), expected.len()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// T's statistics with T's strings lowered as the program's are (decision 0009) and T's node ids as the vtabs':
/// `stat_kind`/`stat_edge` by kind atoms, `stat_val` by its column's domain, `stat_docrange` as id intervals.
fn atomized_statistics(t: &Connection, db: &Connection, store: &ContentStore, first: &HashMap<i64, i64>) {
    use lower::Domain;
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    let tag = crate::code_rule_dsl::language::tape_tag(python).unwrap();
    let kinds = python.kinds();
    let kind = |s: &str| i64::from(crate::code_rule_dsl::facts::core::kind_atom(tag, kinds.kind_id(s)));
    let field = |s: &str| kinds.field_id(s).map_or(-1, |f| i64::from(crate::code_rule_dsl::facts::vtab::syntax::field_atom(tag, f)));
    let name = |s: &str| i64::from(store.intern_atoms(&[s]).unwrap()[0]);
    let domain = |tbl: &str, col: &str| match (tbl, col) {
        ("node", "kind") => Domain::Kind,
        ("field", "f") => Domain::Field,
        ("text" | "op", "s") | ("doc", "path" | "module") | ("pkg", "p") => Domain::Name,
        _ => Domain::Int,
    };
    db.execute_batch(
        "ATTACH ':memory:' AS s;
         CREATE TABLE s.stat_meta(k TEXT PRIMARY KEY, v INTEGER);
         CREATE TABLE s.stat_col(tbl TEXT, col TEXT, rows INTEGER, ndv INTEGER, sq INTEGER, sqd INTEGER, PRIMARY KEY (tbl, col));
         CREATE TABLE s.stat_kind(kind INTEGER PRIMARY KEY, n INTEGER, size INTEGER);
         CREATE TABLE s.stat_edge(pkind INTEGER, ckind INTEGER, field, n INTEGER);
         CREATE TABLE s.stat_val(tbl TEXT, col TEXT, val, n INTEGER, docs INTEGER, sqd INTEGER, PRIMARY KEY (tbl, col, val));
         CREATE TABLE s.stat_docrange(d INTEGER PRIMARY KEY, lo INTEGER, hi INTEGER);",
    )
    .unwrap();
    let rows = |sql: &str| -> Vec<Vec<SqlValue>> {
        let mut st = t.prepare(sql).unwrap();
        let width = st.column_count();
        st.query_map([], |r| (0..width).map(|i| r.get::<_, SqlValue>(i)).collect()).unwrap().map(|r| r.unwrap()).collect()
    };
    let insert = |table: &str, row: Vec<SqlValue>| {
        let holes = vec!["?"; row.len()].join(", ");
        db.execute(&format!("INSERT INTO s.{table} VALUES ({holes})"), rusqlite::params_from_iter(row)).unwrap();
    };
    for table in ["stat_meta", "stat_col"] {
        rows(&format!("SELECT * FROM {table}")).into_iter().for_each(|r| insert(table, r));
    }
    for r in rows("SELECT kind, n, size FROM stat_kind") {
        insert("stat_kind", vec![SqlValue::Integer(kind(&text(&r[0]))), r[1].clone(), r[2].clone()]);
    }
    for r in rows("SELECT pkind, ckind, field, n FROM stat_edge") {
        insert("stat_edge", vec![SqlValue::Integer(kind(&text(&r[0]))), SqlValue::Integer(kind(&text(&r[1]))), r[2].clone(), r[3].clone()]);
    }
    for mut r in rows("SELECT tbl, col, val, n, docs, sqd FROM stat_val") {
        let v = text(&r[2]);
        r[2] = match domain(&text(&r[0]), &text(&r[1])) {
            Domain::Kind => SqlValue::Integer(kind(&v)),
            Domain::Field => SqlValue::Integer(field(&v)),
            Domain::Name => SqlValue::Integer(name(&v)),
            _ => r[2].clone(),
        };
        insert("stat_val", r);
    }
    for r in rows("SELECT d, lo, hi FROM stat_docrange") {
        let d = int(&r[0]);
        let id = |n: i64| SqlValue::Integer((d << 32) + n - first[&d]);
        insert("stat_docrange", vec![r[0].clone(), id(int(&r[1])), id(int(&r[2]))]);
    }
}

/// O13: lowered to atoms and run over the base-relation vtabs of the same files (`facts::vtab::syntax`), T's programs
/// get T's plans and, decoded, T's outputs.
#[test]
fn t_programs_over_atoms_plan_and_run_like_t() {
    use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
    let path = crate::code_rule_dsl::prefilter::tests::temp();
    let store = ContentStore::open(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    let modules = Modules::default();
    for (name, source) in syntax::sources(&store, syntax::tests::documents()).unwrap() {
        modules.bind(&db, name, source).unwrap();
    }
    let t = Connection::open_in_memory().unwrap();
    let dump = zstd::decode_all(std::fs::File::open(root().join("plan/golden/facts.sql.zst")).unwrap()).unwrap();
    t.execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    let first: HashMap<i64, i64> =
        t.prepare("SELECT d, min(n) FROM node GROUP BY d").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect();
    atomized_statistics(&t, &db, &store, &first);
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    let mut failures = Vec::new();
    for name in ["workload", "workload_demand", "calls"] {
        let golden = root().join(format!("plan/golden/{name}"));
        let p = program(name);
        let lowered = lower::lower(&p, python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let got = run_atoms(&lowered, &db, "s").unwrap_or_else(|e| panic!("{name}: {e}"));
        // The plans in the unlowered program's terms, as T printed them.
        let lines: Vec<String> = got
            .chosen
            .iter()
            .map(|c| {
                let rule = &p.rules[c.rule];
                chosen_line(&Chosen {
                    head: rule.head.to_string(),
                    anchor: rule.body[c.steps[0].0].to_string(),
                    walks: c.steps.iter().filter_map(|&(at, m)| m.map(|m| (rule.body[at].to_string(), m.name()))).collect(),
                    ..c.clone()
                })
            })
            .collect();
        let expected = std::fs::read_to_string(golden.join("rules.txt")).unwrap();
        let expected: Vec<&str> = expected.lines().collect();
        if let Some(i) = (0..lines.len().max(expected.len())).find(|&i| lines.get(i).map(String::as_str) != expected.get(i).copied()) {
            failures.push(format!("{name} plan {i}:\n  got      {:?}\n  expected {:?}", lines.get(i), expected.get(i)));
        }
        assert!(!got.outputs.is_empty() && got.outputs.values().any(|r| !r.is_empty()), "{name}");
        for (rel, rows) in &got.outputs {
            let nodes = &got.nodes[rel];
            let mut rows: Vec<String> = rows
                .iter()
                .map(|row| {
                    row.split('\t')
                        .enumerate()
                        .map(|(i, v)| match v.parse::<i64>() {
                            Ok(v) if nodes.contains(&i) => (first[&(v >> 32)] + (v & 0xffff_ffff)).to_string(),
                            _ => v.to_owned(),
                        })
                        .collect::<Vec<_>>()
                        .join("\t")
                })
                .collect();
            rows.sort();
            let expected = std::fs::read_to_string(golden.join(format!("{rel}.csv"))).unwrap();
            let expected: Vec<&str> = expected.lines().collect();
            if rows.iter().map(String::as_str).collect::<Vec<_>>() != expected {
                let i = (0..rows.len().max(expected.len())).find(|&i| rows.get(i).map(String::as_str) != expected.get(i).copied());
                failures.push(format!(
                    "{name} {rel}: {} rows, expected {}; first difference {:?} vs {:?}",
                    rows.len(),
                    expected.len(),
                    i.and_then(|i| rows.get(i)),
                    i.and_then(|i| expected.get(i))
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
