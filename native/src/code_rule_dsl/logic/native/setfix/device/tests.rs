    use rusqlite::Connection;

    fn hex(s: &str) -> String {
        s.bytes().map(|b| format!("{b:02x}")).collect()
    }

    /// One stratum's seed joins run on the CPU and on the device: the same rows, and the same observed counts per
    /// step. The joins: a scan, a keyed join with a fused comparison and `docof`, a negation, a semi-join, a filter
    /// after a step that binds nothing, a `let`, constants in keys and heads; one rule whose second join's rows
    /// overflow the first guess of its table.
    #[test]
    fn a_strata_seeds_on_the_device_are_the_cpus_rows_and_counts() {
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        let store = crate::code_rule_dsl::fixtures::temp();
        drop(crate::code_rule_dsl::fixtures::open(&store));
        let db = Connection::open(&store).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
        let mut x = 7u64;
        let mut next = |m: i64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((x >> 33) as i64).rem_euclid(m)
        };
        db.execute_batch("CREATE TEMP TABLE a (c0, c1); CREATE TEMP TABLE b (c0, c1); CREATE TEMP TABLE c (c0);").unwrap();
        for _ in 0..3000 {
            db.execute("INSERT INTO a VALUES (?1, ?2)", [((next(4) << 32) | next(500)), next(200)]).unwrap();
        }
        for _ in 0..4000 {
            db.execute("INSERT INTO b VALUES (?1, ?2)", [next(200), ((next(4) << 32) | next(500))]).unwrap();
        }
        for _ in 0..300 {
            db.execute("INSERT INTO c VALUES (?1)", [((next(4) << 32) | next(500))]).unwrap();
        }
        let src = |t: &str, cols: &str, types: &str| format!("t:x:{}:{}:{types}", hex(&format!("temp.{t}")), hex(cols));
        let (a, b, c) = (src("a", "c0, c1", "ii"), src("b", "c0, c1", "ii"), src("c", "c0", "i"));
        let rules = [
            // h0(x, z) :- a(x, y), b(y, z), z < x, @doc(z) = @doc(x), !c(z).
            format!("pos {a} v0 v1\npos {b} v1 v2\nlt v2 v0\ndeq v2 v0\nneg {c} v2\nhead v0 v2"),
            // h0(x, 7) :- a(x, 3), c(x).
            format!("pos {a} v0 i:3\npos {c} v0\nhead v0 i:7"),
            // h0(y, w) :- a(x, y), c(x), w = y, y != 5, d = @doc(x).
            format!("pos {a} v0 v1\npos {c} v0\nne v1 i:5\neq v2 v1\ndocof v3 v0\nhead v1 v2"),
            // h0(x, z) :- a(x, y), b(y, z): many rows (past the first guess of its table).
            format!("pos {a} v0 v1\npos {b} v1 v2\nhead v0 v2"),
        ];
        let mut got = Vec::new();
        for device in [false, true] {
            db.execute_batch("DROP TABLE IF EXISTS temp.h0; CREATE TEMP TABLE h0 (c0, c1);").unwrap();
            let mut spec = vec!["name dev_test".to_owned()];
            if device {
                spec.push("device".into());
            }
            spec.push("rel h0 ii empty".into());
            spec.extend(rules.iter().map(|r| format!("seed 0 {}", hex(r))));
            super::super::open(&db, &spec.join("\n")).unwrap();
            super::super::run(&db, "dev_test", 0).unwrap();
            let report = super::take_report();
            assert_eq!(report.as_ref().is_some_and(|r| r.refused.is_none()), device, "{report:?}");
            if let Some(r) = &report {
                assert!(r.reruns >= 1 && r.submissions >= 2, "the large join overflows its first table: {r:?}");
            }
            let counts = super::super::step_counts(&db, "dev_test").unwrap();
            super::super::close(&db, "dev_test").unwrap();
            let mut rows: Vec<(i64, i64)> = db.prepare("SELECT c0, c1 FROM temp.h0").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
            rows.sort_unstable();
            got.push((rows, counts));
        }
        assert!(got[0].0.len() > 1000, "{} rows", got[0].0.len());
        assert_eq!(got[0].0, got[1].0, "the device's rows are the CPU's");
        assert_eq!(got[0].1, got[1].1, "the device's counts per step are the CPU's");
    }
