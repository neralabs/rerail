//! `select_when` as one statement.
//!
//! A `select_when` reports its `select` side's witnesses when its `exists` side has a witness
//! anywhere. Its sides are regex or AST queries (leaves), or `select_when`s themselves. A leaf is
//! read file by file as rows of `rerail_leaf`: whether the leaf failed in the file (its witnesses
//! there are unknown), whether the file was read around syntax errors, and how many witnesses the
//! file holds. The statement decides everything else: which sides are read at all, whether the
//! answer is known, which witnesses are reported (a witness budget's worth, in row order) and
//! which partially read files the answer relied on.
//!
//! Rows come from the caller (`Leaves`), in the order the caller reads the leaf. A leaf read for a
//! decision (an `exists` side, or the whole query in decision mode) ends with its first row that
//! holds a witness, as its evaluation stops there. The statement reads a leaf only where the
//! answer depends on it, and every row at most once per read, so a caller that evaluates a leaf as
//! its rows are asked for does no work the answer does not need.

use crate::policy_dsl::model::Query;
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
    VTabCursor,
};
use rusqlite::{Connection, Result as SqlResult, ffi};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;

/// One file of a leaf.
#[derive(Clone, Copy, Debug)]
pub(in crate::policy_dsl) struct LeafRow {
    /// The leaf could not answer in this file: its witnesses there are unknown.
    pub(in crate::policy_dsl) failed: bool,
    /// The file was read around syntax errors; reported `source-partial` if the answer uses it.
    pub(in crate::policy_dsl) partial: bool,
    pub(in crate::policy_dsl) witnesses: usize,
}

/// The rows of a plan's leaves.
pub(in crate::policy_dsl) trait Leaves {
    /// Row `position` of leaf `leaf` (in `Plan::leaves` order), or `None` past its last row.
    /// `Err` ends the statement with that message.
    fn row(&self, leaf: usize, position: usize) -> Result<Option<LeafRow>, String>;
}

/// A leaf of a plan: the query it reads, whether it is read for a decision, and whether its
/// witnesses may be reported (no `exists` side above it) or only decide a condition.
pub(in crate::policy_dsl) struct Leaf<L> {
    pub(in crate::policy_dsl) id: L,
    pub(in crate::policy_dsl) decision: bool,
    pub(in crate::policy_dsl) reported: bool,
}

/// A query's statement and its leaves.
pub(in crate::policy_dsl) struct Plan<L> {
    pub(in crate::policy_dsl) leaves: Vec<Leaf<L>>,
    sql: String,
}

/// What a plan's statement answered, where known: the witnesses reported, as `(leaf, row, how
/// many of the row's witnesses)` in order, and the partially read rows it relied on.
pub(in crate::policy_dsl) struct Answer {
    pub(in crate::policy_dsl) witnesses: Vec<(usize, usize, usize)>,
    pub(in crate::policy_dsl) partial: Vec<(usize, usize)>,
}

impl<L> Plan<L> {
    /// `query`'s statement; `decision` when a single witness answers it. `leaf` names each
    /// regex or AST query, given whether its witnesses may be reported.
    pub(in crate::policy_dsl) fn new<'q>(
        query: &'q Query,
        decision: bool,
        leaf: impl FnMut(&'q Query, bool) -> L,
    ) -> Self {
        let mut builder = Builder {
            leaves: Vec::new(),
            nodes: 0,
            ctes: Vec::new(),
            partial: Vec::new(),
            leaf,
        };
        let witnesses = builder
            .node(query, decision, true, "(SELECT known FROM n0)")
            .expect("the root query is reported");
        let sql = format!(
            "WITH {} SELECT 0, NULL, NULL, NULL WHERE NOT (SELECT known FROM n0) \
             UNION ALL SELECT 1, leaf, position, take FROM ({witnesses}) \
             UNION ALL SELECT 2, leaf, position, NULL FROM ({}) ORDER BY 1, 2, 3",
            builder.ctes.join(", "),
            builder.partial.join(" UNION ALL "),
        );
        Self {
            leaves: builder.leaves,
            sql,
        }
    }
}

struct Builder<L, F> {
    leaves: Vec<Leaf<L>>,
    nodes: usize,
    /// One materialized row per node, `n<i>(known, nonempty)`: whether the node's answer is
    /// known, and (read for a decision) whether it holds a witness. Children come first.
    ctes: Vec<String>,
    /// The partially read rows of every leaf, each read only where its gate holds.
    partial: Vec<String>,
    leaf: F,
}

impl<'q, L, F: FnMut(&'q Query, bool) -> L> Builder<L, F> {
    /// Adds `query` as node `n<next>`. `gate` holds where the answer reads this node's rows: the
    /// whole answer is known and every `exists` above has a witness. A gated read names its leaf
    /// as `CASE WHEN gate THEN leaf END`, the value `rerail_leaf` is filtered on, so a read whose
    /// gate fails is an empty table (SQLite does not hoist a subquery out of the scan as a
    /// constant condition). Returns the node's reported witnesses, where it is reported.
    fn node(
        &mut self,
        query: &'q Query,
        decision: bool,
        reported: bool,
        gate: &str,
    ) -> Option<String> {
        let node = self.nodes;
        self.nodes += 1;
        let (row, witnesses) = match query {
            Query::SelectWhen { select, exists } => {
                let condition = self.nodes;
                self.node(exists, true, false, gate);
                let selected = self.nodes;
                let witnesses = self.node(
                    select,
                    decision,
                    reported,
                    &format!("{gate} AND (SELECT known AND nonempty FROM n{condition})"),
                );
                (
                    format!(
                        "SELECT CASE WHEN NOT e.known THEN 0 WHEN NOT e.nonempty THEN 1 \
                         ELSE (SELECT known FROM n{selected}) END, CASE WHEN e.known AND \
                         e.nonempty THEN (SELECT nonempty FROM n{selected}) ELSE 0 END \
                         FROM n{condition} AS e"
                    ),
                    witnesses,
                )
            }
            Query::Regex { .. } | Query::Ast { .. } => {
                let leaf = self.leaves.len();
                let gated = format!("CASE WHEN {gate} THEN {leaf} END");
                let id = (self.leaf)(query, reported);
                self.leaves.push(Leaf {
                    id,
                    decision,
                    reported,
                });
                self.partial.push(format!(
                    "SELECT {leaf} AS leaf, position FROM rerail_leaf \
                     WHERE leaf = {gated} AND partial"
                ));
                let failed =
                    format!("EXISTS (SELECT 1 FROM rerail_leaf WHERE leaf = {leaf} AND failed)");
                if decision {
                    let first = format!(
                        "(SELECT position FROM rerail_leaf WHERE leaf = {leaf} AND witnesses > 0 \
                         ORDER BY position LIMIT 1)"
                    );
                    (
                        format!(
                            "SELECT CASE WHEN f.first IS NOT NULL THEN 1 WHEN {failed} THEN 0 \
                             ELSE 1 END, f.first IS NOT NULL FROM (SELECT {first} AS first) AS f"
                        ),
                        reported.then(|| {
                            format!(
                                "SELECT {leaf} AS leaf, position, 1 AS take FROM rerail_leaf \
                                 WHERE leaf = {gated} AND witnesses > 0 \
                                 ORDER BY position LIMIT 1"
                            )
                        }),
                    )
                } else {
                    // The witness budget (`?1`) counts rows' witnesses in order; the row where
                    // it runs out gives only what is left of it.
                    (
                        format!("SELECT NOT {failed}, 0"),
                        reported.then(|| {
                            format!(
                                "SELECT {leaf} AS leaf, position, MIN(witnesses, ?1 - before) \
                                 AS take FROM (SELECT position, witnesses, COALESCE(SUM(witnesses) \
                                 OVER (ORDER BY position ROWS BETWEEN UNBOUNDED PRECEDING AND 1 \
                                 PRECEDING), 0) AS before FROM rerail_leaf WHERE leaf = {gated} \
                                 AND NOT failed) WHERE witnesses > 0 AND before < ?1"
                            )
                        }),
                    )
                }
            }
        };
        self.ctes
            .push(format!("n{node}(known, nonempty) AS MATERIALIZED ({row})"));
        witnesses
    }
}

/// Runs `plan`'s statement over `leaves`' rows with a witness budget of `cap`: `None` where the
/// answer is unknown.
pub(in crate::policy_dsl) fn answer<L>(
    plan: &Plan<L>,
    cap: usize,
    leaves: &dyn Leaves,
) -> Result<Option<Answer>, String> {
    CONNECTION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let connection = match &mut *slot {
            Some(connection) => connection,
            empty => empty.insert(open().map_err(|error| error.to_string())?),
        };
        let _reading = Reading::start(leaves);
        let run = || -> SqlResult<Option<Answer>> {
            let mut statement = connection.prepare_cached(&plan.sql)?;
            // Only a reported leaf not read for a decision counts the witness budget.
            let budget = i64::try_from(cap).unwrap_or(i64::MAX);
            let mut rows = if statement.parameter_count() == 0 {
                statement.query([])?
            } else {
                statement.query([budget])?
            };
            let mut answer = Answer {
                witnesses: Vec::new(),
                partial: Vec::new(),
            };
            while let Some(row) = rows.next()? {
                let index = |column| row.get::<_, i64>(column).map(|value| value as usize);
                match row.get::<_, i64>(0)? {
                    0 => return Ok(None),
                    1 => answer.witnesses.push((index(1)?, index(2)?, index(3)?)),
                    _ => answer.partial.push((index(1)?, index(2)?)),
                }
            }
            Ok(Some(answer))
        };
        run().map_err(|error| match error {
            rusqlite::Error::SqliteFailure(_, Some(message)) => message,
            error => error.to_string(),
        })
    })
}

thread_local! {
    /// The connection plans run on: nothing but `rerail_leaf` and cached statements.
    static CONNECTION: RefCell<Option<Connection>> = const { RefCell::new(None) };
    /// The rows `rerail_leaf` reads while a statement runs (`Reading`).
    static LEAVES: Cell<Option<*const (dyn Leaves + 'static)>> = const { Cell::new(None) };
}

fn open() -> SqlResult<Connection> {
    const LEAF: Module<LeafTable> = Module::eponymous_only_module();
    let connection = Connection::open_in_memory()?;
    connection.set_prepared_statement_cache_capacity(64);
    connection.create_module(c"rerail_leaf", &LEAF, None)?;
    Ok(connection)
}

/// Lends `leaves` to `rerail_leaf` for as long as it lives: the statement runs, and is reset
/// (its cursors closed), within that time.
struct Reading(Option<*const (dyn Leaves + 'static)>);

impl Reading {
    fn start(leaves: &dyn Leaves) -> Self {
        // SAFETY: the lifetime is erased only for the thread-local slot; `Drop` restores the
        // previous value before `leaves` can go away, and `answer` finishes its statement first.
        let erased: *const (dyn Leaves + 'static) = unsafe { std::mem::transmute(leaves) };
        Self(LEAVES.replace(Some(erased)))
    }
}

impl Drop for Reading {
    fn drop(&mut self) {
        LEAVES.set(self.0);
    }
}

fn module_error(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::ModuleError(message.into())
}

const LEAF_COLUMN: c_int = 0;
const POSITION_COLUMN: c_int = 1;

#[repr(C)]
struct LeafTable {
    base: ffi::sqlite3_vtab,
}

unsafe impl<'vtab> VTab<'vtab> for LeafTable {
    type Aux = ();
    type Cursor = LeafCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        _aux: Option<&()>,
        _module_name: &[u8],
        _database_name: &[u8],
        _table_name: &[u8],
        _args: &[&[u8]],
    ) -> SqlResult<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(leaf HIDDEN,position,failed,partial,witnesses)"),
            Self {
                base: ffi::sqlite3_vtab::default(),
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> SqlResult<bool> {
        let leaf = info.constraints().position(|constraint| {
            constraint.is_usable()
                && constraint.column() == LEAF_COLUMN
                && constraint.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
        });
        let Some(leaf) = leaf else {
            info.set_idx_num(0);
            info.set_estimated_cost(f64::MAX);
            return Ok(true);
        };
        let mut usage = info.constraint_usage(leaf);
        usage.set_argv_index(1);
        usage.set_omit(true);
        info.set_idx_num(1);
        info.set_estimated_cost(100.0);
        // Rows come in position order.
        let ordered = info
            .order_bys()
            .all(|order| order.column() == POSITION_COLUMN && !order.is_order_by_desc());
        info.set_order_by_consumed(ordered);
        Ok(true)
    }

    fn open(&mut self) -> SqlResult<Self::Cursor> {
        Ok(LeafCursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            leaf: 0,
            position: 0,
            row: None,
            phantom: PhantomData,
        })
    }
}

#[repr(C)]
struct LeafCursor<'vtab> {
    base: ffi::sqlite3_vtab_cursor,
    leaf: usize,
    position: usize,
    row: Option<LeafRow>,
    phantom: PhantomData<&'vtab LeafTable>,
}

impl LeafCursor<'_> {
    fn read(&mut self) -> SqlResult<()> {
        let leaves = LEAVES
            .get()
            .ok_or_else(|| module_error("rerail_leaf is read outside a plan"))?;
        // SAFETY: `Reading` keeps the pointer valid while any statement of this thread runs.
        self.row = unsafe { &*leaves }
            .row(self.leaf, self.position)
            .map_err(module_error)?;
        Ok(())
    }
}

unsafe impl VTabCursor for LeafCursor<'_> {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> SqlResult<()> {
        if idx_num == 0 {
            return Err(module_error("rerail_leaf requires a leaf"));
        }
        self.position = 0;
        // A gated read whose gate failed names no leaf: nothing is read.
        let Some(leaf) = args.get::<Option<i64>>(0)? else {
            self.row = None;
            return Ok(());
        };
        self.leaf =
            usize::try_from(leaf).map_err(|_| module_error("rerail_leaf leaf is negative"))?;
        self.read()
    }

    fn next(&mut self) -> SqlResult<()> {
        self.position += 1;
        self.read()
    }

    fn eof(&self) -> bool {
        self.row.is_none()
    }

    fn column(&self, context: &mut Context, column: c_int) -> SqlResult<()> {
        let row = self.row.expect("a row before eof");
        match column {
            LEAF_COLUMN => context.set_result(&(self.leaf as i64)),
            POSITION_COLUMN => context.set_result(&(self.position as i64)),
            2 => context.set_result(&row.failed),
            3 => context.set_result(&row.partial),
            4 => context.set_result(&(row.witnesses as i64)),
            _ => Err(rusqlite::Error::InvalidColumnIndex(column as usize)),
        }
    }

    fn rowid(&self) -> SqlResult<i64> {
        Ok(self.position as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy_dsl::model::RegexSource;
    use regex::Regex;

    fn leaf() -> Query {
        Query::Regex {
            regex: Regex::new("x").unwrap(),
            source: RegexSource::Content,
        }
    }

    fn when(exists: Query, select: Query) -> Query {
        Query::SelectWhen {
            select: Box::new(select),
            exists: Box::new(exists),
        }
    }

    fn row(witnesses: usize) -> LeafRow {
        LeafRow {
            failed: false,
            partial: false,
            witnesses,
        }
    }

    const FAILED: LeafRow = LeafRow {
        failed: true,
        partial: false,
        witnesses: 0,
    };

    const PARTIAL: LeafRow = LeafRow {
        failed: false,
        partial: true,
        witnesses: 0,
    };

    /// Fixed rows per leaf (in plan order), recording how far each leaf was read.
    struct Fixed {
        rows: Vec<Vec<LeafRow>>,
        read: RefCell<Vec<usize>>,
    }

    impl Fixed {
        fn new(rows: Vec<Vec<LeafRow>>) -> Self {
            let read = RefCell::new(vec![0; rows.len()]);
            Self { rows, read }
        }
    }

    impl Leaves for Fixed {
        fn row(&self, leaf: usize, position: usize) -> Result<Option<LeafRow>, String> {
            let read = &mut self.read.borrow_mut()[leaf];
            *read = (*read).max(position + 1).min(self.rows[leaf].len());
            Ok(self.rows[leaf].get(position).copied())
        }
    }

    fn run(query: &Query, decision: bool, cap: usize, leaves: &Fixed) -> Option<Answer> {
        let plan = Plan::new(query, decision, |_, reported| reported);
        answer(&plan, cap, leaves).unwrap()
    }

    #[test]
    fn a_condition_without_a_witness_leaves_the_select_side_unread() {
        let leaves = Fixed::new(vec![vec![PARTIAL, row(0)], vec![row(3)]]);
        let answer = run(&when(leaf(), leaf()), false, 10, &leaves).expect("known");
        assert!(answer.witnesses.is_empty());
        assert_eq!(answer.partial, [(0, 0)]);
        assert_eq!(*leaves.read.borrow(), [2, 0]);
    }

    #[test]
    fn an_unknown_condition_is_the_answer_and_leaves_the_select_side_unread() {
        let leaves = Fixed::new(vec![vec![PARTIAL, FAILED], vec![row(3)]]);
        assert!(run(&when(leaf(), leaf()), false, 10, &leaves).is_none());
        assert_eq!(leaves.read.borrow()[1], 0);
    }

    #[test]
    fn a_witness_decides_a_condition_despite_failed_files_before_it() {
        // Read for a decision, the condition's rows end at its witness.
        let leaves = Fixed::new(vec![vec![FAILED, PARTIAL, row(1)], vec![row(2), PARTIAL]]);
        let answer = run(&when(leaf(), leaf()), false, 10, &leaves).expect("known");
        assert_eq!(answer.witnesses, [(1, 0, 2)]);
        assert_eq!(answer.partial, [(0, 1), (1, 1)]);
    }

    #[test]
    fn a_failed_select_file_makes_the_answer_unknown() {
        let leaves = Fixed::new(vec![vec![row(1)], vec![row(2), FAILED, row(1)]]);
        assert!(run(&when(leaf(), leaf()), false, 10, &leaves).is_none());
    }

    #[test]
    fn the_witness_budget_takes_rows_in_order() {
        let leaves = Fixed::new(vec![vec![row(2), row(0), row(3), row(5)]]);
        let answer = run(&leaf(), false, 4, &leaves).expect("known");
        assert_eq!(answer.witnesses, [(0, 0, 2), (0, 2, 2)]);
    }

    #[test]
    fn a_decision_reports_one_witness() {
        let leaves = Fixed::new(vec![vec![row(1)], vec![PARTIAL, row(3)]]);
        let answer = run(&when(leaf(), leaf()), true, 10, &leaves).expect("known");
        assert_eq!(answer.witnesses, [(1, 1, 1)]);
    }

    #[test]
    fn nested_conditions_gate_in_order() {
        // exists: (select_when exists: leaf 0, select: leaf 1); select: leaf 2.
        let query = when(when(leaf(), leaf()), leaf());
        // Leaf 0 holds a witness, leaf 1 none: the outer condition is empty, leaf 2 unread.
        let leaves = Fixed::new(vec![vec![row(1)], vec![PARTIAL], vec![row(4)]]);
        let answer = run(&query, false, 10, &leaves).expect("known");
        assert!(answer.witnesses.is_empty());
        assert_eq!(answer.partial, [(1, 0)]);
        assert_eq!(leaves.read.borrow()[2], 0);
        // Leaf 1 holds one too: leaf 2 is reported.
        let leaves = Fixed::new(vec![vec![row(1)], vec![row(1)], vec![row(4)]]);
        let answer = run(&query, false, 10, &leaves).expect("known");
        assert_eq!(answer.witnesses, [(2, 0, 4)]);
    }
}
