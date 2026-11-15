//! The parser: recursive descent over tree-sitter-css's grammar, writing the tape as it goes.
//!
//! tree-sitter-css is LR(1) with no conflicts left to GLR, and its lexer reads, in each parse state,
//! the longest of the tokens that state takes. The parser reads each token where it knows the
//! state, by the same rule: a value's word is a `plain_value` where that is longer than an
//! `identifier`, a number's unit is read only right after it, a keyword (`and`, `not`, `from`, …)
//! wins over an identifier of the same length.
//!
//! **The external scanner** decides two things the grammar cannot: whether white space after a
//! selector is a descendant combinator (`_descendant_operator`), and whether a `:` after a selector
//! starts a pseudo-class (a `{` comes before any `;` or `}`) or is a declaration's. The parser asks
//! the same questions at the same bytes (`descendant`, `pseudo_colon`).
//!
//! **Selectors** are left-associative, and `.class` binds tighter than a combinator, while `:`, `::`,
//! `#` and `[` bind looser (they take the whole selector before them): `a > b.c` is `a > (b.c)`, but
//! `a > b:hover` is `(a > b):hover`, as tree-sitter-css's precedences give.
//!
//! **Extras** (comments) are written when the parse next opens a node or reads a token, at the
//! node then open.

use crate::block::Blocks;
use crate::kinds::K;
use crate::{Refusal, Tape, key};
use rerail_tape_format::{Redo, Widths, Writer};

type R<T> = Result<T, Refusal>;

/// A node written: its index and kind.
#[derive(Clone, Copy, Debug)]
struct N {
    at: usize,
    kind: K,
}

struct P<'a> {
    src: &'a [u8],
    /// Where the next token's search starts: past the last token read (white space before it
    /// unread, which the scanner reads).
    at: usize,
    /// Comments read ahead, not yet written: start, end, whether a `//` one.
    pending: Vec<(u32, u32, bool)>,
    /// The end of the last comment read ahead: one before it is not read again.
    seen: usize,
    w: Writer<Widths>,
    /// The text's byte classes (stage 1).
    b: Blocks<'a>,
    last_end: u32,
    depth: u32,
}

const MAX_DEPTH: u32 = 200;

/// `src` parsed.
pub fn parse(src: &[u8]) -> R<Tape> {
    if src.contains(&0) {
        return Err(Refusal::Lex(src.iter().position(|&c| c == 0).unwrap_or(0) as u32));
    }
    let mut widths = Widths::guess(src.len());
    loop {
        // A leading byte order mark is skipped (tree-sitter's lexer skips it): the text starts after.
        let start = if src.starts_with("\u{feff}".as_bytes()) { 3 } else { 0 };
        let mut p = P { src, at: start, pending: Vec::new(), seen: 0, w: Writer::new(0, widths, src.len() / 3 + 16), b: Blocks::of(src), last_end: 0, depth: 0 };
        p.stylesheet()?;
        match p.w.finish_columns() {
            Ok(columns) => return Ok(columns),
            Err(Redo(w)) => {
                if w == widths {
                    return Err(Refusal::TooLarge);
                }
                widths = w;
            }
        }
    }
}

#[inline(always)]
fn is_blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

#[inline(always)]
fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

#[inline(always)]
fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_'
}

/// A delimiter no `plain_value` byte is: white space, `,`, `;`, `!`, braces, parentheses, brackets.
#[inline(always)]
fn is_value_stop(c: u8) -> bool {
    is_blank(c) || matches!(c, b',' | b';' | b'!' | b'{' | b'}' | b'(' | b')' | b'[' | b']')
}

impl<'a> P<'a> {
    fn refuse<T>(&self) -> R<T> {
        Err(Refusal::Parse(self.at as u32))
    }

    #[inline(always)]
    fn byte(&self, at: usize) -> u8 {
        self.src.get(at).copied().unwrap_or(0)
    }

    // ---- White space and comments.

    /// The next token's start: past white space and comments from `at` (the comments kept to be
    /// written). A byte past ASCII there is refused (`\s` and the scanner's `iswspace` may differ
    /// on it, and a name's characters past ASCII are not read).
    fn skip(&mut self) -> R<usize> {
        let mut at = self.at;
        loop {
            at = self.b.blank_end(at);
            match (self.byte(at), self.byte(at + 1)) {
                (b'/', b'*') => {
                    let Some(end) = self.b.comment_end(at + 2) else {
                        return Err(Refusal::Parse(at as u32));
                    };
                    if at >= self.seen {
                        self.pending.push((at as u32, end as u32, false));
                        self.seen = end;
                    }
                    at = end;
                }
                (b'/', b'/') => {
                    let end = self.b.line_end(at);
                    if at >= self.seen {
                        self.pending.push((at as u32, end as u32, true));
                        self.seen = end;
                    }
                    at = end;
                }
                _ if self.latin1(at) => return Ok(at),
                (c, _) if c >= 0x80 => return Err(Refusal::Parse(at as u32)),
                _ => return Ok(at),
            }
        }
    }

    /// The next token's first byte.
    fn peek(&mut self) -> R<u8> {
        let at = self.skip()?;
        Ok(self.byte(at))
    }

    /// The comments before `before`, written at the node open now.
    fn flush(&mut self, before: usize) {
        let mut k = 0;
        while k < self.pending.len() && (self.pending[k].0 as usize) < before {
            let (s, e, js) = self.pending[k];
            let n = self.w.push(key(if js { K::JsComment } else { K::Comment }, true), s);
            self.w.close(n, e);
            k += 1;
        }
        self.pending.drain(..k);
    }

    // ---- The writer.

    fn open(&mut self, kind: K, start: usize) -> N {
        self.flush(start);
        N { at: self.w.push(key(kind, false), start as u32), kind }
    }

    fn close(&mut self, n: N) {
        self.w.close(n.at, self.last_end);
    }

    /// A leaf of `kind` at `from..to`, which the parse has reached.
    fn leaf(&mut self, kind: K, from: usize, to: usize) -> N {
        self.flush(from);
        let at = self.w.push(key(kind, false), from as u32);
        self.w.close(at, to as u32);
        self.at = to;
        self.last_end = to as u32;
        N { at, kind }
    }

    /// Token `tok` (`len` bytes at the next token) under `parent`: a leaf where the tape stores it.
    fn tok(&mut self, parent: K, tok: K, len: usize) -> R<()> {
        let at = self.skip()?;
        self.flush(at);
        if crate::stores(parent, tok) {
            let n = self.w.push(key(tok, false), at as u32);
            self.w.close(n, (at + len) as u32);
        }
        self.at = at + len;
        self.last_end = self.at as u32;
        Ok(())
    }

    /// Token `tok` (`len` bytes at the next token), stored whatever its parent: where the gap's
    /// state, not the kinds, decides it is kept.
    fn stored(&mut self, tok: K, len: usize) -> R<()> {
        let at = self.skip()?;
        self.flush(at);
        let n = self.w.push(key(tok, false), at as u32);
        self.w.close(n, (at + len) as u32);
        self.at = at + len;
        self.last_end = self.at as u32;
        Ok(())
    }

    /// The next token must be the byte `c`, of kind `tok`, under `parent`.
    fn expect(&mut self, parent: K, c: u8, tok: K) -> R<()> {
        if self.peek()? != c {
            return Err(Refusal::Parse(self.skip()? as u32));
        }
        self.tok(parent, tok, 1)
    }

    /// `kind` put before `n` (wrapping it and what follows).
    fn wrap(&mut self, n: N, kind: K) -> N {
        let start = self.w.start(n.at);
        self.w.insert(n.at, key(kind, false), start);
        N { at: n.at, kind }
    }

    fn deeper(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.refuse();
        }
        Ok(())
    }

    // ---- Words.

    /// Whether a character from U+00A0 to U+00FF starts at `at` (two bytes in UTF-8): an
    /// identifier's character past ASCII (`[\xA0-\xFF]` in the grammar's patterns).
    #[inline(always)]
    fn latin1(&self, at: usize) -> bool {
        match self.byte(at) {
            0xc2 => (0xa0..=0xbf).contains(&self.byte(at + 1)),
            0xc3 => (0x80..=0xbf).contains(&self.byte(at + 1)),
            _ => false,
        }
    }

    /// The end of a run of identifier characters from `at`; one past ASCII that is not U+00A0 to
    /// U+00FF is refused.
    fn ident_run(&self, mut e: usize) -> R<usize> {
        loop {
            e = self.b.ident_end(e);
            let c = self.byte(e);
            if self.latin1(e) {
                e += 2;
            } else if c >= 0x80 {
                return Err(Refusal::Parse(e as u32));
            } else {
                return Ok(e);
            }
        }
    }

    /// The end of an `identifier` at `at` (`(--|-?[a-zA-Z_])[a-zA-Z0-9-_]*`), or `None`.
    fn identifier_end(&self, at: usize) -> R<Option<usize>> {
        let c = self.byte(at);
        let body = if c == b'-' && self.byte(at + 1) == b'-' {
            at + 2
        } else if c == b'-' && is_ident_start(self.byte(at + 1)) {
            at + 2
        } else if c == b'-' && self.latin1(at + 1) {
            at + 3
        } else if is_ident_start(c) {
            at + 1
        } else if self.latin1(at) {
            at + 2
        } else {
            if c >= 0x80 || (c == b'-' && self.byte(at + 1) >= 0x80) {
                return Err(Refusal::Parse(at as u32));
            }
            return Ok(None);
        };
        Ok(Some(self.ident_run(body)?))
    }

    /// The end of a `plain_value` at `at`, or `None`.
    fn plain_value_end(&self, at: usize) -> R<Option<usize>> {
        let mut e = at;
        loop {
            match self.byte(e) {
                b'-' | b'_' => e += 1,
                b'/' if !matches!(self.byte(e + 1), b'*') && !is_value_stop(self.byte(e + 1)) && self.byte(e + 1) != 0 => e += 2,
                _ => break,
            }
        }
        if !self.byte(e).is_ascii_alphabetic() {
            return Ok(None);
        }
        e += 1;
        loop {
            let c = self.byte(e);
            if c == 0 || is_value_stop(c) {
                break;
            }
            if c >= 0x80 {
                return Err(Refusal::Parse(e as u32));
            }
            if c == b'/' {
                let d = self.byte(e + 1);
                if d == b'*' || d == 0 || is_value_stop(d) {
                    break;
                }
                e += 2;
            } else {
                e += 1;
            }
        }
        Ok(Some(e))
    }

    /// Whether the word at `at..at + w.len()` is `w` and no identifier byte follows it (a keyword
    /// wins over an identifier only where they are as long).
    fn is_word(&self, at: usize, w: &[u8]) -> bool {
        self.src.get(at..at + w.len()) == Some(w) && !is_ident(self.byte(at + w.len())) && self.byte(at + w.len()) < 0x80
    }

    // ---- The scanner's questions.

    /// After a selector, at `at` (the byte past it): whether white space there is a descendant
    /// combinator (`_descendant_operator`), and where the next selector starts. `Some(false)`: the
    /// scanner returns no token at a `:` it read past (no pseudo-class colon is tried after).
    fn descendant(&self, at: usize) -> Option<bool> {
        if !is_blank(self.byte(at)) {
            return None;
        }
        let mut q = at;
        while is_blank(self.byte(q)) {
            q += 1;
        }
        let c = self.byte(q);
        if matches!(c, b'#' | b'.' | b'[' | b'-' | b'*') || c.is_ascii_alphanumeric() {
            return Some(true);
        }
        if c == b':' {
            q += 1;
            if is_blank(self.byte(q)) {
                return Some(false);
            }
            loop {
                match self.byte(q) {
                    b';' | b'}' | 0 => return Some(false),
                    b'{' => return Some(true),
                    _ => q += 1,
                }
            }
        }
        None
    }

    /// At a `:` (white space before it skipped): whether it is a pseudo-class's colon (a `{`
    /// before the next `;` or `}`, outside a comment, or the text's end).
    fn pseudo_colon(&self, colon: usize) -> bool {
        if self.byte(colon + 1) == b':' {
            return false;
        }
        let mut at = colon + 1;
        let mut in_comment = false;
        // The scanner's loop: it moves past a byte, then looks at the next one.
        while !matches!(self.byte(at), b';' | b'}') && at < self.src.len() {
            at += 1;
            let c = self.byte(at);
            if c == b'{' && !in_comment {
                return true;
            }
            if c == b'/' && !in_comment {
                at += 1;
                if self.byte(at) == b'*' {
                    in_comment = true;
                }
            } else if c == b'*' && in_comment {
                at += 1;
                if self.byte(at) == b'/' {
                    in_comment = false;
                }
            }
        }
        at >= self.src.len()
    }

    // ---- Statements.

    fn stylesheet(&mut self) -> R<()> {
        let first = self.skip()?;
        let first = self.pending.first().map_or(first, |c| (c.0 as usize).min(first));
        let root = self.w.push(key(K::Stylesheet, false), first as u32);
        loop {
            let at = self.skip()?;
            if at >= self.src.len() {
                break;
            }
            self.item(false)?;
        }
        self.flush(usize::MAX);
        self.w.close(root, self.src.len() as u32);
        Ok(())
    }

    /// A statement at the top level or in a block (`in_block`: `postcss_statement` is a block's).
    fn item(&mut self, in_block: bool) -> R<()> {
        self.deeper()?;
        let at = self.skip()?;
        let c = self.byte(at);
        if c == b'@' {
            self.at_statement(in_block)?;
        } else if let Some(e) = self.identifier_end(at)? {
            // A declaration's property, or a selector's tag name: decided after the name, by the
            // scanner where a `:` follows.
            let decl = match self.byte(e) {
                b':' => self.byte(e + 1) != b':' && !self.pseudo_colon(e),
                c if is_blank(c) => match self.descendant(e) {
                    Some(true) => false,
                    Some(false) => true,
                    None => {
                        let mut q = e;
                        while is_blank(self.byte(q)) {
                            q += 1;
                        }
                        // White space and a `:` with white space after it: a declaration's.
                        self.byte(q) == b':'
                    }
                },
                _ => false,
            };
            if decl {
                self.declaration(in_block)?;
            } else {
                self.rule_set()?;
            }
        } else if matches!(c, b'.' | b'#' | b'[' | b'*' | b'&' | b':' | b'>' | b'~' | b'+' | b'|' | b'"' | b'\'') {
            self.rule_set()?;
        } else {
            return self.refuse();
        }
        self.depth -= 1;
        Ok(())
    }

    fn rule_set(&mut self) -> R<()> {
        let at = self.skip()?;
        let rs = self.open(K::RuleSet, at);
        self.selectors()?;
        self.block()?;
        self.close(rs);
        Ok(())
    }

    fn selectors(&mut self) -> R<()> {
        let at = self.skip()?;
        let n = self.open(K::Selectors, at);
        loop {
            self.selector()?;
            if self.peek()? == b',' {
                self.tok(K::Selectors, K::TComma, 1)?;
                continue;
            }
            break;
        }
        self.close(n);
        Ok(())
    }

    /// `{` statements, a last declaration without its `;`, `}`.
    fn block(&mut self) -> R<()> {
        let at = self.skip()?;
        if self.byte(at) != b'{' {
            return Err(Refusal::Parse(at as u32));
        }
        let b = self.open(K::Block, at);
        self.tok(K::Block, K::TLBrace, 1)?;
        loop {
            let at = self.skip()?;
            match self.byte(at) {
                b'}' => break,
                0 => return Err(Refusal::Parse(at as u32)),
                _ => self.item(true)?,
            }
        }
        self.tok(K::Block, K::TRBrace, 1)?;
        self.close(b);
        Ok(())
    }

    /// `property: values [!important] ;` (in a block, the last one's `;` may be left out).
    fn declaration(&mut self, in_block: bool) -> R<()> {
        let at = self.skip()?;
        let d = self.open(K::Declaration, at);
        let e = self.identifier_end(at)?.expect("a property name");
        self.leaf(K::PropertyName, at, e);
        self.expect(K::Declaration, b':', K::TColon)?;
        self.value()?;
        loop {
            match self.peek()? {
                b';' => {
                    self.tok(K::Declaration, K::TSemi, 1)?;
                    break;
                }
                b'}' if in_block => break,
                b',' => {
                    self.tok(K::Declaration, K::TComma, 1)?;
                    self.value()?;
                }
                _ => self.value()?,
            }
        }
        self.close(d);
        Ok(())
    }

    // ---- At-rules.

    fn at_statement(&mut self, in_block: bool) -> R<()> {
        let at = self.skip()?;
        let mut e = at + 1;
        while self.byte(e).is_ascii_alphabetic() || matches!(self.byte(e), b'-' | b'_') {
            e += 1;
        }
        if e == at + 1 || self.byte(e) >= 0x80 {
            return Err(Refusal::Parse(at as u32));
        }
        let word = &self.src[at..e];
        match word {
            b"@import" => {
                let n = self.open(K::ImportStatement, at);
                self.tok(K::ImportStatement, K::TAtImport, e - at)?;
                self.value()?;
                if self.peek()? != b';' {
                    self.queries(K::ImportStatement)?;
                }
                self.expect(K::ImportStatement, b';', K::TSemi)?;
                self.close(n);
            }
            b"@media" => {
                let n = self.open(K::MediaStatement, at);
                self.tok(K::MediaStatement, K::TAtMedia, e - at)?;
                self.queries(K::MediaStatement)?;
                self.block()?;
                self.close(n);
            }
            b"@charset" => {
                let n = self.open(K::CharsetStatement, at);
                self.tok(K::CharsetStatement, K::TAtCharset, e - at)?;
                self.value()?;
                self.expect(K::CharsetStatement, b';', K::TSemi)?;
                self.close(n);
            }
            b"@namespace" => {
                let n = self.open(K::NamespaceStatement, at);
                self.tok(K::NamespaceStatement, K::TAtNamespace, e - at)?;
                let s = self.skip()?;
                if let Some(ie) = self.identifier_end(s)? {
                    if self.byte(ie) != b'(' {
                        self.leaf(K::NamespaceName, s, ie);
                    }
                }
                let s = self.skip()?;
                if matches!(self.byte(s), b'"' | b'\'') {
                    self.string_value()?;
                } else {
                    let Some(ie) = self.identifier_end(s)? else { return self.refuse() };
                    if self.byte(ie) != b'(' {
                        return self.refuse();
                    }
                    self.call(s, ie)?;
                }
                self.expect(K::NamespaceStatement, b';', K::TSemi)?;
                self.close(n);
            }
            b"@supports" => {
                let n = self.open(K::SupportsStatement, at);
                self.tok(K::SupportsStatement, K::TAtSupports, e - at)?;
                self.query()?;
                self.block()?;
                self.close(n);
            }
            b"@scope" => {
                let n = self.open(K::ScopeStatement, at);
                self.tok(K::ScopeStatement, K::TAtScope, e - at)?;
                if self.peek()? == b'(' {
                    self.tok(K::ScopeStatement, K::TLParen, 1)?;
                    self.selector()?;
                    self.expect(K::ScopeStatement, b')', K::TRParen)?;
                    let s = self.skip()?;
                    if self.is_word(s, b"to") {
                        self.tok(K::ScopeStatement, K::TTo, 2)?;
                        self.expect(K::ScopeStatement, b'(', K::TLParen)?;
                        self.selector()?;
                        self.expect(K::ScopeStatement, b')', K::TRParen)?;
                    }
                }
                self.block()?;
                self.close(n);
            }
            _ if word == b"@keyframes" || (word.ends_with(b"keyframes") && word.len() > 10 && word[1..].iter().all(|&c| c.is_ascii_lowercase() || c == b'-')) => {
                let n = self.open(K::KeyframesStatement, at);
                if word == b"@keyframes" {
                    self.tok(K::KeyframesStatement, K::TAtKeyframes, e - at)?;
                } else {
                    self.leaf(K::AtKeyword, at, e);
                }
                let s = self.skip()?;
                let Some(ie) = self.identifier_end(s)? else { return self.refuse() };
                self.leaf(K::KeyframesName, s, ie);
                self.keyframe_block_list()?;
                self.close(n);
            }
            _ => {
                let n = self.open(K::AtRule, at);
                self.leaf(K::AtKeyword, at, e);
                let _ = in_block;
                if !matches!(self.peek()?, b';' | b'{') {
                    self.queries(K::AtRule)?;
                }
                if self.peek()? == b';' {
                    self.tok(K::AtRule, K::TSemi, 1)?;
                } else {
                    self.block()?;
                }
                self.close(n);
            }
        }
        Ok(())
    }

    fn keyframe_block_list(&mut self) -> R<()> {
        let at = self.skip()?;
        if self.byte(at) != b'{' {
            return self.refuse();
        }
        let l = self.open(K::KeyframeBlockList, at);
        self.tok(K::KeyframeBlockList, K::TLBrace, 1)?;
        while self.peek()? != b'}' {
            let s = self.skip()?;
            let b = self.open(K::KeyframeBlock, s);
            loop {
                let s = self.skip()?;
                if self.is_word(s, b"from") {
                    self.leaf(K::From, s, s + 4);
                } else if self.is_word(s, b"to") {
                    // `to` keeps its token; `from` holds none.
                    let n = self.open(K::To, s);
                    self.stored(K::TTo, 2)?;
                    self.close(n);
                } else if self.byte(s).is_ascii_digit() || (matches!(self.byte(s), b'+' | b'-') && self.byte(s + 1).is_ascii_digit()) {
                    // A keyframe's selector is an integer (`integer_value`); a float is an error.
                    if self.number(false)?.kind != K::IntegerValue {
                        return Err(Refusal::Parse(s as u32));
                    }
                } else {
                    return self.refuse();
                }
                if self.peek()? == b',' {
                    self.tok(K::KeyframeBlock, K::TComma, 1)?;
                    continue;
                }
                break;
            }
            self.block()?;
            self.close(b);
        }
        self.tok(K::KeyframeBlockList, K::TRBrace, 1)?;
        self.close(l);
        Ok(())
    }

    // ---- Queries.

    /// Queries between `,`s.
    fn queries(&mut self, parent: K) -> R<()> {
        loop {
            self.query()?;
            if self.peek()? == b',' {
                self.tok(parent, K::TComma, 1)?;
                continue;
            }
            return Ok(());
        }
    }

    /// A query, its `and`s and `or`s left-associative.
    fn query(&mut self) -> R<()> {
        self.deeper()?;
        let mut left = self.query_operand()?;
        loop {
            let s = self.skip()?;
            let w: &[u8] = if self.is_word(s, b"and") {
                b"and"
            } else if self.is_word(s, b"or") {
                b"or"
            } else {
                break;
            };
            let b = self.wrap(left, K::BinaryQuery);
            self.tok(K::BinaryQuery, if w == b"and" { K::TAnd } else { K::TOr }, w.len())?;
            self.query_operand()?;
            self.close(b);
            left = b;
        }
        self.depth -= 1;
        Ok(())
    }

    fn query_operand(&mut self) -> R<N> {
        let s = self.skip()?;
        if self.is_word(s, b"not") || self.is_word(s, b"only") {
            let len = if self.byte(s) == b'n' { 3 } else { 4 };
            let n = self.open(K::UnaryQuery, s);
            self.tok(K::UnaryQuery, if len == 3 { K::TNot } else { K::TOnly }, len)?;
            // `unary_query` binds tighter than `and`: its operand is one query's operand.
            self.query_operand()?;
            self.close(n);
            return Ok(n);
        }
        if self.is_word(s, b"selector") && self.byte(s + 8) == b'(' {
            let n = self.open(K::SelectorQuery, s);
            self.tok(K::SelectorQuery, K::TSelector, 8)?;
            self.tok(K::SelectorQuery, K::TLParen, 1)?;
            self.selector()?;
            self.expect(K::SelectorQuery, b')', K::TRParen)?;
            self.close(n);
            return Ok(n);
        }
        if self.byte(s) == b'(' {
            // A feature (`(name: values)`) or a query in parentheses.
            let mut q = s + 1;
            while is_blank(self.byte(q)) {
                q += 1;
            }
            let feature = match self.identifier_end(q)? {
                Some(e) => {
                    let mut r = e;
                    while is_blank(self.byte(r)) {
                        r += 1;
                    }
                    self.byte(r) == b':'
                }
                None => false,
            };
            if feature {
                let n = self.open(K::FeatureQuery, s);
                self.tok(K::FeatureQuery, K::TLParen, 1)?;
                let a = self.skip()?;
                let e = self.identifier_end(a)?.expect("a name");
                self.leaf(K::FeatureName, a, e);
                self.expect(K::FeatureQuery, b':', K::TColon)?;
                self.value()?;
                while self.peek()? != b')' {
                    self.value()?;
                }
                self.tok(K::FeatureQuery, K::TRParen, 1)?;
                self.close(n);
                return Ok(n);
            }
            let n = self.open(K::ParenthesizedQuery, s);
            self.tok(K::ParenthesizedQuery, K::TLParen, 1)?;
            self.query()?;
            self.expect(K::ParenthesizedQuery, b')', K::TRParen)?;
            self.close(n);
            return Ok(n);
        }
        let Some(e) = self.identifier_end(s)? else { return Err(Refusal::Parse(s as u32)) };
        Ok(self.leaf(K::KeywordQuery, s, e))
    }


    // ---- Selectors.

    /// A selector: compound selectors and combinators, left-associative. A `.class` binds to the
    /// operand before it; `:`, `::`, `#` and `[` take the whole selector so far.
    fn selector(&mut self) -> R<()> {
        self.deeper()?;
        let s = self.skip()?;
        let mut whole = if matches!(self.byte(s), b'>' | b'~' | b'+' | b'|') && !matches!(self.byte(s + 1), b'=') { None } else { Some(self.compound()?) };
        loop {
            let at = self.at;
            let s = self.skip()?;
            let c = self.byte(s);
            let comb = match c {
                b'>' => Some((K::ChildSelector, K::TGt)),
                b'~' if self.byte(s + 1) != b'=' => Some((K::SiblingSelector, K::TTilde)),
                b'+' => Some((K::AdjacentSiblingSelector, K::TPlus)),
                b'|' if self.byte(s + 1) != b'=' => Some((K::NamespaceSelector, K::TPipe)),
                _ => None,
            };
            if let Some((kind, tk)) = comb {
                let n = match whole {
                    Some(w) => self.wrap(w, kind),
                    None => self.open(kind, s),
                };
                // With nothing before it, the combinator is the node's first token, kept.
                if whole.is_none() {
                    self.stored(tk, 1)?;
                } else {
                    self.tok(kind, tk, 1)?;
                }
                self.compound()?;
                self.close(n);
                whole = Some(n);
                continue;
            }
            let Some(w) = whole else { return self.refuse() };
            if s > at {
                if self.descendant(at) == Some(true) && self.pending.last().is_none_or(|p| (p.0 as usize) < at) {
                    let n = self.wrap(w, K::DescendantSelector);
                    self.compound()?;
                    self.close(n);
                    whole = Some(n);
                    continue;
                }
                break;
            }
            match c {
                b'.' | b'#' | b'[' => whole = Some(self.suffix(w, c)?),
                b':' if self.byte(s + 1) == b':' || self.pseudo_colon(s) => whole = Some(self.suffix(w, b':')?),
                _ => break,
            }
        }
        self.depth -= 1;
        Ok(())
    }

    /// A simple selector and the `.class`es right after it.
    fn compound(&mut self) -> R<N> {
        let mut n = self.simple()?;
        while self.byte(self.at) == b'.' {
            n = self.suffix(n, b'.')?;
        }
        Ok(n)
    }

    /// A simple selector with nothing before it.
    fn simple(&mut self) -> R<N> {
        let s = self.skip()?;
        match self.byte(s) {
            b'*' => {
                // `*` is the node's token, kept.
                let n = self.open(K::UniversalSelector, s);
                self.tok(K::UniversalSelector, K::TStar, 1)?;
                self.close(n);
                Ok(n)
            }
            b'&' => Ok(self.leaf(K::NestingSelector, s, s + 1)),
            b'"' | b'\'' => self.string_value(),
            b'.' | b'#' | b'[' => self.suffix_of(None, self.byte(s)),
            b':' if self.byte(s + 1) == b':' || self.pseudo_colon(s) => self.suffix_of(None, b':'),
            _ => match self.identifier_end(s)? {
                Some(e) => Ok(self.leaf(K::TagName, s, e)),
                None => Err(Refusal::Parse(s as u32)),
            },
        }
    }

    fn suffix(&mut self, left: N, c: u8) -> R<N> {
        self.suffix_of(Some(left), c)
    }

    /// `.class`, `#id`, `[attribute]`, `:pseudo-class` or `::pseudo-element`, after `left`.
    fn suffix_of(&mut self, left: Option<N>, c: u8) -> R<N> {
        let s = self.skip()?;
        let kind = match c {
            b'.' => K::ClassSelector,
            b'#' => K::IdSelector,
            b'[' => K::AttributeSelector,
            _ if self.byte(s + 1) == b':' => K::PseudoElementSelector,
            _ => K::PseudoClassSelector,
        };
        let n = match left {
            Some(l) => self.wrap(l, kind),
            None => self.open(kind, s),
        };
        match kind {
            K::ClassSelector => {
                self.tok(kind, K::TDot, 1)?;
                self.class_name()?;
            }
            K::IdSelector => {
                self.tok(kind, K::THash, 1)?;
                let a = self.at;
                let Some(e) = self.identifier_end(a)? else { return self.refuse() };
                self.leaf(K::IdName, a, e);
            }
            K::AttributeSelector => {
                self.tok(kind, K::TLBracket, 1)?;
                let a = self.skip()?;
                let Some(e) = self.identifier_end(a)? else { return self.refuse() };
                self.leaf(K::AttributeName, a, e);
                let o = self.skip()?;
                let op = match (self.byte(o), self.byte(o + 1)) {
                    (b'=', _) => Some((K::TEq, 1)),
                    (b'~', b'=') => Some((K::TTildeEq, 2)),
                    (b'^', b'=') => Some((K::TCaretEq, 2)),
                    (b'|', b'=') => Some((K::TPipeEq, 2)),
                    (b'*', b'=') => Some((K::TStarEq, 2)),
                    (b'$', b'=') => Some((K::TDollarEq, 2)),
                    _ => None,
                };
                if let Some((tk, len)) = op {
                    self.tok(kind, tk, len)?;
                    self.value()?;
                }
                self.expect(kind, b']', K::TRBracket)?;
            }
            K::PseudoElementSelector => {
                self.tok(kind, K::TColonColon, 2)?;
                let a = self.at;
                let Some(e) = self.identifier_end(a)? else { return self.refuse() };
                self.leaf(K::TagName, a, e);
                if self.byte(e) == b'(' {
                    self.pseudo_arguments(K::PseudoElementArguments)?;
                }
            }
            _ => {
                self.tok(kind, K::TColon, 1)?;
                let a = self.at;
                let ie = self.identifier_end(a)?;
                let word = |w: &[u8]| ie == Some(a + w.len()) && self.src.get(a..a + w.len()) == Some(w);
                if [&b"has"[..], b"not", b"is", b"where", b"host", b"host-context"].iter().any(|w| word(w)) && self.byte(ie.unwrap()) == b'(' {
                    self.leaf(K::ClassName, a, ie.unwrap());
                    self.selector_arguments()?;
                } else if word(b"host") {
                    self.leaf(K::ClassName, a, ie.unwrap());
                } else if [&b"nth-child"[..], b"nth-last-child"].iter().any(|w| word(w)) {
                    if self.byte(ie.unwrap()) != b'(' {
                        return self.refuse();
                    }
                    self.leaf(K::ClassName, a, ie.unwrap());
                    self.nth_arguments()?;
                } else if [&b"has"[..], b"not", b"is", b"where", b"host-context"].iter().any(|w| word(w)) {
                    // The keyword wins over the name, and wants its arguments.
                    return self.refuse();
                } else {
                    self.class_name()?;
                    if self.byte(self.at) == b'(' {
                        self.pseudo_arguments(K::PseudoClassArguments)?;
                    }
                }
            }
        }
        self.close(n);
        Ok(n)
    }

    /// A class name: an identifier or an escape, then names and escapes, each a child.
    fn class_name(&mut self) -> R<()> {
        let a = self.at;
        let n = self.open(K::ClassName, a);
        match self.identifier_end(a)? {
            Some(e) => {
                self.leaf(K::Identifier, a, e);
            }
            None if self.byte(a) == b'\\' => self.escape()?,
            None => return self.refuse(),
        }
        loop {
            let c = self.byte(self.at);
            if c == b'\\' {
                self.escape()?;
            } else if is_ident(c) || self.latin1(self.at) {
                let s = self.at;
                let e = self.ident_run(s)?;
                self.leaf(K::Identifier, s, e);
            } else if c >= 0x80 {
                return self.refuse();
            } else {
                break;
            }
        }
        self.close(n);
        Ok(())
    }

    /// `\` and a hex number (and one white space) or a character.
    fn escape(&mut self) -> R<()> {
        let s = self.at;
        let mut e = s + 1;
        let c = self.byte(e);
        if c.is_ascii_hexdigit() {
            while e < s + 7 && self.byte(e).is_ascii_hexdigit() {
                e += 1;
            }
            if is_blank(self.byte(e)) {
                e += 1;
            }
        } else if c == 0 || c == b'\n' || c == b'\r' || c >= 0x80 {
            return self.refuse();
        } else {
            e += 1;
        }
        self.leaf(K::EscapeSequence, s, e);
        Ok(())
    }

    /// `(` selectors `)`: a pseudo-class's that takes selectors alone.
    fn selector_arguments(&mut self) -> R<()> {
        let s = self.at;
        let n = self.open(K::Arguments, s);
        // Written as `arguments`; its tokens are its rule's.
        let rule = K::PseudoClassWithSelectorArguments;
        self.tok(rule, K::TLParen, 1)?;
        if self.peek()? != b')' {
            loop {
                self.selector()?;
                if self.peek()? == b',' {
                    self.tok(rule, K::TComma, 1)?;
                    continue;
                }
                break;
            }
        }
        self.expect(rule, b')', K::TRParen)?;
        self.close(n);
        Ok(())
    }

    /// `(` selectors or values, between `,`s `)`: a pseudo-class's or a pseudo-element's.
    fn pseudo_arguments(&mut self, kind: K) -> R<()> {
        let s = self.at;
        let n = self.open(K::Arguments, s);
        self.tok(kind, K::TLParen, 1)?;
        if self.peek()? != b')' {
            loop {
                let a = self.skip()?;
                let c = self.byte(a);
                let values = c.is_ascii_digit()
                    || (matches!(c, b'+' | b'-') && (self.byte(a + 1).is_ascii_digit() || self.byte(a + 1) == b'.'))
                    || (c == b'.' && self.byte(a + 1).is_ascii_digit())
                    || matches!(c, b'(' | b'!')
                    || self.identifier_end(a)?.is_some_and(|e| self.byte(e) == b'(');
                if values {
                    self.value()?;
                    while !matches!(self.peek()?, b',' | b')') {
                        self.value()?;
                    }
                } else {
                    self.selector()?;
                }
                if self.peek()? == b',' {
                    self.tok(kind, K::TComma, 1)?;
                    continue;
                }
                break;
            }
        }
        self.expect(kind, b')', K::TRParen)?;
        self.close(n);
        Ok(())
    }

    /// `(` an `An+B` (or `even`, `odd`, a number) [`of` a selector] `)`.
    fn nth_arguments(&mut self) -> R<()> {
        let s = self.at;
        let n = self.open(K::Arguments, s);
        let rule = K::PseudoClassNthChildArguments;
        self.tok(rule, K::TLParen, 1)?;
        let a = self.skip()?;
        if self.is_word(a, b"even") || self.is_word(a, b"odd") {
            let len = if self.byte(a) == b'e' { 4 } else { 3 };
            self.leaf(K::PlainValue, a, a + len);
        } else if let Some(e) = self.nth_end(a) {
            self.leaf(K::PlainValue, a, e);
        } else {
            self.number(false)?;
        }
        let o = self.skip()?;
        if self.is_word(o, b"of") {
            self.tok(rule, K::TOf, 2)?;
            self.selector()?;
        }
        self.expect(rule, b')', K::TRParen)?;
        self.close(n);
        Ok(())
    }

    /// The end of `-?\d*n\s*(\+\s*\d+)?` at `at`.
    fn nth_end(&self, at: usize) -> Option<usize> {
        let mut e = at;
        if self.byte(e) == b'-' {
            e += 1;
        }
        while self.byte(e).is_ascii_digit() {
            e += 1;
        }
        if self.byte(e) != b'n' {
            return None;
        }
        e += 1;
        let after_n = e;
        let mut q = e;
        while is_blank(self.byte(q)) {
            q += 1;
        }
        if self.byte(q) == b'+' {
            let mut r = q + 1;
            while is_blank(self.byte(r)) {
                r += 1;
            }
            if self.byte(r).is_ascii_digit() {
                while self.byte(r).is_ascii_digit() {
                    r += 1;
                }
                return Some(r);
            }
        }
        // The regex's `\s*` after `n` is taken only with a `+` and digits after it.
        let _ = q;
        Some(after_n)
    }

    // ---- Values.

    /// A value: operands and `+`, `-`, `*`, `/`, left-associative.
    fn value(&mut self) -> R<()> {
        self.deeper()?;
        let mut left = self.operand()?;
        loop {
            let s = self.skip()?;
            let op = match self.byte(s) {
                b'*' => Some(K::TStar),
                b'+' | b'-' => {
                    let d = self.byte(s + 1);
                    // `-2`, `+.5` and `-name` are operands, the longer tokens.
                    if d.is_ascii_digit() || (d == b'.' && self.byte(s + 2).is_ascii_digit()) || (self.byte(s) == b'-' && self.identifier_end(s)?.is_some()) {
                        None
                    } else if self.byte(s) == b'-' && self.plain_value_end(s)?.is_some() {
                        None
                    } else if self.byte(s) == b'+' {
                        Some(K::TPlus)
                    } else {
                        Some(K::TMinus)
                    }
                }
                b'/' if self.byte(s + 1) != b'*' && self.byte(s + 1) != b'/' => {
                    if self.plain_value_end(s)?.is_some() { None } else { Some(K::TSlash) }
                }
                _ => None,
            };
            let Some(op) = op else { break };
            let b = self.wrap(left, K::BinaryExpression);
            self.tok(K::BinaryExpression, op, 1)?;
            self.operand()?;
            self.close(b);
            left = b;
        }
        self.depth -= 1;
        Ok(())
    }

    /// One operand of a value.
    fn operand(&mut self) -> R<N> {
        let s = self.skip()?;
        if self.pending.iter().any(|p| p.2) {
            // A `//` comment where a value may follow: a `plain_value` as long as it wins over it.
            return Err(Refusal::Parse(s as u32));
        }
        let c = self.byte(s);
        match c {
            b'"' | b'\'' => self.string_value(),
            b'#' => {
                let n = self.open(K::ColorValue, s);
                self.tok(K::ColorValue, K::THash, 1)?;
                let mut e = s + 1;
                while self.byte(e).is_ascii_hexdigit() {
                    e += 1;
                }
                if e - (s + 1) < 3 || e - (s + 1) > 8 {
                    return Err(Refusal::Parse(s as u32));
                }
                self.at = e;
                self.last_end = e as u32;
                self.close(n);
                Ok(n)
            }
            b'(' => {
                let n = self.open(K::ParenthesizedValue, s);
                self.tok(K::ParenthesizedValue, K::TLParen, 1)?;
                self.value()?;
                self.expect(K::ParenthesizedValue, b')', K::TRParen)?;
                self.close(n);
                Ok(n)
            }
            b'[' => {
                let n = self.open(K::GridValue, s);
                self.tok(K::GridValue, K::TLBracket, 1)?;
                loop {
                    self.value()?;
                    if self.peek()? == b',' {
                        self.tok(K::GridValue, K::TComma, 1)?;
                        continue;
                    }
                    break;
                }
                self.expect(K::GridValue, b']', K::TRBracket)?;
                self.close(n);
                Ok(n)
            }
            b'!' => {
                if self.src.get(s..s + 10) == Some(b"!important") && !is_ident(self.byte(s + 10)) {
                    Ok(self.leaf(K::Important, s, s + 10))
                } else {
                    Err(Refusal::Parse(s as u32))
                }
            }
            _ if c.is_ascii_digit()
                || (c == b'.' && self.byte(s + 1).is_ascii_digit())
                || (matches!(c, b'+' | b'-') && (self.byte(s + 1).is_ascii_digit() || (self.byte(s + 1) == b'.' && self.byte(s + 2).is_ascii_digit()))) =>
            {
                self.number(true)
            }
            _ => {
                let ie = self.identifier_end(s)?;
                let pe = self.plain_value_end(s)?;
                match (ie, pe) {
                    (Some(i), p) if p.is_none_or(|p| p <= i) => {
                        if self.byte(i) == b'(' {
                            self.call(s, i)
                        } else {
                            Ok(self.leaf(K::PlainValue, s, i))
                        }
                    }
                    (_, Some(p)) => Ok(self.leaf(K::PlainValue, s, p)),
                    _ => Err(Refusal::Parse(s as u32)),
                }
            }
        }
    }

    /// A function's name at `s..e` and its arguments: `(` values between `,`s or `;`s `)`.
    fn call(&mut self, s: usize, e: usize) -> R<N> {
        let n = self.open(K::CallExpression, s);
        self.leaf(K::FunctionName, s, e);
        let a = self.open(K::Arguments, e);
        self.tok(K::Arguments, K::TLParen, 1)?;
        if self.peek()? != b')' {
            loop {
                self.value()?;
                match self.peek()? {
                    b',' => self.stored(K::TComma, 1)?, // kept after a value: the gap's state decides
                    b';' => self.tok(K::Arguments, K::TSemi, 1)?,
                    b')' => break,
                    _ => continue,
                }
            }
        }
        self.expect(K::Arguments, b')', K::TRParen)?;
        self.close(a);
        self.close(n);
        Ok(n)
    }

    /// An integer or a float, and its unit right after it.
    /// A number and its unit. In a value (`in_value`), a `plain_value` may follow the number, and
    /// the lexer takes the longer token: a unit that is the start of a longer `plain_value`
    /// (`px/1`) is not the number's.
    fn number(&mut self, in_value: bool) -> R<N> {
        let s = self.skip()?;
        let mut e = s;
        if matches!(self.byte(e), b'+' | b'-') {
            e += 1;
        }
        let digits = e;
        while self.byte(e).is_ascii_digit() {
            e += 1;
        }
        let int_end = if e > digits { Some(e) } else { None };
        // A float: `.digits`, an exponent, or both.
        let exponent = |p: &Self, at: usize| -> Option<usize> {
            if !matches!(p.byte(at), b'e' | b'E') {
                return None;
            }
            let mut q = at + 1;
            if p.byte(q) == b'-' {
                q += 1;
            }
            let d = q;
            while p.byte(q).is_ascii_digit() {
                q += 1;
            }
            (q > d).then_some(q)
        };
        let mut float_end = None;
        if self.byte(e) == b'.' && self.byte(e + 1).is_ascii_digit() {
            let mut q = e + 1;
            while self.byte(q).is_ascii_digit() {
                q += 1;
            }
            float_end = Some(exponent(self, q).unwrap_or(q));
        } else if let Some(q) = exponent(self, e) {
            float_end = Some(q);
        }
        let (kind, end) = match (float_end, int_end) {
            (Some(f), _) => (K::FloatValue, f),
            (None, Some(i)) => (K::IntegerValue, i),
            _ => return Err(Refusal::Parse(s as u32)),
        };
        let n = self.open(kind, s);
        self.at = end;
        self.last_end = end as u32;
        let mut u = end;
        while self.byte(u).is_ascii_alphabetic() || self.byte(u) == b'%' {
            u += 1;
        }
        if u > end && in_value && self.plain_value_end(end)?.is_some_and(|p| p > u) {
            u = end;
        }
        if u > end {
            self.leaf(K::Unit, end, u);
        }
        self.close(n);
        Ok(n)
    }

    /// A quoted string: its pieces (`string_content`, `escape_sequence`) between its quotes.
    fn string_value(&mut self) -> R<N> {
        let s = self.skip()?;
        let q = self.byte(s);
        let qk = if q == b'"' { K::TDQuote } else { K::TQuote };
        let n = self.open(K::StringValue, s);
        // The automata keep the opening quote, and the closing one only right after it (an empty
        // string): the gap's state decides, not the kinds.
        self.stored(qk, 1)?;
        let empty = self.byte(s + 1) == q;
        loop {
            let a = self.at;
            match self.byte(a) {
                c if c == q => break,
                // A line break cannot start a `string_content`, so the lexer skips it (`\s` is an
                // extra) and the string goes on past it.
                b'\n' => self.at = a + 1,
                0 if a >= self.src.len() => return Err(Refusal::Parse(a as u32)),
                b'\\' => self.escape()?,
                _ => {
                    let e = self.b.string_stop(a, q);
                    if e >= self.src.len() {
                        return Err(Refusal::Parse(a as u32));
                    }
                    self.leaf(K::StringContent, a, e);
                }
            }
        }
        if empty {
            self.stored(qk, 1)?;
        } else {
            self.tok(K::StringValue, qk, 1)?;
        }
        self.close(n);
        Ok(n)
    }
}
