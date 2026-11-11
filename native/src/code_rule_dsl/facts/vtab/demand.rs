//! Immutable unary demand frames shared by batch operators within one statement.
//! SQL passes handles; readers share the key vector without serializing or rebuilding IN sets.
use rusqlite::{
    Connection, Result,
    functions::{Aggregate, Context, FunctionFlags},
};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

type Frames = HashMap<i64, Arc<Vec<i64>>>;
thread_local! {static FRAMES:RefCell<Option<Frames>>=const{RefCell::new(None)};}
static NEXT: AtomicI64 = AtomicI64::new(1);
pub(crate) struct Scope(Option<Frames>);
impl Scope {
    pub(crate) fn enter() -> Self {
        Self(FRAMES.with(|f| f.replace(Some(Frames::new()))))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        FRAMES.with(|f| {
            f.replace(self.0.take());
        });
    }
}
pub(crate) fn read(id: i64) -> Result<Arc<Vec<i64>>> {
    FRAMES
        .with(|f| f.borrow().as_ref().and_then(|f| f.get(&id).cloned()))
        .ok_or_else(|| super::failure("demand frame is outside its statement"))
}
struct Collect;
impl Aggregate<Vec<i64>, i64> for Collect {
    fn init(&self, _: &mut Context<'_>) -> Result<Vec<i64>> {
        Ok(Vec::new())
    }
    fn step(&self, ctx: &mut Context<'_>, keys: &mut Vec<i64>) -> Result<()> {
        if let Some(n) = ctx.get::<Option<i64>>(0)? {
            keys.push(n)
        }
        Ok(())
    }
    fn finalize(&self, _: &mut Context<'_>, keys: Option<Vec<i64>>) -> Result<i64> {
        let mut keys = keys.unwrap_or_default();
        keys.sort_unstable();
        keys.dedup();
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        FRAMES.with(|f| {
            let mut f = f.borrow_mut();
            let f = f
                .as_mut()
                .ok_or_else(|| super::failure("demand collection requires a statement scope"))?;
            f.insert(id, Arc::new(keys));
            Ok(id)
        })
    }
}
pub(super) fn register(db: &Connection) -> Result<()> {
    db.create_aggregate_function(c"rr_demand_frame", 1, FunctionFlags::SQLITE_UTF8, Collect)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_share_keys_and_cannot_escape_their_statement() {
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let first;
        {
            let _scope = Scope::enter();
            first=db.query_row("WITH keys(n) AS (VALUES(3),(1),(3),(NULL)) SELECT rr_demand_frame(n) FROM keys",[],|r|r.get(0)).unwrap();
            let a = read(first).unwrap();
            let b = read(first).unwrap();
            assert!(Arc::ptr_eq(&a, &b));
            assert_eq!(&**a, &[1, 3]);
            {
                let _nested = Scope::enter();
                assert!(read(first).is_err());
            }
            assert!(read(first).is_ok());
        }
        let _next = Scope::enter();
        assert!(read(first).is_err());
    }
}
