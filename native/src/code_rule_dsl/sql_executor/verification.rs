//! Headless verification controls. These methods do not alter the Node API.
use super::*;

impl CodeRuleSqlExecutor {
    #[doc(hidden)]
    pub fn use_cold_oracle(&mut self) {
        self.cold_oracle = true;
    }

    #[doc(hidden)]
    pub fn verification_rows(&self, sql: &str) -> Result<Vec<Vec<Option<i64>>>> {
        self.profiled_read("verification snapshot", |this| {
            let mut statement = this.db.prepare(sql).map_err(error)?;
            if !statement.readonly() || statement.column_count() == 0 {
                return Err(error("verification requires a read-only row query"));
            }
            let width = statement.column_count();
            statement
                .query_map([], |r| {
                    (0..width)
                        .map(|c| r.get(c))
                        .collect::<rusqlite::Result<Vec<Option<i64>>>>()
                })
                .map_err(error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(error)
        })
    }
}
