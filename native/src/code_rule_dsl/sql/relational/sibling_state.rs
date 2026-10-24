//! SQL-supplied completed sibling classifications. No AST or rule evaluation occurs here.
use rusqlite::{
    Connection, Error, Result,
    functions::FunctionFlags,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor, sqlite3_vtab, sqlite3_vtab_cursor,
    },
};
use std::{
    borrow::Cow,
    collections::HashMap,
    ffi::{CStr, c_int},
    marker::PhantomData,
    sync::{Arc, Mutex},
};

type Key = (u32, u32, u32, u32);
#[derive(Default)]
struct Store {
    entries: HashMap<Key, Arc<[u32]>>,
}
thread_local! {static COUNTS:std::cell::Cell<[usize;4]>=const{std::cell::Cell::new([0;4])};}
pub(super) fn counts() -> [usize; 4] {
    COUNTS.get()
}
fn count(index: usize, amount: usize) {
    let mut counts = COUNTS.get();
    counts[index] += amount;
    COUNTS.set(counts);
}
fn invalid(message: &str) -> Error {
    Error::ModuleError(format!("sibling classification: {message}"))
}
pub(super) fn register(db: &Connection) -> Result<()> {
    let store = Arc::new(Mutex::new(Store::default()));
    let reset = Arc::clone(&store);
    db.create_scalar_function(
        "relation_sibling_reset",
        1,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            let clause = ctx.get::<u32>(0)?;
            reset
                .lock()
                .map_err(|_| invalid("poisoned store"))?
                .entries
                .retain(|key, _| key.0 != clause);
            Ok(1_i64)
        },
    )?;
    db.create_scalar_function(
        "relation_sibling_fill",
        1,
        FunctionFlags::SQLITE_UTF8,
        |ctx| {
            let amount =
                usize::try_from(ctx.get::<i64>(0)?).map_err(|_| invalid("negative fill cost"))?;
            crate::code_rule_dsl::matcher::note_sql_sibling_fill(amount);
            Ok(1_i64)
        },
    )?;
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(c"relation_sibling_state", &MODULE, Some(store))
}
#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    store: Arc<Mutex<Store>>,
}
// SAFETY: SQLite base is first; every cursor keeps the typed store alive.
unsafe impl<'v> VTab<'v> for Table {
    type Aux = Arc<Mutex<Store>>;
    type Cursor = Cursor<'v>;
    fn connect(
        db: &mut VTabConnection,
        store: Option<&Self::Aux>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        Ok((Cow::Borrowed(c"CREATE TABLE x(found INTEGER,position INTEGER,mode INTEGER HIDDEN,clause INTEGER HIDDEN,term INTEGER HIDDEN,file INTEGER HIDDEN,parent INTEGER HIDDEN,node INTEGER HIDDEN,reverse INTEGER HIDDEN,nodes TEXT HIDDEN)"),Self{base:sqlite3_vtab::default(),store:store.ok_or_else(||invalid("missing store"))?.clone()}))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut keys = [None; 8];
        for (index, c) in info.constraints().enumerate() {
            if c.is_usable()
                && (2..10).contains(&c.column())
                && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                keys[(c.column() - 2) as usize] = Some(index);
            }
        }
        if keys.iter().any(Option::is_none) {
            return Ok(false);
        }
        for (arg, index) in keys.into_iter().enumerate() {
            let mut usage = info.constraint_usage(index.unwrap());
            usage.set_argv_index((arg + 1) as c_int);
            usage.set_omit(true);
        }
        info.set_estimated_cost(1.);
        info.set_estimated_rows(1);
        Ok(true)
    }
    fn open(&'v mut self) -> Result<Self::Cursor> {
        Ok(Cursor {
            base: sqlite3_vtab_cursor::default(),
            store: Arc::clone(&self.store),
            found: false,
            position: None,
            done: true,
            marker: PhantomData,
        })
    }
}
#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    store: Arc<Mutex<Store>>,
    found: bool,
    position: Option<i64>,
    done: bool,
    marker: PhantomData<&'v Table>,
}
// SAFETY: SQLite base is first; a result owns its numeric rank and no environment references.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, _: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.found = false;
        self.position = None;
        self.done = false;
        let mode = args.get::<i64>(0)?;
        if mode == -1 {
            return Ok(());
        }
        let key = (args.get(1)?, args.get(2)?, args.get(3)?, args.get(4)?);
        let node = args.get::<u32>(5)?;
        let reverse = args.get::<i64>(6)? != 0;
        let nodes = match mode {
            0 => {
                count(0, 1);
                self.store
                    .lock()
                    .map_err(|_| invalid("poisoned store"))?
                    .entries
                    .get(&key)
                    .cloned()
            }
            1 | 2 => {
                let nodes: Vec<u32> = serde_json::from_str(&args.get::<String>(7)?)
                    .map_err(|_| invalid("invalid SQL integer list"))?;
                if nodes.windows(2).any(|pair| pair[0] >= pair[1]) {
                    return Err(invalid("SQL classifications must be sorted and unique"));
                }
                let nodes: Arc<[u32]> = nodes.into();
                if mode == 1 {
                    let mut store = self.store.lock().map_err(|_| invalid("poisoned store"))?;
                    if let Some(before) = store.entries.get(&key) {
                        if before.as_ref() != nodes.as_ref() {
                            return Err(invalid("conflicting completed classification"));
                        }
                    } else {
                        count(2, 1);
                        count(3, nodes.len());
                        store.entries.insert(key, Arc::clone(&nodes));
                    }
                }
                Some(nodes)
            }
            _ => return Err(invalid("invalid operation")),
        };
        if let Some(nodes) = nodes {
            self.found = true;
            if mode == 0 {
                count(1, 1);
            }
            self.position = nodes.binary_search(&node).ok().map(|index| {
                if reverse {
                    (nodes.len() - index) as i64
                } else {
                    (index + 1) as i64
                }
            });
        }
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
        match column {
            0 => ctx.set_result(&self.found),
            1 => ctx.set_result(&self.position),
            _ => Err(invalid("hidden controls cannot be projected")),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_empty_lists_are_not_misses_and_partial_lists_are_not_retained() {
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let get = |mode: i64, term: i64, node: i64, reverse: i64, nodes: &str| {
            db.query_row(
                "SELECT found,position FROM relation_sibling_state(?1,1,?2,0,3,?3,?4,?5)",
                rusqlite::params![mode, term, node, reverse, nodes],
                |row| Ok((row.get::<_, bool>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
        };
        let before = counts();
        assert_eq!(get(0, 2, 5, 0, "").unwrap(), (false, None));
        assert_eq!(get(1, 2, 5, 0, "[2,5,8]").unwrap(), (true, Some(2)));
        assert_eq!(get(0, 2, 2, 1, "").unwrap(), (true, Some(3)));
        assert!(get(1, 2, 5, 0, "[2,5]").is_err());
        assert_eq!(get(1, 4, 5, 0, "[]").unwrap(), (true, None));
        assert_eq!(get(0, 4, 5, 0, "").unwrap(), (true, None));
        assert_eq!(get(2, 6, 5, 0, "[5]").unwrap(), (true, Some(1)));
        assert_eq!(get(0, 6, 5, 0, "").unwrap(), (false, None));
        db.query_row("SELECT relation_sibling_reset(1)", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
        assert_eq!(get(0, 2, 5, 0, "").unwrap(), (false, None));
        let after = counts();
        assert!(after[1] > before[1]);
        assert_eq!(after[2] - before[2], 2);
    }
}
