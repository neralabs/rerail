    use rusqlite::Connection;

    fn hex(s: &str) -> String {
        s.bytes().map(|b| format!("{b:02x}")).collect()
    }

    /// One stratum's seed joins run on the CPU and on the device: the same rows, and the same observed counts per
    /// step. The joins: a scan, a keyed join with a fused comparison and `docof`, a negation, a semi-join, a filter
    /// after a step that binds nothing, a `let`, constants in keys and heads; one rule whose second join's rows
    /// overflow the first guess of its table. The rules' rows, one head's, are written in place on the device
    /// (decision 0325 step 4): made distinct across the rules, and, with the head holding rows before the run (some
    /// of them derived again, some not), less those, read where they lie.
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
        for held in [false, true] {
        let mut got = Vec::new();
        for device in [false, true] {
            db.execute_batch("DROP TABLE IF EXISTS temp.h0; CREATE TEMP TABLE h0 (c0, c1);").unwrap();
            if held {
                // Rows the fourth rule derives again, and rows no rule derives.
                db.execute_batch("INSERT INTO temp.h0 SELECT a.c0, b.c1 FROM temp.a JOIN temp.b ON a.c1 = b.c0 LIMIT 700;").unwrap();
                for v in 0..300 {
                    db.execute("INSERT INTO temp.h0 VALUES (?1, ?2)", [-1 - v, v]).unwrap();
                }
            }
            let mut spec = vec!["name dev_test".to_owned()];
            if device {
                spec.push("device".into());
            }
            spec.push(if held { "rel h0 ii".into() } else { "rel h0 ii empty".into() });
            spec.extend(rules.iter().map(|r| format!("seed 0 {}", hex(r))));
            super::super::open(&db, &spec.join("\n")).unwrap();
            super::super::run(&db, "dev_test", 0).unwrap();
            let report = super::take_report();
            assert_eq!(report.as_ref().is_some_and(|r| r.refused.is_none()), device, "{report:?}");
            if let Some(r) = &report {
                // The first run only: the next guesses the join's growth from it (`GROWTH`).
                assert!(held || r.reruns >= 1 && r.submissions >= 2, "the large join overflows its first table: {r:?}");
                assert!(r.placed > 1000 && r.placed == r.distinct, "the head's rows written in place: {r:?}");
                assert!(!held || r.in_place >= 1, "the rows held read in place: {r:?}");
            }
            let counts = super::super::step_counts(&db, "dev_test").unwrap();
            super::super::close(&db, "dev_test").unwrap();
            let mut rows: Vec<(i64, i64)> = db.prepare("SELECT c0, c1 FROM temp.h0").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
            rows.sort_unstable();
            got.push((rows, counts));
        }
        assert!(got[0].0.len() > 1000, "{} rows", got[0].0.len());
        assert_eq!(got[0].0, got[1].0, "the device's rows are the CPU's (held: {held})");
        assert_eq!(got[0].1, got[1].1, "the device's counts per step are the CPU's (held: {held})");
        }
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
                // Written in place into the head's block (decision 0325 step 4): none read back.
                assert!(r.placed > 1000 && r.placed == r.distinct, "the fixpoint's rows written in place: {r:?}");
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

    /// A recursive component that keeps levels (decision 0311): the device's rows and each row's level are the CPU's,
    /// for a set head (its rows appended a round at a time) and a `min` lattice head (its values improved in place,
    /// the level column `lat_merge` writes).
    #[test]
    fn a_leveled_recursion_on_the_device_has_the_cpus_levels() {
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        let store = crate::code_rule_dsl::fixtures::temp();
        drop(crate::code_rule_dsl::fixtures::open(&store));
        let db = Connection::open(&store).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
        let mut x = 5u64;
        let mut next = |m: i64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((x >> 33) as i64).rem_euclid(m)
        };
        db.execute_batch("CREATE TEMP TABLE e (c0, c1);").unwrap();
        // Chains with shortcuts: a node is reached first by a long route, then by a shorter one in a later round.
        for v in 0..80 {
            db.execute("INSERT INTO e VALUES (?1, ?2)", [v, v + 1]).unwrap();
            if next(4) == 0 {
                db.execute("INSERT INTO e VALUES (?1, ?2)", [v, v + 2 + next(6)]).unwrap();
            }
        }
        let e = format!("t:x:{}:{}:ii", hex("temp.e"), hex("c0, c1"));
        // reach(x, y) :- e(x, y).  reach(x, z) :- reach(x, y), e(y, z).
        let reach = (None, format!("pos {e} v0 v1\nhead v0 v1"), format!("pos d0 v0 v1\npos {e} v1 v2\nhead v0 v2"));
        // dist(y, 1) :- e(0, y).  dist(z, d + 1) :- dist(y, d), e(y, z).  (`min` by y)
        let dist = (Some(format!("lattice 0 1 {} -", hex("min"))), format!("pos {e} i:0 v0\nhead v0 i:1"), format!("pos d0 v0 v1\npos {e} v0 v2\nfunc rr_add v3 v1 i:1\nhead v2 v3"));
        for (lattice, seed, join) in [reach, dist] {
            let mut got: Vec<Vec<(i64, i64, u32)>> = Vec::new();
            for device in [false, true] {
                db.execute_batch("DROP TABLE IF EXISTS temp.h0; CREATE TEMP TABLE h0 (c0, c1);").unwrap();
                let mut spec = vec!["name dev_lev".to_owned()];
                if device {
                    spec.push("device".into());
                }
                spec.push("rel h0 ii empty".into());
                spec.extend(lattice.clone());
                spec.push("leveled 0".into());
                spec.push(format!("seed 0 {}", hex(&seed)));
                spec.push(format!("join 0 0 {}", hex(&join)));
                super::super::open(&db, &spec.join("\n")).unwrap();
                super::super::run(&db, "dev_lev", 0).unwrap();
                let report = super::take_report();
                if device {
                    let r = report.as_ref().expect("a device report");
                    assert!(r.refused.is_none(), "{r:?}");
                    assert!(r.rounds > 3, "the recursion takes rounds: {r:?}");
                    // A set head's rows written in place with their rounds (decision 0325 step 4); a lattice's folded
                    // in place in its one relation (decision 0384 step 5, R3). Every row placed: none read back (R4).
                    assert!(r.placed > 0, "{r:?}");
                    assert_eq!(r.placed, r.distinct, "no head row read back: {r:?}");
                }
                super::super::close(&db, "dev_lev").unwrap();
                let held = super::super::held(&db, "h0").expect("the relation held");
                let rows = held.live_rows().expect("its rows");
                let levels = held.live_levels().expect("a level per row");
                let mut rows: Vec<(i64, i64, u32)> = rows.chunks(2).zip(levels).map(|(r, l)| (r[0], r[1], l)).collect();
                rows.sort_unstable();
                got.push(rows);
            }
            assert!(got[0].len() > 50, "{} rows", got[0].len());
            assert!(got[0].iter().any(|r| r.2 > 3), "rows of later rounds");
            assert_eq!(got[0], got[1], "the device's rows and levels are the CPU's ({lattice:?})");
        }
    }

    /// Decision 0384 step 5 (R5): the device folds a `min` lattice's rounds into its one relation through its key map
    /// in its block; the relation takes the rows as they lie and the map as its own, and a CPU fold goes on in that
    /// map (the device's slots are the host's: one hash, one layout). Its rows and each key's changes are those of a
    /// fold made on the CPU alone.
    #[test]
    fn a_cpu_fold_goes_on_in_the_devices_key_map() {
        use crate::code_rule_dsl::facts::rows::{BlockVec, Part};
        use crate::code_rule_dsl::transducer::packed::metal_join as mj;
        use super::super::keys::{KeyMap, SLOT};
        if !mj::available() {
            return;
        }
        let lattice = super::super::TestLattice::min;
        // Round r: keys 0..400 + 300r (new keys each round), each value lower for some keys each round; a key's rows
        // more than once in a round (the fold groups them).
        let round = |r: i64| -> Vec<i64> {
            (0..400 + 300 * r).flat_map(|key| [[key, 1000 - r * (key % 3)], [key, 1001]]).flatten().collect()
        };
        let mut cpu = lattice();
        for r in 0..4 {
            cpu.fold(&round(r));
        }
        let mut rel = lattice();
        let (code, top, after) = rel.device_lattice();
        let lat = mj::LatCode { code, top, after };
        let part = Part::for_writes_with_room(2, 1, 64 << 20).unwrap();
        let mut map: Option<KeyMap> = None;
        let mut had = 0usize;
        for r in 0..3 {
            let rows = round(r);
            let n = rows.len() / 2;
            assert!(part.reserve((had + n) as u64));
            let need = (2 * (had + n)).next_power_of_two().max(16);
            let old = match map.take() {
                Some(m) if m.places() >= need => {
                    map = Some(m);
                    None
                }
                old => {
                    // SAFETY: the fold's first pass writes every slot before any read.
                    map = Some(KeyMap::of(unsafe { BlockVec::unwritten_in(Some(&part.home()), need * SLOT) }, had));
                    Some(old)
                }
            };
            let m = map.as_ref().unwrap();
            let first = match &old {
                None => mj::MapFirst::Ready,
                Some(None) => mj::MapFirst::Clear,
                Some(Some(o)) => mj::MapFirst::Grow(super::map_at(o)),
            };
            assert!(r == 0 || matches!(first, mj::MapFirst::Grow(_)), "the map grows on the device");
            let r_rel = mj::DevRel::in_block(&part.in_block(had as u32), 2).unwrap();
            let u = mj::Table::of_rows(2, &rows).unwrap();
            let changed = mj::Table::new(2, n).unwrap();
            let counters = mj::Words::zeroed(2).unwrap();
            let mut batch = super::direct_batch(None).unwrap();
            batch.lattice_fold(&u, n, &r_rel, had, 1, lat, super::map_at(m), first, &counters, &changed, None, None).unwrap();
            batch.finish().unwrap();
            drop(old);
            let added = counters.get(1) as usize;
            // SAFETY: the batch finished; `lat_merge` wrote every new key's row.
            unsafe { part.written_more(added as u32) };
            had += added;
            map.as_mut().unwrap().held(had);
        }
        assert_eq!(had, 400 + 300 * 2, "a row a key");
        let map = map.unwrap();
        assert!(map.facts().is_some_and(|f| std::sync::Arc::ptr_eq(f, part.home().facts())), "the map in its relation's block");
        assert!(rel.take_folded(part, map));
        assert_eq!(rel.len(), had, "the rows taken as they lie");
        // The CPU's fold goes on in the device's map: held keys found, new keys put, the map grown.
        rel.fold(&round(3));
        let (got, want) = (rel.live(), cpu.live());
        assert_eq!(got.len(), 400 + 300 * 3);
        assert_eq!(got, want, "the device's fold, then the CPU's, is the CPU's alone");
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

    /// Decision 0386, the queue's bound: a stratum that waits for its wave leaves no command buffer open on its
    /// thread. The wave keeps its strata's committed work and waits once when flushed; an open one held by a thread
    /// that then waits (or goes idle, as a pool's thread after its job) holds one of the queue's 64 until a thread
    /// asking for one more waits for ever (every thread of a 300-file Metal run did, 2026-11-17).
    #[test]
    fn a_stratum_waiting_for_its_wave_leaves_no_command_buffer_open() {
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        use crate::code_rule_dsl::logic::lanes::wave;
        let db = wave_db(400);
        let n = format!("t:x:{}:{}:si", hex("temp.n"), hex("c0, c1"));
        let strata = [("open_a", "h0", format!("pos {n} v0 v1\nlt v1 i:100\nhead v0 v1")), ("open_b", "h1", format!("pos {n} v0 v1\npos {n} v0 v2\nlt v1 v2\nlt v2 i:60\nhead v1 v2"))];
        for (name, h, rule) in &strata {
            open_stratum(&db, name, h, rule);
        }
        {
            let _w = wave::suspendable();
            for (name, _, _) in &strata {
                let status = super::super::run(&db, name, 0).unwrap();
                assert!(status.ends_with(" 3"), "{name} waits for its wave: {status}");
                assert_eq!(rerail_accel::exec::uncommitted_here(), 0, "{name}'s joins are committed: the wave holds no open command buffer");
            }
        }
        wave::flush(strata.len()).unwrap();
        for (name, h, _) in &strata {
            let status = super::super::run(&db, name, 0).unwrap();
            assert!(!status.ends_with(" 3"), "{name} finished: {status}");
            super::super::close(&db, name).unwrap();
            assert!(!stratum_rows(&db, h).is_empty(), "{name} made rows");
        }
    }

    /// Decision 0386, a job's wave: a job that starts on a thread with a wave open (a wait running the pool's jobs)
    /// opens one of its own, and the wave it leaves unflushed is finished when it ends, on its thread: its stratum
    /// resumes with nothing else flushed, and the outer wave holds the outer stratum alone. Before, the job's stratum
    /// joined the outer wave and was left for a flush on the thread that owned it: a job resumed elsewhere found its
    /// joins "not submitted" and ran on the CPU (20 strata of the 300-file Metal run).
    #[test]
    fn a_jobs_wave_is_its_own_and_ends_with_it() {
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        use crate::code_rule_dsl::logic::lanes::wave;
        wave::install_job_scope();
        let db = wave_db(400);
        let n = format!("t:x:{}:{}:si", hex("temp.n"), hex("c0, c1"));
        let outer = format!("pos {n} v0 v1\nlt v1 i:100\nhead v0 v1");
        let inner = format!("pos {n} v0 v1\npos {n} v0 v2\nlt v1 v2\nlt v2 i:60\nhead v1 v2");
        // The inner stratum's rows run alone, for the comparison.
        open_stratum(&db, "inner_alone", "h2", &inner);
        super::super::run(&db, "inner_alone", 0).unwrap();
        super::super::close(&db, "inner_alone").unwrap();
        let want = stratum_rows(&db, "h2");
        assert!(!want.is_empty());
        open_stratum(&db, "outer", "h0", &outer);
        open_stratum(&db, "inner", "h1", &inner);
        let _w = wave::suspendable();
        let status = super::super::run(&db, "outer", 0).unwrap();
        assert!(status.ends_with(" 3"), "the outer stratum waits for its wave: {status}");
        // A job on this thread, the outer wave open: it waits for a wave and ends without flushing it.
        let status = rerail_accel::exec::as_job("a test's job", || super::super::run(&db, "inner", 0).unwrap());
        assert!(status.ends_with(" 3"), "the job's stratum waited for its wave: {status}");
        assert_eq!(rerail_accel::exec::uncommitted_here(), 0);
        // Its wave was finished as the job ended: it resumes now, nothing flushed.
        let status = super::super::run(&db, "inner", 0).unwrap();
        assert!(!status.ends_with(" 3"), "the job's stratum finished: {status}");
        super::super::close(&db, "inner").unwrap();
        assert_eq!(stratum_rows(&db, "h1"), want, "the job's stratum's rows are its own run's");
        // The outer wave holds the outer stratum's work alone: one deferred submission, its own.
        assert_eq!(wave::flush(1).unwrap(), 1, "the outer wave: one submission");
        let status = super::super::run(&db, "outer", 0).unwrap();
        assert!(!status.ends_with(" 3"), "the outer stratum finished: {status}");
        super::super::close(&db, "outer").unwrap();
        assert!(!stratum_rows(&db, "h0").is_empty());
    }

    /// A store with `temp.n` of `rows` rows (`s<k % 37>`, k), the native module registered: the wave tests' input.
    fn wave_db(rows: i64) -> Connection {
        let store = crate::code_rule_dsl::fixtures::temp();
        drop(crate::code_rule_dsl::fixtures::open(&store));
        let db = Connection::open(&store).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
        db.execute_batch("CREATE TEMP TABLE n (c0, c1);").unwrap();
        for k in 0..rows {
            db.execute("INSERT INTO n VALUES (?1, ?2)", rusqlite::params![format!("s{}", k % 37), k]).unwrap();
        }
        db
    }

    fn open_stratum(db: &Connection, name: &str, h: &str, rule: &str) {
        db.execute_batch(&format!("DROP TABLE IF EXISTS temp.{h}; CREATE TEMP TABLE {h} (c0, c1);")).unwrap();
        super::super::open(db, &format!("name {name}\ndevice\nrel {h} si empty\nseed 0 {}", hex(rule))).unwrap();
    }

    fn stratum_rows(db: &Connection, h: &str) -> Vec<(i64, i64)> {
        let mut r: Vec<(i64, i64)> = db.prepare(&format!("SELECT c0, c1 FROM temp.{h}")).unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        r.sort_unstable();
        r
    }

    /// Q48: a stratum run on its own (its driver cannot wait) beside an open wave on its thread. The wave's stratum
    /// made the relation's verdicts and its sort in the wave's batch, and the resident cache hands them to the next
    /// stratum at once. The stratum run apart commits its own batch before the wave is flushed: unless the wave's
    /// work goes first, it reads them unwritten (the verdicts all false: no rows).
    #[test]
    fn a_stratum_run_apart_beside_an_open_wave_reads_what_the_wave_made() {
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
        for k in 0..3000_i64 {
            db.execute("INSERT INTO n VALUES (?1, ?2)", rusqlite::params![format!("{}{}", ["foo", "bar", "baz"][(k % 3) as usize], k % 7), k]).unwrap();
        }
        let n = format!("t:x:{}:{}:si", hex("temp.n"), hex("c0, c1"));
        let foo = format!("x:{}", hex("foo"));
        // Both read `n`'s verdicts for one text test and its sort by `v0`.
        let waved = format!("pos {n} v0 v1\ntest prefix v0 {foo} 1\npos {n} v0 v2\nlt v1 v2\nlt v2 i:200\nhead v1 v2");
        let apart = format!("pos {n} v0 v1\ntest prefix v0 {foo} 1\npos {n} v0 v2\nlt v2 v1\nlt v1 i:300\nhead v1 v2");
        fn rows(db: &Connection, h: &str) -> Vec<(i64, i64)> {
            let mut r: Vec<(i64, i64)> = db.prepare(&format!("SELECT c0, c1 FROM temp.{h}")).unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
            r.sort_unstable();
            r
        }
        fn open(db: &Connection, name: &str, h: &str, rule: &str) {
            db.execute_batch(&format!("DROP TABLE IF EXISTS temp.{h}; CREATE TEMP TABLE {h} (c0, c1);")).unwrap();
            super::super::open(db, &format!("name {name}\ndevice\nrel {h} si empty\nseed 0 {}", hex(rule))).unwrap();
        }
        // The stratum run apart, alone, on a thread of its own (the device's resident copies are a thread's): its
        // rows.
        let apart2 = apart.clone();
        let (db, want) = std::thread::spawn(move || {
            open(&db, "alone", "h1", &apart2);
            super::super::run(&db, "alone", 0).unwrap();
            assert!(super::take_report().is_some_and(|r| r.refused.is_none()));
            super::super::close(&db, "alone").unwrap();
            let want = rows(&db, "h1");
            (db, want)
        })
        .join()
        .unwrap();
        assert!(want.len() > 100, "{} rows", want.len());
        // Here no relation is resident yet: the wave's stratum makes the verdicts and the sort.
        open(&db, "in_wave", "h0", &waved);
        open(&db, "beside", "h2", &apart);
        {
            let _w = wave::suspendable();
            let status = super::super::run(&db, "in_wave", 0).unwrap();
            assert!(status.ends_with(" 3"), "it waits for its wave: {status}");
        }
        // Its driver cannot wait: it runs in a batch of its own, the wave still open.
        let status = super::super::run(&db, "beside", 0).unwrap();
        assert!(!status.ends_with(" 3"), "it ran apart: {status}");
        let report = super::take_report().expect("a device report");
        assert!(report.refused.is_none() && report.waved == 0 && report.resident_hits + report.sort_hits > 0, "it read the resident copy: {report:?}");
        super::super::close(&db, "beside").unwrap();
        wave::flush(1).unwrap();
        super::super::run(&db, "in_wave", 0).unwrap();
        super::super::close(&db, "in_wave").unwrap();
        let got = rows(&db, "h2");
        let waved_rows = rows(&db, "h0");
        assert_eq!(got, want, "the stratum run apart reads the verdicts and the sort the wave made");
        assert!(!waved_rows.is_empty());
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
            // Matches past a chunk of 32 bytes (long names, long runs), their ends and starts found across chunks.
            format!("pos {n} v0 v1\nfunc replace_all v2 v0 {} {}\nhead v2 v1", sym("[A-Za-z_./αβ]{3,}"), sym("~")),
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
                        assert!(r.per_symbol >= 12, "{case}: text functions made once per distinct name, chunked: {r:?}");
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

    /// Decision 0325 step 3: a whole view of a source whose rows are in a block (the vtab's `Relation`) is bound
    /// where they lie, nothing uploaded, every source column at its own place; the device's reads of it (its column
    /// statistics, through the generated `clif_rel_get`) are the host's values.
    #[test]
    fn a_sources_rows_in_their_block_are_read_in_place() {
        use crate::code_rule_dsl::facts::vtab::{Relation, Source};
        use crate::code_rule_dsl::logic::device::StatColumn;
        use crate::code_rule_dsl::logic::native::setfix::InRel;
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        let mut x = 0x0325_u64;
        let mut next = |m: u64| { x ^= x << 13; x ^= x >> 7; x ^= x << 17; (x % m) as i64 };
        // Three columns, past several chunks of the block's relation; the view reads its first two.
        let n = 5_000;
        let mut r = Relation::new(&["a", "b", "c"]);
        for i in 0..n {
            r.push(&[i as i64, next(97) - 40, (next(1 << 20) << 20) | i as i64]);
        }
        let src: std::sync::Arc<dyn Source> = std::sync::Arc::new(r);
        assert!(src.in_block().is_some_and(|b| b.rows as usize == n), "the source's rows are one relation in a block");
        let rel = InRel::over_source(std::sync::Arc::clone(&src), vec![b'i'; 3]);
        let mut batch = crate::code_rule_dsl::transducer::packed::metal_join::Batch::new().expect("the device");
        let mut rep = super::Report::default();
        let (dev, _, place) = super::resident(&rel, &[1, 2], &[], &mut batch, &mut rep).expect("resident");
        assert_eq!((rep.in_place, rep.uploads, rep.upload_bytes), (1, 0, 0), "read in place: {rep:?}");
        assert_eq!((dev.rows, dev.cols), (n, 3));
        assert_eq!(place, [0, 1, 2], "each column at its own place");
        let cols = [StatColumn::Rel(&rel, 1), StatColumn::Rel(&rel, 2)];
        let (read, in_place) = super::column_stats(&cols, 8).expect("the device");
        assert_eq!(in_place, 2);
        for (c, got) in [1, 2].into_iter().zip(&read) {
            let values: Vec<i64> = (0..n).map(|i| src.value(i, c)).collect();
            let (host, _) = super::column_stats(&[StatColumn::Values(&values)], 8).expect("the device");
            assert_eq!(*got, host[0], "column {c}");
        }
    }

    /// Decision 0261: the device's column statistics read a relation it holds in place (its resident upload), and
    /// are the same as over the values gathered on the host; a relation it does not hold is gathered.
    #[test]
    fn column_statistics_read_a_resident_relation_in_place() {
        use crate::code_rule_dsl::logic::device::StatColumn;
        use crate::code_rule_dsl::logic::native::setfix::InRel;
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        let mut x = 0x0223_u64;
        let mut next = |m: u64| { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x % m };
        let n = 6_000;
        let mut rows = Vec::with_capacity(2 * n);
        for _ in 0..n {
            let r = next(1000);
            rows.push(3_288_784_809_065_000_000 + (if r < 600 { next(20) } else { next(2_000) }) as i64);
            rows.push(if next(2) == 0 { -(1i64 << 40) } else { 1i64 << 40 } + next(9) as i64);
        }
        let rel = InRel::from_flat(rows.clone(), n, vec![b'i'; 2]);
        let cols = [StatColumn::Rel(&rel, 0), StatColumn::Rel(&rel, 1)];
        let (gathered, in_place) = super::column_stats(&cols, 16).expect("the device");
        assert_eq!(in_place, 0);
        let mut batch = crate::code_rule_dsl::transducer::packed::metal_join::Batch::new().expect("the device");
        let mut rep = super::Report::default();
        // Both columns uploaded: a copy holds only the columns asked for (decision 0277).
        super::resident(&rel, &[0, 1], &[], &mut batch, &mut rep).expect("resident");
        let (read, in_place) = super::column_stats(&cols, 16).expect("the device");
        assert_eq!(in_place, 2);
        assert_eq!(read, gathered);
        let values: Vec<i64> = rows.iter().step_by(2).copied().collect();
        let (of_values, _) = super::column_stats(&[StatColumn::Values(&values)], 16).expect("the device");
        assert_eq!(of_values[0], read[0]);
        assert_eq!(read[1].ndv, 18);
    }

    /// Decision 0221 §4, through the device interface: the device's batch makes the CPU's heavy values.
    #[test]
    fn the_gpu_batch_makes_the_cpu_batch_heavy_values() {
        use crate::code_rule_dsl::logic::plan::table;
        // A skewed key of node-like ids, and a narrow one.
        let mut x = 0x0209_u64;
        let mut next = || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
        let wide: Vec<i64> = (0..20_000).map(|_| { let r = next() % 1000; 3_288_784_809_065_000_000 + (if r < 500 { next() % 30 } else { next() % 3_000 }) as i64 }).collect();
        let narrow: Vec<i64> = (0..3_000).map(|_| (next() % 5) as i64).collect();
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        let gpu = table::heavy_of_values_gpu(&[&wide, &narrow]).expect("the registered device");
        for (col, g) in [&wide, &narrow].into_iter().zip(gpu) {
            let mut counts: rustc_hash::FxHashMap<i64, u64> = rustc_hash::FxHashMap::default();
            for &v in col { *counts.entry(v).or_default() += 1; }
            let cpu = table::HeavyCol::of_counts(counts);
            assert_eq!((g.rows, g.ndv, g.sq, g.cube, g.tail_rows, g.tail_ndv), (cpu.rows, cpu.ndv, cpu.sq, cpu.cube, cpu.tail_rows, cpu.tail_ndv));
            // The same top counts (ties may take other values of the same count).
            let counts_of = |h: &table::HeavyCol| { let mut c: Vec<u64> = h.top.iter().map(|t| t.1 as u64).collect(); c.sort_unstable(); c };
            assert_eq!(counts_of(&g), counts_of(&cpu));
            assert!((table::heavy_fan(&g, &[&g, &g]).unwrap() / table::heavy_fan(&cpu, &[&cpu, &cpu]).unwrap() - 1.0).abs() < 1e-6);
        }
    }
    /// The CPU's and the GPU's batch on columns of 1 M and 8 M rows, warm (a measure: `--ignored`).
    #[test]
    #[ignore]
    fn heavy_batches_timed() {
        use crate::code_rule_dsl::logic::plan::table;
        let mut x = 0x0209_u64;
        let mut next = || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
        for n in [1usize << 20, 8 << 20] {
            let col: Vec<i64> = (0..n).map(|_| { let r = next() % 1000; 3_288_784_809_065_000_000 + (if r < 500 { next() % 600 } else { next() % 200_000 }) as i64 }).collect();
            let _ = table::heavy_of_values_gpu(&[&col]);
            let t = std::time::Instant::now();
            let gpu = table::heavy_of_values_gpu(&[&col]).expect("a device");
            let g = t.elapsed().as_secs_f64();
            let t = std::time::Instant::now();
            let mut counts: rustc_hash::FxHashMap<i64, u64> = rustc_hash::FxHashMap::default();
            for &v in &col { *counts.entry(v).or_default() += 1; }
            let cpu = table::HeavyCol::of_counts(counts);
            let c = t.elapsed().as_secs_f64();
            assert_eq!((gpu[0].ndv, gpu[0].sq), (cpu.ndv, cpu.sq));
            eprintln!("heavy batch, {n} rows: GPU {:.1} ms, CPU {:.1} ms", g * 1e3, c * 1e3);
        }
    }

    /// Past the step that binds the head, the device keeps one witness per binding (decision 0277), as the CPU's walk
    /// does (decision 0229): the same rows, and no binding table of every row the rest of the body would make. The
    /// rest: an atom that binds, then semi-joins keyed by its columns, a constant and a slot bound before, and a
    /// negated atom.
    #[test]
    fn past_the_bound_head_the_device_keeps_one_witness() {
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        let store = crate::code_rule_dsl::fixtures::temp();
        drop(crate::code_rule_dsl::fixtures::open(&store));
        let db = Connection::open(&store).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
        let mut x = 23u64;
        let mut next = |m: i64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((x >> 33) as i64).rem_euclid(m)
        };
        // Documents `d` with many nodes `n` each (`a(d, n)`), a line per most nodes (`b(n, l)`), a column for some
        // (`c(n, k)`).
        db.execute_batch("CREATE TEMP TABLE dm (c0); CREATE TEMP TABLE a (c0, c1); CREATE TEMP TABLE b (c0, c1); CREATE TEMP TABLE c (c0, c1);").unwrap();
        for d in 0..200 {
            db.execute("INSERT INTO dm VALUES (?1)", [d]).unwrap();
            for k in 0..40 {
                let n = d * 1000 + k;
                db.execute("INSERT INTO a VALUES (?1, ?2)", [d, n]).unwrap();
                if next(4) != 0 {
                    db.execute("INSERT INTO b VALUES (?1, ?2)", [n, next(50)]).unwrap();
                }
                if next(3) == 0 {
                    db.execute("INSERT INTO c VALUES (?1, ?2)", [n, next(3)]).unwrap();
                }
            }
        }
        let src = |t: &str, cols: &str, types: &str| format!("t:x:{}:{}:{types}", hex(&format!("temp.{t}")), hex(cols));
        let (dm, a, b, c) = (src("dm", "c0", "i"), src("a", "c0, c1", "ii"), src("b", "c0, c1", "ii"), src("c", "c0, c1", "ii"));
        let rules = [
            // h0(d) :- dm(d), a(d, n), b(n, _), c(n, _).
            format!("pos {dm} v0\npos {a} v0 v1\npos {b} v1 v2\npos {c} v1 v3\nhead v0"),
            // h0(d) :- dm(d), a(d, n), c(n, 2): a constant key.
            format!("pos {dm} v0\npos {a} v0 v1\npos {c} v1 i:2\nhead v0"),
            // h0(d) :- dm(d), a(d, n), !b(n, _): a negated atom.
            format!("pos {dm} v0\npos {a} v0 v1\nneg {b} v1 _\nhead v0"),
        ];
        for (case, rule) in rules.iter().enumerate() {
            let mut got = Vec::new();
            for device in [false, true] {
                db.execute_batch("DROP TABLE IF EXISTS temp.h0; CREATE TEMP TABLE h0 (c0);").unwrap();
                let mut spec = vec!["name dev_wit".to_owned()];
                if device {
                    spec.push("device".into());
                }
                spec.push("rel h0 i empty".into());
                spec.push(format!("seed 0 {}", hex(rule)));
                super::super::open(&db, &spec.join("\n")).unwrap();
                super::super::run(&db, "dev_wit", 0).unwrap();
                let report = super::take_report();
                if device {
                    let r = report.as_ref().expect("a device report");
                    assert!(r.refused.is_none(), "{case}: {r:?}");
                    // The documents scanned and those with a witness: never a binding per node (8,000 nodes).
                    assert!(r.bindings <= 400, "{case}: {} bindings, a witness per document", r.bindings);
                }
                super::super::close(&db, "dev_wit").unwrap();
                let mut rows: Vec<i64> = db.prepare("SELECT c0 FROM temp.h0").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
                rows.sort_unstable();
                got.push(rows);
            }
            assert!(!got[0].is_empty(), "{case}: rows");
            assert_eq!(got[0], got[1], "{case}: the device's rows are the CPU's");
        }
    }

    /// Decision 0325 step 6: a view the device made in a batch a refusal dropped uncommitted is not kept as made. The
    /// next stratum makes it again and reads the host's rows, not a relation nothing wrote (the lost findings of a
    /// refused `attr_of`, 2026-11-17).
    #[test]
    fn a_view_made_in_a_refused_batch_is_made_again() {
        use crate::code_rule_dsl::facts::core;
        use crate::code_rule_dsl::facts::vtab::syntax::{Document, sources};
        use crate::code_rule_dsl::logic::native::setfix::InRel;
        use crate::code_rule_dsl::transducer::packed::metal_join as mj;
        use std::sync::Arc;
        if !mj::available() {
            return;
        }
        crate::test_languages();
        // Tapes held in vectors of their own, not where the device reads them (a parse not placed): the batch's views
        // are made, not read in place (decision 0353 step 4d).
        let lang = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let grammar = Arc::new(crate::code_rule_dsl::tape::Grammar::new(lang).unwrap());
        let docs: Vec<Document> = ["import os\nx = f(1, y=2)\n", "class C(B):\n    def m(self, a):\n        return a\n"].iter().enumerate().map(|(at, source)| {
            let path = format!("r{at}.py");
            let source = Arc::new((*source).to_owned());
            let image = crate::code_rule_dsl::tape::TapeImage::parse(&grammar, &source, &|| false).unwrap().unwrap().primary;
            let lines = Arc::new(crate::code_rule_dsl::lines::LineTable::new(&source, crate::code_rule_dsl::lines::Terminators::of(lang)));
            let facts = Arc::new(core::from_image(source, &grammar, image, lines).unwrap());
            Document { key: (5 + 2 * at) as u32, core: format!("/w#core:{path}"), digest: core::digest(&facts), path, facts, tape: None, guest: None, module: None }
        }).collect();
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let store_path = std::env::temp_dir().join(format!("rerail-refused-view-{}-{nonce}.sqlite", std::process::id()));
        let store = crate::content_store::ContentStore::open(&store_path).unwrap();
        let all = sources(&store, docs).unwrap();
        let src = Arc::clone(&all.iter().find(|(n, s)| *n == "node" && s.python_batch().is_some()).expect("node").1);
        assert!(src.device_view().is_some_and(|d| d.in_place.is_none()), "made on the device");
        let rel = InRel::over_source(Arc::clone(&src), vec![b'i'; src.columns().len()]);
        // Encoded into a batch the refusal drops uncommitted: nothing ran it.
        let mut refused = mj::Batch::new().expect("the device");
        let mut rep = super::Report::default();
        super::resident(&rel, &[0, 1], &[], &mut refused, &mut rep).expect("resident");
        assert_eq!(rep.uploads, 1, "made: {rep:?}");
        drop(refused);
        let _ = super::unrun::<()>(Err("refused at run time".to_owned()));
        let mut batch = mj::Batch::new().expect("the device");
        let mut rep = super::Report::default();
        let (dev, _, place) = super::resident(&rel, &[0, 1], &[], &mut batch, &mut rep).expect("resident");
        assert_eq!((rep.uploads, rep.resident_hits), (1, 0), "made again, not found: {rep:?}");
        batch.finish().unwrap();
        let n = src.len();
        for c in [0, 1] {
            let host: Vec<i64> = (0..n).map(|r| src.value(r, c)).collect();
            assert_eq!(dev.column(place[c] as usize), host, "column {c}");
        }
        let _ = std::fs::remove_file(&store_path);
    }

    /// Decision 0289 §2: a view made on the device from the batch's chunk block is the host's rows (`Source::value`),
    /// for each view the device makes, over documents with fields, nesting and a broken one.
    #[test]
    fn views_made_on_the_device_are_the_hosts_rows() {
        use crate::code_rule_dsl::facts::core::{self, Built};
        use crate::code_rule_dsl::facts::vtab::syntax::{Document, sources};
        use crate::code_rule_dsl::model::FileInput;
        use crate::code_rule_dsl::transducer::packed::metal_join as mj;
        use std::sync::Arc;
        if !mj::available() {
            return;
        }
        crate::test_languages();
        let texts = ["import os\nx = f(1, y=2)\n", "class C(B):\n    def m(self, a, *b):\n        return [a for a in b if a]\n", "def f(a, b:\n    return [a for\n", "", "@d\ndef g():\n    try:\n        pass\n    except E as e:\n        raise\n"];
        let make = || -> Vec<Document> { texts.iter().enumerate().map(|(at, source)| {
            let path = format!("d{at}.py");
            let file = FileInput::new(path.clone(), "python".to_owned(), Arc::new((*source).to_owned()), None, None, "core").unwrap();
            let facts = match core::build(&file).unwrap() { Built::Facts(f) => Arc::new(f), Built::Unparsed => panic!("{path}") };
            Document { key: (7 + 3 * at) as u32, core: format!("/w#core:{path}"), digest: core::digest(&facts), path, facts, tape: None, guest: None, module: None }
        }).collect() };
        let docs = make();
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let store_path = std::env::temp_dir().join(format!("rerail-chunk-views-{}-{nonce}.sqlite", std::process::id()));
        let store = crate::content_store::ContentStore::open(&store_path).unwrap();
        let all = sources(&store, docs).unwrap();
        let device = rerail_accel::gpu::join::device().expect("the device");
        let (mut seen, mut in_place) = (0, 0);
        // `line` (`View::Line`, decision 0325 step 3): the view, not a relation of that name.
        for name in ["node", "child", "field", "core_node", "line"] {
            let src = &all.iter().find(|(n, s)| *n == name && s.python_batch().is_some()).unwrap_or_else(|| panic!("{name}")).1;
            let d = src.device_view().unwrap_or_else(|| panic!("{name}: made on the device"));
            let (n, k) = (src.len(), src.columns().len());
            assert_eq!(d.rows, n, "{name}");
            assert!(n > 0, "{name}: rows");
            // Every column, and a subset out of order, as a stratum asks for.
            for cols in [(0..k).collect::<Vec<_>>(), vec![k - 1, 0]] {
                seen += 1;
                // Made (a batch whose tapes are not on the device: `a_view_made_in_a_refused_batch_is_made_again`).
                if d.in_place.is_none() {
                    let out = mj::DevRel::empty(cols.len(), n).unwrap();
                    let mut batch = mj::Batch::new().expect("the device");
                    batch.expand_view(&d.block.gpu_buffer(device), d.root, d.view, d.pairs, &cols, &out).unwrap();
                    batch.finish().unwrap();
                    for (j, &c) in cols.iter().enumerate() {
                        let host: Vec<i64> = (0..n).map(|r| src.value(r, c)).collect();
                        assert_eq!(out.column(j), &host[..], "{name}: column {c}");
                    }
                }
                // Read in place (decision 0353 step 4d): the views' reads over the batch's directory, copied out of
                // the view by `rows_copy`, are the same rows; nothing made them.
                if let Some(v) = d.in_place.clone() {
                    let view = mj::DevRel::view(k, n, v.clone()).unwrap();
                    let out = mj::DevRel::empty(cols.len(), n).unwrap();
                    let mut batch = mj::Batch::new().expect("the device");
                    let from: Vec<mj::Col> = cols.iter().map(|&c| mj::Col::From(c)).collect();
                    batch.copy_rows(mj::Rows::Rel(&view), n, &from, mj::Rows::Rel(&out), 0).unwrap();
                    batch.finish().unwrap();
                    // A 16z batch's blocks through its cache (0353 step 4b): every read found room.
                    assert_eq!(v.cache_failed(), 0, "{name}: the 16z cache had room: {:?}", v.cache_counts());
                    for (j, &c) in cols.iter().enumerate() {
                        let host: Vec<i64> = (0..n).map(|r| src.value(r, c)).collect();
                        assert_eq!(out.column(j), &host[..], "{name}: column {c} read in place");
                    }
                    in_place += 1;
                }
            }
        }
        assert_eq!(seen, 10);
        assert_eq!(in_place, 10, "node, child, field, core_node and line read in place");
        // core.rerail's places (`View::Loc`), over nodes and the spans past them (tokens, the text, lines, end points).
        {
            use crate::code_rule_dsl::facts::vtab::syntax::{Guest, Made, Placement, Projection, shaped};
            // With a guest (decision 0055): its places are its host's lines, through its placement, found on the
            // host and put in the chunk (`ChunkDoc::placed`).
            let mut docs = make();
            let text = "y = [2,\n  3]\n";
            let file = FileInput::new("d0.py".to_owned(), "python".to_owned(), Arc::new(text.to_owned()), None, None, "core").unwrap();
            let facts = match core::build(&file).unwrap() { Built::Facts(f) => Arc::new(f), Built::Unparsed => panic!("the guest") };
            // At the host's second line (byte 10 of "import os\nx = f(1, y=2)\n"): the guest's two lines are the
            // host's line 2.
            let place = Placement::Copied { delta: 10, within: [10, 24] };
            let guest = Guest { host: 7, at: 7 << 32, grammar: "python".to_owned(), role: "program".to_owned(), root: 0, place: place.clone(), in_file: place, made: Made::Host };
            docs.push(Document { key: 100, core: "/w#core:d0.py#guest:python:0".to_owned(), digest: core::digest(&facts), path: "d0.py".to_owned(), facts, tape: None, guest: Some(Arc::new(guest)), module: None });
            let core_views = shaped(&store, docs, &Projection::core()).unwrap();
            {
                let view = |name: &str| core_views.iter().find(|(n, _)| *n == name).unwrap().1.clone();
                let (docof, line) = (view("docof"), view("line"));
                let guest: Vec<usize> = (0..docof.len()).filter(|&r| docof.value(r, 1) == 100).collect();
                assert!(!guest.is_empty(), "the guest's places are rows");
                assert!(guest.iter().all(|&r| line.value(r, 1) == 2), "the guest's places are its host's lines");
            }
            // Each document's places read in place (decision 0353 step 4d): the views' reads over the batch's
            // directory, its tapes and core.rerail's lines in the batch's block; a guest's (a copied stretch) through its
            // host's lines and its placement. With the guest and without.
            let own_views = shaped(&store, make(), &Projection::core()).unwrap();
            for views in [&core_views, &own_views] {
                for name in ["line", "end_line", "col", "docof"] {
                    let src = &views.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("{name}")).1;
                    let v = src.device_view().and_then(|d| d.in_place.clone()).unwrap_or_else(|| panic!("{name}: read in place"));
                    let n = src.len();
                    assert!(n > 0, "{name}: rows");
                    for cols in [vec![0, 1], vec![1]] {
                        let view = mj::DevRel::view(2, n, v.clone()).unwrap();
                        let out = mj::DevRel::empty(cols.len(), n).unwrap();
                        let mut batch = mj::Batch::new().expect("the device");
                        let from: Vec<mj::Col> = cols.iter().map(|&c| mj::Col::From(c)).collect();
                        batch.copy_rows(mj::Rows::Rel(&view), n, &from, mj::Rows::Rel(&out), 0).unwrap();
                        batch.finish().unwrap();
                        for (j, &c) in cols.iter().enumerate() {
                            let host: Vec<i64> = (0..n).map(|r| src.value(r, c)).collect();
                            assert_eq!(out.column(j), &host[..], "{name}: column {c} read in place");
                        }
                    }
                }
            }
        }
        // The kind-set axes and containment, read whole: the batch's solve, its pairs made ids on the device.
        use crate::code_rule_dsl::facts::vtab::syntax::within::{Axis, Mode, Within};
        use crate::code_rule_dsl::facts::nearest::Binding;
        use crate::code_rule_dsl::facts::vtab::Source as _;
        // The engine registers the batch's nearest solve as it loads.
        crate::code_rule_dsl::facts::nearest::install_nearest(crate::code_rule_dsl::typing::file_local::nearest);
        let node = &all.iter().find(|(n, _)| *n == "node").unwrap().1;
        let batch = node.python_batch().expect("the batch");
        let mut kinds: Vec<i64> = (0..node.len()).map(|r| node.value(r, 2)).collect();
        kinds.sort_unstable();
        kinds.dedup();
        let kinds: Vec<i64> = kinds.into_iter().step_by(3).collect();
        let axes = [
            ("nearest", Within::axis(Arc::clone(&batch), Axis::Nearest, &kinds, Binding::Cpu)),
            ("inside", Within::axis(Arc::clone(&batch), Axis::Inside, &kinds, Binding::Cpu)),
            ("within", Within::new(Arc::clone(&batch), Mode::Tree)),
        ];
        for (name, src) in &axes {
            let n = src.len();
            assert!(n > 0, "{name}: rows");
            let d = src.device_view().unwrap_or_else(|| panic!("{name}: made on the device"));
            assert_eq!(d.rows, n, "{name}");
            for cols in [vec![0, 1], vec![1]] {
                let out = mj::DevRel::empty(cols.len(), n).unwrap();
                let mut batch = mj::Batch::new().expect("the device");
                batch.expand_view(&d.block.gpu_buffer(device), d.root, d.view, d.pairs, &cols, &out).unwrap();
                batch.finish().unwrap();
                for (j, &c) in cols.iter().enumerate() {
                    let host: Vec<i64> = (0..n).map(|r| src.value(r, c)).collect();
                    assert_eq!(out.column(j), &host[..], "{name}: column {c}");
                }
            }
        }
        // On the device's binding the solve itself is made on the device in the chunk: the host's rows, in its order,
        // and its runs per looked-up node.
        for axis in [Axis::Nearest, Axis::Inside] {
            let host = Within::axis(Arc::clone(&batch), axis, &kinds, Binding::Cpu);
            let dev = Within::axis(Arc::clone(&batch), axis, &kinds, Binding::Metal);
            let n = host.len();
            assert_eq!(dev.len(), n);
            for c in 0..2 {
                let (h, d): (Vec<i64>, Vec<i64>) = ((0..n).map(|r| host.value(r, c)).collect(), (0..n).map(|r| dev.value(r, c)).collect());
                assert_eq!(d, h, "column {c}");
            }
            for r in (0..node.len()).step_by(7) {
                let id = node.value(r, 1);
                assert_eq!(dev.lookup(0, &[id]), host.lookup(0, &[id]), "the rows of {id}");
            }
            let d = dev.device_view().expect("made on the device");
            let out = mj::DevRel::empty(2, n).unwrap();
            let mut batch = mj::Batch::new().expect("the device");
            batch.expand_view(&d.block.gpu_buffer(device), d.root, d.view, d.pairs, &[0, 1], &out).unwrap();
            batch.finish().unwrap();
            assert_eq!(out.column(1), &(0..n).map(|r| host.value(r, 1)).collect::<Vec<_>>()[..]);
        }
        // Containment over every kind, in one submission (space sized before the count) and past a bound of no rows
        // (written again once the total is read): the same pairs, as many as the host's containment.
        let chunk = batch.chunk();
        let read = |c: &crate::code_rule_dsl::facts::nearest::InChunk| -> Vec<u64> {
            // SAFETY: the solve completed; `len` pairs at `pairs`.
            unsafe { std::slice::from_raw_parts(chunk.block.ptr::<u64>(c.pairs), c.len) }.to_vec()
        };
        let once = mj::axis_in_chunk_bounded(chunk, None, true, 8).unwrap();
        let again = mj::axis_in_chunk_bounded(chunk, None, true, 0).unwrap();
        assert!(once.len > 0);
        assert_eq!(once.len, Within::new(Arc::clone(&batch), Mode::Tree).len());
        assert_eq!(read(&again), read(&once));
        // `has` is made per looked-up node, not by the batch's solve: uploaded as before.
        assert!(Within::axis(Arc::clone(&batch), Axis::Has, &kinds, Binding::Cpu).device_view().is_none());
        let _ = std::fs::remove_file(&store_path);
    }
