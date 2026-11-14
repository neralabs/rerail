//! The parser: recursive descent over tree-sitter-rust's grammar, writing the tape as it goes.
//!
//! **Tokens** are lexed into a buffer as the parser looks ahead, 16 at a time. Where the parse
//! state reads a token shorter than the one lexed (a `>>` closing two type argument lists, `&&`
//! as two references, an integer field after `.` where the lexer read a float), the buffer is cut
//! at that byte and lexed again from there (`relex_at`, `split`).
//!
//! **Where tree-sitter's parse keeps two readings** (GLR) and a later token picks one, the parser
//! reads ahead at that token instead: a block-like expression at a statement's start ends the
//! statement unless the next token can only continue an expression; a path before `{` in a
//! condition is no struct; a parameter is a pattern where a `:` follows it, else a type.
//!
//! **Stored tokens.** A token other than a delimiter is stored unless its kind implies it
//! (`kinds::DROPPED`, `stores`). A delimiter its kind implies is still stored where its gap (the
//! tokens between two named children) is not the gap's one shortest word: a trailing comma, an
//! empty token tree, a `_` among a list's items. Each list's caller reads the gap ahead and decides
//! before it writes.

use crate::kinds::{K, f};
use crate::lex::{Doc, Extra, Lexer, R, Refusal, T, Tok, w};
use crate::{Tape, key, unkey};
use rerail_tape_format::{Redo, Widths, Writer};

mod expr;
mod items;
mod pat;
mod tt;
mod types;

// Supertype sets (`crate::CLASSES`).
const C_NONE: u8 = 0;
const C_E: u8 = 1;
const C_T: u8 = 2;
const C_PAT: u8 = 3;
const C_EL: u8 = 4;
const C_L: u8 = 5;
const C_PL: u8 = 6;
const C_LP: u8 = 7;

/// The context a node stands in: which supertypes its kind is in there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctx {
    /// An `_expression`: a literal is in `_literal` too.
    Expr,
    /// A `_type`.
    Type,
    /// A `_pattern`: a literal pattern is in `_literal_pattern` too.
    Pattern,
    /// A `_literal` alone (a token tree's, a type argument's, a const parameter's default).
    Literal,
    /// A `_literal_pattern` alone (a range pattern's sides).
    LitPat,
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
const TYPE: Slot = slot(f::NONE, Ctx::Type);
const PAT: Slot = slot(f::NONE, Ctx::Pattern);
const BARE: Slot = slot(f::NONE, Ctx::Bare);

fn is_literal(k: K) -> bool {
    use K::*;
    matches!(k, StringLiteral | RawStringLiteral | CharLiteral | BooleanLiteral | IntegerLiteral | FloatLiteral)
}

fn is_expression(k: K) -> bool {
    use K::*;
    matches!(
        k,
        UnaryExpression
            | ReferenceExpression
            | TryExpression
            | BinaryExpression
            | AssignmentExpression
            | CompoundAssignmentExpr
            | TypeCastExpression
            | CallExpression
            | ReturnExpression
            | YieldExpression
            | Identifier
            | SelfK
            | ScopedIdentifier
            | GenericFunction
            | AwaitExpression
            | FieldExpression
            | ArrayExpression
            | TupleExpression
            | MacroInvocation
            | UnitExpression
            | BreakExpression
            | ContinueExpression
            | IndexExpression
            | Metavariable
            | ClosureExpression
            | ParenthesizedExpression
            | StructExpression
            | UnsafeBlock
            | AsyncBlock
            | GenBlock
            | TryBlock
            | Block
            | IfExpression
            | MatchExpression
            | WhileExpression
            | LoopExpression
            | ForExpression
            | ConstBlock
            | RangeExpression
    )
}

fn is_type(k: K) -> bool {
    use K::*;
    matches!(
        k,
        AbstractType
            | ReferenceType
            | Metavariable
            | PointerType
            | GenericType
            | ScopedTypeIdentifier
            | TupleType
            | UnitType
            | ArrayType
            | FunctionType
            | TypeIdentifier
            | MacroInvocation
            | NeverType
            | DynamicType
            | BoundedType
            | RemovedTraitBound
            | PrimitiveType
    )
}

fn is_pattern(k: K) -> bool {
    use K::*;
    matches!(
        k,
        Identifier
            | ScopedIdentifier
            | GenericPattern
            | TuplePattern
            | TupleStructPattern
            | StructPattern
            | RefPattern
            | SlicePattern
            | CapturedPattern
            | ReferencePattern
            | RemainingFieldPattern
            | MutPattern
            | RangePattern
            | OrPattern
            | ConstBlock
            | MacroInvocation
            | TUnderscore
    )
}

fn class(k: K, ctx: Ctx) -> u8 {
    match ctx {
        Ctx::Expr if is_literal(k) => C_EL,
        Ctx::Expr if is_expression(k) => C_E,
        Ctx::Type if is_type(k) => C_T,
        Ctx::Pattern if is_literal(k) || k == K::NegativeLiteral => C_PL,
        Ctx::Pattern if is_pattern(k) => C_PAT,
        Ctx::Literal if is_literal(k) => C_L,
        Ctx::LitPat if is_literal(k) || k == K::NegativeLiteral => C_LP,
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

/// Where a statement list stands: a block's (a last expression may end it), a file's, or a
/// declaration list's (items only).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stmts {
    Block,
    File,
    Decl,
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
    /// In a condition (an `if`'s, a `match`'s value…), outside brackets: a path before `{` is no
    /// struct, and `&& let` goes on a let chain.
    no_struct: bool,
    pieces: Vec<(u32, u32, bool)>,
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
            no_struct: false,
            pieces: Vec::new(),
            far: 0,
        };
        if let Err(r) = p.source_file() {
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
        Colon => K::TColon,
        ColonColon => K::TColonColon,
        Dot => K::TDot,
        DotDot => K::TDotDot,
        DotDotDot => K::TDotDotDot,
        DotDotEq => K::TDotDotEq,
        Arrow => K::TArrow,
        FatArrow => K::TFatArrow,
        Hash => K::THash,
        Dollar => K::TDollar,
        Question => K::TQuestion,
        At => K::TAt,
        Plus => K::TPlus,
        Minus => K::TMinus,
        Star => K::TStar,
        Slash => K::TSlash,
        Percent => K::TPercent,
        Caret => K::TCaret,
        Bang => K::TBang,
        Amp => K::TAmp,
        Pipe => K::TPipe,
        AmpAmp => K::TAmpAmp,
        PipePipe => K::TPipePipe,
        LtLt => K::TLtLt,
        GtGt => K::TGtGt,
        PlusEq => K::TPlusEq,
        MinusEq => K::TMinusEq,
        StarEq => K::TStarEq,
        SlashEq => K::TSlashEq,
        PercentEq => K::TPercentEq,
        CaretEq => K::TCaretEq,
        AmpEq => K::TAmpEq,
        PipeEq => K::TPipeEq,
        LtLtEq => K::TLtLtEq,
        GtGtEq => K::TGtGtEq,
        Eq => K::TEq,
        EqEq => K::TEqEq,
        Ne => K::TNe,
        Gt => K::TGt,
        Lt => K::TLt,
        Ge => K::TGe,
        Le => K::TLe,
        Quote => K::TQuote,
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

    /// Whether `t` is `_` alone.
    fn underscore(&self, t: Tok) -> bool {
        t.kind == T::Name && t.end == t.start + 1 && self.src[t.start as usize] == b'_'
    }

    fn is_underscore(&mut self) -> bool {
        let t = self.tok();
        self.underscore(t)
    }

    /// Whether two tokens touch (`macro_rules!` is one token).
    fn adjacent(&mut self, k: usize) -> bool {
        self.peek_at(k).end == self.peek_at(k + 1).start
    }

    fn refuse<X>(&mut self) -> R<X> {
        let at = self.tok().start;
        self.far = self.far.max(at);
        Err(Refusal::Parse(at))
    }

    fn refuse_at<X>(&mut self, at: usize) -> R<X> {
        self.far = self.far.max(at as u32);
        Err(Refusal::Parse(at as u32))
    }

    /// The buffer cut at the next token, and lexing resumed at `pos`. Extras from `pos` on are
    /// lexed again.
    fn relex_at(&mut self, pos: usize) {
        self.toks.truncate(self.i);
        self.next = pos;
        let keep = self.extras.partition_point(|x| (x.start as usize) < pos);
        self.extras.truncate(keep.max(self.e));
    }

    /// The next token read as its first `len` bytes, of kind `kind` (`>` of `>>`, `&` of `&&`):
    /// the rest is lexed again after it.
    fn split(&mut self, kind: T, len: u32) {
        let t = self.tok();
        self.relex_at((t.start + len) as usize);
        self.toks.push(Tok { kind, word: 0, start: t.start, end: t.start + len });
    }

    /// The next token read as an integer from its start (a field after `.`, where the lexer read a
    /// float `0.1`).
    fn relex_int(&mut self) {
        let t = self.tok();
        let end = self.lx.int_end(t.start as usize);
        self.relex_at(end);
        self.toks.push(Tok { kind: T::Int, word: 0, start: t.start, end: end as u32 });
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

    /// A comment: a line or block comment, its doc marker and text as children.
    fn extra(&mut self, x: Extra) {
        let kind = if x.block { K::BlockComment } else { K::LineComment };
        let n = self.w.push(key(kind, f::NONE, C_NONE, true), x.start);
        if x.doc != Doc::Plain {
            let (marker, field, token) = match x.doc {
                Doc::Outer => (K::OuterDocCommentMarker, f::OUTER, K::TSlash),
                _ => (K::InnerDocCommentMarker, f::INNER, K::TBang),
            };
            let m = self.w.push(key(marker, field, C_NONE, false), x.start + 2);
            if !x.block {
                let t = self.w.push(key(token, f::NONE, C_NONE, false), x.start + 2);
                self.w.close(t, x.start + 3);
            }
            self.w.close(m, x.start + 3);
            if !x.block || x.doc_end > x.doc_start {
                let d = self.w.push(key(K::DocComment, f::DOC, C_NONE, false), x.doc_start);
                self.w.close(d, x.doc_end);
            }
        }
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

    fn expect_word(&mut self, code: u8) -> R<Tok> {
        if !self.is_word(code) {
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
    fn tk(&mut self, parent: K, tok: K, field: u8) -> R<()> {
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
            self.tk(parent, tok, f::NONE)
        }
    }

    /// The next token (punctuation) as its own kind, under `parent` in an implied gap.
    fn tk_punct(&mut self, parent: K, keep: bool) -> R<()> {
        let t = self.tok();
        let Some(k) = punct(t.kind) else { return self.refuse() };
        self.gap_tk(parent, k, keep)
    }

    /// A leaf over `start..end` (a piece of a token).
    fn leaf_span(&mut self, kind: K, field: u8, class: u8, start: u32, end: u32) {
        self.flush(start);
        let at = self.w.push(key(kind, field, class, false), start);
        self.w.close(at, end);
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

    /// The node at `at` as kind `k`, in slot `s`.
    fn rekind(&mut self, at: usize, k: K, s: Slot) {
        self.w.set_key(at, key(k, s.field, class(k, s.ctx), false));
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
    /// by `item`, between `sep`s. A `_` item (where `underscore` is set: a pattern list's) is a
    /// token, stored with its gap. A gap's delimiters are dropped where the gap is its kind's
    /// implied word: the opener before the first named item, a `sep` between two, the closer after
    /// the last; an empty list's `empty` (whether `opener closer` is the kind's one shortest word).
    /// A trailing `sep` keeps itself and the closer.
    fn list(&mut self, parent: K, close: T, underscore: bool, empty: bool, mut item: impl FnMut(&mut Self) -> R<()>) -> R<()> {
        let open = self.tok();
        let Some(open_k) = punct(open.kind) else { return self.refuse() };
        let close_k = punct(close).expect("a closer");
        let first = self.peek_at(1);
        if first.kind == close {
            // An empty list.
            self.gap_tk(parent, open_k, !empty)?;
            return self.gap_tk(parent, close_k, !empty);
        }
        let lead = underscore && self.underscore(first) && matches!(self.peek_at(2).kind, k if k == close || k == T::Comma);
        self.gap_tk(parent, open_k, lead)?;
        // Whether the gap being read holds a `_` (so all of its tokens are stored).
        let mut keep;
        loop {
            if underscore && self.is_underscore() && matches!(self.peek_at(1).kind, k if k == close || k == T::Comma) {
                // A pattern's `_` is a pattern; a parameter list's is the token alone.
                let class = if parent == K::Parameters { C_NONE } else { C_PAT };
                self.leaf(K::TUnderscore, f::NONE, class)?;
                keep = true;
            } else {
                item(self)?;
                keep = false;
            }
            match self.peek() {
                T::Comma => {
                    let after = self.peek_at(1);
                    let trailing = after.kind == close;
                    let next_underscore = underscore && self.underscore(after) && matches!(self.peek_at(2).kind, k if k == close || k == T::Comma);
                    self.gap_tk(parent, K::TComma, keep || trailing || next_underscore)?;
                    if trailing {
                        self.leaf(close_k, f::NONE, C_NONE)?;
                        return Ok(());
                    }
                }
                k if k == close => return self.gap_tk(parent, close_k, keep),
                _ => return self.refuse(),
            }
        }
    }

    /// Attributes (`#[…]`) before an item of a list, each a child of the node open.
    fn outer_attributes(&mut self) -> R<()> {
        while self.peek() == T::Hash && self.peek_at(1).kind == T::LBracket {
            self.attribute_item()?;
        }
        Ok(())
    }

    // ---- The file.

    fn source_file(&mut self) -> R<()> {
        let root = self.w.push(key(K::SourceFile, f::NONE, C_NONE, false), 0);
        self.shebang()?;
        self.statements(Stmts::File)?;
        if self.peek() != T::Eof {
            return self.refuse();
        }
        let t = self.tok();
        if t.word == u8::MAX {
            return Err(Refusal::Lex(t.start));
        }
        self.flush(u32::MAX);
        self.w.close(root, self.src.len() as u32);
        Ok(())
    }

    /// `#!…` on the first line, but `#![` (an inner attribute): a `shebang`, through its `\n`.
    fn shebang(&mut self) -> R<()> {
        if !self.src.starts_with(b"#!") || self.src.get(2) == Some(&b'[') {
            return Ok(());
        }
        let Some(nl) = self.src.iter().position(|&c| c == b'\n') else { return self.refuse_at(0) };
        let end = nl as u32 + 1;
        let at = self.w.push(key(K::Shebang, f::NONE, C_NONE, false), 0);
        self.w.close(at, end);
        self.relex_at(end as usize);
        self.last_end = end;
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
