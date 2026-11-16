//! Declarations: a declaration or a function definition (one parse until the `{`), `typedef`s,
//! `extern "C"`, specifiers (storage classes, qualifiers, attributes, the type), struct, union and
//! enum bodies, declarators of each family (`_declarator`, `_field_declarator`,
//! `_type_declarator`, `_abstract_declarator`), parameter lists and type descriptors.

use super::*;

/// A declarator family: the rule kinds its declarators are read by, and the supertype they are in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Fam {
    Decl,
    Field,
    Type,
    Abstract,
}

impl Fam {
    fn ctx(self) -> Ctx {
        match self {
            Fam::Decl => Ctx::Decl,
            Fam::Field => Ctx::FDecl,
            Fam::Type => Ctx::TDecl,
            Fam::Abstract => Ctx::ADecl,
        }
    }

    /// The rule kind of `kind` (as the tape names it) in this family: its implied tokens'.
    fn rule(self, kind: K) -> K {
        use K::*;
        match (self, kind) {
            (Fam::Field, PointerDeclarator) => PointerFieldDeclarator,
            (Fam::Field, ArrayDeclarator) => ArrayFieldDeclarator,
            (Fam::Field, ParenthesizedDeclarator) => ParenthesizedFieldDeclarator,
            (Fam::Field, FunctionDeclarator) => FunctionFieldDeclarator,
            (Fam::Field, AttributedDeclarator) => AttributedFieldDeclarator,
            (Fam::Type, PointerDeclarator) => PointerTypeDeclarator,
            (Fam::Type, ArrayDeclarator) => ArrayTypeDeclarator,
            (Fam::Type, ParenthesizedDeclarator) => ParenthesizedTypeDeclarator,
            (Fam::Type, FunctionDeclarator) => FunctionTypeDeclarator,
            (Fam::Type, AttributedDeclarator) => AttributedTypeDeclarator,
            (Fam::Abstract, PointerDeclarator) => AbstractPointerDeclarator,
            (Fam::Abstract, ArrayDeclarator) => AbstractArrayDeclarator,
            (Fam::Abstract, ParenthesizedDeclarator) => AbstractParenthesizedDeclarator,
            (Fam::Abstract, FunctionDeclarator) => AbstractFunctionDeclarator,
            _ => kind,
        }
    }

    /// The kind a family's declarator is written as.
    fn kind(self, kind: K) -> K {
        if self == Fam::Abstract { self.rule(kind) } else { kind }
    }
}

/// Whether a word is a storage class.
pub(super) fn is_storage(code: u8) -> bool {
    matches!(
        code,
        w::EXTERN | w::STATIC | w::AUTO | w::REGISTER | w::INLINE | w::U_INLINE | w::U_INLINE_U | w::U_FORCEINLINE | w::THREAD_LOCAL | w::U_THREAD
    )
}

/// Whether a word is a type qualifier (C's; C++'s are added by the dialect).
pub(super) fn is_qualifier(code: u8) -> bool {
    matches!(
        code,
        w::CONST | w::CONSTEXPR | w::VOLATILE | w::RESTRICT | w::U_RESTRICT_U | w::U_EXTENSION_U | w::ATOMIC | w::NORETURN_ | w::NORETURN | w::NONNULL
    )
}

fn is_sized(code: u8) -> bool {
    matches!(code, w::SIGNED | w::UNSIGNED | w::LONG | w::SHORT)
}

pub(super) fn is_ms_call(code: u8) -> bool {
    matches!(code, w::U_CDECL | w::U_CLRCALL | w::U_STDCALL | w::U_FASTCALL | w::U_THISCALL | w::U_VECTORCALL)
}

pub(super) fn is_ms_pointer(code: u8) -> bool {
    matches!(code, w::U_RESTRICT | w::U_UPTR | w::U_SPTR | w::UNALIGNED_ | w::U_UNALIGNED)
}

impl<'a> P<'a> {
    fn is_qualifier_word(&self, code: u8) -> bool {
        is_qualifier(code) || self.cpp && matches!(code, w::MUTABLE | w::CONSTINIT | w::CONSTEVAL)
    }

    /// Whether a word starts a declaration's specifiers for sure: a storage class, a qualifier, a
    /// type's keyword, an attribute.
    fn specifier_word(&self, code: u8) -> bool {
        is_storage(code)
            || self.is_qualifier_word(code)
            || is_sized(code)
            || is_primitive(code)
            || matches!(code, w::STRUCT | w::UNION | w::ENUM | w::U_ATTRIBUTE_U | w::U_ATTRIBUTE | w::U_DECLSPEC | w::TYPEDEF | w::ALIGNAS | w::ALIGNAS_)
            || is_ms_call(code)
    }

    /// Whether the item ahead is a declaration (or a definition) rather than a statement: its first
    /// words are specifiers, or a name is a type before a declarator (`T x`, `T *x;`, `T (*f)(…)`).
    ///
    /// In a block, a name's call before `*`, `=` or a parenthesized pointer is an expression
    /// (tree-sitter's choice): `M(x) *p = 0;`, `T (*f)(void) = g;`, `M(x) = 1;`.
    pub(super) fn declaration_ahead(&mut self) -> bool {
        let top = self.top;
        let t = self.tok();
        if t.kind != T::Name {
            return false;
        }
        if t.word == w::U_EXTENSION_U {
            // `__extension__` before a declaration, or before an expression.
            return self.kind_at(1) == T::Name && ({ let c = self.peek_at(1).word; self.specifier_word(c) } || self.is_ident_at(1) && self.kind_at(2) == T::Name);
        }
        if t.word == w::EXTERN || self.specifier_word(t.word) {
            return true;
        }
        if self.reserved(t.word) || matches!(t.word, w::TRUE | w::FALSE | w::TRUE_C | w::FALSE_C | w::NULL) {
            return false;
        }
        // A name: a type when a declarator follows it.
        let u = self.peek_at(1);
        match u.kind {
            T::Name => true,
            T::Star => self.pointer_declarator_ahead(1),
            T::LParen => self.macro_type_ahead(top) || self.paren_declarator_ahead(1, top),
            _ => false,
        }
    }

    /// Whether `*`s from token `k` lead to a declarator: `T *x;`, `T **x = …`, `T *x[…]`, `T *f(…)`.
    fn pointer_declarator_ahead(&mut self, mut k: usize) -> bool {
        while self.kind_at(k) == T::Star || self.peek_at(k).kind == T::Name && { let c = self.peek_at(k).word; self.is_qualifier_word(c) } {
            k += 1;
        }
        if !self.is_ident_at(k) {
            return self.kind_at(k) == T::LParen && self.kind_at(k + 1) == T::Star;
        }
        matches!(self.kind_at(k + 1), T::Semi | T::Comma | T::Eq | T::LBracket | T::LParen | T::RParen)
    }

    /// Whether `(` at token `k` holds a declarator after a type: `T (*f)(…)`, `T (x);` is a call.
    /// Where not `top`, only a declarator no expression reads (`T (*a[])(…)`).
    fn paren_declarator_ahead(&mut self, k: usize, top: bool) -> bool {
        // `T (x) {` (a macro's function: `TEST(name) {…}`), `T (x) = …`.
        if self.is_ident_at(k + 1) && self.kind_at(k + 2) == T::RParen {
            return top && matches!(self.kind_at(k + 3), T::LBrace | T::Eq);
        }
        if self.kind_at(k + 1) != T::Star {
            return false;
        }
        let mut j = k + 1;
        while self.kind_at(j) == T::Star {
            j += 1;
        }
        if !self.is_ident_at(j) {
            return false;
        }
        // `T (*f)(…)`, `T (*a)[n]`, `T (*p) = …`, `T (*a[n])(…)`; `f(*x);` and `f(*a[i].b)` are
        // calls.
        let Some(e) = self.paren_end(k) else { return false };
        let mut i = j + 1;
        let mut empty = false;
        while i < e {
            match self.kind_at(i) {
                T::LBracket => empty |= self.kind_at(i + 1) == T::RBracket,
                T::RBracket | T::Number => {}
                T::Name if self.is_ident_at(i) => {}
                _ => return false,
            }
            i += 1;
        }
        // In a block: empty brackets, or parameters only a type reads (`(T *)`, `(int)`).
        let typed = self.kind_at(e + 1) == T::LParen && self.is_ident_at(e + 2) && self.kind_at(e + 3) == T::Star && {
            let mut i = e + 3;
            while self.kind_at(i) == T::Star {
                i += 1;
            }
            self.kind_at(i) == T::RParen
        };
        (top || empty || typed) && matches!(self.kind_at(e + 1), T::LParen | T::LBracket | T::Eq)
    }

    /// A declaration item at `level` under `parent`: a declaration, a function definition, a
    /// `typedef`, `extern "C"`, or specifiers alone and `;` (an empty declaration: its children are
    /// the parent's).
    pub(super) fn declaration_item(&mut self, level: Level, parent: K) -> R<()> {
        let t = self.tok();
        if t.word == w::TYPEDEF || t.word == w::U_EXTENSION_U && self.is_word_at(1, w::TYPEDEF) {
            self.type_definition()?;
            return Ok(());
        }
        if t.word == w::EXTERN && self.kind_at(1) == T::Str {
            return self.linkage();
        }
        let _ = level;
        let start = self.tok().start;
        self.flush(start);
        let first = self.w.len();
        let ty = self.specifiers(K::Declaration)?;
        if self.peek() == T::Semi {
            let Some(ty) = ty else { return self.refuse() };
            // `_empty_declaration`: the type (no field) and the `;`, the parent's.
            self.rekey(ty, BARE.with_ctx(Ctx::Type));
            self.tk(parent, K::TSemi)?;
            return Ok(());
        }
        if ty.is_none() {
            return self.refuse();
        }
        let n = self.insert_at(first, start, K::Declaration, BARE);
        self.declarators(n, true)?;
        self.close(n);
        Ok(())
    }

    /// A declaration (with its `;`) in slot `s`: a `for`'s initializer.
    pub(super) fn declaration_in(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::Declaration, s);
        if self.specifiers(K::Declaration)?.is_none() {
            return self.refuse();
        }
        self.declarators(n, false)?;
        self.close(n);
        Ok(n)
    }

    /// A node of `kind` put before the node at `at` (which starts at `start`), over it and what
    /// follows, in slot `s`.
    pub(super) fn insert_at(&mut self, at: usize, start: u32, kind: K, s: Slot) -> N {
        self.w.insert(at, key(kind, s.field, class(kind, s.ctx), false), start);
        N { at, kind }
    }

    /// A declaration's declarators after its specifiers, to its `;`: each a declarator or an
    /// `init_declarator`. The first, a function's declarator before `{`, makes the declaration a
    /// `function_definition` (where `body` allows one).
    fn declarators(&mut self, n: N, body: bool) -> R<()> {
        let mut first = true;
        loop {
            let d = self.declarator(Fam::Decl, slot(f::DECLARATOR, Ctx::Bare))?;
            // A function's definition: its declarator (a function declarator, or one around it:
            // `char *f(…)`) and a body.
            if first && body && d.kind == K::FunctionDeclarator && self.peek() == T::Name && !self.is_attribute() && !self.is_asm() {
                // An old-style definition: its parameters' declarations, then the body.
                self.rekind(n.at, K::FunctionDefinition, BARE);
                self.rekey(d.at, slot(f::DECLARATOR, Ctx::Bare));
                while self.peek() != T::LBrace {
                    if !self.declaration_ahead() {
                        return self.refuse();
                    }
                    self.declaration_item(Level::Block, K::FunctionDefinition)?;
                }
                return self.function_body();
            }
            if first && body && self.function_body_ahead() {
                self.rekind(n.at, K::FunctionDefinition, BARE);
                self.rekey(d.at, slot(f::DECLARATOR, Ctx::Decl));
                return self.function_body();
            }
            first = false;
            // An attribute after a declarator other than a function's is tree-sitter's error.
            if self.is_asm() {
                self.gnu_asm(BARE)?;
            }
            match self.peek() {
                T::Eq => {
                    let i = self.wrap(d, K::InitDeclarator, slot(f::DECLARATOR, Ctx::Bare), slot(f::DECLARATOR, Ctx::Decl));
                    self.tk(K::InitDeclarator, K::TEq)?;
                    if self.peek() == T::LBrace {
                        self.initializer_list(slot(f::VALUE, Ctx::Bare))?;
                    } else {
                        self.expr(slot(f::VALUE, Ctx::Expr))?;
                    }
                    self.close(i);
                }
                _ => {}
            }
            match self.peek() {
                T::Comma => self.tk(K::Declaration, K::TComma)?,
                T::Semi => {
                    self.tk(K::Declaration, K::TSemi)?;
                    return Ok(());
                }
                _ => return self.refuse(),
            }
        }
    }

    /// Whether a function's body follows its declarator: `{` (attributes passed).
    fn function_body_ahead(&mut self) -> bool {
        self.peek() == T::LBrace
    }

    fn function_body(&mut self) -> R<()> {
        self.compound(slot(f::BODY, Ctx::Bare))?;
        Ok(())
    }

    /// `typedef` a type and its declarators.
    fn type_definition(&mut self) -> R<()> {
        let n = self.open(K::TypeDefinition, f::NONE, C_NONE);
        if self.is_word(w::U_EXTENSION_U) {
            self.leaf(K::T__extension__, f::NONE, C_NONE)?;
        }
        self.kw(K::TypeDefinition, w::TYPEDEF)?;
        // `type_qualifier* type: type_specifier type_qualifier*`.
        while self.tok().kind == T::Name && { let c = self.tok().word; self.is_qualifier_word(c) } {
            self.qualifier()?;
        }
        if !self.type_specifier(slot(f::TYPE, Ctx::Type))? {
            return self.refuse();
        }
        while self.tok().kind == T::Name && { let c = self.tok().word; self.is_qualifier_word(c) } {
            self.qualifier()?;
        }
        loop {
            self.declarator(Fam::Type, slot(f::DECLARATOR, Ctx::TDecl))?;
            match self.peek() {
                T::Comma => self.tk(K::TypeDefinition, K::TComma)?,
                _ => break,
            }
        }
        while self.is_attribute() {
            self.attribute_specifier()?;
        }
        self.punct_tk(K::TypeDefinition, T::Semi, false)?;
        self.close(n);
        Ok(())
    }

    /// `extern "C"` and a definition, a declaration or a `declaration_list`.
    fn linkage(&mut self) -> R<()> {
        let n = self.open(K::LinkageSpecification, f::NONE, C_NONE);
        self.kw(K::LinkageSpecification, w::EXTERN)?;
        self.string(slot(f::VALUE, Ctx::Bare))?;
        if self.peek() == T::LBrace {
            let l = self.open(K::DeclarationList, f::BODY, C_NONE);
            self.tk(K::DeclarationList, K::TLBrace)?;
            while self.peek() != T::RBrace {
                if self.peek() == T::Eof {
                    return self.refuse();
                }
                self.item(Level::Top, K::DeclarationList)?;
            }
            self.tk(K::DeclarationList, K::TRBrace)?;
            self.close(l);
        } else {
            let start = self.tok().start;
            self.flush(start);
            let first = self.w.len();
            if self.specifiers(K::Declaration)?.is_none() {
                return self.refuse();
            }
            let d = self.insert_at(first, start, K::Declaration, slot(f::BODY, Ctx::Bare));
            self.declarators(d, true)?;
            self.close(d);
        }
        self.close(n);
        Ok(())
    }

    // ---- Specifiers.

    /// `_declaration_specifiers`: modifiers, the type (`type:`), modifiers. The type's node, where
    /// one was read.
    pub(super) fn specifiers(&mut self, _parent: K) -> R<Option<usize>> {
        let mut ty = None;
        loop {
            let t = self.tok();
            match t.kind {
                T::Name => {}
                T::LBracket if self.kind_at(1) == T::LBracket => {
                    self.attribute_declaration()?;
                    continue;
                }
                _ => return Ok(ty),
            }
            let c = t.word;
            if is_storage(c) {
                let n = self.open(K::StorageClassSpecifier, f::NONE, C_NONE);
                self.kw_any(K::StorageClassSpecifier)?;
                self.close(n);
            } else if self.is_qualifier_word(c) || matches!(c, w::ALIGNAS | w::ALIGNAS_) {
                self.qualifier()?;
            } else if matches!(c, w::U_ATTRIBUTE_U | w::U_ATTRIBUTE) {
                self.attribute_specifier()?;
            } else if c == w::U_DECLSPEC {
                self.declspec()?;
            } else if is_ms_call(c) {
                let m = self.open(K::MsCallModifier, f::NONE, C_NONE);
                self.kw_any(K::MsCallModifier)?;
                self.close(m);
            } else if ty.is_none() && (is_sized(c) || is_primitive(c) || matches!(c, w::STRUCT | w::UNION | w::ENUM) || !self.reserved(c)) {
                let at = self.w.len();
                if !self.type_specifier(slot(f::TYPE, Ctx::Type))? {
                    return self.refuse();
                }
                ty = Some(at);
            } else if ty.is_some() && is_sized(c) {
                return self.refuse();
            } else {
                return Ok(ty);
            }
        }
    }

    /// A `type_qualifier` (or `alignas_qualifier`): its keyword stored.
    fn qualifier(&mut self) -> R<()> {
        let n = self.open(K::TypeQualifier, f::NONE, C_NONE);
        if matches!(self.tok().word, w::ALIGNAS | w::ALIGNAS_) {
            let a = self.open(K::AlignasQualifier, f::NONE, C_NONE);
            self.kw_any(K::AlignasQualifier)?;
            self.leaf(K::TLParen, f::NONE, C_NONE)?;
            if self.type_in_parens_ahead(0) {
                self.type_descriptor(BARE)?;
            } else {
                self.expr(EXPR)?;
            }
            self.punct_tk(K::AlignasQualifier, T::RParen, false)?;
            self.close(a);
        } else {
            self.kw_any(K::TypeQualifier)?;
        }
        self.close(n);
        Ok(())
    }

    /// A `type_specifier` in slot `s`: a struct, union or enum; a sized type; a primitive type; a
    /// type's name (or a macro's, `M(type)`). Whether one was read.
    pub(super) fn type_specifier(&mut self, s: Slot) -> R<bool> {
        let t = self.tok();
        if t.kind != T::Name {
            return Ok(false);
        }
        match t.word {
            w::STRUCT | w::UNION => self.struct_specifier(s)?,
            w::ENUM => self.enum_specifier(s)?,
            c if is_sized(c) => self.sized(s)?,
            c if is_primitive(c) => {
                self.leaf_in(K::PrimitiveType, s)?;
            }
            c if self.reserved(c) => return Ok(false),
            _ => {
                if self.kind_at(1) == T::LParen && self.macro_type_ahead(true) {
                    let n = self.open_in(K::MacroTypeSpecifier, s);
                    self.leaf(K::Identifier, f::NAME, C_NONE)?;
                    self.tk(K::MacroTypeSpecifier, K::TLParen)?;
                    self.type_descriptor(slot(f::TYPE, Ctx::Bare))?;
                    self.punct_tk(K::MacroTypeSpecifier, T::RParen, false)?;
                    self.close(n);
                } else {
                    self.leaf_in(K::TypeIdentifier, s)?;
                }
            }
        }
        Ok(true)
    }

    /// Whether `M(…)` ahead is a macro's type (`macro_type_specifier`): a type in the parentheses,
    /// and a declarator after them.
    fn macro_type_ahead(&mut self, star: bool) -> bool {
        let Some(e) = self.paren_end(1) else { return false };
        // A type in the parentheses: a type's words, or one name (`STACK_OF(X) *p`).
        let one = self.is_ident_at(2) && self.kind_at(3) == T::RParen;
        (one || self.type_in_parens_ahead(1)) && (self.is_ident_at(e + 1) || star && self.kind_at(e + 1) == T::Star)
    }

    /// The index of the token closing the bracket at token `k`.
    pub(super) fn paren_end(&mut self, k: usize) -> Option<usize> {
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

    /// `signed`, `unsigned`, `long` and `short`, and a type among them.
    fn sized(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::SizedTypeSpecifier, s);
        let mut ty = false;
        loop {
            let t = self.tok();
            if t.kind != T::Name {
                break;
            }
            if is_sized(t.word) {
                self.kw_any(K::SizedTypeSpecifier)?;
            } else if !ty && is_primitive(t.word) {
                self.leaf(K::PrimitiveType, f::TYPE, C_NONE)?;
                ty = true;
            } else if !ty && !self.reserved(t.word) && (self.is_ident_at(1) || self.kind_at(1) == T::Star) && !self.specifier_word(t.word) {
                self.leaf(K::TypeIdentifier, f::TYPE, C_NONE)?;
                ty = true;
            } else {
                break;
            }
        }
        self.close(n);
        Ok(())
    }

    /// `struct` or `union`: attributes, a name, a body.
    fn struct_specifier(&mut self, s: Slot) -> R<()> {
        let kind = if self.is_word(w::STRUCT) { K::StructSpecifier } else { K::UnionSpecifier };
        let n = self.open_in(kind, s);
        self.kw_any(kind)?;
        while self.is_attribute() || self.is_word(w::U_DECLSPEC) {
            if self.is_word(w::U_DECLSPEC) {
                self.declspec()?;
            } else {
                self.attribute_specifier()?;
            }
        }
        let mut named = false;
        if self.is_ident() {
            self.leaf(K::TypeIdentifier, f::NAME, C_NONE)?;
            named = true;
        }
        if self.peek() == T::LBrace {
            self.field_list()?;
        } else if !named {
            return self.refuse();
        }
        while self.is_attribute() {
            self.attribute_specifier()?;
        }
        self.close(n);
        Ok(())
    }

    /// `{` field items `}`.
    fn field_list(&mut self) -> R<()> {
        let n = self.open(K::FieldDeclarationList, f::BODY, C_NONE);
        self.tk(K::FieldDeclarationList, K::TLBrace)?;
        while self.peek() != T::RBrace {
            if self.peek() == T::Eof {
                return self.refuse();
            }
            self.item(Level::Field, K::FieldDeclarationList)?;
        }
        self.tk(K::FieldDeclarationList, K::TRBrace)?;
        self.close(n);
        Ok(())
    }

    /// A field item (not a directive): a `field_declaration`.
    pub(super) fn field_item(&mut self, _parent: K) -> R<()> {
        let n = self.open(K::FieldDeclaration, f::NONE, C_NONE);
        if self.specifiers(K::FieldDeclaration)?.is_none() {
            return self.refuse();
        }
        if self.peek() != T::Semi && !self.is_attribute() {
            loop {
                if self.peek() == T::Colon {
                    // An unnamed bit field.
                    self.bitfield()?;
                } else {
                    self.declarator(Fam::Field, slot(f::DECLARATOR, Ctx::FDecl))?;
                    if self.peek() == T::Colon {
                        self.bitfield()?;
                    }
                }
                match self.peek() {
                    T::Comma => self.tk(K::FieldDeclaration, K::TComma)?,
                    _ => break,
                }
            }
        }
        if self.is_attribute() {
            self.attribute_specifier()?;
        }
        self.punct_tk(K::FieldDeclaration, T::Semi, false)?;
        self.close(n);
        Ok(())
    }

    fn bitfield(&mut self) -> R<()> {
        let b = self.open(K::BitfieldClause, f::NONE, C_NONE);
        self.tk(K::BitfieldClause, K::TColon)?;
        self.expr(EXPR)?;
        self.close(b);
        Ok(())
    }

    /// `enum`: a name, an underlying type, a body.
    fn enum_specifier(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::EnumSpecifier, s);
        self.kw(K::EnumSpecifier, w::ENUM)?;
        let mut named = false;
        if self.is_ident() {
            self.leaf(K::TypeIdentifier, f::NAME, C_NONE)?;
            named = true;
            if self.peek() == T::Colon {
                self.tk(K::EnumSpecifier, K::TColon)?;
                let t = self.tok();
                if t.kind != T::Name || !is_primitive(t.word) {
                    return self.refuse();
                }
                self.leaf(K::PrimitiveType, f::UNDERLYING_TYPE, C_NONE)?;
            }
        }
        if self.peek() == T::LBrace {
            let l = self.open(K::EnumeratorList, f::BODY, C_NONE);
            self.tk(K::EnumeratorList, K::TLBrace)?;
            let mut trailing = false;
            while self.peek() != T::RBrace {
                match self.peek() {
                    T::Eof => return self.refuse(),
                    T::Directive => self.directive(Level::Enum)?,
                    _ => {
                        self.enumerator()?;
                        match self.peek() {
                            // A trailing `,`: it and the `}` stored.
                            T::Comma if self.kind_at(1) == T::RBrace => {
                                self.leaf(K::TComma, f::NONE, C_NONE)?;
                                trailing = true;
                            }
                            // A `,` before a branch's end (`#endif`, `#else` …) is stored.
                            T::Comma if self.kind_at(1) == T::Directive && matches!(self.peek_at(1).word, w::ELSE | w::ELIF | w::ELIFDEF | w::ELIFNDEF | w::ENDIF) => {
                                self.leaf(K::TComma, f::NONE, C_NONE)?;
                            }
                            T::Comma => self.tk(K::EnumeratorList, K::TComma)?,
                            T::RBrace => {}
                            T::Directive => {}
                            _ => return self.refuse(),
                        }
                    }
                }
            }
            if trailing {
                self.leaf(K::TRBrace, f::NONE, C_NONE)?;
            } else {
                self.tk(K::EnumeratorList, K::TRBrace)?;
            }
            self.close(l);
        } else if !named {
            return self.refuse();
        }
        if self.is_attribute() {
            self.attribute_specifier()?;
        }
        self.close(n);
        Ok(())
    }

    /// `name (= value)?`.
    pub(super) fn enumerator(&mut self) -> R<()> {
        if !self.is_ident() {
            return self.refuse();
        }
        let n = self.open(K::Enumerator, f::NONE, C_NONE);
        self.leaf(K::Identifier, f::NAME, C_NONE)?;
        if self.peek() == T::Eq {
            self.tk(K::Enumerator, K::TEq)?;
            self.expr(slot(f::VALUE, Ctx::Expr))?;
        }
        self.close(n);
        Ok(())
    }

    // ---- Attributes.

    pub(super) fn is_attribute(&mut self) -> bool {
        let t = self.tok();
        t.kind == T::Name && matches!(t.word, w::U_ATTRIBUTE_U | w::U_ATTRIBUTE)
    }

    /// `__attribute__((…))`: its argument list.
    pub(super) fn attribute_specifier(&mut self) -> R<()> {
        let n = self.open(K::AttributeSpecifier, f::NONE, C_NONE);
        self.kw_any(K::AttributeSpecifier)?;
        // The gap before the arguments has two words (`__attribute__ (`, `__attribute (`): stored.
        self.punct_tk(K::AttributeSpecifier, T::LParen, true)?;
        if self.peek() != T::LParen {
            return self.refuse();
        }
        self.arguments(BARE)?;
        self.punct_tk(K::AttributeSpecifier, T::RParen, false)?;
        self.close(n);
        Ok(())
    }

    /// `__declspec(name)`.
    fn declspec(&mut self) -> R<()> {
        let n = self.open(K::MsDeclspecModifier, f::NONE, C_NONE);
        self.kw(K::MsDeclspecModifier, w::U_DECLSPEC)?;
        self.punct_tk(K::MsDeclspecModifier, T::LParen, false)?;
        if !self.is_ident() {
            return self.refuse();
        }
        self.leaf(K::Identifier, f::NONE, C_NONE)?;
        self.punct_tk(K::MsDeclspecModifier, T::RParen, false)?;
        self.close(n);
        Ok(())
    }

    /// `[[attribute, …]]`.
    pub(super) fn attribute_declaration(&mut self) -> R<()> {
        let n = self.open(K::AttributeDeclaration, f::NONE, C_NONE);
        // `[[` and `]]` are one token each in the grammar; the lexer reads two.
        let a = self.tok();
        let b = self.peek_at(1);
        if b.start != a.end {
            return self.refuse();
        }
        if self.stores(K::AttributeDeclaration, K::TLBracketLBracket, f::NONE) {
            return self.refuse();
        }
        self.bump()?;
        self.bump()?;
        loop {
            let at = self.open(K::Attribute, f::NONE, C_NONE);
            if !self.is_ident() {
                return self.refuse();
            }
            if self.cpp && self.kind_at(1) == T::ColonColon {
                self.leaf(K::Identifier, f::PREFIX, C_NONE)?;
                self.tk(K::Attribute, K::TColonColon)?;
            }
            if !self.is_ident() {
                return self.refuse();
            }
            self.leaf(K::Identifier, f::NAME, C_NONE)?;
            if self.peek() == T::LParen {
                self.arguments(BARE)?;
            }
            self.close(at);
            match self.peek() {
                T::Comma => self.tk(K::AttributeDeclaration, K::TComma)?,
                T::RBracket => break,
                _ => return self.refuse(),
            }
        }
        let a = self.tok();
        let b = self.peek_at(1);
        if a.kind != T::RBracket || b.kind != T::RBracket || b.start != a.end {
            return self.refuse();
        }
        self.bump()?;
        self.bump()?;
        self.close(n);
        Ok(())
    }

    // ---- Declarators.

    /// A declarator of family `fam` in slot `s`.
    pub(super) fn declarator(&mut self, fam: Fam, s: Slot) -> R<N> {
        self.deeper()?;
        let r = self.declarator_inner(fam, s);
        self.depth -= 1;
        r
    }

    fn declarator_inner(&mut self, fam: Fam, s: Slot) -> R<N> {
        let inner = slot(f::DECLARATOR, fam.ctx());
        let t = self.tok();
        let mut d = match t.kind {
            T::Star => {
                let kind = fam.kind(K::PointerDeclarator);
                let rule = fam.rule(K::PointerDeclarator);
                let n = self.open_in(kind, s);
                self.tk(rule, K::TStar)?;
                loop {
                    let t = self.tok();
                    if t.kind != T::Name {
                        break;
                    }
                    if is_ms_pointer(t.word) {
                        let m = self.open(K::MsPointerModifier, f::NONE, C_NONE);
                        let k = match t.word {
                            w::U_RESTRICT => K::MsRestrictModifier,
                            w::U_UPTR => K::MsUnsignedPtrModifier,
                            w::U_SPTR => K::MsSignedPtrModifier,
                            _ => K::MsUnalignedPtrModifier,
                        };
                        let k2 = self.open(k, f::NONE, C_NONE);
                        self.kw_any(k)?;
                        self.close(k2);
                        self.close(m);
                    } else if self.is_qualifier_word(t.word) {
                        self.qualifier()?;
                    } else {
                        break;
                    }
                }
                if fam != Fam::Abstract || self.abstract_continues() {
                    self.declarator(fam, inner)?;
                }
                self.close(n);
                return Ok(n);
            }
            T::LParen if fam == Fam::Abstract => {
                if self.abstract_paren_ahead() {
                    let kind = K::AbstractParenthesizedDeclarator;
                    let n = self.open_in(kind, s);
                    self.tk(kind, K::TLParen)?;
                    self.declarator(fam, BARE.with_ctx(Ctx::ADecl))?;
                    self.punct_tk(kind, T::RParen, false)?;
                    self.close(n);
                    n
                } else {
                    // Parameters alone: a function declarator with no declarator.
                    let n = self.open_in(K::AbstractFunctionDeclarator, s);
                    self.parameter_list()?;
                    self.close(n);
                    n
                }
            }
            T::LBracket if fam == Fam::Abstract => {
                let n = self.open_in(K::AbstractArrayDeclarator, s);
                self.array_suffix(K::AbstractArrayDeclarator)?;
                self.close(n);
                n
            }
            T::LParen => {
                let n = self.open_in(K::ParenthesizedDeclarator, s);
                let rule = fam.rule(K::ParenthesizedDeclarator);
                self.tk(rule, K::TLParen)?;
                if self.tok().kind == T::Name && is_ms_call(self.tok().word) {
                    let m = self.open(K::MsCallModifier, f::NONE, C_NONE);
                    self.kw_any(K::MsCallModifier)?;
                    self.close(m);
                }
                self.declarator(fam, slot(f::NONE, fam.ctx()))?;
                self.punct_tk(rule, T::RParen, false)?;
                self.close(n);
                n
            }
            T::Name if fam == Fam::Type && (is_primitive(t.word) || is_sized(t.word)) => self.leaf_in(K::PrimitiveType, s)?,
            T::Name if !self.reserved(t.word) || fam == Fam::Field => {
                let kind = match fam {
                    Fam::Decl => K::Identifier,
                    Fam::Field => K::FieldIdentifier,
                    Fam::Type => K::TypeIdentifier,
                    Fam::Abstract => return self.refuse(),
                };
                self.leaf_in(kind, s)?
            }
            _ => return self.refuse(),
        };
        // Suffixes: parameters, array sizes, attributes.
        loop {
            match self.peek() {
                T::LParen => {
                    let kind = fam.kind(K::FunctionDeclarator);
                    let n = self.wrap(d, kind, s, inner);
                    let old = self.parameter_list()?;
                    if fam == Fam::Decl {
                        if self.is_asm() {
                            self.gnu_asm(BARE)?;
                        }
                        // Attributes, and names and macro calls (keywords too, where none is
                        // valid) before a definition's body: `int f(void) NORETURN {`. A
                        // declaration's names are tree-sitter's errors: refused.
                        let mut names = false;
                        while !old && self.peek() == T::Name {
                            if self.is_attribute() {
                                self.attribute_specifier()?;
                            } else if self.kind_at(1) == T::LParen {
                                names = true;
                                let c = self.open(K::CallExpression, f::NONE, C_NONE);
                                self.leaf(K::Identifier, f::FUNCTION, C_NONE)?;
                                self.arguments(slot(f::ARGUMENTS, Ctx::Bare))?;
                                self.close(c);
                            } else {
                                names = true;
                                self.leaf(K::Identifier, f::NONE, C_NONE)?;
                            }
                        }
                        if names && self.peek() != T::LBrace {
                            return self.refuse();
                        }
                    }
                    self.close(n);
                    d = n;
                }
                T::LBracket if self.kind_at(1) == T::LBracket => {
                    let kind = K::AttributedDeclarator;
                    let n = self.wrap(d, kind, s, slot(f::NONE, fam.ctx()));
                    while self.peek() == T::LBracket && self.kind_at(1) == T::LBracket {
                        self.attribute_declaration()?;
                    }
                    self.close(n);
                    d = n;
                }
                T::LBracket => {
                    let kind = fam.kind(K::ArrayDeclarator);
                    let n = self.wrap(d, kind, s, inner);
                    self.array_suffix(fam.rule(K::ArrayDeclarator))?;
                    self.close(n);
                    d = n;
                }
                _ => return Ok(d),
            }
        }
    }

    /// `[` qualifiers and `static`, a size or `*`, `]`.
    fn array_suffix(&mut self, rule: K) -> R<()> {
        self.tk(rule, K::TLBracket)?;
        loop {
            let t = self.tok();
            if t.kind == T::Name && self.is_qualifier_word(t.word) {
                self.qualifier()?;
            } else if t.kind == T::Name && t.word == w::STATIC {
                self.leaf(K::Tstatic, f::NONE, C_NONE)?;
            } else {
                break;
            }
        }
        match self.peek() {
            T::RBracket => {}
            T::Star if self.kind_at(1) == T::RBracket => {
                self.leaf(K::TStar, f::SIZE, C_NONE)?;
            }
            _ => {
                self.expr(slot(f::SIZE, Ctx::Expr))?;
            }
        }
        self.punct_tk(rule, T::RBracket, false)
    }

    /// Whether an abstract pointer's declarator goes on: `*` then `*`, `(`, `[`.
    fn abstract_continues(&mut self) -> bool {
        matches!(self.peek(), T::Star | T::LBracket) || self.peek() == T::LParen
    }

    /// Whether `(` ahead opens an abstract parenthesized declarator (`(*)`, `(*)[…]`, `([…])`), not
    /// parameters.
    fn abstract_paren_ahead(&mut self) -> bool {
        matches!(self.kind_at(1), T::Star | T::LBracket) || self.kind_at(1) == T::LParen && self.kind_at(2) != T::RParen && !self.type_in_parens_ahead(1)
    }

    /// `(` parameters `)`.
    /// Whether it is an old-style definition's.
    pub(super) fn parameter_list(&mut self) -> R<bool> {
        let n = self.open(K::ParameterList, f::PARAMETERS, C_NONE);
        if self.old_style_ahead() {
            // An old-style definition's names: identifiers.
            self.list(K::ParameterList, T::RParen, true, false, |p| {
                p.leaf(K::Identifier, f::NONE, C_NONE)?;
                Ok(())
            })?;
            self.close(n);
            return Ok(true);
        }
        self.list(K::ParameterList, T::RParen, true, false, |p| {
            if p.peek() == T::DotDotDot {
                let v = p.open(K::VariadicParameter, f::NONE, C_NONE);
                p.leaf(K::TDotDotDot, f::NONE, C_NONE)?;
                p.close(v);
                return Ok(());
            }
            p.parameter_declaration()
        })?;
        self.close(n);
        Ok(false)
    }

    fn parameter_declaration(&mut self) -> R<()> {
        let n = self.open(K::ParameterDeclaration, f::NONE, C_NONE);
        if self.specifiers(K::ParameterDeclaration)?.is_none() {
            return self.refuse();
        }
        if !matches!(self.peek(), T::Comma | T::RParen) && !self.is_attribute() {
            if self.names_ahead() {
                self.declarator(Fam::Decl, slot(f::DECLARATOR, Ctx::Decl))?;
            } else {
                self.declarator(Fam::Abstract, slot(f::DECLARATOR, Ctx::ADecl))?;
            }
        }
        while self.is_attribute() {
            self.attribute_specifier()?;
        }
        self.close(n);
        Ok(())
    }

    /// Whether the parameter list ahead is an old-style definition's (`f(a, b) int a; char *b; {`):
    /// names alone, and a declaration after the `)`.
    fn old_style_ahead(&mut self) -> bool {
        let mut k = 1;
        loop {
            if !self.is_ident_at(k) {
                return false;
            }
            match self.kind_at(k + 1) {
                T::Comma => k += 2,
                T::RParen => break,
                _ => return false,
            }
        }
        let t = self.peek_at(k + 2);
        t.kind == T::Name && !matches!(t.word, w::U_ATTRIBUTE_U | w::U_ATTRIBUTE | w::ASM | w::U_ASM_U | w::U_ASM | w::TYPEDEF) && {
            let c = t.word;
            self.specifier_word(c) || !self.reserved(c)
        }
    }

    /// Whether the declarator ahead names something (a concrete declarator), not an abstract one.
    fn names_ahead(&mut self) -> bool {
        let mut k = 0;
        loop {
            let t = self.peek_at(k);
            match t.kind {
                // `(name)`: a parameter list holding a type (tree-sitter's choice).
                T::LParen if self.is_ident_at(k + 1) && self.kind_at(k + 2) == T::RParen => return false,
                T::Star | T::LParen => k += 1,
                T::Name if self.is_qualifier_word(t.word) || is_ms_pointer(t.word) || is_ms_call(t.word) => k += 1,
                T::Name => return !self.reserved(t.word),
                _ => return false,
            }
            if t.kind == T::LParen && self.type_in_parens_ahead(k - 1) {
                return false;
            }
        }
    }

    /// `type_descriptor` in slot `s`: qualifiers, a type, qualifiers, an abstract declarator.
    pub(super) fn type_descriptor(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::TypeDescriptor, s);
        while self.tok().kind == T::Name && { let c = self.tok().word; self.is_qualifier_word(c) } {
            self.qualifier()?;
        }
        if !self.type_specifier(slot(f::TYPE, Ctx::Type))? {
            return self.refuse();
        }
        while self.tok().kind == T::Name && { let c = self.tok().word; self.is_qualifier_word(c) } {
            self.qualifier()?;
        }
        if matches!(self.peek(), T::Star | T::LBracket | T::LParen) {
            self.declarator(Fam::Abstract, slot(f::DECLARATOR, Ctx::ADecl))?;
        }
        self.close(n);
        Ok(n)
    }

    /// Whether the parentheses at token `k` hold a type (a cast's, `sizeof`'s): a type's keyword
    /// or qualifier first, or a name then `*`s and `)`, or a name and `)` where `cast` allows.
    pub(super) fn type_in_parens_ahead(&mut self, k: usize) -> bool {
        let t = self.peek_at(k + 1);
        if t.kind != T::Name {
            return false;
        }
        if self.specifier_word(t.word) && !is_storage(t.word) && t.word != w::TYPEDEF && !is_ms_call(t.word) {
            return true;
        }
        if self.reserved(t.word) || matches!(t.word, w::TRUE | w::FALSE | w::TRUE_C | w::FALSE_C | w::NULL) {
            return false;
        }
        let mut j = k + 2;
        while self.kind_at(j) == T::Star || self.peek_at(j).kind == T::Name && { let c = self.peek_at(j).word; self.is_qualifier_word(c) } {
            j += 1;
        }
        if j > k + 2 {
            return self.kind_at(j) == T::RParen;
        }
        // `(T)`: a type or an expression; the caller decides by what follows.
        false
    }
}
