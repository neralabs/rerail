    use super::*;

    #[test]
    fn catalog_inspection_does_not_attach_its_entries_to_another_run() {
        inspect_separately(|| {
            read_before_the_run([1; 32]);
            inspect_separately(|| read_before_the_run([2; 32]));
            assert_eq!(take_read_before(), vec![[1; 32]]);
        });
    }

    /// A batch's earlier run records its manifest and starts the store's collection while the pack's run has combined
    /// its program and queued it, and names it only in its own manifest, later. The entry is pinned until that run
    /// ends (`Pins`): the collection meanwhile keeps it. Before, it took the entry, and every batch combined its pack
    /// again.
    #[test]
    fn a_combined_program_outlives_a_collection_before_its_run_names_it() {
        let dir = std::env::temp_dir().join(format!("rerail-combined-pins-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = ContentStore::open(&dir.join("store.sqlite")).unwrap();
        let program = rr::parse_version(".decl e(x: int)\n.output e\ne(1).\n").unwrap();
        let key = [9u8; 32];
        record_version(&store, "the combined program", &key, &[], &program, &[]);
        store.settle(&queued(&key));
        // The collection another run of the batch starts: its passes, each of which ends a generation (a source is
        // collectable once a pass has ended the generation it was made in).
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let passes = || (0..2).for_each(|_| { store.collect(now, 256).unwrap(); });
        passes();
        assert!(lookup_version(&store, &key).unwrap().is_some(), "kept through another run's collection");
        // Once the run that combined it ends, the pins go: named by no manifest here, the entry is collected.
        drop(Pins::new(&store, &key));
        store.written().unwrap();
        passes();
        assert!(lookup_version(&store, &key).unwrap().is_none(), "named by no manifest, collected once its pins are released");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_combined_program_is_read_back_until_a_library_file_it_read_changes() {
        let dir = std::env::temp_dir().join(format!("rerail-combined-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = ContentStore::open(&dir.join("store.sqlite")).unwrap();
        let lib = dir.join("lib.rerail");
        std::fs::write(&lib, "a").unwrap();
        let missing = dir.join("missing.rerail");
        let program = rr::parse_version(".decl e(x: int)\n.output e\ne(1).\n").unwrap();
        let read = [LibraryFile { path: lib.clone(), sha256: Some(Sha256::digest(b"a").into()) }, LibraryFile { path: missing.clone(), sha256: None }];
        let key = [7u8; 32];
        record_version(&store, "the combined program", &key, &read, &program, &[]);
        store.settle(&queued(&key));
        let back = lookup_version(&store, &key).unwrap().expect("kept");
        assert_eq!((back.size(), back.digest()), (program.size(), program.digest()));
        assert!(retains_read(&store, &key, &read).unwrap());
        assert!(!retains_read(&store, &key, &read[..1]).unwrap(), "the destination must have the same include observations");
        // The source the manifest names is the store's.
        let (text, _) = named(&key);
        assert!(store.sources_by_hashes(&[HashKey::sha256(text.as_bytes())]).unwrap()[0].is_some());
        // Another key finds nothing.
        assert!(lookup_version(&store, &[8u8; 32]).unwrap().is_none());
        // A file appearing where an include found none combines again.
        // Each edit comes in a later run (`flush` ends one): a run reads its files as they were when it first looked
        // (decision 0105).
        let next_run = || super::super::observe(|o| o.flush());
        std::fs::write(&missing, "b").unwrap();
        next_run();
        assert!(lookup_version(&store, &key).unwrap().is_none());
        std::fs::remove_file(&missing).unwrap();
        next_run();
        assert!(lookup_version(&store, &key).unwrap().is_some());
        // So does an edited library file.
        std::fs::write(&lib, "c").unwrap();
        next_run();
        assert!(lookup_version(&store, &key).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_process_hit_reinstalls_a_collected_program_with_a_surviving_read_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rule.rerail");
        let text = ".decl e(x: int)\n.output e\ne(1).\n";
        std::fs::write(&path, text).unwrap();
        let path = path.canonicalize().unwrap();
        let program = rr::parse_own_version(text).unwrap();
        let digest = Sha256::digest(text.as_bytes()).into();
        let files = [LibraryFile { path: path.clone(), sha256: Some(digest) }];
        let key = rule_file_key(&path, &rr::library_root(), &digest);
        let database = dir.path().join("store.sqlite");
        super::super::with_tape_cache_path(&database, None, || {
            let store = ContentStore::open(&database).unwrap();
            rr::retain_read_program(&path, &rr::library_root(), &files, &program);
            store.settle(&queued(&key));
            assert!(retains_read(&store, &key, &files).unwrap());

            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
            let entry = store.source_by_hash(HashKey::sha256(source_text(&key).as_bytes())).unwrap().unwrap();
            store.pin_source(entry, "keep just the input record", now + 3600).unwrap();
            drop(Pins::new(&store, &key));
            store.written().unwrap();
            for _ in 0..2 { store.collect(now, 256).unwrap(); }
            assert!(read_record(&store, &key).unwrap().is_some());
            assert!(!retains_read(&store, &key, &files).unwrap());

            rr::retain_read_program(&path, &rr::library_root(), &files, &program);
            store.settle(&queued(&key));
            assert!(retains_read(&store, &key, &files).unwrap());
            assert!(lookup_read(&store, &key).unwrap().is_some());
        });
    }
