//! One batch scan of a clause's selected root domain, before relational filtering.
use super::*;
pub(in crate::code_rule_dsl::sql) fn register(
    db: &Connection,
    name: &CStr,
    active: Active,
    estimate: Option<u64>,
) -> Result<()> {
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(name, &MODULE, Some((active, estimate)))
}
#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    active: Active,
    estimate: Option<u64>,
}
// SAFETY: SQLite base is first and the batch slot is owned.
unsafe impl<'v> VTab<'v> for Table {
    type Aux = (Active, Option<u64>);
    type Cursor = Cursor<'v>;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Self::Aux>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let (active, estimate) = aux.ok_or_else(|| invalid("missing candidate slot"))?;
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(file INTEGER,node INTEGER,request INTEGER)"),
            Self {
                base: sqlite3_vtab::default(),
                active: active.clone(),
                estimate: *estimate,
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let count = self
            .estimate
            .map_or(i64::MAX, |v| v.min(i64::MAX as u64) as i64);
        info.set_estimated_rows(count);
        info.set_estimated_cost(count.max(1) as f64);
        Ok(true)
    }
    fn open(&'v mut self) -> Result<Cursor<'v>> {
        Ok(Cursor {
            base: sqlite3_vtab_cursor::default(),
            active: self.active.clone(),
            rows: vec![],
            at: 0,
            phantom: PhantomData,
        })
    }
}
#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    active: Active,
    rows: Vec<Request>,
    at: usize,
    phantom: PhantomData<&'v Table>,
}
// SAFETY: SQLite base is first; rows and active slot remain owned by this cursor.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, _: c_int, _: Option<&str>, _: &Filters<'_>) -> Result<()> {
        let (batch, f) = self
            .active
            .read()
            .unwrap()
            .clone()
            .ok_or_else(|| invalid("no candidate batch"))?;
        self.rows = if let Some(scope) = &f.scope {
            f.requests_for(&batch, scope)?
        } else {
            f.requests.clone()
        };
        self.at = 0;
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.at += 1;
        if self.at == self.rows.len() {
            // SQLite can keep an exhausted UNION arm open until the statement finishes.
            self.rows = Vec::new();
        }
        Ok(())
    }
    fn eof(&self) -> bool {
        self.at >= self.rows.len()
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        match col {
            0 => ctx.set_result(&self.rows[self.at].file),
            1 => ctx.set_result(&self.rows[self.at].node),
            _ => ctx.set_result(&(self.at as i64)),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.at as i64)
    }
}
