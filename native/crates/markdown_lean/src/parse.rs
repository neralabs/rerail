//! The parser: tree-sitter-md's block grammar (`markdown/grammar.js`) over the scanner's tokens
//! (`scan`), writing the tape as it goes.
//!
//! The grammar's states are the functions below. At each token the parser asks the scanner first,
//! with the external tokens the state takes (`Valid`), as tree-sitter does; where it finds none,
//! the token is the grammar's own (`line_token`: a word, white space, a punctuation character).
//! A node starts at its first token and ends at its last, hidden tokens included (the grammar has
//! no extras: every byte is some token's).
//!
//! A `block_continuation` (the scanner's match of the open blocks at a line's start) belongs to
//! the node the parse is in when it comes: a paragraph's line end, an inline's soft line break,
//! a container after a blank line (`next`).

use crate::kinds::{K, f};
use crate::scan::{Found, Scanner, Valid, bit, t::*};
use crate::{Refusal, Tape, key, stores};
use rerail_tape_format::{Redo, Sink, Widths, Writer};

type R<T> = Result<T, Refusal>;

/// A token from the scanner: its kind, start and end.
#[derive(Clone, Copy, Debug)]
struct Tok {
    tok: u8,
    start: usize,
    end: usize,
}

/// The block starts (the external tokens a block may begin with).
const V_BLOCK: Valid = bit(BLOCK_QUOTE_START)
    | bit(INDENTED_CHUNK_START)
    | (0x3f << ATX_H1_MARKER)
    | bit(THEMATIC_BREAK)
    | (0x3ff << LIST_MARKER_MINUS)
    | bit(FENCED_CODE_BLOCK_START_BACKTICK)
    | bit(FENCED_CODE_BLOCK_START_TILDE)
    | bit(BLANK_LINE_START)
    | bit(HTML_BLOCK_1_START)
    | (0x3f << HTML_BLOCK_2_START)
    | bit(PIPE_TABLE_START);
/// After a paragraph's line end: a setext underline too.
const V_AFTER_PARAGRAPH: Valid = V_BLOCK | bit(SETEXT_H1_UNDERLINE) | bit(SETEXT_H2_UNDERLINE);
const V_LINE_END: Valid = bit(LINE_ENDING);
const V_LINE_OR_EOF: Valid = bit(LINE_ENDING) | bit(TOKEN_EOF);
const V_PARAGRAPH: Valid = bit(LINE_ENDING) | bit(SOFT_LINE_ENDING) | bit(TOKEN_EOF);
const V_TABLE: Valid = bit(PIPE_TABLE_LINE_ENDING) | bit(LINE_ENDING) | bit(TOKEN_EOF);

/// The deepest nesting the parser reads (containers, sections).
const MAX_DEPTH: usize = 200;

/// A token of the grammar's own, from `line_token`: a punctuation character's kind, or hidden (a
/// word, white space, a task-list word).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lexed {
    Punct(K),
    Word,
    Space,
}

fn punct(c: u8) -> Option<K> {
    Some(match c {
        b'!' => K::TBang,
        b'"' => K::TDQuote,
        b'#' => K::THash,
        b'$' => K::TDollar,
        b'%' => K::TPercent,
        b'&' => K::TAmp,
        b'\'' => K::TQuote,
        b'(' => K::TLParen,
        b')' => K::TRParen,
        b'*' => K::TStar,
        b'+' => K::TPlus,
        b',' => K::TComma,
        b'-' => K::TMinus,
        b'.' => K::TDot,
        b'/' => K::TSlash,
        b':' => K::TColon,
        b';' => K::TSemi,
        b'<' => K::TLt,
        b'=' => K::TEq,
        b'>' => K::TGt,
        b'?' => K::TQuestion,
        b'@' => K::TAt,
        b'[' => K::TLBracket,
        b'\\' => K::TBackslash,
        b']' => K::TRBracket,
        b'^' => K::TCaret,
        b'_' => K::TUnderscore,
        b'`' => K::TBacktick,
        b'{' => K::TLBrace,
        b'|' => K::TPipe,
        b'}' => K::TRBrace,
        b'~' => K::TTilde,
        _ => return None,
    })
}

/// A list marker's type (minus, plus, star, parenthesis, dot), the same for its two tokens.
fn marker_type(tok: u8) -> Option<u8> {
    (LIST_MARKER_MINUS..=LIST_MARKER_DOT_DONT_INTERRUPT).contains(&tok).then(|| (tok - LIST_MARKER_MINUS) % 5)
}

const MARKERS: [K; 5] = [K::ListMarkerMinus, K::ListMarkerPlus, K::ListMarkerStar, K::ListMarkerParenthesis, K::ListMarkerDot];

const ATX: [K; 6] = [K::AtxH1Marker, K::AtxH2Marker, K::AtxH3Marker, K::AtxH4Marker, K::AtxH5Marker, K::AtxH6Marker];

/// Where a list of blocks is: the document, or a container the scanner closes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    Document,
    Container,
    /// A list item's content (`_list_item_content`: two blank lines first close it).
    Item,
}

struct P<'a> {
    src: &'a [u8],
    sc: Scanner<'a>,
    /// Where the next token starts.
    at: usize,
    /// The last token's end.
    last: usize,
    w: Writer<Widths>,
    /// The open nodes: their writer index and kind.
    open: Vec<(usize, K)>,
    /// Where the link title being read starts (its opening quote).
    title_at: usize,
    /// The tokens of the gap being read (`gap_token`): written when a node ends it (`flush_gap`),
    /// a `block_continuation` among them (`next`).
    gap: Vec<(K, usize, usize)>,
}

/// The parse's state, to go back to (`P::save`).
struct Saved {
    at: usize,
    last: usize,
    nodes: usize,
    open: usize,
    sc: crate::scan::Saved,
}

/// `src` parsed.
pub fn parse(src: &[u8]) -> R<Tape> {
    parse_into(src, &mut Writer::<Widths>::finish_columns)
}

/// [`parse`], the writer finished by `finish`: format 15's columns, or format 16 (decision 0293).
pub fn parse_into<Out>(src: &[u8], finish: &mut dyn Sink<Out>) -> R<Out> {
    if let Some(at) = src.iter().position(|&c| c == 0) {
        return Err(Refusal::Lex(at as u32));
    }
    if let Err(e) = std::str::from_utf8(src) {
        return Err(Refusal::Lex(e.valid_up_to() as u32));
    }
    let mut widths = Widths::guess(src.len());
    loop {
        let mut p = P { src, sc: Scanner::new(src), at: 0, last: 0, w: Writer::new(0, widths, src.len() / 2 + 16).to(finish.target(), src), open: Vec::new(), title_at: usize::MAX, gap: Vec::new() };
        p.document()?;
        if p.sc.unsure {
            return Err(Refusal::Parse(0));
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

    // ---- The writer.

    fn open(&mut self, kind: K, field: u8, start: usize) -> usize {
        if self.open.len() >= MAX_DEPTH {
            // Refused at the next check (`next`).
        }
        let n = self.w.push(key(kind, field), start as u32);
        self.open.push((n, kind));
        n
    }

    fn close(&mut self) {
        let (n, _) = self.open.pop().expect("an open node");
        self.w.close(n, self.last as u32);
    }

    fn top(&self) -> K {
        self.open.last().map_or(K::Document, |o| o.1)
    }

    fn leaf(&mut self, kind: K, field: u8, from: usize, to: usize) {
        let n = self.w.push(key(kind, field), from as u32);
        self.w.close(n, to as u32);
    }

    /// A token taken: the parse goes on past it.
    fn take(&mut self, t: Tok) {
        self.at = t.end;
        self.last = t.end;
    }

    // ---- Tokens.

    /// The scanner's token here, given the external tokens the state takes; a
    /// `block_continuation` it finds is written in the node the parse is in, and the scan goes on.
    fn next(&mut self, valid: Valid) -> R<Option<Tok>> {
        if self.open.len() >= MAX_DEPTH {
            return self.refuse();
        }
        loop {
            let start = self.at;
            let Some(Found { tok, end }) = self.sc.scan_at(start, valid | bit(BLOCK_CONTINUATION)) else {
                return Ok(None);
            };
            let t = Tok { tok, start, end };
            if tok == ERROR {
                return Err(Refusal::Parse(start as u32));
            }
            if tok == BLOCK_CONTINUATION {
                if !self.gap.is_empty() {
                    self.flush_gap(self.top());
                }
                self.leaf(K::BlockContinuation, 0, start, end);
                self.take(t);
                continue;
            }
            return Ok(Some(t));
        }
    }

    /// The grammar's own token at the parse: `None` at a line's end or the text's.
    fn line_token(&self) -> Option<(Lexed, usize)> {
        let at = self.at;
        let c = self.byte(at);
        if at >= self.src.len() || c == b'\n' || c == b'\r' {
            return None;
        }
        if c == b' ' || c == b'\t' {
            return Some((Lexed::Space, self.sc.b.blank_end(at)));
        }
        // `[x]`, `[X]`, `[ ]`, `[\t]`: a word of three bytes (`_word`'s other patterns), longer
        // than `[`.
        if c == b'[' && self.byte(at + 2) == b']' && matches!(self.byte(at + 1), b'x' | b'X' | b' ' | b'\t') {
            return Some((Lexed::Word, at + 3));
        }
        if let Some(k) = punct(c) {
            return Some((Lexed::Punct(k), at + 1));
        }
        Some((Lexed::Word, self.sc.b.word_end(at)))
    }

    /// A line token taken under `parent`: a punctuation character written where the tape keeps it.
    fn take_line_token(&mut self, parent: K, lexed: Lexed, end: usize) {
        if let Lexed::Punct(k) = lexed
            && stores(parent, k)
        {
            self.leaf(k, 0, self.at, end);
        }
        self.at = end;
        self.last = end;
    }

    /// A token into the gap `gap` under `parent` (a run of tokens between child nodes): the
    /// engine's automata drop a gap's implied tokens only where all of its tokens are implied.
    fn gap_token(&mut self, lexed: Lexed, end: usize) {
        if let Lexed::Punct(k) = lexed {
            self.gap.push((k, self.at, end));
        }
        self.at = end;
        self.last = end;
    }

    /// The gap's tokens written under `parent`: all of them where one is stored, none where all
    /// are implied. A title's opening quote is always stored (the title's choice of quote), so the
    /// gap it starts is.
    fn flush_gap(&mut self, parent: K) {
        let gap = std::mem::take(&mut self.gap);
        let opens_title = parent == K::LinkTitle && gap.first().is_some_and(|&(k, s, _)| s == self.title_at && matches!(k, K::TDQuote | K::TQuote | K::TLParen));
        if opens_title || gap.iter().any(|&(k, _, _)| stores(parent, k)) {
            for &(k, from, to) in gap.iter() {
                self.leaf(k, 0, from, to);
            }
        }
    }

    /// The character reference at `at` (an `&`), if one is there: `entity_reference` (`&name;`,
    /// a name the grammar lists) or `numeric_character_reference` (`&#…;`, `&#x…;`), and its end.
    fn reference_at(&self, at: usize) -> Option<(K, usize)> {
        let rest = &self.src[at + 1..];
        if rest.first() == Some(&b'#') {
            let (hex, from) = if matches!(rest.get(1), Some(b'x' | b'X')) { (true, 2) } else { (false, 1) };
            let digits = rest[from..].iter().take_while(|c| if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() }).count();
            let max = if hex { 6 } else { 7 };
            return (1..=max).contains(&digits).then_some(()).filter(|_| rest.get(from + digits) == Some(&b';')).map(|_| (K::NumericCharacterReference, at + 1 + from + digits + 1));
        }
        let n = rest.iter().take_while(|c| c.is_ascii_alphanumeric()).count();
        let name = std::str::from_utf8(&rest[..n]).ok()?;
        (n > 0 && rest.get(n) == Some(&b';') && crate::kinds::ENTITIES.binary_search(&name).is_ok()).then_some((K::EntityReference, at + 1 + n + 1))
    }

    /// The task-list word at the parse (`task_list_marker_checked`, `…_unchecked`), if any.
    fn task_marker(&self) -> Option<K> {
        let at = self.at;
        if self.byte(at) == b'[' && self.byte(at + 2) == b']' {
            match self.byte(at + 1) {
                b'x' | b'X' => return Some(K::TaskListMarkerChecked),
                b' ' | b'\t' => return Some(K::TaskListMarkerUnchecked),
                _ => {}
            }
        }
        None
    }

    // ---- The document.

    fn document(&mut self) -> R<()> {
        // tree-sitter's lexer skips a leading byte order mark.
        let start = if self.src.starts_with("\u{feff}".as_bytes()) { 3 } else { 0 };
        self.at = start;
        self.last = start;
        self.open(K::Document, 0, start);
        let mut next = self.next(V_BLOCK | bit(MINUS_METADATA) | bit(PLUS_METADATA))?;
        if let Some(t) = next
            && (t.tok == MINUS_METADATA || t.tok == PLUS_METADATA)
        {
            let kind = if t.tok == MINUS_METADATA { K::MinusMetadata } else { K::PlusMetadata };
            self.leaf(kind, 0, t.start, t.end);
            self.take(t);
            next = self.next(V_BLOCK)?;
        }
        if self.blocks(Ctx::Document, next)?.is_some() {
            return self.refuse();
        }
        if self.at < self.src.len() {
            return self.refuse();
        }
        self.last = self.src.len();
        self.close();
        Ok(())
    }

    /// The blocks of a container from `first` (the first block's token, scanned): until its
    /// `_block_close`, which is returned (taken by the container), or the text's end in the
    /// document. Sections open and close here, by their headings' levels.
    fn blocks(&mut self, ctx: Ctx, first: Option<Tok>) -> R<Option<Tok>> {
        let mut next = first;
        // The open sections' levels (0: the document's blocks before its first heading).
        let mut sections: Vec<u8> = Vec::new();
        // The paragraph just read, ended by a line end: a setext underline may follow.
        let mut after_paragraph: Option<usize> = None;
        // Blank lines at the start of a list item's content.
        let mut blank_start = if ctx == Ctx::Item { 0 } else { u8::MAX };
        loop {
            let paragraph = after_paragraph.take();
            // A document's blocks so far are final, unless a setext underline may follow the paragraph just read:
            // their nodes are committed (passed to format 16's ops; decision 0426 §2).
            if ctx == Ctx::Document && paragraph.is_none() {
                self.w.commit();
            }
            let Some(t) = next else {
                if self.at >= self.src.len() {
                    if ctx != Ctx::Document {
                        return self.refuse();
                    }
                    self.close_sections(&mut sections);
                    return Ok(None);
                }
                if ctx == Ctx::Document && sections.is_empty() {
                    self.open_section(&mut sections, 0);
                }
                blank_start = u8::MAX;
                let (n, p) = self.paragraph()?;
                next = n;
                after_paragraph = p;
                continue;
            };
            if t.tok == BLOCK_CLOSE {
                self.close_sections(&mut sections);
                return Ok(Some(t));
            }
            if t.tok == CLOSE_BLOCK {
                // Two blank lines first in a list item: it closes after them.
                self.take(t);
                next = self.next(0)?;
                if next.is_none_or(|t| t.tok != BLOCK_CLOSE) {
                    return self.refuse();
                }
                continue;
            }
            if (ATX_H1_MARKER..=ATX_H6_MARKER).contains(&t.tok) {
                let level = t.tok - ATX_H1_MARKER + 1;
                while sections.last().is_some_and(|&l| l == 0 || l >= level) {
                    sections.pop();
                    self.close();
                }
                self.open_section(&mut sections, level);
                blank_start = u8::MAX;
                next = self.atx_heading(t, level)?;
                continue;
            }
            if ctx == Ctx::Document && sections.is_empty() {
                self.open_section(&mut sections, 0);
            }
            next = match t.tok {
                SETEXT_H1_UNDERLINE | SETEXT_H2_UNDERLINE => {
                    let Some(p) = paragraph else { return self.refuse() };
                    self.setext(p, t)?
                }
                BLANK_LINE_START => {
                    self.take(t);
                    let mut follow = V_BLOCK;
                    if blank_start < 2 {
                        blank_start += 1;
                        if blank_start == 2 {
                            // `_blank_line _blank_line _close_block`: after the second, the scanner
                            // is told to close the item (before it matches the next line).
                            follow |= bit(CLOSE_BLOCK);
                        }
                    }
                    self.line_end_or_eof(follow)?
                }
                THEMATIC_BREAK => {
                    blank_start = u8::MAX;
                    self.open(K::ThematicBreak, 0, t.start);
                    self.take(t);
                    let n = self.line_end_or_eof(V_BLOCK)?;
                    n
                }
                BLOCK_QUOTE_START => {
                    blank_start = u8::MAX;
                    self.block_quote(t)?
                }
                FENCED_CODE_BLOCK_START_BACKTICK | FENCED_CODE_BLOCK_START_TILDE => {
                    blank_start = u8::MAX;
                    self.fenced(t)?
                }
                INDENTED_CHUNK_START => {
                    blank_start = u8::MAX;
                    self.indented(t)?
                }
                HTML_BLOCK_1_START..=HTML_BLOCK_7_START => {
                    blank_start = u8::MAX;
                    self.html_block(t)?
                }
                PIPE_TABLE_START => {
                    blank_start = u8::MAX;
                    self.table(t)?
                }
                _ if marker_type(t.tok).is_some() => {
                    blank_start = u8::MAX;
                    self.list(t)?
                }
                _ => return Err(Refusal::Parse(t.start as u32)),
            };
        }
    }

    fn open_section(&mut self, sections: &mut Vec<u8>, level: u8) {
        self.open(K::Section, 0, self.at);
        sections.push(level);
    }

    fn close_sections(&mut self, sections: &mut Vec<u8>) {
        while sections.pop().is_some() {
            self.close();
        }
    }

    /// `_newline | _eof` after a block's last token: the line end taken, and the next block's
    /// token (with `follow`). A leaf block open at the parse is closed after it.
    fn line_end_or_eof(&mut self, follow: Valid) -> R<Option<Tok>> {
        let leaf_open = matches!(self.top(), K::ThematicBreak);
        match self.next(V_LINE_OR_EOF)? {
            Some(t) if t.tok == LINE_ENDING => {
                self.take(t);
                let n = self.next(follow)?;
                if leaf_open {
                    // The line end's continuation was written in it; it ends before the next.
                    self.close_before(n);
                }
                Ok(n)
            }
            Some(t) if t.tok == TOKEN_EOF => {
                self.take(t);
                if leaf_open {
                    self.close();
                }
                self.next(follow)
            }
            _ => self.refuse(),
        }
    }

    /// The node open at the parse closed, at the last token before `next`.
    fn close_before(&mut self, _next: Option<Tok>) {
        self.close();
    }

    // ---- Leaf blocks.

    /// A paragraph from the parse: its lines, to the line end that ends it. Returns the next
    /// block's token, and the paragraph (its writer index) where a line end ended it.
    fn paragraph(&mut self) -> R<(Option<Tok>, Option<usize>)> {
        // A paragraph that may be a link reference definition (`[`, white space before it or
        // not): tree-sitter tries both, and the definition wins where it holds (its dynamic
        // precedence). So does the parser: the definition is read from a saved state, and the
        // paragraph from that state where it does not hold.
        {
            let mut q = self.at;
            while matches!(self.byte(q), b' ' | b'\t') {
                q += 1;
            }
            if self.byte(q) == b'[' && !(self.byte(q + 2) == b']' && matches!(self.byte(q + 1), b'x' | b'X' | b' ' | b'\t')) {
                let saved = self.save();
                if let Some(done) = self.link_reference_definition()? {
                    return Ok(done);
                }
                self.restore(saved);
                // A label that starts with an escape, or unindented with a character reference
                // (`[&amp; b](u)`), and is not a definition: tree-sitter's parse has an error.
                let reference = q == self.at && self.byte(q + 1) == b'&' && self.reference_at(q + 1).is_some();
                if reference || (self.byte(q + 1) == b'\\' && punct(self.byte(q + 2)).is_some()) {
                    return Err(Refusal::Parse(q as u32));
                }
            }
        }
        let p = self.open(K::Paragraph, 0, self.at);
        self.open(K::Inline, 0, self.at);
        loop {
            match self.next(V_PARAGRAPH)? {
                Some(t) if t.tok == SOFT_LINE_ENDING => self.take(t),
                Some(t) if t.tok == LINE_ENDING => {
                    self.close(); // the inline
                    self.take(t);
                    let n = self.next(V_AFTER_PARAGRAPH)?;
                    self.close(); // the paragraph
                    return Ok((n, Some(p)));
                }
                Some(t) if t.tok == TOKEN_EOF => {
                    self.close();
                    self.take(t);
                    self.close();
                    return Ok((self.next(V_BLOCK)?, None));
                }
                Some(t) => return Err(Refusal::Parse(t.start as u32)),
                None => {
                    let Some((lexed, end)) = self.line_token() else { return self.refuse() };
                    self.take_line_token(K::Inline, lexed, end);
                }
            }
        }
    }

    /// The parse's state, to go back to (`restore`): the writer's nodes, the open nodes, the
    /// position and the scanner's.
    fn save(&self) -> Saved {
        Saved { at: self.at, last: self.last, nodes: self.w.len(), open: self.open.len(), sc: self.sc.save() }
    }

    fn restore(&mut self, s: Saved) {
        self.at = s.at;
        self.last = s.last;
        self.w.truncate(s.nodes);
        self.open.truncate(s.open);
        self.sc.restore(s.sc);
        // A gap is within a node: the state saved has none of the tokens after it.
        self.gap.clear();
    }

    /// A link reference definition from the parse: white space, the label, `:`, the
    /// destination, the title, and the line's end, any of them after a soft line break (one
    /// before the destination, one before the title; the label and the title may hold more).
    /// `None` where it does not hold: the parse is to go back, and read a paragraph. A title on
    /// a line of its own that does not hold leaves the definition ending before it, as
    /// tree-sitter's parse does. A reference (`&`) is not read.
    fn link_reference_definition(&mut self) -> R<Option<(Option<Tok>, Option<usize>)>> {
        self.open(K::LinkReferenceDefinition, 0, self.at);
        // The parts, in order: 0 before the label, 1 the label, 2 before the destination, 3 the
        // destination, 4 before the title, 5 the title, 6 after it.
        let mut part = 0;
        let mut depth = 0usize;
        let mut angle = false;
        let mut close_title = 0u8;
        debug_assert!(self.gap.is_empty());
        // A soft line break taken before the destination, and where the definition ends if a
        // title after a line break does not hold (the state before that break).
        let mut broken = false;
        // With it, the soft line break's token: its scan is in the saved scanner state, so it is
        // taken again, not scanned again (a second scan would start a line there).
        let mut before_title: Option<(Saved, Tok)> = None;
        // Ends the definition at line end `t`.
        macro_rules! end {
            ($t:expr) => {{
                let t = $t;
                if part == 3 || part == 5 {
                    let parent = if part == 3 { K::LinkDestination } else { K::LinkTitle };
                    self.flush_gap(parent);
                    self.close();
                }
                self.flush_gap(K::LinkReferenceDefinition);
                self.take(t);
                let n = self.next(V_BLOCK)?;
                self.close();
                return Ok(Some((n, None)));
            }};
        }
        // The definition does not hold here: before a title on its own line, it ends there;
        // otherwise there is none.
        macro_rules! fail {
            () => {{
                match before_title.take() {
                    Some((s, t)) => {
                        self.restore(s);
                        part = 4;
                        end!(t)
                    }
                    None => {
                        self.gap.clear();
                        return Ok(None);
                    }
                }
            }};
        }
        loop {
            let at_line = self.at;
            match self.next(V_PARAGRAPH)? {
                Some(t) if t.tok == SOFT_LINE_ENDING => match part {
                    1 | 5 => {
                        self.take(t);
                        continue;
                    }
                    2 if !broken => {
                        broken = true;
                        self.take(t);
                        continue;
                    }
                    3 | 4 if before_title.is_none() => {
                        // The destination ends; a title may follow on the next line. The state
                        // before the break: where the definition ends if it does not.
                        if part == 3 {
                            if depth > 0 {
                                return Ok(None);
                            }
                            self.flush_gap(K::LinkDestination);
                            self.close();
                        }
                        self.flush_gap(K::LinkReferenceDefinition);
                        part = 4;
                        let mut s = self.save();
                        s.at = at_line;
                        before_title = Some((s, t));
                        self.take(t);
                        continue;
                    }
                    6 => end!(t),
                    _ => fail!(),
                },
                Some(t) if t.tok == LINE_ENDING || t.tok == TOKEN_EOF => match part {
                    3 if depth > 0 => return Ok(None),
                    3 | 4 | 6 => end!(t),
                    _ => fail!(),
                },
                Some(t) => return Err(Refusal::Parse(t.start as u32)),
                None => {}
            }
            let Some((lexed, end)) = self.line_token() else { return self.refuse() };
            let c = self.byte(self.at);
            let reference = if c == b'&' { self.reference_at(self.at) } else { None };
            match part {
                0 if lexed == Lexed::Space => {}
                0 => {
                    debug_assert_eq!(c, b'[');
                    self.flush_gap(K::LinkReferenceDefinition);
                    self.open(K::LinkLabel, 0, self.at);
                    self.gap_token(lexed, end);
                    part = 1;
                    continue;
                }
                1 if c == b'[' => {
                    self.gap.clear();
                    return Ok(None);
                }
                1 if c == b']' => {
                    let empty = self.w.len() == self.open.last().expect("the label").0 + 1 && self.gap.len() == 1 && self.gap[0].1 + 1 == self.at;
                    if empty || self.byte(self.at + 1) != b':' {
                        return Ok(None);
                    }
                    self.gap_token(lexed, end);
                    self.flush_gap(K::LinkLabel);
                    self.close();
                    let (colon, cend) = self.line_token().expect("a colon");
                    self.gap_token(colon, cend);
                    part = 2;
                    continue;
                }
                2 if lexed != Lexed::Space => {
                    if c == b')' {
                        return Ok(None);
                    }
                    self.flush_gap(K::LinkReferenceDefinition);
                    self.open(K::LinkDestination, 0, self.at);
                    angle = c == b'<';
                    part = 3;
                    if angle {
                        self.gap_token(lexed, end);
                        continue;
                    }
                }
                3 if angle && c == b'<' => return Ok(None),
                3 if angle && c == b'>' => {
                    self.gap_token(lexed, end);
                    self.flush_gap(K::LinkDestination);
                    self.close();
                    // White space or the line's end next.
                    if !matches!(self.byte(self.at), b' ' | b'\t' | b'\n' | b'\r') && self.at < self.src.len() {
                        return Ok(None);
                    }
                    part = 4;
                    continue;
                }
                3 if !angle && lexed == Lexed::Space => {
                    if depth > 0 {
                        return Ok(None);
                    }
                    self.flush_gap(K::LinkDestination);
                    self.close();
                    part = 4;
                }
                3 if !angle && c == b')' && depth == 0 => return Ok(None),
                4 if lexed == Lexed::Space => {}
                4 => {
                    // A title, after white space or a line break.
                    let spaced = before_title.is_some() || matches!(self.byte(self.at.wrapping_sub(1)), b' ' | b'\t');
                    if !spaced || !matches!(c, b'"' | b'\'' | b'(') {
                        fail!();
                    }
                    self.flush_gap(K::LinkReferenceDefinition);
                    self.open(K::LinkTitle, 0, self.at);
                    self.title_at = self.at;
                    close_title = if c == b'(' { b')' } else { c };
                    self.gap_token(lexed, end);
                    part = 5;
                    continue;
                }
                5 if c == close_title => {
                    self.gap_token(lexed, end);
                    self.flush_gap(K::LinkTitle);
                    self.close();
                    part = 6;
                    continue;
                }
                5 if close_title == b')' && c == b'(' => fail!(),
                6 if lexed == Lexed::Space => {}
                6 => fail!(),
                _ => {}
            }
            let parent = match part {
                1 => K::LinkLabel,
                3 => K::LinkDestination,
                5 => K::LinkTitle,
                _ => K::LinkReferenceDefinition,
            };
            if part == 3 && !angle {
                match c {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
            }
            if c == b'\\' && punct(self.byte(self.at + 1)).is_some() {
                self.flush_gap(parent);
                self.leaf(K::BackslashEscape, 0, self.at, self.at + 2);
                self.at += 2;
                self.last = self.at;
                continue;
            }
            if let Some((k, e)) = reference {
                self.flush_gap(parent);
                self.leaf(k, 0, self.at, e);
                self.at = e;
                self.last = e;
                continue;
            }
            self.gap_token(lexed, end);
        }
    }

    /// A setext heading: the paragraph at writer index `p` is its content, `t` its underline.
    fn setext(&mut self, p: usize, t: Tok) -> R<Option<Tok>> {
        let start = self.w.start(p);
        self.w.insert(p, key(K::SetextHeading, 0), start);
        self.w.set_key(p + 1, key(K::Paragraph, f::HEADING_CONTENT));
        self.open.push((p, K::SetextHeading));
        let kind = if t.tok == SETEXT_H1_UNDERLINE { K::SetextH1Underline } else { K::SetextH2Underline };
        self.leaf(kind, 0, t.start, t.end);
        self.take(t);
        match self.next(V_LINE_OR_EOF)? {
            Some(e) if e.tok == LINE_ENDING => {
                self.take(e);
                let n = self.next(V_BLOCK)?;
                self.close();
                Ok(n)
            }
            Some(e) if e.tok == TOKEN_EOF => {
                self.take(e);
                self.close();
                self.next(V_BLOCK)
            }
            _ => self.refuse(),
        }
    }

    /// An ATX heading from its marker `t`: the marker, white space, the content (`inline`), and
    /// its line end.
    fn atx_heading(&mut self, t: Tok, level: u8) -> R<Option<Tok>> {
        self.open(K::AtxHeading, 0, t.start);
        self.leaf(ATX[usize::from(level - 1)], 0, t.start, t.end);
        self.take(t);
        let mut inline = false;
        let mut first = true;
        loop {
            match self.next(V_LINE_END)? {
                Some(e) if e.tok == LINE_ENDING => {
                    if inline {
                        self.close();
                    }
                    self.take(e);
                    let n = self.next(V_BLOCK)?;
                    self.close();
                    return Ok(n);
                }
                Some(e) => return Err(Refusal::Parse(e.start as u32)),
                None => {
                    let Some((lexed, end)) = self.line_token() else { return self.refuse() };
                    if !(first && lexed == Lexed::Space) && !inline {
                        self.open(K::Inline, f::HEADING_CONTENT, self.at);
                        inline = true;
                    }
                    first = false;
                    self.take_line_token(K::Inline, lexed, end);
                }
            }
        }
    }

    // ---- Containers.

    /// A list from its first marker: its items, while the next block is a marker of its type.
    fn list(&mut self, first: Tok) -> R<Option<Tok>> {
        let ty = marker_type(first.tok).expect("a marker");
        self.open(K::List, 0, first.start);
        let mut t = first;
        loop {
            let next = self.list_item(t, ty)?;
            match next {
                Some(n) if marker_type(n.tok) == Some(ty) => t = n,
                _ => {
                    self.close();
                    return Ok(next);
                }
            }
        }
    }

    fn list_item(&mut self, t: Tok, ty: u8) -> R<Option<Tok>> {
        self.open(K::ListItem, 0, t.start);
        self.leaf(MARKERS[usize::from(ty)], 0, t.start, t.end);
        self.take(t);
        // A marker that ends the text: tree-sitter-md's parse has an error.
        if self.at >= self.src.len() {
            return self.refuse();
        }
        let first = self.next(V_BLOCK)?;
        // A task-list marker first: then white space, and a paragraph.
        if first.is_none()
            && let Some(k) = self.task_marker()
        {
            self.leaf(k, 0, self.at, self.at + 3);
            self.at += 3;
            self.last = self.at;
            if !matches!(self.byte(self.at), b' ' | b'\t') {
                return self.refuse();
            }
            // The white space, then the paragraph (the scanner first: it finds nothing there).
            let (lexed, end) = self.line_token().expect("white space");
            debug_assert_eq!(lexed, Lexed::Space);
            if self.next(V_PARAGRAPH)?.is_some() {
                return self.refuse();
            }
            self.at = end;
            self.last = end;
            let (n, _) = self.paragraph()?;
            let close = self.blocks(Ctx::Container, n)?;
            return self.end_container(close);
        }
        let close = self.blocks(Ctx::Item, first)?;
        self.end_container(close)
    }

    /// A container's `_block_close` (`close`) taken, the container closed after it, and the next
    /// block's token.
    fn end_container(&mut self, close: Option<Tok>) -> R<Option<Tok>> {
        let Some(c) = close else { return self.refuse() };
        self.take(c);
        // `_block_close (block_continuation)?`: written in the container by `next`.
        let n = self.next(V_BLOCK)?;
        self.close();
        Ok(n)
    }

    fn block_quote(&mut self, t: Tok) -> R<Option<Tok>> {
        self.open(K::BlockQuote, 0, t.start);
        self.leaf(K::BlockQuoteMarker, 0, t.start, t.end);
        self.take(t);
        let first = self.next(V_BLOCK)?;
        let close = self.blocks(Ctx::Container, first)?;
        self.end_container(close)
    }

    /// A fenced code block from its opening delimiter: the info string, the content's lines, the
    /// closing delimiter (or the close of an enclosing block, or the text's end).
    fn fenced(&mut self, t: Tok) -> R<Option<Tok>> {
        let end_tok = if t.tok == FENCED_CODE_BLOCK_START_BACKTICK { FENCED_CODE_BLOCK_END_BACKTICK } else { FENCED_CODE_BLOCK_END_TILDE };
        self.open(K::FencedCodeBlock, 0, t.start);
        self.leaf(K::FencedCodeBlockDelimiter, 0, t.start, t.end);
        self.take(t);
        self.info_string()?;
        // The content's lines.
        let mut content = false;
        loop {
            match self.next(bit(end_tok) | bit(LINE_ENDING))? {
                Some(e) if e.tok == end_tok => {
                    if content {
                        self.close();
                    }
                    self.leaf(K::FencedCodeBlockDelimiter, 0, e.start, e.end);
                    self.take(e);
                    match self.sc.scan_at(self.at, bit(CLOSE_BLOCK)) {
                        Some(_) => {}
                        None => return self.refuse(),
                    }
                    match self.next(V_LINE_OR_EOF)? {
                        Some(l) if l.tok == LINE_ENDING || l.tok == TOKEN_EOF => self.take(l),
                        _ => return self.refuse(),
                    }
                    let Some(c) = self.next(0)? else { return self.refuse() };
                    if c.tok != BLOCK_CLOSE {
                        return self.refuse();
                    }
                    self.take(c);
                    let n = self.next(V_BLOCK)?;
                    self.close();
                    return Ok(n);
                }
                Some(e) if e.tok == LINE_ENDING => {
                    if !content {
                        self.open(K::CodeFenceContent, 0, e.start);
                        content = true;
                    }
                    self.take(e);
                }
                Some(e) if e.tok == BLOCK_CLOSE => {
                    if content {
                        self.close();
                    }
                    self.take(e);
                    let n = self.next(V_BLOCK)?;
                    self.close();
                    return Ok(n);
                }
                Some(e) => return Err(Refusal::Parse(e.start as u32)),
                None => {
                    if !content {
                        self.open(K::CodeFenceContent, 0, self.at);
                        content = true;
                    }
                    // A line: its tokens, to its end.
                    loop {
                        match self.next(V_LINE_OR_EOF)? {
                            Some(l) if l.tok == LINE_ENDING => {
                                self.take(l);
                                break;
                            }
                            Some(l) if l.tok == TOKEN_EOF => {
                                self.take(l);
                                break;
                            }
                            Some(l) => return Err(Refusal::Parse(l.start as u32)),
                            None => {
                                let Some((lexed, end)) = self.line_token() else { return self.refuse() };
                                self.take_line_token(K::CodeFenceContent, lexed, end);
                            }
                        }
                    }
                }
            }
        }
    }

    /// A fenced code block's info string, white space before it, and the line end after it.
    fn info_string(&mut self) -> R<()> {
        let mut info = false;
        // In the language (its first word and punctuation), until white space, `,`, `{` or `}`.
        let mut language = false;
        let mut braces = false;
        let mut first = true;
        loop {
            match self.next(V_LINE_END)? {
                Some(e) if e.tok == LINE_ENDING => {
                    if language {
                        self.close();
                    }
                    if info {
                        self.close();
                    }
                    self.take(e);
                    return Ok(());
                }
                Some(e) => return Err(Refusal::Parse(e.start as u32)),
                None => {
                    let Some((lexed, end)) = self.line_token() else { return self.refuse() };
                    let c = self.byte(self.at);
                    // A character reference is a node of the info string, read as a word is.
                    let reference = if c == b'&' { self.reference_at(self.at) } else { None };
                    let (lexed, end) = if let Some((_, e)) = reference { (Lexed::Word, e) } else { (lexed, end) };
                    if first && lexed == Lexed::Space {
                        // White space before the info string: outside it.
                        self.at = end;
                        self.last = end;
                        continue;
                    }
                    if first {
                        if c == b',' {
                            // No info string starts with `,`.
                            return self.refuse();
                        }
                        self.open(K::InfoString, f::INFO_STRING, self.at);
                        info = true;
                        first = false;
                        if matches!(c, b'{' | b'}') {
                            // The brace form: braces, then a language or white space.
                            braces = true;
                        } else {
                            self.open(K::Language, 0, self.at);
                            language = true;
                        }
                    } else if braces {
                        if !matches!(c, b'{' | b'}') {
                            braces = false;
                            if lexed != Lexed::Space && c != b',' {
                                self.open(K::Language, 0, self.at);
                                language = true;
                            } else if c == b',' {
                                return self.refuse();
                            }
                        }
                    } else if language && (lexed == Lexed::Space || matches!(c, b',' | b'{' | b'}')) {
                        self.close();
                        language = false;
                    }
                    // A backslash and a punctuation character: `backslash_escape`.
                    if c == b'\\' && punct(self.byte(self.at + 1)).is_some() {
                        self.leaf(K::BackslashEscape, 0, self.at, self.at + 2);
                        self.at += 2;
                        self.last = self.at;
                        continue;
                    }
                    if let Some((k, e)) = reference {
                        self.leaf(k, 0, self.at, e);
                        self.at = e;
                        self.last = e;
                        continue;
                    }
                    let parent = if language { K::Language } else { K::InfoString };
                    self.take_line_token(parent, lexed, end);
                }
            }
        }
    }

    /// An indented code block from its first chunk's start: chunks, and the blank lines between
    /// and after them.
    fn indented(&mut self, t: Tok) -> R<Option<Tok>> {
        self.open(K::IndentedCodeBlock, 0, t.start);
        let mut t = t;
        loop {
            // A chunk: lines, to its `_block_close`.
            self.take(t);
            loop {
                match self.next(V_LINE_END)? {
                    Some(e) if e.tok == LINE_ENDING => self.take(e),
                    Some(e) if e.tok == BLOCK_CLOSE => {
                        self.take(e);
                        break;
                    }
                    Some(e) => return Err(Refusal::Parse(e.start as u32)),
                    None => {
                        let Some((lexed, end)) = self.line_token() else { return self.refuse() };
                        self.take_line_token(K::IndentedCodeBlock, lexed, end);
                    }
                }
            }
            // Further chunks and blank lines.
            let mut n = self.next(V_BLOCK)?;
            loop {
                match n {
                    Some(b) if b.tok == BLANK_LINE_START => {
                        self.take(b);
                        match self.next(V_LINE_OR_EOF)? {
                            Some(l) if l.tok == LINE_ENDING || l.tok == TOKEN_EOF => self.take(l),
                            _ => return self.refuse(),
                        }
                        n = self.next(V_BLOCK)?;
                    }
                    Some(c) if c.tok == INDENTED_CHUNK_START => {
                        t = c;
                        break;
                    }
                    _ => {
                        self.close();
                        return Ok(n);
                    }
                }
            }
        }
    }

    /// An HTML block from its start: its lines, to its end condition, and its `_block_close`.
    fn html_block(&mut self, t: Tok) -> R<Option<Tok>> {
        let kind = t.tok;
        self.open(K::HtmlBlock, 0, t.start);
        self.take(t);
        // Where an end condition has been met (`_close_block`): the rest are lines to the close.
        let mut at_line_start = false;
        loop {
            let mut valid = V_LINE_END;
            if kind == HTML_BLOCK_1_START {
                valid |= bit(HTML_BLOCK_1_END);
            }
            if (kind == HTML_BLOCK_6_START || kind == HTML_BLOCK_7_START) && at_line_start {
                valid |= bit(BLANK_LINE_START);
            }
            match self.next(valid)? {
                Some(e) if e.tok == LINE_ENDING => {
                    self.take(e);
                    at_line_start = true;
                }
                Some(e) if e.tok == BLOCK_CLOSE => {
                    self.take(e);
                    let n = self.next(V_BLOCK)?;
                    self.close();
                    return Ok(n);
                }
                Some(e) if e.tok == HTML_BLOCK_1_END => {
                    self.take(e);
                    self.close_block()?;
                    at_line_start = false;
                }
                Some(e) if e.tok == BLANK_LINE_START => {
                    // `_newline _blank_line _close_block`. In a list item or a block quote,
                    // tree-sitter-md's parse has an error here: not read.
                    if self.open.iter().any(|&(_, k)| matches!(k, K::ListItem | K::BlockQuote)) {
                        return Err(Refusal::Parse(e.start as u32));
                    }
                    self.take(e);
                    match self.next(V_LINE_OR_EOF)? {
                        Some(l) if l.tok == LINE_ENDING => self.take(l),
                        _ => return self.refuse(),
                    }
                    self.close_block()?;
                    at_line_start = true;
                }
                Some(e) => return Err(Refusal::Parse(e.start as u32)),
                None => {
                    at_line_start = false;
                    // The end conditions of blocks 2, 3 and 5: a token of their own, longer than
                    // the punctuation it starts with. Block 4's `>` is the line's own `>` token,
                    // and tree-sitter's parse keeps reading it as one: block 4 ends with its
                    // container (`<!DOCTYPE html>` runs to the text's end).
                    let (closer, k): (&[u8], K) = match kind {
                        HTML_BLOCK_2_START => (b"-->", K::TCommentEnd),
                        HTML_BLOCK_3_START => (b"?>", K::TQGt),
                        HTML_BLOCK_5_START => (b"]]>", K::TCdataEnd),
                        _ => (b"", K::TGt),
                    };
                    if !closer.is_empty() && self.src[self.at..].starts_with(closer) {
                        if stores(K::HtmlBlock, k) {
                            self.leaf(k, 0, self.at, self.at + closer.len());
                        }
                        self.at += closer.len();
                        self.last = self.at;
                        self.close_block()?;
                        continue;
                    }
                    let Some((lexed, end)) = self.line_token() else { return self.refuse() };
                    self.take_line_token(K::HtmlBlock, lexed, end);
                }
            }
        }
    }

    /// `_close_block`: the scanner told to close the innermost block after the next line end.
    fn close_block(&mut self) -> R<()> {
        match self.sc.scan_at(self.at, bit(CLOSE_BLOCK)) {
            Some(_) => Ok(()),
            None => self.refuse(),
        }
    }

    // ---- Tables.

    fn table(&mut self, t: Tok) -> R<Option<Tok>> {
        self.open(K::PipeTable, 0, t.start);
        self.take(t);
        let e = self.row(K::PipeTableHeader, V_LINE_END)?;
        if e.tok != LINE_ENDING {
            return self.refuse();
        }
        self.take(e);
        if self.next(0)?.is_some() {
            return self.refuse();
        }
        let mut e = self.delimiter_row()?;
        loop {
            match e.tok {
                PIPE_TABLE_LINE_ENDING => {
                    self.take(e);
                    e = match self.next(V_TABLE)? {
                        None => self.row(K::PipeTableRow, V_TABLE)?,
                        Some(n) => n,
                    };
                }
                LINE_ENDING => {
                    self.take(e);
                    let n = self.next(V_BLOCK)?;
                    self.close();
                    return Ok(n);
                }
                TOKEN_EOF => {
                    self.take(e);
                    self.close();
                    return self.next(V_BLOCK);
                }
                _ => return Err(Refusal::Parse(e.start as u32)),
            }
        }
    }

    /// A row's tokens, to the scanner's token that ends it (scanned, not taken): `(start, end,
    /// class, punctuation)`, the class 0 a pipe, 1 white space, 2 a cell's token.
    fn row_tokens(&mut self, valid: Valid) -> R<(Vec<(usize, usize, u8, Option<K>)>, Tok)> {
        let mut toks = Vec::new();
        loop {
            if let Some(e) = self.next(valid)? {
                return Ok((toks, e));
            }
            let Some((lexed, end)) = self.line_token() else { return self.refuse() };
            let c = self.byte(self.at);
            let (end, class, k) = if c == b'|' {
                (end, 0, None)
            } else if c == b'\\' && punct(self.byte(self.at + 1)).is_some() {
                // `_backslash_escape`: hidden.
                (self.at + 2, 2, None)
            } else if lexed == Lexed::Space {
                (end, 1, None)
            } else if let Lexed::Punct(k) = lexed {
                (end, 2, Some(k))
            } else {
                (end, 2, None)
            };
            toks.push((self.at, end, class, k));
            self.at = end;
        }
    }

    /// A header or body row: cells between pipes. Returns the token that ends it.
    fn row(&mut self, kind: K, valid: Valid) -> R<Tok> {
        let (toks, end_tok) = self.row_tokens(valid)?;
        let Some(&(row_start, _, _, _)) = toks.first() else { return self.refuse() };
        let row_end = toks.last().map_or(row_start, |t| t.1);
        let n_row = self.w.push(key(kind, 0), row_start as u32);
        let pipes: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].2 == 0).collect();
        // Two pipes with nothing between: the grammar has no empty cell.
        if pipes.windows(2).any(|w| w[1] == w[0] + 1) {
            return Err(Refusal::Parse(row_start as u32));
        }
        let lead = pipes.first().copied().filter(|&i| toks[..i].iter().all(|t| t.2 == 1));
        let trail = pipes.last().copied().filter(|&i| toks[i + 1..].iter().all(|t| t.2 == 1));
        let mut i = 0;
        let mut cells = 0;
        while i < toks.len() {
            let (s, e, class, _) = toks[i];
            if class == 0 {
                // The first and last pipes are kept, those between cells are not.
                if Some(i) == lead || Some(i) == trail {
                    self.leaf(K::TPipe, 0, s, e);
                }
                i += 1;
                continue;
            }
            let mut j = i;
            while j < toks.len() && toks[j].2 != 0 {
                j += 1;
            }
            let seg = &toks[i..j];
            let before_pipe = j < toks.len();
            match seg.iter().position(|t| t.2 == 2) {
                Some(k) => {
                    // The cell runs to the pipe, white space included; at the row's end, to its
                    // last token (the line end took the white space after), or at the text's
                    // end, to the end.
                    let from = seg[k].0;
                    let to = if before_pipe || end_tok.tok == TOKEN_EOF { seg[seg.len() - 1].1 } else { seg.iter().rev().find(|t| t.2 == 2).map_or(from, |t| t.1) };
                    let n = self.w.push(key(K::PipeTableCell, 0), from as u32);
                    for &(s, e, class, pk) in &seg[k..] {
                        if class == 2
                            && let Some(pk) = pk
                            && stores(K::PipeTableCell, pk)
                        {
                            self.leaf(pk, 0, s, e);
                        }
                    }
                    self.w.close(n, to as u32);
                    cells += 1;
                }
                None => {
                    // White space alone between two pipes is a cell; before the first pipe, or
                    // after the last, it is not.
                    if i > 0 && before_pipe {
                        self.leaf(K::PipeTableCell, 0, seg[0].0, seg[seg.len() - 1].1);
                        cells += 1;
                    }
                }
            }
            i = j;
        }
        if cells == 0 {
            return Err(Refusal::Parse(row_start as u32));
        }
        self.w.close(n_row, row_end as u32);
        self.last = row_end;
        Ok(end_tok)
    }

    /// The delimiter row: `|`, cells of `-` with `:` at either end, white space. Returns the token
    /// that ends it.
    fn delimiter_row(&mut self) -> R<Tok> {
        let mut toks: Vec<(usize, u8)> = Vec::new();
        let e = loop {
            if let Some(e) = self.next(V_TABLE)? {
                break e;
            }
            let c = self.byte(self.at);
            match c {
                b'|' | b':' | b'-' => {
                    toks.push((self.at, c));
                    self.at += 1;
                }
                b' ' | b'\t' => {
                    let s = self.at;
                    while matches!(self.byte(self.at), b' ' | b'\t') {
                        self.at += 1;
                    }
                    toks.push((s, b' '));
                }
                _ => return self.refuse(),
            }
        };
        let Some(&(row_start, _)) = toks.first() else { return self.refuse() };
        let n_row = self.w.push(key(K::PipeTableDelimiterRow, 0), row_start as u32);
        let pipes: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].1 == b'|').collect();
        // An empty cell between two pipes is a delimiter cell tree-sitter inserts, MISSING: an error.
        if pipes.windows(2).any(|w| toks[w[0] + 1..w[1]].iter().all(|t| t.1 == b' ')) {
            return Err(Refusal::Parse(row_start as u32));
        }
        let lead = pipes.first().copied().filter(|&i| toks[..i].iter().all(|t| t.1 == b' '));
        // The trailing `|` is stored where the row could end without it: after a second cell, which
        // may be the grammar's last pipeless cell. After a lone cell, `cell |` is the only way on
        // (the pipeless cell follows at least one), so the `|` is implied and left out.
        let cells = (0..toks.len()).filter(|&i| matches!(toks[i].1, b':' | b'-') && (i == 0 || matches!(toks[i - 1].1, b' ' | b'|'))).count();
        let trail = pipes.last().copied().filter(|&i| cells > 1 && toks[i + 1..].iter().all(|t| t.1 == b' '));
        let mut i = 0;
        let mut cells_before_pipe = 0;
        let mut row_end = row_start;
        while i < toks.len() {
            let (s, c) = toks[i];
            match c {
                b'|' => {
                    if Some(i) == lead || Some(i) == trail {
                        self.leaf(K::TPipe, 0, s, s + 1);
                    }
                    row_end = s + 1;
                    i += 1;
                }
                b' ' => i += 1,
                _ => {
                    // A cell: `:`? `-`+ `:`?
                    let n = self.w.push(key(K::PipeTableDelimiterCell, 0), s as u32);
                    let mut j = i;
                    if toks[j].1 == b':' {
                        self.leaf(K::PipeTableAlignLeft, 0, toks[j].0, toks[j].0 + 1);
                        j += 1;
                    }
                    let dashes = j;
                    while j < toks.len() && toks[j].1 == b'-' {
                        if stores(K::PipeTableDelimiterCell, K::TMinus) {
                            self.leaf(K::TMinus, 0, toks[j].0, toks[j].0 + 1);
                        }
                        j += 1;
                    }
                    if j == dashes {
                        return Err(Refusal::Parse(s as u32));
                    }
                    if j < toks.len() && toks[j].1 == b':' {
                        self.leaf(K::PipeTableAlignRight, 0, toks[j].0, toks[j].0 + 1);
                        j += 1;
                    }
                    let end = toks[j - 1].0 + 1;
                    self.w.close(n, end as u32);
                    row_end = end;
                    let mut k = j;
                    while k < toks.len() && toks[k].1 == b' ' {
                        k += 1;
                    }
                    if k < toks.len() {
                        if toks[k].1 != b'|' {
                            return Err(Refusal::Parse(toks[k].0 as u32));
                        }
                        cells_before_pipe += 1;
                    }
                    i = j;
                }
            }
        }
        if cells_before_pipe == 0 {
            return Err(Refusal::Parse(row_start as u32));
        }
        self.w.close(n_row, row_end as u32);
        self.last = row_end;
        Ok(e)
    }
}
