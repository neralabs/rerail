use super::super::super::tests::{executor, rows};
use super::*;

#[test]
fn wide_budgeted_lattice_and_scalar_feedback_replays_atomically() {
    let (_dir, mut e) = executor();
    let mut program = String::from(
        ".decl seed(g:number,v:number)\n.decl owner(g:number)\n.input seed,owner\n\
         .decl item(g:number,v:number)\n.decl amount(g:number,v:max<>) widen 20\n",
    );
    for i in 0..36 {
        program.push_str(&format!("item(g,v) :- seed(g,v), v >= {i}.\n"));
    }
    program.push_str(
        "item(g,v) :- amount(g,v), v < 4.\n\
         amount(g,n) :- owner(g), n=count : {item(g,_)}.\n\
         .output item,amount\n",
    );
    e.set_limits(json!({"max_submissions":100}).to_string())
        .unwrap();
    let bind = |e: &mut CodeRuleSqlExecutor, seeds: Value| {
        e.bind_relation(json!({"name":"seed","columns":["g","v"],"rows":seeds}).to_string())
            .unwrap();
        e.bind_relation(json!({"name":"owner","columns":["g"],"rows":[[0],[1]]}).to_string())
            .unwrap();
    };
    bind(&mut e, json!([[0, 0], [1, 3]]));
    e.run(program.clone(), "python".into(), Some(32)).unwrap();
    let before = rows(&mut e, "SELECT * FROM amount ORDER BY c0");
    assert!(e.executions[0].temporal[&0].batch.is_some());
    let edit = json!([{"name":"seed","delete":[[1,3]],"insert":[[0,9]]}]).to_string();
    e.set_limits(json!({"max_submissions":100,"max_steps":1}).to_string())
        .unwrap();
    assert!(e.apply_changes(edit.clone(), Some(32)).is_err());
    e.set_limits(json!({"max_submissions":100}).to_string())
        .unwrap();
    assert_eq!(rows(&mut e, "SELECT * FROM amount ORDER BY c0"), before);
    e.apply_changes(edit, Some(32)).unwrap();
    let (_dir, mut oracle) = executor();
    oracle.cold_oracle = true;
    bind(&mut oracle, json!([[0, 0], [0, 9]]));
    oracle.run(program, "python".into(), Some(32)).unwrap();
    for relation in ["item", "amount", "__sql_changes_amount"] {
        let sql = format!("SELECT * FROM {relation} ORDER BY 1,2");
        assert_eq!(rows(&mut e, &sql), rows(&mut oracle, &sql), "{relation}");
    }
    let history = e.executions[0].temporal[&0].batch.as_ref().unwrap();
    assert_eq!(history.explain()["scope"], "warm");
    assert_eq!(history.explain()["planned_submissions"], 1);
    assert!(e.db.admission().snapshot()["executed"].as_u64().unwrap() <= 100);
}
