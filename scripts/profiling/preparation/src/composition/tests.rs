use super::*;
use rusqlite::{params_from_iter, types::Value as SqlValue};

fn varint(mut v: u32, out: &mut Vec<u8>) {
    while v >= 128 {
        out.push((v as u8 & 127) | 128);
        v >>= 7;
    }
    out.push(v as u8);
}

pub(super) fn fixture() -> (tempfile::TempDir, crate::store::Store) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(include_str!(
        "../../../../../native/src/code_rule_dsl/matcher/anchors/sql/schema.sql"
    ))
    .unwrap();
    db.execute_batch("INSERT INTO lang VALUES(1,'python'); INSERT INTO kind VALUES(1,1,'module',1),(1,2,'function_definition',1),(1,3,'assignment',1),(1,4,'block',1),(1,5,'call',1),(1,6,'attribute',1),(1,7,'identifier',1); INSERT INTO field VALUES(1,0,''),(1,1,'function'),(1,2,'attribute')").unwrap();
    let nodes = [
        (-1, 12, 1, 0),
        (0, 8, 2, 0),
        (1, 8, 4, 0),
        (2, 6, 5, 0),
        (3, 6, 6, 1),
        (4, 6, 7, 2),
        (2, 7, 7, 0),
        (2, 8, 5, 0),
        (0, 12, 3, 0),
        (8, 12, 5, 0),
        (9, 12, 6, 1),
        (10, 12, 7, 2),
    ];
    let mut props = Vec::new();
    let mut values = Vec::new();
    for fid in 1..=2i64 {
        let mut rec = Vec::new();
        let mut spans = Vec::new();
        let mut src = String::new();
        let mut last = 0;
        for (pre, (parent, end, kind, field)) in nodes.iter().copied().enumerate() {
            let text = if pre == 5 && fid == 1 {
                "raw_execute"
            } else {
                "safe"
            };
            let start = src.len();
            src.push_str(text);
            varint(((start - last) * 2) as u32, &mut spans);
            varint(text.len() as u32, &mut spans);
            last = start;
            for word in [
                kind as u16 | ((field as u16) << 10),
                (pre as i64 - parent) as u16,
                (end - pre) as u16,
            ] {
                rec.extend(word.to_le_bytes());
            }
            props.push(format!("({fid},{pre},{parent},{end},{kind},{field},1)"));
        }
        values.extend([
            SqlValue::Integer(fid),
            SqlValue::Integer(12),
            SqlValue::Text(format!("{fid}.py")),
            SqlValue::Blob(vec![fid as u8]),
            SqlValue::Integer(1),
            SqlValue::Blob(src.into_bytes()),
            SqlValue::Blob(rec),
            SqlValue::Blob(spans),
            SqlValue::Blob(vec![0; 6]),
            SqlValue::Blob(Vec::new()),
            SqlValue::Blob(Vec::new()),
            SqlValue::Blob(vec![1; 12]),
        ]);
    }
    db.execute("INSERT INTO file(fid,nodes,path,digest,lang,src,rec,spans,checkpoints,parb,sizb,flags) VALUES(?,?,?,?,?,?,?,?,?,?,?,?),(?,?,?,?,?,?,?,?,?,?,?,?)",params_from_iter(values)).unwrap();
    db.execute_batch(&format!(
        "INSERT INTO decoded_properties VALUES {}",
        props.join(",")
    ))
    .unwrap();
    drop(db);
    let store = crate::store::Store::open(&path).unwrap();
    facts(&store.db, &[1, 2], &mut Meter::default()).unwrap();
    (dir, store)
}

#[test]
fn demand_expression_tracks_the_authored_data_layer_rule() {
    let policy: Value = serde_json::from_str(include_str!(
        "../../../../../packs/data-layer/candidates/rules/data-layer-raw-execute-regression.json"
    ))
    .unwrap();
    assert_eq!(
        policy
            .pointer("/clauses/1/forbid/ast/rule/all/1/all/1/has/all/1/has/all/1/regex")
            .unwrap(),
        shapes::REMOVED
    );
}
fn sorted(mut rows: Vec<Vec<i64>>) -> Vec<Vec<i64>> {
    rows.sort();
    rows
}

#[test]
fn intervals_preserve_nested_roots_counts_first_witness_and_root_stops() {
    let (_dir, s) = fixture();
    let v = vocabulary(&s.db).unwrap();
    let reference = sorted(shapes::ranges(&s.db, &v, "closure", &mut Meter::default()).unwrap());
    let pairs = sorted(
        Meter::default()
            .statement(&s.db, "SELECT fid,root,node FROM closure")
            .unwrap(),
    );
    let intervals=sorted(Meter::default().statement(&s.db,"SELECT r.fid,r.pre,c.pre FROM n r JOIN n c ON c.fid=r.fid AND c.pre>=r.pre AND c.pre<CASE WHEN r.kind IN (1,4) THEN r.pre+1 ELSE r.end END").unwrap());
    assert_eq!(pairs, intervals);
    assert_eq!(pairs.iter().filter(|r| r[1] == r[2]).count(), 24);
    let candidate = sorted(shapes::ranges(&s.db, &v, "interval", &mut Meter::default()).unwrap());
    assert_eq!(reference, candidate);
    s.db.execute_batch("DROP TABLE closure").unwrap();
    assert_eq!(
        candidate,
        sorted(shapes::ranges(&s.db, &v, "demanded-closure", &mut Meter::default()).unwrap())
    );
    assert!(candidate.contains(&vec![1, 1, 2, 3]));
    assert!(candidate.contains(&vec![1, 2, 0, -1])); // Block roots do not expand.
    assert!(candidate.contains(&vec![1, 8, 1, 9]));
    s.db.execute_batch("DELETE FROM n").unwrap();
    assert!(
        shapes::ranges(&s.db, &v, "interval", &mut Meter::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn interval_input_check_rejects_a_non_tree_parent() {
    let (_dir, s) = fixture();
    assert!(check_tree(&s.db).is_ok());
    s.db.execute_batch("UPDATE n SET parent=9 WHERE fid=1 AND pre=3")
        .unwrap();
    assert!(check_tree(&s.db).is_err());
}

#[test]
fn selective_edges_keep_positive_and_clean_results_and_reject_wrong_fields() {
    let (_dir, s) = fixture();
    let v = vocabulary(&s.db).unwrap();
    let eager = shapes::demand(&s.db, &v, "eager", &mut Meter::default()).unwrap();
    assert_eq!(eager, vec![vec![1, 3, 4, 5]]);
    assert_eq!(
        eager,
        shapes::demand(&s.db, &v, "selective", &mut Meter::default()).unwrap()
    );
    assert_eq!(
        eager,
        shapes::demand(&s.db, &v, "cut", &mut Meter::default()).unwrap()
    );
    s.db.execute_batch("UPDATE n SET field=0 WHERE pre=5")
        .unwrap();
    assert!(
        shapes::demand(&s.db, &v, "selective", &mut Meter::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn utility_sharing_keeps_file_keys_and_set_semantics() {
    let (_dir, s) = fixture();
    let v = vocabulary(&s.db).unwrap();
    let reference = sorted(shapes::utility(&s.db, &v, "inline", 4, &mut Meter::default()).unwrap());
    for variant in ["cte", "cuts"] {
        assert_eq!(
            reference,
            sorted(shapes::utility(&s.db, &v, variant, 4, &mut Meter::default()).unwrap())
        );
    }
    assert_eq!(
        reference,
        vec![vec![1, 3], vec![1, 9], vec![2, 3], vec![2, 9]]
    );
}

#[test]
fn batched_columns_equal_production_decoders_and_preserve_repeated_queries() {
    let (_dir, s) = fixture();
    let (reference, _) = batch::run(&s, &[1, 2], "scalar", &mut Meter::default()).unwrap();
    for variant in ["stream", "columns", "columns"] {
        let (rows, metrics) = batch::run(&s, &[1, 2], variant, &mut Meter::default()).unwrap();
        assert_eq!(sorted(reference.clone()), sorted(rows));
        if variant == "columns" {
            assert_eq!(metrics["filters"], 1);
            assert_eq!(metrics["waves"], 1);
        }
    }
    assert_eq!(reference.len(), 24);
}

#[test]
fn batch_rejects_missing_and_invalid_ranges_and_accepts_empty_ranges() {
    use batch::Task;
    assert!(
        batch::validate(
            &[Task {
                fid: 1,
                lo: 5,
                hi: 4
            }],
            &[(1, 12)]
        )
        .is_err()
    );
    assert!(
        batch::validate(
            &[Task {
                fid: 1,
                lo: 0,
                hi: 13
            }],
            &[(1, 12)]
        )
        .is_err()
    );
    assert!(
        batch::validate(
            &[Task {
                fid: 9,
                lo: 0,
                hi: 0
            }],
            &[(1, 12)]
        )
        .is_err()
    );
    assert!(
        batch::validate(
            &[Task {
                fid: 1,
                lo: 12,
                hi: 12
            }],
            &[(1, 12)]
        )
        .is_ok()
    );
}

#[test]
fn generic_graph_keeps_cycle_safe_union_and_multiple_parents() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE edge(a INTEGER,b INTEGER); INSERT INTO edge VALUES(1,2),(2,3),(3,1),(1,4),(4,3)").unwrap();
    let rows=Meter::default().statement(&db,"WITH RECURSIVE w(n) AS (VALUES(1) UNION SELECT b FROM edge JOIN w ON a=n) SELECT n FROM w ORDER BY n").unwrap();
    assert_eq!(rows, vec![vec![1], vec![2], vec![3], vec![4]]);
    // No interval lowering is applied to a graph: the lab only accepts captured tree facts.
}
