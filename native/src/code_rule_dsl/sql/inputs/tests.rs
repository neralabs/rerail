use super::*;

fn rows(connection: &Connection, sql: &str) -> Vec<(i64, i64, i64)> {
    connection
        .prepare(sql)
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<Vec<_>>>()
        .unwrap()
}

#[test]
fn virtual_root_reads_agree_with_the_original_integer_primary_key_table() {
    let connection = Connection::open_in_memory().unwrap();
    let inputs = Arc::new(RootInputs::default());
    inputs.register(&connection).unwrap();
    connection.execute_batch("CREATE TABLE expected(run INTEGER NOT NULL,file INTEGER NOT NULL,node INTEGER NOT NULL,PRIMARY KEY(run,file,node)) WITHOUT ROWID").unwrap();
    let runs = [
        ClauseRun {
            statement: 3,
            files: vec![
                (0, Some(Arc::from([7, 1, 7, 3]))),
                (2, Some(Arc::from([]))),
                (4, None),
            ],
        },
        ClauseRun {
            statement: 1,
            files: vec![(1, Some(Arc::from([2, 9, u32::MAX])))],
        },
        ClauseRun {
            statement: 3,
            files: vec![(0, Some(Arc::from([5, 3]))), (1, Some(Arc::from([6])))],
        },
    ];
    for run in &runs {
        for (file, nodes) in &run.files {
            if let Some(nodes) = nodes {
                for node in nodes.iter() {
                    connection
                        .execute(
                            "INSERT OR IGNORE INTO expected VALUES (?1,?2,?3)",
                            [run.statement as i64, *file as i64, i64::from(*node)],
                        )
                        .unwrap();
                }
            }
        }
    }
    assert_eq!(inputs.install(&runs), 8);
    for condition in [
        "1",
        "run=3",
        "run=3 AND file=0",
        "run=3 AND file=1 AND node=6",
        "run='3' AND file='0' AND node='0007'",
        "run='3' AND file=0 AND node=7",
        "file=1",
        "node=7",
        "run IN (1,3)",
        "node IN (1,5,9)",
        "run='3'",
        "file='1.0'",
        "node='0007'",
        "node=7.0",
        "run=3.5",
        "node=7.5",
        "run=NULL",
        "node IS NULL",
        "run IS 3",
        "run=-1",
        "node=4294967295",
        "node=4294967296",
        "node=-1",
        "node=9223372036854775807",
        "node=9223372036854775808.0",
        "run=x'33'",
        "run='not a number'",
        "run='3' COLLATE NOCASE",
        "run=3 AND node=7 AND node=5",
        "run>=1 AND file<=1 AND node>2",
    ] {
        for order in [
            "run,file,node",
            "node DESC,file,run",
            "run DESC,file DESC,node DESC",
        ] {
            let query = format!("SELECT run,file,node FROM @ WHERE {condition} ORDER BY {order}");
            assert_eq!(
                rows(&connection, &query.replace('@', "roots")),
                rows(&connection, &query.replace('@', "expected")),
                "{query}"
            );
        }
    }
    let query = "SELECT a.run,a.file,b.node FROM @ a JOIN @ b ON b.run=a.run AND b.file=a.file AND b.node=a.node WHERE a.run=3 ORDER BY a.run,a.file,b.node";
    assert_eq!(
        rows(&connection, &query.replace('@', "roots")),
        rows(&connection, &query.replace('@', "expected"))
    );
    let plan: String = connection
        .query_row(
            "EXPLAIN QUERY PLAN SELECT * FROM roots WHERE run=3",
            [],
            |row| row.get(3),
        )
        .unwrap();
    assert!(plan.contains("VIRTUAL TABLE INDEX 1:"), "{plan}");
}

#[test]
fn ordered_root_slices_are_shared_and_replacement_and_clear_release_the_old_input() {
    let connection = Connection::open_in_memory().unwrap();
    let inputs = Arc::new(RootInputs::default());
    inputs.register(&connection).unwrap();
    let nodes: Arc<[u32]> = Arc::from([1, 3, 5, 7]);
    let weak = Arc::downgrade(&nodes);
    let runs = [ClauseRun {
        statement: 0,
        files: vec![
            (0, Some(nodes.clone())),
            (1, None),
            (2, Some(Arc::from([]))),
        ],
    }];
    inputs.install(&runs);
    assert!(Arc::ptr_eq(&nodes, &inputs.current.read().files[0].nodes));
    drop(runs);
    drop(nodes);
    assert_eq!(
        rows(
            &connection,
            "SELECT run,file,node FROM roots ORDER BY run,file,node"
        ),
        [(0, 0, 1), (0, 0, 3), (0, 0, 5), (0, 0, 7)]
    );
    let mut cached = connection
        .prepare_cached("SELECT run,file,node FROM roots WHERE run=0 ORDER BY file,node")
        .unwrap();
    assert_eq!(
        cached
            .query_map([], |row| row.get::<_, i64>(2))
            .unwrap()
            .collect::<Result<Vec<_>>>()
            .unwrap(),
        [1, 3, 5, 7]
    );
    inputs.clear();
    assert!(
        weak.upgrade().is_none(),
        "completed statements do not retain the old root pages"
    );
    assert!(
        cached
            .query_map([], |row| row.get::<_, i64>(2))
            .unwrap()
            .collect::<Result<Vec<_>>>()
            .unwrap()
            .is_empty()
    );
    inputs.install(&[ClauseRun {
        statement: 0,
        files: vec![(3, Some(Arc::from([8, 10])))],
    }]);
    assert_eq!(
        cached
            .query_map([], |row| row.get::<_, i64>(2))
            .unwrap()
            .collect::<Result<Vec<_>>>()
            .unwrap(),
        [8, 10]
    );
}
