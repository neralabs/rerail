    use super::*;

    #[test]
    fn kept_choices_are_taken_at_their_scale_or_less_and_read_back_from_the_store() {
        let dir = std::env::temp_dir().join(format!("rerail-choices-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("store.sqlite");
        let program = super::super::super::rr::parse(".decl e(x: int)\n.decl f(x: int)\n.output f\ne(1).\nf(x) :- e(x).\n").unwrap();
        let rewritten = super::super::super::rr::parse(".decl e(x: int)\n.decl f(x: int)\n.output f\ne(1).\nf(1).\n").unwrap();
        let key = key(&program);
        assert_ne!(key, super::key(&rewritten), "the key is the program's");
        assert!(holds(1000.0, 10.0) && holds(1000.0, 2000.0) && !holds(1000.0, 2001.0));
        let prepared = [3u8; 32];
        {
            let _durable = Durable::start(&db, prepared, Default::default(), Default::default());
            remember(key, Chosen { program: Some(Arc::new(rewritten.clone())), inlined: vec!["e".into()], demanded: vec![], nodes: 1000.0 });
        }
        // Another process: nothing kept in memory, the store's entry read back.
        kept().lock().unwrap().clear();
        let store = ContentStore::open(&db).unwrap();
        // A new store's first collection sweeps the retired corpus memo schemas: never these entries (the choices
        // were schema 3, one of them, O-kept-strata-flake).
        store.collect_corpus_pass(&crate::content_store::CorpusPolicy::default(), 1_000).unwrap();
        let entry = read_all(&store, &prepared).unwrap().remove(&path(&key)).expect("kept in the store");
        let texts = super::super::super::program_text::texts(&store, &[super::rewritten(&entry).expect("a rewritten program")]).unwrap().into_iter().map(|(d, (text, _))| (d, text)).collect();
        let back = decode_chosen(&texts, &entry, &Default::default()).expect("decodes");
        assert_eq!(format!("{:?}", back.program.as_deref()), format!("{:?}", Some(&rewritten)));
        assert_eq!((back.inlined.clone(), back.nodes), (vec!["e".to_owned()], 1000.0));
        let (text, _) = named(&prepared);
        assert!(store.sources_by_hashes(&[HashKey::sha256(text.as_bytes())]).unwrap()[0].is_some());
        // Under another prepared program, nothing.
        assert!(read_all(&store, &[4u8; 32]).unwrap().is_empty());
        {
            let _durable = Durable::start(&db, prepared, Default::default(), Default::default());
            assert!(lookup(&key, 10.0).is_some(), "a smaller run takes the choices");
            assert!(lookup(&key, 5000.0).is_none(), "a run over many more nodes chooses again");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A plan kept in the store (decision 0116 step 2) reads back as it was written, by its rule and scale, and is
    /// rebuilt over the rule's body with the body's own literals; over a shorter body it is not taken.
    #[test]
    fn kept_plans_read_back_from_the_store_by_rule_and_scale() {
        use super::super::{Kept, Mode, Priced, Step, StoredPlan, scale};
        let prog = super::super::super::rr::parse(".decl e(x: int, y: int)\n.decl f(x: int)\n.decl g(x: int)\n.output g\ng(x) :- e(x, y), f(y), x != y.\n").unwrap();
        let body = &prog.rules[0].body;
        let order = vec![
            Step { at: 1, lit: body[1].clone(), mode: None },
            Step { at: 0, lit: body[0].clone(), mode: Some(Mode::Range) },
            Step { at: 2, lit: body[2].clone(), mode: Some(Mode::Descend) },
        ];
        let priced = Priced { sizes: vec![3700.0, 10.0, 0.0, -2.0], got: ((order.clone(), 12.5, 3.0), None) };
        let kept: Kept = (order.clone().into(), 0.25, None, 1.5, Some(Arc::new(priced)), Some(40.0));
        let stored = StoredPlan::of(&kept).expect("a plan the store keeps");
        assert_eq!((scale(3700.0), scale(55_000.0), scale(0.0)), (5, 7, 0), "scales by powers of 4");
        let key = (0x0123_4567_89ab_cdef_fedc_ba98_7654_3210_u128, scale(3700.0));
        let row = StoredPlan::row(key, &stored);
        assert_eq!(StoredPlan::of_row(&row), Some((key, stored.clone())));
        // The work its line last ran is kept with it (decision 0122), and a row stored without it reads as not run.
        assert_eq!(StoredPlan::of_row(&row).and_then(|(_, p)| p.kept(body)).and_then(|k| k.5), Some(40.0));
        assert_eq!(StoredPlan::of_row(&row[..row.len() - 1]).and_then(|(_, p)| p.kept(body)).map(|k| k.5), Some(None));
        // Within 2x of that work it is the same plan, not written again; past it, written.
        let ran = |r: f64| StoredPlan::of(&(kept.0.clone(), kept.1, None, kept.3, kept.4.clone(), Some(r))).unwrap();
        assert!(stored.same(&ran(70.0)) && !stored.same(&ran(90.0)));
        let back = stored.kept(body).expect("rebuilt over its body");
        let same = |a: &[Step], b: &[Step]| a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.at == b.at && a.lit == b.lit && a.mode == b.mode);
        assert!(same(&back.0, &order) && (back.1, back.3) == (0.25, 1.5));
        let price = back.4.expect("its price");
        assert!(same(&price.got.0.0, &order) && price.sizes == [3700.0, 10.0, 0.0, -2.0] && (price.got.0.1, price.got.0.2) == (12.5, 3.0));
        assert!(stored.kept(&body[..2]).is_none(), "a step past the body");
        // A run at another scale takes the plan stored at the nearest one (O-replan-churn), to re-price it.
        let mut plans = super::super::RunPlans { kept: Default::default(), stored: Arc::new([(key, stored.clone())].into_iter().collect()) };
        let near = plans.find(&key.0, body, 55_000.0).expect("the plan stored two scales down");
        assert!(same(&near.0, &order));
        assert!(plans.find(&(key.0 ^ 1), body, 3700.0).is_none(), "another rule's plan is not taken");
        let dir = std::env::temp_dir().join(format!("rerail-kept-plans-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = ContentStore::open(&dir.join("store.sqlite")).unwrap();
        let prepared = [9u8; 32];
        assert!(read_plans(&store, &prepared).unwrap().is_empty());
        assert!(write_plans(&store, &prepared, &[row.into_iter().map(Value::Int).collect()]).unwrap());
        let read = read_plans(&store, &prepared).unwrap();
        assert_eq!((read.len(), read.get(&key)), (1, Some(&stored)));
        assert!(read_plans(&store, &[8u8; 32]).unwrap().is_empty(), "another prepared program's are not taken");
        assert!(read_all(&store, &prepared).unwrap().is_empty(), "the choices do not read the plans");
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
