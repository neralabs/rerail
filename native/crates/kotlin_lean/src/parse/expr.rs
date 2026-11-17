//! Expressions: precedence climbing over tree-sitter-kotlin's `PREC` table, each operator's node
//! put over its left operand at the operand's mark (`Tw::wrap`), as the GLR parse reduces it.

use super::{P, ident_word, punct, soft};
use crate::kinds::{K, f};
use crate::lex::{R, T, w};
use crate::tw::Mark;

/// Binding powers (tree-sitter-kotlin's `PREC`).
pub(super) mod prec {
    pub const PREFIX: u8 = 15;
    pub const AS: u8 = 13;
    pub const MUL: u8 = 12;
    pub const ADD: u8 = 11;
    pub const RANGE: u8 = 10;
    pub const INFIX: u8 = 9;
    pub const ELVIS: u8 = 8;
    pub const CHECK: u8 = 7;
    pub const COMPARISON: u8 = 6;
    pub const EQUALITY: u8 = 5;
    pub const CONJ: u8 = 4;
    pub const DISJ: u8 = 3;
    pub const SPREAD: u8 = 2;
}

/// The words that start an expression where they are keywords.
fn expression_word(code: u8) -> bool {
    matches!(code, w::IF | w::WHEN | w::TRY | w::THIS | w::SUPER | w::NULL | w::TRUE | w::FALSE | w::OBJECT | w::FUN | w::RETURN | w::THROW | w::BREAK | w::CONTINUE)
}

impl<'a> P<'a> {
    /// An `_expression`: its node's place.
    pub(super) fn expression(&mut self) -> R<u32> {
        self.expr(0)
    }

    /// An expression whose operators all bind tighter than `min`.
    pub(super) fn expr(&mut self, min: u8) -> R<u32> {
        self.deeper()?;
        let kw = if self.i == self.kw_at {
            self.kw_at = usize::MAX;
            Some(std::mem::replace(&mut self.kw_depth, self.depth))
        } else {
            None
        };
        let m = self.mark();
        let at = self.t.mark_child(m);
        let r = self.expr_in(m, min);
        if let Some(k) = kw {
            self.kw_depth = k;
        }
        self.depth -= 1;
        r?;
        self.t.unmark(m);
        Ok(at)
    }

    /// Whether the next token can start an expression.
    pub(super) fn starts_expression(&mut self) -> bool {
        let t = self.tok();
        match t.kind {
            T::Name => t.tick || ident_word(t.word) || expression_word(t.word),
            T::Int | T::Hex | T::Bin | T::Real | T::Char | T::Str | T::LParen | T::LBracket | T::LBrace | T::ColonColon | T::At => true,
            T::PlusPlus | T::MinusMinus | T::Minus | T::Plus | T::Bang | T::Star => true,
            _ => false,
        }
    }

    /// Whether the statement ends before the next token: it is on a new line (or a `;`) and the
    /// scanner's semicolon comes first.
    #[inline]
    pub(super) fn ended(&mut self) -> bool {
        let t = self.tok();
        (t.nl || t.kind == T::Semi || t.kind == T::Eof) && self.ends()
    }

    fn expr_in(&mut self, m: Mark, min: u8) -> R<()> {
        self.unary(m, min)?;
        loop {
            let t = self.tok();
            if t.nl && self.ends() {
                return Ok(());
            }
            // A generic call or navigation over what is read since `m` (an operand's, deferred to
            // here: see `postfix`).
            if t.kind == T::Lt && min <= prec::COMPARISON && self.generic_suffix(m)? {
                self.postfix(m, usize::MAX, true)?;
                continue;
            }
            let (kind, p) = match t.kind {
                T::Star | T::Slash | T::Percent => (K::MultiplicativeExpression, prec::MUL),
                T::Plus | T::Minus => (K::AdditiveExpression, prec::ADD),
                T::DotDot | T::DotDotLt => (K::RangeExpression, prec::RANGE),
                T::Elvis => (K::ElvisExpression, prec::ELVIS),
                T::Lt | T::Gt | T::Le | T::Ge => (K::ComparisonExpression, prec::COMPARISON),
                T::EqEq | T::EqEqEq | T::Ne | T::NeEq => (K::EqualityExpression, prec::EQUALITY),
                T::AmpAmp => (K::ConjunctionExpression, prec::CONJ),
                T::PipePipe => (K::DisjunctionExpression, prec::DISJ),
                T::Bang if self.is_word_at(1, w::IN) || self.is_word_at(1, w::IS) => (K::CheckExpression, prec::CHECK),
                T::Name if !t.tick && (t.word == w::IN || t.word == w::IS) => (K::CheckExpression, prec::CHECK),
                T::Name if !t.tick && t.word == w::AS => (K::AsExpression, prec::AS),
                T::Name if t.tick || ident_word(t.word) => (K::InfixExpression, prec::INFIX),
                // A keyword as an infix name where keywords are names (not `in`, `is`, `as`).
                T::Name if self.kw_name(t) => (K::InfixExpression, prec::INFIX),
                _ => return Ok(()),
            };
            if p <= min {
                return Ok(());
            }
            let n = self.t.wrap(m, kind, f::NONE);
            match kind {
                K::CheckExpression => {
                    if self.peek() == T::Bang {
                        self.tk(K::TBang)?;
                    }
                    if self.is_word(w::IN) {
                        self.tk(K::TIn)?;
                        self.expr(p)?;
                    } else {
                        self.tk(K::TIs)?;
                        if self.split_type(m, n)? {
                            continue;
                        }
                        self.ty()?;
                    }
                }
                K::AsExpression => {
                    self.as_operator()?;
                    if self.split_type(m, n)? {
                        continue;
                    }
                    self.ty()?;
                }
                K::InfixExpression => {
                    if self.kw_name(t) {
                        self.leaf(K::SimpleIdentifier, f::NONE)?;
                    } else {
                        self.simple_identifier()?;
                    }
                    self.expr(p)?;
                }
                _ => {
                    let k = punct(t.kind).expect("an operator");
                    self.tk(k)?;
                    self.expr(p)?;
                }
            }
            self.close(n);
        }
    }

    /// After `as`, `as?` or `is` (node `n` open since `m`): its type, where tree-sitter splits it
    /// (`as_split`): `x as a`, the rest navigations over it (closing `n`), then a call's type
    /// arguments where `(` or `{` starts the next line; else the comparisons are the operator
    /// loop's, over all the operation at their precedence. Whether it split.
    fn split_type(&mut self, m: Mark, n: crate::tw::Node) -> R<bool> {
        let Some((split, j)) = self.as_split(0) else { return Ok(false) };
        let at = self.i + j;
        if let Split::Compare = split {
            if !self.split_operand_reads(j) {
                return Ok(false);
            }
        }
        let stop = std::mem::replace(&mut self.type_stop, self.i + 1);
        let r = self.ty();
        self.type_stop = stop;
        r?;
        self.close(n);
        while self.peek() == T::Dot {
            self.navigation(m)?;
        }
        if let Split::Compare = split {
            // The operand's keywords are names when the loop reads it.
            self.kw_at = at;
        }
        if let Split::Call = split {
            let c = self.t.wrap(m, K::CallExpression, f::NONE);
            let s = self.open(K::CallSuffix);
            self.type_arguments()?;
            if self.peek() == T::LParen {
                self.value_arguments()?;
                if !self.no_lambda && self.lambda_ahead() {
                    self.annotated_lambda()?;
                }
            } else {
                self.annotated_lambda()?;
            }
            self.close(s);
            self.close(c);
            self.postfix(m, usize::MAX, false)?;
        }
        Ok(true)
    }

    /// After `as` (`k` tokens ahead): whether tree-sitter splits the type `a.b.C<T>` (one name or
    /// path as its argument, `>` at the line's end; probed). A declaration on the next line, a
    /// token no expression starts with, an argument of more than names, or a name alone keep the
    /// type.
    fn as_split(&mut self, k: usize) -> Option<(Split, usize)> {
        let mut j = k;
        if !self.is_ident_at(j) {
            return None;
        }
        let mut dots = 0;
        while self.kind_at(j + 1) == T::Dot && self.is_ident_at(j + 2) && !self.peek_at(j + 1).nl {
            j += 2;
            dots += 1;
        }
        if dots == 0 || self.kind_at(j + 1) != T::Lt || !self.is_ident_at(j + 2) {
            return None;
        }
        j += 2;
        while self.kind_at(j + 1) == T::Dot && self.is_ident_at(j + 2) {
            j += 2;
        }
        if self.kind_at(j + 1) != T::Gt {
            return None;
        }
        let nx = self.peek_at(j + 2);
        if !nx.nl {
            return None;
        }
        match nx.kind {
            T::LParen | T::LBrace => Some((Split::Call, j + 2)),
            T::Name | T::Int | T::Hex | T::Bin | T::Real | T::Char | T::Str | T::Minus | T::Plus | T::Bang | T::PlusPlus | T::MinusMinus | T::At => Some((Split::Compare, j + 2)),
            _ => None,
        }
    }

    /// Whether the next line, from `k` tokens ahead, reads as the split's comparison operand to
    /// the statement's end (tree-sitter's comparison reading survives only then): a trial read,
    /// rewound. In it, as tree-sitter lexes it where only a name is valid, a keyword no expression
    /// starts with is a name (`for (…) {}` a call, `class D` an infix that fails).
    fn split_operand_reads(&mut self, k: usize) -> bool {
        let saved = (self.t.save(), self.i, self.depth, self.far, self.stmt, self.cx, self.asi_memo, self.no_lambda, self.type_stop, self.kw_depth);
        self.i += k;
        self.kw_at = self.i;
        let r = self.expr(prec::COMPARISON);
        self.kw_at = usize::MAX;
        let ok = r.is_ok() && {
            let t = self.tok();
            t.nl || matches!(t.kind, T::Semi | T::RBrace | T::Eof)
        };
        let (t, i, depth, far, stmt, cx, asi, nl, ts, kw) = saved;
        self.t.rewind(t);
        (self.i, self.depth, self.far, self.stmt, self.cx, self.asi_memo, self.no_lambda, self.type_stop, self.kw_depth) = (i, depth, far, stmt, cx, asi, nl, ts, kw);
        ok
    }

    /// Whether a keyword is read as a name here (`kw_depth`): one no expression starts with.
    #[inline]
    fn kw_name(&self, t: crate::lex::Tok) -> bool {
        self.depth == self.kw_depth
            && t.kind == T::Name
            && !t.tick
            && t.word != 0
            && t.word < w::SOFT
            && !ident_word(t.word)
            && !matches!(t.word, w::TRUE | w::FALSE | w::NULL | w::THIS | w::SUPER | w::IF | w::WHEN | w::TRY | w::RETURN | w::THROW | w::BREAK | w::CONTINUE | w::OBJECT | w::FUN)
    }

    /// `as` or `as?` (one token where the `?` touches it).
    fn as_operator(&mut self) -> R<()> {
        let a = self.tok();
        let q = self.peek_at(1);
        if q.kind == T::Question && q.start == a.end {
            self.bump()?;
            self.bump()?;
            self.t.token(K::TAsQuestion, f::NONE, a.start, q.end);
            return Ok(());
        }
        if matches!(q.kind, T::QuestionDot | T::Elvis) && q.start == a.end {
            // `as?` lexed with what follows it: the `?` is the operator's.
            self.split_at(1, T::Question, 1);
            self.bump()?;
            self.bump()?;
            self.t.token(K::TAsQuestion, f::NONE, a.start, a.end + 1);
            return Ok(());
        }
        self.tk(K::TAs).map(|_| ())
    }

    /// A prefix operator, an annotation or label before an expression, a spread, or a primary
    /// expression and its suffixes.
    fn unary(&mut self, m: Mark, min: u8) -> R<()> {
        let t = self.tok();
        match t.kind {
            T::PlusPlus | T::MinusMinus | T::Minus | T::Plus | T::Bang => {
                let n = self.open(K::PrefixExpression);
                self.tk(punct(t.kind).expect("an operator"))?;
                self.expr(prec::PREFIX)?;
                self.close(n);
                Ok(())
            }
            T::At => {
                let n = self.open(K::PrefixExpression);
                self.annotation_node()?;
                self.expr(0)?;
                self.close(n);
                Ok(())
            }
            T::Name if self.label_ahead() => {
                let n = self.open(K::PrefixExpression);
                self.label()?;
                self.expr(0)?;
                self.close(n);
                Ok(())
            }
            T::Star => {
                let n = self.open(K::SpreadExpression);
                self.tk(K::TStar)?;
                self.expr(prec::SPREAD)?;
                self.close(n);
                Ok(())
            }
            _ => {
                self.primary()?;
                self.postfix(m, usize::MAX, min <= prec::COMPARISON)
            }
        }
    }

    /// The index after the `>` closing the type arguments at token `k` ahead, where what is between
    /// reads as types.
    pub(super) fn type_args_end(&mut self, k: usize) -> Option<usize> {
        let (mut angle, mut paren) = (0u32, 0u32);
        let mut j = k;
        loop {
            let t = self.peek_at(j);
            match t.kind {
                T::Lt => angle += 1,
                T::Gt => {
                    angle -= 1;
                    if angle == 0 {
                        return Some(j + 1);
                    }
                }
                T::LParen => paren += 1,
                T::RParen => {
                    if paren == 0 {
                        return None;
                    }
                    paren -= 1;
                }
                T::Name => {
                    if !t.tick && !ident_word(t.word) && !matches!(t.word, w::IN | w::OUT) {
                        return None;
                    }
                }
                T::Comma | T::Dot | T::Question | T::Star | T::At | T::Colon | T::Arrow | T::Amp | T::LBracket | T::RBracket => {}
                _ => return None,
            }
            j += 1;
            if j - k > 512 {
                return None;
            }
        }
    }

    /// A call or navigation with type arguments (`<` next) over what is read since `m`, where the
    /// type arguments close before what such a suffix continues with: whether it was read.
    fn generic_suffix(&mut self, m: Mark) -> R<bool> {
        let Some(e) = self.type_args_end(0) else { return Ok(false) };
        match self.kind_at(e) {
            T::LParen => self.call(m, true)?,
            T::LBrace if !self.no_lambda => self.call(m, true)?,
            T::Dot | T::QuestionDot | T::ColonColon => self.navigation(m)?,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The suffixes after a primary expression (calls, navigations, indexes, postfix operators),
    /// each node put over what is read since `m`; none from token `limit` on. A generic call's
    /// (`f<T>()`) only where `generic`: in an operand of an operator that binds tighter than a
    /// comparison, tree-sitter's GLR parse keeps the reading where `<` is a comparison's, the operand
    /// reduced first, and the type arguments end up over the whole operation
    /// (`a ?: f<T>()` is `call(elvis(a, f), <T>())`), its symbol the lower.
    pub(super) fn postfix(&mut self, m: Mark, limit: usize, generic: bool) -> R<()> {
        loop {
            if self.i >= limit {
                return Ok(());
            }
            let t = self.tok();
            if t.nl && self.ends() {
                return Ok(());
            }
            match t.kind {
                T::LParen => self.call(m, false)?,
                T::LBrace if !self.no_lambda => self.call(m, false)?,
                T::Name if !self.no_lambda && self.label_ahead() && self.kind_at(2) == T::LBrace => self.call(m, false)?,
                T::Lt if generic => {
                    if !self.generic_suffix(m)? {
                        return Ok(());
                    }
                }
                T::LBracket => {
                    let n = self.t.wrap(m, K::IndexingExpression, f::NONE);
                    self.indexing_suffix()?;
                    self.close(n);
                }
                T::Dot | T::QuestionDot | T::ColonColon => self.navigation(m)?,
                T::PlusPlus | T::MinusMinus | T::BangBang => {
                    let n = self.t.wrap(m, K::PostfixExpression, f::NONE);
                    self.tk(punct(t.kind).expect("an operator"))?;
                    self.close(n);
                }
                _ => return Ok(()),
            }
        }
    }

    /// A call over what is read since `m`: `call_suffix` (type arguments, value arguments, a
    /// trailing lambda).
    fn call(&mut self, m: Mark, generic: bool) -> R<()> {
        let n = self.t.wrap(m, K::CallExpression, f::NONE);
        let s = self.open(K::CallSuffix);
        if generic {
            self.type_arguments()?;
        }
        if self.peek() == T::LParen {
            self.value_arguments()?;
        }
        if !self.no_lambda && self.lambda_ahead() {
            self.annotated_lambda()?;
        }
        self.close(s);
        self.close(n);
        Ok(())
    }

    /// Whether a trailing lambda follows (`{`, or a label touching `{`), and the statement does not
    /// end first.
    fn lambda_ahead(&mut self) -> bool {
        let t = self.tok();
        if t.nl && self.ends() {
            return false;
        }
        t.kind == T::LBrace || (self.label_ahead() && self.kind_at(2) == T::LBrace)
    }

    fn annotated_lambda(&mut self) -> R<()> {
        let n = self.open(K::AnnotatedLambda);
        if self.peek() != T::LBrace {
            self.label()?;
        }
        self.lambda_literal()?;
        self.close(n);
        Ok(())
    }

    fn navigation(&mut self, m: Mark) -> R<()> {
        let n = self.t.wrap(m, K::NavigationExpression, f::NONE);
        let s = self.open(K::NavigationSuffix);
        if self.peek() == T::Lt {
            self.type_arguments()?;
        }
        let op = self.tok().kind;
        match op {
            T::Dot | T::QuestionDot | T::ColonColon => {
                self.tk(punct(op).expect("an operator"))?;
            }
            _ => return self.refuse(),
        }
        if self.peek() == T::LParen {
            self.parenthesized_expression()?;
        } else if self.is_word(w::CLASS) {
            self.tk(K::TClass)?;
        } else {
            self.simple_identifier()?;
        }
        self.close(s);
        self.close(n);
        Ok(())
    }

    pub(super) fn indexing_suffix(&mut self) -> R<()> {
        let s = self.open(K::IndexingSuffix);
        self.tk(K::TLBracket)?;
        self.level(false, |p| {
            loop {
                p.expression()?;
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
                if p.peek() == T::RBracket {
                    break;
                }
            }
            p.expect(T::RBracket).map(|_| ())
        })?;
        self.close(s);
        Ok(())
    }

    /// `value_arguments`.
    pub(super) fn value_arguments(&mut self) -> R<()> {
        let n = self.open(K::ValueArguments);
        self.tk(K::TLParen)?;
        self.level(false, |p| {
            while p.peek() != T::RParen {
                let a = p.open(K::ValueArgument);
                if p.peek() == T::At {
                    p.annotation_node()?;
                }
                if p.is_ident() && p.kind_at(1) == T::Eq {
                    p.simple_identifier()?;
                    p.tk(K::TEq)?;
                }
                p.expression()?;
                p.close(a);
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
            }
            p.expect(T::RParen).map(|_| ())
        })?;
        self.close(n);
        Ok(())
    }

    // ---- Primary expressions.

    fn primary(&mut self) -> R<()> {
        let t = self.tok();
        match t.kind {
            T::LParen => self.parenthesized_expression(),
            T::Int | T::Hex | T::Bin => self.integer(),
            T::Real => self.leaf(K::RealLiteral, f::NONE).map(|_| ()),
            T::Char => self.character_literal(),
            T::Str => self.string_literal(),
            T::LBracket => self.collection_literal(),
            T::LBrace => self.lambda_literal(),
            T::ColonColon => {
                let n = self.open(K::CallableReference);
                self.callable_reference_rest()?;
                self.close(n);
                Ok(())
            }
            T::Name if self.kw_name(t) => self.leaf(K::SimpleIdentifier, f::NONE).map(|_| ()),
            T::Name if !t.tick => match t.word {
                w::TRUE | w::FALSE => {
                    let n = self.open(K::BooleanLiteral);
                    self.tk(if t.word == w::TRUE { K::TTrue } else { K::TFalse })?;
                    self.close(n);
                    Ok(())
                }
                w::NULL => self.leaf(K::NullLiteral, f::NONE).map(|_| ()),
                w::THIS => self.this_expression(),
                w::SUPER => self.super_expression(),
                w::IF => self.if_expression(),
                w::WHEN => self.when_expression(),
                w::TRY => self.try_expression(),
                w::RETURN | w::THROW | w::BREAK | w::CONTINUE => self.jump_expression(),
                w::OBJECT => self.object_literal(),
                w::DATA if self.is_word_at(1, w::OBJECT) => self.object_literal(),
                w::FUN => self.anonymous_function(),
                w::SUSPEND if self.is_word_at(1, w::FUN) => self.anonymous_function(),
                _ if self.kind_at(1) == T::ColonColon && ident_word(t.word) => self.callable_reference(),
                _ => self.simple_identifier().map(|_| ()),
            },
            T::Name if self.kind_at(1) == T::ColonColon => self.callable_reference(),
            T::Name => self.simple_identifier().map(|_| ()),
            _ => self.refuse(),
        }
    }

    pub(super) fn parenthesized_expression(&mut self) -> R<()> {
        let n = self.open(K::ParenthesizedExpression);
        self.tk(K::TLParen)?;
        self.level(false, |p| p.expression().map(|_| ()))?;
        self.expect(T::RParen)?;
        self.close(n);
        Ok(())
    }

    /// An integer, hex or binary literal, in `long_literal` or `unsigned_literal` where a suffix
    /// touches it.
    fn integer(&mut self) -> R<()> {
        let t = self.tok();
        let inner = match t.kind {
            T::Int => K::IntegerLiteral,
            T::Hex => K::HexLiteral,
            _ => K::BinLiteral,
        };
        let s = self.peek_at(1);
        let suffix = if s.kind == T::Name && s.start == t.end && !s.tick {
            match &self.src[s.start as usize..s.end as usize] {
                b"L" => Some(true),
                b"u" | b"U" | b"uL" | b"UL" => Some(false),
                _ => return self.refuse(),
            }
        } else {
            None
        };
        match suffix {
            None => self.leaf(inner, f::NONE).map(|_| ()),
            Some(long) => {
                let n = self.open(if long { K::LongLiteral } else { K::UnsignedLiteral });
                self.leaf(inner, f::NONE)?;
                if long {
                    self.tk(K::TL)?;
                } else {
                    self.bump()?;
                }
                self.close(n);
                Ok(())
            }
        }
    }

    fn character_literal(&mut self) -> R<()> {
        let t = self.bump()?;
        let (s, e) = (t.start, t.end);
        let n = self.t.open(K::CharacterLiteral, f::NONE, s);
        self.t.token(K::TQuote, f::NONE, s, s + 1);
        if self.src[s as usize + 1] == b'\\' {
            if self.src[s as usize + 2] == b'u' {
                let x = self.t.open(K::CharacterEscapeSeq, f::NONE, s + 1);
                self.t.token(K::TBackslashU, f::NONE, s + 1, s + 3);
                self.t.close_at(x, s + 7);
            } else {
                self.t.leaf(K::CharacterEscapeSeq, f::NONE, s + 1, s + 3);
            }
        }
        self.t.token(K::TQuote, f::NONE, e - 1, e);
        self.t.close_at(n, e);
        Ok(())
    }

    /// `string_literal`: its pieces as the external scanner reads them (`scan_string_content`),
    /// an interpolation's expression parsed from the token buffer cut at its `${`.
    fn string_literal(&mut self) -> R<()> {
        // The string's token read: the tokens after it are lexed again past its end.
        let t = self.bump()?;
        let base = self.i;
        let src = self.src;
        let n = self.t.open(K::StringLiteral, f::NONE, t.start);
        let mut i = t.start as usize;
        let mut prefix = 0usize;
        while src[i] == b'$' {
            i += 1;
            prefix += 1;
        }
        let prefix = prefix.max(1);
        let triple = src[i..].starts_with(b"\"\"\"");
        i += if triple { 3 } else { 1 };
        let at = |k: usize| src.get(k).copied().unwrap_or(0);
        // One scanner call from `i` a step: a piece, or the end.
        let end = 'string: loop {
            let mut j = i;
            let mut has = false;
            loop {
                if j >= src.len() {
                    return self.refuse();
                }
                let c = src[j];
                if c == b'$' {
                    if has {
                        self.t.leaf(K::StringContent, f::NONE, i as u32, j as u32);
                        i = j;
                        break;
                    }
                    j += 1;
                    let first = j;
                    while at(j) == b'$' {
                        j += 1;
                    }
                    let total = j - i;
                    let starts = match at(j) {
                        b'{' | b'_' => true,
                        c if c.is_ascii_alphabetic() => true,
                        c if c >= 0x80 => crate::lex::decode(src, j).is_some_and(|(ch, _)| ch.is_alphabetic()),
                        _ => false,
                    };
                    if total >= prefix && starts {
                        if total > prefix {
                            self.t.leaf(K::StringContent, f::NONE, i as u32, first as u32);
                            i = first;
                            break;
                        }
                        if at(j) == b'{' {
                            if at(j + 1) == b'}' {
                                return self.refuse();
                            }
                            self.t.leaf(K::InterpolationExpressionStart, f::NONE, i as u32, j as u32 + 1);
                            i = self.interpolation(j + 1)?;
                        } else {
                            self.t.leaf(K::InterpolationIdentifierStart, f::NONE, i as u32, j as u32);
                            let (e, _) = self.lx.name_end(j)?;
                            let code = crate::lex::word(src, j, e);
                            match soft(code) {
                                Some(k) => {
                                    let x = self.t.open(K::InterpolatedIdentifier, f::NONE, j as u32);
                                    self.t.token(k, f::NONE, j as u32, e as u32);
                                    self.t.close_at(x, e as u32);
                                }
                                None => {
                                    self.t.leaf(K::InterpolatedIdentifier, f::NONE, j as u32, e as u32);
                                }
                            }
                            i = e;
                        }
                        break;
                    }
                    self.t.leaf(K::StringContent, f::NONE, i as u32, j as u32);
                    i = j;
                    break;
                }
                if c == b'\\' {
                    j += 1;
                    if at(j) == b'$' {
                        j += 1;
                        if at(j) == b'"' {
                            // `\$` before the closer: the end, the content before it with it.
                            break 'string j + 1;
                        }
                    } else if triple && at(j) == b'"' {
                        has = true;
                        continue;
                    }
                    if j >= src.len() {
                        return self.refuse();
                    }
                    j += char_len(src, j);
                    has = true;
                    continue;
                }
                if c == b'"' {
                    if triple {
                        if at(j + 1) != b'"' {
                            self.t.leaf(K::StringContent, f::NONE, i as u32, j as u32 + 1);
                            i = j + 1;
                            break;
                        }
                        if at(j + 2) != b'"' {
                            self.t.leaf(K::StringContent, f::NONE, i as u32, j as u32 + 2);
                            i = j + 2;
                            break;
                        }
                        if has {
                            self.t.leaf(K::StringContent, f::NONE, i as u32, j as u32);
                            i = j;
                            break;
                        }
                        let mut k = j + 3;
                        while at(k) == b'"' {
                            k += 1;
                        }
                        break 'string k;
                    }
                    if has {
                        self.t.leaf(K::StringContent, f::NONE, i as u32, j as u32);
                        i = j;
                        break;
                    }
                    break 'string j + 1;
                }
                j += char_len(src, j);
                has = true;
            }
        };
        // The interpolations' tokens out of the buffer: the string is one token, as the indices
        // read ahead of it count it.
        self.i = base;
        self.relex_at(end);
        self.t.last_end = end as u32;
        self.t.close_at(n, end as u32);
        Ok(())
    }

    /// An interpolation's expression from `at` (past its `${`), and its `}`: the byte after it.
    fn interpolation(&mut self, at: usize) -> R<usize> {
        self.relex_at(at);
        self.t.last_end = at as u32;
        self.level(false, |p| {
            let n = p.open(K::InterpolatedExpression);
            p.expression()?;
            p.close(n);
            Ok(())
        })?;
        let t = self.tok();
        if t.kind != T::RBrace {
            return self.refuse();
        }
        self.bump()?;
        self.t.leaf(K::InterpolationExpressionEnd, f::NONE, t.start, t.end);
        // The tokens lexed past the `}` are the string's: lexed again after it.
        self.relex_at(t.end as usize);
        Ok(t.end as usize)
    }

    fn collection_literal(&mut self) -> R<()> {
        let n = self.open(K::CollectionLiteral);
        self.tk(K::TLBracket)?;
        self.level(false, |p| {
            loop {
                p.expression()?;
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
                if p.peek() == T::RBracket {
                    break;
                }
            }
            p.expect(T::RBracket).map(|_| ())
        })?;
        self.close(n);
        Ok(())
    }

    /// The index of the `->` ending a lambda's parameters, the `{` the next token; `None` where
    /// the tokens before the first `->` cannot be parameters.
    pub(super) fn lambda_params_end(&mut self) -> Option<usize> {
        let mut depth = 0u32;
        let mut k = 1;
        loop {
            let t = self.peek_at(k);
            match t.kind {
                T::Arrow if depth == 0 => return Some(k),
                T::LParen | T::LBracket | T::Lt => depth += 1,
                T::RParen | T::RBracket | T::Gt => {
                    if depth == 0 {
                        return None;
                    }
                    depth -= 1;
                }
                T::Name => {
                    if !t.tick && !ident_word(t.word) && !matches!(t.word, w::IN | w::OUT) {
                        return None;
                    }
                }
                T::Comma | T::Colon | T::Dot | T::Question | T::Star | T::At | T::Arrow | T::Amp => {}
                _ => return None,
            }
            k += 1;
            if k > 512 {
                return None;
            }
        }
    }

    pub(super) fn lambda_literal(&mut self) -> R<()> {
        let n = self.open(K::LambdaLiteral);
        let params = self.lambda_params_end().is_some();
        self.tk(K::TLBrace)?;
        if params {
            if self.peek() != T::Arrow {
                self.level(false, |p| p.lambda_parameters())?;
            }
            self.expect(T::Arrow)?;
        }
        self.statements()?;
        self.level(true, |p| p.expect(T::RBrace).map(|_| ()))?;
        self.close(n);
        Ok(())
    }

    fn lambda_parameters(&mut self) -> R<()> {
        let n = self.open(K::LambdaParameters);
        loop {
            if matches!(self.peek(), T::LParen | T::LBracket) {
                self.multi_variable_declaration()?;
                if self.peek() == T::Colon {
                    self.tk(K::TColon)?;
                    self.ty()?;
                }
            } else {
                self.variable_declaration()?;
            }
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::TComma)?;
            if self.peek() == T::Arrow {
                break;
            }
        }
        self.close(n);
        Ok(())
    }

    fn anonymous_function(&mut self) -> R<()> {
        let n = self.open(K::AnonymousFunction);
        if self.is_word(w::SUSPEND) {
            self.tk(K::TSuspend)?;
        }
        self.kw(w::FUN, K::TFun)?;
        if self.peek() != T::LParen {
            // A receiver: simple user types, then `.`.
            loop {
                self.type_identifier()?;
                if self.peek() == T::Lt {
                    self.type_arguments()?;
                }
                self.expect(T::Dot)?;
                if self.peek() == T::LParen {
                    break;
                }
            }
        }
        self.function_value_parameters()?;
        if self.peek() == T::Colon {
            self.tk(K::TColon)?;
            self.ty()?;
        }
        if !self.ended() && matches!(self.peek(), T::LBrace | T::Eq) {
            self.function_body()?;
        }
        self.close(n);
        Ok(())
    }

    fn this_expression(&mut self) -> R<()> {
        let n = self.open(K::ThisExpression);
        if self.kind_at(1) == T::At && self.adjacent(0) && self.kind_at(2) == T::Name && self.adjacent(1) {
            let a = self.bump()?;
            let b = self.bump()?;
            self.t.token(K::TThisAt, f::NONE, a.start, b.end);
            self.leaf(K::TypeIdentifier, f::NONE)?;
        } else {
            self.tk(K::TThis)?;
        }
        self.close(n);
        Ok(())
    }

    fn super_expression(&mut self) -> R<()> {
        let n = self.open(K::SuperExpression);
        if self.kind_at(1) == T::At && self.adjacent(0) && self.kind_at(2) == T::Name && self.adjacent(1) {
            let a = self.bump()?;
            let b = self.bump()?;
            self.t.token(K::TSuperAt, f::NONE, a.start, b.end);
            self.leaf(K::TypeIdentifier, f::NONE)?;
        } else {
            self.tk(K::TSuper)?;
            if self.peek() == T::Lt && self.type_args_end(0).is_some() {
                self.tk(K::TLt)?;
                self.ty()?;
                self.expect_gt()?;
                if self.peek() == T::At && self.tok().start == self.t.last_end {
                    self.tk(K::TAt)?;
                    self.leaf(K::TypeIdentifier, f::NONE)?;
                }
            }
        }
        self.close(n);
        Ok(())
    }

    fn callable_reference(&mut self) -> R<()> {
        let n = self.open(K::CallableReference);
        self.type_identifier()?;
        self.callable_reference_rest()?;
        self.close(n);
        Ok(())
    }

    fn callable_reference_rest(&mut self) -> R<()> {
        self.tk(K::TColonColon)?;
        if self.is_word(w::CLASS) {
            self.tk(K::TClass)?;
        } else {
            self.simple_identifier()?;
        }
        Ok(())
    }

    fn if_expression(&mut self) -> R<()> {
        let n = self.open(K::IfExpression);
        self.tk(K::TIf)?;
        self.expect(T::LParen)?;
        let c = self.level(false, |p| p.expression())?;
        self.t.set_field(c, f::CONDITION);
        self.expect(T::RParen)?;
        let semi_else = |p: &mut Self| p.peek() == T::Semi && p.is_word_at(1, w::ELSE);
        if self.peek() == T::Semi && !semi_else(self) {
            self.tk(K::TSemi)?;
            self.close(n);
            return Ok(());
        }
        if !self.is_word(w::ELSE) && !semi_else(self) {
            self.control_structure_body(Some(f::CONSEQUENCE))?;
        }
        if semi_else(self) {
            self.tk(K::TSemi)?;
        }
        let t = self.tok();
        if self.is_word(w::ELSE) && !(t.nl && self.ends()) {
            self.tk(K::TElse)?;
            if self.peek() == T::Semi {
                let t = self.bump()?;
                self.t.token(K::TSemi, f::ALTERNATIVE, t.start, t.end);
            } else {
                self.control_structure_body(Some(f::ALTERNATIVE))?;
            }
        }
        self.close(n);
        Ok(())
    }

    fn when_expression(&mut self) -> R<()> {
        let n = self.open(K::WhenExpression);
        self.tk(K::TWhen)?;
        if self.peek() == T::LParen {
            let s = self.open(K::WhenSubject);
            self.tk(K::TLParen)?;
            self.level(false, |p| {
                let mut k = Some(0);
                while let Some(j) = k.filter(|&j| p.peek_at(j).kind == T::At) {
                    k = p.skip_annotation(j);
                }
                if k.is_some_and(|k| p.is_word_at(k, w::VAL)) {
                    while p.peek() == T::At {
                        p.annotation_node()?;
                    }
                    p.tk(K::TVal)?;
                    p.variable_declaration()?;
                    p.expect(T::Eq)?;
                }
                p.expression().map(|_| ())
            })?;
            self.expect(T::RParen)?;
            self.close(s);
        }
        self.expect(T::LBrace)?;
        self.level(true, |p| {
            while p.peek() != T::RBrace {
                p.when_entry()?;
            }
            p.expect(T::RBrace).map(|_| ())
        })?;
        self.close(n);
        Ok(())
    }

    fn when_entry(&mut self) -> R<()> {
        let n = self.open(K::WhenEntry);
        if self.is_word(w::ELSE) {
            self.tk(K::TElse)?;
        } else {
            self.level(false, |p| {
                loop {
                    p.when_condition()?;
                    if p.is_word(w::IF) {
                        let g = p.open(K::GuardCondition);
                        p.tk(K::TIf)?;
                        p.expression()?;
                        p.close(g);
                    }
                    if p.peek() != T::Comma {
                        break;
                    }
                    p.tk(K::TComma)?;
                    if p.peek() == T::Arrow {
                        break;
                    }
                }
                Ok(())
            })?;
        }
        self.expect(T::Arrow)?;
        self.control_structure_body(None)?;
        self.semi()?;
        self.close(n);
        Ok(())
    }

    fn when_condition(&mut self) -> R<()> {
        let n = self.open(K::WhenCondition);
        let bang = self.peek() == T::Bang && (self.is_word_at(1, w::IN) || self.is_word_at(1, w::IS));
        let k = usize::from(bang);
        if self.is_word_at(k, w::IN) {
            let r = self.open(K::RangeTest);
            if bang {
                self.tk(K::TBang)?;
            }
            self.tk(K::TIn)?;
            self.expression()?;
            self.close(r);
        } else if self.is_word_at(k, w::IS) {
            let r = self.open(K::TypeTest);
            if bang {
                self.tk(K::TBang)?;
            }
            self.tk(K::TIs)?;
            self.ty()?;
            self.close(r);
        } else {
            self.expression()?;
        }
        self.close(n);
        Ok(())
    }

    fn try_expression(&mut self) -> R<()> {
        let n = self.open(K::TryExpression);
        self.tk(K::TTry)?;
        self.block()?;
        let mut any = false;
        while self.is_word(w::CATCH) {
            any = true;
            let c = self.open(K::CatchBlock);
            self.tk(K::TCatch)?;
            self.expect(T::LParen)?;
            self.level(false, |p| {
                while p.peek() == T::At {
                    p.annotation_node()?;
                }
                p.simple_identifier()?;
                p.expect(T::Colon)?;
                p.ty().map(|_| ())
            })?;
            self.expect(T::RParen)?;
            self.block()?;
            self.close(c);
        }
        if self.is_word(w::FINALLY) {
            any = true;
            let c = self.open(K::FinallyBlock);
            self.tk(K::TFinally)?;
            self.block()?;
            self.close(c);
        }
        if !any {
            return self.refuse();
        }
        self.close(n);
        Ok(())
    }

    fn jump_expression(&mut self) -> R<()> {
        let n = self.open(K::JumpExpression);
        let t = self.tok();
        if t.word == w::THROW {
            self.tk(K::TThrow)?;
            self.expression()?;
            self.close(n);
            return Ok(());
        }
        let at = self.kind_at(1) == T::At && self.adjacent(0) && self.kind_at(2) == T::Name && self.adjacent(1);
        if at {
            let a = self.bump()?;
            let b = self.bump()?;
            let k = match t.word {
                w::RETURN => K::TReturnAt,
                w::BREAK => K::TBreakAt,
                _ => K::TContinueAt,
            };
            self.t.token(k, f::NONE, a.start, b.end);
            self.leaf(K::Label, f::NONE)?;
        } else {
            let k = match t.word {
                w::RETURN => K::TReturn,
                w::BREAK => K::TBreak,
                _ => K::TContinue,
            };
            self.tk(k)?;
        }
        if t.word == w::RETURN && !self.ended() && self.starts_expression() {
            self.expression()?;
        }
        self.close(n);
        Ok(())
    }

    /// The target of an assignment, ahead of its operator: `directly_assignable_expression`'s
    /// `_postfix_unary_expression` (a primary expression, which may be a call, and the suffixes after
    /// the last call, flat), or a name.
    pub(super) fn assignable(&mut self) -> R<()> {
        // The token after the last call's suffix: the primary expression ends there.
        let mut k = 0;
        let mut depth = 0u32;
        let mut call_end = 0;
        loop {
            let t = self.peek_at(k);
            match t.kind {
                T::Eq | T::PlusEq | T::MinusEq | T::StarEq | T::SlashEq | T::PercentEq if depth == 0 => break,
                T::LParen | T::LBrace => {
                    if depth == 0 && k > 0 && !matches!(self.peek_at(k - 1).kind, T::Dot | T::QuestionDot | T::ColonColon) {
                        // A call's arguments or lambda: the primary goes on to its end.
                        let mut d = 0u32;
                        let mut j = k;
                        loop {
                            match self.peek_at(j).kind {
                                T::LParen | T::LBrace | T::LBracket => d += 1,
                                T::RParen | T::RBrace | T::RBracket => {
                                    d -= 1;
                                    if d == 0 {
                                        break;
                                    }
                                }
                                T::Eof => return self.refuse(),
                                _ => {}
                            }
                            j += 1;
                        }
                        call_end = j + 1;
                        k = j + 1;
                        continue;
                    }
                    depth += 1;
                }
                T::LBracket => depth += 1,
                T::RParen | T::RBrace | T::RBracket => depth = depth.saturating_sub(1),
                T::Lt if depth == 0 => {
                    if let Some(e) = self.type_args_end(k) {
                        if self.kind_at(e) == T::LParen {
                            k = e;
                            continue;
                        }
                        k = e;
                        continue;
                    }
                }
                T::Eof => return self.refuse(),
                _ => {}
            }
            k += 1;
        }
        let limit = self.i + call_end;
        let m = self.mark();
        self.primary()?;
        if call_end > 0 {
            self.postfix(m, limit, true)?;
        }
        self.t.unmark(m);
        // The suffixes after it, flat.
        loop {
            match self.peek() {
                T::Dot | T::QuestionDot | T::ColonColon => {
                    let s = self.open(K::NavigationSuffix);
                    let op = self.tok().kind;
                    self.tk(punct(op).expect("an operator"))?;
                    if self.peek() == T::LParen {
                        self.parenthesized_expression()?;
                    } else if self.is_word(w::CLASS) {
                        self.tk(K::TClass)?;
                    } else {
                        self.simple_identifier()?;
                    }
                    self.close(s);
                }
                T::Lt => {
                    let s = self.open(K::NavigationSuffix);
                    self.type_arguments()?;
                    let op = self.tok().kind;
                    if !matches!(op, T::Dot | T::QuestionDot | T::ColonColon) {
                        return self.refuse();
                    }
                    self.tk(punct(op).expect("an operator"))?;
                    self.simple_identifier()?;
                    self.close(s);
                }
                T::LBracket => self.indexing_suffix()?,
                T::PlusPlus | T::MinusMinus | T::BangBang => {
                    let op = self.tok().kind;
                    self.tk(punct(op).expect("an operator"))?;
                }
                _ => return Ok(()),
            }
        }
    }
}

/// The length of the character at `j` (one byte, or a UTF-8 sequence's).
fn char_len(src: &[u8], j: usize) -> usize {
    match src[j] {
        c if c < 0x80 => 1,
        c if c >= 0xf0 => 4,
        c if c >= 0xe0 => 3,
        _ => 2,
    }
}

/// How tree-sitter reads `x as a.b.C<T>` at a line's end (`P::as_split`).
#[derive(Clone, Copy)]
enum Split {
    /// `C<T>` and the next line a call (`(…)` or `{ … }`).
    Call,
    /// `C < T > next`: two comparisons.
    Compare,
}
