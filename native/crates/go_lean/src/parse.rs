//! The parser: recursive descent over tree-sitter-go's grammar, writing the tape as it goes.
//!
//! **Tokens** are lexed into a buffer as the parser looks ahead, 16 at a time.
//!
//! **Terminators.** tree-sitter-go ends a statement, a spec or a field at `;` or at a `\n` its
//! parse state takes as a token (elsewhere a `\n` is white space). The `\n` is hidden: it is no node.
//! Where a construct may end, the parser stops at a token with a `\n` before it (`Tok::nl`) when it
//! reads at a level where a terminator is valid (`nl_ends`: statement lists, spec lists, field
//! lists; not inside brackets).
//!
//! **Where tree-sitter's parse keeps two readings** (GLR) and a later token picks one, the parser
//! reads ahead at that token instead: a parameter list's names by the shape of its items, a type
//! switch by its `.(type)`, type parameters by a name and a constraint.
//!
//! **Stored tokens.** A token other than a delimiter is stored unless its kind implies it
//! (`kinds::DROPPED`, `stores`). A delimiter its kind implies is still stored where its gap (the
//! tokens between two named children) is not the gap's one shortest word: a trailing comma. Each
//! list's caller reads the gap ahead and decides before it writes.

use crate::kinds::{K, f};
use crate::lex::{Extra, Lexer, NO_NL, R, Refusal, T, Tok, w};
use crate::{Tape, key, unkey};
use rerail_tape_format::{Redo, Widths, Writer};

mod decl;
mod expr;
mod stmt;
mod types;

// Supertype sets (`crate::CLASSES`).
const C_NONE: u8 = 0;
const C_E: u8 = 1;
const C_ST: u8 = 2;
const C_S: u8 = 3;
const C_SS: u8 = 4;
const C_SSO: u8 = 5;

/// The context a node stands in: which supertypes its kind is in there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctx {
    /// An `_expression`.
    Expr,
    /// A `_type` (`_simple_type` or `parenthesized_type`).
    Type,
    /// A `_simple_type`.
    SType,
    /// A `_statement`: a simple statement is in `_simple_statement` too.
    Stmt,
    /// A `_simple_statement`.
    Simple,
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
const BARE: Slot = slot(f::NONE, Ctx::Bare);

fn is_simple_type(k: K) -> bool {
    use K::*;
    matches!(
        k,
        TypeIdentifier
            | GenericType
            | QualifiedType
            | PointerType
            | StructType
            | InterfaceType
            | ArrayType
            | SliceType
            | MapType
            | ChannelType
            | FunctionType
            | NegatedType
    )
}

fn is_simple_statement(k: K) -> bool {
    use K::*;
    matches!(k, ExpressionStatement | SendStatement | IncStatement | DecStatement | AssignmentStatement | ShortVarDeclaration)
}

fn class(k: K, ctx: Ctx) -> u8 {
    match ctx {
        Ctx::Expr => C_E,
        Ctx::Type if is_simple_type(k) => C_ST,
        Ctx::Type => C_NONE,
        Ctx::SType => C_ST,
        Ctx::Stmt if is_simple_statement(k) => C_SSO,
        Ctx::Stmt => C_S,
        Ctx::Simple => C_SS,
        Ctx::Bare => C_NONE,
    }
}

const MAX_DEPTH: u32 = 300;

/// `kinds::DROPPED` as an open-addressed table, built at compile time: a key's slot by one
/// multiply-shift, then the next slots to an empty one (as `rerail-java-lean`'s).
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
    /// Whether a `\n` ends what is read here (a statement list's, a spec list's level; not inside
    /// brackets).
    nl_ends: bool,
    /// In an `if`, `for` or `switch` header, outside brackets: a type name before `{` is no
    /// composite literal's (the `{` is the body's).
    no_literal: bool,
}

/// `src` parsed.
pub fn parse(src: &[u8]) -> R<Tape> {
    let lx = Lexer::new(src);
    if let Some(at) = lx.blocks.first_nul() {
        return Err(Refusal::Lex(at as u32));
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
            extras: Vec::new(),
            e: 0,
            w: Writer::new(0, widths, src.len() / 3 + 16),
            last_end: 0,
            depth: 0,
            far: 0,
            nl_ends: true,
            no_literal: false,
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
        Colon => K::TColon,
        ColonEq => K::TColonEq,
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
        AmpCaret => K::TAmpCaret,
        AmpCaretEq => K::TAmpCaretEq,
        Pipe => K::TPipe,
        PipePipe => K::TPipePipe,
        PipeEq => K::TPipeEq,
        Lt => K::TLt,
        Le => K::TLe,
        LtLt => K::TLtLt,
        LtLtEq => K::TLtLtEq,
        LtMinus => K::TLtMinus,
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
        w::BREAK => TBreak,
        w::CASE => TCase,
        w::CHAN => TChan,
        w::CONST => TConst,
        w::CONTINUE => TContinue,
        w::DEFAULT => TDefault,
        w::DEFER => TDefer,
        w::ELSE => TElse,
        w::FALLTHROUGH => TFallthrough,
        w::FOR => TFor,
        w::FUNC => TFunc,
        w::GO => TGo,
        w::GOTO => TGoto,
        w::IF => TIf,
        w::IMPORT => TImport,
        w::INTERFACE => TInterface,
        w::MAP => TMap,
        w::PACKAGE => TPackage,
        w::RANGE => TRange,
        w::RETURN => TReturn,
        w::SELECT => TSelect,
        w::STRUCT => TStruct,
        w::SWITCH => TSwitch,
        w::TYPE => TType,
        w::VAR => TVar,
        _ => return None,
    })
}

/// Whether a word is one of Go's keywords, never an identifier.
fn reserved(code: u8) -> bool {
    code != w::NONE && code <= w::VAR
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
                Err(Refusal::Lex(at)) | Err(Refusal::Parse(at)) => Tok { kind: T::Eof, word: u8::MAX, start: at, end: at, nl: NO_NL, nl_last: NO_NL },
                Err(Refusal::TooLarge) => Tok { kind: T::Eof, word: u8::MAX, start: 0, end: 0, nl: NO_NL, nl_last: NO_NL },
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

    /// Whether token `k` ahead is a name that can be an identifier (no keyword).
    fn is_ident_at(&mut self, k: usize) -> bool {
        let t = self.peek_at(k);
        t.kind == T::Name && !reserved(t.word)
    }

    fn is_ident(&mut self) -> bool {
        self.is_ident_at(0)
    }

    /// Whether a `\n` before the next token ends what is read here (a terminator).
    #[inline(always)]
    fn at_nl(&mut self) -> bool {
        self.nl_ends && self.tok().nl != NO_NL
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

    /// The node at `at` as kind `kind` in slot `s` (a name found to be a type's).
    fn rekind(&mut self, at: usize, kind: K, s: Slot) {
        self.w.set_key(at, key(kind, s.field, class(kind, s.ctx), false));
    }

    /// `f` read inside brackets: a `\n` is white space there, and a composite literal may follow a
    /// type name again.
    fn nested<X>(&mut self, f: impl FnOnce(&mut Self) -> R<X>) -> R<X> {
        let saved = (self.nl_ends, self.no_literal);
        (self.nl_ends, self.no_literal) = (false, false);
        let r = f(self);
        (self.nl_ends, self.no_literal) = saved;
        r
    }

    /// `f` read at a level where a `\n` ends a statement, a spec or a field (a block's, a struct's).
    fn lines<X>(&mut self, f: impl FnOnce(&mut Self) -> R<X>) -> R<X> {
        let saved = (self.nl_ends, self.no_literal);
        (self.nl_ends, self.no_literal) = (true, false);
        let r = f(self);
        (self.nl_ends, self.no_literal) = saved;
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
    /// by `item`, between `,`s, read inside the brackets. A gap's delimiters are dropped where the
    /// gap is its kind's implied word; an empty list's where `empty`. A trailing `,` (where
    /// `trailing` allows one) keeps itself and the closer.
    fn list(&mut self, parent: K, close: T, empty: bool, trailing: bool, item: impl FnMut(&mut Self) -> R<()>) -> R<()> {
        self.nested(|p| p.list_in(parent, close, empty, trailing, item))
    }

    fn list_in(&mut self, parent: K, close: T, empty: bool, trailing: bool, mut item: impl FnMut(&mut Self) -> R<()>) -> R<()> {
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

    /// A terminator after an item of `parent`'s line list: `;` (a token in the gap), a hidden
    /// `\n`, or none before `close` (or the end). Whether a `;` was read.
    fn terminator(&mut self, parent: K, close: T) -> R<bool> {
        let t = self.tok();
        if t.kind == T::Semi {
            self.tk(parent, K::TSemi)?;
            return Ok(true);
        }
        if t.nl != NO_NL {
            // The hidden `\n`: no node, but its list's span takes it, and the comments before it.
            self.flush(t.nl_last);
            self.last_end = t.nl_last + 1;
            return Ok(false);
        }
        if t.kind == close || t.kind == T::Eof {
            return Ok(false);
        }
        self.refuse()
    }

    // ---- The file.

    fn program(&mut self) -> R<()> {
        // The root starts at its first token or comment (past the file's leading white space; at
        // the end in a file of white space alone).
        let first = self.tok().start.min(self.extras.first().map_or(u32::MAX, |x| x.start));
        let root = self.w.push(key(K::SourceFile, f::NONE, C_NONE, false), first);
        loop {
            match self.peek() {
                T::Eof => break,
                T::Semi => {
                    self.leaf(K::TSemi, f::NONE, C_NONE)?;
                    continue;
                }
                _ => {}
            }
            self.toplevel()?;
            self.terminator(K::SourceFile, T::Eof)?;
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
