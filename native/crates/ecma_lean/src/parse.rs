//! The parser: recursive descent over tree-sitter-javascript's grammar (and -typescript's), writing
//! the tape as it goes.
//!
//! **Tokens** are lexed into a buffer as the parser looks ahead, in normal mode. Where the parse
//! state reads another mode (a regular expression where an operand starts, a template's text, JSX
//! text and tags), the buffer is cut at that byte and the piece read in its mode (`relex_at`).
//!
//! **Automatic semicolons** are tree-sitter's external scanner's (`asi`, ported from
//! `scanner.c`): at a point where the statement may end, the scanner reads the bytes after the last
//! token, and the parse states' valid symbols it asks about are the parser's (`V`).
//!
//! **Speculation.** Where tree-sitter's GLR parse keeps two readings until a later token picks one
//! (`(a, b)` before `=>`, `[a, b]` before `=`), the parser tries the rarer reading and goes back
//! (`mark`, `restore`): the writer, the tokens and the extras are cut back to the mark.

use crate::kinds::{K, f};
use crate::lex::{Extra, Lexer, Piece, R, Refusal, T, Tok, w};
use crate::{Dialect, Tape, key, unkey};
use rerail_tape_format::{Redo, Widths, Writer};

// Supertype sets (`crate::CLASSES`).
const C_NONE: u8 = 0;
const C_EP: u8 = 1;
const C_E: u8 = 2;
const C_P: u8 = 3;
const C_DS: u8 = 4;
const C_S: u8 = 5;
const C_PAT: u8 = 6;
const C_D: u8 = 7;
const C_TP: u8 = 8;
const C_T: u8 = 9;
const C_PT: u8 = 10;

mod ts;
use ts::{is_primary_type, is_type_only};

/// The context a node stands in: which supertypes its kind is in there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctx {
    /// An `expression`: a primary one is in both supertypes, another in `expression`.
    Expr,
    /// `choice(expression, primary_expression)` (a member's object): a primary one is read as a
    /// `primary_expression`, another as an `expression`.
    Object,
    /// A `primary_expression` alone (a `new`'s constructor, a tagged template's function).
    Primary,
    /// A `statement`: a declaration is in both supertypes.
    Stmt,
    /// A `declaration` (an export's).
    Decl,
    /// A `pattern`.
    Pattern,
    /// A `type`: a primary type is in both supertypes.
    Type,
    /// A `primary_type` alone (an array type's element, a lookup's object, `keyof`'s operand).
    PrimaryType,
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
const BARE: Slot = slot(f::NONE, Ctx::Bare);

fn is_primary(k: K) -> bool {
    use K::*;
    matches!(
        k,
        SubscriptExpression
            | MemberExpression
            | ParenthesizedExpression
            | Identifier
            | Undefined
            | This
            | Super
            | Number
            | String
            | TemplateString
            | Regex
            | True
            | False
            | Null
            | Object
            | Array
            | FunctionExpression
            | ArrowFunction
            | GeneratorFunction
            | Class
            | MetaProperty
            | CallExpression
            | NonNullExpression
    )
}

fn is_expression_only(k: K) -> bool {
    use K::*;
    matches!(
        k,
        JsxElement
            | JsxSelfClosingElement
            | AssignmentExpression
            | AugmentedAssignmentExpression
            | AwaitExpression
            | UnaryExpression
            | BinaryExpression
            | TernaryExpression
            | UpdateExpression
            | NewExpression
            | YieldExpression
            | AsExpression
            | SatisfiesExpression
            | InstantiationExpression
            | TypeAssertion
            | InternalModule
    )
}

fn is_declaration(k: K) -> bool {
    use K::*;
    matches!(
        k,
        FunctionDeclaration
            | GeneratorFunctionDeclaration
            | ClassDeclaration
            | LexicalDeclaration
            | VariableDeclaration
            | UsingDeclaration
            | FunctionSignature
            | AbstractClassDeclaration
            | Module
            | InternalModule
            | TypeAliasDeclaration
            | EnumDeclaration
            | InterfaceDeclaration
            | ImportAlias
            | AmbientDeclaration
    )
}

fn is_pattern(k: K) -> bool {
    use K::*;
    matches!(k, MemberExpression | SubscriptExpression | Identifier | Undefined | ObjectPattern | ArrayPattern | RestPattern | ParenthesizedExpression | NonNullExpression)
}

fn class(k: K, ctx: Ctx) -> u8 {
    match ctx {
        Ctx::Expr if is_primary(k) => C_EP,
        Ctx::Expr if is_expression_only(k) => C_E,
        Ctx::Object if is_primary(k) => C_P,
        Ctx::Object if is_expression_only(k) => C_E,
        Ctx::Primary if is_primary(k) => C_P,
        Ctx::Stmt if is_declaration(k) => C_DS,
        Ctx::Stmt => C_S,
        Ctx::Decl if is_declaration(k) => C_D,
        Ctx::Pattern if is_pattern(k) => C_PAT,
        Ctx::Type if is_primary_type(k) => C_TP,
        Ctx::Type if is_type_only(k) => C_T,
        Ctx::PrimaryType if is_primary_type(k) => C_PT,
        _ => C_NONE,
    }
}

/// The valid symbols the scanner asks about at an automatic semicolon (`scanner.c`'s).
type V = u16;
const V_LOGICAL_OR: V = 1;
const V_LBRACKET: V = 2;
const V_LPAREN: V = 4;
const V_STAR: V = 8;
const V_PLUS: V = 16;
const V_MINUS: V = 32;
const V_ARROW_BODY_END: V = 64;
const V_NO_LT: V = 128;
const V_MEMBER_NAME: V = 256;
/// TypeScript's `_function_signature_automatic_semicolon` (after an overload's signature).
const V_FN_SIG: V = 512;
/// After an expression, where it may go on.
const V_EXPR: V = V_LOGICAL_OR | V_LBRACKET | V_LPAREN | V_STAR | V_PLUS | V_MINUS;

const MAX_DEPTH: u32 = 300;

/// A node written: its index and kind.
#[derive(Clone, Copy, Debug)]
struct N {
    at: usize,
    kind: K,
}

/// Where the parse was, to go back to (`restore`).
#[derive(Clone, Copy)]
struct Mark {
    i: usize,
    n: usize,
    e: usize,
    last_end: u32,
    depth: u32,
    after: V,
    asi: bool,
    /// `P::relexes` at the mark.
    relexes: u32,
}

struct P<'a> {
    src: &'a [u8],
    lx: &'a Lexer<'a>,
    dialect: Dialect,
    toks: Vec<Tok>,
    i: usize,
    /// Where the next token is lexed into the buffer.
    next: usize,
    /// The times the buffer was cut for another mode (`relex_at`): a restore keeps the tokens
    /// lexed after its mark unless one was.
    relexes: u32,
    extras: Vec<Extra>,
    e: usize,
    w: Writer<Widths>,
    last_end: u32,
    depth: u32,
    /// The valid symbols after what was read last, for the scanner at the statement's end.
    after: V,
    /// Whether an automatic semicolon may end the expression being read (it stands at a
    /// statement's level, not inside brackets).
    asi: bool,
    pieces: Vec<(u32, u32, bool)>,
    /// The furthest refusal (a speculation's included): where a refused file is reported.
    far: u32,
    /// The tokens the dialect's tapes leave out (`kinds::DROPPED`).
    dropped: &'static [u32],
    /// TypeScript's grammar (`.ts` and `.tsx`).
    ts: bool,
    /// In a class's `extends` (TypeScript): a `<` after the value is its type arguments.
    in_extends: bool,
}

/// `src` parsed in `dialect`.
pub fn parse(src: &[u8], dialect: Dialect) -> R<Tape> {
    let lx = Lexer::new(src);
    let mut widths = Widths::guess(src.len());
    loop {
        let mut p = P {
            src,
            lx: &lx,
            dialect,
            toks: Vec::with_capacity(src.len() / 4 + 4),
            i: 0,
            next: 0,
            relexes: 0,
            extras: Vec::new(),
            e: 0,
            w: Writer::new(0, widths, src.len() / 3 + 16),
            last_end: 0,
            depth: 0,
            after: 0,
            asi: false,
            pieces: Vec::new(),
            far: 0,
            dropped: crate::kinds::DROPPED[dialect as usize],
            ts: dialect != Dialect::JavaScript,
            in_extends: false,
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

fn is_space_byte(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn is_identifier_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

/// The scanner's `scan_whitespace_and_comments` results.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ws {
    Reject,
    NoNewline,
    Accept,
    BlockLineTerminator,
}

impl<'a> P<'a> {
    // ---- Tokens.

    /// The buffer holds the token `k` ahead.
    #[inline(always)]
    fn fill(&mut self, k: usize) {
        while self.toks.len() <= self.i + k {
            self.lex_one();
        }
    }

    #[cold]
    fn lex_one(&mut self) {
        if let Some(t) = self.toks.last().filter(|t| t.kind == T::Eof) {
            let t = *t;
            self.toks.push(t);
            return;
        }
        let t = match self.lx.token(self.next, &mut self.extras) {
            Ok(t) => t,
            // A token the lexer refuses is refused where the parser reads it: a look ahead past
            // a regular expression's start may read its text as tokens.
            Err(Refusal::Lex(at)) | Err(Refusal::Parse(at)) => Tok { kind: T::Eof, word: u8::MAX, nl: false, start: at, end: at },
            Err(Refusal::TooLarge) => Tok { kind: T::Eof, word: u8::MAX, nl: false, start: 0, end: 0 },
        };
        self.next = t.end as usize;
        self.toks.push(t);
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

    fn refuse<X>(&mut self) -> R<X> {
        let at = self.tok().start;
        self.far = self.far.max(at);
        Err(Refusal::Parse(at))
    }

    /// A refusal at byte `at` (a piece read in another mode).
    fn refuse_at<X>(&mut self, at: usize) -> R<X> {
        self.far = self.far.max(at as u32);
        Err(Refusal::Parse(at as u32))
    }

    /// The buffer cut at token `i`, and lexing resumed at `pos`: a piece read in another mode
    /// ends there. Extras from `pos` on are lexed again.
    fn relex_at(&mut self, pos: usize) {
        self.relexes += 1;
        self.toks.truncate(self.i);
        self.next = pos;
        let keep = self.extras.partition_point(|x| (x.start as usize) < pos);
        self.extras.truncate(keep);
        debug_assert!(self.e <= self.extras.len());
    }

    fn mark(&mut self) -> Mark {
        self.fill(0);
        Mark { i: self.i, n: self.w.len(), e: self.e, last_end: self.last_end, depth: self.depth, after: self.after, asi: self.asi, relexes: self.relexes }
    }

    /// Back to `m`. The tokens lexed since are kept where every one was lexed in normal mode
    /// (no `relex_at` since the mark): lexing has no state, so they are the tokens lexed again.
    /// Otherwise the token at the mark is kept and all after it lexed again.
    fn restore(&mut self, m: Mark) {
        self.i = m.i;
        if self.relexes != m.relexes {
            let t = self.toks[m.i];
            self.toks.truncate(m.i + 1);
            self.next = t.end as usize;
            let keep = self.extras.partition_point(|x| x.start < t.end);
            self.extras.truncate(keep);
        }
        self.e = m.e;
        self.w.truncate(m.n);
        self.last_end = m.last_end;
        self.depth = m.depth;
        self.after = m.after;
        self.asi = m.asi;
    }

    /// The extras before `before`, written into the node open.
    #[inline(always)]
    fn flush(&mut self, before: u32) {
        while let Some(x) = self.extras.get(self.e) {
            if x.start >= before {
                break;
            }
            let n = self.w.push(key(K::Comment, f::NONE, C_NONE, true), x.start);
            self.w.close(n, x.end);
            self.e += 1;
        }
    }

    /// The extras before the next token written: where a node opened there goes.
    #[inline(always)]
    fn here(&mut self) -> usize {
        let s = self.tok().start;
        self.flush(s);
        self.w.len()
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

    /// Whether the dialect's tapes store token `tok` under `parent` in `field`: a token other than
    /// a delimiter is stored wherever its kind does not imply it (the engine's automata,
    /// `kinds::DROPPED`); a delimiter the table names is implied only in some gaps, which its
    /// caller tells apart.
    #[inline]
    fn stores(&self, parent: K, tok: K, field: u8) -> bool {
        self.dropped.binary_search(&(parent as u32 | (tok as u32) << 9 | u32::from(field) << 18)).is_err()
    }

    /// The next token, `tok`, under `parent`: a leaf where the tape stores it.
    #[inline]
    fn tk(&mut self, parent: K, tok: K, field: u8) -> R<()> {
        if self.stores(parent, tok, field) {
            self.leaf(tok, field, C_NONE)?;
        } else {
            self.bump()?;
        }
        Ok(())
    }

    /// `?.` as an `optional_chain`: a leaf in JavaScript, a node holding its `?.` token in
    /// TypeScript (whose grammar reads `?.` as a token elsewhere too).
    fn optional_chain(&mut self) -> R<()> {
        if self.dialect == Dialect::JavaScript {
            self.leaf(K::OptionalChain, f::OPTIONAL_CHAIN, C_NONE)?;
            return Ok(());
        }
        let n = self.open(K::OptionalChain, f::OPTIONAL_CHAIN, C_NONE);
        self.leaf(K::TQDot, f::NONE, C_NONE)?;
        self.close(n);
        Ok(())
    }

    /// A leaf over `start..end` (a piece read in another mode).
    fn leaf_span(&mut self, kind: K, field: u8, start: u32, end: u32) {
        self.flush(start);
        let at = self.w.push(key(kind, field, C_NONE, false), start);
        self.w.close(at, end);
        self.last_end = end;
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

    /// A node opened at `start` (a piece read in another mode).
    fn open_span(&mut self, kind: K, field: u8, start: u32) -> N {
        self.flush(start);
        N { at: self.w.push(key(kind, field, C_NONE, false), start), kind }
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

    fn deeper(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.refuse();
        }
        Ok(())
    }

    // ---- Automatic semicolons: tree-sitter-javascript's scanner, ported.

    /// The character at `p` and its length (a byte past 0x80: decoded).
    fn ch(&self, p: usize) -> (u32, usize) {
        match self.src.get(p) {
            None => (0, 0),
            Some(&c) if c < 0x80 => (u32::from(c), 1),
            Some(_) => {
                let end = (p + 4).min(self.src.len());
                let s = match std::str::from_utf8(&self.src[p..end]) {
                    Ok(s) => s,
                    Err(e) => std::str::from_utf8(&self.src[p..p + e.valid_up_to()]).unwrap_or(""),
                };
                s.chars().next().map_or((0xfffd, 1), |c| (c as u32, c.len_utf8()))
            }
        }
    }

    fn is_space(c: u32) -> bool {
        matches!(c, 0x09..=0x0d | 0x20 | 0x85 | 0xa0 | 0x1680 | 0x200b | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x2060 | 0x3000 | 0xfeff) || (0x2000..=0x200a).contains(&c)
    }

    fn is_lt(c: u32) -> bool {
        matches!(c, 0x0a | 0x0d | 0x2028 | 0x2029)
    }

    fn at(&self, p: usize) -> u8 {
        self.src.get(p).copied().unwrap_or(0)
    }

    /// `scan_whitespace_and_comments`: the result and where it stopped.
    fn ws_comments(&self, mut p: usize, consume: bool, scanned: &mut bool) -> (Ws, usize) {
        let mut saw_block_newline = false;
        loop {
            loop {
                let (c, n) = self.ch(p);
                if n == 0 || !Self::is_space(c) {
                    break;
                }
                p += n;
            }
            if self.at(p) != b'/' {
                return (Ws::Accept, p);
            }
            p += 1;
            match self.at(p) {
                b'/' => {
                    p += 1;
                    loop {
                        let (c, n) = self.ch(p);
                        if n == 0 || Self::is_lt(c) {
                            break;
                        }
                        p += n;
                    }
                    *scanned = true;
                }
                b'*' => {
                    p += 1;
                    while p < self.src.len() {
                        if self.at(p) == b'*' {
                            p += 1;
                            if self.at(p) == b'/' {
                                p += 1;
                                *scanned = true;
                                if self.at(p) != b'/' && !consume {
                                    return (if saw_block_newline { Ws::BlockLineTerminator } else { Ws::NoNewline }, p);
                                }
                                break;
                            }
                        } else {
                            let (c, n) = self.ch(p);
                            if Self::is_lt(c) {
                                saw_block_newline = true;
                            }
                            p += n.max(1);
                        }
                    }
                }
                _ => return (Ws::Reject, p),
            }
        }
    }

    /// `scan_automatic_semicolon` from `p`: tree-sitter-javascript's, or -typescript's.
    fn scan_asi(&self, p: usize, v: V, scanned: &mut bool) -> bool {
        if self.dialect == Dialect::JavaScript { self.scan_asi_js(p, v, scanned) } else { self.scan_asi_ts(p, v) }
    }

    /// tree-sitter-typescript's `scan_automatic_semicolon` (`common/scanner.h`, patched) from `p`.
    fn scan_asi_ts(&self, mut p: usize, v: V) -> bool {
        let or = v & V_LOGICAL_OR != 0;
        loop {
            if p >= self.src.len() {
                return true;
            }
            let c = self.at(p);
            if c == b'}' {
                // `({a}: {a: number}) => …`: no semicolon before a type's `:`.
                p += 1;
                loop {
                    let (c, n) = self.ch(p);
                    if n == 0 || !Self::is_space(c) {
                        break;
                    }
                    p += n;
                }
                if self.at(p) == b':' {
                    return or;
                }
                return true;
            }
            if c == b'/' {
                // A block comment holding a line terminator reads as one; any other comment, no.
                if self.at(p + 1) != b'*' {
                    return false;
                }
                let mut q = p + 2;
                let mut saw = false;
                loop {
                    if q >= self.src.len() {
                        return false;
                    }
                    if self.at(q) == b'*' && self.at(q + 1) == b'/' {
                        q += 2;
                        break;
                    }
                    let (c, n) = self.ch(q);
                    saw |= Self::is_lt(c);
                    q += n.max(1);
                }
                if !saw {
                    return false;
                }
                p = q;
                break;
            }
            let (c, n) = self.ch(p);
            if !Self::is_space(c) {
                return false;
            }
            p += n;
            if Self::is_lt(c) {
                break;
            }
        }
        let restricted = v & V_NO_LT != 0;
        // `scan_whitespace_and_comments`.
        loop {
            loop {
                let (c, n) = self.ch(p);
                if n == 0 || !Self::is_space(c) {
                    break;
                }
                p += n;
            }
            if self.at(p) != b'/' {
                break;
            }
            match self.at(p + 1) {
                b'/' => {
                    p += 2;
                    loop {
                        let (c, n) = self.ch(p);
                        if n == 0 || Self::is_lt(c) {
                            break;
                        }
                        p += n;
                    }
                }
                b'*' => {
                    p += 2;
                    while p < self.src.len() {
                        if self.at(p) == b'*' && self.at(p + 1) == b'/' {
                            p += 2;
                            break;
                        }
                        p += self.ch(p).1.max(1);
                    }
                }
                _ => return restricted,
            }
        }
        let c = self.at(p);
        if restricted && matches!(c, b'`' | b'<' | b'[' | b'(' | b'*' | b'+' | b'-') {
            return true;
        }
        if v & V_ARROW_BODY_END != 0 {
            return c != b',' && c != b':' && c != b';';
        }
        if v & V_MEMBER_NAME != 0 && (is_identifier_char(c) || matches!(c, b'\\' | b'#' | b'[' | b'"' | b'\'') || c > 0x7f) {
            return false;
        }
        match c {
            b'`' | b',' | b'.' | b';' | b'%' | b'>' | b'=' | b'?' | b'^' | b'|' | b'&' | b'/' | b':' => false,
            b'<' => !or,
            b'{' if v & V_FN_SIG != 0 => false,
            b'(' | b'[' | b'*' if or => false,
            b'+' => self.at(p + 1) == b'+' || !or,
            b'-' => self.at(p + 1) == b'-' || !or,
            b'!' => self.at(p + 1) != b'=',
            b'i' => {
                if self.at(p + 1) != b'n' {
                    return true;
                }
                if !is_identifier_char(self.at(p + 2)) {
                    // `in` then `:` or `?`: a member named `in`.
                    let mut q = p + 2;
                    while matches!(self.at(q), b' ' | b'\t') {
                        q += 1;
                    }
                    return !or && matches!(self.at(q), b':' | b'?');
                }
                if self.src.get(p + 2..p + 10) != Some(b"stanceof") {
                    return true;
                }
                if !is_identifier_char(self.at(p + 10)) {
                    return !or;
                }
                true
            }
            _ => true,
        }
    }

    /// tree-sitter-javascript's `scan_automatic_semicolon` from `p`.
    fn scan_asi_js(&self, mut p: usize, v: V, scanned: &mut bool) -> bool {
        let comment_condition = v & V_LOGICAL_OR == 0;
        loop {
            if p >= self.src.len() {
                return true;
            }
            if self.at(p) == b'/' {
                let (r, q) = self.ws_comments(p, false, scanned);
                if r == Ws::Reject {
                    return false;
                }
                p = q;
                let blt = r == Ws::BlockLineTerminator;
                if (r == Ws::Accept || blt) && comment_condition && self.at(p) != b',' && self.at(p) != b'=' {
                    return true;
                }
                if blt {
                    return self.after_line_terminator(p, v, scanned);
                }
                if p >= self.src.len() {
                    return true;
                }
            }
            if self.at(p) == b'}' {
                return true;
            }
            let (c, n) = self.ch(p);
            if Self::is_lt(c) {
                p += n;
                break;
            }
            if !Self::is_space(c) {
                return false;
            }
            p += n;
        }
        self.after_line_terminator(p, v, scanned)
    }

    /// `scan_after_line_terminator` from `p`.
    fn after_line_terminator(&self, p: usize, v: V, scanned: &mut bool) -> bool {
        let restricted = v & V_NO_LT != 0;
        let (r, p) = self.ws_comments(p, true, scanned);
        if r == Ws::Reject {
            return restricted;
        }
        let c = self.at(p);
        if v & V_ARROW_BODY_END != 0 {
            return c != b',' && c != b':' && c != b';';
        }
        if v & V_MEMBER_NAME != 0 && (is_identifier_char(c) || matches!(c, b'\\' | b'#' | b'"' | b'\'') || c > 0x7f) {
            return false;
        }
        match c {
            b',' | b':' | b';' | b'%' | b'>' | b'=' | b'?' | b'^' | b'|' | b'&' | b'/' => false,
            b'`' | b'<' => restricted,
            b'[' => restricted || v & V_LBRACKET == 0,
            b'(' => restricted || v & V_LPAREN == 0,
            b'*' => restricted || v & V_STAR == 0,
            b'.' => self.at(p + 1).is_ascii_digit(),
            b'+' => self.at(p + 1) == b'+' || restricted || v & V_PLUS == 0,
            b'-' => self.at(p + 1) == b'-' || restricted || v & V_MINUS == 0,
            b'!' => self.at(p + 1) != b'=',
            b'i' => {
                if self.at(p + 1) != b'n' {
                    return true;
                }
                if !is_identifier_char(self.at(p + 2)) {
                    return false;
                }
                if &self.src[(p + 2).min(self.src.len())..(p + 10).min(self.src.len())] != b"stanceof" {
                    return true;
                }
                if !is_identifier_char(self.at(p + 10)) {
                    return v & V_LOGICAL_OR == 0;
                }
                true
            }
            _ => true,
        }
    }

    /// Whether tree-sitter's scanner gives an automatic semicolon after the last token read, where
    /// the parse state's valid symbols are `v`: at the last token's end, and again after each
    /// comment before the next token (the lexer reads a comment as an extra, and the scanner runs
    /// again at its end).
    fn asi(&mut self, v: V) -> bool {
        self.asi_at(v).is_some()
    }

    /// `asi`, and where the semicolon stands: the last token's end, or a comment's after it (the
    /// comments before it are then in the statement).
    fn asi_at(&mut self, v: V) -> Option<u32> {
        let t = self.tok();
        if !t.nl && !matches!(t.kind, T::RBrace | T::Eof) && self.extras.get(self.e).is_none_or(|x| x.start >= t.start) {
            return None;
        }
        let mut scanned = false;
        if self.scan_asi(self.last_end as usize, v, &mut scanned) {
            return Some(self.last_end);
        }
        let mut k = self.e;
        while let Some(x) = self.extras.get(k).copied() {
            if x.start >= t.start {
                break;
            }
            let mut s = false;
            if self.scan_asi(x.end as usize, v, &mut s) {
                return Some(x.end);
            }
            k += 1;
        }
        None
    }

    /// An automatic semicolon read: the comments before it written, the statement ending there.
    fn asi_end(&mut self, v: V) -> bool {
        match self.asi_at(v) {
            Some(at) => {
                if at > self.last_end {
                    self.flush(at);
                    self.last_end = at;
                }
                true
            }
            None => false,
        }
    }

    /// Whether the expression read stops before the next token, on an automatic semicolon.
    #[inline(always)]
    fn stops(&mut self) -> bool {
        if !self.asi {
            return false;
        }
        let t = self.tok();
        (t.nl || self.extras.get(self.e).is_some_and(|x| x.start < t.start)) && self.asi(V_EXPR)
    }

    /// A statement's end: its `;`, or an automatic semicolon (after what `self.after` says).
    fn semicolon(&mut self) -> R<()> {
        if self.peek() == T::Semi {
            self.leaf(K::TSemi, f::NONE, C_NONE)?;
            return Ok(());
        }
        let v = self.after;
        if self.asi_end(v) {
            return Ok(());
        }
        self.refuse()
    }

    // ---- The program and statements.

    fn program(&mut self) -> R<()> {
        let len = self.src.len() as u32;
        // `#!` on the first line.
        let mut hash_bang = None;
        if self.src.starts_with(b"#!") {
            let end = self.src.iter().position(|&c| c == b'\n' || c == b'\r').unwrap_or(self.src.len());
            hash_bang = Some(end as u32);
            self.next = end;
        }
        let first = self.tok();
        let first_start = if first.kind == T::Eof { len } else { first.start };
        let start = if hash_bang.is_some() { 0 } else { self.extras.first().map_or(first_start, |x| x.start.min(first_start)) };
        let m = self.w.push(key(K::Program, f::NONE, C_NONE, false), start);
        if let Some(end) = hash_bang {
            let n = self.w.push(key(K::HashBangLine, f::NONE, C_NONE, false), 0);
            self.w.close(n, end);
        }
        while self.peek() != T::Eof {
            self.statement(slot(f::NONE, Ctx::Stmt))?;
            self.w.commit();
        }
        // The text's end: a token the lexer refused ends it early.
        let t = self.tok();
        if t.word == u8::MAX {
            return Err(Refusal::Lex(t.start));
        }
        self.flush(u32::MAX);
        self.w.close(m, len);
        Ok(())
    }

    fn statement(&mut self, s: Slot) -> R<()> {
        self.deeper()?;
        let saved = self.asi;
        self.asi = true;
        self.statement_inner(s)?;
        self.asi = saved;
        self.depth -= 1;
        Ok(())
    }

    fn statement_inner(&mut self, s: Slot) -> R<()> {
        let t = self.tok();
        match t.kind {
            T::Semi => {
                let n = self.open_in(K::EmptyStatement, s);
                self.leaf(K::TSemi, f::NONE, C_NONE)?;
                self.close(n);
                return Ok(());
            }
            T::LBrace => return self.statement_block(s, false),
            T::At => return self.class_declaration(s),
            T::Name => {}
            _ => return self.expression_statement(s),
        }
        if self.ts {
            if let Some(r) = self.ts_statement(s, t) {
                return r;
            }
        }
        match t.word {
            w::VAR => self.variable_declaration(s),
            w::CONST => self.lexical_declaration(s, K::TConst),
            w::LET if self.let_declares() => self.lexical_declaration(s, K::TLet),
            w::IF => self.if_statement(s),
            w::FOR => self.for_statement(s),
            w::WHILE => {
                let n = self.open_in(K::WhileStatement, s);
                self.bump()?;
                self.parenthesized(slot(f::CONDITION, Ctx::Bare))?;
                self.statement(slot(f::BODY, Ctx::Stmt))?;
                self.close(n);
                Ok(())
            }
            w::DO => {
                let n = self.open_in(K::DoStatement, s);
                self.bump()?;
                self.statement(slot(f::BODY, Ctx::Stmt))?;
                if !self.is_word(w::WHILE) {
                    return self.refuse();
                }
                self.bump()?;
                self.parenthesized(slot(f::CONDITION, Ctx::Bare))?;
                // `choice(_semicolon, blank)`.
                if self.peek() == T::Semi {
                    self.leaf(K::TSemi, f::NONE, C_NONE)?;
                } else {
                    self.after = V_EXPR;
                    if !self.asi_end(V_EXPR) && !matches!(self.peek(), T::RBrace | T::Eof) {
                        return self.refuse();
                    }
                }
                self.close(n);
                Ok(())
            }
            w::TRY => self.try_statement(s),
            w::SWITCH => self.switch_statement(s),
            w::RETURN => {
                let n = self.open_in(K::ReturnStatement, s);
                self.tk(K::ReturnStatement, K::TReturn, f::NONE)?;
                self.after = V_NO_LT;
                if self.peek() != T::Semi && !self.asi(V_NO_LT) {
                    self.expressions(EXPR)?;
                }
                self.semicolon()?;
                self.close(n);
                Ok(())
            }
            w::THROW => {
                let n = self.open_in(K::ThrowStatement, s);
                self.bump()?;
                self.expressions(EXPR)?;
                self.semicolon()?;
                self.close(n);
                Ok(())
            }
            w::BREAK | w::CONTINUE => {
                let (kind, tk) = if t.word == w::BREAK { (K::BreakStatement, K::TBreak) } else { (K::ContinueStatement, K::TContinue) };
                let n = self.open_in(kind, s);
                self.tk(kind, tk, f::NONE)?;
                self.after = 0;
                if self.peek() == T::Name && !self.asi(0) {
                    self.leaf(K::StatementIdentifier, f::LABEL, C_NONE)?;
                }
                self.semicolon()?;
                self.close(n);
                Ok(())
            }
            w::DEBUGGER => {
                let n = self.open_in(K::DebuggerStatement, s);
                self.bump()?;
                self.after = 0;
                self.semicolon()?;
                self.close(n);
                Ok(())
            }
            w::WITH => {
                let n = self.open_in(K::WithStatement, s);
                self.bump()?;
                self.parenthesized(slot(f::OBJECT, Ctx::Bare))?;
                self.statement(slot(f::BODY, Ctx::Stmt))?;
                self.close(n);
                Ok(())
            }
            w::FUNCTION => self.function(s, true, false),
            w::ASYNC if self.is_word_at(1, w::FUNCTION) && !self.peek_at(1).nl => self.function(s, true, true),
            w::CLASS => self.class_declaration(s),
            w::IMPORT if !matches!(self.peek_at(1).kind, T::LParen | T::Dot) => self.import_statement(s),
            w::EXPORT => self.export_statement(s),
            _ if self.peek_at(1).kind == T::Colon && t.word != w::DEFAULT => self.labeled_statement(s),
            _ => self.expression_statement(s),
        }
    }

    /// Whether a `let` starts a declaration: a name, `[` or `{` follows (else `let` is a name).
    fn let_declares(&mut self) -> bool {
        let n = self.peek_at(1);
        match n.kind {
            T::LBrace | T::LBracket => !n.nl || true,
            T::Name => !matches!(n.word, w::IN | w::INSTANCEOF) || n.nl,
            _ => false,
        }
    }

    fn expression_statement(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::ExpressionStatement, s);
        self.expressions(EXPR)?;
        self.semicolon()?;
        self.close(n);
        Ok(())
    }

    fn labeled_statement(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::LabeledStatement, s);
        self.leaf(K::StatementIdentifier, f::LABEL, C_NONE)?;
        self.expect(T::Colon)?;
        self.statement(slot(f::BODY, Ctx::Stmt))?;
        self.close(n);
        Ok(())
    }

    /// `{ statement* }`; `body` for a function's (no supertype).
    fn statement_block(&mut self, s: Slot, _function: bool) -> R<()> {
        let n = self.open_in(K::StatementBlock, s);
        self.bump()?;
        let saved = self.asi;
        while !matches!(self.peek(), T::RBrace | T::Eof) {
            self.statement(slot(f::NONE, Ctx::Stmt))?;
        }
        self.expect(T::RBrace)?;
        self.asi = saved;
        self.close(n);
        self.after = V_EXPR;
        Ok(())
    }

    fn parenthesized(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ParenthesizedExpression, s);
        self.bump()?;
        let saved = self.asi;
        self.asi = false;
        self.expressions(EXPR)?;
        self.asi = saved;
        self.expect(T::RParen)?;
        self.close(n);
        Ok(n)
    }

    fn variable_declaration(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::VariableDeclaration, s);
        self.bump()?;
        self.declarators()?;
        self.semicolon()?;
        self.close(n);
        Ok(())
    }

    fn lexical_declaration(&mut self, s: Slot, kind: K) -> R<()> {
        let n = self.open_in(K::LexicalDeclaration, s);
        self.tk(K::LexicalDeclaration, kind, f::KIND)?;
        self.declarators()?;
        self.semicolon()?;
        self.close(n);
        Ok(())
    }

    fn declarators(&mut self) -> R<()> {
        loop {
            self.declarator()?;
            if self.peek() != T::Comma {
                return Ok(());
            }
            self.bump()?;
        }
    }

    fn declarator(&mut self) -> R<()> {
        let n = self.open(K::VariableDeclarator, f::NONE, C_NONE);
        self.binding(slot(f::NAME, Ctx::Bare))?;
        self.after = 0;
        if self.ts {
            if self.peek() == T::Bang {
                self.tk(K::VariableDeclarator, K::TBang, f::NONE)?;
            }
            if self.peek() == T::Colon {
                self.type_annotation(f::TYPE)?;
                self.after = 0;
            }
        }
        if self.peek() == T::Eq {
            self.bump()?;
            self.assignment(slot(f::VALUE, Ctx::Expr))?;
        }
        self.close(n);
        Ok(())
    }

    /// A declared name: an identifier or a destructuring pattern.
    fn binding(&mut self, s: Slot) -> R<()> {
        match self.peek() {
            T::LBrace => self.object_pattern(s),
            T::LBracket => self.array_pattern(s),
            T::Name => {
                let kind = if !self.ts && self.is_word(w::UNDEFINED) { K::Undefined } else { K::Identifier };
                self.leaf_in(kind, s)?;
                Ok(())
            }
            _ => self.refuse(),
        }
    }

    fn if_statement(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::IfStatement, s);
        self.bump()?;
        self.parenthesized(slot(f::CONDITION, Ctx::Bare))?;
        self.statement(slot(f::CONSEQUENCE, Ctx::Stmt))?;
        if self.is_word(w::ELSE) {
            let e = self.open(K::ElseClause, f::ALTERNATIVE, C_NONE);
            self.bump()?;
            self.statement(slot(f::NONE, Ctx::Stmt))?;
            self.close(e);
        }
        self.close(n);
        Ok(())
    }

    fn try_statement(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::TryStatement, s);
        self.bump()?;
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        self.statement_block(slot(f::BODY, Ctx::Bare), false)?;
        if self.is_word(w::CATCH) {
            let c = self.open(K::CatchClause, f::HANDLER, C_NONE);
            self.bump()?;
            if self.peek() == T::LParen {
                self.bump()?;
                self.binding(slot(f::PARAMETER, Ctx::Bare))?;
                if self.ts && self.peek() == T::Colon {
                    self.type_annotation(f::TYPE)?;
                }
                self.expect(T::RParen)?;
            }
            if self.peek() != T::LBrace {
                return self.refuse();
            }
            self.statement_block(slot(f::BODY, Ctx::Bare), false)?;
            self.close(c);
        }
        if self.is_word(w::FINALLY) {
            let c = self.open(K::FinallyClause, f::FINALIZER, C_NONE);
            self.bump()?;
            if self.peek() != T::LBrace {
                return self.refuse();
            }
            self.statement_block(slot(f::BODY, Ctx::Bare), false)?;
            self.close(c);
        }
        self.close(n);
        Ok(())
    }

    fn switch_statement(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::SwitchStatement, s);
        self.bump()?;
        self.parenthesized(slot(f::VALUE, Ctx::Bare))?;
        let b = self.open(K::SwitchBody, f::BODY, C_NONE);
        self.expect(T::LBrace)?;
        loop {
            if self.is_word(w::CASE) {
                let c = self.open(K::SwitchCase, f::NONE, C_NONE);
                self.bump()?;
                let saved = self.asi;
                self.asi = false;
                self.expressions(slot(f::VALUE, Ctx::Expr))?;
                self.asi = saved;
                self.expect(T::Colon)?;
                self.case_body()?;
                self.close(c);
            } else if self.is_word(w::DEFAULT) {
                let c = self.open(K::SwitchDefault, f::NONE, C_NONE);
                self.bump()?;
                self.expect(T::Colon)?;
                self.case_body()?;
                self.close(c);
            } else {
                break;
            }
        }
        self.expect(T::RBrace)?;
        self.close(b);
        self.close(n);
        Ok(())
    }

    fn case_body(&mut self) -> R<()> {
        while !matches!(self.peek(), T::RBrace | T::Eof) && !self.is_word(w::CASE) && !(self.is_word(w::DEFAULT) && self.peek_at(1).kind == T::Colon) {
            self.statement(slot(f::BODY, Ctx::Stmt))?;
        }
        Ok(())
    }

    fn for_statement(&mut self, s: Slot) -> R<()> {
        // `for await (` and `for (` with `in`/`of`: a `for_in_statement`; else `for_statement`.
        let m = self.mark();
        let for_tok = self.tok();
        let is_await = self.is_word_at(1, w::AWAIT);
        if is_await || self.for_in_head()? {
            self.restore(m);
            let n = self.open_in(K::ForInStatement, s);
            self.tk(K::ForInStatement, K::TFor, f::NONE)?;
            if is_await {
                self.tk(K::ForInStatement, K::TAwait, f::NONE)?;
            }
            // The `(` is stored after `await`, or before a declaration's kind.
            let declares = self.is_word_at(1, w::VAR) || self.is_word_at(1, w::CONST) || (self.is_word_at(1, w::LET) && {
                self.i += 1;
                let d = self.let_declares();
                self.i -= 1;
                d
            });
            if is_await || declares {
                self.leaf(K::TLParen, f::NONE, C_NONE)?;
            } else {
                self.bump()?;
            }
            let saved = self.asi;
            self.asi = false;
            if self.is_word(w::VAR) || self.is_word(w::CONST) || (self.is_word(w::LET) && self.let_declares()) {
                let k = match self.tok().word {
                    w::VAR => K::TVar,
                    w::CONST => K::TConst,
                    _ => K::TLet,
                };
                self.tk(K::ForInStatement, k, f::KIND)?;
                self.binding(slot(f::LEFT, Ctx::Bare))?;
            } else {
                self.lhs(slot(f::LEFT, Ctx::Bare))?;
            }
            if self.is_word(w::IN) {
                self.tk(K::ForInStatement, K::TIn, f::OPERATOR)?;
                self.expressions(slot(f::RIGHT, Ctx::Expr))?;
            } else if self.is_word(w::OF) {
                self.tk(K::ForInStatement, K::TOf, f::OPERATOR)?;
                self.assignment(slot(f::RIGHT, Ctx::Expr))?;
            } else {
                return self.refuse();
            }
            self.expect(T::RParen)?;
            self.asi = saved;
            self.statement(slot(f::BODY, Ctx::Stmt))?;
            self.close(n);
            return Ok(());
        }
        self.restore(m);
        let _ = for_tok;
        let n = self.open_in(K::ForStatement, s);
        self.bump()?;
        self.expect(T::LParen)?;
        let saved = self.asi;
        self.asi = false;
        // The initializer: a declaration (its `;` its own), expressions and `;`, or `;`.
        if self.peek() == T::Semi {
            let e = self.open(K::EmptyStatement, f::INITIALIZER, C_NONE);
            self.leaf(K::TSemi, f::NONE, C_NONE)?;
            self.close(e);
        } else if self.is_word(w::VAR) {
            let d = self.open(K::VariableDeclaration, f::INITIALIZER, C_NONE);
            self.bump()?;
            self.declarators()?;
            self.leaf(K::TSemi, f::NONE, C_NONE).and_then(|_| Ok(())).or_else(|_| self.refuse())?;
            self.close(d);
        } else if self.is_word(w::CONST) || (self.is_word(w::LET) && self.let_declares()) {
            let d = self.open(K::LexicalDeclaration, f::INITIALIZER, C_NONE);
            let k = if self.is_word(w::CONST) { K::TConst } else { K::TLet };
            self.tk(K::LexicalDeclaration, k, f::KIND)?;
            self.declarators()?;
            if self.peek() != T::Semi {
                return self.refuse();
            }
            self.leaf(K::TSemi, f::NONE, C_NONE)?;
            self.close(d);
        } else {
            self.expressions(slot(f::INITIALIZER, Ctx::Expr))?;
            self.expect(T::Semi)?;
        }
        // The condition: expressions and `;`, or `;` (an empty statement).
        if self.peek() == T::Semi {
            let e = self.open(K::EmptyStatement, f::CONDITION, C_NONE);
            self.leaf(K::TSemi, f::NONE, C_NONE)?;
            self.close(e);
        } else {
            self.expressions(slot(f::CONDITION, Ctx::Expr))?;
            self.expect(T::Semi)?;
        }
        if self.peek() != T::RParen {
            self.expressions(slot(f::INCREMENT, Ctx::Expr))?;
        }
        self.expect(T::RParen)?;
        self.asi = saved;
        self.statement(slot(f::BODY, Ctx::Stmt))?;
        self.close(n);
        Ok(())
    }

    /// After `for`: whether its head is `( left in|of right )` (read and discarded).
    fn for_in_head(&mut self) -> R<bool> {
        self.bump()?;
        if self.peek() != T::LParen {
            return self.refuse();
        }
        self.bump()?;
        self.asi = false;
        if self.is_word(w::VAR) || self.is_word(w::CONST) || (self.is_word(w::LET) && self.let_declares()) {
            self.bump()?;
            if self.binding(BARE).is_err() {
                return Ok(false);
            }
        } else if self.lhs(BARE).is_err() {
            return Ok(false);
        }
        Ok(self.is_word(w::IN) || self.is_word(w::OF))
    }

    /// A `for … in`'s left side: `_lhs_expression` or a parenthesized expression.
    fn lhs(&mut self, s: Slot) -> R<()> {
        match self.peek() {
            T::LBrace => self.object_pattern(s),
            T::LBracket => self.array_pattern(s),
            T::LParen => {
                self.parenthesized(s)?;
                Ok(())
            }
            _ => {
                // A name, member or subscript, read without `in`.
                let m = self.mark();
                let k = self.unary(s, true)?;
                if !matches!(k.kind, K::Identifier | K::Undefined | K::MemberExpression | K::SubscriptExpression) {
                    self.restore(m);
                    return self.refuse();
                }
                self.rekey(k.at, s);
                Ok(())
            }
        }
    }

    // ---- Functions and classes.

    /// A function declaration (or expression, `decl` false), `async` before it if `is_async`.
    fn function(&mut self, s: Slot, decl: bool, is_async: bool) -> R<()> {
        // `function*`?
        let k0 = usize::from(is_async);
        let star = self.peek_at(k0 + 1).kind == T::Star;
        let kind = match (decl, star) {
            (true, false) => K::FunctionDeclaration,
            (true, true) => K::GeneratorFunctionDeclaration,
            (false, false) => K::FunctionExpression,
            (false, true) => K::GeneratorFunction,
        };
        let n = self.open_in(kind, s);
        if is_async {
            self.tk(kind, K::TAsync, f::NONE)?;
        }
        self.tk(kind, K::TFunction, f::NONE)?;
        if star {
            self.tk(kind, K::TStar, f::NONE)?;
        }
        if self.peek() == T::Name {
            self.leaf(K::Identifier, f::NAME, C_NONE)?;
        } else if decl {
            return self.refuse();
        }
        self.signature()?;
        if self.peek() != T::LBrace {
            if !self.ts || !decl || star {
                return self.refuse();
            }
            // An overload's signature: `function_signature`, its `;` stored.
            self.rekey_kind(n.at, K::FunctionSignature);
            if self.peek() == T::Semi {
                self.leaf(K::TSemi, f::NONE, C_NONE)?;
            } else if !self.asi_end(V_FN_SIG) {
                return self.refuse();
            }
            self.close(n);
            return Ok(());
        }
        let saved = self.asi;
        self.statement_block(slot(f::BODY, Ctx::Bare), true)?;
        self.asi = saved;
        self.close(n);
        self.after = V_EXPR;
        Ok(())
    }

    /// A function's parameters, and in TypeScript its type parameters and return type.
    fn signature(&mut self) -> R<()> {
        if self.ts { self.call_signature() } else { self.formal_parameters() }
    }

    /// The node at `at` made a `kind`, its field and slot kept.
    fn rekey_kind(&mut self, at: usize, kind: K) {
        let (_, field, class, extra) = unkey(self.w.key(at));
        self.w.set_key(at, key(kind, field, class, extra));
    }

    fn formal_parameters(&mut self) -> R<()> {
        let n = self.open(K::FormalParameters, f::PARAMETERS, C_NONE);
        self.expect(T::LParen)?;
        let saved = self.asi;
        self.asi = false;
        while self.peek() != T::RParen {
            if self.ts {
                self.ts_parameter()?;
            } else {
                self.parameter()?;
            }
            if self.peek() == T::Comma {
                let c = self.tok();
                self.bump()?;
                if self.peek() == T::RParen {
                    // A trailing comma: stored, with the `)`.
                    self.leaf_tok(c, K::TComma);
                    self.leaf(K::TRParen, f::NONE, C_NONE)?;
                    self.asi = saved;
                    self.close(n);
                    return Ok(());
                }
            } else if self.peek() != T::RParen {
                return self.refuse();
            }
        }
        self.bump()?;
        self.asi = saved;
        self.close(n);
        Ok(())
    }

    /// A token read before, as a leaf now.
    fn leaf_tok(&mut self, t: Tok, kind: K) {
        let at = self.w.push(key(kind, f::NONE, C_NONE, false), t.start);
        self.w.close(at, t.end);
    }

    /// A formal parameter: a pattern, or a pattern with a default (`assignment_pattern`).
    fn parameter(&mut self) -> R<()> {
        let pat = slot(f::NONE, Ctx::Pattern);
        if self.peek() == T::Ellipsis {
            return self.rest_pattern(pat);
        }
        let start = self.here();
        let first = self.open_pattern_target(pat)?;
        if self.peek() == T::Eq {
            let n = N { at: start, kind: first };
            let a = self.wrap(n, K::AssignmentPattern, BARE, slot(f::LEFT, Ctx::Pattern));
            self.bump()?;
            self.assignment(slot(f::RIGHT, Ctx::Expr))?;
            self.close(a);
        }
        Ok(())
    }

    /// A pattern's target: a name, an object or array pattern. Its kind.
    fn open_pattern_target(&mut self, s: Slot) -> R<K> {
        match self.peek() {
            T::LBrace => {
                self.object_pattern(s)?;
                Ok(K::ObjectPattern)
            }
            T::LBracket => {
                self.array_pattern(s)?;
                Ok(K::ArrayPattern)
            }
            T::Name => {
                let k = self.peek_at(1).kind;
                if !matches!(k, T::Dot | T::QDot | T::LBracket) {
                    let kind = if self.is_word(w::UNDEFINED) { K::Undefined } else { K::Identifier };
                    self.leaf_in(kind, s)?;
                    return Ok(kind);
                }
                // A member or subscript (`_lhs_expression`).
                let e = self.postfix(s)?;
                if !matches!(e.kind, K::MemberExpression | K::SubscriptExpression) {
                    return self.refuse();
                }
                Ok(e.kind)
            }
            _ => self.refuse(),
        }
    }

    fn rest_pattern(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::RestPattern, s);
        self.bump()?;
        match self.peek() {
            T::LBrace => self.object_pattern(BARE)?,
            T::LBracket => self.array_pattern(BARE)?,
            T::Name => {
                let kind = if self.is_word(w::UNDEFINED) { K::Undefined } else { K::Identifier };
                self.leaf_in(kind, BARE)?;
            }
            _ => return self.refuse(),
        }
        self.close(n);
        Ok(())
    }

    fn object_pattern(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::ObjectPattern, s);
        self.bump()?;
        let saved = self.asi;
        self.asi = false;
        loop {
            match self.peek() {
                T::RBrace => break,
                T::Comma => return self.refuse(),
                T::Ellipsis => self.rest_pattern(BARE)?,
                _ => self.pattern_property()?,
            }
            if self.peek() == T::Comma {
                let c = self.tok();
                self.bump()?;
                if self.peek() == T::RBrace {
                    self.leaf_tok(c, K::TComma);
                    self.leaf(K::TRBrace, f::NONE, C_NONE)?;
                    self.asi = saved;
                    self.close(n);
                    return Ok(());
                }
            } else if self.peek() != T::RBrace {
                return self.refuse();
            }
        }
        self.bump()?;
        self.asi = saved;
        self.close(n);
        Ok(())
    }

    /// `pair_pattern`, `object_assignment_pattern` or a shorthand name.
    fn pattern_property(&mut self) -> R<()> {
        let t = self.tok();
        let next = self.peek_at(1).kind;
        if t.kind == T::Name && matches!(next, T::Comma | T::RBrace) {
            self.leaf(K::ShorthandPropertyIdentifierPattern, f::NONE, C_NONE)?;
            return Ok(());
        }
        if t.kind == T::Name && next == T::Eq {
            let n = self.open(K::ObjectAssignmentPattern, f::NONE, C_NONE);
            self.leaf(K::ShorthandPropertyIdentifierPattern, f::LEFT, C_NONE)?;
            self.bump()?;
            self.assignment(slot(f::RIGHT, Ctx::Expr))?;
            self.close(n);
            return Ok(());
        }
        let n = self.open(K::PairPattern, f::NONE, C_NONE);
        self.property_name(f::KEY)?;
        self.expect(T::Colon)?;
        let start = self.here();
        let first = self.open_pattern_target(slot(f::VALUE, Ctx::Pattern)).or_else(|_| self.refuse())?;
        if self.peek() == T::Eq {
            let v = N { at: start, kind: first };
            let a = self.wrap(v, K::AssignmentPattern, slot(f::VALUE, Ctx::Bare), slot(f::LEFT, Ctx::Pattern));
            self.bump()?;
            self.assignment(slot(f::RIGHT, Ctx::Expr))?;
            self.close(a);
        }
        self.close(n);
        Ok(())
    }

    fn array_pattern(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::ArrayPattern, s);
        self.bracket_list(n, Self::array_pattern_element)?;
        self.close(n);
        Ok(())
    }

    fn array_pattern_element(&mut self) -> R<()> {
        if self.peek() == T::Ellipsis {
            return self.rest_pattern(slot(f::NONE, Ctx::Pattern));
        }
        let start = self.here();
        let first = self.open_pattern_target(slot(f::NONE, Ctx::Pattern))?;
        if self.peek() == T::Eq {
            let v = N { at: start, kind: first };
            let a = self.wrap(v, K::AssignmentPattern, BARE, slot(f::LEFT, Ctx::Pattern));
            self.bump()?;
            self.assignment(slot(f::RIGHT, Ctx::Expr))?;
            self.close(a);
        }
        Ok(())
    }

    /// `[ elements ]` from the `[`, holes allowed (an array's or an array pattern's). The tape
    /// stores, for each hole, the delimiter before it (`[` or `,`) and its `,`; and after a
    /// trailing `,`, it and the `]`.
    fn bracket_list(&mut self, _n: N, element: fn(&mut Self) -> R<()>) -> R<()> {
        let mut delim = self.bump()?;
        let mut delim_kind = K::TLBracket;
        let mut written = false;
        let saved = self.asi;
        self.asi = false;
        loop {
            match self.peek() {
                T::RBracket => {
                    if delim_kind == K::TComma && !written {
                        // A trailing `,` after an element.
                        self.leaf_tok(delim, K::TComma);
                        self.leaf(K::TRBracket, f::NONE, C_NONE)?;
                    } else if delim_kind == K::TComma {
                        // After a hole's `,`: the `]` too.
                        self.leaf(K::TRBracket, f::NONE, C_NONE)?;
                    } else {
                        self.bump()?;
                    }
                    break;
                }
                T::Comma => {
                    // A hole.
                    if !written {
                        self.leaf_tok(delim, delim_kind);
                    }
                    delim = self.tok();
                    self.leaf(K::TComma, f::NONE, C_NONE)?;
                    delim_kind = K::TComma;
                    written = true;
                    continue;
                }
                _ => element(self)?,
            }
            match self.peek() {
                T::Comma => {
                    delim = self.bump()?;
                    delim_kind = K::TComma;
                    written = false;
                }
                T::RBracket => {
                    self.bump()?;
                    break;
                }
                _ => return self.refuse(),
            }
        }
        self.asi = saved;
        self.after = V_EXPR;
        Ok(())
    }

    fn class_declaration(&mut self, s: Slot) -> R<()> {
        if self.peek() == T::At {
            return self.refuse();
        }
        let kind = if self.is_word(w::ABSTRACT) { K::AbstractClassDeclaration } else { K::ClassDeclaration };
        let n = self.open_in(kind, s);
        if kind == K::AbstractClassDeclaration {
            self.tk(kind, K::TAbstract, f::NONE)?;
        }
        self.tk(kind, K::TClass, f::NONE)?;
        if self.peek() != T::Name || self.is_word(w::EXTENDS) || self.is_word(w::IMPLEMENTS) && self.ts {
            return self.refuse();
        }
        self.leaf(if self.ts { K::TypeIdentifier } else { K::Identifier }, f::NAME, C_NONE)?;
        self.class_rest()?;
        self.close(n);
        self.after = V_EXPR;
        Ok(())
    }

    /// `class_heritage?` and the body.
    fn class_rest(&mut self) -> R<()> {
        let saved = self.asi;
        self.asi = false;
        if self.ts && self.peek() == T::Lt {
            self.type_parameters()?;
        }
        if self.ts && (self.is_word(w::EXTENDS) || self.is_word(w::IMPLEMENTS)) {
            self.ts_class_heritage()?;
        } else if self.is_word(w::EXTENDS) {
            let h = self.open(K::ClassHeritage, f::NONE, C_NONE);
            self.bump()?;
            self.assignment(EXPR)?;
            self.close(h);
        }
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        let b = self.open(K::ClassBody, f::BODY, C_NONE);
        self.bump()?;
        loop {
            match self.peek() {
                T::RBrace => break,
                T::Semi => {
                    self.leaf(K::TSemi, f::NONE, C_NONE)?;
                }
                T::At => return self.refuse(),
                _ if self.ts => self.ts_class_member()?,
                _ => self.class_member()?,
            }
        }
        // The `}` is stored after a `;` (the body's last child).
        let last = self.last_child(b.at);
        if last > b.at && kind_of(unkey(self.w.key(last)).0) == K::TSemi {
            self.leaf(K::TRBrace, f::NONE, C_NONE)?;
        } else {
            self.bump()?;
        }
        self.close(b);
        self.asi = saved;
        Ok(())
    }

    fn class_member(&mut self) -> R<()> {
        // `static {`: a static block.
        if self.is_word(w::STATIC) && self.peek_at(1).kind == T::LBrace {
            let n = self.open(K::ClassStaticBlock, f::MEMBER, C_NONE);
            self.tk(K::ClassStaticBlock, K::TStatic, f::NONE)?;
            self.statement_block(slot(f::BODY, Ctx::Bare), true)?;
            self.close(n);
            return Ok(());
        }
        let start = self.here();
        let n = self.open(K::MethodDefinition, f::MEMBER, C_NONE);
        let mut is_static = false;
        let mut modifiers = 0;
        if self.is_word(w::STATIC) && self.member_modifier() {
            self.tk(K::MethodDefinition, K::TStatic, f::NONE)?;
            is_static = true;
            modifiers += 1;
        }
        let _ = is_static;
        if self.is_word(w::ASYNC) && self.member_modifier() && !self.peek_at(1).nl {
            self.tk(K::MethodDefinition, K::TAsync, f::NONE)?;
            modifiers += 1;
        }
        let mut star = false;
        if (self.is_word(w::GET) || self.is_word(w::SET)) && self.member_modifier() {
            let k = if self.is_word(w::GET) { K::TGet } else { K::TSet };
            self.tk(K::MethodDefinition, k, f::NONE)?;
            modifiers += 1;
        } else if self.peek() == T::Star {
            self.tk(K::MethodDefinition, K::TStar, f::NONE)?;
            star = true;
        }
        let _ = (modifiers, star);
        self.property_name(f::NAME)?;
        if self.peek() == T::LParen {
            self.formal_parameters()?;
            if self.peek() != T::LBrace {
                return self.refuse();
            }
            self.statement_block(slot(f::BODY, Ctx::Bare), true)?;
            if self.peek() == T::Semi {
                // `seq(method_definition, optional(';'))`: the `;` is the class body's.
                self.close(n);
                self.leaf(K::TSemi, f::NONE, C_NONE)?;
                return Ok(());
            }
            self.close(n);
            return Ok(());
        }
        // A field: `static`? name, `= value`?, `;`.
        if star || self.w.len() > start + 1 && !self.field_after_modifiers(start) {
            return self.refuse();
        }
        self.w.set_key(n.at, key(K::FieldDefinition, f::MEMBER, C_NONE, false));
        // The name's field: `property`.
        let name_at = self.last_child(n.at);
        self.retag_field(name_at, f::PROPERTY);
        self.after = 0;
        if self.peek() == T::Eq {
            self.bump()?;
            let saved = self.asi;
            self.asi = true;
            self.assignment(slot(f::VALUE, Ctx::Expr))?;
            self.asi = saved;
        } else {
            self.after = V_LPAREN;
        }
        self.close(n);
        self.semicolon_in_class()
    }

    /// A field's modifiers: `static` alone.
    fn field_after_modifiers(&self, start: usize) -> bool {
        // The children before the name: at most `static`.
        let end = self.w.len();
        let mut c = start + 1;
        let mut n = 0;
        while c < end {
            n += 1;
            c += self.w.size(c).max(1) as usize;
        }
        n <= 2 && (n == 1 || kind_of(unkey(self.w.key(start + 1)).0) == K::TStatic)
    }

    /// The last child of the open node `at`.
    fn last_child(&self, at: usize) -> usize {
        let end = self.w.len();
        let mut c = at + 1;
        let mut last = c;
        while c < end {
            last = c;
            c += self.w.size(c).max(1) as usize;
        }
        last
    }

    fn retag_field(&mut self, at: usize, field: u8) {
        let (k, _, class, extra) = unkey(self.w.key(at));
        self.w.set_key(at, key(kind_of(k), field, class, extra));
    }

    /// A class field's `;`: stored where written, else an automatic semicolon.
    fn semicolon_in_class(&mut self) -> R<()> {
        if self.peek() == T::Semi {
            self.leaf(K::TSemi, f::NONE, C_NONE)?;
            return Ok(());
        }
        let v = self.after;
        if self.asi_end(v) {
            return Ok(());
        }
        self.refuse()
    }

    /// Whether the word read (`static`, `async`, `get`, `set`) modifies the member: a name, `[`,
    /// `*`, a string or a number follows (else it is the member's name).
    fn member_modifier(&mut self) -> bool {
        let n = self.peek_at(1);
        match n.kind {
            T::Name | T::Private | T::Str | T::Number | T::LBracket => true,
            T::Star => true,
            _ => false,
        }
    }

    /// A property's name: an identifier, `#name`, a string, a number or `[expression]`.
    fn property_name(&mut self, field: u8) -> R<()> {
        match self.peek() {
            T::Name => {
                self.leaf(K::PropertyIdentifier, field, C_NONE)?;
            }
            T::Private => {
                self.leaf(K::PrivatePropertyIdentifier, field, C_NONE)?;
            }
            T::Str => self.string(slot(field, Ctx::Bare))?,
            T::Number => {
                self.leaf(K::Number, field, C_NONE)?;
            }
            T::LBracket => {
                let n = self.open(K::ComputedPropertyName, field, C_NONE);
                self.bump()?;
                let saved = self.asi;
                self.asi = false;
                self.assignment(EXPR)?;
                self.asi = saved;
                self.expect(T::RBracket)?;
                self.close(n);
            }
            _ => return self.refuse(),
        }
        Ok(())
    }

    // ---- Modules.

    fn import_statement(&mut self, s: Slot) -> R<()> {
        if self.ts && self.peek_at(1).kind == T::Name && self.peek_at(2).kind == T::Eq && !self.is_word_at(1, w::TYPE) {
            return self.import_alias(s);
        }
        let n = self.open_in(K::ImportStatement, s);
        self.tk(K::ImportStatement, K::TImport, f::NONE)?;
        if self.ts && self.is_word(w::TYPE) {
            let n1 = self.peek_at(1);
            if matches!(n1.kind, T::LBrace | T::Star) || n1.kind == T::Name && n1.word != w::FROM {
                self.tk(K::ImportStatement, K::TType, f::NONE)?;
            }
        }
        if self.peek() == T::Str {
            self.string(slot(f::SOURCE, Ctx::Bare))?;
        } else {
            let c = self.open(K::ImportClause, f::NONE, C_NONE);
            match self.peek() {
                T::Star => self.namespace_import()?,
                T::LBrace => self.named_imports()?,
                T::Name => {
                    self.leaf(K::Identifier, f::NONE, C_NONE)?;
                    if self.peek() == T::Comma {
                        self.bump()?;
                        match self.peek() {
                            T::Star => self.namespace_import()?,
                            T::LBrace => self.named_imports()?,
                            _ => return self.refuse(),
                        }
                    }
                }
                _ => return self.refuse(),
            }
            self.close(c);
            if !self.is_word(w::FROM) {
                return self.refuse();
            }
            self.tk(K::ImportStatement, K::TFrom, f::NONE)?;
            self.string(slot(f::SOURCE, Ctx::Bare))?;
        }
        if self.is_word(w::WITH) && self.peek_at(1).kind == T::LBrace {
            self.import_attribute()?;
        } else if self.is_word(w::WITH) || self.is_word(w::ASSERT) {
            return self.refuse();
        }
        self.after = 0;
        self.semicolon()?;
        self.close(n);
        Ok(())
    }

    /// `import A = B.C;` (`import_alias`) and `import A = require("b");` (an `import_statement`).
    fn import_alias(&mut self, s: Slot) -> R<()> {
        if self.is_word_at(3, w::REQUIRE) && self.peek_at(4).kind == T::LParen {
            let n = self.open_in(K::ImportStatement, s);
            self.tk(K::ImportStatement, K::TImport, f::NONE)?;
            let c = self.open(K::ImportRequireClause, f::NONE, C_NONE);
            self.leaf(K::Identifier, f::NONE, C_NONE)?;
            self.bump()?;
            self.bump()?;
            self.bump()?;
            if self.peek() != T::Str {
                return self.refuse();
            }
            self.string(slot(f::SOURCE, Ctx::Bare))?;
            self.expect(T::RParen)?;
            self.close(c);
            self.after = 0;
            self.semicolon()?;
            self.close(n);
            return Ok(());
        }
        let n = self.open_in(K::ImportAlias, s);
        self.tk(K::ImportAlias, K::TImport, f::NONE)?;
        self.leaf(K::Identifier, f::NONE, C_NONE)?;
        self.tk(K::ImportAlias, K::TEq, f::NONE)?;
        let mut k = 0;
        while self.peek_at(2 * k + 1).kind == T::Dot && self.peek_at(2 * k + 2).kind == T::Name {
            k += 1;
        }
        if self.peek() != T::Name {
            return self.refuse();
        }
        self.nested_identifier(k, f::NONE, K::NestedIdentifier)?;
        self.after = 0;
        self.semicolon()?;
        self.close(n);
        Ok(())
    }

    /// `with { type: "json" }`.
    fn import_attribute(&mut self) -> R<()> {
        let n = self.open(K::ImportAttribute, f::NONE, C_NONE);
        self.tk(K::ImportAttribute, K::TWith, f::NONE)?;
        let saved = self.asi;
        self.asi = false;
        self.object(BARE)?;
        self.asi = saved;
        self.close(n);
        Ok(())
    }

    fn namespace_import(&mut self) -> R<()> {
        let n = self.open(K::NamespaceImport, f::NONE, C_NONE);
        self.bump()?;
        if !self.is_word(w::AS) {
            return self.refuse();
        }
        self.bump()?;
        self.leaf(K::Identifier, f::NONE, C_NONE)?;
        self.close(n);
        Ok(())
    }

    fn named_imports(&mut self) -> R<()> {
        let n = self.open(K::NamedImports, f::NONE, C_NONE);
        self.bump()?;
        loop {
            match self.peek() {
                T::RBrace => break,
                T::Name | T::Str => {
                    let sp = self.open(K::ImportSpecifier, f::NONE, C_NONE);
                    if self.ts && self.is_word(w::TYPE) && matches!(self.peek_at(1).kind, T::Name | T::Str) && !self.is_word_at(1, w::AS) {
                        self.tk(K::ImportSpecifier, K::TType, f::NONE)?;
                    }
                    if self.is_word_at(1, w::AS) {
                        self.module_export_name(K::ImportSpecifier, f::NAME)?;
                        self.bump()?;
                        self.leaf(K::Identifier, f::ALIAS, C_NONE)?;
                    } else if self.peek() == T::Name {
                        self.leaf(K::Identifier, f::NAME, C_NONE)?;
                    } else {
                        return self.refuse();
                    }
                    self.close(sp);
                }
                _ => return self.refuse(),
            }
            if self.peek() == T::Comma {
                let c = self.tok();
                self.bump()?;
                if self.peek() == T::RBrace {
                    self.leaf_tok(c, K::TComma);
                    self.leaf(K::TRBrace, f::NONE, C_NONE)?;
                    self.close(n);
                    return Ok(());
                }
            } else if self.peek() != T::RBrace {
                return self.refuse();
            }
        }
        self.bump()?;
        self.close(n);
        Ok(())
    }

    /// `_module_export_name`: an identifier, a string, or `default`.
    fn module_export_name(&mut self, parent: K, field: u8) -> R<()> {
        match self.peek() {
            T::Str => self.string(slot(field, Ctx::Bare)),
            T::Name if self.is_word(w::DEFAULT) && !self.ts => {
                self.tk(parent, K::TDefault, field)?;
                Ok(())
            }
            T::Name => {
                self.leaf(K::Identifier, field, C_NONE)?;
                Ok(())
            }
            _ => self.refuse(),
        }
    }

    fn export_statement(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::ExportStatement, s);
        self.tk(K::ExportStatement, K::TExport, f::NONE)?;
        if self.ts {
            if self.is_word(w::TYPE) && matches!(self.peek_at(1).kind, T::LBrace | T::Star) {
                self.tk(K::ExportStatement, K::TType, f::NONE)?;
            } else if self.peek() == T::Eq {
                self.tk(K::ExportStatement, K::TEq, f::NONE)?;
                self.expressions(EXPR)?;
                self.semicolon()?;
                self.close(n);
                return Ok(());
            } else if self.is_word(w::AS) && self.is_word_at(1, w::NAMESPACE) {
                self.tk(K::ExportStatement, K::TAs, f::NONE)?;
                self.tk(K::ExportStatement, K::TNamespace, f::NONE)?;
                self.leaf(K::Identifier, f::NONE, C_NONE)?;
                self.after = 0;
                self.semicolon()?;
                self.close(n);
                return Ok(());
            } else if self.is_word(w::IMPORT) {
                self.import_alias(slot(f::DECLARATION, Ctx::Decl))?;
                self.close(n);
                return Ok(());
            }
        }
        match self.peek() {
            T::Star => {
                if self.is_word_at(1, w::AS) {
                    let ns = self.open(K::NamespaceExport, f::NONE, C_NONE);
                    self.tk(K::NamespaceExport, K::TStar, f::NONE)?;
                    self.tk(K::NamespaceExport, K::TAs, f::NONE)?;
                    self.module_export_name(K::NamespaceExport, f::NONE)?;
                    self.close(ns);
                } else {
                    self.tk(K::ExportStatement, K::TStar, f::NONE)?;
                }
                self.from_clause()?;
                self.after = 0;
                self.semicolon()?;
            }
            T::LBrace => {
                self.export_clause()?;
                if self.is_word(w::FROM) {
                    self.from_clause()?;
                }
                self.after = 0;
                self.semicolon()?;
            }
            T::Name if self.is_word(w::DEFAULT) => {
                self.tk(K::ExportStatement, K::TDefault, f::NONE)?;
                let t = self.tok();
                if self.ts && t.kind == T::Name && (t.word == w::INTERFACE && self.peek_at(1).kind == T::Name || t.word == w::ABSTRACT && self.is_word_at(1, w::CLASS)) {
                    self.declaration(slot(f::DECLARATION, Ctx::Decl))?;
                    self.close(n);
                    return Ok(());
                }
                let declares = t.kind == T::Name
                    && (t.word == w::FUNCTION && true || t.word == w::CLASS || (t.word == w::ASYNC && self.is_word_at(1, w::FUNCTION) && !self.peek_at(1).nl));
                if declares {
                    // `export default function () {}`: a declaration whose name is optional is
                    // the grammar's `function_expression`? tree-sitter reads a named one as a
                    // declaration; refused where it has no name.
                    let k = usize::from(t.word == w::ASYNC);
                    let named = match t.word {
                        w::CLASS => self.peek_at(1).kind == T::Name && !self.is_word_at(1, w::EXTENDS),
                        _ => {
                            let star = self.peek_at(k + 1).kind == T::Star;
                            self.peek_at(k + 1 + usize::from(star)).kind == T::Name
                        }
                    };
                    if !named {
                        // A function or class without a name: an expression, the `value`.
                        self.assignment(slot(f::VALUE, Ctx::Expr))?;
                        self.semicolon()?;
                    } else if t.word == w::CLASS {
                        self.class_declaration(slot(f::DECLARATION, Ctx::Decl))?;
                    } else {
                        self.function(slot(f::DECLARATION, Ctx::Decl), true, t.word == w::ASYNC)?;
                    }
                } else {
                    self.assignment(slot(f::VALUE, Ctx::Expr))?;
                    self.semicolon()?;
                }
            }
            T::Name => self.declaration(slot(f::DECLARATION, Ctx::Decl))?,
            _ => return self.refuse(),
        }
        self.close(n);
        Ok(())
    }

    fn from_clause(&mut self) -> R<()> {
        if !self.is_word(w::FROM) {
            return self.refuse();
        }
        self.tk(K::ExportStatement, K::TFrom, f::NONE)?;
        self.string(slot(f::SOURCE, Ctx::Bare))?;
        if self.is_word(w::WITH) && self.peek_at(1).kind == T::LBrace {
            self.import_attribute()?;
        } else if self.is_word(w::WITH) || self.is_word(w::ASSERT) {
            return self.refuse();
        }
        Ok(())
    }

    fn export_clause(&mut self) -> R<()> {
        let n = self.open(K::ExportClause, f::NONE, C_NONE);
        self.bump()?;
        loop {
            match self.peek() {
                T::RBrace => break,
                T::Name | T::Str => {
                    let sp = self.open(K::ExportSpecifier, f::NONE, C_NONE);
                    if self.ts && self.is_word(w::TYPE) && matches!(self.peek_at(1).kind, T::Name | T::Str) && !self.is_word_at(1, w::AS) {
                        self.tk(K::ExportSpecifier, K::TType, f::NONE)?;
                    }
                    self.module_export_name(K::ExportSpecifier, f::NAME)?;
                    if self.is_word(w::AS) {
                        self.tk(K::ExportSpecifier, K::TAs, f::NONE)?;
                        self.module_export_name(K::ExportSpecifier, f::ALIAS)?;
                    }
                    self.close(sp);
                }
                _ => return self.refuse(),
            }
            if self.peek() == T::Comma {
                let c = self.tok();
                self.bump()?;
                if self.peek() == T::RBrace {
                    self.leaf_tok(c, K::TComma);
                    self.leaf(K::TRBrace, f::NONE, C_NONE)?;
                    self.close(n);
                    return Ok(());
                }
            } else if self.peek() != T::RBrace {
                return self.refuse();
            }
        }
        self.bump()?;
        self.close(n);
        Ok(())
    }

    // ---- Expressions.

    /// `_expressions`: an expression, or several as a `sequence_expression` (no supertype).
    fn expressions(&mut self, s: Slot) -> R<N> {
        let start = self.here();
        let first = self.assignment(s)?;
        if self.peek() != T::Comma {
            return Ok(first);
        }
        let seq = self.wrap(N { at: start, kind: first.kind }, K::SequenceExpression, slot(s.field, Ctx::Bare), EXPR);
        while self.peek() == T::Comma {
            self.bump()?;
            self.assignment(EXPR)?;
        }
        self.close(seq);
        Ok(seq)
    }

    /// An assignment-level expression: arrow functions, `yield`, assignments, ternaries.
    fn assignment(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        // After an expression, it may go on (`V_EXPR`); an arrow function's block body ends it
        // (`V_ARROW_BODY_END`, set there).
        self.after = V_EXPR;
        let r = self.assignment_inner(s);
        self.depth -= 1;
        r
    }

    fn assignment_inner(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        // Arrow functions.
        if t.kind == T::Name {
            let n1 = self.peek_at(1);
            if n1.kind == T::Arrow && !n1.nl {
                return self.arrow(s, false);
            }
            if t.word == w::ASYNC && !n1.nl {
                if n1.kind == T::Name && !self.is_word_at(1, w::FUNCTION) && self.peek_at(2).kind == T::Arrow {
                    return self.arrow(s, true);
                }
                if n1.kind == T::LParen || self.ts && n1.kind == T::Lt {
                    let m = self.mark();
                    if let Ok(n) = self.arrow(s, true) {
                        return Ok(n);
                    }
                    self.restore(m);
                }
            }
            if t.word == w::YIELD {
                return self.yield_expression(s);
            }
        }
        // `<T>(x) => …`; in TSX, `<T,>` or `<T extends …>` (else `<` opens an element).
        let generic = self.ts
            && t.kind == T::Lt
            && (self.dialect == Dialect::TypeScript || self.peek_at(2).kind == T::Comma || self.is_word_at(2, w::EXTENDS));
        if t.kind == T::LParen || generic {
            let m = self.mark();
            if let Ok(n) = self.arrow(s, false) {
                return Ok(n);
            }
            self.restore(m);
        }
        // A destructuring assignment: `[a, b] = c`, `({a} = b)`.
        if matches!(t.kind, T::LBracket | T::LBrace) {
            let m = self.mark();
            let pattern = if t.kind == T::LBracket { self.array_pattern(slot(f::LEFT, Ctx::Bare)) } else { self.object_pattern(slot(f::LEFT, Ctx::Bare)) };
            if pattern.is_ok() && self.peek() == T::Eq {
                self.restore(m);
                let start = self.here();
                let n = self.open_in(K::AssignmentExpression, s);
                let _ = start;
                if t.kind == T::LBracket {
                    self.array_pattern(slot(f::LEFT, Ctx::Bare))?;
                } else {
                    self.object_pattern(slot(f::LEFT, Ctx::Bare))?;
                }
                self.bump()?;
                self.assignment(slot(f::RIGHT, Ctx::Expr))?;
                self.close(n);
                return Ok(n);
            }
            self.restore(m);
        }
        let start = self.here();
        let left = self.ternary(s)?;
        let op = self.tok();
        let aug = augmented(op.kind);
        if (op.kind == T::Eq || aug.is_some()) && !self.stops() {
            let lhs_ok = match left.kind {
                K::Identifier | K::Undefined | K::MemberExpression | K::SubscriptExpression | K::ParenthesizedExpression => true,
                _ => false,
            };
            if !lhs_ok {
                return self.refuse();
            }
            let n = N { at: start, kind: left.kind };
            if let Some(k) = aug {
                let a = self.wrap(n, K::AugmentedAssignmentExpression, s, slot(f::LEFT, Ctx::Bare));
                self.leaf(k, f::OPERATOR, C_NONE)?;
                self.assignment(slot(f::RIGHT, Ctx::Expr))?;
                self.close(a);
                return Ok(a);
            }
            let a = self.wrap(n, K::AssignmentExpression, s, slot(f::LEFT, Ctx::Bare));
            self.bump()?;
            self.assignment(slot(f::RIGHT, Ctx::Expr))?;
            self.close(a);
            return Ok(a);
        }
        Ok(left)
    }

    /// An arrow function: `x => …`, `async x => …`, `(params) => …`, `async (params) => …`.
    fn arrow(&mut self, s: Slot, is_async: bool) -> R<N> {
        let n = self.open_in(K::ArrowFunction, s);
        if is_async {
            self.tk(K::ArrowFunction, K::TAsync, f::NONE)?;
        }
        if self.peek() == T::Name {
            self.leaf(K::Identifier, f::PARAMETER, C_NONE)?;
        } else {
            self.signature()?;
        }
        let a = self.tok();
        if a.kind != T::Arrow || a.nl {
            return self.refuse();
        }
        self.bump()?;
        if self.peek() == T::LBrace {
            let saved = self.asi;
            self.statement_block(slot(f::BODY, Ctx::Bare), true)?;
            self.asi = saved;
            self.close(n);
            self.after = V_ARROW_BODY_END;
            // An arrow function with a block body ends its statement at a line break (Patch 9).
            if self.asi && self.asi(V_ARROW_BODY_END) {
                return Ok(n);
            }
            return Ok(n);
        }
        self.assignment(slot(f::BODY, Ctx::Expr))?;
        self.close(n);
        Ok(n)
    }

    fn yield_expression(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::YieldExpression, s);
        self.tk(K::YieldExpression, K::TYield, f::NONE)?;
        self.after = V_NO_LT;
        let t = self.tok();
        if t.kind == T::Star && !t.nl {
            self.tk(K::YieldExpression, K::TStar, f::NONE)?;
            self.assignment(EXPR)?;
        } else if can_start_expression(t.kind) && !(self.asi && self.asi(V_NO_LT)) && !matches!(t.kind, T::Star) {
            if t.nl && !self.asi {
                return self.refuse();
            }
            self.assignment(EXPR)?;
        } else if !matches!(t.kind, T::RParen | T::RBracket | T::RBrace | T::Comma | T::Semi | T::Colon | T::Eof) && !t.nl {
            return self.refuse();
        }
        self.close(n);
        self.after = V_EXPR;
        Ok(n)
    }

    /// A ternary: `condition ? consequence : alternative` (right to left), over binaries.
    fn ternary(&mut self, s: Slot) -> R<N> {
        let start = self.here();
        let cond = self.binary(s, 0)?;
        if self.peek() != T::Question || self.stops() {
            return Ok(cond);
        }
        let n = self.wrap(N { at: start, kind: cond.kind }, K::TernaryExpression, s, slot(f::CONDITION, Ctx::Expr));
        self.bump()?;
        let saved = self.asi;
        self.asi = false;
        self.assignment(slot(f::CONSEQUENCE, Ctx::Expr))?;
        self.asi = saved;
        self.expect(T::Colon)?;
        self.assignment(slot(f::ALTERNATIVE, Ctx::Expr))?;
        self.close(n);
        Ok(n)
    }

    /// Binary operators by precedence climbing, at `min` and above.
    fn binary(&mut self, s: Slot, min: u8) -> R<N> {
        self.binary_as(s, min, true)
    }

    /// `binary`; with `allow`, `as` and `satisfies` taken where they follow (TypeScript): after a
    /// left-associative chain, on the chain read (`(a + b) as T`); in a right-associative
    /// operand, on the operand (`a ** (b as T)`).
    fn binary_as(&mut self, s: Slot, min: u8, allow: bool) -> R<N> {
        let start = self.here();
        let mut left = self.unary(s, false)?;
        loop {
            let t = self.tok();
            if self.ts && allow && t.kind == T::Name && matches!(t.word, w::AS | w::SATISFIES) && !self.stops() {
                left = self.as_expression(start, left, s)?;
                continue;
            }
            let Some((prec, op, right_assoc)) = binop(t) else { break };
            if prec < min || self.stops() {
                break;
            }
            let n = self.wrap(N { at: start, kind: left.kind }, K::BinaryExpression, s, slot(f::LEFT, Ctx::Expr));
            self.leaf(op, f::OPERATOR, C_NONE)?;
            self.binary_as(slot(f::RIGHT, Ctx::Expr), if right_assoc { prec } else { prec + 1 }, right_assoc)?;
            self.close(n);
            self.after = V_EXPR;
            left = n;
        }
        Ok(left)
    }

    /// `left as T`, `left as const`, `left satisfies T`, `left` written at `start`.
    fn as_expression(&mut self, start: usize, left: N, s: Slot) -> R<N> {
        let is_as = self.is_word(w::AS);
        let (kind, tok) = if is_as { (K::AsExpression, K::TAs) } else { (K::SatisfiesExpression, K::TSatisfies) };
        let n = self.wrap(N { at: start, kind: left.kind }, kind, s, EXPR);
        self.tk(kind, tok, f::NONE)?;
        let mut members = 0;
        if is_as && self.is_word(w::CONST) {
            self.tk(kind, K::TConst, f::NONE)?;
        } else if self.peek() == T::Name && self.peek_at(1).kind == T::Dot && {
            members = self.dotted_names();
            self.asi && members >= 2 && !matches!(self.peek_at(2 * members + 1).kind, T::Lt | T::LBracket | T::Pipe | T::Amp)
        } {
            // `x as A.B.C`: tree-sitter reads `(x as A.B).C`.
            self.type_reference_k(slot(f::NONE, Ctx::Type), members - 1)?;
        } else {
            members = 0;
            self.ty(slot(f::NONE, Ctx::Type))?;
        }
        self.close(n);
        let mut e = n;
        if members > 0 {
            let m = self.wrap(N { at: start, kind: e.kind }, K::MemberExpression, s, slot(f::OBJECT, Ctx::Object));
            self.tk(K::MemberExpression, K::TDot, f::NONE)?;
            self.leaf(K::PropertyIdentifier, f::PROPERTY, C_NONE)?;
            self.close(m);
            e = m;
        }
        self.after = V_EXPR;
        Ok(e)
    }

    /// Unary operators, `await`, prefix `++`/`--`, then a postfix expression. With `no_in`, a
    /// `for … in` head's left side (no binary `in` after).
    fn unary(&mut self, s: Slot, no_in: bool) -> R<N> {
        let _ = no_in;
        let t = self.tok();
        let op = match t.kind {
            T::Bang => Some(K::TBang),
            T::Tilde => Some(K::TTilde),
            T::Minus => Some(K::TMinus),
            T::Plus => Some(K::TPlus),
            T::Name if t.word == w::TYPEOF => Some(K::TTypeof),
            T::Name if t.word == w::VOID => Some(K::TVoid),
            T::Name if t.word == w::DELETE => Some(K::TDelete),
            _ => None,
        };
        if let Some(op) = op {
            self.deeper()?;
            let n = self.open_in(K::UnaryExpression, s);
            self.leaf(op, f::OPERATOR, C_NONE)?;
            self.unary(slot(f::ARGUMENT, Ctx::Expr), false)?;
            self.close(n);
            self.depth -= 1;
            self.after = V_EXPR;
            return Ok(n);
        }
        if t.kind == T::Lt && self.dialect == Dialect::TypeScript {
            // `<T>x`.
            self.deeper()?;
            let n = self.open_in(K::TypeAssertion, s);
            self.type_arguments(f::NONE)?;
            self.unary(EXPR, false)?;
            self.close(n);
            self.depth -= 1;
            self.after = V_EXPR;
            return Ok(n);
        }
        if matches!(t.kind, T::PlusPlus | T::MinusMinus) {
            self.deeper()?;
            let n = self.open_in(K::UpdateExpression, s);
            let k = if t.kind == T::PlusPlus { K::TPlusPlus } else { K::TMinusMinus };
            self.leaf(k, f::OPERATOR, C_NONE)?;
            self.unary(slot(f::ARGUMENT, Ctx::Expr), false)?;
            self.close(n);
            self.depth -= 1;
            self.after = V_EXPR;
            return Ok(n);
        }
        if t.kind == T::Name && t.word == w::AWAIT {
            let next = self.peek_at(1);
            if can_start_expression(next.kind) && !(next.nl && self.asi) {
                self.deeper()?;
                let n = self.open_in(K::AwaitExpression, s);
                self.bump()?;
                let arg_start = self.here();
                let mut arg = self.unary(EXPR, false)?;
                while self.ts && (self.is_word(w::AS) || self.is_word(w::SATISFIES)) && !self.stops() {
                    arg = self.as_expression(arg_start, arg, EXPR)?;
                }
                // `await (…)` and a postfix: tree-sitter reads a call of a name `await`.
                if next.kind == T::LParen && arg.kind != K::ParenthesizedExpression {
                    return self.refuse();
                }
                self.close(n);
                self.depth -= 1;
                self.after = V_EXPR;
                return Ok(n);
            }
            return self.refuse();
        }
        let start = self.here();
        let e = self.postfix(s)?;
        let t = self.tok();
        if matches!(t.kind, T::PlusPlus | T::MinusMinus) && !self.stops() && !t.nl {
            if !matches!(e.kind, K::Identifier | K::Undefined | K::MemberExpression | K::SubscriptExpression | K::ParenthesizedExpression) {
                return self.refuse();
            }
            let n = self.wrap(N { at: start, kind: e.kind }, K::UpdateExpression, s, slot(f::ARGUMENT, Ctx::Expr));
            let k = if t.kind == T::PlusPlus { K::TPlusPlus } else { K::TMinusMinus };
            self.leaf(k, f::OPERATOR, C_NONE)?;
            self.close(n);
            self.after = V_EXPR;
            return Ok(n);
        }
        Ok(e)
    }

    /// A primary expression and its postfixes: members, subscripts, calls, tagged templates.
    fn postfix(&mut self, s: Slot) -> R<N> {
        let start = self.here();
        let mut e = if self.is_word(w::NEW) { self.new_expression(s)? } else { self.primary(s)? };
        loop {
            let t = self.tok();
            match t.kind {
                T::QDot if matches!(self.peek_at(1).kind, T::LBracket | T::LParen) => {
                    if self.stops() {
                        break;
                    }
                    if self.peek_at(1).kind == T::LBracket {
                        // `a?.[i]`.
                        let n = self.wrap(N { at: start, kind: e.kind }, K::SubscriptExpression, s, slot(f::OBJECT, Ctx::Object));
                        self.optional_chain()?;
                        self.bump()?;
                        let saved = self.asi;
                        self.asi = false;
                        self.expressions(slot(f::INDEX, Ctx::Expr))?;
                        self.asi = saved;
                        self.expect(T::RBracket)?;
                        self.close(n);
                        e = n;
                    } else {
                        // `f?.(…)`: the function a `primary_expression` alone.
                        let n = self.wrap(N { at: start, kind: e.kind }, K::CallExpression, s, slot(f::FUNCTION, Ctx::Primary));
                        if self.ts {
                            self.leaf(K::TQDot, f::NONE, C_NONE)?;
                        } else {
                            self.optional_chain()?;
                        }
                        self.arguments()?;
                        self.close(n);
                        e = n;
                    }
                }
                T::Dot | T::QDot => {
                    if self.stops() {
                        break;
                    }
                    let n = self.wrap(N { at: start, kind: e.kind }, K::MemberExpression, s, slot(f::OBJECT, Ctx::Object));
                    if t.kind == T::QDot {
                        self.optional_chain()?;
                    } else {
                        self.tk(K::MemberExpression, K::TDot, f::NONE)?;
                    }
                    match self.peek() {
                        T::Name => {
                            self.leaf(K::PropertyIdentifier, f::PROPERTY, C_NONE)?;
                        }
                        T::Private => {
                            self.leaf(K::PrivatePropertyIdentifier, f::PROPERTY, C_NONE)?;
                        }
                        _ => return self.refuse(),
                    }
                    self.close(n);
                    e = n;
                }
                T::LBracket => {
                    if self.stops() {
                        break;
                    }
                    let n = self.wrap(N { at: start, kind: e.kind }, K::SubscriptExpression, s, slot(f::OBJECT, Ctx::Object));
                    self.bump()?;
                    let saved = self.asi;
                    self.asi = false;
                    self.expressions(slot(f::INDEX, Ctx::Expr))?;
                    self.asi = saved;
                    self.expect(T::RBracket)?;
                    self.close(n);
                    e = n;
                }
                T::LParen => {
                    if self.stops() {
                        break;
                    }
                    let n = self.wrap(N { at: start, kind: e.kind }, K::CallExpression, s, slot(f::FUNCTION, Ctx::Expr));
                    self.arguments()?;
                    self.close(n);
                    e = n;
                }
                T::Bang if self.ts && !t.nl => {
                    let n = self.wrap(N { at: start, kind: e.kind }, K::NonNullExpression, s, slot(f::NONE, Ctx::Primary));
                    self.tk(K::NonNullExpression, K::TBang, f::NONE)?;
                    self.close(n);
                    e = n;
                }
                T::Lt if self.ts && !self.in_extends => {
                    if self.stops() {
                        break;
                    }
                    // `f<T>(…)`, `f<T>`: type arguments if they read as such and a call or an
                    // instantiation's end follows; else `<` is an operator.
                    let m = self.mark();
                    if self.type_arguments(f::TYPE_ARGUMENTS).is_err() {
                        self.restore(m);
                        break;
                    }
                    let next = self.tok();
                    match next.kind {
                        T::LParen if !self.stops() => {
                            let n = self.wrap(N { at: start, kind: e.kind }, K::CallExpression, s, slot(f::FUNCTION, Ctx::Primary));
                            self.arguments()?;
                            self.close(n);
                            e = n;
                        }
                        T::Semi | T::RParen | T::RBracket | T::RBrace | T::Comma | T::Colon | T::Eof => {
                            let n = self.wrap(N { at: start, kind: e.kind }, K::InstantiationExpression, s, slot(f::NONE, Ctx::Primary));
                            self.close(n);
                            e = n;
                        }
                        _ if next.nl && self.stops() => {
                            let n = self.wrap(N { at: start, kind: e.kind }, K::InstantiationExpression, s, slot(f::NONE, Ctx::Primary));
                            self.close(n);
                            e = n;
                        }
                        _ => {
                            self.restore(m);
                            break;
                        }
                    }
                }
                T::Backtick => {
                    if self.stops() {
                        break;
                    }
                    if !is_primary(e.kind) {
                        return self.refuse();
                    }
                    let n = self.wrap(N { at: start, kind: e.kind }, K::CallExpression, s, slot(f::FUNCTION, Ctx::Primary));
                    self.template(slot(f::ARGUMENTS, Ctx::Bare))?;
                    self.close(n);
                    e = n;
                }
                _ => break,
            }
            self.after = V_EXPR;
        }
        Ok(e)
    }

    fn arguments(&mut self) -> R<()> {
        let n = self.open(K::Arguments, f::ARGUMENTS, C_NONE);
        self.bump()?;
        let saved = self.asi;
        self.asi = false;
        loop {
            match self.peek() {
                T::RParen => break,
                T::Comma => return self.refuse(),
                T::Ellipsis => self.spread()?,
                _ => {
                    self.assignment(EXPR)?;
                }
            }
            if self.peek() == T::Comma {
                let c = self.tok();
                self.bump()?;
                if self.peek() == T::RParen {
                    self.leaf_tok(c, K::TComma);
                    self.leaf(K::TRParen, f::NONE, C_NONE)?;
                    self.asi = saved;
                    self.close(n);
                    self.after = V_EXPR;
                    return Ok(());
                }
            } else if self.peek() != T::RParen {
                return self.refuse();
            }
        }
        self.bump()?;
        self.asi = saved;
        self.close(n);
        self.after = V_EXPR;
        Ok(())
    }

    fn spread(&mut self) -> R<()> {
        let n = self.open(K::SpreadElement, f::NONE, C_NONE);
        self.bump()?;
        self.assignment(EXPR)?;
        self.close(n);
        Ok(())
    }

    /// `new constructor arguments?`.
    fn new_expression(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        if self.peek_at(1).kind == T::Dot {
            // `new.target`.
            let n = self.open_in(K::MetaProperty, s);
            self.tk(K::MetaProperty, K::TNew, f::NONE)?;
            self.tk(K::MetaProperty, K::TDot, f::NONE)?;
            if !self.is_word(w::TARGET) {
                return self.refuse();
            }
            self.tk(K::MetaProperty, K::TTarget, f::NONE)?;
            self.close(n);
            self.depth -= 1;
            return Ok(n);
        }
        let n = self.open_in(K::NewExpression, s);
        self.bump()?;
        if self.is_word(w::NEW) {
            return self.refuse();
        }
        // The constructor: a primary expression and its members (no call).
        let start = self.here();
        let mut e = self.primary(slot(f::CONSTRUCTOR, Ctx::Primary))?;
        loop {
            let t = self.tok();
            match t.kind {
                T::Dot => {
                    let m = self.wrap(N { at: start, kind: e.kind }, K::MemberExpression, slot(f::CONSTRUCTOR, Ctx::Primary), slot(f::OBJECT, Ctx::Object));
                    self.tk(K::MemberExpression, K::TDot, f::NONE)?;
                    match self.peek() {
                        T::Name => {
                            self.leaf(K::PropertyIdentifier, f::PROPERTY, C_NONE)?;
                        }
                        T::Private => {
                            self.leaf(K::PrivatePropertyIdentifier, f::PROPERTY, C_NONE)?;
                        }
                        _ => return self.refuse(),
                    }
                    self.close(m);
                    e = m;
                }
                T::LBracket => {
                    let m = self.wrap(N { at: start, kind: e.kind }, K::SubscriptExpression, slot(f::CONSTRUCTOR, Ctx::Primary), slot(f::OBJECT, Ctx::Object));
                    self.bump()?;
                    let saved = self.asi;
                    self.asi = false;
                    self.expressions(slot(f::INDEX, Ctx::Expr))?;
                    self.asi = saved;
                    self.expect(T::RBracket)?;
                    self.close(m);
                    e = m;
                }
                T::QDot | T::Backtick => return self.refuse(),
                _ => break,
            }
        }
        if self.ts && self.peek() == T::Lt {
            self.type_arguments(f::TYPE_ARGUMENTS)?;
        }
        if self.peek() == T::LParen && !self.stops() {
            self.arguments()?;
        }
        self.close(n);
        self.depth -= 1;
        self.after = V_EXPR;
        Ok(n)
    }

    fn primary(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Number => self.leaf_in(K::Number, s),
            T::Str => {
                let n = self.open_in(K::String, s);
                self.string_body(n)?;
                Ok(n)
            }
            T::Backtick => {
                let n = self.open_in(K::TemplateString, s);
                self.template_body(n)?;
                Ok(n)
            }
            T::Slash | T::SlashEq => self.regex(s),
            T::LParen => self.parenthesized(s),
            T::LBracket => self.array(s),
            T::LBrace => self.object(s),
            T::Lt if self.dialect != Dialect::TypeScript => self.jsx(s),
            T::Name => match t.word {
                w::THIS => self.leaf_in(K::This, s),
                w::SUPER => self.leaf_in(K::Super, s),
                w::NULL => self.leaf_in(K::Null, s),
                w::TRUE => self.leaf_in(K::True, s),
                w::FALSE => self.leaf_in(K::False, s),
                w::UNDEFINED => self.leaf_in(K::Undefined, s),
                w::FUNCTION => {
                    let at = self.w.len();
                    self.function(s, false, false)?;
                    Ok(N { at, kind: kind_of(unkey(self.w.key(at)).0) })
                }
                w::ASYNC if self.is_word_at(1, w::FUNCTION) && !self.peek_at(1).nl => {
                    let at = self.w.len();
                    self.function(s, false, true)?;
                    Ok(N { at, kind: kind_of(unkey(self.w.key(at)).0) })
                }
                w::CLASS => {
                    let n = self.open_in(K::Class, s);
                    self.tk(K::Class, K::TClass, f::NONE)?;
                    if self.peek() == T::Name && !self.is_word(w::EXTENDS) && !(self.ts && self.is_word(w::IMPLEMENTS)) {
                        self.leaf(if self.ts { K::TypeIdentifier } else { K::Identifier }, f::NAME, C_NONE)?;
                    }
                    self.class_rest()?;
                    self.close(n);
                    Ok(n)
                }
                w::IMPORT => {
                    let next = self.peek_at(1).kind;
                    if next == T::Dot {
                        let n = self.open_in(K::MetaProperty, s);
                        self.tk(K::MetaProperty, K::TImport, f::NONE)?;
                        self.tk(K::MetaProperty, K::TDot, f::NONE)?;
                        if !self.is_word(w::META) {
                            return self.refuse();
                        }
                        self.tk(K::MetaProperty, K::TMeta, f::NONE)?;
                        self.close(n);
                        return Ok(n);
                    }
                    if next == T::LParen {
                        // `import(…)`: the named `import`, holding its token.
                        let n = self.open(K::Import, f::FUNCTION, C_NONE);
                        self.tk(K::Import, K::TImport, f::NONE)?;
                        self.close(n);
                        let start = n.at;
                        let c = self.wrap(n, K::CallExpression, s, slot(f::FUNCTION, Ctx::Bare));
                        let _ = start;
                        self.arguments()?;
                        self.close(c);
                        return Ok(c);
                    }
                    self.refuse()
                }
                // Keywords that start no expression.
                w::IF | w::FOR | w::WHILE | w::DO | w::RETURN | w::VAR | w::CONST | w::SWITCH | w::CASE | w::DEFAULT | w::TRY | w::CATCH | w::FINALLY | w::THROW | w::BREAK | w::CONTINUE | w::ELSE | w::EXPORT | w::DEBUGGER | w::WITH | w::NEW | w::DELETE | w::TYPEOF | w::VOID | w::IN | w::INSTANCEOF | w::EXTENDS | w::ENUM => self.refuse(),
                // tree-sitter's lexer reads `using` as its declarations' keyword where one may
                // start, and a name `using` can fail to parse: refused.
                w::USING => self.refuse(),
                _ => self.leaf_in(K::Identifier, s),
            },
            _ => self.refuse(),
        }
    }

    /// A string in slot `s`.
    fn string(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::String, s);
        self.string_body(n)?;
        Ok(())
    }

    /// A string token's pieces: its quotes (stored) and its fragments and escapes.
    fn string_body(&mut self, n: N) -> R<()> {
        let t = self.bump()?;
        let q = if self.src[t.start as usize] == b'"' { K::TDQuote } else { K::TSQuote };
        let mut pieces = std::mem::take(&mut self.pieces);
        pieces.clear();
        self.lx.string_pieces(t.start as usize, t.end as usize, &mut pieces)?;
        self.leaf_tok(Tok { start: t.start, end: t.start + 1, ..t }, q);
        for &(a, b, esc) in &pieces {
            let k = if esc { K::EscapeSequence } else { K::StringFragment };
            let at = self.w.push(key(k, f::NONE, C_NONE, false), a);
            self.w.close(at, b);
        }
        // The closing quote is stored after no piece alone (an empty string).
        if pieces.is_empty() {
            self.leaf_tok(Tok { start: t.end - 1, end: t.end, ..t }, q);
        }
        self.pieces = pieces;
        self.w.close(n.at, t.end);
        self.after = V_EXPR;
        Ok(())
    }

    fn template(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::TemplateString, s);
        self.template_body(n)
    }

    /// A template from its `` ` ``: text, escapes and substitutions, read in template mode.
    fn template_body(&mut self, n: N) -> R<()> {
        let t = self.bump()?;
        let mut pos = t.end as usize;
        self.relex_at(pos);
        loop {
            let (piece, end) = self.lx.template_piece(pos)?;
            match piece {
                Piece::Chars => self.leaf_span(K::StringFragment, f::NONE, pos as u32, end as u32),
                Piece::Escape => self.leaf_span(K::EscapeSequence, f::NONE, pos as u32, end as u32),
                Piece::Close => {
                    self.last_end = end as u32;
                    self.relex_at(end);
                    break;
                }
                Piece::Open => {
                    let sub = self.open_span(K::TemplateSubstitution, f::NONE, pos as u32);
                    self.last_end = end as u32;
                    self.relex_at(end);
                    let saved = self.asi;
                    self.asi = false;
                    self.expressions(EXPR)?;
                    self.asi = saved;
                    let r = self.tok();
                    if r.kind != T::RBrace {
                        return self.refuse();
                    }
                    self.flush(r.start);
                    self.last_end = r.end;
                    self.close(sub);
                    self.relex_at(r.end as usize);
                    pos = r.end as usize;
                    continue;
                }
            }
            pos = end;
        }
        self.close(n);
        self.after = V_EXPR;
        Ok(())
    }

    fn regex(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        let (pattern_end, end) = self.lx.regex(t.start as usize)?;
        self.flush(t.start);
        let n = self.open_in(K::Regex, s);
        let p = self.w.push(key(K::RegexPattern, f::PATTERN, C_NONE, false), t.start + 1);
        self.w.close(p, pattern_end as u32);
        if end > pattern_end + 1 {
            let fl = self.w.push(key(K::RegexFlags, f::FLAGS, C_NONE, false), pattern_end as u32 + 1);
            self.w.close(fl, end as u32);
        }
        self.last_end = end as u32;
        self.relex_at(end);
        self.close(n);
        self.after = V_EXPR;
        Ok(n)
    }

    fn array(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::Array, s);
        self.bracket_list(n, Self::array_element)?;
        self.close(n);
        self.after = V_EXPR;
        Ok(n)
    }

    fn array_element(&mut self) -> R<()> {
        if self.peek() == T::Ellipsis {
            return self.spread();
        }
        self.assignment(EXPR)?;
        Ok(())
    }

    fn object(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::Object, s);
        self.bump()?;
        let saved = self.asi;
        self.asi = false;
        loop {
            match self.peek() {
                T::RBrace => break,
                T::Comma => return self.refuse(),
                T::Ellipsis => self.spread()?,
                _ => self.object_member()?,
            }
            if self.peek() == T::Comma {
                let c = self.tok();
                self.bump()?;
                if self.peek() == T::RBrace {
                    self.leaf_tok(c, K::TComma);
                    self.leaf(K::TRBrace, f::NONE, C_NONE)?;
                    self.asi = saved;
                    self.close(n);
                    self.after = V_EXPR;
                    return Ok(n);
                }
            } else if self.peek() != T::RBrace {
                return self.refuse();
            }
        }
        self.bump()?;
        self.asi = saved;
        self.close(n);
        self.after = V_EXPR;
        Ok(n)
    }

    /// An object's member: `pair`, a method, or a shorthand name.
    fn object_member(&mut self) -> R<()> {
        let t = self.tok();
        let next = self.peek_at(1).kind;
        if t.kind == T::Name && matches!(next, T::Comma | T::RBrace) {
            self.leaf(K::ShorthandPropertyIdentifier, f::NONE, C_NONE)?;
            return Ok(());
        }
        if t.kind == T::Name && next == T::Eq {
            return self.refuse();
        }
        // A method: `name(`, `get name(`, `async name(`, `*name(`.
        let method = next == T::LParen
            || self.ts && next == T::Lt
            || t.kind == T::Star
            || (t.kind == T::Name && matches!(t.word, w::GET | w::SET | w::ASYNC) && matches!(next, T::Name | T::Str | T::Number | T::LBracket | T::Private | T::Star));
        if method {
            let n = self.open(K::MethodDefinition, f::NONE, C_NONE);
            if self.is_word(w::ASYNC) && self.peek_at(1).kind != T::LParen {
                self.tk(K::MethodDefinition, K::TAsync, f::NONE)?;
            }
            if (self.is_word(w::GET) || self.is_word(w::SET)) && self.peek_at(1).kind != T::LParen {
                let k = if self.is_word(w::GET) { K::TGet } else { K::TSet };
                self.tk(K::MethodDefinition, k, f::NONE)?;
            } else if self.peek() == T::Star {
                self.tk(K::MethodDefinition, K::TStar, f::NONE)?;
            }
            self.property_name(f::NAME)?;
            self.signature()?;
            if self.peek() != T::LBrace {
                return self.refuse();
            }
            self.statement_block(slot(f::BODY, Ctx::Bare), true)?;
            self.close(n);
            return Ok(());
        }
        let n = self.open(K::Pair, f::NONE, C_NONE);
        self.property_name(f::KEY)?;
        if self.peek() == T::LParen {
            // `[name](…) {}`: a method after all.
            self.w.set_key(n.at, key(K::MethodDefinition, f::NONE, C_NONE, false));
            self.retag_field(n.at + 1, f::NAME);
            self.signature()?;
            if self.peek() != T::LBrace {
                return self.refuse();
            }
            self.statement_block(slot(f::BODY, Ctx::Bare), true)?;
            self.close(n);
            return Ok(());
        }
        self.expect(T::Colon)?;
        self.assignment(slot(f::VALUE, Ctx::Expr))?;
        self.close(n);
        Ok(())
    }

    // ---- JSX.

    /// A JSX element from its `<`: tags and children read in their modes.
    fn jsx(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let t = self.tok();
        // `<` then a name or `>`: the element's opening tag.
        let lt = t.start as usize;
        self.flush(t.start);
        let el = self.open_span(K::JsxElement, s.field, t.start);
        self.w.set_key(el.at, key(K::JsxElement, s.field, class(K::JsxElement, s.ctx), false));
        self.last_end = t.end;
        let mut pos = lt + 1;
        // The name.
        let (q, _) = self.jsx_skip(pos)?;
        pos = q;
        let fragment = self.at(pos) == b'>';
        if !fragment {
            pos = self.jsx_element_name(pos, f::NAME)?;
            // `<C<T> …>`: type arguments, read in normal mode.
            let (q, _) = self.jsx_skip(pos)?;
            if self.ts && self.at(q) == b'<' {
                self.relex_at(q);
                self.type_arguments(f::TYPE_ARGUMENTS)?;
                pos = self.last_end as usize;
                self.relex_at(pos);
            }
        }
        // Attributes, written in the element: the opening tag is put around them at its `>`.
        loop {
            let (q, _) = self.jsx_skip(pos)?;
            pos = q;
            match self.at(pos) {
                b'>' => break,
                b'/' if self.at(pos + 1) == b'>' && !fragment => {
                    // A self-closing element: the element is the tag.
                    self.w.set_key(el.at, key(K::JsxSelfClosingElement, s.field, class(K::JsxSelfClosingElement, s.ctx), false));
                    self.last_end = pos as u32 + 2;
                    self.close(el);
                    self.relex_at(pos + 2);
                    self.depth -= 1;
                    self.after = V_EXPR;
                    return Ok(N { at: el.at, kind: K::JsxSelfClosingElement });
                }
                b'{' => pos = self.jsx_expression(pos, f::ATTRIBUTE)?,
                _ if !fragment => pos = self.jsx_attribute(pos)?,
                _ => return self.refuse_at(pos),
            }
        }
        self.w.insert(el.at + 1, key(K::JsxOpeningElement, f::OPEN_TAG, C_NONE, false), t.start);
        let open = N { at: el.at + 1, kind: K::JsxOpeningElement };
        self.last_end = pos as u32 + 1;
        self.close(open);
        pos += 1;
        // Children, to the closing tag.
        loop {
            match self.lx.jsx_text(pos) {
                Some(end) => {
                    self.leaf_span(K::JsxText, f::NONE, pos as u32, end as u32);
                    pos = end;
                    continue;
                }
                None => {}
            }
            // White space alone, on lines of its own, is passed (as extras' white space).
            let q = self.blank_end(pos);
            match self.at(q) {
                b'&' => match self.lx.character_reference(q) {
                    Some(end) => {
                        self.leaf_span(K::HtmlCharacterReference, f::NONE, q as u32, end as u32);
                        pos = end;
                    }
                    None => return self.refuse_at(q),
                },
                b'{' => pos = self.jsx_expression(q, f::NONE)?,
                b'<' if self.at(q + 1) == b'/' => {
                    let close = self.open_span(K::JsxClosingElement, f::CLOSE_TAG, q as u32);
                    let mut p = q + 2;
                    let (r, _) = self.jsx_skip(p)?;
                    p = r;
                    if self.at(p) != b'>' {
                        if fragment {
                            return self.refuse_at(p);
                        }
                        p = self.jsx_element_name(p, f::NAME)?;
                        let (r, _) = self.jsx_skip(p)?;
                        p = r;
                    } else if !fragment {
                        return self.refuse_at(p);
                    }
                    if self.at(p) != b'>' {
                        return self.refuse_at(p);
                    }
                    self.last_end = p as u32 + 1;
                    self.close(close);
                    pos = p + 1;
                    break;
                }
                b'<' => {
                    self.last_end = pos as u32;
                    self.relex_at(q);
                    let saved = self.asi;
                    self.asi = false;
                    self.jsx(BARE)?;
                    self.asi = saved;
                    pos = self.last_end as usize;
                }
                _ => return self.refuse_at(q),
            }
        }
        self.relex_at(pos);
        self.close(el);
        self.depth -= 1;
        self.after = V_EXPR;
        Ok(el)
    }

    /// White space (no comments) from `p`.
    fn blank_end(&self, mut p: usize) -> usize {
        while is_space_byte(self.at(p)) {
            p += 1;
        }
        p
    }

    /// Inside a tag: white space and comments passed (comments are extras); where the next piece
    /// starts.
    fn jsx_skip(&mut self, p: usize) -> R<(usize, bool)> {
        self.relex_at(p);
        let r = self.lx.skip(p, &mut self.extras)?;
        Ok(r)
    }

    /// A JSX element's name from `p`: an identifier, a member expression of identifiers, or a
    /// namespaced name. Its end.
    fn jsx_element_name(&mut self, p: usize, field: u8) -> R<usize> {
        self.flush(p as u32);
        let start = self.w.len();
        let (end, _) = self.lx.jsx_name_end(p)?;
        if end == p {
            return self.refuse_at(p);
        }
        let (q, _) = self.jsx_skip(end)?;
        if self.at(q) == b':' {
            return self.jsx_namespace_name(p, end, q, field);
        }
        let word = crate::lex::word(self.src, p, end);
        let kind = if word == w::THIS { K::This } else { K::Identifier };
        self.leaf_span(kind, field, p as u32, end as u32);
        let mut e = N { at: start, kind };
        let mut pos = end;
        loop {
            let (q, _) = self.jsx_skip(pos)?;
            if self.at(q) != b'.' {
                return Ok(pos);
            }
            let (r, _) = self.jsx_skip(q + 1)?;
            let e2 = self.lx.name_end(r)?;
            if e2 == r {
                return self.refuse_at(r);
            }
            let m = self.wrap(e, K::MemberExpression, slot(field, Ctx::Bare), slot(f::OBJECT, Ctx::Bare));
            if self.stores(K::MemberExpression, K::TDot, f::NONE) {
                self.leaf_span(K::TDot, f::NONE, q as u32, q as u32 + 1);
            }
            self.leaf_span(K::PropertyIdentifier, f::PROPERTY, r as u32, e2 as u32);
            self.close(m);
            e = m;
            pos = e2;
        }
    }

    /// `a:b` from `p` (`a` ends at `end`, the `:` at `colon`): its end.
    fn jsx_namespace_name(&mut self, p: usize, end: usize, colon: usize, field: u8) -> R<usize> {
        let n = self.open_span(K::JsxNamespaceName, field, p as u32);
        self.leaf_span(K::Identifier, f::NONE, p as u32, end as u32);
        let (r, _) = self.jsx_skip(colon + 1)?;
        let (e2, _) = self.lx.jsx_name_end(r)?;
        if e2 == r {
            return self.refuse_at(r);
        }
        self.leaf_span(K::Identifier, f::NONE, r as u32, e2 as u32);
        self.close(n);
        Ok(e2)
    }

    /// `name` or `name=value`.
    fn jsx_attribute(&mut self, p: usize) -> R<usize> {
        let (end, _) = self.lx.jsx_name_end(p)?;
        if end == p {
            return self.refuse_at(p);
        }
        let a = self.open_span(K::JsxAttribute, f::ATTRIBUTE, p as u32);
        let (q, _) = self.jsx_skip(end)?;
        let (mut pos, q) = if self.at(q) == b':' {
            let e = self.jsx_namespace_name(p, end, q, f::NONE)?;
            let (q, _) = self.jsx_skip(e)?;
            (e, q)
        } else {
            self.leaf_span(K::PropertyIdentifier, f::NONE, p as u32, end as u32);
            (end, q)
        };
        if self.at(q) == b'=' {
            let (v, _) = self.jsx_skip(q + 1)?;
            match self.at(v) {
                b'"' | b'\'' => {
                    let quote = self.at(v);
                    let close = self.src[v + 1..].iter().position(|&c| c == quote).map(|k| v + 1 + k).ok_or(Refusal::Parse(v as u32))?;
                    let s = self.open_span(K::String, f::NONE, v as u32);
                    let qk = if quote == b'"' { K::TDQuote } else { K::TSQuote };
                    self.leaf_span(qk, f::NONE, v as u32, v as u32 + 1);
                    self.jsx_string_pieces(v + 1, close)?;
                    if close == v + 1 {
                        self.leaf_span(qk, f::NONE, close as u32, close as u32 + 1);
                    }
                    self.last_end = close as u32 + 1;
                    self.close(s);
                    pos = close + 1;
                }
                b'{' => pos = self.jsx_expression(v, f::NONE)?,
                _ => return self.refuse_at(v),
            }
        }
        self.last_end = pos as u32;
        self.close(a);
        Ok(pos)
    }

    /// A JSX string's text: fragments and character references.
    fn jsx_string_pieces(&mut self, from: usize, to: usize) -> R<()> {
        let mut i = from;
        let mut run = from;
        while i < to {
            if self.src[i] == b'&' {
                if let Some(end) = self.lx.character_reference(i).filter(|&e| e <= to) {
                    if run < i {
                        self.leaf_span(K::StringFragment, f::NONE, run as u32, i as u32);
                    }
                    self.leaf_span(K::HtmlCharacterReference, f::NONE, i as u32, end as u32);
                    i = end;
                    run = end;
                    continue;
                }
                // A reference tried and failed (`&` then a letter, a digit or `#`): the `&` is a
                // fragment of its own (`x&y`: `x`, `&`, `y`). A `&` no reference starts at is text.
                if self.at(i + 1).is_ascii_alphanumeric() || self.at(i + 1) == b'#' {
                    if run < i {
                        self.leaf_span(K::StringFragment, f::NONE, run as u32, i as u32);
                    }
                    self.leaf_span(K::StringFragment, f::NONE, i as u32, i as u32 + 1);
                    i += 1;
                    run = i;
                    continue;
                }
            }
            i += 1;
        }
        if run < to {
            self.leaf_span(K::StringFragment, f::NONE, run as u32, to as u32);
        }
        Ok(())
    }

    /// `{ expression }` from its `{` at `p` (an attribute's or a child's): its end.
    fn jsx_expression(&mut self, p: usize, field: u8) -> R<usize> {
        let n = self.open_span(K::JsxExpression, field, p as u32);
        self.last_end = p as u32 + 1;
        self.relex_at(p + 1);
        let saved = self.asi;
        self.asi = false;
        match self.peek() {
            T::RBrace => {}
            T::Ellipsis => self.spread()?,
            _ => {
                self.expressions(EXPR)?;
            }
        }
        self.asi = saved;
        let r = self.tok();
        if r.kind != T::RBrace {
            return self.refuse();
        }
        self.flush(r.start);
        self.last_end = r.end;
        self.close(n);
        self.relex_at(r.end as usize);
        Ok(r.end as usize)
    }
}

fn can_start_expression(t: T) -> bool {
    use T::*;
    matches!(t, Name | Private | Number | Str | Backtick | LParen | LBracket | LBrace | Lt | Slash | SlashEq | Plus | Minus | Bang | Tilde | PlusPlus | MinusMinus)
}

fn augmented(t: T) -> Option<K> {
    use T::*;
    Some(match t {
        PlusEq => K::TPlusEq,
        MinusEq => K::TMinusEq,
        StarEq => K::TStarEq,
        SlashEq => K::TSlashEq,
        PercentEq => K::TPercentEq,
        CaretEq => K::TCaretEq,
        AmpEq => K::TAmpEq,
        PipeEq => K::TPipeEq,
        GtGtEq => K::TGtGtEq,
        GtGtGtEq => K::TGtGtGtEq,
        LtLtEq => K::TLtLtEq,
        StarStarEq => K::TStarStarEq,
        AmpAmpEq => K::TAmpAmpEq,
        PipePipeEq => K::TPipePipeEq,
        QQEq => K::TQQEq,
        _ => return None,
    })
}

/// A binary operator's precedence (higher binds tighter), kind, and whether it is right to left.
fn binop(t: Tok) -> Option<(u8, K, bool)> {
    use T::*;
    Some(match t.kind {
        QQ => (3, K::TQQ, false),
        PipePipe => (4, K::TPipePipe, false),
        AmpAmp => (5, K::TAmpAmp, false),
        Pipe => (6, K::TPipe, false),
        Caret => (7, K::TCaret, false),
        Amp => (8, K::TAmp, false),
        EqEq => (9, K::TEqEq, false),
        Ne => (9, K::TNe, false),
        EqEqEq => (9, K::TEqEqEq, false),
        NeEq => (9, K::TNeEq, false),
        Lt => (10, K::TLt, false),
        Le => (10, K::TLe, false),
        Gt => (10, K::TGt, false),
        Ge => (10, K::TGe, false),
        Name if t.word == w::INSTANCEOF => (10, K::TInstanceof, false),
        Name if t.word == w::IN => (10, K::TIn, false),
        LtLt => (12, K::TLtLt, false),
        GtGt => (12, K::TGtGt, false),
        GtGtGt => (12, K::TGtGtGt, false),
        Plus => (13, K::TPlus, false),
        Minus => (13, K::TMinus, false),
        Star => (14, K::TStar, false),
        Slash => (14, K::TSlash, false),
        Percent => (14, K::TPercent, false),
        StarStar => (15, K::TStarStar, true),
        _ => return None,
    })
}

fn kind_of(k: u16) -> K {
    debug_assert!(k <= crate::kinds::LAST as u16);
    // SAFETY: `K` is `repr(u16)` with variants 0 to `KINDS.len() - 1`, and every key's kind is one.
    unsafe { std::mem::transmute::<u16, K>(k) }
}
