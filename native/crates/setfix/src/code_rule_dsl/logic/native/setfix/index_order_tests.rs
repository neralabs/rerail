//! An input's index is kept by its set of key columns (`InRel::index`), in the column order of the first to ask for
//! it: a join that probes those columns in another order reads its key in the index's order.
use super::spec::hex;
use super::*;

fn connection() -> Connection {
    let path = crate::code_rule_dsl::fixtures::temp();
    drop(crate::content_store::ContentStore::open(&path).unwrap());
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

/// The rows of a relation derived from `rows` through a run (`copy`), so that it holds the chains its wave kept.
fn derived(db: &Connection, name: &str, rows: &[[i64; 2]]) -> Arc<InRel> {
    let refs: Vec<&[i64]> = rows.iter().map(|r| r.as_slice()).collect();
    let input = format!("{name}_in");
    adopt(db, &input, Arc::new(rel_of(&refs, b"ii")));
    let body = format!("pos r:{} v0 v1\nhead v0 v1", hex(&input));
    open(db, &format!("name {name}\nrel {name} ii empty\nnowrite\nseed 0 {}", hex(&body))).unwrap();
    run(db, name, 0).unwrap();
    close(db, name).unwrap();
    held(db, name).expect("the derived relation")
}

/// A probe of both columns of a derived relation leads with a column's chain only where a probe meets few rows of
/// it. One value holding most rows (a module scope's bindings in `bound_in(s, x)`) fails that, though the mean rows
/// a value is small; the probe then reads the sorted order over both columns. A column of evenly small groups leads.
#[test]
fn a_probe_leads_with_a_column_only_where_its_values_hold_few_rows_each() {
    let db = connection();
    // 2,000 rows of value 1, then 2,000 values of one row each: 2 rows a value on average, 1,000 met by a probe.
    let skewed: Vec<[i64; 2]> = (0..2000).map(|x| [1, x]).chain((0..2000).map(|s| [10 + s, 0])).collect();
    let even: Vec<[i64; 2]> = (0..4000).map(|i| [i / 4, i % 4]).collect();
    let (skewed, even) = (derived(&db, "lead_skewed", &skewed), derived(&db, "lead_even", &even));
    assert!(!skewed.chains.is_empty() && !even.chains.is_empty(), "the waves kept their chains");
    assert_eq!(skewed.chain_lead(&[0, 1]).map(|(_, j)| j), None, "the skewed column does not lead");
    assert_eq!(even.chain_lead(&[0, 1]).map(|(_, j)| j), Some(0), "the even column leads");
}
