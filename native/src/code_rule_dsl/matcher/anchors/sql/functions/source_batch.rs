//! Source confirmation over a SQL-owned demand frame. One filter consumes the batch.
use super::reader::Shared;
use rusqlite::{
    Connection, Result, ffi,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor,
    },
};
use std::{
    borrow::Cow,
    collections::HashMap,
    ffi::{CStr, c_int},
};

pub(super) const SCHEMA: &str = "
CREATE TEMP TABLE source_batch_definition(producer INTEGER PRIMARY KEY,expression TEXT NOT NULL,literal INTEGER NOT NULL);
CREATE TEMP TABLE source_batch_demand(producer INTEGER NOT NULL,fid INTEGER NOT NULL,pre INTEGER NOT NULL);
CREATE TEMP TABLE source_batch_stats(n INTEGER NOT NULL);
INSERT INTO source_batch_stats VALUES(0);
CREATE TEMP TABLE source_batch_match(producer INTEGER NOT NULL,fid INTEGER NOT NULL,pre INTEGER NOT NULL,PRIMARY KEY(producer,fid,pre)) WITHOUT ROWID;";

pub(super) const CLEAR: &str = "DELETE FROM source_batch_match;DELETE FROM source_batch_demand;DELETE FROM source_batch_definition;UPDATE source_batch_stats SET n=0";

pub(super) fn install(db: &Connection, reader: Shared) -> Result<()> {
    const MODULE: Module<Table> = Module::eponymous_only_module();
    db.create_module(c"nera_source_batch", &MODULE, Some(reader))
}

#[repr(C)]
struct Table {
    base: ffi::sqlite3_vtab,
    reader: Shared,
}
#[repr(C)]
struct Cursor {
    base: ffi::sqlite3_vtab_cursor,
    reader: Shared,
    rows: Vec<[i64; 3]>,
    at: usize,
}

// SAFETY: SQLite's base is first; the reader lives with the owning connection.
unsafe impl<'a> VTab<'a> for Table {
    type Aux = Shared;
    type Cursor = Cursor;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Shared>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        Ok((
            Cow::Borrowed(
                c"CREATE TABLE x(producer INTEGER,fid INTEGER,pre INTEGER,frame INTEGER HIDDEN)",
            ),
            Self {
                base: ffi::sqlite3_vtab::default(),
                reader: aux.expect("source reader").clone(),
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut found = false;
        for (c, mut u) in info.constraints_and_usages() {
            if c.is_usable()
                && c.column() == 3
                && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                u.set_argv_index(1);
                u.set_omit(true);
                found = true;
                break;
            }
        }
        let n = self.reader.lock().connection()?.query_row(
            "SELECT n FROM source_batch_stats",
            [],
            |r| r.get::<_, i64>(0),
        )?;
        info.set_estimated_rows(n);
        info.set_estimated_cost(n.max(1) as f64);
        Ok(found)
    }
    fn open(&'a mut self) -> Result<Cursor> {
        Ok(Cursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            reader: self.reader.clone(),
            rows: Vec::new(),
            at: 0,
        })
    }
}
enum Predicate {
    Literal(String),
    Regex(regex::Regex),
}
impl Predicate {
    fn matches(&self, text: &str) -> bool {
        match self {
            Self::Literal(s) => s == text,
            Self::Regex(r) => r.is_match(text),
        }
    }
}
// SAFETY: the cursor owns output rows; no borrowed SQLite input survives filter.
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, _: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.rows.clear();
        self.at = 0;
        if args.get::<i64>(0)? != 1 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mut reader = self.reader.lock();
        #[cfg(feature = "code-rule-profile")]
        {
            reader.counters.source_batches += 1;
        }
        let db = reader.connection()?;
        let mut definitions=db.prepare("SELECT producer,expression,literal FROM source_batch_definition WHERE producer IN (SELECT producer FROM source_batch_demand)")?;
        let predicates: HashMap<i64, Predicate> = definitions
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            })?
            .map(|r| {
                let (id, expression, literal) = r?;
                let predicate = if literal {
                    Predicate::Literal(expression)
                } else {
                    #[cfg(feature = "code-rule-profile")]
                    {
                        reader.counters.regex_compiles += 1;
                    }
                    Predicate::Regex(
                        regex::Regex::new(&expression)
                            .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?,
                    )
                };
                Ok((id, predicate))
            })
            .collect::<Result<_>>()?;
        let mut input = db.prepare(
            "SELECT producer,fid,pre FROM source_batch_demand ORDER BY fid,pre,producer",
        )?;
        let mut rows = input.query([])?;
        let mut cached: Option<((i64, i64), String)> = None;
        let mut consumed = 0usize;
        while let Some(row) = rows.next()? {
            if consumed.is_multiple_of(256)
                && crate::code_rule_dsl::budget::matcher_budget_exhausted()
            {
                return Err(rusqlite::Error::SqliteFailure(
                    ffi::Error::new(ffi::SQLITE_INTERRUPT),
                    None,
                ));
            }
            consumed += 1;
            let producer = row.get::<_, i64>(0)?;
            let fid = row.get::<_, i64>(1)?;
            let pre = row.get::<_, i64>(2)?;
            let key = (fid, pre);
            if cached.as_ref().is_none_or(|(previous, _)| *previous != key) {
                let pre = usize::try_from(pre).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let span = reader.span(fid, pre)?;
                cached = Some((key, reader.slice(fid, span.start, span.len())?));
            }
            let predicate = predicates
                .get(&producer)
                .ok_or(rusqlite::Error::InvalidQuery)?;
            if predicate.matches(&cached.as_ref().expect("source slice").1) {
                self.rows.push([producer, fid, pre]);
            }
        }
        #[cfg(feature = "code-rule-profile")]
        {
            reader.counters.source_input_rows += consumed as u64;
            reader.counters.source_output_rows += self.rows.len() as u64;
        }
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.at += 1;
        Ok(())
    }
    fn eof(&self) -> bool {
        self.at >= self.rows.len()
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        if col == 3 {
            ctx.set_result(&1_i64)
        } else {
            ctx.set_result(&self.rows[self.at][col as usize])
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.at as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        language::GrammarDialect,
        matcher::anchors::sql::{store::Store, tests::document},
    };

    #[test]
    fn typed_frames_match_sql_without_inflating_distinct_consumer_scopes() -> Result<()> {
        let store = Store::temporary()?;
        store.db.execute_batch(
            "INSERT INTO source_batch_definition VALUES(1,'target',1),(2,'é|other',0)",
        )?;
        let mut input = Vec::new();
        for (producer, path, text) in [
            (1, "a.ts", "target('é'); other();"),
            (2, "b.ts", "other('é'); target();"),
        ] {
            let doc = document(text, GrammarDialect::TypeScript);
            let fid = store.put(path, &doc)?;
            for pre in (0..doc.transducer_len()).filter(|pre| pre % 2 == 0) {
                input.push([producer, fid, pre as i64].map(rusqlite::types::Value::Integer));
            }
        }
        super::super::store::publication::insert(
            &store.db,
            "INSERT INTO source_batch_demand",
            "",
            input,
        )?;
        store.db.execute(
            "UPDATE source_batch_stats SET n=(SELECT count(*) FROM source_batch_demand)",
            [],
        )?;
        let collect = |sql: &str| -> Result<Vec<(i64, i64, i64)>> {
            store
                .db
                .prepare(sql)?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect()
        };
        let expected = collect(
            "SELECT d.producer,d.fid,d.pre FROM source_batch_demand d JOIN source_batch_definition p USING(producer) WHERE CASE WHEN p.literal THEN nera_equal(p.expression,d.fid,d.pre) ELSE nera_matches(p.expression,d.fid,d.pre) END ORDER BY 1,2,3",
        )?;
        #[cfg(feature = "code-rule-profile")]
        let before = store.readers.0.lock().counters;
        let actual = collect("SELECT producer,fid,pre FROM nera_source_batch(1) ORDER BY 1,2,3")?;
        assert_eq!(actual, expected);
        assert!(!actual.is_empty());
        #[cfg(feature = "code-rule-profile")]
        {
            let counters = store.readers.0.lock().counters.since(before);
            assert_eq!(counters.source_batches, 1);
            let expected: i64 =
                store
                    .db
                    .query_row("SELECT n FROM source_batch_stats", [], |r| r.get(0))?;
            assert_eq!(counters.source_input_rows, expected as u64);
            assert_eq!(counters.source_output_rows, actual.len() as u64);
        }
        Ok(())
    }

    #[test]
    fn an_empty_frame_does_not_enter_the_native_producer() -> Result<()> {
        let store = Store::temporary()?;
        #[cfg(feature = "code-rule-profile")]
        let before = store.readers.0.lock().counters;
        let rows:i64=store.db.query_row("SELECT count(*) FROM source_batch_stats stats CROSS JOIN nera_source_batch(1) p WHERE stats.n>0",[],|r|r.get(0))?;
        assert_eq!(rows, 0);
        #[cfg(feature = "code-rule-profile")]
        assert_eq!(
            store.readers.0.lock().counters.since(before).source_batches,
            0
        );
        Ok(())
    }
}
