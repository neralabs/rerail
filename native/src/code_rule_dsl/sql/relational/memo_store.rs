//! Integer-only keyed scratch storage. SQL supplies keys, answers, and measured costs.
//! No AST access, rule evaluation, budget accumulator, or nested SQL execution.
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
    ffi::{CStr, CString, c_int},
    marker::PhantomData,
    sync::{Arc, Mutex},
};

const SLOTS: usize = 128;
const OUTPUTS: usize = 2 + SLOTS;
const MODE: usize = OUTPUTS;
const CLAUSE: usize = MODE + 1;
const NODE: usize = MODE + 4;
const KEYS: usize = MODE + 5;
const TRUTH: usize = KEYS + SLOTS;
const COST: usize = TRUTH + 1;
const VALUES: usize = COST + 1;
const WIDTH: usize = VALUES + SLOTS;

type SparseRow = Vec<(usize, i64)>;

#[derive(Default)]
struct Store {
    entries: rustc_hash::FxHashMap<SparseRow, SparseRow>,
}
thread_local! {
    static COUNTS: std::cell::Cell<[usize; 5]> = const { std::cell::Cell::new([0; 5]) };
}
pub(super) fn counts() -> [usize; 5] {
    COUNTS.get()
}
fn count(index: usize, amount: usize) {
    let mut c = COUNTS.get();
    c[index] += amount;
    COUNTS.set(c);
}

pub(super) fn register(db: &Connection) -> Result<()> {
    let store = Arc::new(Mutex::new(Store::default()));
    let reset = store.clone();
    db.create_scalar_function(
        "relation_memo_reset",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DIRECTONLY,
        move |ctx| {
            let clause = ctx.get::<i64>(0)?;
            reset
                .lock()
                .unwrap()
                .entries
                .retain(|key, _| key[0] != (CLAUSE, clause));
            count(0, 1);
            Ok(1_i64)
        },
    )?;
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(c"relation_memo", &MODULE, Some(store))
}
#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    store: Arc<Mutex<Store>>,
}
// SAFETY: SQLite base is first. The shared store owns integers only.
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
        let mut columns = vec!["truth INTEGER".into(), "cost INTEGER".into()];
        columns.extend((0..SLOTS).map(|s| format!("o{s} INTEGER")));
        columns.extend(
            ["mode", "clause", "utility", "file", "node"].map(|s| format!("{s} INTEGER HIDDEN")),
        );
        columns.extend((0..SLOTS).map(|s| format!("k{s} INTEGER HIDDEN")));
        columns.extend([
            "vtruth INTEGER HIDDEN".into(),
            "vcost INTEGER HIDDEN".into(),
        ]);
        columns.extend((0..SLOTS).map(|s| format!("v{s} INTEGER HIDDEN")));
        Ok((
            Cow::Owned(CString::new(format!("CREATE TABLE x({})", columns.join(","))).unwrap()),
            Self {
                base: sqlite3_vtab::default(),
                store: store.unwrap().clone(),
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut inputs = std::collections::BTreeMap::new();
        for (index, c) in info.constraints().enumerate() {
            if c.is_usable()
                && (MODE as i32..WIDTH as i32).contains(&c.column())
                && matches!(
                    c.operator(),
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_IS
                        | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_ISNULL
                )
            {
                inputs.entry(c.column() as usize).or_insert((
                    index,
                    c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_ISNULL,
                ));
            }
        }
        if !(MODE..=NODE).all(|i| inputs.contains_key(&i)) {
            return Ok(false);
        }
        let mut arg = 0;
        for &(index, is_null) in inputs.values() {
            let mut usage = info.constraint_usage(index);
            if !is_null {
                arg += 1;
                usage.set_argv_index(arg);
            }
            usage.set_omit(true);
        }
        info.set_idx_str(
            &inputs
                .iter()
                .map(|(column, (_, is_null))| {
                    format!("{}{column}", if *is_null { "n" } else { "" })
                })
                .collect::<Vec<_>>()
                .join(","),
        );
        info.set_estimated_cost(1.);
        info.set_estimated_rows(1);
        Ok(true)
    }
    fn open(&'v mut self) -> Result<Self::Cursor> {
        Ok(Cursor {
            base: sqlite3_vtab_cursor::default(),
            store: self.store.clone(),
            values: vec![],
            done: true,
            marker: PhantomData,
        })
    }
}
#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    store: Arc<Mutex<Store>>,
    values: SparseRow,
    done: bool,
    marker: PhantomData<&'v Table>,
}
// SAFETY: SQLite base is first. All arguments are copied before filter returns.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, _: c_int, columns: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.done = true;
        let mut values = vec![];
        let mut arg = 0;
        for column in columns.unwrap().split(',') {
            if column.starts_with('n') {
                continue;
            }
            if let Some(value) = args.get::<Option<i64>>(arg)? {
                values.push((column.parse::<usize>().unwrap(), value));
            }
            arg += 1;
        }
        let get = |column: usize| {
            values
                .binary_search_by_key(&column, |&(c, _)| c)
                .ok()
                .map(|i| values[i].1)
        };
        let Some(mode) = get(MODE) else {
            return Ok(());
        };
        if (CLAUSE..=NODE).any(|i| get(i).is_none()) {
            return Err(Error::ModuleError("memo: missing identity".into()));
        }
        let key: Vec<_> = values
            .iter()
            .copied()
            .filter(|(c, _)| (CLAUSE..TRUTH).contains(c))
            .collect();
        let mut store = self.store.lock().unwrap();
        match mode {
            0 => {
                count(1, 1);
                if let Some(answer) = store.entries.get(&key) {
                    count(2, 1);
                    self.values.clone_from(answer);
                    self.done = false;
                }
            }
            1 => {
                let truth =
                    get(TRUTH).ok_or_else(|| Error::ModuleError("memo: missing answer".into()))?;
                if !(0..=2).contains(&truth) || get(COST).is_none_or(|v| v < 1) {
                    return Err(Error::ModuleError(
                        "memo: only completed answers with positive costs can be published".into(),
                    ));
                }
                let mut answer = vec![(0, truth), (1, get(COST).unwrap())];
                answer.extend(
                    values
                        .iter()
                        .filter(|(c, _)| *c >= VALUES)
                        .map(|&(c, v)| (c - VALUES + 2, v)),
                );
                if let Some(prior) = store.entries.get(&key) {
                    if prior != &answer {
                        return Err(Error::ModuleError("memo: conflicting publication".into()));
                    }
                } else {
                    count(3, 1);
                    count(4, 2 * (key.len() + answer.len()));
                    store.entries.insert(key, answer.clone());
                }
                self.values = answer;
                self.done = false;
            }
            _ => return Err(Error::ModuleError("memo: invalid mode".into())),
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
        if column < 0 || column as usize >= OUTPUTS {
            return Err(Error::ModuleError(
                "memo: hidden input cannot be projected".into(),
            ));
        }
        ctx.set_result(
            &self
                .values
                .binary_search_by_key(&(column as usize), |&(c, _)| c)
                .ok()
                .map(|i| self.values[i].1),
        )
    }
    fn rowid(&self) -> Result<i64> {
        Ok(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scratch_storage_is_typed_guarded_and_resets_on_statement_reuse() {
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let identity = "m.clause IS 0 AND m.utility IS 1 AND m.file IS 2 AND m.node IS 3";
        let query = format!(
            "WITH init AS MATERIALIZED (SELECT relation_memo_reset(0) ready), put AS MATERIALIZED (SELECT m.truth FROM init CROSS JOIN relation_memo m ON m.mode IS CASE WHEN init.ready=1 THEN 1 END AND {identity} AND m.k127 IS 17 AND m.vtruth IS 1 AND m.vcost IS ?1 AND m.v127 IS 42) SELECT m.truth,m.cost,m.o127,m.o0 FROM put CROSS JOIN relation_memo m ON m.mode IS 0 AND {identity} AND m.k127 IS 17 WHERE put.truth=1"
        );
        let mut stmt = db.prepare(&query).unwrap();
        for cost in [2, 7] {
            let result: (i64, i64, i64, Option<i64>) = stmt
                .query_row([cost], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .unwrap();
            assert_eq!(result, (1, cost, 42, None));
        }
        for extra in ["m.k127 IS NULL", "m.k127 IS 18"] {
            let count: i64 = db.query_row(&format!("SELECT count(*) FROM relation_memo m WHERE m.mode IS 0 AND {identity} AND {extra}"),[],|r|r.get(0)).unwrap();
            assert_eq!(count, 0);
        }
        let before = counts();
        let count: i64 = db.query_row(&format!("SELECT count(*) FROM relation_memo m WHERE m.mode IS NULL AND {identity} AND m.vtruth IS -2"),[],|r|r.get(0)).unwrap();
        assert_eq!(count, 0);
        assert_eq!(counts(), before);
        assert!(db.query_row::<i64,_,_>(&format!("SELECT truth FROM relation_memo m WHERE m.mode IS 1 AND {identity} AND m.vtruth IS -2 AND m.vcost IS 1"),[],|r|r.get(0)).is_err());
        assert!(db.prepare("SELECT * FROM relation_memo").is_err());
    }
}
