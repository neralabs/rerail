//! Declarations: functions, classes, interfaces, traits, enums, their members and parameters, and
//! attributes.

use super::*;

impl<'a> P<'a> {
    /// `#[…]` groups (an `attribute_list`) in slot `s`.
    pub(super) fn attribute_list(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::AttributeList, s)?;
        while self.peek() == T::HashLBracket {
            let g = self.open(K::AttributeGroup, f::NONE, C_NONE)?;
            self.list(K::AttributeGroup, T::RBracket, false, true, |p| {
                let a = p.open(K::Attribute, f::NONE, C_NONE)?;
                p.class_ref_name(BARE)?;
                if p.peek() == T::LParen {
                    p.arguments(slot(f::PARAMETERS, Ctx::Bare))?;
                }
                p.close(a);
                Ok(())
            })?;
            self.close(g);
        }
        self.close(n);
        Ok(n)
    }

    /// A statement after attributes: a function, a class, an interface, a trait, an enum (or an
    /// expression whose closure they are).
    pub(super) fn attributed_statement(&mut self, s: Slot) -> R<N> {
        let mut k = 0;
        while self.kind_at(k) == T::HashLBracket {
            k = self.matching(k).ok_or_else(|| Refusal::Parse(self.tok().start))? + 1;
        }
        let t = self.peek_at(k);
        let next = self.kind_at(k + 1);
        match t.word {
            w::FUNCTION if next == T::Name || next == T::Amp && self.kind_at(k + 2) == T::Name => self.function_definition(s),
            w::ABSTRACT | w::FINAL | w::READONLY | w::CLASS => self.class_declaration(s),
            w::INTERFACE => self.interface_declaration(s),
            w::TRAIT => self.trait_declaration(s),
            w::ENUM => self.enum_declaration(s),
            _ => self.expression_statement(s),
        }
    }

    fn attributes_opt(&mut self) -> R<()> {
        if self.peek() == T::HashLBracket {
            self.attribute_list(slot(f::ATTRIBUTES, Ctx::Bare))?;
        }
        Ok(())
    }

    pub(super) fn function_definition(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::FunctionDefinition, s)?;
        self.attributes_opt()?;
        self.kw(K::FunctionDefinition, w::FUNCTION)?;
        if self.peek() == T::Amp {
            self.token_node(K::ReferenceModifier, f::NONE, K::TAmp)?;
        }
        self.name(slot(f::NAME, Ctx::Bare))?;
        self.formal_parameters()?;
        self.return_type_opt(K::FunctionDefinition)?;
        self.compound(slot(f::BODY, Ctx::Bare))?;
        self.close(n);
        Ok(n)
    }

    /// `: type` (or `never`) in `return_type`.
    pub(super) fn return_type_opt(&mut self, parent: K) -> R<()> {
        if self.peek() != T::Colon {
            return Ok(());
        }
        self.tk(parent, K::TColon)?;
        if self.is_word(w::NEVER) && !matches!(self.kind_at(1), T::Pipe | T::Amp) {
            self.leaf(K::BottomType, f::RETURN_TYPE, C_NONE)?;
            return Ok(());
        }
        self.ty(slot(f::RETURN_TYPE, Ctx::Type))?;
        Ok(())
    }

    /// `formal_parameters`, `(` parameters `)`.
    pub(super) fn formal_parameters(&mut self) -> R<N> {
        let n = self.open(K::FormalParameters, f::PARAMETERS, C_NONE)?;
        if self.peek() != T::LParen {
            return self.refuse();
        }
        self.list(K::FormalParameters, T::RParen, true, true, |p| p.parameter())?;
        self.close(n);
        Ok(n)
    }

    fn parameter(&mut self) -> R<()> {
        // Its kind, from what follows its attributes.
        let mut k = 0;
        while self.kind_at(k) == T::HashLBracket {
            k = self.matching(k).ok_or_else(|| Refusal::Parse(self.tok().start))? + 1;
        }
        let promoted = matches!(self.word_at(k), w::PUBLIC | w::PROTECTED | w::PRIVATE | w::READONLY);
        if promoted {
            let n = self.open(K::PropertyPromotionParameter, f::NONE, C_NONE)?;
            self.attributes_opt()?;
            if self.is_word(w::READONLY) {
                // `readonly public`: the visibility first is the grammar's only order.
                return self.refuse();
            }
            self.visibility_modifier(slot(f::VISIBILITY, Ctx::Bare))?;
            if self.is_word(w::READONLY) {
                self.leaf(K::ReadonlyModifier, f::READONLY, C_NONE)?;
            }
            if !matches!(self.peek(), T::Dollar | T::Amp) {
                self.ty(slot(f::TYPE, Ctx::Type))?;
            }
            if self.peek() == T::Amp {
                self.by_ref(slot(f::NAME, Ctx::Bare))?;
            } else {
                self.variable_name(slot(f::NAME, Ctx::Bare))?;
            }
            if self.peek() == T::Eq {
                self.tk(K::PropertyPromotionParameter, K::TEq)?;
                self.expr(slot(f::DEFAULT_VALUE, Ctx::Expr))?;
            }
            if self.peek() == T::LBrace {
                self.property_hooks()?;
            }
            self.close(n);
            return Ok(());
        }
        // `simple_parameter` or `variadic_parameter`: the `...` decides.
        let mut j = k;
        let mut variadic = false;
        loop {
            match self.kind_at(j) {
                T::DotDotDot => {
                    variadic = true;
                    break;
                }
                T::Dollar | T::Comma | T::RParen | T::Eof => break,
                _ => j += 1,
            }
        }
        let kind = if variadic { K::VariadicParameter } else { K::SimpleParameter };
        let n = self.open(kind, f::NONE, C_NONE)?;
        self.attributes_opt()?;
        if !matches!(self.peek(), T::Dollar | T::Amp | T::DotDotDot) {
            self.ty(slot(f::TYPE, Ctx::Type))?;
        }
        if self.peek() == T::Amp {
            self.token_node(K::ReferenceModifier, f::REFERENCE_MODIFIER, K::TAmp)?;
        }
        if variadic {
            self.expect(kind, T::DotDotDot)?;
        }
        self.variable_name(slot(f::NAME, Ctx::Bare))?;
        if !variadic && self.peek() == T::Eq {
            self.tk(K::SimpleParameter, K::TEq)?;
            self.expr(slot(f::DEFAULT_VALUE, Ctx::Expr))?;
        }
        self.close(n);
        Ok(())
    }

    /// `visibility_modifier`: `public`, `protected` or `private`, and `(set)`.
    pub(super) fn visibility_modifier(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::VisibilityModifier, s)?;
        let k = match self.word_at(0) {
            w::PUBLIC => K::TPublic,
            w::PROTECTED => K::TProtected,
            w::PRIVATE => K::TPrivate,
            _ => return self.refuse(),
        };
        let end = self.tok().end;
        self.tk(K::VisibilityModifier, k)?;
        if self.peek() == T::LParen && self.tok().start == end {
            // `private(set)`: immediate tokens.
            // Its `(` is stored: the gap before `operation` holds the visibility's word too.
            self.tok_leaf(K::TLParen)?;
            self.leaf(K::Operation, f::NONE, C_NONE)?;
            self.tk(K::VisibilityModifier, K::TRParen)?;
        }
        self.close(n);
        Ok(n)
    }

    /// The modifiers before a member or a class: `_modifier`s.
    fn modifiers(&mut self) -> R<()> {
        loop {
            match self.word_at(0) {
                w::PUBLIC | w::PROTECTED | w::PRIVATE => {
                    self.visibility_modifier(BARE)?;
                }
                w::STATIC => {
                    let n = self.open(K::StaticModifier, f::NONE, C_NONE)?;
                    self.tk(K::StaticModifier, K::TStatic)?;
                    self.close(n);
                }
                w::FINAL => {
                    let n = self.open(K::FinalModifier, f::NONE, C_NONE)?;
                    self.tk(K::FinalModifier, K::TFinal)?;
                    self.close(n);
                }
                w::ABSTRACT => {
                    let n = self.open(K::AbstractModifier, f::NONE, C_NONE)?;
                    self.tk(K::AbstractModifier, K::TAbstract)?;
                    self.close(n);
                }
                w::READONLY => {
                    let n = self.open(K::ReadonlyModifier, f::NONE, C_NONE)?;
                    self.tk(K::ReadonlyModifier, K::TReadonly)?;
                    self.close(n);
                }
                w::VAR => {
                    self.leaf(K::VarModifier, f::NONE, C_NONE)?;
                }
                _ => return Ok(()),
            }
        }
    }

    pub(super) fn class_declaration(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ClassDeclaration, s)?;
        self.attributes_opt()?;
        self.modifiers()?;
        self.kw(K::ClassDeclaration, w::CLASS)?;
        self.class_name(slot(f::NAME, Ctx::Bare))?;
        self.class_tail()?;
        self.declaration_list(K::DeclarationList)?;
        self.close(n);
        Ok(n)
    }

    /// `base_clause` and `class_interface_clause`, where they are.
    pub(super) fn class_tail(&mut self) -> R<()> {
        if self.is_word(w::EXTENDS) {
            self.base_clause()?;
        }
        if self.is_word(w::IMPLEMENTS) {
            let c = self.open(K::ClassInterfaceClause, f::NONE, C_NONE)?;
            self.kw(K::ClassInterfaceClause, w::IMPLEMENTS)?;
            loop {
                self.class_ref_name(BARE)?;
                if self.peek() != T::Comma {
                    break;
                }
                self.tk(K::ClassInterfaceClause, K::TComma)?;
            }
            self.close(c);
        }
        Ok(())
    }

    fn base_clause(&mut self) -> R<()> {
        let c = self.open(K::BaseClause, f::NONE, C_NONE)?;
        self.kw(K::BaseClause, w::EXTENDS)?;
        loop {
            self.class_ref_name(BARE)?;
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::BaseClause, K::TComma)?;
        }
        self.close(c);
        Ok(())
    }

    pub(super) fn interface_declaration(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::InterfaceDeclaration, s)?;
        self.attributes_opt()?;
        self.kw(K::InterfaceDeclaration, w::INTERFACE)?;
        self.class_name(slot(f::NAME, Ctx::Bare))?;
        if self.is_word(w::EXTENDS) {
            self.base_clause()?;
        }
        self.declaration_list(K::DeclarationList)?;
        self.close(n);
        Ok(n)
    }

    pub(super) fn trait_declaration(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::TraitDeclaration, s)?;
        self.attributes_opt()?;
        self.kw(K::TraitDeclaration, w::TRAIT)?;
        self.class_name(slot(f::NAME, Ctx::Bare))?;
        self.declaration_list(K::DeclarationList)?;
        self.close(n);
        Ok(n)
    }

    pub(super) fn enum_declaration(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::EnumDeclaration, s)?;
        self.attributes_opt()?;
        self.tk(K::EnumDeclaration, K::TEnum)?;
        self.class_name(slot(f::NAME, Ctx::Bare))?;
        if self.peek() == T::Colon {
            self.tk(K::EnumDeclaration, K::TColon)?;
            if !(self.is_exact_at(0, w::STRING) || self.is_exact_at(0, w::INT)) {
                return self.refuse();
            }
            self.leaf(K::PrimitiveType, f::NONE, C_NONE)?;
        }
        if self.is_word(w::IMPLEMENTS) {
            self.class_tail()?;
        }
        self.declaration_list(K::EnumDeclarationList)?;
        self.close(n);
        Ok(n)
    }

    /// A class's, an interface's, a trait's or an enum's body (`kind`): `{` members `}`.
    pub(super) fn declaration_list(&mut self, kind: K) -> R<N> {
        let n = self.open(kind, f::BODY, C_NONE)?;
        self.expect(kind, T::LBrace)?;
        while self.peek() != T::RBrace {
            if self.peek() == T::Eof {
                return self.refuse();
            }
            self.member()?;
        }
        self.expect(kind, T::RBrace)?;
        self.close(n);
        Ok(n)
    }

    /// A member: a constant, a property, a method, a trait's use, an enum's case.
    fn member(&mut self) -> R<()> {
        self.deeper()?;
        let mut k = 0;
        while self.kind_at(k) == T::HashLBracket {
            k = self.matching(k).ok_or_else(|| Refusal::Parse(self.tok().start))? + 1;
        }
        let start = k;
        while matches!(self.word_at(k), w::PUBLIC | w::PROTECTED | w::PRIVATE | w::STATIC | w::FINAL | w::ABSTRACT | w::READONLY | w::VAR) {
            if self.kind_at(k + 1) == T::LParen && matches!(self.word_at(k), w::PUBLIC | w::PROTECTED | w::PRIVATE) {
                k += 4;
            } else {
                k += 1;
            }
        }
        let modifiers = k > start;
        let r = match self.word_at(k) {
            w::CONST => self.class_const().map(|_| ()),
            w::FUNCTION => self.method_declaration().map(|_| ()),
            w::USE if !modifiers => self.use_member().map(|_| ()),
            w::CASE if !modifiers => self.enum_case().map(|_| ()),
            _ if modifiers => self.property_declaration().map(|_| ()),
            _ => self.refuse(),
        };
        self.depth -= 1;
        r
    }

    fn class_const(&mut self) -> R<N> {
        let n = self.open(K::ConstDeclaration, f::NONE, C_NONE)?;
        self.attributes_opt()?;
        self.modifiers()?;
        self.kw(K::ConstDeclaration, w::CONST)?;
        self.const_elements(false)?;
        self.close(n);
        Ok(n)
    }

    /// A `const` statement.
    pub(super) fn const_declaration(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ConstDeclaration, s)?;
        self.kw(K::ConstDeclaration, w::CONST)?;
        self.const_elements(true)?;
        self.close(n);
        Ok(n)
    }

    /// An optional type, then `const_element`s and `;`. A statement's names are `name`s whose
    /// reserved set is `global`; a class's, `nothing`.
    fn const_elements(&mut self, statement: bool) -> R<()> {
        // A type where two words come before the `=`. A type's word before `=` (`const FALSE =`)
        // is lexed as the type: tree-sitter's tree has an error there.
        if self.peek() == T::Name && self.kind_at(1) == T::Eq && self.is_type_word_at(0) {
            return self.refuse();
        }
        let typed = !(self.peek() == T::Name && self.kind_at(1) == T::Eq);
        if typed {
            self.ty(slot(f::TYPE, Ctx::Type))?;
        }
        loop {
            let e = self.open(K::ConstElement, f::NONE, C_NONE)?;
            // After a type, a reserved word is not read as a name (tree-sitter's tree has an error).
            if statement || typed {
                self.name(BARE)?;
            } else {
                self.name_any(BARE)?;
            }
            self.tk(K::ConstElement, K::TEq)?;
            self.expr(EXPR)?;
            self.close(e);
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::ConstDeclaration, K::TComma)?;
        }
        self.semi(K::ConstDeclaration)
    }

    fn method_declaration(&mut self) -> R<N> {
        let n = self.open(K::MethodDeclaration, f::NONE, C_NONE)?;
        self.attributes_opt()?;
        self.modifiers()?;
        self.kw(K::MethodDeclaration, w::FUNCTION)?;
        if self.peek() == T::Amp {
            self.token_node(K::ReferenceModifier, f::NONE, K::TAmp)?;
        }
        self.name_any(slot(f::NAME, Ctx::Bare))?;
        self.formal_parameters()?;
        self.return_type_opt(K::MethodDeclaration)?;
        if self.peek() == T::LBrace {
            self.compound(slot(f::BODY, Ctx::Bare))?;
        } else {
            self.semi(K::MethodDeclaration)?;
        }
        self.close(n);
        Ok(n)
    }

    fn property_declaration(&mut self) -> R<N> {
        let n = self.open(K::PropertyDeclaration, f::NONE, C_NONE)?;
        self.attributes_opt()?;
        self.modifiers()?;
        if self.peek() != T::Dollar {
            self.ty(slot(f::TYPE, Ctx::Type))?;
        }
        loop {
            let e = self.open(K::PropertyElement, f::NONE, C_NONE)?;
            self.variable_name(slot(f::NAME, Ctx::Bare))?;
            if self.peek() == T::Eq {
                self.tk(K::PropertyElement, K::TEq)?;
                self.expr(slot(f::DEFAULT_VALUE, Ctx::Expr))?;
            }
            self.close(e);
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::PropertyDeclaration, K::TComma)?;
        }
        if self.peek() == T::LBrace {
            self.property_hooks()?;
        } else {
            self.semi(K::PropertyDeclaration)?;
        }
        self.close(n);
        Ok(n)
    }

    /// `property_hook_list`: `{` hooks `}`.
    fn property_hooks(&mut self) -> R<()> {
        let n = self.open(K::PropertyHookList, f::NONE, C_NONE)?;
        self.expect(K::PropertyHookList, T::LBrace)?;
        while self.peek() != T::RBrace {
            let h = self.open(K::PropertyHook, f::NONE, C_NONE)?;
            self.attributes_opt()?;
            if self.is_word(w::FINAL) {
                let m = self.open(K::FinalModifier, f::FINAL, C_NONE)?;
                self.tk(K::FinalModifier, K::TFinal)?;
                self.close(m);
            }
            if self.peek() == T::Amp {
                self.token_node(K::ReferenceModifier, f::REFERENCE_MODIFIER, K::TAmp)?;
            }
            self.name_any(BARE)?;
            if self.peek() == T::LParen {
                let p = self.open(K::FormalParameters, f::PARAMETERS, C_NONE)?;
                self.list(K::FormalParameters, T::RParen, true, true, |p| p.parameter())?;
                self.close(p);
            }
            match self.peek() {
                T::FatArrow => {
                    self.tk(K::PropertyHook, K::TFatArrow)?;
                    self.expr(slot(f::BODY, Ctx::Expr))?;
                    self.semi(K::PropertyHook)?;
                }
                T::LBrace => {
                    self.compound(slot(f::BODY, Ctx::Bare))?;
                }
                _ => self.semi(K::PropertyHook)?,
            }
            self.close(h);
        }
        self.expect(K::PropertyHookList, T::RBrace)?;
        self.close(n);
        Ok(())
    }

    /// A trait's `use`: `use A, B;` or `use A { … }`.
    fn use_member(&mut self) -> R<N> {
        let n = self.open(K::UseDeclaration, f::NONE, C_NONE)?;
        self.kw(K::UseDeclaration, w::USE)?;
        loop {
            self.class_ref_name(BARE)?;
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::UseDeclaration, K::TComma)?;
        }
        if self.peek() == T::LBrace {
            let l = self.open(K::UseList, f::NONE, C_NONE)?;
            self.expect(K::UseList, T::LBrace)?;
            let mut semi = false;
            while self.peek() != T::RBrace {
                self.use_rule()?;
                semi = self.peek() == T::Semi;
                self.semi(K::UseList)?;
            }
            // After a `;` (a `_semicolon`, which may be empty) the closing gap is no implied word.
            self.punct_tk(K::UseList, T::RBrace, semi)?;
            self.close(l);
        } else {
            self.semi(K::UseDeclaration)?;
        }
        self.close(n);
        Ok(n)
    }

    /// `A::b insteadof C` or `b as [visibility] c`.
    fn use_rule(&mut self) -> R<()> {
        // `A::b` (a class constant access) or a name, then `insteadof` or `as`.
        let mut k = 0;
        while matches!(self.kind_at(k), T::Name | T::Backslash | T::ColonColon) && !(self.kind_at(k) == T::Name && k > 0 && matches!(self.word_at(k), w::INSTEADOF | w::AS) && self.kind_at(k - 1) == T::Name) {
            k += 1;
        }
        let instead = self.is_word_at(k, w::INSTEADOF);
        let kind = if instead { K::UseInsteadOfClause } else { K::UseAsClause };
        let n = self.open(kind, f::NONE, C_NONE)?;
        let scoped = (0..k).any(|j| self.kind_at(j) == T::ColonColon);
        if scoped {
            let c = self.open(K::ClassConstantAccessExpression, f::NONE, C_NONE)?;
            self.class_ref_name(BARE)?;
            self.tk(K::ClassConstantAccessExpression, K::TColonColon)?;
            self.name_any(BARE)?;
            self.close(c);
        } else if instead {
            return self.refuse();
        } else {
            self.name_any(BARE)?;
        }
        if instead {
            self.kw(kind, w::INSTEADOF)?;
            self.name_any(BARE)?;
        } else {
            self.kw(kind, w::AS)?;
            if matches!(self.word_at(0), w::PUBLIC | w::PROTECTED | w::PRIVATE) {
                self.visibility_modifier(BARE)?;
            }
            if !matches!(self.peek(), T::Semi | T::CloseTag) {
                self.name_any(BARE)?;
            }
        }
        self.close(n);
        Ok(())
    }

    fn enum_case(&mut self) -> R<N> {
        let n = self.open(K::EnumCase, f::NONE, C_NONE)?;
        self.attributes_opt()?;
        self.kw(K::EnumCase, w::CASE)?;
        self.name_any(slot(f::NAME, Ctx::Bare))?;
        if self.peek() == T::Eq {
            self.tk(K::EnumCase, K::TEq)?;
            self.expr(slot(f::VALUE, Ctx::Expr))?;
        }
        self.semi(K::EnumCase)?;
        self.close(n);
        Ok(n)
    }
}
