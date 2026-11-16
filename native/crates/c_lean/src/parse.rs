//! The parser: recursive descent over tree-sitter-c's grammar (and tree-sitter-cpp's, which extends
//! it), writing the tape as it goes.
//!
//! **Tokens** are lexed into a buffer as the parser looks ahead, 16 at a time.
//!
//! **Where tree-sitter's parse keeps two readings** (GLR) and a later token or a dynamic precedence
//! picks one, the parser reads ahead at that point instead: a declaration against an expression
//! statement by the shape of its first tokens, a cast against a parenthesized expression by what
//! follows the `)`, a parameter's declarator as abstract or not by whether it names anything.
//!
//! **Directives** (`#include`, `#define`, `#if` …) end at their line's `\n`, which is in the
//! directive's span but no node: each token knows the first `\n` before it (`Tok::nl`).
//!
//! **Stored tokens.** A token other than a delimiter is stored unless its kind implies it
//! (`kinds::DROPPED_C`, `kinds::DROPPED_CPP`, `stores`). A delimiter its kind implies is still
//! stored where its gap (the tokens between two named children) is not the gap's one shortest
//! word: a trailing comma, an empty string's quotes.

use crate::kinds::{K, f};
use crate::lex::{Extra, Lexer, NO_NL, R, Refusal, T, Tok, w};
use crate::{Dialect, Tape, key, unkey};
use rerail_tape_format::{Checkpoint, Redo, Widths, Writer};

mod cpp;
mod decl;
mod expr;
mod pre;
mod stmt;

// Supertype sets (`crate::CLASSES`).
const C_NONE: u8 = 0;
const C_E: u8 = 1;
const C_S: u8 = 2;
const C_T: u8 = 3;
const C_D: u8 = 4;
const C_FD: u8 = 5;
const C_TD: u8 = 6;
const C_AD: u8 = 7;

/// The context a node stands in: which supertype its kind is in there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctx {
    /// An `expression`.
    Expr,
    /// A `statement`.
    Stmt,
    /// A `type_specifier`.
    Type,
    /// A `_declarator`.
    Decl,
    /// A `_field_declarator`.
    FDecl,
    /// A `_type_declarator`.
    TDecl,
    /// An `_abstract_declarator`.
    ADecl,
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

impl Slot {
    /// The slot's field, in context `ctx`.
    const fn with_ctx(self, ctx: Ctx) -> Slot {
        Slot { field: self.field, ctx }
    }
}

const EXPR: Slot = slot(f::NONE, Ctx::Expr);
const STMT: Slot = slot(f::NONE, Ctx::Stmt);
const BARE: Slot = slot(f::NONE, Ctx::Bare);

fn class(_k: K, ctx: Ctx) -> u8 {
    match ctx {
        Ctx::Expr => C_E,
        Ctx::Stmt => C_S,
        Ctx::Type => C_T,
        Ctx::Decl => C_D,
        Ctx::FDecl => C_FD,
        Ctx::TDecl => C_TD,
        Ctx::ADecl => C_AD,
        Ctx::Bare => C_NONE,
    }
}

const MAX_DEPTH: u32 = 400;

/// `kinds::DROPPED_C` and `kinds::DROPPED_CPP` as open-addressed tables, built at compile time: a
/// key's slot by one multiply-shift, then the next slots to an empty one (as `rerail-java-lean`'s).
const DROP_SLOTS: usize = 1024;
const NO_DROP: u32 = u32::MAX;

#[inline(always)]
const fn drop_slot(key: u32) -> usize {
    (key.wrapping_mul(0x9e37_79b1) >> 22) as usize
}

const fn drop_table(keys: &[u32]) -> [u32; DROP_SLOTS] {
    let mut t = [NO_DROP; DROP_SLOTS];
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
}

static DROP_C: [u32; DROP_SLOTS] = drop_table(crate::kinds::DROPPED_C);
static DROP_CPP: [u32; DROP_SLOTS] = drop_table(crate::kinds::DROPPED_CPP);

/// A node written: its index and kind.
#[derive(Clone, Copy, Debug)]
struct N {
    at: usize,
    kind: K,
}

/// Where an item list is: what its items may be.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Level {
    /// The file's (`_top_level_item`).
    Top,
    /// A block's (`_block_item`).
    Block,
    /// A struct's or union's fields (`_field_declaration_list_item`).
    Field,
    /// An enum's enumerators, each with its `,`.
    Enum,
}

struct P<'a> {
    src: &'a [u8],
    lx: &'a Lexer<'a>,
    cpp: bool,
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
    /// Whether an ambiguous item ahead reads as a declaration (outside function bodies, and a
    /// block's first item), not an expression.
    top: bool,
    /// In a template's argument list outside brackets: a `>` closes it, no operator.
    no_gt: bool,
    /// The next unary is an expression's first operand (`expr_prec`'s), not another unary's.
    prefix_top: bool,
    /// In a pointer's or a reference's declarator: tree-sitter's parse forks at a `(` after the
    /// name (parameters or an initializer's arguments), so each fork lexes the next word.
    in_pointer: bool,
    /// The next expression is the whole of a parentheses' content (C++: any expression may be
    /// assigned to there).
    paren_top: bool,
    /// The `>>` tokens split into two `>` (a template's closer): their tokens before, for a
    /// rewind.
    splits: Vec<(usize, Tok)>,
    /// How many times the tokens ahead were lexed again (a directive's argument).
    relexes: u32,
}

/// Where a parse that may not hold started (`P::mark`, `P::reset`).
#[derive(Clone, Copy)]
struct Mark {
    i: usize,
    n: Checkpoint,
    /// The writer's node count: the next node's index.
    n_len: usize,
    e: usize,
    last_end: u32,
    depth: u32,
    splits: usize,
    relexes: u32,
    no_gt: bool,
    top: bool,
    in_pointer: bool,
}

/// `src` parsed.
pub fn parse(src: &[u8], dialect: Dialect) -> R<Tape> {
    let cpp = dialect == Dialect::Cpp;
    let lx = Lexer::new(src, cpp);
    if let Some(at) = lx.blocks.first_nul() {
        return Err(Refusal::Lex(at as u32));
    }
    let mut widths = Widths::guess(src.len());
    loop {
        let mut p = P {
            src,
            lx: &lx,
            cpp,
            toks: Vec::with_capacity(src.len() / 4 + 4),
            i: 0,
            // A byte-order mark: tree-sitter's lexer passes it.
            next: if src.starts_with("\u{feff}".as_bytes()) { 3 } else { 0 },
            extras: Vec::new(),
            e: 0,
            w: Writer::new(0, widths, src.len() / 3 + 16),
            last_end: 0,
            depth: 0,
            far: 0,
            top: true,
            no_gt: false,
            prefix_top: false,
            in_pointer: false,
            paren_top: false,
            splits: Vec::new(),
            relexes: 0,
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
        DotStar => K::TDotStar,
        DotDotDot => K::TDotDotDot,
        Colon => K::TColon,
        ColonColon => K::TColonColon,
        Question => K::TQuestion,
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
        Arrow => K::TArrow,
        ArrowStar => K::TArrowStar,
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
        Spaceship => K::TSpaceship,
        Gt => K::TGt,
        Ge => K::TGe,
        GtGt => K::TGtGt,
        GtGtEq => K::TGtGtEq,
        _ => return None,
    })
}

/// A keyword's word code as the tape's token kind.
fn keyword(code: u8) -> Option<K> {
    use K::*;
    Some(match code {
        w::SIGNED => Tsigned,
        w::UNSIGNED => Tunsigned,
        w::LONG => Tlong,
        w::SHORT => Tshort,
        w::EXTERN => Textern,
        w::STATIC => Tstatic,
        w::AUTO => Tauto,
        w::REGISTER => Tregister,
        w::INLINE => Tinline,
        w::U_INLINE => T__inline,
        w::U_INLINE_U => T__inline__,
        w::U_FORCEINLINE => T__forceinline,
        w::THREAD_LOCAL => Tthread_local,
        w::U_THREAD => T__thread,
        w::CONST => Tconst,
        w::CONSTEXPR => Tconstexpr,
        w::VOLATILE => Tvolatile,
        w::RESTRICT => Trestrict,
        w::U_RESTRICT_U => T__restrict__,
        w::U_EXTENSION_U => T__extension__,
        w::ATOMIC => T_Atomic,
        w::NORETURN_ => T_Noreturn,
        w::NORETURN => Tnoreturn,
        w::NONNULL => T_Nonnull,
        w::ALIGNAS => Talignas,
        w::ALIGNAS_ => T_Alignas,
        w::MUTABLE => Tmutable,
        w::CONSTINIT => Tconstinit,
        w::CONSTEVAL => Tconsteval,
        w::U_DECLSPEC => T__declspec,
        w::U_BASED => T__based,
        w::U_CDECL => T__cdecl,
        w::U_CLRCALL => T__clrcall,
        w::U_STDCALL => T__stdcall,
        w::U_FASTCALL => T__fastcall,
        w::U_THISCALL => T__thiscall,
        w::U_VECTORCALL => T__vectorcall,
        w::U_RESTRICT => T__restrict,
        w::U_UPTR => T__uptr,
        w::U_SPTR => T__sptr,
        w::UNALIGNED_ => T_unaligned,
        w::U_UNALIGNED => T__unaligned,
        w::BREAK => Tbreak,
        w::CASE => Tcase,
        w::CONTINUE => Tcontinue,
        w::DEFAULT => Tdefault,
        w::DO => Tdo,
        w::ELSE => Telse,
        w::ENUM => Tenum,
        w::FOR => Tfor,
        w::GOTO => Tgoto,
        w::IF => Tif,
        w::RETURN => Treturn,
        w::SIZEOF => Tsizeof,
        w::STRUCT => Tstruct,
        w::SWITCH => Tswitch,
        w::TYPEDEF => Ttypedef,
        w::UNION => Tunion,
        w::WHILE => Twhile,
        w::U_ATTRIBUTE_U => T__attribute__,
        w::U_ATTRIBUTE => T__attribute,
        w::ASM => Tasm,
        w::U_ASM_U => T__asm__,
        w::U_ASM => T__asm,
        w::U_VOLATILE_U => T__volatile__,
        w::U_TRY => T__try,
        w::U_EXCEPT => T__except,
        w::U_FINALLY => T__finally,
        w::U_LEAVE => T__leave,
        w::OFFSETOF => Toffsetof,
        w::U_ALIGNOF_U => T__alignof__,
        w::U_ALIGNOF => T__alignof,
        w::ALIGNOF_ => T_alignof,
        w::ALIGNOF => Talignof,
        w::ALIGNOF_C => T_Alignof,
        w::GENERIC => T_Generic,
        w::DEFINED => Tdefined,
        w::CLASS => Tclass,
        w::NAMESPACE => Tnamespace,
        w::TEMPLATE => Ttemplate,
        w::TYPENAME => Ttypename,
        w::USING => Tusing,
        w::VIRTUAL => Tvirtual,
        w::PUBLIC => Tpublic,
        w::PRIVATE => Tprivate,
        w::PROTECTED => Tprotected,
        w::FRIEND => Tfriend,
        w::OPERATOR => Toperator,
        w::NEW => Tnew,
        w::DELETE => Tdelete,
        w::THROW => Tthrow,
        w::TRY => Ttry,
        w::CATCH => Tcatch,
        w::NOEXCEPT => Tnoexcept,
        w::DECLTYPE => Tdecltype,
        w::EXPLICIT => Texplicit,
        w::FINAL => Tfinal,
        w::OVERRIDE => Toverride,
        w::CONCEPT => Tconcept,
        w::REQUIRES => Trequires,
        w::CO_AWAIT => Tco_await,
        w::CO_RETURN => Tco_return,
        w::CO_YIELD => Tco_yield,
        w::STATIC_ASSERT => Tstatic_assert,
        w::NULL => TNULL,
        w::NULLPTR => Tnullptr,
        _ => return None,
    })
}

/// Whether a word is one of C's keywords that start a statement or a declaration, never an
/// identifier there.
fn reserved(code: u8) -> bool {
    code != w::NONE && code < w::TRUE && code >= w::SIGNED || is_primitive(code)
}

/// C++'s own keywords.
fn reserved_cpp(code: u8) -> bool {
    (w::CLASS..=w::STATIC_ASSERT).contains(&code) && !matches!(code, w::FINAL | w::OVERRIDE)
}

/// Whether a word is a `primitive_type`.
fn is_primitive(code: u8) -> bool {
    (w::BOOL..=w::CHAR64_T).contains(&code)
}

fn kind_of(k: u16) -> K {
    debug_assert!(k <= crate::kinds::LAST as u16);
    // SAFETY: `K` is `repr(u16)` with variants 0 to `KINDS.len() - 1`, and every key's kind is one.
    unsafe { std::mem::transmute::<u16, K>(k) }
}

impl<'a> P<'a> {
    // ---- Tokens.

    #[inline(always)]
    fn fill(&mut self, k: usize) {
        while self.toks.len() <= self.i + k {
            self.lex_batch();
        }
    }

    /// Tokens lexed into the buffer, up to 16 in one loop (simdjson's stage 2 over its index).
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
                Err(Refusal::Lex(at)) | Err(Refusal::Parse(at)) => Tok { kind: T::Eof, word: u8::MAX, start: at, end: at, nl: NO_NL },
                Err(Refusal::TooLarge) => Tok { kind: T::Eof, word: u8::MAX, start: 0, end: 0, nl: NO_NL },
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

    /// Whether token `k` ahead is a name that can be an identifier (no keyword of the dialect).
    fn is_ident_at(&mut self, k: usize) -> bool {
        let t = self.peek_at(k);
        t.kind == T::Name && !self.reserved(t.word)
    }

    /// Whether a word is a keyword of the dialect, never an identifier.
    pub(super) fn reserved(&self, code: u8) -> bool {
        reserved(code) || self.cpp && reserved_cpp(code)
    }

    fn is_ident(&mut self) -> bool {
        self.is_ident_at(0)
    }

    fn refuse<X>(&mut self) -> R<X> {
        let at = self.tok().start;
        self.far = self.far.max(at);
        Err(Refusal::Parse(at))
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

    /// A comment, a leaf.
    fn extra(&mut self, x: Extra) {
        let n = self.w.push(key(K::Comment, f::NONE, C_NONE, true), x.start);
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
        let table = if self.cpp { &DROP_CPP } else { &DROP_C };
        let mut slot = drop_slot(key);
        loop {
            // SAFETY: `slot` is masked to the table's slots.
            let k = unsafe { *table.get_unchecked(slot) };
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
        self.gap_tk_field(parent, tok, f::NONE, keep)
    }

    fn gap_tk_field(&mut self, parent: K, tok: K, field: u8, keep: bool) -> R<()> {
        if keep {
            self.leaf(tok, field, C_NONE)?;
            Ok(())
        } else {
            self.tk_field(parent, tok, field)
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

    /// The next token, a keyword, under `parent`.
    fn kw_any(&mut self, parent: K) -> R<()> {
        let t = self.tok();
        let Some(k) = (t.kind == T::Name).then(|| keyword(t.word)).flatten() else { return self.refuse() };
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

    /// The node at `at` as kind `kind`, its field kept (a declaration found to be a definition).
    fn rekind_keep(&mut self, at: usize, kind: K) {
        let (_, field, _, _) = unkey(self.w.key(at));
        self.w.set_key(at, key(kind, field as u8, class(kind, Ctx::Bare), false));
    }

    /// The node at `at` as kind `kind` in slot `s` (a name found to be a type's).
    fn rekind(&mut self, at: usize, kind: K, s: Slot) {
        self.w.set_key(at, key(kind, s.field, class(kind, s.ctx), false));
    }

    /// Where the parse is: what `reset` goes back to.
    fn mark(&mut self) -> Mark {
        self.fill(0);
        Mark {
            i: self.i,
            n: self.w.checkpoint(),
            n_len: self.w.len(),
            e: self.e,
            last_end: self.last_end,
            depth: self.depth,
            splits: self.splits.len(),
            relexes: self.relexes,
            no_gt: self.no_gt,
            top: self.top,
            in_pointer: self.in_pointer,
        }
    }

    /// Back to `m` (a reading that did not hold). Lexing has no state, so the tokens lexed since
    /// are kept, unless a directive's argument was lexed again since: then those after the mark's
    /// token are lexed again.
    fn reset(&mut self, m: Mark) {
        while self.splits.len() > m.splits {
            let (at, t) = self.splits.pop().expect("a split");
            self.toks[at] = t;
            if at + 1 < self.toks.len() {
                self.toks.remove(at + 1);
            }
        }
        self.i = m.i;
        if self.relexes != m.relexes {
            let t = self.toks[m.i];
            self.toks.truncate(m.i + 1);
            self.next = t.end as usize;
            let keep = self.extras.partition_point(|x| x.start < t.end);
            self.extras.truncate(keep.max(m.e));
            self.relexes = m.relexes;
        }
        self.e = m.e;
        self.w.rewind(m.n);
        self.last_end = m.last_end;
        self.depth = m.depth;
        self.no_gt = m.no_gt;
        self.top = m.top;
        self.in_pointer = m.in_pointer;
    }

    /// The next token, a `>>`, read as two `>`: the first left as the next token.
    fn split_gt(&mut self) {
        let t = self.tok();
        debug_assert!(matches!(t.kind, T::GtGt | T::Ge | T::GtGtEq));
        self.splits.push((self.i, t));
        let rest = match t.kind {
            T::GtGt => T::Gt,
            T::GtGtEq => T::Ge,
            _ => T::Eq,
        };
        // The second half stays in the buffer as the next token after the first.
        self.toks[self.i] = Tok { kind: T::Gt, word: 0, start: t.start, end: t.start + 1, nl: t.nl };
        self.toks.insert(self.i + 1, Tok { kind: rest, word: 0, start: t.start + 1, end: t.end, nl: NO_NL });
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
    /// by `item`, between `,`s, read inside the brackets. A gap's delimiters are dropped where the
    /// gap is its kind's implied word; an empty list's where `empty`. A trailing `,` (where
    /// `trailing` allows one) keeps itself and the closer.
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
        // The root starts at its first token or comment (past the file's leading white space; at
        // the end in a file of white space alone).
        let first = self.tok().start.min(self.extras.first().map_or(u32::MAX, |x| x.start));
        let root = self.w.push(key(K::TranslationUnit, f::NONE, C_NONE, false), first);
        while self.peek() != T::Eof {
            self.item(Level::Top, K::TranslationUnit)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The hash tables hold `kinds::DROPPED_C`'s and `DROPPED_CPP`'s keys, and no other key.
    #[test]
    fn the_drop_tables_are_dropped() {
        for (table, keys) in [(&DROP_C, crate::kinds::DROPPED_C), (&DROP_CPP, crate::kinds::DROPPED_CPP)] {
            assert_eq!(table.iter().filter(|&&k| k != NO_DROP).count(), keys.len());
            let find = |key: u32| {
                let mut slot = drop_slot(key);
                loop {
                    match table[slot] {
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
}
