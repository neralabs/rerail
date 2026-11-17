//! Declarations: classes, objects, functions, properties, their members and modifiers, and
//! annotations.

use super::{P, soft};
use crate::kinds::{K, f};
use crate::lex::{Asi, R, T, w};

/// A modifier word's node kind (`class_modifier`, `visibility_modifier`, …).
fn modifier(code: u8) -> Option<K> {
    Some(match code {
        w::SEALED | w::ANNOTATION | w::DATA | w::INNER | w::VALUE => K::ClassModifier,
        w::OVERRIDE | w::LATEINIT => K::MemberModifier,
        w::PUBLIC | w::PRIVATE | w::INTERNAL | w::PROTECTED => K::VisibilityModifier,
        w::TAILREC | w::OPERATOR | w::INFIX | w::INLINE | w::EXTERNAL | w::SUSPEND => K::FunctionModifier,
        w::CONST => K::PropertyModifier,
        w::ABSTRACT | w::FINAL | w::OPEN => K::InheritanceModifier,
        w::VARARG | w::NOINLINE | w::CROSSINLINE => K::ParameterModifier,
        w::EXPECT | w::ACTUAL => K::PlatformModifier,
        _ => return None,
    })
}

/// A declaration's kind, by the word that starts it after its modifiers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Decl {
    Class,
    Object,
    Function,
    Property,
    TypeAlias,
    Getter,
    Setter,
    Companion,
    Init,
    Constructor,
}

impl<'a> P<'a> {
    /// The token index after the annotation at `k` ahead, where one reads there.
    pub(super) fn skip_annotation(&mut self, k: usize) -> Option<usize> {
        let mut j = k + 1;
        let t = self.peek_at(j);
        if t.kind == T::Name && !t.tick && self.kind_at(j + 1) == T::Colon && matches!(t.word, w::FIELD | w::PROPERTY | w::GET | w::SET | w::RECEIVER | w::PARAM | w::SETPARAM | w::DELEGATE | w::FILE) {
            j += 2;
        }
        if self.kind_at(j) == T::LBracket {
            let mut d = 0u32;
            loop {
                match self.kind_at(j) {
                    T::LBracket | T::LParen => d += 1,
                    T::RBracket | T::RParen => {
                        d -= 1;
                        if d == 0 {
                            return Some(j + 1);
                        }
                    }
                    T::Eof => return None,
                    _ => {}
                }
                j += 1;
            }
        }
        // A user type, and its arguments where they follow on the line.
        loop {
            if !self.is_ident_at(j) {
                return None;
            }
            j += 1;
            if self.kind_at(j) == T::Lt {
                j = self.type_args_end(j)?;
            }
            if self.kind_at(j) == T::Dot && self.is_ident_at(j + 1) {
                j += 1;
            } else {
                break;
            }
        }
        if self.kind_at(j) == T::LParen && !self.peek_at(j).nl {
            j = self.paren_end(j)? + 1;
        }
        Some(j)
    }

    /// The declaration that starts here (after annotations and modifiers), where one does: its
    /// kind, and the index of its keyword.
    pub(super) fn declaration_ahead(&mut self, member: bool) -> Option<(Decl, usize)> {
        let mut k = 0;
        loop {
            let t = self.peek_at(k);
            if t.kind == T::At {
                k = self.skip_annotation(k)?;
                continue;
            }
            if t.kind != T::Name || t.tick {
                return None;
            }
            let next = self.peek_at(k + 1);
            let d = match t.word {
                w::CLASS | w::INTERFACE => Decl::Class,
                w::ENUM if self.is_word_at(k + 1, w::CLASS) => Decl::Class,
                w::FUN if self.is_word_at(k + 1, w::INTERFACE) => Decl::Class,
                w::FUN if next.kind != T::LParen => Decl::Function,
                w::VAL | w::VAR => Decl::Property,
                w::TYPEALIAS => Decl::TypeAlias,
                w::OBJECT if self.is_ident_at(k + 1) => Decl::Object,
                w::COMPANION if self.is_word_at(k + 1, w::OBJECT) => Decl::Companion,
                w::INIT if member && next.kind == T::LBrace => Decl::Init,
                w::CONSTRUCTOR if member && next.kind == T::LParen => Decl::Constructor,
                w::GET | w::SET => {
                    if self.accessor_ahead(k) {
                        if t.word == w::GET { Decl::Getter } else { Decl::Setter }
                    } else {
                        return None;
                    }
                }
                code => {
                    // A modifier, where a declaration follows.
                    modifier(code)?;
                    k += 1;
                    continue;
                }
            };
            return Some((d, k));
        }
    }

    /// Whether `get`/`set` at `k` ahead is an accessor: with its parameters and a body, or alone
    /// where the statement ends.
    fn accessor_ahead(&mut self, k: usize) -> bool {
        let next = self.peek_at(k + 1);
        if next.kind == T::LParen {
            // A getter's `()`, a setter's one parameter (annotations, modifiers, a name, a type).
            let Some(e) = self.paren_end(k + 1) else { return false };
            let params = if self.is_word_at(k, w::GET) {
                e == k + 2
            } else {
                let mut j = k + 2;
                while matches!(self.peek_at(j).kind, T::At | T::Name) && self.kind_at(j + 1) != T::RParen && self.kind_at(j + 1) != T::Colon {
                    match self.peek_at(j).kind {
                        T::At => match self.skip_annotation(j) {
                            Some(x) => j = x,
                            None => return false,
                        },
                        _ => j += 1,
                    }
                }
                self.is_ident_at(j) && (self.kind_at(j + 1) == T::RParen || self.kind_at(j + 1) == T::Colon)
            };
            return params && matches!(self.kind_at(e + 1), T::Eq | T::LBrace | T::Colon);
        }
        next.nl || matches!(next.kind, T::Semi | T::RBrace | T::Eof)
    }

    /// A declaration: `member` in a class body.
    pub(super) fn declaration(&mut self, member: bool) -> R<()> {
        let Some((d, k)) = self.declaration_ahead(member) else { return self.refuse() };
        let kind = match d {
            Decl::Class => K::ClassDeclaration,
            Decl::Object => K::ObjectDeclaration,
            Decl::Function => K::FunctionDeclaration,
            Decl::Property => K::PropertyDeclaration,
            Decl::TypeAlias => K::TypeAlias,
            Decl::Getter => K::Getter,
            Decl::Setter => K::Setter,
            Decl::Companion => K::CompanionObject,
            Decl::Init => K::AnonymousInitializer,
            Decl::Constructor => K::SecondaryConstructor,
        };
        let n = self.open(kind);
        if k > 0 {
            self.modifiers(K::Modifiers, k)?;
        }
        match d {
            Decl::Class => self.class_rest()?,
            Decl::Object => {
                self.tk(K::TObject)?;
                self.type_identifier()?;
                self.supertypes_and_body()?;
            }
            Decl::Companion => {
                self.tk(K::TCompanion)?;
                self.tk(K::TObject)?;
                if self.is_ident() && !self.ended() {
                    self.type_identifier()?;
                }
                self.supertypes_and_body()?;
            }
            Decl::Function => self.function_rest()?,
            Decl::Property => self.property_rest()?,
            Decl::TypeAlias => {
                self.tk(K::TTypealias)?;
                self.type_identifier()?;
                if self.peek() == T::Lt {
                    self.type_parameters()?;
                }
                self.expect(T::Eq)?;
                self.ty()?;
            }
            Decl::Getter | Decl::Setter => self.accessor_rest(d == Decl::Getter)?,
            Decl::Init => {
                self.tk(K::TInit)?;
                self.block()?;
            }
            Decl::Constructor => {
                self.tk(K::TConstructor)?;
                self.function_value_parameters()?;
                if self.peek() == T::Colon {
                    self.tk(K::TColon)?;
                    let c = self.open(K::ConstructorDelegationCall);
                    if self.is_word(w::THIS) {
                        self.tk(K::TThis)?;
                    } else {
                        self.kw(w::SUPER, K::TSuper)?;
                    }
                    self.value_arguments()?;
                    self.close(c);
                }
                if self.peek() == T::LBrace && !self.ended() {
                    self.block()?;
                }
            }
        }
        self.close(n);
        Ok(())
    }

    /// Modifiers (`kind`: `modifiers`, `parameter_modifiers`): annotations and modifier words, up
    /// to token `k` ahead.
    fn modifiers(&mut self, kind: K, k: usize) -> R<()> {
        let end = self.i + k;
        let n = self.open(kind);
        while self.i < end {
            if self.peek() == T::At {
                self.annotation_node()?;
                continue;
            }
            let t = self.tok();
            let (Some(node), Some(tok)) = (modifier(t.word), soft(t.word)) else { return self.refuse() };
            let m = self.open(node);
            self.tk(tok)?;
            self.close(m);
        }
        if self.i != end {
            return self.refuse();
        }
        self.close(n);
        Ok(())
    }

    /// The modifiers ahead of a parameter (annotations, modifier words before a name): their count.
    fn parameter_modifiers_ahead(&mut self, params: bool) -> usize {
        let mut k = 0;
        loop {
            let t = self.peek_at(k);
            if t.kind == T::At {
                match self.skip_annotation(k) {
                    Some(e) => k = e,
                    None => return 0,
                }
                continue;
            }
            if t.kind == T::Name && !t.tick && modifier(t.word).is_some() && (!params || matches!(t.word, w::VARARG | w::NOINLINE | w::CROSSINLINE)) {
                // A modifier where a name, `val`/`var` or another modifier follows it.
                let n = self.peek_at(k + 1);
                if n.kind == T::Name || n.kind == T::At {
                    k += 1;
                    continue;
                }
            }
            return k;
        }
    }

    // ---- Annotations.

    /// `annotation`.
    pub(super) fn annotation_node(&mut self) -> R<()> {
        let n = self.open(K::Annotation);
        self.tk(K::TAt)?;
        let t = self.tok();
        if t.kind == T::Name && !t.tick && self.kind_at(1) == T::Colon {
            let k = match t.word {
                w::FIELD => Some(K::TField),
                w::PROPERTY => Some(K::TProperty),
                w::GET => Some(K::TGet),
                w::SET => Some(K::TSet),
                w::RECEIVER => Some(K::TReceiver),
                w::PARAM => Some(K::TParam),
                w::SETPARAM => Some(K::TSetparam),
                w::DELEGATE => Some(K::TDelegate),
                _ => None,
            };
            if let Some(k) = k {
                let u = self.open(K::UseSiteTarget);
                self.tk(k)?;
                self.tk(K::TColon)?;
                self.close(u);
            }
        }
        if self.peek() == T::LBracket {
            self.tk(K::TLBracket)?;
            while self.peek() != T::RBracket {
                self.unescaped_annotation()?;
            }
            self.tk(K::TRBracket)?;
        } else {
            self.unescaped_annotation()?;
        }
        self.close(n);
        Ok(())
    }

    /// `_unescaped_annotation`: a user type, a `constructor_invocation` where arguments follow.
    pub(super) fn unescaped_annotation(&mut self) -> R<()> {
        let m = self.mark();
        self.user_type()?;
        let t = self.tok();
        if t.kind == T::LParen && !t.nl {
            let n = self.t.wrap(m, K::ConstructorInvocation, f::NONE);
            self.value_arguments()?;
            self.close(n);
        }
        self.t.unmark(m);
        Ok(())
    }

    // ---- Classes.

    fn class_rest(&mut self) -> R<()> {
        let enumeration = self.is_word(w::ENUM);
        if enumeration {
            self.tk(K::TEnum)?;
            self.tk(K::TClass)?;
        } else if self.is_word(w::FUN) {
            self.tk(K::TFun)?;
            self.tk(K::TInterface)?;
        } else if self.is_word(w::CLASS) {
            self.tk(K::TClass)?;
        } else {
            self.tk(K::TInterface)?;
        }
        self.type_identifier()?;
        if self.peek() == T::Lt && !self.ended() {
            self.type_parameters()?;
        }
        // The primary constructor: the scanner reads `constructor` on the next line here.
        self.cx.ctor = true;
        let ctor = match self.tok() {
            t if t.nl && self.stmt => match self.asi() {
                Asi::No => true,
                Asi::Ctor => true,
                _ => false,
            },
            _ => true,
        };
        self.cx.ctor = false;
        if ctor && self.primary_constructor_ahead() {
            self.primary_constructor()?;
        }
        if self.peek() == T::Colon && !self.ended() {
            self.tk(K::TColon)?;
            self.delegation_specifiers()?;
        }
        if self.is_word(w::WHERE) && !self.ended() {
            self.type_constraints()?;
        }
        if self.peek() == T::LBrace && !self.ended() {
            if enumeration {
                self.enum_class_body()?;
            } else {
                self.class_body()?;
            }
        }
        Ok(())
    }

    /// Whether a primary constructor follows: `(`, or modifiers and `constructor`.
    fn primary_constructor_ahead(&mut self) -> bool {
        if self.peek() == T::LParen || self.is_word(w::CONSTRUCTOR) {
            return true;
        }
        let k = self.parameter_modifiers_ahead(false);
        k > 0 && self.is_word_at(k, w::CONSTRUCTOR)
    }

    fn primary_constructor(&mut self) -> R<()> {
        let n = self.open(K::PrimaryConstructor);
        if self.peek() != T::LParen {
            let k = self.parameter_modifiers_ahead(false);
            if k > 0 {
                self.modifiers(K::Modifiers, k)?;
            }
            // `_primary_constructor_keyword`: hidden.
            self.bump()?;
        }
        self.tk(K::TLParen)?;
        self.level(false, |p| {
            while p.peek() != T::RParen {
                let c = p.open(K::ClassParameter);
                let k = p.parameter_modifiers_ahead(false);
                if k > 0 {
                    p.modifiers(K::Modifiers, k)?;
                }
                if p.is_word(w::VAL) || p.is_word(w::VAR) {
                    p.binding_pattern_kind()?;
                }
                p.simple_identifier()?;
                p.expect(T::Colon)?;
                p.ty()?;
                if p.peek() == T::Eq {
                    p.tk(K::TEq)?;
                    p.expression()?;
                }
                p.close(c);
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

    fn binding_pattern_kind(&mut self) -> R<()> {
        let n = self.open(K::BindingPatternKind);
        if self.is_word(w::VAL) {
            self.tk(K::TVal)?;
        } else {
            self.kw(w::VAR, K::TVar)?;
        }
        self.close(n);
        Ok(())
    }

    /// `(":" _delegation_specifiers)? class_body?` (an object's, a companion object's).
    fn supertypes_and_body(&mut self) -> R<()> {
        if self.peek() == T::Colon && !self.ended() {
            self.tk(K::TColon)?;
            self.delegation_specifiers()?;
        }
        if self.peek() == T::LBrace && !self.ended() {
            self.class_body()?;
        }
        Ok(())
    }

    fn delegation_specifiers(&mut self) -> R<()> {
        loop {
            let n = self.open(K::DelegationSpecifier);
            let m = self.mark();
            if self.is_word(w::SUSPEND) && matches!(self.kind_at(1), T::LParen | T::Name) {
                // `"suspend" function_type`: the keyword is the specifier's.
                self.tk(K::TSuspend)?;
                self.ty()?;
            } else if self.peek() == T::LParen {
                self.ty()?;
            } else {
                self.user_type()?;
                if self.peek() == T::Dot && self.kind_at(1) == T::LParen {
                    // A function type with this receiver.
                    let r = self.t.wrap(m, K::ReceiverType, f::RECEIVER);
                    self.close(r);
                    let ft = self.t.wrap(m, K::FunctionType, f::NONE);
                    self.tk(K::TDot)?;
                    self.function_type_params_rest()?;
                    self.close(ft);
                } else if self.peek() == T::LParen && !self.tok().nl {
                    let c = self.t.wrap(m, K::ConstructorInvocation, f::NONE);
                    self.value_arguments()?;
                    self.close(c);
                }
            }
            // `by`: the scanner reads it on the next line here.
            self.cx.by = true;
            let by = self.is_word(w::BY) && !self.ended();
            self.cx.by = false;
            if by {
                let e = self.t.wrap(m, K::ExplicitDelegation, f::NONE);
                self.tk(K::TBy)?;
                let saved = std::mem::replace(&mut self.no_lambda, true);
                let r = self.expression();
                self.no_lambda = saved;
                r?;
                self.close(e);
            }
            self.t.unmark(m);
            self.close(n);
            if self.peek() != T::Comma {
                return Ok(());
            }
            self.tk(K::TComma)?;
        }
    }

    /// A function type's parameters and result, after its receiver and `.`.
    fn function_type_params_rest(&mut self) -> R<()> {
        // `ty` reads a whole function type; here the receiver is read: its parameters, as a type
        // in parentheses would read, then `->` and the result.
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

    /// `class_body`: members, each ending with `_semi`.
    pub(super) fn class_body(&mut self) -> R<()> {
        let n = self.open(K::ClassBody);
        self.tk(K::TLBrace)?;
        self.level(true, |p| {
            p.members()?;
            p.expect(T::RBrace).map(|_| ())
        })?;
        self.close(n);
        Ok(())
    }

    /// `_class_member_declarations`, to the `}`.
    fn members(&mut self) -> R<()> {
        while self.peek() != T::RBrace {
            self.declaration(true)?;
            if !self.semi()? {
                return self.refuse();
            }
        }
        Ok(())
    }

    fn enum_class_body(&mut self) -> R<()> {
        let n = self.open(K::EnumClassBody);
        self.tk(K::TLBrace)?;
        self.level(false, |p| {
            while !matches!(p.peek(), T::Semi | T::RBrace) {
                let e = p.open(K::EnumEntry);
                let k = p.parameter_modifiers_ahead(false);
                if k > 0 {
                    p.modifiers(K::Modifiers, k)?;
                }
                p.simple_identifier()?;
                if p.peek() == T::LParen {
                    p.value_arguments()?;
                }
                if p.peek() == T::LBrace {
                    p.class_body()?;
                }
                p.close(e);
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
            }
            Ok(())
        })?;
        if self.peek() == T::Semi {
            self.tk(K::TSemi)?;
            self.level(true, |p| p.members())?;
        }
        self.expect(T::RBrace)?;
        self.close(n);
        Ok(())
    }

    // ---- Functions and properties.

    /// The receiver's end ahead (a `.` or `?.` before the declared name): its index, where there is
    /// one. `name_follows`: the tokens that may follow the declared name.
    fn receiver_end(&mut self, function: bool) -> Option<usize> {
        let mut k = 0;
        let mut last = None;
        let mut depth = 0u32;
        loop {
            let t = self.peek_at(k);
            match t.kind {
                T::Lt | T::LParen => depth += 1,
                T::Gt | T::RParen => {
                    if depth == 0 {
                        return None;
                    }
                    depth -= 1;
                }
                T::Name | T::Comma | T::Question | T::Star | T::Arrow | T::Colon | T::At if depth > 0 => {}
                T::Dot | T::QuestionDot if depth == 0 => last = Some(k),
                T::Name | T::Question if depth == 0 => {
                    let n = self.kind_at(k + 1);
                    let ends = if function { n == T::LParen } else { !matches!(n, T::Dot | T::QuestionDot | T::Lt | T::Question) };
                    if t.kind == T::Name && ends {
                        return last.filter(|&l| l + 1 == k);
                    }
                }
                _ => return None,
            }
            k += 1;
            if k > 256 {
                return None;
            }
        }
    }

    /// The receiver before a declared name, its `.` read.
    fn receiver(&mut self, end: usize) -> R<()> {
        let dot = self.peek_at(end);
        let stop = self.i + end + usize::from(dot.kind == T::QuestionDot);
        let saved = std::mem::replace(&mut self.type_stop, stop);
        let n = self.open_field(K::ReceiverType, f::RECEIVER);
        let r = self.receiver_core();
        self.type_stop = saved;
        r?;
        self.close(n);
        self.expect(T::Dot)?;
        Ok(())
    }

    fn function_rest(&mut self) -> R<()> {
        self.tk(K::TFun)?;
        if self.peek() == T::Lt {
            self.type_parameters()?;
        }
        if !(self.is_ident() && self.kind_at(1) == T::LParen) {
            let Some(end) = self.receiver_end(true) else { return self.refuse() };
            self.receiver(end)?;
        }
        self.simple_identifier()?;
        self.function_value_parameters()?;
        if self.peek() == T::Colon && !self.ended() {
            self.tk(K::TColon)?;
            self.ty()?;
        }
        if self.is_word(w::WHERE) && !self.ended() {
            self.type_constraints()?;
        }
        if matches!(self.peek(), T::LBrace | T::Eq) && !self.ended() {
            self.function_body()?;
        }
        Ok(())
    }

    pub(super) fn function_value_parameters(&mut self) -> R<()> {
        let n = self.open(K::FunctionValueParameters);
        self.tk(K::TLParen)?;
        self.level(false, |p| {
            while p.peek() != T::RParen {
                let k = p.parameter_modifiers_ahead(true);
                if k > 0 {
                    p.modifiers(K::ParameterModifiers, k)?;
                }
                let q = p.open(K::Parameter);
                p.simple_identifier()?;
                p.expect(T::Colon)?;
                p.ty()?;
                p.close(q);
                if p.peek() == T::Eq {
                    p.tk(K::TEq)?;
                    p.expression()?;
                }
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

    pub(super) fn function_body(&mut self) -> R<()> {
        let n = self.open(K::FunctionBody);
        if self.peek() == T::LBrace {
            self.block()?;
        } else {
            self.tk(K::TEq)?;
            self.expression()?;
        }
        self.close(n);
        Ok(())
    }

    fn property_rest(&mut self) -> R<()> {
        self.binding_pattern_kind()?;
        if self.peek() == T::Lt {
            self.type_parameters()?;
        }
        if !matches!(self.peek(), T::LParen | T::LBracket)
            && let Some(end) = self.receiver_end(false) {
                self.receiver(end)?;
            }
        if matches!(self.peek(), T::LParen | T::LBracket) {
            self.multi_variable_declaration()?;
        } else {
            self.variable_declaration()?;
        }
        if self.is_word(w::WHERE) && !self.ended() {
            self.type_constraints()?;
        }
        self.cx.by = true;
        let by = self.is_word(w::BY) && !self.ended();
        self.cx.by = false;
        if self.peek() == T::Eq && !self.ended() {
            self.tk(K::TEq)?;
            self.expression()?;
        } else if by {
            let n = self.open(K::PropertyDelegate);
            self.tk(K::TBy)?;
            self.expression()?;
            self.close(n);
        }
        // Accessors on the same line.
        if !self.ended()
            && let Some((d @ (Decl::Getter | Decl::Setter), k)) = self.declaration_ahead(true) {
                let first = d;
                let n = self.open(if d == Decl::Getter { K::Getter } else { K::Setter });
                if k > 0 {
                    self.modifiers(K::Modifiers, k)?;
                }
                self.accessor_rest(d == Decl::Getter)?;
                self.close(n);
                if !self.ended()
                    && let Some((d2 @ (Decl::Getter | Decl::Setter), k)) = self.declaration_ahead(true)
                        && d2 != first {
                            let n = self.open(if d2 == Decl::Getter { K::Getter } else { K::Setter });
                            if k > 0 {
                                self.modifiers(K::Modifiers, k)?;
                            }
                            self.accessor_rest(d2 == Decl::Getter)?;
                            self.close(n);
                        }
            }
        Ok(())
    }

    fn accessor_rest(&mut self, getter: bool) -> R<()> {
        if getter {
            self.tk(K::TGet)?;
        } else {
            self.tk(K::TSet)?;
        }
        if self.peek() == T::LParen {
            self.tk(K::TLParen)?;
            if !getter {
                self.level(false, |p| {
                    let q = p.open(K::ParameterWithOptionalType);
                    let k = p.parameter_modifiers_ahead(true);
                    if k > 0 {
                        p.modifiers(K::ParameterModifiers, k)?;
                    }
                    p.simple_identifier()?;
                    if p.peek() == T::Colon {
                        p.tk(K::TColon)?;
                        p.ty()?;
                    }
                    p.close(q);
                    Ok(())
                })?;
            }
            self.expect(T::RParen)?;
            if self.peek() == T::Colon {
                self.tk(K::TColon)?;
                self.ty()?;
            }
            self.function_body()?;
        }
        Ok(())
    }

    /// `variable_declaration`: a name and its type.
    pub(super) fn variable_declaration(&mut self) -> R<()> {
        let n = self.open(K::VariableDeclaration);
        self.simple_identifier()?;
        if self.peek() == T::Colon {
            self.tk(K::TColon)?;
            self.ty()?;
        }
        self.close(n);
        Ok(())
    }

    pub(super) fn multi_variable_declaration(&mut self) -> R<()> {
        let n = self.open(K::MultiVariableDeclaration);
        let square = self.peek() == T::LBracket;
        if square {
            self.tk(K::TLBracket)?;
        } else {
            self.expect(T::LParen)?;
        }
        self.level(false, |p| {
            loop {
                let v = p.open(K::VariableDeclaration);
                if p.is_word(w::VAL) || p.is_word(w::VAR) {
                    p.binding_pattern_kind()?;
                }
                p.simple_identifier()?;
                if p.peek() == T::Colon {
                    p.tk(K::TColon)?;
                    p.ty()?;
                }
                if !square && p.peek() == T::Eq {
                    p.tk(K::TEq)?;
                    p.simple_identifier()?;
                }
                p.close(v);
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
                if matches!(p.peek(), T::RParen | T::RBracket) {
                    break;
                }
            }
            Ok(())
        })?;
        if square {
            self.expect(T::RBracket)?;
        } else {
            self.expect(T::RParen)?;
        }
        self.close(n);
        Ok(())
    }

    /// `object_literal`.
    pub(super) fn object_literal(&mut self) -> R<()> {
        let n = self.open(K::ObjectLiteral);
        if self.is_word(w::DATA) {
            self.tk(K::TData)?;
        }
        self.kw(w::OBJECT, K::TObject)?;
        if self.peek() == T::Colon {
            self.tk(K::TColon)?;
            let saved = std::mem::replace(&mut self.no_lambda, true);
            let r = self.delegation_specifiers();
            self.no_lambda = saved;
            r?;
        }
        if self.peek() == T::LBrace && !self.ended() {
            self.class_body()?;
        }
        self.close(n);
        Ok(())
    }
}

