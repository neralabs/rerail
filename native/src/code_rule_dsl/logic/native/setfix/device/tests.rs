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
            // h0(x, w) :- a(x, y), s = x + y, w = s / (y - 3), w < x: integer arithmetic, a zero divisor dropped.
            format!("pos {a} v0 v1\nfunc rr_add v2 v0 v1\nfunc rr_sub v3 v1 i:3\nfunc rr_div v4 v2 v3\nlt v4 v0\nhead v0 v4"),
            // h0(x, w) :- a(x, y), w = x * x % 7, max and min: the product overflows past the first document.
            format!("pos {a} v0 v1\nfunc rr_mul v2 v0 v0\nfunc mod v3 v2 i:7\nfunc rr_max v4 v3 v1\nfunc rr_min v5 v4 i:100\nhead v0 v5"),
            // Aggregates. h0(x, n) :- c(x), n = count : { a(x, _) }.
            format!("pos {c} v0\ncount {a} v0 _ v1\nhead v0 v1"),
            // h0(x, s) :- c(x), s = sum y : { a(x, y) }.
            format!("pos {c} v0\nsum {a} v0 _ 1 v1\nhead v0 v1"),
            // h0(z, m) :- b(y, z), m = min x : { a(x, y) }: by another key; a key with no rows has no minimum.
            format!("pos {b} v0 v1\nmin {a} _ v0 0 v2\nhead v1 v2"),
            // h0(x, y) :- a(x, y), y = max y2 : { a(x, y2) }: the value tested, not bound.
            format!("pos {a} v0 v1\nmax {a} v0 _ 1 v1\nhead v0 v1"),
            // h0(x, n) :- c(x), n = count : { b(_, _) }: no keys, the whole relation.
            format!("pos {c} v0\ncount {b} _ _ v1\nhead v0 v1"),
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

    /// A recursive stratum on the device: its rounds to the fixpoint there, the same rows as the CPU's waves.
    #[test]
    fn a_recursive_stratum_on_the_device_is_the_cpus_fixpoint() {
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        let store = crate::code_rule_dsl::fixtures::temp();
        drop(crate::code_rule_dsl::fixtures::open(&store));
        let db = Connection::open(&store).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
        let mut x = 11u64;
        let mut next = |m: i64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((x >> 33) as i64).rem_euclid(m)
        };
        db.execute_batch("CREATE TEMP TABLE e (c0, c1);").unwrap();
        // A sparse graph of chains: its closure takes many rounds.
        for v in 0..100 {
            db.execute("INSERT INTO e VALUES (?1, ?2)", [v, v + 1]).unwrap();
            if next(5) == 0 {
                db.execute("INSERT INTO e VALUES (?1, ?2)", [v, next(100)]).unwrap();
            }
        }
        let e = format!("t:x:{}:{}:ii", hex("temp.e"), hex("c0, c1"));
        // h0(x, y) :- e(x, y).   h0(x, z) :- h0(x, y), h0(y, z), z != x (both atoms recursive: not a closure's walk).
        let seed = format!("pos {e} v0 v1\nhead v0 v1");
        let left = "pos d0 v0 v1\npos c0 v1 v2\nne v2 v0\nhead v0 v2".to_owned();
        let right = "pos c0 v0 v1\npos d0 v1 v2\nne v2 v0\nhead v0 v2".to_owned();
        let mut got = Vec::new();
        let gpu_jobs = || crate::code_rule_dsl::logic::lanes::executor_for(true).score().units[rerail_accel::Lane::Gpu.index()].jobs;
        let jobs0 = gpu_jobs();
        for device in [false, true] {
            db.execute_batch("DROP TABLE IF EXISTS temp.h0; CREATE TEMP TABLE h0 (c0, c1);").unwrap();
            let mut spec = vec!["name dev_rec".to_owned()];
            if device {
                spec.push("device".into());
            }
            spec.push("rel h0 ii empty".into());
            spec.push(format!("seed 0 {}", hex(&seed)));
            spec.push(format!("join 0 0 {}", hex(&left)));
            spec.push(format!("join 0 0 {}", hex(&right)));
            super::super::open(&db, &spec.join("\n")).unwrap();
            super::super::run(&db, "dev_rec", 0).unwrap();
            let report = super::take_report();
            if device {
                let r = report.as_ref().expect("a device report");
                assert!(r.refused.is_none(), "{r:?}");
                assert!(r.rounds > 5, "the closure takes rounds: {r:?}");
            }
            super::super::close(&db, "dev_rec").unwrap();
            let mut rows: Vec<(i64, i64)> = db.prepare("SELECT c0, c1 FROM temp.h0").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
            rows.sort_unstable();
            got.push(rows);
        }
        assert!(got[0].len() > 1000, "{} rows", got[0].len());
        assert_eq!(got[0], got[1], "the device's fixpoint is the CPU's");
        assert!(gpu_jobs() > jobs0, "the device's rounds are a job on the GPU lane's scoreboard");
    }

    /// Text tests on the device: each test's verdicts per distinct symbol made by its automaton, the same rows as
    /// the CPU's matcher gives (names past one chunk of 32 bytes among them).
    #[test]
    fn text_tests_on_the_device_are_the_cpus() {
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        let store = crate::code_rule_dsl::fixtures::temp();
        drop(crate::code_rule_dsl::fixtures::open(&store));
        let db = Connection::open(&store).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
        let mut x = 5u64;
        let mut next = |m: u64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (x >> 33) % m
        };
        let parts = ["foo", "bar", "Src", "src/lib", "test", "_", ".ts", "αβ", "x9", "TODO", "a_b", "/"];
        db.execute_batch("CREATE TEMP TABLE n (c0, c1);").unwrap();
        for k in 0..2000 {
            let len = 1 + next(if k % 50 == 0 { 30 } else { 5 });
            let name: String = (0..len).map(|_| parts[next(parts.len() as u64) as usize]).collect();
            db.execute("INSERT INTO n VALUES (?1, ?2)", rusqlite::params![name, k]).unwrap();
        }
        db.execute_batch("CREATE TEMP TABLE m (c0);").unwrap();
        for k in 0..60 {
            let len = 1 + next(4);
            let name: String = (0..len).map(|_| parts[next(parts.len() as u64) as usize]).collect();
            db.execute("INSERT INTO m VALUES (?1)", rusqlite::params![if k == 0 { String::new() } else { name }]).unwrap();
        }
        let n = format!("t:x:{}:{}:si", hex("temp.n"), hex("c0, c1"));
        let m = format!("t:x:{}:{}:s", hex("temp.m"), hex("c0"));
        let sym = |t: &str| format!("x:{}", hex(t));
        let rules = [
            format!("pos {n} v0 v1\ntest prefix v0 {} 1\nhead v0 v1", sym("foo")),
            format!("pos {n} v0 v1\ntest contains v0 {} 0\nhead v0 v1", sym("_b")),
            format!("pos {n} v0 v1\ntest rx v0 {} 1\nhead v0 v1", sym("^[a-z]+_[a-z]+[0-9]*$")),
            format!("pos {n} v0 v1\ntestset suffix v0 1 {} {}\nhead v0 v1", sym(".ts"), sym("bar")),
            format!("pos {n} v0 v1\neq v2 v0\ntest iprefix v2 {} 1\nhead v0 v1", sym("SRC")),
            format!("pos {n} v0 v1\ntest glob v0 {} 1\nhead v0 v1", sym("src/lib*.ts")),
            format!("pos {n} v0 v1\ntest contains v0 {} 1\ntest suffix v0 {} 0\nhead v0 v1", sym("αβ"), sym("x9")),
            // Ordering by names: against a constant, and between two relations' symbols (an empty name among them).
            format!("pos {n} v0 v1\nfunc str_cmp v2 v0 {}\nlt v2 i:0\nhead v0 v1", sym("foo_bar")),
            format!("pos {m} v0\npos {n} v1 v2\nfunc str_cmp v3 v1 v0\neq v3 i:0\nhead v1 v2"),
            format!("pos {m} v0\npos {n} v1 v2\nlt v2 i:40\nfunc str_cmp v3 v0 v1\nlt i:0 v3\nhead v0 v2"),
            // A cat's text compared: with a constant, with another relation's symbol, and not equal.
            format!("pos {n} v0 v1\ncat v2 v0 {}\neq v2 {}\nhead v0 v1", sym("ts"), sym("foo.ts")),
            format!("pos {m} v0\npos {n} v1 v2\ncat v3 v0 {}\neq v3 v1\nhead v1 v2", sym("ts")),
            format!("pos {n} v0 v1\nlt v1 i:100\ncat v2 v0 {}\nne v2 {}\nlt v1 i:90\nhead v0 v1", sym("x9"), sym("foo.x9")),
            // A concatenation compared (`str_concat`: no "." between, none dropped).
            format!("pos {n} v0 v1\nfunc str_concat v2 v0 {}\neq v2 {}\nhead v0 v1", sym("ts"), sym("foots")),
            format!("pos {m} v0\npos {n} v1 v2\nfunc str_concat v3 v0 {}\neq v3 v1\nhead v1 v2", sym("x9")),
            format!("pos {n} v0 v1\nfunc str_concat v2 {} v0\nne v2 {}\nhead v0 v1", sym("a"), sym("afoo")),
            // Texts made into the head, new symbols: a cat bound, a cat in the head (an empty part among them), a
            // concatenation.
            format!("pos {n} v0 v1\nlt v1 i:700\ncat v2 v0 {}\nhead v2 v1", sym("made")),
            format!("pos {m} v0\npos {n} v1 v2\nlt v2 i:25\nhead cat:v0:v1 v2"),
            format!("pos {n} v0 v1\nlt v1 i:300\nfunc str_concat v2 v0 {}\nhead v2 v1", sym("!")),
            // Bound patterns, pairwise: a relation's names as prefixes, suffixes, equals of another's (an empty one
            // among them), and a cat's text as the pattern.
            format!("pos {m} v0\npos {n} v1 v2\ntest prefix v1 v0 1\nhead v1 v2"),
            format!("pos {m} v0\npos {n} v1 v2\nlt v2 i:400\ntest suffix v1 v0 1\nhead v0 v2"),
            format!("pos {m} v0\npos {n} v1 v2\nlt v2 i:300\ntest suffix v1 v0 0\nhead v0 v2"),
            format!("pos {m} v0\npos {n} v1 v2\ntest equals v1 v0 1\nhead v1 v2"),
            format!("pos {m} v0\npos {n} v1 v2\ncat v3 {} v0\ntest suffix v1 v3 1\nhead v1 v2", sym("")),
            format!("pos {m} v0\npos {n} v1 v2\nfunc str_concat v3 v0 {}\ntest prefix v1 v3 1\nhead v0 v2", sym("_")),
            // A part a let filled with a constant (`q = @dot("builtins", x)`).
            format!("pos {n} v0 v1\nlt v1 i:200\neq v2 {}\ncat v3 v2 v0\nhead v3 v1", sym("builtins")),
            // A cat's text ordered against a name, and its characters.
            format!("pos {m} v0\npos {n} v1 v2\nlt v2 i:200\ncat v3 v0 {}\nfunc str_cmp v4 v3 v1\nlt v4 i:0\nhead v1 v2", sym("x9")),
            format!("pos {m} v0\nfunc str_concat v1 v0 {}\nfunc rr_strlen v2 v1\nhead v0 v2", sym("αβ")),
            // Characters of names (multibyte ones among them, names past a word): strlen bound and compared.
            format!("pos {n} v0 v1\nlt v1 i:300\nfunc rr_strlen v2 v0\nhead v0 v2"),
            format!("pos {m} v0\nfunc rr_strlen v1 v0\nlt i:9 v1\nhead v0 v1"),
            // A cat's text both compared and in the head.
            format!("pos {m} v0\npos {n} v1 v2\nlt v2 i:500\ncat v3 v0 {}\nne v3 v1\ntest prefix v1 v0 1\nhead v3 v2", sym("ts")),
        ];
        let mut got = Vec::new();
        // The device first: the texts it makes are new symbols.
        for device in [true, false] {
            db.execute_batch("DROP TABLE IF EXISTS temp.h0; CREATE TEMP TABLE h0 (c0, c1);").unwrap();
            let mut spec = vec!["name dev_text".to_owned()];
            if device {
                spec.push("device".into());
            }
            spec.push("rel h0 si empty".into());
            spec.extend(rules.iter().map(|r| format!("seed 0 {}", hex(r))));
            super::super::open(&db, &spec.join("\n")).unwrap();
            super::super::run(&db, "dev_text", 0).unwrap();
            let report = super::take_report();
            assert_eq!(report.as_ref().is_some_and(|r| r.refused.is_none()), device, "{report:?}");
            if device {
                assert!(report.as_ref().is_some_and(|r| r.made > 1000), "texts made on the device: {report:?}");
            }
            let counts = super::super::step_counts(&db, "dev_text").unwrap();
            super::super::close(&db, "dev_text").unwrap();
            let mut rows: Vec<(i64, i64)> = db.prepare("SELECT c0, c1 FROM temp.h0").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
            rows.sort_unstable();
            got.push((rows, counts));
        }
        assert!(got[0].0.len() > 500, "{} rows", got[0].0.len());
        let longest: i64 = db.query_row("SELECT max(length(CAST(c0 AS BLOB))) FROM temp.n", [], |r| r.get(0)).unwrap();
        assert!(longest > 64, "a name past two chunks: {longest} bytes");
        assert_eq!(got[1].0, got[0].0, "the device's text tests are the CPU's");
        assert_eq!(got[1].1, got[0].1, "the device's counts per step are the CPU's");
    }

    /// Strata placed on the device, when their driver can wait, go out in one wave: each encodes its joins into the
    /// level's one command buffer and returns `3`; one flush commits it; each then finishes, with the rows it makes
    /// on its own.
    #[test]
    fn strata_on_the_device_wait_for_their_wave() {
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        use crate::code_rule_dsl::logic::lanes::wave;
        let store = crate::code_rule_dsl::fixtures::temp();
        drop(crate::code_rule_dsl::fixtures::open(&store));
        let db = Connection::open(&store).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
        db.execute_batch("CREATE TEMP TABLE n (c0, c1);").unwrap();
        for k in 0..400 {
            db.execute("INSERT INTO n VALUES (?1, ?2)", rusqlite::params![format!("s{}", k % 37), k]).unwrap();
        }
        let n = format!("t:x:{}:{}:si", hex("temp.n"), hex("c0, c1"));
        let strata = [
            ("wave_a", "h0", format!("pos {n} v0 v1\nlt v1 i:100\nhead v0 v1")),
            ("wave_b", "h1", format!("pos {n} v0 v1\npos {n} v0 v2\nlt v1 v2\nlt v2 i:60\nhead v1 v2")),
            ("wave_c", "h2", format!("pos {n} v0 v1\ncat v2 v0 x:{}\nhead v2 v1", hex("w"))),
        ];
        let mut got = Vec::new();
        for waves in [false, true] {
            for (name, h, rule) in &strata {
                db.execute_batch(&format!("DROP TABLE IF EXISTS temp.{h}; CREATE TEMP TABLE {h} (c0, c1);")).unwrap();
                super::super::open(&db, &format!("name {name}\ndevice\nrel {h} si empty\nseed 0 {}", hex(rule))).unwrap();
            }
            if waves {
                let (w0, _) = wave::flushed();
                {
                    let _w = wave::suspendable();
                    for (name, _, _) in &strata {
                        let status = super::super::run(&db, name, 0).unwrap();
                        assert!(status.ends_with(" 3"), "{name} waits for its wave: {status}");
                    }
                }
                assert_eq!(wave::flush(strata.len()).unwrap(), 1, "one commit for the level's strata");
                assert_eq!(wave::flushed().0, w0 + 1);
                for (name, _, _) in &strata {
                    let status = super::super::run(&db, name, 0).unwrap();
                    assert!(!status.ends_with(" 3"), "{name} finished: {status}");
                    let report = super::take_report().expect("a device report");
                    assert!(report.refused.is_none() && report.waved == 1, "{report:?}");
                }
            } else {
                for (name, _, _) in &strata {
                    super::super::run(&db, name, 0).unwrap();
                    assert!(super::take_report().is_some_and(|r| r.refused.is_none() && r.waved == 0));
                }
            }
            let mut rows = Vec::new();
            for (name, h, _) in &strata {
                super::super::close(&db, name).unwrap();
                let mut r: Vec<(i64, i64)> = db.prepare(&format!("SELECT c0, c1 FROM temp.{h}")).unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
                r.sort_unstable();
                assert!(!r.is_empty(), "{name} made rows");
                rows.push(r);
            }
            got.push(rows);
        }
        assert_eq!(got[1], got[0], "the strata's rows after the wave are their own runs'");
    }

    /// Text functions on the device: their texts made into the batch's arena (an arena too small run again in one
    /// as big as its texts asked), read by tests, `str_cmp`, `strlen` and other functions, stored as symbols in the
    /// head: the CPU's rows. `lower` of a name past ASCII is the host's: the stratum runs on the CPU.
    #[test]
    fn text_functions_on_the_device_are_the_cpus() {
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        let store = crate::code_rule_dsl::fixtures::temp();
        drop(crate::code_rule_dsl::fixtures::open(&store));
        let db = Connection::open(&store).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
        let mut x = 7u64;
        let mut next = |m: u64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (x >> 33) % m
        };
        let parts = ["foo", "Bar", "SRC", "src/lib", "a.b", "_", ".ts", "αβ", "x9", "TODO", "a_b", "/", "é"];
        db.execute_batch("CREATE TEMP TABLE n (c0, c1); CREATE TEMP TABLE m (c0); CREATE TEMP TABLE a (c0, c1);").unwrap();
        for k in 0..2000 {
            let len = 1 + next(if k % 50 == 0 { 30 } else { 6 });
            let name: String = (0..len).map(|_| parts[next(parts.len() as u64) as usize]).collect();
            db.execute("INSERT INTO n VALUES (?1, ?2)", rusqlite::params![if k == 7 { String::new() } else { name }, k]).unwrap();
        }
        for k in 0..80 {
            let len = 1 + next(3);
            let name: String = (0..len).map(|_| parts[next(5) as usize]).collect();
            db.execute("INSERT INTO m VALUES (?1)", rusqlite::params![name]).unwrap();
            db.execute("INSERT INTO a VALUES (?1, ?2)", rusqlite::params![name.to_ascii_uppercase(), k]).unwrap();
        }
        let n = format!("t:x:{}:{}:si", hex("temp.n"), hex("c0, c1"));
        let m = format!("t:x:{}:{}:s", hex("temp.m"), hex("c0"));
        let a = format!("t:x:{}:{}:si", hex("temp.a"), hex("c0, c1"));
        let sym = |t: &str| format!("x:{}", hex(t));
        let rules = [
            format!("pos {n} v0 v1\nlt v1 i:800\nfunc sql_lower v2 v0\nhead v2 v1"),
            format!("pos {n} v0 v1\nfunc sql_upper v2 v0\ntest prefix v2 {} 1\nhead v0 v1", sym("SRC")),
            format!("pos {n} v0 v1\nfunc head v2 v0\nhead v2 v1"),
            format!("pos {n} v0 v1\nfunc dir v2 v0\nhead v2 v1"),
            format!("pos {n} v0 v1\nfunc rr_substr v2 v0 i:1 i:4\nhead v2 v1"),
            format!("pos {n} v0 v1\nlt v1 i:40\nfunc slice v2 v0 i:-3 v1\nhead v2 v1"),
            format!("pos {n} v0 v1\nfunc sql_substr v2 v0 i:-2 i:5\nhead v2 v1"),
            format!("pos {n} v0 v1\nlt v1 i:500\nfunc rr_sub v3 i:250 v1\nfunc rr_to_string v2 v3\nhead v2 v1"),
            format!("pos {n} v0 v1\nfunc sql_replace v2 v0 {} {}\nhead v2 v1", sym("_"), sym("--")),
            // Chained: a range of a name, made upper, its characters counted.
            format!("pos {n} v0 v1\nfunc rr_substr v2 v0 i:0 i:6\nfunc sql_upper v3 v2\nfunc rr_strlen v4 v3\nlt v4 i:5\nhead v3 v1"),
            // Compared with another relation's symbols, and ordered against a constant.
            format!("pos {m} v0\npos {n} v1 v2\nfunc head v3 v1\neq v3 v0\nhead v1 v2"),
            format!("pos {n} v0 v1\nfunc dir v2 v0\nfunc str_cmp v3 v2 {}\nlt i:0 v3\nhead v0 v1", sym("src")),
            // Regex replacements: leftmost first, anchored, multibyte classes.
            format!("pos {n} v0 v1\nfunc replace_all v2 v0 {} {}\nhead v2 v1", sym("_+|\\.ts$"), sym("-")),
            format!("pos {n} v0 v1\nfunc replace_all v2 v0 {} {}\nhead v2 v1", sym("^[A-Za-z]+|[0-9]"), sym("#")),
            format!("pos {n} v0 v1\nlt v1 i:900\nfunc rx_replace v2 v0 {} {}\nfunc rr_strlen v3 v2\nhead v2 v3", sym("[αβé]+"), sym("<u>")),
            // Unicode's lower of ASCII names.
            format!("pos {a} v0 v1\nfunc lower v2 v0\nhead v2 v1"),
        ];
        let lowered = [format!("pos {n} v0 v1\nfunc lower v2 v0\nhead v2 v1")];
        for (case, rules, on_device) in [("arena", &rules[..], true), ("past ASCII", &lowered[..], false)] {
            let mut got = Vec::new();
            for device in [true, false] {
                db.execute_batch("DROP TABLE IF EXISTS temp.h0; CREATE TEMP TABLE h0 (c0, c1);").unwrap();
                let mut spec = vec!["name dev_fn".to_owned()];
                if device {
                    spec.push("device".into());
                }
                spec.push("rel h0 si empty".into());
                spec.extend(rules.iter().map(|r| format!("seed 0 {}", hex(r))));
                super::super::open(&db, &spec.join("\n")).unwrap();
                super::super::run(&db, "dev_fn", 0).unwrap();
                let report = super::take_report();
                if device {
                    let r = report.as_ref().expect("a device report");
                    assert_eq!(r.refused.is_none(), on_device, "{case}: {r:?}");
                    if on_device {
                        assert!(r.made > 2000 && r.arena_reruns >= 1, "{case}: texts made in an arena that filled once: {r:?}");
                    }
                }
                let counts = super::super::step_counts(&db, "dev_fn").unwrap();
                super::super::close(&db, "dev_fn").unwrap();
                let mut rows: Vec<(String, i64)> = db
                    .prepare("SELECT sym_text(c0), c1 FROM temp.h0")
                    .unwrap()
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                rows.sort_unstable();
                got.push((rows, counts));
            }
            assert!(got[0].0.len() > 100, "{case}: {} rows", got[0].0.len());
            assert_eq!(got[1].0, got[0].0, "{case}: the device's text functions are the CPU's");
            assert_eq!(got[1].1, got[0].1, "{case}: the device's counts per step are the CPU's");
        }
    }
