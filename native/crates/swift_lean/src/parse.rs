//! The parser: recursive descent over tree-sitter-swift's grammar, writing the tape as it goes
//! (`Tw`, the writer the lean Kotlin, Swift and Scala parsers share).
//!
//! **Tokens** are lexed into a buffer as the parser looks ahead, 16 at a time. Where the parse
//! state reads a token otherwise (a string's pieces, an interpolation's expression, `>` of `>>` in
//! a type), the buffer is cut at that byte and lexed again from there (`relex_at`, `split_at`).
//!
//! **Implicit semicolons.** Where the grammar takes `_semi` (a statement may end), the external
//! scanner decides from the bytes after the last token (`Lexer::semi`): a line end before a token
//! that does not continue the statement ends it. The semicolon has no width and sits where the
//! next line's token (or comment) starts: a statement list that ends with one ends there.
//!
//! **Expressions** are read by precedence climbing over the precedences tree-sitter's parse table
//! resolves its conflicts by (`expr::Cx`): an operator after an operand is read into it where its
//! precedence is above the operand's context, as a shift beats a reduce.

use crate::kinds::{K, f};
use crate::lex::{Lexer, R, Refusal, Semi, SemiCtx, T, Tok, w};
use crate::tw::{Mark, Node, Tw};
use rerail_tape_format::{Redo, Sink, Widths};

mod decl;
mod expr;
mod types;

const MAX_DEPTH: u32 = 300;

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
    /// Whether the parse is at a statement's level: an implicit semicolon may end what is read.
    stmt: bool,
    /// The scanner's keywords valid after a statement (`default` in a switch's entry).
    sx: SemiCtx,
    /// Whether a `{` after an expression is not a trailing closure (an `if`'s condition, …).
    no_lambda: bool,
    /// In a ternary's middle operand (`-.x` there is read otherwise than elsewhere).
    if_true: bool,
    /// The end of the member after the last `Name<…>.member` read as a type.
    u_end: u32,
    /// A `!` right after `try`: its `bang` holds the `!` token.
    try_bang: bool,
    /// Calls read as binary operators' right operands in the expression open: past a few, tree-sitter's
    /// GLR versions are pruned and the nesting it keeps is not read.
    hack_calls: u32,
    /// The last semicolon asked: (from, flags, answer, where the scanner was last asked).
    semi_memo: (u32, u8, Semi),
    /// A compilation condition stopped at a function's name not of the grammar's (`_runtime(…)`):
    /// tree-sitter ends the directive there and reads the call as an expression's.
    cc_call: bool,
}

/// `src` parsed.
pub fn parse(src: &[u8]) -> R<crate::Tape> {
    parse_into(src, &mut crate::sink()).map(crate::written)
}

/// [`parse`], the writer finished by `finish`: format 15's columns, or format 16 (decision 0293).
pub fn parse_into<Out>(src: &[u8], finish: &mut dyn Sink<Out>) -> R<Out> {
    let lx = Lexer::new(src);
    if let Some(at) = lx.blocks.first_nul() {
        return Err(Refusal::Lex(at as u32));
    }
    if std::str::from_utf8(src).is_err() {
        return Err(Refusal::Lex(0));
    }
    let mut widths = Widths::guess(src.len());
    loop {
        let mut p = P {
            src,
            lx: &lx,
            toks: Vec::with_capacity(src.len() / 4 + 4),
            i: 0,
            next: if src.starts_with("\u{feff}".as_bytes()) { 3 } else { 0 },
            t: Tw::new(widths, src.len() / 3 + 16, finish.target(), src),
            depth: 0,
            far: 0,
            stmt: true,
            sx: SemiCtx::default(),
            no_lambda: false,
            if_true: false,
            u_end: 0,
            try_bang: false,
            hack_calls: 0,
            semi_memo: (u32::MAX, 0, Semi::No),
            cc_call: false,
        };
        if let Err(r) = p.source_file() {
            return Err(match r {
                Refusal::Parse(at) => Refusal::Parse(at.max(p.far)),
                r => r,
            });
        }
        match finish.finish(p.t.w) {
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

/// A punctuation or operator token's kind (`T`) as the tape's (`K`).
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
        Colon => K::TColon,
        At => K::TAt,
        Hash => K::THash,
        Backslash => K::TBackslash,
        Dot => K::TDot,
        DotDotDot => K::TDotDotDot,
        DotDotLt => K::TDotDotLt,
        Arrow => K::TArrow,
        Eq => K::TEq,
        EqEq => K::TEqEq,
        EqEqEq => K::TEqEqEq,
        Bang => K::TBang,
        Question => K::TQuestion,
        QQ => K::TQQ,
        AmpAmp => K::TAmpAmp,
        PipePipe => K::TPipePipe,
        Plus => K::TPlus,
        Minus => K::TMinus,
        Star => K::TStar,
        Slash => K::TSlash,
        Percent => K::TPercent,
        Lt => K::TLt,
        Gt => K::TGt,
        Amp => K::TAmp,
        Pipe => K::TPipe,
        Caret => K::TCaret,
        Tilde => K::TTilde,
        PlusEq => K::TPlusEq,
        MinusEq => K::TMinusEq,
        StarEq => K::TStarEq,
        SlashEq => K::TSlashEq,
        PercentEq => K::TPercentEq,
        PlusPlus => K::TPlusPlus,
        MinusMinus => K::TMinusMinus,
        LtLt => K::TLtLt,
        GtGt => K::TGtGt,
        _ => return None,
    })
}

/// A contextual word's token: a `simple_identifier` of it is a node holding it
/// (`_contextual_simple_identifier`).
fn contextual(code: u8) -> Option<K> {
    use K::*;
    Some(match code {
        w::ACTOR => TActor,
        w::ASYNC => TAsync,
        w::CONSUME => TConsume,
        w::COPY => TCopy,
        w::DISCARD => TDiscard,
        w::EACH => TEach,
        w::LAZY => TLazy,
        w::REPEAT => TRepeat,
        w::PACKAGE => TPackage,
        w::UNSAFE => TUnsafe,
        w::BORROWING => TBorrowing,
        w::CONSUMING => TConsuming,
        w::SENDING => TSending,
        w::ISOLATED => TIsolated,
        _ => return None,
    })
}

impl<'a> P<'a> {
    // ---- Tokens.

    #[inline(always)]
    fn fill(&mut self, k: usize) {
        while self.toks.len() <= self.i + k {
            self.lex_batch();
        }
    }

    /// Tokens lexed into the buffer, up to 16 in one loop. A token lexed ahead and cut
    /// (`relex_at`) is only work lost.
    #[cold]
    fn lex_batch(&mut self) {
        const BATCH: usize = 16;
        if let Some(t) = self.toks.last().filter(|t| t.kind == T::Eof) {
            let t = *t;
            self.toks.push(t);
            return;
        }
        // A block at a time (`Lexer::lex_run`), at least a batch.
        match self.lx.lex_run(self.next, &mut self.toks, BATCH, &mut self.t.extras) {
            Ok(next) => self.next = next,
            // A token the lexer refuses is refused where the parser reads it.
            Err(Refusal::Lex(at)) | Err(Refusal::Parse(at)) => self.toks.push(Tok { kind: T::Eof, word: u8::MAX, nl: false, tick: false, start: at, end: at }),
            Err(Refusal::TooLarge) => self.toks.push(Tok { kind: T::Eof, word: u8::MAX, nl: false, tick: false, start: 0, end: 0 }),
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

    /// Whether token `k` ahead is the word `code`.
    #[inline(always)]
    fn is_word_at(&mut self, k: usize, code: u8) -> bool {
        let t = self.peek_at(k);
        t.kind == T::Name && t.word == code && !t.tick
    }

    #[inline(always)]
    fn is_word(&mut self, code: u8) -> bool {
        self.is_word_at(0, code)
    }

    /// The word code of the next token (0: not a plain word).
    #[inline(always)]
    fn word(&mut self) -> u8 {
        let t = self.tok();
        if t.kind == T::Name && !t.tick { t.word } else { 0 }
    }

    #[inline(always)]
    fn word_at(&mut self, k: usize) -> u8 {
        let t = self.peek_at(k);
        if t.kind == T::Name && !t.tick { t.word } else { 0 }
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

    fn refuse<X>(&mut self) -> R<X> {
        let at = self.tok().start;
        #[cfg(test)]
        if std::env::var_os("RERAIL_SWIFT_TRACE").is_some() {
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
    }

    /// The token `k` ahead read as its first `len` bytes, of kind `kind` (`>` of `>>`): the rest is
    /// lexed again after it.
    fn split_at(&mut self, k: usize, kind: T, len: u32) {
        let t = self.peek_at(k);
        self.toks.truncate(self.i + k);
        let pos = (t.start + len) as usize;
        self.next = pos;
        let keep = self.t.extras.partition_point(|x| (x.1 as usize) < pos);
        self.t.extras.truncate(keep.max(self.t.flushed));
        self.toks.push(Tok { kind, word: 0, nl: t.nl, tick: false, start: t.start, end: t.start + len });
    }

    /// The next token read, its extras before it written.
    #[inline(always)]
    fn bump(&mut self) -> R<Tok> {
        let t = self.tok();
        if t.kind == T::Eof && t.word == u8::MAX {
            return Err(Refusal::Lex(t.start));
        }
        self.t.flush(t.start);
        if t.kind != T::Eof {
            self.i += 1;
        }
        self.t.last_end = t.end;
        Ok(t)
    }

    /// The next token, a child of the node open: a token of `kind`.
    #[inline]
    fn tk(&mut self, kind: K) -> R<Tok> {
        let t = self.bump()?;
        self.t.token(kind, f::NONE, t.start, t.end);
        Ok(t)
    }

    /// The next token, a token child of `kind` given `field`.
    #[inline]
    fn tkf(&mut self, kind: K, field: u8) -> R<Tok> {
        let t = self.bump()?;
        self.t.token(kind, field, t.start, t.end);
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

    /// A named node of `kind` holding the next token as a token of `tok`.
    fn wrap_tok(&mut self, kind: K, field: u8, tok: K) -> R<u32> {
        let n = self.open_field(kind, field);
        self.tk(tok)?;
        Ok(self.close(n))
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

    /// Whether `f` reads from here (a trial: the tape and the parser's state go back after it,
    /// whatever it read). Tree-sitter's GLR keeps a reading where it holds: this asks whether one does.
    fn trial(&mut self, f: impl FnOnce(&mut Self) -> R<()>) -> bool {
        let saved = self.t.save();
        let (i, depth, far, stmt, sx, no_lambda, if_true) = (self.i, self.depth, self.far, self.stmt, self.sx, self.no_lambda, self.if_true);
        let (u_end, try_bang, hack_calls, semi_memo) = (self.u_end, self.try_bang, self.hack_calls, self.semi_memo);
        let ok = f(self).is_ok();
        self.t.rewind(saved);
        (self.i, self.depth, self.far, self.stmt, self.sx, self.no_lambda, self.if_true) = (i, depth, far, stmt, sx, no_lambda, if_true);
        (self.u_end, self.try_bang, self.hack_calls, self.semi_memo) = (u_end, try_bang, hack_calls, semi_memo);
        ok
    }

    fn deeper(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.refuse();
        }
        Ok(())
    }

    /// `f` read with the statement level `stmt`, the scanner's keywords cleared and trailing
    /// closures taken, then all back.
    #[inline]
    fn level<X>(&mut self, stmt: bool, f: impl FnOnce(&mut Self) -> R<X>) -> R<X> {
        let saved = (std::mem::replace(&mut self.stmt, stmt), std::mem::take(&mut self.sx), std::mem::replace(&mut self.no_lambda, false));
        let r = f(self);
        (self.stmt, self.sx, self.no_lambda) = saved;
        r
    }

    // ---- Implicit semicolons.

    /// The semicolon after the last token, where the state takes one. A block comment the scanner
    /// reads on a line after the token is an extra; the scanner is asked again after it.
    fn semi_at(&mut self) -> Semi {
        let from = self.t.last_end;
        let mut cx = self.sx;
        cx.expr = true;
        let flags = u8::from(cx.default) | u8::from(cx.else_) << 1 | u8::from(cx.catch) << 2 | u8::from(cx.where_) << 3 | u8::from(cx.arrow) << 4;
        if self.semi_memo.0 == from && self.semi_memo.1 == flags {
            return self.semi_memo.2;
        }
        let mut at = from as usize;
        let s = loop {
            match self.lx.semi(at, cx) {
                Semi::Comment(c) => match self.lx.block_comment_end(c as usize) {
                    Some(e) => at = e,
                    None => break Semi::No,
                },
                s => break s,
            }
        };
        self.semi_memo = (from, flags, s);
        s
    }

    /// Whether what is read ends here: at a statement's level, a semicolon.
    #[inline]
    fn ends(&mut self) -> bool {
        self.stmt && self.semi_at() != Semi::No
    }

    /// The `_semi` after a statement or member: a `;` read (hidden), or one of no width. `false`
    /// where none is (the next token continues the line).
    fn semi(&mut self) -> R<bool> {
        match self.semi_at() {
            Semi::No | Semi::Comment(_) => Ok(false),
            Semi::Implicit(at) => {
                // The comments before it lexed (the next token's), then written inside.
                let _ = self.tok();
                self.t.flush(at);
                self.t.last_end = self.t.last_end.max(at);
                Ok(true)
            }
            Semi::Explicit(at) => {
                let t = self.tok();
                if t.kind != T::Semi || t.start != at {
                    return self.refuse();
                }
                self.bump()?;
                Ok(true)
            }
        }
    }

    // ---- The file.

    fn source_file(&mut self) -> R<()> {
        let root = self.t.open_root(K::SourceFile);
        let first = self.tok();
        if first.kind == T::Shebang {
            let n = self.open(K::ShebangLine);
            self.bump()?;
            self.t.token(K::THash, f::NONE, first.start, first.start + 1);
            self.t.token(K::TBang, f::NONE, first.start + 1, first.end);
            let e = self.lx.blocks.eol(first.end as usize);
            self.t.last_end = e as u32;
            self.relex_at(e);
            self.close(n);
        }
        while self.peek() != T::Eof {
            if self.peek() == T::Semi {
                self.bump()?;
                continue;
            }
            self.statement(true)?;
            if !self.semi()? && self.peek() != T::Eof && !self.directive_next() {
                return self.refuse();
            }
        }
        let t = self.tok();
        if t.word == u8::MAX {
            return Err(Refusal::Lex(t.start));
        }
        let first = self.t.extras.first().map_or(t.start, |x| x.1).min(if self.t.w.len() > 1 { self.t.w.start(1) } else { t.start });
        self.t.close_root(root, first, self.src.len() as u32);
        Ok(())
    }

    /// Whether `#error`, `#warning` or `#sourceLocation` is next.
    fn diagnostic_next(&mut self) -> bool {
        self.peek() == T::Hash && self.adjacent(0) && matches!(self.word_at(1), w::ERROR | w::WARNING | w::SOURCELOCATION)
    }

    /// `diagnostic`: `#` and the rest of its line (the grammar's `/error([^\r\n]*)/`).
    fn diagnostic(&mut self) -> R<()> {
        let t = self.bump()?;
        let e = self.lx.blocks.eol(t.end as usize);
        self.t.leaf(K::Diagnostic, f::NONE, t.start, e as u32);
        self.relex_at(e);
        self.t.last_end = e as u32;
        Ok(())
    }

    /// Whether a directive (`#if`, …) is next: it needs no semicolon before it.
    fn directive_next(&mut self) -> bool {
        self.peek() == T::Hash && matches!(self.word_at(1), w::IF | w::ELSEIF | w::ELSE | w::ENDIF) && self.adjacent(0)
    }

    // ---- Names.

    /// A `simple_identifier`: a name (a contextual word: a node holding its token).
    fn simple_identifier(&mut self) -> R<u32> {
        self.ident_as(K::SimpleIdentifier, f::NONE)
    }

    fn simple_identifier_f(&mut self, field: u8) -> R<u32> {
        self.ident_as(K::SimpleIdentifier, field)
    }

    /// A `simple_identifier` aliased as `kind` (`type_identifier`).
    fn ident_as(&mut self, kind: K, field: u8) -> R<u32> {
        let t = self.tok();
        if t.kind != T::Name {
            return self.refuse();
        }
        if !t.tick
            && let Some(k) = contextual(t.word) {
                return self.wrap_tok(kind, field, k);
            }
        self.leaf(kind, field)
    }

    /// `identifier`: names between `.`s.
    fn identifier(&mut self, field: u8) -> R<u32> {
        let n = self.open_field(K::Identifier, field);
        loop {
            self.simple_identifier()?;
            if self.peek() == T::Dot && self.kind_at(1) == T::Name {
                self.tk(K::TDot)?;
            } else {
                break;
            }
        }
        Ok(self.close(n))
    }

    // ---- Statements.

    /// `statements` inside braces (the `{` read): to the `}` (or, in a switch, the next entry),
    /// which is not read. `entry`: a switch entry's (`case` and `default` end it).
    fn statements(&mut self, entry: bool) -> R<()> {
        if self.statements_end(entry) {
            return Ok(());
        }
        self.level(true, |p| {
            p.sx.default = entry;
            let n = p.open(K::Statements);
            loop {
                if p.peek() == T::Semi {
                    p.bump()?;
                } else {
                    p.statement(false)?;
                    if !p.semi()? && !p.statements_end(entry) && !p.directive_next() {
                        return p.refuse();
                    }
                }
                if p.statements_end(entry) {
                    break;
                }
            }
            p.close(n);
            Ok(())
        })
    }

    fn statements_end(&mut self, entry: bool) -> bool {
        match self.peek() {
            T::RBrace | T::Eof => true,
            T::Name if entry => {
                matches!(self.word(), w::CASE | w::DEFAULT)
            }
            T::At if entry => self.attributes_then_case(),
            _ => false,
        }
    }

    /// A statement: `top`, at the file's level (`_top_level_statement`), else a local one.
    fn statement(&mut self, top: bool) -> R<()> {
        self.deeper()?;
        let r = self.statement_inner(top);
        self.depth -= 1;
        r
    }

    fn statement_inner(&mut self, top: bool) -> R<()> {
        let t = self.tok();
        if self.diagnostic_next() {
            return self.diagnostic();
        }
        match t.kind {
            T::Hash if matches!(self.word_at(1), w::IF | w::ELSEIF | w::ELSE | w::ENDIF) && self.adjacent(0) => {
                let m = self.mark();
                self.directive()?;
                if std::mem::take(&mut self.cc_call) {
                    // `#if _runtime(_ObjC) && os(macOS)`: the directive ends at the name, the
                    // rest is an expression's, the directive the first call's function (probed).
                    self.directive_call(m)?;
                }
                self.t.unmark(m);
                return Ok(());
            }
            T::Name if !t.tick => {
                match t.word {
                    w::IF => return self.if_statement(),
                    w::GUARD => return self.guard_statement(),
                    w::SWITCH => return self.switch_statement(),
                    w::FOR => return self.for_statement(),
                    w::WHILE => return self.while_statement(),
                    w::REPEAT if self.kind_at(1) == T::LBrace => return self.repeat_while_statement(),
                    w::DO if self.kind_at(1) == T::LBrace || self.is_word_at(1, w::THROWS) => return self.do_statement(),
                    w::RETURN | w::BREAK | w::CONTINUE | w::YIELD | w::THROW => return self.control_transfer(top),
                    _ => {}
                }
                // A label before a loop: `name:` touching.
                if self.kind_at(1) == T::Colon && self.adjacent(0) && t.word == 0 {
                    let w2 = self.word_at(2);
                    if matches!(w2, w::FOR | w::WHILE | w::REPEAT | w::DO | w::IF | w::GUARD | w::SWITCH) {
                        // `statement_label`: one token, the name and its `:`.
                        let colon = self.peek_at(1);
                        self.bump()?;
                        self.bump()?;
                        self.t.leaf(K::StatementLabel, f::NONE, t.start, colon.end);
                        return match w2 {
                            w::FOR => self.for_statement(),
                            w::WHILE => self.while_statement(),
                            w::REPEAT => self.repeat_while_statement(),
                            w::DO => self.do_statement(),
                            w::IF => self.if_statement(),
                            w::GUARD => self.guard_statement(),
                            _ => self.switch_statement(),
                        };
                    }
                }
            }
            _ => {}
        }
        if let Some(d) = self.declaration_ahead() {
            return self.declaration(d, if top { decl::Level::Top } else { decl::Level::Local });
        }
        // A modifier's word starting a statement is the keyword for tree-sitter's lexer (the state
        // takes it, no word rule extracts it), and no declaration follows: an error there.
        if t.kind == T::Name && !t.tick && contextual(t.word).is_none() && decl::starts_declaration(t.word) {
            return self.refuse();
        }
        self.expression_statement()
    }

    /// An expression as a statement.
    fn expression_statement(&mut self) -> R<()> {
        self.expression()
    }

    fn control_transfer(&mut self, top: bool) -> R<()> {
        let n = self.open(K::ControlTransferStatement);
        let word = self.word();
        if word == w::THROW {
            self.leaf(K::ThrowKeyword, f::NONE)?;
            self.expression()?;
            self.close(n);
            return Ok(());
        }
        let _ = top;
        let k = match word {
            w::RETURN => K::TReturn,
            w::BREAK => K::TBreak,
            w::CONTINUE => K::TContinue,
            _ => K::TYield,
        };
        self.tk(k)?;
        // A result: on the same line, an expression's start.
        if !self.ends() && self.expression_starts() {
            let m = self.mark();
            self.expression()?;
            self.t.field_since(m, f::RESULT);
            self.t.unmark(m);
        }
        self.close(n);
        Ok(())
    }

    /// `{` statements `}`, the braces given to the node open.
    fn block(&mut self) -> R<()> {
        self.expect(T::LBrace)?;
        self.statements(false)?;
        self.expect(T::RBrace)?;
        Ok(())
    }

    /// `function_body`.
    fn function_body(&mut self) -> R<()> {
        let n = self.open_field(K::FunctionBody, f::BODY);
        self.level(true, |p| p.block())?;
        self.close(n);
        Ok(())
    }

    // ---- Control flow.

    /// A condition list (`_if_condition_sequence_item`s between `,`s), each given `condition`.
    fn conditions(&mut self) -> R<()> {
        self.level(false, |p| {
            p.no_lambda = true;
            loop {
                let m = p.mark();
                p.condition()?;
                p.t.field_since(m, f::CONDITION);
                p.t.unmark(m);
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
            }
            Ok(())
        })
    }

    /// One condition: `let`/`var`/`case` binding, `#available`, or an expression.
    fn condition(&mut self) -> R<()> {
        match self.word() {
            w::LET | w::VAR => {
                self.value_binding_pattern(f::CONDITION)?;
                self.no_expr_pattern(true)?;
                if self.peek() == T::Colon {
                    self.type_annotation()?;
                }
                if self.peek() == T::Eq {
                    self.tkf(K::TEq, f::CONDITION)?;
                    self.expression()?;
                }
                Ok(())
            }
            w::CASE => {
                self.tkf(K::TCase, f::CONDITION)?;
                self.binding_pattern_no_expr()?;
                if self.peek() == T::Colon {
                    self.type_annotation()?;
                }
                self.tkf(K::TEq, f::CONDITION).and_then(|t| if t.kind == T::Eq { Ok(t) } else { self.refuse() })?;
                self.expression()
            }
            _ if self.peek() == T::Hash && matches!(self.word_at(1), w::AVAILABLE | w::UNAVAILABLE) => self.availability_condition(),
            _ => self.expression(),
        }
    }

    fn availability_condition(&mut self) -> R<()> {
        let n = self.open(K::AvailabilityCondition);
        self.tk(K::THash)?;
        if self.is_word(w::AVAILABLE) {
            self.tk(K::TAvailable)?;
        } else {
            self.tk(K::TUnavailable)?;
        }
        self.expect(T::LParen)?;
        loop {
            if self.peek() == T::Star {
                self.tk(K::TStar)?;
            } else {
                self.identifier(f::NONE)?;
                if matches!(self.peek(), T::Int | T::Real) {
                    self.version_numbers()?;
                }
            }
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::TComma)?;
        }
        self.expect(T::RParen)?;
        self.close(n);
        Ok(())
    }

    /// `integer_literal ("." integer_literal)*`: a version, which the lexer reads as reals.
    fn version_numbers(&mut self) -> R<()> {
        let t = self.tok();
        let (s, e) = (t.start as usize, t.end as usize);
        self.bump()?;
        let mut a = s;
        for i in s..=e {
            if i == e || self.src[i] == b'.' {
                self.t.leaf(K::IntegerLiteral, f::NONE, a as u32, i as u32);
                if i < e {
                    self.t.token(K::TDot, f::NONE, i as u32, i as u32 + 1);
                }
                a = i + 1;
            }
        }
        // `1.2.3`: the lexer's real `1.2`, then `.3`.
        while self.peek() == T::Dot && self.adjacent(0) && self.kind_at(1) == T::Int && self.touching() {
            self.tk(K::TDot)?;
            self.leaf(K::IntegerLiteral, f::NONE)?;
        }
        Ok(())
    }

    fn if_statement(&mut self) -> R<()> {
        let n = self.open(K::IfStatement);
        self.kw(w::IF, K::TIf)?;
        self.conditions()?;
        self.level(true, |p| p.block())?;
        if self.is_word(w::ELSE) {
            self.leaf(K::Else, f::NONE)?;
            if self.is_word(w::IF) {
                self.if_statement()?;
            } else {
                self.level(true, |p| p.block())?;
            }
        }
        self.close(n);
        Ok(())
    }

    fn guard_statement(&mut self) -> R<()> {
        let n = self.open(K::GuardStatement);
        self.kw(w::GUARD, K::TGuard)?;
        self.conditions()?;
        if !self.is_word(w::ELSE) {
            return self.refuse();
        }
        self.leaf(K::Else, f::NONE)?;
        self.level(true, |p| p.block())?;
        self.close(n);
        Ok(())
    }

    fn while_statement(&mut self) -> R<()> {
        let n = self.open(K::WhileStatement);
        self.kw(w::WHILE, K::TWhile)?;
        self.conditions()?;
        self.level(true, |p| p.block())?;
        self.close(n);
        Ok(())
    }

    fn repeat_while_statement(&mut self) -> R<()> {
        let n = self.open(K::RepeatWhileStatement);
        self.kw(w::REPEAT, K::TRepeat)?;
        self.level(true, |p| p.block())?;
        self.kw(w::WHILE, K::TWhile)?;
        self.conditions()?;
        self.close(n);
        Ok(())
    }

    fn for_statement(&mut self) -> R<()> {
        let n = self.open(K::ForStatement);
        self.kw(w::FOR, K::TFor)?;
        if self.is_word(w::TRY) {
            self.try_operator()?;
        }
        if self.is_word(w::AWAIT) {
            self.tk(K::TAwait)?;
        }
        let m = self.mark();
        let p = self.open_field(K::Pattern, f::ITEM);
        self.binding_pattern_no_expr_inner(true)?;
        self.close(p);
        self.t.unmark(m);
        if self.peek() == T::Colon {
            self.type_annotation()?;
        }
        self.kw(w::IN, K::TIn)?;
        self.level(false, |p| {
            p.no_lambda = true;
            let m = p.mark();
            p.expression()?;
            p.t.field_since(m, f::COLLECTION);
            p.t.unmark(m);
            if p.is_word(w::WHERE) {
                p.where_clause()?;
            }
            Ok(())
        })?;
        self.level(true, |p| p.block())?;
        self.close(n);
        Ok(())
    }

    fn where_clause(&mut self) -> R<()> {
        let n = self.open(K::WhereClause);
        self.leaf(K::WhereKeyword, f::NONE)?;
        self.expression()?;
        self.close(n);
        Ok(())
    }

    fn do_statement(&mut self) -> R<()> {
        let n = self.open(K::DoStatement);
        self.kw(w::DO, K::TDo)?;
        if self.is_word(w::THROWS) {
            self.throws()?;
        }
        self.level(true, |p| p.block())?;
        while self.is_word(w::CATCH) {
            let c = self.open(K::CatchBlock);
            self.leaf(K::CatchKeyword, f::NONE)?;
            if self.peek() != T::LBrace && !self.is_word(w::WHERE) {
                self.level(false, |p| {
                    p.no_lambda = true;
                    let q = p.open_field(K::Pattern, f::ERROR);
                    p.binding_pattern_no_expr_inner(false)?;
                    p.close(q);
                    Ok(())
                })?;
            }
            if self.is_word(w::WHERE) {
                self.level(false, |p| {
                    p.no_lambda = true;
                    p.where_clause()
                })?;
            }
            self.level(true, |p| p.block())?;
            self.close(c);
        }
        self.close(n);
        Ok(())
    }

    fn switch_statement(&mut self) -> R<()> {
        let n = self.open(K::SwitchStatement);
        self.kw(w::SWITCH, K::TSwitch)?;
        self.level(false, |p| {
            p.no_lambda = true;
            let m = p.mark();
            p.expression()?;
            p.t.field_since(m, f::EXPR);
            p.t.unmark(m);
            Ok(())
        })?;
        self.expect(T::LBrace)?;
        loop {
            match self.peek() {
                T::RBrace => break,
                // A directive between entries: tree-sitter reads it into statements (none open
                // before the first entry) and fails.
                T::Hash if self.directive_next() => return self.refuse(),
                _ => self.switch_entry()?,
            }
        }
        self.expect(T::RBrace)?;
        self.close(n);
        Ok(())
    }

    fn switch_entry(&mut self) -> R<()> {
        let n = self.open(K::SwitchEntry);
        if self.peek() == T::At {
            let m = self.open(K::Modifiers);
            while self.peek() == T::At {
                self.attribute()?;
            }
            self.close(m);
        }
        if self.is_word(w::DEFAULT) {
            self.leaf(K::DefaultKeyword, f::NONE)?;
        } else {
            self.kw(w::CASE, K::TCase)?;
            self.level(false, |p| {
                loop {
                    let s = p.open(K::SwitchPattern);
                    let q = p.open(K::Pattern);
                    p.binding_pattern_with_expr()?;
                    p.close(q);
                    p.close(s);
                    if p.is_word(w::WHERE) {
                        p.leaf(K::WhereKeyword, f::NONE)?;
                        p.expression()?;
                    }
                    if p.peek() != T::Comma {
                        break;
                    }
                    p.tk(K::TComma)?;
                }
                Ok(())
            })?;
        }
        self.expect(T::Colon)?;
        // `fallthrough` is read as a name (the grammar's token is never taken).
        self.statements(true)?;
        self.close(n);
        Ok(())
    }

    /// Whether `@attributes` then `case`/`default` come next (a switch entry's modifiers).
    fn attributes_then_case(&mut self) -> bool {
        let mut k = 0;
        while self.kind_at(k) == T::At {
            match self.attribute_end(k) {
                Some(e) => k = e,
                None => return false,
            }
        }
        matches!(self.word_at(k), w::CASE | w::DEFAULT)
    }

    // ---- Directives.

    /// `directive`: `#if` and its condition, `#elseif`, `#else`, `#endif`.
    fn directive(&mut self) -> R<()> {
        let n = self.open(K::Directive);
        let hash = self.tok();
        let word = self.peek_at(1);
        let kind = match self.word_at(1) {
            w::IF => K::THashIf,
            w::ELSEIF => K::THashElseif,
            w::ELSE => K::THashElse,
            _ => K::THashEndif,
        };
        self.bump()?;
        self.bump()?;
        self.t.token(kind, f::NONE, hash.start, word.end);
        if matches!(kind, K::THashIf | K::THashElseif) {
            self.level(false, |p| p.compilation_condition(0))?;
        }
        self.close(n);
        Ok(())
    }

    /// `_compilation_condition` (hidden): its pieces are the directive's children. `min`: the
    /// binding of the operator before it (`&&` 2, `||` 1).
    fn compilation_condition(&mut self, min: u8) -> R<()> {
        self.deeper()?;
        match self.peek() {
            T::Bang => {
                self.tk(K::TBang)?;
                self.compilation_condition(3)?;
            }
            T::LParen => {
                self.tk(K::TLParen)?;
                self.compilation_condition(0)?;
                if self.cc_call {
                    return self.refuse();
                }
                self.expect(T::RParen)?;
            }
            T::Name => {
                let word = self.word();
                let call = self.kind_at(1) == T::LParen;
                match word {
                    w::OS | w::ARCH | w::TARGETENVIRONMENT if call => {
                        self.tk(match word {
                            w::OS => K::TOs,
                            w::ARCH => K::TArch,
                            _ => K::TTargetEnvironment,
                        })?;
                        self.tk(K::TLParen)?;
                        self.simple_identifier()?;
                        self.expect(T::RParen)?;
                    }
                    w::SWIFT | w::COMPILER if call => {
                        self.tk(if word == w::SWIFT { K::TSwift } else { K::TCompiler })?;
                        self.tk(K::TLParen)?;
                        match self.peek() {
                            T::Lt => self.tk(K::TLt)?,
                            T::Gt => self.tk(K::TGt)?,
                            // `>=` and `<=` are one token there (the operator's choice).
                            T::Custom if matches!(&self.src[self.tok().start as usize..self.tok().end as usize], b">=" | b"<=") => {
                                let k = if self.src[self.tok().start as usize] == b'>' { K::TGe } else { K::TLe };
                                self.tk(k)?
                            }
                            _ => return self.refuse(),
                        };
                        self.version_numbers()?;
                        self.expect(T::RParen)?;
                    }
                    w::CANIMPORT if call => {
                        self.tk(K::TCanImport)?;
                        self.tk(K::TLParen)?;
                        loop {
                            self.simple_identifier()?;
                            if self.peek() != T::Dot {
                                break;
                            }
                            self.tk(K::TDot)?;
                        }
                        self.expect(T::RParen)?;
                    }
                    w::TRUE | w::FALSE => {
                        let n = self.open(K::BooleanLiteral);
                        self.tk(if word == w::TRUE { K::TTrue } else { K::TFalse })?;
                        self.close(n);
                    }
                    _ => {
                        self.simple_identifier()?;
                        if call && self.touching() {
                            self.cc_call = true;
                            self.depth -= 1;
                            return Ok(());
                        }
                    }
                }
            }
            _ => return self.refuse(),
        }
        loop {
            if self.cc_call {
                break;
            }
            match self.peek() {
                T::AmpAmp if min < 2 => {
                    self.tk(K::TAmpAmp)?;
                    self.compilation_condition(2)?;
                }
                T::PipePipe if min < 1 => {
                    self.tk(K::TPipePipe)?;
                    self.compilation_condition(1)?;
                }
                _ => break,
            }
        }
        self.depth -= 1;
        Ok(())
    }
}
