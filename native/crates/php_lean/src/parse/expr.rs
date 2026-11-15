//! Expressions: precedence climbing over tree-sitter-php's `PREC`, the primary expressions and
//! their postfix chains (`->`, `?->`, `::`, `[`, calls, `++`).
//!
//! **An assignable operand before `=`** (a variable, a member or a call, an element) is the
//! assignment's left at any precedence: tree-sitter shifts `=` (precedence 4) over reducing the
//! variable to an expression (precedence 0), so `!$a = f()` is `!($a = f())` and `$a + $b = 1` is
//! `$a + ($b = 1)`.

use super::*;

// `PREC` (higher binds tighter).
const P_OR2: u8 = 1;
const P_XOR: u8 = 2;
const P_AND2: u8 = 3;
const P_ASSIGN: u8 = 4;
const P_TERNARY: u8 = 5;
const P_NULL_COALESCE: u8 = 6;
const P_OR1: u8 = 7;
const P_AND1: u8 = 8;
const P_BOR: u8 = 9;
const P_BXOR: u8 = 10;
const P_BAND: u8 = 11;
const P_EQUALITY: u8 = 12;
const P_INEQUALITY: u8 = 13;
const P_PIPE: u8 = 14;
const P_CONCAT: u8 = 15;
const P_SHIFT: u8 = 16;
const P_PLUS: u8 = 17;
const P_TIMES: u8 = 18;
const P_EXP: u8 = 19;
const P_NEG: u8 = 20;
const P_INSTANCEOF: u8 = 21;
const P_INC: u8 = 22;

/// Whether `k` is a `_variable`'s kind (an assignment's left, `++`'s argument).
fn is_variable(k: K) -> bool {
    use K::*;
    matches!(
        k,
        VariableName
            | DynamicVariableName
            | SubscriptExpression
            | MemberAccessExpression
            | NullsafeMemberAccessExpression
            | ScopedPropertyAccessExpression
            | MemberCallExpression
            | NullsafeMemberCallExpression
            | FunctionCallExpression
            | ScopedCallExpression
    )
}

impl<'a> P<'a> {
    /// A binary operator's precedence, whether it is right-associative, and its token.
    fn binary_op(&mut self) -> Option<(u8, bool, K)> {
        let t = self.tok();
        Some(match t.kind {
            T::QQ => (P_NULL_COALESCE, true, K::TQQ),
            T::StarStar => (P_EXP, true, K::TStarStar),
            T::PipePipe => (P_OR1, false, K::TPipePipe),
            T::AmpAmp => (P_AND1, false, K::TAmpAmp),
            T::Pipe => (P_BOR, false, K::TPipe),
            T::Caret => (P_BXOR, false, K::TCaret),
            T::Amp => (P_BAND, false, K::TAmp),
            T::EqEq => (P_EQUALITY, false, K::TEqEq),
            T::Ne => (P_EQUALITY, false, K::TNe),
            T::LtGt => (P_EQUALITY, false, K::TLtGt),
            T::EqEqEq => (P_EQUALITY, false, K::TEqEqEq),
            T::NeEq => (P_EQUALITY, false, K::TNeEq),
            T::Spaceship => (P_EQUALITY, false, K::TSpaceship),
            T::Lt => (P_INEQUALITY, false, K::TLt),
            T::Gt => (P_INEQUALITY, false, K::TGt),
            T::Le => (P_INEQUALITY, false, K::TLe),
            T::Ge => (P_INEQUALITY, false, K::TGe),
            T::PipeGt => (P_PIPE, false, K::TPipeGt),
            T::Dot => (P_CONCAT, false, K::TDot),
            T::LtLt => (P_SHIFT, false, K::TLtLt),
            T::GtGt => (P_SHIFT, false, K::TGtGt),
            T::Plus => (P_PLUS, false, K::TPlus),
            T::Minus => (P_PLUS, false, K::TMinus),
            T::Star => (P_TIMES, false, K::TStar),
            T::Slash => (P_TIMES, false, K::TSlash),
            T::Percent => (P_TIMES, false, K::TPercent),
            T::Name => match t.word {
                w::AND => (P_AND2, false, K::TAnd),
                w::OR => (P_OR2, false, K::TOr),
                w::XOR => (P_XOR, false, K::TXor),
                _ => return None,
            },
            _ => return None,
        })
    }

    /// An augmented assignment's token.
    fn augmented_op(&mut self) -> Option<K> {
        Some(match self.peek() {
            T::StarStarEq => K::TStarStarEq,
            T::StarEq => K::TStarEq,
            T::SlashEq => K::TSlashEq,
            T::PercentEq => K::TPercentEq,
            T::PlusEq => K::TPlusEq,
            T::MinusEq => K::TMinusEq,
            T::DotEq => K::TDotEq,
            T::LtLtEq => K::TLtLtEq,
            T::GtGtEq => K::TGtGtEq,
            T::AmpEq => K::TAmpEq,
            T::CaretEq => K::TCaretEq,
            T::PipeEq => K::TPipeEq,
            T::QQEq => K::TQQEq,
            _ => return None,
        })
    }

    /// An expression in slot `s`.
    pub(super) fn expr(&mut self, s: Slot) -> R<N> {
        self.binary(s, 0)
    }

    /// An expression whose binary operators bind at `min` or tighter, in slot `s`.
    fn binary(&mut self, s: Slot, min: u8) -> R<N> {
        self.deeper()?;
        let r = self.binary_inner(s, min);
        self.depth -= 1;
        r
    }

    fn binary_inner(&mut self, s: Slot, min: u8) -> R<N> {
        let mut left = self.unary(s)?;
        loop {
            // An assignment at any precedence (the module's note).
            if (is_variable(left.kind) || left.kind == K::ListLiteral) && self.w.len() > left.at {
                if self.peek() == T::Eq {
                    let reference = self.kind_at(1) == T::Amp;
                    let kind = if reference { K::ReferenceAssignmentExpression } else { K::AssignmentExpression };
                    let n = self.wrap(left, kind, s, slot(f::LEFT, Ctx::Bare));
                    self.tk(kind, K::TEq)?;
                    if reference {
                        self.tk(kind, K::TAmp)?;
                    }
                    self.binary(slot(f::RIGHT, Ctx::Expr), P_ASSIGN)?;
                    self.close(n);
                    left = n;
                    continue;
                }
                if left.kind != K::ListLiteral {
                    if let Some(op) = self.augmented_op() {
                        let n = self.wrap(left, K::AugmentedAssignmentExpression, s, slot(f::LEFT, Ctx::Bare));
                        self.tk_field(K::AugmentedAssignmentExpression, op, f::OPERATOR)?;
                        self.binary(slot(f::RIGHT, Ctx::Expr), P_ASSIGN)?;
                        self.close(n);
                        left = n;
                        continue;
                    }
                }
            }
            if left.kind == K::ListLiteral {
                return self.refuse();
            }
            if self.peek() == T::Question && P_TERNARY >= min {
                let n = self.wrap(left, K::ConditionalExpression, s, slot(f::CONDITION, Ctx::Expr));
                self.tk(K::ConditionalExpression, K::TQuestion)?;
                if self.peek() != T::Colon {
                    self.expr(slot(f::BODY, Ctx::Expr))?;
                }
                self.tk(K::ConditionalExpression, K::TColon)?;
                self.binary(slot(f::ALTERNATIVE, Ctx::Expr), P_TERNARY + 1)?;
                self.close(n);
                left = n;
                continue;
            }
            if self.is_word(w::INSTANCEOF) && P_INSTANCEOF >= min {
                if !(is_primary(left.kind) || is_unary(left.kind)) {
                    return self.refuse();
                }
                let n = self.wrap(left, K::BinaryExpression, s, slot(f::LEFT, Ctx::Unary));
                self.tk_field(K::BinaryExpression, K::TInstanceof, f::OPERATOR)?;
                self.class_name_reference(slot(f::RIGHT, Ctx::Bare))?;
                self.close(n);
                left = n;
                continue;
            }
            let Some((p, right, op)) = self.binary_op() else { break };
            if p < min {
                break;
            }
            let n = self.wrap(left, K::BinaryExpression, s, slot(f::LEFT, Ctx::Expr));
            self.tk_field(K::BinaryExpression, op, f::OPERATOR)?;
            self.binary(slot(f::RIGHT, Ctx::Expr), if right { p } else { p + 1 })?;
            self.close(n);
            left = n;
        }
        Ok(left)
    }

    /// A prefix expression or a primary one, in slot `s`.
    fn unary(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Bang | T::Tilde | T::Minus | T::Plus => {
                let n = self.open_in(K::UnaryOpExpression, s)?;
                let op = punct(t.kind).expect("an operator");
                self.tk_field(K::UnaryOpExpression, op, f::OPERATOR)?;
                self.binary(slot(f::ARGUMENT, Ctx::Expr), P_NEG + 1)?;
                self.close(n);
                Ok(n)
            }
            T::At => {
                let n = self.open_in(K::ErrorSuppressionExpression, s)?;
                self.tk(K::ErrorSuppressionExpression, K::TAt)?;
                self.binary(EXPR, P_INC + 1)?;
                self.close(n);
                Ok(n)
            }
            T::LParen if self.cast_ahead() => {
                let n = self.open_in(K::CastExpression, s)?;
                self.tk(K::CastExpression, K::TLParen)?;
                self.leaf(K::CastType, f::TYPE, C_NONE)?;
                self.tk(K::CastExpression, K::TRParen)?;
                self.cast_value()?;
                self.close(n);
                Ok(n)
            }
            T::PlusPlus | T::MinusMinus => {
                let n = self.open_in(K::UpdateExpression, s)?;
                let op = punct(t.kind).expect("an operator");
                self.tk_field(K::UpdateExpression, op, f::OPERATOR)?;
                let a = self.postfix_primary(slot(f::ARGUMENT, Ctx::Bare))?;
                if !is_variable(a.kind) {
                    return self.refuse();
                }
                self.close(n);
                Ok(n)
            }
            T::Name => match t.word {
                w::CLONE => {
                    let n = self.open_in(K::CloneExpression, s)?;
                    self.tk(K::CloneExpression, K::TClone)?;
                    self.postfix_primary(slot(f::NONE, Ctx::Primary))?;
                    self.close(n);
                    Ok(n)
                }
                w::PRINT => self.prefix_rule(s, K::PrintIntrinsic, K::TPrint),
                w::THROW => self.prefix_rule(s, K::ThrowExpression, K::TThrow),
                w::INCLUDE => self.prefix_rule(s, K::IncludeExpression, K::TInclude),
                w::INCLUDE_ONCE => self.prefix_rule(s, K::IncludeOnceExpression, K::TIncludeOnce),
                w::REQUIRE => self.prefix_rule(s, K::RequireExpression, K::TRequire),
                w::REQUIRE_ONCE => self.prefix_rule(s, K::RequireOnceExpression, K::TRequireOnce),
                w::YIELD => self.yield_expression(s),
                _ => self.postfix_primary(s),
            },
            _ => self.postfix_primary(s),
        }
    }

    /// `kind`: its keyword `tok`, then an expression (a rule of no precedence takes every operator).
    fn prefix_rule(&mut self, s: Slot, kind: K, tok: K) -> R<N> {
        let n = self.open_in(kind, s)?;
        self.tk(kind, tok)?;
        self.expr(EXPR)?;
        self.close(n);
        Ok(n)
    }

    /// A cast's value: a `_unary_expression`, an include or require, or an `@`.
    fn cast_value(&mut self) -> R<N> {
        let s = slot(f::VALUE, Ctx::Unary);
        let t = self.tok();
        match t.kind {
            T::At => {
                let n = self.open(K::ErrorSuppressionExpression, f::VALUE, C_NONE)?;
                self.tk(K::ErrorSuppressionExpression, K::TAt)?;
                self.binary(EXPR, P_INC + 1)?;
                self.close(n);
                Ok(n)
            }
            T::Name if matches!(t.word, w::INCLUDE | w::INCLUDE_ONCE | w::REQUIRE | w::REQUIRE_ONCE) => self.unary(slot(f::VALUE, Ctx::Bare)),
            T::Name if matches!(t.word, w::PRINT | w::THROW | w::YIELD) => self.refuse(),
            _ => {
                let n = self.unary(s)?;
                if matches!(self.peek(), T::Eq) || self.augmented_op().is_some() {
                    return self.refuse();
                }
                Ok(n)
            }
        }
    }

    /// Whether `(` here opens a cast: `(` a cast's type word `)`.
    fn cast_ahead(&mut self) -> bool {
        let t = self.peek_at(1);
        t.kind == T::Name && w::is_cast_type(t.word) && self.kind_at(2) == T::RParen
    }

    fn yield_expression(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::YieldExpression, s)?;
        let t = self.tok();
        let from = self.peek_at(1);
        if from.kind == T::Name && from.word == w::FROM && from.start == t.end + 1 && self.src[t.end as usize] == b' ' {
            // `yield from`: one token.
            self.bump()?;
            self.bump()?;
            self.expr(EXPR)?;
            self.close(n);
            return Ok(n);
        }
        self.tk(K::YieldExpression, K::TYield)?;
        if !matches!(self.peek(), T::Semi | T::RParen | T::RBracket | T::Comma | T::CloseTag | T::RBrace) {
            self.array_element()?;
        }
        self.close(n);
        Ok(n)
    }

    /// A primary expression and its postfix chain, in slot `s`.
    pub(super) fn postfix_primary(&mut self, s: Slot) -> R<N> {
        let left = self.primary(s)?;
        self.postfix(left, s)
    }

    /// The postfix chain after `left` (in slot `s`).
    fn postfix(&mut self, mut left: N, s: Slot) -> R<N> {
        loop {
            match self.peek() {
                T::Arrow | T::QArrow => {
                    let nullsafe = self.peek() == T::QArrow;
                    if !self.dereferencable(left.kind) {
                        return self.refuse();
                    }
                    let (access, call) = if nullsafe {
                        (K::NullsafeMemberAccessExpression, K::NullsafeMemberCallExpression)
                    } else {
                        (K::MemberAccessExpression, K::MemberCallExpression)
                    };
                    let n = self.wrap(left, access, s, slot(f::OBJECT, Ctx::Bare));
                    let arrow = if nullsafe { K::TQArrow } else { K::TArrow };
                    self.tk(access, arrow)?;
                    self.member_name(access)?;
                    if self.peek() == T::LParen {
                        let (kind, _, _, _) = unkey(self.w.key(n.at));
                        let _ = kind;
                        self.rekey_kind(n.at, call, s);
                        self.arguments(slot(f::ARGUMENTS, Ctx::Bare))?;
                        self.close(n);
                        left = N { at: n.at, kind: call };
                    } else {
                        self.close(n);
                        left = N { at: n.at, kind: access };
                    }
                }
                T::ColonColon => {
                    if !self.scope_qualifier(left.kind) {
                        return self.refuse();
                    }
                    match self.kind_at(1) {
                        T::Dollar => {
                            // `A::$b(…)`: a scoped call whose name is a variable.
                            // (Its `::` is stored there: `scoped_call_expression` implies none.)
                            let call = self.kind_at(2) == T::Name && self.kind_at(3) == T::LParen;
                            let kind = if call { K::ScopedCallExpression } else { K::ScopedPropertyAccessExpression };
                            let n = self.wrap(left, kind, s, slot(f::SCOPE, Ctx::Bare));
                            self.tk(kind, K::TColonColon)?;
                            self.simple_variable(slot(f::NAME, Ctx::Bare))?;
                            if call {
                                self.arguments(slot(f::ARGUMENTS, Ctx::Bare))?;
                            } else if self.peek() == T::LParen {
                                // `A::$$b(…)`.
                                return self.refuse();
                            }
                            self.close(n);
                            left = N { at: n.at, kind };
                        }
                        T::Name if self.kind_at(2) == T::LParen => {
                            let n = self.wrap(left, K::ScopedCallExpression, s, slot(f::SCOPE, Ctx::Bare));
                            self.tk(K::ScopedCallExpression, K::TColonColon)?;
                            self.name_any(slot(f::NAME, Ctx::Bare))?;
                            self.arguments(slot(f::ARGUMENTS, Ctx::Bare))?;
                            self.close(n);
                            left = N { at: n.at, kind: K::ScopedCallExpression };
                        }
                        T::Name => {
                            let n = self.wrap(left, K::ClassConstantAccessExpression, s, BARE);
                            self.tk(K::ClassConstantAccessExpression, K::TColonColon)?;
                            self.name_any(BARE)?;
                            self.close(n);
                            left = N { at: n.at, kind: K::ClassConstantAccessExpression };
                        }
                        T::LBrace => {
                            // `A::{expr}` and `A::{expr}()`.
                            let close = self.matching(1).ok_or_else(|| Refusal::Parse(self.tok().start))?;
                            let call = self.kind_at(close + 1) == T::LParen;
                            let kind = if call { K::ScopedCallExpression } else { K::ClassConstantAccessExpression };
                            let inner = if call { slot(f::SCOPE, Ctx::Bare) } else { BARE };
                            let n = self.wrap(left, kind, s, inner);
                            self.tk(kind, K::TColonColon)?;
                            self.tk(kind, K::TLBrace)?;
                            if call {
                                self.expr(slot(f::NAME, Ctx::Expr))?;
                            } else {
                                // `alias($.expression, $.name)`.
                                return self.refuse();
                            }
                            self.tk(kind, K::TRBrace)?;
                            self.arguments(slot(f::ARGUMENTS, Ctx::Bare))?;
                            self.close(n);
                            left = N { at: n.at, kind };
                        }
                        _ => return self.refuse(),
                    }
                }
                T::LBracket => {
                    if !self.dereferencable(left.kind) {
                        return self.refuse();
                    }
                    let n = self.wrap(left, K::SubscriptExpression, s, BARE);
                    self.tk(K::SubscriptExpression, K::TLBracket)?;
                    if self.peek() != T::RBracket {
                        self.expr(EXPR)?;
                    }
                    self.tk(K::SubscriptExpression, K::TRBracket)?;
                    self.close(n);
                    left = N { at: n.at, kind: K::SubscriptExpression };
                }
                T::LParen => {
                    if !self.callable(left.kind) {
                        return Ok(left);
                    }
                    let n = self.wrap(left, K::FunctionCallExpression, s, slot(f::FUNCTION, Ctx::Bare));
                    self.arguments(slot(f::ARGUMENTS, Ctx::Bare))?;
                    self.close(n);
                    left = N { at: n.at, kind: K::FunctionCallExpression };
                }
                T::PlusPlus | T::MinusMinus if is_variable(left.kind) => {
                    let n = self.wrap(left, K::UpdateExpression, s, slot(f::ARGUMENT, Ctx::Bare));
                    let op = punct(self.peek()).expect("an operator");
                    self.tk_field(K::UpdateExpression, op, f::OPERATOR)?;
                    self.close(n);
                    return Ok(n);
                }
                _ => return Ok(left),
            }
        }
    }

    /// The node at `at` given kind `kind` (its class in slot `s`).
    fn rekey_kind(&mut self, at: usize, kind: K, s: Slot) {
        self.w.set_key(at, key(kind, s.field, class(kind, s.ctx), false));
    }

    /// Whether `k` can stand before `->` or `[` (`_dereferencable_expression`).
    fn dereferencable(&self, k: K) -> bool {
        use K::*;
        is_variable(k)
            || matches!(
                k,
                ObjectCreationExpression | ClassConstantAccessExpression | ParenthesizedExpression | ArrayCreationExpression | String | EncapsedString | Heredoc | Nowdoc | Name | QualifiedName | RelativeName
            )
    }

    /// Whether `k` can stand before `::` (`_scope_resolution_qualifier`).
    fn scope_qualifier(&self, k: K) -> bool {
        k == K::RelativeScope || self.dereferencable(k)
    }

    /// Whether `k` can be called (`_name` or `_callable_expression`).
    fn callable(&self, k: K) -> bool {
        use K::*;
        matches!(
            k,
            Name | QualifiedName
                | RelativeName
                | VariableName
                | DynamicVariableName
                | SubscriptExpression
                | MemberCallExpression
                | NullsafeMemberCallExpression
                | FunctionCallExpression
                | ScopedCallExpression
                | ParenthesizedExpression
                | ArrayCreationExpression
                | String
                | EncapsedString
                | Heredoc
                | Nowdoc
                | ObjectCreationExpression
        )
    }

    /// `_member_name` under `parent`: a name, a variable, or `{expression}`.
    fn member_name(&mut self, parent: K) -> R<()> {
        match self.peek() {
            T::Name => {
                self.name_any(slot(f::NAME, Ctx::Bare))?;
            }
            T::Dollar => {
                self.simple_variable(slot(f::NAME, Ctx::Bare))?;
            }
            T::LBrace => {
                // Its `{` is stored before a variable (`->$v` is the gap's shortest word there).
                self.braced(slot(f::NAME, Ctx::Expr), None, |k| matches!(k, K::VariableName | K::DynamicVariableName))?;
                self.tk(parent, K::TRBrace)?;
            }
            _ => return self.refuse(),
        }
        Ok(())
    }

    /// `{`'s expression, its `{` stored where `stored` holds for the expression's kind (where that
    /// kind could stand without the braces, the gap's shortest word is empty): read again without
    /// it elsewhere.
    /// `before`: the `}` of an interpolation right before (`}{`), stored with the `{` (its gap's
    /// shortest word is that `}` alone where the expression could stand bare).
    pub(super) fn braced(&mut self, s: Slot, before: Option<u32>, stored: impl Fn(K) -> bool) -> R<N> {
        let (mark, at, last_end, e) = (self.w.len(), self.i, self.last_end, self.e);
        let brace = self.tok().start;
        if let Some(b) = before {
            self.mark(K::TRBrace, f::NONE, C_NONE, b as usize, b as usize + 1);
        }
        self.tok_leaf(K::TLBrace)?;
        let x = self.expr(s)?;
        if stored(x.kind) {
            return Ok(x);
        }
        // The tokens after the `{` may have been lexed again (a string's pieces): lexed again here.
        self.w.truncate(mark);
        (self.i, self.last_end, self.e) = (at, last_end, e);
        self.relex_at(brace as usize);
        self.bump()?;
        self.expr(s)
    }

    /// `_simple_variable`: `variable_name` or `dynamic_variable_name`.
    pub(super) fn simple_variable(&mut self, s: Slot) -> R<N> {
        if self.peek() != T::Dollar {
            return self.refuse();
        }
        match self.kind_at(1) {
            T::Name => self.variable_name(s),
            T::Dollar => {
                let n = self.open_in(K::DynamicVariableName, s)?;
                self.tk(K::DynamicVariableName, K::TDollar)?;
                self.simple_variable(BARE)?;
                self.close(n);
                Ok(n)
            }
            T::LBrace => {
                // `{` is stored before a variable (`$$v` is the gap's shortest word there).
                let n = self.open_in(K::DynamicVariableName, s)?;
                self.tk(K::DynamicVariableName, K::TDollar)?;
                self.braced(EXPR, None, |k| matches!(k, K::VariableName | K::DynamicVariableName))?;
                self.tk(K::DynamicVariableName, K::TRBrace)?;
                self.close(n);
                Ok(n)
            }
            _ => self.refuse(),
        }
    }

    /// `variable_name`: `$` and a name.
    pub(super) fn variable_name(&mut self, s: Slot) -> R<N> {
        if self.peek() != T::Dollar || self.kind_at(1) != T::Name {
            return self.refuse();
        }
        let n = self.open_in(K::VariableName, s)?;
        self.tk(K::VariableName, K::TDollar)?;
        self.name_any(BARE)?;
        self.close(n);
        Ok(n)
    }

    /// `by_ref`: `&` and a variable.
    pub(super) fn by_ref(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ByRef, s)?;
        self.tk(K::ByRef, K::TAmp)?;
        self.variable(BARE)?;
        self.close(n);
        Ok(n)
    }

    /// A `_variable` in slot `s`.
    pub(super) fn variable(&mut self, s: Slot) -> R<N> {
        let n = self.postfix_primary(s)?;
        if !is_variable(n.kind) {
            return self.refuse();
        }
        Ok(n)
    }

    /// `_class_name_reference` (`new`'s class, `instanceof`'s right): a name, a new variable
    /// (`$a`, `$a->b`, `$a[0]`, `A::$b`), or a parenthesized expression.
    fn class_name_reference(&mut self, s: Slot) -> R<N> {
        match self.peek() {
            T::LParen => self.parenthesized(s),
            T::Dollar => {
                let mut left = self.simple_variable(s)?;
                loop {
                    match self.peek() {
                        T::Arrow | T::QArrow => {
                            let nullsafe = self.peek() == T::QArrow;
                            let kind = if nullsafe { K::NullsafeMemberAccessExpression } else { K::MemberAccessExpression };
                            let n = self.wrap(left, kind, s, slot(f::OBJECT, Ctx::Bare));
                            self.tk(kind, if nullsafe { K::TQArrow } else { K::TArrow })?;
                            self.member_name(kind)?;
                            self.close(n);
                            left = N { at: n.at, kind };
                        }
                        T::LBracket => {
                            let n = self.wrap(left, K::SubscriptExpression, s, BARE);
                            self.tk(K::SubscriptExpression, K::TLBracket)?;
                            if self.peek() != T::RBracket {
                                self.expr(EXPR)?;
                            }
                            self.tk(K::SubscriptExpression, K::TRBracket)?;
                            self.close(n);
                            left = N { at: n.at, kind: K::SubscriptExpression };
                        }
                        T::ColonColon if self.kind_at(1) == T::Dollar => {
                            let n = self.wrap(left, K::ScopedPropertyAccessExpression, s, slot(f::SCOPE, Ctx::Bare));
                            self.tk(K::ScopedPropertyAccessExpression, K::TColonColon)?;
                            self.simple_variable(slot(f::NAME, Ctx::Bare))?;
                            self.close(n);
                            left = N { at: n.at, kind: K::ScopedPropertyAccessExpression };
                        }
                        _ => return Ok(left),
                    }
                }
            }
            _ => {
                let n = self.class_ref_name(s)?;
                if self.peek() == T::ColonColon && self.kind_at(1) == T::Dollar {
                    let m = self.wrap(n, K::ScopedPropertyAccessExpression, s, slot(f::SCOPE, Ctx::Bare));
                    self.tk(K::ScopedPropertyAccessExpression, K::TColonColon)?;
                    self.simple_variable(slot(f::NAME, Ctx::Bare))?;
                    self.close(m);
                    return Ok(N { at: m.at, kind: K::ScopedPropertyAccessExpression });
                }
                Ok(n)
            }
        }
    }

    /// A primary expression in slot `s` (no postfix).
    fn primary(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Dollar => self.simple_variable(s),
            T::Int => self.leaf_in(K::Integer, s),
            T::Float => self.leaf_in(K::Float, s),
            T::Str => self.single_string(s),
            T::DStr => self.double_string(s),
            T::Heredoc => self.heredoc(s),
            T::Shell => self.shell_command(s),
            T::LParen => self.parenthesized(s),
            T::LBracket => {
                if self.list_ahead() {
                    self.list_literal(s)
                } else {
                    self.array_creation(s)
                }
            }
            T::Backslash => self.qualified_name(s),
            T::HashLBracket => {
                // Attributes before a closure.
                let mut k = 0;
                while self.kind_at(k) == T::HashLBracket {
                    k = self.matching(k).ok_or_else(|| Refusal::Parse(self.tok().start))? + 1;
                }
                if self.is_word_at(k, w::STATIC) {
                    k += 1;
                }
                match self.word_at(k) {
                    w::FUNCTION => self.anonymous_function(s),
                    w::FN => self.arrow_function(s),
                    _ => self.refuse(),
                }
            }
            T::Name => self.name_primary(s, t),
            _ => self.refuse(),
        }
    }

    /// A primary that starts with a word.
    fn name_primary(&mut self, s: Slot, t: Tok) -> R<N> {
        let next = self.kind_at(1);
        match t.word {
            w::TRUE | w::FALSE if next != T::Backslash => self.leaf_in(K::Boolean, s),
            w::NULL if next != T::Backslash => self.leaf_in(K::Null, s),
            w::NEW => self.object_creation(s),
            w::MATCH if next == T::LParen => self.match_expression(s),
            w::FUNCTION => self.anonymous_function(s),
            w::FN => self.arrow_function(s),
            w::STATIC if matches!(self.word_at(1), w::FUNCTION | w::FN) => {
                if self.is_word_at(1, w::FUNCTION) {
                    self.anonymous_function(s)
                } else {
                    self.arrow_function(s)
                }
            }
            w::SELF | w::PARENT | w::STATIC if next == T::ColonColon => {
                let n = self.open_in(K::RelativeScope, s)?;
                let k = keyword(t.word).expect("a scope word");
                self.tk(K::RelativeScope, k)?;
                self.close(n);
                Ok(n)
            }
            w::ARRAY if next == T::LParen => self.array_creation(s),
            w::LIST if next == T::LParen => self.list_literal(s),
            w::NAMESPACE if next == T::Backslash => self.relative_name(s),
            _ if next == T::Backslash => self.qualified_name(s),
            _ => {
                // A name: its reserved set is `classes` where a call or `::` follows (`_name`),
                // `global` otherwise (`primary_expression`'s `name`).
                if w::is_reserved(t.word) {
                    return self.refuse();
                }
                if w::is_class_reserved(t.word) && matches!(next, T::LParen | T::ColonColon) {
                    return self.refuse();
                }
                self.leaf_in(K::Name, s)
            }
        }
    }

    /// Whether `[` here starts a list (destructuring): its `]` is followed by `=` (not `==`, `=>`).
    fn list_ahead(&mut self) -> bool {
        self.matching(0).is_some_and(|e| self.kind_at(e + 1) == T::Eq)
    }

    /// `list_literal`: `list(` … `)` or `[` … `]`, its items optional.
    pub(super) fn list_literal(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ListLiteral, s)?;
        let list = self.is_word(w::LIST);
        let close = if list {
            self.tk(K::ListLiteral, K::TList)?;
            T::RParen
        } else {
            T::RBracket
        };
        let opener = punct(self.peek()).ok_or_else(|| Refusal::Parse(self.tok().start))?;
        let closer = punct(close).expect("a closer");
        // An empty item makes its gap no implied word: its tokens are stored. So does `list`
        // before `(` (`[` is the shorter opening).
        let mut keep = matches!(self.kind_at(1), T::Comma) || self.kind_at(1) == close;
        if list {
            self.leaf(opener, f::NONE, C_NONE)?;
        } else {
            self.gap_tk(K::ListLiteral, opener, keep)?;
        }
        loop {
            if self.peek() == close {
                self.gap_tk(K::ListLiteral, closer, keep)?;
                break;
            }
            if self.peek() == T::Comma {
                keep = true;
                self.tok_leaf(K::TComma)?;
                continue;
            }
            let mut prev = self.list_item(close)?;
            loop {
                if self.peek() != T::Comma || matches!(self.kind_at(1), T::Comma) || self.kind_at(1) == close {
                    break;
                }
                // A `,` between two items: stored where `=>` is as short a word for the gap (after
                // a variable, which may be a key, before an item that may be a value), read
                // again without it where not.
                if !is_variable(prev) {
                    self.tk(K::ListLiteral, K::TComma)?;
                    prev = self.list_item(close)?;
                    continue;
                }
                let (mark, at, last_end, e) = (self.w.len(), self.i, self.last_end, self.e);
                let comma = self.tok().start;
                self.tok_leaf(K::TComma)?;
                let item = self.list_item(close)?;
                let (first, _, _, _) = unkey(self.w.key(mark + 1));
                if is_variable(kind_of(first)) || matches!(kind_of(first), K::ListLiteral | K::ByRef) {
                    prev = item;
                    continue;
                }
                self.w.truncate(mark);
                (self.i, self.last_end, self.e) = (at, last_end, e);
                self.relex_at(comma as usize);
                self.tk(K::ListLiteral, K::TComma)?;
                prev = self.list_item(close)?;
            }
            match self.peek() {
                T::Comma => {
                    keep = matches!(self.kind_at(1), T::Comma) || self.kind_at(1) == close;
                    self.tok_leaf(K::TComma)?;
                }
                k if k == close => {
                    self.gap_tk(K::ListLiteral, closer, false)?;
                    break;
                }
                _ => return self.refuse(),
            }
        }
        self.close(n);
        Ok(n)
    }

    /// A list's item: a nested list, a variable, `&$v`, or `key => ` one of them.
    /// The item's last child's kind (its value).
    fn list_item(&mut self, close: T) -> R<K> {
        // `key =>` first.
        let mut k = 0;
        let mut depth = 0usize;
        let keyed = loop {
            match self.kind_at(k) {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace if depth > 0 => depth -= 1,
                T::FatArrow if depth == 0 => break true,
                T::Comma if depth == 0 => break false,
                t if t == close && depth == 0 => break false,
                T::Eof => break false,
                _ => {}
            }
            k += 1;
        };
        if keyed {
            self.expr(EXPR)?;
            self.tk(K::ListLiteral, K::TFatArrow)?;
        }
        let n = match self.peek() {
            T::Amp => self.by_ref(BARE)?,
            T::LBracket => self.list_literal(BARE)?,
            _ if self.is_word(w::LIST) && self.kind_at(1) == T::LParen => self.list_literal(BARE)?,
            _ => self.variable(BARE)?,
        };
        Ok(n.kind)
    }

    /// `array_creation_expression`: `array(` … `)` or `[` … `]`.
    fn array_creation(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ArrayCreationExpression, s)?;
        if self.is_word(w::ARRAY) {
            // `array (` is not the kind's shortest opening (`[` is): stored.
            self.tk(K::ArrayCreationExpression, K::TArray)?;
            if self.kind_at(1) == T::RParen {
                self.tok_leaf(K::TLParen)?;
                self.tok_leaf(K::TRParen)?;
                self.close(n);
                return Ok(n);
            }
            self.tok_leaf(K::TLParen)?;
            self.elements(T::RParen)?;
        } else {
            if self.kind_at(1) == T::RBracket {
                self.tk(K::ArrayCreationExpression, K::TLBracket)?;
                self.tk(K::ArrayCreationExpression, K::TRBracket)?;
                self.close(n);
                return Ok(n);
            }
            self.tk(K::ArrayCreationExpression, K::TLBracket)?;
            self.elements(T::RBracket)?;
        }
        self.close(n);
        Ok(n)
    }

    /// An array's elements to `close` (its opener read).
    fn elements(&mut self, close: T) -> R<()> {
        let closer = punct(close).expect("a closer");
        loop {
            self.array_element()?;
            match self.peek() {
                T::Comma if self.kind_at(1) == close => {
                    self.tok_leaf(K::TComma)?;
                    self.leaf(closer, f::NONE, C_NONE)?;
                    return Ok(());
                }
                T::Comma => self.tk(K::ArrayCreationExpression, K::TComma)?,
                t if t == close => return self.tk(K::ArrayCreationExpression, closer),
                _ => return self.refuse(),
            }
        }
    }

    /// `array_element_initializer`: `&$v`, an expression, `k => v` (`v` maybe `&$v`), `...$a`.
    fn array_element(&mut self) -> R<()> {
        let n = self.open(K::ArrayElementInitializer, f::NONE, C_NONE)?;
        match self.peek() {
            T::DotDotDot => {
                let v = self.open(K::VariadicUnpacking, f::NONE, C_NONE)?;
                self.tk(K::VariadicUnpacking, K::TDotDotDot)?;
                self.expr(EXPR)?;
                self.close(v);
            }
            T::Amp => {
                self.by_ref(BARE)?;
            }
            _ => {
                self.expr(EXPR)?;
                if self.peek() == T::FatArrow {
                    self.tk(K::ArrayElementInitializer, K::TFatArrow)?;
                    if self.peek() == T::Amp {
                        self.by_ref(BARE)?;
                    } else {
                        self.expr(EXPR)?;
                    }
                }
            }
        }
        self.close(n);
        Ok(())
    }

    /// `arguments`: `(` arguments `)`, or `(...)`.
    pub(super) fn arguments(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::Arguments, s)?;
        if self.peek() != T::LParen {
            return self.refuse();
        }
        if self.kind_at(1) == T::DotDotDot && self.kind_at(2) == T::RParen {
            self.tk(K::Arguments, K::TLParen)?;
            self.token_node(K::VariadicPlaceholder, f::NONE, K::TDotDotDot)?;
            self.tk(K::Arguments, K::TRParen)?;
            self.close(n);
            return Ok(n);
        }
        self.list(K::Arguments, T::RParen, true, true, |p| p.argument())?;
        self.close(n);
        Ok(n)
    }

    fn argument(&mut self) -> R<()> {
        let n = self.open(K::Argument, f::NONE, C_NONE)?;
        if self.peek() == T::Name && self.kind_at(1) == T::Colon {
            self.leaf(K::Name, f::NAME, C_NONE)?;
            self.tk(K::Argument, K::TColon)?;
        }
        if self.peek() == T::Amp {
            self.token_node(K::ReferenceModifier, f::REFERENCE_MODIFIER, K::TAmp)?;
        }
        if self.peek() == T::DotDotDot {
            let v = self.open(K::VariadicUnpacking, f::NONE, C_NONE)?;
            self.tk(K::VariadicUnpacking, K::TDotDotDot)?;
            self.expr(EXPR)?;
            self.close(v);
        } else {
            self.expr(EXPR)?;
        }
        self.close(n);
        Ok(())
    }

    /// `new` …: an object creation, or an anonymous class.
    fn object_creation(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ObjectCreationExpression, s)?;
        self.tk(K::ObjectCreationExpression, K::TNew)?;
        let mut k = 0;
        while self.kind_at(k) == T::HashLBracket {
            k = self.matching(k).ok_or_else(|| Refusal::Parse(self.tok().start))? + 1;
        }
        while matches!(self.word_at(k), w::ABSTRACT | w::FINAL | w::READONLY) {
            k += 1;
        }
        if self.is_word_at(k, w::CLASS) {
            let c = self.open(K::AnonymousClass, f::NONE, C_NONE)?;
            if self.peek() == T::HashLBracket {
                self.attribute_list(slot(f::ATTRIBUTES, Ctx::Bare))?;
            }
            while matches!(self.word_at(0), w::ABSTRACT | w::FINAL | w::READONLY) {
                let (kind, tok) = match self.word_at(0) {
                    w::ABSTRACT => (K::AbstractModifier, K::TAbstract),
                    w::FINAL => (K::FinalModifier, K::TFinal),
                    _ => (K::ReadonlyModifier, K::TReadonly),
                };
                let m = self.open(kind, f::NONE, C_NONE)?;
                self.tk(kind, tok)?;
                self.close(m);
            }
            self.kw(K::AnonymousClass, w::CLASS)?;
            if self.peek() == T::LParen {
                self.arguments(BARE)?;
            }
            self.class_tail()?;
            self.declaration_list(K::DeclarationList)?;
            self.close(c);
            self.close(n);
            return Ok(n);
        }
        self.class_name_reference(BARE)?;
        if self.peek() == T::LParen {
            self.arguments(BARE)?;
        }
        self.close(n);
        Ok(n)
    }

    fn match_expression(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::MatchExpression, s)?;
        self.kw(K::MatchExpression, w::MATCH)?;
        self.parenthesized(slot(f::CONDITION, Ctx::Bare))?;
        let b = self.open(K::MatchBlock, f::BODY, C_NONE)?;
        self.list(K::MatchBlock, T::RBrace, true, true, |p| {
            if p.is_word(w::DEFAULT) && p.kind_at(1) == T::FatArrow {
                let a = p.open(K::MatchDefaultExpression, f::NONE, C_NONE)?;
                p.kw(K::MatchDefaultExpression, w::DEFAULT)?;
                p.tk(K::MatchDefaultExpression, K::TFatArrow)?;
                p.expr(slot(f::RETURN_EXPRESSION, Ctx::Expr))?;
                p.close(a);
                return Ok(());
            }
            let a = p.open(K::MatchConditionalExpression, f::NONE, C_NONE)?;
            let l = p.open(K::MatchConditionList, f::CONDITIONAL_EXPRESSIONS, C_NONE)?;
            loop {
                p.expr(EXPR)?;
                if p.peek() != T::Comma {
                    break;
                }
                if p.kind_at(1) == T::FatArrow {
                    p.leaf(K::TComma, f::NONE, C_NONE)?;
                    break;
                }
                p.tk(K::MatchConditionList, K::TComma)?;
            }
            p.close(l);
            p.tk(K::MatchConditionalExpression, K::TFatArrow)?;
            p.expr(slot(f::RETURN_EXPRESSION, Ctx::Expr))?;
            p.close(a);
            Ok(())
        })?;
        self.close(b);
        self.close(n);
        Ok(n)
    }

    fn anonymous_function(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::AnonymousFunction, s)?;
        if self.peek() == T::HashLBracket {
            self.attribute_list(slot(f::ATTRIBUTES, Ctx::Bare))?;
        }
        if self.is_word(w::STATIC) {
            let m = self.open(K::StaticModifier, f::STATIC_MODIFIER, C_NONE)?;
            self.tk(K::StaticModifier, K::TStatic)?;
            self.close(m);
        }
        self.kw(K::AnonymousFunction, w::FUNCTION)?;
        if self.peek() == T::Amp {
            self.token_node(K::ReferenceModifier, f::REFERENCE_MODIFIER, K::TAmp)?;
        }
        self.formal_parameters()?;
        if self.is_word(w::USE) {
            let u = self.open(K::AnonymousFunctionUseClause, f::NONE, C_NONE)?;
            self.kw(K::AnonymousFunctionUseClause, w::USE)?;
            self.list(K::AnonymousFunctionUseClause, T::RParen, false, true, |p| {
                if p.peek() == T::Amp {
                    p.by_ref(BARE)?;
                } else {
                    p.variable_name(BARE)?;
                }
                Ok(())
            })?;
            self.close(u);
        }
        self.return_type_opt(K::AnonymousFunction)?;
        self.compound(slot(f::BODY, Ctx::Bare))?;
        self.close(n);
        Ok(n)
    }

    fn arrow_function(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ArrowFunction, s)?;
        if self.peek() == T::HashLBracket {
            self.attribute_list(slot(f::ATTRIBUTES, Ctx::Bare))?;
        }
        if self.is_word(w::STATIC) {
            let m = self.open(K::StaticModifier, f::STATIC_MODIFIER, C_NONE)?;
            self.tk(K::StaticModifier, K::TStatic)?;
            self.close(m);
        }
        self.kw(K::ArrowFunction, w::FN)?;
        if self.peek() == T::Amp {
            self.token_node(K::ReferenceModifier, f::REFERENCE_MODIFIER, K::TAmp)?;
        }
        self.formal_parameters()?;
        self.return_type_opt(K::ArrowFunction)?;
        self.tk(K::ArrowFunction, K::TFatArrow)?;
        self.expr(slot(f::BODY, Ctx::Expr))?;
        self.close(n);
        Ok(n)
    }

    /// A `literal` in slot `s` (a `declare` directive's value).
    pub(super) fn literal(&mut self, s: Slot) -> R<N> {
        match self.peek() {
            T::Int => self.leaf_in(K::Integer, s),
            T::Float => self.leaf_in(K::Float, s),
            T::Str => self.single_string(s),
            T::DStr => self.double_string(s),
            T::Heredoc => self.heredoc(s),
            T::Name if matches!(self.word_at(0), w::TRUE | w::FALSE) => self.leaf_in(K::Boolean, s),
            T::Name if self.is_word(w::NULL) => self.leaf_in(K::Null, s),
            _ => self.refuse(),
        }
    }
}
