//! An input's index is kept by its set of key columns (`InRel::index`), in the column order of the first to ask for
//! it: a join that probes those columns in another order reads its key in the index's order.
use super::spec::hex;
use super::*;

fn connection() -> Connection {
    let path = crate::code_rule_dsl::fixtures::temp();
    drop(crate::code_rule_dsl::fixtures::open(&path));
    let db = Connection::open(path).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    db
}

fn rel_of(rows: &[&[i64]], types: &[u8]) -> InRel {
    let mut r = Rel::new("x", types);
    for row in rows {
        r.push(row);
    }
    InRel::moved(r)
}

/// The planner prices a probe of `pair` by its columns `[1, 0]` (`probe_size`, from `work_at`) before a join probes it
/// by `[0, 1]`: the index made for the first serves the second, and the join finds its rows. Before, the join read its
/// key in its own order through the index's: `rf_ref` probed by `[0, 1, 2, 3]` through an index made for
/// `[1, 0, 2, 3]` looked a node up in a constant's chain, and a handoff's replacement derived none of its rows (t50
/// step 1: 669 references lost on 34 files).
#[test]
fn a_join_probes_an_input_through_an_index_made_for_its_columns_in_another_order() {
    let db = connection();
    adopt(&db, "ixo_pair", Arc::new(rel_of(&[&[1, 10], &[2, 20], &[3, 30], &[4, 40]], b"ii")));
    adopt(&db, "ixo_want", Arc::new(rel_of(&[&[1, 10], &[2, 99], &[3, 30], &[40, 4]], b"ii")));
    let pair = held(&db, "ixo_pair").expect("adopted");
    assert_eq!(pair.probe_size(&[1, 0], &[30, 3]), 1.0, "priced by its columns reversed");
    let r = |t: &str| format!("r:{}", hex(t));
    let body = format!("pos {} v0 v1\npos {} v0 v1\nhead v0", r("ixo_want"), r("ixo_pair"));
    open(&db, &format!("name ixo_hit\nrel ixo_hit i empty\nnowrite\nseed 0 {}", hex(&body))).unwrap();
    run(&db, "ixo_hit", 0).unwrap();
    close(&db, "ixo_hit").unwrap();
    let hit = held(&db, "ixo_hit").expect("the derived relation");
    let mut got: Vec<i64> = (0..hit.len()).filter(|&i| !is_dead(&hit.dead, i as u32)).map(|i| hit.at(i).at(0)).collect();
    got.sort_unstable();
    assert_eq!(got, vec![1, 3], "the rows of `want` that `pair` holds");
}
