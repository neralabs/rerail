//! The parser: recursive descent over tree-sitter-php's grammar (`php_only`), writing the tape as
//! it goes.
//!
//! **Tokens** are lexed into a buffer as the parser looks ahead, 16 at a time. Where the parse
//! state reads a token the lexer lexed otherwise (a string's pieces, an interpolation's
//! expression), the buffer is cut at that byte and lexed again from there (`relex_at`).
//!
//! **Where tree-sitter's parse keeps two readings** (GLR) and a later token picks one, the parser
//! reads ahead at that token instead: `[` starts a list where its brackets close before `=`; `(`
//! starts a cast where a cast's type word and `)` follow it.
//!
//! **Stored tokens.** A token other than a delimiter is stored unless its kind implies it
//! (`kinds::DROPPED`, `stores`). A delimiter its kind implies is still stored where its gap (the
//! tokens between two named children) is not the gap's one shortest word: a trailing comma, an
//! empty list. Each list's caller reads the gap ahead and decides before it writes.

use crate::kinds::{K, f};
use crate::lex::{Extra, Lexer, R, Refusal, T, Tok, w};
use crate::{Tape, key, unkey};
use rerail_tape_format::{Redo, Widths, Writer};

mod decl;
mod expr;
mod stmt;
mod string;
mod types;

// Supertype sets (`crate::CLASSES`).
const C_NONE: u8 = 0;
const C_LEP: u8 = 1;
const C_EP: u8 = 2;
const C_E: u8 = 3;
const C_LP: u8 = 4;
const C_P: u8 = 5;
const C_L: u8 = 6;
const C_T: u8 = 7;
const C_S: u8 = 8;

/// The context a node stands in: which supertypes its kind is in there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctx {
    /// An `expression`: a `primary_expression`'s kind in `primary_expression` too (a literal in
    /// `literal` too).
    Expr,
    /// A `_unary_expression` (a cast's value, `instanceof`'s left): a `primary_expression`'s kind
    /// in `primary_expression` (a literal in `literal` too); the others in none.
    Unary,
    /// A `primary_expression` (a `clone`'s operand).
    Primary,
    /// A `literal` (a `declare` directive's value).
    Lit,
    /// A `type`.
    Type,
    /// A `statement`.
    Stmt,
    /// None: a kind named directly by its parent's rule (or through hidden rules alone).
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
const BARE: Slot = slot(f::NONE, Ctx::Bare);

fn is_literal(k: K) -> bool {
    use K::*;
    matches!(k, Integer | Float | EncapsedString | String | Heredoc | Nowdoc | Boolean | Null)
}

/// Whether `k` is a `primary_expression`'s (a cast through `_variable` is keyed by its caller).
fn is_primary(k: K) -> bool {
    use K::*;
    is_literal(k)
        || matches!(
            k,
            DynamicVariableName
                | VariableName
                | SubscriptExpression
                | MemberAccessExpression
                | NullsafeMemberAccessExpression
                | ScopedPropertyAccessExpression
                | MemberCallExpression
                | NullsafeMemberCallExpression
                | FunctionCallExpression
                | ScopedCallExpression
                | ClassConstantAccessExpression
                | QualifiedName
                | RelativeName
                | Name
                | ArrayCreationExpression
                | PrintIntrinsic
                | AnonymousFunction
                | ArrowFunction
                | ObjectCreationExpression
                | UpdateExpression
                | ShellCommandExpression
                | ParenthesizedExpression
                | ThrowExpression
        )
}

/// Whether `k` is an `expression`'s that is no `primary_expression`'s.
fn is_expression(k: K) -> bool {
    use K::*;
    matches!(
        k,
        ConditionalExpression
            | MatchExpression
            | AugmentedAssignmentExpression
            | AssignmentExpression
            | ReferenceAssignmentExpression
            | YieldExpression
            | CloneExpression
            | UnaryOpExpression
            | CastExpression
            | ErrorSuppressionExpression
            | BinaryExpression
            | IncludeExpression
            | IncludeOnceExpression
            | RequireExpression
            | RequireOnceExpression
    )
}

/// Whether `k` is a `_unary_expression`'s that is no `primary_expression`'s.
fn is_unary(k: K) -> bool {
    matches!(k, K::CloneExpression | K::UnaryOpExpression | K::CastExpression)
}

fn is_type(k: K) -> bool {
    use K::*;
    matches!(k, NamedType | OptionalType | PrimitiveType | UnionType | IntersectionType | DisjunctiveNormalFormType)
}

fn is_statement(k: K) -> bool {
    use K::*;
    matches!(
        k,
        EmptyStatement
            | CompoundStatement
            | NamedLabelStatement
            | ExpressionStatement
            | IfStatement
            | SwitchStatement
            | WhileStatement
            | DoStatement
            | ForStatement
            | ForeachStatement
            | GotoStatement
            | ContinueStatement
            | BreakStatement
            | ReturnStatement
            | TryStatement
            | DeclareStatement
            | EchoStatement
            | ExitStatement
            | UnsetStatement
            | ConstDeclaration
            | FunctionDefinition
            | ClassDeclaration
            | InterfaceDeclaration
            | TraitDeclaration
            | EnumDeclaration
            | NamespaceDefinition
            | NamespaceUseDeclaration
            | GlobalDeclaration
            | FunctionStaticDeclaration
    )
}

fn class(k: K, ctx: Ctx) -> u8 {
    match ctx {
        Ctx::Expr if is_literal(k) => C_LEP,
        Ctx::Expr if is_primary(k) => C_EP,
        Ctx::Expr if is_expression(k) => C_E,
        Ctx::Unary | Ctx::Primary if is_literal(k) => C_LP,
        Ctx::Unary | Ctx::Primary if is_primary(k) => C_P,
        Ctx::Lit if is_literal(k) => C_L,
        Ctx::Type if is_type(k) => C_T,
        Ctx::Stmt if is_statement(k) => C_S,
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
        Dollar => K::TDollar,
        LParen => K::TLParen,
        RParen => K::TRParen,
        LBracket => K::TLBracket,
        RBracket => K::TRBracket,
        LBrace => K::TLBrace,
        RBrace => K::TRBrace,
        Semi => K::TSemi,
        Comma => K::TComma,
        Dot => K::TDot,
        DotEq => K::TDotEq,
        DotDotDot => K::TDotDotDot,
        Colon => K::TColon,
        ColonColon => K::TColonColon,
        Question => K::TQuestion,
        QQ => K::TQQ,
        QQEq => K::TQQEq,
        Arrow => K::TArrow,
        QArrow => K::TQArrow,
        FatArrow => K::TFatArrow,
        Eq => K::TEq,
        EqEq => K::TEqEq,
        EqEqEq => K::TEqEqEq,
        Bang => K::TBang,
        Ne => K::TNe,
        NeEq => K::TNeEq,
        LtGt => K::TLtGt,
        Tilde => K::TTilde,
        Plus => K::TPlus,
        PlusPlus => K::TPlusPlus,
        PlusEq => K::TPlusEq,
        Minus => K::TMinus,
        MinusMinus => K::TMinusMinus,
        MinusEq => K::TMinusEq,
        Star => K::TStar,
        StarStar => K::TStarStar,
        StarEq => K::TStarEq,
        StarStarEq => K::TStarStarEq,
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
        PipeGt => K::TPipeGt,
        Lt => K::TLt,
        Le => K::TLe,
        LtLt => K::TLtLt,
        LtLtEq => K::TLtLtEq,
        Spaceship => K::TSpaceship,
        Gt => K::TGt,
        Ge => K::TGe,
        GtGt => K::TGtGt,
        GtGtEq => K::TGtGtEq,
        At => K::TAt,
        Backslash => K::TBackslash,
        HashLBracket => K::THashLBracket,
        CloseTag => K::TQGt,
        _ => return None,
    })
}

/// A keyword's word code as the tape's token kind.
fn keyword(code: u8) -> Option<K> {
    use K::*;
    Some(match code {
        w::ABSTRACT => TAbstract,
        w::AND => TAnd,
        w::AS => TAs,
        w::BREAK => TBreak,
        w::CASE => TCase,
        w::CATCH => TCatch,
        w::CLASS => TClass,
        w::CLONE => TClone,
        w::CONST => TConst,
        w::CONTINUE => TContinue,
        w::DECLARE => TDeclare,
        w::DEFAULT => TDefault,
        w::DO => TDo,
        w::ECHO => TEcho,
        w::ELSE => TElse,
        w::ELSEIF => TElseif,
        w::ENDDECLARE => TEnddeclare,
        w::ENDFOR => TEndfor,
        w::ENDFOREACH => TEndforeach,
        w::ENDIF => TEndif,
        w::ENDSWITCH => TEndswitch,
        w::ENDWHILE => TEndwhile,
        w::EXTENDS => TExtends,
        w::FINAL => TFinal,
        w::FINALLY => TFinally,
        w::FN => TFn,
        w::FOR => TFor,
        w::FOREACH => TForeach,
        w::FUNCTION => TFunction,
        w::GLOBAL => TGlobal,
        w::GOTO => TGoto,
        w::IF => TIf,
        w::IMPLEMENTS => TImplements,
        w::INCLUDE => TInclude,
        w::INCLUDE_ONCE => TIncludeOnce,
        w::INSTANCEOF => TInstanceof,
        w::INSTEADOF => TInsteadof,
        w::INTERFACE => TInterface,
        w::MATCH => TMatch,
        w::NAMESPACE => TNamespace,
        w::NEW => TNew,
        w::OR => TOr,
        w::PRINT => TPrint,
        w::PRIVATE => TPrivate,
        w::PROTECTED => TProtected,
        w::PUBLIC => TPublic,
        w::READONLY => TReadonly,
        w::REQUIRE => TRequire,
        w::REQUIRE_ONCE => TRequireOnce,
        w::RETURN => TReturn,
        w::STATIC => TStatic,
        w::SWITCH => TSwitch,
        w::THROW => TThrow,
        w::TRAIT => TTrait,
        w::TRY => TTry,
        w::USE => TUse,
        w::WHILE => TWhile,
        w::XOR => TXor,
        w::YIELD => TYield,
        w::ARRAY => TArray,
        w::LIST => TList,
        w::SELF => TSelf,
        w::PARENT => TParent,
        w::EXIT => TExit,
        w::UNSET => TUnset,
        w::ENUM => TEnum,
        w::TICKS => TTicks,
        w::ENCODING => TEncoding,
        w::STRICT_TYPES => TStrictTypes,
        w::BOOL => TBool,
        w::FLOAT => TFloat,
        w::INT => TInt,
        w::NULL => TNull,
        w::OBJECT => TObject,
        w::STRING => TString,
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

    /// Whether the next token is the word `code` (in any case).
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

    /// Whether token `k` ahead is the word `code` in lower case (a case-sensitive string of the
    /// grammar: `'array'` in `primitive_type`).
    fn is_exact_at(&mut self, k: usize, code: u8) -> bool {
        let t = self.peek_at(k);
        t.kind == T::Name && t.word == code && self.src[t.start as usize..t.end as usize] == *w::TEXT[usize::from(code)]
    }

    /// Whether token `k` ahead is a name tree-sitter reads as a `name` where its reserved set is
    /// `global`.
    fn is_name_at(&mut self, k: usize) -> bool {
        let t = self.peek_at(k);
        t.kind == T::Name && !w::is_reserved(t.word)
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

    /// The extras before `before`, written into the node open.
    #[inline(always)]
    fn flush(&mut self, before: u32) -> R<()> {
        while let Some(&x) = self.extras.get(self.e) {
            if x.start >= before {
                break;
            }
            self.e += 1;
            let n = self.w.push(key(K::Comment, f::NONE, C_NONE, true), x.start);
            self.w.close(n, x.end);
        }
        Ok(())
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

    /// A refusal unless the next token is of kind `tok`.
    #[inline]
    fn check(&mut self, tok: K) -> R<()> {
        let t = self.tok();
        let is = if t.kind == T::Name { keyword(t.word) } else { punct(t.kind) };
        if is != Some(tok) {
            return self.refuse();
        }
        Ok(())
    }

    /// The next token, which must be of kind `tok`, as a leaf.
    fn tok_leaf(&mut self, tok: K) -> R<N> {
        self.check(tok)?;
        self.leaf(tok, f::NONE, C_NONE)
    }

    /// The next token, of kind `tok`, under `parent` in an implied gap: a leaf where the tape
    /// stores it.
    #[inline]
    fn tk(&mut self, parent: K, tok: K) -> R<()> {
        self.tk_field(parent, tok, f::NONE)
    }

    fn tk_field(&mut self, parent: K, tok: K, field: u8) -> R<()> {
        self.check(tok)?;
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
            self.tok_leaf(tok)?;
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

    /// The next token, which must be `t`, under `parent` in an implied gap.
    fn expect(&mut self, parent: K, t: T) -> R<()> {
        self.punct_tk(parent, t, false)
    }

    /// The next token, the keyword `code`, under `parent`.
    fn kw(&mut self, parent: K, code: u8) -> R<()> {
        if !self.is_word(code) {
            return self.refuse();
        }
        let k = keyword(code).expect("a keyword");
        self.tk(parent, k)
    }

    /// `_semicolon` under `parent`: `;`, or the automatic one before `?>` (no token).
    fn semi(&mut self, parent: K) -> R<()> {
        match self.peek() {
            T::Semi => self.tk(parent, K::TSemi),
            T::CloseTag => Ok(()),
            _ => self.refuse(),
        }
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
        // A cast read through `_variable` keeps its class, moved to the new context.
        let c = if k == K::CastExpression && matches!(old, C_EP | C_P) {
            match s.ctx {
                Ctx::Expr => C_EP,
                Ctx::Unary | Ctx::Primary => C_P,
                _ => C_NONE,
            }
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

    /// A rule of one token (`reference_modifier`, `variadic_placeholder`): its node, and the token in
    /// it.
    fn token_node(&mut self, kind: K, field: u8, tok: K) -> R<N> {
        let n = self.open(kind, field, C_NONE)?;
        self.tok_leaf(tok)?;
        self.close(n);
        Ok(n)
    }

    /// A node opened at byte `start` (inside a string).
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
        self.gap_tk(parent, open_k, false)?;
        loop {
            item(self)?;
            match self.peek() {
                T::Comma => {
                    if self.kind_at(1) == close {
                        if !trailing {
                            return self.refuse();
                        }
                        self.tok_leaf(K::TComma)?;
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

    /// A `name` in slot `s`: any word (its reserved set `nothing`).
    fn name_any(&mut self, s: Slot) -> R<N> {
        if self.peek() != T::Name {
            return self.refuse();
        }
        self.leaf_in(K::Name, s)
    }

    /// A `name` in slot `s` whose reserved set is `global`.
    fn name(&mut self, s: Slot) -> R<N> {
        if !self.is_name_at(0) {
            return self.refuse();
        }
        self.leaf_in(K::Name, s)
    }

    /// A `name` in slot `s` whose reserved set is `classes`.
    fn class_name(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        if t.kind != T::Name || w::is_class_reserved(t.word) {
            return self.refuse();
        }
        self.leaf_in(K::Name, s)
    }

    // ---- The file.

    fn program(&mut self) -> R<()> {
        // The root starts at its first token or extra (past the file's leading white space).
        // (A text of white space alone: its end.)
        let first = self.tok().start.min(self.extras.first().map_or(u32::MAX, |x| x.start));
        let root = self.w.push(key(K::Program, f::NONE, C_NONE, false), first);
        if self.peek() == T::OpenTag {
            self.leaf(K::PhpTag, f::NONE, C_NONE)?;
        }
        while !matches!(self.peek(), T::Eof | T::CloseTag) {
            self.statement(STMT)?;
        }
        if self.peek() == T::CloseTag {
            self.leaf(K::PhpEndTag, f::NONE, C_NONE)?;
            if self.peek() != T::Eof {
                return self.refuse();
            }
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
