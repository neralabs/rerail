//! Patterns (`_pattern`): `a | b` (alternative), `p: T` (typed), `p*` (repeat), `a :: b` (infix,
//! left to right), `x @ p` (capture), `C(p, q)` (case class), tuples, paths, literals, `_`. Each
//! reader returns the node it wrote, the caller giving it its field.

use super::P;
use super::types::Tx;
use crate::kinds::{K, f};
use crate::lex::{R, T, op, w};

impl P<'_> {
    /// `_pattern`, given `field`.
    pub(super) fn pattern_f(&mut self, field: u8) -> R<u32> {
        self.deeper()?;
        let n = self.pattern()?;
        self.depth -= 1;
        if field != f::NONE {
            self.t.refield(n, field);
        }
        Ok(n)
    }

    fn pattern(&mut self) -> R<u32> {
        let m = self.mark();
        let mut n = self.typed_pattern()?;
        while self.is_op(op::PIPE) {
            let a = self.t.wrap(m, K::AlternativePattern, f::NONE);
            self.tk(K::TPipe)?;
            self.typed_pattern()?;
            n = self.close(a);
        }
        self.t.unmark(m);
        Ok(n)
    }

    /// `p: T` (`typed_pattern`: `pattern`, `type`), `p*` (`repeat_pattern`), else an infix one.
    fn typed_pattern(&mut self) -> R<u32> {
        let m = self.mark();
        let mut n = self.infix_pattern()?;
        loop {
            if self.is_op(op::COLON) {
                self.t.refield(n, f::PATTERN);
                let t = self.t.wrap(m, K::TypedPattern, f::NONE);
                self.tk(K::TColon)?;
                self.typ_x(f::TYPE, Tx { no_arrow: true })?;
                n = self.close(t);
            } else if self.is_op(op::STAR) {
                self.t.refield(n, f::PATTERN);
                let t = self.t.wrap(m, K::RepeatPattern, f::NONE);
                self.tk(K::TStar)?;
                n = self.close(t);
            } else {
                break;
            }
        }
        self.t.unmark(m);
        Ok(n)
    }

    /// `a op b` (`infix_pattern`: `left`, `operator`, `right`), left to right.
    fn infix_pattern(&mut self) -> R<u32> {
        let m = self.mark();
        let mut n = self.simple_pattern()?;
        loop {
            let t = self.tok();
            let infix = t.kind == T::Op && !op::is_reserved(t.code) && !matches!(t.code, op::ARROW | op::PIPE | op::STAR | op::QARROW) || t.kind == T::BackId;
            if !infix {
                break;
            }
            self.t.refield(n, f::LEFT);
            let i = self.t.wrap(m, K::InfixPattern, f::NONE);
            if t.kind == T::BackId {
                self.leaf(K::Identifier, f::OPERATOR)?;
            } else {
                self.leaf(K::OperatorIdentifier, f::OPERATOR)?;
            }
            let r = self.simple_pattern()?;
            self.t.refield(r, f::RIGHT);
            n = self.close(i);
        }
        self.t.unmark(m);
        Ok(n)
    }

    /// `_definition_pattern`'s atoms.
    fn simple_pattern(&mut self) -> R<u32> {
        let t = self.tok();
        match t.kind {
            T::Id if t.code == w::UNDERSCORE => {
                let n = self.open(K::Wildcard);
                self.tk(K::TUnderscore)?;
                Ok(self.close(n))
            }
            T::Id if matches!(t.code, w::TRUE | w::FALSE) => {
                let n = self.open(K::BooleanLiteral);
                self.tk(if t.code == w::TRUE { K::TTrue } else { K::TFalse })?;
                Ok(self.close(n))
            }
            T::Id if t.code == w::NULL => {
                self.leaf(K::NullLiteral, f::NONE)
            }
            T::Id if t.code == w::GIVEN => {
                let n = self.open(K::GivenPattern);
                self.tk(K::TGiven)?;
                self.typ_x(f::TYPE, Tx { no_arrow: true })?;
                Ok(self.close(n))
            }
            T::Id | T::BackId if !w::is_reserved(t.code) || matches!(t.code, w::THIS | w::SUPER) => {
                if self.kind_at(1) == T::Str && self.adjacent(0) {
                    return self.interpolated_string_expression();
                }
                if self.is_op_at(1, op::AT) {
                    let n = self.open(K::CapturePattern);
                    self.identifier(f::NAME)?;
                    self.tk(K::TAt)?;
                    // Its pattern binds tighter than an infix one (`r @ A -> B` is `(r @ A) -> B`).
                    self.deeper()?;
                    let p = self.simple_pattern()?;
                    self.depth -= 1;
                    self.t.refield(p, f::PATTERN);
                    return Ok(self.close(n));
                }
                self.path_pattern()
            }
            T::Int | T::Float | T::Char | T::Str => self.literal(),
            T::Op if t.code == op::MINUS && matches!(self.kind_at(1), T::Int | T::Float) && self.adjacent(0) => self.literal(),
            T::LParen => {
                if self.kind_at(1) == T::RParen {
                    let n = self.open(K::Unit);
                    self.tk(K::TLParen)?;
                    self.tk(K::TRParen)?;
                    return Ok(self.close(n));
                }
                if self.kind_at(1) == T::Id && self.is_op_at(2, op::EQ) {
                    // A named tuple pattern: refused.
                    return self.refuse();
                }
                let n = self.open(K::TuplePattern);
                self.tk(K::TLParen)?;
                loop {
                    self.pattern_f(f::NONE)?;
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
            _ => self.refuse(),
        }
    }

    /// A path in a pattern: `x` (`identifier`), `a.b` (`stable_identifier`), and either before
    /// `(`: a case class's (`case_class_pattern`, its type `type_identifier` or
    /// `stable_type_identifier`).
    fn path_pattern(&mut self) -> R<u32> {
        let mut k = 0;
        while self.kind_at(k + 1) == T::Dot && matches!(self.kind_at(k + 2), T::Id | T::BackId) {
            k += 2;
        }
        let names = k / 2 + 1;
        let class = matches!(self.kind_at(k + 1), T::LParen | T::LBracket) && self.adjacent(k);
        if !class {
            if names == 1 {
                return self.ident_as(K::Identifier, f::NONE);
            }
            let m = self.mark();
            self.ident_as(K::Identifier, f::NONE)?;
            let mut n = 0;
            for _ in 1..names {
                let s = self.t.wrap(m, K::StableIdentifier, f::NONE);
                self.tk(K::TDot)?;
                self.ident_as(K::Identifier, f::NONE)?;
                n = self.close(s);
            }
            self.t.unmark(m);
            return Ok(n);
        }
        let n = self.open(K::CaseClassPattern);
        // Its type: a type identifier, or `a.b.C`.
        if names == 1 {
            self.ident_as(K::TypeIdentifier, f::TYPE)?;
        } else {
            let m = self.mark();
            self.ident_as(K::Identifier, f::NONE)?;
            for _ in 2..names {
                let s = self.t.wrap(m, K::StableIdentifier, f::NONE);
                self.tk(K::TDot)?;
                self.ident_as(K::Identifier, f::NONE)?;
                self.close(s);
            }
            let s = self.t.wrap(m, K::StableTypeIdentifier, f::TYPE);
            self.tk(K::TDot)?;
            self.ident_as(K::TypeIdentifier, f::NONE)?;
            self.close(s);
            self.t.unmark(m);
        }
        if self.peek() == T::LBracket {
            self.type_arguments(f::TYPE_ARGUMENTS)?;
        }
        self.expect(T::LParen)?;
        if self.kind_at(0) == T::Id && self.is_op_at(1, op::EQ) {
            // Named patterns: refused.
            return self.refuse();
        }
        while self.peek() != T::RParen {
            self.pattern_f(f::PATTERN)?;
            if self.peek() == T::Comma {
                self.tkf(K::TComma, f::PATTERN)?;
            } else {
                break;
            }
        }
        self.expect(T::RParen)?;
        Ok(self.close(n))
    }

    #[inline]
    pub(super) fn is_op_at(&mut self, k: usize, code: u8) -> bool {
        self.op_at(k) == code
    }
}
