//! Arbitrary positive CTEs participate in the global initial schedule and sparse edits.
use super::super::super::logic::sql::{
    select::{Cte, FromItem, Query, Relation as Rel, Select},
    sql_ast::SqlExpression as E,
};
use super::*;

const PROGRAM: &str = ".decl root(n:number)\n.decl edge(a:number,b:number,w:number)\n.input root,edge\n.decl distance(n:number,d:min<>) widen 3\ndistance(n,0) :- root(n).\ndistance(b,d+w) :- distance(a,d),edge(a,b,w).\n.output distance\n";

#[test]
fn named_positive_ctes_use_one_initial_schedule_and_bounded_edits() {
    for (groups, arms, distinct) in [
        (32, 1, false),
        (512, 1, false),
        (32, 2, false),
        (512, 2, false),
        (32, 2, true),
        (512, 2, true),
    ] {
        let (_directory, mut e) = executor();
        let edges = (0..groups)
            .flat_map(|g| {
                [(0, 1, 5), (0, 2, 1), (2, 1, 1), (1, 3, 2), (3, 2, 1)]
                    .map(move |(a, b, w)| vec![g * 10 + a, g * 10 + b, w])
            })
            .collect::<Vec<_>>();
        for (name, columns, data) in [
            ("edge", json!(["a", "b", "w"]), json!(edges)),
            (
                "root",
                json!(["n"]),
                json!((0..groups).map(|g| vec![g * 10]).collect::<Vec<_>>()),
            ),
        ] {
            e.bind_relation(json!({"name":name,"columns":columns,"rows":data}).to_string())
                .unwrap();
        }
        e.run(PROGRAM.into(), "python".into(), None).unwrap();
        let (_oracle_directory, mut oracle) = executor();
        oracle.cold_oracle = true;
        for (name, columns) in [("edge", json!(["a", "b", "w"])), ("root", json!(["n"]))] {
            oracle.bind_relation(json!({"name":name,"columns":columns,"rows":rows(&e,&format!("SELECT * FROM {name}"))}).to_string()).unwrap();
        }
        oracle.run(PROGRAM.into(), "python".into(), None).unwrap();
        let expected = rows(&oracle, "SELECT * FROM distance ORDER BY c0");
        let counters = rows(&oracle, "SELECT * FROM __sql_changes_distance ORDER BY c0");
        let frontiers = rows(
            &oracle,
            "SELECT * FROM wave_delta WHERE rows>0 ORDER BY wave,rel",
        );
        let mut run = e.executions[0].clone();
        run.temporal.clear();
        let emitted = Arc::make_mut(&mut run.emitted);
        let old = emitted.queries[0].take().unwrap();
        let width = old.body.columns.len();
        let mut ctes = old.ctes;
        let mut seed = Cte::walk("extension_seed", None, vec![old.body; arms]);
        seed.distinct = distinct;
        ctes.push(seed);
        let query = Query {
            ctes,
            body: Select {
                distinct: true,
                columns: (0..width)
                    .map(|c| {
                        (
                            E::Qualified {
                                alias: "x".into(),
                                column: format!("c{c}"),
                            },
                            Some(format!("c{c}")),
                        )
                    })
                    .collect(),
                from: vec![FromItem::cross(Rel::Cte("extension_seed".into()), "x")],
                ..Default::default()
            },
        };
        emitted.program.steps[0].sql =
            format!("INSERT OR IGNORE INTO new_distance {}", query.render());
        emitted.queries[0] = Some(query);
        let original = emitted.program.clone();
        e.db.execute_batch("DELETE FROM distance;DELETE FROM wave_delta")
            .unwrap();
        let old_counts = e.versions.version("__sql_changes_distance");
        e.versions
            .replace("__sql_changes_distance", old_counts.empty_like())
            .unwrap();
        let tx = e.versions.transaction(&e.db).unwrap();
        let mut work = super::super::maintenance::Work::new(&e.db, &e.scratch);
        work.versions = Some(&e.versions);
        let waves = super::super::temporal::initial::run(
            &mut work,
            &e.versions,
            &mut run,
            777,
            &original,
            false,
            BTreeSet::new(),
        )
        .unwrap();
        assert!(
            waves.is_some(),
            "positive checked CTE must avoid cold/retained fallback"
        );
        work.cleanup().unwrap();
        let pool = work.pool;
        tx.commit();
        e.scratch = pool;
        let owned = e
            .generated
            .iter()
            .map(|(n, _)| n.clone())
            .collect::<BTreeSet<_>>();
        e.generated.extend(
            run.objects
                .iter()
                .filter(|(n, _)| !owned.contains(n))
                .cloned(),
        );
        e.executions[0] = run;
        assert_eq!(rows(&e, "SELECT * FROM distance ORDER BY c0"), expected);
        assert_eq!(
            rows(&e, "SELECT * FROM __sql_changes_distance ORDER BY c0"),
            counters
        );
        assert_eq!(
            rows(
                &e,
                "SELECT * FROM wave_delta WHERE rows>0 ORDER BY wave,rel"
            ),
            frontiers
        );
        let patch = json!([{"name":"edge","delete":[[2,1,1]]}]).to_string();
        assert!(e.apply_changes(patch.clone(), Some(1)).is_err());
        assert_eq!(rows(&e, "SELECT * FROM distance ORDER BY c0"), expected);
        for patch in [
            patch,
            json!([{"name":"root","delete":[[0]]}]).to_string(),
            json!([{"name":"root","insert":[[0]]}]).to_string(),
        ] {
            let (_, work) =
                super::scaling::profile(&mut e, |e| e.apply_changes(patch, None).unwrap());
            assert!(work.vm.get() < 50_000, "{groups}: {work:?}");
            for (name, columns) in [("edge", json!(["a", "b", "w"])), ("root", json!(["n"]))] {
                oracle.bind_relation(json!({"name":name,"columns":columns,"rows":rows(&e,&format!("SELECT * FROM {name}"))}).to_string()).unwrap();
            }
            oracle.run(PROGRAM.into(), "python".into(), None).unwrap();
            assert_eq!(
                rows(&e, "SELECT * FROM distance ORDER BY c0"),
                rows(&oracle, "SELECT * FROM distance ORDER BY c0")
            );
            assert_eq!(
                rows(&e, "SELECT * FROM __sql_changes_distance ORDER BY c0"),
                rows(&oracle, "SELECT * FROM __sql_changes_distance ORDER BY c0")
            );
        }
    }
}
