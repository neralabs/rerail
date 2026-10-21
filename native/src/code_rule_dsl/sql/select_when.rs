//! `select_when` answered in SQL over its leaves' rows: the set-based counterpart of
//! `query::select_when::answer`, which stays the parity oracle.
//!
//! # Why the plan's tree shape drops out
//!
//! The native answer reads the plan's leaves depth first, `exists` before `select`, which is
//! `Plan::leaves` order. A leaf's verdict is *holds* (a decision leaf that found a witness),
//! *empty* (no witness and no failed row it could not do without) or *unknown* (at a failed row).
//! Followed up the tree, a verdict that holds always moves on to the next leaf in that order (the
//! `select` side of the nearest `select_when` whose `exists` side it ended), or ends the answer
//! at the root; an empty or unknown verdict reaches the root unchanged, skipping every `select`
//! side above it. So the leaves read are exactly those up to and including the first that does
//! not hold (the *stop* leaf), and the answer is unknown exactly where the stop leaf is.
//!
//! # How a caller materialises rows
//!
//! Each leaf's rows go in as `(leaf, position, failed, partial, witnesses)`, positions from 0 in
//! the order `Leaves::row` gives them. The statement computes where native reading ends each leaf
//! itself (a decision leaf at its first row holding a witness, any other leaf at its first failed
//! row) and which leaves are read at all (up to the stop leaf), so rows past those ends, and the
//! rows of leaves past the stop leaf, may be given or left out: they are ignored. What must be
//! given is every row native reading reads: each read leaf's rows up to its end, or all of them
//! where it does not end early. `answer_leaves` materialises exactly those through a `Leaves`
//! implementation, making the same `row` calls in the same order as the native answer, so a lazy
//! implementation evaluates nothing more than it does natively and an error it returns ends the
//! answer at the same call.
//!
//! The statement is one `SELECT` of window functions, aggregates and joins over the two tables;
//! it returns the unknown row, or the partial rows and the witness triples in order, with the
//! witness budget applied by a running sum.

use super::SqlError;
use super::exec::AuxiliaryConnection;
use crate::code_rule_dsl::query::select_when::{Answer, LeafRow, Leaves, Plan, Unknown};
use rusqlite::Connection;
use std::cell::RefCell;

const SCHEMA: &str = "
    CREATE TABLE sw_leaf(
        leaf INTEGER PRIMARY KEY,
        decision INTEGER NOT NULL,
        reported INTEGER NOT NULL
    );
    CREATE TABLE sw_row(
        leaf INTEGER NOT NULL,
        position INTEGER NOT NULL,
        failed INTEGER NOT NULL,
        partial INTEGER NOT NULL,
        witnesses INTEGER NOT NULL,
        PRIMARY KEY (leaf, position)
    ) WITHOUT ROWID;
";

/// The answer over the loaded leaves: rows tagged 0 (the unknown `(leaf, position)`), 1 (a
/// partial row the answer relied on) and 2 (a witness `(leaf, position, take)`), in order.
///
/// - `summary`: per leaf, its first row holding a witness (`hit`) and its first failed row.
/// - `verdict`: where reading ends it (`end_at`, NULL for every row), whether it holds, and the
///   failed row it is unknown at.
/// - `stop`: the first leaf that does not hold; `read`: the leaves up to it.
/// - `taken`: a reported leaf not read for a decision, its rows' witnesses before each row
///   (`spent`).
const ANSWER: &str = "
    WITH summary AS (
        SELECT l.leaf AS leaf, l.decision AS decision, l.reported AS reported,
               MIN(CASE WHEN r.witnesses > 0 THEN r.position END) AS hit,
               MIN(CASE WHEN r.failed THEN r.position END) AS fail
        FROM sw_leaf AS l LEFT JOIN sw_row AS r ON r.leaf = l.leaf
        GROUP BY l.leaf
    ),
    verdict AS (
        SELECT leaf, decision, reported,
               CASE WHEN decision THEN hit ELSE fail END AS end_at,
               decision AND hit IS NOT NULL AS holds,
               CASE WHEN decision AND hit IS NOT NULL THEN NULL ELSE fail END AS unknown
        FROM summary
    ),
    stop AS (SELECT MIN(leaf) AS leaf FROM verdict WHERE NOT holds),
    read AS MATERIALIZED (
        SELECT verdict.* FROM verdict, stop
        WHERE stop.leaf IS NULL OR verdict.leaf <= stop.leaf
    ),
    unknown AS MATERIALIZED (
        SELECT read.leaf AS leaf, read.unknown AS position
        FROM read JOIN stop ON read.leaf = stop.leaf
        WHERE read.unknown IS NOT NULL
    ),
    known AS (SELECT COUNT(*) = 0 AS known FROM unknown),
    taken AS (
        SELECT r.leaf AS leaf, r.position AS position, r.witnesses AS witnesses,
               COALESCE(SUM(r.witnesses) OVER (
                   PARTITION BY r.leaf ORDER BY r.position
                   ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING
               ), 0) AS spent
        FROM read JOIN sw_row AS r ON r.leaf = read.leaf
        WHERE read.reported AND NOT read.decision
    )
    SELECT 0, leaf, position, 0 FROM unknown
    UNION ALL
    SELECT 1, r.leaf, r.position, 0
    FROM known, read JOIN sw_row AS r ON r.leaf = read.leaf
    WHERE known.known AND r.partial AND (read.end_at IS NULL OR r.position <= read.end_at)
    UNION ALL
    SELECT 2, read.leaf, read.end_at, 1
    FROM known, read
    WHERE known.known AND read.reported AND read.holds
    UNION ALL
    SELECT 2, leaf, position, MIN(witnesses, ?1 - spent)
    FROM known, taken
    WHERE known.known AND witnesses > 0 AND spent < ?1
    ORDER BY 1, 2, 3
";

/// Compose the existing decision expression with symbolic input relations in compiler spikes.
#[cfg(test)]
pub(super) fn expression() -> &'static str {
    ANSWER
}

thread_local! {
    /// This thread's connection, its tables created once.
    static CONNECTION: RefCell<Option<AuxiliaryConnection>> = const { RefCell::new(None) };
}

fn with_connection<T>(
    body: impl FnOnce(&Connection) -> Result<T, SqlError>,
) -> Result<T, SqlError> {
    CONNECTION.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let connection = AuxiliaryConnection::new("select-when")?;
            connection.execute_batch(SCHEMA)?;
            *slot = Some(connection);
        }
        let connection = slot.as_ref().expect("connected");
        let result = body(connection);
        connection.checkpoint()?;
        result
    })
}

fn integer(value: usize) -> Result<i64, SqlError> {
    i64::try_from(value).map_err(|_| SqlError::unsupported("select_when row index past i64"))
}

/// Answers `plan` over `rows` (indexed by leaf in `Plan::leaves` order; a leaf past the end has
/// no rows) with a witness budget of `cap`, exactly as `query::select_when::answer` answers over
/// the same rows read lazily. See the module documentation for which rows must be given.
///
/// Refuses (`Unsupported`) a leaf whose given rows hold more than `i64::MAX` witnesses in all.
pub(in crate::code_rule_dsl) fn answer<L>(
    plan: &Plan<L>,
    cap: usize,
    rows: &[Vec<LeafRow>],
) -> Result<Result<Answer, Unknown>, SqlError> {
    for leaf in rows {
        let total = leaf.iter().try_fold(0i64, |total, row| {
            total.checked_add(i64::try_from(row.witnesses).ok()?)
        });
        if total.is_none() {
            return Err(SqlError::unsupported(
                "select_when leaf with more than i64::MAX witnesses",
            ));
        }
    }
    // Exact: every running sum of a leaf's witnesses is at most `i64::MAX` (above), so a budget
    // past it never runs out, as `cap` does not.
    let cap = i64::try_from(cap).unwrap_or(i64::MAX);
    with_connection(|connection| {
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch("DELETE FROM sw_leaf; DELETE FROM sw_row;")?;
        {
            let mut insert = transaction.prepare_cached(
                "INSERT INTO sw_leaf(leaf, decision, reported) VALUES (?1, ?2, ?3)",
            )?;
            for (index, leaf) in plan.leaves.iter().enumerate() {
                insert.execute((integer(index)?, leaf.decision, leaf.reported))?;
            }
            let mut insert = transaction.prepare_cached(
                "INSERT INTO sw_row(leaf, position, failed, partial, witnesses) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (leaf, rows) in rows.iter().enumerate().take(plan.leaves.len()) {
                for (position, row) in rows.iter().enumerate() {
                    insert.execute((
                        integer(leaf)?,
                        integer(position)?,
                        row.failed,
                        row.partial,
                        integer(row.witnesses)?,
                    ))?;
                }
            }
        }
        let mut answer = Answer {
            witnesses: Vec::new(),
            partial: Vec::new(),
        };
        let mut unknown = None;
        {
            let mut statement = transaction.prepare_cached(ANSWER)?;
            let mut found = statement.query([cap])?;
            while let Some(row) = found.next()? {
                let tag: i64 = row.get(0)?;
                let leaf = row.get::<_, i64>(1)? as usize;
                let position = row.get::<_, i64>(2)? as usize;
                match tag {
                    0 => unknown = Some((leaf, position)),
                    1 => answer.partial.push((leaf, position)),
                    _ => answer
                        .witnesses
                        .push((leaf, position, row.get::<_, i64>(3)? as usize)),
                }
            }
        }
        transaction.execute_batch("DELETE FROM sw_leaf; DELETE FROM sw_row;")?;
        transaction.commit()?;
        Ok(unknown.map_or(Ok(answer), Err))
    })
}

/// Answers `plan` over `leaves` with a witness budget of `cap`, as `query::select_when::answer`
/// does: same arguments, same result. The rows are read through `leaves` exactly as the native
/// answer reads them (same calls, same order; an `Err` ends the answer with its message), then
/// answered by `answer`; a SQL error is returned as its message.
pub(in crate::code_rule_dsl) fn answer_leaves<L>(
    plan: &Plan<L>,
    cap: usize,
    leaves: &dyn Leaves,
) -> Result<Result<Answer, Unknown>, String> {
    let rows = materialise(plan, leaves)?;
    answer(plan, cap, &rows).map_err(|error| error.to_string())
}

/// Each leaf's rows as native reading reads them: leaves in order, each until `row` gives `None`,
/// a decision leaf until its first row holding a witness and any other leaf until its first
/// failed row; the next leaf is read only after a decision leaf that found its witness (any
/// other verdict reaches the root, module documentation).
fn materialise<L>(plan: &Plan<L>, leaves: &dyn Leaves) -> Result<Vec<Vec<LeafRow>>, String> {
    let mut tables = Vec::new();
    for (index, leaf) in plan.leaves.iter().enumerate() {
        let mut rows = Vec::new();
        let mut holds = false;
        while let Some(row) = leaves.row(index, rows.len())? {
            rows.push(row);
            if leaf.decision && row.witnesses > 0 {
                holds = true;
                break;
            }
            if !leaf.decision && row.failed {
                break;
            }
        }
        tables.push(rows);
        if !holds {
            break;
        }
    }
    Ok(tables)
}
