//! Patterns: `is`, `case` labels, switch expression arms, and the designations they bind.

use super::expr::{P_COND, P_CONST, binary_prec, is_binary};
use super::*;

impl<'a> P<'a> {
    /// `left is …`: an `is_expression` (a type) or an `is_pattern_expression`, as tree-sitter
    /// reads them:
    /// - a type that is no expression (`int`, `T[]`, `A<B>`) alone is an `is_expression`;
    /// - a name before a binary operator is one too inside a constant that already absorbs
    ///   (`Cmode`), or with a left operand where `constant_alone` reads the constant alone;
    /// - every other is a pattern. At a chain's start (`Cmode::Start`) its constant takes the
    ///   operators after it (`x is null || y` is `x is (null || y)`) when `constant_takes` agrees:
    ///   with no other `is` in the range, or with a plain constant ending it; with a left operand,
    ///   unless `constant_alone` reads it alone;
    /// - shapes whose reading depends on more than this (a dotted name read alone, a conditional
    ///   access that wrapped the chain, a range ending at `?`) refuse.
    pub(super) fn is_tail(&mut self, left: N, s: Slot, min: u8) -> R<N> {
        let start = !self.in_constant;
        // `x is A<T> && y`, `x is not A<T> - 1`: an operator after the type that may also start an
        // operand lets tree-sitter read comparisons (`(x is A) < T > &&y`); not read.
        let k = if self.is_word_at(1, w::NOT) { 2 } else { 1 };
        if let Some(e) = self.scan_type(k)
            && self.kind_at(e - 1) == T::Gt
            && matches!(self.kind_at(e), T::Amp | T::AmpAmp | T::Star | T::Minus | T::Plus | T::Bang | T::Tilde | T::PlusPlus | T::MinusMinus)
        {
            return self.refuse();
        }
        let is_expression = match self.is_type_ahead() {
            Some((false, _)) => true,
            // A name alone before an operator, where the `is` does not start its chain (or stands
            // in a constant): an `is_expression`.
            // A dotted name there: tree-sitter's readings vary; not read.
            Some((true, after)) if is_binary(after) && self.in_constant => true,
            // After a conditional access wrapped the chain before it (`a || b?.c != d || e is A ||`),
            // not read.
            Some((true, after)) if is_binary(after) && self.wrapped_chain => return self.refuse(),
            // With a left operand, as `constant_alone` reads it; a dotted name read alone is a
            // member access over the `is` (`(b is X).Y`): not read.
            Some((true, after)) if is_binary(after) && min > P_CONST => match self.constant_alone(1, min) {
                Some(true) if self.kind_at(2) == T::Dot => return self.refuse(),
                Some(alone) => alone,
                None => return self.refuse(),
            },
            // A dotted name before a conditional's `?` is a member access over the `is` too.
            Some((true, T::Question)) if self.kind_at(2) == T::Dot => return self.refuse(),
            Some((true, _)) => false,
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
        let mut e = self.scan_type(1)?;
        // A `?` before no designation is a conditional's (`x is A ? b : c`), not a nullable's.
        if self.kind_at(e - 1) == T::Question && !self.is_ident_at(e) {
            e -= 1;
        }
        let after = self.peek_at(e);
        let continues = (after.kind == T::Name && !matches!(after.word, w::AS | w::IS | w::SWITCH | w::WITH)) || matches!(after.kind, T::LBrace | T::LParen);
        if continues {
            return None;
        }
        Some((self.name_only(1, e), after.kind))
    }

    /// The operators after a constant (from `from` tokens ahead) up to its chain's end: the end's
    /// index, the last `is` among them, their count, and where the last operand starts (after the
    /// last binary operator or `is`). `None` where tree-sitter's choice is not read (a conditional
    /// with operators before its `?`, `or`/`and` patterns with another `is`, the text's end).
    fn chain_scan(&mut self, from: usize) -> Option<Chain> {
        let (mut k, mut depth) = (from, 0u32);
        let mut c = Chain { end: 0, last_is: None, others: 0, last: from, low: u8::MAX };
        let (mut ops, mut combinator) = (0u32, false);
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
                T::Name if matches!(t.word, w::OR | w::AND) => combinator = true,
                T::Name if t.word == w::IS => {
                    c.last_is = Some(k);
                    c.others += 1;
                    c.last = k + 1;
                }
                k2 if is_binary(k2) => {
                    ops += 1;
                    c.last = k + 1;
                    c.low = c.low.min(binary_prec(k2).expect("a binary operator"));
                }
                k2 if super::expr::assignment(k2).is_some() => break,
                _ => {}
            }
            k += 1;
        }
        c.end = k;
        // `or`/`and` patterns with another `is` in the chain (`y is A && b is C or D && …`):
        // tree-sitter's grouping there is not read.
        if combinator && c.others > 0 {
            return None;
        }
        Some(c)
    }

    /// After an `is` that starts its chain, whether its constant (from the next token) takes the
    /// operators after it as tree-sitter does: `Some(false)` where no other `is` stands in them,
    /// `Some(true)` where the last `is` among them has a plain constant that ends them, `None`
    /// otherwise (tree-sitter's choice there is not read).
    fn constant_takes(&mut self) -> Option<bool> {
        let c = self.chain_scan(0)?;
        // A comment among the operators moves where tree-sitter's readings meet: not read.
        if c.low < u8::MAX && self.extras_between(0, c.end) {
            return None;
        }
        let Some(j) = c.last_is else { return Some(false) };
        // A dotted name there is a member access over its `is` (`(b is S).O`): not read.
        let dotted = (j + 1..c.end).any(|k| self.kind_at(k) == T::Dot);
        (self.plain_is_ends(&c, j) && !dotted).then_some(true)
    }

    /// Whether the `is` at `j`, the last in a chain (`c`), has a plain constant that ends it (a
    /// literal, `null`, a dotted name, `not` or `-` before one). `false` in a longer chain too:
    /// tree-sitter's readings vary there (its parse versions are bounded).
    fn plain_is_ends(&mut self, c: &Chain, j: usize) -> bool {
        if c.others > 3 {
            return false;
        }
        let mut k = j + 1;
        if self.is_word_at(k, w::NOT) {
            k += 1;
        }
        if self.kind_at(k) == T::Minus {
            k += 1;
        }
        let t = self.peek_at(k);
        let plain = match t.kind {
            T::Int | T::Real | T::Char | T::Str => k + 1,
            T::Name if matches!(t.word, w::NULL | w::TRUE | w::FALSE) => k + 1,
            T::Name if !w::is_reserved(t.word) => match self.scan_type(k) {
                Some(e) if self.name_only(k, e) => e,
                _ => return false,
            },
            _ => return false,
        };
        plain == c.end
    }

    /// Whether a comment or a directive stands between tokens `from` and `to` (ahead).
    fn extras_between(&mut self, from: usize, to: usize) -> bool {
        (from..to).any(|k| {
            let (a, b) = (self.peek_at(k).end, self.peek_at(k + 1).start);
            self.src[a as usize..b as usize].iter().any(|&c| c == b'/' || c == b'#')
        })
    }

    /// After an `is` whose chain has a left operand (`a || b is X || …`), whether tree-sitter
    /// reads its constant alone (`(a || b is X) || …`) rather than taking the operators after it
    /// (`a || b is (X || …)`), from `from` tokens ahead (the operand before the `is` of
    /// precedence `min`). Its two parse versions meet where the chain's last operand reduces, and
    /// tree-sitter keeps the tree that compares smaller, symbol by symbol:
    /// - operators that all bind tighter than `min` are the constant's;
    /// - else the alone reading needs a last operand that is a literal (`1`, `"s"`, `null`,
    ///   `-1`), `default`, `sizeof(…)`, `typeof(…)`, or an `is` with a plain constant (a name, a
    ///   call, an access or any other expression lets the constant take the operators), and a
    ///   left operand whose leftmost operand is no call, `!a`, `(…)`, `new A()`, `await` or `a++`.
    ///
    /// `None` where neither is read (an `is` with any other pattern last, a comment in the chain).
    fn constant_alone(&mut self, from: usize, min: u8) -> Option<bool> {
        // Operators that all bind tighter than the one before the `is` (`p && b is X == 1`): the
        // constant takes them.
        if self.chain_scan(from)?.low >= min {
            return Some(false);
        }
        let alone = self.alone_by_last(from)?;
        if !alone {
            return Some(false);
        }
        // A comment in the chain is a node in both readings, and moves where they meet: not read.
        let end = self.chain_scan(from)?.end;
        if self.extras_between(from, end) {
            return None;
        }
        // Where both readings meet, tree-sitter keeps the one whose left operand's leftmost
        // operand has the smaller symbol through its supertypes: a `non_lvalue_expression`'s
        // `binary_expression` (the alone reading's) beats an `lvalue_expression` (a name, an
        // access), and loses to an `_expression_statement_expression` (a call, `!a`, `(…)`,
        // `new A()`, `await`, `a++`).
        let Some(mut at) = self.chain_left else { return Some(true) };
        loop {
            let (kind, _, class, _) = unkey(self.w.key(at));
            let k = kind_of(kind);
            if k == K::BinaryExpression {
                at += 1;
                continue;
            }
            if matches!(class, C_EL | C_L) {
                // `(a)` and `*p` are lvalues' aliases whose symbols are not read.
                return (!matches!(k, K::ParenthesizedExpression | K::PrefixUnaryExpression)).then_some(true);
            }
            return Some(!matches!(
                k,
                K::InvocationExpression
                    | K::PrefixUnaryExpression
                    | K::PostfixUnaryExpression
                    | K::AwaitExpression
                    | K::ObjectCreationExpression
                    | K::ParenthesizedExpression
                    | K::AssignmentExpression
            ));
        }
    }

    /// `constant_alone`'s reading by the chain's last operand alone.
    fn alone_by_last(&mut self, from: usize) -> Option<bool> {
        let c = self.chain_scan(from)?;
        if let Some(j) = c.last_is
            && j + 1 == c.last
        {
            // A dotted name there is a member access over its `is` (`(c is T).U`): not read.
            let dotted = (j + 1..c.end).any(|k| self.kind_at(k) == T::Dot);
            return (self.plain_is_ends(&c, j) && !dotted).then_some(true);
        }
        let mut k = c.last;
        while matches!(self.kind_at(k), T::Minus | T::Plus | T::Bang | T::Tilde) {
            k += 1;
        }
        let t = self.peek_at(k);
        Some(match t.kind {
            T::Int | T::Real | T::Char | T::Str | T::Verbatim | T::Raw | T::Interp => k + 1 == c.end,
            T::Name if matches!(t.word, w::NULL | w::TRUE | w::FALSE) => k + 1 == c.end,
            T::Name if t.word == w::DEFAULT && k + 1 == c.end => true,
            T::Name if matches!(t.word, w::DEFAULT | w::SIZEOF | w::TYPEOF) && self.kind_at(k + 1) == T::LParen => {
                self.matching(k + 1).is_some_and(|e| e + 1 == c.end)
            }
            _ => false,
        })
    }

    /// Whether tokens `from..to` are a dotted name alone (after an alias's `A::`), type arguments
    /// allowed (no suffix): a constant pattern's, to tree-sitter.
    pub(super) fn name_only(&mut self, from: usize, to: usize) -> bool {
        let mut k = from;
        if self.is_ident_at(k) && self.kind_at(k + 1) == T::ColonColon {
            k += 2;
        }
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
                // `negated_pattern` is `not` and a whole pattern: `not 1 and 2` is `not (1 and 2)`.
                let n = self.open_in(K::NegatedPattern, s)?;
                self.tk(K::NegatedPattern, K::TNot)?;
                self.pattern(PATTERN)?;
                self.close(n);
                n
            }
            T::Lt | T::Le | T::Gt | T::Ge => {
                // An `is` in its operand (`>= a || o is A or B`): tree-sitter's grouping of the
                // `or` after it is not read.
                if self.is_in_operand(1) {
                    return self.refuse();
                }
                let n = self.open_in(K::RelationalPattern, s)?;
                self.leaf(punct(t.kind).expect("an operator"), f::NONE, C_NONE)?;
                // Its operand is a whole expression to tree-sitter, a conditional's included
                // (`x is > 0 ? a : b` is `x is > (0 ? a : b)`).
                self.binary_expr(EXPR, P_COND)?;
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
                if !self.paren_is_pattern(0, close) {
                    // `("a")`, `(1)`, `(a + 1)`: a constant's parenthesized expression.
                    self.constant_pattern(s)?
                } else if self.top_comma_pub(1, close) && self.positional_ahead(close) {
                    let n = self.open_in(K::RecursivePattern, s)?;
                    let c = self.open(K::PositionalPatternClause, f::NONE, C_NONE)?;
                    self.list(K::PositionalPatternClause, T::RParen, true, false, |p| p.subpattern())?;
                    self.close(c);
                    if self.peek() == T::LBrace {
                        self.property_clause()?;
                    }
                    self.designation_opt()?;
                    self.close(n);
                    n
                } else if self.top_comma_pub(1, close) {
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
            T::Name if self.discard_at(0) => self.leaf_in(K::Discard, s)?,
            // `x is var (a, b)` after `is`: an invocation over `x is var` to tree-sitter; not read.
            T::Name if t.word == w::VAR && self.kind_at(1) == T::LParen && self.cmode != Cmode::Plain => return self.refuse(),
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

    /// Whether `(` … `)` next (its `)` at `close`), with `,`s in it, is a `positional_pattern_clause`
    /// to tree-sitter rather than a constant's tuple: its first element decides (`(_, 1)` is
    /// positional, `(1, _)` a tuple whose `_` is a name), unless an element after it can only be a
    /// pattern's (`(1, > 2)`), or a designation or a property clause follows (`(1, 2) y`).
    fn positional_ahead(&mut self, close: usize) -> bool {
        let after = self.peek_at(close + 1);
        if after.kind == T::LBrace || (after.kind == T::Name && !w::is_reserved(after.word) && !matches!(after.word, w::AND | w::OR | w::WHEN)) {
            return true;
        }
        // The first element: `_`, `var`, `not`, a relational operator, `{`, a declaration
        // (`A a`), or a parenthesized pattern.
        let t = self.peek_at(1);
        let first = match t.kind {
            T::Lt | T::Le | T::Gt | T::Ge | T::LBrace => true,
            T::Name if t.word == w::UNDERSCORE => matches!(self.kind_at(2), T::Comma),
            T::Name if matches!(t.word, w::VAR | w::NOT) => true,
            T::Name => self.scan_type(1).is_some_and(|e| self.is_ident_at(e) && self.kind_at(e + 1) == T::Comma),
            T::LParen => self.matching(1).is_some_and(|c| self.paren_is_pattern(1, c)),
            _ => false,
        };
        if first {
            return true;
        }
        // A later element only a pattern may start.
        let mut j = 1;
        while j < close {
            match self.kind_at(j) {
                T::LParen | T::LBracket | T::LBrace => match self.matching(j) {
                    Some(e) => j = e,
                    None => return false,
                },
                T::Comma if matches!(self.kind_at(j + 1), T::Lt | T::Le | T::Gt | T::Ge | T::LBrace) => return true,
                _ => {}
            }
            j += 1;
        }
        false
    }

    /// Whether `(` at `k` (its `)` at `close`) opens a `parenthesized_pattern` to tree-sitter, not a
    /// constant pattern's `parenthesized_expression`: a pattern-only start (`not`, a relational
    /// operator, `{`, `var`, `_`, a `(` that is one itself), or a name chain (`A`, `A<B>.C`,
    /// `this.M()`: an invocation after a `.` only) before `)`, `or`, `and`, a designation or `{`.
    /// Anything else (a literal, an operator, `M()`, `[…]`) is an expression's.
    fn paren_is_pattern(&mut self, k: usize, close: usize) -> bool {
        // A combinator at the top level: a pattern's (`(1 or 2)`).
        let mut depth = 0usize;
        for j in k + 1..close {
            let t = self.peek_at(j);
            match t.kind {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => depth = depth.saturating_sub(1),
                T::Name if depth == 0 && matches!(t.word, w::OR | w::AND) => return true,
                _ => {}
            }
        }
        let t = self.peek_at(k + 1);
        match t.kind {
            T::Lt | T::Le | T::Gt | T::Ge | T::LBrace => return true,
            T::LParen => return self.matching(k + 1).is_some_and(|c| self.paren_is_pattern(k + 1, c)),
            T::Name if matches!(t.word, w::NOT | w::VAR | w::UNDERSCORE) => return true,
            T::Name => {}
            _ => return false,
        }
        let mut j = k + 1;
        let mut dotted = false;
        if self.is_word_at(j, w::GLOBAL) && self.kind_at(j + 1) == T::ColonColon {
            j += 2;
        }
        loop {
            let n = self.peek_at(j);
            let head = j == k + 1 && n.kind == T::Name && (matches!(n.word, w::THIS | w::BASE) || w::is_predefined(n.word));
            if !head && !self.is_ident_at(j) {
                return false;
            }
            j += 1;
            if self.kind_at(j) == T::Lt {
                match self.scan_type_args(j) {
                    Some(e) => j = e,
                    None => return false,
                }
            }
            while dotted && self.kind_at(j) == T::LParen {
                match self.matching(j) {
                    Some(e) => j = e + 1,
                    None => return false,
                }
            }
            if self.kind_at(j) != T::Dot {
                break;
            }
            dotted = true;
            j += 1;
        }
        let after = self.peek_at(j);
        j == close || after.kind == T::LBrace || (after.kind == T::Name && (matches!(after.word, w::OR | w::AND) || !w::is_reserved(after.word)))
    }

    /// Whether the token `k` ahead continues an expression (an operator or a postfix).
    fn continues_expression(&mut self, k: usize) -> bool {
        let t = self.kind_at(k);
        is_binary(t) || matches!(t, T::Dot | T::LParen | T::LBracket | T::Arrow | T::PlusPlus | T::MinusMinus)
    }

    /// A pattern that starts with a type: a declaration, a recursive pattern, a type pattern, or a
    /// constant (a name).
    fn type_led_pattern(&mut self, s: Slot) -> R<N> {
        let Some(mut e) = self.scan_type(0) else { return self.constant_pattern(s) };
        // A `?` before no designation is a conditional's (`x is A ? b : c`).
        if self.kind_at(e - 1) == T::Question && !self.is_ident_at(e) {
            e -= 1;
        }
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
            // `A(1, 2)`, `A.M()`: a constant's invocation to tree-sitter, not a positional pattern.
            if self.name_only(0, e) {
                return self.constant_pattern(s);
            }
            return self.refuse();
        }
        if self.name_only(0, e) || self.continues_expression(e) {
            return self.constant_pattern(s);
        }
        let n = self.open_in(K::TypePattern, s)?;
        self.ty_in_expr(slot(f::TYPE, Ctx::Type))?;
        self.close(n);
        Ok(n)
    }

    /// Whether `(` at `k` is a parenthesized designation: names and `_` between `,`s, `)`.
    fn designation_paren_ahead(&mut self, k: usize) -> bool {
        let Some(e) = self.matching(k) else { return false };
        // `A(b)` after `is`: one name is a constant's invocation to tree-sitter (`x is nameof(T)`);
        // in a `case` label, a declaration's.
        if e == k + 1 || (e == k + 2 && self.cmode != Cmode::Plain) {
            return false;
        }
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
                // With a left operand, as `constant_alone` reads it.
                let (alone, has_is) = if min > P_CONST {
                    let Some(alone) = self.constant_alone(0, min) else { return self.refuse() };
                    let Some(c) = self.chain_scan(0) else { return self.refuse() };
                    (alone, c.others > 0)
                } else {
                    let Some(has_is) = self.constant_takes() else { return self.refuse() };
                    (false, has_is)
                };
                if alone {
                    self.binary_expr(lit, u8::MAX)?;
                    if binary_prec(self.peek()).is_some_and(|p| p >= min) {
                        return self.refuse();
                    }
                    self.constant_invocation(n);
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
        self.constant_invocation(n);
        self.close(n);
        Ok(n)
    }

    /// A constant's own invocation (`x is A.M()`) is `_complex_invocation_expression`'s or
    /// `_name_invocation_pattern`'s to tree-sitter: its `function` is reached through no
    /// `expression` supertype, so it has no class.
    fn constant_invocation(&mut self, n: N) {
        let (call, function) = (n.at + 1, n.at + 2);
        if call >= self.w.len() || function >= self.w.len() {
            return;
        }
        let (kind, _, _, extra) = crate::unkey(self.w.key(call));
        if extra || kind != K::InvocationExpression as u16 {
            return;
        }
        let key = self.w.key(function);
        if !crate::unkey(key).3 {
            self.w.set_key(function, key & !(0xf << 15));
        }
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

    /// Whether an `is` stands at the top level from `k` tokens ahead to the expression's end.
    fn is_in_operand(&mut self, mut k: usize) -> bool {
        let mut depth = 0u32;
        loop {
            match self.kind_at(k) {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace if depth > 0 => depth -= 1,
                T::RParen | T::RBracket | T::RBrace | T::Semi | T::Comma | T::Colon | T::FatArrow | T::Eof if depth == 0 => return false,
                T::Name if depth == 0 && self.is_word_at(k, w::IS) => return true,
                _ => {}
            }
            k += 1;
        }
    }

    /// Whether a `discard` stands `k` tokens ahead: `_` that no expression continues.
    fn discard_at(&mut self, k: usize) -> bool {
        self.is_word_at(k, w::UNDERSCORE) && !self.continues_expression(k + 1)
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
                // with a pattern) is not in its grammar. Only `..,` is read, and not first
                // (`[.., var w]` is a constant's element binding to tree-sitter).
                if self.kind_at(j + 1) != T::Comma || j == 1 {
                    return self.refuse();
                }
                spread = true;
            }
            // `, _`: a discard after a `,` stores it.
            spread |= self.kind_at(j) == T::Comma && self.discard_at(j + 1);
        }
        if spread {
            // Gap by gap: one that holds a `..` (or a trailing `,`, or a `,` before a discard)
            // stores its tokens, another is its kind's implied word.
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
                let stored = (0..k).any(|j| self.kind_at(j) == T::DotDot) || (end && !first && k > 1) || (!first && !end && self.discard_at(k));
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
            let saved = p.arm_arrow;
            p.arm_arrow = p.arm_arrow_ahead();
            let r = p.pattern(PATTERN);
            p.arm_arrow = saved;
            r?;
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

    /// The token index of the arm's `=>`: the first at the top level (a `when` clause's lambdas
    /// stand in brackets), or none before the arm's `,` or `}`.
    fn arm_arrow_ahead(&mut self) -> usize {
        let mut depth = 0usize;
        let mut k = 0;
        loop {
            match self.kind_at(k) {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace if depth > 0 => depth -= 1,
                T::FatArrow if depth == 0 => return self.i + k,
                T::Comma | T::RBrace if depth == 0 => return usize::MAX,
                T::Eof => return usize::MAX,
                _ => {}
            }
            k += 1;
        }
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
