use super::super::tests::{executor, rows};
use super::*;

#[test]
fn recursive_sql_and_host_components_match_complete_sets_and_limits() {
    let (_dir, mut e) = executor();
    e.bind_relation(json!({"name":"edge","columns":["a","b"],"rows":(0..24).map(|i|vec![i,i+1]).chain([vec![24,0]]).collect::<Vec<_>>()} ).to_string()).unwrap();
    let program = ".decl edge(a:number,b:number)\n.input edge\n.decl reach(a:number,b:number)\nreach(a,b) :- edge(a,b).\nreach(a,c) :- reach(a,b),edge(b,c).\n.output reach\n";
    e.run(program.into(), "python".into(), Some(128)).unwrap();
    let before = rows(&mut e, "SELECT * FROM reach ORDER BY c0,c1");
    let compared: Value =
        serde_json::from_str(&e.compare_recursive_components(Some(128), false).unwrap()).unwrap();
    let result = &compared["components"][0];
    assert_eq!(result["parity"], true, "{compared}");
    assert_eq!(result["rows"]["reach"], 625);
    assert_eq!(result["sql"]["submissions"], 1);
    assert!(result["host"]["submissions"].as_u64().unwrap() > 20);
    assert_eq!(
        result["host"]["new_rows_per_iteration"],
        result["sql"]["new_rows_per_iteration"]
    );
    assert!(result["sql"]["vm_steps"].as_i64().unwrap() > 0);
    assert_eq!(rows(&mut e, "SELECT * FROM reach ORDER BY c0,c1"), before);
    assert!(
        e.compare_recursive_components(Some(1), false)
            .unwrap_err()
            .reason
            .contains("no fixpoint")
    );
    let (_, plan) = eligibility(&e.executions[0].emitted, &BTreeSet::from(["reach".into()]));
    let plan = plan.unwrap();
    assert!(
        execute(&e.db, &plan, true, 1)
            .err()
            .unwrap()
            .reason
            .contains("no fixpoint")
    );
    assert!(e.db.prepare("SELECT * FROM __rr_component_rows").is_err());
    e.set_limits(json!({"max_steps":1}).to_string()).unwrap();
    assert!(e.compare_recursive_components(Some(128), false).is_err());
    e.set_limits("{}".into()).unwrap();
    e.compare_recursive_components(Some(128), true).unwrap();
    assert_eq!(rows(&mut e, "SELECT * FROM reach ORDER BY c0,c1"), before);
}

#[test]
fn mutually_recursive_components_and_external_summaries_keep_sql_novelty() {
    let (_dir, mut e) = executor();
    e.bind_relation(
        json!({"name":"edge","columns":["a","b"],"rows":[[0,1],[1,2],[2,3],[3,0]]}).to_string(),
    )
    .unwrap();
    e.bind_relation(json!({"name":"seed","columns":["n"],"rows":[[0]]}).to_string())
        .unwrap();
    e.bind_relation(json!({"name":"blocked","columns":["n"],"rows":[]}).to_string())
        .unwrap();
    let program = ".decl edge(a:number,b:number)\n.decl seed(n:number)\n.decl blocked(n:number)\n.input edge,seed,blocked\n.decl a(n:number)\n.decl b(n:number,m:number)\na(n) :- seed(n).\nb(m,n) :- a(n),edge(n,m), !blocked(m).\na(n) :- b(n,_), k=count : {seed(_)}, k>0.\n.output a,b\n";
    e.run(program.into(), "python".into(), Some(128)).unwrap();
    let compared: Value =
        serde_json::from_str(&e.compare_recursive_components(Some(128), false).unwrap()).unwrap();
    let result = &compared["components"][0];
    assert_eq!(result["component"], json!(["a", "b"]));
    assert_eq!(result["parity"], true, "{compared}");
    assert_eq!(result["rows"]["a"], 4);
    assert_eq!(result["rows"]["b"], 4);
    assert_eq!(
        result["host"]["new_rows_per_iteration"],
        result["sql"]["new_rows_per_iteration"]
    );
    let explain:Value=serde_json::from_str(&e.explain().unwrap()).unwrap();
    let analyses = explain["executions"][0]["component_loop_analysis"].as_array().unwrap();
    assert!(
        analyses
            .iter()
            .any(|item| item["component"] == json!(["a", "b"])
                && item["analysis"]["decision"]["proven"] == true)
    );
}
