//! The C tape (tree-sitter-c's, `rerail-c-lean`'s columns) read in place: a node's kind, field, span, text and
//! children, by its index in preorder.

use rerail_c_lean::{FIELDS, KINDS, Tape, unkey};
use rerail_tape_format::{CHECKPOINT_STRIDE, TAIL_FROM_START};

/// The C tape and the text it was parsed from.
pub struct C<'t> {
    tape: &'t Tape,
    src: &'t [u8],
    n: usize,
}

impl<'t> C<'t> {
    pub fn new(tape: &'t Tape, src: &'t [u8]) -> Self {
        C { tape, src, n: tape.code.len() / 2 }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    fn cell(column: &[u8], width: u8, i: usize) -> u32 {
        let w = usize::from(width & 0x7f);
        let b = &column[i * w..i * w + w];
        if w == 2 { u32::from(u16::from_le_bytes([b[0], b[1]])) } else { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) }
    }

    fn key(&self, i: usize) -> (u16, u8, bool) {
        let code = u16::from_le_bytes([self.tape.code[2 * i], self.tape.code[2 * i + 1]]);
        let (kind, field, _, extra) = unkey(self.tape.keys[usize::from(code)]);
        (kind, field, extra)
    }

    /// Node `i`'s kind, as tree-sitter-c names it (`binary_expression`, `+`).
    pub fn kind(&self, i: usize) -> &'static str {
        KINDS[usize::from(self.key(i).0)].0
    }

    /// Node `i`'s field in its parent (`""`: none).
    pub fn field(&self, i: usize) -> &'static str {
        FIELDS[usize::from(self.key(i).1)]
    }

    /// Whether node `i` is an extra (a comment).
    pub fn extra(&self, i: usize) -> bool {
        self.key(i).2
    }

    /// Node `i`'s subtree, itself counted.
    pub fn size(&self, i: usize) -> usize {
        (Self::cell(&self.tape.size, self.tape.widths[0], i) as usize).max(1)
    }

    pub fn start(&self, i: usize) -> usize {
        let b = i / CHECKPOINT_STRIDE;
        (u32::from_le_bytes(self.tape.base[4 * b..4 * b + 4].try_into().expect("a base")) + Self::cell(&self.tape.off, self.tape.widths[1], i)) as usize
    }

    pub fn end(&self, i: usize) -> usize {
        let from = if self.tape.widths[2] & TAIL_FROM_START != 0 { self.start(i) } else { self.start(i + self.size(i) - 1) };
        from + Self::cell(&self.tape.tail, self.tape.widths[2], i) as usize
    }

    /// Node `i`'s text.
    pub fn text(&self, i: usize) -> &'t str {
        std::str::from_utf8(&self.src[self.start(i)..self.end(i)]).unwrap_or("")
    }

    /// Node `i`'s children, extras left out.
    pub fn children(&self, i: usize) -> Vec<usize> {
        let (mut at, end) = (i + 1, i + self.size(i));
        let mut out = Vec::new();
        while at < end {
            if !self.extra(at) {
                out.push(at);
            }
            at += self.size(at);
        }
        out
    }

    /// Whether node `i` is a named node (not punctuation or a keyword's token).
    pub fn named(&self, i: usize) -> bool {
        KINDS[usize::from(self.key(i).0)].1
    }

    /// Node `i`'s named children.
    pub fn named_children(&self, i: usize) -> Vec<usize> {
        self.children(i).into_iter().filter(|&c| self.named(c)).collect()
    }

    /// Node `i`'s child in field `field`.
    pub fn child(&self, i: usize, field: &str) -> Option<usize> {
        self.children(i).into_iter().find(|&c| self.field(c) == field)
    }

    /// Node `i`'s children in field `field`.
    pub fn children_in(&self, i: usize, field: &str) -> Vec<usize> {
        self.children(i).into_iter().filter(|&c| self.field(c) == field).collect()
    }

    /// The tape's root (`translation_unit`).
    pub fn root(&self) -> usize {
        0
    }
}
