//! Types (`_type` and its forms). Each reader returns the node it wrote (its place), the caller
//! giving it its field (`Tw::refield`).

use super::P;
use crate::kinds::{K, f};
use crate::lex::{R, T, op, w};

/// Where a type is read: what ends it.
#[derive(Clone, Copy, Default)]
pub(crate) struct Tx {
    /// A `=>` after it is not a function type's (a case's, a lambda's parameter's).
    pub no_arrow: bool,
}

impl P<'_> {
    /// `_type`, given `field`: its place.
    pub(super) fn typ_f(&mut self, field: u8) -> R<u32> {
        self.typ_x(field, Tx::default())
    }

    pub(super) fn typ_x(&mut self, field: u8, tx: Tx) -> R<u32> {
        self.deeper()?;
        let n = self.typ(tx)?;
        self.depth -= 1;
        if field != f::NONE {
            self.t.refield(n, field);
        }
        Ok(n)
    }

    /// `_type`: a function type (`A => B`, `(A, B) => C`), or an infix one.
    fn typ(&mut self, tx: Tx) -> R<u32> {
        let m = self.mark();
        if self.peek() == T::LBracket {
            // A type lambda, a polymorphic function type: refused.
            return self.refuse();
        }
        if self.peek() == T::LParen && !tx.no_arrow && self.arrow_after_parens(0) {
            // `(A, B) => C`: the parameters in `parameter_types`.
            let pt = self.open_field(K::ParameterTypes, f::PARAMETER_TYPES);
            self.tk(K::TLParen)?;
            while self.peek() != T::RParen {
                self.param_type(f::NONE)?;
                if self.peek() == T::Comma {
                    self.tk(K::TComma)?;
                } else {
                    break;
                }
            }
            self.expect(T::RParen)?;
            self.close(pt);
            let n = self.t.wrap(m, K::FunctionType, f::NONE);
            self.arrow_then_type()?;
            let n = self.close(n);
            self.t.unmark(m);
            return Ok(n);
        }
        let first = self.infix_type(tx)?;
        if !tx.no_arrow && self.type_arrow_next() {
            self.t.refield(first, f::NONE);
            let pt = self.t.wrap(m, K::ParameterTypes, f::PARAMETER_TYPES);
            self.close(pt);
            let n = self.t.wrap(m, K::FunctionType, f::NONE);
            self.arrow_then_type()?;
            let n = self.close(n);
            self.t.unmark(m);
            return Ok(n);
        }
        self.t.unmark(m);
        Ok(first)
    }

    /// Whether a function type's arrow is next (`=>`, `?=>`, `->`, `?->`).
    fn type_arrow_next(&mut self) -> bool {
        matches!(self.op_at(0), op::ARROW | op::QARROW | op::THIN | op::QTHIN)
    }

    /// `_arrow_then_type`: the arrow, then `return_type`.
    fn arrow_then_type(&mut self) -> R<()> {
        let code = self.op_at(0);
        match code {
            op::ARROW | op::QARROW | op::THIN | op::QTHIN => {
                self.tk(super::op_kind(code).expect("an arrow"))?;
            }
            _ => return self.refuse(),
        }
        if self.peek() == T::LBrace {
            // A capture set: refused.
            return self.refuse();
        }
        self.typ_f(f::RETURN_TYPE)?;
        Ok(())
    }

    /// Whether the `(` `k` tokens ahead closes before `=>` (a function type's parameters).
    pub(super) fn arrow_after_parens(&mut self, k: usize) -> bool {
        let (mut d, mut j) = (0i32, k);
        loop {
            match self.kind_at(j) {
                T::LParen | T::LBracket | T::LBrace => d += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    d -= 1;
                    if d == 0 {
                        return matches!(self.op_at(j + 1), op::ARROW | op::QARROW | op::THIN | op::QTHIN);
                    }
                }
                T::Eof => return false,
                _ => {}
            }
            j += 1;
            if j > k + 400 {
                return false;
            }
        }
    }

    /// `_param_type`: `=> T` (lazy), `T*` (repeated), or a type (field `type` in each).
    pub(super) fn param_type(&mut self, field: u8) -> R<u32> {
        self.param_type_x(field, Tx::default())
    }

    pub(super) fn param_type_x(&mut self, field: u8, tx: Tx) -> R<u32> {
        if matches!(self.op_at(0), op::ARROW | op::THIN | op::QTHIN) {
            let n = self.open_field(K::LazyParameterType, field);
            let code = self.op_at(0);
            self.tk(super::op_kind(code).expect("an arrow"))?;
            if self.peek() == T::LBrace {
                return self.refuse();
            }
            self.param_value_type(f::TYPE, tx)?;
            return Ok(self.close(n));
        }
        self.param_value_type(field, tx)
    }

    /// `_param_value_type`: a type, or `T*`.
    fn param_value_type(&mut self, field: u8, tx: Tx) -> R<u32> {
        let m = self.mark();
        let t = self.typ_x(f::TYPE, tx)?;
        if self.is_op(op::STAR) {
            let n = self.t.wrap(m, K::RepeatedParameterType, field);
            self.tk(K::TStar)?;
            let n = self.close(n);
            self.t.unmark(m);
            return Ok(n);
        }
        self.t.unmark(m);
        self.t.refield(t, if field == f::NONE { f::TYPE } else { field });
        Ok(t)
    }

    /// A bound's type: `<:`, `>:` and `<%` after it are the next bounds' (`A >: L <: U`).
    fn bound_type(&mut self) -> R<u32> {
        let saved = std::mem::replace(&mut self.in_bound, true);
        let r = self.typ_f(f::TYPE);
        self.in_bound = saved;
        r
    }

    /// An infix type (`A op B`, left to right), or a compound, annotated or simple one.
    /// Infix types are left-associative, all of one precedence; an alphanumeric operator is an
    /// `identifier`, a symbolic one an `operator_identifier`. In a typed pattern (`no_arrow`) a `|`
    /// is the pattern's alternative.
    fn infix_type(&mut self, tx: Tx) -> R<u32> {
        let m = self.mark();
        let mut n = self.compound_type()?;
        loop {
            let t = self.tok();
            // No automatic semicolon is valid inside a type: `&` or `|` starting the next line
            // continues it.
            if t.nl && !(t.kind == T::Op && (t.code == op::PIPE || self.text(t) == b"&")) {
                break;
            }
            let alpha = match t.kind {
                T::Op if self.in_bound && matches!(t.code, op::SUBTYPE | op::SUPERTYPE | op::VIEW) => break,
                T::Op if t.code == op::PIPE && tx.no_arrow => break,
                T::Op if matches!(t.code, op::ARROW | op::QARROW | op::THIN | op::QTHIN | op::EQ | op::COLON | op::LARROW | op::HASH | op::AT | op::TYPE_LAMBDA | op::STAR) => break,
                T::Op => false,
                T::Id if w::is_reserved(t.code) || matches!(t.code, w::WITH | w::FORSOME | w::MATCH | w::IF | w::THEN | w::ELSE | w::DERIVES | w::USES | w::AS | w::UNDERSCORE) => break,
                T::Id | T::BackId => true,
                _ => break,
            };
            if alpha {
                // A type must follow on the line.
                let nx = self.peek_at(1);
                if nx.nl || !(Self::is_name(nx) || matches!(nx.kind, T::LParen)) {
                    break;
                }
            }
            self.t.refield(n, f::LEFT);
            let w = self.t.wrap(m, K::InfixType, f::NONE);
            self.leaf(if alpha { K::Identifier } else { K::OperatorIdentifier }, f::OPERATOR)?;
            let r = self.compound_type()?;
            self.t.refield(r, f::RIGHT);
            n = self.close(w);
        }
        self.t.unmark(m);
        Ok(n)
    }

    /// `A with B with C` (`compound_type`: `base`, `extra`), else an annotated type.
    fn compound_type(&mut self) -> R<u32> {
        let m = self.mark();
        let b = self.annotated_type()?;
        // `{` on the line: a refinement (`A { type T }`); only types reach here (a template's
        // `extends` and `new` read `annotated_type`, the `{` theirs).
        let refined = |p: &mut Self| p.peek() == T::LBrace && !p.tok().nl;
        if !self.is_word(w::WITH) && !refined(self) {
            self.t.unmark(m);
            return Ok(b);
        }
        self.t.refield(b, f::BASE);
        let n = self.t.wrap(m, K::CompoundType, f::NONE);
        while self.is_word(w::WITH) {
            self.tk(K::TWith)?;
            let x = self.annotated_type()?;
            self.t.refield(x, f::EXTRA);
        }
        if refined(self) {
            self.refinement()?;
        }
        let n = self.close(n);
        self.t.unmark(m);
        Ok(n)
    }

    /// `_annotated_type`: a simple type, its annotations after it (`annotated_type`).
    pub(super) fn annotated_type(&mut self) -> R<u32> {
        let m = self.mark();
        let n = self.simple_type()?;
        if self.is_op(op::AT) && !self.tok().nl {
            let a = self.t.wrap(m, K::AnnotatedType, f::NONE);
            while self.is_op(op::AT) {
                self.annotation()?;
            }
            let a = self.close(a);
            self.t.unmark(m);
            return Ok(a);
        }
        self.t.unmark(m);
        Ok(n)
    }

    /// `_simple_type`: a name, a path (`a.b.C`, `a.type`), a tuple, `_`, then type arguments and
    /// projections after it.
    pub(super) fn simple_type(&mut self) -> R<u32> {
        let m = self.mark();
        let t = self.tok();
        let mut n = match t.kind {
            T::Id if t.code == w::UNDERSCORE => {
                let n = self.open(K::Wildcard);
                self.tk(K::TUnderscore)?;
                self.close(n)
            }
            T::Id | T::BackId if !w::is_reserved(t.code) || matches!(t.code, w::THIS | w::SUPER) => self.type_path()?,
            T::Op if t.code == op::QUESTION || t.code == op::NONE && !t.nl => self.leaf(K::TypeIdentifier, f::NONE)?,
            // A kind projector's `*` (`Map[K, *]`): a type identifier.
            T::Op if t.code == op::STAR && matches!(self.kind_at(1), T::Comma | T::RBracket) => self.leaf(K::TypeIdentifier, f::NONE)?,
            T::LParen => self.tuple_type()?,
            T::Int | T::Float | T::Char | T::Str => {
                // A literal type: refused.
                return self.refuse();
            }
            _ => return self.refuse(),
        };
        loop {
            match self.peek() {
                T::LBracket => {
                    self.t.refield(n, f::TYPE);
                    let g = self.t.wrap(m, K::GenericType, f::NONE);
                    self.type_arguments(f::TYPE_ARGUMENTS)?;
                    n = self.close(g);
                }
                T::Op if self.is_op(op::HASH) => {
                    self.t.refield(n, f::TYPE);
                    let g = self.t.wrap(m, K::ProjectedType, f::NONE);
                    self.tk(K::THash)?;
                    self.ident_as(K::TypeIdentifier, f::SELECTOR)?;
                    n = self.close(g);
                }
                _ => break,
            }
        }
        self.t.unmark(m);
        Ok(n)
    }

    /// A type's path: `C` (`type_identifier`), `a.C` (`stable_type_identifier`), `a.b.C` (its
    /// prefix a `stable_identifier`), `a.type` (`singleton_type`).
    pub(super) fn type_path(&mut self) -> R<u32> {
        // The path's names: up to a name not followed by `.` and a name.
        let mut k = 0;
        while self.kind_at(k + 1) == T::Dot && matches!(self.kind_at(k + 2), T::Id | T::BackId) && !self.peek_at(k + 1).nl {
            k += 2;
        }
        let names = k / 2 + 1;
        if names == 1 {
            return self.ident_as(K::TypeIdentifier, f::NONE);
        }
        let last_type = self.word_at(k) == w::TYPE;
        let m = self.mark();
        // The prefix: `a` an identifier, `a.b` a stable identifier, nested to the left.
        let prefix = names - 1;
        self.ident_as(K::Identifier, f::NONE)?;
        for _ in 1..prefix {
            let s = self.t.wrap(m, K::StableIdentifier, f::NONE);
            self.tk(K::TDot)?;
            self.ident_as(K::Identifier, f::NONE)?;
            self.close(s);
        }
        if last_type {
            let s = self.t.wrap(m, K::SingletonType, f::NONE);
            self.tk(K::TDot)?;
            self.tk(K::TType)?;
            let n = self.close(s);
            self.t.unmark(m);
            return Ok(n);
        }
        let s = self.t.wrap(m, K::StableTypeIdentifier, f::NONE);
        self.tk(K::TDot)?;
        self.ident_as(K::TypeIdentifier, f::NONE)?;
        let n = self.close(s);
        self.t.unmark(m);
        Ok(n)
    }

    /// `(A, B)` (`tuple_type`).
    fn tuple_type(&mut self) -> R<u32> {
        if Self::is_name(self.peek_at(1)) && self.is_op_at(2, op::COLON) {
            // `(a: A, b: B)` (`named_tuple_type`): each a `name_and_type` (`name`, `type`).
            let n = self.open(K::NamedTupleType);
            self.tk(K::TLParen)?;
            loop {
                let x = self.open(K::NameAndType);
                self.identifier(f::NAME)?;
                self.tk(K::TColon)?;
                self.typ_f(f::TYPE)?;
                self.close(x);
                if self.peek() == T::Comma {
                    self.tk(K::TComma)?;
                    if self.peek() == T::RParen {
                        break;
                    }
                } else {
                    break;
                }
            }
            self.expect(T::RParen)?;
            return Ok(self.close(n));
        }
        let n = self.open(K::TupleType);
        self.tk(K::TLParen)?;
        loop {
            self.typ_f(f::NONE)?;
            if self.peek() == T::Comma {
                self.tk(K::TComma)?;
                if self.peek() == T::RParen {
                    break;
                }
            } else {
                break;
            }
        }
        self.expect(T::RParen)?;
        Ok(self.close(n))
    }

    /// `[A, B]` (`type_arguments`), given `field`.
    pub(super) fn type_arguments(&mut self, field: u8) -> R<u32> {
        // Inside the brackets a bound's operators are infix ones again (`A <: F[_ <: B]`).
        let saved = std::mem::replace(&mut self.in_bound, false);
        let r = self.type_arguments_in(field);
        self.in_bound = saved;
        r
    }

    fn type_arguments_in(&mut self, field: u8) -> R<u32> {
        let n = self.open_field(K::TypeArguments, field);
        self.tk(K::TLBracket)?;
        loop {
            if self.kind_at(1) == T::Op && self.op_at(1) == op::EQ {
                // A named type argument: refused.
                return self.refuse();
            }
            self.typ_f(f::NONE)?;
            if self.peek() == T::Comma {
                self.tk(K::TComma)?;
                if self.peek() == T::RBracket {
                    break;
                }
            } else {
                break;
            }
        }
        self.expect(T::RBracket)?;
        Ok(self.close(n))
    }

    /// `[A, +B <: C]` (`type_parameters`), given `field`.
    pub(super) fn type_parameters(&mut self, field: u8) -> R<u32> {
        let n = self.open_field(K::TypeParameters, field);
        self.tk(K::TLBracket)?;
        loop {
            while self.is_op(op::AT) {
                self.annotation()?;
            }
            match self.op_at(0) {
                op::PLUS => {
                    let c = self.open(K::CovariantTypeParameter);
                    self.tk(K::TPlus)?;
                    self.type_parameter()?;
                    self.close(c);
                }
                op::MINUS => {
                    let c = self.open(K::ContravariantTypeParameter);
                    self.tk(K::TMinus)?;
                    self.type_parameter()?;
                    self.close(c);
                }
                _ => self.type_parameter()?,
            }
            if self.peek() == T::Comma {
                self.tk(K::TComma)?;
                if self.peek() == T::RBracket {
                    break;
                }
            } else {
                break;
            }
        }
        self.expect(T::RBracket)?;
        Ok(self.close(n))
    }

    /// `_type_parameter`: a name (`_` too), its own type parameters, its bounds.
    fn type_parameter(&mut self) -> R<()> {
        if self.is_word(w::UNDERSCORE) {
            let n = self.open_field(K::Wildcard, f::NAME);
            self.tk(K::TUnderscore)?;
            self.close(n);
        } else {
            self.identifier(f::NAME)?;
        }
        if self.is_op(op::CARET) {
            return self.refuse();
        }
        if self.peek() == T::LBracket {
            self.type_parameters(f::TYPE_PARAMETERS)?;
        }
        self.bounds(true)
    }

    /// A type parameter's or type definition's bounds (`bound`): lower, upper, view bounds
    /// (`view`), context bounds.
    pub(super) fn bounds(&mut self, view: bool) -> R<()> {
        if self.is_op(op::SUPERTYPE) {
            let b = self.open_field(K::LowerBound, f::BOUND);
            self.tk(K::TSupertype)?;
            self.bound_type()?;
            self.close(b);
        }
        if self.is_op(op::SUBTYPE) {
            let b = self.open_field(K::UpperBound, f::BOUND);
            self.tk(K::TSubtype)?;
            self.bound_type()?;
            self.close(b);
        }
        while view && self.is_op(op::VIEW) {
            let b = self.open_field(K::ViewBound, f::BOUND);
            self.tk(K::TViewBound)?;
            self.typ_f(f::TYPE)?;
            self.close(b);
        }
        while self.is_op(op::COLON) {
            // `_context_bounds`, field `bound` on its `:` too.
            self.tkf(K::TColon, f::BOUND)?;
            if self.peek() == T::LBrace {
                return self.refuse();
            }
            let b = self.open_field(K::ContextBound, f::BOUND);
            self.typ_f(f::TYPE)?;
            if self.is_word(w::AS) {
                return self.refuse();
            }
            self.close(b);
        }
        Ok(())
    }
}
