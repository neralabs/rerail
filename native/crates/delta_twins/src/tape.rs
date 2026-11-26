//! A `.rerail` tape's reads (decision 0686): a node's kind, field, children, span and text, read where the tape and
//! its source lie, a slot's bytes in a twin, a document's home on the host. Format 16's reads (`read_in_place`,
//! `F16`) with the rr language's table; nothing is copied.

use rerail_rr_syntax::tape::{FIRST_ANONYMOUS, Kind, entry_kind_field};
use rerail_rr_syntax::tape_format::f16::{F16, read_in_place};
use rerail_rr_syntax::tape_format::tape::Tape as _;

/// A tape's reads: a node's kind, field, children and text, where the tape lies.
pub struct Reads<'t> {
    r: F16<'t>,
    src: &'t str,
}

impl<'t> Reads<'t> {
    /// A tape's reads over its reader and its source.
    pub fn new(r: F16<'t>, src: &'t str) -> Reads<'t> {
        Reads { r, src }
    }

    /// The reads of a tape's bytes and its source where they lie (a slot's), with the rr language's table.
    pub fn in_place(tape: &'t [u8], src: &'t str) -> Result<Reads<'t>, String> {
        Ok(Reads { r: read_in_place(tape, rerail_rr_syntax::tape::lang())?, src })
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
        (k, f, e.flags & rerail_rr_syntax::tape_format::CODE_EXTRA != 0)
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

    /// Node `i`'s source span: its first byte, and the byte past its last.
    pub fn start(&self, i: usize) -> u32 {
        self.r.start(i) as u32
    }

    pub fn end(&self, i: usize) -> u32 {
        self.r.end(i) as u32
    }

    /// Node `i`'s source text.
    pub fn text(&self, i: usize) -> &'t str {
        &self.src[self.start(i) as usize..self.end(i) as usize]
    }

    /// A string's unescaped contents, when node `i` is one.
    pub fn string(&self, i: usize) -> Option<String> {
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
    pub fn number(&self, i: usize) -> Option<i64> {
        (self.node(i).0 == Kind::Number as u16)
            .then(|| self.text(i).parse::<i64>().ok())
            .flatten()
    }

    /// Whether node `i` has a `value`.
    pub fn has_value(&self, i: usize) -> bool {
        self.number(i).is_some() || self.string(i).is_some()
    }

    /// Whether node `i` is a leaf (no children) and no comment: what `text` reads, an identifier, a variable, an
    /// operator, a literal.
    pub fn is_leaf(&self, i: usize) -> bool {
        !self.node(i).2 && self.r.first_child(i).is_none()
    }

    /// Node `c`'s place among its parent `p`'s counted children.
    pub fn index(&self, p: usize, c: usize) -> i64 {
        self.children(p)
            .filter(|&x| self.counted(x))
            .take_while(|&x| x != c)
            .count() as i64
    }
}

