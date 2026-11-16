//! C++'s own constructs: namespaces, `using`, templates and their argument lists, qualified
//! names, classes' bases and members (access specifiers, constructors, inline methods, friends),
//! `static_assert`, and the expressions and statements tree-sitter-cpp adds (`new`, `delete`,
//! lambdas, `this`, `try`, `throw`, range `for`, condition clauses).

use super::decl::Fam;
use super::*;

/// What a name stands for: which kinds its parts are written as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Role {
    /// An expression's: `identifier`, `template_function`, `operator_name`, `destructor_name`.
    Expr,
    /// A type's: `type_identifier`, `template_type`.
    Type,
    /// A field's: `field_identifier`, `template_method`.
    Field,
    /// A declarator's: as an expression's.
    Decl,
}

impl<'a> P<'a> {
    // ---- Items.

    /// A C++ item at `level` under `parent`, where the next tokens start one. Whether one was read.
    pub(super) fn cpp_item(&mut self, level: Level, parent: K) -> R<bool> {
        let t = self.tok();
        if t.kind == T::Name {
            match t.word {
                w::NAMESPACE => {
                    self.namespace()?;
                    return Ok(true);
                }
                w::INLINE if self.is_word_at(1, w::NAMESPACE) => {
                    self.namespace()?;
                    return Ok(true);
                }
                w::USING => {
                    self.using()?;
                    return Ok(true);
                }
                w::TEMPLATE => {
                    self.template_item(level, parent)?;
                    return Ok(true);
                }
                w::STATIC_ASSERT => {
                    self.static_assert()?;
                    return Ok(true);
                }
                w::CONCEPT => {
                    self.concept()?;
                    return Ok(true);
                }
                w::PUBLIC | w::PRIVATE | w::PROTECTED if level == Level::Field && self.kind_at(1) == T::Colon => {
                    let n = self.open(K::AccessSpecifier, f::NONE, C_NONE);
                    self.kw_any(K::AccessSpecifier)?;
                    self.close(n);
                    self.tk(K::FieldDeclarationList, K::TColon)?;
                    return Ok(true);
                }
                w::FRIEND => {
                    self.friend(level)?;
                    return Ok(true);
                }
                w::TYPEDEF if level == Level::Field => {
                    self.type_definition()?;
                    return Ok(true);
                }
                w::CONSTEXPR if self.is_word_at(1, w::FRIEND) => {
                    self.friend(level)?;
                    return Ok(true);
                }
                _ => {}
            }
        }
        if level == Level::Field && t.kind == T::Semi {
            self.leaf(K::TSemi, f::NONE, C_NONE)?;
            return Ok(true);
        }
        // A namespace's items are the top's.
        let level = if parent == K::DeclarationList && level == Level::Block { Level::Top } else { level };
        if self.ctor_ahead(level) {
            self.ctor(level)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// `namespace name { … }`, `inline namespace`, `namespace a::b { … }`, `namespace x = y;`.
    fn namespace(&mut self) -> R<()> {
        let k = usize::from(self.is_word(w::INLINE));
        if self.is_ident_at(k + 1) && self.kind_at(k + 2) == T::Eq {
            let n = self.open(K::NamespaceAliasDefinition, f::NONE, C_NONE);
            self.kw(K::NamespaceAliasDefinition, w::NAMESPACE)?;
            self.leaf(K::NamespaceIdentifier, f::NAME, C_NONE)?;
            self.tk(K::NamespaceAliasDefinition, K::TEq)?;
            self.namespace_name(BARE)?;
            self.punct_tk(K::NamespaceAliasDefinition, T::Semi, false)?;
            self.close(n);
            return Ok(());
        }
        let n = self.open(K::NamespaceDefinition, f::NONE, C_NONE);
        if self.is_word(w::INLINE) {
            self.kw(K::NamespaceDefinition, w::INLINE)?;
        }
        self.kw(K::NamespaceDefinition, w::NAMESPACE)?;
        if self.peek() == T::LBracket && self.kind_at(1) == T::LBracket {
            self.attribute_declaration()?;
        }
        if self.is_ident() || self.peek() == T::ColonColon {
            self.namespace_name(slot(f::NAME, Ctx::Bare))?;
        }
        self.declaration_list(slot(f::BODY, Ctx::Bare))?;
        self.close(n);
        Ok(())
    }

    /// A namespace's name: `a`, or `a::b::c` (a `nested_namespace_specifier`, right-nested).
    fn namespace_name(&mut self, s: Slot) -> R<N> {
        let k = usize::from(self.is_word(w::INLINE));
        let nested = self.peek() == T::ColonColon || self.kind_at(k + 1) == T::ColonColon;
        if !nested {
            if !self.is_ident() {
                return self.refuse();
            }
            return self.leaf_in(K::NamespaceIdentifier, s);
        }
        let n = self.open_in(K::NestedNamespaceSpecifier, s);
        if self.peek() != T::ColonColon {
            if self.is_word(w::INLINE) {
                self.kw(K::NestedNamespaceSpecifier, w::INLINE)?;
            }
            if !self.is_ident() {
                return self.refuse();
            }
            self.leaf(K::NamespaceIdentifier, f::NONE, C_NONE)?;
        }
        self.tk(K::NestedNamespaceSpecifier, K::TColonColon)?;
        self.namespace_name(BARE)?;
        self.close(n);
        Ok(n)
    }

    /// `{` block items `}`: a namespace's or `extern "C"`'s body.
    pub(super) fn declaration_list(&mut self, s: Slot) -> R<N> {
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        let l = self.open_in(K::DeclarationList, s);
        self.tk(K::DeclarationList, K::TLBrace)?;
        let top = std::mem::replace(&mut self.top, true);
        while self.peek() != T::RBrace {
            if self.peek() == T::Eof {
                return self.refuse();
            }
            self.item(Level::Block, K::DeclarationList)?;
            self.top = true;
        }
        self.top = top;
        self.tk(K::DeclarationList, K::TRBrace)?;
        self.close(l);
        Ok(l)
    }

    /// `using namespace x;`, `using a::b;`, `using T = type;`.
    fn using(&mut self) -> R<()> {
        if self.is_ident_at(1) && (self.kind_at(2) == T::Eq || self.kind_at(2) == T::LBracket && self.kind_at(3) == T::LBracket) {
            let n = self.open(K::AliasDeclaration, f::NONE, C_NONE);
            self.kw(K::AliasDeclaration, w::USING)?;
            self.leaf(K::TypeIdentifier, f::NAME, C_NONE)?;
            while self.peek() == T::LBracket {
                self.attribute_declaration()?;
            }
            self.tk(K::AliasDeclaration, K::TEq)?;
            self.type_descriptor(slot(f::TYPE, Ctx::Bare))?;
            self.punct_tk(K::AliasDeclaration, T::Semi, false)?;
            self.close(n);
            return Ok(());
        }
        let n = self.open(K::UsingDeclaration, f::NONE, C_NONE);
        self.kw(K::UsingDeclaration, w::USING)?;
        if self.is_word(w::NAMESPACE) || self.is_word(w::ENUM) {
            self.kw_any(K::UsingDeclaration)?;
        }
        self.qname(Role::Expr, BARE)?;
        self.punct_tk(K::UsingDeclaration, T::Semi, false)?;
        self.close(n);
        Ok(())
    }

    /// `static_assert(condition, message);`.
    fn static_assert(&mut self) -> R<()> {
        let n = self.open(K::StaticAssertDeclaration, f::NONE, C_NONE);
        self.kw(K::StaticAssertDeclaration, w::STATIC_ASSERT)?;
        self.punct_tk(K::StaticAssertDeclaration, T::LParen, false)?;
        self.expr(slot(f::CONDITION, Ctx::Expr))?;
        if self.peek() == T::Comma {
            self.tk(K::StaticAssertDeclaration, K::TComma)?;
            self.strings(slot(f::MESSAGE, Ctx::Bare))?;
        }
        self.punct_tk(K::StaticAssertDeclaration, T::RParen, false)?;
        self.punct_tk(K::StaticAssertDeclaration, T::Semi, false)?;
        self.close(n);
        Ok(())
    }

    /// `concept name = expression;`.
    fn concept(&mut self) -> R<()> {
        let n = self.open(K::ConceptDefinition, f::NONE, C_NONE);
        self.kw(K::ConceptDefinition, w::CONCEPT)?;
        if !self.is_ident() {
            return self.refuse();
        }
        self.leaf(K::Identifier, f::NAME, C_NONE)?;
        self.tk(K::ConceptDefinition, K::TEq)?;
        self.expr(EXPR)?;
        self.punct_tk(K::ConceptDefinition, T::Semi, false)?;
        self.close(n);
        Ok(())
    }

    /// `friend` and a declaration, a definition, or `class Name;`.
    fn friend(&mut self, level: Level) -> R<()> {
        let n = self.open(K::FriendDeclaration, f::NONE, C_NONE);
        if self.is_word(w::CONSTEXPR) {
            self.kw(K::FriendDeclaration, w::CONSTEXPR)?;
        }
        self.kw(K::FriendDeclaration, w::FRIEND)?;
        // `friend class X;`, `friend X;`.
        let k = usize::from(self.is_word(w::CLASS) || self.is_word(w::STRUCT) || self.is_word(w::UNION));
        if let Some(e) = self.class_name_end(k) {
            if self.kind_at(e) == T::Semi {
                if k == 1 {
                    self.kw_any(K::FriendDeclaration)?;
                }
                self.class_name(BARE)?;
                self.tk(K::FriendDeclaration, K::TSemi)?;
                self.close(n);
                return Ok(());
            }
        }
        let _ = level;
        if self.ctor_ahead(Level::Top) {
            self.ctor(Level::Top)?;
        } else {
            self.declaration_item(Level::Top, K::FriendDeclaration)?;
        }
        self.close(n);
        Ok(())
    }

    /// `template <parameters> item`, or `template declaration;` (an instantiation).
    fn template_item(&mut self, level: Level, _parent: K) -> R<()> {
        if self.kind_at(1) != T::Lt {
            let n = self.open(K::TemplateInstantiation, f::NONE, C_NONE);
            self.kw(K::TemplateInstantiation, w::TEMPLATE)?;
            self.specifiers(K::TemplateInstantiation)?;
            self.declarator(Fam::Decl, slot(f::DECLARATOR, Ctx::Decl))?;
            self.punct_tk(K::TemplateInstantiation, T::Semi, false)?;
            self.close(n);
            return Ok(());
        }
        let n = self.open(K::TemplateDeclaration, f::NONE, C_NONE);
        self.kw(K::TemplateDeclaration, w::TEMPLATE)?;
        self.template_parameters(slot(f::PARAMETERS, Ctx::Bare))?;
        if self.is_word(w::REQUIRES) {
            self.requires_clause(BARE)?;
        }
        // The declaration: never a field's (a member template's declarators are `_declarator`s).
        let top = std::mem::replace(&mut self.top, true);
        let _ = level;
        if self.is_word(w::FRIEND) || self.is_word(w::CONSTEXPR) && self.is_word_at(1, w::FRIEND) {
            self.friend(Level::Top)?;
        } else if !self.cpp_item(if level == Level::Field { Level::Field } else { Level::Top }, K::TemplateDeclaration)? {
            if !self.declaration_ahead() {
                return self.refuse();
            }
            self.declaration_item(Level::Top, K::TemplateDeclaration)?;
        }
        self.top = top;
        self.close(n);
        Ok(())
    }

    /// `<` template parameters `>`.
    fn template_parameters(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::TemplateParameterList, s);
        self.tk(K::TemplateParameterList, K::TLt)?;
        let no_gt = std::mem::replace(&mut self.no_gt, true);
        if !self.at_angle_close() {
            loop {
                self.template_parameter()?;
                if self.peek() == T::Comma {
                    self.tk(K::TemplateParameterList, K::TComma)?;
                    continue;
                }
                break;
            }
        }
        self.no_gt = no_gt;
        self.angle_close(K::TemplateParameterList)?;
        self.close(n);
        Ok(n)
    }

    fn template_parameter(&mut self) -> R<()> {
        let t = self.tok();
        if t.kind == T::Name && matches!(t.word, w::TYPENAME | w::CLASS) {
            // `typename T`, `class... Ts`, `typename T = int`; `typename T::x y` is a parameter.
            let k = if self.kind_at(1) == T::DotDotDot { 2 } else { 1 };
            let named = self.is_ident_at(k);
            let after = if named { k + 1 } else { k };
            if matches!(self.kind_at(after), T::Comma | T::Gt | T::GtGt | T::Eq) {
                let variadic = k == 2;
                let optional = self.kind_at(after) == T::Eq;
                let kind = if variadic {
                    K::VariadicTypeParameterDeclaration
                } else if optional {
                    K::OptionalTypeParameterDeclaration
                } else {
                    K::TypeParameterDeclaration
                };
                let n = self.open(kind, f::NONE, C_NONE);
                self.kw_any(kind)?;
                if variadic {
                    self.tk(kind, K::TDotDotDot)?;
                }
                if named {
                    let field = if optional { f::NAME } else { f::NONE };
                    self.leaf(K::TypeIdentifier, field, C_NONE)?;
                }
                if optional {
                    self.tk(kind, K::TEq)?;
                    if !self.type_specifier(slot(f::DEFAULT_TYPE, Ctx::Type))? {
                        return self.refuse();
                    }
                }
                self.close(n);
                return Ok(());
            }
        }
        if t.kind == T::Name && t.word == w::TEMPLATE {
            let n = self.open(K::TemplateTemplateParameterDeclaration, f::NONE, C_NONE);
            self.kw(K::TemplateTemplateParameterDeclaration, w::TEMPLATE)?;
            self.template_parameters(slot(f::PARAMETERS, Ctx::Bare))?;
            self.template_parameter()?;
            self.close(n);
            return Ok(());
        }
        self.parameter_declaration()
    }

    /// Whether the next token closes an angle-bracketed list (a `>`, or a `>>`'s first half).
    fn at_angle_close(&mut self) -> bool {
        matches!(self.peek(), T::Gt | T::GtGt | T::GtGtEq | T::Ge)
    }

    /// The closing `>` under `parent` (a `>>` split).
    fn angle_close(&mut self, parent: K) -> R<()> {
        if matches!(self.peek(), T::GtGt | T::GtGtEq | T::Ge) {
            self.split_gt();
        }
        if self.peek() != T::Gt {
            return self.refuse();
        }
        self.tk(parent, K::TGt)
    }

    /// `requires` and a constraint.
    pub(super) fn requires_clause(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::RequiresClause, s);
        self.kw(K::RequiresClause, w::REQUIRES)?;
        self.constraint(slot(f::CONSTRAINT, Ctx::Bare))?;
        self.close(n);
        Ok(n)
    }

    /// A requires clause's constraint: `&&` and `||` over primaries.
    fn constraint(&mut self, s: Slot) -> R<N> {
        let mut left = self.constraint_primary(s)?;
        let mut min_or = false;
        loop {
            let (kind, op) = match self.peek() {
                T::AmpAmp => (K::ConstraintConjunction, K::TAmpAmp),
                T::PipePipe => (K::ConstraintDisjunction, K::TPipePipe),
                _ => return Ok(left),
            };
            let _ = &mut min_or;
            let n = self.wrap(left, kind, s, slot(f::LEFT, Ctx::Bare));
            self.leaf(op, f::OPERATOR, C_NONE)?;
            if kind == K::ConstraintDisjunction {
                self.constraint(slot(f::RIGHT, Ctx::Bare))?;
            } else {
                self.constraint_primary(slot(f::RIGHT, Ctx::Bare))?;
            }
            self.close(n);
            left = n;
            min_or = true;
        }
    }

    fn constraint_primary(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::LParen => {
                // `( expression )`: the parentheses the clause's.
                self.tk(K::RequiresClause, K::TLParen)?;
                let e = self.expr(EXPR)?;
                self.punct_tk(K::RequiresClause, T::RParen, false)?;
                Ok(e)
            }
            T::Name if matches!(t.word, w::TRUE) => self.leaf_in(K::True, s),
            T::Name if matches!(t.word, w::FALSE) => self.leaf_in(K::False, s),
            T::Name if t.word == w::REQUIRES => self.requires_expression(s),
            _ => self.class_name(s),
        }
    }

    /// `requires (parameters) { requirements }`.
    fn requires_expression(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::RequiresExpression, s);
        self.kw(K::RequiresExpression, w::REQUIRES)?;
        if self.peek() == T::LParen {
            self.parameter_list()?;
        }
        let r = self.open(K::RequirementSeq, f::REQUIREMENTS, C_NONE);
        self.punct_tk(K::RequirementSeq, T::LBrace, false)?;
        while self.peek() != T::RBrace {
            match self.tok() {
                t if t.kind == T::Eof => return self.refuse(),
                t if t.kind == T::Name && t.word == w::TYPENAME => {
                    let q = self.open(K::TypeRequirement, f::NONE, C_NONE);
                    self.kw(K::TypeRequirement, w::TYPENAME)?;
                    self.class_name(BARE)?;
                    self.punct_tk(K::TypeRequirement, T::Semi, false)?;
                    self.close(q);
                }
                t if t.kind == T::LBrace => {
                    let q = self.open(K::CompoundRequirement, f::NONE, C_NONE);
                    self.tk(K::CompoundRequirement, K::TLBrace)?;
                    self.expr(EXPR)?;
                    self.punct_tk(K::CompoundRequirement, T::RBrace, false)?;
                    if self.is_word(w::NOEXCEPT) {
                        self.kw(K::CompoundRequirement, w::NOEXCEPT)?;
                    }
                    if self.peek() == T::Arrow {
                        self.trailing_return()?;
                    }
                    self.punct_tk(K::CompoundRequirement, T::Semi, false)?;
                    self.close(q);
                }
                _ => {
                    let q = self.open(K::SimpleRequirement, f::NONE, C_NONE);
                    if self.peek() != T::Semi {
                        self.comma_expr(EXPR)?;
                    }
                    self.punct_tk(K::SimpleRequirement, T::Semi, false)?;
                    self.close(q);
                }
            }
        }
        self.tk(K::RequirementSeq, K::TRBrace)?;
        self.close(r);
        self.close(n);
        Ok(n)
    }

    // ---- Constructors, destructors, operator casts.

    /// Whether a constructor's, a destructor's or a conversion operator's declaration or definition
    /// is ahead: modifiers, then a name (qualified: `A::A`, `A::~A`) and its parameters, with no
    /// type before it.
    fn ctor_ahead(&mut self, level: Level) -> bool {
        let mut k = 0;
        loop {
            let t = self.peek_at(k);
            match t.kind {
                T::Name if matches!(t.word, w::EXPLICIT) => {
                    k += 1;
                    if self.kind_at(k) == T::LParen {
                        let Some(e) = self.paren_end(k) else { return false };
                        k = e + 1;
                    }
                }
                T::Name if matches!(t.word, w::VIRTUAL | w::INLINE | w::CONSTEXPR | w::STATIC | w::EXTERN | w::CONSTEVAL | w::U_INLINE | w::U_FORCEINLINE) => k += 1,
                T::LBracket if self.kind_at(k + 1) == T::LBracket => {
                    let Some(e) = self.paren_end(k) else { return false };
                    k = e + 1;
                }
                T::Name if matches!(t.word, w::U_ATTRIBUTE_U | w::U_ATTRIBUTE) => {
                    let Some(e) = self.paren_end(k + 1) else { return false };
                    k = e + 1;
                }
                _ => break,
            }
        }
        // `operator T()`: a conversion.
        if self.is_word_at(k, w::OPERATOR) {
            return !matches!(self.kind_at(k + 1), T::LParen) && self.operator_cast_ahead(k);
        }
        // The name: `~X`, `X`, `A::B::X`, `A<T>::X`.
        let mut j = k;
        let last;
        let mut prev = None;
        let mut destructor = false;
        loop {
            if self.kind_at(j) == T::Tilde {
                if !self.is_ident_at(j + 1) {
                    return false;
                }
                last = Some(self.peek_at(j + 1));
                j += 2;
                destructor = true;
                break;
            }
            if self.is_word_at(j, w::OPERATOR) {
                // `A::operator T()`.
                return j > k && self.operator_cast_ahead(j);
            }
            if !self.is_ident_at(j) {
                return false;
            }
            let name = self.peek_at(j);
            let mut e = j + 1;
            if self.kind_at(e) == T::Lt {
                match self.angle_end(e) {
                    Some(g) => e = g + 1,
                    None => return false,
                }
            }
            if self.kind_at(e) == T::ColonColon {
                prev = Some(name);
                j = e + 1;
                continue;
            }
            last = Some(name);
            j = e;
            break;
        }
        if self.kind_at(j) != T::LParen {
            return false;
        }
        let qualified = prev.is_some();
        // Outside a class, a constructor's name repeats its class's: `A::A(`, `A::~A(`.
        // Outside a class, a typeless `M(a, b) {` is a function's definition (a macro's), its
        // arguments read as parameters.
        // Outside blocks, a constructor's initializer list may follow (`X(T a) : b(a) {}`).
        if level != Level::Field && !qualified {
            let colon = level != Level::Block || self.top;
            return k == j - 1 && self.paren_end(j).is_some_and(|e| self.kind_at(e + 1) == T::LBrace || colon && self.kind_at(e + 1) == T::Colon && self.kind_at(e + 2) != T::Colon) && self.params_at(j);
        }
        if qualified && !destructor {
            let (Some(a), Some(b)) = (prev, last) else { return false };
            if self.src[a.start as usize..a.end as usize] != self.src[b.start as usize..b.end as usize] {
                return false;
            }
        }
        // After the parameters: a body, an initializer list, `= default` …, or `;`.
        let Some(e) = self.paren_end(j) else { return false };
        let mut a = e + 1;
        loop {
            let t = self.peek_at(a);
            match t.kind {
                T::Name if matches!(t.word, w::NOEXCEPT | w::THROW) => {
                    a += 1;
                    if self.kind_at(a) == T::LParen {
                        let Some(e) = self.paren_end(a) else { return false };
                        a = e + 1;
                    }
                }
                T::Name if matches!(t.word, w::OVERRIDE | w::FINAL | w::CONST) => a += 1,
                T::Name if matches!(t.word, w::U_ATTRIBUTE_U | w::U_ATTRIBUTE) => {
                    let Some(e) = self.paren_end(a + 1) else { return false };
                    a = e + 1;
                }
                _ => break,
            }
        }
        matches!(self.kind_at(a), T::Colon | T::LBrace | T::Semi | T::Eq) || self.is_word_at(a, w::TRY)
    }

    /// Whether `operator` at `k` names a conversion: `operator T(`, `operator T*(`.
    fn operator_cast_ahead(&mut self, k: usize) -> bool {
        let t = self.peek_at(k + 1);
        t.kind == T::ColonColon || t.kind == T::Name && (!self.reserved(t.word) || {
            let c = t.word;
            is_primitive(c) || matches!(c, w::CONST | w::VOLATILE | w::SIGNED | w::UNSIGNED | w::LONG | w::SHORT | w::AUTO | w::DECLTYPE | w::TYPENAME | w::STRUCT | w::CLASS | w::ENUM)
        })
    }

    /// A constructor's, a destructor's or a conversion's declaration or definition.
    fn ctor(&mut self, level: Level) -> R<()> {
        let start = self.tok().start;
        self.flush(start);
        let first = self.w.len();
        // Modifiers.
        loop {
            let t = self.tok();
            if t.kind == T::LBracket && self.kind_at(1) == T::LBracket {
                self.attribute_declaration()?;
                continue;
            }
            if t.kind != T::Name {
                break;
            }
            match t.word {
                w::EXPLICIT => {
                    let n = self.open(K::ExplicitFunctionSpecifier, f::NONE, C_NONE);
                    self.kw(K::ExplicitFunctionSpecifier, w::EXPLICIT)?;
                    if self.peek() == T::LParen {
                        self.tk(K::ExplicitFunctionSpecifier, K::TLParen)?;
                        self.expr(EXPR)?;
                        self.punct_tk(K::ExplicitFunctionSpecifier, T::RParen, false)?;
                    }
                    self.close(n);
                }
                w::VIRTUAL => {
                    self.leaf(K::Tvirtual, f::NONE, C_NONE)?;
                }
                w::INLINE | w::STATIC | w::EXTERN | w::U_INLINE | w::U_FORCEINLINE => {
                    let n = self.open(K::StorageClassSpecifier, f::NONE, C_NONE);
                    self.kw_any(K::StorageClassSpecifier)?;
                    self.close(n);
                }
                w::CONSTEXPR | w::CONSTEVAL => {
                    let n = self.open(K::TypeQualifier, f::NONE, C_NONE);
                    self.kw_any(K::TypeQualifier)?;
                    self.close(n);
                }
                w::U_ATTRIBUTE_U | w::U_ATTRIBUTE => self.attribute_specifier()?,
                _ => break,
            }
        }
        if self.ctor_is_cast() {
            return self.operator_cast_item(first, start);
        }
        // The declarator: a function declarator, its name in no supertype.
        self.ctor_args = u8::from(self.kind_at(1) == T::ColonColon || self.kind_at(1) == T::Lt && self.angle_end(1).is_some_and(|e| self.kind_at(e + 1) == T::ColonColon));
        let d = self.declarator(Fam::Decl, slot(f::DECLARATOR, Ctx::Bare))?;
        self.ctor_args = 0;
        if d.kind != K::FunctionDeclarator {
            return self.refuse();
        }
        let _ = level;
        self.ctor_rest(first, start)
    }

    /// Whether the next tokens are a conversion's name: `operator T`, `A::operator T`.
    fn ctor_is_cast(&mut self) -> bool {
        let mut j = 0;
        loop {
            if self.is_word_at(j, w::OPERATOR) {
                return true;
            }
            if !self.is_ident_at(j) {
                return false;
            }
            let mut e = j + 1;
            if self.kind_at(e) == T::Lt {
                match self.angle_end(e) {
                    Some(g) => e = g + 1,
                    None => return false,
                }
            }
            if self.kind_at(e) != T::ColonColon {
                return false;
            }
            j = e + 1;
        }
    }

    /// After a constructor's declarator: its definition (an initializer list and a body, a `try`,
    /// `= default` …) or `;` (a declaration). The node over it all is put before `first`.
    fn ctor_rest(&mut self, first: usize, start: u32) -> R<()> {
        match self.peek() {
            T::Semi => {
                let n = self.insert_at(first, start, K::Declaration, BARE);
                self.tk(K::Declaration, K::TSemi)?;
                self.close(n);
            }
            _ => {
                let n = self.insert_at(first, start, K::FunctionDefinition, BARE);
                self.method_body(K::FunctionDefinition, true)?;
                self.close(n);
            }
        }
        Ok(())
    }

    /// A definition's body: `{ … }`, `: inits { … }` (where `inits`), `try …`, `= default;`,
    /// `= delete;`, `= 0;`.
    pub(super) fn method_body(&mut self, parent: K, inits: bool) -> R<()> {
        match self.peek() {
            T::Colon if inits => {
                self.field_initializers()?;
                self.compound(slot(f::BODY, Ctx::Bare))?;
            }
            T::LBrace => {
                self.compound(slot(f::BODY, Ctx::Bare))?;
            }
            T::Eq => {
                let t = self.peek_at(1);
                let kind = match t.kind {
                    T::Name if t.word == w::DEFAULT => K::DefaultMethodClause,
                    T::Name if t.word == w::DELETE => K::DeleteMethodClause,
                    T::Number if &self.src[t.start as usize..t.end as usize] == b"0" => K::PureVirtualClause,
                    _ => return self.refuse(),
                };
                let n = self.open(kind, f::NONE, C_NONE);
                self.tk(kind, K::TEq)?;
                match kind {
                    K::PureVirtualClause => {
                        self.bump()?;
                    }
                    _ => self.kw_any(kind)?,
                }
                self.punct_tk(kind, T::Semi, false)?;
                self.close(n);
            }
            T::Name if self.is_word(w::TRY) => {
                let n = self.open(K::TryStatement, f::BODY, C_NONE);
                self.kw(K::TryStatement, w::TRY)?;
                if inits && self.peek() == T::Colon {
                    self.field_initializers()?;
                }
                self.compound(slot(f::BODY, Ctx::Bare))?;
                self.catches()?;
                self.close(n);
            }
            _ => return self.refuse(),
        }
        let _ = parent;
        Ok(())
    }

    /// `: field(args), base{…}, …`.
    fn field_initializers(&mut self) -> R<()> {
        let n = self.open(K::FieldInitializerList, f::NONE, C_NONE);
        self.tk(K::FieldInitializerList, K::TColon)?;
        loop {
            let i = self.open(K::FieldInitializer, f::NONE, C_NONE);
            self.qname(Role::Field, BARE)?;
            match self.peek() {
                T::LParen => {
                    self.arguments(BARE)?;
                }
                T::LBrace => {
                    self.initializer_list(BARE)?;
                }
                _ => return self.refuse(),
            }
            if self.peek() == T::DotDotDot {
                self.tk(K::FieldInitializer, K::TDotDotDot)?;
            }
            self.close(i);
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::FieldInitializerList, K::TComma)?;
        }
        self.close(n);
        Ok(())
    }

    /// `catch (parameter) { … }`, one or more.
    pub(super) fn catches(&mut self) -> R<()> {
        if !self.is_word(w::CATCH) {
            return self.refuse();
        }
        while self.is_word(w::CATCH) {
            let c = self.open(K::CatchClause, f::NONE, C_NONE);
            self.kw(K::CatchClause, w::CATCH)?;
            if self.kind_at(1) == T::DotDotDot && self.kind_at(2) == T::RParen {
                // `catch (...)`: its tokens alone.
                let l = self.open(K::ParameterList, f::PARAMETERS, C_NONE);
                self.leaf(K::TLParen, f::NONE, C_NONE)?;
                self.leaf(K::TDotDotDot, f::NONE, C_NONE)?;
                self.leaf(K::TRParen, f::NONE, C_NONE)?;
                self.close(l);
            } else {
                self.parameter_list()?;
            }
            self.compound(slot(f::BODY, Ctx::Bare))?;
            self.close(c);
        }
        Ok(())
    }

    /// `operator T() …`: a conversion's declaration or definition.
    fn operator_cast_item(&mut self, first: usize, start: u32) -> R<()> {
        self.operator_cast_name(slot(f::DECLARATOR, Ctx::Bare))?;
        match self.peek() {
            T::Semi | T::Eq if !(self.peek() == T::Eq && matches!(self.kind_at(1), T::Name)) => {
                let n = self.insert_at(first, start, K::Declaration, BARE);
                if self.peek() == T::Eq {
                    self.tk(K::OperatorCastDeclaration, K::TEq)?;
                    self.expr(slot(f::DEFAULT_VALUE, Ctx::Expr))?;
                }
                self.punct_tk(K::OperatorCastDeclaration, T::Semi, false)?;
                self.close(n);
            }
            _ => {
                let n = self.insert_at(first, start, K::FunctionDefinition, BARE);
                self.method_body(K::FunctionDefinition, false)?;
                self.close(n);
            }
        }
        Ok(())
    }

    /// `operator T(…) …` or `A::operator T(…) …` (a qualified identifier over it).
    fn operator_cast_name(&mut self, s: Slot) -> R<N> {
        if !self.is_word(w::OPERATOR) {
            let n = self.open_in(K::QualifiedIdentifier, s);
            self.scope()?;
            self.tk(K::QualifiedIdentifier, K::TColonColon)?;
            self.operator_cast_name(slot(f::NAME, Ctx::Bare))?;
            self.close(n);
            return Ok(n);
        }
        let n = self.open_in(K::OperatorCast, s);
        self.kw(K::OperatorCast, w::OPERATOR)?;
        if self.specifiers(K::OperatorCast)?.is_none() {
            return self.refuse();
        }
        // The declarator: an abstract function declarator (`()`, `() const`), maybe after `*`/`&`.
        self.declarator(Fam::Abstract, slot(f::DECLARATOR, Ctx::ADecl))?;
        self.close(n);
        Ok(n)
    }

    // ---- Names.

    /// The end of the template arguments whose `<` is at token `k`: the index of its `>` (or of
    /// the `>>` whose first half closes it). `None` where the brackets do not balance before a
    /// `;`, `{`, `}` or an unmatched closer.
    pub(super) fn angle_end(&mut self, k: usize) -> Option<usize> {
        self.angle_scan(k).map(|(e, _)| e)
    }

    /// `angle_end`, and whether its token is a `>>` closing an inner list too (so what follows the
    /// `>>` follows this list).
    pub(super) fn angle_scan(&mut self, k: usize) -> Option<(usize, bool)> {
        // Open `<`s at each bracket depth: inside brackets, a `>` closes one there or compares.
        let mut angles = [0u16; 16];
        let mut depth = 0usize;
        let mut j = k;
        loop {
            match self.kind_at(j) {
                T::Lt => angles[depth] += 1,
                T::Gt => {
                    if angles[depth] > 0 {
                        angles[depth] -= 1;
                        if depth == 0 && angles[0] == 0 {
                            return Some((j, false));
                        }
                    }
                }
                T::GtGt => {
                    if depth == 0 {
                        match angles[0] {
                            1 => return Some((j, false)),
                            2 => return Some((j, true)),
                            _ => angles[0] -= 2,
                        }
                    } else {
                        angles[depth] = angles[depth].saturating_sub(2);
                    }
                }
                T::LParen | T::LBracket => {
                    depth += 1;
                    if depth == angles.len() {
                        return None;
                    }
                    angles[depth] = 0;
                }
                T::RParen | T::RBracket => {
                    if depth == 0 {
                        return None;
                    }
                    depth -= 1;
                }
                T::Semi | T::LBrace | T::RBrace | T::Eof | T::Directive => return None,
                _ => {}
            }
            j += 1;
        }
    }

    /// Whether `<` at token `k`, after a name in an expression, opens template arguments: they
    /// close, and what follows reads after a template's name (`(`, `::`, `{`, `;`, `,`, `)` …).
    pub(super) fn template_args_ahead(&mut self, k: usize) -> bool {
        let Some(e) = self.angle_end(k) else { return false };
        if self.kind_at(e) == T::GtGt {
            // A `>>` closing this list and an outer one.
            return true;
        }
        // A call's arguments then an operand (`a < b > (c) d`): comparisons.
        if self.kind_at(e + 1) == T::LParen {
            if let Some(p) = self.paren_end(e + 1) {
                if matches!(self.kind_at(p + 1), T::Number | T::Str | T::Char) || self.is_ident_at(p + 1) {
                    return false;
                }
            }
        }
        matches!(
            self.kind_at(e + 1),
            T::LParen | T::ColonColon | T::LBrace | T::Semi | T::Comma | T::RParen | T::RBracket | T::Gt | T::GtGt | T::Eq | T::DotDotDot | T::Colon | T::RBrace | T::Eof | T::EqEq | T::Ne | T::AmpAmp | T::PipePipe | T::Question
        ) || self.kind_at(e + 1) == T::Name && self.no_gt
    }

    /// The token index after a class's name at `k` (`X`, `A::X`, `X<T>`, `::X`). `None` where none
    /// is.
    pub(super) fn class_name_end(&mut self, k: usize) -> Option<usize> {
        let mut j = k;
        if self.kind_at(j) == T::ColonColon {
            j += 1;
        }
        loop {
            if self.is_word_at(j, w::TEMPLATE) {
                j += 1;
            }
            let after_scope = j > k && self.kind_at(j - 1) == T::ColonColon;
            if !(self.is_ident_at(j) || after_scope && self.kind_at(j) == T::Name && { let c = self.peek_at(j).word; is_primitive(c) || super::decl::is_sized_word(c) }) {
                return None;
            }
            j += 1;
            if self.kind_at(j) == T::Lt {
                let (e, both) = self.angle_scan(j)?;
                if self.kind_at(e) == T::GtGt && !both {
                    // The `>>`'s second half closes an outer list: the name ends inside it.
                    return Some(e);
                }
                j = e + 1;
            }
            if self.kind_at(j) != T::ColonColon {
                return Some(j);
            }
            j += 1;
        }
    }

    /// A class's name (`_class_name`): a type's name, in slot `s`.
    pub(super) fn class_name(&mut self, s: Slot) -> R<N> {
        self.qname(Role::Type, s)
    }

    /// Whether a scope's name is ahead (`a::`, `a<T>::`, `::`, `decltype(x)::`).
    pub(super) fn scope_ahead(&mut self) -> bool {
        let t = self.tok();
        match t.kind {
            T::ColonColon => true,
            T::Name if t.word == w::DECLTYPE => self.paren_end(1).is_some_and(|e| self.kind_at(e + 1) == T::ColonColon),
            T::Name if t.word == w::TEMPLATE => self.is_ident_at(1) && self.kind_at(2) == T::Lt && self.angle_end(2).is_some_and(|e| self.kind_at(e + 1) == T::ColonColon),
            T::Name if !self.reserved(t.word) => match self.kind_at(1) {
                T::ColonColon => true,
                T::Lt => self.angle_scan(1).is_some_and(|(e, both)| (self.kind_at(e) == T::Gt || both) && self.kind_at(e + 1) == T::ColonColon),
                _ => false,
            },
            _ => false,
        }
    }

    /// A scope (before its `::`): `a` (a namespace's), `a<T>` (a template type), `decltype(x)`,
    /// `template a<T>` (a dependent name).
    fn scope(&mut self) -> R<N> {
        let t = self.tok();
        let s = slot(f::SCOPE, Ctx::Bare);
        if t.kind == T::Name && t.word == w::DECLTYPE {
            return self.decltype(s);
        }
        if t.kind == T::Name && t.word == w::TEMPLATE {
            let n = self.open_in(K::DependentName, s);
            self.kw(K::DependentName, w::TEMPLATE)?;
            self.template_type(BARE)?;
            self.close(n);
            return Ok(n);
        }
        if self.kind_at(1) == T::Lt {
            return self.template_type(s);
        }
        if !self.is_ident() {
            return self.refuse();
        }
        self.leaf_in(K::NamespaceIdentifier, s)
    }

    /// A name in role `role` in slot `s`: qualified (`a::b::c`, right-nested) or not.
    pub(super) fn qname(&mut self, role: Role, s: Slot) -> R<N> {
        self.deeper()?;
        let r = self.qname_inner(role, s);
        self.depth -= 1;
        r
    }

    fn qname_inner(&mut self, role: Role, s: Slot) -> R<N> {
        if self.scope_ahead() {
            let n = self.open_in(K::QualifiedIdentifier, s);
            if self.peek() != T::ColonColon {
                self.scope()?;
            }
            self.tk(K::QualifiedIdentifier, K::TColonColon)?;
            if self.is_word(w::TEMPLATE) && role != Role::Type {
                // `a::template b<T>`: a dependent name.
                let d = self.open(K::DependentName, f::NAME, C_NONE);
                self.kw(K::DependentName, w::TEMPLATE)?;
                self.simple_name(role, BARE)?;
                self.close(d);
            } else if role == Role::Decl && self.peek() == T::Star {
                // `A::*p`: a pointer to a member, its name a pointer type declarator.
                let p = self.open(K::PointerTypeDeclarator, f::NAME, C_NONE);
                self.tk(K::PointerTypeDeclarator, K::TStar)?;
                if !self.is_ident() {
                    return self.refuse();
                }
                self.leaf(K::TypeIdentifier, f::DECLARATOR, class(K::TypeIdentifier, Ctx::TDecl))?;
                self.close(p);
            } else {
                self.qname(role, slot(f::NAME, Ctx::Bare))?;
            }
            self.close(n);
            return Ok(n);
        }
        self.simple_name(role, s)
    }

    /// An unqualified name in role `role`.
    fn simple_name(&mut self, role: Role, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Tilde if role != Role::Type && self.is_ident_at(1) => {
                let n = self.open_in(K::DestructorName, s);
                self.tk(K::DestructorName, K::TTilde)?;
                self.leaf(K::Identifier, f::NONE, C_NONE)?;
                self.close(n);
                Ok(n)
            }
            T::Name if t.word == w::OPERATOR && role != Role::Type => {
                let n = self.operator_name(s)?;
                if role == Role::Field && self.peek() == T::Lt && self.template_args_ahead(0) {
                    let m = self.wrap(n, K::TemplateMethod, s, slot(f::NAME, Ctx::Bare));
                    self.template_args(slot(f::ARGUMENTS, Ctx::Bare))?;
                    self.close(m);
                    return Ok(m);
                }
                Ok(n)
            }
            T::Name if !self.reserved(t.word) || role == Role::Field || is_primitive(t.word) || super::decl::is_sized_word(t.word) => {
                let templ = self.kind_at(1) == T::Lt && (role == Role::Type && self.angle_end(1).is_some() || self.template_args_ahead(1));
                let (leaf, kind) = match role {
                    Role::Expr | Role::Decl => (K::Identifier, K::TemplateFunction),
                    Role::Type => (K::TypeIdentifier, K::TemplateType),
                    Role::Field => (K::FieldIdentifier, K::TemplateMethod),
                };
                if !templ {
                    return self.leaf_in(leaf, s);
                }
                let n = self.open_in(kind, s);
                self.leaf(leaf, f::NAME, C_NONE)?;
                self.template_args(slot(f::ARGUMENTS, Ctx::Bare))?;
                self.close(n);
                Ok(n)
            }
            _ => self.refuse(),
        }
    }

    /// `name<arguments>`: a `template_type` in slot `s`.
    fn template_type(&mut self, s: Slot) -> R<N> {
        if !self.is_ident() || self.kind_at(1) != T::Lt {
            return self.refuse();
        }
        let n = self.open_in(K::TemplateType, s);
        self.leaf(K::TypeIdentifier, f::NAME, C_NONE)?;
        self.template_args(slot(f::ARGUMENTS, Ctx::Bare))?;
        self.close(n);
        Ok(n)
    }

    /// `operator` and its operator: `operator+`, `operator()`, `operator new[]`, `operator""_x`.
    fn operator_name(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::OperatorName, s);
        self.kw(K::OperatorName, w::OPERATOR)?;
        let t = self.tok();
        match t.kind {
            T::LParen if self.kind_at(1) == T::RParen => {
                self.bump()?;
                self.bump()?;
                let p = self.w.push(key(K::TParens, f::NONE, C_NONE, false), t.start);
                self.w.close(p, self.last_end);
            }
            T::LBracket if self.kind_at(1) == T::RBracket => {
                self.bump()?;
                self.bump()?;
                let p = self.w.push(key(K::TBrackets, f::NONE, C_NONE, false), t.start);
                self.w.close(p, self.last_end);
            }
            T::Str if t.end - t.start == 2 => {
                self.leaf(K::TEmptyString, f::NONE, C_NONE)?;
                if !self.is_ident() {
                    return self.refuse();
                }
                self.leaf(K::Identifier, f::NONE, C_NONE)?;
            }
            T::Name if matches!(t.word, w::NEW | w::DELETE) => {
                self.kw_any(K::OperatorName)?;
                if self.peek() == T::LBracket && self.kind_at(1) == T::RBracket {
                    let a = self.tok();
                    self.bump()?;
                    self.bump()?;
                    let p = self.w.push(key(K::TBrackets, f::NONE, C_NONE, false), a.start);
                    self.w.close(p, self.last_end);
                }
            }
            T::Name if t.word == w::CO_AWAIT => self.kw(K::OperatorName, w::CO_AWAIT)?,
            T::Name => match alt_operator(t.word) {
                Some(k) => {
                    self.leaf(k, f::NONE, C_NONE)?;
                }
                None => return self.refuse(),
            },
            _ => match punct(t.kind) {
                Some(k) if !matches!(t.kind, T::LParen | T::RParen | T::LBracket | T::RBracket | T::LBrace | T::RBrace | T::Semi | T::Colon | T::ColonColon | T::Question | T::Dot | T::DotStar | T::DotDotDot) => {
                    self.leaf(k, f::NONE, C_NONE)?;
                }
                _ => return self.refuse(),
            },
        }
        self.close(n);
        Ok(n)
    }

    /// `<` template arguments `>` in slot `s`: each a type where one reads (tree-sitter's dynamic
    /// precedence), else an expression; `...` after one.
    pub(super) fn template_args(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::TemplateArgumentList, s);
        self.tk(K::TemplateArgumentList, K::TLt)?;
        let no_gt = std::mem::replace(&mut self.no_gt, true);
        // The first parameter's type's arguments of an out-of-line constructor (`ctor_args`).
        let ctor = std::mem::replace(&mut self.ctor_args, 0) == 2;
        if !self.at_angle_close() {
            loop {
                if ctor && self.peek() == T::Name && { let c = self.tok().word; !self.reserved(c) } && matches!(self.kind_at(1), T::Gt | T::GtGt | T::Comma | T::Lt) {
                    self.ctor_args = 3;
                }
                self.template_arg()?;
                self.ctor_args = 0;
                if self.peek() == T::Comma {
                    self.tk(K::TemplateArgumentList, K::TComma)?;
                    continue;
                }
                break;
            }
        }
        self.no_gt = no_gt;
        self.angle_close(K::TemplateArgumentList)?;
        self.close(n);
        Ok(n)
    }

    /// One template argument: a type where its tokens read as one to the next `,` or `>`, else an
    /// expression.
    fn template_arg(&mut self) -> R<()> {
        if std::mem::replace(&mut self.ctor_args, 0) == 3 {
            // An out-of-line constructor's first parameter's name argument: an expression.
            let m = self.mark();
            match self.expr(EXPR) {
                Ok(_) if matches!(self.peek(), T::Comma | T::Gt | T::GtGt) => return Ok(()),
                _ => self.reset(m),
            }
        }
        let m = self.mark();
        let typed = self.type_arg_ahead() && match self.type_descriptor(BARE) {
            Ok(_) => matches!(self.peek(), T::Comma | T::Gt | T::GtGt | T::GtGtEq | T::Ge | T::DotDotDot),
            Err(_) => false,
        };
        if typed {
            if self.peek() == T::DotDotDot {
                // `Ts...`: a pack expansion of the type.
                let t = N { at: m.n_at(), kind: K::TypeDescriptor };
                let p = self.wrap(t, K::ParameterPackExpansion, BARE, slot(f::PATTERN, Ctx::Bare));
                self.tk(K::ParameterPackExpansion, K::TDotDotDot)?;
                self.close(p);
            }
            return Ok(());
        }
        self.reset(m);
        let e = self.expr(EXPR)?;
        if self.peek() == T::DotDotDot {
            let p = self.wrap(e, K::ParameterPackExpansion, EXPR, slot(f::PATTERN, Ctx::Expr));
            self.tk(K::ParameterPackExpansion, K::TDotDotDot)?;
            self.close(p);
        }
        Ok(())
    }

    /// Whether a template argument may be a type: it starts with a type's word or a name.
    fn type_arg_ahead(&mut self) -> bool {
        let t = self.tok();
        match t.kind {
            T::ColonColon => true,
            T::Name if matches!(t.word, w::TRUE | w::FALSE | w::NULLPTR | w::THIS) => false,
            T::Name => !self.reserved(t.word) || {
                let c = t.word;
                is_primitive(c) || matches!(c, w::CONST | w::VOLATILE | w::SIGNED | w::UNSIGNED | w::LONG | w::SHORT | w::STRUCT | w::CLASS | w::UNION | w::ENUM | w::TYPENAME | w::DECLTYPE | w::AUTO)
            },
            _ => false,
        }
    }

    // ---- Types.

    /// `decltype(expression)` or `decltype(auto)`.
    pub(super) fn decltype(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::Decltype, s);
        self.kw(K::Decltype, w::DECLTYPE)?;
        self.punct_tk(K::Decltype, T::LParen, false)?;
        if self.is_word(w::AUTO) && self.kind_at(1) == T::RParen {
            self.leaf(K::Auto, f::NONE, C_NONE)?;
        } else {
            let no_gt = std::mem::replace(&mut self.no_gt, false);
            self.expr(EXPR)?;
            self.no_gt = no_gt;
        }
        self.punct_tk(K::Decltype, T::RParen, false)?;
        self.close(n);
        Ok(n)
    }

    /// C++'s type specifiers: `auto`, `decltype(…)`, `typename T::x`, a qualified or template
    /// type, `Concept auto`. Whether one was read.
    pub(super) fn cpp_type(&mut self, s: Slot) -> R<bool> {
        let t = self.tok();
        match t.kind {
            T::Name if t.word == w::AUTO => {
                let n = self.open_in(K::PlaceholderTypeSpecifier, s);
                self.leaf(K::Auto, f::NONE, C_NONE)?;
                self.close(n);
                Ok(true)
            }
            T::Name if t.word == w::DECLTYPE => {
                if self.is_word_at(2, w::AUTO) && self.kind_at(3) == T::RParen {
                    let n = self.open_in(K::PlaceholderTypeSpecifier, s);
                    self.decltype(BARE)?;
                    self.close(n);
                    return Ok(true);
                }
                if self.scope_ahead() {
                    self.qname(Role::Type, s)?;
                } else {
                    self.decltype(s)?;
                }
                Ok(true)
            }
            T::Name if t.word == w::TYPENAME => {
                let n = self.open_in(K::DependentType, s);
                self.kw(K::DependentType, w::TYPENAME)?;
                if !self.type_specifier(BARE.with_ctx(Ctx::Type))? {
                    return self.refuse();
                }
                self.close(n);
                Ok(true)
            }
            T::ColonColon => {
                self.qname(Role::Type, s)?;
                Ok(true)
            }
            T::Name if !self.reserved(t.word) && (self.scope_ahead() || self.kind_at(1) == T::Lt) => {
                let q = self.qname(Role::Type, s)?;
                // `Concept auto`: a constrained placeholder.
                if self.is_word(w::AUTO) {
                    let n = self.wrap(q, K::PlaceholderTypeSpecifier, s, slot(f::CONSTRAINT, Ctx::Type));
                    self.leaf(K::Auto, f::NONE, C_NONE)?;
                    self.close(n);
                    return Ok(true);
                }
                Ok(true)
            }
            T::Name if !self.reserved(t.word) && self.is_word_at(1, w::AUTO) => {
                let q = self.leaf_in(K::TypeIdentifier, slot(f::CONSTRAINT, Ctx::Type))?;
                let n = self.wrap(q, K::PlaceholderTypeSpecifier, s, slot(f::CONSTRAINT, Ctx::Type));
                self.leaf(K::Auto, f::NONE, C_NONE)?;
                self.close(n);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// `: base, virtual public other<T>…`.
    pub(super) fn base_classes(&mut self) -> R<()> {
        let n = self.open(K::BaseClassClause, f::NONE, C_NONE);
        self.tk(K::BaseClassClause, K::TColon)?;
        loop {
            while self.peek() == T::LBracket && self.kind_at(1) == T::LBracket {
                self.attribute_declaration()?;
            }
            loop {
                let t = self.tok();
                if t.kind != T::Name {
                    break;
                }
                match t.word {
                    w::PUBLIC | w::PRIVATE | w::PROTECTED => {
                        let a = self.open(K::AccessSpecifier, f::NONE, C_NONE);
                        self.kw_any(K::AccessSpecifier)?;
                        self.close(a);
                    }
                    w::VIRTUAL => {
                        self.kw(K::BaseClassClause, w::VIRTUAL)?;
                    }
                    _ => break,
                }
            }
            self.class_name(BARE)?;
            if self.peek() == T::DotDotDot {
                self.tk(K::BaseClassClause, K::TDotDotDot)?;
            }
            if self.peek() != T::Comma {
                break;
            }
            // A `,` before `virtual` is stored (its keyword is a bare token there).
            let keep = self.is_word_at(1, w::VIRTUAL);
            self.punct_tk(K::BaseClassClause, T::Comma, keep)?;
        }
        self.close(n);
        Ok(())
    }

    // ---- Declarators.

    /// A function declarator's parts after its parameters: qualifiers, a ref-qualifier,
    /// `noexcept`/`throw(…)`, attributes, a trailing return type, `override`/`final`, `requires`.
    pub(super) fn function_tail(&mut self, parent: K) -> R<()> {
        loop {
            let t = self.tok();
            match t.kind {
                T::Name => match t.word {
                    w::CONST | w::VOLATILE | w::RESTRICT | w::U_RESTRICT_U | w::MUTABLE | w::CONSTEXPR | w::CONSTEVAL => {
                        let n = self.open(K::TypeQualifier, f::NONE, C_NONE);
                        self.kw_any(K::TypeQualifier)?;
                        self.close(n);
                    }
                    w::NOEXCEPT => {
                        let n = self.open(K::Noexcept, f::NONE, C_NONE);
                        self.kw(K::Noexcept, w::NOEXCEPT)?;
                        if self.peek() == T::LParen {
                            self.tk(K::Noexcept, K::TLParen)?;
                            if self.peek() != T::RParen {
                                let no_gt = std::mem::replace(&mut self.no_gt, false);
                                self.expr(EXPR)?;
                                self.no_gt = no_gt;
                            }
                            self.punct_tk(K::Noexcept, T::RParen, false)?;
                        }
                        self.close(n);
                    }
                    w::THROW => {
                        let n = self.open(K::ThrowSpecifier, f::NONE, C_NONE);
                        self.kw(K::ThrowSpecifier, w::THROW)?;
                        self.punct_tk(K::ThrowSpecifier, T::LParen, false)?;
                        while self.peek() != T::RParen {
                            self.type_descriptor(BARE)?;
                            if self.peek() != T::Comma {
                                break;
                            }
                            self.tk(K::ThrowSpecifier, K::TComma)?;
                        }
                        self.punct_tk(K::ThrowSpecifier, T::RParen, false)?;
                        self.close(n);
                    }
                    w::OVERRIDE | w::FINAL => {
                        let n = self.open(K::VirtualSpecifier, f::NONE, C_NONE);
                        self.kw_any(K::VirtualSpecifier)?;
                        self.close(n);
                    }
                    w::REQUIRES => {
                        self.requires_clause(BARE)?;
                    }
                    w::U_ATTRIBUTE_U | w::U_ATTRIBUTE => self.attribute_specifier()?,
                    _ => return Ok(()),
                },
                T::Amp | T::AmpAmp => {
                    let n = self.open(K::RefQualifier, f::NONE, C_NONE);
                    let k = punct(t.kind).expect("an operator");
                    self.tk(K::RefQualifier, k)?;
                    self.close(n);
                }
                T::Arrow => self.trailing_return()?,
                T::LBracket if self.kind_at(1) == T::LBracket => self.attribute_declaration()?,
                _ => {
                    let _ = parent;
                    return Ok(());
                }
            }
        }
    }

    /// `-> type`.
    fn trailing_return(&mut self) -> R<()> {
        let n = self.open(K::TrailingReturnType, f::NONE, C_NONE);
        self.tk(K::TrailingReturnType, K::TArrow)?;
        self.type_descriptor(BARE)?;
        self.close(n);
        Ok(())
    }

    // ---- Expressions.

    /// `new (placement) T[n](args)`, `::new T{…}`.
    pub(super) fn new_expression(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::NewExpression, s);
        if self.peek() == T::ColonColon {
            self.tk(K::NewExpression, K::TColonColon)?;
        }
        self.kw(K::NewExpression, w::NEW)?;
        if self.peek() == T::LParen {
            self.arguments(slot(f::PLACEMENT, Ctx::Bare))?;
        }
        if self.peek() == T::LParen {
            // `new (T)`: a parenthesized type.
            return self.refuse();
        }
        while self.tok().kind == T::Name && matches!(self.tok().word, w::CONST | w::VOLATILE) {
            return self.refuse();
        }
        if !self.type_specifier(slot(f::TYPE, Ctx::Type))? {
            return self.refuse();
        }
        if self.peek() == T::LBracket {
            self.new_declarator(slot(f::DECLARATOR, Ctx::Bare))?;
        }
        match self.peek() {
            T::LParen => {
                self.arguments(slot(f::ARGUMENTS, Ctx::Bare))?;
            }
            T::LBrace => {
                self.initializer_list(slot(f::ARGUMENTS, Ctx::Bare))?;
            }
            _ => {}
        }
        self.close(n);
        Ok(n)
    }

    fn new_declarator(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::NewDeclarator, s);
        self.tk(K::NewDeclarator, K::TLBracket)?;
        let no_gt = std::mem::replace(&mut self.no_gt, false);
        self.expr(slot(f::LENGTH, Ctx::Expr))?;
        self.no_gt = no_gt;
        self.punct_tk(K::NewDeclarator, T::RBracket, false)?;
        if self.peek() == T::LBracket {
            self.new_declarator(BARE)?;
        }
        self.close(n);
        Ok(n)
    }

    /// `delete p`, `delete[] p`, `::delete p`.
    pub(super) fn delete_expression(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::DeleteExpression, s);
        if self.peek() == T::ColonColon {
            self.tk(K::DeleteExpression, K::TColonColon)?;
        }
        self.kw(K::DeleteExpression, w::DELETE)?;
        if self.peek() == T::LBracket && self.kind_at(1) == T::RBracket {
            self.leaf(K::TLBracket, f::NONE, C_NONE)?;
            self.leaf(K::TRBracket, f::NONE, C_NONE)?;
        }
        self.unary(EXPR)?;
        self.close(n);
        Ok(n)
    }

    /// `[captures] <T> (params) specifiers -> R { body }`.
    pub(super) fn lambda(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::LambdaExpression, s);
        let c = self.open(K::LambdaCaptureSpecifier, f::CAPTURES, C_NONE);
        // `[` is stored before a bare token (`[&x]`, `[*this]`), not before a default capture.
        // `&x = e`: an init capture, its `&` no bare token.
        let init = |p: &mut Self| p.kind_at(1) == T::Amp && p.is_ident_at(2) && p.kind_at(3) == T::Eq;
        let bare = matches!(self.kind_at(1), T::Amp | T::Star) && !matches!(self.kind_at(2), T::Comma | T::RBracket) && !init(self);
        if bare {
            self.leaf(K::TLBracket, f::NONE, C_NONE)?;
        } else {
            self.tk(K::LambdaCaptureSpecifier, K::TLBracket)?;
        }
        let no_gt = std::mem::replace(&mut self.no_gt, false);
        let mut first = true;
        while self.peek() != T::RBracket {
            if !first {
                // A `,` before a bare `&` is stored.
                let keep = self.kind_at(1) == T::Amp && !init(self);
                self.punct_tk(K::LambdaCaptureSpecifier, T::Comma, keep)?;
            }
            let t = self.tok();
            if first && matches!(t.kind, T::Eq | T::Amp) && matches!(self.kind_at(1), T::Comma | T::RBracket) {
                let d = self.open(K::LambdaDefaultCapture, f::NONE, C_NONE);
                self.leaf(punct(t.kind).expect("a capture"), f::NONE, C_NONE)?;
                self.close(d);
            } else {
                self.capture()?;
            }
            first = false;
        }
        self.tk(K::LambdaCaptureSpecifier, K::TRBracket)?;
        self.close(c);
        self.no_gt = no_gt;
        if self.peek() == T::Lt {
            self.template_parameters(slot(f::TEMPLATE_PARAMETERS, Ctx::Bare))?;
            if self.is_word(w::REQUIRES) {
                self.requires_clause(slot(f::CONSTRAINT, Ctx::Bare))?;
            }
        }
        if self.peek() != T::LBrace {
            let d = self.open(K::AbstractFunctionDeclarator, f::DECLARATOR, C_NONE);
            if self.peek() == T::LParen {
                self.parameter_list()?;
            } else if self.peek() == T::Arrow {
                // `[&] -> T {…}`: tree-sitter-cpp's error (a trailing return needs parameters).
                return self.refuse();
            }
            self.function_tail(K::AbstractFunctionDeclarator)?;
            self.close(d);
        }
        let top = std::mem::replace(&mut self.top, true);
        self.compound(slot(f::BODY, Ctx::Bare))?;
        self.top = top;
        self.close(n);
        Ok(n)
    }

    fn capture(&mut self) -> R<()> {
        let t = self.tok();
        match t.kind {
            T::Star if self.is_word_at(1, w::THIS) => {
                self.leaf(K::TStar, f::NONE, C_NONE)?;
                self.leaf(K::This, f::NONE, C_NONE)?;
            }
            T::Name if t.word == w::THIS => {
                self.leaf(K::This, f::NONE, C_NONE)?;
            }
            _ => {
                let k = usize::from(t.kind == T::Amp);
                let k2 = k + usize::from(self.kind_at(k) == T::DotDotDot);
                if self.is_ident_at(k2) && self.kind_at(k2 + 1) == T::Eq {
                    let n = self.open(K::LambdaCaptureInitializer, f::NONE, C_NONE);
                    if k == 1 {
                        self.leaf(K::TAmp, f::NONE, C_NONE)?;
                    }
                    if k2 > k {
                        self.leaf(K::TDotDotDot, f::NONE, C_NONE)?;
                    }
                    self.leaf(K::Identifier, f::LEFT, C_NONE)?;
                    self.tk(K::LambdaCaptureInitializer, K::TEq)?;
                    self.expr(slot(f::RIGHT, Ctx::Expr))?;
                    self.close(n);
                    return Ok(());
                }
                if k == 1 {
                    self.leaf(K::TAmp, f::NONE, C_NONE)?;
                }
                let q = self.qname(Role::Expr, BARE)?;
                if self.peek() == T::DotDotDot {
                    let p = self.wrap(q, K::ParameterPackExpansion, BARE, slot(f::PATTERN, Ctx::Bare));
                    self.tk(K::ParameterPackExpansion, K::TDotDotDot)?;
                    self.close(p);
                }
            }
        }
        Ok(())
    }

    /// A field expression's field: a name, `template f<T>`, `~X`, `A::b`, `operator()`.
    pub(super) fn field_name(&mut self, s: Slot) -> R<N> {
        // `a.operator*()`: tree-sitter-cpp's error (an operator's name is a field's only in a
        // `template_method`, with template arguments: `a.operator()<T>()`).
        if self.is_word(w::OPERATOR) {
            let n = self.qname(Role::Field, s)?;
            if n.kind != K::TemplateMethod {
                return self.refuse();
            }
            return Ok(n);
        }
        if self.is_word(w::TEMPLATE) {
            let n = self.open_in(K::DependentName, s);
            self.kw(K::DependentName, w::TEMPLATE)?;
            self.simple_name(Role::Field, BARE)?;
            self.close(n);
            return Ok(n);
        }
        if self.peek() == T::Tilde {
            return self.simple_name(Role::Expr, s);
        }
        // `p->A::~A()`: tree-sitter-cpp's error (a field's qualified name ends in no destructor).
        if self.peek() == T::Name && self.kind_at(1) == T::ColonColon && self.kind_at(2) == T::Tilde {
            return self.refuse();
        }
        if self.peek() == T::Name && { let c = self.tok().word; self.reserved(c) } && !self.is_word(w::OPERATOR) {
            return self.leaf_in(K::FieldIdentifier, s);
        }
        self.qname(Role::Field, s)
    }

    /// C++'s primary expressions: `this`, `new`, `delete`, lambdas, qualified and template names,
    /// `T{…}`, `int(x)`, raw strings, user-defined literals. `None` where C's reading applies.
    pub(super) fn cpp_primary(&mut self, s: Slot) -> R<Option<N>> {
        let t = self.tok();
        if t.kind == T::Name && self.kind_at(1) == T::Str && !self.reserved(t.word) {
            return Ok(None);
        }
        let adjacent_suffix = |p: &mut Self| {
            let u = p.peek_at(1);
            u.kind == T::Name && u.start == t.end
        };
        Ok(Some(match t.kind {
            T::LBracket => self.lambda(s)?,
            T::ColonColon if self.is_word_at(1, w::NEW) => self.new_expression(s)?,
            T::ColonColon if self.is_word_at(1, w::DELETE) => self.delete_expression(s)?,
            T::ColonColon => self.name_expr(s)?,
            T::RawStr | T::Str => {
                // Strings, and a suffix after the last: a user-defined literal over them all.
                let n = self.strings(BARE)?;
                let u = self.tok();
                if u.kind == T::Name && u.start == self.last_end {
                    let l = self.wrap(n, K::UserDefinedLiteral, s, BARE);
                    self.leaf(K::LiteralSuffix, f::NONE, C_NONE)?;
                    self.close(l);
                    l
                } else {
                    self.rekey(n.at, s);
                    n
                }
            }
            T::Number | T::Char if adjacent_suffix(self) => self.user_literal(s)?,
            T::Name => match t.word {
                w::THIS => self.leaf_in(K::This, s)?,
                // `static_assert(…)` where no declaration is: a call.
                w::STATIC_ASSERT => self.leaf_in(K::Identifier, s)?,
                w::NEW => self.new_expression(s)?,
                w::DELETE => self.delete_expression(s)?,
                w::OPERATOR => self.qname(Role::Expr, s)?,
                w::REQUIRES => self.requires_expression(s)?,
                c if is_primitive(c) && matches!(self.kind_at(1), T::LParen | T::LBrace) => {
                    if self.kind_at(1) == T::LParen {
                        let n = self.open_in(K::CallExpression, s);
                        self.leaf(K::PrimitiveType, f::FUNCTION, C_NONE)?;
                        self.arguments(slot(f::ARGUMENTS, Ctx::Bare))?;
                        self.close(n);
                        n
                    } else {
                        let n = self.open_in(K::CompoundLiteralExpression, s);
                        self.leaf(K::PrimitiveType, f::TYPE, C_NONE)?;
                        self.initializer_list(slot(f::VALUE, Ctx::Bare))?;
                        self.close(n);
                        n
                    }
                }
                w::TRUE | w::FALSE | w::TRUE_C | w::FALSE_C | w::NULL | w::NULLPTR => return Ok(None),
                c if !self.reserved(c) || c == w::DECLTYPE => self.name_expr(s)?,
                _ => return Ok(None),
            },
            _ => return Ok(None),
        }))
    }

    /// A name in an expression: `T{…}` (a compound literal of a class), or a (qualified,
    /// template) name.
    fn name_expr(&mut self, s: Slot) -> R<N> {
        if let Some(e) = self.class_name_end(0) {
            if self.kind_at(e) == T::LBrace && !self.no_brace_literal() {
                let n = self.open_in(K::CompoundLiteralExpression, s);
                self.class_name(slot(f::TYPE, Ctx::Bare))?;
                self.initializer_list(slot(f::VALUE, Ctx::Bare))?;
                self.close(n);
                return Ok(n);
            }
        }
        if self.is_word(w::DECLTYPE) && !self.scope_ahead() {
            return self.decltype(s);
        }
        self.qname(Role::Expr, s)
    }

    /// Where a `{` after a name is no compound literal's: never, yet (a hook for the statements
    /// whose body follows an expression).
    fn no_brace_literal(&mut self) -> bool {
        false
    }

    /// A literal and its suffix: `10_km`, `"x"s`.
    fn user_literal(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::UserDefinedLiteral, s);
        match self.peek() {
            T::Number => {
                self.leaf(K::NumberLiteral, f::NONE, C_NONE)?;
            }
            T::Char => {
                self.char_literal(BARE)?;
            }
            T::RawStr => {
                self.raw_string(BARE)?;
            }
            _ => {
                self.string(BARE)?;
            }
        }
        self.leaf(K::LiteralSuffix, f::NONE, C_NONE)?;
        self.close(n);
        Ok(n)
    }

    /// `R"delim(content)delim"`: its opener, delimiter, `(`, content, `)`, delimiter, `"`.
    pub(super) fn raw_string(&mut self, s: Slot) -> R<N> {
        let t = self.bump()?;
        let (a, b) = (t.start as usize, t.end as usize);
        let n = self.w.push(key(K::RawStringLiteral, s.field, class(K::RawStringLiteral, s.ctx), false), t.start);
        let q = a + self.src[a..b].iter().position(|&c| c == b'"').expect("a quote");
        let opener = match &self.src[a..=q] {
            b"R\"" => K::TRDQuote,
            b"LR\"" => K::TLRDQuote,
            b"uR\"" => K::TUSmallRDQuote,
            b"UR\"" => K::TURDQuote,
            b"u8R\"" => K::TU8RDQuote,
            _ => return self.refuse(),
        };
        let leaf = |p: &mut Self, kind: K, field: u8, x: usize, y: usize| {
            let m = p.w.push(key(kind, field, C_NONE, false), x as u32);
            p.w.close(m, y as u32);
        };
        leaf(self, opener, f::NONE, a, q + 1);
        let open = q + 1 + self.src[q + 1..b].iter().position(|&c| c == b'(').ok_or(Refusal::Lex(t.start))?;
        let delim = open - (q + 1);
        // The expected tapes: `R"`, `(`, the content (no delimiter); or `R"`, the delimiter
        // (`delimiter:`), the content, the delimiter again.
        let close = b - 2 - delim;
        if delim > 0 {
            leaf(self, K::RawStringDelimiter, f::DELIMITER, q + 1, open);
        } else {
            leaf(self, K::TLParen, f::NONE, open, open + 1);
        }
        leaf(self, K::RawStringContent, f::NONE, open + 1, close);
        if delim > 0 {
            leaf(self, K::RawStringDelimiter, f::NONE, close + 1, b - 1);
        }
        self.w.close(n, t.end);
        Ok(N { at: n, kind: K::RawStringLiteral })
    }

    // ---- Statements.

    /// `( init; value )`: an `if`'s, `while`'s or `switch`'s condition.
    pub(super) fn condition_clause(&mut self) -> R<N> {
        let n = self.open(K::ConditionClause, f::CONDITION, C_NONE);
        self.punct_tk(K::ConditionClause, T::LParen, false)?;
        let no_gt = std::mem::replace(&mut self.no_gt, false);
        // An init statement: a declaration or an expression statement, then the value.
        if self.semi_in_parens() {
            self.init_statement()?;
        }
        if self.condition_declaration_ahead() {
            let d = self.open(K::Declaration, f::VALUE, C_NONE);
            if self.specifiers(K::Declaration)?.is_none() {
                return self.refuse();
            }
            self.declarator(Fam::Decl, slot(f::DECLARATOR, Ctx::Decl))?;
            if self.peek() == T::Eq {
                self.tk(K::ConditionDeclaration, K::TEq)?;
                self.expr(slot(f::VALUE, Ctx::Expr))?;
            } else {
                self.initializer_list(slot(f::VALUE, Ctx::Bare))?;
            }
            self.close(d);
        } else {
            self.comma_expr(slot(f::VALUE, Ctx::Expr))?;
        }
        self.no_gt = no_gt;
        self.punct_tk(K::ConditionClause, T::RParen, false)?;
        self.close(n);
        Ok(n)
    }

    /// An `init_statement` (`initializer:`): a declaration or an expression statement, with its `;`.
    fn init_statement(&mut self) -> R<()> {
        let i = self.open(K::InitStatement, f::INITIALIZER, C_NONE);
        if self.is_word(w::USING) {
            self.using()?;
        } else if self.is_word(w::TYPEDEF) {
            self.declaration_item(Level::Block, K::InitStatement)?;
        } else if self.declaration_ahead() {
            self.declaration_in(BARE)?;
        } else {
            let e = self.open(K::ExpressionStatement, f::NONE, C_NONE);
            if self.peek() != T::Semi {
                self.comma_expr(EXPR)?;
            }
            self.punct_tk(K::ExpressionStatement, T::Semi, false)?;
            self.close(e);
        }
        self.close(i);
        Ok(())
    }

    /// Whether the braces at the next token hold a `;` at their depth: a block's.
    pub(super) fn semi_in_braces(&mut self) -> bool {
        let mut depth = 0usize;
        let mut j = 1;
        loop {
            match self.kind_at(j) {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                T::Semi if depth == 0 => return true,
                T::Eof => return false,
                _ => {}
            }
            j += 1;
        }
    }

    /// Whether the parentheses ahead (the next token is their `(`… already passed) hold a `;` at
    /// their depth: an init statement.
    fn semi_in_parens(&mut self) -> bool {
        let mut depth = 0usize;
        let mut j = 0;
        loop {
            match self.kind_at(j) {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                T::Semi if depth == 0 => return true,
                T::Eof => return false,
                _ => {}
            }
            j += 1;
        }
    }

    /// Whether a condition's value is a declaration: `T x = …`, `auto x{…}`.
    fn condition_declaration_ahead(&mut self) -> bool {
        if !self.declaration_ahead() {
            return false;
        }
        // An initializer: `=` or `{` at the parentheses' depth.
        let mut depth = 0usize;
        let mut j = 0;
        loop {
            match self.kind_at(j) {
                T::LParen | T::LBracket => depth += 1,
                T::RParen | T::RBracket => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                T::Eq | T::LBrace if depth == 0 => return true,
                T::Semi | T::Eof => return false,
                _ => {}
            }
            j += 1;
        }
    }

    /// `for (T x : range)`. Whether one was read (the `for` and `(` read by then).
    pub(super) fn range_for_ahead(&mut self) -> bool {
        // A `:` at the parentheses' depth before their `)` and with no `;`.
        let mut depth = 0usize;
        let mut j = 2;
        loop {
            match self.kind_at(j) {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                T::Semi if depth == 0 => return false,
                T::Colon if depth == 0 => return true,
                T::Eof => return false,
                _ => {}
            }
            j += 1;
        }
    }

    pub(super) fn range_for(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ForRangeLoop, s);
        self.kw(K::ForRangeLoop, w::FOR)?;
        self.punct_tk(K::ForRangeLoop, T::LParen, false)?;
        if self.semi_in_parens() {
            self.init_statement()?;
        }
        if self.specifiers(K::ForRangeLoop)?.is_none() {
            return self.refuse();
        }
        self.declarator(Fam::Decl, slot(f::DECLARATOR, Ctx::Decl))?;
        self.punct_tk(K::ForRangeLoop, T::Colon, false)?;
        if self.peek() == T::LBrace {
            self.initializer_list(slot(f::RIGHT, Ctx::Bare))?;
        } else {
            self.expr(slot(f::RIGHT, Ctx::Expr))?;
        }
        self.punct_tk(K::ForRangeLoop, T::RParen, false)?;
        self.statement(slot(f::BODY, Ctx::Stmt), false)?;
        self.close(n);
        Ok(n)
    }

    /// A C++ statement at the next token (`try`, `throw`, `co_return`, `co_yield`). `None` where
    /// none starts.
    pub(super) fn cpp_statement(&mut self, s: Slot) -> R<Option<N>> {
        let t = self.tok();
        if t.kind != T::Name {
            return Ok(None);
        }
        Ok(Some(match t.word {
            w::TRY => {
                let n = self.open_in(K::TryStatement, s);
                self.kw(K::TryStatement, w::TRY)?;
                self.compound(slot(f::BODY, Ctx::Bare))?;
                self.catches()?;
                self.close(n);
                n
            }
            w::THROW => {
                let n = self.open_in(K::ThrowStatement, s);
                self.kw(K::ThrowStatement, w::THROW)?;
                if self.peek() != T::Semi {
                    self.expr(EXPR)?;
                }
                self.punct_tk(K::ThrowStatement, T::Semi, false)?;
                self.close(n);
                n
            }
            w::CO_RETURN => {
                let n = self.open_in(K::CoReturnStatement, s);
                self.kw(K::CoReturnStatement, w::CO_RETURN)?;
                if self.peek() != T::Semi {
                    self.expr(EXPR)?;
                }
                self.punct_tk(K::CoReturnStatement, T::Semi, false)?;
                self.close(n);
                n
            }
            w::CO_YIELD => {
                let n = self.open_in(K::CoYieldStatement, s);
                self.kw(K::CoYieldStatement, w::CO_YIELD)?;
                self.expr(EXPR)?;
                self.punct_tk(K::CoYieldStatement, T::Semi, false)?;
                self.close(n);
                n
            }
            _ => return Ok(None),
        }))
    }
}

impl Mark {
    /// The node index the mark's writer was at: the first node written since.
    fn n_at(&self) -> usize {
        self.n_len
    }
}

/// An alternative token's word as an operator's kind (`and`, `or`, `not` …).
fn alt_operator(code: u8) -> Option<K> {
    Some(match code {
        w::AND => K::Tand,
        w::OR => K::Tor,
        w::NOT => K::Tnot,
        w::XOR => K::Txor,
        w::BITAND => K::Tbitand,
        w::BITOR => K::Tbitor,
        w::COMPL => K::Tcompl,
        w::AND_EQ => K::Tand_eq,
        w::OR_EQ => K::Tor_eq,
        w::XOR_EQ => K::Txor_eq,
        w::NOT_EQ => K::Tnot_eq,
        _ => return None,
    })
}
