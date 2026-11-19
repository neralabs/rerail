//! Types: `_type` and the type lists (type arguments, type parameters, constraints).

use super::{P, ident_word};
use crate::kinds::{K, f};
use crate::lex::{R, T, w};

impl<'a> P<'a> {
    /// `_type`: type modifiers (siblings: `_type` is hidden), then the type.
    pub(super) fn ty(&mut self) -> R<()> {
        if self.type_modifiers_ahead() {
            self.type_modifiers()?;
        }
        self.ty_core(true)
    }

    /// Whether type modifiers come next: an annotation, or `suspend` before a type.
    fn type_modifiers_ahead(&mut self) -> bool {
        match self.peek() {
            T::At => true,
            T::Name if self.is_word(w::SUSPEND) => matches!(self.kind_at(1), T::Name | T::LParen | T::At),
            _ => false,
        }
    }

    fn type_modifiers(&mut self) -> R<()> {
        let n = self.open(K::TypeModifiers);
        while self.type_modifiers_ahead() {
            if self.peek() == T::At {
                self.annotation_node()?;
            } else {
                self.tk(K::TSuspend)?;
            }
        }
        self.close(n);
        Ok(())
    }

    /// The type after its modifiers: a function type (with a receiver where `.(` follows a type),
    /// a parenthesized, nullable, not-null or plain type. `functions`: whether a function type may
    /// be read (not a receiver's).
    fn ty_core(&mut self, functions: bool) -> R<()> {
        let m = self.mark();
        match self.peek() {
            T::LParen => {
                let Some(close) = self.paren_end(0) else { return self.refuse() };
                if functions && self.kind_at(close + 1) == T::Arrow {
                    let n = self.open(K::FunctionType);
                    self.function_type_rest()?;
                    self.close(n);
                    self.t.unmark(m);
                    return Ok(());
                }
                if self.kind_at(close + 1) == T::Amp {
                    self.parenthesized_user_type()?;
                } else {
                    let n = self.open(K::ParenthesizedType);
                    self.tk(K::TLParen)?;
                    self.level(false, |p| p.ty())?;
                    self.expect(T::RParen)?;
                    self.close(n);
                }
            }
            T::Name if self.is_word(w::DYNAMIC) && !matches!(self.kind_at(1), T::Dot | T::Lt) => {
                self.tk(K::TDynamic)?;
            }
            T::Name => self.user_type()?,
            _ => return self.refuse(),
        }
        // Nullable: `?`s (a `?.` after a type is its `?` and a `.`).
        if matches!(self.peek(), T::Question | T::QuestionDot) && self.tok().start == self.t.last_end {
            let n = self.t.wrap(m, K::NullableType, f::NONE);
            loop {
                if self.peek() == T::QuestionDot {
                    self.split_at(0, T::Question, 1);
                }
                if self.peek() != T::Question || self.tok().start != self.t.last_end {
                    break;
                }
                self.leaf(K::Quest, f::NONE)?;
            }
            self.close(n);
        } else if self.peek() == T::Amp {
            let n = self.t.wrap(m, K::NotNullableType, f::NONE);
            self.tk(K::TAmp)?;
            if self.type_modifiers_ahead() {
                self.type_modifiers()?;
            }
            if self.peek() == T::LParen {
                self.parenthesized_user_type()?;
            } else {
                self.user_type()?;
            }
            self.close(n);
        }
        // A receiver: `.(` after the type.
        if functions && self.peek() == T::Dot && self.kind_at(1) == T::LParen && self.i != self.type_stop {
            let r = self.t.wrap(m, K::ReceiverType, f::RECEIVER);
            self.close(r);
            let n = self.t.wrap(m, K::FunctionType, f::NONE);
            self.tk(K::TDot)?;
            self.function_type_rest()?;
            self.close(n);
        }
        self.t.unmark(m);
        Ok(())
    }

    /// A receiver type (`receiver_type`'s content after its modifiers): no function type.
    pub(super) fn receiver_core(&mut self) -> R<()> {
        if self.type_modifiers_ahead() {
            self.type_modifiers()?;
        }
        self.ty_core(false)
    }

    /// The index of the `)` closing the `(` at token `k` ahead.
    pub(super) fn paren_end(&mut self, k: usize) -> Option<usize> {
        let mut d = 0u32;
        let mut j = k;
        loop {
            match self.peek_at(j).kind {
                T::LParen | T::LBracket | T::LBrace => d += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    d -= 1;
                    if d == 0 {
                        return Some(j);
                    }
                }
                T::Eof => return None,
                _ => {}
            }
            j += 1;
        }
    }

    fn parenthesized_user_type(&mut self) -> R<()> {
        let n = self.open(K::ParenthesizedUserType);
        self.tk(K::TLParen)?;
        if self.peek() == T::LParen {
            self.parenthesized_user_type()?;
        } else {
            self.user_type()?;
        }
        self.expect(T::RParen)?;
        self.close(n);
        Ok(())
    }

    /// A function type's parameters, `->` and result.
    fn function_type_rest(&mut self) -> R<()> {
        let n = self.open(K::FunctionTypeParameters);
        self.tk(K::TLParen)?;
        self.level(false, |p| {
            while p.peek() != T::RParen {
                if p.is_ident() && p.kind_at(1) == T::Colon {
                    let q = p.open(K::Parameter);
                    p.simple_identifier()?;
                    p.tk(K::TColon)?;
                    p.ty()?;
                    p.close(q);
                } else {
                    p.ty()?;
                }
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
            }
            p.expect(T::RParen).map(|_| ())
        })?;
        self.close(n);
        self.expect(T::Arrow)?;
        self.ty()
    }

    /// `user_type`: simple user types between `.`s (not past the receiver's `.`, `type_stop`).
    pub(super) fn user_type(&mut self) -> R<()> {
        let n = self.open(K::UserType);
        loop {
            self.type_identifier()?;
            if self.peek() == T::Lt {
                self.type_arguments()?;
            }
            if self.peek() == T::Dot && self.i != self.type_stop && self.is_ident_at(1) {
                self.tk(K::TDot)?;
            } else {
                break;
            }
        }
        self.close(n);
        Ok(())
    }

    /// The `>` closing a type list (a `>=` lexed there is its `>` and a `=`).
    pub(super) fn expect_gt(&mut self) -> R<()> {
        if self.peek() == T::Ge {
            self.split_at(0, T::Gt, 1);
        }
        self.expect(T::Gt).map(|_| ())
    }

    pub(super) fn type_arguments(&mut self) -> R<()> {
        let n = self.open(K::TypeArguments);
        self.tk(K::TLt)?;
        self.level(false, |p| {
            loop {
                let q = p.open(K::TypeProjection);
                if p.peek() == T::Star {
                    p.tk(K::TStar)?;
                } else {
                    if (p.is_word(w::IN) || p.is_word(w::OUT)) && p.variance_ahead(1) {
                        let ms = p.open(K::TypeProjectionModifiers);
                        while (p.is_word(w::IN) || p.is_word(w::OUT)) && p.variance_ahead(1) {
                            p.variance_modifier()?;
                        }
                        p.close(ms);
                    }
                    p.ty()?;
                }
                p.close(q);
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
                if matches!(p.peek(), T::Gt | T::Ge) {
                    break;
                }
            }
            p.expect_gt()
        })?;
        self.close(n);
        Ok(())
    }

    /// Whether the token `k` ahead starts a type (an `in`/`out` before it is a variance).
    fn variance_ahead(&mut self, k: usize) -> bool {
        let t = self.peek_at(k);
        matches!(t.kind, T::Name | T::LParen | T::At)
    }

    fn variance_modifier(&mut self) -> R<()> {
        let n = self.open(K::VarianceModifier);
        if self.is_word(w::IN) {
            self.tk(K::TIn)?;
        } else {
            self.tk(K::TOut)?;
        }
        self.close(n);
        Ok(())
    }

    pub(super) fn type_parameters(&mut self) -> R<()> {
        let n = self.open(K::TypeParameters);
        self.tk(K::TLt)?;
        self.level(false, |p| {
            loop {
                let q = p.open(K::TypeParameter);
                if p.type_parameter_modifiers_ahead() {
                    let ms = p.open(K::TypeParameterModifiers);
                    while p.type_parameter_modifiers_ahead() {
                        if p.peek() == T::At {
                            p.annotation_node()?;
                        } else if p.is_word(w::REIFIED) {
                            let r = p.open(K::ReificationModifier);
                            p.tk(K::TReified)?;
                            p.close(r);
                        } else {
                            p.variance_modifier()?;
                        }
                    }
                    p.close(ms);
                }
                p.type_identifier()?;
                if p.peek() == T::Colon {
                    p.tk(K::TColon)?;
                    p.ty()?;
                }
                p.close(q);
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
                if matches!(p.peek(), T::Gt | T::Ge) {
                    break;
                }
            }
            p.expect_gt()
        })?;
        self.close(n);
        Ok(())
    }

    fn type_parameter_modifiers_ahead(&mut self) -> bool {
        let t = self.tok();
        match t.kind {
            T::At => true,
            T::Name if !t.tick && matches!(t.word, w::REIFIED | w::IN | w::OUT) => {
                let n = self.peek_at(1);
                n.kind == T::At || (n.kind == T::Name && (n.tick || ident_word(n.word) || matches!(n.word, w::IN | w::OUT)))
            }
            _ => false,
        }
    }

    pub(super) fn type_constraints(&mut self) -> R<()> {
        let n = self.open(K::TypeConstraints);
        self.tk(K::TWhere)?;
        loop {
            let c = self.open(K::TypeConstraint);
            while self.peek() == T::At {
                self.annotation_node()?;
            }
            self.type_identifier()?;
            self.expect(T::Colon)?;
            self.ty()?;
            self.close(c);
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::TComma)?;
        }
        self.close(n);
        Ok(())
    }
}
