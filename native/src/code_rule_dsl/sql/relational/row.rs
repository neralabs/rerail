//! One-row, integer-only projection for a correlated SQL row-value subquery.
//! This cursor copies supplied columns. It cannot inspect ASTs, evaluate rules, or run SQL.
use rusqlite::{
    Connection, Error, Result,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor, sqlite3_vtab, sqlite3_vtab_cursor,
    },
};
use std::{
    borrow::Cow,
    ffi::{CStr, CString, c_int},
    marker::PhantomData,
};

// Truth, remaining work, plus the existing maximum of 128 typed capture slots.
const WIDTH: usize = 130;
thread_local! {
    pub(super) static ROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static CELLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
pub(super) fn register(db: &Connection) -> Result<()> {
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(c"relation_row", &MODULE, None::<()>)
}
#[repr(C)]
struct Table {
    base: sqlite3_vtab,
}
// SAFETY: SQLite's base is the first field. Each cursor owns only its current input row.
unsafe impl<'v> VTab<'v> for Table {
    type Aux = ();
    type Cursor = Cursor<'v>;
    fn connect(
        db: &mut VTabConnection,
        _: Option<&()>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let columns = (0..WIDTH)
            .map(|i| format!("c{i} INTEGER"))
            .collect::<Vec<_>>()
            .join(",");
        Ok((
            Cow::Owned(CString::new(format!("CREATE TABLE x({columns})")).unwrap()),
            Self {
                base: sqlite3_vtab::default(),
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut columns = std::collections::BTreeMap::new();
        for (index, c) in info.constraints().enumerate() {
            if c.is_usable()
                && c.column() >= 0
                && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_IS
            {
                columns.entry(c.column() as usize).or_insert(index);
            }
        }
        if columns.is_empty() || columns.keys().copied().ne(0..columns.len()) {
            return Ok(false);
        }
        for (&column, &index) in &columns {
            let mut usage = info.constraint_usage(index);
            usage.set_argv_index((column + 1) as c_int);
            usage.set_omit(true);
        }
        info.set_idx_num(columns.len() as c_int);
        info.set_estimated_cost(1.);
        info.set_estimated_rows(1);
        Ok(true)
    }
    fn open(&'v mut self) -> Result<Self::Cursor> {
        Ok(Cursor {
            base: sqlite3_vtab_cursor::default(),
            values: vec![],
            done: true,
            marker: PhantomData,
        })
    }
}
#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    values: Vec<Option<i64>>,
    done: bool,
    marker: PhantomData<&'v Table>,
}
// SAFETY: SQLite's base is first. No value or SQLite pointer outlives its filter call.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, width: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.values.clear();
        for index in 0..width as usize {
            self.values.push(args.get::<Option<i64>>(index)?);
        }
        self.done = false;
        ROWS.set(ROWS.get() + 1);
        CELLS.set(CELLS.get() + self.values.len());
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.done = true;
        Ok(())
    }
    fn eof(&self) -> bool {
        self.done
    }
    fn column(&self, ctx: &mut Context, column: c_int) -> Result<()> {
        let value = self
            .values
            .get(column as usize)
            .ok_or_else(|| Error::ModuleError("relation_row: unconstrained column".into()))?;
        ctx.set_result(value)
    }
    fn rowid(&self) -> Result<i64> {
        Ok(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::functions::FunctionFlags;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[test]
    fn row_subquery_evaluates_once_and_preserves_null() {
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        db.create_scalar_function("tick", 1, FunctionFlags::SQLITE_UTF8, move |ctx| {
            count.fetch_add(1, Ordering::Relaxed);
            ctx.get::<i64>(0)
        })
        .unwrap();
        let query = "WITH RECURSIVE walk(n) AS (SELECT 0 UNION ALL SELECT r.c0 FROM walk w CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH result AS MATERIALIZED (SELECT tick(w.n+1) AS n) SELECT n,NULL FROM result) WHERE w.n<4) SELECT n FROM walk";
        let output: Vec<i64> = db
            .prepare(query)
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(output, vec![0, 1, 2, 3, 4]);
        assert_eq!(calls.load(Ordering::Relaxed), 4);
        let value: Option<i64> = db
            .query_row(
                "SELECT c1 FROM relation_row WHERE (c0,c1) IS (SELECT 1,NULL)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(value, None);
        let columns = (0..WIDTH)
            .map(|i| format!("c{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let values = (0..WIDTH)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let last: i64 = db
            .query_row(
                &format!("SELECT c129 FROM relation_row WHERE ({columns}) IS (SELECT {values})"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(last, 129);
        assert!(
            db.query_row::<i64, _, _>("SELECT c1 FROM relation_row WHERE c0 IS 1", [], |r| r
                .get(0))
                .is_err()
        );
        assert!(db.prepare("SELECT * FROM relation_row").is_err());
        assert!(
            db.query_row::<i64, _, _>(
                "SELECT c0 FROM relation_row WHERE c0 IS (SELECT 'not an integer')",
                [],
                |r| r.get(0)
            )
            .is_err()
        );
    }
    #[test]
    fn child_scope_does_not_repeat_a_correlated_prefix() {
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        db.create_scalar_function("tick", 1, FunctionFlags::SQLITE_UTF8, move |ctx| {
            count.fetch_add(1, Ordering::Relaxed);
            ctx.get::<i64>(0)
        })
        .unwrap();
        let query = "WITH RECURSIVE walk(n) AS (SELECT 0 UNION ALL SELECT r.c0 FROM walk w CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH a AS MATERIALIZED (SELECT tick(w.n+1) AS n) SELECT a.n,b.c0 FROM a CROSS JOIN relation_row b ON b.c0 IS (WITH child AS MATERIALIZED (SELECT a.n AS n) SELECT n FROM child)) WHERE w.n<4) SELECT n FROM walk";
        let output: Vec<i64> = db
            .prepare(query)
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(output, vec![0, 1, 2, 3, 4]);
        assert_eq!(calls.load(Ordering::Relaxed), 4);
    }
}
