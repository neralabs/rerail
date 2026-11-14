//! Patterns: `is`, `case` labels, switch expression arms, and the designations they bind.

use super::expr::{P_CONST, binary_prec, is_binary};
use super::*;

impl<'a> P<'a> {
    /// `left is …`: an `is_expression` (a type) or an `is_pattern_expression`, as tree-sitter
    /// reads them:
    /// - a type that is no expression (`int`, `T[]`, `A<B>`) alone is an `is_expression`;
    /// - a name before a binary operator is one too when the operator binds tighter than the
    ///   constant's slot allows, or inside a constant that already absorbs (`Cmode`);
    /// - every other is a pattern. At a chain's start (`Cmode::Start`) its constant takes the
    ///   operators after it (`x is null || y` is `x is (null || y)`) when `constant_takes` agrees:
    ///   with no other `is` in the range, or with a plain constant ending it;
    /// - shapes whose reading depends on more than this (dotted names before an operator, a
    ///   conditional access that wrapped the chain, a range ending at `?`) refuse.
    pub(super) fn is_tail(&mut self, left: N, s: Slot, min: u8) -> R<N> {
        let start = !self.in_constant;
        let is_expression = match self.is_type_ahead() {
            Some((false, _)) => true,
            // A name alone before an operator, where the `is` does not start its chain (or stands
            // in a constant): an `is_expression`.
            // A dotted name there: tree-sitter's readings vary; not read.
            Some((true, after)) if is_binary(after) && self.kind_at(2) == T::Dot => return self.refuse(),
            // After a conditional access wrapped the chain before it (`a || b?.c != d || e is A ||`),
            // likewise.
            Some((true, after)) if is_binary(after) && self.wrapped_chain => return self.refuse(),
            Some((true, after)) => is_binary(after) && (min > P_CONST || self.in_constant),
            None => false,
        };
        if is_expression {
            let n = self.wrap(left, K::IsExpression, s, slot(f::LEFT, Ctx::Expr));
            self.leaf(K::TIs, f::OPERATOR, C_NONE)?;
            self.ty_in_expr(slot(f::RIGHT, Ctx::Type))?;
            self.close(n);
            return Ok(n);
        }
        let n = self.wrap(left, K::IsPatternExpression, s, slot(f::EXPRESSION, Ctx::Expr));
        self.tk(K::IsPatternExpression, K::TIs)?;
        let cmode = self.cmode;
        self.cmode = if start { Cmode::Start(min) } else { Cmode::Inner(min) };
        self.pattern(slot(f::PATTERN, Ctx::Pattern))?;
        self.cmode = cmode;
        self.close(n);
        Ok(n)
    }

    /// The type after `is` (next but one), where it is not continued as an expression: whether it
    /// is a dotted name alone (else an `is_expression`'s), and the token after it.
    fn is_type_ahead(&mut self) -> Option<(bool, T)> {
        let t = self.peek_at(1);
        if t.kind != T::Name || matches!(t.word, w::NOT | w::VAR | w::NULL | w::TRUE | w::FALSE | w::UNDERSCORE) {
            return None;
        }
        let e = self.scan_type(1)?;
        let after = self.peek_at(e);
        let continues = (after.kind == T::Name && !matches!(after.word, w::AS | w::IS | w::SWITCH | w::WITH)) || matches!(after.kind, T::LBrace | T::LParen);
        if continues {
            return None;
        }
        Some((self.name_only(1, e), after.kind))
    }

    /// After an `is` that starts its chain, whether its constant (from the next token) takes the
    /// operators after it as tree-sitter does: `Some(false)` where no other `is` stands in them,
    /// `Some(true)` where the last `is` among them has a plain constant that ends them, `None`
    /// otherwise (tree-sitter's choice there is not read).
    fn constant_takes(&mut self) -> Option<bool> {
        let (mut k, mut depth, mut last_is, mut others, mut ops) = (0usize, 0u32, None, 0u32, 0u32);
        loop {
            let t = self.peek_at(k);
            match t.kind {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace if depth > 0 => depth -= 1,
                T::Eof => return None,
                _ if depth > 0 => {}
                T::RParen | T::RBracket | T::RBrace | T::Semi | T::Comma | T::Colon | T::FatArrow => break,
                // Before a conditional's `?`, tree-sitter's choice is not read where operators
                // stand between.
                T::Question if ops == 0 && !matches!(self.kind_at(k + 1), T::Dot | T::LBracket) => break,
                T::Question => return None,
                T::Name if t.word == w::IS => {
                    last_is = Some(k);
                    others += 1;
                }
                k2 if is_binary(k2) => ops += 1,
                k2 if super::expr::assignment(k2).is_some() => break,
                _ => {}
            }
            k += 1;
        }
        let Some(j) = last_is else { return Some(false) };
        // In a longer chain, tree-sitter's readings vary (its parse versions are bounded).
        if others > 3 {
            return None;
        }
        // `is` and a plain constant (a literal, `null`, a dotted name, `-` a number), then the end.
        let mut c = j + 1;
        if self.is_word_at(c, w::NOT) {
            c += 1;
        }
        if self.kind_at(c) == T::Minus {
            c += 1;
        }
        let t = self.peek_at(c);
        let plain = match t.kind {
            T::Int | T::Real | T::Char | T::Str => c + 1,
            T::Name if matches!(t.word, w::NULL | w::TRUE | w::FALSE) => c + 1,
            T::Name if !w::is_reserved(t.word) => {
                let e = self.scan_type(c)?;
                if !self.name_only(c, e) {
                    return None;
                }
                e
            }
            _ => return None,
        };
        (plain == k).then_some(true)
    }

    /// Whether tokens `from..to` are a dotted name alone, type arguments allowed (no suffix): a
    /// constant pattern's, to tree-sitter.
    fn name_only(&mut self, from: usize, to: usize) -> bool {
        let mut k = from;
        loop {
            if !self.is_ident_at(k) {
                return false;
            }
            k += 1;
            if self.kind_at(k) == T::Lt {
                match self.scan_type_args(k) {
                    Some(e) => k = e,
                    None => return false,
                }
            }
            if k == to {
                return true;
            }
            if self.kind_at(k) != T::Dot {
                return false;
            }
            k += 1;
        }
    }

    /// A `pattern` in slot `s`: `or` patterns of `and` patterns of the rest.
    pub(super) fn pattern(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let mut left = self.and_pattern(s)?;
        while self.is_word(w::OR) {
            left = self.wrap(left, K::OrPattern, s, slot(f::LEFT, Ctx::Pattern));
            self.tk_field(K::OrPattern, K::TOr, f::OPERATOR)?;
            self.and_pattern(slot(f::RIGHT, Ctx::Pattern))?;
            self.close(left);
        }
        self.depth -= 1;
        Ok(left)
    }

    fn and_pattern(&mut self, s: Slot) -> R<N> {
        let mut left = self.unary_pattern(s)?;
        while self.is_word(w::AND) {
            left = self.wrap(left, K::AndPattern, s, slot(f::LEFT, Ctx::Pattern));
            self.tk_field(K::AndPattern, K::TAnd, f::OPERATOR)?;
            self.unary_pattern(slot(f::RIGHT, Ctx::Pattern))?;
            self.close(left);
        }
        Ok(left)
    }

    fn unary_pattern(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let t = self.tok();
        let n = match t.kind {
            T::Name if t.word == w::NOT => {
                let n = self.open_in(K::NegatedPattern, s)?;
                self.tk(K::NegatedPattern, K::TNot)?;
                self.unary_pattern(PATTERN)?;
                self.close(n);
                n
            }
            T::Lt | T::Le | T::Gt | T::Ge => {
                let n = self.open_in(K::RelationalPattern, s)?;
                self.leaf(punct(t.kind).expect("an operator"), f::NONE, C_NONE)?;
                self.binary_expr(EXPR, P_CONST)?;
                self.close(n);
                n
            }
            T::LBracket => self.list_pattern(s)?,
            T::LBrace => {
                let n = self.open_in(K::RecursivePattern, s)?;
                self.property_clause()?;
                self.designation_opt()?;
                self.close(n);
                n
            }
            T::LParen => {
                let close = self.matching(0).ok_or_else(|| Refusal::Parse(t.start))?;
                if self.top_comma_pub(1, close) {
                    let n = self.open_in(K::ConstantPattern, s)?;
                    self.binary_expr(slot(f::NONE, Ctx::Lit), P_CONST)?;
                    self.close(n);
                    n
                } else {
                    let n = self.open_in(K::ParenthesizedPattern, s)?;
                    self.tk(K::ParenthesizedPattern, K::TLParen)?;
                    self.pattern(PATTERN)?;
                    self.punct_tk(K::ParenthesizedPattern, T::RParen, false)?;
                    self.close(n);
                    n
                }
            }
            T::Name if t.word == w::UNDERSCORE && !self.continues_expression(1) => self.leaf_in(K::Discard, s)?,
            T::Name if t.word == w::VAR && self.kind_at(1) == T::LParen => {
                let n = self.open_in(K::VarPattern, s)?;
                self.tk(K::VarPattern, K::TVar)?;
                self.designation()?;
                self.close(n);
                n
            }
            T::Name if !w::is_reserved(t.word) || w::is_predefined(t.word) => self.type_led_pattern(s)?,
            _ => self.constant_pattern(s)?,
        };
        self.depth -= 1;
        Ok(n)
    }

    /// Whether the token `k` ahead continues an expression (an operator or a postfix).
    fn continues_expression(&mut self, k: usize) -> bool {
        let t = self.kind_at(k);
        is_binary(t) || matches!(t, T::Dot | T::LParen | T::LBracket | T::Arrow | T::PlusPlus | T::MinusMinus)
    }

    /// A pattern that starts with a type: a declaration, a recursive pattern, a type pattern, or a
    /// constant (a name).
    fn type_led_pattern(&mut self, s: Slot) -> R<N> {
        let Some(e) = self.scan_type(0) else { return self.constant_pattern(s) };
        let after = self.peek_at(e);
        let designation = (after.kind == T::Name && !w::is_reserved(after.word) && !matches!(after.word, w::AND | w::OR | w::WHEN))
            || (after.kind == T::LParen && self.designation_paren_ahead(e));
        if designation {
            let n = self.open_in(K::DeclarationPattern, s)?;
            self.ty(slot(f::TYPE, Ctx::Type))?;
            self.designation()?;
            self.close(n);
            return Ok(n);
        }
        if after.kind == T::LBrace {
            let n = self.open_in(K::RecursivePattern, s)?;
            self.ty(slot(f::TYPE, Ctx::Type))?;
            self.property_clause()?;
            self.designation_opt()?;
            self.close(n);
            return Ok(n);
        }
        if after.kind == T::LParen {
            // A positional pattern (`A(1, 2)`): not read.
            return self.refuse();
        }
        if self.name_only(0, e) || self.continues_expression(e) {
            return self.constant_pattern(s);
        }
        let n = self.open_in(K::TypePattern, s)?;
        self.ty(slot(f::TYPE, Ctx::Type))?;
        self.close(n);
        Ok(n)
    }

    /// Whether `(` at `k` is a parenthesized designation: names and `_` between `,`s, `)`.
    fn designation_paren_ahead(&mut self, k: usize) -> bool {
        let Some(e) = self.matching(k) else { return false };
        let mut j = k + 1;
        while j < e {
            if !self.is_ident_at(j) {
                return false;
            }
            j += 1;
            if j < e && self.kind_at(j) != T::Comma {
                return false;
            }
            j += 1;
        }
        self.kind_at(e + 1) != T::FatArrow
    }

    /// `constant_pattern`: an expression (its literal in `literal`), with the operators after it.
    fn constant_pattern(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ConstantPattern, s)?;
        let lit = slot(f::NONE, Ctx::Lit);
        match self.cmode {
            Cmode::Plain => {
                self.binary_expr(lit, P_CONST)?;
            }
            Cmode::Start(min) => {
                let Some(has_is) = self.constant_takes() else { return self.refuse() };
                // Inside a chain, an `is` with others after it takes none of the operators.
                if has_is && min > P_CONST {
                    self.binary_expr(lit, u8::MAX)?;
                    if binary_prec(self.peek()).is_some_and(|p| p >= min) {
                        return self.refuse();
                    }
                    self.close(n);
                    return Ok(n);
                }
                let (cmode, in_constant) = (self.cmode, self.in_constant);
                (self.cmode, self.in_constant) = (Cmode::Plain, has_is);
                self.binary_expr(lit, P_CONST)?;
                (self.cmode, self.in_constant) = (cmode, in_constant);
            }
            Cmode::Inner(min) => {
                self.binary_expr(lit, u8::MAX)?;
                if binary_prec(self.peek()).is_some_and(|p| p >= min) {
                    return self.refuse();
                }
            }
        }
        self.close(n);
        Ok(n)
    }

    /// `_variable_designation`: `_`, a name, or `(` designations `)`.
    pub(super) fn designation(&mut self) -> R<()> {
        if self.is_word(w::UNDERSCORE) {
            self.leaf(K::Discard, f::NONE, C_NONE)?;
        } else if self.peek() == T::LParen {
            let n = self.open(K::ParenthesizedVariableDesignation, f::NONE, C_NONE)?;
            self.list(K::ParenthesizedVariableDesignation, T::RParen, true, false, |p| p.designation())?;
            self.close(n);
        } else {
            self.ident(slot(f::NAME, Ctx::Bare))?;
        }
        Ok(())
    }

    fn designation_opt(&mut self) -> R<()> {
        let t = self.tok();
        if t.kind == T::Name && !w::is_reserved(t.word) && !matches!(t.word, w::AND | w::OR | w::WHEN) {
            self.designation()?;
        }
        Ok(())
    }

    /// `property_pattern_clause`: `{` subpatterns `}`, a trailing `,` allowed.
    fn property_clause(&mut self) -> R<()> {
        let n = self.open(K::PropertyPatternClause, f::NONE, C_NONE)?;
        self.list(K::PropertyPatternClause, T::RBrace, true, true, |p| p.subpattern())?;
        self.close(n);
        Ok(())
    }

    /// `subpattern`: an expression and `:`, then a pattern.
    fn subpattern(&mut self) -> R<()> {
        let n = self.open(K::Subpattern, f::NONE, C_NONE)?;
        if self.subpattern_name_ahead() {
            self.unary(EXPR)?;
            self.leaf(K::TColon, f::NONE, C_NONE)?;
        }
        self.pattern(PATTERN)?;
        self.close(n);
        Ok(())
    }

    /// Whether a dotted name and `:` start a subpattern.
    fn subpattern_name_ahead(&mut self) -> bool {
        let mut k = 0;
        loop {
            if !self.is_ident_at(k) {
                return false;
            }
            k += 1;
            match self.kind_at(k) {
                T::Colon => return true,
                T::Dot => k += 1,
                _ => return false,
            }
        }
    }

    /// `list_pattern`: `[` patterns and `..` `]`, then a designation. A `..` in the list makes its
    /// gaps no implied word: every token is stored.
    fn list_pattern(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ListPattern, s)?;
        let close = self.matching(0).ok_or_else(|| Refusal::Parse(self.tok().start))?;
        let mut spread = false;
        for j in 1..close {
            if self.kind_at(j) == T::DotDot {
                // `[a, ..]`: tree-sitter reads `..]` as a range and fails; `.. var r` (a slice
                // with a pattern) is not in its grammar. Only `..,` is read.
                if self.kind_at(j + 1) != T::Comma {
                    return self.refuse();
                }
                spread = true;
            }
        }
        if spread {
            // Gap by gap: one that holds a `..` (or a trailing `,`) stores its tokens, another is
            // its kind's implied word.
            let mut first = true;
            loop {
                let mut k = usize::from(first);
                let mut end = false;
                loop {
                    match self.kind_at(k) {
                        T::Comma | T::DotDot => k += 1,
                        T::RBracket => {
                            k += 1;
                            end = true;
                            break;
                        }
                        _ => break,
                    }
                }
                let stored = (0..k).any(|j| self.kind_at(j) == T::DotDot) || (end && !first && k > 1);
                for _ in 0..k {
                    let tok = punct(self.peek()).expect("a list token");
                    self.gap_tk(K::ListPattern, tok, stored)?;
                }
                if end {
                    break;
                }
                self.pattern(PATTERN)?;
                first = false;
            }
        } else {
            self.list(K::ListPattern, T::RBracket, true, true, |p| p.pattern(PATTERN).map(|_| ()))?;
        }
        self.designation_opt()?;
        self.close(n);
        Ok(n)
    }

    /// A switch expression's arms, `{` … `}`.
    pub(super) fn switch_expression_body(&mut self) -> R<()> {
        self.list(K::SwitchExpression, T::RBrace, true, true, |p| {
            let a = p.open(K::SwitchExpressionArm, f::NONE, C_NONE)?;
            p.pattern(PATTERN)?;
            if p.is_word(w::WHEN) {
                p.when_clause()?;
            }
            if p.peek() != T::FatArrow {
                return p.refuse();
            }
            p.leaf(K::TFatArrow, f::NONE, C_NONE)?;
            p.expr(EXPR)?;
            p.close(a);
            Ok(())
        })
    }

    /// `when_clause`: `when` and an expression.
    pub(super) fn when_clause(&mut self) -> R<()> {
        let n = self.open(K::WhenClause, f::NONE, C_NONE)?;
        self.tk(K::WhenClause, K::TWhen)?;
        self.expr(EXPR)?;
        self.close(n);
        Ok(())
    }

    pub(super) fn top_comma_pub(&mut self, from: usize, to: usize) -> bool {
        let mut j = from;
        while j < to {
            match self.kind_at(j) {
                T::Comma => return true,
                T::LParen | T::LBracket | T::LBrace => match self.matching(j) {
                    Some(e) => j = e,
                    None => return false,
                },
                _ => {}
            }
            j += 1;
        }
        false
    }
}
