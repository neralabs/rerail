//! Expressions: precedence climbing over tree-sitter-java's `PREC`, primaries and their postfix
//! chains, casts and lambdas (by looking ahead at `(`), creations, and strings (their pieces read
//! from the bytes).

use super::*;

// tree-sitter-java's `PREC`.
const P_ASSIGN: u8 = 1;
const P_TERNARY: u8 = 3;
const P_REL: u8 = 10;
const P_CAST: u8 = 14;
const P_UNARY: u8 = 15;

/// A binary operator's precedence and its kind.
fn binary(t: T) -> Option<(u8, K)> {
    use T::*;
    Some(match t {
        PipePipe => (4, K::TPipePipe),
        AmpAmp => (5, K::TAmpAmp),
        Pipe => (6, K::TPipe),
        Caret => (7, K::TCaret),
        Amp => (8, K::TAmp),
        EqEq => (9, K::TEqEq),
        Ne => (9, K::TNe),
        Lt => (10, K::TLt),
        Gt => (10, K::TGt),
        Le => (10, K::TLe),
        Ge => (10, K::TGe),
        LtLt => (11, K::TLtLt),
        GtGt => (11, K::TGtGt),
        GtGtGt => (11, K::TGtGtGt),
        Plus => (12, K::TPlus),
        Minus => (12, K::TMinus),
        Star => (13, K::TStar),
        Slash => (13, K::TSlash),
        Percent => (13, K::TPercent),
        _ => return None,
    })
}

fn assignment(t: T) -> Option<K> {
    use T::*;
    Some(match t {
        Eq => K::TEq,
        PlusEq => K::TPlusEq,
        MinusEq => K::TMinusEq,
        StarEq => K::TStarEq,
        SlashEq => K::TSlashEq,
        AmpEq => K::TAmpEq,
        PipeEq => K::TPipeEq,
        CaretEq => K::TCaretEq,
        PercentEq => K::TPercentEq,
        LtLtEq => K::TLtLtEq,
        GtGtEq => K::TGtGtEq,
        GtGtGtEq => K::TGtGtGtEq,
        _ => return None,
    })
}

/// Whether a token can start an expression's operand (a cast's, after `(T)`).
fn starts_operand(t: Tok) -> bool {
    use T::*;
    match t.kind {
        Name => !reserved(t.word) || matches!(t.word, w::THIS | w::SUPER | w::NEW | w::TRUE | w::FALSE | w::NULL | w::SWITCH) || w::is_primitive(t.word),
        Char | Dec | Hex | Oct | Bin | DecFloat | HexFloat | Quote | TripleQuote | LParen | Bang | Tilde => true,
        _ => false,
    }
}

/// Whether a token after `yield` continues it as a name (`yield = 1`, `yield.f()`), not a
/// `yield` statement's value.
pub(super) fn continues_name(t: T) -> bool {
    use T::*;
    assignment(t).is_some() || matches!(t, Dot | LBracket | PlusPlus | MinusMinus | Arrow | ColonColon | Semi | RParen | Comma)
}

impl<'a> P<'a> {
    /// An expression in slot `s`.
    pub(super) fn expr(&mut self, s: Slot) -> R<N> {
        self.expr_prec(s, 0)
    }

    /// An expression whose operators bind at `min` or tighter.
    pub(super) fn expr_prec(&mut self, s: Slot, min: u8) -> R<N> {
        self.deeper()?;
        let mut left = self.unary(s)?;
        loop {
            let t = self.tok();
            if let Some(op) = assignment(t.kind) {
                if min > P_ASSIGN {
                    break;
                }
                if !matches!(left.kind, K::Identifier | K::FieldAccess | K::ArrayAccess) {
                    return self.refuse();
                }
                left = self.wrap(left, K::AssignmentExpression, s, slot(f::LEFT, Ctx::Bare));
                self.leaf(op, f::OPERATOR, C_NONE)?;
                self.expr_prec(slot(f::RIGHT, Ctx::Expr), P_ASSIGN)?;
                self.close(left);
                continue;
            }
            match t.kind {
                T::Question if min <= P_TERNARY => {
                    left = self.wrap(left, K::TernaryExpression, s, slot(f::CONDITION, Ctx::Expr));
                    self.tk(K::TernaryExpression, K::TQuestion)?;
                    self.expr(slot(f::CONSEQUENCE, Ctx::Expr))?;
                    self.punct_tk(K::TernaryExpression, T::Colon, false)?;
                    self.expr_prec(slot(f::ALTERNATIVE, Ctx::Expr), P_TERNARY)?;
                    self.close(left);
                }
                T::PlusPlus | T::MinusMinus if min <= P_UNARY => {
                    left = self.wrap(left, K::UpdateExpression, s, EXPR);
                    self.leaf(punct(t.kind).expect("an operator"), f::NONE, C_NONE)?;
                    self.close(left);
                }
                T::Name if t.word == w::INSTANCEOF && min <= P_REL => {
                    left = self.wrap(left, K::InstanceofExpression, s, slot(f::LEFT, Ctx::Expr));
                    self.leaf(K::TInstanceof, f::NONE, C_NONE)?;
                    if self.is_word(w::FINAL) {
                        self.leaf(K::TFinal, f::NONE, C_NONE)?;
                    }
                    if self.record_pattern_ahead(0) {
                        self.record_pattern(slot(f::PATTERN, Ctx::Bare))?;
                    } else {
                        self.ty(slot(f::RIGHT, Ctx::Type), true)?;
                        if self.is_ident() && !self.is_word(w::INSTANCEOF) {
                            self.leaf(K::Identifier, f::NAME, C_NONE)?;
                        }
                    }
                    self.close(left);
                }
                _ => {
                    let Some((p, op)) = binary(t.kind) else { break };
                    if p < min {
                        break;
                    }
                    left = self.wrap(left, K::BinaryExpression, s, slot(f::LEFT, Ctx::Expr));
                    self.leaf(op, f::OPERATOR, C_NONE)?;
                    self.expr_prec(slot(f::RIGHT, Ctx::Expr), p + 1)?;
                    self.close(left);
                }
            }
        }
        self.depth -= 1;
        Ok(left)
    }

    /// A prefix expression: unary, a prefix update, a cast, a lambda, a `switch`, or a primary.
    fn unary(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Plus | T::Minus | T::Bang | T::Tilde => {
                let n = self.open_in(K::UnaryExpression, s);
                self.leaf(punct(t.kind).expect("an operator"), f::OPERATOR, C_NONE)?;
                self.expr_prec(slot(f::OPERAND, Ctx::Expr), P_UNARY + 1)?;
                self.close(n);
                Ok(n)
            }
            T::PlusPlus | T::MinusMinus => {
                let n = self.open_in(K::UpdateExpression, s);
                self.leaf(punct(t.kind).expect("an operator"), f::NONE, C_NONE)?;
                self.expr_prec(EXPR, P_UNARY + 1)?;
                self.close(n);
                Ok(n)
            }
            T::LParen => self.paren(s),
            T::Name if t.word == w::SWITCH => self.switch_expression(s),
            T::Name if self.kind_at(1) == T::Arrow && !reserved(t.word) && !self.no_lambda => {
                let n = self.open_in(K::LambdaExpression, s);
                self.leaf(K::Identifier, f::PARAMETERS, C_NONE)?;
                self.lambda_body()?;
                self.close(n);
                Ok(n)
            }
            _ => self.primary(s),
        }
    }

    fn lambda_body(&mut self) -> R<()> {
        self.tk(K::LambdaExpression, K::TArrow)?;
        if self.peek() == T::LBrace {
            self.block(slot(f::BODY, Ctx::Bare))?;
        } else {
            self.expr(slot(f::BODY, Ctx::Expr))?;
        }
        Ok(())
    }

    /// At `(`: a lambda's parameters where `->` follows the `)`, a cast where a type and `)`
    /// come first and an operand follows, else a parenthesized expression.
    fn paren(&mut self, s: Slot) -> R<N> {
        let Some(close) = self.skip_brackets(0) else { return self.refuse() };
        if self.kind_at(close) == T::Arrow && !self.no_lambda {
            let n = self.open_in(K::LambdaExpression, s);
            if self.inferred_ahead(close) {
                let p = self.open(K::InferredParameters, f::PARAMETERS, C_NONE);
                self.list(K::InferredParameters, T::RParen, true, false, |p| {
                    if !p.is_ident() {
                        return p.refuse();
                    }
                    p.leaf(K::Identifier, f::NONE, C_NONE)?;
                    Ok(())
                })?;
                self.close(p);
            } else {
                self.formal_parameters(f::PARAMETERS)?;
            }
            self.lambda_body()?;
            self.close(n);
            return Ok(n);
        }
        if let Some(cast) = self.cast_ahead(close) {
            return self.cast(s, cast);
        }
        let n = self.open_in(K::ParenthesizedExpression, s);
        self.tk(K::ParenthesizedExpression, K::TLParen)?;
        self.nested(|p| p.expr(EXPR))?;
        self.punct_tk(K::ParenthesizedExpression, T::RParen, false)?;
        self.close(n);
        self.postfix(n, s)
    }

    /// Whether the parentheses ending at `close` hold names alone, between `,`s.
    fn inferred_ahead(&mut self, close: usize) -> bool {
        if close == 2 {
            return false;
        }
        let mut k = 1;
        loop {
            if !self.is_ident_at(k) {
                return false;
            }
            k += 1;
            match self.kind_at(k) {
                T::Comma => k += 1,
                T::RParen => return k + 1 == close,
                _ => return false,
            }
        }
    }

    /// Whether the parentheses ending at `close` are a cast: `(` types between `&`s `)`, then an
    /// operand. A name alone in them is a cast only before what cannot continue an expression
    /// (`(a) - b` is a difference). Whether it is an intersection (`(A & B)`).
    fn cast_ahead(&mut self, close: usize) -> Option<bool> {
        let mut k = 1;
        let mut types = 0;
        let mut only_name = true;
        loop {
            let j = self.scan_annotations(k)?;
            if j != k {
                only_name = false;
            }
            let t = self.peek_at(j);
            let end = self.scan_type(j, true)?;
            if w::is_primitive(t.word) {
                only_name = false;
            } else {
                for i in j..end {
                    if !matches!(self.kind_at(i), T::Name | T::Dot) {
                        only_name = false;
                    }
                }
            }
            types += 1;
            k = end;
            match self.kind_at(k) {
                T::Amp => k += 1,
                T::RParen => break,
                _ => return None,
            }
        }
        if k + 1 != close {
            return None;
        }
        let next = self.peek_at(close);
        if only_name && types == 1 {
            // A name alone: a cast before an operand that cannot be a binary operator's.
            if !starts_operand(next) {
                return None;
            }
        } else if !(starts_operand(next) || matches!(next.kind, T::Plus | T::Minus | T::PlusPlus | T::MinusMinus)) {
            return None;
        }
        Some(types > 1)
    }

    /// `cast_expression`: `(` types `)` and its operand, a primary (or a lambda) in the
    /// intersection's form, which tree-sitter takes for one type too.
    fn cast(&mut self, s: Slot, _intersection: bool) -> R<N> {
        let n = self.open_in(K::CastExpression, s);
        self.tk(K::CastExpression, K::TLParen)?;
        loop {
            self.ty(slot(f::TYPE, Ctx::Type), true)?;
            if self.peek() != T::Amp {
                break;
            }
            self.tk(K::CastExpression, K::TAmp)?;
        }
        self.punct_tk(K::CastExpression, T::RParen, false)?;
        let v = self.expr_prec(slot(f::VALUE, Ctx::Expr), P_CAST + 1)?;
        if is_primary(v.kind) {
            self.rekey(v.at, slot(f::VALUE, Ctx::Primary));
        } else if v.kind == K::LambdaExpression {
            self.rekey(v.at, slot(f::VALUE, Ctx::Bare));
        }
        self.close(n);
        Ok(n)
    }

    /// A primary expression and its postfix chain (`.`, `[…]`, `::`).
    pub(super) fn primary(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        let n = match t.kind {
            T::Dec => self.leaf_in(K::DecimalIntegerLiteral, s)?,
            T::Hex => self.leaf_in(K::HexIntegerLiteral, s)?,
            T::Oct => self.leaf_in(K::OctalIntegerLiteral, s)?,
            T::Bin => self.leaf_in(K::BinaryIntegerLiteral, s)?,
            T::DecFloat => self.leaf_in(K::DecimalFloatingPointLiteral, s)?,
            T::HexFloat => self.leaf_in(K::HexFloatingPointLiteral, s)?,
            T::Char => self.leaf_in(K::CharacterLiteral, s)?,
            T::Quote | T::TripleQuote => self.string(s)?,
            T::LParen => return self.paren(s),
            T::Name => match t.word {
                w::TRUE => self.leaf_in(K::True, s)?,
                w::FALSE => self.leaf_in(K::False, s)?,
                w::NULL => self.leaf_in(K::NullLiteral, s)?,
                w::THIS => {
                    if self.kind_at(1) == T::LParen {
                        return self.refuse();
                    }
                    self.leaf_in(K::This, s)?
                }
                w::SUPER => {
                    if !matches!(self.kind_at(1), T::Dot | T::ColonColon) {
                        return self.refuse();
                    }
                    self.leaf_in(K::Super, s)?
                }
                w::NEW => self.creation(s)?,
                code if w::is_primitive(code) => return self.type_led(s),
                // A keyword no expression starts with is not valid here: tree-sitter lexes the word
                // as a name (`case null, default`).
                code if reserved(code) && !matches!(self.kind_at(1), T::LParen | T::Dot) => self.leaf_in(K::Identifier, s)?,
                code if reserved(code) => return self.refuse(),
                _ => {
                    if self.type_led_ahead() {
                        return self.type_led(s);
                    }
                    if self.kind_at(1) == T::LParen {
                        let n = self.open_in(K::MethodInvocation, s);
                        self.leaf(K::Identifier, f::NAME, C_NONE)?;
                        self.arguments()?;
                        self.close(n);
                        n
                    } else {
                        self.leaf_in(K::Identifier, s)?
                    }
                }
            },
            _ => return self.refuse(),
        };
        self.postfix(n, s)
    }

    /// Whether a name starts a type in an expression: a class literal's (`a.B.class`, `B[].class`)
    /// or a method reference's where it is no primary (`B<C>::m`, `B[]::new`).
    fn type_led_ahead(&mut self) -> bool {
        let mut k = 1;
        while self.kind_at(k) == T::Dot && self.is_ident_at(k + 1) {
            k += 2;
        }
        let mut typed = false;
        if self.kind_at(k) == T::Lt {
            let Some(end) = self.scan_type(0, false) else { return false };
            k = end;
            typed = true;
        }
        if self.dims_ahead(k) {
            let Some(end) = self.scan_type(0, true) else { return false };
            k = end;
            typed = true;
        }
        match self.kind_at(k) {
            T::Dot => self.is_word_at(k + 1, w::CLASS),
            T::ColonColon => typed,
            _ => false,
        }
    }

    /// A type in an expression: a class literal (`T.class`) or a method reference (`T::m`).
    fn type_led(&mut self, s: Slot) -> R<N> {
        if self.scan_type(0, true).is_some_and(|k| self.kind_at(k) == T::ColonColon) {
            let n = self.open_in(K::MethodReference, s);
            self.ty(TYPE, true)?;
            self.method_reference_tail()?;
            self.close(n);
            return self.postfix(n, s);
        }
        let n = self.open_in(K::ClassLiteral, s);
        self.unannotated(UTYPE, true)?;
        self.tk(K::ClassLiteral, K::TDot)?;
        if !self.is_word(w::CLASS) {
            return self.refuse();
        }
        self.tk(K::ClassLiteral, K::TClass)?;
        self.close(n);
        self.postfix(n, s)
    }

    /// After a method reference's left: `::`, type arguments, and `new` or a name.
    fn method_reference_tail(&mut self) -> R<()> {
        self.tk(K::MethodReference, K::TColonColon)?;
        if self.peek() == T::Lt {
            self.type_arguments(f::NONE)?;
        }
        if self.is_word(w::NEW) {
            self.tk(K::MethodReference, K::TNew)
        } else if self.is_ident() {
            self.leaf(K::Identifier, f::NONE, C_NONE)?;
            Ok(())
        } else {
            self.refuse()
        }
    }

    /// A primary's postfix chain: field accesses, calls, array accesses, method references,
    /// qualified creations and templates.
    fn postfix(&mut self, mut n: N, s: Slot) -> R<N> {
        loop {
            match self.peek() {
                T::Dot => {
                    let t1 = self.peek_at(1);
                    match t1.kind {
                        T::Name if t1.word == w::NEW => {
                            n = self.wrap(n, K::ObjectCreationExpression, s, slot(f::NONE, Ctx::Primary));
                            self.tk(K::ObjectCreationExpression, K::TDot)?;
                            self.unqualified_creation()?;
                            self.close(n);
                        }
                        T::Lt => {
                            n = self.wrap(n, K::MethodInvocation, s, slot(f::OBJECT, Ctx::Primary));
                            self.tk(K::MethodInvocation, K::TDot)?;
                            self.type_arguments(f::TYPE_ARGUMENTS)?;
                            if !self.is_ident() || self.kind_at(1) != T::LParen {
                                return self.refuse();
                            }
                            self.leaf(K::Identifier, f::NAME, C_NONE)?;
                            self.arguments()?;
                            self.close(n);
                        }
                        T::Name if t1.word == w::SUPER => {
                            // `a.super(…)` is a constructor's call: its statement reads it.
                            if self.kind_at(2) != T::Dot {
                                return Ok(n);
                            }
                            let call = self.kind_at(4) == T::LParen || self.kind_at(3) == T::Lt;
                            let kind = if call { K::MethodInvocation } else { K::FieldAccess };
                            n = self.wrap(n, kind, s, slot(f::OBJECT, Ctx::Primary));
                            self.tk(kind, K::TDot)?;
                            self.leaf(K::Super, f::NONE, C_NONE)?;
                            self.tk(kind, K::TDot)?;
                            self.member_tail(kind)?;
                            self.close(n);
                        }
                        T::Name if t1.word == w::THIS => {
                            n = self.wrap(n, K::FieldAccess, s, slot(f::OBJECT, Ctx::Primary));
                            self.tk(K::FieldAccess, K::TDot)?;
                            self.leaf(K::This, f::FIELD, C_NONE)?;
                            self.close(n);
                        }
                        T::Quote | T::TripleQuote => {
                            n = self.wrap(n, K::TemplateExpression, s, slot(f::TEMPLATE_PROCESSOR, Ctx::Primary));
                            self.tk(K::TemplateExpression, K::TDot)?;
                            self.string(slot(f::TEMPLATE_ARGUMENT, Ctx::Bare))?;
                            self.close(n);
                        }
                        T::Name if !reserved(t1.word) => {
                            let kind = if self.kind_at(2) == T::LParen { K::MethodInvocation } else { K::FieldAccess };
                            n = self.wrap(n, kind, s, slot(f::OBJECT, Ctx::Primary));
                            self.tk(kind, K::TDot)?;
                            self.member_tail(kind)?;
                            self.close(n);
                        }
                        _ => return Ok(n),
                    }
                }
                T::LBracket => {
                    n = self.wrap(n, K::ArrayAccess, s, slot(f::ARRAY, Ctx::Primary));
                    self.tk(K::ArrayAccess, K::TLBracket)?;
                    self.nested(|p| p.expr(slot(f::INDEX, Ctx::Expr)))?;
                    self.punct_tk(K::ArrayAccess, T::RBracket, false)?;
                    self.close(n);
                }
                T::ColonColon => {
                    n = self.wrap(n, K::MethodReference, s, slot(f::NONE, Ctx::Primary));
                    self.method_reference_tail()?;
                    self.close(n);
                }
                _ => return Ok(n),
            }
        }
    }

    /// After `.`: a method's type arguments, name and arguments (`MethodInvocation`), or a field.
    fn member_tail(&mut self, kind: K) -> R<()> {
        if kind == K::MethodInvocation {
            if self.peek() == T::Lt {
                self.type_arguments(f::TYPE_ARGUMENTS)?;
            }
            if !self.is_ident() {
                return self.refuse();
            }
            self.leaf(K::Identifier, f::NAME, C_NONE)?;
            self.arguments()
        } else {
            if !self.is_ident() {
                return self.refuse();
            }
            self.leaf(K::Identifier, f::FIELD, C_NONE)?;
            Ok(())
        }
    }

    /// `argument_list`: `(` expressions between `,`s `)`.
    pub(super) fn arguments(&mut self) -> R<()> {
        if self.peek() != T::LParen {
            return self.refuse();
        }
        let n = self.open(K::ArgumentList, f::ARGUMENTS, C_NONE);
        self.nested(|p| p.list(K::ArgumentList, T::RParen, true, false, |p| p.expr(EXPR).map(|_| ())))?;
        self.close(n);
        Ok(())
    }

    /// At `new`: an object creation, or an array creation where `[` follows the type.
    fn creation(&mut self, s: Slot) -> R<N> {
        let mut k = 1;
        if self.kind_at(k) == T::Lt {
            return self.object_creation(s);
        }
        k = match self.scan_annotations(k) {
            Some(k) => k,
            None => return self.refuse(),
        };
        let Some(end) = self.scan_type(k, false) else { return self.refuse() };
        if self.kind_at(end) == T::LParen {
            return self.object_creation(s);
        }
        let n = self.open_in(K::ArrayCreationExpression, s);
        self.tk(K::ArrayCreationExpression, K::TNew)?;
        while self.peek() == T::At {
            self.annotation(BARE)?;
        }
        self.unannotated(slot(f::TYPE, Ctx::Bare), false)?;
        if self.dims_ahead(0) {
            self.dimensions(f::DIMENSIONS)?;
            if self.peek() != T::LBrace {
                return self.refuse();
            }
            self.array_initializer(f::VALUE)?;
        } else {
            let mut any = false;
            loop {
                let Some(j) = self.scan_annotations(0) else { break };
                if self.kind_at(j) != T::LBracket || self.kind_at(j + 1) == T::RBracket {
                    break;
                }
                let d = self.open(K::DimensionsExpr, f::DIMENSIONS, C_NONE);
                while self.peek() == T::At {
                    self.annotation(BARE)?;
                }
                self.tk(K::DimensionsExpr, K::TLBracket)?;
                self.expr(EXPR)?;
                self.punct_tk(K::DimensionsExpr, T::RBracket, false)?;
                self.close(d);
                any = true;
            }
            if !any {
                return self.refuse();
            }
            if self.dims_ahead(0) {
                self.dimensions(f::DIMENSIONS)?;
            }
        }
        self.close(n);
        Ok(n)
    }

    fn object_creation(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ObjectCreationExpression, s);
        self.unqualified_creation()?;
        self.close(n);
        Ok(n)
    }

    /// `new`, type arguments and annotations, the type, the arguments and a class body.
    fn unqualified_creation(&mut self) -> R<()> {
        if !self.is_word(w::NEW) {
            return self.refuse();
        }
        self.tk(K::ObjectCreationExpression, K::TNew)?;
        while self.peek() == T::At {
            self.annotation(BARE)?;
        }
        if self.peek() == T::Lt {
            self.type_arguments(f::TYPE_ARGUMENTS)?;
            while self.peek() == T::At {
                self.annotation(BARE)?;
            }
        }
        self.unannotated(slot(f::TYPE, Ctx::Bare), false)?;
        self.arguments()?;
        if self.peek() == T::LBrace {
            self.class_body(K::ClassBody, f::NONE)?;
        }
        Ok(())
    }

    /// `array_initializer`: `{` initializers between `,`s, a trailing `,` allowed `}`.
    pub(super) fn array_initializer(&mut self, field: u8) -> R<()> {
        let n = self.open(K::ArrayInitializer, field, C_NONE);
        self.nested(|p| p.list(K::ArrayInitializer, T::RBrace, true, true, |p| p.variable_initializer(EXPR)))?;
        self.close(n);
        Ok(())
    }

    /// A `_variable_initializer`: an expression or an array initializer.
    pub(super) fn variable_initializer(&mut self, s: Slot) -> R<()> {
        if self.peek() == T::LBrace {
            self.array_initializer(s.field)
        } else {
            self.expr(s).map(|_| ())
        }
    }

    // ---- Strings.

    /// A `string_literal` (a `"` or `"""` token over the whole literal): its pieces, read from the
    /// bytes. An interpolation's expression is lexed from inside it.
    pub(super) fn string(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::StringLiteral, s);
        let t = self.tok();
        let block = t.kind == T::TripleQuote;
        let (start, end) = (t.start as usize, t.end as usize);
        let src = self.src;
        let mut at = start + if block { 3 } else { 1 };
        let close = end - if block { 3 } else { 1 };
        self.bump()?;
        // An empty `""`: two words of the kind are as short (`""` and `""""""`), so both are stored.
        // So is the opener before an escape or an interpolation, which a text block's `"""` can
        // stand before too.
        let empty = at == close;
        if block {
            self.piece(K::TTripleQuote, start, at);
        } else if empty || src[at] == b'\\' {
            self.piece(K::TDQuote, start, at);
        }
        let mut relexed = false;
        while at < close {
            match src[at] {
                b'"' => {
                    // A text block's `"` and what follows it to the next `"` or `\` (`\"` passed).
                    let mut e = at + 1;
                    while e < close && src[e] != b'"' {
                        if src[e] == b'\\' {
                            if src.get(e + 1) != Some(&b'"') {
                                break;
                            }
                            e += 1;
                        }
                        e += 1;
                    }
                    self.piece(K::MultilineStringFragment, at, e);
                    at = e;
                }
                b'\\' if src.get(at + 1) == Some(&b'{') => {
                    let i = self.open_at(K::StringInterpolation, at as u32);
                    self.relex_at(at + 2);
                    self.last_end = at as u32 + 2;
                    self.expr(EXPR)?;
                    if self.peek() != T::RBrace {
                        return self.refuse();
                    }
                    self.bump()?;
                    self.w.close(i.at, self.last_end);
                    at = self.last_end as usize;
                    relexed = true;
                }
                b'\\' => {
                    let Some(e) = self.lx.escape_end(at).filter(|&e| e <= close) else { return self.refuse() };
                    if block && e == at + 2 && !b"bfnrts'\"\\".contains(&src[at + 1]) {
                        // A text block's `\` and a character no escape names: a token the tape
                        // leaves out (`_escape_sequence`'s first form).
                        at = e;
                        continue;
                    }
                    self.piece(K::EscapeSequence, at, e);
                    at = e;
                }
                _ => {
                    let mut e = at + 1;
                    while e < close && src[e] != b'"' && src[e] != b'\\' {
                        e += 1;
                    }
                    self.piece(if block { K::MultilineStringFragment } else { K::StringFragment }, at, e);
                    at = e;
                }
            }
        }
        if at != close {
            return self.refuse();
        }
        if block || empty {
            self.piece(if block { K::TTripleQuote } else { K::TDQuote }, close, end);
        }
        if relexed {
            self.relex_at(end);
        }
        self.last_end = end as u32;
        self.close(n);
        Ok(n)
    }

    /// A string's piece over `start..end`.
    fn piece(&mut self, kind: K, start: usize, end: usize) {
        let at = self.w.push(key(kind, f::NONE, C_NONE, false), start as u32);
        self.w.close(at, end as u32);
    }

    /// A node opened at byte `start` (inside a string).
    fn open_at(&mut self, kind: K, start: u32) -> N {
        N { at: self.w.push(key(kind, f::NONE, C_NONE, false), start), kind }
    }
}
