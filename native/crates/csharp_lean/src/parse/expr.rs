//! Expressions: precedence climbing over tree-sitter-c-sharp's `PREC`, the postfix chain, the
//! primaries, creation, lambdas, strings and queries.

use super::*;

/// A binary operator's precedence, kind and right associativity (tree-sitter-c-sharp's `PREC`).
fn binary(t: T) -> Option<(u8, K, bool)> {
    use T::*;
    Some(match t {
        QQ => (3, K::TQQ, true),
        PipePipe => (4, K::TPipePipe, false),
        AmpAmp => (5, K::TAmpAmp, false),
        Pipe => (6, K::TPipe, false),
        Caret => (7, K::TCaret, false),
        Amp => (8, K::TAmp, false),
        EqEq => (9, K::TEqEq, false),
        Ne => (9, K::TNe, false),
        Lt => (10, K::TLt, false),
        Gt => (10, K::TGt, false),
        Le => (10, K::TLe, false),
        Ge => (10, K::TGe, false),
        LtLt => (11, K::TLtLt, false),
        GtGt => (11, K::TGtGt, false),
        GtGtGt => (11, K::TGtGtGt, false),
        Plus => (12, K::TPlus, false),
        Minus => (12, K::TMinus, false),
        Star => (13, K::TStar, false),
        Slash => (13, K::TSlash, false),
        Percent => (13, K::TPercent, false),
        _ => return None,
    })
}

/// Whether `t` is a binary operator's token.
pub(super) fn is_binary(t: T) -> bool {
    binary(t).is_some()
}

/// A binary operator's precedence.
pub(super) fn binary_prec(t: T) -> Option<u8> {
    binary(t).map(|b| b.0)
}

pub(super) fn assignment(t: T) -> Option<K> {
    use T::*;
    Some(match t {
        Eq => K::TEq,
        PlusEq => K::TPlusEq,
        MinusEq => K::TMinusEq,
        StarEq => K::TStarEq,
        SlashEq => K::TSlashEq,
        PercentEq => K::TPercentEq,
        AmpEq => K::TAmpEq,
        CaretEq => K::TCaretEq,
        PipeEq => K::TPipeEq,
        LtLtEq => K::TLtLtEq,
        GtGtEq => K::TGtGtEq,
        GtGtGtEq => K::TGtGtGtEq,
        QQEq => K::TQQEq,
        _ => return None,
    })
}

const P_COND: u8 = 2;
pub(super) const P_CONST: u8 = 3;
const P_REL: u8 = 10;
const P_WITH: u8 = 14;
const P_SWITCH: u8 = 15;
const P_RANGE: u8 = 16;

/// Whether a token starts an operand (an expression after a cast's `)`).
pub(super) fn starts_operand(t: Tok) -> bool {
    use T::*;
    match t.kind {
        Name => !matches!(t.word, w::AS | w::IS | w::SWITCH),
        Int | Real | Char | Str | Verbatim | Raw | Interp | LParen | Bang | Tilde | Plus | Minus | PlusPlus | MinusMinus | Amp | Star | Caret => true,
        _ => false,
    }
}

/// Whether a token follows type arguments in an expression (C#'s disambiguation of `<`).
fn follows_type_args(t: T) -> bool {
    use T::*;
    matches!(t, LParen | RParen | RBracket | RBrace | Colon | Semi | Comma | Dot | Question | EqEq | Ne | Pipe | Caret | AmpAmp | PipePipe | Amp | LBracket | Eof | QQ | FatArrow)
}

impl<'a> P<'a> {
    /// Whether node `n` is an lvalue's, as `lvalue_expression` names them.
    pub(super) fn lvalue_node(&self, n: N) -> bool {
        match n.kind {
            K::PrefixUnaryExpression | K::ParenthesizedExpression => matches!(unkey(self.w.key(n.at)).2, C_EL | C_L),
            k => is_lvalue(k),
        }
    }

    /// An `expression` in slot `s`: an assignment, or a conditional and what it binds.
    pub(super) fn expr(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let (in_constant, cmode) = (self.in_constant, self.cmode);
        (self.in_constant, self.cmode) = (false, Cmode::Plain);
        let left = self.binary_expr(s, 0)?;
        (self.in_constant, self.cmode) = (in_constant, cmode);
        let n = if let Some(op) = assignment(self.peek()) {
            if !self.lvalue_node(left) {
                return self.refuse();
            }
            let n = self.wrap(left, K::AssignmentExpression, s, slot(f::LEFT, Ctx::Lv));
            self.leaf(op, f::OPERATOR, C_NONE)?;
            self.expr(slot(f::RIGHT, Ctx::Expr))?;
            self.close(n);
            n
        } else {
            self.bare(left)?;
            left
        };
        self.depth -= 1;
        Ok(n)
    }

    /// Operators of precedence `min` and above, from an operand.
    pub(super) fn binary_expr(&mut self, s: Slot, min: u8) -> R<N> {
        let left = self.unary(s)?;
        self.binary_tail(left, s, min)
    }

    /// The operators of precedence `min` and above after `left`.
    pub(super) fn binary_tail(&mut self, mut left: N, s: Slot, min: u8) -> R<N> {
        loop {
            let t = self.tok();
            if let Some((p, op, right)) = binary(t.kind) {
                if p < min {
                    break;
                }
                left = self.wrap(left, K::BinaryExpression, s, slot(f::LEFT, Ctx::Expr));
                self.leaf(op, f::OPERATOR, C_NONE)?;
                self.binary_expr(slot(f::RIGHT, Ctx::Expr), if right { p } else { p + 1 })?;
                self.close(left);
                continue;
            }
            match t.kind {
                T::Question if min <= P_COND => {
                    if self.conditional_access_ahead() {
                        left = self.conditional_access(left, s)?;
                        continue;
                    }
                    left = self.wrap(left, K::ConditionalExpression, s, slot(f::CONDITION, Ctx::Expr));
                    self.tk(K::ConditionalExpression, K::TQuestion)?;
                    self.expr(slot(f::CONSEQUENCE, Ctx::Expr))?;
                    if self.peek() != T::Colon {
                        return self.refuse();
                    }
                    self.tk(K::ConditionalExpression, K::TColon)?;
                    self.binary_expr(slot(f::ALTERNATIVE, Ctx::Expr), P_COND)?;
                    self.close(left);
                }
                T::Name if t.word == w::IS && min <= P_REL => {
                    left = self.is_tail(left, s, min)?;
                }
                T::Name if t.word == w::AS && min <= P_REL => {
                    left = self.wrap(left, K::AsExpression, s, slot(f::LEFT, Ctx::Expr));
                    self.leaf(K::TAs, f::OPERATOR, C_NONE)?;
                    self.ty_in_expr(slot(f::RIGHT, Ctx::Type))?;
                    self.close(left);
                }
                T::Name if t.word == w::SWITCH && min <= P_SWITCH && self.kind_at(1) == T::LBrace => {
                    left = self.wrap(left, K::SwitchExpression, s, EXPR);
                    self.tk(K::SwitchExpression, K::TSwitch)?;
                    self.switch_expression_body()?;
                    self.close(left);
                }
                T::Name if t.word == w::WITH && min <= P_WITH && self.kind_at(1) == T::LBrace => {
                    left = self.wrap(left, K::WithExpression, s, EXPR);
                    self.tk(K::WithExpression, K::TWith)?;
                    self.list(K::WithExpression, T::RBrace, true, true, |p| {
                        let n = p.open(K::WithInitializer, f::NONE, C_NONE)?;
                        p.ident(BARE)?;
                        p.punct_tk(K::WithInitializer, T::Eq, false)?;
                        p.expr(EXPR)?;
                        p.close(n);
                        Ok(())
                    })?;
                    self.close(left);
                }
                T::DotDot if min <= P_RANGE => {
                    left = self.wrap(left, K::RangeExpression, s, EXPR);
                    self.tk(K::RangeExpression, K::TDotDot)?;
                    if starts_operand(self.tok()) {
                        self.binary_expr(EXPR, P_RANGE + 1)?;
                    }
                    self.close(left);
                }
                _ => break,
            }
        }
        Ok(left)
    }

    /// Whether the `?` next is a conditional access's: `?.`, or `?[` with no blank between.
    fn conditional_access_ahead(&mut self) -> bool {
        let q = self.tok();
        match self.kind_at(1) {
            T::Dot => true,
            T::LBracket => self.peek_at(1).start == q.end,
            _ => false,
        }
    }

    /// `left?.name` or `left?[args]`, and the postfix chain after it.
    fn conditional_access(&mut self, left: N, s: Slot) -> R<N> {
        let n = self.wrap(left, K::ConditionalAccessExpression, s, slot(f::CONDITION, Ctx::Expr));
        self.tk(K::ConditionalAccessExpression, K::TQuestion)?;
        if self.peek() == T::Dot {
            let b = self.open(K::MemberBindingExpression, f::NONE, C_NONE)?;
            self.tk(K::MemberBindingExpression, K::TDot)?;
            self.simple_name(slot(f::NAME, Ctx::Bare), true)?;
            self.close(b);
        } else {
            self.bracketed_arguments(K::ElementBindingExpression, f::NONE, C_NONE)?;
        }
        self.close(n);
        self.postfix(n, s)
    }

    /// A unary expression (precedence 17) or tighter, in slot `s`.
    pub(super) fn unary(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let t = self.tok();
        let n = match t.kind {
            T::PlusPlus | T::MinusMinus | T::Plus | T::Minus | T::Bang | T::Tilde | T::Amp | T::Caret => {
                let n = self.open_in(K::PrefixUnaryExpression, s)?;
                self.leaf(punct(t.kind).expect("an operator"), f::NONE, C_NONE)?;
                self.unary(EXPR)?;
                self.close(n);
                n
            }
            T::Star => {
                // A pointer indirection: an lvalue of an lvalue.
                let n = self.open(K::PrefixUnaryExpression, s.field, lvalue_class(s.ctx))?;
                self.leaf(K::TStar, f::NONE, C_NONE)?;
                let inner = self.unary(slot(f::NONE, Ctx::Lv))?;
                if !self.lvalue_node(inner) {
                    return self.refuse();
                }
                self.close(n);
                n
            }
            T::DotDot => {
                let n = self.open_in(K::RangeExpression, s)?;
                self.tk(K::RangeExpression, K::TDotDot)?;
                if starts_operand(self.tok()) {
                    self.binary_expr(EXPR, P_RANGE + 1)?;
                }
                self.close(n);
                n
            }
            T::LParen => self.paren(s)?,
            T::Name if t.word == w::AWAIT && self.await_ahead() => {
                let n = self.open_in(K::AwaitExpression, s)?;
                self.tk(K::AwaitExpression, K::TAwait)?;
                self.unary(EXPR)?;
                self.close(n);
                n
            }
            T::Name if t.word == w::REF => {
                let n = self.open_in(K::RefExpression, s)?;
                // `ref` takes a whole expression (its rule has no precedence).
                self.tk(K::RefExpression, K::TRef)?;
                self.expr(EXPR)?;
                self.close(n);
                n
            }
            T::Name if t.word == w::THROW => {
                let n = self.open_in(K::ThrowExpression, s)?;
                self.tk(K::ThrowExpression, K::TThrow)?;
                self.expr(EXPR)?;
                self.close(n);
                n
            }
            T::Name if self.lambda_ahead() => self.lambda(s)?,
            _ => {
                let n = self.primary(s)?;
                self.postfix(n, s)?
            }
        };
        self.depth -= 1;
        Ok(n)
    }

    /// Whether `await` next is an await expression's (an operand follows it).
    fn await_ahead(&mut self) -> bool {
        let t = self.peek_at(1);
        starts_operand(t) && !(t.kind == T::Name && matches!(t.word, w::AND | w::OR)) && !matches!(t.kind, T::Plus | T::Minus | T::Star | T::Amp | T::Caret)
    }

    /// `(`: a lambda's parameters, a cast, a tuple or a parenthesized expression.
    fn paren(&mut self, s: Slot) -> R<N> {
        let close = self.matching(0).ok_or_else(|| Refusal::Parse(self.tok().start))?;
        if self.kind_at(close + 1) == T::FatArrow {
            return self.lambda(s);
        }
        if self.cast_ahead(close) {
            let n = self.open_in(K::CastExpression, s)?;
            self.tk(K::CastExpression, K::TLParen)?;
            self.ty(slot(f::TYPE, Ctx::Type))?;
            self.punct_tk(K::CastExpression, T::RParen, false)?;
            let v = self.unary(slot(f::VALUE, Ctx::Expr))?;
            self.bare(v)?;
            self.close(n);
            return Ok(n);
        }
        if self.top_comma(1, close) {
            let n = self.open_in(K::TupleExpression, s)?;
            self.list(K::TupleExpression, T::RParen, true, false, |p| p.argument())?;
            self.close(n);
            return Ok(n);
        }
        let n = self.open_in(K::ParenthesizedExpression, s)?;
        self.tk(K::ParenthesizedExpression, K::TLParen)?;
        let inner = self.expr(slot(f::NONE, Ctx::Paren))?;
        self.punct_tk(K::ParenthesizedExpression, T::RParen, false)?;
        self.close(n);
        if self.lvalue_node(inner) {
            self.w.set_key(n.at, key(K::ParenthesizedExpression, s.field, lvalue_class(s.ctx), false));
        }
        let n = self.postfix(n, s)?;
        Ok(n)
    }

    /// The index (from the next token) of the bracket closing the one `k` tokens ahead.
    pub(super) fn matching(&mut self, k: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut j = k;
        loop {
            match self.kind_at(j) {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                }
                T::Eof => return None,
                _ => {}
            }
            j += 1;
        }
    }

    /// Whether a `,` stands at the top level between tokens `from` and `to` (exclusive).
    fn top_comma(&mut self, from: usize, to: usize) -> bool {
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

    /// Whether `(` … `)` (closing `close` tokens ahead) is a cast's: a type fills it and an operand
    /// follows (tree-sitter prefers the cast: `(a) - b` is one).
    fn cast_ahead(&mut self, close: usize) -> bool {
        if self.scan_type(1) != Some(close) {
            return false;
        }
        let after = self.peek_at(close + 1);
        if after.kind == T::AmpAmp {
            // `(a) && b`: a cast of `&(&b)` to tree-sitter (a cast's dynamic precedence wins, and
            // its lexer gives the cast's `&`).
            self.split_at(close + 1, T::Amp, 1);
            return true;
        }
        if after.kind == T::LBracket && self.kind_at(close + 2) == T::RBracket {
            // `(T)[]`: a cast of an empty collection.
            return true;
        }
        if !starts_operand(after) {
            return false;
        }
        // A word that cannot start an expression ends it.
        !(after.kind == T::Name && matches!(after.word, w::AND | w::OR | w::WHEN | w::INTO | w::BY | w::ON | w::EQUALS | w::ASCENDING | w::DESCENDING))
    }

    /// `n`, the last node written, a member access of names alone (`a.b.c`), written again as a
    /// `qualified_name` (its `.`s implied); else `n` as it is.
    fn qualified_from_member_access(&mut self, n: N) -> N {
        let mut names = Vec::new();
        for i in n.at..self.w.len() {
            let kind = kind_of(unkey(self.w.key(i)).0);
            match kind {
                K::MemberAccessExpression | K::TDot => {}
                K::Identifier => names.push((self.w.start(i), self.w.end(i))),
                _ => return n,
            }
        }
        if names.len() < 2 {
            return n;
        }
        let key = self.w.key(n.at);
        let (_, field, _, _) = unkey(key);
        self.w.truncate(n.at);
        // `qualified_name`s nest to the left: one opened per name past the first.
        let opens: Vec<usize> = (1..names.len()).map(|k| self.w.push(crate::key(K::QualifiedName, if k == 1 { field } else { f::QUALIFIER }, C_NONE, false), names[0].0)).collect();
        for (k, &(a, b)) in names.iter().enumerate() {
            let fld = if k == 0 { f::QUALIFIER } else { f::NAME };
            self.piece(K::Identifier, fld, a as usize, b as usize);
            if k > 0 {
                self.w.close(opens[names.len() - 1 - k], b);
            }
        }
        N { at: n.at, kind: K::QualifiedName }
    }

    /// The postfix chain after `n` (precedence 18): member access, invocation, element access,
    /// `++`, `--` and `!`.
    pub(super) fn postfix(&mut self, mut n: N, s: Slot) -> R<N> {
        loop {
            match self.peek() {
                T::Dot | T::Arrow => {
                    if !self.is_ident_at(1) {
                        return self.refuse();
                    }
                    // Before `->`, a name is tree-sitter's `_name` (no expression): `a.b->c`'s
                    // `a.b` a `qualified_name`.
                    if self.peek() == T::Arrow && n.kind == K::MemberAccessExpression {
                        n = self.qualified_from_member_access(n);
                    }
                    let bare = self.peek() == T::Arrow && matches!(n.kind, K::Identifier | K::QualifiedName);
                    let inner = if bare { slot(f::EXPRESSION, Ctx::Bare) } else { slot(f::EXPRESSION, Ctx::Expr) };
                    n = self.wrap(n, K::MemberAccessExpression, s, inner);
                    let op = if self.peek() == T::Dot { K::TDot } else { K::TArrow };
                    self.leaf(op, f::NONE, C_NONE)?;
                    self.simple_name(slot(f::NAME, Ctx::Bare), true)?;
                    self.close(n);
                }
                T::LParen => {
                    n = self.wrap(n, K::InvocationExpression, s, slot(f::FUNCTION, Ctx::Expr));
                    self.arguments(f::ARGUMENTS)?;
                    self.close(n);
                }
                T::LBracket => {
                    n = self.wrap(n, K::ElementAccessExpression, s, slot(f::EXPRESSION, Ctx::Expr));
                    self.bracketed_arguments(K::BracketedArgumentList, f::SUBSCRIPT, C_NONE)?;
                    self.close(n);
                }
                T::PlusPlus | T::MinusMinus => {
                    let k = punct(self.peek()).expect("an operator");
                    n = self.wrap(n, K::PostfixUnaryExpression, s, EXPR);
                    self.leaf(k, f::NONE, C_NONE)?;
                    self.close(n);
                }
                T::Bang => {
                    n = self.wrap(n, K::PostfixUnaryExpression, s, EXPR);
                    self.leaf(K::TBang, f::NONE, C_NONE)?;
                    self.close(n);
                }
                _ => return Ok(n),
            }
        }
    }

    /// A primary expression in slot `s`.
    fn primary(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Int => self.leaf_in(K::IntegerLiteral, s),
            T::Real => self.leaf_in(K::RealLiteral, s),
            T::Char => self.char_literal(s),
            T::Str => self.string(s),
            T::Verbatim => self.leaf_in(K::VerbatimStringLiteral, s),
            T::Raw => self.raw_string(s),
            T::Interp => self.interpolated(s),
            T::LBracket => self.collection(s),
            T::LBrace => self.initializer(s),
            T::Name => self.name_primary(s, t),
            _ => self.refuse(),
        }
    }

    /// A primary that starts with a word.
    fn name_primary(&mut self, s: Slot, t: Tok) -> R<N> {
        match t.word {
            w::THIS => self.leaf_in(K::TThis, s),
            w::BASE => self.leaf_in(K::TBase, s),
            w::NULL => self.leaf_in(K::NullLiteral, s),
            w::TRUE | w::FALSE => {
                let n = self.open_in(K::BooleanLiteral, s)?;
                self.leaf(keyword(t.word).expect("a boolean"), f::NONE, C_NONE)?;
                self.close(n);
                Ok(n)
            }
            w::NEW => self.creation(s),
            w::TYPEOF | w::SIZEOF => {
                let kind = if t.word == w::TYPEOF { K::TypeofExpression } else { K::SizeofExpression };
                let n = self.open_in(kind, s)?;
                self.tk(kind, keyword(t.word).expect("a keyword"))?;
                self.punct_tk(kind, T::LParen, false)?;
                self.ty(slot(f::TYPE, Ctx::Type))?;
                self.punct_tk(kind, T::RParen, false)?;
                self.close(n);
                Ok(n)
            }
            w::DEFAULT => {
                let n = self.open_in(K::DefaultExpression, s)?;
                self.tk(K::DefaultExpression, K::TDefault)?;
                if self.peek() == T::LParen {
                    self.punct_tk(K::DefaultExpression, T::LParen, false)?;
                    self.ty(slot(f::TYPE, Ctx::Type))?;
                    self.punct_tk(K::DefaultExpression, T::RParen, false)?;
                }
                self.close(n);
                Ok(n)
            }
            w::CHECKED | w::UNCHECKED => {
                let n = self.open_in(K::CheckedExpression, s)?;
                self.leaf(keyword(t.word).expect("a keyword"), f::NONE, C_NONE)?;
                self.punct_tk(K::CheckedExpression, T::LParen, true)?;
                self.expr(EXPR)?;
                self.punct_tk(K::CheckedExpression, T::RParen, false)?;
                self.close(n);
                Ok(n)
            }
            w::STACKALLOC => self.stackalloc(s),
            w::DELEGATE => self.anonymous_method(s),
            w::MAKEREF | w::REFTYPE if self.kind_at(1) == T::LParen => {
                let kind = if t.word == w::MAKEREF { K::MakerefExpression } else { K::ReftypeExpression };
                let n = self.open_in(kind, s)?;
                self.tk(kind, keyword(t.word).expect("a keyword"))?;
                self.punct_tk(kind, T::LParen, false)?;
                self.expr(EXPR)?;
                self.punct_tk(kind, T::RParen, false)?;
                self.close(n);
                Ok(n)
            }
            w::FROM if self.query_ahead() => self.query(s),
            code if w::is_predefined(code) => {
                // `int.Parse`: a predefined type as a member access's object.
                if self.kind_at(1) != T::Dot {
                    return self.refuse();
                }
                self.leaf(K::PredefinedType, s.field, C_NONE)
            }
            code if w::is_reserved(code) => self.refuse(),
            _ => {
                if self.kind_at(1) == T::ColonColon {
                    // `global::A`: an alias-qualified name as a member access's object.
                    let n = self.open(K::AliasQualifiedName, s.field, C_NONE)?;
                    self.ident(slot(f::ALIAS, Ctx::Bare))?;
                    self.tk(K::AliasQualifiedName, K::TColonColon)?;
                    self.simple_name(slot(f::NAME, Ctx::Bare), true)?;
                    self.close(n);
                    if self.peek() != T::Dot {
                        return self.refuse();
                    }
                    return Ok(n);
                }
                self.simple_name(s, true)
            }
        }
    }

    /// `identifier` or `generic_name` in slot `s`; in an expression (`expr`), a `<` is type
    /// arguments only where they close before a token that follows a type.
    pub(super) fn simple_name(&mut self, s: Slot, expr: bool) -> R<N> {
        if self.kind_at(1) == T::Lt {
            // In a type, a `>>` may close this list and the one around it.
            let generic = match self.scan_type_args_owing(1) {
                Some((e, 0)) => !expr || follows_type_args(self.kind_at(e)) || self.generic_before_name(e),
                Some(_) => !expr,
                None => false,
            };
            if generic {
                let n = self.open_in(K::GenericName, s)?;
                self.ident(BARE)?;
                self.type_argument_list()?;
                self.close(n);
                return Ok(n);
            }
        }
        self.ident(s)
    }

    /// In an argument, a generic name before a name is a declaration's type (`f(a < b, c > d)`).
    fn generic_before_name(&mut self, e: usize) -> bool {
        self.in_argument && self.is_ident_at(e) && matches!(self.kind_at(e + 1), T::Comma | T::RParen)
    }

    // ---- Arguments.

    /// `argument_list`: `(` arguments `)`.
    pub(super) fn arguments(&mut self, field: u8) -> R<N> {
        let n = self.open(K::ArgumentList, field, C_NONE)?;
        self.list(K::ArgumentList, T::RParen, true, false, |p| p.argument())?;
        self.close(n);
        Ok(n)
    }

    /// `bracketed_argument_list` (as `kind`: an element binding's too): `[` arguments `]`.
    pub(super) fn bracketed_arguments(&mut self, kind: K, field: u8, class: u8) -> R<N> {
        let n = self.open(kind, field, class)?;
        self.list(kind, T::RBracket, true, true, |p| p.argument())?;
        self.close(n);
        Ok(n)
    }

    /// An `argument`: a name and `:`, `ref`/`out`/`in`, and an expression or a declaration.
    pub(super) fn argument(&mut self) -> R<()> {
        let n = self.open(K::Argument, f::NONE, C_NONE)?;
        if self.is_ident() && self.kind_at(1) == T::Colon {
            self.ident(slot(f::NAME, Ctx::Bare))?;
            self.leaf(K::TColon, f::NONE, C_NONE)?;
            // `name: new A<B, C> { … }`: tree-sitter reads `new A < B` and `C > { … }` (two
            // arguments); not read.
            if self.is_word(w::NEW) && self.is_ident_at(1) && self.kind_at(2) == T::Lt {
                let mut j = 3;
                while self.is_ident_at(j) && self.kind_at(j + 1) == T::Comma {
                    j += 2;
                }
                if j > 3 && self.is_ident_at(j) && self.kind_at(j + 1) == T::Gt && self.kind_at(j + 2) == T::LBrace {
                    return self.refuse();
                }
            }
        }
        if self.is_word(w::REF) || self.is_word(w::OUT) || self.is_word(w::IN) {
            let k = keyword(self.tok().word).expect("a keyword");
            self.leaf(k, f::NONE, C_NONE)?;
        }
        if self.declaration_expression_ahead(0) {
            self.declaration_expression(BARE)?;
        } else {
            let saved = std::mem::replace(&mut self.in_argument, true);
            let r = self.expr(EXPR);
            self.in_argument = saved;
            r?;
        }
        self.close(n);
        Ok(())
    }

    /// Whether a declaration expression (`var x`, `int x`, `A<B> c`) starts `k` tokens ahead: a
    /// type and a name, then `,` or `)`.
    pub(super) fn declaration_expression_ahead(&mut self, k: usize) -> bool {
        if self.is_word_at(k, w::REF) {
            return false;
        }
        let Some(e) = self.scan_type(k) else { return false };
        self.is_ident_at(e) && matches!(self.kind_at(e + 1), T::Comma | T::RParen)
    }

    /// `declaration_expression`: a type and a name.
    pub(super) fn declaration_expression(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::DeclarationExpression, s)?;
        self.ty(slot(f::TYPE, Ctx::Type))?;
        self.ident(slot(f::NAME, Ctx::Bare))?;
        self.close(n);
        Ok(n)
    }

    // ---- Creation.

    /// `new`: an object, an array, an implicit array or object, or an anonymous object.
    fn creation(&mut self, s: Slot) -> R<N> {
        match self.kind_at(1) {
            T::LBracket => {
                // `new[] { … }`: commas for its rank.
                let n = self.open_in(K::ImplicitArrayCreationExpression, s)?;
                self.leaf(K::TNew, f::NONE, C_NONE)?;
                let mut k = 1;
                while self.kind_at(k) == T::Comma {
                    k += 1;
                }
                if self.kind_at(k) != T::RBracket {
                    return self.refuse();
                }
                let keep = k > 1;
                self.punct_tk(K::ImplicitArrayCreationExpression, T::LBracket, keep)?;
                while self.peek() == T::Comma {
                    self.leaf(K::TComma, f::NONE, C_NONE)?;
                }
                self.punct_tk(K::ImplicitArrayCreationExpression, T::RBracket, keep)?;
                if self.peek() != T::LBrace {
                    return self.refuse();
                }
                self.initializer(BARE)?;
                self.close(n);
                Ok(n)
            }
            T::LParen => {
                let n = self.open_in(K::ImplicitObjectCreationExpression, s)?;
                self.tk(K::ImplicitObjectCreationExpression, K::TNew)?;
                self.arguments(f::NONE)?;
                if self.peek() == T::LBrace {
                    self.initializer(BARE)?;
                }
                self.close(n);
                Ok(n)
            }
            T::LBrace => {
                let n = self.open_in(K::AnonymousObjectCreationExpression, s)?;
                self.leaf(K::TNew, f::NONE, C_NONE)?;
                self.list(K::AnonymousObjectCreationExpression, T::RBrace, true, true, |p| {
                    if p.is_ident() && p.kind_at(1) == T::Eq {
                        p.ident(BARE)?;
                        p.leaf(K::TEq, f::NONE, C_NONE)?;
                    }
                    p.expr(EXPR)?;
                    Ok(())
                })?;
                self.close(n);
                Ok(n)
            }
            _ => {
                // `new T(…) { … }`, or `new T[…]…` (an array).
                let Some(base) = self.scan_type_base(1) else { return self.refuse() };
                if self.kind_at(base) == T::LBracket {
                    let n = self.open_in(K::ArrayCreationExpression, s)?;
                    self.tk(K::ArrayCreationExpression, K::TNew)?;
                    self.array_type_with_ranks(slot(f::TYPE, Ctx::Bare))?;
                    if self.peek() == T::LBrace {
                        self.initializer(BARE)?;
                    }
                    self.close(n);
                    return Ok(n);
                }
                let n = self.open_in(K::ObjectCreationExpression, s)?;
                self.tk(K::ObjectCreationExpression, K::TNew)?;
                self.ty(slot(f::TYPE, Ctx::Type))?;
                if self.peek() == T::LParen {
                    self.arguments(f::ARGUMENTS)?;
                }
                if self.peek() == T::LBrace {
                    self.initializer(slot(f::INITIALIZER, Ctx::Bare))?;
                }
                self.close(n);
                Ok(n)
            }
        }
    }

    /// `stackalloc T[n] { … }` or `stackalloc[] { … }`.
    fn stackalloc(&mut self, s: Slot) -> R<N> {
        if self.kind_at(1) == T::LBracket {
            let n = self.open_in(K::ImplicitStackallocExpression, s)?;
            self.tk(K::ImplicitStackallocExpression, K::TStackalloc)?;
            self.punct_tk(K::ImplicitStackallocExpression, T::LBracket, false)?;
            self.punct_tk(K::ImplicitStackallocExpression, T::RBracket, false)?;
            if self.peek() != T::LBrace {
                return self.refuse();
            }
            self.initializer(BARE)?;
            self.close(n);
            return Ok(n);
        }
        let n = self.open_in(K::StackallocExpression, s)?;
        self.tk(K::StackallocExpression, K::TStackalloc)?;
        self.array_type_with_ranks(slot(f::TYPE, Ctx::Bare))?;
        if self.peek() == T::LBrace {
            self.initializer(BARE)?;
        }
        self.close(n);
        Ok(n)
    }

    /// `initializer_expression`: `{` expressions `}`, a trailing `,` allowed.
    pub(super) fn initializer(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::InitializerExpression, s)?;
        self.list(K::InitializerExpression, T::RBrace, true, true, |p| {
            if p.peek() == T::LBracket && p.element_binding_ahead() {
                // `[k] = v`: an element binding as an assignment's left.
                let b = p.bracketed_arguments(K::ElementBindingExpression, f::NONE, C_NONE)?;
                let Some(op) = assignment(p.peek()) else { return p.refuse() };
                let a = p.wrap(b, K::AssignmentExpression, EXPR, slot(f::LEFT, Ctx::Lv));
                p.leaf(op, f::OPERATOR, C_NONE)?;
                p.expr(slot(f::RIGHT, Ctx::Expr))?;
                p.close(a);
                return Ok(());
            }
            p.expr(EXPR)?;
            Ok(())
        })?;
        self.close(n);
        Ok(n)
    }

    /// Whether `[` … `]` next is followed by an assignment operator.
    fn element_binding_ahead(&mut self) -> bool {
        self.matching(0).is_some_and(|e| assignment(self.kind_at(e + 1)).is_some())
    }

    /// `collection_expression`: `[` elements `]`, a trailing `,` allowed.
    fn collection(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::CollectionExpression, s)?;
        self.list(K::CollectionExpression, T::RBracket, true, true, |p| {
            let e = p.open(K::CollectionElement, f::NONE, C_NONE)?;
            if p.peek() == T::DotDot {
                let sp = p.open(K::SpreadElement, f::NONE, C_NONE)?;
                p.leaf(K::TDotDot, f::NONE, C_NONE)?;
                p.expr(EXPR)?;
                p.close(sp);
            } else {
                let x = p.open(K::ExpressionElement, f::NONE, C_NONE)?;
                p.expr(EXPR)?;
                p.close(x);
            }
            p.close(e);
            Ok(())
        })?;
        self.close(n);
        Ok(n)
    }

    /// `delegate (params) { … }`.
    fn anonymous_method(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::AnonymousMethodExpression, s)?;
        self.tk(K::AnonymousMethodExpression, K::TDelegate)?;
        if self.peek() == T::LParen {
            self.parameter_list(K::ParameterList, f::PARAMETERS)?;
        }
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        self.block(BARE)?;
        self.close(n);
        Ok(n)
    }

    // ---- Lambdas.

    /// Whether a lambda starts here: `static`/`async` modifiers, then `x =>` or `(…) =>`.
    pub(super) fn lambda_ahead(&mut self) -> bool {
        let mut k = 0;
        while self.is_word_at(k, w::STATIC) || (self.is_word_at(k, w::ASYNC) && (self.is_ident_at(k + 1) || self.kind_at(k + 1) == T::LParen)) {
            k += 1;
        }
        if self.is_ident_at(k) && self.kind_at(k + 1) == T::FatArrow {
            return true;
        }
        if self.kind_at(k) == T::LParen {
            return self.matching(k).is_some_and(|e| self.kind_at(e + 1) == T::FatArrow);
        }
        false
    }

    /// A lambda: modifiers, parameters, `=>` and a block or an expression.
    fn lambda(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::LambdaExpression, s)?;
        let mut bare = true;
        while self.is_word(w::STATIC) || self.is_word(w::ASYNC) {
            self.leaf(K::Modifier, f::NONE, C_NONE)?;
            bare = false;
        }
        if self.peek() == T::LParen {
            let saved = self.bare_lambda;
            self.bare_lambda = bare;
            self.parameter_list(K::ParameterList, f::PARAMETERS)?;
            self.bare_lambda = saved;
        } else {
            let t = self.tok();
            if w::is_reserved_identifier(t.word) {
                let p = self.open(K::ImplicitParameter, f::PARAMETERS, C_NONE)?;
                self.leaf(keyword(t.word).expect("a reserved identifier"), f::NONE, C_NONE)?;
                self.close(p);
            } else {
                self.leaf(K::ImplicitParameter, f::PARAMETERS, C_NONE)?;
            }
        }
        if self.peek() != T::FatArrow {
            return self.refuse();
        }
        self.leaf(K::TFatArrow, f::NONE, C_NONE)?;
        if self.peek() == T::LBrace {
            self.block(slot(f::BODY, Ctx::Bare))?;
        } else {
            self.expr(slot(f::BODY, Ctx::Expr))?;
        }
        self.close(n);
        Ok(n)
    }

    // ---- Literals.

    /// `character_literal`: its content or escape between its quotes.
    fn char_literal(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        let n = self.open_in(K::CharacterLiteral, s)?;
        self.bump()?;
        let (a, e) = (t.start as usize + 1, t.end as usize - 1);
        let kind = if self.src[a] == b'\\' { K::EscapeSequence } else { K::CharacterLiteralContent };
        self.piece(kind, f::NONE, a, e);
        self.close(n);
        Ok(n)
    }

    /// `string_literal`: its pieces (content runs and escapes), and its `u8`.
    fn string(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        let n = self.open_in(K::StringLiteral, s)?;
        self.bump()?;
        let src = self.src;
        let (start, mut end) = (t.start as usize, t.end as usize);
        let encoded = src[end - 1] == b'8';
        if encoded {
            end -= 2;
        }
        let close = end - 1;
        let mut at = start + 1;
        while at < close {
            if src[at] == b'\\' {
                let Some(e) = self.lx.escape_end(at) else { return self.refuse() };
                self.piece(K::EscapeSequence, f::NONE, at, e);
                at = e;
            } else {
                let e = self.lx.blocks.string_stop(at).min(close);
                self.piece(K::StringLiteralContent, f::NONE, at, e);
                at = e;
            }
        }
        if encoded {
            self.piece(K::StringLiteralEncoding, f::NONE, end, end + 2);
        }
        self.close(n);
        Ok(n)
    }

    /// `raw_string_literal`: its start, content and end.
    fn raw_string(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        let (start, end) = (t.start as usize, t.end as usize);
        if self.src[end - 1] == b'8' {
            return self.refuse();
        }
        let q = self.lx.quotes(start);
        let n = self.open_in(K::RawStringLiteral, s)?;
        self.bump()?;
        self.piece(K::RawStringStart, f::NONE, start, start + q);
        if end - q > start + q {
            self.piece(K::RawStringContent, f::NONE, start + q, end - q);
        }
        self.piece(K::RawStringEnd, f::NONE, end - q, end);
        self.close(n);
        Ok(n)
    }

    /// `interpolated_string_expression`: its start, pieces and interpolations, each read again
    /// from its bytes (the lexer took the whole literal as one token).
    fn interpolated(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        let src = self.src;
        let (start, end) = (t.start as usize, t.end as usize);
        let Some((dollars, verbatim, quotes, content)) = self.lx.interpolation_start(start) else { return self.refuse() };
        let n = self.open_in(K::InterpolatedStringExpression, s)?;
        self.bump()?;
        let open_quote = content - quotes;
        self.piece(K::InterpolationStart, f::NONE, start, open_quote);
        if quotes > 1 {
            self.piece(K::InterpolationQuote, f::NONE, open_quote, content);
        }
        let close = end - quotes;
        let regular = !verbatim && quotes == 1;
        let raw = quotes > 1;
        // As tree-sitter-c-sharp's scanner reads it: at each place, an escape (a regular
        // string's), or the braces of an interpolation (exactly `dollars` of them, no `{` after),
        // or a run of content. A run takes the braces the interpolation did not, and ends before
        // any other `{` (and at a quote, or a regular string's `\` or line end).
        let mut at = content;
        let mut relexed = false;
        while at < close {
            if regular && src[at] == b'\\' {
                let Some(e) = self.lx.escape_end(at) else { return self.refuse() };
                self.piece(K::EscapeSequence, f::NONE, at, e);
                at = e;
                continue;
            }
            let mut braces = 0;
            while braces < dollars && src.get(at + braces) == Some(&b'{') {
                braces += 1;
            }
            if braces == dollars && src.get(at + braces) != Some(&b'{') {
                at = self.interpolation(at, dollars)?;
                relexed = true;
                continue;
            }
            let (from, mut pos, mut held) = (at, at + braces, braces);
            let mut moved = false;
            let stop = loop {
                if pos >= close {
                    break close;
                }
                let c = src[pos];
                if raw {
                    // Quotes end it only as many as opened the string.
                    if c == b'"' {
                        let q = self.lx.quotes(pos);
                        if q == quotes {
                            break pos;
                        }
                        pos += q;
                        held = 0;
                        moved = true;
                        continue;
                    }
                } else if verbatim {
                    if c == b'"' {
                        if src.get(pos + 1) == Some(&b'"') {
                            pos += 2;
                            continue;
                        }
                        break pos;
                    }
                } else if matches!(c, b'\\' | b'\n' | b'"') {
                    break pos;
                }
                if c == b'{' && held == 0 {
                    break pos;
                }
                if c != b'{' {
                    held = 0;
                }
                pos += 1;
                moved = true;
            };
            // No run (tree-sitter's scanner gives no token), or one stopped at a quote inside (its
            // string ends there, not where C#'s does) or a line end: not read.
            if !moved || (stop < close && matches!(src[stop], b'"' | b'\n')) {
                return self.refuse();
            }
            self.piece(K::StringContent, f::NONE, from, stop);
            at = stop;
        }
        if quotes > 1 {
            self.piece(K::InterpolationQuote, f::NONE, close, end);
        }
        if relexed {
            self.relex_at(end);
            // Extras lexed past a hole, in the string's content, are none.
            let e = self.e;
            let stale = self.extras[e..].partition_point(|x| (x.start as usize) < end);
            self.extras.drain(e..e + stale);
        }
        self.last_end = end as u32;
        self.close(n);
        Ok(n)
    }

    /// An `interpolation` from its braces at `at`: its expression, alignment and format, read
    /// over tokens lexed again from its bytes. Past its closing braces.
    fn interpolation(&mut self, at: usize, braces: usize) -> R<usize> {
        let i = self.open_at(K::Interpolation, f::NONE, C_NONE, at as u32);
        self.piece(K::InterpolationBrace, f::NONE, at, at + braces);
        self.relex_at(at + braces);
        self.last_end = (at + braces) as u32;
        self.expr(EXPR)?;
        if self.peek() == T::Comma {
            let a = self.open(K::InterpolationAlignmentClause, f::NONE, C_NONE)?;
            self.tk(K::InterpolationAlignmentClause, K::TComma)?;
            self.expr(EXPR)?;
            self.close(a);
        }
        let mut e = self.tok().start as usize;
        if self.peek() == T::Colon {
            // The format: `:` and what follows to the `}` (`[^}"]+`).
            self.flush(e as u32)?;
            let mut k = e + 1;
            while k < self.src.len() && self.src[k] != b'}' && self.src[k] != b'"' {
                k += 1;
            }
            if k == e + 1 {
                return self.refuse();
            }
            self.piece(K::InterpolationFormatClause, f::NONE, e, k);
            e = k;
        } else if self.peek() != T::RBrace {
            return self.refuse();
        } else {
            self.flush(e as u32)?;
        }
        let mut b = 0;
        while self.src.get(e + b) == Some(&b'}') && b < braces {
            b += 1;
        }
        if b != braces {
            return self.refuse();
        }
        self.piece(K::InterpolationBrace, f::NONE, e, e + braces);
        self.w.close(i.at, (e + braces) as u32);
        self.last_end = (e + braces) as u32;
        Ok(e + braces)
    }

    // ---- Queries.

    /// Whether `from` starts a query: `from [type] name in`.
    fn query_ahead(&mut self) -> bool {
        if self.is_ident_at(1) && self.is_word_at(2, w::IN) {
            return true;
        }
        self.scan_type(1).is_some_and(|e| self.is_ident_at(e) && self.is_word_at(e + 1, w::IN))
    }

    /// `query_expression`: a `from` clause, then clauses to a `select` or a `group`, and `into`
    /// continuations.
    fn query(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::QueryExpression, s)?;
        self.from_clause()?;
        loop {
            match self.word_at(0) {
                w::FROM if self.query_ahead() => self.from_clause()?,
                w::WHERE => {
                    let c = self.open(K::WhereClause, f::NONE, C_NONE)?;
                    self.leaf(K::TWhere, f::NONE, C_NONE)?;
                    self.query_expr()?;
                    self.close(c);
                }
                w::LET => {
                    let c = self.open(K::LetClause, f::NONE, C_NONE)?;
                    self.tk(K::LetClause, K::TLet)?;
                    self.ident(BARE)?;
                    self.punct_tk(K::LetClause, T::Eq, true)?;
                    self.query_expr()?;
                    self.close(c);
                }
                w::ORDERBY => {
                    let c = self.open(K::OrderByClause, f::NONE, C_NONE)?;
                    self.leaf(K::TOrderby, f::NONE, C_NONE)?;
                    loop {
                        self.query_expr()?;
                        if self.is_word(w::ASCENDING) || self.is_word(w::DESCENDING) {
                            let k = keyword(self.tok().word).expect("a keyword");
                            self.leaf(k, f::NONE, C_NONE)?;
                        }
                        if self.peek() != T::Comma {
                            break;
                        }
                        self.tk(K::OrderByClause, K::TComma)?;
                    }
                    self.close(c);
                }
                w::JOIN => {
                    let c = self.open(K::JoinClause, f::NONE, C_NONE)?;
                    self.tk(K::JoinClause, K::TJoin)?;
                    if !(self.is_ident_at(0) && self.is_word_at(1, w::IN)) {
                        self.ty(slot(f::TYPE, Ctx::Type))?;
                    }
                    self.ident(BARE)?;
                    self.kw(K::JoinClause, w::IN)?;
                    self.query_expr()?;
                    self.kw(K::JoinClause, w::ON)?;
                    self.query_expr()?;
                    self.kw(K::JoinClause, w::EQUALS)?;
                    self.query_expr()?;
                    if self.is_word(w::INTO) {
                        let j = self.open(K::JoinIntoClause, f::NONE, C_NONE)?;
                        self.tk(K::JoinIntoClause, K::TInto)?;
                        self.ident(BARE)?;
                        self.close(j);
                    }
                    self.close(c);
                }
                w::SELECT => {
                    let c = self.open(K::SelectClause, f::NONE, C_NONE)?;
                    self.leaf(K::TSelect, f::NONE, C_NONE)?;
                    self.query_expr()?;
                    self.close(c);
                    if !self.into_ahead() {
                        break;
                    }
                    self.into()?;
                }
                w::GROUP => {
                    let c = self.open(K::GroupClause, f::NONE, C_NONE)?;
                    self.tk(K::GroupClause, K::TGroup)?;
                    self.query_expr()?;
                    self.kw(K::GroupClause, w::BY)?;
                    self.query_expr()?;
                    self.close(c);
                    if !self.into_ahead() {
                        break;
                    }
                    self.into()?;
                }
                _ => return self.refuse(),
            }
        }
        self.close(n);
        Ok(n)
    }

    fn into_ahead(&mut self) -> bool {
        self.is_word(w::INTO) && self.is_ident_at(1)
    }

    /// A query continuation's `into name`, written into the query.
    fn into(&mut self) -> R<()> {
        self.tk(K::QueryExpression, K::TInto)?;
        self.ident(BARE)?;
        Ok(())
    }

    /// `from_clause`: `from [type] name in expression`.
    fn from_clause(&mut self) -> R<()> {
        let c = self.open(K::FromClause, f::NONE, C_NONE)?;
        self.tk(K::FromClause, K::TFrom)?;
        if !(self.is_ident_at(0) && self.is_word_at(1, w::IN)) {
            self.ty(slot(f::TYPE, Ctx::Type))?;
        }
        self.ident(slot(f::NAME, Ctx::Bare))?;
        self.kw(K::FromClause, w::IN)?;
        self.query_expr()?;
        self.close(c);
        Ok(())
    }

    /// An expression in a query clause.
    fn query_expr(&mut self) -> R<N> {
        self.expr(EXPR)
    }
}
