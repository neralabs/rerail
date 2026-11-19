//! Expressions: precedence climbing over tree-sitter-go's `PREC`, unary operators, the postfix
//! chain (selectors, type assertions, index, slice and instantiation, calls), operands that are
//! types (composite literals, conversions), function literals, literals and strings (their pieces
//! read from the bytes).

use super::*;

/// A binary operator's precedence and token kind.
fn binary(t: T) -> Option<(u8, K)> {
    use T::*;
    Some(match t {
        Star => (5, K::TStar),
        Slash => (5, K::TSlash),
        Percent => (5, K::TPercent),
        LtLt => (5, K::TLtLt),
        GtGt => (5, K::TGtGt),
        Amp => (5, K::TAmp),
        AmpCaret => (5, K::TAmpCaret),
        Plus => (4, K::TPlus),
        Minus => (4, K::TMinus),
        Pipe => (4, K::TPipe),
        Caret => (4, K::TCaret),
        EqEq => (3, K::TEqEq),
        Ne => (3, K::TNe),
        Lt => (3, K::TLt),
        Le => (3, K::TLe),
        Gt => (3, K::TGt),
        Ge => (3, K::TGe),
        AmpAmp => (2, K::TAmpAmp),
        PipePipe => (1, K::TPipePipe),
        _ => return None,
    })
}

/// An operand as read: an expression, or a type standing where one is (a `new` or `make`
/// argument, an instantiation's argument), which only a conversion or a literal makes an
/// expression.
#[derive(Clone, Copy)]
pub(super) struct Operand {
    pub n: N,
    pub ty: bool,
}

impl<'a> P<'a> {
    /// An `expression_list` in slot `s`: expressions between `,`s.
    pub(super) fn expression_list(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ExpressionList, s);
        loop {
            self.expr(EXPR)?;
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::ExpressionList, K::TComma)?;
        }
        self.close(n);
        Ok(n)
    }

    /// An expression in slot `s`.
    pub(super) fn expr(&mut self, s: Slot) -> R<N> {
        let o = self.expr_prec(s, 1)?;
        if o.ty {
            return self.refuse();
        }
        Ok(o.n)
    }

    /// An expression or a type, in slot `s`.
    pub(super) fn expr_or_type(&mut self, s: Slot) -> R<Operand> {
        self.expr_prec(s, 1)
    }

    /// An expression whose operators bind at `min` or tighter.
    fn expr_prec(&mut self, s: Slot, min: u8) -> R<Operand> {
        self.deeper()?;
        let mut left = self.unary(s)?;
        loop {
            let t = self.tok();
            let Some((p, op)) = binary(t.kind) else { break };
            if p < min || self.at_nl() {
                break;
            }
            if left.ty {
                return self.refuse();
            }
            let n = self.wrap(left.n, K::BinaryExpression, s, slot(f::LEFT, Ctx::Expr));
            self.leaf(op, f::OPERATOR, C_NONE)?;
            let r = self.expr_prec(slot(f::RIGHT, Ctx::Expr), p + 1)?;
            if r.ty {
                return self.refuse();
            }
            self.close(n);
            left = Operand { n, ty: false };
        }
        self.depth -= 1;
        Ok(left)
    }

    /// A unary expression (`+ - ! ^ * & <-` and its operand), or a postfix chain.
    fn unary(&mut self, s: Slot) -> R<Operand> {
        let t = self.tok();
        let op = match t.kind {
            T::Plus => K::TPlus,
            T::Minus => K::TMinus,
            T::Bang => K::TBang,
            T::Caret => K::TCaret,
            // `*(*[]T)(x)`: tree-sitter's tie goes to a pointer type's conversion.
            T::Star if self.kind_at(1) == T::LParen && self.pointer_conversion_ahead(1) => return self.postfix(s),
            T::Star => K::TStar,
            T::Amp => K::TAmp,
            T::LtMinus if !self.is_word_at(1, w::CHAN) => K::TLtMinus,
            _ => return self.postfix(s),
        };
        let n = self.open_in(K::UnaryExpression, s);
        self.leaf(op, f::OPERATOR, C_NONE)?;
        self.deeper()?;
        let o = self.unary(slot(f::OPERAND, Ctx::Expr))?;
        self.depth -= 1;
        if o.ty {
            return self.refuse();
        }
        self.close(n);
        Ok(Operand { n, ty: false })
    }

    /// A primary operand and its postfix chain.
    fn postfix(&mut self, s: Slot) -> R<Operand> {
        let mut o = self.primary(s)?;
        loop {
            if self.at_nl() {
                break;
            }
            let t = self.tok();
            match t.kind {
                T::Dot => {
                    if o.ty {
                        return self.refuse();
                    }
                    match self.kind_at(1) {
                        T::Name if !reserved(self.peek_at(1).word) => {
                            let n = self.wrap(o.n, K::SelectorExpression, s, slot(f::OPERAND, Ctx::Expr));
                            self.tk(K::SelectorExpression, K::TDot)?;
                            self.leaf(K::FieldIdentifier, f::FIELD, C_NONE)?;
                            self.close(n);
                            o = Operand { n, ty: false };
                        }
                        T::LParen if self.is_word_at(2, w::TYPE) => break,
                        T::LParen => {
                            let n = self.wrap(o.n, K::TypeAssertionExpression, s, slot(f::OPERAND, Ctx::Expr));
                            self.tk(K::TypeAssertionExpression, K::TDot)?;
                            self.nested(|p| {
                                p.tk(K::TypeAssertionExpression, K::TLParen)?;
                                p.ty(slot(f::TYPE, Ctx::Type))?;
                                p.punct_tk(K::TypeAssertionExpression, T::RParen, false)
                            })?;
                            self.close(n);
                            o = Operand { n, ty: false };
                        }
                        _ => return self.refuse(),
                    }
                }
                T::LParen => {
                    if o.ty {
                        o = self.conversion(o, s)?;
                        continue;
                    }
                    let n = self.wrap(o.n, K::CallExpression, s, slot(f::FUNCTION, Ctx::Expr));
                    self.arguments(false)?;
                    self.close(n);
                    o = Operand { n, ty: false };
                }
                T::LBracket => {
                    if o.ty {
                        return self.refuse();
                    }
                    o = self.index(o, s)?;
                }
                T::LBrace if !self.no_literal && self.literal_type(o.n) => {
                    o = self.composite(o, s)?;
                }
                _ => break,
            }
        }
        Ok(o)
    }

    /// Whether the operand `n` can be a composite literal's type (a name, `pkg.T`, `T[A]`), its
    /// kind made the type's.
    fn literal_type(&mut self, n: N) -> bool {
        match n.kind {
            K::Identifier => {
                self.rekind(n.at, K::TypeIdentifier, BARE);
                true
            }
            K::SelectorExpression => {
                // `pkg.T`: an identifier and a field.
                let (op, _, _, _) = unkey(self.w.key(n.at + 1));
                if kind_of(op) != K::Identifier || self.w.len() != n.at + 3 {
                    return false;
                }
                self.rekind(n.at, K::QualifiedType, BARE);
                self.w.set_key(n.at + 1, key(K::PackageIdentifier, f::PACKAGE, C_NONE, false));
                self.w.set_key(n.at + 2, key(K::TypeIdentifier, f::NAME, C_NONE, false));
                true
            }
            K::GenericType => true,
            _ => false,
        }
    }

    /// `type{…}`: the operand `o` as a composite literal's type, and its body.
    fn composite(&mut self, o: Operand, s: Slot) -> R<Operand> {
        let n = self.wrap(o.n, K::CompositeLiteral, s, slot(f::TYPE, Ctx::Bare));
        self.literal_value(slot(f::BODY, Ctx::Bare))?;
        self.close(n);
        Ok(Operand { n, ty: false })
    }

    /// `type(x)`: a type's conversion of one operand.
    fn conversion(&mut self, o: Operand, s: Slot) -> R<Operand> {
        let n = self.wrap(o.n, K::TypeConversionExpression, s, slot(f::TYPE, Ctx::Type));
        self.nested(|p| {
            p.tk(K::TypeConversionExpression, K::TLParen)?;
            p.expr(slot(f::OPERAND, Ctx::Expr))?;
            if p.peek() == T::Comma && p.kind_at(1) == T::RParen {
                p.leaf(K::TComma, f::NONE, C_NONE)?;
                p.leaf(K::TRParen, f::NONE, C_NONE)?;
                return Ok(());
            }
            p.punct_tk(K::TypeConversionExpression, T::RParen, false)
        })?;
        self.close(n);
        Ok(Operand { n, ty: false })
    }

    /// `[…]` after `o`: an index, a slice, or an instantiation (type arguments, before a call's
    /// arguments).
    fn index(&mut self, o: Operand, s: Slot) -> R<Operand> {
        // A slice: a `:` at the brackets' depth.
        let (mut depth, mut j, mut colon, mut comma) = (0usize, 0usize, false, false);
        loop {
            match self.kind_at(j) {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                T::Colon if depth == 1 => colon = true,
                T::Comma if depth == 1 => comma = true,
                T::Eof => return self.refuse(),
                _ => {}
            }
            j += 1;
        }
        if colon {
            let n = self.wrap(o.n, K::SliceExpression, s, slot(f::OPERAND, Ctx::Expr));
            self.nested(|p| {
                p.tk(K::SliceExpression, K::TLBracket)?;
                if p.peek() != T::Colon {
                    p.expr(slot(f::START, Ctx::Expr))?;
                }
                p.tk(K::SliceExpression, K::TColon)?;
                if !matches!(p.peek(), T::RBracket | T::Colon) {
                    p.expr(slot(f::END, Ctx::Expr))?;
                }
                if p.peek() == T::Colon {
                    p.tk(K::SliceExpression, K::TColon)?;
                    p.expr(slot(f::CAPACITY, Ctx::Expr))?;
                }
                p.punct_tk(K::SliceExpression, T::RBracket, false)
            })?;
            self.close(n);
            return Ok(Operand { n, ty: false });
        }
        let after = self.kind_at(j + 1);
        // tree-sitter's GLR ties, settled by dynamic precedence (oracle-checked): types in the
        // brackets before one argument are a generic type's conversion (`F[T](x)`, `m[k](x)`
        // too); before none or several, type arguments unless the brackets hold `x` or `*x`,
        // which index.
        let types = after == T::LParen && self.types_between(1, j);
        if types && self.one_argument_at(j + 1) && self.literal_type(o.n) {
            let g = self.wrap(o.n, K::GenericType, s, slot(f::TYPE, Ctx::Bare));
            self.type_arguments()?;
            self.close(g);
            return Ok(Operand { n: g, ty: true });
        }
        let plain = match j {
            2 => self.kind_at(1) == T::Name,
            3 => self.kind_at(1) == T::Star && self.kind_at(2) == T::Name,
            _ => false,
        };
        // `a[[8]byte{1}]`: a composite literal indexes.
        let literal = self.kind_at(j - 1) == T::RBrace && self.kind_at(1) == T::LBracket;
        if !literal && (comma || !self.expression_starts_at(1) || (types && !plain)) {
            // Type arguments: before a call's arguments, the call's; else an instantiation.
            if after == T::LParen {
                let n = self.wrap(o.n, K::CallExpression, s, slot(f::FUNCTION, Ctx::Expr));
                self.type_arguments()?;
                self.arguments(false)?;
                self.close(n);
                return Ok(Operand { n, ty: false });
            }
            let n = self.wrap(o.n, K::TypeInstantiationExpression, s, slot(f::TYPE, Ctx::Type));
            self.list(K::TypeInstantiationExpression, T::RBracket, false, true, |p| p.ty(TYPE).map(|_| ()))?;
            self.close(n);
            return Ok(Operand { n, ty: false });
        }
        let n = self.wrap(o.n, K::IndexExpression, s, slot(f::OPERAND, Ctx::Expr));
        self.nested(|p| {
            p.tk(K::IndexExpression, K::TLBracket)?;
            p.expr(slot(f::INDEX, Ctx::Expr))?;
            p.punct_tk(K::IndexExpression, T::RBracket, false)
        })?;
        self.close(n);
        Ok(Operand { n, ty: false })
    }

    /// Whether the tokens `from..to` ahead can be types (names, `.`, `*`, brackets, `,`, `|`, `~`,
    /// `struct{…}`, `interface{…}`, `func(…)`, an array's length).
    fn types_between(&mut self, from: usize, to: usize) -> bool {
        let (mut braces, mut depth, mut length) = (0usize, 0usize, usize::MAX);
        for k in from..to {
            let t = self.peek_at(k);
            // An array's length (`[1 << 24]T`) is an expression: anything to its `]`.
            if length != usize::MAX {
                match t.kind {
                    T::LBracket | T::LParen => depth += 1,
                    T::RBracket | T::RParen => {
                        depth -= 1;
                        if depth == length {
                            length = usize::MAX;
                        }
                    }
                    _ => {}
                }
                continue;
            }
            if t.kind == T::LBracket
                && k + 1 < to
                && self.kind_at(k + 1) != T::RBracket
                && (k == from || !matches!(self.kind_at(k - 1), T::Name | T::RBracket | T::RParen))
            {
                length = depth;
                depth += 1;
                continue;
            }
            match t.kind {
                T::LBracket | T::LParen => depth += 1,
                T::RBracket | T::RParen => depth = depth.saturating_sub(1),
                _ => {}
            }
            let ok = match t.kind {
                T::Name => !matches!(t.word, w::NIL | w::TRUE | w::FALSE | w::IOTA) || braces > 0,
                T::LBrace => {
                    let ok = braces > 0 || self.is_word_at(k - 1, w::STRUCT) || self.is_word_at(k - 1, w::INTERFACE);
                    braces += 1;
                    ok
                }
                T::RBrace => {
                    braces = braces.saturating_sub(1);
                    true
                }
                T::Dot | T::Star | T::LBracket | T::RBracket | T::Comma | T::Pipe | T::Tilde | T::LtMinus
                | T::LParen | T::RParen | T::DotDotDot | T::Semi => true,
                _ => braces > 0,
            };
            if !ok {
                return false;
            }
        }
        true
    }

    /// Whether the `(` at token `k` ahead holds exactly one argument (a trailing `,` allowed).
    fn one_argument_at(&mut self, k: usize) -> bool {
        let (mut depth, mut j, mut commas, mut last) = (0usize, k, 0usize, T::Eof);
        loop {
            let t = self.kind_at(j);
            match t {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                T::Comma if depth == 1 => commas += 1,
                T::Eof => return false,
                _ => {}
            }
            if depth > 0 && j > k {
                last = t;
            }
            j += 1;
        }
        j > k + 1 && (commas == 0 || (commas == 1 && last == T::Comma))
    }

    /// Whether token `k` ahead can start an expression (not a type alone: `[]T`, `map[…]…`,
    /// `chan T`, `interface{…}`, `*T` in brackets is an expression).
    fn expression_starts_at(&mut self, k: usize) -> bool {
        let t = self.peek_at(k);
        match t.kind {
            T::LBracket => false,
            T::Name => !matches!(t.word, w::MAP | w::CHAN | w::INTERFACE),
            _ => true,
        }
    }

    /// A call's `argument_list` (`special`: `new`'s or `make`'s, a type first).
    fn arguments(&mut self, special: bool) -> R<()> {
        let n = self.open(K::ArgumentList, f::ARGUMENTS, C_NONE);
        let mut first = special;
        self.list(K::ArgumentList, T::RParen, true, true, |p| {
            if std::mem::take(&mut first) {
                p.ty(TYPE)?;
                return Ok(());
            }
            let e = p.expr(EXPR)?;
            if p.peek() == T::DotDotDot {
                let v = p.wrap(e, K::VariadicArgument, BARE, EXPR);
                p.tk(K::VariadicArgument, K::TDotDotDot)?;
                p.close(v);
            }
            Ok(())
        })?;
        self.close(n);
        Ok(())
    }

    /// A primary operand in slot `s`.
    fn primary(&mut self, s: Slot) -> R<Operand> {
        let t = self.tok();
        let e = |n: N| Ok(Operand { n, ty: false });
        match t.kind {
            T::Name => match t.word {
                w::NIL => e(self.leaf_in(K::Nil, s)?),
                w::TRUE => e(self.leaf_in(K::True, s)?),
                w::FALSE => e(self.leaf_in(K::False, s)?),
                w::IOTA => e(self.leaf_in(K::Iota, s)?),
                w::NEW | w::MAKE if self.kind_at(1) == T::LParen => {
                    let n = self.open_in(K::CallExpression, s);
                    self.leaf(K::Identifier, f::FUNCTION, C_NONE)?;
                    self.arguments(true)?;
                    self.close(n);
                    e(n)
                }
                w::FUNC => {
                    let n = self.open_in(K::FuncLiteral, s);
                    self.tk(K::FuncLiteral, K::TFunc)?;
                    self.signature()?;
                    if self.peek() == T::LBrace {
                        self.block(slot(f::BODY, Ctx::Bare))?;
                        self.close(n);
                        return e(n);
                    }
                    self.rekind(n.at, K::FunctionType, slot(s.field, Ctx::Type));
                    self.close(n);
                    Ok(Operand { n: N { at: n.at, kind: K::FunctionType }, ty: true })
                }
                w::MAP | w::CHAN | w::STRUCT | w::INTERFACE => self.type_operand(s),
                _ if reserved(t.word) => self.refuse(),
                _ => {
                    // `T[A]{`: a generic type's literal.
                    if !self.no_literal && self.generic_literal_ahead() {
                        let n = self.ty(slot(s.field, Ctx::Bare))?;
                        if self.peek() != T::LBrace {
                            return self.refuse();
                        }
                        return self.composite(Operand { n, ty: true }, s);
                    }
                    e(self.leaf_in(K::Identifier, s)?)
                }
            },
            T::Int => e(self.leaf_in(K::IntLiteral, s)?),
            T::Float => e(self.leaf_in(K::FloatLiteral, s)?),
            T::Imaginary => e(self.leaf_in(K::ImaginaryLiteral, s)?),
            T::Rune => e(self.leaf_in(K::RuneLiteral, s)?),
            T::Quote | T::Raw => e(self.string(s)?),
            T::LParen if self.pointer_conversion_ahead(0) => {
                let n = self.ty(slot(s.field, Ctx::Type))?;
                Ok(Operand { n, ty: true })
            }
            T::LParen => {
                let n = self.open_in(K::ParenthesizedExpression, s);
                let inner = self.nested(|p| {
                    p.tk(K::ParenthesizedExpression, K::TLParen)?;
                    let o = p.expr_or_type(EXPR)?;
                    p.punct_tk(K::ParenthesizedExpression, T::RParen, false)?;
                    Ok(o)
                })?;
                self.close(n);
                if inner.ty {
                    // `([]T)(x)`: a parenthesized type, converting.
                    self.rekind(n.at, K::ParenthesizedType, slot(s.field, Ctx::Type));
                    self.rekey(inner.n.at, TYPE);
                    return Ok(Operand { n: N { at: n.at, kind: K::ParenthesizedType }, ty: true });
                }
                e(n)
            }
            T::LBracket | T::LtMinus | T::Star => self.type_operand(s),
            _ => self.refuse(),
        }
    }

    /// Whether `(*T)(x)` ahead converts (a `parenthesized_type`) rather than calls: tree-sitter's
    /// dynamic precedences favour the type when `T` holds brackets (`*T[A]`, `*[]T`) or a type's
    /// keyword (`*map[…]…`), and the call for `*T` and `*pkg.T`.
    fn pointer_conversion_ahead(&mut self, at: usize) -> bool {
        if self.kind_at(at + 1) != T::Star {
            return false;
        }
        let (mut depth, mut j, mut typed) = (0usize, at, false);
        loop {
            let t = self.peek_at(j);
            match t.kind {
                T::LParen | T::LBracket | T::LBrace => {
                    typed |= t.kind == T::LBracket;
                    depth += 1;
                }
                T::RParen | T::RBracket | T::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                T::Name => typed |= matches!(t.word, w::MAP | w::CHAN | w::FUNC | w::STRUCT | w::INTERFACE),
                T::LtMinus => typed = true,
                T::Eof => return false,
                _ => {}
            }
            j += 1;
        }
        typed && self.kind_at(j + 1) == T::LParen && self.types_between(at + 2, j) && self.one_argument_at(j + 1)
    }

    /// Whether a generic type's composite literal is ahead: `T[A]{` or `pkg.T[A]{`.
    fn generic_literal_ahead(&mut self) -> bool {
        let k = if self.kind_at(1) == T::Dot && self.is_ident_at(2) { 3 } else { 1 };
        self.kind_at(k) == T::LBracket && self.bracket_end(k).is_some_and(|e| self.kind_at(e) == T::LBrace)
    }

    /// A type where an operand is: a composite literal's (`[]T{…}`), a conversion's (`[]T(x)`), or a
    /// type alone.
    fn type_operand(&mut self, s: Slot) -> R<Operand> {
        let n = self.ty(slot(s.field, Ctx::Type))?;
        match self.peek() {
            T::LBrace if !matches!(n.kind, K::ChannelType | K::InterfaceType | K::FunctionType | K::PointerType) => {
                self.rekey(n.at, slot(s.field, Ctx::Bare));
                self.composite(Operand { n, ty: true }, s)
            }
            _ => Ok(Operand { n, ty: true }),
        }
    }

    /// A `literal_value` in slot `s`: `{` elements `}`, each a `literal_element` or a
    /// `keyed_element`.
    fn literal_value(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::LiteralValue, s);
        self.list(K::LiteralValue, T::RBrace, true, true, |p| {
            let k = p.literal_element(BARE)?;
            if p.peek() == T::Colon {
                let kv = p.wrap(k, K::KeyedElement, BARE, slot(f::KEY, Ctx::Bare));
                p.tk(K::KeyedElement, K::TColon)?;
                p.literal_element(slot(f::VALUE, Ctx::Bare))?;
                p.close(kv);
            }
            Ok(())
        })?;
        self.close(n);
        Ok(n)
    }

    /// A `literal_element`: an expression or a literal value.
    fn literal_element(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::LiteralElement, s);
        if self.peek() == T::LBrace {
            self.literal_value(BARE)?;
        } else {
            self.expr(EXPR)?;
        }
        self.close(n);
        Ok(n)
    }

    /// A string in slot `s`: raw (`` ` ``, its content), or interpreted (`"`, its pieces: content
    /// runs and escapes).
    pub(super) fn string(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Raw => {
                let n = self.open_in(K::RawStringLiteral, s);
                self.bump()?;
                let (a, b) = (t.start, t.end);
                if self.stores(K::RawStringLiteral, K::TBacktick, f::NONE) {
                    self.piece(K::TBacktick, a, a + 1);
                }
                self.piece(K::RawStringLiteralContent, a + 1, b - 1);
                if self.stores(K::RawStringLiteral, K::TBacktick, f::NONE) {
                    self.piece(K::TBacktick, b - 1, b);
                }
                self.close(n);
                Ok(n)
            }
            T::Quote => {
                let n = self.open_in(K::InterpretedStringLiteral, s);
                self.bump()?;
                let (a, b) = (t.start as usize, t.end as usize - 1);
                let quotes = self.stores(K::InterpretedStringLiteral, K::TDQuote, f::NONE);
                if quotes {
                    self.piece(K::TDQuote, a as u32, a as u32 + 1);
                }
                let mut i = a + 1;
                while i < b {
                    if self.src[i] == b'\\' {
                        let Some(e) = self.lx.escape_end(i) else { return self.refuse() };
                        self.piece(K::EscapeSequence, i as u32, e as u32);
                        i = e;
                    } else {
                        let mut e = i;
                        while e < b && self.src[e] != b'\\' {
                            e += 1;
                        }
                        self.piece(K::InterpretedStringLiteralContent, i as u32, e as u32);
                        i = e;
                    }
                }
                if quotes {
                    self.piece(K::TDQuote, b as u32, b as u32 + 1);
                }
                self.close(n);
                Ok(n)
            }
            _ => self.refuse(),
        }
    }

    /// A leaf over `a..b` (a string's piece).
    fn piece(&mut self, kind: K, a: u32, b: u32) {
        let at = self.w.push(key(kind, f::NONE, C_NONE, false), a);
        self.w.close(at, b);
    }
}
