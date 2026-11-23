//! The parser: tree-sitter-html's grammar and its external scanner's tag stack, read in one pass
//! over the text, writing the tape as it goes.
//!
//! tree-sitter-html's tree is decided by its scanner (`scanner.c`): it keeps the open elements'
//! tags, and ends an element where its end tag comes, or implicitly (`_implicit_end_tag`, a token
//! of no width) where the next tag may not stand inside it, where an end tag closes an element
//! further out, where a void element is followed by `<`, or at the text's end. The parser keeps the
//! same stack (`open`) and takes the same steps at the same bytes, so each element ends where
//! tree-sitter's does.
//!
//! **Stored tokens.** A token is stored unless its kind implies it (`kinds::DROPPED`), and a quote
//! where its gap is not its kind's one shortest word: a quoted value's opening quote (either quote
//! may open it), and both quotes of an empty one.

use crate::block::{Blocks, is_blank};
use crate::kinds::K;
use crate::tags;
use crate::{Refusal, Tape, key};
use rerail_tape_format::{Redo, Sink, Widths, Writer};

type R<T> = Result<T, Refusal>;

/// An open element: its tag (a type from `tags`, and its name's bytes for a custom one), and its
/// node.
#[derive(Clone, Copy)]
struct Open {
    ty: u8,
    name: (u32, u32),
    node: usize,
}

struct P<'a> {
    src: &'a [u8],
    b: &'a Blocks,
    w: Writer<Widths>,
    open: Vec<Open>,
    /// The open tags' size as the scanner serializes them (`tags::serialized_size`).
    serialized: usize,
    at: usize,
}


/// `src` parsed.
pub fn parse(src: &[u8]) -> R<Tape> {
    parse_into(src, &mut crate::sink()).map(crate::written)
}

/// [`parse`], the writer finished by `finish`: format 16 or 16z (decisions 0293, 0434).
pub fn parse_into<Out>(src: &[u8], finish: &mut dyn Sink<Out>) -> R<Out> {
    parse_from(src, Start::Document, finish)
}

/// The node kind a parse starts at (decision 0466): a whole document, or one element alone, its root (a fragment).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Document,
    Element,
}

/// [`parse_into`] from `start`.
pub fn parse_from<Out>(src: &[u8], start: Start, finish: &mut dyn Sink<Out>) -> R<Out> {
    // The scanner reads a NUL as the text's end.
    if src.contains(&0) {
        return Err(Refusal::Lex(src.iter().position(|&c| c == 0).unwrap_or(0) as u32));
    }
    let b = Blocks::of(src);
    let mut widths = Widths::guess(src.len());
    loop {
        let mut p = P { src, b: &b, w: Writer::new(widths, src.len() / 4 + 16).to(finish.target(), src).omitting(finish.omissions()), open: Vec::new(), serialized: tags::SERIALIZED_EMPTY, at: 0 };
        match start {
            Start::Document => p.document()?,
            Start::Element => p.element_fragment()?,
        }
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

impl<'a> P<'a> {
    fn refuse<T>(&self) -> R<T> {
        Err(Refusal::Parse(self.at as u32))
    }

    #[inline(always)]
    fn byte(&self, at: usize) -> u8 {
        self.src.get(at).copied().unwrap_or(0)
    }

    /// Past white space (the scanner's `iswspace`, the lexer's `\s`): ASCII's alone, as both read
    /// it (a no-break space is a text's, the oracle shows).
    #[inline]
    fn blanks(&mut self) -> R<()> {
        self.at = self.b.blank_end(self.at);
        Ok(())
    }

    // ---- The writer.

    #[inline]
    fn leaf(&mut self, kind: K, from: usize, to: usize) {
        let at = self.w.push(key(kind, false), from as u32);
        self.w.close(at, to as u32);
    }

    #[inline]
    fn open_node(&mut self, kind: K, from: usize) -> usize {
        self.w.push(key(kind, false), from as u32)
    }

    #[inline]
    fn close(&mut self, node: usize, end: usize) {
        self.w.close(node, end as u32);
    }

    /// Token `tok` under `parent`, at `from..to`: a leaf where the tape stores it.
    #[inline]
    fn tok(&mut self, parent: K, tok: K, from: usize, to: usize) {
        if crate::stores(parent, tok) {
            self.leaf(tok, from, to);
        }
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
    fn end_implicitly(&mut self) {
        let o = self.pop();
        self.close(o.node, self.at);
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
        self.blanks()?;
        let root = self.open_node(K::Document, self.at);
        loop {
            self.blanks()?;
            if self.at >= self.src.len() {
                // At the end, the scanner ends a void element, a `colgroup`, and `html`, `head` and
                // `body`; any other element open there is an error.
                while let Some(o) = self.open.last() {
                    if !tags::ends_at_eof(o.ty) {
                        return self.refuse();
                    }
                    self.end_implicitly();
                }
                break;
            }
            match self.src[self.at] {
                b'<' => self.tag()?,
                b'&' => self.entity()?,
                b'>' => return self.refuse(),
                _ => self.text()?,
            }
            // A tag, entity or text read is final (an element it ends is closed, as a node passed on may be): its
            // nodes are committed (passed to format 16's ops; decision 0434 §2).
            self.w.commit();
        }
        self.close(root, self.src.len());
        Ok(())
    }

    /// One element alone, the root (decision 0466): read as inside a document, its end where the scanner ends it, with
    /// no `html`, `head` or `body` implied around it. A text after it (another element, a comment) is refused: a
    /// fragment's tape has one root. An element open at the end ends there where it may (a void element, `p`, …), at
    /// the text's end less blanks.
    fn element_fragment(&mut self) -> R<()> {
        self.blanks()?;
        if self.byte(self.at) != b'<' || !self.byte(self.at + 1).is_ascii_alphabetic() {
            return self.refuse();
        }
        // `script` and `style` are kinds of their own (`script_element`, `style_element`), not an `element`.
        let (_, ty) = self.name(self.at + 1)?;
        if ty == tags::SCRIPT || ty == tags::STYLE {
            return self.refuse();
        }
        let last = self.src.len() - self.src.iter().rev().take_while(|&&c| is_blank(c)).count();
        loop {
            if self.at >= last {
                while let Some(o) = self.open.last() {
                    if !tags::ends_at_eof(o.ty) {
                        return self.refuse();
                    }
                    self.at = last;
                    self.end_implicitly();
                }
                return Ok(());
            }
            match self.src[self.at] {
                b'<' => self.tag()?,
                b'&' => self.entity()?,
                b'>' => return self.refuse(),
                _ => self.text()?,
            }
            self.w.commit();
            if self.open.is_empty() {
                // The root ended: nothing but blanks may follow.
                self.blanks()?;
                return if self.at >= self.src.len() { Ok(()) } else { self.refuse() };
            }
            if self.at < last {
                self.blanks()?;
            }
        }
    }

    /// At a `<`: a comment, a doctype, an end tag, or a start tag, after the implicit ends the
    /// scanner finds there.
    fn tag(&mut self) -> R<()> {
        let at = self.at;
        match self.byte(at + 1) {
            b'!' => {
                if self.byte(at + 2) == b'-' && self.byte(at + 3) == b'-' {
                    return self.comment();
                }
                self.doctype()
            }
            b'/' => {
                if let Some(&top) = self.open.last() {
                    let (end, ty) = self.name(at + 2)?;
                    // A name right after `</` that closes no open element but one further out
                    // ends the innermost one (the scanner compares types alone there).
                    if end > at + 2 && !self.tag_eq(top, ty, (at + 2, end)) && self.open.iter().any(|o| o.ty == ty) {
                        self.end_implicitly();
                        return Ok(());
                    }
                }
                if self.open.is_empty() {
                    // Outside every element the scanner reads no end tag name: an error.
                    return self.refuse();
                }
                self.end_tag()
            }
            _ => {
                if let Some(&top) = self.open.last() {
                    if tags::is_void(top.ty) {
                        self.end_implicitly();
                        return Ok(());
                    }
                    let (end, ty) = self.name(at + 1)?;
                    if end == at + 1 {
                        // `<` and white space: a start tag only for the lexer, not the scanner.
                        return self.refuse();
                    }
                    if !tags::can_contain(top.ty, ty) {
                        self.end_implicitly();
                        return Ok(());
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
        let node = self.open_node(K::Doctype, from);
        self.tok(K::Doctype, K::TLtBang, from, from + 2);
        self.at = from + 2;
        self.blanks()?;
        let word = self.at;
        if !self.src.get(word..word + 7).is_some_and(|w| w.eq_ignore_ascii_case(b"doctype")) {
            return self.refuse();
        }
        self.tok(K::Doctype, K::TDoctype, word, word + 7);
        self.at = word + 7;
        self.blanks()?;
        let gt = self.b.gt(self.at);
        if gt >= self.src.len() || gt == self.at {
            return self.refuse();
        }
        self.tok(K::Doctype, K::TGt, gt, gt + 1);
        self.close(node, gt + 1);
        self.at = gt + 1;
        Ok(())
    }

    /// `</` name `>`: the innermost element's end tag, or an erroneous one.
    fn end_tag(&mut self) -> R<()> {
        let from = self.at;
        self.at = from + 2;
        self.blanks()?;
        let start = self.at;
        let (end, ty) = self.name(start)?;
        if end == start {
            return self.refuse();
        }
        let closes = self.open.last().is_some_and(|&top| self.tag_eq(top, ty, (start, end)));
        let kind = if closes { K::EndTag } else { K::ErroneousEndTag };
        let node = self.open_node(kind, from);
        self.tok(kind, K::TLtSlash, from, from + 2);
        self.leaf(if closes { K::TagName } else { K::ErroneousEndTagName }, start, end);
        self.at = end;
        self.blanks()?;
        if self.byte(self.at) != b'>' {
            return self.refuse();
        }
        self.tok(kind, K::TGt, self.at, self.at + 1);
        self.at += 1;
        self.close(node, self.at);
        if closes {
            let o = self.pop();
            self.close(o.node, self.at);
        }
        Ok(())
    }

    /// `<` name attributes, then `>` (an element opened, a script's or a style's text read to its
    /// end tag) or `/>` (a self-closing tag: the element ends there).
    fn start_tag(&mut self) -> R<()> {
        let from = self.at;
        let (end, ty) = self.name(from + 1)?;
        if end == from + 1 {
            // No name (`<%`): no start tag, and `<` is no text.
            return self.refuse();
        }
        // The scanner holds the tag from its name on (a self-closing one too, until its `/>`).
        let len = end - from - 1;
        if !tags::serializes(self.serialized, ty, len) {
            return Err(Refusal::Parse(from as u32));
        }
        let raw = ty == tags::SCRIPT || ty == tags::STYLE;
        let element = self.open_node(
            match ty {
                tags::SCRIPT => K::ScriptElement,
                tags::STYLE => K::StyleElement,
                _ => K::Element,
            },
            from,
        );
        let tag = self.open_node(K::StartTag, from);
        // `<` is stored alike in a start tag and a self-closing one (`kinds`' test).
        self.tok(K::StartTag, K::TLt, from, from + 1);
        self.leaf(K::TagName, from + 1, end);
        self.at = end;
        loop {
            self.blanks()?;
            match self.byte(self.at) {
                b'>' => {
                    let gt = self.at;
                    self.at += 1;
                    self.tok(K::StartTag, K::TGt, gt, gt + 1);
                    self.close(tag, self.at);
                    self.open.push(Open { ty, name: (from as u32 + 1, end as u32), node: element });
                    self.serialized += tags::serialized_size(ty, len);
                    if raw {
                        return self.raw_text(ty);
                    }
                    return Ok(());
                }
                b'/' if self.byte(self.at + 1) == b'>' && !raw => {
                    let s = self.at;
                    self.at += 2;
                    self.w.set_key(tag, key(K::SelfClosingTag, false));
                    self.tok(K::SelfClosingTag, K::TSlashGt, s, s + 2);
                    self.close(tag, self.at);
                    self.close(element, self.at);
                    return Ok(());
                }
                0 | b'<' | b'"' | b'\'' | b'=' | b'/' => return self.refuse(),
                _ => self.attribute()?,
            }
        }
    }

    /// An attribute: a name, and `=` and a value.
    fn attribute(&mut self) -> R<()> {
        let from = self.at;
        let end = self.b.attribute_name_end(from);
        let node = self.open_node(K::Attribute, from);
        self.leaf(K::AttributeName, from, end);
        self.at = end;
        let mut last = end;
        let after = self.b.blank_end(end);
        if self.byte(after) == b'=' {
            self.at = after;
            self.tok(K::Attribute, K::TEq, after, after + 1);
            self.at = after + 1;
            self.blanks()?;
            let v = self.at;
            match self.byte(v) {
                q @ (b'"' | b'\'') => {
                    let close = self.b.quote(v + 1, q);
                    if close >= self.src.len() {
                        return self.refuse();
                    }
                    let qk = if q == b'"' { K::TDQuote } else { K::TQuote };
                    let value = self.open_node(K::QuotedAttributeValue, v);
                    // Either quote may open a value: the opening one is stored, and an empty
                    // value's closing one.
                    self.leaf(qk, v, v + 1);
                    if close > v + 1 {
                        self.leaf(K::AttributeValue, v + 1, close);
                        self.tok(K::QuotedAttributeValue, qk, close, close + 1);
                    } else {
                        self.leaf(qk, close, close + 1);
                    }
                    self.close(value, close + 1);
                    last = close + 1;
                }
                _ => {
                    let e = self.b.attribute_value_end(v);
                    if e == v {
                        return self.refuse();
                    }
                    self.leaf(K::AttributeValue, v, e);
                    last = e;
                }
            }
        }
        self.at = last;
        self.close(node, last);
        Ok(())
    }

    /// A script's or a style's text, to its end tag (`scan_raw_text`), and the end tag.
    fn raw_text(&mut self, ty: u8) -> R<()> {
        let delimiter: &[u8] = if ty == tags::SCRIPT { b"</SCRIPT" } else { b"</STYLE" };
        let from = self.at;
        // The scanner's loop: a byte that breaks a partial match is passed over, not tried again
        // as the delimiter's first.
        let mut at = from;
        let end = loop {
            let lt = self.b.lt(at);
            if lt >= self.src.len() {
                return self.refuse();
            }
            let mut k = 1;
            while k < delimiter.len() {
                let c = self.byte(lt + k);
                if c >= 0x80 {
                    return Err(Refusal::Parse((lt + k) as u32));
                }
                if c.to_ascii_uppercase() != delimiter[k] {
                    break;
                }
                k += 1;
            }
            if k == delimiter.len() {
                break lt;
            }
            if lt + k >= self.src.len() {
                return self.refuse();
            }
            at = lt + k + 1;
        };
        // An empty text is still a `raw_text` (the scanner returns it at no width).
        self.leaf(K::RawText, from, end);
        self.at = end;
        // Its end tag: `</script` or `</style`, then the rest of its name (which must be that).
        let o = *self.open.last().expect("an open element");
        let (e, t) = self.name(end + 2)?;
        if t != o.ty {
            return Err(Refusal::Parse(e as u32));
        }
        self.end_tag()
    }

    /// `&`, then a name or a number, and an optional `;`.
    fn entity(&mut self) -> R<()> {
        let from = self.at;
        let mut at = from + 1;
        if self.byte(at) == b'#' {
            at += 1;
            if matches!(self.byte(at), b'x' | b'X') {
                let s = at + 1;
                let mut e = s;
                while e < s + 6 && self.byte(e).is_ascii_hexdigit() {
                    e += 1;
                }
                if e == s {
                    return self.refuse();
                }
                at = e;
            } else {
                let s = at;
                let mut e = s;
                while e < s + 5 && self.byte(e).is_ascii_digit() {
                    e += 1;
                }
                if e == s {
                    return self.refuse();
                }
                at = e;
            }
        } else {
            let s = at;
            let mut e = s;
            while e < s + 30 && self.byte(e).is_ascii_alphabetic() {
                e += 1;
            }
            if e == s {
                return self.refuse();
            }
            at = e;
        }
        if self.byte(at) == b';' {
            at += 1;
        }
        self.leaf(K::Entity, from, at);
        self.at = at;
        Ok(())
    }

    /// A text: to its last character that is not white space before a `<`, `>` or `&`.
    fn text(&mut self) -> R<()> {
        let from = self.at;
        let stop = self.b.text_stop(from);
        let mut end = stop;
        while end > from && is_blank(self.src[end - 1]) {
            end -= 1;
        }
        self.leaf(K::Text, from, end);
        self.at = end;
        Ok(())
    }
}
