//! The parser: recursive descent over tree-sitter-java's grammar, writing the tape as it goes.
//!
//! **Tokens** are lexed into a buffer as the parser looks ahead, 16 at a time. Where the parse
//! state reads a token shorter than the one lexed (a `>>` closing two type argument lists), the
//! buffer is cut at that byte and lexed again from there (`relex_at`, `split`).
//!
//! **Where tree-sitter's parse keeps two readings** (GLR) and a later token picks one, the parser
//! reads ahead at that token instead: a statement is a declaration where a type and a name start
//! it; `(` starts a cast where a type and `)` follow it and an operand comes after; a name before
//! `<` is a generic type where the angle brackets close before `::`.
//!
//! **Stored tokens.** A token other than a delimiter is stored unless its kind implies it
//! (`kinds::DROPPED`, `stores`). A delimiter its kind implies is still stored where its gap (the
//! tokens between two named children) is not the gap's one shortest word: a trailing comma, an
//! empty statement among a block's. Each list's caller reads the gap ahead and decides before it
//! writes.

use crate::kinds::{K, f};
use crate::lex::{Extra, Lexer, R, Refusal, T, Tok, w};
use crate::{Tape, key, unkey};
use rerail_tape_format::{Redo, Widths, Writer};

mod decl;
mod expr;
mod stmt;
mod types;

// Supertype sets (`crate::CLASSES`).
const C_NONE: u8 = 0;
const C_E: u8 = 1;
const C_EP: u8 = 2;
const C_LEP: u8 = 3;
const C_P: u8 = 4;
const C_LP: u8 = 5;
const C_S: u8 = 6;
const C_SD: u8 = 7;
const C_UT: u8 = 8;
const C_TUT: u8 = 9;
const C_T: u8 = 10;
const C_MD: u8 = 11;

/// The context a node stands in: which supertypes its kind is in there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctx {
    /// An `expression`: a primary expression is in `primary_expression` too, a literal in
    /// `_literal`.
    Expr,
    /// A `primary_expression` (an object, a cast's operand…).
    Primary,
    /// A `statement`: a declaration is in `declaration` too.
    Stmt,
    /// A `_type`: an unannotated type is in `_unannotated_type` too.
    Type,
    /// An `_unannotated_type`.
    UType,
    /// A `module_directive`.
    Directive,
    /// None: a kind named directly by its parent's rule.
    Bare,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    field: u8,
    ctx: Ctx,
}

const fn slot(field: u8, ctx: Ctx) -> Slot {
    Slot { field, ctx }
}

const EXPR: Slot = slot(f::NONE, Ctx::Expr);
const STMT: Slot = slot(f::NONE, Ctx::Stmt);
const TYPE: Slot = slot(f::NONE, Ctx::Type);
const UTYPE: Slot = slot(f::NONE, Ctx::UType);
const BARE: Slot = slot(f::NONE, Ctx::Bare);

fn is_literal(k: K) -> bool {
    use K::*;
    matches!(
        k,
        DecimalIntegerLiteral
            | HexIntegerLiteral
            | OctalIntegerLiteral
            | BinaryIntegerLiteral
            | DecimalFloatingPointLiteral
            | HexFloatingPointLiteral
            | True
            | False
            | CharacterLiteral
            | StringLiteral
            | NullLiteral
    )
}

fn is_primary(k: K) -> bool {
    use K::*;
    is_literal(k)
        || matches!(
            k,
            ClassLiteral
                | This
                | Identifier
                | ParenthesizedExpression
                | ObjectCreationExpression
                | FieldAccess
                | ArrayAccess
                | MethodInvocation
                | MethodReference
                | ArrayCreationExpression
                | TemplateExpression
        )
}

fn is_expression(k: K) -> bool {
    use K::*;
    matches!(
        k,
        AssignmentExpression
            | BinaryExpression
            | InstanceofExpression
            | LambdaExpression
            | TernaryExpression
            | UpdateExpression
            | UnaryExpression
            | CastExpression
            | SwitchExpression
    )
}

fn is_declaration(k: K) -> bool {
    use K::*;
    matches!(
        k,
        ModuleDeclaration
            | PackageDeclaration
            | ImportDeclaration
            | ClassDeclaration
            | RecordDeclaration
            | InterfaceDeclaration
            | AnnotationTypeDeclaration
            | EnumDeclaration
    )
}

fn is_statement(k: K) -> bool {
    use K::*;
    matches!(
        k,
        ExpressionStatement
            | LabeledStatement
            | IfStatement
            | WhileStatement
            | ForStatement
            | EnhancedForStatement
            | Block
            | TSemi
            | AssertStatement
            | DoStatement
            | BreakStatement
            | ContinueStatement
            | ReturnStatement
            | YieldStatement
            | SwitchExpression
            | SynchronizedStatement
            | LocalVariableDeclaration
            | ThrowStatement
            | TryStatement
            | TryWithResourcesStatement
    )
}

fn is_unannotated(k: K) -> bool {
    use K::*;
    matches!(k, VoidType | IntegralType | FloatingPointType | BooleanType | TypeIdentifier | ScopedTypeIdentifier | GenericType | ArrayType)
}

fn is_directive(k: K) -> bool {
    use K::*;
    matches!(k, RequiresModuleDirective | ExportsModuleDirective | OpensModuleDirective | UsesModuleDirective | ProvidesModuleDirective)
}

fn class(k: K, ctx: Ctx) -> u8 {
    match ctx {
        Ctx::Expr if is_literal(k) => C_LEP,
        Ctx::Expr if is_primary(k) => C_EP,
        Ctx::Expr if is_expression(k) => C_E,
        Ctx::Primary if is_literal(k) => C_LP,
        Ctx::Primary if is_primary(k) => C_P,
        Ctx::Stmt if is_declaration(k) => C_SD,
        Ctx::Stmt if is_statement(k) => C_S,
        Ctx::Type if is_unannotated(k) => C_TUT,
        Ctx::Type if k == K::AnnotatedType => C_T,
        Ctx::UType if is_unannotated(k) => C_UT,
        Ctx::Directive if is_directive(k) => C_MD,
        _ => C_NONE,
    }
}

const MAX_DEPTH: u32 = 300;

/// `kinds::DROPPED` as an open-addressed table, built at compile time: a key's slot by one
/// multiply-shift, then the next slots to an empty one (as `rerail-ecma-lean`'s).
const DROP_SLOTS: usize = 1024;
const NO_DROP: u32 = u32::MAX;

#[inline(always)]
const fn drop_slot(key: u32) -> usize {
    (key.wrapping_mul(0x9e37_79b1) >> 22) as usize
}

static DROP_TABLE: [u32; DROP_SLOTS] = {
    let mut t = [NO_DROP; DROP_SLOTS];
    let keys = crate::kinds::DROPPED;
    assert!(keys.len() * 2 < DROP_SLOTS, "DROPPED outgrew its table");
    let mut i = 0;
    while i < keys.len() {
        let mut slot = drop_slot(keys[i]);
        while t[slot] != NO_DROP {
            slot = (slot + 1) & (DROP_SLOTS - 1);
        }
        t[slot] = keys[i];
        i += 1;
    }
    t
};

/// A node written: its index and kind.
#[derive(Clone, Copy, Debug)]
struct N {
    at: usize,
    kind: K,
}

struct P<'a> {
    src: &'a [u8],
    lx: &'a Lexer<'a>,
    toks: Vec<Tok>,
    i: usize,
    /// Where the next token is lexed into the buffer.
    next: usize,
    extras: Vec<Extra>,
    e: usize,
    w: Writer<Widths>,
    last_end: u32,
    depth: u32,
    /// The furthest refusal (a look ahead's included): where a refused file is reported.
    far: u32,
    /// In a `case` label, outside brackets: `x ->` ends the label, no lambda.
    no_lambda: bool,
}

/// `src` parsed.
pub fn parse(src: &[u8]) -> R<Tape> {
    let lx = Lexer::new(src);
    let mut widths = Widths::guess(src.len());
    loop {
        let mut p = P {
            src,
            lx: &lx,
            toks: Vec::with_capacity(src.len() / 4 + 4),
            i: 0,
            next: 0,
            extras: Vec::new(),
            e: 0,
            w: Writer::new(0, widths, src.len() / 3 + 16),
            last_end: 0,
            depth: 0,
            far: 0,
            no_lambda: false,
        };
        if let Err(r) = p.program() {
            return Err(match r {
                Refusal::Parse(at) => Refusal::Parse(at.max(p.far)),
                r => r,
            });
        }
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
        DotDotDot => K::TDotDotDot,
        At => K::TAt,
        AtInterface => K::TAtInterface,
        ColonColon => K::TColonColon,
        Colon => K::TColon,
        Question => K::TQuestion,
        Arrow => K::TArrow,
        Eq => K::TEq,
        EqEq => K::TEqEq,
        Bang => K::TBang,
        Ne => K::TNe,
        Tilde => K::TTilde,
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
        Caret => K::TCaret,
        CaretEq => K::TCaretEq,
        Amp => K::TAmp,
        AmpAmp => K::TAmpAmp,
        AmpEq => K::TAmpEq,
        Pipe => K::TPipe,
        PipePipe => K::TPipePipe,
        PipeEq => K::TPipeEq,
        Lt => K::TLt,
        Le => K::TLe,
        LtLt => K::TLtLt,
        LtLtEq => K::TLtLtEq,
        Gt => K::TGt,
        Ge => K::TGe,
        GtGt => K::TGtGt,
        GtGtEq => K::TGtGtEq,
        GtGtGt => K::TGtGtGt,
        GtGtGtEq => K::TGtGtGtEq,
        Quote => K::TDQuote,
        TripleQuote => K::TTripleQuote,
        _ => return None,
    })
}

/// A keyword's word code as the tape's token kind.
fn keyword(code: u8) -> Option<K> {
    use K::*;
    Some(match code {
        w::ABSTRACT => TAbstract,
        w::ASSERT => TAssert,
        w::BREAK => TBreak,
        w::CASE => TCase,
        w::CATCH => TCatch,
        w::CLASS => TClass,
        w::CONTINUE => TContinue,
        w::DEFAULT => TDefault,
        w::DO => TDo,
        w::ELSE => TElse,
        w::ENUM => TEnum,
        w::EXPORTS => TExports,
        w::EXTENDS => TExtends,
        w::FINAL => TFinal,
        w::FINALLY => TFinally,
        w::FOR => TFor,
        w::IF => TIf,
        w::IMPLEMENTS => TImplements,
        w::IMPORT => TImport,
        w::INSTANCEOF => TInstanceof,
        w::INTERFACE => TInterface,
        w::MODULE => TModule,
        w::NATIVE => TNative,
        w::NEW => TNew,
        w::OPEN => TOpen,
        w::OPENS => TOpens,
        w::PACKAGE => TPackage,
        w::PERMITS => TPermits,
        w::PRIVATE => TPrivate,
        w::PROTECTED => TProtected,
        w::PROVIDES => TProvides,
        w::PUBLIC => TPublic,
        w::RECORD => TRecord,
        w::REQUIRES => TRequires,
        w::RETURN => TReturn,
        w::SEALED => TSealed,
        w::STATIC => TStatic,
        w::STRICTFP => TStrictfp,
        w::SWITCH => TSwitch,
        w::SYNCHRONIZED => TSynchronized,
        w::THROW => TThrow,
        w::THROWS => TThrows,
        w::TO => TTo,
        w::TRANSIENT => TTransient,
        w::TRANSITIVE => TTransitive,
        w::TRY => TTry,
        w::USES => TUses,
        w::VOLATILE => TVolatile,
        w::WHEN => TWhen,
        w::WHILE => TWhile,
        w::WITH => TWith,
        w::YIELD => TYield,
        w::BYTE => TByte,
        w::SHORT => TShort,
        w::INT => TInt,
        w::LONG => TLong,
        w::CHAR => TChar,
        w::FLOAT => TFloat,
        w::DOUBLE => TDouble,
        w::BOOLEAN => TBoolean,
        w::VOID => TVoid,
        _ => return None,
    })
}

/// Whether a word is a reserved word: tree-sitter-java's keywords, which are never an identifier
/// (the contextual words `open`, `module`, `record`, `with`, `sealed`, `yield`, `when`, `to`… are).
fn reserved(code: u8) -> bool {
    matches!(
        code,
        w::ABSTRACT
            | w::ASSERT
            | w::BREAK
            | w::CASE
            | w::CATCH
            | w::CLASS
            | w::CONTINUE
            | w::DEFAULT
            | w::DO
            | w::ELSE
            | w::ENUM
            | w::EXTENDS
            | w::FALSE
            | w::FINAL
            | w::FINALLY
            | w::FOR
            | w::IF
            | w::IMPLEMENTS
            | w::IMPORT
            | w::INSTANCEOF
            | w::INTERFACE
            | w::NATIVE
            | w::NEW
            | w::NULL
            | w::PACKAGE
            | w::PRIVATE
            | w::PROTECTED
            | w::PUBLIC
            | w::RETURN
            | w::STATIC
            | w::STRICTFP
            | w::SUPER
            | w::SWITCH
            | w::SYNCHRONIZED
            | w::THIS
            | w::THROW
            | w::THROWS
            | w::TRANSIENT
            | w::TRUE
            | w::TRY
            | w::VOLATILE
            | w::WHILE
    ) || w::is_primitive(code)
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
        self.toks.reserve(BATCH);
        for _ in 0..BATCH {
            let t = match self.lx.token(self.next, &mut self.extras) {
                Ok(t) => t,
                // A token the lexer refuses is refused where the parser reads it.
                Err(Refusal::Lex(at)) | Err(Refusal::Parse(at)) => Tok { kind: T::Eof, word: u8::MAX, start: at, end: at },
                Err(Refusal::TooLarge) => Tok { kind: T::Eof, word: u8::MAX, start: 0, end: 0 },
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

    fn peek_at(&mut self, k: usize) -> Tok {
        self.fill(k);
        self.toks[self.i + k]
    }

    fn kind_at(&mut self, k: usize) -> T {
        self.peek_at(k).kind
    }

    /// Whether the next token is the word `code`.
    #[inline(always)]
    fn is_word(&mut self, code: u8) -> bool {
        let t = self.tok();
        t.kind == T::Name && t.word == code
    }

    fn is_word_at(&mut self, k: usize, code: u8) -> bool {
        let t = self.peek_at(k);
        t.kind == T::Name && t.word == code
    }

    /// Whether token `k` ahead is a name that can be an identifier (no reserved word).
    fn is_ident_at(&mut self, k: usize) -> bool {
        let t = self.peek_at(k);
        t.kind == T::Name && !reserved(t.word)
    }

    fn is_ident(&mut self) -> bool {
        self.is_ident_at(0)
    }

    /// Whether `t` is `_` alone.
    fn underscore(&self, t: Tok) -> bool {
        t.kind == T::Name && t.end == t.start + 1 && self.src[t.start as usize] == b'_'
    }

    fn refuse<X>(&mut self) -> R<X> {
        let at = self.tok().start;
        self.far = self.far.max(at);
        Err(Refusal::Parse(at))
    }

    /// The buffer cut at the next token, and lexing resumed at `pos`. Extras from `pos` on are
    /// lexed again.
    fn relex_at(&mut self, pos: usize) {
        self.toks.truncate(self.i);
        self.next = pos;
        let keep = self.extras.partition_point(|x| (x.start as usize) < pos);
        self.extras.truncate(keep.max(self.e));
    }

    /// The token `k` ahead read as its first `len` bytes, of kind `kind` (`>` of `>>`): the rest
    /// is lexed again after it.
    fn split_at(&mut self, k: usize, kind: T, len: u32) {
        let t = self.peek_at(k);
        self.toks.truncate(self.i + k);
        let pos = (t.start + len) as usize;
        self.next = pos;
        let keep = self.extras.partition_point(|x| (x.start as usize) < pos);
        self.extras.truncate(keep.max(self.e));
        self.toks.push(Tok { kind, word: 0, start: t.start, end: t.start + len });
    }

    /// The extras before `before`, written into the node open.
    #[inline(always)]
    fn flush(&mut self, before: u32) {
        while let Some(&x) = self.extras.get(self.e) {
            if x.start >= before {
                break;
            }
            self.e += 1;
            self.extra(x);
        }
    }

    /// A comment: a line or block comment, a leaf.
    fn extra(&mut self, x: Extra) {
        let kind = if x.block { K::BlockComment } else { K::LineComment };
        let n = self.w.push(key(kind, f::NONE, C_NONE, true), x.start);
        self.w.close(n, x.end);
    }

    /// The next token read, its extras before it written.
    #[inline(always)]
    fn bump(&mut self) -> R<Tok> {
        let t = self.tok();
        if t.kind == T::Eof && t.word == u8::MAX {
            return Err(Refusal::Lex(t.start));
        }
        self.flush(t.start);
        if t.kind != T::Eof {
            self.i += 1;
        }
        self.last_end = t.end;
        Ok(t)
    }

    fn expect(&mut self, kind: T) -> R<Tok> {
        if self.peek() != kind {
            return self.refuse();
        }
        self.bump()
    }

    /// The next token as a leaf.
    #[inline(always)]
    fn leaf(&mut self, kind: K, field: u8, class: u8) -> R<N> {
        let t = self.bump()?;
        let at = self.w.push(key(kind, field, class, false), t.start);
        self.w.close(at, t.end);
        Ok(N { at, kind })
    }

    fn leaf_in(&mut self, kind: K, s: Slot) -> R<N> {
        self.leaf(kind, s.field, class(kind, s.ctx))
    }

    /// Whether the tape stores token `tok` under `parent` in `field` where its gap is its kind's
    /// one shortest word (`kinds::DROPPED`): a token not listed is stored in every gap.
    #[inline]
    fn stores(&self, parent: K, tok: K, field: u8) -> bool {
        let key = parent as u32 | (tok as u32) << 9 | u32::from(field) << 18;
        let mut slot = drop_slot(key);
        loop {
            // SAFETY: `slot` is masked to the table's slots.
            let k = unsafe { *DROP_TABLE.get_unchecked(slot) };
            if k == key {
                return false;
            }
            if k == NO_DROP {
                return true;
            }
            slot = (slot + 1) & (DROP_SLOTS - 1);
        }
    }

    /// The next token, of kind `tok`, under `parent` in an implied gap: a leaf where the tape
    /// stores it.
    #[inline]
    fn tk(&mut self, parent: K, tok: K) -> R<()> {
        self.tk_field(parent, tok, f::NONE)
    }

    fn tk_field(&mut self, parent: K, tok: K, field: u8) -> R<()> {
        if self.stores(parent, tok, field) {
            self.leaf(tok, field, C_NONE)?;
        } else {
            self.bump()?;
        }
        Ok(())
    }

    /// The next token under `parent`: stored where `keep` (its gap is not implied), else as `tk`.
    #[inline]
    fn gap_tk(&mut self, parent: K, tok: K, keep: bool) -> R<()> {
        if keep {
            self.leaf(tok, f::NONE, C_NONE)?;
            Ok(())
        } else {
            self.tk(parent, tok)
        }
    }

    /// The next token, which must be `t`, under `parent` in a gap kept where `keep`.
    fn punct_tk(&mut self, parent: K, t: T, keep: bool) -> R<()> {
        if self.peek() != t {
            return self.refuse();
        }
        let k = punct(t).expect("a punctuation token");
        self.gap_tk(parent, k, keep)
    }

    /// The next token, the keyword `code`, under `parent`.
    fn kw(&mut self, parent: K, code: u8) -> R<()> {
        if !self.is_word(code) {
            return self.refuse();
        }
        let k = keyword(code).expect("a keyword");
        self.tk(parent, k)
    }

    /// A node opened at the next token.
    #[inline(always)]
    fn open(&mut self, kind: K, field: u8, class: u8) -> N {
        let s = self.tok().start;
        self.flush(s);
        N { at: self.w.push(key(kind, field, class, false), s), kind }
    }

    #[inline(always)]
    fn open_in(&mut self, kind: K, s: Slot) -> N {
        self.open(kind, s.field, class(kind, s.ctx))
    }

    #[inline(always)]
    fn close(&mut self, n: N) {
        self.w.close(n.at, self.last_end);
    }

    /// `kind` put before `n` (wrapping it and what follows), in slot `s`; `n` moves to slot `inner`.
    fn wrap(&mut self, n: N, kind: K, s: Slot, inner: Slot) -> N {
        self.rekey(n.at, inner);
        let start = self.w.start(n.at);
        self.w.insert(n.at, key(kind, s.field, class(kind, s.ctx), false), start);
        N { at: n.at, kind }
    }

    /// The node at `at` in slot `s`.
    fn rekey(&mut self, at: usize, s: Slot) {
        let (kind, _, _, extra) = unkey(self.w.key(at));
        let k = kind_of(kind);
        self.w.set_key(at, key(k, s.field, class(k, s.ctx), extra));
    }

    /// `f` read inside brackets: a lambda may start there again.
    fn nested<X>(&mut self, f: impl FnOnce(&mut Self) -> R<X>) -> R<X> {
        let saved = std::mem::replace(&mut self.no_lambda, false);
        let r = f(self);
        self.no_lambda = saved;
        r
    }

    fn deeper(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.refuse();
        }
        Ok(())
    }

    // ---- Lists.

    /// A bracketed list under `parent` (its node open), from its opener to its closer: items read
    /// by `item`, between `,`s. A gap's delimiters are dropped where the gap is its kind's implied
    /// word: the opener before the first item, a `,` between two, the closer after the last; an
    /// empty list's where `empty` (whether `opener closer` is the kind's one shortest word). A
    /// trailing `,` (where `trailing` allows one) keeps itself and the closer.
    fn list(&mut self, parent: K, close: T, empty: bool, trailing: bool, mut item: impl FnMut(&mut Self) -> R<()>) -> R<()> {
        let open = self.tok();
        let Some(open_k) = punct(open.kind) else { return self.refuse() };
        let close_k = punct(close).expect("a closer");
        if self.kind_at(1) == close {
            self.gap_tk(parent, open_k, !empty)?;
            return self.gap_tk(parent, close_k, !empty);
        }
        if trailing && self.kind_at(1) == T::Comma && self.kind_at(2) == close {
            // `{,}`: one gap, not its kind's implied word.
            self.leaf(open_k, f::NONE, C_NONE)?;
            self.leaf(K::TComma, f::NONE, C_NONE)?;
            self.leaf(close_k, f::NONE, C_NONE)?;
            return Ok(());
        }
        self.gap_tk(parent, open_k, false)?;
        loop {
            item(self)?;
            match self.peek() {
                T::Comma => {
                    if self.kind_at(1) == close {
                        if !trailing {
                            return self.refuse();
                        }
                        self.leaf(K::TComma, f::NONE, C_NONE)?;
                        self.leaf(close_k, f::NONE, C_NONE)?;
                        return Ok(());
                    }
                    self.gap_tk(parent, K::TComma, false)?;
                }
                k if k == close => return self.gap_tk(parent, close_k, false),
                _ => return self.refuse(),
            }
        }
    }

    // ---- The file.

    fn program(&mut self) -> R<()> {
        // The root starts at its first token or comment (past the file's leading white space).
        let first = self.tok().start.min(self.extras.first().map_or(u32::MAX, |x| x.start));
        let first = if self.tok().kind == T::Eof && self.extras.is_empty() { 0 } else { first };
        let root = self.w.push(key(K::Program, f::NONE, C_NONE, false), first);
        while self.peek() != T::Eof {
            self.toplevel()?;
        }
        let t = self.tok();
        if t.word == u8::MAX {
            return Err(Refusal::Lex(t.start));
        }
        self.flush(u32::MAX);
        self.w.close(root, self.src.len() as u32);
        Ok(())
    }
}

fn kind_of(k: u16) -> K {
    debug_assert!(k <= crate::kinds::LAST as u16);
    // SAFETY: `K` is `repr(u16)` with variants 0 to `KINDS.len() - 1`, and every key's kind is one.
    unsafe { std::mem::transmute::<u16, K>(k) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hash table holds `kinds::DROPPED`'s keys, and no other key.
    #[test]
    fn the_drop_table_is_dropped() {
        let keys = crate::kinds::DROPPED;
        assert_eq!(DROP_TABLE.iter().filter(|&&k| k != NO_DROP).count(), keys.len());
        let find = |key: u32| {
            let mut slot = drop_slot(key);
            loop {
                match DROP_TABLE[slot] {
                    k if k == key => return true,
                    NO_DROP => return false,
                    _ => slot = (slot + 1) & (DROP_SLOTS - 1),
                }
            }
        };
        for key in 0..1u32 << 24 {
            assert_eq!(find(key), keys.binary_search(&key).is_ok(), "{key}");
        }
    }
}
