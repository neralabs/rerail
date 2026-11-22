use rerail_blocks::vector::Vector;
    use super::*;
    use crate::code_rule_dsl::logic::{native, plan, rr};
    use crate::code_rule_dsl::logic::plan::lower;
    /// A program's version, as a run reads it (t36 item 7).
    type V = std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>;
    fn pl(p: &V) -> crate::code_rule_dsl::logic::plan::body::Placed { crate::code_rule_dsl::logic::plan::body::Placed::whole(p.clone()) }
    fn st(p: &V) -> crate::code_rule_dsl::logic::front::Components {
        let s = crate::code_rule_dsl::logic::program_image::manager::analysis::<crate::code_rule_dsl::logic::program_image::Strata>(p).unwrap();
        crate::code_rule_dsl::logic::front::Components::of_names(s.sccs.iter().map(|scc| scc.iter().map(|&r| p.rel_name(r))))
    }
    /// An entry of one integer row.
    fn one(rel: &KeptRel, digest: u64, reads: Vec<(String, u64)>, v: i64) -> KeptPut { flat_put(rel, digest, reads, vec![b'i'], rerail_blocks::vector![v], 1) }
    fn gone(rel: &KeptRel) -> KeptPut { KeptPut { rel: rel.name.clone(), ident: rel.ident, part: 0, digest: 0, reads: vec![], types: vec![], what: KeptWhat::Gone } }

    #[test]
    fn cross_process_admission_batches_inputs_and_eligible_derived_frames() {
        use crate::code_rule_dsl::logic::profile;
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        profile::set(true);
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
        let mut text = String::new();
        let mut inputs = Vec::new();
        for i in 0..257 {
            text += &format!(".decl input{i}(x:number)\n.input input{i}\n.decl result{i}(x:number)\nresult{i}(x) :- input{i}(x).\n.output result{i}\n");
            inputs.push((format!("input{i}"), native::component::columns_source(vec!["c0".into()], vec![Some(rerail_blocks::vector![i])])));
        }
        let prog = rr::parse_version(&text).unwrap();
        let now: HashMap<_, _> = inputs.iter().map(|(r, s)| (r.clone(), super::super::retain::digest(s.as_ref(), &crate::code_rule_dsl::logic::plan::Cells::plain(s.columns().len())))).collect();
        let (_, part) = load(&store, dir.path(), [7; 32], [7; 32], &pl(&prog), &Default::default(), &st(&prog), &inputs, &now).unwrap();
        let puts: Vec<_> = part.frames.iter().map(|(name, (rel, _))| {
            let i: usize = name.trim_start_matches("input").trim_start_matches("result").parse().unwrap();
            one(rel, now[&format!("input{i}")], if name.starts_with("result") {vec![(format!("input{i}"), now[&format!("input{i}")])]} else {vec![]}, i as i64)
        }).collect();
        store.queue_kept(part.space, puts);
        // An entry of a derived relation whose inputs do not match: its rows are not bound.
        let stale = &part.frames["result256"].0;
        store.queue_kept(part.space, vec![one(stale, 99, vec![("input256".into(), now["input256"].wrapping_add(1))], 999)]);
        profile::reset();
        profile::set(true);
        let wave = profile::waves();
        let span = profile::span(|| "bulk retained admission".into());
        let (kept, _) = load(&store, dir.path(), [7; 32], [7; 32], &pl(&prog), &Default::default(), &st(&prog), &inputs, &now).unwrap();
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
        eprintln!("257 input entries and 256 eligible derived entries: {statements} storage SQL executions");
        assert!(statements <= 16, "the space's blocks are bound once, not read a query per relation: {statements}");
    }

    /// A run reads one commit of the space's blocks (decision 0401 F3): an entry replaced or forgotten after it bound
    /// them is not seen by it, its rows are the commit's; the next run reads the new commit, and a derived entry whose
    /// reads are not the inputs' digests now is not read.
    #[test]
    fn a_run_reads_one_commit_of_the_kept_blocks() {
        for changed in ["input", "result"] {
            for deleted in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
                let prog = rr::parse_version(".decl input(x:number)\n.input input\n.decl result(x:number)\nresult(x) :- input(x).\n.output result\n").unwrap();
                let inputs = vec![("input".to_owned(), native::component::columns_source(vec!["c0".into()], vec![Some(rerail_blocks::vector![42])]))];
                let now = HashMap::from([("input".to_owned(), super::super::retain::digest(inputs[0].1.as_ref(), &crate::code_rule_dsl::logic::plan::Cells::plain(inputs[0].1.columns().len())))]);
                let (_, part) = load(&store, dir.path(), [7; 32], [7; 32], &pl(&prog), &Default::default(), &st(&prog), &inputs, &now).unwrap();
                let puts: Vec<_> = part.frames.iter().map(|(name, (rel, _))| one(rel, now["input"], if name == "result" { vec![("input".into(), now["input"])] } else { vec![] }, 42)).collect();
                store.queue_kept(part.space, puts);
                let snapshot = super::super::retain::kept_read::Bound::of(&store, &part.space).unwrap().unwrap();
                let (kept, _) = load(&store, dir.path(), [7; 32], [7; 32], &pl(&prog), &Default::default(), &st(&prog), &inputs, &now).unwrap();
                assert!(kept.is_some());
                let rel = part.frames[changed].0.clone();
                store.queue_kept(part.space, vec![if deleted { gone(&rel) } else { one(&rel, 99, vec![], 999) }]);
                // The commit bound before the change reads as it was.
                let entry = snapshot.entry(&rel.name, &rel.ident, 0).expect("the bound commit's entry");
                let mut rows = Vec::new();
                entry.each_rows(&mut |r| rows.extend_from_slice(r));
                assert_eq!((entry.digest(), rows), (now["input"], vec![42]), "{changed}, deleted={deleted}: the bound commit's rows");
                // The next run reads the new commit: the result read only when its reads are the input's digest now.
                let (kept, _) = load(&store, dir.path(), [7; 32], [7; 32], &pl(&prog), &Default::default(), &st(&prog), &inputs, &now).unwrap();
                let rels: Vector<i64> = kept.iter().flat_map(|k| k.parts().1).flat_map(|(_, _, held)| held.live_rows().unwrap()).collect();
                assert!(!rels.contains(&999), "{changed}, deleted={deleted}: a replaced entry of other reads is not read");
                let result_kept = !(changed == "result" || (changed == "input" && !deleted));
                assert_eq!(rels == [42], result_kept, "{changed}, deleted={deleted}: {rels:?}");
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
            let p = rr::parse_version(text).unwrap();
            let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
            let lowered = lower::lowered(p, python, &mut |s| store.intern_atoms(s).map_err(|e| plan::Error::Plan(e.to_string()))).unwrap();
            let db = rusqlite::Connection::open(&path).unwrap();
            native::register(&db).unwrap();
            native::register_symbols(&db).unwrap();
            let mut kept = Kept::new(&store, [42; 32], &[], HashMap::new());
            let outcome = plan::run_atoms_native_hooked(&lowered, &db, "main", plan::Stats::empty_of_lowered(&lowered).unwrap(),
                Default::default(), "program", plan::Hooks { strata: Some(&mut kept), ..Default::default() }).unwrap();
            let counts = kept.strata.unwrap();
            let admitted = kept.bound_rels.clone();
            kept.finish();
            (counts, admitted, outcome.shown())
        };
        let first = run();
        assert!(first.0.0 > 0, "cold native evaluation must do work");
        let warm = run();
        assert_eq!(warm.2, first.2);
        assert_eq!(warm.0.0, 0, "an unchanged native transformation must not execute");
        assert!(!warm.1.contains(&"r".to_owned()), "the retained closure has no running consumer and is not bound: {:?}", warm.1);
        assert!(warm.1.contains(&"total".to_owned()), "the final output is bound");
    }

    /// An entry put after the strata's run bound its space is not read by it: the rows it binds are those of the
    /// commit it read the digests of, never newer ones under old digests.
    #[test]
    fn an_entry_put_after_the_run_bound_its_space_is_not_read_by_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
        let rel = KeptRel { name: "r".into(), ident: [1; 32] };
        store.queue_kept([42; 32], vec![one(&rel, 1, vec![], 1)]);
        let mut kept = Kept::new(&store, [42; 32], &[], HashMap::new());
        kept.program(&HashMap::from([("r".into(), [1; 32])]));
        assert!(kept.has_rows("r"));
        store.queue_kept([42; 32], vec![one(&rel, 2, vec![], 2)]);
        let (_, held) = kept.rows("r", b"i").expect("bound from the commit the run read");
        assert_eq!((kept.was("r"), held.live_rows().unwrap()), (Some(1), rerail_blocks::vector![1]));
    }

    /// A kept relation's frame keeps its kind and field columns as no atoms (`Kept::typed`), as `load` reads it back:
    /// typed by its declaration (`sym`, `s`), `load` refused it as kept under other types, and a forked process's
    /// writer mapped the grammar's codes as atoms.
    #[test]
    fn a_kept_relations_kind_column_is_no_atom() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
        let p = rr::parse_version(
            ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.decl text(n: node, s: symbol)\n\
             .input node, text\n.decl kinded(n: node, k: symbol, s: symbol)\n.output kinded\n\
             kinded(n, k, s) :- node(_, n, k, _, _), text(n, s).\n",
        )
        .unwrap();
        let domains = lower::domains_of(&p).unwrap();
        let mut kept = Kept::new(&store, [42; 32], &[], HashMap::new()).typed(&crate::code_rule_dsl::logic::plan::body::Placed::whole(p.clone()), &domains);
        kept.program(&HashMap::from([("kinded".into(), [1; 32])]));
        kept.ran("kinded", Some((7, vec![b's', b's', b's'], rerail_blocks::vector![1, 1132, 5], 1, vec![])));
        assert_eq!(kept.puts[0].types, b"iis", "the node and the kind are no atoms, the text is");
    }
