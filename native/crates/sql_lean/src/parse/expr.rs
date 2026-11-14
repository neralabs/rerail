//! Expressions (`_expression`), terms and types (`_type`), by precedence climbing over the
//! grammar's precedences (`prec`): an operator binds its right operand at its own level plus one
//! (every binary operator is `prec_left`), a prefix operator its operand the same.

use super::*;

/// The keywords that may follow a term, which the parser reads where they stand (a clause, an
/// operator, a join): a word among them after a term is not its alias.
pub(super) const AFTER_TERM: &[K] = &[
    K::KeywordFrom,
    K::KeywordWhere,
    K::KeywordGroup,
    K::KeywordHaving,
    K::KeywordOrder,
    K::KeywordLimit,
    K::KeywordOffset,
    K::KeywordUnion,
    K::KeywordExcept,
    K::KeywordIntersect,
    K::KeywordInto,
    K::KeywordWindow,
    K::KeywordFetch,
    K::KeywordFor,
    K::KeywordAs,
    K::KeywordOn,
    K::KeywordUsing,
    K::KeywordJoin,
    K::KeywordInner,
    K::KeywordLeft,
    K::KeywordRight,
    K::KeywordFull,
    K::KeywordCross,
    K::KeywordNatural,
    K::KeywordWhen,
    K::KeywordThen,
    K::KeywordElse,
    K::KeywordEnd,
    K::KeywordAnd,
    K::KeywordOr,
    K::KeywordNot,
    K::KeywordIs,
    K::KeywordIn,
    K::KeywordLike,
    K::KeywordBetween,
    K::KeywordSimilar,
    K::KeywordCollate,
    K::KeywordAt,
    K::KeywordOver,
    K::KeywordFilter,
    K::KeywordEscape,
    K::KeywordAsc,
    K::KeywordDesc,
    K::KeywordNulls,
    K::KeywordReturning,
    K::KeywordValues,
    K::KeywordSet,
    K::KeywordDo,
    K::KeywordSeparator,
    K::KeywordLateral,
    K::KeywordWith,
    K::KeywordOrdinality,
    K::KeywordIsnull,
    K::KeywordNotnull,
];

/// What `atom` read: an expression, or `*` / `x.*` (an `all_fields`, which only a term and a
/// parenthesized expression take).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Atom {
    Expr,
    AllFields,
}

impl P<'_> {
    /// A node of the children since `m` spanning `s..e` (a node whose last bytes are hidden tokens:
    /// a signed number's digits).
    pub(super) fn node_span(&mut self, kind: K, m: usize, s: usize, e: usize) -> u32 {
        let id = self.a.node(kind, m);
        self.a.nodes[id as usize].s = s as u32;
        self.a.nodes[id as usize].e = e as u32;
        id
    }

    /// An expression, its operators those of level `min` and above, given `field`.
    pub(super) fn expr(&mut self, min: u8, field: u8) -> R<u32> {
        let id = self.expr_or_all(min, false)?;
        self.a.set_field(id, field);
        Ok(id)
    }

    /// An expression, or (`all`) an `all_fields` where one stands alone.
    pub(super) fn expr_or_all(&mut self, min: u8, all: bool) -> R<u32> {
        self.deeper()?;
        let m = self.a.mark();
        let atom = self.atom(all)?;
        if atom == Atom::AllFields {
            self.depth -= 1;
            return Ok(*self.a.stack.last().expect("an atom"));
        }
        loop {
            let t = self.peek()?;
            match t.t {
                T::Punct => {
                    let c = self.byte(t.s);
                    if c == b'[' {
                        if prec::SUBSCRIPT < min {
                            break;
                        }
                        // `x[i]`, `x[i:j]`: `subscript`.
                        self.a.set_field(self.a.stack[m], f::EXPRESSION);
                        self.expect_punct(b"[", K::TLBracket)?;
                        let i = self.expr(0, 0)?;
                        let c = self.peek()?;
                        if !(c.t == T::Punct && self.byte(c.s) == b':' && self.byte(c.s + 1) != b':') {
                            self.a.set_field(i, f::SUBSCRIPT);
                        } else {
                            self.a.set_field(i, f::LOWER);
                            self.take(Tok { e: c.s + 1, ..c });
                            self.a.leaf(K::TColon, 0, c.s, c.s + 1);
                            self.expr(0, f::UPPER)?;
                        }
                        self.expect_punct(b"]", K::TRBracket)?;
                        self.a.node(K::Subscript, m);
                        continue;
                    }
                    if matches!(c, b'(' | b')' | b',' | b';' | b']' | b'.' | b'{' | b'}') {
                        break;
                    }
                    if c == b':' {
                        if self.byte(t.s + 1) != b':' || prec::TYPECAST < min {
                            break;
                        }
                        // `x::type`: `implicit_cast`, aliased `cast`.
                        let first = self.a.stack[m];
                        let _ = first;
                        self.take(Tok { e: t.s + 2, ..t });
                        self.a.leaf(K::TColonColon, 0, t.s, t.s + 2);
                        self.type_(0)?;
                        self.a.node(K::Cast, m);
                        continue;
                    }
                    let (other, n) = self.operator_at(t.s);
                    if n == 0 {
                        break;
                    }
                    let text = &self.src[t.s..t.s + n];
                    let (level, kind) = if other {
                        (prec::OTHER, K::OpOther)
                    } else {
                        match text {
                            b"+" => (prec::PLUS, K::TPlus),
                            b"-" => (prec::PLUS, K::TMinus),
                            b"*" => (prec::TIMES, K::TStar),
                            b"/" => (prec::TIMES, K::TSlash),
                            b"%" => (prec::TIMES, K::TPercent),
                            b"^" => (prec::EXP, K::TCaret),
                            b"=" => (prec::RELATION, K::TEq),
                            b"<" => (prec::RELATION, K::TLt),
                            b"<=" => (prec::RELATION, K::TLe),
                            b"!=" => (prec::RELATION, K::TNe),
                            b">=" => (prec::RELATION, K::TGe),
                            b">" => (prec::RELATION, K::TGt),
                            b"<>" => (prec::RELATION, K::TLtGt),
                            _ => return self.refuse(),
                        }
                    };
                    if level < min {
                        break;
                    }
                    self.take(Tok { e: t.s + n, ..t });
                    self.a.set_field(self.a.stack[m], f::LEFT);
                    self.a.leaf(kind, f::OPERATOR, t.s, t.s + n);
                    self.expr(level + 1, f::RIGHT)?;
                    self.a.node(K::BinaryExpression, m);
                }
                T::Word => {
                    let ws = self.words()?;
                    let has = |k: K| ws.iter().any(|&(_, x)| x == k);
                    let (level, op) = if has(K::KeywordAnd) {
                        (prec::AND, 0)
                    } else if has(K::KeywordOr) {
                        (prec::OR, 0)
                    } else if has(K::KeywordIs) {
                        (prec::IS, 1)
                    } else if has(K::KeywordIn) {
                        if !self.next_is_paren()? {
                            // `in` with no list after it is not this expression's: `position(a in b)`.
                            break;
                        }
                        (prec::IN, 2)
                    } else if has(K::KeywordLike) {
                        (prec::PATTERN, 3)
                    } else if has(K::KeywordBetween) {
                        (prec::BETWEEN, 4)
                    } else if has(K::KeywordNot) {
                        // `not in`, `not like`, `not between`, `not similar to`: by the word after.
                        (0, 5)
                    } else if has(K::KeywordCollate) {
                        if prec::COLLATE < min {
                            break;
                        }
                        // `collate_expression`: `x collate "C"`.
                        self.a.set_field(self.a.stack[m], f::EXPRESSION);
                        self.expect(K::KeywordCollate)?;
                        if self.object_reference(NONE, f::COLLATION)?.is_none() {
                            return self.refuse();
                        }
                        self.a.node(K::CollateExpression, m);
                        continue;
                    } else if has(K::KeywordAt) && self.next_is(K::KeywordTime)? {
                        if prec::AT_TIME_ZONE < min {
                            break;
                        }
                        // `x at time zone y`: `at_time_zone`, a binary expression.
                        self.a.set_field(self.a.stack[m], f::LEFT);
                        let n = self.a.mark();
                        self.expect(K::KeywordAt)?;
                        self.expect(K::KeywordTime)?;
                        self.expect(K::KeywordZone)?;
                        let id = self.a.node(K::AtTimeZone, n);
                        self.a.set_field(id, f::OPERATOR);
                        self.expr(prec::AT_TIME_ZONE + 1, f::RIGHT)?;
                        self.a.node(K::BinaryExpression, m);
                        continue;
                    } else if has(K::KeywordSimilar) && self.next_is(K::KeywordTo)? {
                        if prec::PATTERN < min {
                            break;
                        }
                        // `x similar to y`.
                        self.a.set_field(self.a.stack[m], f::LEFT);
                        let n = self.a.mark();
                        self.expect(K::KeywordSimilar)?;
                        self.expect(K::KeywordTo)?;
                        let id = self.a.node(K::SimilarTo, n);
                        self.a.set_field(id, f::OPERATOR);
                        self.like_tail(m)?;
                        continue;
                    } else if has(K::KeywordAt) || has(K::KeywordIsnull) || has(K::KeywordNotnull) || has(K::KeywordSimilar) {
                        return self.refuse();
                    } else {
                        break;
                    };
                    match op {
                        0 => {
                            if level < min {
                                break;
                            }
                            self.a.set_field(self.a.stack[m], f::LEFT);
                            let k = if level == prec::AND { K::KeywordAnd } else { K::KeywordOr };
                            self.kw_field(k, f::OPERATOR)?;
                            self.expr(level + 1, f::RIGHT)?;
                            self.a.node(K::BinaryExpression, m);
                        }
                        1 => {
                            if level < min {
                                break;
                            }
                            self.is_tail(m)?;
                        }
                        2 => {
                            if level < min {
                                break;
                            }
                            self.a.set_field(self.a.stack[m], f::LEFT);
                            self.kw_field(K::KeywordIn, f::OPERATOR)?;
                            self.in_right()?;
                            self.a.node(K::BinaryExpression, m);
                        }
                        3 => {
                            if level < min {
                                break;
                            }
                            self.a.set_field(self.a.stack[m], f::LEFT);
                            self.kw_field(K::KeywordLike, f::OPERATOR)?;
                            self.like_tail(m)?;
                        }
                        4 => {
                            if level < min {
                                break;
                            }
                            self.a.set_field(self.a.stack[m], f::LEFT);
                            self.kw_field(K::KeywordBetween, f::OPERATOR)?;
                            self.between_tail(m)?;
                        }
                        _ => {
                            // `not` after an expression: the word after it decides.
                            let save = (self.at, self.last_end, self.look);
                            let t = self.peek()?;
                            self.take(t);
                            let next = self.words()?;
                            (self.at, self.last_end, self.look) = save;
                            let nx = |k: K| next.iter().any(|&(_, x)| x == k);
                            let level = if nx(K::KeywordIn) {
                                prec::IN
                            } else if nx(K::KeywordLike) {
                                prec::PATTERN
                            } else if nx(K::KeywordBetween) {
                                prec::BETWEEN
                            } else if nx(K::KeywordSimilar) {
                                prec::PATTERN
                            } else {
                                // No operator starts so: the expression ends (`default 0 not null`).
                                break;
                            };
                            if level < min {
                                break;
                            }
                            self.a.set_field(self.a.stack[m], f::LEFT);
                            if nx(K::KeywordIn) {
                                let n = self.a.mark();
                                self.expect(K::KeywordNot)?;
                                self.expect(K::KeywordIn)?;
                                let id = self.a.node(K::NotIn, n);
                                self.a.set_field(id, f::OPERATOR);
                                self.in_right()?;
                                self.a.node(K::BinaryExpression, m);
                            } else if nx(K::KeywordLike) {
                                let n = self.a.mark();
                                self.expect(K::KeywordNot)?;
                                self.expect(K::KeywordLike)?;
                                let id = self.a.node(K::NotLike, n);
                                self.a.set_field(id, f::OPERATOR);
                                self.like_tail(m)?;
                            } else if nx(K::KeywordSimilar) {
                                let n = self.a.mark();
                                self.expect(K::KeywordNot)?;
                                self.expect(K::KeywordSimilar)?;
                                self.expect(K::KeywordTo)?;
                                let id = self.a.node(K::NotSimilarTo, n);
                                self.a.set_field(id, f::OPERATOR);
                                self.like_tail(m)?;
                            } else {
                                self.kw_field(K::KeywordNot, f::OPERATOR)?;
                                self.kw_field(K::KeywordBetween, f::OPERATOR)?;
                                self.between_tail(m)?;
                            }
                        }
                    }
                }
                _ => break,
            }
        }
        self.depth -= 1;
        Ok(*self.a.stack.last().expect("an expression"))
    }

    /// After `x is`: `is null`, `is not null`, `is true`, `is distinct from y`… (`binary_is`).
    fn is_tail(&mut self, m: usize) -> R<()> {
        self.a.set_field(self.a.stack[m], f::LEFT);
        let save = (self.at, self.last_end, self.look);
        let t = self.peek()?;
        self.take(t);
        let next = self.words()?;
        (self.at, self.last_end, self.look) = save;
        let nx = |k: K| next.iter().any(|&(_, x)| x == k);
        if nx(K::KeywordNot) {
            let n = self.a.mark();
            self.expect(K::KeywordIs)?;
            self.expect(K::KeywordNot)?;
            if self.is(K::KeywordDistinct)? {
                self.expect(K::KeywordDistinct)?;
                self.expect(K::KeywordFrom)?;
                let id = self.a.node(K::NotDistinctFrom, n);
                self.a.set_field(id, f::OPERATOR);
                self.expr(prec::IS + 1, f::RIGHT)?;
                self.a.node(K::BinaryExpression, m);
                return Ok(());
            } else {
                let id = self.a.node(K::IsNot, n);
                self.a.set_field(id, f::OPERATOR);
            }
        } else if nx(K::KeywordDistinct) {
            let n = self.a.mark();
            self.expect(K::KeywordIs)?;
            self.expect(K::KeywordDistinct)?;
            self.expect(K::KeywordFrom)?;
            let id = self.a.node(K::DistinctFrom, n);
            self.a.set_field(id, f::OPERATOR);
            self.expr(prec::IS + 1, f::RIGHT)?;
            self.a.node(K::BinaryExpression, m);
            return Ok(());
        } else if nx(K::KeywordNull) || nx(K::KeywordTrue) || nx(K::KeywordFalse) {
            self.kw_field(K::KeywordIs, f::OPERATOR)?;
        } else {
            return self.refuse();
        }
        if !(self.is(K::KeywordNull)? || self.is(K::KeywordTrue)? || self.is(K::KeywordFalse)?) {
            return self.refuse();
        }
        self.expr(prec::IS + 1, f::RIGHT)?;
        self.a.node(K::BinaryExpression, m);
        Ok(())
    }

    /// `in`'s right operand: a list or a subquery.
    fn in_right(&mut self) -> R<()> {
        if !self.at_punct(b"(")? {
            return self.refuse();
        }
        let id = self.paren(false)?;
        let k = self.a.nodes[id as usize].kind;
        if k == K::ParenthesizedExpression {
            // `(x)` after `in` is a one-item list.
            self.a.nodes[id as usize].kind = K::List;
        } else if k != K::List && k != K::Subquery {
            return self.refuse();
        }
        self.a.set_field(id, f::RIGHT);
        Ok(())
    }

    /// After `x like` (or `not like`): the pattern, and an escape.
    fn like_tail(&mut self, m: usize) -> R<()> {
        self.expr(prec::PATTERN + 1, f::RIGHT)?;
        if self.is(K::KeywordEscape)? {
            return self.refuse();
        }
        self.a.node(K::BinaryExpression, m);
        Ok(())
    }

    /// After `x between` (or `not between`): `low and high`.
    fn between_tail(&mut self, m: usize) -> R<()> {
        self.expr(prec::BETWEEN + 1, f::LOW)?;
        self.expect(K::KeywordAnd)?;
        self.expr(prec::BETWEEN + 1, f::HIGH)?;
        self.a.node(K::BetweenExpression, m);
        Ok(())
    }

    /// An operand: a literal, a field, an invocation, a parenthesized expression, a prefix
    /// operator's expression…
    pub(super) fn atom(&mut self, all: bool) -> R<Atom> {
        let t = self.peek()?;
        match t.t {
            T::Num => {
                self.take(t);
                self.a.leaf(K::Literal, 0, t.s, t.e);
            }
            T::Str => self.string_literal(0)?,
            T::EStr => {
                self.take(t);
                self.a.leaf(K::Literal, 0, t.s, t.e);
            }
            T::Bits => {
                self.take(t);
                self.a.leaf(K::Literal, 0, t.s, t.e);
                // `_bit_string`'s continuations (`B'01' '10'`) are refused.
                if self.peek()?.t == T::Str {
                    return self.refuse();
                }
            }
            T::Dollar => {
                let e = self.dollar_end(t.s)?;
                self.take(Tok { e, ..t });
                self.a.leaf(K::Literal, 0, t.s, e);
            }
            T::Param => {
                self.take(t);
                self.a.leaf(K::Parameter, 0, t.s, t.e);
            }
            T::Punct => {
                let c = self.byte(t.s);
                if c == b'(' {
                    self.paren(all)?;
                } else if c == b'?' && self.op_len(t.s, 1) {
                    // `?` where an operand goes: a `parameter` (where an operator goes, `op_other`).
                    self.take(Tok { e: t.s + 1, ..t });
                    self.a.leaf(K::Parameter, 0, t.s, t.s + 1);
                } else if c == b':' && (self.byte(t.s + 1).is_ascii_alphabetic() || self.byte(t.s + 1) == b'_') {
                    // `:name` where an operand goes: a `parameter` (`:[A-Za-z_][A-Za-z0-9_]*`). Its end
                    // is the word's, from stage 1; a word with `$` or bytes past ASCII is refused, as
                    // are the quoted forms (`:'name'`, psql's).
                    let e = self.b.word_end(t.s + 1);
                    if self.src[t.s + 1..e].iter().any(|&b| b == b'$' || b >= 0x80) {
                        return self.refuse();
                    }
                    self.take(Tok { e, ..t });
                    self.a.leaf(K::Parameter, 0, t.s, e);
                } else if c == b'*' && self.op_len(t.s, 1) {
                    if !all {
                        return self.refuse();
                    }
                    let m = self.a.mark();
                    self.punct(b"*", K::TStar)?;
                    self.a.node(K::AllFields, m);
                    return Ok(Atom::AllFields);
                } else if c == b'-' || c == b'+' {
                    // A signed number is a literal (`_integer`, `_decimal_number`); any other operand
                    // a unary expression.
                    if !self.op_len(t.s, 1) {
                        return self.refuse();
                    }
                    let kind = if c == b'-' { K::TMinus } else { K::TPlus };
                    let m = self.a.mark();
                    self.take(Tok { e: t.s + 1, ..t });
                    let sign = self.a.leaf(kind, 0, t.s, t.s + 1);
                    let n = self.peek()?;
                    if n.t == T::Num {
                        self.take(n);
                        self.node_span(K::Literal, m, t.s, n.e);
                    } else {
                        self.a.set_field(sign, f::OPERATOR);
                        self.expr(prec::MINUS + 1, f::OPERAND)?;
                        self.a.node(K::UnaryExpression, m);
                    }
                } else {
                    return self.refuse();
                }
            }
            T::Word | T::Quoted => {
                let ws = self.words()?;
                let has = |k: K| ws.iter().any(|&(_, x)| x == k);
                if t.t == T::Word && ws.iter().any(|&(_, k)| kinds::FIRST_EXPRESSION.contains(&k)) {
                    let m = self.a.mark();
                    if has(K::KeywordNull) || has(K::KeywordTrue) || has(K::KeywordFalse) {
                        let k = ws.iter().find(|&&(_, k)| matches!(k, K::KeywordNull | K::KeywordTrue | K::KeywordFalse)).expect("one").1;
                        self.expect(k)?;
                        self.a.node(K::Literal, m);
                    } else if has(K::KeywordNot) {
                        self.kw_field(K::KeywordNot, f::OPERATOR)?;
                        self.expr(prec::NOT + 1, f::OPERAND)?;
                        self.a.node(K::UnaryExpression, m);
                    } else if has(K::KeywordAny) || has(K::KeywordSome) || has(K::KeywordAll) {
                        let k = ws.iter().find(|&&(_, k)| matches!(k, K::KeywordAny | K::KeywordSome | K::KeywordAll)).expect("one").1;
                        self.kw_field(k, f::OPERATOR)?;
                        self.expr(prec::NOT + 1, f::OPERAND)?;
                        self.a.node(K::UnaryExpression, m);
                    } else if has(K::KeywordCase) {
                        self.case()?;
                    } else if has(K::KeywordCast) {
                        self.kw_field(K::KeywordCast, f::NAME)?;
                        self.expect_punct(b"(", K::TLParen)?;
                        self.expr(0, f::PARAMETER)?;
                        self.expect(K::KeywordAs)?;
                        self.type_(0)?;
                        self.expect_punct(b")", K::TRParen)?;
                        self.a.node(K::Cast, m);
                    } else if has(K::KeywordExists) {
                        self.expect(K::KeywordExists)?;
                        if !self.at_punct(b"(")? {
                            return self.refuse();
                        }
                        let id = self.paren(false)?;
                        if self.a.nodes[id as usize].kind != K::Subquery {
                            return self.refuse();
                        }
                        self.a.node(K::Exists, m);
                    } else if has(K::KeywordArray) {
                        self.expect(K::KeywordArray)?;
                        if self.at_punct(b"(")? {
                            let id = self.paren(false)?;
                            if self.a.nodes[id as usize].kind != K::Subquery {
                                return self.refuse();
                            }
                            // `array (select …)`: the subquery's parentheses are the array's.
                            self.unwrap_last();
                        } else {
                            self.array_elements()?;
                        }
                        self.a.node(K::Array, m);
                    } else if has(K::KeywordInterval) {
                        self.expect(K::KeywordInterval)?;
                        let n = self.peek()?;
                        if n.t != T::Str || self.byte(n.s) != b'\'' {
                            return self.refuse();
                        }
                        self.take(n);
                        let e = self.continued(n.e)?;
                        if self.words()?.iter().any(|&(_, k)| matches!(k, K::KeywordYear | K::KeywordMonth | K::KeywordDay | K::KeywordHour | K::KeywordMinute | K::KeywordSecond | K::KeywordTo)) {
                            return self.refuse();
                        }
                        let id = self.a.node(K::Interval, m);
                        self.a.nodes[id as usize].e = e as u32;
                    } else if has(K::KeywordTimestamp) || has(K::KeywordTime) {
                        // `_string_casting`: `timestamp [with time zone] '…'`, a literal of the type
                        // and the string.
                        let k = if has(K::KeywordTimestamp) { K::KeywordTimestamp } else { K::KeywordTime };
                        self.builtin_type(k)?;
                        self.string_cast(m)?;
                    } else {
                        return self.refuse();
                    }
                } else {
                    return self.name_atom(all);
                }
            }
            T::Eof => return self.refuse(),
        }
        Ok(Atom::Expr)
    }

    /// The last child pushed, a node, replaced by its children (a hidden rule's place).
    fn unwrap_last(&mut self) {
        let id = self.a.stack.pop().expect("a node");
        let n = self.a.nodes[id as usize];
        for i in n.at..n.at + n.n {
            let k = self.a.kids[i as usize];
            self.a.stack.push(k);
        }
    }

    /// `_array_elements`: `[a, b]`.
    fn array_elements(&mut self) -> R<()> {
        self.expect_punct(b"[", K::TLBracket)?;
        if !self.at_punct(b"]")? {
            loop {
                if self.at_punct(b"[")? {
                    return self.refuse();
                }
                self.expr(0, 0)?;
                if self.punct(b",", K::TComma)?.is_none() {
                    break;
                }
            }
        }
        self.expect_punct(b"]", K::TRBracket)?;
        Ok(())
    }

    /// The end of the dollar-quoted string at `s` (`$tag$…$tag$`), as the scanner reads it.
    pub(super) fn dollar_end(&self, s: usize) -> R<usize> {
        let mut e = s + 1;
        while self.byte(e) != b'$' {
            let c = self.byte(e);
            let ok = if e == s + 1 { c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 } else { c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80 };
            if !ok || e - s > 255 {
                return self.refuse();
            }
            e += 1;
        }
        let tag = &self.src[s..=e];
        let body = e + 1;
        let mut at = body;
        loop {
            let Some(i) = self.src[at..].iter().position(|&c| c == b'$') else { return self.refuse() };
            let d = at + i;
            if self.src[d..].starts_with(tag) {
                return Ok(d + tag.len());
            }
            at = d + 1;
        }
    }

    /// A string literal (`'…'`, `N'…'`) and its continuations (`'a' 'b'`: one literal).
    pub(super) fn string_literal(&mut self, field: u8) -> R<()> {
        let t = self.peek()?;
        self.take(t);
        let e = self.continued(t.e)?;
        let id = self.a.leaf(K::Literal, field, t.s, e);
        let _ = id;
        Ok(())
    }

    /// The end of a single-quoted string's continuations after `e` (`/'([^']|'')*'/*`).
    fn continued(&mut self, mut e: usize) -> R<usize> {
        loop {
            let n = self.peek()?;
            if n.t == T::Str && self.byte(n.s) == b'\'' {
                self.take(n);
                e = n.e;
            } else {
                return Ok(e);
            }
        }
    }

    /// `(…)` in an expression: a subquery, a list, or a parenthesized expression; pushed, its id.
    pub(super) fn paren(&mut self, all: bool) -> R<u32> {
        let _ = all;
        let m = self.a.mark();
        self.expect_punct(b"(", K::TLParen)?;
        if self.is(K::KeywordSelect)? || self.is(K::KeywordWith)? || self.is(K::KeywordValues)? {
            self.dml_read()?;
            self.expect_punct(b")", K::TRParen)?;
            return Ok(self.a.node(K::Subquery, m));
        }
        if self.at_punct(b"(")? {
            // `((select …) union …)`, `((a))`: read as an expression first.
        }
        if self.at_punct(b")")? {
            self.expect_punct(b")", K::TRParen)?;
            return Ok(self.a.node(K::List, m));
        }
        let first = self.expr_or_all(0, true)?;
        if self.a.nodes[first as usize].kind == K::AllFields {
            self.expect_punct(b")", K::TRParen)?;
            return Ok(self.a.node(K::ParenthesizedExpression, m));
        }
        if self.at_punct(b",")? {
            while self.punct(b",", K::TComma)?.is_some() {
                self.expr(0, 0)?;
            }
            self.expect_punct(b")", K::TRParen)?;
            return Ok(self.a.node(K::List, m));
        }
        self.expect_punct(b")", K::TRParen)?;
        Ok(self.a.node(K::ParenthesizedExpression, m))
    }

    /// `case … end`.
    fn case(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordCase)?;
        if !self.is(K::KeywordWhen)? {
            self.expr(0, 0)?;
        }
        if !self.is(K::KeywordWhen)? {
            return self.refuse();
        }
        while self.kw(K::KeywordWhen)?.is_some() {
            self.expr(0, 0)?;
            self.expect(K::KeywordThen)?;
            self.expr(0, 0)?;
        }
        if self.kw(K::KeywordElse)?.is_some() {
            self.expr(0, 0)?;
        }
        self.expect(K::KeywordEnd)?;
        self.a.node(K::Case, m);
        Ok(())
    }

    /// A name in an expression: a field (`a`, `t.a`, `s.t.a`), an invocation (`f(…)`, `s.f(…)`), or
    /// `t.*`.
    fn name_atom(&mut self, all: bool) -> R<Atom> {
        let m = self.a.mark();
        let first = self.expect_ident(Ctx::Sure(&[kinds::FIRST_EXPRESSION]), f::NAME)?;
        let mut parts = [first; 4];
        let mut n = 1;
        let mut star = false;
        while self.at_punct(b".")? {
            let t = self.peek()?;
            self.take(Tok { e: t.s + 1, ..t });
            if self.at_punct(b"*")? {
                star = true;
                break;
            }
            let Some(id) = self.ident(NONE, f::NAME)? else { return self.refuse() };
            if n == 4 {
                return self.refuse();
            }
            parts[n] = id;
            n += 1;
        }
        let parts = &parts[..n];
        self.a.stack.truncate(m);
        if star {
            if !all || parts.len() > 3 {
                return self.refuse();
            }
            self.push_reference(&parts, 0);
            let d = self.peek()?.s;
            let _ = d;
            let dot = self.last_end - 1;
            self.a.leaf(K::TDot, 0, dot, dot + 1);
            self.punct(b"*", K::TStar)?;
            self.a.node(K::AllFields, m);
            return Ok(Atom::AllFields);
        }
        if self.at_punct(b"(")? {
            if parts.len() > 3 {
                return self.refuse();
            }
            self.push_reference(&parts, 0);
            self.arguments()?;
            if self.is(K::KeywordWithin)? {
                return self.refuse();
            }
            if self.is(K::KeywordFilter)? {
                // `filter_expression`: `f(x) filter (where p)`, the invocation's last child.
                let fm = self.a.mark();
                self.expect(K::KeywordFilter)?;
                self.expect_punct(b"(", K::TLParen)?;
                self.where_()?;
                self.expect_punct(b")", K::TRParen)?;
                self.a.node(K::FilterExpression, fm);
            }
            self.a.node(K::Invocation, m);
            if self.kw(K::KeywordOver)?.is_some() {
                // `window_function`: `f(x) over w | (…)`.
                if self.at_punct(b"(")? {
                    self.window_specification()?;
                } else {
                    self.expect_ident(Ctx::Unsure(&[]), 0)?;
                }
                self.a.node(K::WindowFunction, m);
            }
            return Ok(Atom::Expr);
        }
        if self.peek()?.t == T::Str {
            // `_string_casting`: `name '…'` (`date '2020-01-01'`), a literal of the identifier and
            // the string.
            if parts.len() != 1 {
                return self.refuse();
            }
            self.a.nodes[parts[0] as usize].field = 0;
            self.a.stack.push(parts[0]);
            self.string_cast(m)?;
            return Ok(Atom::Expr);
        }
        // A field: `_qualified_field`, its last part the name, the parts before an object reference.
        match parts.len() {
            1 => {
                self.a.stack.push(parts[0]);
            }
            2 | 3 => {
                let (head, name) = parts.split_at(parts.len() - 1);
                self.push_reference(head, 0);
                let d = self.dot_before(name[0]);
                self.a.leaf(K::TDot, 0, d, d + 1);
                self.a.nodes[name[0] as usize].field = f::NAME;
                self.a.stack.push(name[0]);
            }
            _ => return self.refuse(),
        }
        self.a.node(K::Field, m);
        Ok(Atom::Expr)
    }

    /// An invocation's arguments: `(…)`, each a term (`parameter:`) or a named argument.
    fn arguments(&mut self) -> R<()> {
        self.expect_punct(b"(", K::TLParen)?;
        if self.punct(b")", K::TRParen)?.is_some() {
            return Ok(());
        }
        if self.is(K::KeywordVariadic)? || self.is(K::KeywordAll)? {
            return self.refuse();
        }
        if self.unit_next()? {
            // `extract(epoch from t)`: `unit: object_reference`, `from`, a term, by commas. Where it
            // does not hold (`substring(a from 1 for 2)`), the `from` form below.
            let saved = self.save();
            if self.unit_arguments().is_ok() {
                return Ok(());
            }
            self.restore(saved);
        }
        if self.is(K::KeywordBoth)? || self.is(K::KeywordLeading)? || self.is(K::KeywordTrailing)? {
            // `trim(both [x] from y)`.
            let ws = self.words()?;
            let k = ws.iter().find(|&&(_, k)| matches!(k, K::KeywordBoth | K::KeywordLeading | K::KeywordTrailing)).expect("one").1;
            self.expect(k)?;
            if !self.is(K::KeywordFrom)? {
                let id = self.term()?;
                self.a.set_field(id, f::PARAMETER);
            }
            self.expect(K::KeywordFrom)?;
            let id = self.term()?;
            self.a.set_field(id, f::PARAMETER);
            self.expect_punct(b")", K::TRParen)?;
            return Ok(());
        }
        let mut first = true;
        loop {
            // A named argument: `name => value`.
            let t = self.peek()?;
            if t.t == T::Word || t.t == T::Quoted {
                let save = (self.at, self.last_end, self.look);
                self.take(t);
                let named = self.at_punct(b"=>")? || self.at_punct(b":=")?;
                (self.at, self.last_end, self.look) = save;
                if named {
                    let m = self.a.mark();
                    self.expect_ident(NONE, f::NAME)?;
                    let n = self.peek()?;
                    let k = if self.byte(n.s) == b'=' { K::TFatArrow } else { K::TColonEq };
                    self.take(Tok { e: n.s + 2, ..n });
                    self.a.leaf(k, 0, n.s, n.s + 2);
                    self.expr(0, f::VALUE)?;
                    let id = self.a.node(K::NamedArgument, m);
                    self.a.set_field(id, f::PARAMETER);
                    if self.punct(b",", K::TComma)?.is_none() {
                        break;
                    }
                    continue;
                }
            }
            // `count(distinct a)`, `string_agg(a, ', ' order by a)`: `distinct` before an argument,
            // an `order_by` after it.
            let distinct = self.kw(K::KeywordDistinct)?.is_some();
            if self.is(K::KeywordVariadic)? {
                return self.refuse();
            }
            let id = self.term()?;
            self.a.set_field(id, f::PARAMETER);
            if first && !distinct && (self.is(K::KeywordFrom)? || self.is(K::KeywordFor)? || self.is(K::KeywordIn)? || self.is(K::KeywordPlacing)?) {
                // `substring(a [placing b] from i [for n])`, `substring(a for n [from i])`,
                // `position(a in b)`.
                if self.kw(K::KeywordIn)?.is_some() {
                    let id = self.term()?;
                    self.a.set_field(id, f::PARAMETER);
                } else if self.kw(K::KeywordFor)?.is_some() {
                    let id = self.term()?;
                    self.a.set_field(id, f::PARAMETER);
                    if self.kw(K::KeywordFrom)?.is_some() {
                        let id = self.term()?;
                        self.a.set_field(id, f::PARAMETER);
                    }
                } else {
                    if self.kw(K::KeywordPlacing)?.is_some() {
                        let id = self.term()?;
                        self.a.set_field(id, f::PARAMETER);
                    }
                    self.expect(K::KeywordFrom)?;
                    let id = self.term()?;
                    self.a.set_field(id, f::PARAMETER);
                    if self.kw(K::KeywordFor)?.is_some() {
                        let id = self.term()?;
                        self.a.set_field(id, f::PARAMETER);
                    }
                }
                break;
            }
            first = false;
            if self.is(K::KeywordOrder)? {
                self.order_by()?;
            }
            // A string after a comma is a term too: tree-sitter takes the general form over the last
            // one's `_expression_string` (`f(a, 'b')`; the oracle's corpus has no other reading).
            if self.punct(b",", K::TComma)?.is_none() {
                break;
            }
        }
        self.expect_punct(b")", K::TRParen)?;
        Ok(())
    }

    /// `unit from term, …)`.
    fn unit_arguments(&mut self) -> R<()> {
        loop {
            if !self.unit_next()? || self.object_reference(NONE, f::UNIT)?.is_none() {
                return self.refuse();
            }
            self.expect(K::KeywordFrom)?;
            self.term()?;
            if self.punct(b",", K::TComma)?.is_none() {
                break;
            }
        }
        self.expect_punct(b")", K::TRParen).map(|_| ())
    }

    /// Whether a name and `from` are next (an invocation's `unit from term`).
    fn unit_next(&mut self) -> R<bool> {
        let t = self.peek()?;
        if t.t != T::Word && t.t != T::Quoted {
            return Ok(false);
        }
        let save = (self.at, self.last_end, self.look);
        self.take(t);
        let from = self.is(K::KeywordFrom)?;
        (self.at, self.last_end, self.look) = save;
        Ok(from)
    }

    /// A term: an expression or `*` (`value:`), and its alias.
    pub(super) fn term(&mut self) -> R<u32> {
        let m = self.a.mark();
        let v = self.expr_or_all(0, true)?;
        self.a.set_field(v, f::VALUE);
        self.alias()?;
        Ok(self.a.node(K::Term, m))
    }

    /// `_alias`: `as name`, `as 'name'`, or a bare name where the next word is no keyword that may
    /// follow.
    pub(super) fn alias(&mut self) -> R<()> {
        if self.kw(K::KeywordAs)?.is_some() {
            let t = self.peek()?;
            if t.t == T::Str && self.byte(t.s) == b'\'' {
                self.take(t);
                self.a.leaf(K::Literal, f::ALIAS, t.s, t.e);
                return Ok(());
            }
            self.expect_ident(NONE, f::ALIAS)?;
            return Ok(());
        }
        self.ident(Ctx::Unsure(&[AFTER_TERM]), f::ALIAS)?;
        Ok(())
    }

    // ---- Types.

    /// `_type`, its nodes given `field`.
    pub(super) fn type_(&mut self, field: u8) -> R<()> {
        let m = self.a.mark();
        let t = self.peek()?;
        let ws = self.words()?;
        let k = ws.iter().map(|&(_, k)| k).find(|k| kinds::FIRST_TYPE.contains(k));
        if k.is_none() && t.t == T::Word && self.src[t.s..t.e].eq_ignore_ascii_case(b"float8") {
            // `double`'s own pattern (`/[fF][lL][oO][aA][tT][88]/`): a `double` with no keyword in it.
            self.take(t);
            self.a.leaf(K::Double, 0, t.s, t.e);
            self.a.field_since(m, field);
            return Ok(());
        }
        match k {
            None => {
                // A custom type: an object reference.
                if t.t != T::Word && t.t != T::Quoted {
                    return self.refuse();
                }
                self.object_reference(NONE, f::CUSTOM_TYPE)?;
            }
            Some(k) => self.builtin_type(k)?,
        }
        // `array_size_definition`: `[]`, `[n]`, `array`.
        if self.at_punct(b"[")? {
            let n = self.a.mark();
            while self.punct(b"[", K::TLBracket)?.is_some() {
                let t = self.peek()?;
                if t.t == T::Num {
                    self.take(t);
                    self.a.leaf(K::Literal, f::SIZE, t.s, t.e);
                }
                self.expect_punct(b"]", K::TRBracket)?;
            }
            self.a.node(K::ArraySizeDefinition, n);
        } else if self.is(K::KeywordArray)? {
            return self.refuse();
        }
        self.a.field_since(m, field);
        Ok(())
    }

    /// A built-in type, its first keyword `k`.
    /// The string of a `_string_casting` (one `'…'`, no continuation), then the literal of the
    /// children since `m` and it.
    fn string_cast(&mut self, m: usize) -> R<()> {
        let n = self.peek()?;
        if n.t != T::Str || self.byte(n.s) != b'\'' {
            return self.refuse();
        }
        self.take(n);
        if self.peek()?.t == T::Str {
            return self.refuse();
        }
        let id = self.a.node(K::Literal, m);
        self.a.nodes[id as usize].e = n.e as u32;
        Ok(())
    }

    fn builtin_type(&mut self, k: K) -> R<()> {
        let m = self.a.mark();
        // A size in parentheses, as the type's field `name` (`size`, `precision`).
        let sized = |p: &mut Self, name: u8| -> R<bool> {
            if !p.at_punct(b"(")? {
                return Ok(false);
            }
            p.expect_punct(b"(", K::TLParen)?;
            let t = p.peek()?;
            if t.t != T::Num || !p.src[t.s..t.e].iter().all(u8::is_ascii_digit) {
                return p.refuse();
            }
            p.take(t);
            p.a.leaf(K::Literal, name, t.s, t.e);
            p.expect_punct(b")", K::TRParen)?;
            Ok(true)
        };
        // The word `character` is `keyword_char`, or the start of `character varying`.
        let k = if k == K::KeywordChar && self.is(K::KeywordCharacter)? { K::KeywordCharacter } else { k };
        let node = match k {
            K::KeywordInt | K::KeywordSmallint | K::KeywordBigint | K::KeywordTinyint | K::KeywordMediumint => {
                self.expect(k)?;
                sized(self, f::SIZE)?;
                if self.is(K::KeywordUnsigned)? || self.is(K::KeywordZerofill)? {
                    return self.refuse();
                }
                Some(match k {
                    K::KeywordInt => K::Int,
                    K::KeywordSmallint => K::Smallint,
                    K::KeywordBigint => K::Bigint,
                    K::KeywordTinyint => K::Tinyint,
                    _ => K::Mediumint,
                })
            }
            K::KeywordDecimal | K::KeywordNumeric => {
                self.expect(k)?;
                if self.punct(b"(", K::TLParen)?.is_some() {
                    let t = self.peek()?;
                    if t.t != T::Num {
                        return self.refuse();
                    }
                    self.take(t);
                    self.a.leaf(K::Literal, f::PRECISION, t.s, t.e);
                    if self.punct(b",", K::TComma)?.is_some() {
                        let t = self.peek()?;
                        if t.t != T::Num {
                            return self.refuse();
                        }
                        self.take(t);
                        self.a.leaf(K::Literal, f::SCALE, t.s, t.e);
                    }
                    self.expect_punct(b")", K::TRParen)?;
                }
                Some(if k == K::KeywordDecimal { K::Decimal } else { K::Numeric })
            }
            K::KeywordChar | K::KeywordNchar | K::KeywordNvarchar => {
                self.expect(k)?;
                sized(self, f::SIZE)?;
                Some(match k {
                    K::KeywordChar => K::Char,
                    K::KeywordNchar => K::Nchar,
                    _ => K::Nvarchar,
                })
            }
            K::KeywordVarchar => {
                self.expect(k)?;
                sized(self, f::SIZE)?;
                Some(K::Varchar)
            }
            K::KeywordCharacter => {
                // `character varying`: one `keyword_varchar` over both words, `keyword_varying` in it
                // (the token's second form). `character` alone is `keyword_char`.
                let t = self.peek()?;
                let save = (self.at, self.last_end, self.look);
                self.take(t);
                let varying = self.is(K::KeywordVarying)?;
                (self.at, self.last_end, self.look) = save;
                if varying {
                    let w = self.a.mark();
                    self.take(t);
                    let v = self.expect(K::KeywordVarying)?;
                    let e = self.a.nodes[v as usize].e as usize;
                    self.node_span(K::KeywordVarchar, w, t.s, e);
                    sized(self, f::SIZE)?;
                    Some(K::Varchar)
                } else {
                    self.expect(K::KeywordChar)?;
                    sized(self, f::SIZE)?;
                    Some(K::Char)
                }
            }
            K::KeywordDouble => {
                self.expect(k)?;
                if self.is(K::KeywordPrecision)? {
                    self.expect(K::KeywordPrecision)?;
                }
                if self.at_punct(b"(")? {
                    return self.refuse();
                }
                Some(K::Double)
            }
            K::KeywordReal => {
                self.expect(k)?;
                if self.at_punct(b"(")? {
                    return self.refuse();
                }
                Some(K::Double)
            }
            K::KeywordFloat => {
                self.expect(k)?;
                sized(self, f::PRECISION)?;
                Some(K::Float)
            }
            K::KeywordTime | K::KeywordTimestamp => {
                self.expect(k)?;
                sized(self, f::SIZE)?;
                if self.is(K::KeywordWith)? || self.is(K::KeywordWithout)? {
                    let save = (self.at, self.last_end, self.look);
                    let t = self.peek()?;
                    self.take(t);
                    let time = self.is(K::KeywordTime)?;
                    (self.at, self.last_end, self.look) = save;
                    if time {
                        let w = if self.is(K::KeywordWith)? { K::KeywordWith } else { K::KeywordWithout };
                        self.expect(w)?;
                        self.expect(K::KeywordTime)?;
                        self.expect(K::KeywordZone)?;
                    }
                }
                Some(if k == K::KeywordTime { K::Time } else { K::Timestamp })
            }
            K::KeywordBit | K::KeywordBinary | K::KeywordVarbinary | K::KeywordEnum | K::KeywordDatetimeoffset | K::KeywordUnsigned => return self.refuse(),
            _ => {
                // A type that is its keyword alone.
                self.expect(k)?;
                None
            }
        };
        if let Some(node) = node {
            self.a.node(node, m);
        }
        Ok(())
    }
}
