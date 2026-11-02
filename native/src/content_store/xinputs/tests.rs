use super::*;
use crate::content_store::tests::store;

fn sample() -> XinputSet {
    let mut rows: Vec<Rows> = vec![Vec::new(); TABLES.len()];
    for (k, table) in TABLES.iter().enumerate() {
        for variant in 0..if table.variant { 3u8 } else { 1 } {
            for n in 0..2i64 {
                let values = table
                    .cols
                    .iter()
                    .enumerate()
                    .map(|(c, (_, col))| match (col, (n + c as i64) % 3) {
                        (Col::Atom, 2) if !table.variant => Value::Null,
                        (Col::Int, _) => Value::Integer(n * 100 + c as i64),
                        (Col::Atom, _) => Value::Text(format!("{}.{variant}.{n}.{c}", table.name)),
                    })
                    .collect();
                rows[k].push((variant, values));
            }
        }
    }
    XinputSet { producer: [7; 32], digest: [9; 32], failure: None, rows }
}

#[test]
fn a_set_round_trips_replaces_and_dies_with_its_tape() {
    let (_, store) = store("xinputs");
    let source = store.put_source(b"def f(): pass\n", &[]).unwrap();
    let tape = store.put_tape(source, 1, 1, 1, b"tape").unwrap();
    assert_eq!(store.xinputs(tape).unwrap(), None);
    let set = sample();
    store.put_xinputs(tape, &set).unwrap();
    assert_eq!(store.xinputs(tape).unwrap().as_ref(), Some(&set));
    let failed = XinputSet { failure: Some("the parser gave up".into()), rows: vec![Vec::new(); TABLES.len()], ..set.clone() };
    store.put_xinputs(tape, &failed).unwrap();
    assert_eq!(store.xinputs(tape).unwrap(), Some(failed));
    store.put_xinputs(tape, &set).unwrap();
    let rows = |table: &str| -> i64 { store.connection.lock().unwrap().query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap() };
    assert_eq!(rows("xinput_cgn_edges"), 6);
    store.connection.lock().unwrap().execute("DELETE FROM tape_objects WHERE tape_id=?1", [tape.row()]).unwrap();
    assert_eq!(store.xinputs(tape).unwrap(), None);
    for table in &TABLES {
        assert_eq!(rows(table.name), 0, "{}", table.name);
    }
}

#[test]
fn every_table_has_the_columns_the_spec_names() {
    let (_, store) = store("xinputs-schema");
    let connection = store.connection.lock().unwrap();
    for table in &TABLES {
        let got: Vec<String> = connection
            .prepare(&format!("SELECT name FROM pragma_table_info('{}') ORDER BY cid", table.name))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut want = vec!["tape_id".to_owned()];
        if table.variant {
            want.push("variant".into());
        }
        want.push("ord".into());
        want.extend(table.cols.iter().map(|(name, _)| (*name).to_owned()));
        assert_eq!(got, want, "{}", table.name);
    }
}
