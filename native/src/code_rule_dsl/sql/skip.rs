//! Which statements a batch passes over. The emitter records, with each statement, the frames it
//! reads, the table it writes and its emptiness rule (`emit::Effect`); the session counts every
//! counted table's rows for the batch (`Counts`) and reads the rule over them (`outcome`) before
//! it runs the statement. A statement the rule proves empty writes nothing, so passing over it
//! leaves its output as empty as running it would, and the skip carries on to what reads it.
//!
//! The counts are exact. A frame's rows are its buffers' length (`frames::FrameStore::rows`):
//! every insert and clear goes through them. A recursion table (`rec`, one some statement
//! updates or deletes from by a condition) or a B-tree (an enumeration's) is written only by
//! statements that record how (`emit::Writes`): an `INSERT` adds the rows `sqlite3_changes()`
//! reports, a `DELETE` with a condition removes them, emptying it sets 0, and an `UPDATE`
//! changes no count. Tables the session does not count (`node`, the terminals, `runs`,
//! `roots`) are never a rule's input: a statement that reads only them always runs.
//!
//! Beyond single statements (`exec::Runner`):
//! - **Clause and stage runs.** Every other statement of a clause or stage reads, through its
//!   operators, the rows of its root frame (`emit::Emitted::root`): where the root frame is empty
//!   for the batch (no root of its kinds in its files, or the RequiredText gate and the anchors
//!   settled every document), the statements after it and the last are passed over whole.
//! - **Recursion blocks** (`emit::BLOCK`/`RUN`): a block's run ends with no dirty key, and only a
//!   new demand row is dirty outside it, so a run that finds its units' demand rows as many as
//!   the block's last run left them (or none) has nothing to evaluate, and is passed over.
//! - **`violation.each` rounds**: a round runs only while roots are still enumerated (`a`).
//!
//! `RERAIL_SQL_SKIP`: unset, passes over what the rules prove empty; `check`, runs everything and
//! panics where a statement the rules would pass over changed a row, or where one the rules say
//! is exactly its input (`CopyOf`, an anti-join whose probes are empty) changed another number;
//! `0`, runs everything (the counts are kept, unread).

use super::emit::{Rule, Writes};
use super::frames::FrameStore;
use std::sync::{Arc, LazyLock};

/// What `RERAIL_SQL_SKIP` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Skip,
    Check,
    Run,
}

pub(super) static MODE: LazyLock<Mode> =
    LazyLock::new(|| match std::env::var("RERAIL_SQL_SKIP").as_deref() {
        Ok("0") => Mode::Run,
        Ok("check") => Mode::Check,
        _ => Mode::Skip,
    });

/// The rows of every counted table as the batch's statements left them, and each recursion
/// block's mark: its units' demand rows when its last run ended.
pub(super) struct Counts {
    /// A recursion table's or a B-tree's rows, by counted table (a frame's entry is unused).
    rows: Vec<i64>,
    /// Each counted table's name, and whether it is a frame of `store`.
    tables: Vec<(String, bool)>,
    store: Arc<FrameStore>,
    pub(super) marks: Vec<i64>,
}

impl Counts {
    /// The counts of `tables` (each a name, and whether it is a frame of `store`), all empty, and
    /// `marks` blocks, none run.
    pub(super) fn new(store: Arc<FrameStore>, tables: Vec<(String, bool)>, marks: usize) -> Self {
        Self {
            rows: vec![0; tables.len()],
            tables,
            store,
            marks: vec![0; marks],
        }
    }

    /// The rows `table` holds.
    pub(super) fn rows(&self, table: usize) -> i64 {
        match &self.tables[table] {
            (name, true) => self.store.rows(name).expect("a registered frame") as i64,
            (_, false) => self.rows[table],
        }
    }

    /// The name of `table`.
    pub(super) fn name(&self, table: usize) -> &str {
        &self.tables[table].0
    }

    /// Accounts a statement that changed `changed` rows (a frame counts its own).
    pub(super) fn wrote(&mut self, writes: &Writes<usize>, changed: usize) {
        match *writes {
            Writes::None => {}
            Writes::Insert(table) => self.rows[table] += changed as i64,
            Writes::Delete(table) => self.rows[table] -= changed as i64,
            Writes::Clear(table) => self.rows[table] = 0,
        }
    }

    /// Every recursion table and B-tree empty and every block unrun: a batch that failed leaves
    /// no row.
    pub(super) fn reset(&mut self) {
        self.rows.fill(0);
        self.marks.fill(0);
    }

    /// The rows of `tables`, summed.
    pub(super) fn sum(&self, tables: impl IntoIterator<Item = usize>) -> i64 {
        tables.into_iter().map(|table| self.rows(table)).sum()
    }
}

/// What a statement's rule says it will change, over the counted rows of its inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    /// Nothing the rule decides: it runs.
    Unknown,
    /// No row: it is passed over.
    Empty,
    /// Exactly this many rows (more than 0): it runs.
    Exactly(i64),
}

fn exactly(rows: i64) -> Outcome {
    if rows == 0 {
        Outcome::Empty
    } else {
        Outcome::Exactly(rows)
    }
}

/// The dispatch from a statement's rule over its `inputs` (`emit::Effect`) to what it will
/// change, over the rows each counted table holds (`rows`).
pub(super) fn outcome<T: Copy>(rule: Rule, inputs: &[T], rows: impl Fn(T) -> i64) -> Outcome {
    let empty = |table: &T| rows(*table) == 0;
    match rule {
        // A root frame, a seed, a marker: reads only what the session does not count.
        Rule::AlwaysRun => Outcome::Unknown,
        // Joins, semi-joins (`IN`, `EXISTS`), relations, terminals over an input set, and frames
        // derived from their inputs: each output row stems from a row of every input.
        Rule::EmptyIfAnyInputEmpty => {
            if inputs.iter().any(empty) {
                Outcome::Empty
            } else {
                Outcome::Unknown
            }
        }
        // A union whose branches each read one input: some branch has rows where any input does.
        Rule::EmptyIfAllInputsEmpty => {
            if !inputs.is_empty() && inputs.iter().all(empty) {
                Outcome::Empty
            } else {
                Outcome::Unknown
            }
        }
        // An anti-join (`not`, `any`'s remainder, `every`, an exclusion): the primary's rows the
        // probes do not remove. An empty primary leaves nothing; empty probes remove nothing, so
        // the output is the whole primary, and the statement still runs to copy it.
        Rule::EmptyIfPrimaryEmpty => {
            let (primary, probes) = inputs.split_first().expect("a primary input");
            if empty(primary) {
                Outcome::Empty
            } else if probes.iter().all(empty) {
                // Mutant 16: empty probes taken for an empty output.
                if super::mutant(16) {
                    Outcome::Empty
                } else {
                    Outcome::Exactly(rows(*primary))
                }
            } else {
                Outcome::Unknown
            }
        }
        // The input's rows, each once.
        Rule::CopyOf => exactly(rows(inputs[0])),
    }
}

/// `RERAIL_SQL_SKIP=check`: panics where `sql`, whose rule said `outcome`, changed (or returned)
/// `changed` rows.
pub(super) fn check(sql: &str, outcome: Outcome, changed: usize) {
    let expected = match outcome {
        Outcome::Unknown => return,
        Outcome::Empty => 0,
        Outcome::Exactly(rows) => rows,
    };
    assert!(
        changed as i64 == expected,
        "RERAIL_SQL_SKIP=check: {changed} rows where the skip rules expect {expected}: {sql}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_rule_reads_its_inputs_rows() {
        let rows = [0, 3, 5];
        let of = |rule, inputs: &[usize]| outcome(rule, inputs, |table| rows[table]);
        assert_eq!(of(Rule::AlwaysRun, &[0]), Outcome::Unknown);
        assert_eq!(of(Rule::EmptyIfAnyInputEmpty, &[1, 0]), Outcome::Empty);
        assert_eq!(of(Rule::EmptyIfAnyInputEmpty, &[1, 2]), Outcome::Unknown);
        assert_eq!(of(Rule::EmptyIfAllInputsEmpty, &[0, 0]), Outcome::Empty);
        assert_eq!(of(Rule::EmptyIfAllInputsEmpty, &[0, 1]), Outcome::Unknown);
        assert_eq!(of(Rule::EmptyIfPrimaryEmpty, &[0, 1]), Outcome::Empty);
        assert_eq!(of(Rule::EmptyIfPrimaryEmpty, &[1, 0]), Outcome::Exactly(3));
        assert_eq!(of(Rule::EmptyIfPrimaryEmpty, &[1, 2]), Outcome::Unknown);
        assert_eq!(of(Rule::CopyOf, &[0]), Outcome::Empty);
        assert_eq!(of(Rule::CopyOf, &[2]), Outcome::Exactly(5));
    }

    #[test]
    fn counts_follow_what_statements_wrote() {
        let tables = vec![("a".to_owned(), false), ("b".to_owned(), false)];
        let mut counts = Counts::new(Arc::default(), tables, 1);
        counts.wrote(&Writes::Insert(0), 4);
        counts.wrote(&Writes::Delete(0), 1);
        counts.wrote(&Writes::Insert(1), 2);
        counts.wrote(&Writes::None, 9);
        assert_eq!((counts.rows(0), counts.rows(1)), (3, 2));
        counts.wrote(&Writes::Clear(1), 2);
        assert_eq!(counts.sum([0, 1]), 3);
        counts.reset();
        assert_eq!(counts.sum([0, 1]), 0);
    }

    #[test]
    #[should_panic(expected = "RERAIL_SQL_SKIP=check")]
    fn check_panics_where_a_skipped_statement_wrote() {
        check("INSERT …", Outcome::Empty, 1);
    }
}
