//! Specialized keyed-decoder dependency waves over compiler-generated CTEs.
use std::{
    collections::{BTreeSet, HashMap},
    sync::LazyLock,
};

pub(super) fn identifiers(sql: &str) -> impl Iterator<Item = &str> {
    static IDENTIFIER: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"[A-Za-z_][A-Za-z_0-9]*").expect("generated identifier")
    });
    IDENTIFIER.find_iter(sql).map(|token| token.as_str())
}

/// A batched lookup adds one barrier. Other generated operators inherit their input barriers.
/// Recursive SQL stays inside its operation; this is not a general graph scheduler.
pub(super) fn analyze(
    ctes: &[String],
    barriers: &BTreeSet<String>,
    query: &str,
) -> (HashMap<String, usize>, HashMap<String, usize>) {
    let names: Vec<_> = ctes
        .iter()
        .map(|cte| cte.split_once('(').expect("generated columns").0)
        .collect();
    let mut references: HashMap<_, _> = names
        .iter()
        .enumerate()
        .map(|(i, &name)| (name, (i, 0)))
        .collect();
    let mut levels = Vec::with_capacity(ctes.len());
    for (i, cte) in ctes.iter().enumerate() {
        let body = cte.split_once(" AS ").expect("generated body").1;
        let mut level = 0;
        for token in identifiers(body) {
            if let Some((index, uses)) = references.get_mut(token) {
                *uses += 1;
                if *index < i {
                    level = level.max(levels[*index]);
                }
            }
        }
        levels.push(level + usize::from(barriers.contains(names[i])));
    }
    for token in identifiers(query) {
        if let Some((_, uses)) = references.get_mut(token) {
            *uses += 1;
        }
    }
    let uses = references
        .into_iter()
        .map(|(name, (_, uses))| (name.to_owned(), uses))
        .collect();
    let levels = names
        .into_iter()
        .zip(levels)
        .map(|(name, level)| (name.to_owned(), level))
        .collect();
    (levels, uses)
}

/// Bound each compound SELECT below SQLite's term limit without another host submission.
/// Key deduplication belongs to the enclosing missing-coverage relation.
pub(super) fn union(scopes: impl IntoIterator<Item = String>) -> String {
    compound(
        "fid,pre",
        scopes
            .into_iter()
            .map(|scope| format!("SELECT fid,pre FROM {scope}"))
            .collect(),
    )
}

pub(super) fn compound(columns: &str, mut arms: Vec<String>) -> String {
    assert!(!arms.is_empty(), "a decoder wave has admitted demand");
    while arms.len() > 128 {
        arms = arms
            .chunks(128)
            .map(|arms| format!("SELECT {columns} FROM ({})", arms.join(" UNION ALL ")))
            .collect();
    }
    arms.join(" UNION ALL ")
}

/// One materialized missing-key set and one decode per key, with SQL-owned coverage.
pub(super) fn fill(scopes: impl IntoIterator<Item = String>) -> (Vec<String>, &'static str) {
    let requests = union(scopes);
    (vec![
        format!("needed(fid,pre) AS MATERIALIZED (SELECT DISTINCT s.fid,s.pre FROM ({requests}) s LEFT JOIN decoded_spans d ON d.fid=s.fid AND d.pre=s.pre WHERE d.fid IS NULL)"),
        "decoded(fid,pre,packed) AS MATERIALIZED (SELECT s.fid,s.pre,nera_span(s.fid,s.pre) FROM needed s)".into(),
    ], "INSERT INTO decoded_spans(fid,pre,start,len) SELECT fid,pre,nera_word(packed,0),nera_word(packed,1) FROM decoded")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn child_span_and_later_child_requests_keep_their_real_dependencies() {
        let ctes = [
            "roots(fid,pre) AS MATERIALIZED (VALUES(1,1))",
            "children(fid,pre,child) AS NOT MATERIALIZED (SELECT fid,pre,2 FROM roots)",
            "selected(fid,pre) AS MATERIALIZED (SELECT fid,child FROM children WHERE child IS NOT NULL)",
            "spans(fid,pre,start,len) AS MATERIALIZED (SELECT fid,pre,0,1 FROM selected)",
            "filtered(fid,pre) AS MATERIALIZED (SELECT fid,pre FROM spans WHERE len>0)",
            "later(fid,pre,child) AS NOT MATERIALIZED (SELECT fid,pre,3 FROM filtered)",
        ].map(str::to_owned);
        let (levels, _) = analyze(
            &ctes,
            &BTreeSet::from(["children".into(), "spans".into(), "later".into()]),
            "SELECT * FROM later",
        );
        assert_eq!(levels["children"], 1);
        assert_eq!(levels["spans"], 2);
        assert_eq!(levels["later"], 3);
    }
    #[test]
    fn independent_selectors_share_a_wave_and_dependent_source_predicates_wait() {
        let ctes = [
            "left(fid,pre) AS MATERIALIZED (VALUES(1,1))",
            "span_a(fid,pre,start,len) AS MATERIALIZED (SELECT fid,pre,0,1 FROM left)",
            "filtered(fid,pre) AS MATERIALIZED (SELECT fid,pre FROM span_a WHERE start=0)",
            "span_b(fid,pre,start,len) AS MATERIALIZED (SELECT fid,pre,0,1 FROM filtered)",
            "right(fid,pre) AS MATERIALIZED (VALUES(1,2))",
            "span_c(fid,pre,start,len) AS MATERIALIZED (SELECT fid,pre,0,1 FROM right)",
        ]
        .map(str::to_owned);
        let (levels, _) = analyze(
            &ctes,
            &BTreeSet::from(["span_a".into(), "span_b".into(), "span_c".into()]),
            "SELECT * FROM span_b UNION ALL SELECT * FROM span_c",
        );
        assert_eq!(levels["left"], 0);
        assert_eq!(levels["right"], 0);
        assert_eq!(levels["span_a"], 1);
        assert_eq!(levels["span_c"], 1);
        assert_eq!(levels["filtered"], 1);
        assert_eq!(levels["span_b"], 2);
    }
    #[test]
    fn many_demand_sources_keep_one_statement_and_preserve_overlap_before_key_deduplication() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE input(fid INTEGER,pre INTEGER);INSERT INTO input VALUES(1,1),(1,2)",
        )
        .unwrap();
        let union = union(std::iter::repeat_n("input".to_owned(), 700));
        let sql = format!("SELECT count(*) FROM ({union})");
        assert_eq!(
            db.query_row(&sql, [], |r| r.get::<_, i64>(0)).unwrap(),
            1400
        );
        let sql = format!("SELECT count(*) FROM (SELECT DISTINCT fid,pre FROM ({union}))");
        assert_eq!(db.query_row(&sql, [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    use rusqlite::{Connection, Result, functions::FunctionFlags};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    fn database(calls: Arc<AtomicUsize>, fail: Arc<AtomicBool>) -> Result<Connection> {
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE decoded_spans(fid INTEGER,pre INTEGER,start INTEGER,len INTEGER,PRIMARY KEY(fid,pre)) WITHOUT ROWID; CREATE TABLE left_keys(fid INTEGER,pre INTEGER); CREATE TABLE right_keys(fid INTEGER,pre INTEGER); INSERT INTO left_keys VALUES(1,1),(1,2),(1,2);INSERT INTO right_keys VALUES(1,2),(1,3);")?;
        db.create_scalar_function(c"nera_span", 2, FunctionFlags::SQLITE_UTF8, move |ctx| {
            let pre = ctx.get::<i64>(1)?;
            calls.fetch_add(1, Ordering::Relaxed);
            if fail.load(Ordering::Relaxed) && pre == 2 {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok((pre * 10) << 32 | 4)
        })?;
        db.create_scalar_function(c"nera_word", 2, FunctionFlags::SQLITE_UTF8, move |ctx| {
            let packed = ctx.get::<i64>(0)?;
            Ok(if ctx.get::<i64>(1)? == 0 {
                packed >> 32
            } else {
                packed & 0xffff_ffff
            })
        })?;
        Ok(db)
    }
    fn sql() -> String {
        let (ctes, body) = fill(["left_keys".into(), "right_keys".into()]);
        format!("WITH {} {body}", ctes.join(","))
    }
    #[test]
    fn overlap_empty_demand_repeated_consumption_and_generation_invalidation_preserve_coverage()
    -> Result<()> {
        let calls = Arc::new(AtomicUsize::new(0));
        let db = database(calls.clone(), Arc::default())?;
        let sql = sql();
        db.execute(&sql, [])?;
        assert_eq!(
            calls.load(Ordering::Relaxed),
            3,
            "one decode per distinct key across all demand sources"
        );
        let rows = db
            .prepare("SELECT pre,start,len FROM decoded_spans ORDER BY pre")?
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>>>()?;
        assert_eq!(rows, [(1, 10, 4), (2, 20, 4), (3, 30, 4)]);
        db.execute(&sql, [])?;
        assert_eq!(
            calls.load(Ordering::Relaxed),
            3,
            "later consumers reuse completed keys"
        );
        db.execute_batch("DELETE FROM left_keys;DELETE FROM right_keys")?;
        db.execute(&sql, [])?;
        assert_eq!(
            calls.load(Ordering::Relaxed),
            3,
            "empty demand performs no decode"
        );
        db.execute_batch("INSERT INTO left_keys VALUES(2,1)")?;
        db.execute(&sql, [])?;
        assert_eq!(
            calls.load(Ordering::Relaxed),
            4,
            "another source never aliases an ordinal"
        );
        db.execute_batch(
            "DELETE FROM decoded_spans WHERE fid=1;INSERT INTO right_keys VALUES(1,1)",
        )?;
        db.execute(&sql, [])?;
        assert_eq!(
            calls.load(Ordering::Relaxed),
            5,
            "source replacement invalidates its coverage"
        );
        Ok(())
    }
    #[test]
    fn interrupted_publication_rolls_back_and_a_retry_recomputes_missing_keys() -> Result<()> {
        let calls = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(AtomicBool::new(true));
        let db = database(calls.clone(), fail.clone())?;
        let tx = db.unchecked_transaction()?;
        assert!(tx.execute(&sql(), []).is_err());
        tx.rollback()?;
        assert_eq!(
            db.query_row("SELECT count(*) FROM decoded_spans", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        fail.store(false, Ordering::Relaxed);
        calls.store(0, Ordering::Relaxed);
        let tx = db.unchecked_transaction()?;
        tx.execute(&sql(), [])?;
        tx.commit()?;
        assert_eq!(calls.load(Ordering::Relaxed), 3);
        assert_eq!(
            db.query_row("SELECT count(*) FROM decoded_spans", [], |r| r
                .get::<_, i64>(0))?,
            3
        );
        Ok(())
    }
}
