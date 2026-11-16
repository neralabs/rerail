//! Types: `_type` and its kinds, type arguments and parameters, constraints.
//!
//! A field given a type (`type:`, `return_type:`) is given its modifiers and its `!`; the type
//! itself has `_type`'s own `name:` (a field given inside a hidden rule wins).

use super::P;
use crate::kinds::{K, f};
use crate::lex::{R, T, w};

impl<'a> P<'a> {
    /// `_type` given `field`: its modifiers get `field`, the type `name`.
    pub(super) fn ty_f(&mut self, field: u8) -> R<()> {
        let m = self.mark();
        if self.peek() == T::At {
            self.type_modifiers()?;
        }
        self.unannotated_f(f::NAME)?;
        self.t.field_since(m, field);
        self.t.unmark(m);
        Ok(())
    }

    /// `_possibly_implicitly_unwrapped_type` given `field`: a type and a `!` touching it.
    pub(super) fn possibly_unwrapped_type(&mut self, field: u8) -> R<()> {
        self.ty_f(field)?;
        if self.peek() == T::Bang && self.touching() {
            self.tkf(K::TBang, field)?;
        }
        Ok(())
    }

    /// A parameter's type: its parameter modifiers, then the type (`type:`).
    pub(super) fn parameter_type(&mut self) -> R<()> {
        if self.parameter_modifier_ahead() {
            let n = self.open(K::ParameterModifiers);
            while self.parameter_modifier_ahead() {
                self.parameter_modifier()?;
            }
            self.close(n);
        }
        self.possibly_unwrapped_type(f::TYPE)
    }

    pub(super) fn parameter_modifier_ahead(&mut self) -> bool {
        match self.peek() {
            T::Name => matches!(self.word(), w::INOUT | w::BORROWING | w::CONSUMING | w::SENDING | w::ISOLATED) && self.type_follows(1),
            T::At => self.adjacent(0) && matches!(self.name_text(1), b"escaping" | b"autoclosure"),
            _ => false,
        }
    }

    fn parameter_modifier(&mut self) -> R<()> {
        let n = self.open(K::ParameterModifier);
        if self.peek() == T::At {
            let a = self.tok();
            let b = self.peek_at(1);
            self.bump()?;
            self.bump()?;
            let k = if &self.src[b.start as usize..b.end as usize] == b"escaping" { K::TX40escaping } else { K::TX40autoclosure };
            self.t.token(k, f::NONE, a.start, b.end);
        } else {
            let k = match self.word() {
                w::INOUT => K::TInout,
                w::BORROWING => K::TBorrowing,
                w::CONSUMING => K::TConsuming,
                w::SENDING => K::TSending,
                _ => K::TIsolated,
            };
            self.tk(k)?;
        }
        self.close(n);
        Ok(())
    }

    /// The text of token `k` ahead.
    pub(super) fn name_text(&mut self, k: usize) -> &'a [u8] {
        let t = self.peek_at(k);
        &self.src[t.start as usize..t.end as usize]
    }

    /// Whether a type starts `k` tokens ahead.
    pub(super) fn type_follows(&mut self, k: usize) -> bool {
        matches!(self.kind_at(k), T::Name | T::LParen | T::LBracket | T::At | T::Tilde)
    }

    pub(super) fn type_modifiers(&mut self) -> R<()> {
        let n = self.open(K::TypeModifiers);
        while self.peek() == T::At {
            self.attribute()?;
        }
        self.close(n);
        Ok(())
    }

    /// `_unannotated_type` given `field`, a function type's arrow and result included.
    pub(super) fn unannotated_f(&mut self, field: u8) -> R<()> {
        self.deeper()?;
        let m = self.mark();
        self.type_core()?;
        // A function type: async, throws, `->` after its parameters.
        if self.peek() == T::Arrow || (matches!(self.word(), w::ASYNC | w::THROWS | w::RETHROWS) && self.arrow_after_effects()) {
            self.t.field_since(m, f::PARAMS);
            let n = self.t.wrap(m, K::FunctionType, field);
            if self.is_word(w::ASYNC) {
                self.tk(K::TAsync)?;
            }
            if matches!(self.word(), w::THROWS | w::RETHROWS) {
                self.throws()?;
            }
            self.expect(T::Arrow)?;
            self.ty_f(f::RETURN_TYPE)?;
            self.close(n);
        } else if self.peek() == T::Amp && self.type_follows(1) {
            // `A & B`: a protocol composition.
            let n = self.t.wrap(m, K::ProtocolCompositionType, field);
            while self.peek() == T::Amp {
                self.tk(K::TAmp)?;
                self.type_core()?;
            }
            self.close(n);
        } else {
            self.t.field_since(m, field);
        }
        self.t.unmark(m);
        self.depth -= 1;
        Ok(())
    }

    /// Whether `->` follows the effects (`async`, `throws`) next.
    fn arrow_after_effects(&mut self) -> bool {
        let mut k = 0;
        loop {
            match self.word_at(k) {
                w::ASYNC | w::THROWS | w::RETHROWS => k += 1,
                _ => break,
            }
            if self.kind_at(k) == T::LParen {
                // `throws(E)`.
                k += 3;
            }
        }
        self.kind_at(k) == T::Arrow
    }

    /// One type: user, tuple, array, dictionary, `some`, `any`, …, then `?`s and `.Type`.
    fn type_core(&mut self) -> R<()> {
        let m = self.mark();
        let t = self.tok();
        match t.kind {
            T::Name if !t.tick && matches!(t.word, w::SOME | w::ANY | w::EACH | w::REPEAT) && self.type_follows(1) && !self.peek_at(1).nl => {
                let kind = match t.word {
                    w::SOME => K::OpaqueType,
                    w::ANY => K::ExistentialType,
                    w::EACH => K::TypeParameterPack,
                    _ => K::TypePackExpansion,
                };
                let n = self.open(kind);
                self.tk(match t.word {
                    w::SOME => K::TSome,
                    w::ANY => K::TAny,
                    w::EACH => K::TEach,
                    _ => K::TRepeat,
                })?;
                self.unannotated_f(f::NONE)?;
                self.close(n);
                self.t.unmark(m);
                return Ok(());
            }
            T::Name => self.user_type()?,
            T::LParen => self.tuple_type()?,
            T::LBracket => {
                let b = self.mark();
                self.tk(K::TLBracket)?;
                let dict = self.level(false, |p| {
                    let e = p.mark();
                    p.ty_f(f::NONE)?;
                    let dict = p.peek() == T::Colon;
                    p.t.field_since(e, if dict { f::KEY } else { f::ELEMENT });
                    p.t.unmark(e);
                    if dict {
                        p.tk(K::TColon)?;
                        p.ty_f(f::VALUE)?;
                    }
                    Ok(dict)
                })?;
                self.expect(T::RBracket)?;
                let n = self.t.wrap(b, if dict { K::DictionaryType } else { K::ArrayType }, f::NONE);
                self.close(n);
                self.t.unmark(b);
            }
            T::Tilde => {
                let n = self.open(K::SuppressedConstraint);
                self.tk(K::TTilde)?;
                self.ident_as(K::TypeIdentifier, f::SUPPRESSED)?;
                self.close(n);
                self.t.unmark(m);
                return Ok(());
            }
            _ => return self.refuse(),
        }
        // `?`s touching: an optional type.
        if matches!(self.peek(), T::Question | T::QQ | T::Custom) && self.touching() && matches!(self.src[self.tok().start as usize], b'?') {
            self.t.field_since(m, f::WRAPPED);
            let n = self.t.wrap(m, K::OptionalType, f::NONE);
            while self.touching() && self.src.get(self.tok().start as usize) == Some(&b'?') {
                match self.peek() {
                    T::Question => {}
                    T::QQ => {
                        // `??`: two `?`s (the scanner's `_double_optional` is not read here).
                        return self.refuse();
                    }
                    _ => self.split_at(0, T::Question, 1),
                }
                self.wrap_tok(K::TQuestion, f::NONE, K::TQuestion)?;
            }
            self.close(n);
        }
        // `.Type` after a type other than a user type's names: a metatype, never seen.
        if self.peek() == T::Dot && self.kind_at(1) == T::Name {
            return self.refuse();
        }
        self.t.unmark(m);
        Ok(())
    }

    /// `user_type`: simple user types between `.`s (a `.Type` after them a metatype's).
    pub(super) fn user_type(&mut self) -> R<()> {
        self.user_type_f(f::NONE)
    }

    pub(super) fn user_type_f(&mut self, field: u8) -> R<()> {
        let n = self.open_field(K::UserType, field);
        loop {
            self.ident_as(K::TypeIdentifier, f::NONE)?;
            if self.peek() == T::Lt && self.touching() {
                self.type_arguments()?;
            }
            if self.peek() == T::Dot && self.kind_at(1) == T::Name {
                self.tk(K::TDot)?;
            } else {
                break;
            }
        }
        self.close(n);
        Ok(())
    }

    /// `tuple_type`: `(` items `)`.
    fn tuple_type(&mut self) -> R<()> {
        let n = self.open(K::TupleType);
        self.tk(K::TLParen)?;
        let items = self.level(false, |p| {
            let mut items = (0u32, 0u32);
            while p.peek() != T::RParen {
                let named = p.peek() == T::Name && p.kind_at(1) == T::Colon;
                let q = p.open_field(K::TupleTypeItem, f::ELEMENT);
                // A name: `name:`, or `_ name:`.
                if p.peek() == T::Name && p.kind_at(1) == T::Colon {
                    p.simple_identifier_f(f::NAME)?;
                    p.tk(K::TColon)?;
                } else if p.is_word(w::UNDERSCORE) && p.kind_at(1) == T::Name && p.kind_at(2) == T::Colon {
                    p.leaf(K::WildcardPattern, f::NONE)?;
                    p.simple_identifier_f(f::NAME)?;
                    p.tk(K::TColon)?;
                }
                if p.parameter_modifier_ahead() {
                    let m = p.open(K::ParameterModifiers);
                    while p.parameter_modifier_ahead() {
                        p.parameter_modifier()?;
                    }
                    p.close(m);
                }
                p.ty_f(f::TYPE)?;
                let id = p.close(q);
                items = (items.0 + 1, if named { 0 } else { id });
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
            }
            Ok(items)
        })?;
        // `(dictionary)`, `(any P)`: `_parenthesized_type`, its item no field and its type
        // `element`.
        if items.0 == 1 && items.1 != 0 && matches!(self.t.kind_at(items.1 + 1), K::DictionaryType | K::ExistentialType) {
            self.t.refield(items.1, f::NONE);
            self.t.refield(items.1 + 1, f::ELEMENT);
        }
        self.expect(T::RParen)?;
        self.close(n);
        Ok(())
    }

    /// The `>` closing a type list: a token starting with `>` split there.
    fn expect_gt(&mut self) -> R<()> {
        let t = self.tok();
        if t.kind != T::Gt && self.src.get(t.start as usize) == Some(&b'>') && t.end > t.start + 1 {
            self.split_at(0, T::Gt, 1);
        }
        self.expect(T::Gt).map(|_| ())
    }

    pub(super) fn type_arguments(&mut self) -> R<()> {
        let n = self.open(K::TypeArguments);
        self.tk(K::TLt)?;
        self.level(false, |p| {
            loop {
                if p.peek() == T::Int {
                    p.leaf(K::IntegerLiteral, f::NONE)?;
                } else {
                    p.ty_f(f::NONE)?;
                }
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
            }
            p.expect_gt()
        })?;
        self.close(n);
        Ok(())
    }

    pub(super) fn type_parameters(&mut self) -> R<()> {
        let n = self.open(K::TypeParameters);
        self.tk(K::TLt)?;
        self.level(false, |p| {
            loop {
                let q = p.open(K::TypeParameter);
                if p.peek() == T::At {
                    let m = p.open(K::TypeParameterModifiers);
                    while p.peek() == T::At {
                        p.attribute()?;
                    }
                    p.close(m);
                }
                if p.is_word(w::EACH) && p.kind_at(1) == T::Name {
                    let e = p.open(K::TypeParameterPack);
                    p.tk(K::TEach)?;
                    p.user_type()?;
                    p.close(e);
                } else {
                    p.ident_as(K::TypeIdentifier, f::NONE)?;
                }
                if p.peek() == T::Colon {
                    p.tk(K::TColon)?;
                    p.ty_f(f::NONE)?;
                }
                p.close(q);
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
            }
            if p.is_word(w::WHERE) {
                p.type_constraints()?;
            }
            p.expect_gt()
        })?;
        self.close(n);
        Ok(())
    }

    /// `type_constraints`: `where` and constraints.
    pub(super) fn type_constraints(&mut self) -> R<()> {
        let n = self.open(K::TypeConstraints);
        self.leaf(K::WhereKeyword, f::NONE)?;
        loop {
            let c = self.open(K::TypeConstraint);
            // The constrained type: names between dots (`identifier`), else a type.
            let mut k = 0;
            while self.kind_at(k) == T::Name && self.kind_at(k + 1) == T::Dot {
                k += 2;
            }
            let ident = self.kind_at(k) == T::Name && matches!(self.kind_at(k + 1), T::Colon | T::EqEq | T::Eq) && self.kind_at(0) != T::At;
            let m = self.mark();
            while self.peek() == T::At {
                self.attribute()?;
            }
            if ident {
                self.identifier(f::CONSTRAINED_TYPE)?;
            } else {
                self.unannotated_f(f::CONSTRAINED_TYPE)?;
            }
            match self.peek() {
                T::Colon => {
                    let x = self.t.wrap(m, K::InheritanceConstraint, f::NONE);
                    self.tk(K::TColon)?;
                    self.possibly_unwrapped_type(f::INHERITS_FROM)?;
                    self.close(x);
                }
                T::EqEq | T::Eq => {
                    let x = self.t.wrap(m, K::EqualityConstraint, f::NONE);
                    let k = if self.peek() == T::EqEq { K::TEqEq } else { K::TEq };
                    self.tk(k)?;
                    self.ty_f(f::MUST_EQUAL)?;
                    self.close(x);
                }
                _ => return self.refuse(),
            }
            self.t.unmark(m);
            self.close(c);
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::TComma)?;
        }
        self.close(n);
        Ok(())
    }

    /// `throws` (or `rethrows`), or `throws_clause` (`throws(E)`).
    pub(super) fn throws(&mut self) -> R<()> {
        if self.is_word(w::THROWS) && self.kind_at(1) == T::LParen && self.adjacent(0) {
            let n = self.open(K::ThrowsClause);
            let t = self.bump()?;
            let _ = t;
            self.tk(K::TLParen)?;
            self.unannotated_f(f::TYPE)?;
            self.expect(T::RParen)?;
            self.close(n);
            return Ok(());
        }
        self.leaf(K::Throws, f::NONE)?;
        Ok(())
    }

    /// `type_annotation`: `:` and a type (`type:`).
    pub(super) fn type_annotation(&mut self) -> R<()> {
        let n = self.open(K::TypeAnnotation);
        self.expect(T::Colon)?;
        self.possibly_unwrapped_type(f::TYPE)?;
        self.close(n);
        Ok(())
    }
}
