//! `field`'s reads by its field alone (`field($p, #f, $c)`, neither node bound): a walk of each document's tape.
use super::*;
use super::fixtures::documents;
use crate::code_rule_dsl::facts::vtab::ProbeCursor;
use std::collections::BTreeMap;
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
