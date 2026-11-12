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
            let _durable = Durable::start(&db, prepared);
            remember(key, Chosen { program: Some(Arc::new(rewritten.clone())), inlined: vec!["e".into()], demanded: vec![], nodes: 1000.0 });
        }
        // Another process: nothing kept in memory, the store's entry read back.
        kept().lock().unwrap().clear();
        let store = ContentStore::open(&db).unwrap();
        let back = read_all(&store, &prepared).unwrap().remove(&path(&key)).expect("kept in the store");
        assert_eq!(format!("{:?}", back.program.as_deref()), format!("{:?}", Some(&rewritten)));
        assert_eq!((back.inlined.clone(), back.nodes), (vec!["e".to_owned()], 1000.0));
        let (text, _) = named(&prepared);
        assert!(store.sources_by_hashes(&[HashKey::sha256(text.as_bytes())]).unwrap()[0].is_some());
        // Under another prepared program, nothing.
        assert!(read_all(&store, &[4u8; 32]).unwrap().is_empty());
        {
            let _durable = Durable::start(&db, prepared);
            assert!(lookup(&key, 10.0).is_some(), "a smaller run takes the choices");
            assert!(lookup(&key, 5000.0).is_none(), "a run over many more nodes chooses again");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
