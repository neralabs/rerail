    use super::super::thread_cpu;
    use rusqlite::Connection;
    use std::path::Path;

    /// The distinct rows of `schema.table`, each as text, sorted.
    fn rows(db: &Connection, schema: &str, table: &str) -> Vec<String> {
        let n: i64 = db.query_row(&format!("SELECT count(*) FROM {schema}.pragma_table_info('{table}')"), [], |r| r.get(0)).unwrap();
        let n = n as usize;
        let mut st = db.prepare(&format!("SELECT * FROM {schema}.{table}")).unwrap();
        let mut out: Vec<String> = st
            .query_map([], |r| (0..n).map(|c| r.get_ref(c).map(|v| format!("{v:?}"))).collect::<rusqlite::Result<Vec<_>>>().map(|v| v.join("\t")))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        out.sort();
        out.dedup();
        out
    }

    /// Gives `db`'s new store the run's atoms, at the run's ids: the texts of `atoms` interned in id order.
    fn same_atoms(db: &Connection, atoms: &Path) {
        let arena = crate::content_store::atoms::Atoms::for_connection(db);
        let src = Connection::open(atoms).unwrap();
        let mut st = src.prepare("SELECT id, text FROM atoms ORDER BY id").unwrap();
        let texts: Vec<(i64, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        let have = arena.len();
        for (id, text) in &texts {
            let id = *id as usize;
            if id < have {
                let mine = arena.with_str(id as crate::content_store::atoms::Sym, str::to_owned).unwrap();
                assert_eq!(&mine, text, "the new store's atom {id} is not the run's");
            }
        }
        let rest: Vec<&str> = texts.iter().filter(|(id, _)| *id as usize >= have).map(|(_, t)| t.as_str()).collect();
        for (chunk, start) in rest.chunks(4096).zip((have..).step_by(4096)) {
            let ids = arena.intern_all(chunk).unwrap();
            for (k, id) in ids.iter().enumerate() {
                assert_eq!(*id as usize, start + k, "atom {:?} took id {id}, the run's is {}", chunk[k], start + k);
            }
        }
    }

    /// Each capture of `RERAIL_RELATIONAL_LAB_REPLAY` run again on a store of its own: its rows against the run's,
    /// and its CPU.
    #[test]
    #[ignore = "reads captures a run made with RERAIL_RELATIONAL_LAB"]
    fn relational_lab() {
        let dir = std::env::var("RERAIL_RELATIONAL_LAB_REPLAY").expect("RERAIL_RELATIONAL_LAB_REPLAY");
        let mut captures: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "sqlite") && p.file_name().is_some_and(|n| n != "atoms.sqlite")).collect();
        captures.sort();
        let (mut same, mut all) = (0, 0);
        #[cfg(target_os = "macos")]
        let mut device = Device::default();
        println!("{:>9} {:>9} {:>10}  {:<5} capture", "cpu ms", "solve ms", "rows", "rows");
        for path in captures {
            let store = crate::code_rule_dsl::fixtures::temp();
            drop(crate::content_store::ContentStore::open(&store).unwrap());
            let db = Connection::open(&store).unwrap();
            crate::code_rule_dsl::logic::native::register(&db).unwrap();
            crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
            same_atoms(&db, &Path::new(&dir).join("atoms.sqlite"));
            db.execute("ATTACH ?1 AS cap", [path.to_string_lossy()]).unwrap();
            let get = |k: &str| -> String { db.query_row("SELECT value FROM cap.meta WHERE key = ?1", [k], |r| r.get(0)).unwrap() };
            let (name, spec) = (get("name"), get("spec"));
            let tables: Vec<String> = {
                let mut st = db.prepare("SELECT name FROM cap.sqlite_master WHERE type = 'table' AND name != 'meta'").unwrap();
                st.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
            };
            // Each table into the store, its texts interned as this store's atoms.
            let load = |db: &Connection, only_rels: bool| {
                for t in &tables {
                    if only_rels && !(t.starts_with('r') && t[1..].chars().all(|c| c.is_ascii_digit())) {
                        continue;
                    }
                    let info: Vec<(String, String)> = {
                        let mut st = db.prepare(&format!("SELECT name, type FROM cap.pragma_table_info('{t}')")).unwrap();
                        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
                    };
                    let cols: Vec<String> = info.iter().map(|(c, _)| c.clone()).collect();
                    // In `temp`: a write there commits nothing of the store, whose new atoms are pending (0049).
                    db.execute_batch(&format!("DROP TABLE IF EXISTS temp.{t}; CREATE TEMP TABLE {t} ({}); INSERT INTO temp.{t} SELECT * FROM cap.{t};", cols.join(", "))).unwrap();
                }
            };
            load(&db, false);
            let outs: Vec<&String> = tables.iter().filter(|t| t.starts_with("out")).collect();
            // The same stratum on the device (decision 0095, step 4), first: its rows against the run's, row for row,
            // or the reason the device refuses it. Then the relations as they were, for the CPU's run.
            #[cfg(target_os = "macos")]
            {
                let dspec = spec.replacen('\n', "\ndevice\n", 1);
                super::super::open(&db, &dspec).unwrap();
                let c0 = process_cpu();
                let result = super::super::run(&db, &name, 0).unwrap();
                let cpu = (process_cpu() - c0) * 1e3;
                super::super::close(&db, &name).unwrap();
                let report = super::super::device_report();
                let mut ok = true;
                for out in &outs {
                    let k = &out[3..];
                    let (got, want) = (rows(&db, "temp", &format!("r{k}")), rows(&db, "cap", out));
                    if got != want {
                        ok = false;
                        println!("    device r{k}: {} rows, the run had {}", got.len(), want.len());
                    }
                }
                let _ = result;
                match report {
                    Some(r) if r.refused.is_none() => {
                        device.strata += 1;
                        device.strata_same += usize::from(ok);
                        device.strata_cpu += cpu;
                        device.strata_gpu += r.gpu_seconds * 1e3;
                        device.strata_submissions += r.submissions as usize;
                        println!("    device: {} ({cpu:.1} ms process CPU, GPU {:.2} ms, {} submissions, {} rules, {} stages, uploads {}, resident {})",
                            if ok { "same" } else { "DIFF" }, r.gpu_seconds * 1e3, r.submissions, r.rules, r.stages, r.uploads, r.resident_hits);
                    }
                    Some(r) => {
                        device.strata_refused += 1;
                        println!("    device refused: {}", r.refused.unwrap_or_default());
                    }
                    None => println!("    device: no report"),
                }
                load(&db, true);
            }
            super::super::open(&db, &spec).unwrap();
            // `RERAIL_RELATIONAL_LAB_HOST_ONLY`: the captures alone, no device work (the run's own CPU and memory).
            #[cfg(target_os = "macos")]
            if std::env::var_os("RERAIL_RELATIONAL_LAB_HOST_ONLY").is_none() {
                device.add(&super::super::scans(&db, &name).unwrap());
                device.tiles(&super::super::tiles(&db, &name).unwrap());
            }
            let c0 = thread_cpu();
            #[cfg(target_os = "macos")]
            let p0 = process_cpu();
            let result = super::super::run(&db, &name, 0).unwrap();
            let cpu = (thread_cpu() - c0) * 1e3;
            #[cfg(target_os = "macos")]
            {
                device.strata_cpu_host += (process_cpu() - p0) * 1e3;
            }
            super::super::close(&db, &name).unwrap();
            let solve: f64 = result.split_whitespace().nth(5).and_then(|s| s.parse().ok()).unwrap_or(f64::NAN);
            let mut ok = true;
            let mut n = 0;
            for out in &outs {
                let k = &out[3..];
                let (got, want) = (rows(&db, "temp", &format!("r{k}")), rows(&db, "cap", out));
                n += want.len();
                if got != want {
                    ok = false;
                    println!("    r{k}: {} rows, the run had {}", got.len(), want.len());
                }
            }
            all += 1;
            same += usize::from(ok);
            println!("{cpu:>9.1} {solve:>9.1} {n:>10}  {:<5} {}", if ok { "same" } else { "DIFF" }, path.file_name().unwrap().to_string_lossy());
            drop(db);
            let _ = std::fs::remove_file(&store);
        }
        println!("{same} of {all} captures give the run's rows");
        #[cfg(target_os = "macos")]
        device.report();
        assert_eq!(same, all);
    }

    /// The process's CPU seconds, user and system (every thread's: Metal's completion threads too).
    #[cfg(target_os = "macos")]
    fn process_cpu() -> f64 {
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: a valid rusage out.
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
        let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
        t(u.ru_utime) + t(u.ru_stime)
    }

    /// The captures' fused filters over input relations (`setfix::scans`), each run on Metal over its rows in the
    /// 16-bit layout (`metal_rows`) and checked against the CPU's rows.
    #[cfg(target_os = "macos")]
    #[derive(Default)]
    struct Device { strata: usize, strata_same: usize, strata_refused: usize, strata_cpu: f64, strata_cpu_host: f64, strata_gpu: f64, strata_submissions: usize, probe: Probe, sort_cpu: f64, sort_same: usize, joins: usize, joins_same: usize, pairs: usize, tile_pairs: usize, compared: u64, join_cpu: f64, join_gpu: f64, tile_host: f64, cases: usize, same: usize, unsupported: usize, rows: usize, passed: usize, cpu: f64, gpu: f64, bytes: usize, wide: usize, forms: [usize; 3] }

    /// The device's sort and probe joins: joins run and right, sorts and the joins that found theirs resident, GPU and
    /// host CPU (the uploads, the sorts' and probes' encoding and the pairs' read), and the keys' bytes.
    #[cfg(target_os = "macos")]
    #[derive(Default)]
    struct Probe { upload: f64, submissions: usize, joins: usize, same: usize, off: usize, sorts: usize, resident: usize, sort_gpu: f64, probe_gpu: f64, host: f64, right_host: f64, bytes: usize, wide: usize }

    #[cfg(target_os = "macos")]
    impl Device {
        fn add(&mut self, cases: &[super::super::ScanCase]) {
            use crate::code_rule_dsl::logic::native::setfix::rowcode::Word;
            use crate::code_rule_dsl::transducer::packed::metal_rows::{Form, Input, Resident, scan};
            for case in cases {
                let n = case.gathered.first().map_or(0, |g| g.1.len());
                let at = |c: usize| case.gathered.iter().position(|g| g.0 == c).expect("gathered");
                let inputs: Vec<Input> = case.inputs.iter().map(|w| match *w { Word::Col(c) => Input::Col(at(c)), Word::Doc(c) => Input::Doc(at(c)) }).collect();
                let cols: Vec<&[i64]> = case.gathered.iter().map(|g| g.1.as_slice()).collect();
                let resident = Resident::new(&cols, n).expect("resident");
                let got = match scan(&case.program, &inputs, &resident, &case.params) {
                    Ok(s) => s,
                    Err(_) => { self.unsupported += 1; continue; }
                };
                let mut rows = got.rows.clone();
                rows.sort_unstable();
                self.cases += 1;
                if rows == case.passed { self.same += 1 } else { println!("    device DIFF {}: {} rows, the CPU {}", case.rule, rows.len(), case.passed.len()) }
                self.rows += n;
                self.passed += case.passed.len();
                self.cpu += case.cpu_seconds;
                self.gpu += got.gpu_seconds;
                self.bytes += resident.bytes;
                self.wide += 8 * n * cols.len();
                for f in &resident.forms { self.forms[match f { Form::Low16 => 0, Form::I32 => 1, Form::I64 => 2 }] += 1; }
            }
        }

        fn tiles(&mut self, cases: &[super::super::TileCase]) {
            use crate::code_rule_dsl::transducer::packed::metal_rows::{Form, Joined, Resident, tile_join};
            use crate::code_rule_dsl::transducer::packed::metal_sort::{Column, Keyed, probe};
            use crate::code_rule_dsl::transducer::packed::rows_sort;
            let same = |got: &[(u32, u32)], case: &super::super::TileCase, how: &str| -> bool {
                let mut pairs = got.to_vec();
                pairs.sort_unstable();
                let mut want = case.pairs.clone();
                want.sort_unstable();
                if pairs != want { println!("    join DIFF {how} {}: {} pairs, the CPU {}", case.rule, pairs.len(), want.len()) }
                pairs == want
            };
            // The capture's right sides sorted on the device, each once: the joins after its first find it resident.
            let mut resident: std::collections::HashMap<&str, Keyed> = std::collections::HashMap::new();
            let mut host_sorted: std::collections::HashMap<&str, rows_sort::Keyed> = std::collections::HashMap::new();
            for case in cases {
                let (nl, nr) = (case.left[0].len(), case.right[0].len());
                let k = case.left.len();
                self.pairs += case.pairs.len();
                self.join_cpu += case.cpu_seconds;
                let l: Vec<&[i64]> = case.left.iter().map(Vec::as_slice).collect();
                let r: Vec<&[i64]> = case.right.iter().map(Vec::as_slice).collect();
                // The CPU's sort join: the right radix-sorted by key 0 on the host once a capture, the left per join,
                // their runs merged.
                let c0 = process_cpu();
                let keyed = host_sorted.entry(case.right_key.as_str()).or_insert_with(|| rows_sort::Keyed::new(r[0]));
                let sorted = rows_sort::join_keyed(&l, keyed, &r);
                self.sort_cpu += process_cpu() - c0;
                self.sort_same += usize::from(same(&sorted, case, "sort join"));
                // The device: the right side sorted by key 0 (once a capture), the left uploaded and probing it. Its
                // host CPU is the process's (Metal's completion threads too).
                let h0 = process_cpu();
                let keyed = match resident.entry(case.right_key.as_str()) {
                    std::collections::hash_map::Entry::Occupied(e) => {
                        self.probe.resident += 1;
                        Ok(&*e.into_mut())
                    }
                    std::collections::hash_map::Entry::Vacant(e) => {
                        let u0 = process_cpu();
                        let columns = r.iter().map(|c| Column::upload(c)).collect::<Result<Vec<_>, _>>();
                        self.probe.upload += process_cpu() - u0;
                        let sorted = columns.and_then(Keyed::sort);
                        self.probe.right_host += process_cpu() - h0;
                        sorted.map(|s| {
                            self.probe.sorts += 1;
                            self.probe.bytes += s.bytes;
                            &*e.insert(s)
                        })
                    }
                };
                let got = keyed.and_then(|keyed| {
                    let u0 = process_cpu();
                    let left = l.iter().map(|c| Column::upload(c)).collect::<Result<Vec<_>, _>>()?;
                    self.probe.upload += process_cpu() - u0;
                    let bytes: usize = left.iter().map(Column::bytes).sum();
                    probe(&left, keyed, case.left[0].len() * 2).map(|p| (p, bytes))
                });
                self.probe.host += process_cpu() - h0;
                match got {
                    Ok((p, bytes)) => {
                        self.probe.joins += 1;
                        self.probe.same += usize::from(same(&p.pairs, case, "by sort and probe"));
                        self.probe.probe_gpu += p.gpu_seconds;
                        self.probe.submissions += p.submissions as usize;
                        self.probe.bytes += bytes;
                        self.probe.wide += 8 * k * (nl + nr);
                    }
                    Err(_) => self.probe.off += 1,
                }
                // By chunk, when key 0 is a node column of one chunk on both sides: the first key that is.
                let t0 = process_cpu();
                let order = (0..k).find_map(|k0| {
                    let mut keys: Vec<usize> = vec![k0];
                    keys.extend((0..k).filter(|&x| x != k0));
                    let l: Vec<&[i64]> = keys.iter().map(|&x| case.left[x].as_slice()).collect();
                    let r: Vec<&[i64]> = keys.iter().map(|&x| case.right[x].as_slice()).collect();
                    // Node ids: `d << 32 | i`, so a key 0 under 2^32 is not one (a number of a chunk of its own).
                    let nodes = case.left[k0].iter().chain(&case.right[k0]).all(|&v| v >> 32 > 0);
                    if !nodes { return None; }
                    let (l, r) = (Resident::new(&l, nl).ok()?, Resident::new(&r, nr).ok()?);
                    (l.forms[0] == Form::Low16 && r.forms[0] == Form::Low16).then_some((l, r))
                });
                let Some((lr, rr)) = order else { continue };
                let keys: Vec<usize> = (0..k).collect();
                let got: Result<Joined, _> = tile_join(&lr, &keys, &rr, &keys);
                self.tile_host += process_cpu() - t0;
                let Ok(got) = got else { continue };
                self.joins += 1;
                self.joins_same += usize::from(same(&got.pairs, case, "by chunk"));
                self.compared += got.compared;
                self.tile_pairs += got.pairs.len();
                self.join_gpu += got.gpu_seconds;
            }
            // The sorts ran ahead of their probes, unwaited: their GPU time once all completed.
            self.probe.sort_gpu += resident.values().map(Keyed::gpu_seconds).sum::<f64>();
        }

        fn report(&self) {
            println!(
                "device strata: {} of {} give the run's rows, {} refused; process CPU {:.1} ms on the device ({:.1} ms GPU, {} submissions) against {:.1} ms on the CPU (every capture)",
                self.strata_same, self.strata, self.strata_refused, self.strata_cpu, self.strata_gpu, self.strata_submissions, self.strata_cpu_host
            );
            assert_eq!(self.strata_same, self.strata, "every stratum the device runs gives the run's rows");
            let p = &self.probe;
            println!(
                "joins: {} pairs; CPU hash join {:.1} ms; CPU sort join, each right side sorted once a capture, {:.1} ms ({} of {} give the hash join's pairs)",
                self.pairs, self.join_cpu * 1e3, self.sort_cpu * 1e3, self.sort_same, p.joins + p.off
            );
            println!(
                "device sort and probe: {} of {} give the CPU's pairs ({} not on the device); {} sorts, {} joins found theirs resident, {} submissions; GPU sort {:.1} ms, probe {:.1} ms; host CPU {:.1} ms ({:.1} ms of it the right sides' upload and sort, {:.1} ms every upload); on the device {} kB, the keys {} kB as i64",
                p.same, p.joins, p.off, p.sorts, p.resident, p.submissions + p.sorts, p.sort_gpu * 1e3, p.probe_gpu * 1e3, p.host * 1e3, p.right_host * 1e3, p.upload * 1e3, p.bytes >> 10, p.wide >> 10
            );
            println!(
                "device tile by chunk: {} of {} give the CPU's pairs ({} pairs), {} compared, GPU {:.1} ms, host CPU {:.1} ms",
                self.joins_same, self.joins, self.tile_pairs, self.compared, self.join_gpu * 1e3, self.tile_host * 1e3
            );
            assert_eq!(self.sort_same, p.joins + p.off, "every CPU sort join gives the hash join's pairs");
            assert_eq!(self.joins_same, self.joins, "every chunk tile join gives the CPU's pairs");
            assert_eq!(p.same, p.joins, "every sort and probe join gives the CPU's pairs");
            println!(
                "device scans: {} of {} give the CPU's rows ({} not on the device); {} rows in, {} passed; CPU {:.1} ms, GPU {:.1} ms; columns {} kB as 16-bit layout, {} kB as i64 (low16 {}, i32 {}, i64 {})",
                self.same, self.cases, self.unsupported, self.rows, self.passed, self.cpu * 1e3, self.gpu * 1e3, self.bytes >> 10, self.wide >> 10, self.forms[0], self.forms[1], self.forms[2]
            );
            assert_eq!(self.same, self.cases, "every device scan gives the CPU's rows");
        }
    }
