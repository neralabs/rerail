//! What a rule's part reads (`change::body` reads one table type): a relation of the level, a view over its
//! documents' tapes, or a fixpoint's rounds of a head. Each is read where it lies.

use crate::tape::TapeView;
use rerail_blocks::bistate::{
    ColumnIndex, ColumnRead, Read, RowId, Since, ViewColumn, WordIndex, below,
};
use rerail_blocks::relation::{Relation, View};
use rerail_blocks::table::{Access, Run, Runs, Table};

/// A part's input at one state.
pub enum Src<'a> {
    Rel(View<'a>),
    Tape(TapeView<'a>),
    Rounds(Rounds<'a>),
}

/// A recursive head's rows as a fixpoint finds them: its rounds, each a frozen relation of the head's spec, read as
/// one table whose ids run through the rounds in order. A round's rows are new: none is in an earlier round.
pub struct Rounds<'a> {
    rels: Vec<&'a Relation>,
    starts: Vec<u32>,
    arity: usize,
    end: u32,
    /// The end of the state before (`since`): the rounds before the last.
    since: Option<u32>,
}

impl<'a> Rounds<'a> {
    /// Rounds `rels`, the last of them new since the state before when `last_new`.
    pub fn new(rels: &[&'a Relation], arity: usize, last_new: bool) -> Rounds<'a> {
        let mut starts = Vec::with_capacity(rels.len());
        let mut end = 0u32;
        for r in rels {
            starts.push(end);
            end += r.rows.len();
        }
        let since = last_new.then(|| starts.last().copied().unwrap_or(0));
        Rounds {
            rels: rels.to_vec(),
            starts,
            arity,
            end,
            since,
        }
    }

    fn locate(&self, id: u32) -> (&'a Relation, u32) {
        let i = self.starts.partition_point(|&s| s <= id) - 1;
        (self.rels[i], id - self.starts[i])
    }
}

/// A part's column.
pub enum SrcColumn<'s, 'a> {
    Rel(ViewColumn),
    Other(&'s Src<'a>, ColumnIndex),
}

impl ColumnRead for SrcColumn<'_, '_> {
    fn get(&self, id: RowId) -> i64 {
        match self {
            SrcColumn::Rel(c) => c.get(id),
            SrcColumn::Other(s, c) => s.get(id, *c),
        }
    }
}

impl<'a> Read for Src<'a> {
    type Column<'s>
        = SrcColumn<'s, 'a>
    where
        Self: 's;

    fn arity(&self) -> usize {
        match self {
            Src::Rel(v) => Read::arity(v),
            Src::Tape(t) => t.arity(),
            Src::Rounds(r) => r.arity,
        }
    }

    fn end(&self) -> RowId {
        match self {
            Src::Rel(v) => Read::end(v),
            Src::Tape(t) => t.end(),
            Src::Rounds(r) => RowId::new(r.end),
        }
    }

    fn live_word(&self, w: WordIndex) -> u64 {
        match self {
            Src::Rel(v) => v.live_word(w),
            Src::Tape(t) => t.live_word(w),
            Src::Rounds(r) => below(RowId::new(r.end), w),
        }
    }

    fn get(&self, id: RowId, c: ColumnIndex) -> i64 {
        match self {
            Src::Rel(v) => Read::get(v, id, c),
            Src::Tape(t) => t.get(id, c),
            Src::Rounds(r) => {
                let (rel, i) = r.locate(id.get());
                rel.get(i, c.get())
            }
        }
    }

    fn column(&self, c: ColumnIndex) -> SrcColumn<'_, 'a> {
        match self {
            Src::Rel(v) => SrcColumn::Rel(v.column(c)),
            _ => SrcColumn::Other(self, c),
        }
    }

    fn since(&self) -> Option<Since<'_>> {
        match self {
            Src::Rel(v) => v.since(),
            Src::Tape(t) => t.since(),
            Src::Rounds(r) => r.since.map(|end| Since {
                end: RowId::new(end),
                flips: &[],
            }),
        }
    }
}

impl Table for Src<'_> {
    fn sort(&self) -> Option<ColumnIndex> {
        match self {
            Src::Rel(v) => v.sort(),
            _ => None,
        }
    }

    fn runs(&self) -> Runs {
        match self {
            Src::Rel(v) => v.runs(),
            Src::Tape(t) => t.runs(),
            Src::Rounds(r) => Runs::one(Run::new(0, r.end)),
        }
    }

    fn equal_range(&self, run: Run, key: i64) -> Run {
        match self {
            Src::Rel(v) => v.equal_range(run, key),
            _ => unreachable!("an equal range of an unsorted table"),
        }
    }

    fn lower_bound(&self, run: Run, key: i64) -> RowId {
        match self {
            Src::Rel(v) => v.lower_bound(run, key),
            _ => unreachable!("a bound of an unsorted table"),
        }
    }

    fn access(&self, cols: &[ColumnIndex]) -> Access {
        match self {
            Src::Rel(v) => v.access(cols),
            _ => Access::Scan,
        }
    }

    fn probe(
        &self,
        cols: &[ColumnIndex],
        key: &[i64],
        each: &mut dyn FnMut(RowId) -> bool,
    ) -> bool {
        match self {
            Src::Rel(v) => Table::probe(v, cols, key, each),
            Src::Tape(t) => t.probe(cols, key, each),
            Src::Rounds(r) => {
                for (rel, &start) in r.rels.iter().zip(&r.starts) {
                    if !Table::probe(&View::full(rel), cols, key, &mut |id| {
                        each(RowId::new(start + id.get()))
                    }) {
                        return false;
                    }
                }
                true
            }
        }
    }
}
