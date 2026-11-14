//! The parser: recursive descent over tree-sitter-c-sharp's grammar, writing the tape as it goes.
//!
//! **Tokens** are lexed into a buffer as the parser looks ahead, 16 at a time. Where the parse
//! state reads a token shorter than the one lexed (a `>>` closing two type argument lists), the
//! buffer is cut at that byte and lexed again from there (`relex_at`, `split_at`).
//!
//! **Where tree-sitter's parse keeps two readings** (GLR) and a later token picks one, the parser
//! reads ahead at that token instead: a statement is a declaration where a type and a name start
//! it; `(` starts a cast where a type and `)` follow it and an operand comes after; a name before
//! `<` is generic where the angle brackets close before a token that follows a type.
//!
//! **Stored tokens.** A token other than a delimiter is stored unless its kind implies it
//! (`kinds::DROPPED`, `stores`). A delimiter its kind implies is still stored where its gap (the
//! tokens between two named children) is not the gap's one shortest word: a trailing comma, an
//! empty statement among a block's. Each list's caller reads the gap ahead and decides before it
//! writes.

use crate::kinds::{K, f};
use crate::lex::{Extra, Lexer, R, Refusal, T, Tok, X, w};
use crate::{Tape, key, unkey};
use rerail_tape_format::{Redo, Widths, Writer};

mod decl;
mod expr;
mod pattern;
mod stmt;
mod types;

// Supertype sets (`crate::CLASSES`).
const C_NONE: u8 = 0;
const C_EL: u8 = 1;
const C_EN: u8 = 2;
const C_ENL: u8 = 3;
const C_L: u8 = 4;
const C_N: u8 = 5;
const C_NL: u8 = 6;
const C_LIT: u8 = 7;
const C_TY: u8 = 8;
const C_S: u8 = 9;
const C_D: u8 = 10;
const C_PAT: u8 = 11;
const C_TD: u8 = 12;

/// The context a node stands in: which supertypes its kind is in there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctx {
    /// An `expression`: an lvalue in `lvalue_expression` too, else in `non_lvalue_expression` (a
    /// literal in `literal` too).
    Expr,
    /// An `lvalue_expression`.
    Lv,
    /// A parenthesized expression's: `lvalue_expression` or `non_lvalue_expression`, by its kind.
    Paren,
    /// A `literal` (in a constant pattern).
    Lit,
    /// A `type`.
    Type,
    /// A `statement`.
    Stmt,
    /// A `declaration`.
    Decl,
    /// A `type_declaration`.
    TypeDecl,
    /// A `pattern`.
    Pattern,
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
const DECL: Slot = slot(f::NONE, Ctx::Decl);
const PATTERN: Slot = slot(f::NONE, Ctx::Pattern);
const BARE: Slot = slot(f::NONE, Ctx::Bare);

fn is_literal(k: K) -> bool {
    use K::*;
    matches!(k, NullLiteral | CharacterLiteral | IntegerLiteral | RealLiteral | BooleanLiteral | StringLiteral | VerbatimStringLiteral | RawStringLiteral)
}

/// Whether `k` is in `lvalue_expression` (a pointer indirection and a parenthesized lvalue are
/// keyed by their callers, `C_L` or `C_EL` by hand).
fn is_lvalue(k: K) -> bool {
    use K::*;
    matches!(k, TThis | MemberAccessExpression | TupleExpression | Identifier | GenericName | ElementAccessExpression | ElementBindingExpression)
}

fn is_non_lvalue(k: K) -> bool {
    use K::*;
    is_literal(k)
        || matches!(
            k,
            TBase
                | BinaryExpression
                | InterpolatedStringExpression
                | ConditionalExpression
                | ConditionalAccessExpression
                | AssignmentExpression
                | InvocationExpression
                | PostfixUnaryExpression
                | PrefixUnaryExpression
                | AwaitExpression
                | ObjectCreationExpression
                | ParenthesizedExpression
                | IsExpression
                | IsPatternExpression
                | AsExpression
                | CastExpression
                | CheckedExpression
                | CollectionExpression
                | SwitchExpression
                | ThrowExpression
                | DefaultExpression
                | LambdaExpression
                | WithExpression
                | SizeofExpression
                | TypeofExpression
                | MakerefExpression
                | RefExpression
                | ReftypeExpression
                | RefvalueExpression
                | StackallocExpression
                | RangeExpression
                | ArrayCreationExpression
                | AnonymousMethodExpression
                | AnonymousObjectCreationExpression
                | ImplicitArrayCreationExpression
                | ImplicitObjectCreationExpression
                | ImplicitStackallocExpression
                | InitializerExpression
                | QueryExpression
                | PreprocIf
        )
}

fn is_type(k: K) -> bool {
    use K::*;
    matches!(
        k,
        ImplicitType | ArrayType | Identifier | GenericName | QualifiedName | AliasQualifiedName | NullableType | PointerType | FunctionPointerType | PredefinedType | TupleType | RefType | ScopedType
    )
}

fn is_statement(k: K) -> bool {
    use K::*;
    matches!(
        k,
        Block
            | BreakStatement
            | CheckedStatement
            | ContinueStatement
            | DoStatement
            | EmptyStatement
            | ExpressionStatement
            | FixedStatement
            | ForStatement
            | ReturnStatement
            | LockStatement
            | YieldStatement
            | SwitchStatement
            | ThrowStatement
            | TryStatement
            | UnsafeStatement
            | UsingStatement
            | ForeachStatement
            | GotoStatement
            | LabeledStatement
            | IfStatement
            | WhileStatement
            | LocalDeclarationStatement
            | LocalFunctionStatement
            | PreprocIf
    )
}

fn is_type_declaration(k: K) -> bool {
    use K::*;
    matches!(k, ClassDeclaration | StructDeclaration | EnumDeclaration | InterfaceDeclaration | DelegateDeclaration | RecordDeclaration)
}

fn is_declaration(k: K) -> bool {
    use K::*;
    is_type_declaration(k)
        || matches!(
            k,
            FieldDeclaration
                | MethodDeclaration
                | EventDeclaration
                | EventFieldDeclaration
                | ConstructorDeclaration
                | DestructorDeclaration
                | IndexerDeclaration
                | NamespaceDeclaration
                | OperatorDeclaration
                | ConversionOperatorDeclaration
                | PropertyDeclaration
                | UsingDirective
                | PreprocIf
        )
}

fn is_pattern(k: K) -> bool {
    use K::*;
    matches!(
        k,
        ConstantPattern | DeclarationPattern | Discard | RecursivePattern | VarPattern | NegatedPattern | ParenthesizedPattern | RelationalPattern | OrPattern | AndPattern | ListPattern | TypePattern
    )
}

fn class(k: K, ctx: Ctx) -> u8 {
    match ctx {
        Ctx::Expr if is_lvalue(k) => C_EL,
        Ctx::Expr if is_literal(k) => C_ENL,
        Ctx::Expr if is_non_lvalue(k) => C_EN,
        Ctx::Lv if is_lvalue(k) => C_L,
        Ctx::Paren if is_lvalue(k) => C_L,
        Ctx::Paren if is_literal(k) => C_NL,
        Ctx::Paren if is_non_lvalue(k) => C_N,
        Ctx::Lit if is_literal(k) => C_LIT,
        Ctx::Type if is_type(k) => C_TY,
        Ctx::Stmt if is_statement(k) => C_S,
        Ctx::Decl if is_declaration(k) => C_D,
        Ctx::TypeDecl if is_type_declaration(k) => C_TD,
        Ctx::Pattern if is_pattern(k) => C_PAT,
        _ => C_NONE,
    }
}

/// The class of an lvalue its kind does not say (`*p`, `(x)`) in slot context `ctx`.
fn lvalue_class(ctx: Ctx) -> u8 {
    match ctx {
        Ctx::Expr => C_EL,
        Ctx::Lv | Ctx::Paren => C_L,
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
    /// In an argument: a generic name before a name is a declaration's type.
    in_argument: bool,
    /// In a block (not a declaration list): `new` starts an expression, not a modifier.
    in_statement: bool,
    /// Reading a constant pattern that took the operators after its `is` (`x is null || y is
    /// null`): an `is` in it takes none.
    in_constant: bool,
    /// In the parameters of a lambda with no modifiers: `ref T` is a `ref_type`.
    bare_lambda: bool,
    /// How the next constant pattern reads (set by `is`).
    cmode: Cmode,
    /// The tokens dropped since the last node written (`tk`): their token index, parent, kind,
    /// field and span, and the writer's length then.
    dropped: Vec<Dropped>,
    /// After a bare `this` or `base` (the writer's length and the closes then): the tokens up to
    /// its parent's next named child are stored.
    bare_after: Option<(usize, u32)>,
    closes: u32,
}

/// How a constant pattern reads the operators after it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cmode {
    /// All of precedence `??` and above (a `case` label, a switch arm).
    Plain,
    /// After an `is` outside a constant (an operand of an operator of precedence `min`): all of
    /// `??` and above, where tree-sitter's reading is known.
    Start(u8),
    /// After an `is` inside a chain (an operand of an operator of precedence `min`): none; an
    /// operator of `min` or above after it is refused.
    Inner(u8),
}

#[derive(Clone, Copy)]
struct Dropped {
    tok: usize,
    parent: K,
    kind: K,
    field: u8,
    start: u32,
    end: u32,
    len: usize,
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
            in_argument: false,
            in_statement: false,
            in_constant: false,
            bare_lambda: false,
            cmode: Cmode::Plain,
            dropped: Vec::new(),
            bare_after: None,
            closes: 0,
        };
        if let Err(r) = p.compilation_unit() {
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
        DotDot => K::TDotDot,
        ColonColon => K::TColonColon,
        Colon => K::TColon,
        Question => K::TQuestion,
        QQ => K::TQQ,
        QQEq => K::TQQEq,
        FatArrow => K::TFatArrow,
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
        PpIf => K::THashIf,
        PpElif => K::THashElif,
        PpElse => K::THashElse,
        PpEndif => K::THashEndif,
        _ => return None,
    })
}

/// A keyword's word code as the tape's token kind.
fn keyword(code: u8) -> Option<K> {
    use K::*;
    Some(match code {
        w::ABSTRACT => TAbstract,
        w::AS => TAs,
        w::BASE => TBase,
        w::BREAK => TBreak,
        w::CASE => TCase,
        w::CATCH => TCatch,
        w::CHECKED => TChecked,
        w::CLASS => TClass,
        w::CONST => TConst,
        w::CONTINUE => TContinue,
        w::DEFAULT => TDefault,
        w::DELEGATE => TDelegate,
        w::DO => TDo,
        w::ELSE => TElse,
        w::ENUM => TEnum,
        w::EVENT => TEvent,
        w::EXPLICIT => TExplicit,
        w::EXTERN => TExtern,
        w::FALSE => TFalse,
        w::FINALLY => TFinally,
        w::FIXED => TFixed,
        w::FOR => TFor,
        w::FOREACH => TForeach,
        w::GOTO => TGoto,
        w::IF => TIf,
        w::IMPLICIT => TImplicit,
        w::IN => TIn,
        w::INTERFACE => TInterface,
        w::INTERNAL => TInternal,
        w::IS => TIs,
        w::LOCK => TLock,
        w::NAMESPACE => TNamespace,
        w::NEW => TNew,
        w::NULL => TNull,
        w::OPERATOR => TOperator,
        w::OUT => TOut,
        w::OVERRIDE => TOverride,
        w::PARAMS => TParams,
        w::PRIVATE => TPrivate,
        w::PROTECTED => TProtected,
        w::PUBLIC => TPublic,
        w::READONLY => TReadonly,
        w::REF => TRef,
        w::RETURN => TReturn,
        w::SEALED => TSealed,
        w::SIZEOF => TSizeof,
        w::STACKALLOC => TStackalloc,
        w::STATIC => TStatic,
        w::STRUCT => TStruct,
        w::SWITCH => TSwitch,
        w::THIS => TThis,
        w::THROW => TThrow,
        w::TRUE => TTrue,
        w::TRY => TTry,
        w::TYPEOF => TTypeof,
        w::UNCHECKED => TUnchecked,
        w::UNSAFE => TUnsafe,
        w::USING => TUsing,
        w::VIRTUAL => TVirtual,
        w::VOLATILE => TVolatile,
        w::WHILE => TWhile,
        w::ALIAS => TAlias,
        w::ASCENDING => TAscending,
        w::BY => TBy,
        w::DESCENDING => TDescending,
        w::EQUALS => TEquals,
        w::FILE => TFile,
        w::FROM => TFrom,
        w::GLOBAL => TGlobal,
        w::GROUP => TGroup,
        w::INTO => TInto,
        w::JOIN => TJoin,
        w::LET => TLet,
        w::NOTNULL => TNotnull,
        w::ON => TOn,
        w::ORDERBY => TOrderby,
        w::SCOPED => TScoped,
        w::SELECT => TSelect,
        w::UNMANAGED => TUnmanaged,
        w::VAR => TVar,
        w::WHEN => TWhen,
        w::WHERE => TWhere,
        w::YIELD => TYield,
        w::ADD => TAdd,
        w::AND => TAnd,
        w::ASSEMBLY => TAssembly,
        w::ASYNC => TAsync,
        w::AWAIT => TAwait,
        w::FIELD => TField,
        w::GET => TGet,
        w::INIT => TInit,
        w::MANAGED => TManaged,
        w::METHOD => TMethod,
        w::MODULE => TModule,
        w::NOT => TNot,
        w::OR => TOr,
        w::PARAM => TParam,
        w::PARTIAL => TPartial,
        w::PROPERTY => TProperty,
        w::RECORD => TRecord,
        w::REMOVE => TRemove,
        w::REQUIRED => TRequired,
        w::SET => TSet,
        w::TYPE => TType,
        w::TYPEVAR => TTypevar,
        w::WITH => TWith,
        w::UNDERSCORE => TUnderscore,
        w::MAKEREF => TMakeref,
        w::REFTYPE => TReftype,
        w::REFVALUE => TRefvalue,
        w::CDECL => TCdeclCc,
        w::STDCALL => TStdcallCc,
        w::THISCALL => TThiscallCc,
        w::FASTCALL => TFastcallCc,
        w::WARNING => TWarning,
        w::DISABLE => TDisable,
        w::RESTORE => TRestore,
        w::CHECKSUM => TChecksum,
        w::ENABLE => TEnable,
        w::ANNOTATIONS => TAnnotations,
        w::WARNINGS => TWarnings,
        w::HIDDEN => THidden,
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

    fn word_at(&mut self, k: usize) -> u8 {
        let t = self.peek_at(k);
        if t.kind == T::Name { t.word } else { u8::MAX }
    }

    /// Whether token `k` ahead is a name that can be an identifier (no reserved keyword).
    fn is_ident_at(&mut self, k: usize) -> bool {
        let t = self.peek_at(k);
        t.kind == T::Name && !w::is_reserved(t.word)
    }

    fn is_ident(&mut self) -> bool {
        self.is_ident_at(0)
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
    fn flush(&mut self, before: u32) -> R<()> {
        while let Some(&x) = self.extras.get(self.e) {
            if x.start >= before {
                break;
            }
            self.e += 1;
            self.extra(x)?;
        }
        Ok(())
    }

    /// A comment (a leaf), or a directive line with its pieces.
    fn extra(&mut self, x: Extra) -> R<()> {
        if x.kind == X::Comment {
            let n = self.w.push(key(K::Comment, f::NONE, C_NONE, true), x.start);
            self.w.close(n, x.end);
            return Ok(());
        }
        self.directive(x)
    }

    /// The next token read, its extras before it written.
    #[inline(always)]
    fn bump(&mut self) -> R<Tok> {
        let t = self.tok();
        if t.kind == T::Eof && t.word == u8::MAX {
            return Err(Refusal::Lex(t.start));
        }
        self.flush(t.start)?;
        if t.kind != T::Eof {
            self.i += 1;
        }
        self.last_end = t.end;
        Ok(t)
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
        let len = self.w.len();
        let after = self.bare_after == Some((len, self.closes));
        if after || self.stores(parent, tok, field) {
            self.leaf(tok, field, C_NONE)?;
            if after {
                self.bare_after = Some((len + 1, self.closes));
            }
        } else {
            if self.dropped.last().is_some_and(|d| d.len != len) {
                self.dropped.clear();
            }
            let i = self.i;
            let t = self.bump()?;
            self.dropped.push(Dropped { tok: i, parent, kind: tok, field, start: t.start, end: t.end, len });
        }
        Ok(())
    }

    /// `n`, an expression final in its parent's slot: where it is a bare `this` or `base` (a
    /// token, to the gap rule), its gap's tokens are stored, those dropped before it and those
    /// up to the next named child.
    fn bare(&mut self, n: N) -> R<()> {
        if !matches!(n.kind, K::TThis | K::TBase) || self.w.len() != n.at + 1 || self.i == 0 {
            return Ok(());
        }
        let this = self.i - 1;
        if self.toks.get(this).is_none_or(|t| t.end != self.w.end(n.at)) {
            return Ok(());
        }
        self.bare_after = Some((n.at + 1, self.closes));
        let Some(parent) = self.dropped.iter().find(|d| d.tok + 1 == this && d.len == n.at).map(|d| d.parent) else {
            return Ok(());
        };
        let key = self.w.key(n.at);
        let (s, e) = (self.w.start(n.at), self.w.end(n.at));
        self.w.truncate(n.at);
        for d in std::mem::take(&mut self.dropped) {
            if d.len == n.at && d.parent == parent {
                self.piece(d.kind, d.field, d.start as usize, d.end as usize);
            }
        }
        let at = self.w.push(key, s);
        self.w.close(at, e);
        self.bare_after = Some((at + 1, self.closes));
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
    fn open(&mut self, kind: K, field: u8, class: u8) -> R<N> {
        let s = self.tok().start;
        self.flush(s)?;
        Ok(N { at: self.w.push(key(kind, field, class, false), s), kind })
    }

    #[inline(always)]
    fn open_in(&mut self, kind: K, s: Slot) -> R<N> {
        self.open(kind, s.field, class(kind, s.ctx))
    }

    #[inline(always)]
    fn close(&mut self, n: N) {
        self.closes += 1;
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
        let (kind, _, old, extra) = unkey(self.w.key(at));
        let k = kind_of(kind);
        // A kind whose class its node says (`*p`, `(x)`) keeps it, moved to the new context.
        let c = if matches!(k, K::PrefixUnaryExpression | K::ParenthesizedExpression) && matches!(old, C_EL | C_L) {
            lvalue_class(s.ctx)
        } else {
            class(k, s.ctx)
        };
        self.w.set_key(at, key(k, s.field, c, extra));
    }

    fn deeper(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.refuse();
        }
        Ok(())
    }

    /// A node over `start..end` with no children (a string's piece, a directive's word).
    fn piece(&mut self, kind: K, field: u8, start: usize, end: usize) {
        let at = self.w.push(key(kind, field, C_NONE, false), start as u32);
        self.w.close(at, end as u32);
    }

    /// A node opened at byte `start` (inside a string or a directive).
    fn open_at(&mut self, kind: K, field: u8, class: u8, start: u32) -> N {
        N { at: self.w.push(key(kind, field, class, false), start), kind }
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

    // ---- Names.

    /// An identifier in slot `s`: a reserved identifier's word (`var`, `global`…) is its token's
    /// node.
    fn ident(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        if t.kind != T::Name || w::is_reserved(t.word) {
            return self.refuse();
        }
        if w::is_reserved_identifier(t.word) {
            let n = self.open_in(K::Identifier, s)?;
            self.leaf(keyword(t.word).expect("a reserved identifier"), f::NONE, C_NONE)?;
            self.close(n);
            return Ok(n);
        }
        self.leaf_in(K::Identifier, s)
    }

    // ---- Directives.

    /// A directive line (an extra) and its pieces, read from its bytes.
    fn directive(&mut self, x: Extra) -> R<()> {
        let kind = match x.kind {
            X::Region => K::PreprocRegion,
            X::Endregion => K::PreprocEndregion,
            X::Pragma => K::PreprocPragma,
            X::Nullable => K::PreprocNullable,
            X::Define => K::PreprocDefine,
            X::Undef => K::PreprocUndef,
            X::Error => K::PreprocError,
            X::Warning => K::PreprocWarning,
            X::Line => K::PreprocLine,
            X::Comment => unreachable!("a comment"),
        };
        let word = match x.kind {
            X::Region => K::THashRegion,
            X::Endregion => K::THashEndregion,
            X::Pragma => K::THashPragma,
            X::Nullable => K::THashNullable,
            X::Define => K::THashDefine,
            X::Undef => K::THashUndef,
            X::Error => K::THashError,
            X::Warning => K::THashWarning,
            X::Line => K::THashLine,
            X::Comment => unreachable!("a comment"),
        };
        let src = self.src;
        // The line's `\n` (the directive's end also holds the line ends after it).
        let we = x.word_end as usize;
        let end = we + self.src[we..].iter().position(|&c| c == b'\n').unwrap_or(self.src.len() - we);
        let start = x.start as usize;
        let n = self.w.push(key(kind, f::NONE, C_NONE, true), x.start);
        if self.stores(kind, word, f::NONE) {
            self.piece(word, f::NONE, start, x.word_end as usize);
        }
        let blank = |c: u8| matches!(c, b' ' | b'\t' | b'\r' | 0x0b | 0x0c);
        let mut at = x.word_end as usize;
        let skip = |mut at: usize| {
            while at < end && blank(src[at]) {
                at += 1;
            }
            at
        };
        let refuse = Err(Refusal::Lex(x.start));
        match x.kind {
            X::Region | X::Endregion | X::Define | X::Undef | X::Error | X::Warning => {
                // `preproc_arg`: `\S([^/\n]|\/[^*]|\\\r?\n)*`, to the line's end.
                let a = skip(at);
                if a < end && src[a] == b'/' && src.get(a + 1) == Some(&b'/') && matches!(x.kind, X::Region | X::Endregion) {
                    // `#endregion // …`: a comment (an extra inside the directive), no text.
                    let e = self.lx.blocks.eol(a);
                    let c = self.w.push(key(K::Comment, f::NONE, C_NONE, true), a as u32);
                    self.w.close(c, e as u32);
                } else if a < end {
                    if a == at && !matches!(x.kind, X::Region | X::Endregion) {
                        return refuse;
                    }
                    let line = &src[a..end];
                    // A `\` before the line end continues it; not read.
                    let continued = line.ends_with(b"\\") || line.ends_with(b"\\\r");
                    if line.windows(2).any(|p| p == b"/*") || line.last() == Some(&b'/') || continued {
                        return refuse;
                    }
                    let field = if matches!(x.kind, X::Region | X::Endregion) { f::CONTENT } else { f::NONE };
                    self.piece(K::PreprocArg, field, a, end);
                } else if !matches!(x.kind, X::Region | X::Endregion) {
                    return refuse;
                }
            }
            X::Pragma | X::Nullable | X::Line => {
                // Words, names and numbers, between `,`s for a pragma's list.
                let mut words = 0;
                loop {
                    at = skip(at);
                    if at >= end {
                        break;
                    }
                    let c = src[at];
                    if c == b'/' && src.get(at + 1) == Some(&b'/') {
                        // A comment to the line's end: an extra inside the directive.
                        let e = self.lx.blocks.eol(at);
                        let c = self.w.push(key(K::Comment, f::NONE, C_NONE, true), at as u32);
                        self.w.close(c, e as u32);
                        at = e;
                        continue;
                    }
                    if c == b',' {
                        if x.kind != X::Pragma || words < 3 {
                            return refuse;
                        }
                        if self.stores(kind, K::TComma, f::NONE) {
                            self.piece(K::TComma, f::NONE, at, at + 1);
                        }
                        at += 1;
                        continue;
                    }
                    let mut e = at;
                    while e < end && (src[e].is_ascii_alphanumeric() || src[e] == b'_') {
                        e += 1;
                    }
                    if e == at {
                        return refuse;
                    }
                    let code = crate::lex::word(src, at, e);
                    let k = match (x.kind, words, code) {
                        (X::Pragma, 0, w::WARNING) => K::TWarning,
                        (X::Pragma, 1, w::DISABLE) => K::TDisable,
                        (X::Pragma, 1, w::RESTORE) => K::TRestore,
                        (X::Nullable, 0, w::ENABLE) => K::TEnable,
                        (X::Nullable, 0, w::DISABLE) => K::TDisable,
                        (X::Nullable, 0, w::RESTORE) => K::TRestore,
                        (X::Nullable, 1, w::ANNOTATIONS) => K::TAnnotations,
                        (X::Nullable, 1, w::WARNINGS) => K::TWarnings,
                        (X::Line, 0, w::DEFAULT) => K::TDefault,
                        (X::Line, 0, w::HIDDEN) => K::THidden,
                        (X::Pragma, 2.., _) if src[at].is_ascii_digit() => K::IntegerLiteral,
                        (X::Pragma, 2.., _) if !src[at].is_ascii_digit() => K::Identifier,
                        (X::Line, 0, _) if src[at..e].iter().all(u8::is_ascii_digit) => K::IntegerLiteral,
                        _ => return refuse,
                    };
                    if !crate::KINDS[k as usize].1 && !self.stores(kind, k, f::NONE) {
                        // A word the kind implies.
                    } else {
                        self.piece(k, f::NONE, at, e);
                    }
                    words += 1;
                    at = e;
                }
                if (x.kind == X::Pragma && words < 2) || (x.kind == X::Nullable && words == 0) || (x.kind == X::Line && words != 1) {
                    return refuse;
                }
            }
            X::Comment => unreachable!("a comment"),
        }
        self.w.close(n, x.end);
        Ok(())
    }

    // ---- The file.

    fn compilation_unit(&mut self) -> R<()> {
        // The root starts at its first token or extra (past the file's leading white space).
        let first = self.tok().start.min(self.extras.first().map_or(u32::MAX, |x| x.start));
        let first = if self.tok().kind == T::Eof && self.extras.is_empty() { 0 } else { first };
        let root = self.w.push(key(K::CompilationUnit, f::NONE, C_NONE, false), first);
        while self.peek() != T::Eof {
            self.toplevel()?;
        }
        let t = self.tok();
        if t.word == u8::MAX {
            return Err(Refusal::Lex(t.start));
        }
        self.flush(u32::MAX)?;
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
