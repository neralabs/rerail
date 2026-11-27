//! A `.rerail` tape's reads (decision 0686): a node's kind, field, children, span and text, read where the tape and
//! its source lie, a slot's bytes in a twin, a document's home on the host. Format 16's reads (`read_in_place`,
//! `F16`) with the rr language's table; nothing is copied.

use rerail_rr_syntax::tape::{FIRST_ANONYMOUS, Kind, entry_kind_field};
use rerail_rr_syntax::syntax::unescape_each;
use rerail_rr_syntax::tape_format::f16::{F16, read_checked};
use rerail_rr_syntax::tape_format::tape::Tape as _;

/// A string literal's value, read where its body lies in the source: the body itself when it has no escape, else its
/// characters unescaped as they are read (`unescape_each`), each time, with nothing gathered.
#[derive(Clone, Copy, Debug)]
pub struct Str<'t> {
    body: &'t str,
}

impl<'t> Str<'t> {
    /// The value, when it is its body as written (no escape).
    pub fn plain(&self) -> Option<&'t str> {
        (!self.body.contains('\\')).then_some(self.body)
    }

    /// Each character of the value, in turn.
    pub fn each(&self, out: impl FnMut(char)) {
        unescape_each(self.body, out).expect("a string's escapes, checked when it was read");
    }

    /// Each byte run of the value's UTF-8, in turn.
    pub fn each_utf8(&self, mut out: impl FnMut(&[u8])) {
        self.each(|c| out(c.encode_utf8(&mut [0; 4]).as_bytes()));
    }

    /// Whether the value is `s`.
    pub fn is(&self, s: &str) -> bool {
        if let Some(p) = self.plain() {
            return p == s;
        }
        let (mut rest, mut same) = (s.as_bytes(), true);
        self.each_utf8(|b| {
            same &= rest.starts_with(b);
            rest = rest.get(b.len()..).unwrap_or(&[]);
        });
        same && rest.is_empty()
    }

    /// Writes the value's UTF-8 into `to`, exactly as long.
    pub fn write(&self, to: &mut [u8]) {
        let mut at = 0;
        self.each_utf8(|b| {
            to[at..at + b.len()].copy_from_slice(b);
            at += b.len();
        });
        assert_eq!(at, to.len(), "a string's value written whole");
    }
}

/// A tape's reads: a node's kind, field, children and text, where the tape lies.
#[derive(Clone, Copy)]
pub struct Reads<'t> {
    r: F16<'t>,
    src: &'t str,
}

impl<'t> Reads<'t> {
    /// A tape's reads over its reader and its source.
    pub fn new(r: F16<'t>, src: &'t str) -> Reads<'t> {
        Reads { r, src }
    }

    /// The reads of a tape's bytes and its source where they lie (a slot's), with the rr language's table: `None` when
    /// they are not a tape of it (`read_in_place` says why).
    pub fn in_place(tape: &'t [u8], src: &'t str) -> Option<Reads<'t>> {
        Some(Reads { r: read_checked(tape, rerail_rr_syntax::tape::lang())?, src })
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
        core::iter::successors(self.r.first_child(i), move |&c| self.r.next_sibling(c, end))
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

    /// A string's value, when node `i` is one and its escapes are (`Str`: its body, unescaped as it is read).
    pub fn string(&self, i: usize) -> Option<Str<'t>> {
        if self.node(i).0 != Kind::String as u16 {
            return None;
        }
        let body = self
            .text(i)
            .strip_prefix('"')
            .and_then(|b| b.strip_suffix('"'))?;
        unescape_each(body, |_| {}).ok().map(|()| Str { body })
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

