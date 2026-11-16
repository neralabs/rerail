//! Expressions: precedence climbing over tree-sitter-c's `PREC` (assignment and the conditional
//! right-associative, below the binary operators), unary and postfix operators, casts, compound
//! literals, `sizeof`, literals and strings, initializer lists.

use super::*;

// Precedences, tree-sitter-c's shifted by 3 so that all are positive.
const ASSIGN: u8 = 1;
const COND: u8 = 2;

/// A binary operator's precedence and its token's kind.
pub(super) fn binary(t: T) -> Option<(u8, K)> {
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
        Le => (10, K::TLe),
        Gt => (10, K::TGt),
        Ge => (10, K::TGe),
        LtLt => (12, K::TLtLt),
        GtGt => (12, K::TGtGt),
        Plus => (13, K::TPlus),
        Minus => (13, K::TMinus),
        Star => (14, K::TStar),
        Slash => (14, K::TSlash),
        Percent => (14, K::TPercent),
        _ => return None,
    })
}

fn assignment(t: T) -> Option<K> {
    use T::*;
    Some(match t {
        Eq => K::TEq,
        StarEq => K::TStarEq,
        SlashEq => K::TSlashEq,
        PercentEq => K::TPercentEq,
        PlusEq => K::TPlusEq,
        MinusEq => K::TMinusEq,
        LtLtEq => K::TLtLtEq,
        GtGtEq => K::TGtGtEq,
        AmpEq => K::TAmpEq,
        CaretEq => K::TCaretEq,
        PipeEq => K::TPipeEq,
        _ => return None,
    })
}

impl<'a> P<'a> {
    /// An expression in slot `s`.
    pub(super) fn expr(&mut self, s: Slot) -> R<N> {
        self.expr_prec(s, ASSIGN)
    }

    /// An expression, or a `comma_expression` (`a, b, c`: right-nested).
    pub(super) fn comma_expr(&mut self, s: Slot) -> R<N> {
        let e = self.expr(s)?;
        if self.peek() != T::Comma {
            return Ok(e);
        }
        // `comma_expression` is in no supertype.
        let n = self.wrap(e, K::CommaExpression, s.with_ctx(Ctx::Bare), slot(f::LEFT, Ctx::Expr));
        self.tk(K::CommaExpression, K::TComma)?;
        self.comma_expr(slot(f::RIGHT, Ctx::Expr))?;
        self.close(n);
        Ok(n)
    }

    fn expr_prec(&mut self, s: Slot, min: u8) -> R<N> {
        self.deeper()?;
        let mut left = self.unary(s)?;
        loop {
            let t = self.tok();
            if let Some((p, op)) = binary(t.kind) {
                if p < min {
                    break;
                }
                let n = self.wrap(left, K::BinaryExpression, s, slot(f::LEFT, Ctx::Expr));
                self.leaf(op, f::OPERATOR, C_NONE)?;
                self.expr_prec(slot(f::RIGHT, Ctx::Expr), p + 1)?;
                self.close(n);
                left = n;
                continue;
            }
            if t.kind == T::Question {
                if COND < min {
                    break;
                }
                let n = self.wrap(left, K::ConditionalExpression, s, slot(f::CONDITION, Ctx::Expr));
                self.tk(K::ConditionalExpression, K::TQuestion)?;
                if self.peek() != T::Colon {
                    self.comma_expr(slot(f::CONSEQUENCE, Ctx::Expr))?;
                }
                self.punct_tk(K::ConditionalExpression, T::Colon, false)?;
                self.expr_prec(slot(f::ALTERNATIVE, Ctx::Expr), COND)?;
                self.close(n);
                left = n;
                continue;
            }
            if let Some(op) = assignment(t.kind) {
                // An assignment binds an assignable operand of a tighter operator too: `a * b = c`
                // is `a * (b = c)` (tree-sitter's only valid reading).
                if ASSIGN < min && !Self::assignable(left.kind) {
                    break;
                }
                if !Self::assignable(left.kind) {
                    return self.refuse();
                }
                // `_assignment_left_expression` is inlined: the left is in no supertype.
                let n = self.wrap(left, K::AssignmentExpression, s, slot(f::LEFT, Ctx::Bare));
                self.leaf(op, f::OPERATOR, C_NONE)?;
                self.expr_prec(slot(f::RIGHT, Ctx::Expr), ASSIGN)?;
                self.close(n);
                left = n;
                continue;
            }
            break;
        }
        self.depth -= 1;
        Ok(left)
    }

    /// Whether an expression of `kind` can be assigned to (`_assignment_left_expression`).
    fn assignable(kind: K) -> bool {
        matches!(kind, K::Identifier | K::CallExpression | K::FieldExpression | K::PointerExpression | K::SubscriptExpression | K::ParenthesizedExpression)
    }

    /// `-1`, `+1`: one `number_literal` where the sign touches the number.
    pub(super) fn signed_number(&mut self, s: Slot) -> R<N> {
        let sign = self.bump()?;
        let num = self.bump()?;
        let n = self.w.push(key(K::NumberLiteral, s.field, class(K::NumberLiteral, s.ctx), false), sign.start);
        self.w.close(n, num.end);
        Ok(N { at: n, kind: K::NumberLiteral })
    }

    /// A unary expression: prefix operators, casts, `sizeof`, or a postfix chain.
    fn unary(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let r = self.unary_inner(s);
        self.depth -= 1;
        r
    }

    fn unary_inner(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Bang | T::Tilde | T::Minus | T::Plus => {
                if matches!(t.kind, T::Minus | T::Plus) && self.kind_at(1) == T::Number && self.peek_at(1).start == t.end {
                    let n = self.signed_number(s)?;
                    return self.postfix(n, s, true);
                }
                let n = self.open_in(K::UnaryExpression, s);
                self.leaf(punct(t.kind).expect("an operator"), f::OPERATOR, C_NONE)?;
                // The operand's postfix `++`/`--` binds after the operator (equal precedence,
                // the operator's left associativity): `-a++` is `(-a)++`.
                let a = self.unary_operand(slot(f::ARGUMENT, Ctx::Expr))?;
                let _ = a;
                self.close(n);
                self.update_after(n, s)
            }
            T::Star | T::Amp => {
                let n = self.open_in(K::PointerExpression, s);
                self.leaf(punct(t.kind).expect("an operator"), f::OPERATOR, C_NONE)?;
                self.unary(slot(f::ARGUMENT, Ctx::Expr))?;
                self.close(n);
                Ok(n)
            }
            T::PlusPlus | T::MinusMinus => {
                let n = self.open_in(K::UpdateExpression, s);
                self.leaf(punct(t.kind).expect("an operator"), f::OPERATOR, C_NONE)?;
                self.unary(slot(f::ARGUMENT, Ctx::Expr))?;
                self.close(n);
                Ok(n)
            }
            T::Name if t.word == w::SIZEOF => {
                let n = self.open_in(K::SizeofExpression, s);
                self.kw(K::SizeofExpression, w::SIZEOF)?;
                if self.peek() == T::LParen && self.cast_ahead(0, false) {
                    self.tk(K::SizeofExpression, K::TLParen)?;
                    self.type_descriptor(slot(f::TYPE, Ctx::Bare))?;
                    self.punct_tk(K::SizeofExpression, T::RParen, false)?;
                } else {
                    self.unary(slot(f::VALUE, Ctx::Expr))?;
                }
                self.close(n);
                Ok(n)
            }
            T::Name if matches!(t.word, w::U_ALIGNOF_U | w::U_ALIGNOF | w::ALIGNOF_ | w::ALIGNOF | w::ALIGNOF_C) && self.kind_at(1) == T::LParen => {
                let n = self.open_in(K::AlignofExpression, s);
                self.kw_any(K::AlignofExpression)?;
                self.tk(K::AlignofExpression, K::TLParen)?;
                self.type_descriptor(slot(f::TYPE, Ctx::Bare))?;
                self.punct_tk(K::AlignofExpression, T::RParen, false)?;
                self.close(n);
                Ok(n)
            }
            T::Name if t.word == w::U_EXTENSION_U => {
                let n = self.open_in(K::ExtensionExpression, s);
                self.tk(K::ExtensionExpression, K::T__extension__)?;
                self.expr(EXPR)?;
                self.close(n);
                Ok(n)
            }
            T::LParen if self.cast_ahead(0, true) => {
                let e = self.paren_end(0).unwrap_or(0);
                if self.kind_at(e + 1) == T::LBrace {
                    let n = self.open_in(K::CompoundLiteralExpression, s);
                    self.tk(K::CompoundLiteralExpression, K::TLParen)?;
                    self.type_descriptor(slot(f::TYPE, Ctx::Bare))?;
                    self.punct_tk(K::CompoundLiteralExpression, T::RParen, false)?;
                    self.initializer_list(slot(f::VALUE, Ctx::Bare))?;
                    self.close(n);
                    return self.postfix(n, s, true);
                }
                let n = self.open_in(K::CastExpression, s);
                self.tk(K::CastExpression, K::TLParen)?;
                self.type_descriptor(slot(f::TYPE, Ctx::Bare))?;
                self.punct_tk(K::CastExpression, T::RParen, false)?;
                self.unary(slot(f::VALUE, Ctx::Expr))?;
                self.close(n);
                Ok(n)
            }
            _ => {
                let p = self.primary(s)?;
                self.postfix(p, s, true)
            }
        }
    }

    /// A `! ~ - +` operator's operand: a unary expression whose postfix chain stops before `++`
    /// and `--`.
    fn unary_operand(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Bang | T::Tilde | T::Minus | T::Plus | T::Star | T::Amp | T::PlusPlus | T::MinusMinus | T::LParen => self.unary(s),
            T::Name if matches!(t.word, w::SIZEOF | w::U_EXTENSION_U) => self.unary(s),
            _ => {
                let p = self.primary(s)?;
                self.postfix(p, s, false)
            }
        }
    }

    /// Postfix `++`/`--` after a unary expression `n`.
    fn update_after(&mut self, mut n: N, s: Slot) -> R<N> {
        while matches!(self.peek(), T::PlusPlus | T::MinusMinus) {
            let u = self.wrap(n, K::UpdateExpression, s, slot(f::ARGUMENT, Ctx::Expr));
            let t = self.tok();
            self.leaf(punct(t.kind).expect("an operator"), f::OPERATOR, C_NONE)?;
            self.close(u);
            n = u;
        }
        Ok(n)
    }

    /// Whether `(` at token `k` opens a cast or a compound literal (`expr`: an operand follows), or
    /// `sizeof`'s type: a type in the parentheses, read by its first tokens; `(name)` is a type
    /// where an operand that cannot continue an expression follows.
    fn cast_ahead(&mut self, k: usize, expr: bool) -> bool {
        if self.type_in_parens_ahead(k) {
            return true;
        }
        // `(T[]){…}`, `(T[n]){…}`: a compound literal's array type.
        if expr && self.is_ident_at(k + 1) && self.kind_at(k + 2) == T::LBracket {
            if let Some(e) = self.paren_end(k + 2) {
                if self.kind_at(e + 1) == T::RParen && self.kind_at(e + 2) == T::LBrace {
                    return true;
                }
            }
        }
        // `(T)`: a name alone.
        let t = self.peek_at(k + 1);
        if t.kind != T::Name || self.reserved(t.word) || self.kind_at(k + 2) != T::RParen {
            return false;
        }
        if !expr {
            return false;
        }
        let u = self.peek_at(k + 3);
        match u.kind {
            T::Name => !matches!(u.word, w::U_ATTRIBUTE_U),
            // `(T)(x)` is a call of a parenthesized name.
            T::Number | T::Char | T::Str | T::RawStr | T::Bang | T::Tilde | T::LBrace => true,
            T::PlusPlus | T::MinusMinus => true,
            _ => false,
        }
    }

    /// A postfix chain after `n`: calls, subscripts, fields, and `++`/`--` where `update`.
    fn postfix(&mut self, mut n: N, s: Slot, update: bool) -> R<N> {
        loop {
            let t = self.tok();
            match t.kind {
                T::LParen => {
                    let c = self.wrap(n, K::CallExpression, s, slot(f::FUNCTION, Ctx::Expr));
                    self.arguments(slot(f::ARGUMENTS, Ctx::Bare))?;
                    self.close(c);
                    n = c;
                }
                T::LBracket => {
                    let c = self.wrap(n, K::SubscriptExpression, s, slot(f::ARGUMENT, Ctx::Expr));
                    self.tk(K::SubscriptExpression, K::TLBracket)?;
                    self.comma_or_expr(slot(f::INDEX, Ctx::Expr))?;
                    self.punct_tk(K::SubscriptExpression, T::RBracket, false)?;
                    self.close(c);
                    n = c;
                }
                T::Dot | T::Arrow => {
                    let c = self.wrap(n, K::FieldExpression, s, slot(f::ARGUMENT, Ctx::Expr));
                    self.leaf(punct(t.kind).expect("an operator"), f::OPERATOR, C_NONE)?;
                    if !self.is_ident() && self.peek() != T::Name {
                        return self.refuse();
                    }
                    self.leaf(K::FieldIdentifier, f::FIELD, C_NONE)?;
                    self.close(c);
                    n = c;
                }
                T::PlusPlus | T::MinusMinus if update => {
                    let c = self.wrap(n, K::UpdateExpression, s, slot(f::ARGUMENT, Ctx::Expr));
                    self.leaf(punct(t.kind).expect("an operator"), f::OPERATOR, C_NONE)?;
                    self.close(c);
                    n = c;
                }
                _ => return Ok(n),
            }
        }
    }

    fn comma_or_expr(&mut self, s: Slot) -> R<N> {
        self.expr(s)
    }

    /// A primary expression.
    fn primary(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Number => self.leaf_in(K::NumberLiteral, s),
            T::Char => self.char_literal(s),
            T::Str => self.strings(s),
            T::LParen => self.paren_expr(s),
            T::Name => match t.word {
                w::TRUE | w::TRUE_C => self.leaf_in(K::True, s),
                w::FALSE | w::FALSE_C => self.leaf_in(K::False, s),
                w::NULL | w::NULLPTR => {
                    let n = self.open_in(K::Null, s);
                    self.kw_any(K::Null)?;
                    self.close(n);
                    Ok(n)
                }
                w::OFFSETOF if self.kind_at(1) == T::LParen => {
                    let n = self.open_in(K::OffsetofExpression, s);
                    self.kw(K::OffsetofExpression, w::OFFSETOF)?;
                    self.tk(K::OffsetofExpression, K::TLParen)?;
                    self.type_descriptor(slot(f::TYPE, Ctx::Bare))?;
                    self.punct_tk(K::OffsetofExpression, T::Comma, false)?;
                    self.leaf(K::FieldIdentifier, f::MEMBER, C_NONE)?;
                    self.punct_tk(K::OffsetofExpression, T::RParen, false)?;
                    self.close(n);
                    Ok(n)
                }
                _ if self.is_asm() => self.gnu_asm(s),
                // A type's or a qualifier's word where no type can be is a name (keyword
                // extraction: `va_arg(ap, int64_t)`, `__attribute__((noreturn))`).
                c if self.reserved(c) && !(is_primitive(c) || super::decl::is_qualifier(c) || super::decl::is_ms_call(c) || super::decl::is_ms_pointer(c)) => self.refuse(),
                _ => {
                    if self.kind_at(1) == T::Str {
                        return self.strings(s);
                    }
                    self.leaf_in(K::Identifier, s)
                }
            },
            _ => self.refuse(),
        }
    }

    /// `(` an expression, a comma expression or a block `)` in slot `s`.
    pub(super) fn paren_expr(&mut self, s: Slot) -> R<N> {
        if self.peek() != T::LParen {
            return self.refuse();
        }
        let n = self.open_in(K::ParenthesizedExpression, s);
        self.tk(K::ParenthesizedExpression, K::TLParen)?;
        if self.peek() == T::LBrace {
            self.compound(BARE)?;
        } else {
            self.comma_expr(EXPR)?;
        }
        self.punct_tk(K::ParenthesizedExpression, T::RParen, false)?;
        self.close(n);
        Ok(n)
    }

    /// An `argument_list`: `(` expressions (or blocks, initializer lists) `)`.
    pub(super) fn arguments(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ArgumentList, s);
        self.list(K::ArgumentList, T::RParen, true, false, |p| {
            match p.peek() {
                T::LBrace if p.cpp => {
                    p.initializer_list(BARE)?;
                }
                T::LBrace => {
                    p.compound(BARE)?;
                }
                _ => {
                    p.expr(EXPR)?;
                }
            }
            Ok(())
        })?;
        self.close(n);
        Ok(n)
    }

    /// `{` initializers `}`: expressions, designated pairs, nested lists; a trailing `,`.
    pub(super) fn initializer_list(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::InitializerList, s);
        self.list(K::InitializerList, T::RBrace, true, true, |p| p.initializer())?;
        self.close(n);
        Ok(n)
    }

    fn initializer(&mut self) -> R<()> {
        let t = self.tok();
        let designated = match t.kind {
            T::Dot => true,
            T::LBracket => true,
            T::Name if self.kind_at(1) == T::Colon => true,
            _ => false,
        };
        if !designated {
            if t.kind == T::LBrace {
                self.initializer_list(BARE)?;
            } else {
                self.expr(EXPR)?;
            }
            return Ok(());
        }
        let n = self.open(K::InitializerPair, f::NONE, C_NONE);
        if t.kind == T::Name {
            self.leaf(K::FieldIdentifier, f::DESIGNATOR, C_NONE)?;
            self.tk(K::InitializerPair, K::TColon)?;
        } else {
            loop {
                match self.peek() {
                    T::Dot => {
                        let d = self.open(K::FieldDesignator, f::DESIGNATOR, C_NONE);
                        self.tk(K::FieldDesignator, K::TDot)?;
                        self.leaf(K::FieldIdentifier, f::NONE, C_NONE)?;
                        self.close(d);
                    }
                    T::LBracket => {
                        let range = self.range_designator_ahead();
                        let kind = if range { K::SubscriptRangeDesignator } else { K::SubscriptDesignator };
                        let d = self.open(kind, f::DESIGNATOR, C_NONE);
                        self.tk(kind, K::TLBracket)?;
                        if range {
                            self.expr(slot(f::START, Ctx::Expr))?;
                            self.punct_tk(kind, T::DotDotDot, false)?;
                            self.expr(slot(f::END, Ctx::Expr))?;
                        } else {
                            self.expr(EXPR)?;
                        }
                        self.punct_tk(kind, T::RBracket, false)?;
                        self.close(d);
                    }
                    _ => break,
                }
            }
            self.punct_tk(K::InitializerPair, T::Eq, false)?;
        }
        if self.peek() == T::LBrace {
            self.initializer_list(slot(f::VALUE, Ctx::Bare))?;
        } else {
            self.expr(slot(f::VALUE, Ctx::Expr))?;
        }
        self.close(n);
        Ok(())
    }

    /// Whether `[` ahead holds `a ... b`.
    fn range_designator_ahead(&mut self) -> bool {
        let Some(e) = self.paren_end(0) else { return false };
        (1..e).any(|k| self.kind_at(k) == T::DotDotDot)
    }

    // ---- Literals.

    /// Strings and names side by side: a `concatenated_string`, or one string.
    pub(super) fn strings(&mut self, s: Slot) -> R<N> {
        let piece = |p: &mut Self, k: usize| matches!(p.kind_at(k), T::Str | T::RawStr) || p.kind_at(k) == T::Name && !{ let c = p.peek_at(k).word; p.reserved(c) };
        let mut k = 0;
        while piece(self, k) {
            k += 1;
        }
        let strings = (0..k).filter(|&j| matches!(self.kind_at(j), T::Str | T::RawStr)).count();
        if k < 2 || strings == 0 {
            if self.peek() != T::Str {
                return self.refuse();
            }
            return self.string(s);
        }
        let n = self.open_in(K::ConcatenatedString, s);
        for _ in 0..k {
            match self.peek() {
                T::Str => {
                    self.string(BARE)?;
                }
                T::Name => {
                    self.leaf(K::Identifier, f::NONE, C_NONE)?;
                }
                _ => return self.refuse(),
            }
        }
        self.close(n);
        Ok(n)
    }

    /// A `string_literal` in slot `s`: its opener (`"`, `L"` …) stored, its content runs and
    /// escapes, its closer stored where the string is empty.
    pub(super) fn string(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        if t.kind != T::Str {
            return self.refuse();
        }
        self.bump()?;
        let (a, b) = (t.start as usize, t.end as usize);
        let n = self.w.push(key(K::StringLiteral, s.field, class(K::StringLiteral, s.ctx), false), t.start);
        let q = a + self.src[a..b].iter().position(|&c| c == b'"').expect("a quote");
        let opener = match &self.src[a..=q] {
            b"\"" => K::TDQuote,
            b"L\"" => K::TLDQuote,
            b"u\"" => K::TUSmallDQuote,
            b"U\"" => K::TUDQuote,
            b"u8\"" => K::TU8DQuote,
            _ => return self.refuse(),
        };
        self.piece(opener, a, q + 1);
        let mut i = q + 1;
        let end = b - 1;
        while i < end {
            if self.src[i] == b'\\' {
                let e = self.lx.escape_end(i).filter(|&e| e <= end).ok_or(Refusal::Lex(i as u32))?;
                self.piece(K::EscapeSequence, i, e);
                i = e;
            } else {
                let mut e = i;
                while e < end && self.src[e] != b'\\' {
                    e += 1;
                }
                self.piece(K::StringContent, i, e);
                i = e;
            }
        }
        if q + 1 == end {
            self.piece(K::TDQuote, end, b);
        }
        self.w.close(n, t.end);
        Ok(N { at: n, kind: K::StringLiteral })
    }

    /// A leaf inside a literal, over `a..b`.
    fn piece(&mut self, kind: K, a: usize, b: usize) {
        let n = self.w.push(key(kind, f::NONE, C_NONE, false), a as u32);
        self.w.close(n, b as u32);
    }

    /// A `char_literal` in slot `s`: its opener stored, a `character` a character or an escape.
    pub(super) fn char_literal(&mut self, s: Slot) -> R<N> {
        let t = self.bump()?;
        let (a, b) = (t.start as usize, t.end as usize);
        let n = self.w.push(key(K::CharLiteral, s.field, class(K::CharLiteral, s.ctx), false), t.start);
        let q = a + self.src[a..b].iter().position(|&c| c == b'\'').expect("a quote");
        let opener = match &self.src[a..=q] {
            b"'" => K::TQuote,
            b"L'" => K::TLQuote,
            b"u'" => K::TUSmallQuote,
            b"U'" => K::TUQuote,
            b"u8'" => K::TU8Quote,
            _ => return self.refuse(),
        };
        self.piece(opener, a, q + 1);
        let mut i = q + 1;
        let end = b - 1;
        while i < end {
            if self.src[i] == b'\\' {
                let e = self.lx.escape_end(i).filter(|&e| e <= end).ok_or(Refusal::Lex(i as u32))?;
                self.piece(K::EscapeSequence, i, e);
                i = e;
            } else {
                let len = match self.src[i] {
                    c if c < 0x80 => 1,
                    c if c >= 0xf0 => 4,
                    c if c >= 0xe0 => 3,
                    _ => 2,
                };
                self.piece(K::Character, i, (i + len).min(end));
                i += len;
            }
        }
        self.w.close(n, t.end);
        Ok(N { at: n, kind: K::CharLiteral })
    }

    // ---- GNU assembly.

    pub(super) fn is_asm(&mut self) -> bool {
        let t = self.tok();
        t.kind == T::Name && matches!(t.word, w::ASM | w::U_ASM_U | w::U_ASM) && matches!(self.kind_at(1), T::LParen | T::Name)
    }

    /// `asm qualifiers ( code : outputs : inputs : clobbers : labels )`.
    pub(super) fn gnu_asm(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::GnuAsmExpression, s);
        self.kw_any(K::GnuAsmExpression)?;
        // The `(` is stored right after the keyword, dropped after qualifiers.
        let bare = self.peek() == T::LParen;
        while self.tok().kind == T::Name && matches!(self.tok().word, w::VOLATILE | w::U_VOLATILE_U | w::INLINE | w::GOTO) {
            let q = self.open(K::GnuAsmQualifier, f::NONE, C_NONE);
            self.kw_any(K::GnuAsmQualifier)?;
            self.close(q);
        }
        self.punct_tk(K::GnuAsmExpression, T::LParen, bare)?;
        self.strings(slot(f::ASSEMBLY_CODE, Ctx::Bare))?;
        let lists = [
            (K::GnuAsmOutputOperandList, f::OUTPUT_OPERANDS),
            (K::GnuAsmInputOperandList, f::INPUT_OPERANDS),
            (K::GnuAsmClobberList, f::CLOBBERS),
            (K::GnuAsmGotoList, f::GOTO_LABELS),
        ];
        for (kind, field) in lists {
            if self.peek() != T::Colon {
                break;
            }
            let l = self.open(kind, field, C_NONE);
            self.tk(kind, K::TColon)?;
            if !matches!(self.peek(), T::Colon | T::RParen) {
                loop {
                    match kind {
                        K::GnuAsmClobberList => {
                            self.strings(slot(f::REGISTER, Ctx::Bare))?;
                        }
                        K::GnuAsmGotoList => {
                            self.leaf(K::Identifier, f::LABEL, C_NONE)?;
                        }
                        _ => {
                            let op = if kind == K::GnuAsmOutputOperandList { K::GnuAsmOutputOperand } else { K::GnuAsmInputOperand };
                            let o = self.open(op, f::OPERAND, C_NONE);
                            if self.peek() == T::LBracket {
                                self.tk(op, K::TLBracket)?;
                                self.leaf(K::Identifier, f::SYMBOL, C_NONE)?;
                                self.punct_tk(op, T::RBracket, false)?;
                            }
                            self.string(slot(f::CONSTRAINT, Ctx::Bare))?;
                            self.punct_tk(op, T::LParen, false)?;
                            self.expr(slot(f::VALUE, Ctx::Expr))?;
                            self.punct_tk(op, T::RParen, false)?;
                            self.close(o);
                        }
                    }
                    if self.peek() != T::Comma {
                        break;
                    }
                    self.tk(kind, K::TComma)?;
                }
            }
            self.close(l);
        }
        self.punct_tk(K::GnuAsmExpression, T::RParen, false)?;
        self.close(n);
        Ok(n)
    }
}
