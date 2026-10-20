//! SQLite owns graph normalization, bounded root batches, and cycle-safe closure.
//! Consumers use a stable relation so another physical adapter need not change rules.
use super::{Connection, Result, Stage, quote, sql_error};

#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Reachability {
    pub(in crate::code_rule_dsl) roots: String,
    pub(in crate::code_rule_dsl) edges: String,
    pub(in crate::code_rule_dsl) batch_size: usize,
}

pub(super) fn create(db: &Connection, stage: &Stage, spec: &Reachability) -> Result<()> {
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
    #[cfg(feature = "code-rule-profile")]
    let span = super::super::profile::trace::Span::start("sql-reach-prepare", &stage.name);
    db.execute_batch(&format!(
        "CREATE TEMP TABLE {roots} AS SELECT DISTINCT __path,id FROM ({root_query}) WHERE __path IS NOT NULL AND id IS NOT NULL;
         CREATE TEMP TABLE {edges} AS SELECT DISTINCT __path,src,dst FROM ({edge_query}) WHERE __path IS NOT NULL AND src IS NOT NULL AND dst IS NOT NULL;
         CREATE TEMP TABLE {keys}(__path TEXT NOT NULL,key,compact INTEGER NOT NULL);
         INSERT INTO {keys} SELECT __path,key,row_number() OVER(ORDER BY __path,key) FROM (
           SELECT __path,id AS key FROM {roots} UNION SELECT __path,src FROM {edges} UNION SELECT __path,dst FROM {edges});
         CREATE UNIQUE INDEX {key_text} ON {keys}(__path,key);
         CREATE UNIQUE INDEX {key_int} ON {keys}(compact);
         CREATE TEMP TABLE {compact} AS SELECT a.compact AS src,b.compact AS dst FROM {edges} e
           JOIN {keys} a ON a.__path=e.__path AND a.key=e.src JOIN {keys} b ON b.__path=e.__path AND b.key=e.dst;
         CREATE INDEX {forward} ON {compact}(src,dst);
         CREATE TEMP TABLE {requests}(root INTEGER PRIMARY KEY);
         INSERT INTO {requests} SELECT k.compact FROM {roots} r JOIN {keys} k ON k.__path=r.__path AND k.key=r.id;
         DROP TABLE {roots}; DROP TABLE {edges};
         CREATE TEMP TABLE {batch}(root INTEGER PRIMARY KEY);
         CREATE TEMP TABLE {pairs}(root INTEGER,node INTEGER,PRIMARY KEY(root,node)) WITHOUT ROWID;
         ANALYZE temp.{keys}; ANALYZE temp.{compact}; ANALYZE temp.{requests};",
        root_query = spec.roots, edge_query = spec.edges,
        key_text = name("key_text"), key_int = name("key_int"), forward = name("forward"),
    )).map_err(sql_error)?;
    #[cfg(feature = "code-rule-profile")]
    drop(span);
    #[cfg(feature = "code-rule-profile")]
    let mut span = super::super::profile::trace::Span::start("sql-reach-walk", &stage.name);
    let mut fill = db
        .prepare(&format!(
            "INSERT INTO {batch} SELECT root FROM {requests} WHERE root>?1 ORDER BY root LIMIT ?2"
        ))
        .map_err(sql_error)?;
    let mut walk = db.prepare(&format!(
        "WITH RECURSIVE walk(root,node) AS (
           SELECT root,root FROM {batch} UNION SELECT w.root,e.dst FROM walk w JOIN {compact} e ON e.src=w.node)
         INSERT INTO {pairs} SELECT root,node FROM walk"
    )).map_err(sql_error)?;
    let mut last = 0_i64;
    let mut batches = 0_usize;
    let mut requested = 0_usize;
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
    #[cfg(feature = "code-rule-profile")]
    {
        span.detail(
            || serde_json::json!({"batchRows":spec.batch_size,"batches":batches,"roots":requested}),
        );
        drop(span);
    }
    #[cfg(not(feature = "code-rule-profile"))]
    let _ = (batches, requested);
    #[cfg(feature = "code-rule-profile")]
    let _span = super::super::profile::trace::Span::start("sql-reach-index", &stage.name);
    db.execute_batch(&format!(
        "CREATE INDEX {reverse} ON {pairs}(node,root);
         ANALYZE temp.{pairs};
         CREATE TEMP VIEW {output} AS SELECT a.__path,a.key AS root,b.key AS node FROM {pairs} p
           JOIN {keys} a ON a.compact=p.root JOIN {keys} b ON b.compact=p.node AND b.__path=a.__path;
         DROP TABLE {batch}; DROP TABLE {requests}; DROP TABLE {compact};",
        reverse = name("reverse"), output = quote(&stage.name),
    )).map_err(sql_error)?;
    Ok(())
}
