//! A document's tape views, read where the tape lies (decision 0353: a view is a read of the tape, not a relation;
//! nothing makes its rows). Each view's first column is the document's id, the partition column (node ids are the
//! document's own, L9):
//!
//! - `kind(d, n, k)`: node `n`'s kind, a symbol;
//! - `field(d, p, f, c)`: `c` is `p`'s child in field `f`;
//! - `child(d, p, c, i)`: `c` is `p`'s `i`-th child, counting named children that are no comment;
//! - `text(d, n, s)`: `n`'s source text, a symbol;
//! - `value(d, n, v)`: a literal's value: a string's unescaped contents (a symbol), a number's value;
//! - `doc(d, path)`: the document's path.
//!
//! A `TapeView` is one view of one document's tape: a `Table` whose row ids are the tape's node ids. How the views
//! of many documents are read as one relation, partitioned by document, comes with the traces (rebuild plan, M1 and
//! M4); nothing here gives a node an id past its own tape.

use crate::sym::Symbols;
use rerail_blocks::bistate::{ColumnIndex, ColumnRead, Read, RowId, WordIndex, below};
use rerail_blocks::table::{Run, Runs, Table};
use rerail_rr_syntax::tape::{FIELD_NAMES, FIRST_ANONYMOUS, KIND_NAMES, Kind, entry_kind_field};
use rerail_tape_format::f16::{F16, Tape16};
use rerail_tape_format::tape::Tape as _;

/// A document's text: its source, loaded from disk, and its tape, made by the rr parser.
pub struct Text {
    // copy: load (the file's bytes, which the tape's spans are read from)
    pub src: String,
    pub tape: Tape16,
}

impl Text {
    /// `src` parsed: refused when a statement does not parse (the parser writes it as an ERROR leaf, and a file the
    /// engine refuses to run still has a tape).
    pub fn parse(src: String) -> Result<Text, String> {
        let tape = rerail_rr_syntax::tape(&src)?;
        let text = Text { src, tape };
        let r = Reads::of(&text);
        if let Some(i) = (0..r.len()).find(|&i| r.node(i).0 == rerail_rr_syntax::tape::ERROR) {
            return Err(format!("a statement that does not parse: {:?}", r.text(i)));
        }
        Ok(text)
    }
}

/// The views, by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Kind,
    Field,
    Child,
    Text,
    Value,
    Doc,
}

impl View {
    pub const ALL: [(View, &'static str, usize); 6] = [
        (View::Kind, "kind", 3),
        (View::Field, "field", 4),
        (View::Child, "child", 4),
        (View::Text, "text", 3),
        (View::Value, "value", 3),
        (View::Doc, "doc", 2),
    ];

    pub fn arity(self) -> usize {
        View::ALL.iter().find(|v| v.0 == self).expect("a view").2
    }

    /// The column that names a row's node, which is the row's id.
    fn node_column(self) -> usize {
        match self {
            View::Kind | View::Text | View::Value => 1,
            View::Field => 3,
            View::Child => 2,
            View::Doc => 0,
        }
    }
}

/// The kinds' and fields' names as symbols, resolved once, so a view compares ids.
pub struct Names {
    kind: Vec<i64>,
    field: Vec<i64>,
}

impl Names {
    pub fn new(sym: &Symbols) -> Names {
        Names {
            kind: KIND_NAMES.iter().map(|n| sym.intern(n)).collect(),
            field: FIELD_NAMES.iter().map(|n| sym.intern(n)).collect(),
        }
    }

    pub fn kind(&self, k: u16) -> i64 {
        self.kind.get(k as usize).copied().unwrap_or(0)
    }

    pub fn field(&self, f: u16) -> i64 {
        self.field.get(f as usize).copied().unwrap_or(0)
    }
}

/// A tape's reads: a node's kind, field, children and text, where the tape lies.
pub struct Reads<'t> {
    r: F16<'t>,
    src: &'t str,
}

impl<'t> Reads<'t> {
    pub fn of(text: &'t Text) -> Reads<'t> {
        let r = text
            .tape
            .reader(rerail_rr_syntax::tape::lang())
            .expect("an rr tape read with the rr table");
        Reads { r, src: &text.src }
    }

    pub fn len(&self) -> usize {
        self.r.len()
    }

    pub fn is_empty(&self) -> bool {
        self.r.len() == 0
    }

    /// Node `i`'s kind and field, and whether it is an extra (a comment).
    pub fn node(&self, i: usize) -> (u16, u16, bool) {
        let e = self.r.table().entry(self.r.code(i));
        let (k, f) = entry_kind_field(e);
        (k, f, e.flags & rerail_tape_format::CODE_EXTRA != 0)
    }

    pub fn parent(&self, i: usize) -> Option<usize> {
        self.r.parent(i)
    }

    /// Node `i`'s children, in order.
    pub fn children(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let end = self.r.subtree_end(i);
        std::iter::successors(self.r.first_child(i), move |&c| self.r.next_sibling(c, end))
    }

    /// Whether node `i` is counted by `child`: named, and no comment.
    pub fn counted(&self, i: usize) -> bool {
        let (k, _, extra) = self.node(i);
        !extra && k < FIRST_ANONYMOUS
    }

    /// Node `i`'s source text.
    pub fn text(&self, i: usize) -> &'t str {
        &self.src[self.r.start(i) as usize..self.r.end(i) as usize]
    }

    /// A string's unescaped contents, when node `i` is one.
    fn string(&self, i: usize) -> Option<String> {
        if self.node(i).0 != Kind::String as u16 {
            return None;
        }
        let body = self
            .text(i)
            .strip_prefix('"')
            .and_then(|b| b.strip_suffix('"'))?;
        rerail_rr_syntax::syntax::unescape(body).ok()
    }

    /// A number's value, when node `i` is one.
    fn number(&self, i: usize) -> Option<i64> {
        (self.node(i).0 == Kind::Number as u16)
            .then(|| self.text(i).parse::<i64>().ok())
            .flatten()
    }

    /// Whether node `i` has a `value`.
    pub fn has_value(&self, i: usize) -> bool {
        self.number(i).is_some() || self.string(i).is_some()
    }

    /// A literal's value: a string's unescaped contents, interned; a number's value. `None` for any other node.
    pub fn value(&self, i: usize, sym: &Symbols) -> Option<i64> {
        self.number(i)
            .or_else(|| self.string(i).map(|s| sym.intern(&s)))
    }

    /// Node `c`'s place among its parent `p`'s counted children.
    pub fn index(&self, p: usize, c: usize) -> i64 {
        self.children(p)
            .filter(|&x| self.counted(x))
            .take_while(|&x| x != c)
            .count() as i64
    }
}

/// One view of one document's tape: a `Table` read where the tape lies, its row ids the tape's node ids.
pub struct TapeView<'t> {
    pub view: View,
    /// The document's id and path, as symbols.
    d: i64,
    path: i64,
    reads: Reads<'t>,
    sym: &'t Symbols,
    names: &'t Names,
}

impl<'t> TapeView<'t> {
    pub fn new(
        view: View,
        d: i64,
        path: i64,
        text: &'t Text,
        sym: &'t Symbols,
        names: &'t Names,
    ) -> Self {
        TapeView {
            view,
            d,
            path,
            reads: Reads::of(text),
            sym,
            names,
        }
    }

    /// Whether node `n` is a row of the view.
    fn live_at(&self, n: usize) -> bool {
        let r = &self.reads;
        match self.view {
            View::Kind | View::Text => true,
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
            (View::Doc, 1) => self.path,
            (View::Kind, 2) => self.names.kind(r.node(n).0),
            (View::Field, 1) | (View::Child, 1) => r.parent(n).map_or(-1, |x| x as i64),
            (View::Field, 2) => self.names.field(r.node(n).1),
            (View::Child, 3) => r.parent(n).map_or(-1, |x| r.index(x, n)),
            // A symbol made of the node's text: a derived fact, written once into the symbols' block.
            (View::Text, 2) => self.sym.intern(r.text(n)),
            (View::Value, 2) => r.value(n, self.sym).expect("a literal's value"),
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
