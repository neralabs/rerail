//! Expressions, by the precedences tree-sitter-swift's parse table resolves its conflicts by.
//!
//! Each operator after an operand has a precedence and an associativity (the grammar's `prec`
//! on its rule). An operand is read in a context (`Cx`): the precedence of the rule that would be
//! reduced around it. An operator is read into the operand (a shift) where its precedence is above
//! the context's, or equal to it and the context's rule associates to the right; else the operand
//! ends (a reduce). So:
//!
//! - a call (`(…)`, `[…]`, a trailing closure; `prec(-2)`) binds looser than every binary operator
//!   but assignment: `"a" + f(x)` is a call of `"a" + f`;
//! - the right operand of a comparison, an equality, `&&`, `||`, `??`, a range, a bitwise or custom
//!   operator, and a ternary's last operand, is `_expr_hack_at_ternary_binary_suffix`
//!   (`prec.left(-2)`): it takes every binary operator after it (`a == b && c` is `a == (b && c)`),
//!   and a call with arguments (the hack's own call, precedence 0), not a trailing closure;
//! - a prefix operator (`prec.left(7)`) takes the operators tighter than 7 after its operand
//!   (`-a + b` is `-(a + b)`), not a call (`!f(x)` is a call of `!f`).

use super::P;
use crate::kinds::{K, f};
use crate::lex::{R, T, w};

/// An operand's context: the precedence and associativity of the rule reduced around it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Cx {
    prec: i8,
    right: bool,
    /// The right operand of a binary operator (`_expr_hack_at_ternary_binary_suffix`): a call with
    /// arguments is read into it.
    hack: bool,
    /// The operand of `try` (1) or `await` (2): it ends at a call (and for `try`, a binary
    /// operator's or a ternary's node), the grammar's `prec.left(0)` choices winning the
    /// reduction over `_expression`'s `prec(-1)`.
    stop: u8,
    /// A trailing closure after arguments is a call of its own (a property's value, `try`'s
    /// operand): `f(x) { … }` is `call(call(f, (x)), { … })`.
    split: bool,
}

pub(super) const VALUE: Cx = Cx { prec: -100, right: false, hack: false, stop: 0, split: true };
pub(super) const TOP: Cx = Cx { prec: -100, right: false, hack: false, stop: 0, split: false };
const HACK: Cx = Cx { prec: -2, right: false, hack: true, stop: 0, split: false };
const PREFIX: Cx = Cx { prec: 7, right: false, hack: false, stop: 0, split: false };
const TRY: Cx = Cx { prec: -2, right: true, hack: false, stop: 1, split: false };
const AWAIT: Cx = Cx { prec: -2, right: true, hack: false, stop: 2, split: false };

impl Cx {
    #[inline]
    fn shifts(self, p: i8) -> bool {
        p > self.prec || (p == self.prec && self.right)
    }
}

/// What an operator after an operand is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    /// A binary operator: its node kind, precedence, left and right fields, the operator's
    /// field, whether its right operand is the hack, whether it associates to the right.
    Bin(K, i8, u8, u8, u8, bool, bool),
}

impl<'a> P<'a> {
    /// An expression (`_expression`), at the top context.
    pub(super) fn expression(&mut self) -> R<()> {
        let calls = std::mem::take(&mut self.hack_calls);
        let r = self.expr(TOP);
        self.hack_calls = calls;
        r
    }

    /// An expression, its nodes given `field`.
    pub(super) fn expression_f(&mut self, field: u8) -> R<()> {
        let calls = std::mem::take(&mut self.hack_calls);
        let r = self.expr_f(TOP, field);
        self.hack_calls = calls;
        r
    }

    pub(super) fn expr_f(&mut self, cx: Cx, field: u8) -> R<()> {
        let m = self.mark();
        self.expr(cx)?;
        self.t.field_since(m, field);
        self.t.unmark(m);
        Ok(())
    }

    pub(super) fn expr(&mut self, cx: Cx) -> R<()> {
        self.deeper()?;
        let m = self.mark();
        // `try await …`: `try`'s operand ends with the `await`'s (its `prec.left(0)` choice).
        let awaits = cx.stop == 1 && self.is_word(w::AWAIT);
        self.unary(cx)?;
        if !awaits {
            self.operators(m, cx)?;
        }
        self.t.unmark(m);
        self.depth -= 1;
        Ok(())
    }

    /// Whether an expression starts at the next token.
    pub(super) fn expression_starts(&mut self) -> bool {
        let t = self.tok();
        match t.kind {
            T::Name => {
                t.tick
                    || !matches!(
                        t.word,
                        w::CASE | w::DEFAULT | w::ELSE | w::IN | w::WHERE | w::CATCH | w::LET | w::VAR | w::FUNC | w::CLASS | w::STRUCT | w::ENUM
                            | w::PROTOCOL | w::EXTENSION | w::IMPORT | w::RETURN | w::BREAK | w::CONTINUE | w::FOR | w::WHILE | w::GUARD
                            | w::REPEAT | w::DO | w::THROW | w::TYPEALIAS
                    )
            }
            T::Int | T::Hex | T::Oct | T::Bin | T::Real | T::Str | T::LParen | T::LBracket | T::LBrace | T::Backslash => true,
            T::Hash => !matches!(self.word_at(1), w::IF | w::ELSEIF | w::ELSE | w::ENDIF),
            T::Minus | T::Plus | T::Bang | T::Tilde | T::Amp | T::Dot | T::Custom | T::PlusPlus | T::MinusMinus | T::DotDotDot | T::DotDotLt => true,
            _ => false,
        }
    }

    /// At a `?` touching a call ending `try`'s or `await`'s operand: whether tree-sitter's GLR
    /// keeps the operand going on through the optional chain. It does where the chain (`.name`s)
    /// reaches a call or subscript and the expression goes on after that one (an operator, a
    /// `.`, `[`, `?`, `!`, `as` on its line, a `.` on the next); it ends at the first call where
    /// the statement ends there, a closure follows, or no call does (`?.b ?? c`). Probed.
    fn chain_goes_on(&mut self) -> bool {
        let mut j = 1;
        loop {
            match self.kind_at(j) {
                T::Dot if matches!(self.kind_at(j + 1), T::Name | T::Int) => j += 2,
                T::LParen | T::LBracket if self.adjacent(j - 1) => break,
                _ => return false,
            }
        }
        let mut d = 0;
        loop {
            match self.kind_at(j) {
                T::LParen | T::LBracket | T::LBrace => d += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    d -= 1;
                    if d == 0 {
                        break;
                    }
                }
                T::Eof => return false,
                _ => {}
            }
            j += 1;
        }
        let t = self.peek_at(j + 1);
        if t.nl {
            return t.kind == T::Dot;
        }
        !matches!(t.kind, T::Eof | T::RBrace | T::Semi | T::RParen | T::RBracket | T::Comma | T::LBrace | T::Colon)
    }

    /// After a directive ended at a function's name (`#if _runtime`): the call of it, and the
    /// operators after.
    pub(super) fn directive_call(&mut self, m: crate::tw::Mark) -> R<()> {
        let n = self.t.wrap(m, K::CallExpression, f::NONE);
        self.call_suffix(true, false)?;
        self.close(n);
        self.operators(m, TOP)
    }

    // ---- Operators after an operand.

    /// The operators after the operand read since `m`, in context `cx`.
    fn operators(&mut self, m: crate::tw::Mark, cx: Cx) -> R<()> {
        // `try`'s operand gone on through an optional chain past its first call.
        let mut chained = false;
        loop {
            if self.ends() {
                return Ok(());
            }
            let t = self.tok();
            match t.kind {
                T::Dot => {
                    if !cx.shifts(12) {
                        return Ok(());
                    }
                    if !matches!(self.kind_at(1), T::Name | T::Int) {
                        return self.refuse();
                    }
                    self.t.field_since(m, f::TARGET);
                    let n = self.t.wrap(m, K::NavigationExpression, f::NONE);
                    let s = self.open_field(K::NavigationSuffix, f::SUFFIX);
                    self.tk(K::TDot)?;
                    if self.peek() == T::Int {
                        self.leaf(K::IntegerLiteral, f::SUFFIX)?;
                    } else {
                        self.simple_identifier_f(f::SUFFIX)?;
                    }
                    self.close(s);
                    self.close(n);
                }
                T::Real if self.src[t.start as usize] == b'.' => return self.refuse(),
                T::Question if self.touching() => {
                    // `_expression _immediate_quest`: a `?` node after the operand, no node over them.
                    if !cx.shifts(-1) {
                        return Ok(());
                    }
                    self.wrap_tok(K::TQuestion, f::NONE, K::TQuestion)?;
                }
                T::Bang | T::PlusPlus | T::MinusMinus if self.touching() => {
                    if !cx.shifts(6) {
                        return Ok(());
                    }
                    // A `!` before `=`: lexed as one operator (`!=`), not here.
                    self.t.field_since(m, f::TARGET);
                    let n = self.t.wrap(m, K::PostfixExpression, f::NONE);
                    match t.kind {
                        T::Bang => {
                            self.leaf(K::Bang, f::OPERATION)?;
                        }
                        T::PlusPlus => {
                            self.tkf(K::TPlusPlus, f::OPERATION)?;
                        }
                        _ => {
                            self.tkf(K::TMinusMinus, f::OPERATION)?;
                        }
                    }
                    self.close(n);
                }
                T::LParen | T::LBracket => {
                    // In a hack operand the call is always read (past the GLR's pruning, the
                    // operand is at its operator's precedence, the call still its own).
                    let p = if cx.hack { i8::MAX } else { -2 };
                    if !cx.shifts(p) {
                        return Ok(());
                    }
                    if cx.hack {
                        self.hack_calls += 1;
                    }
                    let n = self.t.wrap(m, K::CallExpression, f::NONE);
                    self.call_suffix(true, cx.split)?;
                    self.close(n);
                    if cx.stop != 0 {
                        if self.peek() == T::Question && self.touching() && self.chain_goes_on() {
                            // An optional chain's `?` after the call: the operand goes on to the
                            // chain's next call where the expression goes on after it (probed).
                            chained = true;
                            continue;
                        }
                        if chained && cx.stop == 1 && self.peek() == T::Question && !self.touching() {
                            // Gone on so, `try`'s operand takes a ternary after the call too
                            // (a binary operator stays outside it; probed).
                            continue;
                        }
                        return Ok(());
                    }
                }
                T::LBrace => {
                    if self.no_lambda && cx.stop != 0 && cx.shifts(-2) {
                        // A condition's `{` after `try`/`await`'s operand: tree-sitter takes a
                        // trailing closure there (its `prec.right` operand), and the parse fails.
                        return self.refuse();
                    }
                    if self.no_lambda || !cx.shifts(-2) {
                        return Ok(());
                    }
                    let n = self.t.wrap(m, K::CallExpression, f::NONE);
                    self.call_suffix(false, false)?;
                    self.close(n);
                    if cx.stop != 0 {
                        return Ok(());
                    }
                }
                T::Question => {
                    // A ternary: `prec.right(-2)`.
                    if !cx.shifts(-2) {
                        return Ok(());
                    }
                    self.t.field_since(m, f::CONDITION);
                    let n = self.t.wrap(m, K::TernaryExpression, f::NONE);
                    self.tk(K::TQuestion)?;
                    let lambda = std::mem::replace(&mut self.no_lambda, false);
                    let stmt = std::mem::replace(&mut self.stmt, false);
                    let if_true = std::mem::replace(&mut self.if_true, true);
                    let r = self.expr_f(TOP, f::IF_TRUE);
                    self.if_true = if_true;
                    self.no_lambda = lambda;
                    self.stmt = stmt;
                    r?;
                    self.expect(T::Colon)?;
                    self.expr_f(HACK, f::IF_FALSE)?;
                    self.close(n);
                    if cx.stop == 1 {
                        return Ok(());
                    }
                }
                T::Eq | T::PlusEq | T::MinusEq | T::StarEq | T::SlashEq | T::PercentEq => {
                    // In `try`'s operand: a `try_expression` is never assignable.
                    if !cx.shifts(-3) && cx.stop != 1 {
                        return Ok(());
                    }
                    let d = self.t.wrap(m, K::DirectlyAssignableExpression, f::TARGET);
                    self.close(d);
                    let n = self.t.wrap(m, K::Assignment, f::NONE);
                    let k = super::punct(t.kind).expect("an operator");
                    self.tkf(k, f::OPERATOR)?;
                    self.expr_f(Cx { prec: -3, right: false, hack: false, stop: 0, split: false }, f::RESULT)?;
                    self.close(n);
                }
                T::DotDotDot | T::DotDotLt => {
                    if !cx.shifts(-1) {
                        return Ok(());
                    }
                    // An open end: no operand after it.
                    if t.kind == T::DotDotDot && !self.range_end_follows() {
                        self.t.field_since(m, f::START);
                        let n = self.t.wrap(m, K::OpenEndRangeExpression, f::NONE);
                        self.tk(K::TDotDotDot)?;
                        self.close(n);
                        continue;
                    }
                    self.t.field_since(m, f::START);
                    let n = self.t.wrap(m, K::RangeExpression, f::NONE);
                    self.tkf(super::punct(t.kind).expect("an operator"), f::OP)?;
                    self.expr_f(HACK, f::END)?;
                    self.close(n);
                }
                T::Name if !t.tick && t.word == w::AS => {
                    if !cx.shifts(-1) {
                        return Ok(());
                    }
                    self.t.field_since(m, f::EXPR);
                    let n = self.t.wrap(m, K::AsExpression, f::NONE);
                    self.as_operator()?;
                    self.cast_type()?;
                    self.close(n);
                }
                T::Name if !t.tick && t.word == w::IS => {
                    if !cx.shifts(7) {
                        return Ok(());
                    }
                    self.t.field_since(m, f::TARGET);
                    let n = self.t.wrap(m, K::CheckExpression, f::NONE);
                    self.tkf(K::TIs, f::OP)?;
                    self.cast_type()?;
                    self.close(n);
                }
                _ => {
                    let Some(Op::Bin(kind, prec, lf, rf, of, hack, right)) = self.binary(t.kind) else { return Ok(()) };
                    if !cx.shifts(prec) {
                        return Ok(());
                    }
                    self.t.field_since(m, lf);
                    let n = self.t.wrap(m, kind, f::NONE);
                    if t.kind == T::Custom {
                        self.leaf(K::CustomOperator, of)?;
                    } else {
                        self.tkf(super::punct(t.kind).expect("an operator"), of)?;
                    }
                    // Past seven calls read into right operands, tree-sitter's GLR has pruned the
                    // versions that nest right: the chain's rest nests left inside the last one
                    // (`a(x) ^ … ^ (g(x) ^ ((h(x) ^ i(x)) ^ j(x)))`, probed).
                    let rcx = if !hack {
                        Cx { prec, right, hack: false, stop: 0, split: false }
                    } else if self.hack_calls < 7 {
                        HACK
                    } else {
                        Cx { prec, right, hack: true, stop: 0, split: false }
                    };
                    self.expr_f(rcx, rf)?;
                    self.close(n);
                    if cx.stop == 1 {
                        return Ok(());
                    }
                }
            }
        }
    }

    /// A binary operator's node, precedence, fields and right operand.
    fn binary(&mut self, t: T) -> Option<Op> {
        use T::*;
        Some(match t {
            Star | Slash | Percent => Op::Bin(K::MultiplicativeExpression, 11, f::LHS, f::RHS, f::OP, false, false),
            Plus | Minus => Op::Bin(K::AdditiveExpression, 10, f::LHS, f::RHS, f::OP, false, false),
            Custom => Op::Bin(K::InfixExpression, 9, f::LHS, f::RHS, f::OP, true, false),
            QQ => Op::Bin(K::NilCoalescingExpression, 8, f::VALUE, f::IF_NIL, f::NONE, true, true),
            Lt | Gt => Op::Bin(K::ComparisonExpression, 0, f::LHS, f::RHS, f::OP, true, false),
            EqEq | EqEqEq => Op::Bin(K::EqualityExpression, 5, f::LHS, f::RHS, f::OP, true, false),
            AmpAmp => Op::Bin(K::ConjunctionExpression, 4, f::LHS, f::RHS, f::OP, true, false),
            PipePipe => Op::Bin(K::DisjunctionExpression, 3, f::LHS, f::RHS, f::OP, true, false),
            Amp | Pipe | Caret | LtLt | GtGt => Op::Bin(K::BitwiseOperation, 0, f::LHS, f::RHS, f::OP, true, false),
            _ => return None,
        })
    }

    /// Whether an operand follows a `...` (not a closing bracket, a `,`, a line's end).
    fn range_end_follows(&mut self) -> bool {
        let next = self.peek_at(1);
        if next.nl {
            return false;
        }
        !matches!(next.kind, T::RParen | T::RBracket | T::RBrace | T::Comma | T::Colon | T::Eof | T::Semi | T::LBrace)
    }

    /// The type after `is` or `as`. Where a dotted name ends at a `)` (`(x is A.B)`), the check
    /// or cast reduces before the `.` (a navigation of `x is A`), as seen; elsewhere the type
    /// takes the dots.
    fn cast_type(&mut self) -> R<()> {
        if self.peek() == T::Name && self.kind_at(1) == T::Dot && self.adjacent(0) {
            let mut j = 1;
            while self.kind_at(j) == T::Dot && self.kind_at(j + 1) == T::Name {
                j += 2;
            }
            if self.kind_at(j) == T::RParen && !matches!(self.word_at(j - 1), w::TYPE_T | w::PROTOCOL_T) {
                let u = self.open_field(K::UserType, f::NAME);
                self.ident_as(K::TypeIdentifier, f::NONE)?;
                self.close(u);
                return Ok(());
            }
        }
        self.ty_f(f::TYPE)
    }

    /// `as_operator`: `as`, `as?`, `as!` (one token).
    fn as_operator(&mut self) -> R<()> {
        let n = self.open(K::AsOperator);
        let t = self.tok();
        let next = self.peek_at(1);
        if next.start == t.end && matches!(next.kind, T::Question | T::Bang) {
            self.bump()?;
            self.bump()?;
            let k = if next.kind == T::Question { K::TAsQuestion } else { K::TAsBang };
            self.t.token(k, f::NONE, t.start, next.end);
        } else if next.start == t.end && matches!(next.kind, T::QQ | T::Custom) && matches!(self.src[next.start as usize], b'?' | b'!') {
            // `as?` before an operator: the operator's first character is the `as`'s.
            self.bump()?;
            self.split_at(0, T::Question, 1);
            self.bump()?;
            let k = if self.src[next.start as usize] == b'?' { K::TAsQuestion } else { K::TAsBang };
            self.t.token(k, f::NONE, t.start, next.start + 1);
        } else {
            self.tk(K::TAs)?;
        }
        self.close(n);
        Ok(())
    }

    // ---- Operands.

    /// A prefix operator and its operand, or a primary expression.
    fn unary(&mut self, cx: Cx) -> R<()> {
        let t = self.tok();
        let try_bang = std::mem::take(&mut self.try_bang);
        match t.kind {
            T::Bang | T::Minus | T::Plus | T::Tilde if self.kind_at(1) == T::LParen && self.adjacent(0) => {
                // `!(…)`, `-(…)`: a call of the operator (`_referenceable_operator`).
                let n = self.open(K::CallExpression);
                if t.kind == T::Bang {
                    self.leaf(K::Bang, f::NONE)?;
                } else {
                    self.tk(super::punct(t.kind).expect("an operator"))?;
                }
                self.call_suffix(true, true)?;
                self.close(n);
                Ok(())
            }
            T::Minus | T::Plus if self.if_true && self.kind_at(1) == T::Dot && self.adjacent(0) => {
                // `c ? -.x : y`: in a ternary's middle, a navigation of the operator
                // (`_referenceable_operator`, the operator its target; probed), elsewhere prefixes.
                // The operator here, the navigation the operators' after it.
                self.tk(super::punct(t.kind).expect("an operator"))?;
                Ok(())
            }
            T::Minus | T::Plus | T::Bang | T::Tilde | T::Amp | T::Dot | T::PlusPlus | T::MinusMinus | T::Custom => {
                let n = self.open(K::PrefixExpression);
                match t.kind {
                    T::Bang if try_bang => {
                        let b = self.open_field(K::Bang, f::OPERATION);
                        self.tk(K::TBang)?;
                        self.close(b);
                    }
                    T::Bang => {
                        self.leaf(K::Bang, f::OPERATION)?;
                    }
                    T::Custom => {
                        self.leaf(K::CustomOperator, f::OPERATION)?;
                    }
                    k => {
                        self.tkf(super::punct(k).expect("an operator"), f::OPERATION)?;
                    }
                }
                // The operand touches its operator.
                if !self.touching() {
                    return self.refuse();
                }
                if t.kind == T::Dot && self.peek() == T::Name && !self.tok().tick && matches!(self.word(), w::REPEAT | w::EACH) && self.kind_at(1) == T::LParen {
                    // `.repeat(…)`, `.each(…)`: the keyword's pack (`value_pack_expansion`,
                    // `value_parameter_pack`) over a tuple, the prefix's target (probed).
                    let m = self.mark();
                    let each = self.word() == w::EACH;
                    let p = self.open(if each { K::ValueParameterPack } else { K::ValuePackExpansion });
                    self.tk(if each { K::TEach } else { K::TRepeat })?;
                    self.tuple_expression()?;
                    self.close(p);
                    let stmt = std::mem::replace(&mut self.stmt, false);
                    let r = self.operators(m, PREFIX);
                    self.stmt = stmt;
                    r?;
                    self.t.field_since(m, f::TARGET);
                    self.t.unmark(m);
                    self.close(n);
                    return Ok(());
                }
                if t.kind == T::Dot && self.peek() == T::Name && !self.tok().tick && matches!(self.word(), w::DEFAULT | w::CASE | w::ELSE | w::IN | w::WHERE | w::CATCH | w::LET | w::VAR | w::FUNC | w::RETURN | w::IMPORT | w::INIT) {
                    // `.default`: after a `.`, any word is a name.
                    let m = self.mark();
                    self.simple_identifier()?;
                    let stmt = std::mem::replace(&mut self.stmt, false);
                    let r = self.operators(m, PREFIX);
                    self.stmt = stmt;
                    r?;
                    self.t.field_since(m, f::TARGET);
                    self.t.unmark(m);
                    self.close(n);
                    return Ok(());
                }
                let stmt = std::mem::replace(&mut self.stmt, false);
                let r = self.expr_f(PREFIX, f::TARGET);
                self.stmt = stmt;
                r?;
                self.close(n);
                Ok(())
            }
            T::DotDotDot | T::DotDotLt => {
                if t.kind == T::DotDotDot && !self.range_end_follows() {
                    let n = self.open(K::FullyOpenRange);
                    self.tk(K::TDotDotDot)?;
                    self.close(n);
                    return Ok(());
                }
                let n = self.open(K::OpenStartRangeExpression);
                self.tk(super::punct(t.kind).expect("an operator"))?;
                self.expr_f(Cx { prec: -2, right: true, hack: false, stop: 0, split: false }, f::END)?;
                self.close(n);
                Ok(())
            }
            _ => self.primary(cx),
        }
    }

    fn primary(&mut self, cx: Cx) -> R<()> {
        let t = self.tok();
        match t.kind {
            T::Name => {
                if !t.tick {
                    match t.word {
                        w::SELF => {
                            self.wrap_tok(K::SelfExpression, f::NONE, K::TSelf)?;
                            return Ok(());
                        }
                        w::SUPER => {
                            self.leaf(K::SuperExpression, f::NONE)?;
                            return Ok(());
                        }
                        w::TRUE | w::FALSE => {
                            self.wrap_tok(K::BooleanLiteral, f::NONE, if t.word == w::TRUE { K::TTrue } else { K::TFalse })?;
                            return Ok(());
                        }
                        w::NIL => {
                            self.leaf(K::NilLiteral, f::NONE)?;
                            return Ok(());
                        }
                        w::IF => return self.level(false, |p| p.if_statement()),
                        w::SWITCH => return self.level(false, |p| p.switch_statement()),
                        w::TRY => {
                            let n = self.open(K::TryExpression);
                            self.try_operator()?;
                            // A property's value splits a trailing closure off the call in `try`'s operand too.
                            self.try_bang = self.peek() == T::Bang;
                            self.expr_f(Cx { split: cx.split, ..TRY }, f::EXPR)?;
                            self.close(n);
                            return Ok(());
                        }
                        w::AWAIT => {
                            let n = self.open(K::AwaitExpression);
                            self.tk(K::TAwait)?;
                            self.expr_f(Cx { split: cx.split, ..AWAIT }, f::EXPR)?;
                            self.close(n);
                            return Ok(());
                        }
                        w::CASE | w::DEFAULT | w::ELSE | w::IN | w::WHERE | w::CATCH | w::LET | w::VAR | w::FUNC | w::RETURN | w::IMPORT => {
                            return self.refuse();
                        }
                        w::UNSAFE if !self.peek_at(1).nl && matches!(self.kind_at(1), T::Name | T::LParen | T::Int | T::Str | T::LBracket) => {
                            // `unsafe x`: its operand as `try`'s.
                            let n = self.open(K::UnsafeExpression);
                            self.tk(K::TUnsafe)?;
                            self.expr_f(Cx { split: cx.split, ..TRY }, f::EXPR)?;
                            self.close(n);
                            return Ok(());
                        }
                        w::CONSUME | w::COPY | w::EACH | w::REPEAT | w::UNSAFE | w::DISCARD if self.kind_at(1) == T::Name && !self.peek_at(1).nl => {
                            return self.refuse();
                        }
                        _ => {}
                    }
                }
                // `A.B<C, D>(…)`, `A.B<C, D> { … }`: a qualified generic type's constructor,
                // where the comparisons tree-sitter also parses cannot (probed: `a.b<C>(d)` is
                // comparisons).
                if self.kind_at(1) == T::Dot && self.adjacent(0) {
                    let mut k = 0;
                    while self.kind_at(k + 1) == T::Dot && self.kind_at(k + 2) == T::Name && self.adjacent(k) && self.adjacent(k + 1) {
                        k += 2;
                    }
                    if self.kind_at(k + 1) == T::Lt && self.adjacent(k) {
                        if let Some(close) = self.type_args_end(k + 1) {
                            let args = self.kind_at(close + 1) == T::LParen && self.adjacent(close);
                            let lambda = self.kind_at(close + 1) == T::LBrace && !self.no_lambda;
                            if (args || lambda) && self.comparisons_fail(k + 1, close) {
                                let n = self.open(K::ConstructorExpression);
                                self.user_type_f(f::CONSTRUCTED_TYPE)?;
                                let s = self.open(K::ConstructorSuffix);
                                if args {
                                    self.value_arguments()?;
                                }
                                if self.peek() == T::LBrace && !self.no_lambda && !(args && self.ends()) {
                                    self.lambda_literal()?;
                                }
                                self.close(s);
                                self.close(n);
                                return Ok(());
                            }
                        }
                    }
                }
                // A generic type's constructor: `Name<…>(`.
                if self.kind_at(1) == T::Lt && self.adjacent(0)
                    && let Some(close) = self.type_args_end(1) {
                        if self.kind_at(close + 1) == T::Dot && self.adjacent(close) && self.generic_member_is_type(1, close + 2) {
                            // `Name<…>.member`: the type a navigation's target.
                            self.u_end = self.peek_at(close + 2).end;
                            let u = self.open(K::UserType);
                            self.ident_as(K::TypeIdentifier, f::NONE)?;
                            self.type_arguments()?;
                            self.close(u);
                            return Ok(());
                        }
                        let args = self.kind_at(close + 1) == T::LParen && self.adjacent(close);
                        let lambda = self.kind_at(close + 1) == T::LBrace && !self.no_lambda;
                        if args || lambda {
                            // `Name<…>(…)`, `Name<…> { … }`: a constructor, its trailing closure in
                            // its suffix.
                            let n = self.open(K::ConstructorExpression);
                            let u = self.open_field(K::UserType, f::CONSTRUCTED_TYPE);
                            self.ident_as(K::TypeIdentifier, f::NONE)?;
                            self.type_arguments()?;
                            self.close(u);
                            let s = self.open(K::ConstructorSuffix);
                            if args {
                                self.value_arguments()?;
                            }
                            if self.peek() == T::LBrace && !self.no_lambda && !(args && self.ends()) {
                                self.lambda_literal()?;
                            }
                            self.close(s);
                            self.close(n);
                            return Ok(());
                        }
                    }
                self.simple_identifier()?;
                Ok(())
            }
            T::Int => self.leaf(K::IntegerLiteral, f::NONE).map(|_| ()),
            T::Hex => self.leaf(K::HexLiteral, f::NONE).map(|_| ()),
            T::Oct => self.leaf(K::OctLiteral, f::NONE).map(|_| ()),
            T::Bin => self.leaf(K::BinLiteral, f::NONE).map(|_| ()),
            T::Real => self.leaf(K::RealLiteral, f::NONE).map(|_| ()),
            T::Str => self.string_literal(),
            T::LParen => self.tuple_expression(),
            T::LBracket if self.kind_at(1) == T::Name && self.kind_at(2) == T::RBracket && self.kind_at(3) == T::Dot && self.adjacent(2) => {
                // `[Name].member`: an array type, a navigation's target.
                let n = self.open(K::ArrayType);
                self.tk(K::TLBracket)?;
                self.ty_f(f::NONE)?;
                self.tk(K::TRBracket)?;
                self.close(n);
                Ok(())
            }
            T::LBracket if self.bracket_type_constructor() => {
                let n = self.open(K::ConstructorExpression);
                let b = self.mark();
                self.tk(K::TLBracket)?;
                let dict = self.level(false, |p| {
                    p.ty_f(f::NONE)?;
                    if p.peek() == T::Colon {
                        p.tk(K::TColon)?;
                        p.ty_f(f::NONE)?;
                        return Ok(true);
                    }
                    Ok(false)
                })?;
                self.expect(T::RBracket)?;
                let a = self.t.wrap(b, if dict { K::DictionaryType } else { K::ArrayType }, f::CONSTRUCTED_TYPE);
                self.close(a);
                self.t.unmark(b);
                let s = self.open(K::ConstructorSuffix);
                self.value_arguments()?;
                self.close(s);
                self.close(n);
                Ok(())
            }
            T::LBracket => self.collection_literal(),
            T::LBrace => self.lambda_literal(),
            T::Backslash => self.key_path_expression(),
            T::Hash => self.hash_expression(),
            _ => self.refuse(),
        }
    }

    /// Whether `Name < … >` from the `<` at `lt` to the `>` at `close` cannot parse as
    /// comparisons: a `>` inside before no operand, or a `,` at the arguments' top where no list
    /// takes it.
    fn comparisons_fail(&mut self, lt: usize, close: usize) -> bool {
        let mut d = 0;
        for j in lt..close {
            match self.kind_at(j) {
                T::Lt => d += 1,
                T::Gt | T::GtGt => {
                    d -= if self.kind_at(j) == T::Gt { 1 } else { 2 };
                    if matches!(self.kind_at(j + 1), T::Comma | T::Gt | T::GtGt | T::RParen) {
                        return true;
                    }
                }
                T::Comma if d == 1 && self.stmt => return true,
                _ => {}
            }
        }
        false
    }

    /// After `Name<…>.member` (`k`: the member's index), whether tree-sitter's GLR keeps the
    /// type (a navigation of `user_type`) over the comparisons it also parses (`Name < … > .member`).
    /// Measured on the corpus: the type where a binary operator follows and its operand is no call,
    /// or where a call is navigated on; else the comparisons.
    fn generic_member_is_type(&mut self, lt: usize, k: usize) -> bool {
        if self.kind_at(k) != T::Name {
            return false;
        }
        if self.comparisons_fail(lt, k - 2) {
            return true;
        }
        let mut j = k + 1;
        if self.kind_at(j) == T::LParen && self.adjacent(k) {
            let mut d = 0;
            loop {
                match self.kind_at(j) {
                    T::LParen | T::LBracket => d += 1,
                    T::RParen | T::RBracket => {
                        d -= 1;
                        if d == 0 {
                            break;
                        }
                    }
                    T::Eof => return false,
                    _ => {}
                }
                j += 1;
            }
            return self.kind_at(j + 1) == T::Dot && self.adjacent(j);
        }
        if self.peek_at(j).nl {
            return false;
        }
        // The operand of `*`, `/`, `+`, `-` (seen: `A<T>.size / A<U>.size`).
        // The right operand of `*`, `/`, `+`, `-` whose left one is such a type (seen:
        // `A<T>.size / A<U>.size`).
        if self.i > 1 && matches!(self.toks[self.i - 1].kind, T::Star | T::Slash | T::Percent | T::Plus | T::Minus) && self.toks[self.i - 2].end == self.u_end {
            return true;
        }
        match self.kind_at(j) {
            T::Star | T::Slash | T::Percent | T::Plus | T::Minus | T::Custom => {
                !(self.kind_at(j + 1) == T::Name && self.kind_at(j + 2) == T::LParen)
            }
            _ => false,
        }
    }

    /// Whether the `[` next opens a type a constructor calls (`[T<U>]()`, `[K: () -> V]()`): a
    /// `<` or `->` in it, `(` touching its `]`. (`[Int](…)` is a call of an array literal.)
    fn bracket_type_constructor(&mut self) -> bool {
        let mut d = 0;
        let mut typed = false;
        let mut j = 0;
        loop {
            match self.kind_at(j) {
                T::LBracket | T::LParen => d += 1,
                T::RBracket | T::RParen => {
                    d -= 1;
                    if d == 0 {
                        return typed && self.kind_at(j) == T::RBracket && self.kind_at(j + 1) == T::LParen && self.adjacent(j);
                    }
                }
                T::Lt | T::Arrow => typed = true,
                T::Name | T::Comma | T::Colon | T::Dot | T::Gt | T::GtGt | T::Question => {}
                _ => return false,
            }
            j += 1;
            if j > 64 {
                return false;
            }
        }
    }

    /// `try_operator`: `try`, `try?`, `try!` (the mark touching).
    pub(super) fn try_operator(&mut self) -> R<()> {
        let n = self.open(K::TryOperator);
        self.tk(K::TTry)?;
        if self.touching() {
            match self.peek() {
                T::Question => {
                    self.tk(K::TQuestion)?;
                }
                T::Bang => {
                    self.tk(K::TBang)?;
                }
                _ => {}
            }
        }
        self.close(n);
        Ok(())
    }

    /// The index of the `>` closing the type arguments whose `<` is token `k` ahead; `None` where
    /// the tokens between cannot be types.
    pub(super) fn type_args_end(&mut self, k: usize) -> Option<usize> {
        let mut depth = 0i32;
        let mut j = k;
        loop {
            match self.kind_at(j) {
                T::Lt => depth += 1,
                T::Gt => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                }
                T::GtGt => {
                    if depth == 2 {
                        return Some(j);
                    }
                    if depth < 2 {
                        return None;
                    }
                    depth -= 2;
                }
                T::Name | T::Comma | T::Dot | T::LBracket | T::RBracket | T::Colon | T::Question | T::LParen | T::RParen | T::Arrow | T::Amp | T::Int => {}
                _ => return None,
            }
            j += 1;
            if j > k + 64 {
                return None;
            }
        }
    }

    // ---- Calls.

    /// `call_suffix`: value arguments, a trailing closure, or both; trailing closures after
    /// arguments where `args` (`{` on the same line).
    fn call_suffix(&mut self, args: bool, split: bool) -> R<()> {
        let n = self.open(K::CallSuffix);
        if args {
            self.value_arguments()?;
        }
        if self.peek() == T::LBrace && !self.no_lambda && !(args && (split || self.ends())) {
            self.lambda_literal()?;
            // Labeled trailing closures: `label: { … }`.
            while self.peek() == T::Name && self.kind_at(1) == T::Colon && self.kind_at(2) == T::LBrace {
                self.simple_identifier_f(f::NAME)?;
                self.tk(K::TColon)?;
                self.lambda_literal()?;
            }
        }
        self.close(n);
        Ok(())
    }

    /// `value_arguments`: `(…)` or `[…]`.
    pub(super) fn value_arguments(&mut self) -> R<()> {
        let n = self.open(K::ValueArguments);
        let close = if self.peek() == T::LParen {
            self.tk(K::TLParen)?;
            T::RParen
        } else {
            self.expect(T::LBracket)?;
            T::RBracket
        };
        self.level(false, |p| {
            while p.peek() != close {
                p.value_argument()?;
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
            }
            Ok(())
        })?;
        self.expect(close)?;
        self.close(n);
        Ok(())
    }

    fn value_argument(&mut self) -> R<()> {
        let n = self.open(K::ValueArgument);
        if self.peek() == T::At {
            let m = self.open(K::TypeModifiers);
            while self.peek() == T::At {
                self.attribute()?;
            }
            self.close(m);
        }
        // Labels: `name:`; several, or one before `)`/`,`, are a reference's (`f(_:b:)`).
        if self.label_ahead(0) {
            let mut k = 0;
            while self.label_ahead(k) {
                k += 2;
            }
            let reference = k > 2 || matches!(self.kind_at(k), T::RParen | T::Comma | T::RBracket);
            if reference {
                while self.label_ahead(0) {
                    self.value_argument_label(f::REFERENCE_SPECIFIER)?;
                    self.tk(K::TColon)?;
                }
                self.close(n);
                return Ok(());
            }
            self.value_argument_label(f::NAME)?;
            self.tk(K::TColon)?;
        }
        // An operator as a value (`reduce(0, +)`).
        let t = self.tok();
        if matches!(self.kind_at(1), T::Comma | T::RParen)
            && matches!(t.kind, T::Plus | T::Minus | T::Star | T::Slash | T::Percent | T::Lt | T::Gt | T::EqEq | T::AmpAmp | T::PipePipe | T::Amp | T::Pipe | T::Caret)
        {
            self.tkf(super::punct(t.kind).expect("an operator"), f::VALUE)?;
            self.close(n);
            return Ok(());
        }
        self.expression_f(f::VALUE)?;
        self.close(n);
        Ok(())
    }

    /// Whether a label (`name:`, not `::`) is `k` tokens ahead.
    fn label_ahead(&mut self, k: usize) -> bool {
        self.kind_at(k) == T::Name && self.kind_at(k + 1) == T::Colon
    }

    fn value_argument_label(&mut self, field: u8) -> R<()> {
        let n = self.open_field(K::ValueArgumentLabel, field);
        self.simple_identifier()?;
        self.close(n);
        Ok(())
    }

    // ---- Literals.

    /// `tuple_expression`: `(` elements `)`, a name before an element's value.
    fn tuple_expression(&mut self) -> R<()> {
        if self.kind_at(1) == T::RParen {
            return self.refuse();
        }
        let n = self.open(K::TupleExpression);
        self.tk(K::TLParen)?;
        self.level(false, |p| {
            loop {
                if p.label_ahead(0) {
                    p.simple_identifier_f(f::NAME)?;
                    p.tk(K::TColon)?;
                }
                p.expression_f(f::VALUE)?;
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
                if p.peek() == T::RParen {
                    break;
                }
            }
            Ok(())
        })?;
        self.expect(T::RParen)?;
        self.close(n);
        Ok(())
    }

    /// `array_literal` or `dictionary_literal`, by what follows the first element.
    fn collection_literal(&mut self) -> R<()> {
        let m = self.mark();
        self.tk(K::TLBracket)?;
        let kind = self.level(false, |p| {
            if p.peek() == T::RBracket {
                p.tk(K::TRBracket)?;
                return Ok(K::ArrayLiteral);
            }
            if p.peek() == T::Colon && p.kind_at(1) == T::RBracket {
                p.tk(K::TColon)?;
                p.tk(K::TRBracket)?;
                return Ok(K::DictionaryLiteral);
            }
            let e = p.mark();
            p.expression()?;
            let dict = p.peek() == T::Colon;
            p.t.field_since(e, if dict { f::KEY } else { f::ELEMENT });
            p.t.unmark(e);
            if dict {
                p.tk(K::TColon)?;
                p.expression_f(f::VALUE)?;
            }
            while p.peek() == T::Comma {
                p.tk(K::TComma)?;
                if p.peek() == T::RBracket {
                    break;
                }
                if dict {
                    p.expression_f(f::KEY)?;
                    p.expect(T::Colon)?;
                    p.expression_f(f::VALUE)?;
                } else {
                    p.expression_f(f::ELEMENT)?;
                }
            }
            p.expect(T::RBracket)?;
            Ok(if dict { K::DictionaryLiteral } else { K::ArrayLiteral })
        })?;
        let n = self.t.wrap(m, kind, f::NONE);
        self.close(n);
        self.t.unmark(m);
        Ok(())
    }

    /// `\\`: a key path's root; its components are navigation suffixes read after it (a root
    /// type, never seen in the corpus, is refused).
    fn key_path_expression(&mut self) -> R<()> {
        if !self.adjacent(0) {
            return self.refuse();
        }
        let n = self.open(K::KeyPathExpression);
        self.tk(K::TBackslash)?;
        if self.peek() == T::Name {
            // `\Foo.bar`: the root type's name in the key path (generic roots unseen: refused).
            self.ident_as(K::TypeIdentifier, f::NONE)?;
            if !matches!(self.peek(), T::Dot) || !self.touching() {
                return self.refuse();
            }
        } else if self.peek() != T::Dot {
            return self.refuse();
        }
        self.close(n);
        Ok(())
    }

    /// `#…`: a special literal, `#selector`, `#keyPath`, a macro invocation.
    fn hash_expression(&mut self) -> R<()> {
        if !self.adjacent(0) || self.kind_at(1) != T::Name {
            return self.refuse();
        }
        let word = self.word_at(1);
        let special = match word {
            w::FILE => Some(K::TFile),
            w::FILEID => Some(K::TFileID),
            w::FILEPATH => Some(K::TFilePath),
            w::LINE => Some(K::TLine),
            w::COLUMN => Some(K::TColumn),
            w::FUNCTION => Some(K::TFunction),
            w::DSOHANDLE => Some(K::TDsohandle),
            _ => None,
        };
        if let Some(k) = special {
            let n = self.open(K::SpecialLiteral);
            self.tk(K::THash)?;
            self.tk(k)?;
            self.close(n);
            return Ok(());
        }
        match word {
            w::SELECTOR => {
                let n = self.open(K::SelectorExpression);
                self.tk(K::THash)?;
                self.tk(K::TSelector)?;
                self.expect(T::LParen)?;
                if matches!(self.word(), w::GET | w::SET) && self.kind_at(1) == T::Colon {
                    return self.refuse();
                }
                self.level(false, |p| p.expression())?;
                self.expect(T::RParen)?;
                self.close(n);
                Ok(())
            }
            w::KEYPATH => {
                let n = self.open(K::KeyPathStringExpression);
                self.tk(K::THash)?;
                self.tk(K::TKeyPath)?;
                self.expect(T::LParen)?;
                self.level(false, |p| p.expression())?;
                self.expect(T::RParen)?;
                self.close(n);
                Ok(())
            }
            w::COLORLITERAL | w::FILELITERAL | w::IMAGELITERAL | w::AVAILABLE | w::UNAVAILABLE | w::IF | w::ELSE | w::ELSEIF | w::ENDIF
            | w::ERROR | w::WARNING | w::SOURCELOCATION => self.refuse(),
            _ => {
                let n = self.open(K::MacroInvocation);
                self.tk(K::THash)?;
                self.simple_identifier()?;
                if self.peek() == T::Lt && self.touching() {
                    return self.refuse();
                }
                if matches!(self.peek(), T::LParen) && self.touching() || self.peek() == T::LBrace && !self.no_lambda {
                    let args = self.peek() == T::LParen;
                    self.call_suffix(args, false)?;
                }
                self.close(n);
                Ok(())
            }
        }
    }

    // ---- Strings.

    /// A string literal: its pieces read from the bytes, an interpolation's expression from tokens
    /// lexed inside it.
    fn string_literal(&mut self) -> R<()> {
        let t = self.bump()?;
        let base = self.i;
        let src = self.src;
        let at = |k: usize| src.get(k).copied().unwrap_or(0);
        let mut interpolated = false;
        let mut i = t.start as usize;
        if at(i) == b'#' {
            // A raw string: one `raw_str_end_part` where no interpolation is in it.
            let mut h = 0;
            while at(i + h) == b'#' {
                h += 1;
            }
            let body = &src[i..t.end as usize];
            let mut needle = vec![b'\\'];
            needle.extend(std::iter::repeat_n(b'#', h));
            needle.push(b'(');
            if body.windows(needle.len()).any(|w| w == needle.as_slice()) {
                return self.refuse();
            }
            let n = self.t.open(K::RawStringLiteral, f::NONE, t.start);
            self.t.leaf(K::RawStrEndPart, f::TEXT, t.start, t.end);
            self.t.close_at(n, t.end);
            return Ok(());
        }
        let triple = src[i..].starts_with(b"\"\"\"");
        let (kind, text) = if triple { (K::MultiLineStringLiteral, K::MultiLineStrText) } else { (K::LineStringLiteral, K::LineStrText) };
        let n = self.t.open(kind, f::NONE, t.start);
        if triple {
            self.t.token(K::TTripleQuote, f::NONE, t.start, t.start + 3);
            i += 3;
        } else {
            self.t.token(K::TDQuote, f::NONE, t.start, t.start + 1);
            i += 1;
        }
        let end = t.end as usize;
        let close = end - if triple { 3 } else { 1 };
        while i < close {
            match src[i] {
                b'\\' => match at(i + 1) {
                    b'(' => {
                        self.t.token(K::TBackslashLParen, f::NONE, i as u32, i as u32 + 2);
                        i = self.interpolation(i + 2)?;
                        interpolated = true;
                    }
                    b'u' if at(i + 2) == b'{' => {
                        let mut k = i + 3;
                        while at(k).is_ascii_hexdigit() {
                            k += 1;
                        }
                        if k == i + 3 || at(k) != b'}' {
                            return self.refuse();
                        }
                        let x = self.t.open(K::StrEscapedChar, f::TEXT, i as u32);
                        self.t.token(K::TBackslash, f::NONE, i as u32, i as u32 + 1);
                        self.t.token(K::TU, f::NONE, i as u32 + 1, i as u32 + 2);
                        self.t.last_end = k as u32 + 1;
                        self.t.close_at(x, k as u32 + 1);
                        i = k + 1;
                    }
                    b'0' | b'\\' | b't' | b'n' | b'r' | b'"' | b'\'' | b'\n' => {
                        self.t.leaf(K::StrEscapedChar, f::TEXT, i as u32, i as u32 + 2);
                        i += 2;
                    }
                    _ => return self.refuse(),
                },
                b'"' if triple => {
                    self.t.token(K::TDQuote, f::TEXT, i as u32, i as u32 + 1);
                    i += 1;
                }
                _ => {
                    // The text to the next `"` or `\\` (a `$` stop passed).
                    let mut k = self.lx.blocks.string_stop(i);
                    while k < close && src[k] == b'$' {
                        k = self.lx.blocks.string_stop(k + 1);
                    }
                    let k = k.min(close);
                    if !triple && k < close && src[k] == b'"' {
                        return self.refuse();
                    }
                    self.t.leaf(text, f::TEXT, i as u32, k as u32);
                    i = k;
                }
            }
        }
        if triple {
            self.t.token(K::TTripleQuote, f::NONE, close as u32, end as u32);
        } else {
            self.t.token(K::TDQuote, f::NONE, close as u32, end as u32);
        }
        // Without an interpolation's tokens, those read after the string are those lexed from its
        // end already.
        self.i = base;
        if interpolated || end != t.end as usize {
            self.relex_at(end);
        }
        self.t.last_end = end as u32;
        self.t.close_at(n, end as u32);
        Ok(())
    }

    /// An interpolation's expressions from `at` (past its `\(`), and its `)`: the byte after it.
    fn interpolation(&mut self, at: usize) -> R<usize> {
        self.relex_at(at);
        self.t.last_end = at as u32;
        self.level(false, |p| {
            loop {
                let n = p.open_field(K::InterpolatedExpression, f::INTERPOLATION);
                if p.label_ahead(0) {
                    p.value_argument_label(f::NAME)?;
                    p.tk(K::TColon)?;
                }
                p.expression_f(f::VALUE)?;
                p.close(n);
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
            }
            Ok(())
        })?;
        let t = self.tok();
        if t.kind != T::RParen {
            return self.refuse();
        }
        self.tk(K::TRParen)?;
        self.relex_at(t.end as usize);
        Ok(t.end as usize)
    }

    // ---- Closures.

    /// `lambda_literal`: `{`, a header (captures, parameters, `in`) where an `in` ends one, the
    /// statements, `}`.
    pub(super) fn lambda_literal(&mut self) -> R<()> {
        let n = self.open(K::LambdaLiteral);
        self.expect(T::LBrace)?;
        if let Some(k) = self.lambda_header_end() {
            let _ = k;
            self.level(false, |p| p.lambda_header())?;
        }
        self.level(true, |p| p.statements(false))?;
        self.expect(T::RBrace)?;
        self.close(n);
        Ok(())
    }

    /// The index of the `in` ending a closure's header (the `{` read); `None` where the tokens
    /// before it cannot be a header.
    fn lambda_header_end(&mut self) -> Option<usize> {
        let mut depth = 0i32;
        let mut k = 0;
        loop {
            let t = self.peek_at(k);
            match t.kind {
                T::Name if !t.tick && t.word == w::IN && depth == 0 => return if k == 0 { None } else { Some(k) },
                T::LParen | T::LBracket => depth += 1,
                T::RParen | T::RBracket => {
                    depth -= 1;
                    if depth < 0 {
                        return None;
                    }
                }
                T::Name if !t.tick && matches!(t.word, w::LET | w::VAR | w::FOR | w::IF | w::RETURN | w::GUARD | w::SWITCH | w::WHILE) => return None,
                T::Name | T::Comma | T::Colon | T::Arrow | T::Dot | T::Question | T::Bang | T::Lt | T::Gt | T::At | T::Amp => {}
                _ => return None,
            }
            k += 1;
            if k > 200 {
                return None;
            }
        }
    }

    fn lambda_header(&mut self) -> R<()> {
        while self.peek() == T::At {
            self.attribute()?;
        }
        if self.peek() == T::LBracket {
            self.capture_list()?;
        }
        if !self.is_word(w::IN) {
            let n = self.open_field(K::LambdaFunctionType, f::TYPE);
            if self.peek() == T::LParen {
                self.tk(K::TLParen)?;
                if self.peek() != T::RParen {
                    self.lambda_parameters()?;
                }
                self.expect(T::RParen)?;
            } else {
                self.lambda_parameters()?;
            }
            if self.is_word(w::ASYNC) {
                self.tk(K::TAsync)?;
            }
            if matches!(self.word(), w::THROWS | w::RETHROWS) {
                self.throws()?;
            }
            if self.peek() == T::Arrow {
                self.tk(K::TArrow)?;
                self.possibly_unwrapped_type(f::RETURN_TYPE)?;
            }
            self.close(n);
        }
        self.kw(w::IN, K::TIn)?;
        Ok(())
    }

    fn lambda_parameters(&mut self) -> R<()> {
        let n = self.open(K::LambdaFunctionTypeParameters);
        loop {
            let q = self.open(K::LambdaParameter);
            if self.is_word(w::SELF) {
                self.wrap_tok(K::SelfExpression, f::NONE, K::TSelf)?;
            } else if self.peek() == T::Name && self.kind_at(1) == T::Name && self.kind_at(2) == T::Colon {
                self.simple_identifier_f(f::EXTERNAL_NAME)?;
                self.simple_identifier_f(f::NAME)?;
                self.tk(K::TColon)?;
                self.parameter_type()?;
            } else {
                self.simple_identifier_f(f::NAME)?;
                if self.peek() == T::Colon {
                    self.tk(K::TColon)?;
                    self.parameter_type()?;
                }
            }
            self.close(q);
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::TComma)?;
        }
        self.close(n);
        Ok(())
    }

    fn capture_list(&mut self) -> R<()> {
        let n = self.open_field(K::CaptureList, f::CAPTURES);
        self.tk(K::TLBracket)?;
        loop {
            let q = self.open(K::CaptureListItem);
            if self.is_word(w::SELF) && matches!(self.kind_at(1), T::Comma | T::RBracket) {
                let s = self.open_field(K::SelfExpression, f::NAME);
                self.tk(K::TSelf)?;
                self.close(s);
            } else {
                // `weak x`, `unowned x`, `unowned(safe) x`.
                if (matches!(self.word(), w::WEAK | w::UNOWNED) && self.kind_at(1) == T::Name) || (self.is_word(w::UNOWNED) && self.kind_at(1) == T::LParen) {
                    self.ownership_modifier()?;
                }
                self.simple_identifier_f(f::NAME)?;
                if self.peek() == T::Eq {
                    self.tk(K::TEq)?;
                    self.expression_f(f::VALUE)?;
                }
            }
            self.close(q);
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::TComma)?;
            if self.peek() == T::RBracket {
                break;
            }
        }
        self.expect(T::RBracket)?;
        self.close(n);
        Ok(())
    }
}
