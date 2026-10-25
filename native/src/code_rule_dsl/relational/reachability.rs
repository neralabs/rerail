//! SQLite owns graph normalization, bounded root batches, and cycle-safe closure.
//! Consumers use a stable relation so another physical adapter need not change rules.
use super::{Result, Stage, quote, sql_error};
use std::sync::{Arc, atomic::AtomicUsize};

#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Reachability {
    pub(in crate::code_rule_dsl) roots: String,
    pub(in crate::code_rule_dsl) edges: String,
    pub(in crate::code_rule_dsl) batch_size: usize,
    pub(in crate::code_rule_dsl) forest: bool,
    pub(in crate::code_rule_dsl) syntax: Option<String>,
    pub(in crate::code_rule_dsl) expand: bool,
}

pub(super) fn create(
    db: &super::Database,
    stage: &Stage,
    spec: &Reachability,
    work: &Arc<AtomicUsize>,
) -> Result<()> {
    // Internal names cannot be authored as stage/input identifiers or read by policies.
    let name = |suffix: &str| quote(&format!("__reach_{}_{}", stage.name, suffix));
    let (roots, edges, keys, compact, requests, batch, pairs) = (
        name("roots"),
        name("edges"),
        name("keys"),
        name("compact"),
        name("requests"),
        name("batch"),
        name("pairs"),
    );
    let expansion = if spec.expand {
        "CASE WHEN expand THEN 1 ELSE 0 END"
    } else {
        "1"
    };
    #[cfg(feature = "code-rule-profile")]
    let span = super::super::profile::trace::Span::start("sql-reach-prepare", &stage.name);
    // Unary plus preserves each scalar value and removes declared affinity from
    // CTAS identity columns. Otherwise INTEGER source columns force a numeric
    // comparison against the untyped key catalog and disable its key index term.
    db.execute_batch(&format!(
        "CREATE TEMP TABLE {roots} AS SELECT __path,+id AS id,max({expansion}) AS expand FROM ({root_query}) WHERE __path IS NOT NULL AND id IS NOT NULL GROUP BY __path,+id;
         CREATE TEMP TABLE {edges} AS SELECT DISTINCT __path,+src AS src,+dst AS dst FROM ({edge_query}) WHERE __path IS NOT NULL AND src IS NOT NULL AND dst IS NOT NULL;
         CREATE TEMP TABLE {keys}(__path TEXT NOT NULL,key,compact INTEGER PRIMARY KEY);
         INSERT INTO {keys}(__path,key)
           SELECT __path,id AS key FROM {roots} UNION SELECT __path,src FROM {edges} UNION SELECT __path,dst FROM {edges};
",
        root_query=spec.roots,edge_query=spec.edges,
    )).map_err(sql_error)?;
    if db.changes() == 0 {
        // Both root and edge SELECTs have run. An empty normalized key catalog
        // proves there is no graph to validate, index or traverse.
        db.execute_batch(&format!(
            "DROP TABLE {roots};DROP TABLE {edges};DROP TABLE {keys}"
        ))
        .map_err(sql_error)?;
        db.storage
            .borrow_mut()
            .declare(
                db,
                &stage.name,
                format!(
                    "CREATE TEMP TABLE {}(__path TEXT,root,node)",
                    quote(&stage.name)
                ),
                true,
            )
            .map_err(sql_error)?;
        return Ok(());
    }
    db.storage
        .borrow_mut()
        .discard(db, &stage.name)
        .map_err(sql_error)?;
    db.execute_batch(&format!(
        "         CREATE UNIQUE INDEX {key_text} ON {keys}(__path,key);
         CREATE TEMP TABLE {compact} AS SELECT a.compact AS src,b.compact AS dst FROM {edges} e
           JOIN {keys} a ON a.__path=e.__path AND a.key=e.src JOIN {keys} b ON b.__path=e.__path AND b.key=e.dst;
         CREATE INDEX {forward} ON {compact}(src,dst);
         CREATE TEMP TABLE {requests}(root INTEGER PRIMARY KEY,expand INTEGER NOT NULL);
         INSERT INTO {requests} SELECT k.compact,r.expand FROM {roots} r JOIN {keys} k ON k.__path=r.__path AND k.key=r.id;
         DROP TABLE {roots}; DROP TABLE {edges};
         CREATE TEMP TABLE {batch}(root INTEGER PRIMARY KEY,expand INTEGER NOT NULL);
         CREATE TEMP TABLE {pairs}(root INTEGER,node INTEGER,PRIMARY KEY(root,node)) WITHOUT ROWID;
         ANALYZE temp.{keys}; ANALYZE temp.{compact}; ANALYZE temp.{requests};",
        key_text = name("key_text"), forward = name("forward"),
    )).map_err(sql_error)?;
    #[cfg(feature = "code-rule-profile")]
    drop(span);
    #[cfg(feature = "code-rule-profile")]
    let mut span = super::super::profile::trace::Span::start("sql-reach-walk", &stage.name);
    let (mut batches, mut requested) = (0, 0);
    if spec.forest {
        (requested, batches) = super::forest::materialize(
            db,
            &format!("__reach_{}", stage.name),
            spec.batch_size,
            work,
        )?;
    } else {
        let mut fill = db
        .prepare(&format!(
            "INSERT INTO {batch} SELECT root,expand FROM {requests} WHERE root>?1 ORDER BY root LIMIT ?2"
        ))
        .map_err(sql_error)?;
        let mut walk = db.prepare(&format!(
        "WITH RECURSIVE walk(root,node,expand) AS (
           SELECT root,root,expand FROM {batch} UNION SELECT w.root,e.dst,w.expand FROM walk w JOIN {compact} e ON e.src=w.node WHERE w.expand)
         INSERT INTO {pairs} SELECT root,node FROM walk"
    )).map_err(sql_error)?;
        let mut last = 0_i64;
        loop {
            db.execute(&format!("DELETE FROM {batch}"), [])
                .map_err(sql_error)?;
            let count = fill
                .execute(rusqlite::params![last, spec.batch_size as i64])
                .map_err(sql_error)?;
            if count == 0 {
                break;
            }
            last = db
                .query_row(&format!("SELECT max(root) FROM {batch}"), [], |r| r.get(0))
                .map_err(sql_error)?;
            walk.execute([]).map_err(sql_error)?;
            batches += 1;
            requested += count;
        }
    }
    #[cfg(feature = "code-rule-profile")]
    {
        span.detail(
            || serde_json::json!({"batchRows":spec.batch_size,"batches":batches,"roots":requested,"forest":spec.forest}),
        );
        drop(span);
    }
    #[cfg(not(feature = "code-rule-profile"))]
    let _ = (batches, requested);
    #[cfg(feature = "code-rule-profile")]
    let _span = super::super::profile::trace::Span::start("sql-reach-index", &stage.name);
    if !spec.forest {
        db.execute_batch(&format!(
            "CREATE INDEX {} ON {pairs}(node,root); ANALYZE temp.{pairs}",
            name("reverse")
        ))
        .map_err(sql_error)?;
    }
    let source = if spec.forest {
        format!(
            "(SELECT r.root,p.node FROM {} r JOIN {} p ON p.ordinal>=r.lo AND p.ordinal<r.hi)",
            name("ranges"),
            name("positions")
        )
    } else {
        pairs.clone()
    };
    db.execute_batch(&format!(
        "CREATE TEMP VIEW {output} AS SELECT a.__path,a.key AS root,b.key AS node FROM {source} p
           JOIN {keys} a ON a.compact=p.root JOIN {keys} b ON b.compact=p.node AND b.__path=a.__path;
         DROP TABLE {batch}; DROP TABLE {requests}; DROP TABLE {compact};",
        output = quote(&stage.name),
    )).map_err(sql_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_catalog_needs_no_graph_storage_but_nonempty_edges_still_require_forest_validation() {
        let work = Arc::new(AtomicUsize::new(1_000_000));
        let db = super::super::connection(1024, work.clone()).unwrap();
        db.execute_batch("CREATE TABLE roots(__path,id);CREATE TABLE edges(__path,src,dst)")
            .unwrap();
        let spec = Reachability {
            roots: "SELECT * FROM roots".into(),
            edges: "SELECT * FROM edges".into(),
            batch_size: 16,
            forest: true,
            syntax: None,
            expand: false,
        };
        let stage = Stage {
            reads_source: false,
            recursive: false,
            inputs: Default::default(),
            name: "reachable".into(),
            query: String::new(),
            indexes: vec![],
            reachability: Some(spec.clone()),
            deferred: Default::default(),
            stored_columns: vec![],
            output_columns: vec![],
            declaration: Arc::from(""),
        };
        create(&db, &stage, &spec, &work).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM reachable", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM temp.sqlite_schema WHERE name LIKE '__reach_%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        db.execute_batch("INSERT INTO edges VALUES('a.py',1,2),('a.py',2,1)")
            .unwrap();
        let error = create(&db, &stage, &spec, &work).unwrap_err();
        assert!(error.reason.contains("cycle"), "{error}");
    }
}
