// COPY: Tempo 8fe9ffed4:tempo/native/src/content_store.rs l.6192-6244 (test), verbatim.
    #[test]
    fn completed_tree_result_segments_round_trip_atomically() {
        let (_path, store) = store("tree-results");
        let tree = HashKey::git_sha1([31; 20]);
        let snapshot = store
            .put_snapshot(tree, 1, HashKey::sha256(b"grammar"), 1)
            .unwrap();
        let workspace = store
            .open_workspace("/repo/tree-results", "repo", "owner", snapshot, 1, 100)
            .unwrap();
        let catalog = candidate_catalog("tree-results");
        let bytes = br#"{"files":["src/a.py"],"findings":[],"failures":[]}"#;
        let segment = TreeResultSegmentPublication {
            prefix: "src".to_owned(),
            tree,
            page_index: 0,
            page_count: 1,
            raw_length: bytes.len(),
            file_count: 1,
            frame: zstd::stream::encode_all(Cursor::new(bytes), 3).unwrap(),
        };
        store
            .complete_snapshot_trees(
                WorkspaceCompletion {
                    workspace_id: workspace,
                    revision: 2,
                    completed_at: 200,
                },
                snapshot,
                catalog,
                &[("src".to_owned(), tree)],
                vec![segment],
            )
            .unwrap();
        assert_eq!(
            store.completed_workspace_revision(workspace).unwrap(),
            Some(2)
        );
        assert_eq!(
            store
                .reusable_maximal_tree_prefixes(catalog, &[("src".to_owned(), tree)])
                .unwrap(),
            HashSet::from(["src".to_owned()])
        );
        let stored = store
            .tree_result_segments(catalog, &[("src".to_owned(), tree)])
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].page_index, 0);
        assert_eq!(stored[0].page_count, 1);
        assert_eq!(stored[0].file_count, 1);
        assert_eq!(stored[0].bytes, bytes);
    }
