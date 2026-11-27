//! `field`'s reads by its field alone (`field($p, #f, $c)`, neither node bound): a walk of each document's tape.
use super::*;
use super::fixtures::documents;
use crate::code_rule_dsl::facts::vtab::ProbeCursor;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn temp(what: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rerail-field-reads-{what}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// For every field the documents hold, and one they do not: `lookup` by the field and a cursor's read give exactly the
/// rows whose field column is that field (a scan of every row), in order, and the read's values are each row's.
#[test]
fn a_fields_rows_by_field_alone_are_the_rows_a_scan_finds() {
    let store = ContentStore::open(&temp("by-field")).unwrap();
    let (_, field) = sources(&store, documents().into_iter().take(4).collect()).unwrap().into_iter().find(|(name, _)| *name == "field").unwrap();
    let n = field.len();
    assert!(n > 100, "the documents have fielded nodes: {n}");
    let mut by: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for r in 0..n {
        by.entry(field.value(r, 1)).or_default().push(r);
    }
    assert!(by.len() > 5, "fields: {}", by.len());
    let absent = by.keys().last().unwrap() + 1;
    by.insert(absent, Vec::new());
    let columns: Vec<usize> = (0..field.columns().len()).collect();
    let mut cursor = ProbeCursor::default();
    for (&f, want) in &by {
        let got = field.lookup(1, &[f]).expect("field looks up by its field");
        assert_eq!(got.iter().copied().collect::<Vec<_>>(), *want, "field {f}: lookup");
        let mut read = Vec::new();
        assert!(field.lookup_read_at(1, f, &columns, &mut cursor, &mut |r, vals| read.push((r, vals.to_vec()))), "field {f}: read as found");
        let rows: Vec<(usize, Vec<i64>)> = want.iter().map(|&r| (r, columns.iter().map(|&c| field.value(r, c)).collect())).collect();
        assert_eq!(read, rows, "field {f}: a cursor's read");
    }
}

/// A chunk's source as it binds a relation (`Layered`: the batch's rows, then the values admitted past them) filters to
/// the rows a per-row read keeps: for every source, on each column at every seventh row's value, and on two columns at
/// once. Its own scan, where the trait's read each row's columns one select at a time.
#[test]
fn a_layered_sources_filter_scan_holds_the_rows_a_per_row_read_finds() {
    use crate::code_rule_dsl::facts::nearest::Binding;
    let store = ContentStore::open(&temp("layered-filter")).unwrap();
    let tests = Arc::new(crate::code_rule_dsl::logic::strings::Vocabulary::default());
    let projection = Projection::core();
    let file = |k: u32| document(k, &format!("m{k}.py"), "python", &format!("def f{k}(x, y=[1, 2]):\n    return g(x, *y)\n"));
    let (sources, mut grower) = Grower::start(&store, vec![file(20), file(23)], &[], &Matchers::default(), &tests, Binding::Cpu, &projection, &HashMap::new()).unwrap();
    let values = Arc::new(super::values::Values::default());
    values.add(&[5], &[], grower.admit(&store, vec![file(5)], &[], &tests, Binding::Cpu, &projection).unwrap());
    let mut tried = 0;
    for (name, s) in &sources {
        let source = super::values::Layered::new(name, Arc::clone(s), Arc::clone(&values));
        let n = source.len();
        let width = source.columns().len();
        for c in 0..width {
            for r0 in (0..n).step_by(7) {
                let v = source.value(r0, c);
                let mut eqs = vec![(c, v)];
                if width > 1 {
                    let d = (c + 1) % width;
                    eqs.push((d, source.value(r0, d)));
                }
                for eqs in [&eqs[..1], &eqs[..]] {
                    let want: Vec<u32> = (0..n).filter(|&r| eqs.iter().all(|&(c, v)| source.value(r, c) == v)).map(|r| r as u32).collect();
                    let got: Vec<u32> = source.filter_eq(eqs).iter().copied().collect();
                    assert_eq!(got, want, "{name} {eqs:?}");
                    tried += 1;
                }
            }
        }
    }
    assert!(tried > 50, "{tried} filters");
}

/// One Python document of `source`, keyed `key`, with its tape.
fn document(key: u32, path: &str, label: &str, source: &str) -> Document {
    use crate::code_rule_dsl::facts::core;
    use crate::code_rule_dsl::model::FileInput;
    let file = FileInput::new(path.to_owned(), label.to_owned(), Arc::new(source.to_owned().into()), None, None, "core").unwrap();
    let (core::Built::Facts(facts), Some(tape)) = core::build_with_tape(&file, true).unwrap() else { panic!("{path} did not parse") };
    let facts = Arc::new(facts);
    Document { key, core: format!("/w#core:{path}"), digest: core::digest(&facts), path: path.to_owned(), facts, tape: Some((tape, Arc::new(file))), guest: None, module: None, unpackaged: false }
}

/// A node's text read as the atom it has (`Source::node_atom`, `rr_node_text_any`) is the atom of the text `node_read`
/// reads, for every node of the documents, and none where the text has no atom; the terms' memo answers a node only
/// with its own text's atom (each text's atom here its place among the distinct texts, every third one held).
#[test]
fn a_nodes_text_read_as_its_atom_is_the_atom_of_its_text() {
    use crate::code_rule_dsl::facts::vtab::LONG_TEXT_NODES;
    let store = ContentStore::open(&temp("text-atoms")).unwrap();
    let docs: Vec<Document> = (1..=3).map(|k| document(k, &format!("m{k}.py"), "python", &format!("def f{k}(x, y):\n    z = x + y + f{k}(x)\n    return \"s{k}\" if z else x.y\n"))).collect();
    let projection = Projection { shape: Shape::Core, reads: Some(["node", "text", "rr_long_text_node", "doc"].iter().map(|r| (*r).to_owned()).collect()), ..Projection::core() };
    let batch = Batch::new(&store, docs, &projection, &[], &[], false, None, &HashMap::new()).unwrap();
    let sources = made_sources(&batch);
    let texts = Arc::clone(&sources.iter().find(|(n, _, _)| *n == LONG_TEXT_NODES).unwrap().2);
    let ids: Vec<i64> = (0..batch.docs.len()).flat_map(|d| { let p = &batch.docs[d]; (0..p.len()).map(|l| p.global(l)).collect::<Vec<_>>() }).collect();
    let text_of = |n: i64| match texts.node_read("rr_node_text_any", n) {
        Some(rusqlite::types::Value::Text(t)) => Some(t),
        _ => None,
    };
    let distinct: BTreeSet<String> = ids.iter().filter_map(|&n| text_of(n)).collect();
    let atom: BTreeMap<String, i64> = distinct.iter().enumerate().filter(|(i, _)| i % 3 == 0).map(|(i, t)| (t.clone(), i as i64)).collect();
    let existing = |s: &str| atom.get(s).copied();
    let mut held = 0;
    // Twice: the second read answered from the memo.
    for _ in 0..2 {
        for &n in &ids {
            let want = text_of(n).and_then(|t| atom.get(&t).copied());
            assert_eq!(texts.node_atom("rr_node_text_any", n, &existing), want, "node {n:#x} ({:?})", text_of(n));
            held += usize::from(want.is_some());
        }
    }
    assert!(held > 10 && ids.len() > 50, "{held} held of {}", ids.len());
    assert_eq!(texts.node_atom("rr_node_text", ids[1], &existing), None, "another read: none");
}
