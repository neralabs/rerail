//! Expressions: precedence climbing over tree-sitter-rust's precedences (every binary operator
//! left-associative, assignment and ranges included), paths, literals, block-like expressions,
//! closures, conditions and `match`.

use super::*;

/// A binary operator's class.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bin {
    Assign,
    Compound,
    Range,
    Binary,
}

/// The precedence and class of a binary operator (tree-sitter-rust's `PREC`).
fn binary_op(t: T) -> Option<(u8, Bin)> {
    use T::*;
    Some(match t {
        Eq => (0, Bin::Assign),
        PlusEq | MinusEq | StarEq | SlashEq | PercentEq | CaretEq | AmpEq | PipeEq | LtLtEq | GtGtEq => (0, Bin::Compound),
        DotDot | DotDotEq | DotDotDot => (1, Bin::Range),
        PipePipe => (2, Bin::Binary),
        AmpAmp => (3, Bin::Binary),
        EqEq | Ne | Lt | Le | Gt | Ge => (4, Bin::Binary),
        Pipe => (5, Bin::Binary),
        Caret => (6, Bin::Binary),
        Amp => (7, Bin::Binary),
        LtLt | GtGt => (8, Bin::Binary),
        Plus | Minus => (9, Bin::Binary),
        Star | Slash | Percent => (10, Bin::Binary),
        _ => return None,
    })
}

/// The precedence of `as`, and of the prefix operators (`-`, `*`, `!`, `&`).
const CAST: u8 = 11;
const UNARY: u8 = 12;

/// Whether a word names an identifier where a path or a struct's name stands (no keyword,
/// primitive type or reserved word: those are tree-sitter's keywords there).
fn plain_word(word: u8) -> bool {
    word == w::NONE || w::is_fragment(word)
}

impl<'a> P<'a> {
    /// An expression in slot `s`, its operators of precedence `min` and above.
    pub(super) fn expr(&mut self, s: Slot, min: u8) -> R<N> {
        self.deeper()?;
        let n = self.prefix(s)?;
        let n = self.infix(n, s, min)?;
        self.depth -= 1;
        Ok(n)
    }

    /// The operators after `n` (in slot `s`), of precedence `min` and above: each wraps `n`.
    pub(super) fn infix(&mut self, mut n: N, s: Slot, min: u8) -> R<N> {
        loop {
            let t = self.tok();
            match t.kind {
                T::Question => {
                    n = self.wrap(n, K::TryExpression, s, EXPR);
                    self.tk(K::TryExpression, K::TQuestion, f::NONE)?;
                    self.close(n);
                }
                T::Dot => n = self.dot(n, s)?,
                T::LParen => {
                    n = self.wrap(n, K::CallExpression, s, slot(f::FUNCTION, Ctx::Bare));
                    self.arguments()?;
                    self.close(n);
                }
                T::LBracket => {
                    n = self.wrap(n, K::IndexExpression, s, EXPR);
                    self.tk(K::IndexExpression, K::TLBracket, f::NONE)?;
                    let saved = self.no_struct;
                    self.no_struct = false;
                    self.expr(EXPR, 0)?;
                    self.no_struct = saved;
                    self.expect_tk(K::IndexExpression, T::RBracket, K::TRBracket)?;
                    self.close(n);
                }
                T::Name if t.word == w::AS => {
                    if min > CAST {
                        break;
                    }
                    n = self.wrap(n, K::TypeCastExpression, s, slot(f::VALUE, Ctx::Expr));
                    self.tk(K::TypeCastExpression, K::TAs, f::NONE)?;
                    self.ty(slot(f::TYPE, Ctx::Type), false)?;
                    self.close(n);
                }
                _ => {
                    let Some((prec, bin)) = binary_op(t.kind) else { break };
                    if prec < min {
                        break;
                    }
                    if t.kind == T::AmpAmp && self.is_word_at(1, w::LET) {
                        // `&& let`: a condition's let chain goes on (a guard's too); elsewhere
                        // no expression is `let`, and the caller refuses.
                        break;
                    }
                    let op = punct(t.kind).expect("an operator");
                    match bin {
                        Bin::Assign => {
                            n = self.wrap(n, K::AssignmentExpression, s, slot(f::LEFT, Ctx::Expr));
                            self.tk(K::AssignmentExpression, K::TEq, f::NONE)?;
                            self.expr(slot(f::RIGHT, Ctx::Expr), 1)?;
                        }
                        Bin::Compound => {
                            n = self.wrap(n, K::CompoundAssignmentExpr, s, slot(f::LEFT, Ctx::Expr));
                            self.leaf(op, f::OPERATOR, C_NONE)?;
                            self.expr(slot(f::RIGHT, Ctx::Expr), 1)?;
                        }
                        Bin::Range => {
                            n = self.wrap(n, K::RangeExpression, s, EXPR);
                            self.tk(K::RangeExpression, op, f::NONE)?;
                            if if t.kind == T::DotDot { self.optional_operand(false) } else { self.starts_operand() } {
                                self.expr(EXPR, prec + 1)?;
                            }
                        }
                        Bin::Binary => {
                            n = self.wrap(n, K::BinaryExpression, s, slot(f::LEFT, Ctx::Expr));
                            self.leaf(op, f::OPERATOR, C_NONE)?;
                            self.expr(slot(f::RIGHT, Ctx::Expr), prec + 1)?;
                        }
                    }
                    self.close(n);
                }
            }
        }
        Ok(n)
    }

    /// `.field`, `.0`, `.await`, `.method::<T>`.
    fn dot(&mut self, n: N, s: Slot) -> R<N> {
        let t = self.peek_at(1);
        match t.kind {
            T::Name if t.word == w::AWAIT => {
                let a = self.wrap(n, K::AwaitExpression, s, EXPR);
                self.tk(K::AwaitExpression, K::TDot, f::NONE)?;
                self.tk(K::AwaitExpression, K::TAwait, f::NONE)?;
                self.close(a);
                Ok(a)
            }
            T::Name => {
                let fe = self.wrap(n, K::FieldExpression, s, slot(f::VALUE, Ctx::Expr));
                self.tk(K::FieldExpression, K::TDot, f::NONE)?;
                self.leaf(K::FieldIdentifier, f::FIELD, C_NONE)?;
                self.close(fe);
                if self.peek() == T::ColonColon && matches!(self.peek_at(1).kind, T::Lt) {
                    let g = self.wrap(fe, K::GenericFunction, s, slot(f::FUNCTION, Ctx::Bare));
                    self.tk(K::GenericFunction, K::TColonColon, f::NONE)?;
                    self.type_arguments()?;
                    self.close(g);
                    return Ok(g);
                }
                Ok(fe)
            }
            T::Int | T::Float => {
                let fe = self.wrap(n, K::FieldExpression, s, slot(f::VALUE, Ctx::Expr));
                self.tk(K::FieldExpression, K::TDot, f::NONE)?;
                if self.peek() == T::Float {
                    self.relex_int();
                }
                self.leaf(K::IntegerLiteral, f::FIELD, C_NONE)?;
                let next = self.tok();
                if next.start == self.last_end && next.kind == T::Name {
                    // `x.1e0`: tree-sitter reads no field there.
                    return self.refuse();
                }
                self.close(fe);
                Ok(fe)
            }
            _ => {
                self.i += 1;
                self.refuse()
            }
        }
    }

    /// Whether the next token can start an operand (of a range, `return`, `break`, `yield`): in a
    /// condition, a `{` is the block's.
    pub(super) fn starts_operand(&mut self) -> bool {
        use T::*;
        let t = self.tok();
        match t.kind {
            Name => !matches!(t.word, w::AS | w::ELSE | w::IN | w::WHERE),
            Lifetime => self.peek_at(1).kind == Colon,
            Char | Int | Float | Str | RawStr | Metavariable | LParen | LBracket | Minus | Star | Bang | Amp | AmpAmp | Pipe | PipePipe | DotDot | DotDotEq | Lt | ColonColon => true,
            LBrace => !self.no_struct,
            _ => false,
        }
    }

    /// Whether an optional operand (`break`'s, `..`'s) follows: a token that also continues an
    /// expression (`-`, `*`, `&`, `|`, `<`, `[`, and for `break` `(`) is an operator on the
    /// expression without one, as tree-sitter reads it (`a..*b` is `(a..) * b`).
    fn optional_operand(&mut self, call: bool) -> bool {
        use T::*;
        match self.peek() {
            Minus | Star | Amp | AmpAmp | Pipe | PipePipe | Lt | LBracket | DotDot | DotDotEq => false,
            LParen => !call && self.starts_operand(),
            _ => self.starts_operand(),
        }
    }

    /// An operand: a prefix operator's expression, or a primary one.
    fn prefix(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Minus | T::Star | T::Bang => {
                let n = self.open_in(K::UnaryExpression, s);
                self.tk_punct(K::UnaryExpression, false)?;
                self.expr(EXPR, UNARY)?;
                self.close(n);
                Ok(n)
            }
            T::Amp | T::AmpAmp => self.reference(s),
            T::Pipe | T::PipePipe => self.closure(s),
            T::DotDot | T::DotDotEq => {
                if t.kind == T::DotDotEq {
                    // No range starts with `..=`: it is `..`, then `=` (an assignment's).
                    self.split(T::DotDot, 2);
                }
                let n = self.open_in(K::RangeExpression, s);
                self.tk_punct(K::RangeExpression, false)?;
                if self.optional_operand(false) {
                    self.expr(EXPR, 2)?;
                }
                self.close(n);
                Ok(n)
            }
            T::LParen => self.paren(s),
            T::LBracket => self.array(s),
            T::LBrace => self.blocklike(s),
            T::Lifetime if self.peek_at(1).kind == T::Colon => self.blocklike(s),
            T::Int | T::Float | T::Char | T::Str | T::RawStr => self.literal(s),
            T::Lt | T::ColonColon | T::Metavariable => self.expr_path(s),
            T::Name => self.word_expr(s, t),
            _ => self.refuse(),
        }
    }

    /// An expression that starts with a word: a keyword's, or a path.
    fn word_expr(&mut self, s: Slot, t: Tok) -> R<N> {
        let next = self.peek_at(1);
        match t.word {
            w::IF | w::MATCH | w::LOOP | w::WHILE | w::FOR => self.blocklike(s),
            w::UNSAFE | w::CONST | w::TRY if next.kind == T::LBrace => self.blocklike(s),
            w::ASYNC | w::GEN if next.kind == T::LBrace || next.kind == T::Name && next.word == w::MOVE && self.peek_at(2).kind == T::LBrace => self.blocklike(s),
            w::ASYNC | w::MOVE | w::STATIC if matches!(next.kind, T::Pipe | T::PipePipe) || next.kind == T::Name && matches!(next.word, w::MOVE) => self.closure(s),
            w::UNSAFE | w::CONST | w::TRY | w::ASYNC | w::MOVE | w::LET => self.refuse(),
            w::RETURN | w::YIELD => {
                let kind = if t.word == w::RETURN { K::ReturnExpression } else { K::YieldExpression };
                let n = self.open_in(kind, s);
                self.tk(kind, if t.word == w::RETURN { K::TReturn } else { K::TYield }, f::NONE)?;
                if self.starts_operand() {
                    self.expr(EXPR, 1)?;
                }
                self.close(n);
                Ok(n)
            }
            w::BREAK => {
                let n = self.open_in(K::BreakExpression, s);
                self.tk(K::BreakExpression, K::TBreak, f::NONE)?;
                if self.peek() == T::Lifetime && self.peek_at(1).kind != T::Colon {
                    self.label()?;
                }
                if self.optional_operand(true) {
                    self.expr(EXPR, 1)?;
                }
                self.close(n);
                Ok(n)
            }
            w::CONTINUE => {
                let n = self.open_in(K::ContinueExpression, s);
                self.tk(K::ContinueExpression, K::TContinue, f::NONE)?;
                if self.peek() == T::Lifetime {
                    self.label()?;
                }
                self.close(n);
                Ok(n)
            }
            w::TRUE | w::FALSE => self.literal(s),
            _ => self.expr_path(s),
        }
    }

    // ---- Paths.

    /// A path's first segment: `self`, `super`, `crate`, a metavariable, or an identifier (a
    /// primitive type's or a reserved word too).
    pub(super) fn path_start(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        let kind = match t.kind {
            T::Metavariable => K::Metavariable,
            T::Name => match t.word {
                w::SELF => K::SelfK,
                w::SUPER => K::Super,
                w::CRATE => K::Crate,
                _ => K::Identifier,
            },
            _ => return self.refuse(),
        };
        self.leaf_in(kind, s)
    }

    /// `n::name` (the next tokens `::` and a name): a `scoped_identifier` in slot `s`.
    pub(super) fn scoped_segment(&mut self, n: N, s: Slot) -> R<N> {
        let name = self.peek_at(1);
        if name.kind != T::Name {
            self.i += 1;
            return self.refuse();
        }
        let sc = self.wrap(n, K::ScopedIdentifier, s, slot(f::PATH, Ctx::Bare));
        self.tk(K::ScopedIdentifier, K::TColonColon, f::NONE)?;
        let kind = if name.word == w::SUPER { K::Super } else { K::Identifier };
        self.leaf(kind, f::NAME, C_NONE)?;
        self.close(sc);
        Ok(sc)
    }

    /// A `_path` (an attribute's, a visibility's): segments, no generics.
    pub(super) fn path(&mut self, s: Slot) -> R<N> {
        let mut n = if self.peek() == T::ColonColon {
            let n = self.open_in(K::ScopedIdentifier, s);
            self.tk(K::ScopedIdentifier, K::TColonColon, f::NONE)?;
            self.identifier(slot(f::NAME, Ctx::Bare))?;
            self.close(n);
            n
        } else {
            self.path_start(s)?
        };
        while self.peek() == T::ColonColon && self.peek_at(1).kind == T::Name {
            n = self.scoped_segment(n, s)?;
        }
        Ok(n)
    }

    /// A path in an expression, and what it heads: a macro invocation (`path!(…)`), a struct
    /// (`Path { … }`, outside a condition), a generic function (`f::<T>`).
    fn expr_path(&mut self, s: Slot) -> R<N> {
        // Whether the path's last segment is a plain identifier (a struct's name may be one).
        let mut plain;
        let mut n = match self.peek() {
            T::Lt => {
                let b = self.bracketed_type(slot(f::PATH, Ctx::Bare))?;
                if self.peek() != T::ColonColon || self.peek_at(1).kind != T::Name {
                    return self.refuse();
                }
                let name = self.peek_at(1);
                plain = plain_word(name.word);
                self.scoped_segment(b, s)?
            }
            T::ColonColon => {
                let n = self.open_in(K::ScopedIdentifier, s);
                self.tk(K::ScopedIdentifier, K::TColonColon, f::NONE)?;
                let name = self.tok();
                plain = name.kind == T::Name && plain_word(name.word);
                self.identifier(slot(f::NAME, Ctx::Bare))?;
                self.close(n);
                n
            }
            _ => {
                let t = self.tok();
                plain = t.kind == T::Name && plain_word(t.word);
                self.path_start(s)?
            }
        };
        // The last segment's name, where it is a scoped path's.
        let mut name_at = None;
        loop {
            if self.peek() != T::ColonColon {
                break;
            }
            match self.peek_at(1).kind {
                T::Name => {
                    plain = plain_word(self.peek_at(1).word);
                    n = self.scoped_segment(n, s)?;
                    name_at = Some(self.w.len() - 1);
                }
                T::Lt | T::LtLt => {
                    // A turbofish: the tokens after its `>` say what it heads.
                    let Some(end) = self.angle_end(1) else { return self.refuse() };
                    let after = self.peek_at(end).kind;
                    let inner = if n.kind == K::Identifier { K::TypeIdentifier } else { n.kind };
                    if after == T::ColonColon {
                        // `A::<T>::b`: the path is a `generic_type` (whose `::` is stored).
                        self.rekind(n.at, inner, slot(f::TYPE, Ctx::Bare));
                        let g = self.wrap(N { at: n.at, kind: inner }, K::GenericType, s, slot(f::TYPE, Ctx::Bare));
                        self.leaf(K::TColonColon, f::NONE, C_NONE)?;
                        self.type_arguments()?;
                        self.close(g);
                        n = g;
                        plain = false;
                        name_at = None;
                        continue;
                    }
                    if after == T::LBrace && !self.no_struct && plain {
                        self.rekind(n.at, inner, slot(f::TYPE, Ctx::Bare));
                        let g = self.wrap(N { at: n.at, kind: inner }, K::GenericTypeWithTurbofish, slot(f::NAME, Ctx::Bare), slot(f::TYPE, Ctx::Bare));
                        self.tk(K::GenericTypeWithTurbofish, K::TColonColon, f::NONE)?;
                        self.type_arguments()?;
                        self.close(g);
                        let st = self.wrap(g, K::StructExpression, s, slot(f::NAME, Ctx::Bare));
                        self.field_initializer_list()?;
                        self.close(st);
                        return Ok(st);
                    }
                    let g = self.wrap(n, K::GenericFunction, s, slot(f::FUNCTION, Ctx::Bare));
                    self.tk(K::GenericFunction, K::TColonColon, f::NONE)?;
                    self.type_arguments()?;
                    self.close(g);
                    return Ok(g);
                }
                _ => break,
            }
        }
        match self.peek() {
            T::Bang if matches!(n.kind, K::Identifier | K::ScopedIdentifier) && matches!(self.peek_at(1).kind, T::LParen | T::LBracket | T::LBrace) => {
                let m = self.wrap(n, K::MacroInvocation, s, slot(f::MACRO, Ctx::Bare));
                self.tk(K::MacroInvocation, K::TBang, f::NONE)?;
                self.token_tree(tt::Tt::Delim, f::NONE)?;
                self.close(m);
                Ok(m)
            }
            T::LBrace if !self.no_struct && plain && matches!(n.kind, K::Identifier | K::ScopedIdentifier) => {
                // A struct: its name is a type's.
                if n.kind == K::Identifier {
                    self.rekind(n.at, K::TypeIdentifier, slot(f::NAME, Ctx::Bare));
                } else {
                    self.rekind(n.at, K::ScopedTypeIdentifier, slot(f::NAME, Ctx::Bare));
                    if let Some(at) = name_at {
                        self.rekind(at, K::TypeIdentifier, slot(f::NAME, Ctx::Bare));
                    }
                }
                let st = self.wrap(N { at: n.at, kind: K::TypeIdentifier }, K::StructExpression, s, slot(f::NAME, Ctx::Bare));
                self.field_initializer_list()?;
                self.close(st);
                Ok(st)
            }
            _ => Ok(n),
        }
    }

    /// `{ a, b: c, ..base }`.
    fn field_initializer_list(&mut self) -> R<()> {
        let n = self.open(K::FieldInitializerList, f::BODY, C_NONE);
        let saved = self.no_struct;
        self.no_struct = false;
        self.list(K::FieldInitializerList, T::RBrace, false, true, |p| {
            if p.peek() == T::DotDot {
                let b = p.open(K::BaseFieldInitializer, f::NONE, C_NONE);
                p.tk(K::BaseFieldInitializer, K::TDotDot, f::NONE)?;
                p.expr(EXPR, 0)?;
                p.close(b);
                return Ok(());
            }
            let mut k = 0;
            while p.peek_at(k).kind == T::Hash {
                // Attributes: `#[…]`.
                let mut depth = 0;
                loop {
                    k += 1;
                    match p.peek_at(k).kind {
                        T::LBracket => depth += 1,
                        T::RBracket => {
                            depth -= 1;
                            if depth == 0 {
                                k += 1;
                                break;
                            }
                        }
                        T::Eof => return p.refuse(),
                        _ => {}
                    }
                }
            }
            let shorthand = p.peek_at(k + 1).kind != T::Colon;
            let kind = if shorthand { K::ShorthandFieldInitializer } else { K::FieldInitializer };
            let i = p.open(kind, f::NONE, C_NONE);
            p.outer_attributes()?;
            if shorthand {
                p.identifier(BARE)?;
            } else {
                match p.peek() {
                    T::Name => drop(p.leaf(K::FieldIdentifier, f::FIELD, C_NONE)?),
                    T::Int => drop(p.leaf(K::IntegerLiteral, f::FIELD, C_NONE)?),
                    _ => return p.refuse(),
                }
                p.tk(K::FieldInitializer, K::TColon, f::NONE)?;
                p.expr(slot(f::VALUE, Ctx::Expr), 0)?;
            }
            p.close(i);
            Ok(())
        })?;
        self.no_struct = saved;
        self.close(n);
        Ok(())
    }

    // ---- Literals.

    /// A literal in slot `s`: a string, raw string, character, integer, float or boolean.
    pub(super) fn literal(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Int => self.leaf_in(K::IntegerLiteral, s),
            T::Float => self.leaf_in(K::FloatLiteral, s),
            T::Char => self.leaf_in(K::CharLiteral, s),
            T::Str => {
                let n = self.open_in(K::StringLiteral, s);
                let q = if self.src[t.start as usize] == b'"' { t.start } else { t.start + 1 };
                let mut pieces = std::mem::take(&mut self.pieces);
                pieces.clear();
                self.lx.string_pieces(q as usize, &mut pieces);
                self.bump()?;
                for &(a, b, escape) in &pieces {
                    self.leaf_span(if escape { K::EscapeSequence } else { K::StringContent }, f::NONE, C_NONE, a, b);
                }
                self.pieces = pieces;
                self.close(n);
                Ok(n)
            }
            T::RawStr => {
                let n = self.open_in(K::RawStringLiteral, s);
                let (hashes, text) = self.lx.raw_string_start(t.start as usize).expect("a raw string");
                self.bump()?;
                // The scanner passes blanks before the content starts.
                let end = t.end as usize - 1 - hashes;
                let mut text = text;
                while text < end && matches!(self.src[text], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
                    text += 1;
                }
                self.leaf_span(K::StringContent, f::NONE, C_NONE, text as u32, end as u32);
                self.close(n);
                Ok(n)
            }
            T::Name if matches!(t.word, w::TRUE | w::FALSE) => {
                let n = self.open_in(K::BooleanLiteral, s);
                let k = if t.word == w::TRUE { K::TTrue } else { K::TFalse };
                self.tk(K::BooleanLiteral, k, f::NONE)?;
                self.close(n);
                Ok(n)
            }
            _ => self.refuse(),
        }
    }

    /// A lifetime (`'a`) in slot `s`: its name an identifier.
    pub(super) fn lifetime(&mut self, s: Slot) -> R<()> {
        self.quoted(K::Lifetime, s).map(drop)
    }

    /// A label (`'a`).
    pub(super) fn label(&mut self) -> R<()> {
        self.quoted(K::Label, BARE).map(drop)
    }

    pub(super) fn quoted(&mut self, kind: K, s: Slot) -> R<N> {
        if self.peek() != T::Lifetime {
            return self.refuse();
        }
        let t = self.bump()?;
        let n = self.w.push(key(kind, s.field, C_NONE, false), t.start);
        let i = self.w.push(key(K::Identifier, f::NONE, C_NONE, false), t.start + 1);
        self.w.close(i, t.end);
        self.w.close(n, t.end);
        Ok(N { at: n, kind })
    }

    // ---- Operators and brackets.

    /// `&x`, `&mut x`, `&raw const x`; `&&x` is two.
    fn reference(&mut self, s: Slot) -> R<N> {
        if self.peek() == T::AmpAmp {
            self.split(T::Amp, 1);
        }
        let n = self.open_in(K::ReferenceExpression, s);
        self.tk(K::ReferenceExpression, K::TAmp, f::NONE)?;
        if self.is_word(w::RAW) && (self.is_word_at(1, w::CONST) || self.is_word_at(1, w::MUT)) {
            self.tk(K::ReferenceExpression, K::TRaw, f::NONE)?;
            if self.is_word(w::CONST) {
                self.tk(K::ReferenceExpression, K::TConst, f::NONE)?;
            } else {
                self.leaf(K::MutableSpecifier, f::NONE, C_NONE)?;
            }
        } else if self.is_word(w::MUT) {
            self.leaf(K::MutableSpecifier, f::NONE, C_NONE)?;
        }
        self.expr(slot(f::VALUE, Ctx::Expr), UNARY)?;
        self.close(n);
        Ok(n)
    }

    /// `()`, `(e)`, `(e,)`, `(a, b)`.
    fn paren(&mut self, s: Slot) -> R<N> {
        let saved = self.no_struct;
        self.no_struct = false;
        let n = if self.peek_at(1).kind == T::RParen {
            let n = self.open_in(K::UnitExpression, s);
            self.tk(K::UnitExpression, K::TLParen, f::NONE)?;
            self.tk(K::UnitExpression, K::TRParen, f::NONE)?;
            self.close(n);
            n
        } else {
            let n = self.open_in(K::ParenthesizedExpression, s);
            self.tk(K::ParenthesizedExpression, K::TLParen, f::NONE)?;
            if self.peek() == T::Hash {
                return self.refuse();
            }
            self.expr(EXPR, 0)?;
            match self.peek() {
                T::RParen => self.tk(K::ParenthesizedExpression, K::TRParen, f::NONE)?,
                T::Comma => {
                    let n = N { at: n.at, kind: K::TupleExpression };
                    self.rekind(n.at, K::TupleExpression, s);
                    if self.peek_at(1).kind == T::RParen {
                        // `(a,)`: `, )` is the one word after a first item.
                        self.tk(K::TupleExpression, K::TComma, f::NONE)?;
                        self.tk(K::TupleExpression, K::TRParen, f::NONE)?;
                    } else {
                        self.tk(K::TupleExpression, K::TComma, f::NONE)?;
                        loop {
                            self.expr(EXPR, 0)?;
                            match self.peek() {
                                T::Comma if self.peek_at(1).kind == T::RParen => {
                                    self.leaf(K::TComma, f::NONE, C_NONE)?;
                                    self.leaf(K::TRParen, f::NONE, C_NONE)?;
                                    break;
                                }
                                T::Comma => self.tk(K::TupleExpression, K::TComma, f::NONE)?,
                                T::RParen => {
                                    self.tk(K::TupleExpression, K::TRParen, f::NONE)?;
                                    break;
                                }
                                _ => return self.refuse(),
                            }
                        }
                    }
                    self.close(n);
                    self.no_struct = saved;
                    return Ok(n);
                }
                _ => return self.refuse(),
            }
            self.close(n);
            n
        };
        self.no_struct = saved;
        Ok(n)
    }

    /// `[a, b]`, `[a; n]`.
    fn array(&mut self, s: Slot) -> R<N> {
        let saved = self.no_struct;
        self.no_struct = false;
        let n = self.open_in(K::ArrayExpression, s);
        if self.peek_at(1).kind == T::RBracket {
            self.tk(K::ArrayExpression, K::TLBracket, f::NONE)?;
            self.tk(K::ArrayExpression, K::TRBracket, f::NONE)?;
        } else {
            self.tk(K::ArrayExpression, K::TLBracket, f::NONE)?;
            self.outer_attributes()?;
            self.expr(EXPR, 0)?;
            if self.peek() == T::Semi {
                self.tk(K::ArrayExpression, K::TSemi, f::NONE)?;
                self.expr(slot(f::LENGTH, Ctx::Expr), 0)?;
                self.expect_tk(K::ArrayExpression, T::RBracket, K::TRBracket)?;
            } else {
                loop {
                    match self.peek() {
                        T::Comma if self.peek_at(1).kind == T::RBracket => {
                            self.leaf(K::TComma, f::NONE, C_NONE)?;
                            self.leaf(K::TRBracket, f::NONE, C_NONE)?;
                            break;
                        }
                        T::Comma => {
                            self.tk(K::ArrayExpression, K::TComma, f::NONE)?;
                            self.outer_attributes()?;
                            self.expr(EXPR, 0)?;
                        }
                        T::RBracket => {
                            self.tk(K::ArrayExpression, K::TRBracket, f::NONE)?;
                            break;
                        }
                        _ => return self.refuse(),
                    }
                }
            }
        }
        self.close(n);
        self.no_struct = saved;
        Ok(n)
    }

    /// `( a, b )`: a call's arguments.
    fn arguments(&mut self) -> R<()> {
        let n = self.open(K::Arguments, f::ARGUMENTS, C_NONE);
        let saved = self.no_struct;
        self.no_struct = false;
        self.list(K::Arguments, T::RParen, false, true, |p| {
            p.outer_attributes()?;
            p.expr(EXPR, 0)?;
            Ok(())
        })?;
        self.no_struct = saved;
        self.close(n);
        Ok(())
    }

    /// A closure: `static? async? move? |params| body`, `|| body`, `|x| -> T { … }`.
    fn closure(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ClosureExpression, s);
        loop {
            let t = self.tok();
            match (t.kind, t.word) {
                (T::Name, w::STATIC) => self.tk(K::ClosureExpression, K::TStatic, f::NONE)?,
                (T::Name, w::ASYNC) => self.tk(K::ClosureExpression, K::TAsync, f::NONE)?,
                (T::Name, w::MOVE) => self.tk(K::ClosureExpression, K::TMove, f::NONE)?,
                _ => break,
            }
        }
        match self.peek() {
            T::PipePipe => drop(self.leaf(K::ClosureParameters, f::PARAMETERS, C_NONE)?),
            T::Pipe => {
                let c = self.open(K::ClosureParameters, f::PARAMETERS, C_NONE);
                self.list(K::ClosureParameters, T::Pipe, true, true, |p| {
                    if p.colon_ahead() {
                        let q = p.open(K::Parameter, f::NONE, C_NONE);
                        if p.is_word(w::MUT) {
                            p.leaf(K::MutableSpecifier, f::NONE, C_NONE)?;
                        }
                        p.pattern(slot(f::PATTERN, Ctx::Pattern), K::Parameter, false)?;
                        p.expect_tk(K::Parameter, T::Colon, K::TColon)?;
                        p.ty(slot(f::TYPE, Ctx::Type), false)?;
                        p.close(q);
                        Ok(())
                    } else {
                        p.pattern(PAT, K::ClosureParameters, false)
                    }
                })?;
                self.close(c);
            }
            _ => return self.refuse(),
        }
        if self.peek() == T::Arrow {
            self.tk(K::ClosureExpression, K::TArrow, f::NONE)?;
            self.ty(slot(f::RETURN_TYPE, Ctx::Type), false)?;
            self.block(slot(f::BODY, Ctx::Bare))?;
        } else if self.is_underscore() && !self.continues_after_underscore() {
            self.tk(K::ClosureExpression, K::TUnderscore, f::BODY)?;
        } else {
            self.expr(slot(f::BODY, Ctx::Expr), 0)?;
        }
        self.close(n);
        Ok(n)
    }

    /// Whether an operator follows a `_` (then `_` is an identifier, an operand).
    fn continues_after_underscore(&mut self) -> bool {
        let t = self.peek_at(1);
        binary_op(t.kind).is_some() || matches!(t.kind, T::Dot | T::Question | T::LParen | T::LBracket) || t.kind == T::Name && t.word == w::AS
    }

    // ---- Block-like expressions.

    /// Whether the next tokens start a block-like expression (`_expression_ending_with_block`).
    pub(super) fn blocklike_start(&mut self) -> bool {
        let t = self.tok();
        match t.kind {
            T::LBrace => true,
            T::Lifetime => self.peek_at(1).kind == T::Colon,
            T::Name => match t.word {
                w::IF | w::MATCH | w::LOOP | w::WHILE | w::FOR => true,
                w::UNSAFE | w::CONST | w::TRY => self.peek_at(1).kind == T::LBrace,
                w::ASYNC | w::GEN => self.peek_at(1).kind == T::LBrace || self.is_word_at(1, w::MOVE) && self.peek_at(2).kind == T::LBrace,
                _ => false,
            },
            _ => false,
        }
    }

    /// A block-like expression in slot `s`, a label before it where it has one.
    pub(super) fn blocklike(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let labeled = self.peek() == T::Lifetime;
        let k = if labeled { 2 } else { 0 };
        let t = self.peek_at(k);
        let kind = match (t.kind, t.word) {
            (T::LBrace, _) => K::Block,
            (T::Name, w::IF) => K::IfExpression,
            (T::Name, w::MATCH) => K::MatchExpression,
            (T::Name, w::LOOP) => K::LoopExpression,
            (T::Name, w::WHILE) => K::WhileExpression,
            (T::Name, w::FOR) => K::ForExpression,
            (T::Name, w::UNSAFE) => K::UnsafeBlock,
            (T::Name, w::ASYNC) => K::AsyncBlock,
            (T::Name, w::GEN) => K::GenBlock,
            (T::Name, w::TRY) => K::TryBlock,
            (T::Name, w::CONST) => K::ConstBlock,
            _ => return self.refuse(),
        };
        if labeled && !matches!(kind, K::Block | K::LoopExpression | K::WhileExpression | K::ForExpression) {
            return self.refuse();
        }
        let n = self.open_in(kind, s);
        if labeled {
            self.label()?;
            self.tk(kind, K::TColon, f::NONE)?;
        }
        match kind {
            K::Block => self.block_body()?,
            K::IfExpression => self.if_body()?,
            K::MatchExpression => {
                self.tk(K::MatchExpression, K::TMatch, f::NONE)?;
                let saved = self.no_struct;
                self.no_struct = true;
                self.expr(slot(f::VALUE, Ctx::Expr), 0)?;
                self.no_struct = saved;
                self.match_block()?;
            }
            K::LoopExpression => {
                self.tk(K::LoopExpression, K::TLoop, f::NONE)?;
                self.block(slot(f::BODY, Ctx::Bare))?;
            }
            K::WhileExpression => {
                self.tk(K::WhileExpression, K::TWhile, f::NONE)?;
                self.condition(true)?;
                self.block(slot(f::BODY, Ctx::Bare))?;
            }
            K::ForExpression => {
                self.tk(K::ForExpression, K::TFor, f::NONE)?;
                self.pattern(slot(f::PATTERN, Ctx::Pattern), K::ForExpression, true)?;
                if !self.is_word(w::IN) {
                    return self.refuse();
                }
                self.tk(K::ForExpression, K::TIn, f::NONE)?;
                let saved = self.no_struct;
                self.no_struct = true;
                self.expr(slot(f::VALUE, Ctx::Expr), 0)?;
                self.no_struct = saved;
                self.block(slot(f::BODY, Ctx::Bare))?;
            }
            K::UnsafeBlock => {
                self.tk(K::UnsafeBlock, K::TUnsafe, f::NONE)?;
                self.block(BARE)?;
            }
            K::AsyncBlock | K::GenBlock => {
                self.tk(kind, if kind == K::AsyncBlock { K::TAsync } else { K::TGen }, f::NONE)?;
                if self.is_word(w::MOVE) {
                    self.tk(kind, K::TMove, f::NONE)?;
                }
                self.block(BARE)?;
            }
            K::TryBlock => {
                self.tk(K::TryBlock, K::TTry, f::NONE)?;
                self.block(BARE)?;
            }
            _ => {
                self.tk(K::ConstBlock, K::TConst, f::NONE)?;
                self.block(slot(f::BODY, Ctx::Bare))?;
            }
        }
        self.close(n);
        self.depth -= 1;
        Ok(n)
    }

    /// A block in slot `s`.
    pub(super) fn block(&mut self, s: Slot) -> R<N> {
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        let n = self.open_in(K::Block, s);
        self.block_body()?;
        self.close(n);
        Ok(n)
    }

    /// `{ statements }` (the block open).
    fn block_body(&mut self) -> R<()> {
        self.tk(K::Block, K::TLBrace, f::NONE)?;
        self.statements(Stmts::Block)?;
        self.expect_tk(K::Block, T::RBrace, K::TRBrace)
    }

    /// `if condition { … } else …` (the `if_expression` open).
    fn if_body(&mut self) -> R<()> {
        self.tk(K::IfExpression, K::TIf, f::NONE)?;
        self.condition(true)?;
        self.block(slot(f::CONSEQUENCE, Ctx::Bare))?;
        if self.is_word(w::ELSE) {
            let e = self.open(K::ElseClause, f::ALTERNATIVE, C_NONE);
            self.tk(K::ElseClause, K::TElse, f::NONE)?;
            if self.is_word(w::IF) {
                let i = self.open(K::IfExpression, f::NONE, C_NONE);
                self.deeper()?;
                self.if_body()?;
                self.depth -= 1;
                self.close(i);
            } else {
                self.block(BARE)?;
            }
            self.close(e);
        }
        Ok(())
    }

    /// A condition (`condition:`): an expression, a `let` condition, or a let chain (`let a = b &&
    /// c && let d = e`, flat). In `if` and `while` (`no_struct`) a path before `{` is no struct.
    fn condition(&mut self, no_struct: bool) -> R<()> {
        let saved = self.no_struct;
        self.no_struct = no_struct;
        let (first, inner) = if self.is_word(w::LET) {
            (self.let_condition(slot(f::CONDITION, Ctx::Bare))?, BARE)
        } else {
            (self.expr(slot(f::CONDITION, Ctx::Expr), 0)?, EXPR)
        };
        if self.peek() == T::AmpAmp && (first.kind == K::LetCondition || self.is_word_at(1, w::LET)) {
            let c = self.wrap(first, K::LetChain, slot(f::CONDITION, Ctx::Bare), inner);
            // In the chain, `&&` goes on (a guard's condition reads it so too).
            self.no_struct = true;
            while self.peek() == T::AmpAmp {
                self.leaf(K::TAmpAmp, f::NONE, C_NONE)?;
                if self.is_word(w::LET) {
                    self.no_struct = no_struct;
                    self.let_condition(BARE)?;
                } else {
                    self.no_struct = no_struct;
                    self.expr(EXPR, 4)?;
                }
                self.no_struct = true;
            }
            self.close(c);
        }
        self.no_struct = saved;
        Ok(())
    }

    /// `let pattern = value`: the value's operators above `&&`.
    fn let_condition(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::LetCondition, s);
        self.tk(K::LetCondition, K::TLet, f::NONE)?;
        self.pattern(slot(f::PATTERN, Ctx::Pattern), K::LetCondition, true)?;
        self.expect_tk(K::LetCondition, T::Eq, K::TEq)?;
        self.expr(slot(f::VALUE, Ctx::Expr), 4)?;
        self.close(n);
        Ok(n)
    }

    /// `{ arm… }`.
    fn match_block(&mut self) -> R<()> {
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        let n = self.open(K::MatchBlock, f::BODY, C_NONE);
        let saved = self.no_struct;
        self.no_struct = false;
        self.tk(K::MatchBlock, K::TLBrace, f::NONE)?;
        while self.peek() != T::RBrace {
            self.match_arm()?;
        }
        self.tk(K::MatchBlock, K::TRBrace, f::NONE)?;
        self.no_struct = saved;
        self.close(n);
        Ok(())
    }

    /// `pattern if guard => value,`. A block-like value ends the arm without a `,` (no supertype
    /// then: tree-sitter's `match_arm` names it directly); the last arm's is an expression.
    fn match_arm(&mut self) -> R<()> {
        let a = self.open(K::MatchArm, f::NONE, C_NONE);
        while self.peek() == T::Hash {
            if self.peek_at(1).kind == T::Bang {
                self.inner_attribute_item()?;
            } else {
                self.attribute_item()?;
            }
        }
        let m = self.open(K::MatchPattern, f::PATTERN, C_NONE);
        self.pattern(PAT, K::MatchPattern, true)?;
        if self.is_word(w::IF) {
            self.tk(K::MatchPattern, K::TIf, f::NONE)?;
            self.condition(false)?;
        }
        self.close(m);
        self.expect_tk(K::MatchArm, T::FatArrow, K::TFatArrow)?;
        let value = slot(f::VALUE, Ctx::Expr);
        let (n, ended) = if self.blocklike_start() {
            let n = self.blocklike(value)?;
            if self.continues_expression() { (self.infix(n, value, 0)?, false) } else { (n, true) }
        } else {
            (self.expr(value, 0)?, false)
        };
        match self.peek() {
            T::Comma => self.tk(K::MatchArm, K::TComma, f::NONE)?,
            T::RBrace => {}
            _ if ended => self.rekey(n.at, slot(f::VALUE, Ctx::Bare)),
            _ => return self.refuse(),
        }
        self.close(a);
        Ok(())
    }
}
