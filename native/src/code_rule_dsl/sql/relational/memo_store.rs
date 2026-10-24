//! Typed keyed scratch storage. SQL supplies keys, answers, and measured costs.
//! No AST access, rule evaluation, budget accumulator, or nested SQL execution.
use super::capture_cell::Cell;
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
const PEAK: usize = 2 + SLOTS;
const OUTPUTS: usize = PEAK + 1;
const MODE: usize = OUTPUTS;
const CLAUSE: usize = MODE + 1;
const NODE: usize = MODE + 4;
const KEYS: usize = MODE + 5;
const TRUTH: usize = KEYS + SLOTS;
const COST: usize = TRUTH + 1;
const VALUES: usize = COST + 1;
const VPEAK: usize = VALUES + SLOTS;
const WIDTH: usize = VPEAK + 1;

type SparseRow = Vec<(usize, Cell)>;

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
    let peaks = super::peak_tracker::register(db)?;
    let continuations = super::continuation_store::register(db)?;
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
                .retain(|key, _| key[0] != (CLAUSE, Cell::Integer(clause)));
            peaks.lock().unwrap().reset();
            continuations.lock().unwrap().reset();
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
// SAFETY: SQLite base is first. The shared store owns integers and self-contained capture bytes.
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
        columns.extend((0..SLOTS).map(|s| format!("o{s}")));
        columns.push("peak INTEGER".into());
        columns.extend(
            ["mode", "clause", "utility", "file", "node"].map(|s| format!("{s} INTEGER HIDDEN")),
        );
        columns.extend((0..SLOTS).map(|s| format!("k{s} HIDDEN")));
        columns.extend([
            "vtruth INTEGER HIDDEN".into(),
            "vcost INTEGER HIDDEN".into(),
        ]);
        columns.extend((0..SLOTS).map(|s| format!("v{s} HIDDEN")));
        columns.push("vpeak INTEGER HIDDEN".into());
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
            inputs: vec![],
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
    inputs: SparseRow,
    done: bool,
    marker: PhantomData<&'v Table>,
}
// SAFETY: SQLite base is first. All arguments are copied before filter returns.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, _: c_int, columns: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.done = true;
        self.inputs.clear();
        let mut values = vec![];
        let mut arg = 0;
        for column in columns.unwrap().split(',') {
            if column.starts_with('n') {
                continue;
            }
            let column = column.parse::<usize>().unwrap();
            let value = args.get::<Cell>(arg)?;
            if value != Cell::Null {
                if ((MODE..KEYS).contains(&column)
                    || column == TRUTH
                    || column == COST
                    || column == VPEAK)
                    && value.integer().is_none()
                {
                    return Err(Error::ModuleError(
                        "memo: identity and control columns require integers".into(),
                    ));
                }
                values.push((column, value));
            }
            arg += 1;
        }
        // SQLite may retain residual checks beyond its omit-argument budget.
        // Hidden projections echo this cursor's exact request, never stored keys.
        self.inputs.clone_from(&values);
        let get = |column: usize| {
            values
                .binary_search_by_key(&column, |&(c, _)| c)
                .ok()
                .and_then(|i| values[i].1.integer())
        };
        let Some(mode) = get(MODE) else {
            return Ok(());
        };
        if (CLAUSE..=NODE).any(|i| get(i).is_none()) {
            return Err(Error::ModuleError("memo: missing identity".into()));
        }
        let key: Vec<_> = values
            .iter()
            .filter(|(c, _)| (CLAUSE..TRUTH).contains(c))
            .map(|(column, value)| {
                Ok((
                    *column,
                    if (KEYS..TRUTH).contains(column) {
                        value.memo_key()?
                    } else {
                        value.clone()
                    },
                ))
            })
            .collect::<Result<_>>()?;
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
                let peak = get(VPEAK);
                if peak.is_some_and(|peak| !(1..=256).contains(&peak)) {
                    return Err(Error::ModuleError(
                        "memo: completed peak must be 1..256".into(),
                    ));
                }
                let mut answer = vec![
                    (0, Cell::Integer(truth)),
                    (1, Cell::Integer(get(COST).unwrap())),
                ];
                answer.extend(
                    values
                        .iter()
                        .filter(|(c, _)| (VALUES..VPEAK).contains(c))
                        .map(|(c, v)| (c - VALUES + 2, v.clone())),
                );
                if let Some(peak) = peak {
                    answer.push((PEAK, Cell::Integer(peak)));
                }
                if let Some(prior) = store.entries.get(&key) {
                    if peak.is_some() {
                        // Native completed answers replace earlier entries. This matters
                        // when a depth-rejected hit recomputes a shallower answer.
                        let growth = answer.len().saturating_sub(prior.len());
                        count(4, 2 * growth);
                        store.entries.insert(key.clone(), answer.clone());
                    } else if prior != &answer {
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
        if column < 0 || column as usize >= WIDTH {
            return Err(Error::ModuleError("memo: invalid column".into()));
        }
        let values = if (column as usize) < OUTPUTS {
            &self.values
        } else {
            &self.inputs
        };
        ctx.set_result(
            &values
                .binary_search_by_key(&(column as usize), |&(c, _)| c)
                .ok()
                .map(|i| &values[i].1),
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
    fn list_keys_and_answers_distinguish_empty_absent_and_reset() {
        use super::super::capture_cell::encode_list;
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let empty = encode_list(2, &[]).unwrap();
        let nonempty = encode_list(2, &[3, 1]).unwrap();
        let put = "SELECT o0 FROM relation_memo WHERE mode IS 1 AND clause IS 0 AND utility IS 1 AND file IS 2 AND node IS 3 AND k0 IS ?1 AND vtruth IS 1 AND vcost IS 2 AND v0 IS ?2";
        let answer: Vec<u8> = db
            .query_row(put, rusqlite::params![empty, nonempty], |r| r.get(0))
            .unwrap();
        assert_eq!(answer, nonempty);
        let lookup = "SELECT o0 FROM relation_memo WHERE mode IS 0 AND clause IS 0 AND utility IS 1 AND file IS 2 AND node IS 3 AND k0 IS ?1";
        let answer: Vec<u8> = db.query_row(lookup, [&empty], |r| r.get(0)).unwrap();
        assert_eq!(answer, nonempty);
        assert!(matches!(
            db.query_row::<Vec<u8>, _, _>(lookup, [None::<Vec<u8>>], |r| r.get(0)),
            Err(rusqlite::Error::QueryReturnedNoRows)
        ));
        assert!(matches!(
            db.query_row::<Vec<u8>, _, _>(lookup, [&nonempty], |r| r.get(0)),
            Err(rusqlite::Error::QueryReturnedNoRows)
        ));
        db.query_row::<i64, _, _>("SELECT relation_memo_reset(0)", [], |r| r.get(0))
            .unwrap();
        assert!(matches!(
            db.query_row::<Vec<u8>, _, _>(lookup, [&empty], |r| r.get(0)),
            Err(rusqlite::Error::QueryReturnedNoRows)
        ));
        assert!(db.query_row::<i64,_,_>("SELECT truth FROM relation_memo WHERE mode IS 0 AND clause IS ?1 AND utility IS 1 AND file IS 2 AND node IS 3",[&empty],|r|r.get(0)).is_err());
    }

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

#[cfg(test)]
mod peak_tests {
    use super::*;
    const ID: &str = "m.clause IS 0 AND m.utility IS 4 AND m.file IS 0 AND m.node IS 7";
    #[test]
    fn relative_peak_fit_rejects_a_deep_hit_without_losing_the_answer() {
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let put = format!(
            "SELECT peak FROM relation_memo m WHERE m.mode IS 1 AND {ID} AND m.vtruth IS 2 AND m.vcost IS 7 AND m.vpeak IS 3"
        );
        assert_eq!(
            db.query_row::<i64, _, _>(&put, [], |r| r.get(0)).unwrap(),
            3
        );
        for (incoming, expected) in [(253, Some(2)), (254, None), (0, Some(2))] {
            let query = format!(
                "SELECT m.truth FROM (SELECT ?1 AS incoming) d LEFT JOIN relation_memo m ON m.mode IS 0 AND {ID} AND m.peak<=256-d.incoming"
            );
            assert_eq!(
                db.query_row::<Option<i64>, _, _>(&query, [incoming], |r| r.get(0))
                    .unwrap(),
                expected
            );
        }
        // Native publication replaces a prior completed entry after recomputation.
        let put = format!(
            "SELECT truth,cost,peak FROM relation_memo m WHERE m.mode IS 1 AND {ID} AND m.vtruth IS 0 AND m.vcost IS 2 AND m.vpeak IS 1"
        );
        assert_eq!(
            db.query_row::<(i64, i64, i64), _, _>(&put, [], |r| Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?
            )))
            .unwrap(),
            (0, 2, 1)
        );
    }
    #[test]
    fn failed_answers_and_invalid_peaks_do_not_publish_and_reset_closes_state() {
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        for (truth, peak) in [(-1, 1), (-2, 1), (1, 0), (1, 257)] {
            let query = format!(
                "SELECT truth FROM relation_memo m WHERE m.mode IS 1 AND {ID} AND m.vtruth IS ?1 AND m.vcost IS 1 AND m.vpeak IS ?2"
            );
            assert!(
                db.query_row::<i64, _, _>(&query, rusqlite::params![truth, peak], |r| r.get(0))
                    .is_err()
            );
        }
        assert!(matches!(
            db.query_row::<i64, _, _>(
                &format!("SELECT truth FROM relation_memo m WHERE m.mode IS 0 AND {ID}"),
                [],
                |r| r.get(0)
            ),
            Err(Error::QueryReturnedNoRows)
        ));
        let token: i64 = db
            .query_row("SELECT relation_peak_open(0)", [], |r| r.get(0))
            .unwrap();
        db.query_row::<i64, _, _>(
            "SELECT handle FROM relation_continuation WHERE mode IS 1 AND v0 IS 7",
            [],
            |r| r.get(0),
        )
        .unwrap();
        db.query_row::<i64, _, _>("SELECT relation_memo_reset(0)", [], |r| r.get(0))
            .unwrap();
        assert!(
            db.query_row::<i64, _, _>("SELECT relation_peak_close(?1)", [token], |r| r.get(0))
                .is_err()
        );
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT relation_continuation_rows()", [], |r| r.get(0))
                .unwrap(),
            0
        );
    }
}

#[cfg(test)]
mod residual_tests {
    use super::*;
    #[test]
    fn wide_publications_echo_all_bound_hidden_inputs_for_sqlite_residual_checks() {
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        for (keys, exports) in [(6, 3), (12, 12), (32, 32)] {
            let identity = keys * 10 + exports;
            let inputs = (0..keys)
                .map(|i| format!(" AND m.k{i} IS {}", i + 1))
                .collect::<String>();
            let outputs = (0..exports)
                .map(|i| format!(" AND m.v{i} IS {}", i + 70))
                .collect::<String>();
            let sql = format!(
                "SELECT m.truth,m.peak,m.o{},m.k{},m.v{} FROM relation_memo m WHERE m.mode IS 1 AND m.clause IS 0 AND m.utility IS {identity} AND m.file IS 0 AND m.node IS 1{inputs} AND m.vtruth IS 1 AND m.vcost IS 3 AND m.vpeak IS 2{outputs}",
                exports - 1,
                keys - 1,
                exports - 1
            );
            let answer: (i64, i64, i64, i64, i64) = db
                .query_row(&sql, [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })
                .unwrap();
            assert_eq!(
                answer,
                (
                    1,
                    2,
                    (exports + 69) as i64,
                    keys as i64,
                    (exports + 69) as i64
                ),
                "{} hidden inputs",
                8 + keys + exports
            );
        }
    }
    #[test]
    fn span_requests_echo_raw_bytes_while_memo_keys_ignore_only_shadow_text() {
        use super::super::span_capture::{self, Span};
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let original = span_capture::encode(&Span {
            file: 0,
            node: Some(7),
            text: Some("original".into()),
            range: 4..9,
        })
        .unwrap();
        let changed = span_capture::encode(&Span {
            file: 0,
            node: Some(7),
            text: Some("shadow changed".into()),
            range: 4..9,
        })
        .unwrap();
        let keys = (0..6)
            .map(|i| format!(" AND m.k{i} IS ?1"))
            .collect::<String>();
        let outputs = (0..3)
            .map(|i| format!(" AND m.v{i} IS ?1"))
            .collect::<String>();
        let sql = format!(
            "SELECT m.o2,m.v2 FROM relation_memo m WHERE m.mode IS 1 AND m.clause IS 0 AND m.utility IS 90 AND m.file IS 0 AND m.node IS 1{keys} AND m.vtruth IS 1 AND m.vcost IS 2 AND m.vpeak IS 1{outputs}"
        );
        let (stored, echo): (Vec<u8>, Vec<u8>) = db
            .query_row(&sql, [&original], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(stored, original);
        assert_eq!(echo, original);
        let lookup = format!(
            "SELECT m.o2,m.k5 FROM relation_memo m WHERE m.mode IS 0 AND m.clause IS 0 AND m.utility IS 90 AND m.file IS 0 AND m.node IS 1{keys}"
        );
        let (stored, echo): (Vec<u8>, Vec<u8>) = db
            .query_row(&lookup, [&changed], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(stored, original);
        assert_eq!(echo, changed);
    }
}
