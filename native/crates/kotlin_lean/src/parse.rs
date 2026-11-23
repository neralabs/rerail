//! The parser: recursive descent over tree-sitter-kotlin's grammar, writing the tape as it goes
//! (`Tw`, the writer the lean Kotlin, Swift and Scala parsers share).
//!
//! **Tokens** are lexed into a buffer as the parser looks ahead, 16 at a time. Where the parse
//! state reads a token otherwise (a string's pieces, an interpolation's expression, `?.` in a
//! type), the buffer is cut at that byte and lexed again from there (`relex_at`, `split`).
//!
//! **Automatic semicolons.** Where the grammar takes `_semi` (a statement may end), the external
//! scanner decides from the bytes after the last token (`Lexer::asi`): a line end before a token
//! that does not continue the statement ends it. The parser asks wherever its state takes one
//! (`self.stmt`: at a statement's level, not inside brackets).
//!
//! **Where tree-sitter's parse keeps two readings** (GLR) and a later token picks one, the parser
//! reads ahead at that token instead: `<` starts type arguments where they close before `(`, `{`,
//! `.`, `?.` or `::`; a statement is an assignment where an assignment operator follows its target;
//! a soft keyword is a modifier where a declaration's keyword follows the modifiers.

use crate::kinds::{K, f};
use crate::lex::{Asi, AsiCtx, Lexer, R, Refusal, T, Tok, w};
use crate::tw::{Mark, Node, Tw};
#[allow(unused_imports)]
use decl::Decl;
use rerail_tape_format::{Redo, Sink, Widths, Writer};

mod decl;
mod expr;
mod types;

const MAX_DEPTH: u32 = 400;

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
    /// Whether the parse is at a statement's level: an automatic semicolon may end what is read.
    stmt: bool,
    /// The scanner's context flags (`_by_delegation_hint`, `_primary_constructor_keyword` valid).
    cx: AsiCtx,
    /// The last automatic semicolon asked: (from, flags, answer).
    asi_memo: (u32, u8, Asi, u32),
    /// Whether a `{` after an expression is not a trailing lambda (`_expression_no_trailing_lambda`:
    /// an explicit delegation's, before the class body).
    no_lambda: bool,
    /// The token index of a receiver's `.`: a user type stops before it.
    type_stop: usize,
    /// The expression depth where a keyword no expression starts with is a `simple_identifier`
    /// (tree-sitter lexes it so where only a name is valid: `split_operand_reads`).
    kw_depth: u32,
    /// The token where an expression with `kw_depth` starts (the split's operand).
    kw_at: usize,
}

/// `src` parsed.
pub fn parse(src: &[u8]) -> R<crate::Tape> {
    parse_into(src, &mut Writer::<Widths>::finish_columns)
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
            // A byte-order mark: tree-sitter's lexer passes it.
            next: if src.starts_with("\u{feff}".as_bytes()) { 3 } else { 0 },
            t: Tw::new(widths, src.len() / 3 + 16, finish.target(), src),
            depth: 0,
            far: 0,
            stmt: true,
            cx: AsiCtx::default(),
            asi_memo: (u32::MAX, 0, Asi::No, 0),
            no_lambda: false,
            type_stop: usize::MAX,
            kw_depth: u32::MAX,
            kw_at: usize::MAX,
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
        DotDot => K::TDotDot,
        DotDotLt => K::TDotDotLt,
        QuestionDot => K::TQuestionDot,
        Elvis => K::TElvis,
        Question => K::TQuestion,
        Colon => K::TColon,
        ColonColon => K::TColonColon,
        Arrow => K::TArrow,
        At => K::TAt,
        Eq => K::TEq,
        EqEq => K::TEqEq,
        EqEqEq => K::TEqEqEq,
        Bang => K::TBang,
        BangBang => K::TBangBang,
        Ne => K::TNe,
        NeEq => K::TNeEq,
        Plus => K::TPlus,
        PlusPlus => K::TPlusPlus,
        PlusEq => K::TPlusEq,
        Minus => K::TMinus,
        MinusMinus => K::TMinusMinus,
        MinusEq => K::TMinusEq,
        Star => K::TStar,
        StarEq => K::TStarEq,
        Slash => K::TSlash,
        SlashEq => K::TSlashEq,
        Percent => K::TPercent,
        PercentEq => K::TPercentEq,
        Lt => K::TLt,
        Le => K::TLe,
        Gt => K::TGt,
        Ge => K::TGe,
        AmpAmp => K::TAmpAmp,
        Amp => K::TAmp,
        PipePipe => K::TPipePipe,
        _ => return None,
    })
}

/// A soft keyword's token kind: the keyword a `simple_identifier` holds where it is one.
fn soft(code: u8) -> Option<K> {
    use K::*;
    Some(match code {
        w::EXPECT => TExpect,
        w::DATA => TData,
        w::INNER => TInner,
        w::VALUE => TValue,
        w::ACTUAL => TActual,
        w::SET => TSet,
        w::GET => TGet,
        w::OVERRIDE => TOverride,
        w::SUSPEND => TSuspend,
        w::ANNOTATION => TAnnotation,
        w::SEALED => TSealed,
        w::LATEINIT => TLateinit,
        w::TAILREC => TTailrec,
        w::OPERATOR => TOperator,
        w::INFIX => TInfix,
        w::INLINE => TInline,
        w::EXTERNAL => TExternal,
        w::PUBLIC => TPublic,
        w::PRIVATE => TPrivate,
        w::INTERNAL => TInternal,
        w::PROTECTED => TProtected,
        w::ABSTRACT => TAbstract,
        w::FINAL => TFinal,
        w::OPEN => TOpen,
        w::CONST => TConst,
        w::VARARG => TVararg,
        w::NOINLINE => TNoinline,
        w::CROSSINLINE => TCrossinline,
        w::REIFIED => TReified,
        w::FIELD => TField,
        w::PROPERTY => TProperty,
        w::RECEIVER => TReceiver,
        w::PARAM => TParam,
        w::SETPARAM => TSetparam,
        w::DELEGATE => TDelegate,
        w::COMPANION => TCompanion,
        w::CONSTRUCTOR => TConstructor,
        w::INIT => TInit,
        w::DYNAMIC => TDynamic,
        w::WHERE => TWhere,
        w::CATCH => TCatch,
        w::FINALLY => TFinally,
        w::ENUM => TEnum,
        _ => return None,
    })
}

/// Whether a name with word code `code` can be a `simple_identifier` (no hard keyword).
#[inline]
pub(crate) fn ident_word(code: u8) -> bool {
    code == 0 || code >= w::SOFT || matches!(code, w::IMPORT | w::BY | w::OUT | w::FILE | w::L)
}

impl<'a> P<'a> {
    // ---- Tokens.

    #[inline(always)]
    fn fill(&mut self, k: usize) {
        while self.toks.len() <= self.i + k {
            self.lex_batch();
        }
    }

    /// Tokens lexed into the buffer, up to 16 in one loop (simdjson's stage 2 over its index). A
    /// token lexed ahead and cut (`relex_at`) is only work lost.
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

    /// Whether token `k` ahead is a name that can be a `simple_identifier`.
    #[inline]
    fn is_ident_at(&mut self, k: usize) -> bool {
        let t = self.peek_at(k);
        t.kind == T::Name && (t.tick || ident_word(t.word))
    }

    #[inline]
    fn is_ident(&mut self) -> bool {
        self.is_ident_at(0)
    }

    /// Whether tokens `k` and `k + 1` ahead touch (no white space or comment between them).
    #[inline]
    fn adjacent(&mut self, k: usize) -> bool {
        let (a, b) = (self.peek_at(k), self.peek_at(k + 1));
        a.end == b.start
    }

    fn refuse<X>(&mut self) -> R<X> {
        let at = self.tok().start;
        #[cfg(test)]
        if std::env::var_os("RERAIL_KOTLIN_TRACE").is_some() {
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

    /// The token `k` ahead read as its first `len` bytes, of kind `kind` (`?` of `?.`): the rest is
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

    /// The next token, a child of the node open: a token of `kind` (kept or left out by the
    /// automata).
    #[inline]
    fn tk(&mut self, kind: K) -> R<Tok> {
        let t = self.bump()?;
        self.t.token(kind, f::NONE, t.start, t.end);
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

    /// A node of `kind` opened at the next token.
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

    /// A mark at the next token.
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

    /// `f` read with the statement level `stmt` and the scanner's flags cleared, then both back.
    #[inline]
    fn level<X>(&mut self, stmt: bool, f: impl FnOnce(&mut Self) -> R<X>) -> R<X> {
        let saved = (std::mem::replace(&mut self.stmt, stmt), std::mem::take(&mut self.cx), std::mem::replace(&mut self.no_lambda, false));
        let r = f(self);
        (self.stmt, self.cx, self.no_lambda) = saved;
        r
    }

    // ---- Automatic semicolons.

    /// The automatic semicolon after the last token, where the state takes one. The block comments
    /// the scanner reads past are lost from the tree as tree-sitter loses them (a zero-width comment
    /// in their stead where the scanner leaves one).
    fn asi(&mut self) -> Asi {
        let from = self.t.last_end;
        let flags = u8::from(self.cx.by) | u8::from(self.cx.ctor) << 1;
        if self.asi_memo.0 == from && self.asi_memo.1 == flags {
            return self.asi_memo.2;
        }
        let mut lost = Vec::new();
        let (a, at) = self.lx.asi(from as usize, self.cx, &mut lost);
        for (s, k) in lost {
            // The comment is lexed (the token buffer reaches past the semicolon's line).
            self.fill(1);
            let i = self.t.extras[self.t.flushed..].iter().position(|x| x.1 == s).map(|i| i + self.t.flushed);
            if let Some(i) = i {
                if k == u32::MAX {
                    self.t.extras.remove(i);
                } else {
                    self.t.extras[i] = (self.t.extras[i].0, k, k);
                }
            }
        }
        self.asi_memo = (from, flags, a, at);
        a
    }

    /// Whether what is read ends here: at a statement's level, an automatic semicolon.
    #[inline]
    fn ends(&mut self) -> bool {
        self.stmt && self.asi() != Asi::No
    }

    /// The `_semi` after a statement or member: a `;` read (hidden), or one of no width. `false`
    /// where none is (the next token continues the line).
    fn semi(&mut self) -> R<bool> {
        match self.asi() {
            Asi::No => Ok(false),
            Asi::Zero => {
                // Of no width where the scanner was last asked: past the comments on the line,
                // which are its parent's, as the semicolon is.
                let at = self.asi_memo.3;
                self.t.flush(at);
                self.t.last_end = self.t.last_end.max(at);
                Ok(true)
            }
            Asi::Semi(at) => {
                let t = self.tok();
                if t.kind != T::Semi || t.start != at {
                    return self.refuse();
                }
                self.bump()?;
                Ok(true)
            }
            Asi::Swallow(at) => {
                // The semicolon is its parent's last child: the parent ends where it does.
                self.t.flush(at);
                self.t.last_end = at;
                Ok(true)
            }
            Asi::Ctor => self.refuse(),
        }
    }

    // ---- The file.

    fn source_file(&mut self) -> R<()> {
        let root = self.t.open_root(K::SourceFile);
        let first = self.tok();
        if first.kind == T::Shebang {
            let n = self.open(K::ShebangLine);
            self.tk(K::TShebang)?;
            // The rest of the line, a hidden token.
            let mut e = first.end as usize;
            while e < self.src.len() && !matches!(self.src[e], b'\n' | b'\r') {
                e += 1;
            }
            self.t.last_end = e as u32;
            self.relex_at(e);
            self.close(n);
        }
        // File annotations, the package header, imports: each ends with `_semi`.
        while self.peek() == T::At && self.is_word_at(1, w::FILE) && self.kind_at(2) == T::Colon {
            let n = self.open(K::FileAnnotation);
            self.tk(K::TAt)?;
            self.tk(K::TFile)?;
            self.tk(K::TColon)?;
            if self.peek() == T::LBracket {
                self.tk(K::TLBracket)?;
                while self.peek() != T::RBracket {
                    self.unescaped_annotation()?;
                }
                self.tk(K::TRBracket)?;
            } else {
                self.unescaped_annotation()?;
            }
            if !self.semi()? {
                return self.refuse();
            }
            self.close(n);
        }
        if self.is_word(w::PACKAGE) {
            let n = self.open(K::PackageHeader);
            self.tk(K::TPackage)?;
            self.identifier()?;
            if !self.semi()? {
                return self.refuse();
            }
            self.close(n);
        }
        if self.is_word(w::IMPORT) {
            let list = self.open(K::ImportList);
            while self.is_word(w::IMPORT) {
                self.import_header()?;
            }
            self.close(list);
        }
        while self.peek() != T::Eof {
            self.statement()?;
            if !self.semi()? {
                return self.refuse();
            }
        }
        let t = self.tok();
        if t.word == u8::MAX {
            return Err(Refusal::Lex(t.start));
        }
        // The root spans the text from its first token or comment.
        let first = self.t.extras.first().map_or(t.start, |x| x.1).min(if self.t.w.len() > 1 { self.t.w.start(1) } else { t.start });
        self.t.close_root(root, first, self.src.len() as u32);
        Ok(())
    }

    /// `identifier`: names between `.`s.
    fn identifier(&mut self) -> R<u32> {
        let n = self.open(K::Identifier);
        loop {
            self.simple_identifier()?;
            if self.peek() == T::Dot && self.is_ident_at(1) {
                self.tk(K::TDot)?;
            } else {
                break;
            }
        }
        Ok(self.close(n))
    }

    fn import_header(&mut self) -> R<()> {
        let n = self.open(K::ImportHeader);
        self.tk(K::TImport)?;
        // `_import_identifier`: names between hidden dots, as `identifier`.
        let id = self.open(K::Identifier);
        self.simple_identifier()?;
        let mut wildcard = false;
        while self.peek() == T::Dot {
            // A `.` before a line end and `import` is the scanner's semicolon.
            if self.import_dot_ends() {
                break;
            }
            if self.is_ident_at(1) {
                self.bump()?;
                self.simple_identifier()?;
            } else if self.kind_at(1) == T::Star && self.adjacent(0) {
                wildcard = true;
                break;
            } else {
                return self.refuse();
            }
        }
        self.close(id);
        if wildcard {
            self.bump()?;
            self.leaf(K::WildcardImport, f::NONE)?;
        } else if self.is_word(w::AS) {
            let a = self.open(K::ImportAlias);
            self.tk(K::TAs)?;
            self.type_identifier()?;
            self.close(a);
        }
        if !self.semi()? {
            return self.refuse();
        }
        self.close(n);
        Ok(())
    }

    /// Whether the `.` next is followed by a line end and `import` (`scan_import_dot`).
    fn import_dot_ends(&mut self) -> bool {
        let t = self.tok();
        let mut i = t.end as usize;
        let mut nl = false;
        while i < self.src.len() && matches!(self.src[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
            nl |= matches!(self.src[i], b'\n' | b'\r');
            i += 1;
        }
        nl && self.src[i..].starts_with(b"import") && !self.src.get(i + 6).is_some_and(|&c| c.is_ascii_alphanumeric() || c == b'_')
    }

    // ---- Names.

    /// A `simple_identifier`: a name, or a soft keyword (a node holding its keyword's token).
    fn simple_identifier(&mut self) -> R<u32> {
        self.simple_identifier_as(K::SimpleIdentifier, f::NONE)
    }

    /// A `simple_identifier` aliased as `kind` (`type_identifier`, `interpolated_identifier`).
    fn simple_identifier_as(&mut self, kind: K, field: u8) -> R<u32> {
        let t = self.tok();
        if t.kind != T::Name {
            return self.refuse();
        }
        if !t.tick && t.word != 0 {
            if let Some(k) = soft(t.word) {
                let n = self.open_field(kind, field);
                self.tk(k)?;
                return Ok(self.close(n));
            }
            if !ident_word(t.word) {
                return self.refuse();
            }
        }
        self.leaf(kind, field)
    }

    fn type_identifier(&mut self) -> R<u32> {
        self.simple_identifier_as(K::TypeIdentifier, f::NONE)
    }

    // ---- Statements.

    /// `statements` inside braces (the `{` read): to the `}`, which is not read.
    fn statements(&mut self) -> R<()> {
        if self.peek() == T::RBrace {
            return Ok(());
        }
        self.level(true, |p| {
            let n = p.open(K::Statements);
            loop {
                p.statement()?;
                if !p.semi()? {
                    if p.peek() == T::RBrace {
                        break;
                    }
                    return p.refuse();
                }
                if p.peek() == T::RBrace {
                    break;
                }
            }
            p.close(n);
            Ok(())
        })
    }

    /// A `_statement`: a declaration, or labels and annotations before an assignment, a loop or
    /// an expression.
    fn statement(&mut self) -> R<()> {
        self.deeper()?;
        let r = self.statement_inner();
        self.depth -= 1;
        r
    }

    fn statement_inner(&mut self) -> R<()> {
        if self.declaration_ahead(false).is_some() {
            return self.declaration(false);
        }
        // Labels and annotations: the statement's own before a loop or an assignment; before an
        // expression, a `prefix_expression`'s.
        if self.peek() == T::At || self.label_ahead() {
            let mut k = 0;
            loop {
                let t = self.peek_at(k);
                if t.kind == T::At {
                    match self.skip_annotation(k) {
                        Some(e) => k = e,
                        None => break,
                    }
                } else if t.kind == T::Name && !t.tick && self.kind_at(k + 1) == T::At && self.peek_at(k + 1).start == t.end && !matches!(t.word, w::RETURN | w::BREAK | w::CONTINUE | w::THIS | w::SUPER) {
                    k += 2;
                } else {
                    break;
                }
            }
            let own = self.is_word_at(k, w::FOR) || self.is_word_at(k, w::WHILE) || self.is_word_at(k, w::DO) || self.assignment_ahead().is_some();
            while own && (self.peek() == T::At || self.label_ahead()) {
                if self.peek() == T::At {
                    self.annotation_node()?;
                } else {
                    self.label()?;
                }
            }
        }
        if self.is_word(w::FOR) {
            return self.for_statement();
        }
        if self.is_word(w::WHILE) {
            return self.while_statement();
        }
        if self.is_word(w::DO) {
            return self.do_while();
        }
        if let Some(op) = self.assignment_ahead() {
            return self.assignment(op);
        }
        self.expression()?;
        Ok(())
    }

    /// `label`: a name (`[a-zA-Z_][a-zA-Z_0-9]*`) and `@`, touching.
    fn label_ahead(&mut self) -> bool {
        let t = self.tok();
        t.kind == T::Name && !t.tick && !matches!(t.word, w::RETURN | w::BREAK | w::CONTINUE | w::THIS | w::SUPER) && self.kind_at(1) == T::At && self.adjacent(0) && self.src[t.start as usize..t.end as usize].iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
    }

    fn label(&mut self) -> R<u32> {
        let a = self.bump()?;
        let b = self.bump()?;
        Ok(self.t.leaf(K::Label, f::NONE, a.start, b.end))
    }

    fn for_statement(&mut self) -> R<()> {
        let n = self.open(K::ForStatement);
        self.tk(K::TFor)?;
        self.expect(T::LParen)?;
        self.level(false, |p| {
            while p.peek() == T::At {
                p.annotation_node()?;
            }
            if matches!(p.peek(), T::LParen | T::LBracket) {
                p.multi_variable_declaration()?;
            } else {
                p.variable_declaration()?;
            }
            p.kw(w::IN, K::TIn)?;
            p.expression()?;
            Ok(())
        })?;
        self.expect(T::RParen)?;
        if self.body_ahead() {
            self.control_structure_body(None)?;
        }
        self.close(n);
        Ok(())
    }

    /// Whether a `control_structure_body` follows (an optional one, where the statement may end).
    fn body_ahead(&mut self) -> bool {
        if self.ends() {
            return false;
        }
        !matches!(self.peek(), T::RBrace | T::RParen | T::RBracket | T::Comma | T::Eof | T::Semi | T::Arrow)
    }

    fn while_statement(&mut self) -> R<()> {
        let n = self.open(K::WhileStatement);
        self.tk(K::TWhile)?;
        self.expect(T::LParen)?;
        self.level(false, |p| p.expression().map(|_| ()))?;
        self.expect(T::RParen)?;
        if self.peek() == T::Semi {
            self.tk(K::TSemi)?;
        } else {
            self.control_structure_body(None)?;
        }
        self.close(n);
        Ok(())
    }

    fn do_while(&mut self) -> R<()> {
        let n = self.open(K::DoWhileStatement);
        self.tk(K::TDo)?;
        if !self.is_word(w::WHILE) {
            // The body: the `while` after it is the statement's, no semicolon between.
            let stmt = self.stmt;
            self.stmt = false;
            let r = self.control_structure_body(None);
            self.stmt = stmt;
            r?;
        }
        self.kw(w::WHILE, K::TWhile)?;
        self.expect(T::LParen)?;
        self.level(false, |p| p.expression().map(|_| ()))?;
        self.expect(T::RParen)?;
        self.close(n);
        Ok(())
    }

    /// `control_structure_body`: a block, or a statement.
    fn control_structure_body(&mut self, field: Option<u8>) -> R<u32> {
        let n = self.open_field(K::ControlStructureBody, field.unwrap_or(f::NONE));
        if self.peek() == T::LBrace && self.block_not_lambda() {
            self.block()?;
        } else {
            self.statement()?;
        }
        Ok(self.close(n))
    }

    /// Whether `{` here is a block, not a lambda: a block where both read (`prec(BLOCK)`), but a
    /// `{` whose `->` makes it a lambda's parameters is a lambda.
    fn block_not_lambda(&mut self) -> bool {
        self.lambda_params_end().is_none()
    }

    /// `_block`: `{`, `statements`, `}`, in the node open.
    fn block(&mut self) -> R<()> {
        self.expect(T::LBrace)?;
        self.statements()?;
        self.level(true, |p| p.expect(T::RBrace).map(|_| ()))?;
        Ok(())
    }

    /// The assignment operator after a statement's target, where one follows it (the target a name,
    /// or an expression ending with a navigation, an index or a postfix operator).
    fn assignment_ahead(&mut self) -> Option<T> {
        let mut k = 0;
        let mut depth = 0u32;
        loop {
            let t = self.peek_at(k);
            if depth == 0 && k > 0 && t.nl && self.stmt {
                // A line end at the statement's level: the scanner's semicolon, unless the line
                // continues.
                if !matches!(t.kind, T::Dot | T::QuestionDot | T::LBracket | T::LParen | T::LBrace | T::ColonColon | T::Lt | T::Eq | T::PlusEq | T::MinusEq | T::StarEq | T::SlashEq | T::PercentEq) {
                    return None;
                }
            }
            match t.kind {
                T::Eq | T::PlusEq | T::MinusEq | T::StarEq | T::SlashEq | T::PercentEq if depth == 0 => {
                    return (k > 0).then_some(t.kind);
                }
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    if depth == 0 {
                        return None;
                    }
                    depth -= 1;
                }
                T::Eof => return None,
                _ if depth > 0 => {}
                T::Name if !t.tick && !ident_word(t.word) && !matches!(t.word, w::THIS | w::SUPER) => return None,
                T::Name | T::Dot | T::QuestionDot | T::ColonColon | T::BangBang | T::PlusPlus | T::MinusMinus | T::Str | T::Char | T::Int | T::Hex | T::Bin | T::Real | T::At => {}
                T::Lt => {
                    // Type arguments in the target (`a<T>.b`).
                    let end = self.type_args_end(k)?;
                    k = end;
                    continue;
                }
                _ => return None,
            }
            k += 1;
        }
    }

    /// `assignment`: a `directly_assignable_expression`, an operator, an expression.
    fn assignment(&mut self, op: T) -> R<()> {
        let n = self.open(K::Assignment);
        let d = self.open(K::DirectlyAssignableExpression);
        self.assignable()?;
        self.close(d);
        if self.peek() != op {
            return self.refuse();
        }
        let k = punct(op).expect("an operator");
        self.tk(k)?;
        self.expression()?;
        self.close(n);
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    /// tree-sitter's tape of a short text (the engine's `python_tape_expect`), against the parser's.
    fn same(src: &str, want: &str) {
        let got = crate::tape_text(&crate::tape(src.as_bytes()).expect("a tape"));
        assert_eq!(got, want, "{src}");
    }

    #[test]
    fn strings() {
        same(
            r#"val s = "a$ b${c}d$e\$"
"#,
            r#"source_file [0..24]
  property_declaration [0..23]
    binding_pattern_kind [0..3]
      val [0..3]
    variable_declaration [4..5]
      simple_identifier [4..5]
    string_literal [8..23]
      string_content [9..10]
      string_content [10..11]
      string_content [11..13]
      interpolation_expression_start [13..15]
      interpolated_expression [15..16]
        simple_identifier [15..16]
      interpolation_expression_end [16..17]
      string_content [17..18]
      interpolation_identifier_start [18..19]
      interpolated_identifier [19..20]
"#,
        );
    }

    #[test]
    fn labels_and_jumps() {
        same(
            r#"fun f() { foo@ for (x in xs) { break@foo }; return@f 1 }
"#,
            r#"source_file [0..57]
  function_declaration [0..56]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..56]
      statements [10..54]
        label [10..14]
        for_statement [15..42]
          variable_declaration [20..21]
            simple_identifier [20..21]
          simple_identifier [25..27]
          control_structure_body [29..42]
            statements [31..40]
              jump_expression [31..40]
                break@ [31..37]
                label [37..40]
        jump_expression [44..54]
          return@ [44..51]
          label [51..52]
          integer_literal [53..54]
"#,
        );
    }

    #[test]
    fn automatic_semicolons() {
        same(
            r#"fun f() {
  a
  .b()
  c
  + d
}
"#,
            r#"source_file [0..33]
  function_declaration [0..32]
    simple_identifier [4..5]
    function_value_parameters [5..7]
    function_body [8..32]
      statements [12..30]
        call_expression [12..20]
          navigation_expression [12..18]
            simple_identifier [12..13]
            navigation_suffix [16..18]
              . [16..17]
              simple_identifier [17..18]
          call_suffix [18..20]
            value_arguments [18..20]
        simple_identifier [23..24]
        prefix_expression [27..30]
          + [27..28]
          simple_identifier [29..30]
"#,
        );
    }

    #[test]
    fn generic_calls_over_tighter_operations() {
        same(
            r#"val a = b ?: f<T>()!!
val c = "x" to g<String>(y)
val d = -h<T>()
val e = p < q<T>()
val f = p + q<T>()
val g = p == q<T>()
val h = x in q<T>()
val i = !q<T>()
val j = p * q<T>().z
"#,
            r#"source_file [0..181]
  property_declaration [0..21]
    binding_pattern_kind [0..3]
      val [0..3]
    variable_declaration [4..5]
      simple_identifier [4..5]
    postfix_expression [8..21]
      call_expression [8..19]
        elvis_expression [8..14]
          simple_identifier [8..9]
          simple_identifier [13..14]
        call_suffix [14..19]
          type_arguments [14..17]
            type_projection [15..16]
              user_type [15..16]
                type_identifier [15..16]
          value_arguments [17..19]
      !! [19..21]
  property_declaration [22..49]
    binding_pattern_kind [22..25]
      val [22..25]
    variable_declaration [26..27]
      simple_identifier [26..27]
    call_expression [30..49]
      infix_expression [30..38]
        string_literal [30..33]
          string_content [31..32]
        simple_identifier [34..36]
        simple_identifier [37..38]
      call_suffix [38..49]
        type_arguments [38..46]
          type_projection [39..45]
            user_type [39..45]
              type_identifier [39..45]
        value_arguments [46..49]
          value_argument [47..48]
            simple_identifier [47..48]
  property_declaration [50..65]
    binding_pattern_kind [50..53]
      val [50..53]
    variable_declaration [54..55]
      simple_identifier [54..55]
    call_expression [58..65]
      prefix_expression [58..60]
        - [58..59]
        simple_identifier [59..60]
      call_suffix [60..65]
        type_arguments [60..63]
          type_projection [61..62]
            user_type [61..62]
              type_identifier [61..62]
        value_arguments [63..65]
  property_declaration [66..84]
    binding_pattern_kind [66..69]
      val [66..69]
    variable_declaration [70..71]
      simple_identifier [70..71]
    comparison_expression [74..84]
      simple_identifier [74..75]
      < [76..77]
      call_expression [78..84]
        simple_identifier [78..79]
        call_suffix [79..84]
          type_arguments [79..82]
            type_projection [80..81]
              user_type [80..81]
                type_identifier [80..81]
          value_arguments [82..84]
  property_declaration [85..103]
    binding_pattern_kind [85..88]
      val [85..88]
    variable_declaration [89..90]
      simple_identifier [89..90]
    call_expression [93..103]
      additive_expression [93..98]
        simple_identifier [93..94]
        + [95..96]
        simple_identifier [97..98]
      call_suffix [98..103]
        type_arguments [98..101]
          type_projection [99..100]
            user_type [99..100]
              type_identifier [99..100]
        value_arguments [101..103]
  property_declaration [104..123]
    binding_pattern_kind [104..107]
      val [104..107]
    variable_declaration [108..109]
      simple_identifier [108..109]
    equality_expression [112..123]
      simple_identifier [112..113]
      == [114..116]
      call_expression [117..123]
        simple_identifier [117..118]
        call_suffix [118..123]
          type_arguments [118..121]
            type_projection [119..120]
              user_type [119..120]
                type_identifier [119..120]
          value_arguments [121..123]
  property_declaration [124..143]
    binding_pattern_kind [124..127]
      val [124..127]
    variable_declaration [128..129]
      simple_identifier [128..129]
    call_expression [132..143]
      check_expression [132..138]
        simple_identifier [132..133]
        in [134..136]
        simple_identifier [137..138]
      call_suffix [138..143]
        type_arguments [138..141]
          type_projection [139..140]
            user_type [139..140]
              type_identifier [139..140]
        value_arguments [141..143]
  property_declaration [144..159]
    binding_pattern_kind [144..147]
      val [144..147]
    variable_declaration [148..149]
      simple_identifier [148..149]
    call_expression [152..159]
      prefix_expression [152..154]
        ! [152..153]
        simple_identifier [153..154]
      call_suffix [154..159]
        type_arguments [154..157]
          type_projection [155..156]
            user_type [155..156]
              type_identifier [155..156]
        value_arguments [157..159]
  property_declaration [160..180]
    binding_pattern_kind [160..163]
      val [160..163]
    variable_declaration [164..165]
      simple_identifier [164..165]
    navigation_expression [168..180]
      call_expression [168..178]
        multiplicative_expression [168..173]
          simple_identifier [168..169]
          * [170..171]
          simple_identifier [172..173]
        call_suffix [173..178]
          type_arguments [173..176]
            type_projection [174..175]
              user_type [174..175]
                type_identifier [174..175]
          value_arguments [176..178]
      navigation_suffix [178..180]
        . [178..179]
        simple_identifier [179..180]
"#,
        );
    }
}
