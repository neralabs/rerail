//! The parser: recursive descent over tree-sitter-scala's grammar, writing the tape as it goes
//! (`Tw`, the writer the lean Kotlin, Swift and Scala parsers share).
//!
//! **Tokens** are lexed into a buffer as the parser looks ahead, 16 at a time. Where the parse
//! state reads a token otherwise (a string's pieces, an interpolation's block), the buffer is cut
//! at that byte and lexed again from there (`relex_at`).
//!
//! **Layout.** Where the parse state takes an external token (a semicolon, an indent, an outdent,
//! the gate before `else`, …), the parser asks the scanner (`P::layout`) with the tokens its state
//! takes: those the construct read takes next (`own`), and those the constructs around it take
//! after it (`P::fol`, the follow set, as an LR state's lookaheads). The scanner's state is kept
//! where its token is read (`P::take`). A layout token has no width; the node it ends ends at it.

use crate::kinds::{K, f};
use crate::lex::{Lexer, R, Refusal, T, Tok, op, w};
use crate::scan::{Ext, Scanner, V, v};
use crate::tw::{Mark, Node, Tw};
use rerail_tape_format::{Redo, Widths};

mod decl;
mod expr;
mod pat;
mod types;

const MAX_DEPTH: u32 = 250;

/// The class bit of `{expression}` (`Tw::set_class`).
const EXPR_CLASS: u32 = 1;

pub(crate) struct P<'a> {
    src: &'a [u8],
    lx: &'a Lexer<'a>,
    toks: Vec<Tok>,
    i: usize,
    /// Where the next token is lexed into the buffer.
    next: usize,
    t: Tw,
    depth: u32,
    /// The furthest refusal (a look ahead's included): where a refused file is reported.
    far: u32,
    /// The scanner, as of the last external token read.
    sc: Scanner,
    /// Where the scanner reads from: past the last token, external token or comment read.
    sp: u32,
    /// The external tokens the constructs around the one read take after it.
    fol: V,
    /// A layout token asked and not read: (where, valid, the answer, the scanner after it).
    memo: Option<(u32, V, Option<(Ext, u32, u32)>, Scanner)>,
    /// The kind a definition read is given when it closes (a `def` without a body).
    retype: Option<K>,
    /// Reading an infix expression's right operand: type arguments after it are the infix's.
    right_operand: bool,
    /// The statements the last `statements` read.
    count: u32,
}

/// `src` parsed.
pub fn parse(src: &[u8]) -> R<crate::Tape> {
    let lx = Lexer::new(src);
    if let Some(at) = lx.blocks.first_nul() {
        return Err(Refusal::Lex(at as u32));
    }
    if std::str::from_utf8(src).is_err() {
        return Err(Refusal::Lex(0));
    }
    if src.starts_with(b"#!") || src.starts_with("\u{feff}".as_bytes()) {
        return Err(Refusal::Lex(0));
    }
    let mut widths = Widths::guess(src.len());
    loop {
        let mut p = P {
            src,
            lx: &lx,
            toks: Vec::with_capacity(src.len() / 4 + 4),
            i: 0,
            next: 0,
            t: Tw::new(widths, src.len() / 3 + 16),
            depth: 0,
            far: 0,
            sc: Scanner::default(),
            sp: 0,
            fol: 0,
            memo: None,
            retype: None,
            right_operand: false,
            count: 0,
        };
        if let Err(r) = p.compilation_unit() {
            return Err(match r {
                Refusal::Parse(at) => Refusal::Parse(at.max(p.far)),
                r => r,
            });
        }
        match p.t.w.finish_columns() {
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

/// A punctuation token's kind (`T`) as the tape's (`K`).
fn punct(t: T) -> Option<K> {
    use T::*;
    Some(match t {
        LParen => K::TLParen,
        RParen => K::TRParen,
        LBracket => K::TLBracket,
        RBracket => K::TRBracket,
        LBrace => K::TLBrace,
        RBrace => K::TRBrace,
        Semi => K::TSemi,
        Comma => K::TComma,
        Dot => K::TDot,
        _ => return None,
    })
}

/// A reserved operator's token kind.
fn op_kind(code: u8) -> Option<K> {
    Some(match code {
        op::COLON => K::TColon,
        op::EQ => K::TEq,
        op::ARROW => K::TFatArrow,
        op::LARROW => K::TLArrow,
        op::SUBTYPE => K::TSubtype,
        op::SUPERTYPE => K::TSupertype,
        op::VIEW => K::TViewBound,
        op::HASH => K::THash,
        op::AT => K::TAt,
        op::TYPE_LAMBDA => K::TFatArrowGt,
        op::QARROW => K::TQFatArrow,
        op::STAR => K::TStar,
        op::PIPE => K::TPipe,
        op::MINUS => K::TMinus,
        op::PLUS => K::TPlus,
        op::BANG => K::TBang,
        op::TILDE => K::TTilde,
        op::THIN => K::TArrow,
        op::QTHIN => K::TQArrow,
        op::CARET => K::TCaret,
        _ => return None,
    })
}

/// Where a statement list is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Region {
    /// The file.
    Top,
    /// Braces.
    Brace,
    /// An indented region: it ends at an outdent (a comma's too).
    Indent,
}

impl<'a> P<'a> {
    // ---- Tokens.

    #[inline(always)]
    fn fill(&mut self, k: usize) {
        while self.toks.len() <= self.i + k {
            self.lex_batch();
        }
    }

    /// Tokens lexed into the buffer, up to 16 in one loop.
    #[cold]
    fn lex_batch(&mut self) {
        const BATCH: usize = 16;
        if let Some(t) = self.toks.last().filter(|t| t.kind == T::Eof) {
            let t = *t;
            self.toks.push(t);
            return;
        }
        self.toks.reserve(BATCH);
        for _ in 0..BATCH {
            let t = match self.lx.token(self.next, &mut self.t.extras) {
                Ok(t) => t,
                Err(Refusal::Lex(at)) | Err(Refusal::Parse(at)) => Tok { kind: T::Eof, code: u8::MAX, nl: false, start: at, end: at },
                Err(Refusal::TooLarge) => Tok { kind: T::Eof, code: u8::MAX, nl: false, start: 0, end: 0 },
            };
            self.next = t.end as usize;
            self.toks.push(t);
            if t.kind == T::Eof {
                return;
            }
        }
    }

    #[inline(always)]
    fn tok(&mut self) -> Tok {
        self.fill(0);
        self.toks[self.i]
    }

    #[inline(always)]
    fn peek(&mut self) -> T {
        self.tok().kind
    }

    #[inline(always)]
    fn peek_at(&mut self, k: usize) -> Tok {
        self.fill(k);
        self.toks[self.i + k]
    }

    #[inline(always)]
    fn kind_at(&mut self, k: usize) -> T {
        self.peek_at(k).kind
    }

    /// The word code of the token `k` ahead (0: not a word).
    #[inline(always)]
    fn word_at(&mut self, k: usize) -> u8 {
        let t = self.peek_at(k);
        if t.kind == T::Id { t.code } else { 0 }
    }

    #[inline(always)]
    fn word(&mut self) -> u8 {
        self.word_at(0)
    }

    #[inline(always)]
    fn is_word(&mut self, code: u8) -> bool {
        self.word() == code
    }

    /// The operator code of the token `k` ahead (`u8::MAX`: not an operator).
    #[inline(always)]
    fn op_at(&mut self, k: usize) -> u8 {
        let t = self.peek_at(k);
        if t.kind == T::Op { t.code } else { u8::MAX }
    }

    #[inline(always)]
    fn is_op(&mut self, code: u8) -> bool {
        self.op_at(0) == code
    }

    /// Whether tokens `k` and `k + 1` ahead touch.
    #[inline]
    fn adjacent(&mut self, k: usize) -> bool {
        let (a, b) = (self.peek_at(k), self.peek_at(k + 1));
        a.end == b.start
    }

    /// Whether the next token touches the last one read.
    #[inline]
    fn touching(&mut self) -> bool {
        self.tok().start == self.t.last_end
    }

    fn text(&self, t: Tok) -> &'a [u8] {
        &self.src[t.start as usize..t.end as usize]
    }

    fn refuse<X>(&mut self) -> R<X> {
        let at = self.tok().start;
        #[cfg(debug_assertions)]
        if std::env::var_os("RERAIL_SCALA_TRACE").is_some() {
            eprintln!("refused at {at}:\n{}", std::backtrace::Backtrace::force_capture());
        }
        self.far = self.far.max(at);
        Err(Refusal::Parse(at))
    }

    /// The buffer cut at the next token, and lexing resumed at `pos`. Extras from `pos` on are
    /// lexed again.
    fn relex_at(&mut self, pos: usize) {
        self.toks.truncate(self.i);
        self.next = pos;
        let keep = self.t.extras.partition_point(|x| (x.1 as usize) < pos);
        self.t.extras.truncate(keep.max(self.t.flushed));
        self.memo = None;
    }

    /// The next token read, its extras before it written.
    #[inline(always)]
    fn bump(&mut self) -> R<Tok> {
        let t = self.tok();
        if t.kind == T::Eof && t.code == u8::MAX {
            return Err(Refusal::Lex(t.start));
        }
        self.t.flush(t.start);
        if t.kind != T::Eof {
            self.i += 1;
        }
        self.t.last_end = t.end;
        self.sp = t.end;
        self.memo = None;
        Ok(t)
    }

    /// The next token, a token child of `kind`.
    #[inline]
    fn tk(&mut self, kind: K) -> R<Tok> {
        self.tkf(kind, f::NONE)
    }

    #[inline]
    fn tkf(&mut self, kind: K, field: u8) -> R<Tok> {
        let t = self.bump()?;
        if kind == K::TLParen {
            // `(` shares its symbol with the alias `arguments`: the tape holds it as a named child.
            self.t.leaf(kind, field, t.start, t.end);
        } else {
            self.t.token(kind, field, t.start, t.end);
        }
        Ok(t)
    }

    /// The next token, which must be `t`, as a token child.
    #[inline]
    fn expect(&mut self, t: T) -> R<Tok> {
        if self.peek() != t {
            return self.refuse();
        }
        let k = punct(t).expect("a punctuation token");
        self.tk(k)
    }

    /// The next token, the reserved operator `code`, as a token child.
    #[inline]
    fn expect_op(&mut self, code: u8) -> R<Tok> {
        if !self.is_op(code) {
            return self.refuse();
        }
        self.tk(op_kind(code).expect("an operator's token"))
    }

    /// The next token, the word `code`, as a token child of kind `kind`.
    #[inline]
    fn kw(&mut self, code: u8, kind: K) -> R<Tok> {
        if !self.is_word(code) {
            return self.refuse();
        }
        self.tk(kind)
    }

    /// The next token as a named leaf of `kind`.
    #[inline]
    fn leaf(&mut self, kind: K, field: u8) -> R<u32> {
        let t = self.bump()?;
        Ok(self.t.leaf(kind, field, t.start, t.end))
    }

    #[inline]
    fn open(&mut self, kind: K) -> Node {
        let s = self.tok().start;
        self.t.open(kind, f::NONE, s)
    }

    #[inline]
    fn open_field(&mut self, kind: K, field: u8) -> Node {
        let s = self.tok().start;
        self.t.open(kind, field, s)
    }

    #[inline]
    fn close(&mut self, n: Node) -> u32 {
        self.t.close(n)
    }

    #[inline]
    fn mark(&mut self) -> Mark {
        let s = self.tok().start;
        self.t.mark(s)
    }

    fn deeper(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.refuse();
        }
        Ok(())
    }

    /// `f` read with the follow set `fol`, then the old one back.
    #[inline]
    fn with_fol<X>(&mut self, fol: V, f: impl FnOnce(&mut Self) -> R<X>) -> R<X> {
        let saved = std::mem::replace(&mut self.fol, fol);
        let r = f(self);
        self.fol = saved;
        r
    }

    // ---- Layout.

    /// The external token at the scanner's place where the state takes `own` and the follow set:
    /// `None` where the scanner returns none (the next token is the lexer's). Block comments the
    /// scanner reads on the way are read (its state kept); line comments are passed as the lexer
    /// passes them.
    fn layout(&mut self, own: V) -> R<Option<Ext>> {
        let valid = own | self.fol | v::BLOCK_COMMENT;
        // The token after the scanner's place is lexed first: its comments are in `extras`.
        self.tok();
        if let Some((at, vv, r, _)) = &self.memo {
            if *at == self.sp && *vv == valid {
                return Ok(r.map(|x| x.0));
            }
        }
        loop {
            let mut sc = self.sc.clone();
            let pos = self.sp as usize;
            match sc.scan(self.src, pos, valid) {
                Some((Ext::BlockComment, s, e)) => {
                    self.fit_comment(s as u32, e as u32)?;
                    self.sc = sc;
                    self.sp = e as u32;
                }
                Some((x, s, e)) => {
                    self.memo = Some((self.sp, valid, Some((x, s as u32, e as u32)), sc));
                    return Ok(Some(x));
                }
                None => {
                    // A line comment next: the lexer's token, then the scanner again after it.
                    let s = self.lx.blocks.blank_end(pos);
                    let mut s2 = s;
                    while matches!(self.lx.at(s2), b'\n' | b'\r' | b' ' | b'\t' | 0x0b | 0x0c) {
                        s2 = self.lx.blocks.blank_end(s2 + 1);
                    }
                    if self.lx.at(s2) == b'/' && self.lx.at(s2 + 1) == b'/' {
                        self.sp = self.lx.line_end(s2 + 2) as u32;
                        continue;
                    }
                    if self.lx.at(s2) == b'/' && self.lx.at(s2 + 1) == b'*' {
                        // A block comment the scanner did not read (it is read where the state takes
                        // a block comment: always, but past strings' states).
                        let e = self.lx.block_comment_end(s2).ok_or(Refusal::Lex(s2 as u32))?;
                        self.sp = e as u32;
                        continue;
                    }
                    self.memo = Some((self.sp, valid, None, self.sc.clone()));
                    return Ok(None);
                }
            }
        }
    }

    /// The layout token last asked (`layout`) read: the scanner's state kept, its place moved. Its
    /// place (it has no width but a block comment's).
    fn take(&mut self) -> u32 {
        let (_, _, r, sc) = self.memo.take().expect("a layout token asked");
        let (_, s, e) = r.expect("a layout token");
        self.sc = sc;
        self.sp = e;
        s
    }

    /// The block comment the scanner read at `s..e`, as the extras hold it: a comment the lexer read
    /// to another end (the blanks after it, a line comment on the line) is refused.
    fn fit_comment(&mut self, s: u32, e: u32) -> R<()> {
        let i = self.t.extras.partition_point(|x| x.1 < s);
        match self.t.extras.get(i) {
            Some(&(_, s2, e2)) if s2 == s && e2 == e => Ok(()),
            Some(&(_, s2, e2)) if s2 == s && e2 < e && self.src[e2 as usize..e as usize].iter().all(|&b| b == b' ' || b == b'\t') => {
                // The scanner's comment takes the blanks after it.
                self.t.extras[i].2 = e;
                Ok(())
            }
            _ => Err(Refusal::Parse(s)),
        }
    }

    /// A layout token read at `at` closing node `n` there (an outdent: the comments before it are
    /// the node's).
    /// Close `n` at `at` (a zero-width layout token's place): the nodes around it end there too.
    fn close_at(&mut self, n: Node, at: u32) -> u32 {
        self.t.flush(at);
        let id = self.t.close_at(n, at);
        self.t.last_end = self.t.last_end.max(at);
        id
    }

    // ---- The file and statement lists.

    fn compilation_unit(&mut self) -> R<()> {
        let root = self.t.open_root(K::CompilationUnit);
        self.statements(Region::Top, f::NONE)?;
        // The end the lexer stopped at, where it refused: the refusal.
        let t = self.tok();
        if t.code == u8::MAX {
            return Err(Refusal::Lex(t.start));
        }
        let end = self.src.len() as u32;
        self.t.flush(end + 1);
        self.t.close_root(root, 0, end);
        Ok(())
    }

    /// The follow set inside a region's statements.
    fn region_fol(region: Region) -> V {
        match region {
            Region::Top | Region::Brace => v::SEMI,
            Region::Indent => v::SEMI | v::OUTDENT | v::COMMA_OUTDENT,
        }
    }

    /// A statement list (`_block_statements`, a template's, the file's): statements, each given
    /// `field`, between semicolons. It ends before `}` (braces), at the end (the file), or before
    /// an outdent (an indented region: the caller reads it).
    fn statements(&mut self, region: Region, field: u8) -> R<()> {
        let fol = Self::region_fol(region);
        let mut count = 0;
        let r = self.with_fol(fol, |p| {
            loop {
                // Leading `;`s.
                while p.peek() == T::Semi {
                    p.tk(K::TSemi)?;
                }
                match p.peek() {
                    T::RBrace if region != Region::Top => return Ok(()),
                    T::Eof => return Ok(()),
                    _ => {}
                }
                if region == Region::Indent {
                    // An outdent before any statement (an empty region's).
                    match p.layout(fol | v::MODS)? {
                        Some(Ext::Outdent | Ext::CommaOutdent) => return Ok(()),
                        Some(Ext::Modifier(_)) | None => {}
                        Some(_) => return p.refuse(),
                    }
                }
                p.deeper()?;
                p.statement(field)?;
                p.depth -= 1;
                count += 1;
                // The separator.
                match p.layout(fol)? {
                    Some(Ext::Semi) => {
                        p.take();
                        // `end name` after a definition's semicolon is its marker: refused for now.
                        if p.is_word(w::END) && p.layout(v::END_KW)? == Some(Ext::EndKw) {
                            return p.refuse();
                        }
                    }
                    Some(Ext::Outdent | Ext::CommaOutdent) if region == Region::Indent => return Ok(()),
                    Some(_) => return p.refuse(),
                    None => match p.peek() {
                        T::Semi => {}
                        T::RBrace if region != Region::Top => return Ok(()),
                        T::Eof => return Ok(()),
                        _ => return p.refuse(),
                    },
                }
            }
        });
        self.count = count;
        r
    }

    /// A statement: a definition, or an expression (`{expression}`), given `field`.
    fn statement(&mut self, field: u8) -> R<()> {
        if self.definition_ahead()? {
            return self.definition(field);
        }
        self.expression_f(field)?;
        Ok(())
    }
}
