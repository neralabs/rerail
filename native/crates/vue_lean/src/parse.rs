//! The parser: tree-sitter-vue's grammar and its external scanner (`scanner.c`), read in one pass
//! over the file, writing the tape as it goes.
//!
//! tree-sitter-vue's tree is decided by its scanner as much as by its grammar:
//!
//! - **Texts.** A text is the scanner's `_text_fragment`: it runs to a `<` that a letter, `!`,
//!   `?` or `/` follows, or to `{{`; at a line's end, it ends before the line break where white
//!   space and then `<` follow. White space it starts with is the text's, unless a line break
//!   comes before any other byte (`text`).
//! - **Elements** end where their end tag comes, or implicitly (`_implicit_end_tag`, a token of no
//!   width) where the next tag may not stand inside them, where an end tag closes an element
//!   further out, or where a void element is followed by `<`, as in tree-sitter-html, whose tag
//!   rules the scanner shares (`rerail_html_lean::tags`). A `<template>` element is the grammar's
//!   own (`template_element`) and never ends implicitly; nothing ends at the file's end, where an
//!   open element is an error.
//! - **Directives** (`v-if`, `:x`, `@click.stop`, `#slot`) are the grammar's: a name or a shorthand
//!   and its argument, modifiers, and a value. Shorthands without a value run on into the next
//!   shorthand (`:a :b` is one attribute), as the grammar's `repeat1` reads them.
//!
//! **Stored tokens.** Each node with an automaton (`kinds::AUTOMATON`) keeps its tokens pending
//! until its next named child, or its end, is known; then the gap is decided as the engine's
//! `stored_children` decides it: the tokens of the gap's implied word are left out, any other
//! stored. No token is written and then removed.

use crate::block::Blocks;
use crate::kinds::{self, K};
use crate::{Refusal, Tape, key};
use rerail_html_lean::tags;
use rerail_simd8::swar::load8;
use rerail_tape_format::{Redo, Sink, Widths, Writer};

type R<T> = Result<T, Refusal>;

/// What an open element holds: other nodes, which an implicit end tag may end (`element`); nodes
/// no implicit end tag ends (`template_element`); or raw text (a script or a style).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Holds {
    Nodes,
    Template,
    Raw,
}

/// An open element: its tag (a type from `tags`, and its name's bytes for a custom one), what it
/// holds, and its node.
#[derive(Clone, Copy)]
struct Open {
    ty: u8,
    name: (u32, u32),
    holds: Holds,
    node: usize,
}

/// A node open in the writer: its automaton (`kinds::AUTOMATON`, 0: none, its tokens stored), the
/// automaton's state at the start of the open gap, and where the gap's tokens start in `pending`.
#[derive(Clone, Copy)]
struct Frame {
    node: usize,
    auto: u16,
    state: u32,
    gap: usize,
}

/// What a text scan found (the scanner's `_text_fragment` and `_interpolation_text`).
enum Scan {
    /// A text, from and to.
    Text(usize, usize),
    /// An interpolation's text, from and to.
    Interpolation(usize, usize),
    /// None: the next token starts here.
    Stop(usize),
    /// None: the file ended first.
    End,
}

struct P<'a> {
    src: &'a [u8],
    b: &'a Blocks,
    w: Writer<Widths>,
    open: Vec<Open>,
    /// The open tags' size as the scanner serializes them (`tags::serialized_size`).
    serialized: usize,
    frames: Vec<Frame>,
    /// The open gap's tokens: kind, from, to.
    pending: Vec<(K, u32, u32)>,
    at: usize,
}

/// `src` parsed.
pub fn parse(src: &[u8]) -> R<Tape> {
    parse_into(src, &mut Writer::<Widths>::finish_columns)
}

/// [`parse`], the writer finished by `finish`: format 15's columns, or format 16 (decision 0293).
pub fn parse_into<Out>(src: &[u8], finish: &mut dyn Sink<Out>) -> R<Out> {
    // The scanner reads a NUL as the text's end.
    if src.contains(&0) {
        return Err(Refusal::Lex(src.iter().position(|&c| c == 0).unwrap_or(0) as u32));
    }
    let b = Blocks::of(src);
    let mut widths = Widths::guess(src.len());
    loop {
        let mut p = P {
            src,
            b: &b,
            w: Writer::new(0, widths, src.len() / 4 + 16).to(finish.target(), src).omitting(finish.omissions()),
            open: Vec::new(),
            serialized: tags::SERIALIZED_EMPTY,
            frames: Vec::new(),
            pending: Vec::new(),
            at: 0,
        };
        p.document()?;
        match finish.finish(p.w) {
            Ok(done) => return Ok(done),
            Err(Redo(w)) => {
                if w == widths && !finish.again() {
                    return Err(Refusal::TooLarge);
                }
                widths = w;
            }
        }
    }
}

/// The state automaton state `s` moves to on `label`.
#[inline]
fn step(s: u32, label: u32) -> Option<u32> {
    let moves = &kinds::MOVES[kinds::MOVES_AT[s as usize] as usize..kinds::MOVES_AT[s as usize + 1] as usize];
    moves.iter().find(|&&(l, _)| l == label).map(|&(_, to)| to)
}

#[inline]
fn accepts(s: u32) -> bool {
    kinds::ACCEPTS[s as usize / 64] >> (s % 64) & 1 == 1
}

/// The implied word from `s` to the named child `next` (`None`: to the end), packed
/// (`kinds::IMPLIED`), 0 for none.
#[inline]
fn implied(s: u32, next: Option<u32>) -> u32 {
    match next {
        Some(l) => {
            let rows = &kinds::IMPLIED[kinds::IMPLIED_AT[s as usize] as usize..kinds::IMPLIED_AT[s as usize + 1] as usize];
            rows.iter().find(|&&(x, _)| x == l).map_or(0, |&(_, w)| w)
        }
        None => kinds::IMPLIED_END[s as usize],
    }
}

/// Whether the character at `src[at]`, a byte past ASCII, is white space (Unicode's, as the
/// grammar's `\s` reads it; the scanner's `iswspace` may too).
fn wide_space(src: &[u8], at: usize) -> bool {
    let end = (at + 4).min(src.len());
    let head = match std::str::from_utf8(&src[at..end]) {
        Ok(s) => s,
        Err(e) => std::str::from_utf8(&src[at..at + e.valid_up_to()]).unwrap_or(""),
    };
    head.chars().next().is_some_and(char::is_whitespace)
}

/// `w` with ASCII's lower-case letters made upper case, a byte at a time in one word.
#[inline]
fn upper(w: u64) -> u64 {
    const ONES: u64 = 0x0101_0101_0101_0101;
    let low = w & (0x7f * ONES);
    let from_a = low + (0x80 - u64::from(b'a')) * ONES;
    let past_z = low + (0x80 - u64::from(b'z') - 1) * ONES;
    let lower = (from_a ^ past_z) & !w & (0x80 * ONES);
    w ^ (lower >> 2)
}

impl<'a> P<'a> {
    fn refuse<T>(&self) -> R<T> {
        Err(Refusal::Parse(self.at as u32))
    }

    #[inline(always)]
    fn byte(&self, at: usize) -> u8 {
        self.src.get(at).copied().unwrap_or(0)
    }

    /// Past ASCII white space from `at`; one past ASCII that is white space is refused.
    #[inline]
    fn blanks(&self, at: usize) -> R<usize> {
        let e = self.b.blank_end(at);
        if self.byte(e) >= 0x80 && wide_space(self.src, e) {
            return Err(Refusal::Lex(e as u32));
        }
        Ok(e)
    }

    /// Refuses a token `from..to` that holds a white space past ASCII (where the grammar's `\s`
    /// would end it), looked for only where the token has a byte past ASCII.
    fn no_wide_space(&self, from: usize, to: usize) -> R<()> {
        let mut h = self.b.high(from);
        while h < to {
            if wide_space(self.src, h) {
                return Err(Refusal::Lex(h as u32));
            }
            // The next character past ASCII: past this one's bytes (its lead byte's length).
            h = self.b.high(h + (self.src[h].leading_ones() as usize).max(1));
        }
        Ok(())
    }

    // ---- The writer.

    /// A named child of the innermost node comes: its open gap is decided.
    #[inline]
    fn named(&mut self, kind: K) -> R<()> {
        self.flush(Some(kind as u32))
    }

    fn open_node(&mut self, kind: K, from: usize) -> R<usize> {
        self.named(kind)?;
        let node = self.w.push(key(kind, false), from as u32);
        let auto = kinds::AUTOMATON[kind as usize];
        let state = if auto > 0 { kinds::START[usize::from(auto) - 1] } else { 0 };
        self.frames.push(Frame { node, auto, state, gap: self.pending.len() });
        Ok(node)
    }

    fn leaf(&mut self, kind: K, from: usize, to: usize) -> R<()> {
        self.named(kind)?;
        let at = self.w.push(key(kind, false), from as u32);
        self.w.close(at, to as u32);
        Ok(())
    }

    /// Token `tok` at `from..to`, in the innermost node's open gap.
    #[inline]
    fn tok(&mut self, tok: K, from: usize, to: usize) {
        self.pending.push((tok, from as u32, to as u32));
    }

    fn close(&mut self, node: usize, end: usize) -> R<()> {
        self.flush(None)?;
        let frame = self.frames.pop().expect("an open node");
        debug_assert_eq!(frame.node, node);
        self.w.close(node, end as u32);
        Ok(())
    }

    /// The innermost node's open gap, before its named child of label `next` (`None`: its end):
    /// the tokens of the gap's implied word left out, the others written.
    fn flush(&mut self, next: Option<u32>) -> R<()> {
        let Some(frame) = self.frames.last().copied() else { return Ok(()) };
        let gap = &self.pending[frame.gap..];
        let mut left_out = 0u32;
        if frame.auto > 0 {
            let w = implied(frame.state, next);
            if !gap.is_empty() && w != 0 && (w >> 8 & 0xf) as usize == gap.len() {
                let word = &kinds::WORD[(w >> 12) as usize..(w >> 12) as usize + gap.len()];
                if word.iter().zip(gap).all(|(&l, t)| l == t.0 as u32) {
                    left_out = w & 0xff;
                }
            }
            let mut s = frame.state;
            for t in gap {
                s = step(s, t.0 as u32).ok_or(Refusal::Parse(t.1))?;
            }
            s = match next {
                Some(l) => step(s, l).ok_or(Refusal::Parse(self.at as u32))?,
                None if accepts(s) => s,
                None => return self.refuse(),
            };
            self.frames.last_mut().expect("a frame").state = s;
        }
        for (i, &(kind, from, to)) in self.pending[frame.gap..].iter().enumerate() {
            if left_out >> i & 1 == 0 {
                let at = self.w.push(key(kind, false), from);
                self.w.close(at, to);
            }
        }
        self.pending.truncate(frame.gap);
        Ok(())
    }

    // ---- The tag stack.

    /// The name at `at` (`scan_tag_name`: letters, digits, `-`, `:`), and its type. A name that
    /// runs on into a character past ASCII is refused (`iswalnum` there is the locale's).
    fn name(&self, at: usize) -> R<(usize, u8)> {
        let end = self.b.name_end(at);
        if self.byte(end) >= 0x80 {
            return Err(Refusal::Parse(end as u32));
        }
        Ok((end, tags::type_of(&self.src[at..end])))
    }

    /// Whether the open element `o` has the tag `ty`, `name` (`tag_eq`).
    fn tag_eq(&self, o: Open, ty: u8, name: (usize, usize)) -> bool {
        o.ty == ty && (ty != 0 || self.src[o.name.0 as usize..o.name.1 as usize].eq_ignore_ascii_case(&self.src[name.0..name.1]))
    }

    /// The innermost element ended implicitly, at `at` (a token of no width there).
    fn end_implicitly(&mut self) -> R<()> {
        let o = self.pop();
        self.close(o.node, self.at)
    }

    /// The innermost open element, taken off the stack.
    fn pop(&mut self) -> Open {
        let o = self.open.pop().expect("an open element");
        self.serialized -= tags::serialized_size(o.ty, (o.name.1 - o.name.0) as usize);
        o
    }

    // ---- The grammar.

    fn document(&mut self) -> R<()> {
        // tree-sitter's lexer passes over a byte order mark at the start.
        if self.src.starts_with("\u{feff}".as_bytes()) {
            self.at = 3;
        }
        let root = self.open_node(K::Document, self.at)?;
        // The first node's start, taken before its commit.
        let mut first = None;
        loop {
            if first.is_none() && self.w.len() > root + 1 {
                first = Some(self.w.start(root + 1));
            }
            // A text, interpolation or tag read is final (an element it ends is closed, as a node passed on may
            // be): its nodes are committed (passed to format 16's ops; decision 0434 §2).
            self.w.commit();
            if self.at >= self.src.len() {
                // An element open at the end is an error: the scanner finds no token there.
                if !self.open.is_empty() {
                    return self.refuse();
                }
                break;
            }
            if self.byte(self.at) != b'<' {
                match self.text(self.at, false)? {
                    Scan::Text(from, to) => {
                        self.leaf(K::Text, from, to)?;
                        self.at = to;
                        continue;
                    }
                    Scan::Stop(next) => {
                        self.at = next;
                        if self.byte(next) == b'{' {
                            self.interpolation()?;
                            continue;
                        }
                    }
                    // The scanner found no token to the end; the lexer reads white space alone there.
                    Scan::End => {
                        if self.blanks(self.at)? < self.src.len() {
                            return self.refuse();
                        }
                        self.at = self.src.len();
                        continue;
                    }
                    Scan::Interpolation(..) => unreachable!("an interpolation's text outside one"),
                }
            }
            self.tag()?;
        }
        // The document spans its nodes, from the first one's start (the file's end where it has
        // none), to the file's end.
        let first = first.unwrap_or(self.src.len() as u32);
        self.w.set_start(root, first);
        self.close(root, self.src.len())
    }

    /// The scanner's text loop from `from` (`interpolation`: an interpolation's text, which ends at
    /// `}}`), as `scan` takes it, its stops found by stage 1's mask.
    fn text(&self, from: usize, interpolation: bool) -> R<Scan> {
        let len = self.src.len();
        // The token's start (moved past white space skipped before anything is read), its end so
        // far (`mark_end`), and whether a byte other than a line break was read (`advanced_once`).
        let (mut start, mut pos, mut read) = (from, from, false);
        let mut mark;
        let stop = loop {
            if pos >= len {
                return Ok(Scan::End);
            }
            let next = self.b.text_stop(pos);
            if next > pos {
                read = true;
                pos = next;
                continue;
            }
            let c = self.src[pos];
            if c == b'\r' {
                // A carriage return reads the byte after it, unless it is a line feed.
                if self.byte(pos + 1) != b'\n' {
                    read = true;
                    pos = (pos + 2).min(len);
                    continue;
                }
                pos += 1;
            }
            match self.src[pos] {
                b'<' => {
                    mark = pos;
                    let c = self.byte(pos + 1);
                    if pos + 1 < len && (c.is_ascii_alphabetic() || matches!(c, b'!' | b'?' | b'/')) {
                        break pos;
                    }
                    if c >= 0x80 {
                        return Err(Refusal::Lex(pos as u32 + 1));
                    }
                    read = true;
                    pos += 1;
                }
                b'{' => {
                    mark = pos;
                    if self.byte(pos + 1) == b'{' {
                        break pos;
                    }
                    read = true;
                    pos += 1;
                }
                b'}' if interpolation => {
                    mark = pos;
                    if self.byte(pos + 1) == b'}' {
                        return Ok(if read { Scan::Interpolation(start, mark) } else { Scan::Stop(pos) });
                    }
                    pos += 1;
                }
                b'}' => {
                    read = true;
                    pos += 1;
                }
                // A line break.
                _ if interpolation => pos += 1,
                _ => {
                    mark = pos;
                    let e = self.blanks(pos)?;
                    if !read {
                        start = e;
                    }
                    pos = e;
                    match self.byte(e) {
                        b'<' if e < len => break e,
                        // `>` after a line's end: no token takes it.
                        b'>' => return Err(Refusal::Parse(e as u32)),
                        _ => {}
                    }
                }
            }
        };
        if !read {
            return Ok(Scan::Stop(stop));
        }
        if interpolation {
            // A text fragment, where the state takes an interpolation's text alone.
            return Err(Refusal::Parse(stop as u32));
        }
        Ok(Scan::Text(start, mark))
    }

    /// `{{`, the text to `}}`, and `}}`.
    fn interpolation(&mut self) -> R<()> {
        let from = self.at;
        let node = self.open_node(K::Interpolation, from)?;
        self.tok(K::TLBraceLBrace, from, from + 2);
        self.at = match self.text(from + 2, true)? {
            Scan::Interpolation(s, e) => {
                self.leaf(K::RawText, s, e)?;
                e
            }
            // The scanner found no text: `}}` after white space.
            Scan::Stop(_) => self.blanks(from + 2)?,
            Scan::Text(..) | Scan::End => return self.refuse(),
        };
        if self.byte(self.at) != b'}' || self.byte(self.at + 1) != b'}' {
            return self.refuse();
        }
        self.tok(K::TRBraceRBrace, self.at, self.at + 2);
        self.at += 2;
        self.close(node, self.at)
    }

    /// At a `<`: a comment, a doctype, an end tag, or a start tag, after the implicit ends the
    /// scanner finds there.
    fn tag(&mut self) -> R<()> {
        let at = self.at;
        let top = self.open.last().copied();
        // An implicit end tag is a token only where an `element`'s children may end.
        let implicit = top.filter(|o| o.holds == Holds::Nodes);
        match self.byte(at + 1) {
            b'!' => {
                if self.byte(at + 2) == b'-' && self.byte(at + 3) == b'-' {
                    return self.comment();
                }
                self.doctype()
            }
            b'/' => {
                if let Some(top) = implicit {
                    let (end, ty) = self.name(at + 2)?;
                    // A name right after `</` that closes no open element but one further out
                    // ends the innermost one (the scanner compares types alone there).
                    if end > at + 2 && !self.tag_eq(top, ty, (at + 2, end)) && self.open.iter().any(|o| o.ty == ty) {
                        return self.end_implicitly();
                    }
                }
                if top.is_none() {
                    // Outside every element the scanner reads no end tag name: an error.
                    return self.refuse();
                }
                self.end_tag()
            }
            _ => {
                if let Some(top) = implicit {
                    if tags::is_void(top.ty) {
                        return self.end_implicitly();
                    }
                    let (end, ty) = self.name(at + 1)?;
                    if end > at + 1 && !tags::can_contain(top.ty, ty) {
                        return self.end_implicitly();
                    }
                }
                self.start_tag()
            }
        }
    }

    /// A comment at `<!--`: to the first `>` after two `-`s (`scan_comment`).
    fn comment(&mut self) -> R<()> {
        let from = self.at;
        let mut at = from + 4;
        loop {
            let gt = self.b.gt(at);
            if gt >= self.src.len() {
                return self.refuse();
            }
            if gt >= from + 6 && self.src[gt - 1] == b'-' && self.src[gt - 2] == b'-' {
                self.flush(Some(K::Comment as u32))?;
                let n = self.w.push(key(K::Comment, true), from as u32);
                self.w.close(n, (gt + 1) as u32);
                self.at = gt + 1;
                return Ok(());
            }
            at = gt + 1;
        }
    }

    /// `<!` `doctype` text `>`.
    fn doctype(&mut self) -> R<()> {
        let from = self.at;
        let node = self.open_node(K::Doctype, from)?;
        self.tok(K::TLtBang, from, from + 2);
        let word = self.blanks(from + 2)?;
        self.at = word;
        if !self.src.get(word..word + 7).is_some_and(|w| w.eq_ignore_ascii_case(b"doctype")) {
            return self.refuse();
        }
        self.tok(K::TDoctype, word, word + 7);
        self.at = self.blanks(word + 7)?;
        let gt = self.b.gt(self.at);
        if gt >= self.src.len() || gt == self.at {
            return self.refuse();
        }
        self.tok(K::TGt, gt, gt + 1);
        self.at = gt + 1;
        self.close(node, gt + 1)
    }

    /// `</` name `>`: the innermost element's end tag, or an erroneous one.
    fn end_tag(&mut self) -> R<()> {
        let from = self.at;
        let start = self.blanks(from + 2)?;
        self.at = start;
        let (end, ty) = self.name(start)?;
        if end == start {
            return self.refuse();
        }
        let closes = self.open.last().is_some_and(|&top| self.tag_eq(top, ty, (start, end)));
        let kind = if closes { K::EndTag } else { K::ErroneousEndTag };
        let node = self.open_node(kind, from)?;
        self.tok(K::TLtSlash, from, from + 2);
        self.leaf(if closes { K::TagName } else { K::ErroneousEndTagName }, start, end)?;
        self.at = self.blanks(end)?;
        if self.byte(self.at) != b'>' {
            return self.refuse();
        }
        self.tok(K::TGt, self.at, self.at + 1);
        self.at += 1;
        self.close(node, self.at)?;
        if closes {
            let o = self.pop();
            self.close(o.node, self.at)?;
        }
        Ok(())
    }

    /// `<` name attributes, then `>` (an element opened, a script's or a style's text read to its
    /// end tag) or `/>` (a self-closing tag: the element ends there).
    fn start_tag(&mut self) -> R<()> {
        let from = self.at;
        let (end, ty) = self.name(from + 1)?;
        if end == from + 1 {
            // `<` and no name (white space, a digit's absence): the scanner reads none.
            return self.refuse();
        }
        // The scanner holds the tag from its name on (a self-closing one too, until its `/>`).
        let len = end - from - 1;
        if !tags::serializes(self.serialized, ty, len) {
            return Err(Refusal::Parse(from as u32));
        }
        let (kind, holds) = match ty {
            tags::SCRIPT => (K::ScriptElement, Holds::Raw),
            tags::STYLE => (K::StyleElement, Holds::Raw),
            tags::TEMPLATE => (K::TemplateElement, Holds::Template),
            _ => (K::Element, Holds::Nodes),
        };
        let element = self.open_node(kind, from)?;
        let tag = self.open_node(K::StartTag, from)?;
        self.tok(K::TLt, from, from + 1);
        self.leaf(K::TagName, from + 1, end)?;
        self.at = end;
        loop {
            self.at = self.blanks(self.at)?;
            match self.byte(self.at) {
                b'>' => {
                    self.tok(K::TGt, self.at, self.at + 1);
                    self.at += 1;
                    self.close(tag, self.at)?;
                    self.open.push(Open { ty, name: (from as u32 + 1, end as u32), holds, node: element });
                    self.serialized += tags::serialized_size(ty, len);
                    if holds == Holds::Raw {
                        return self.raw_text(ty);
                    }
                    return Ok(());
                }
                b'/' => {
                    if self.byte(self.at + 1) != b'>' || holds == Holds::Raw {
                        return self.refuse();
                    }
                    self.self_closing(tag);
                    if kind == K::TemplateElement {
                        self.w.set_key(element, key(K::Element, false));
                    }
                    self.tok(K::TSlashGt, self.at, self.at + 2);
                    self.at += 2;
                    self.close(tag, self.at)?;
                    return self.close(element, self.at);
                }
                0 | b'<' | b'"' | b'\'' | b'=' => return self.refuse(),
                b':' | b'.' | b'@' | b'#' => self.directive()?,
                b'v' if self.byte(self.at + 1) == b'-' && self.b.directive_end(self.at + 2) > self.at + 2 => self.directive()?,
                _ => self.attribute()?,
            }
        }
    }

    /// The start tag `tag` is a self-closing tag: its key, and its automaton's state carried over
    /// (the two automata are the same but for their last token, `kinds`' test).
    fn self_closing(&mut self, tag: usize) {
        self.w.set_key(tag, key(K::SelfClosingTag, false));
        let frame = self.frames.last_mut().expect("the tag's frame");
        let (from, to) = (kinds::AUTOMATON[K::StartTag as usize], kinds::AUTOMATON[K::SelfClosingTag as usize]);
        frame.state = frame.state - kinds::START[usize::from(from) - 1] + kinds::START[usize::from(to) - 1];
        frame.auto = to;
    }

    /// An attribute: a name, and `=` and a value.
    fn attribute(&mut self) -> R<()> {
        let from = self.at;
        let end = self.b.attribute_name_end(from);
        self.no_wide_space(from, end)?;
        let node = self.open_node(K::Attribute, from)?;
        self.leaf(K::AttributeName, from, end)?;
        let last = self.value_after(end)?;
        self.at = last;
        self.close(node, last)
    }

    /// After `at`, an optional `=` and its value; where the node ends.
    fn value_after(&mut self, at: usize) -> R<usize> {
        let after = self.blanks(at)?;
        if self.byte(after) != b'=' {
            return Ok(at);
        }
        self.tok(K::TEq, after, after + 1);
        let v = self.blanks(after + 1)?;
        self.at = v;
        match self.byte(v) {
            q @ (b'"' | b'\'') => {
                let close = self.b.quote(v + 1, q);
                if close >= self.src.len() {
                    return self.refuse();
                }
                let qk = if q == b'"' { K::TDQuote } else { K::TQuote };
                let value = self.open_node(K::QuotedAttributeValue, v)?;
                self.tok(qk, v, v + 1);
                if close > v + 1 {
                    self.leaf(K::AttributeValue, v + 1, close)?;
                }
                self.tok(qk, close, close + 1);
                self.close(value, close + 1)?;
                Ok(close + 1)
            }
            _ => {
                let e = self.b.attribute_value_end(v);
                if e == v {
                    return self.refuse();
                }
                self.no_wide_space(v, e)?;
                self.leaf(K::AttributeValue, v, e)?;
                Ok(e)
            }
        }
    }

    /// A directive: `v-name` and an optional `:argument`, or shorthands and their arguments
    /// (`:a`, `@a`, `.a`, `#a`, one after another while none has a value), then optional
    /// modifiers and a value.
    fn directive(&mut self) -> R<()> {
        let from = self.at;
        let node = self.open_node(K::DirectiveAttribute, from)?;
        let mut pos;
        if self.byte(from) == b'v' {
            let e = self.b.directive_end(from + 2);
            self.no_wide_space(from, e)?;
            self.leaf(K::DirectiveName, from, e)?;
            pos = e;
            if self.byte(pos) == b':' {
                self.tok(K::TColon, pos, pos + 1);
                pos = self.argument(pos + 1)?;
            }
        } else {
            pos = from;
            loop {
                let tok = match self.byte(pos) {
                    b':' => K::TColon,
                    b'.' => K::TDot,
                    b'@' => K::TAt,
                    _ => K::THash,
                };
                self.tok(tok, pos, pos + 1);
                pos = self.argument(pos + 1)?;
                // A `.` right after the argument opens its modifiers (`:x.y`); a shorthand after
                // white space continues the attribute.
                if self.byte(pos) == b'.' {
                    break;
                }
                let next = self.blanks(pos)?;
                if !matches!(self.byte(next), b':' | b'.' | b'@' | b'#') {
                    break;
                }
                pos = next;
            }
        }
        if self.byte(pos) == b'.' {
            pos = self.modifiers(pos)?;
        }
        let last = self.value_after(pos)?;
        self.at = last;
        self.close(node, last)
    }

    /// A directive's argument at `at`: a `[dynamic]` one or a name; where it ends.
    fn argument(&mut self, at: usize) -> R<usize> {
        self.at = at;
        if self.byte(at) == b'[' {
            let node = self.open_node(K::DynamicDirectiveValue, at)?;
            self.tok(K::TLBracket, at, at + 1);
            let e = self.b.inner_end(at + 1);
            self.no_wide_space(at + 1, e)?;
            if e > at + 1 {
                self.leaf(K::DynamicDirectiveInnerValue, at + 1, e)?;
            }
            if self.byte(e) != b']' {
                return Err(Refusal::Parse(e as u32));
            }
            self.tok(K::TRBracket, e, e + 1);
            self.close(node, e + 1)?;
            return Ok(e + 1);
        }
        let e = self.b.directive_end(at);
        if e == at {
            return self.refuse();
        }
        self.no_wide_space(at, e)?;
        self.leaf(K::DirectiveValue, at, e)?;
        Ok(e)
    }

    /// `.modifier`s from the `.` at `at`; where they end.
    fn modifiers(&mut self, at: usize) -> R<usize> {
        let node = self.open_node(K::DirectiveModifiers, at)?;
        let mut pos = at;
        loop {
            self.tok(K::TDot, pos, pos + 1);
            let e = self.b.modifier_end(pos + 1);
            if e == pos + 1 {
                return Err(Refusal::Parse(e as u32));
            }
            self.no_wide_space(pos + 1, e)?;
            self.leaf(K::DirectiveModifier, pos + 1, e)?;
            pos = e;
            if self.byte(pos) != b'.' {
                break;
            }
        }
        self.close(node, pos)?;
        Ok(pos)
    }

    /// A script's or a style's text, to its end tag (`scan_raw_text`), and the end tag. Each `<`
    /// is tried against the end tag's first 8 bytes at once (`upper` for the case).
    fn raw_text(&mut self, ty: u8) -> R<()> {
        let (delimiter, len): (u64, usize) =
            if ty == tags::SCRIPT { (u64::from_le_bytes(*b"</SCRIPT"), 8) } else { (u64::from_le_bytes(*b"</STYLE\0"), 7) };
        let keep = if len == 8 { !0u64 } else { (1u64 << (8 * len)) - 1 };
        let from = self.at;
        // The scanner's loop: a byte that breaks a partial match is passed over, not tried again
        // as the delimiter's first.
        let mut at = from;
        let end = loop {
            let lt = self.b.lt(at);
            if lt >= self.src.len() {
                return self.refuse();
            }
            let differ = (upper(load8(self.src, lt)) ^ delimiter) & keep;
            if differ == 0 {
                break lt;
            }
            let k = (differ.trailing_zeros() / 8) as usize;
            if lt + k >= self.src.len() {
                return self.refuse();
            }
            if self.src[lt + k] >= 0x80 {
                // `towupper` there is the locale's.
                return Err(Refusal::Parse((lt + k) as u32));
            }
            at = lt + k + 1;
        };
        // An empty text is still a `raw_text` (the scanner returns it at no width).
        self.leaf(K::RawText, from, end)?;
        self.at = end;
        // Its end tag: `</script` or `</style`, then the rest of its name (which must be that).
        let o = *self.open.last().expect("an open element");
        let (e, t) = self.name(end + 2)?;
        if t != o.ty {
            return Err(Refusal::Parse(e as u32));
        }
        self.end_tag()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_self_closing_tags_automaton_is_a_start_tags() {
        // `self_closing` carries a start tag's state over: the automata must be the same but for
        // `>` and `/>`.
        let (a, b) = (usize::from(kinds::AUTOMATON[K::StartTag as usize]), usize::from(kinds::AUTOMATON[K::SelfClosingTag as usize]));
        let (sa, sb) = (kinds::START[a - 1], kinds::START[b - 1]);
        let count = |s: usize| kinds::START.get(s).map_or(kinds::MOVES_AT.len() as u32 - 1, |&n| n) - kinds::START[s - 1];
        assert_eq!(count(a), count(b));
        let label = |l: u32| if l == K::TGt as u32 { K::TSlashGt as u32 } else { l };
        for i in 0..count(a) {
            let (x, y) = (sa + i, sb + i);
            assert_eq!(accepts(x), accepts(y));
            let mx = &kinds::MOVES[kinds::MOVES_AT[x as usize] as usize..kinds::MOVES_AT[x as usize + 1] as usize];
            let my = &kinds::MOVES[kinds::MOVES_AT[y as usize] as usize..kinds::MOVES_AT[y as usize + 1] as usize];
            let mut mx: Vec<_> = mx.iter().map(|&(l, t)| (label(l), t - sa)).collect();
            let mut my: Vec<_> = my.iter().map(|&(l, t)| (l, t - sb)).collect();
            mx.sort();
            my.sort();
            assert_eq!(mx, my, "state {i}");
            for l in [K::TagName as u32, K::Attribute as u32, K::DirectiveAttribute as u32] {
                assert_eq!(implied(x, Some(l)) & 0xfff, implied(y, Some(l)) & 0xfff, "state {i}");
            }
            assert_eq!(implied(x, None) != 0, implied(y, None) != 0, "state {i}");
        }
    }

    #[test]
    fn upper_makes_letters_upper_case() {
        let w = u64::from_le_bytes(*b"</sCrIpt");
        assert_eq!(upper(w), u64::from_le_bytes(*b"</SCRIPT"));
        let w = u64::from_le_bytes(*b"`az{@AZ[");
        assert_eq!(upper(w), u64::from_le_bytes(*b"`AZ{@AZ["));
    }
}
