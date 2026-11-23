//! The C tape (tree-sitter-c's, `rerail-c-lean`'s format 16 tape) read in place: a node's kind, field, span, text
//! and children, by its index in preorder, named by its language's table (decision 0626).

use rerail_c_lean::Tape;
use rerail_tape_format::f16::{F16, Lang, Written, lang};
use rerail_tape_format::tape::{Entry, Tape as _};

/// The C tape and the text it was parsed from.
pub struct C<'t> {
    tape: F16<'t>,
    lang: &'static Lang,
    src: &'t [u8],
}

impl<'t> C<'t> {
    /// `tape`, a C or C++ tape `rerail-c-lean` wrote (format 16, its language's table).
    pub fn new(tape: &'t Tape, src: &'t [u8]) -> Self {
        let Written::F16(t) = &tape.tape else { panic!("rerail-c-lean writes format 16") };
        let lang = lang(t.tag()).expect("C's table");
        C { tape: t.reader(lang).expect("a tape rerail-c-lean wrote"), lang, src }
    }

    pub fn len(&self) -> usize {
        self.tape.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tape.is_empty()
    }

    fn entry(&self, i: usize) -> &'static Entry {
        &self.lang.table().entries()[self.tape.code(i) as usize]
    }

    /// Node `i`'s kind, as tree-sitter-c names it (`binary_expression`, `+`).
    pub fn kind(&self, i: usize) -> &'static str {
        let e = self.entry(i);
        match &e.alias {
            Some((name, _)) => name,
            None => self.lang.names().kind_name(e.kind).unwrap_or("?"),
        }
    }

    /// Node `i`'s field in its parent (`""`: none).
    pub fn field(&self, i: usize) -> &'static str {
        self.lang.names().field_name(self.entry(i).field).unwrap_or("")
    }

    /// Whether node `i` is an extra (a comment).
    pub fn extra(&self, i: usize) -> bool {
        self.entry(i).flags & rerail_tape_format::CODE_EXTRA != 0
    }

    /// Node `i`'s subtree, itself counted.
    pub fn size(&self, i: usize) -> usize {
        self.tape.subtree_end(i) - i
    }

    pub fn start(&self, i: usize) -> usize {
        self.tape.start(i) as usize
    }

    pub fn end(&self, i: usize) -> usize {
        self.tape.end(i) as usize
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
        let e = self.entry(i);
        e.alias.as_ref().map_or_else(|| self.lang.names().kind_named(e.kind), |a| a.1)
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
