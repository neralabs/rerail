use super::*;

#[test]
fn retained_full_rule_contexts_retract_cycles_and_keep_alternate_support() {
    let (_dir, mut e) = executor();
    let regions = 512;
    for (name, columns, values) in [
        (
            "seed",
            vec!["g", "n"],
            json!((0..regions).map(|g| [g, 0]).collect::<Vec<_>>()),
        ),
        ("other", vec!["g", "n"], json!([[0, 0]])),
        (
            "edge",
            vec!["g", "a", "b"],
            json!(
                (0..regions)
                    .flat_map(|g| [[g, 0, 1], [g, 1, 2], [g, 2, 0]])
                    .collect::<Vec<_>>()
            ),
        ),
        ("blocked", vec!["g"], json!([])),
    ] {
        e.bind_relation(json!({"name":name,"columns":columns,"rows":values}).to_string())
            .unwrap();
    }
    let program = ".decl seed(g:number,n:number)\n.decl other(g:number,n:number)\n.decl edge(g:number,a:number,b:number)\n.decl blocked(g:number)\n.input seed,other,edge,blocked\n.decl a(g:number,n:number)\n.decl b(g:number,n:number)\na(g,n) :- seed(g,n), !blocked(g).\na(g,n) :- other(g,n), !blocked(g).\nb(g,y) :- a(g,x), edge(g,x,y).\nb(g,y) :- a(g,x), a(g,y), edge(g,x,y).\na(g,n) :- b(g,n).\na(g,n) :- a(g,n).\n.output a,b\n";
    let run: Value =
        serde_json::from_str(&e.run(program.into(), "python".into(), None).unwrap()).unwrap();
    assert_eq!(run["preparation"]["initial_execution"], "retained");
    let explain: Value = serde_json::from_str(&e.explain().unwrap()).unwrap();
    let schedule = &explain["executions"][0]["recursive"][0]["component_cache_scheduling"][0];
    assert_eq!(schedule["component"], json!(["a", "b"]));
    assert_eq!(schedule["decision"]["proven"], true, "{schedule}");
    assert!(
        schedule["removed_delta_variants"].as_u64().unwrap() > 0,
        "{schedule}"
    );
    let inputs: &[(&str, &[&str])] = &[
        ("seed", &["g", "n"]),
        ("other", &["g", "n"]),
        ("edge", &["g", "a", "b"]),
        ("blocked", &["g"]),
    ];
    temporal::check(&mut e, program, inputs, &["a", "b"]);
    assert_eq!(
        rows(&mut e, "SELECT count(*) FROM a"),
        json!([[regions * 3]])
    );

    for patch in [
        json!([{"name":"seed","insert":[[0,99]]}]),
        json!([{"name":"seed","delete":[[0,0]]}]),
    ] {
        e.apply_changes(patch.to_string(), None).unwrap();
        temporal::check(&mut e, program, inputs, &["a", "b"]);
    }
    assert_eq!(
        rows(&mut e, "SELECT count(*) FROM a WHERE c0=0"),
        json!([[4]])
    );
    let before = rows(&mut e, "SELECT * FROM a ORDER BY c0,c1");
    let patch = json!([{"name":"other","delete":[[0,0]]}]).to_string();
    assert!(e.apply_changes(patch.clone(), Some(1)).is_err());
    assert_eq!(rows(&mut e, "SELECT * FROM a ORDER BY c0,c1"), before);
    assert_eq!(rows(&mut e, "SELECT * FROM other"), json!([[0, 0]]));
    let (result, work) = scaling::profile(&mut e, |e| e.apply_changes(patch, None));
    result.unwrap();
    assert!(
        work.vm.get() < 1_000_000,
        "sparse cycle retraction: {work:?}"
    );
    temporal::check(&mut e, program, inputs, &["a", "b"]);
    assert_eq!(rows(&mut e, "SELECT c1 FROM a WHERE c0=0"), json!([[99]]));
    for patch in [
        json!([{"name":"blocked","insert":[[0]]}]),
        json!([{"name":"seed","insert":[[0,0]]}]),
        json!([{"name":"blocked","delete":[[0]]}]),
        json!([{"name":"seed","delete":(0..regions).map(|g|vec![g,0]).chain([vec![0,99]]).collect::<Vec<_>>()}]),
        json!([{"name":"seed","insert":[[0,0]]}]),
    ] {
        e.apply_changes(patch.to_string(), None).unwrap();
        temporal::check(&mut e, program, inputs, &["a", "b"]);
    }
    assert_eq!(
        rows(&mut e, "SELECT * FROM a ORDER BY c0,c1"),
        json!([[0, 0], [0, 1], [0, 2]])
    );
}
