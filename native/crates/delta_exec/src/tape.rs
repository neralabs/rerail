//! A document's tape views, read where the tape lies (decision 0353: a view is a read of the tape, not a relation;
//! nothing makes its rows). Each view's first column is the document's id, the partition column (node ids are the
//! document's own, L9):
//!
//! - `node(d, n, k, start, end)`: node `n`, its kind (a symbol) and its source span, as the engine's `node` is;
//! - `kind(d, n, k)`: node `n`'s kind, a symbol;
//! - `field(d, p, f, c)`: `c` is `p`'s child in field `f`;
//! - `child(d, p, c, i)`: `c` is `p`'s `i`-th child, counting named children that are no comment;
//! - `text(d, n, s)`: a leaf's source text (an identifier, a variable, an operator, a literal), a symbol;
//! - `value(d, n, v)`: a literal's value: a string's unescaped contents (a symbol), a number's value;
//! - `doc(d, path, module)`: the document's path, and the module its answer named (as the engine's `doc` is,
//!   `Document::module`).
//!
//! The symbols a view gives are the document's node → symbol facts, which `intern` wrote into its symbols block
//! (decision 0686, B5): a view reads them there, and never writes the atoms.
//!
//! A `TapeView` is one view of one document's tape: a `Table` whose row ids are the tape's node ids. How the views
//! of many documents are read as one relation, partitioned by document, comes with the traces (rebuild plan, M1 and
//! M4); nothing here gives a node an id past its own tape.

use crate::sym::Names;
use rerail_delta_twins::intern as syms;
use rerail_blocks::bistate::{ColumnIndex, ColumnRead, Read, RowId, WordIndex, below};
use rerail_blocks::table::{Run, Runs, Table};
use rerail_rr_syntax::tape_format::f16::Tape16;
use rerail_rr_syntax::tape_format::home;
use rerail_rr_syntax::tape_format::tape::Tape as _;

/// A document's text: its source, where the file layer's read placed it, and its tape, as the rr parser wrote it, both
/// in the document's tape block (`files`). Neither is copied: a view reads both where they lie.
pub struct Text {
    pub src: home::Text,
    pub tape: Tape16,
}

impl Text {
    /// `src` parsed, its tape written by the parser in `room` (a tape block's, `files`).
    pub fn parse_in(src: home::Text, room: std::sync::Arc<dyn rerail_rr_syntax::tape_format::VectorRoom>) -> Result<Text, String> {
        let tape = rerail_rr_syntax::tape_in(src.as_str(), room)?;
        Text { src, tape }.checked()
    }

    /// Refused when a statement does not parse (the parser writes it as an ERROR leaf, and a file the engine refuses
    /// to run still has a tape).
    fn checked(self) -> Result<Text, String> {
        let r = reads(&self);
        if let Some(i) = (0..r.len()).find(|&i| r.node(i).0 == rerail_rr_syntax::tape::ERROR) {
            return Err(format!("a statement that does not parse: {:?}", r.text(i)));
        }
        Ok(self)
    }
}

/// The views, by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Node,
    Kind,
    Field,
    Child,
    Text,
    Value,
    Doc,
}

impl View {
    pub const ALL: [(View, &'static str, usize); 7] = [
        (View::Node, "node", 5),
        (View::Kind, "kind", 3),
        (View::Field, "field", 4),
        (View::Child, "child", 4),
        (View::Text, "text", 3),
        (View::Value, "value", 3),
        (View::Doc, "doc", 3),
    ];

    pub fn arity(self) -> usize {
        View::ALL.iter().find(|v| v.0 == self).expect("a view").2
    }

    /// The column that names a row's node, which is the row's id.
    fn node_column(self) -> usize {
        match self {
            View::Node | View::Kind | View::Text | View::Value => 1,
            View::Field => 3,
            View::Child => 2,
            View::Doc => 0,
        }
    }
}

pub use rerail_delta_twins::tape::Reads;

/// A document's reads, where its tape and source lie.
pub fn reads(text: &Text) -> Reads<'_> {
    Reads::new(text.tape.reader(rerail_rr_syntax::tape::lang()).expect("an rr tape read with the rr table"), text.src.as_str())
}

/// One view of one document's tape: a `Table` read where the tape lies, its row ids the tape's node ids.
pub struct TapeView<'t> {
    pub view: View,
    /// The document's id.
    d: i64,
    reads: Reads<'t>,
    /// The document's symbols block (`intern`'s node → symbol facts), where it lies.
    syms: &'t [u8],
    names: &'t Names,
}

impl<'t> TapeView<'t> {
    pub fn new(view: View, d: i64, text: &'t Text, syms: &'t [u8], names: &'t Names) -> Self {
        TapeView { view, d, reads: reads(text), syms, names }
    }

    /// Whether node `n` is a row of the view.
    fn live_at(&self, n: usize) -> bool {
        let r = &self.reads;
        match self.view {
            View::Node | View::Kind => true,
            View::Text => r.is_leaf(n),
            View::Field => r.node(n).1 != 0 && r.parent(n).is_some(),
            View::Child => r.counted(n) && r.parent(n).is_some(),
            View::Value => r.has_value(n),
            View::Doc => n == 0,
        }
    }

    /// Column `c` of node `n`'s row.
    fn cell(&self, n: usize, c: usize) -> i64 {
        let r = &self.reads;
        match (self.view, c) {
            (_, 0) => self.d,
            (View::Doc, 1) => i64::from(syms::doc_syms(self.syms).0),
            (View::Doc, 2) => i64::from(syms::doc_syms(self.syms).1),
            (View::Node, 2) | (View::Kind, 2) => self.names.kind(r.node(n).0),
            (View::Node, 3) => i64::from(r.start(n)),
            (View::Node, 4) => i64::from(r.end(n)),
            (View::Field, 1) | (View::Child, 1) => r.parent(n).map_or(-1, |x| x as i64),
            (View::Field, 2) => self.names.field(r.node(n).1),
            (View::Child, 3) => r.parent(n).map_or(-1, |x| r.index(x, n)),
            // The node's symbols, as `intern` wrote them: read, never made here.
            (View::Text, 2) => i64::from(syms::text(self.syms, n)),
            (View::Value, 2) => r.number(n).unwrap_or_else(|| i64::from(syms::value(self.syms, n))),
            _ => n as i64,
        }
    }

    /// Whether node `n`'s row agrees with `cols` and `key`.
    fn fits(&self, n: usize, cols: &[ColumnIndex], key: &[i64]) -> bool {
        self.live_at(n)
            && cols
                .iter()
                .zip(key)
                .all(|(c, &k)| self.cell(n, c.get()) == k)
    }
}

/// A view's column, read through its view.
pub struct TapeColumn<'v, 't>(&'v TapeView<'t>, ColumnIndex);

impl ColumnRead for TapeColumn<'_, '_> {
    fn get(&self, id: RowId) -> i64 {
        self.0.get(id, self.1)
    }
}

impl Read for TapeView<'_> {
    type Column<'s>
        = TapeColumn<'s, 's>
    where
        Self: 's;

    fn arity(&self) -> usize {
        self.view.arity()
    }

    fn end(&self) -> RowId {
        RowId::new(self.reads.len() as u32)
    }

    fn live_word(&self, w: WordIndex) -> u64 {
        let mask = below(self.end(), w);
        let first = w.first().get() as usize;
        let mut bits = 0u64;
        for b in 0..64 {
            if mask >> b & 1 != 0 && self.live_at(first + b) {
                bits |= 1 << b;
            }
        }
        bits
    }

    fn get(&self, id: RowId, c: ColumnIndex) -> i64 {
        self.cell(id.get() as usize, c.get())
    }

    fn column(&self, c: ColumnIndex) -> TapeColumn<'_, '_> {
        TapeColumn(self, c)
    }
}

impl Table for TapeView<'_> {
    fn sort(&self) -> Option<ColumnIndex> {
        None
    }

    fn runs(&self) -> Runs {
        Runs::one(Run::new(0, self.reads.len() as u32))
    }

    /// A bound node is read where it lies; a bound parent's children are walked; else every node.
    fn probe(
        &self,
        cols: &[ColumnIndex],
        key: &[i64],
        each: &mut dyn FnMut(RowId) -> bool,
    ) -> bool {
        let bound = |c: usize| cols.iter().position(|x| x.get() == c).map(|j| key[j]);
        if bound(0).is_some_and(|d| d != self.d) {
            return true;
        }
        let len = self.reads.len();
        // Whether `each` stopped the probe at node `n`.
        let mut stop = |n: usize| self.fits(n, cols, key) && !each(RowId::new(n as u32));
        let stopped = if self.view == View::Doc {
            stop(0)
        } else if let Some(n) = bound(self.view.node_column()) {
            usize::try_from(n)
                .ok()
                .filter(|&n| n < len)
                .is_some_and(&mut stop)
        } else if let (Some(parent), View::Field | View::Child) = (bound(1), self.view) {
            usize::try_from(parent)
                .ok()
                .filter(|&x| x < len)
                .is_some_and(|x| self.reads.children(x).any(&mut stop))
        } else {
            (0..len).any(&mut stop)
        };
        !stopped
    }
}
