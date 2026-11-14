//! The parser: recursive descent over tree-sitter-sql's grammar (`grammar.json`), a statement's
//! nodes built as it reads them (`Arena`), then written to the tape (`emit`).
//!
//! **Tokens** are read as tree-sitter's lexer reads them in the parse state: white space and
//! comments skipped (the external scanner's comments, `--` and nested `/* */`, kept as extras), a
//! word a keyword where the state takes that keyword (tree-sitter's keyword lexing over its `word`
//! token, `_identifier`), an operator the longest the state takes.
//!
//! **Keywords or identifiers.** Where the grammar takes an identifier, the parser names the
//! keywords the state takes too (`Ctx`): a word that is one of them is that keyword, any other word
//! an identifier. Where the parser does not know the state's keywords (after an expression, where
//! everything that may follow it counts), a keyword word it does not read is refused.
//!
//! **The tape.** Each node's children are written in order, its tokens decided by its kind's token
//! automaton (`kinds::MOVES`), exactly as the engine's `TokenGrammar::stored_children` decides them;
//! each extra goes in the deepest node with children on both sides of it.

use crate::block::Blocks;
use crate::kinds::{self, K, f};
use crate::{Refusal, Tape, key};
use rerail_tape_format::{Redo, Widths, Writer};

mod expr;
mod stmt;

pub(crate) type R<T> = Result<T, Refusal>;

/// The deepest nesting the parser follows (the stack's bound).
const MAX_DEPTH: u32 = 160;

/// `src` parsed.
pub fn parse(src: &[u8]) -> R<Tape> {
    if let Some(at) = src.iter().position(|&c| c == 0) {
        return Err(Refusal::Lex(at as u32));
    }
    if std::str::from_utf8(src).is_err() {
        return Err(Refusal::Lex(0));
    }
    // A leading byte order mark is skipped (tree-sitter's lexer skips it): the text starts after.
    let start = if src.starts_with("\u{feff}".as_bytes()) { 3 } else { 0 };
    let mut p = P { src, b: Blocks::of(src), at: start, last_end: start, look: None, a: Arena::default(), extras: Vec::new(), depth: 0, lower: [0; 48] };
    let root = p.program()?;
    let mut widths = Widths::guess(src.len());
    loop {
        let mut w = Writer::new(0, widths, p.a.nodes.len() + p.extras.len() + 1);
        let mut x = 0usize;
        p.a.emit(&mut w, root, &p.extras, &mut x, true, src.len() as u32)?;
        match w.finish_columns() {
            Ok(columns) => return Ok(columns),
            Err(Redo(nw)) => {
                if nw == widths {
                    return Err(Refusal::TooLarge);
                }
                widths = nw;
            }
        }
    }
}

// ---- Tokens.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum T {
    Eof,
    /// A word: an identifier or a keyword, as the parse state takes it.
    Word,
    /// `"…"`: an identifier.
    Quoted,
    /// `'…'`, with `N'…'`: a string (its continuations are read by `literal`).
    Str,
    /// `E'…'`: an escape string.
    EStr,
    /// `$tag$…$tag$`.
    Dollar,
    /// A number: an integer or a decimal one.
    Num,
    /// `$1`: a parameter.
    Param,
    /// Anything else: punctuation and operators, which the parser reads by their bytes.
    Punct,
}

#[derive(Clone, Copy, Debug)]
struct Tok {
    t: T,
    s: usize,
    e: usize,
}

#[inline(always)]
fn is_blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

#[inline(always)]
fn is_word_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

/// The binary operators' precedences (the grammar's `precedences`, the first the highest), as
/// levels: the higher binds the tighter.
mod prec {
    pub const OR: u8 = 1;
    pub const AND: u8 = 2;
    pub const BETWEEN: u8 = 3;
    pub const PATTERN: u8 = 4;
    pub const RELATION: u8 = 6;
    pub const IN: u8 = 8;
    pub const OTHER: u8 = 9;
    pub const PLUS: u8 = 11;
    pub const TIMES: u8 = 12;
    pub const EXP: u8 = 13;
    pub const NOT: u8 = 14;
    pub const IS: u8 = 15;
    pub const MINUS: u8 = 19;
    pub const SUBSCRIPT: u8 = 20;
    pub const TYPECAST: u8 = 21;
}

// ---- The arena: a statement's nodes as they are read, written to the tape after.

#[derive(Clone, Copy)]
struct N {
    kind: K,
    field: u8,
    s: u32,
    e: u32,
    /// Its children: `kids[at..at + n]`.
    at: u32,
    n: u32,
}

#[derive(Default)]
struct Arena {
    nodes: Vec<N>,
    kids: Vec<u32>,
    /// The children of the nodes being read, in order.
    stack: Vec<u32>,
}

/// A node's label in its parent's automaton: its kind and field.
#[inline(always)]
fn label(n: &N) -> u32 {
    n.kind as u32 | u32::from(n.field) << 10
}

/// The moves out of automaton state `s`.
#[inline]
fn moves(s: u32) -> &'static [(u32, u32)] {
    &kinds::MOVES[kinds::MOVES_AT[s as usize] as usize..kinds::MOVES_AT[s as usize + 1] as usize]
}

#[inline]
fn step(s: u32, l: u32) -> Option<u32> {
    let m = moves(s);
    m.binary_search_by_key(&l, |&(x, _)| x).ok().map(|i| m[i].1)
}

#[inline]
fn accepts(s: u32) -> bool {
    kinds::ACCEPTS[s as usize / 64] >> (s % 64) & 1 == 1
}

/// The implied word from `s` to the named child `next` (`None`: to the end), where it holds a token
/// the tape can leave out.
fn implied(s: u32, next: Option<u32>) -> Option<&'static [u32]> {
    let (at, n) = match next {
        Some(l) => {
            let i = kinds::IMPLIED.binary_search_by(|&(a, b, _, _)| (a, b).cmp(&(s, l))).ok()?;
            (kinds::IMPLIED[i].2, kinds::IMPLIED[i].3)
        }
        None => {
            let i = kinds::IMPLIED_END.binary_search_by_key(&s, |&(a, _, _)| a).ok()?;
            (kinds::IMPLIED_END[i].1, kinds::IMPLIED_END[i].2)
        }
    };
    Some(&kinds::WORD[at as usize..(at + n) as usize])
}

impl Arena {
    #[inline]
    fn mark(&self) -> usize {
        self.stack.len()
    }

    /// A leaf at `s..e`, pushed as the next child.
    #[inline]
    fn leaf(&mut self, kind: K, field: u8, s: usize, e: usize) -> u32 {
        let id = self.nodes.len() as u32;
        self.nodes.push(N { kind, field, s: s as u32, e: e as u32, at: 0, n: 0 });
        self.stack.push(id);
        id
    }

    /// A node of the children read since `mark`, pushed as the next child in their place.
    fn node(&mut self, kind: K, mark: usize) -> u32 {
        let at = self.kids.len() as u32;
        let n = (self.stack.len() - mark) as u32;
        let (s, e) = match (self.stack.get(mark), self.stack.last()) {
            (Some(&a), Some(&b)) => (self.nodes[a as usize].s, self.nodes[b as usize].e),
            _ => (0, 0),
        };
        self.kids.extend(self.stack.drain(mark..));
        let id = self.nodes.len() as u32;
        self.nodes.push(N { kind, field: 0, s, e, at, n });
        self.stack.push(id);
        id
    }

    /// Gives node `id` `field`, where it has none: a field given inside a hidden rule wins over
    /// one given outside it, as tree-sitter's cursor looks for a node's field from its nearest
    /// hidden parent out.
    #[inline]
    fn set_field(&mut self, id: u32, field: u8) {
        let n = &mut self.nodes[id as usize];
        if n.field == 0 {
            n.field = field;
        }
    }

    /// The children read since `mark` given `field` (a hidden rule's field falls on its nodes),
    /// where they have none.
    fn field_since(&mut self, mark: usize, field: u8) {
        for i in mark..self.stack.len() {
            let id = self.stack[i];
            self.set_field(id, field);
        }
    }

    /// Which of node `id`'s children the tape stores: its automaton's gaps, as the engine's
    /// `stored_children` decides them. `None`: all of them (no automaton, or a walk it refuses).
    fn stored(&self, id: u32) -> Option<Vec<bool>> {
        let n = self.nodes[id as usize];
        let a = kinds::AUTOMATON[n.kind as usize];
        if a == 0 || n.n == 0 {
            return None;
        }
        let a = a - 1;
        let kept = |l: u32| kinds::KEPT.binary_search(&(a, l)).is_ok();
        let kids = &self.kids[n.at as usize..(n.at + n.n) as usize];
        let mut stored = vec![true; kids.len()];
        let mut state = kinds::START[a as usize];
        let mut gap: Vec<usize> = Vec::new();
        let close = |state: u32, gap: &[usize], next: Option<u32>, stored: &mut [bool]| -> Option<u32> {
            let mut at = state;
            for &i in gap {
                at = step(at, label(&self.nodes[kids[i] as usize]))?;
            }
            let after = match next {
                Some(l) => step(at, l)?,
                None if accepts(at) => at,
                None => return None,
            };
            if !gap.is_empty()
                && let Some(word) = implied(state, next)
                && word.len() == gap.len()
                && word.iter().zip(gap).all(|(&w, &i)| w == label(&self.nodes[kids[i] as usize]))
            {
                for &i in gap {
                    if !kept(label(&self.nodes[kids[i] as usize])) {
                        stored[i] = false;
                    }
                }
            }
            Some(after)
        };
        for (i, &k) in kids.iter().enumerate() {
            let c = &self.nodes[k as usize];
            if kinds::KINDS[c.kind as usize].1 {
                state = close(state, &gap, Some(label(c)), &mut stored)?;
                gap.clear();
            } else {
                gap.push(i);
            }
        }
        close(state, &gap, None, &mut stored)?;
        Some(stored)
    }

    /// Writes node `id` and its subtree, the extras from `x` on that fall between its children
    /// placed among them. `root`: the program, whose extras before its first child and after its
    /// last are its own.
    fn emit(&self, w: &mut Writer, id: u32, extras: &[(K, u32, u32)], x: &mut usize, root: bool, end: u32) -> R<()> {
        let n = self.nodes[id as usize];
        let start = if root {
            let first = if n.n == 0 { end } else { n.s };
            extras.first().map_or(first, |e| e.1.min(first))
        } else {
            n.s
        };
        let at = w.push(key(n.kind, n.field, false), start);
        let stored = self.stored(id);
        let kids = &self.kids[n.at as usize..(n.at + n.n) as usize];
        for (i, &k) in kids.iter().enumerate() {
            let c = self.nodes[k as usize];
            if i > 0 || root {
                while let Some(&(xk, xs, xe)) = extras.get(*x)
                    && xs < c.s
                {
                    let e = w.push(key(xk, 0, true), xs);
                    w.close(e, xe);
                    *x += 1;
                }
            }
            if stored.as_ref().is_some_and(|s| !s[i]) {
                continue;
            }
            self.emit(w, k, extras, x, false, end)?;
        }
        if root {
            while let Some(&(xk, xs, xe)) = extras.get(*x) {
                let e = w.push(key(xk, 0, true), xs);
                w.close(e, xe);
                *x += 1;
            }
        }
        w.close(at, if root { end } else { n.e });
        Ok(())
    }
}

/// Where the grammar takes an identifier, the keywords the parse state takes too: `Sure`, the
/// parser knows them all (any other keyword word is an identifier); `Unsure`, it does not (a
/// keyword word not among them is refused).
#[derive(Clone, Copy)]
enum Ctx<'a> {
    Sure(&'a [&'a [K]]),
    Unsure(&'a [&'a [K]]),
}

const NONE: Ctx<'static> = Ctx::Sure(&[]);

pub(crate) struct P<'s> {
    src: &'s [u8],
    b: Blocks<'s>,
    /// Where the next token's white space starts (after the last token read).
    at: usize,
    /// The last token's end.
    last_end: usize,
    look: Option<Tok>,
    a: Arena,
    /// Comments, in order: (kind, start, end).
    extras: Vec<(K, u32, u32)>,
    depth: u32,
    lower: [u8; 48],
}

impl<'s> P<'s> {
    #[inline(always)]
    fn byte(&self, at: usize) -> u8 {
        self.src.get(at).copied().unwrap_or(0)
    }

    fn refuse<X>(&self) -> R<X> {
        Err(Refusal::Parse(self.look.map_or(self.at, |t| t.s) as u32))
    }

    fn deeper(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { self.refuse() } else { Ok(()) }
    }

    // ---- The lexer.

    /// Skips white space and comments (kept as extras) from `self.at`: the next token's start.
    fn skip(&mut self) -> R<usize> {
        let mut at = self.at;
        loop {
            at = self.b.blank_end(at);
            match (self.byte(at), self.byte(at + 1)) {
                (b'-', b'-') => {
                    let e = self.b.line_end(at);
                    self.extra(K::Comment, at, e);
                    at = e;
                }
                (b'/', b'*') => {
                    let Some(e) = self.b.comment_end(at + 2) else { return Err(Refusal::Parse(at as u32)) };
                    self.extra(K::Comment, at, e);
                    at = e;
                }
                // A psql meta-command: an extra, or (a command that sends the query buffer) the token
                // `program` takes in a statement's `;` place.
                (b'\\', _) => match self.meta_command(at)? {
                    Some(e) => {
                        self.extra(K::MetaCommand, at, e);
                        at = e;
                    }
                    None => return Ok(at),
                },
                _ => return Ok(at),
            }
        }
    }

    /// The psql meta-command at `at` (its `\\`), as the scanner reads it: its end, where it is an
    /// extra; `None` for a command that sends the query buffer (`\\g`, `\\gset`, …), which the
    /// grammar's `_meta_command_send` token reads (`send_end`).
    fn meta_command(&self, at: usize) -> R<Option<usize>> {
        let mut e = at + 1;
        while let Some(&c) = self.src.get(e) {
            if is_blank(c) || c == b'\\' {
                break;
            }
            e += 1;
        }
        let name = &self.src[at + 1..e];
        if name.is_empty() {
            return Err(Refusal::Parse(at as u32));
        }
        if name.len() <= 16 && [&b"g"[..], b"gx", b"gset", b"gexec", b"gdesc", b"crosstabview", b"watch"].contains(&name) {
            return Ok(None);
        }
        if name == b"set" || name == b"copy" {
            if name == b"copy" {
                let mut x = e;
                while matches!(self.byte(x), b' ' | b'\t' | 0x0b | 0x0c) {
                    x += 1;
                }
                if self.byte(x) == b'(' {
                    return Err(Refusal::Parse(at as u32));
                }
            }
            let mut x = e;
            while x < self.src.len() && !matches!(self.src[x], b'\n' | b'\r') {
                x += 1;
            }
            return Ok(Some(x));
        }
        let mut x = e;
        while x < self.src.len() && !matches!(self.src[x], b'\n' | b'\r') {
            if self.src[x] == b'\\' {
                return Ok(Some(if self.byte(x + 1) == b'\\' { x + 2 } else { x }));
            }
            x += 1;
        }
        Ok(Some(x))
    }

    /// The end of the send command at `at` (`_meta_command_send`: `\\g`, its arguments after a
    /// blank, and a closing `\\\\`).
    fn send_end(&self, at: usize) -> usize {
        let mut e = at + 1;
        while self.byte(e).is_ascii_alphabetic() {
            e += 1;
        }
        if matches!(self.byte(e), b' ' | b'\t' | 0x0b | 0x0c) {
            while e < self.src.len() && !matches!(self.src[e], b'\n' | b'\r' | b'\\') {
                e += 1;
            }
        }
        if self.byte(e) == b'\\' && self.byte(e + 1) == b'\\' {
            e += 2;
        }
        e
    }

    fn extra(&mut self, kind: K, s: usize, e: usize) {
        // A token read twice (a look ahead, then again) adds its comments once.
        if self.extras.last().is_none_or(|x| (x.1 as usize) < s) {
            self.extras.push((kind, s as u32, e as u32));
        }
    }

    /// The next token.
    fn peek(&mut self) -> R<Tok> {
        if let Some(t) = self.look {
            return Ok(t);
        }
        let s = self.skip()?;
        let c = self.byte(s);
        let tok = |t, e| Tok { t, s, e };
        let t = if s >= self.src.len() {
            tok(T::Eof, s)
        } else if is_word_start(c) {
            let e = self.b.word_end(s);
            if e - s == 1 && self.byte(e) == b'\'' {
                match c | 0x20 {
                    b'n' => tok(T::Str, self.quoted_end(e)?),
                    b'e' => tok(T::EStr, self.escape_end(e)?),
                    _ => return self.refuse(),
                }
            } else if self.byte(e) == b'&' && (c | 0x20) == b'u' && e - s == 1 {
                return self.refuse();
            } else {
                self.check_word(s, e)?;
                tok(T::Word, e)
            }
        } else if c == b'"' {
            let mut e = s + 1;
            loop {
                let q = self.b.dquote_stop(e);
                if q >= self.src.len() {
                    return Err(Refusal::Parse(s as u32));
                }
                if self.byte(q + 1) == b'"' {
                    e = q + 2;
                } else {
                    break tok(T::Quoted, q + 1);
                }
            }
        } else if c == b'\'' {
            tok(T::Str, self.quoted_end(s)?)
        } else if c.is_ascii_digit() || c == b'.' && self.byte(s + 1).is_ascii_digit() {
            let e = self.number_end(s)?;
            tok(T::Num, e)
        } else if c == b'$' {
            let d = self.b.word_end(s + 1);
            if d > s + 1 && self.src[s + 1..d].iter().all(u8::is_ascii_digit) {
                tok(T::Param, d)
            } else {
                tok(T::Dollar, s)
            }
        } else {
            tok(T::Punct, s)
        };
        self.look = Some(t);
        Ok(t)
    }

    /// A word's bytes past ASCII: no white space (tree-sitter's `\s` takes Unicode's, which a word
    /// here would not split at).
    fn check_word(&self, s: usize, e: usize) -> R<()> {
        if e - s > 0 && self.src[s..e].iter().any(|&b| b >= 0x80) {
            let w = std::str::from_utf8(&self.src[s..e]).map_err(|_| Refusal::Lex(s as u32))?;
            if w.chars().any(|c| c.is_whitespace() || c == '\u{feff}' || c == '\u{200b}') {
                return Err(Refusal::Lex(s as u32));
            }
        }
        Ok(())
    }

    /// The end of the string whose quote is at `q`: `'…'`, a `''` inside it a quote.
    fn quoted_end(&self, q: usize) -> R<usize> {
        let mut e = q + 1;
        loop {
            let x = self.b.quote_stop(e);
            if x >= self.src.len() {
                return Err(Refusal::Parse(q as u32));
            }
            if self.src[x] == b'\\' {
                e = x + 1;
                continue;
            }
            if self.byte(x + 1) == b'\'' {
                e = x + 2;
            } else {
                return Ok(x + 1);
            }
        }
    }

    /// The end of the escape string whose quote is at `q`: `\` escapes the next byte.
    fn escape_end(&self, q: usize) -> R<usize> {
        let mut e = q + 1;
        loop {
            let x = self.b.quote_stop(e);
            if x >= self.src.len() {
                return Err(Refusal::Parse(q as u32));
            }
            if self.src[x] == b'\\' {
                e = x + 2;
                continue;
            }
            if self.byte(x + 1) == b'\'' {
                e = x + 2;
            } else {
                return Ok(x + 1);
            }
        }
    }

    /// A number's end: `_integer`'s and `_decimal_number`'s patterns (decimal digits, with `_`
    /// between them, a fraction, an exponent). A number a word byte follows, or a hexadecimal,
    /// octal or binary one, is refused.
    fn number_end(&self, s: usize) -> R<usize> {
        let digits = |mut at: usize| {
            let from = at;
            while self.byte(at).is_ascii_digit() || self.byte(at) == b'_' && self.byte(at + 1).is_ascii_digit() && at > from {
                at += 1;
            }
            at
        };
        if self.byte(s) == b'0' && matches!(self.byte(s + 1) | 0x20, b'x' | b'o' | b'b') {
            return Err(Refusal::Parse(s as u32));
        }
        let mut e = digits(s);
        if self.byte(e) == b'.' {
            e = digits(e + 1);
        }
        if matches!(self.byte(e), b'e' | b'E') {
            let mut x = e + 1;
            if matches!(self.byte(x), b'+' | b'-') {
                x += 1;
            }
            if self.byte(x).is_ascii_digit() {
                e = digits(x);
            }
        }
        if is_word_start(self.byte(e)) || self.byte(e).is_ascii_digit() || self.byte(e) == b'$' || self.byte(e) == b'.' {
            return Err(Refusal::Parse(s as u32));
        }
        Ok(e)
    }

    /// Takes the token `t` (from `peek`).
    #[inline]
    fn take(&mut self, t: Tok) {
        self.at = t.e;
        self.last_end = t.e;
        self.look = None;
    }

    /// The keywords the next token, a word, is (`kinds::WORDS`), lower case; empty for any other.
    fn words(&mut self) -> R<&'static [(&'static str, K)]> {
        let t = self.peek()?;
        if t.t != T::Word || t.e - t.s > self.lower.len() {
            return Ok(&[]);
        }
        let n = t.e - t.s;
        for i in 0..n {
            self.lower[i] = self.src[t.s + i].to_ascii_lowercase();
        }
        let w = &self.lower[..n];
        let lo = kinds::WORDS.partition_point(|(x, _)| x.as_bytes() < w);
        let hi = lo + kinds::WORDS[lo..].iter().take_while(|(x, _)| x.as_bytes() == w).count();
        Ok(&kinds::WORDS[lo..hi])
    }

    /// Whether the next token is keyword `k`.
    fn is(&mut self, k: K) -> R<bool> {
        Ok(self.words()?.iter().any(|&(_, x)| x == k))
    }

    /// Keyword `k`, read as a leaf, where it is the next token.
    fn kw(&mut self, k: K) -> R<Option<u32>> {
        if !self.is(k)? {
            return Ok(None);
        }
        let t = self.peek()?;
        self.take(t);
        Ok(Some(self.a.leaf(k, 0, t.s, t.e)))
    }

    fn expect(&mut self, k: K) -> R<u32> {
        match self.kw(k)? {
            Some(id) => Ok(id),
            None => self.refuse(),
        }
    }

    /// Keyword `k` given `field`.
    fn kw_field(&mut self, k: K, field: u8) -> R<Option<u32>> {
        let id = self.kw(k)?;
        if let Some(id) = id {
            self.a.set_field(id, field);
        }
        Ok(id)
    }

    /// Whether the next token is punctuation `p` (its bytes), and no longer operator.
    fn at_punct(&mut self, p: &[u8]) -> R<bool> {
        let t = self.peek()?;
        Ok(t.t == T::Punct && self.src[t.s..].starts_with(p) && self.op_len(t.s, p.len()))
    }

    /// Whether the token at `s` is the `n` bytes there, as the lexer takes it: no longer operator
    /// (`op_other`, `<=`, `::`) continues it.
    fn op_len(&self, s: usize, n: usize) -> bool {
        let c = self.src[s];
        match c {
            b'(' | b')' | b',' | b';' | b'[' | b']' | b'.' => n == 1,
            b':' => n == 2 || self.byte(s + 1) != b':' && self.byte(s + 1) != b'=',
            _ => self.operator_at(s).1 == n,
        }
    }

    /// Punctuation `p` as an anonymous token leaf, where it is next.
    fn punct(&mut self, p: &[u8], kind: K) -> R<Option<u32>> {
        if !self.at_punct(p)? {
            return Ok(None);
        }
        let t = self.peek()?;
        let e = t.s + p.len();
        self.take(Tok { e, ..t });
        Ok(Some(self.a.leaf(kind, 0, t.s, e)))
    }

    fn expect_punct(&mut self, p: &[u8], kind: K) -> R<u32> {
        match self.punct(p, kind)? {
            Some(id) => Ok(id),
            None => self.refuse(),
        }
    }

    /// The operator at `s` as the lexer reads it after an expression: the longest of the string
    /// operators and `op_other`'s two patterns, a string winning a tie. (whether it is
    /// `op_other`, its length).
    fn operator_at(&self, s: usize) -> (bool, usize) {
        const A: &[u8] = b"+-*/<>=";
        const B: &[u8] = b"~!@#%^&|`?";
        const C: &[u8] = b"*/<>=";
        // The run of operator bytes.
        let mut run = s;
        let mut has_b = false;
        while let Some(&c) = self.src.get(run) {
            if A.contains(&c) {
            } else if B.contains(&c) {
                has_b = true;
            } else {
                break;
            }
            // A comment starts no operator's byte after the first (the scanner reads it first only at a
            // token's start): `*/*` is one operator, as tree-sitter's lexer reads it.
            run += 1;
        }
        let p1 = if has_b { run - s } else { 0 };
        let mut a_run = s;
        while self.src.get(a_run).is_some_and(|c| A.contains(c)) {
            a_run += 1;
        }
        let mut p2 = 0;
        let mut k = a_run;
        while k >= s + 2 {
            if C.contains(&self.src[k - 1]) {
                p2 = k - s;
                break;
            }
            k -= 1;
        }
        let strings: &[&[u8]] = &[b"<=", b"!=", b">=", b"<>", b"::", b"+", b"-", b"*", b"/", b"%", b"^", b"=", b"<", b">"];
        let st = strings.iter().filter(|x| self.src[s..].starts_with(x)).map(|x| x.len()).max().unwrap_or(0);
        let pat = p1.max(p2);
        if pat > st { (true, pat) } else { (false, st) }
    }

    // ---- Identifiers.

    /// An identifier, where the next token is one in `ctx` (see `Ctx`); `None` where it is not.
    fn ident(&mut self, ctx: Ctx, field: u8) -> R<Option<u32>> {
        let t = self.peek()?;
        match t.t {
            T::Quoted => {}
            T::Word => {
                let ws = self.words()?;
                if !ws.is_empty() {
                    let (sets, sure) = match ctx {
                        Ctx::Sure(s) => (s, true),
                        Ctx::Unsure(s) => (s, false),
                    };
                    if ws.iter().any(|&(_, k)| sets.iter().any(|set| set.contains(&k))) {
                        return Ok(None);
                    }
                    if !sure {
                        return self.refuse();
                    }
                }
            }
            _ => return Ok(None),
        }
        self.take(t);
        Ok(Some(self.a.leaf(K::Identifier, field, t.s, t.e)))
    }

    fn expect_ident(&mut self, ctx: Ctx, field: u8) -> R<u32> {
        match self.ident(ctx, field)? {
            Some(id) => Ok(id),
            None => self.refuse(),
        }
    }

    /// `object_reference`: `name`, `schema.name` or `database.schema.name`, the first part an
    /// identifier in `ctx`. `None` where the next token is no identifier.
    fn object_reference(&mut self, ctx: Ctx, field: u8) -> R<Option<u32>> {
        let m = self.a.mark();
        let Some(first) = self.ident(ctx, f::NAME)? else { return Ok(None) };
        let mut parts = vec![first];
        while self.at_punct(b".")? {
            let t = self.peek()?;
            let save = (self.at, self.last_end, self.look);
            self.take(Tok { e: t.s + 1, ..t });
            if let Some(id) = self.ident(NONE, f::NAME)? {
                parts.push(id);
            } else {
                // `a.*`, `a.(`: not this object reference's.
                (self.at, self.last_end, self.look) = save;
                break;
            }
            if parts.len() > 3 {
                return self.refuse();
            }
        }
        // The dots between the parts are tokens: their places are known (each part ends where its dot
        // starts, white space aside).
        self.a.stack.truncate(m);
        self.push_reference(&parts, field);
        Ok(Some(*self.a.stack.last().expect("pushed")))
    }

    /// Pushes an `object_reference` of identifier leaves `parts` (already in the arena), with the
    /// dots between them.
    fn push_reference(&mut self, parts: &[u32], field: u8) {
        let m = self.a.mark();
        let fields: &[u8] = match parts.len() {
            1 => &[f::NAME],
            2 => &[f::SCHEMA, f::NAME],
            _ => &[f::DATABASE, f::SCHEMA, f::NAME],
        };
        for (i, &p) in parts.iter().enumerate() {
            if i > 0 {
                let d = self.dot_before(p);
                self.a.leaf(K::TDot, 0, d, d + 1);
            }
            self.a.nodes[p as usize].field = fields[i];
            self.a.stack.push(p);
        }
        let id = self.a.node(K::ObjectReference, m);
        self.a.set_field(id, field);
    }

    /// The `.` before part `p`: the last one before its start.
    fn dot_before(&self, p: u32) -> usize {
        let mut at = self.a.nodes[p as usize].s as usize;
        while at > 0 {
            at -= 1;
            if self.src[at] == b'.' {
                return at;
            }
        }
        0
    }
}
