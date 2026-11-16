//! The parser: tree-sitter-md-inline's grammar (`markdown-inline/grammar.js`) as tree-sitter's GLR
//! parse reads it.
//!
//! Where the grammar declares a conflict (a `[` that opens a link or is text, an emphasis opener
//! that is lone or opens emphasis or strong emphasis, a `!` or `<` that opens an image or a tag or is
//! text, a destination or a title), tree-sitter keeps a version of each reading and drops a version
//! where it meets a token it cannot take. Here a version (`V`) is a stack of frames (`F`): the
//! constructs open around the parse, a sequence of inline elements (`F::Seq`) on top of the one it
//! is in. The versions advance in step, a token each from the leftmost; two that reach the same
//! place in the same state merge, keeping the higher score; at the text's end the best survivor's
//! events (`Ev`) are its tree.
//!
//! The score is the grammar's dynamic precedence, summed over a tree's nodes: tree-sitter's choice
//! between versions (`ts_parser__select_tree`) by it. Where two different trees tie, tree-sitter
//! compares their internal structure, which this parser does not keep: such a text is refused, as
//! is one that needs more versions than tree-sitter keeps (`MAX_VERSIONS`).
//!
//! A token is lexed in the state of the version that reads it; after a fork, the token that decides
//! between the readings is lexed in the state they share (tree-sitter's state before the fork): its
//! valid tokens are the union of theirs (`V::group`).

use crate::block::Blocks;
use crate::kinds::{self, K};
use crate::scan::{D, Ext, Sc, scan, x};
use crate::{Refusal, Tape, key};
use rerail_simd8::swar::{Class, class_end, class_of, run_end};
use rerail_tape_format::{Redo, Widths, Writer};

type R<T> = Result<T, Refusal>;

/// tree-sitter's `MAX_VERSION_COUNT`: past it, tree-sitter drops versions by a measure of its own.
const MAX_VERSIONS: usize = 6;

/// tree-sitter's `MAX_LINK_COUNT` (stack.c): the links a stack node keeps.
const MAX_LINKS: u32 = 8;

// ---- The lexer: the grammar's own tokens, longest match among those the state takes.

/// The grammar's tokens a state takes, a bit each.
mod l {
    pub const WORD: u16 = 1;
    pub const DIGITS: u16 = 2;
    pub const WS: u16 = 4;
    pub const NL: u16 = 8;
    pub const PUNCT: u16 = 16;
    /// `backslash_escape`.
    pub const ESC: u16 = 32;
    /// `entity_reference`, `numeric_character_reference`.
    pub const ENT: u16 = 64;
    /// `uri_autolink`, `email_autolink`.
    pub const AUTO: u16 = 128;
    /// `<!--`, `<?`, `<![CDATA[`, `<!X`: the HTML constructs' starts (refused).
    pub const HTML: u16 = 256;
    /// `hard_line_break`: a `\` or two blanks before a line's end.
    pub const HARD: u16 = 512;
    /// `_attribute_name`.
    pub const ATTR: u16 = 1024;
    /// An unquoted attribute value.
    pub const UNQ: u16 = 2048;
    pub const TEXT: u16 = WORD | DIGITS | WS | NL | PUNCT | ESC | ENT | AUTO | HTML | HARD;
    pub const LABEL: u16 = WORD | DIGITS | WS | NL | PUNCT | ESC | ENT | HTML;
    pub const LINK: u16 = WORD | DIGITS | WS | NL | PUNCT | ESC | ENT;
    pub const CODE: u16 = WORD | DIGITS | WS | NL | PUNCT | HTML;
}

/// A token.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum T {
    Word,
    Digits,
    Ws,
    Nl,
    /// A punctuation character.
    P(u8),
    Esc,
    Ent,
    Num,
    Uri,
    Email,
    /// A hard line break; `bs`: by a backslash (which the tape stores).
    Hard { bs: bool },
    AttrName,
    Unq,
    X(Ext),
    Eof,
}

#[derive(Clone, Copy, Debug)]
struct Tok {
    t: T,
    s: usize,
    e: usize,
    /// The scanner's state after the token.
    sc: Sc,
}

const SCHEME: Class = {
    let mut t = class_of(b"+.-");
    let mut c = 0;
    while c < 256 {
        t[c] |= (c as u8).is_ascii_alphanumeric();
        c += 1;
    }
    t
};
/// A URI autolink's characters after its scheme: not white space, a line's end, `<` or `>`.
const URI: Class = {
    let mut t = [true; 256];
    let stop = b" \t\r\n<>";
    let mut i = 0;
    while i < stop.len() {
        t[stop[i] as usize] = false;
        i += 1;
    }
    t
};
const EMAIL_LOCAL: Class = {
    let mut t = class_of(b".!#$%&'*+/=?^_`{|}~-");
    let mut c = 0;
    while c < 256 {
        t[c] |= (c as u8).is_ascii_alphanumeric();
        c += 1;
    }
    t
};
const ALNUM_DASH: Class = {
    let mut t = class_of(b"-");
    let mut c = 0;
    while c < 256 {
        t[c] |= (c as u8).is_ascii_alphanumeric();
        c += 1;
    }
    t
};
const ALNUM: Class = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 256 {
        t[c] = (c as u8).is_ascii_alphanumeric();
        c += 1;
    }
    t
};
const HEX: Class = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 256 {
        t[c] = (c as u8).is_ascii_hexdigit();
        c += 1;
    }
    t
};
const DIGIT: Class = {
    let mut t = [false; 256];
    let mut c = b'0';
    while c <= b'9' {
        t[c as usize] = true;
        c += 1;
    }
    t
};
/// `_attribute_name`'s characters after its first.
const ATTR_REST: Class = {
    let mut t = class_of(b"_.:-");
    let mut c = 0;
    while c < 256 {
        t[c] |= (c as u8).is_ascii_alphanumeric();
        c += 1;
    }
    t
};
/// An unquoted attribute value's characters.
const UNQ: Class = {
    let mut t = [true; 256];
    let stop = b" \t\r\n\"'=<>`";
    let mut i = 0;
    while i < stop.len() {
        t[stop[i] as usize] = false;
        i += 1;
    }
    t
};

/// Whether `c` is one of a word's characters (`_word_no_digit`'s class).
#[inline]
fn word_char(c: u8) -> bool {
    !(c.is_ascii_punctuation() || c.is_ascii_digit() || matches!(c, b' ' | b'\t' | b'\n' | b'\r'))
}

/// A punctuation character's token kind.
fn punct(c: u8) -> K {
    match c {
        b'[' => K::TLBracket,
        b']' => K::TRBracket,
        b'<' => K::TLt,
        b'>' => K::TGt,
        b'!' => K::TBang,
        b'"' => K::TDQuote,
        b'#' => K::THash,
        b'$' => K::TDollar,
        b'%' => K::TPercent,
        b'&' => K::TAmp,
        b'\'' => K::TQuote,
        b'*' => K::TStar,
        b'+' => K::TPlus,
        b',' => K::TComma,
        b'-' => K::TMinus,
        b'.' => K::TDot,
        b'/' => K::TSlash,
        b':' => K::TColon,
        b';' => K::TSemi,
        b'=' => K::TEq,
        b'?' => K::TQuestion,
        b'@' => K::TAt,
        b'\\' => K::TBackslash,
        b'^' => K::TCaret,
        b'_' => K::TUnderscore,
        b'`' => K::TBacktick,
        b'{' => K::TLBrace,
        b'|' => K::TPipe,
        b'}' => K::TRBrace,
        b'~' => K::TTilde,
        b'(' => K::TLParen,
        b')' => K::TRParen,
        _ => unreachable!("a punctuation character"),
    }
}

/// The stored leaf of a token in a destination or title: an escape, a reference, punctuation.
fn leaf_kind(t: T) -> Option<K> {
    match t {
        T::Esc => Some(K::BackslashEscape),
        T::Ent => Some(K::EntityReference),
        T::Num => Some(K::NumericCharacterReference),
        T::P(c) => Some(punct(c)),
        _ => None,
    }
}

/// The end of a line end at `at` (`\n`, `\r\n`, `\r`), or `None`.
#[inline]
fn newline_end(src: &[u8], at: usize) -> Option<usize> {
    match src.get(at) {
        Some(b'\n') => Some(at + 1),
        Some(b'\r') => Some(if src.get(at + 1) == Some(&b'\n') { at + 2 } else { at + 1 }),
        _ => None,
    }
}

// ---- The versions.

/// The state of an inline link's or image's tail, `(…)`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Tl {
    /// After `(`: white space, then a destination, a title, or `)`.
    Lead,
    /// In a destination `<…>`.
    Angle,
    /// In a destination, `depth` parentheses deep.
    Dest { depth: u8 },
    /// After the destination; `ws`: after white space (a title may follow).
    AfterDest { ws: bool },
    /// In a title closed by `q`; `nl`: its last token a line's end.
    Title { q: u8, nl: bool },
    AfterTitle,
}

/// The state of an HTML tag.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Tg {
    /// After `<`: a name, or `/`.
    Open0,
    /// After `</`: a name.
    Close0,
    /// In the name.
    Name { closing: bool },
    /// After white space: an attribute, `/` or `>` (a closing tag's: `>`).
    Ws { closing: bool },
    /// After an attribute's name: white space, `=`.
    Attr,
    /// After `=`: white space, the value.
    Eq,
    /// In a value quoted by `q`.
    Val { q: u8 },
    /// After a value.
    AfterVal,
    /// After a self-closing `/`.
    Slash,
}

/// A frame: a construct open around the parse.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum F {
    /// A sequence of inline elements (`_inline` and its variants): `excl` the delimiter whose lone
    /// opener it does not take, `no_link` whether it is in a link's text, `any` whether it has an
    /// element.
    Seq { excl: Option<D>, no_link: bool, any: bool },
    /// Emphasis (or strikethrough) by `d`, its sequence above it; `inner`: strong emphasis's own.
    Emph { d: D, inner: bool, no_link: bool },
    /// Strong emphasis by `d`: before its inner emphasis's opener, or (`closing`) its closer.
    Strong { d: D, closing: bool, no_link: bool },
    /// A link's text, its sequence above it.
    LinkText,
    /// After a link's text or an image's description: `(` or `[`.
    After { image: bool },
    /// After the `[` (at `at`) of a link label or of `[]`.
    LabelStart { image: bool, at: u32 },
    /// In a link label.
    Label { image: bool },
    /// In an inline link's tail; `dest`: it has a destination.
    Tail { image: bool, dest: bool, st: Tl },
    /// After an image's `!`: `[`.
    Bang,
    /// An image's description, its sequence above it.
    Desc,
    /// A code span (or a LaTeX span).
    Code { latex: bool },
    /// An HTML tag; `hidden` in a code span (where the grammar's `_html_tag` is not a node).
    Tag { st: Tg, hidden: bool },
}

/// An event of a version's tree: a node's start (its kind comes at its end), end, or a leaf.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Open,
    Close,
    Leaf,
}

#[derive(Clone, Copy, Debug)]
struct Ev {
    op: Op,
    k: K,
    s: u32,
    e: u32,
    /// The event before, plus one (0: none). The events of all versions are one tree.
    prev: u32,
}

/// A version.
#[derive(Clone, Debug)]
struct V {
    pos: u32,
    f: Vec<F>,
    sc: Sc,
    /// `x::LTW` and `x::LTP`, as the last token leaves them.
    last: u16,
    /// The versions forked by one token: the next token is lexed for them together (0: none).
    group: u32,
    score: i32,
    /// Its last event, plus one.
    ev: u32,
    /// Whether it won a merge against a different tree of the same score.
    tied: bool,
}

impl V {
    fn same_state(&self, o: &V) -> bool {
        self.pos == o.pos && self.sc == o.sc && self.last == o.last && self.group == o.group && self.f == o.f
    }
}

struct P<'a> {
    src: &'a [u8],
    b: Blocks<'a>,
    evs: Vec<Ev>,
    groups: u32,
    done: Vec<V>,
    /// The images so far that merged with their reading as `!` and a link: each a link more in
    /// tree-sitter's stack, which keeps both readings to the text's end.
    merges: u32,
    /// The last such image's start (its readings merge more than once, and a merge may find an
    /// image counted before).
    merged_at: u32,
}

pub fn parse(src: &[u8]) -> R<Tape> {
    if src.is_empty() {
        return Err(Refusal::Parse(0));
    }
    let b = Blocks::of(src);
    if let Some(at) = b.first_nul() {
        return Err(Refusal::Lex(at as u32));
    }
    if let Err(e) = std::str::from_utf8(src) {
        return Err(Refusal::Lex(e.valid_up_to() as u32));
    }
    let mut p = P { src, b, evs: Vec::new(), groups: 0, done: Vec::new(), merges: 0, merged_at: 0 };
    let best = p.run()?;
    p.write(&best)
}

impl<'a> P<'a> {
    fn run(&mut self) -> R<V> {
        let root = V { pos: 0, f: vec![F::Seq { excl: None, no_link: false, any: false }], sc: Sc::default(), last: x::LTW, group: 0, score: 0, ev: 0, tied: false };
        let mut vs = vec![root];
        let mut now: Vec<V> = Vec::new();
        let mut out: Vec<V> = Vec::new();
        while !vs.is_empty() {
            let pos = vs.iter().map(|v| v.pos).min().expect("a version");
            now.clear();
            let mut i = 0;
            while i < vs.len() {
                if vs[i].pos == pos {
                    now.push(vs.swap_remove(i));
                } else {
                    i += 1;
                }
            }
            // Each group (or lone version): one token, lexed in the union of its members' states.
            now.sort_by_key(|v| v.group);
            // The delimiter tokens lexed here, and whether one leans on a link text's last
            // punctuation (see `leans_on_punctuation`).
            let (mut delims, mut leans) = (0u8, false);
            let mut a = 0;
            while a < now.len() {
                let g = now[a].group;
                let mut z = a + 1;
                while g != 0 && z < now.len() && now[z].group == g {
                    z += 1;
                }
                let (mut lex, mut ext) = (0u16, 0u16);
                for v in &now[a..z] {
                    let (lx, ex) = valid(v);
                    lex |= lx;
                    ext |= ex;
                }
                let tok = self.lex(pos as usize, lex, ext, now[a].sc)?;
                if matches!(self.src.get(pos as usize), Some(b'*' | b'_' | b'~')) {
                    delims |= match tok.map(|t| t.t) {
                        Some(T::X(Ext::Open(_))) => 1,
                        Some(T::X(Ext::Close(_))) => 2,
                        _ => 4,
                    };
                    leans |= self.leans_on_punctuation(&now[a..z], tok.map(|t| t.t))?;
                    if leans && delims.count_ones() > 1 {
                        return Err(Refusal::Parse(pos));
                    }
                }
                if let Some(tok) = tok {
                    if ext & (x::LTW | x::LTP) != 0 {
                        self.relex_risk(&now[a..z], &tok)?;
                    }
                    for v in &now[a..z] {
                        out.clear();
                        self.feed(v, &tok, &mut out);
                        let group = if out.len() > 1 {
                            self.groups += 1;
                            self.groups
                        } else {
                            0
                        };
                        for mut w in out.drain(..) {
                            w.group = group;
                            if let Some(start) = merge(&mut vs, w, &self.evs)
                                && (self.merges == 0 || start > self.merged_at)
                            {
                                self.merged_at = start;
                                self.merges += 1;
                                if self.merges > MAX_LINKS {
                                    // tree-sitter's stack keeps at most `MAX_LINK_COUNT` links a
                                    // node, and drops the readings past them (with nine images,
                                    // the first is read as `!` and a link): refused.
                                    return Err(Refusal::Parse(pos));
                                }
                            }
                        }
                    }
                }
                a = z;
            }
            let mut groups: Vec<u32> = vs.iter().filter(|v| v.group != 0).map(|v| v.group).collect();
            groups.sort_unstable();
            groups.dedup();
            #[cfg(test)]
            let cap = std::env::var("RERAIL_MDI_CAP").ok().and_then(|c| c.parse().ok()).unwrap_or(MAX_VERSIONS);
            #[cfg(not(test))]
            let cap = MAX_VERSIONS;
            if vs.iter().filter(|v| v.group == 0).count() + groups.len() > cap {
                return Err(Refusal::Parse(pos));
            }
        }
        #[cfg(test)]
        if std::env::var_os("RERAIL_MDI_DUMP").is_some() {
            for v in &self.done {
                eprintln!("score {} tied {}\n{}", v.score, v.tied, crate::tape_text(&self.write(v)?));
            }
        }
        let mut best: Option<V> = None;
        for v in std::mem::take(&mut self.done) {
            match &mut best {
                None => best = Some(v),
                Some(b) => {
                    if v.score > b.score {
                        *b = v;
                    } else if v.score == b.score && !same_events(&self.evs, v.ev, b.ev) {
                        b.tied = true;
                    }
                }
            }
        }
        match best {
            Some(b) if !b.tied => Ok(b),
            _ => Err(Refusal::Parse(self.src.len() as u32)),
        }
    }

    /// tree-sitter, on a conflict, lexes again for the reading that reduces first, in the state
    /// the reduction leaves: there the last token's white space or punctuation no longer counts,
    /// and an emphasis delimiter the scanner opened may close instead. Which readings it reduces
    /// apart depends on how its stacks merged, which this parser does not keep: a text where some
    /// version's emphasis would close on such a lexing is refused.
    fn relex_risk(&self, vs: &[V], tok: &Tok) -> R<()> {
        let at = tok.s;
        if !matches!(self.src.get(at), Some(b'*' | b'_' | b'~')) {
            return Ok(());
        }
        // After an image's merge with its reading as `!` and a link, the stack holds both
        // readings, and a reduction across them lexes again: an opener that needs the last
        // token's white space or punctuation (`*(`, `*.`) is then lone text.
        if self.merges > 0 && matches!(tok.t, T::X(Ext::Open(_))) {
            for v in vs {
                let (_, ext) = valid(v);
                let mut sc = v.sc;
                if !matches!(scan(self.src, &self.b, at, ext & !(x::LTW | x::LTP), &mut sc), Ok(Some((Ext::Open(_), _)))) {
                    return Err(Refusal::Parse(at as u32));
                }
            }
        }
        for v in vs {
            let n = v.f.len();
            let (F::Seq { any: true, .. }, Some(F::Emph { d, .. })) = (v.f[n - 1], n.checked_sub(2).map(|i| v.f[i])) else { continue };
            let (_, ext) = valid(v);
            let mut sc = v.sc;
            if let Ok(Some((Ext::Close(c), _))) = scan(self.src, &self.b, at, ext & !(x::LTW | x::LTP), &mut sc)
                && c == d && tok.t != T::X(Ext::Close(d)) {
                    return Err(Refusal::Parse(at as u32));
                }
        }
        Ok(())
    }

    /// Whether a group's token at an emphasis delimiter leans on the last token's punctuation in
    /// a link's text: the grammar takes no `_last_token_punctuation` there (`_inline_element_no_link`),
    /// so a version in it lexes otherwise in its own state. tree-sitter lexes each version, reusing
    /// the last token where the lex modes agree: when another version lexes a different token here
    /// (`_[^_]`: a closer for the emphasis, an opener for the text, punctuation for the link),
    /// which the link gets depends on the versions' order, which this parser does not keep.
    fn leans_on_punctuation(&self, vs: &[V], tok: Option<T>) -> R<bool> {
        for v in vs {
            if v.last & x::LTP == 0 || !matches!(v.f.last(), Some(F::Seq { no_link: true, .. })) {
                continue;
            }
            let (_, ext) = valid(v);
            let mut sc = v.sc;
            let alone = scan(self.src, &self.b, v.pos as usize, ext & !x::LTP, &mut sc).map_err(|_| Refusal::Parse(v.pos))?;
            if alone.map(|(e, _)| T::X(e)) != tok {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // ---- The lexer.

    /// The token at `at`: the scanner's where the state takes one of its tokens and it finds one,
    /// else the grammar's, the longest match among `lex`. `None` where the state takes none here
    /// (the versions reading it end).
    fn lex(&self, at: usize, lex: u16, ext: u16, sc: Sc) -> R<Option<Tok>> {
        let src = self.src;
        let tok = |t: T, e: usize| Ok(Some(Tok { t, s: at, e, sc }));
        if at >= src.len() {
            return tok(T::Eof, at);
        }
        let c = src[at];
        if matches!(c, b'`' | b'$' | b'*' | b'_' | b'~') && ext & !(x::LTW | x::LTP) != 0 {
            let mut s = sc;
            match scan(src, &self.b, at, ext, &mut s) {
                Err(_) => return Err(Refusal::Parse(at as u32)),
                Ok(Some((t, e))) => return Ok(Some(Tok { t: T::X(t), s: at, e, sc: s })),
                Ok(None) => {}
            }
        }
        if lex & l::UNQ != 0 && UNQ[c as usize] {
            return tok(T::Unq, class_end(src, at, &UNQ));
        }
        match c {
            b' ' | b'\t' if lex & l::WS != 0 => {
                // `_whitespace_ge_2` (`\t`, or a space and more blanks), or one space.
                let e = if c == b'\t' { at + 1 } else { self.b.blank_end(at) };
                if (c == b'\t' || e - at >= 2) && lex & l::HARD != 0
                    && let Some(n) = newline_end(src, e) {
                        return tok(T::Hard { bs: false }, n);
                    }
                tok(T::Ws, e)
            }
            b'\n' | b'\r' if lex & l::NL != 0 => tok(T::Nl, newline_end(src, at).expect("a line end")),
            b'\\' if lex & l::ESC != 0 && src.get(at + 1).is_some_and(u8::is_ascii_punctuation) => tok(T::Esc, at + 2),
            b'\\' if lex & l::HARD != 0 && newline_end(src, at + 1).is_some() => tok(T::Hard { bs: true }, newline_end(src, at + 1).expect("a line end")),
            b'&' if lex & l::ENT != 0 => match self.reference(at) {
                Some((t, e)) => tok(t, e),
                None => tok(T::P(c), at + 1),
            },
            b'<' => {
                let auto = if lex & l::AUTO != 0 { self.autolink(at) } else { None };
                let html = if lex & l::HTML != 0 { self.html_start(at) } else { 0 };
                match auto {
                    Some((t, e)) if e - at > html => tok(t, e),
                    _ if html > 0 => Err(Refusal::Parse(at as u32)),
                    _ if lex & l::PUNCT != 0 => tok(T::P(c), at + 1),
                    _ => Ok(None),
                }
            }
            _ if lex & l::ATTR != 0 && (c.is_ascii_alphabetic() || c == b'_' || c == b':') => tok(T::AttrName, class_end(src, at + 1, &ATTR_REST)),
            _ if c.is_ascii_punctuation() => {
                if lex & l::PUNCT != 0 {
                    tok(T::P(c), at + 1)
                } else {
                    Ok(None)
                }
            }
            _ if c.is_ascii_digit() => {
                if lex & l::DIGITS != 0 {
                    tok(T::Digits, self.b.digits_end(at))
                } else {
                    Ok(None)
                }
            }
            _ if word_char(c) && lex & l::WORD != 0 => {
                // `_word_no_digit`: word characters, runs of `_` between them.
                let mut e = self.b.word_stop(at);
                while src.get(e) == Some(&b'_') {
                    let u = run_end(src, e, b'_');
                    if u < src.len() && word_char(src[u]) {
                        e = self.b.word_stop(u);
                    } else {
                        break;
                    }
                }
                tok(T::Word, e)
            }
            _ => Ok(None),
        }
    }

    /// An entity or numeric character reference at `at` (`&`), and its end.
    fn reference(&self, at: usize) -> Option<(T, usize)> {
        let src = self.src;
        if src.get(at + 1) == Some(&b'#') {
            let (class, from, max) = if matches!(src.get(at + 2), Some(b'x' | b'X')) { (&HEX, at + 3, 6) } else { (&DIGIT, at + 2, 7) };
            let e = class_end(src, from, class);
            return (e > from && e - from <= max && src.get(e) == Some(&b';')).then_some((T::Num, e + 1));
        }
        let e = class_end(src, at + 1, &ALNUM);
        if e > at + 1 && src.get(e) == Some(&b';') {
            let name = std::str::from_utf8(&src[at + 1..e]).expect("ASCII");
            if kinds::ENTITIES.binary_search(&name).is_ok() {
                return Some((T::Ent, e + 1));
            }
        }
        None
    }

    /// A URI or email autolink at `at` (`<`), and its end.
    fn autolink(&self, at: usize) -> Option<(T, usize)> {
        let src = self.src;
        // `<scheme:…>`: a scheme of two or more characters, the first a letter.
        if src.get(at + 1).is_some_and(u8::is_ascii_alphabetic) {
            let e = class_end(src, at + 1, &SCHEME);
            if e - (at + 1) >= 2 && src.get(e) == Some(&b':') {
                let r = class_end(src, e + 1, &URI);
                if src.get(r) == Some(&b'>') {
                    return Some((T::Uri, r + 1));
                }
            }
        }
        // `<local@label.label>`.
        let e = class_end(src, at + 1, &EMAIL_LOCAL);
        if e == at + 1 || src.get(e) != Some(&b'@') {
            return None;
        }
        let mut p = e + 1;
        loop {
            let r = class_end(src, p, &ALNUM_DASH);
            if r == p || r - p > 63 || src[p] == b'-' || src[r - 1] == b'-' {
                return None;
            }
            match src.get(r) {
                Some(b'.') => p = r + 1,
                Some(b'>') => return Some((T::Email, r + 1)),
                _ => return None,
            }
        }
    }

    /// The length of an HTML construct's start at `at` (`<`): `<!--`, `<?`, `<![CDATA[`, `<!` and
    /// capitals; 0 for none.
    fn html_start(&self, at: usize) -> usize {
        let rest = &self.src[at..];
        if rest.starts_with(b"<![CDATA[") {
            9
        } else if rest.starts_with(b"<!--") {
            4
        } else if rest.starts_with(b"<?") {
            2
        } else if rest.len() > 2 && rest[1] == b'!' && rest[2].is_ascii_uppercase() {
            3
        } else {
            0
        }
    }

    // ---- The grammar.

    /// Event `op` on `v`.
    fn ev(&mut self, v: &mut V, op: Op, k: K, s: usize, e: usize) {
        self.evs.push(Ev { op, k, s: s as u32, e: e as u32, prev: v.ev });
        v.ev = self.evs.len() as u32;
    }

    fn leaf(&mut self, v: &mut V, k: K, s: usize, e: usize) {
        self.ev(v, Op::Leaf, k, s, e);
    }

    fn open(&mut self, v: &mut V, s: usize) {
        self.ev(v, Op::Open, K::Inline, s, s);
    }

    fn close(&mut self, v: &mut V, k: K, e: usize) {
        self.ev(v, Op::Close, k, e, e);
    }

    /// `v` past token `t`.
    fn next(v: &V, t: &Tok) -> V {
        let mut w = v.clone();
        w.pos = t.e as u32;
        w.sc = t.sc;
        w.last = 0;
        w.group = 0;
        w
    }

    /// An element starts at `s` in the sequence on top of `w`: a link's text or an image's
    /// description starts with its first.
    fn elem(&mut self, w: &mut V, s: usize) {
        let n = w.f.len();
        if let F::Seq { any, .. } = &mut w.f[n - 1]
            && !*any {
                *any = true;
                if n >= 2 && matches!(w.f[n - 2], F::LinkText | F::Desc) {
                    self.open(w, s);
                }
            }
    }

    fn top(w: &mut V) -> &mut F {
        w.f.last_mut().expect("a frame")
    }

    /// Each version `v` becomes past token `t`, into `out` (none where it cannot take `t`).
    fn feed(&mut self, v: &V, t: &Tok, out: &mut Vec<V>) {
        match *v.f.last().expect("a frame") {
            F::Seq { excl, no_link, any } => self.seq(v, t, excl, no_link, any, out),
            F::Strong { d, closing, no_link } => {
                let mut w = Self::next(v, t);
                match (closing, t.t) {
                    (false, T::X(Ext::Open(e))) if e == d => {
                        self.leaf(&mut w, K::EmphasisDelimiter, t.s, t.e);
                        w.f.push(F::Emph { d, inner: true, no_link });
                        w.f.push(F::Seq { excl: Some(d), no_link, any: false });
                        w.last = x::LTP;
                        out.push(w);
                    }
                    (true, T::X(Ext::Close(e))) if e == d => {
                        self.leaf(&mut w, K::EmphasisDelimiter, t.s, t.e);
                        w.f.pop();
                        self.close(&mut w, K::StrongEmphasis, t.e);
                        w.score += 2;
                        out.push(w);
                    }
                    _ => {}
                }
            }
            F::After { image } => {
                let mut w = Self::next(v, t);
                match t.t {
                    T::P(b'(') => {
                        self.leaf(&mut w, K::TLParen, t.s, t.e);
                        *Self::top(&mut w) = F::Tail { image, dest: false, st: Tl::Lead };
                        out.push(w);
                    }
                    T::P(b'[') => {
                        *Self::top(&mut w) = F::LabelStart { image, at: t.s as u32 };
                        out.push(w);
                    }
                    _ => {}
                }
            }
            F::LabelStart { image, at } => {
                let mut w = Self::next(v, t);
                let at = at as usize;
                if t.t == T::P(b']') {
                    self.leaf(&mut w, K::TLBracket, at, at + 1);
                    self.leaf(&mut w, K::TRBracket, t.s, t.e);
                    w.f.pop();
                    self.close(&mut w, if image { K::Image } else { K::CollapsedReferenceLink }, t.e);
                    w.score += if image { 40 } else { 20 };
                    out.push(w);
                } else {
                    self.open(&mut w, at);
                    self.leaf(&mut w, K::TLBracket, at, at + 1);
                    *Self::top(&mut w) = F::Label { image };
                    if self.label(&mut w, t) {
                        out.push(w);
                    }
                }
            }
            F::Label { image } => {
                let mut w = Self::next(v, t);
                if t.t == T::P(b']') {
                    self.leaf(&mut w, K::TRBracket, t.s, t.e);
                    self.close(&mut w, K::LinkLabel, t.e);
                    w.f.pop();
                    self.close(&mut w, if image { K::Image } else { K::FullReferenceLink }, t.e);
                    w.score += if image { 40 } else { 30 };
                    out.push(w);
                } else if self.label(&mut w, t) {
                    out.push(w);
                }
            }
            F::Tail { image, dest, st } => self.tail(v, t, image, dest, st, out),
            F::Bang => {
                if t.t == T::P(b'[') {
                    let mut w = Self::next(v, t);
                    self.leaf(&mut w, K::TLBracket, t.s, t.e);
                    *Self::top(&mut w) = F::Desc;
                    w.f.push(F::Seq { excl: None, no_link: false, any: false });
                    out.push(w);
                }
            }
            F::Code { latex } => {
                let mut w = Self::next(v, t);
                match t.t {
                    T::X(Ext::CodeClose) if !latex => {
                        self.leaf(&mut w, K::CodeSpanDelimiter, t.s, t.e);
                        w.f.pop();
                        self.close(&mut w, K::CodeSpan, t.e);
                    }
                    T::X(Ext::LatexClose) if latex => {
                        self.leaf(&mut w, K::LatexSpanDelimiter, t.s, t.e);
                        w.f.pop();
                        self.close(&mut w, K::LatexBlock, t.e);
                    }
                    T::Word | T::Digits | T::Ws | T::Nl => {}
                    T::Esc if latex => self.leaf(&mut w, K::BackslashEscape, t.s, t.e),
                    T::P(b'<') => {
                        // Text, or a tag (hidden here).
                        self.leaf(&mut w, K::TLt, t.s, t.e);
                        let mut tag = w.clone();
                        tag.f.push(F::Tag { st: Tg::Open0, hidden: true });
                        out.push(tag);
                    }
                    T::P(c) => self.leaf(&mut w, punct(c), t.s, t.e),
                    _ => return,
                }
                out.push(w);
            }
            F::Tag { st, hidden } => self.tag(v, t, st, hidden, out),
            F::Emph { .. } | F::LinkText | F::Desc => unreachable!("a sequence is above it"),
        }
    }

    /// A token in a sequence of inline elements.
    #[allow(clippy::too_many_arguments)]
    fn seq(&mut self, v: &V, t: &Tok, excl: Option<D>, no_link: bool, any: bool, out: &mut Vec<V>) {
        let n = v.f.len();
        let parent = if n >= 2 { Some(v.f[n - 2]) } else { None };
        let (s, e) = (t.s, t.e);
        match t.t {
            T::Eof => {
                if n == 1 && any {
                    self.done.push(v.clone());
                }
            }
            T::Word | T::Digits | T::Ws | T::Nl | T::X(Ext::Unclosed) => {
                let mut w = Self::next(v, t);
                self.elem(&mut w, s);
                if matches!(t.t, T::Ws | T::Nl) {
                    w.last = x::LTW;
                }
                out.push(w);
            }
            T::Hard { bs } => {
                let mut w = Self::next(v, t);
                self.elem(&mut w, s);
                self.open(&mut w, s);
                if bs {
                    self.leaf(&mut w, K::TBackslash, s, s + 1);
                }
                self.close(&mut w, K::HardLineBreak, e);
                w.last = x::LTW;
                out.push(w);
            }
            T::Esc | T::Ent | T::Num | T::Uri | T::Email => {
                let mut w = Self::next(v, t);
                self.elem(&mut w, s);
                let k = match t.t {
                    T::Esc => K::BackslashEscape,
                    T::Ent => K::EntityReference,
                    T::Num => K::NumericCharacterReference,
                    T::Uri => K::UriAutolink,
                    _ => K::EmailAutolink,
                };
                self.leaf(&mut w, k, s, e);
                out.push(w);
            }
            T::X(Ext::Open(d)) => {
                if excl != Some(d) {
                    // A lone opener (hidden).
                    let mut w = Self::next(v, t);
                    self.elem(&mut w, s);
                    out.push(w);
                }
                let mut w = Self::next(v, t);
                self.elem(&mut w, s);
                self.open(&mut w, s);
                self.leaf(&mut w, K::EmphasisDelimiter, s, e);
                if d != D::Tilde {
                    let mut strong = w.clone();
                    strong.f.push(F::Strong { d, closing: false, no_link });
                    out.push(strong);
                }
                w.f.push(F::Emph { d, inner: false, no_link });
                w.f.push(F::Seq { excl: Some(d), no_link, any: false });
                w.last = x::LTP;
                out.push(w);
            }
            T::X(Ext::Close(d)) => {
                if any && matches!(parent, Some(F::Emph { d: pd, .. }) if pd == d) {
                    let mut w = Self::next(v, t);
                    w.f.pop();
                    self.leaf(&mut w, K::EmphasisDelimiter, s, e);
                    let Some(F::Emph { inner, .. }) = w.f.pop() else { unreachable!("emphasis") };
                    // Emphasis scores 1 (strong emphasis's own inner emphasis too).
                    w.score += 1;
                    if inner {
                        let F::Strong { closing, .. } = Self::top(&mut w) else { unreachable!("strong emphasis") };
                        *closing = true;
                    } else {
                        self.close(&mut w, if d == D::Tilde { K::Strikethrough } else { K::Emphasis }, e);
                    }
                    out.push(w);
                }
            }
            T::X(Ext::CodeStart) | T::X(Ext::LatexStart) => {
                let latex = t.t == T::X(Ext::LatexStart);
                let mut w = Self::next(v, t);
                self.elem(&mut w, s);
                self.open(&mut w, s);
                self.leaf(&mut w, if latex { K::LatexSpanDelimiter } else { K::CodeSpanDelimiter }, s, e);
                w.f.push(F::Code { latex });
                out.push(w);
            }
            T::P(b'[') => {
                if no_link {
                    return;
                }
                let mut w = Self::next(v, t);
                self.elem(&mut w, s);
                let mut link = w.clone();
                self.leaf(&mut w, K::TLBracket, s, e);
                w.last = x::LTP;
                out.push(w);
                self.open(&mut link, s);
                self.leaf(&mut link, K::TLBracket, s, e);
                link.f.push(F::LinkText);
                link.f.push(F::Seq { excl: None, no_link: true, any: false });
                out.push(link);
            }
            T::P(b']') if matches!(parent, Some(F::LinkText | F::Desc)) => {
                // The end of a link's text or an image's description (the `]` closes it, the
                // grammar's precedence); a shortcut link or image ends here, or its tail follows.
                let image = parent == Some(F::Desc);
                let mut w = Self::next(v, t);
                if any {
                    self.close(&mut w, if image { K::ImageDescription } else { K::LinkText }, s);
                }
                w.f.pop();
                self.leaf(&mut w, K::TRBracket, s, e);
                if any {
                    let mut short = w.clone();
                    short.f.pop();
                    self.close(&mut short, if image { K::Image } else { K::ShortcutLink }, e);
                    short.score += if image { 30 } else { 10 };
                    out.push(short);
                }
                *Self::top(&mut w) = F::After { image };
                out.push(w);
            }
            T::P(b']') => {
                if !no_link {
                    let mut w = Self::next(v, t);
                    self.elem(&mut w, s);
                    self.leaf(&mut w, K::TRBracket, s, e);
                    w.last = x::LTP;
                    out.push(w);
                }
            }
            T::P(c @ (b'!' | b'<')) => {
                // Text, or an image's or a tag's start.
                let mut w = Self::next(v, t);
                self.elem(&mut w, s);
                let mut start = w.clone();
                self.leaf(&mut w, punct(c), s, e);
                w.last = x::LTP;
                out.push(w);
                self.open(&mut start, s);
                self.leaf(&mut start, punct(c), s, e);
                start.f.push(if c == b'!' { F::Bang } else { F::Tag { st: Tg::Open0, hidden: false } });
                out.push(start);
            }
            T::P(c) => {
                let mut w = Self::next(v, t);
                self.elem(&mut w, s);
                self.leaf(&mut w, punct(c), s, e);
                w.last = x::LTP;
                out.push(w);
            }
            _ => {}
        }
    }

    /// A token in a link label (`w` past it); whether the label takes it.
    fn label(&mut self, w: &mut V, t: &Tok) -> bool {
        match t.t {
            T::Word | T::Digits | T::X(Ext::Open(D::Star | D::Us)) | T::X(Ext::Unclosed) => {}
            T::Ws | T::Nl => w.last = x::LTW,
            T::P(b'[' | b']') => return false,
            T::P(c) => {
                self.leaf(w, punct(c), t.s, t.e);
                w.last = x::LTP;
            }
            T::Esc => self.leaf(w, K::BackslashEscape, t.s, t.e),
            T::Ent => self.leaf(w, K::EntityReference, t.s, t.e),
            T::Num => self.leaf(w, K::NumericCharacterReference, t.s, t.e),
            _ => return false,
        }
        true
    }

    /// A token in an inline link's or image's tail.
    #[allow(clippy::too_many_arguments)]
    fn tail(&mut self, v: &V, t: &Tok, image: bool, dest: bool, st: Tl, out: &mut Vec<V>) {
        let (s, e) = (t.s, t.e);
        let mut w = Self::next(v, t);
        let set = |w: &mut V, dest: bool, st: Tl| *Self::top(w) = F::Tail { image, dest, st };
        match st {
            Tl::Lead => match t.t {
                T::Ws | T::Nl => out.push(w),
                T::P(b')') => self.tail_end(w, t, image, out),
                T::P(b'<') => {
                    self.open(&mut w, s);
                    self.leaf(&mut w, K::TLt, s, e);
                    set(&mut w, dest, Tl::Angle);
                    out.push(w);
                }
                T::P(q @ (b'"' | b'\'' | b'(')) => {
                    let mut title = w.clone();
                    self.open(&mut title, s);
                    self.leaf(&mut title, punct(q), s, e);
                    set(&mut title, dest, Tl::Title { q: if q == b'(' { b')' } else { q }, nl: false });
                    out.push(title);
                    self.open(&mut w, s);
                    self.leaf(&mut w, punct(q), s, e);
                    set(&mut w, dest, Tl::Dest { depth: u8::from(q == b'(') });
                    out.push(w);
                }
                // A destination's first token (not `<`, `(` or `)`).
                T::Word | T::Digits | T::Esc | T::Ent | T::Num | T::P(_) => {
                    self.open(&mut w, s);
                    if let Some(k) = leaf_kind(t.t) {
                        self.leaf(&mut w, k, s, e);
                    }
                    set(&mut w, dest, Tl::Dest { depth: 0 });
                    out.push(w);
                }
                _ => {}
            },
            Tl::Angle => match t.t {
                T::P(b'>') => {
                    self.leaf(&mut w, K::TGt, s, e);
                    self.close(&mut w, K::LinkDestination, e);
                    w.score += 10;
                    set(&mut w, true, Tl::AfterDest { ws: false });
                    out.push(w);
                }
                T::P(b'<') | T::Nl => {}
                T::Word | T::Digits | T::Ws => out.push(w),
                T::Esc | T::Ent | T::Num | T::P(_) => {
                    let k = leaf_kind(t.t).expect("a leaf");
                    self.leaf(&mut w, k, s, e);
                    out.push(w);
                }
                _ => {}
            },
            Tl::Dest { depth } => match t.t {
                T::Word | T::Digits => out.push(w),
                T::P(b'(') => {
                    if depth == u8::MAX {
                        return;
                    }
                    self.leaf(&mut w, K::TLParen, s, e);
                    set(&mut w, dest, Tl::Dest { depth: depth + 1 });
                    out.push(w);
                }
                T::P(b')') if depth > 0 => {
                    self.leaf(&mut w, K::TRParen, s, e);
                    set(&mut w, dest, Tl::Dest { depth: depth - 1 });
                    out.push(w);
                }
                T::P(b')') => {
                    self.close(&mut w, K::LinkDestination, s);
                    w.score += 10;
                    self.tail_end(w, t, image, out);
                }
                T::Ws | T::Nl => {
                    if depth == 0 {
                        self.close(&mut w, K::LinkDestination, s);
                        w.score += 10;
                        set(&mut w, true, Tl::AfterDest { ws: true });
                        out.push(w);
                    }
                }
                T::Esc | T::Ent | T::Num | T::P(_) => {
                    let k = leaf_kind(t.t).expect("a leaf");
                    self.leaf(&mut w, k, s, e);
                    out.push(w);
                }
                _ => {}
            },
            Tl::AfterDest { ws } => match t.t {
                T::Ws | T::Nl => {
                    set(&mut w, dest, Tl::AfterDest { ws: true });
                    out.push(w);
                }
                T::P(b')') => self.tail_end(w, t, image, out),
                T::P(q @ (b'"' | b'\'' | b'(')) if ws => {
                    self.open(&mut w, s);
                    self.leaf(&mut w, punct(q), s, e);
                    set(&mut w, dest, Tl::Title { q: if q == b'(' { b')' } else { q }, nl: false });
                    out.push(w);
                }
                _ => {}
            },
            Tl::Title { q, nl } => match t.t {
                T::P(c) if c == q => {
                    self.leaf(&mut w, punct(c), s, e);
                    self.close(&mut w, K::LinkTitle, e);
                    set(&mut w, dest, Tl::AfterTitle);
                    out.push(w);
                }
                T::P(b'(') if q == b')' => {}
                T::Nl => {
                    if !nl {
                        set(&mut w, dest, Tl::Title { q, nl: true });
                        out.push(w);
                    }
                }
                T::Word | T::Digits | T::Ws => {
                    set(&mut w, dest, Tl::Title { q, nl: false });
                    out.push(w);
                }
                T::Esc | T::Ent | T::Num | T::P(_) => {
                    let k = leaf_kind(t.t).expect("a leaf");
                    self.leaf(&mut w, k, s, e);
                    set(&mut w, dest, Tl::Title { q, nl: false });
                    out.push(w);
                }
                _ => {}
            },
            Tl::AfterTitle => match t.t {
                T::Ws | T::Nl => out.push(w),
                T::P(b')') => self.tail_end(w, t, image, out),
                _ => {}
            },
        }
    }

    /// An inline link's or image's `)`: the link ends.
    fn tail_end(&mut self, mut w: V, t: &Tok, image: bool, out: &mut Vec<V>) {
        self.leaf(&mut w, K::TRParen, t.s, t.e);
        w.f.pop();
        self.close(&mut w, if image { K::Image } else { K::InlineLink }, t.e);
        w.score += if image { 40 } else { 20 };
        out.push(w);
    }

    /// A token in an HTML tag.
    fn tag(&mut self, v: &V, t: &Tok, st: Tg, hidden: bool, out: &mut Vec<V>) {
        let (s, e) = (t.s, t.e);
        let mut w = Self::next(v, t);
        let set = |w: &mut V, st: Tg| *Self::top(w) = F::Tag { st, hidden };
        let next = match (st, t.t) {
            (Tg::Open0, T::Word) => Tg::Name { closing: false },
            (Tg::Open0, T::P(b'/')) => {
                self.leaf(&mut w, K::TSlash, s, e);
                Tg::Close0
            }
            (Tg::Close0, T::Word) => Tg::Name { closing: true },
            (Tg::Name { .. }, T::Word | T::Digits) => st,
            (Tg::Name { .. }, T::P(b'-')) => {
                self.leaf(&mut w, K::TMinus, s, e);
                st
            }
            (Tg::Name { closing } | Tg::Ws { closing }, T::Ws | T::Nl) => Tg::Ws { closing },
            (Tg::AfterVal, T::Ws | T::Nl) => Tg::Ws { closing: false },
            (Tg::Name { closing: false } | Tg::Ws { closing: false } | Tg::AfterVal, T::P(b'/')) => {
                self.leaf(&mut w, K::TSlash, s, e);
                Tg::Slash
            }
            (Tg::Ws { closing: false }, T::AttrName) => Tg::Attr,
            (Tg::Attr | Tg::Eq, T::Ws | T::Nl) => st,
            (Tg::Attr, T::P(b'=')) => {
                self.leaf(&mut w, K::TEq, s, e);
                Tg::Eq
            }
            (Tg::Eq, T::Unq) => Tg::AfterVal,
            (Tg::Eq, T::P(q @ (b'"' | b'\''))) => {
                self.leaf(&mut w, punct(q), s, e);
                Tg::Val { q }
            }
            (Tg::Val { q }, T::P(c)) if c == q => {
                self.leaf(&mut w, punct(c), s, e);
                Tg::AfterVal
            }
            (Tg::Val { .. }, T::Word | T::Digits | T::Ws | T::Nl) => st,
            (Tg::Val { .. }, T::P(c)) => {
                self.leaf(&mut w, punct(c), s, e);
                st
            }
            (Tg::Name { .. } | Tg::Ws { .. } | Tg::AfterVal | Tg::Slash, T::P(b'>')) => {
                self.leaf(&mut w, K::TGt, s, e);
                w.f.pop();
                if !hidden {
                    self.close(&mut w, K::HtmlTag, e);
                }
                w.score += 100;
                out.push(w);
                return;
            }
            _ => return,
        };
        set(&mut w, next);
        out.push(w);
    }

    // ---- The tape.

    /// The tape of version `v`'s tree: its events, each node's tokens as the engine's automata
    /// store them.
    fn write(&self, v: &V) -> R<Tape> {
        let mut evs = Vec::new();
        let mut at = v.ev;
        while at != 0 {
            let ev = self.evs[at as usize - 1];
            evs.push(ev);
            at = ev.prev;
        }
        evs.reverse();
        // The nodes in preorder: kind, span, and the number of nodes in each one's subtree.
        let len = self.src.len() as u32;
        let mut nodes: Vec<(K, u32, u32, u32)> = vec![(K::Inline, 0, len, 0)];
        let mut open: Vec<usize> = vec![0];
        for ev in &evs {
            match ev.op {
                Op::Open => {
                    open.push(nodes.len());
                    nodes.push((K::Inline, ev.s, ev.s, 0));
                }
                Op::Close => {
                    let i = open.pop().expect("an open node");
                    nodes[i].0 = ev.k;
                    nodes[i].2 = ev.e;
                    nodes[i].3 = (nodes.len() - i) as u32;
                }
                Op::Leaf => nodes.push((ev.k, ev.s, ev.e, 1)),
            }
        }
        assert_eq!(open.len(), 1, "a node left open");
        nodes[0].3 = nodes.len() as u32;
        // The tokens each node's automaton leaves out.
        let mut dropped = vec![false; nodes.len()];
        let mut kids: Vec<usize> = Vec::new();
        let mut labels: Vec<(u32, bool)> = Vec::new();
        let mut out: Vec<bool> = Vec::new();
        for i in 0..nodes.len() {
            let a = kinds::AUTOMATON[nodes[i].0 as usize];
            if a == 0 || nodes[i].3 == 1 {
                continue;
            }
            kids.clear();
            let (mut c, end) = (i + 1, i + nodes[i].3 as usize);
            while c < end {
                kids.push(c);
                c += nodes[c].3 as usize;
            }
            labels.clear();
            labels.extend(kids.iter().map(|&c| (nodes[c].0 as u32, kinds::KINDS[nodes[c].0 as usize].1)));
            out.clear();
            out.resize(kids.len(), false);
            walk(a as usize - 1, &labels, &mut out);
            for (j, &c) in kids.iter().enumerate() {
                dropped[c] = out[j];
            }
        }
        let mut widths = Widths::guess(self.src.len());
        loop {
            let mut w = Writer::new(0, widths, nodes.len() + 1);
            let mut stack: Vec<(usize, u32, usize)> = Vec::new();
            for (i, &(k, s, e, size)) in nodes.iter().enumerate() {
                while stack.last().is_some_and(|&(_, _, end)| end <= i) {
                    let (n, e, _) = stack.pop().expect("an open node");
                    w.close(n, e);
                }
                if dropped[i] {
                    continue;
                }
                let n = w.push(key(k), s);
                stack.push((n, e, i + size as usize));
            }
            while let Some((n, e, _)) = stack.pop() {
                w.close(n, e);
            }
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
}

/// The lexer's tokens and the scanner's symbols version `v`'s state takes.
fn valid(v: &V) -> (u16, u16) {
    let n = v.f.len();
    let (lex, ext) = match v.f[n - 1] {
        F::Seq { any, .. } => {
            let mut ext = x::OPEN_STAR | x::OPEN_US | x::OPEN_TILDE | x::CODE_START | x::LATEX_START | x::UNCLOSED;
            if let (true, Some(F::Emph { d, .. })) = (any, n.checked_sub(2).map(|i| v.f[i])) {
                ext |= d.bits().1;
            }
            (l::TEXT, ext)
        }
        F::Strong { d, closing, .. } => (l::TEXT, if closing { d.bits().1 } else { d.bits().0 }),
        F::After { .. } | F::Bang => (l::PUNCT, 0),
        F::LabelStart { .. } | F::Label { .. } => (l::LABEL, x::OPEN_STAR | x::OPEN_US),
        F::Tail { .. } => (l::LINK, 0),
        F::Code { latex } => (if latex { l::CODE | l::ESC } else { l::CODE }, if latex { x::LATEX_CLOSE } else { x::CODE_CLOSE }),
        F::Tag { st, .. } => (
            match st {
                Tg::Open0 | Tg::Close0 => l::WORD | l::PUNCT,
                Tg::Name { .. } | Tg::Val { .. } => l::WORD | l::DIGITS | l::PUNCT | l::WS | l::NL,
                Tg::Ws { closing: false } => l::ATTR | l::PUNCT | l::WS | l::NL,
                Tg::Eq => l::UNQ | l::PUNCT | l::WS | l::NL,
                Tg::Ws { closing: true } | Tg::Attr | Tg::AfterVal | Tg::Slash => l::PUNCT | l::WS | l::NL,
            },
            0,
        ),
        F::Emph { .. } | F::LinkText | F::Desc => unreachable!("a sequence is above it"),
    };
    (lex, ext | v.last)
}

/// Version `w` into `vs`, merged with one in the same state: the higher score's tree kept; of two
/// different trees of the same score, the version is marked tied. Where an image it merged with
/// another reading of it starts, if it did.
fn merge(vs: &mut Vec<V>, w: V, evs: &[Ev]) -> Option<u32> {
    for v in vs.iter_mut() {
        if v.same_state(&w) {
            let different = !same_events(evs, v.ev, w.ev);
            let image_start = if different { image_apart(evs, v.ev, w.ev) } else { None };
            if w.score > v.score {
                *v = w;
            } else if w.score == v.score && (w.tied || different) {
                v.tied = true;
            }
            return image_start;
        }
    }
    vs.push(w);
    None
}

/// The start of an image closed on one of the event lists ending at `a` and `b` since they
/// parted (the events of all versions are one tree, an event's index past its `prev`'s), looked
/// for over the last few events. Its readings (`![a]` and `![a](b)`) end apart but start together.
fn image_apart(evs: &[Ev], mut a: u32, mut b: u32) -> Option<u32> {
    let mut found = None;
    for _ in 0..32 {
        if a == b {
            return found;
        }
        let newer = if a > b { &mut a } else { &mut b };
        let x = evs[*newer as usize - 1];
        if found.is_none() && matches!(x, Ev { op: Op::Close, k: K::Image, .. }) {
            found = open_of(evs, x.prev);
        }
        *newer = x.prev;
    }
    None
}

/// The start of the node whose close is the event after `at` (its open, past the nodes in it).
fn open_of(evs: &[Ev], mut at: u32) -> Option<u32> {
    let mut depth = 0u32;
    for _ in 0..256 {
        if at == 0 {
            return None;
        }
        let x = evs[at as usize - 1];
        match x.op {
            Op::Close => depth += 1,
            Op::Open if depth == 0 => return Some(x.s),
            Op::Open => depth -= 1,
            Op::Leaf => {}
        }
        at = x.prev;
    }
    None
}

/// Whether the event lists ending at `a` and `b` are the same.
fn same_events(evs: &[Ev], mut a: u32, mut b: u32) -> bool {
    while a != b {
        if a == 0 || b == 0 {
            return false;
        }
        let (x, y) = (evs[a as usize - 1], evs[b as usize - 1]);
        if x.op != y.op || x.k != y.k || x.s != y.s || x.e != y.e {
            return false;
        }
        a = x.prev;
        b = y.prev;
    }
    true
}

// ---- The engine's token automata (`kinds::MOVES`): which tokens a node's tape leaves out.

#[inline]
fn moves(s: u32) -> &'static [(u32, u32)] {
    &kinds::MOVES[kinds::MOVES_AT[s as usize] as usize..kinds::MOVES_AT[s as usize + 1] as usize]
}

#[inline]
fn step(s: u32, l: u32) -> Option<u32> {
    moves(s).iter().find(|&&(x, _)| x == l).map(|&(_, to)| to)
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

/// The tokens a node of automaton `a` leaves out, as the engine's `stored_children` decides them:
/// `kids` its children's labels in order (and whether each is named), `out[i]` set for each child
/// left out; none where the automaton does not take the children.
fn walk(a: usize, kids: &[(u32, bool)], out: &mut [bool]) {
    let close = |state: u32, from: usize, to: usize, next: Option<u32>, out: &mut [bool]| -> Option<u32> {
        let mut s = state;
        for k in &kids[from..to] {
            s = step(s, k.0)?;
        }
        let after = match next {
            Some(l) => step(s, l)?,
            None if accepts(s) => s,
            None => return None,
        };
        if to > from {
            let w = implied(state, next);
            if w != 0 && (w >> 8 & 0xf) as usize == to - from {
                let word = &kinds::WORD[(w >> 12) as usize..(w >> 12) as usize + (to - from)];
                if word.iter().zip(&kids[from..to]).all(|(&l, k)| l == k.0) {
                    for (j, o) in out[from..to].iter_mut().enumerate() {
                        *o |= w >> j & 1 == 1;
                    }
                }
            }
        }
        Some(after)
    };
    let mut go = || -> Option<()> {
        let mut state = kinds::START[a];
        let mut from = 0;
        for (i, &(l, named)) in kids.iter().enumerate() {
            if named {
                state = close(state, from, i, Some(l), out)?;
                from = i + 1;
            }
        }
        close(state, from, kids.len(), None, out)?;
        Some(())
    };
    if go().is_none() {
        out.fill(false);
    }
}

#[cfg(test)]
mod tests {
    /// With `RERAIL_MDI_TEXT`, the versions that finish on it: their scores and tapes.
    #[test]
    #[ignore = "a probe: set RERAIL_MDI_TEXT and RERAIL_MDI_DUMP"]
    fn dump() {
        let t = std::env::var("RERAIL_MDI_TEXT").expect("set RERAIL_MDI_TEXT").replace("\\n", "\n");
        eprintln!("{:?}", super::parse(t.as_bytes()).map(|t| crate::tape_text(&t)));
    }
}
