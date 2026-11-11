use super::*;

#[test]
fn projections_and_mixed_scalars_use_one_retained_graph_with_exact_supports() {
    for recursive in [false, true] {
        let (_dir, mut e) = executor();
        e.bind_relation(
            json!({"name":"seed","columns":["id","v"],"rows":[[0,7],[1,7],[2,9]]}).to_string(),
        )
        .unwrap();
        let p = format!(
            ".decl seed(id:number,v:number)\n.input seed\n.decl names(v:number)\nnames(v) :- seed(_,v).\n{}\n.decl projected(v:number,id:number)\nprojected(v,id) :- seed(id,v).\n.decl total(n:number)\ntotal(n) :- n=count : {{seed(_,_)}}.\n.output names,projected,total\n",
            if recursive {
                "names(v) :- names(v)."
            } else {
                ""
            }
        );
        let run: Value =
            serde_json::from_str(&e.run(p.clone(), "python".into(), None).unwrap()).unwrap();
        assert_eq!(run["preparation"]["initial_execution"], "retained", "{run}");
        assert!(e.executions[0].temporal.contains_key(&0));
        for name in ["names", "projected", "total"] {
            assert!(
                !e.generated
                    .iter()
                    .any(|(n, _)| n == &format!("new_{name}") || n == &format!("delta_{name}"))
            );
        }
        assert_eq!(
            rows(
                &mut e,
                "SELECT count(*) FROM temp.sqlite_schema WHERE name IN('names','projected','total')"
            ),
            json!([[3]])
        );
        e.create_index("projected".into(), vec![1]).unwrap();
        let public = rows(
            &mut e,
            "SELECT rootpage FROM temp.sqlite_schema WHERE name='projected'",
        );
        e.run(".decl total(n:number)\n.input total\n.decl guarded(v:number)\nguarded((n-3)+9223372036854775807) :- total(n).\n.output guarded\n".into(),"python".into(),None).unwrap();
        let old_guard = rows(&mut e, "SELECT * FROM guarded");
        // The second program fails after the first program has published its new public roots.
        assert!(
            e.apply_changes(json!([{"name":"seed","insert":[[3,11]]}]).to_string(), None)
                .is_err()
        );
        assert_eq!(rows(&mut e, "SELECT * FROM total"), json!([[3]]));
        assert_eq!(
            rows(&mut e, "SELECT * FROM names ORDER BY c0"),
            json!([[7], [9]])
        );
        assert_eq!(rows(&mut e, "SELECT * FROM guarded"), old_guard);
        if recursive {
            assert!(
                e.apply_changes(
                    json!([{"name":"seed","insert":[[3,11]]}]).to_string(),
                    Some(1)
                )
                .is_err()
            );
            assert_eq!(rows(&mut e, "SELECT count(*) FROM seed"), json!([[3]]));
            assert_eq!(
                rows(&mut e, "SELECT * FROM names ORDER BY c0"),
                json!([[7], [9]])
            );
        }
        for (patch, expected) in [
            (json!([{"name":"seed","delete":[[0,7]]}]), json!([[7], [9]])),
            (
                json!([{"name":"seed","delete":[[1,7]],"insert":[[1,9]]}]),
                json!([[9]]),
            ),
            (json!([{"name":"seed","delete":[[1,9],[2,9]]}]), json!([])),
            (json!([{"name":"seed","insert":[[3,11]]}]), json!([[11]])),
        ] {
            e.apply_changes(patch.to_string(), None).unwrap();
            assert_eq!(rows(&mut e, "SELECT * FROM names ORDER BY c0"), expected);
            assert_eq!(
                rows(
                    &mut e,
                    "SELECT rootpage FROM temp.sqlite_schema WHERE name='projected'"
                ),
                public
            );
            assert_eq!(
                rows(
                    &mut e,
                    "SELECT count(*) FROM temp.sqlite_schema WHERE name='__sql_index_projected_1'"
                ),
                json!([[1]])
            );
            let (_cold_dir, mut cold) = executor();
            cold.cold_oracle = true;
            cold.bind_relation(
                json!({"name":"seed","columns":["id","v"],"rows":rows(&mut e,"SELECT * FROM seed")})
                    .to_string(),
            )
            .unwrap();
            cold.run(p.clone(), "python".into(), None).unwrap();
            for sql in [
                "SELECT * FROM names ORDER BY c0",
                "SELECT * FROM projected ORDER BY c0,c1",
                "SELECT * FROM total",
            ] {
                assert_eq!(rows(&mut e, sql), rows(&mut cold, sql), "{sql}");
            }
        }
    }
}
