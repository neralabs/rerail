    use super::*;
    use crate::code_rule_dsl::logic::{native, plan, rr};
    use crate::code_rule_dsl::logic::plan::lower;

    #[test]
    fn cross_process_admission_batches_inputs_and_eligible_derived_frames() {
        use crate::code_rule_dsl::logic::profile;
        let _run = profile::one_run();
        profile::set(true);
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
        let mut text = String::new();
        let mut inputs = Vec::new();
        for i in 0..257 {
            text += &format!(".decl input{i}(x:number)\n.input input{i}\n.decl result{i}(x:number)\nresult{i}(x) :- input{i}(x).\n.output result{i}\n");
            inputs.push((format!("input{i}"), native::component::columns_source(vec!["c0".into()], vec![Some(vec![i])])));
        }
        let prog = rr::parse(&text).unwrap();
        let now: HashMap<_, _> = inputs.iter().map(|(r, s)| (r.clone(), super::super::retain::digest(s.as_ref()))).collect();
        let (_, part) = load(&store, dir.path(), [7;32], &prog, &inputs, &now).unwrap();
        let puts: Vec<_> = part.frames.iter().map(|(name, (rel, _))| {
            let i: usize = name.trim_start_matches("input").trim_start_matches("result").parse().unwrap();
            FramePut { rel: rel.clone(), part: 0, digest: now[&format!("input{i}")],
                reads: if name.starts_with("result") {vec![(format!("input{i}"), now[&format!("input{i}")])]} else {vec![]},
                rows: FrameRows { types: vec![b'i'], rows: vec![i as i64], n: 1 } }
        }).collect();
        store.keep_frames(&part.space, &puts, &[], None).unwrap();
        // Metadata can name a derived frame whose inputs do not match: its payload must not be admitted.
        let stale = &part.frames["result256"].0;
        store.keep_frames(&part.space, &[FramePut { rel: stale.clone(), part: 0, digest: 99,
            reads: vec![("input256".into(), now["input256"].wrapping_add(1))],
            rows: FrameRows { types: vec![b'i'], rows: vec![999], n: 1 } }], &[], None).unwrap();
        profile::reset();
        profile::set(true);
        let wave = profile::waves();
        let span = profile::span(|| "bulk retained admission".into());
        let (kept, _) = load(&store, dir.path(), [7;32], &prog, &inputs, &now).unwrap();
        let kept = kept.unwrap();
        let (read_inputs, rels) = kept.parts();
        assert_eq!(read_inputs.len(), 257);
        assert_eq!(rels.len(), 256);
        assert!(rels.iter().all(|(r, _, _)| *r != "result256"));
        for (name, _, held) in rels {
            let i: i64 = name.trim_start_matches("result").parse().unwrap();
            assert_eq!(held.live_rows().unwrap(), [i]);
        }
        drop(span);
        drop(wave);
        let waits = profile::sql_waits();
        profile::set(false);
        let statements: u64 = waits.iter().filter(|(p, sql, _, _)| p.starts_with("bulk retained admission") && sql.starts_with("store:")).map(|x| x.2).sum();
        eprintln!("257 input frames and 256 eligible derived frames: {statements} storage SQL executions");
        assert!(statements > 0, "the gate must observe the real store connection");
        assert!(statements <= 16, "frame reads must be batches, not one query per relation: {statements}");
    }

    #[test]
    fn cross_process_admission_rejects_replaced_or_deleted_snapshot_frames() {
        for changed in ["input", "result"] {
            for deleted in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
                let prog = rr::parse(".decl input(x:number)\n.input input\n.decl result(x:number)\nresult(x) :- input(x).\n.output result\n").unwrap();
                let inputs = vec![("input".to_owned(), native::component::columns_source(vec!["c0".into()], vec![Some(vec![42])]))];
                let now = HashMap::from([("input".to_owned(), super::super::retain::digest(inputs[0].1.as_ref()))]);
                let (_, part) = load(&store, dir.path(), [7; 32], &prog, &inputs, &now).unwrap();
                let puts: Vec<_> = part.frames.iter().map(|(name,(rel,_))| FramePut {
                    rel:rel.clone(),part:0,digest:now["input"],
                    reads:if name=="result" {vec![("input".into(),now["input"])]} else {vec![]},
                    rows:FrameRows {types:vec![b'i'],n:1,rows:vec![42]}
                }).collect();
                store.keep_frames(&part.space, &puts, &[], None).unwrap();
                let (kept, snapshot) = load(&store, dir.path(), [7; 32], &prog, &inputs, &now).unwrap();
                assert!(kept.is_some());
                let rel = snapshot.frames[changed].0.clone();
                if deleted {
                    store.keep_frames(&part.space, &[], &[(rel,0)], None).unwrap();
                } else {
                    store.keep_frames(&part.space, &[FramePut {rel,part:0,digest:99,reads:vec![],
                        rows:FrameRows {types:vec![b'i'],n:1,rows:vec![999]}}], &[], None).unwrap();
                }
                let (kept, next) = load_from_snapshot(&store, [7;32], &inputs, &now, snapshot).unwrap();
                assert!(kept.is_none(), "{changed}, deleted={deleted}: old metadata must not certify new payloads");
                assert!(next.frames.values().all(|(_,was)|was.is_none()), "fresh results must be republished");
            }
        }
    }

    #[test]
    fn an_unchanged_native_plan_reuses_facts_and_admits_only_consumed_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("facts.sqlite");
        let text = ".decl e(a:number,b:number)\ne(1,2).\ne(2,3).\ne(3,4).\n\
            .decl r(a:number,b:number)\nr(a,b) :- e(a,b).\nr(a,c) :- r(a,b),e(b,c).\n\
            .decl total(n:number)\ntotal(n) :- n=count : {r(_,_)}.\n.output total\n";
        let run = || {
            let store = ContentStore::open(&path).unwrap();
            let p = rr::parse(text).unwrap();
            let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
            let lowered = lower::lower(&p, python, &mut |s| store.intern_atoms(s).map_err(|e| plan::Error::Plan(e.to_string()))).unwrap();
            let db = rusqlite::Connection::open(&path).unwrap();
            native::register(&db).unwrap();
            native::register_symbols(&db).unwrap();
            let mut kept = Kept::new(&store, [42; 32], &[]);
            let outcome = plan::run_atoms_native_hooked(&lowered, &db, "main", plan::Stats::empty(&lowered.program),
                Default::default(), "program", plan::Hooks { strata: Some(&mut kept), ..Default::default() }).unwrap();
            let counts = kept.strata.unwrap();
            let admitted: Vec<_> = kept.admitted.keys().cloned().collect();
            kept.finish();
            (counts, admitted, outcome.outputs)
        };
        let first = run();
        assert!(first.0.0 > 0, "cold native evaluation must do work");
        let warm = run();
        assert_eq!(warm.2, first.2);
        assert_eq!(warm.0.0, 0, "an unchanged native transformation must not execute");
        assert!(!warm.1.contains(&"r".to_owned()), "the retained closure has no running consumer and must not be hydrated: {:?}", warm.1);
        assert!(warm.1.contains(&"total".to_owned()), "the final output is demanded");
    }

    #[test]
    fn publication_between_metadata_and_payload_admission_invalidates_the_cut() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
        let rel = FrameRel { name: "r".into(), ident: [1; 32] };
        let put = |v| FramePut { rel: rel.clone(), part: 0, digest: v as u64, reads: vec![],
            rows: FrameRows { types: vec![0], rows: vec![v], n: 1 } };
        store.keep_frames(&[42; 32], &[put(1)], &[], None).unwrap();
        let mut kept = Kept::new(&store, [42; 32], &[]);
        kept.program(&HashMap::from([("r".into(), [1; 32])]));
        assert!(kept.has_rows("r"));
        store.keep_frames(&[42; 32], &[put(2)], &[], None).unwrap();
        kept.admit(&["r".into()]);
        assert!(!kept.has_rows("r"), "old metadata must never certify replacement rows");
    }
