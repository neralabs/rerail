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

/// Whether a word is a C++ literal's or an operator's that a later parameter reads as a type's
/// name (`f(a, nullptr)` declares a function).
pub(super) fn is_literal_word(code: u8) -> bool {
    matches!(code, w::TRUE | w::FALSE | w::NULLPTR | w::NULL | w::THIS | w::SIZEOF)
}

pub(super) fn is_sized_word(code: u8) -> bool {
    is_sized(code)
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
            || self.cpp && matches!(code, w::CLASS | w::VIRTUAL | w::TYPENAME | w::DECLTYPE | w::EXPLICIT)
    }

    /// Whether the item ahead is a declaration (or a definition) rather than a statement: its first
    /// words are specifiers, or a name is a type before a declarator (`T x`, `T *x;`, `T (*f)(…)`).
    ///
    /// In a block, a name's call before `*`, `=` or a parenthesized pointer is an expression
    /// (tree-sitter's choice): `M(x) *p = 0;`, `T (*f)(void) = g;`, `M(x) = 1;`.
    pub(super) fn declaration_ahead(&mut self) -> bool {
        let top = self.top;
        let t = self.tok();
        if self.cpp && t.kind == T::ColonColon {
            return self.cpp_type_declaration_ahead();
        }
        if self.cpp && t.kind == T::LBracket && self.kind_at(1) == T::LBracket {
            // Attributes, then a declaration's first word (`[[fallthrough]];` is a statement).
            let mut e = 0;
            while self.kind_at(e) == T::LBracket && self.kind_at(e + 1) == T::LBracket {
                match self.paren_end(e) {
                    Some(b) => e = b + 1,
                    None => return false,
                }
            }
            let u = self.peek_at(e);
            return u.kind == T::Name && (self.specifier_word(u.word) || !self.reserved(u.word) && matches!(self.kind_at(e + 1), T::Name | T::ColonColon | T::Lt | T::Star | T::Amp | T::AmpAmp));
        }
        if t.kind != T::Name {
            return false;
        }
        if t.word == w::U_EXTENSION_U {
            // `__extension__` before a declaration, or before an expression.
            return self.kind_at(1) == T::Name && ({ let c = self.peek_at(1).word; self.specifier_word(c) } || self.is_ident_at(1) && self.kind_at(2) == T::Name);
        }
        if self.cpp && t.word == w::DECLTYPE {
            // `decltype(x) y;`; `decltype(x)::f();` reads on.
            let Some(e) = self.paren_end(1) else { return false };
            return self.kind_at(e + 1) == T::Name || self.kind_at(e + 1) == T::ColonColon && self.cpp_type_declaration_ahead();
        }
        if self.cpp && is_primitive(t.word) && self.kind_at(1) == T::LParen && !top {
            // `void (*f)() = g;` in a block: a call's reading wins where both read.
            return self.paren_declarator_ahead(1, top);
        }
        if t.word == w::EXTERN || self.specifier_word(t.word) {
            return true;
        }
        if self.cpp && (self.kind_at(1) == T::ColonColon || self.kind_at(1) == T::Lt) {
            return self.cpp_type_declaration_ahead();
        }
        if self.reserved(t.word) || matches!(t.word, w::TRUE | w::FALSE | w::TRUE_C | w::FALSE_C | w::NULL) {
            return false;
        }
        // A name: a type when a declarator follows it.
        let u = self.peek_at(1);
        match u.kind {
            T::Name => !self.cpp || !matches!(u.word, w::AND | w::OR | w::BITAND | w::BITOR | w::XOR | w::NOT_EQ | w::AND_EQ | w::OR_EQ | w::XOR_EQ),
            T::Star => self.pointer_declarator_ahead(1),
            T::Amp | T::AmpAmp if self.cpp => self.reference_declarator_ahead(1),
            T::LParen => self.macro_type_ahead(top) || self.paren_declarator_ahead(1, top),
            // `T [a](b, c);`: a function declarator named by a structured binding (its reading
            // wins where its parameters read).
            T::LBracket if self.cpp => {
                let Some(e) = self.paren_end(1) else { return false };
                (2..e).all(|i| if i % 2 == 0 { self.is_ident_at(i) } else { self.kind_at(i) == T::Comma }) && self.kind_at(e + 1) == T::LParen && self.params_at(e + 1)
            }
            _ => false,
        }
    }

    /// Whether `*`s from token `k` lead to a declarator: `T *x;`, `T **x = …`, `T *x[…]`, `T *f(…)`
    /// (C++: `&`, `&&`, and a qualified name: `T &A::f(…)`).
    fn pointer_declarator_ahead(&mut self, mut k: usize) -> bool {
        while matches!(self.kind_at(k), T::Star) || self.cpp && matches!(self.kind_at(k), T::Amp | T::AmpAmp) || self.peek_at(k).kind == T::Name && { let c = self.peek_at(k).word; self.is_qualifier_word(c) } {
            k += 1;
        }
        if self.cpp {
            if self.is_word_at(k, w::OPERATOR) {
                return true;
            }
            if self.kind_at(k) == T::LBracket && self.is_ident_at(k + 1) {
                // `auto &[a, b] = …`.
                return true;
            }
            if self.kind_at(k) == T::DotDotDot {
                return true;
            }
            if self.is_ident_at(k) && matches!(self.kind_at(k + 1), T::ColonColon) {
                let Some(e) = self.decl_name_end(k) else { return false };
                return matches!(self.kind_at(e), T::Semi | T::Comma | T::Eq | T::LBracket | T::LParen | T::RParen | T::LBrace | T::Colon);
            }
        }
        if !self.is_ident_at(k) {
            return self.kind_at(k) == T::LParen && self.kind_at(k + 1) == T::Star;
        }
        matches!(self.kind_at(k + 1), T::Semi | T::Comma | T::Eq | T::LBracket | T::LParen | T::RParen) || self.cpp && matches!(self.kind_at(k + 1), T::LBrace | T::Colon)
    }

    /// Whether `&`/`&&` from token `k` lead to a declarator a statement can hold: one with an
    /// initializer, a range `for`'s, or a function's (`a & b;` and `a & b)` are expressions).
    fn reference_declarator_ahead(&mut self, k: usize) -> bool {
        let mut j = k;
        while matches!(self.kind_at(j), T::Amp | T::AmpAmp | T::Star) || self.peek_at(j).kind == T::Name && { let c = self.peek_at(j).word; self.is_qualifier_word(c) } {
            j += 1;
        }
        if self.kind_at(j) == T::LBracket {
            return self.is_ident_at(j + 1);
        }
        if self.is_word_at(j, w::OPERATOR) {
            return true;
        }
        let e = if self.is_ident_at(j) && self.kind_at(j + 1) == T::ColonColon {
            match self.decl_name_end(j) {
                Some(e) => e,
                None => return false,
            }
        } else if self.is_ident_at(j) {
            j + 1
        } else {
            return false;
        };
        match self.kind_at(e) {
            T::Eq | T::LBrace | T::Colon => true,
            T::LParen => self.paren_end(e).is_some_and(|p| matches!(self.kind_at(p + 1), T::LBrace | T::Semi | T::Name | T::Arrow)),
            _ => false,
        }
    }

    /// The token index after a declarator's (qualified) name at `k`: `A::b`, `A<T>::B::c`,
    /// `A::~A`, `A::operator==`, `A::operator T`.
    pub(super) fn decl_name_end(&mut self, k: usize) -> Option<usize> {
        let mut j = k;
        loop {
            if self.kind_at(j) == T::Tilde {
                return self.is_ident_at(j + 1).then_some(j + 2);
            }
            if self.is_word_at(j, w::OPERATOR) {
                // To the parameters' `(` (`operator()(` past its own).
                let mut e = j + 1;
                if self.kind_at(e) == T::LParen && self.kind_at(e + 1) == T::RParen {
                    e += 2;
                }
                while !matches!(self.kind_at(e), T::LParen | T::Semi | T::LBrace | T::Eof) {
                    e += 1;
                }
                return Some(e);
            }
            if !self.is_ident_at(j) {
                return None;
            }
            j += 1;
            if self.kind_at(j) == T::Lt {
                j = self.angle_end(j)? + 1;
            }
            if self.kind_at(j) != T::ColonColon {
                return Some(j);
            }
            j += 1;
        }
    }

    /// Whether a C++ type's name ahead (`A::B`, `T<U>`, `::T`) is followed by a declarator.
    fn cpp_type_declaration_ahead(&mut self) -> bool {
        let e = if self.is_word(w::DECLTYPE) {
            let Some(p) = self.paren_end(1) else { return false };
            match self.class_name_end(p + 2) {
                Some(e) => e,
                None => return false,
            }
        } else {
            match self.class_name_end(0) {
                Some(e) => e,
                None => return false,
            }
        };
        match self.kind_at(e) {
            T::Name => {
                let c = self.peek_at(e).word;
                (!self.reserved(c) || self.is_qualifier_word(c) || c == w::OPERATOR) && !matches!(c, w::AND | w::OR | w::BITAND | w::BITOR | w::XOR | w::NOT_EQ)
            }
            T::Star => self.pointer_declarator_ahead(e),
            T::Amp | T::AmpAmp => self.reference_declarator_ahead(e),
            T::DotDotDot => true,
            _ => false,
        }
    }

    /// Whether `(` at token `k` holds a declarator after a type: `T (*f)(…)`, `T (x);` is a call.
    /// Where not `top`, only a declarator no expression reads (`T (*a[])(…)`).
    fn paren_declarator_ahead(&mut self, k: usize, top: bool) -> bool {
        // `T (x) {` (a macro's function: `TEST(name) {…}`), `T (x) = …`.
        if self.is_ident_at(k + 1) && self.kind_at(k + 2) == T::RParen {
            return top && matches!(self.kind_at(k + 3), T::LBrace | T::Eq);
        }
        // C++: `T (A::*p)(…)`, a pointer to a member.
        if self.cpp && self.is_ident_at(k + 1) && self.kind_at(k + 2) == T::ColonColon && self.kind_at(k + 3) == T::Star {
            return true;
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
        let typed = self.kind_at(e + 1) == T::LParen && self.cpp && self.peek_at(e + 2).kind == T::Name && { let c = self.peek_at(e + 2).word; self.specifier_word(c) && !is_storage(c) }
            || self.kind_at(e + 1) == T::LParen && self.is_ident_at(e + 2) && self.kind_at(e + 3) == T::Star && {
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
            // A declaration's direct declarator is in `_declarator` in C++, in none in C.
            let ctx = if self.cpp { Ctx::Decl } else { Ctx::Bare };
            let d = self.declarator(Fam::Decl, slot(f::DECLARATOR, ctx))?;
            // A function's definition: its declarator (a function declarator, or one around it:
            // `char *f(…)`) and a body.
            if first && body && !self.cpp && d.kind == K::FunctionDeclarator && self.peek() == T::Name && !self.is_attribute() && !self.is_asm() {
                // An old-style definition: its parameters' declarations, then the body.
                self.rekind_keep(n.at, K::FunctionDefinition);
                self.rekey(d.at, slot(f::DECLARATOR, Ctx::Bare));
                while self.peek() != T::LBrace {
                    if !self.declaration_ahead() {
                        return self.refuse();
                    }
                    self.declaration_item(Level::Block, K::FunctionDefinition)?;
                }
                return self.function_body();
            }
            if first && body && self.cpp && self.holds_function(d.at) && self.method_body_ahead() {
                self.rekind_keep(n.at, K::FunctionDefinition);
                self.rekey(d.at, slot(f::DECLARATOR, Ctx::Decl));
                return self.method_body(K::FunctionDefinition, false);
            }
            if first && body && !self.cpp && self.function_body_ahead() {
                self.rekind_keep(n.at, K::FunctionDefinition);
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
                // `T x(args)` (where the parentheses are no parameters), `T x{…}`.
                T::LParen if self.cpp => {
                    let i = self.wrap(d, K::InitDeclarator, slot(f::DECLARATOR, Ctx::Bare), slot(f::DECLARATOR, Ctx::Decl));
                    self.arguments(slot(f::VALUE, Ctx::Bare))?;
                    self.close(i);
                }
                T::LBrace if self.cpp => {
                    let i = self.wrap(d, K::InitDeclarator, slot(f::DECLARATOR, Ctx::Bare), slot(f::DECLARATOR, Ctx::Decl));
                    self.initializer_list(slot(f::VALUE, Ctx::Bare))?;
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
    pub(super) fn type_definition(&mut self) -> R<()> {
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
                T::ColonColon if self.cpp && ty.is_none() => {
                    let at = self.w.len();
                    if !self.type_specifier(slot(f::TYPE, Ctx::Type))? {
                        return self.refuse();
                    }
                    ty = Some(at);
                    continue;
                }
                _ => return Ok(ty),
            }
            let c = t.word;
            if self.cpp && c == w::VIRTUAL {
                self.leaf(K::Tvirtual, f::NONE, C_NONE)?;
            } else if self.cpp && c == w::EXPLICIT {
                let n = self.open(K::ExplicitFunctionSpecifier, f::NONE, C_NONE);
                self.kw(K::ExplicitFunctionSpecifier, w::EXPLICIT)?;
                self.close(n);
            } else if self.cpp && ty.is_none() && matches!(c, w::AUTO | w::DECLTYPE | w::TYPENAME | w::CLASS) {
                let at = self.w.len();
                if !self.type_specifier(slot(f::TYPE, Ctx::Type))? {
                    return self.refuse();
                }
                ty = Some(at);
            } else if is_storage(c) && !(self.cpp && c == w::AUTO) {
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
        if self.cpp && self.cpp_type(s)? {
            return Ok(true);
        }
        let t = self.tok();
        if t.kind != T::Name {
            return Ok(false);
        }
        match t.word {
            w::STRUCT | w::UNION => self.struct_specifier(s)?,
            w::CLASS if self.cpp => self.struct_specifier(s)?,
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
            } else if self.cpp && !ty && self.is_qualifier_word(t.word) && self.kind_at(1) == T::Name && is_primitive(self.peek_at(1).word) {
                // `unsigned const char`: the qualifier inside.
                self.qualifier()?;
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
        let kind = match self.tok().word {
            w::STRUCT => K::StructSpecifier,
            w::CLASS => K::ClassSpecifier,
            _ => K::UnionSpecifier,
        };
        let n = self.open_in(kind, s);
        self.kw_any(kind)?;
        loop {
            if self.is_word(w::U_DECLSPEC) {
                self.declspec()?;
            } else if self.is_attribute() {
                self.attribute_specifier()?;
            } else if self.cpp && (self.is_word(w::ALIGNAS) || self.is_word(w::ALIGNAS_)) {
                self.qualifier()?;
            } else if self.cpp && self.peek() == T::LBracket && self.kind_at(1) == T::LBracket {
                self.attribute_declaration()?;
            } else {
                break;
            }
        }
        let mut named = false;
        if self.cpp {
            // `_class_name`: `X`, `A::X`, `X<T>`; then `final`, bases.
            if (self.is_ident() || self.peek() == T::ColonColon) && !self.is_word(w::FINAL) {
                self.class_name(slot(f::NAME, Ctx::Bare))?;
                named = true;
            } else if self.is_word(w::FINAL) && self.is_ident_at(1) {
                self.class_name(slot(f::NAME, Ctx::Bare))?;
                named = true;
            }
            if self.is_word(w::FINAL) || self.is_word(w::OVERRIDE) {
                let v = self.open(K::VirtualSpecifier, f::NONE, C_NONE);
                self.kw_any(K::VirtualSpecifier)?;
                self.close(v);
            }
            if self.peek() == T::Colon && (self.kind_at(1) == T::Name || self.kind_at(1) == T::ColonColon || self.kind_at(1) == T::LBracket) {
                self.base_classes()?;
                if self.peek() != T::LBrace {
                    return self.refuse();
                }
            }
        } else if self.is_ident() {
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
            let mut first = true;
            loop {
                if self.peek() == T::Colon {
                    // An unnamed bit field.
                    self.bitfield()?;
                } else {
                    let d = self.declarator(Fam::Field, slot(f::DECLARATOR, Ctx::FDecl))?;
                    if self.cpp && first && self.holds_function(d.at) && self.method_body_ahead() {
                        // An inline method's definition.
                        self.rekind_keep(n.at, K::FunctionDefinition);
                        self.method_body(K::FunctionDefinition, false)?;
                        self.close(n);
                        return Ok(());
                    }
                    if self.peek() == T::Colon {
                        self.bitfield()?;
                    } else if self.cpp && self.peek() == T::Eq {
                        self.tk(K::FieldDeclaration, K::TEq)?;
                        if self.peek() == T::LBrace {
                            self.initializer_list(slot(f::DEFAULT_VALUE, Ctx::Bare))?;
                        } else {
                            self.expr(slot(f::DEFAULT_VALUE, Ctx::Expr))?;
                        }
                    } else if self.cpp && self.peek() == T::LBrace {
                        self.initializer_list(slot(f::DEFAULT_VALUE, Ctx::Bare))?;
                    }
                }
                first = false;
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

    /// Whether the declarator written from node `at` holds a function declarator.
    pub(super) fn holds_function(&self, at: usize) -> bool {
        (at..self.w.len()).any(|i| {
            let k = unkey(self.w.key(i)).0;
            k == K::FunctionDeclarator as u16
        })
    }

    /// Whether a method's body follows its declarator: `{`, `try`, `= default`, `= delete`, `= 0`.
    pub(super) fn method_body_ahead(&mut self) -> bool {
        match self.peek() {
            // `f() { };`: an empty initializer list and the field's `;` (tree-sitter's choice).
            T::LBrace => !(self.kind_at(1) == T::RBrace && self.kind_at(2) == T::Semi),
            T::Name => self.is_word(w::TRY),
            T::Eq => {
                let t = self.peek_at(1);
                // `= 0;` is a default value (a pure virtual clause only where no type is).
                t.kind == T::Name && matches!(t.word, w::DEFAULT | w::DELETE)
            }
            _ => false,
        }
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
        if self.cpp {
            if self.is_word(w::CLASS) || self.is_word(w::STRUCT) {
                self.kw_any(K::EnumSpecifier)?;
            }
            if self.is_ident() || self.peek() == T::ColonColon {
                self.class_name(slot(f::NAME, Ctx::Bare))?;
                named = true;
                if self.peek() == T::Colon {
                    self.tk(K::EnumSpecifier, K::TColon)?;
                    let t = self.tok();
                    if t.kind == T::Name && is_sized(t.word) {
                        self.sized(slot(f::BASE, Ctx::Bare))?;
                    } else if t.kind == T::Name && is_primitive(t.word) {
                        self.leaf(K::PrimitiveType, f::BASE, C_NONE)?;
                    } else {
                        self.class_name(slot(f::BASE, Ctx::Bare))?;
                    }
                }
            }
        } else if self.is_ident() {
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
            // C++: any word names an attribute (`noreturn`, `fallthrough`).
            if self.cpp && self.peek() == T::Name && self.kind_at(1) == T::ColonColon {
                self.leaf(K::Identifier, f::PREFIX, C_NONE)?;
                self.tk(K::Attribute, K::TColonColon)?;
            }
            if !self.is_ident() && !(self.cpp && self.peek() == T::Name) {
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
            T::Amp | T::AmpAmp if self.cpp => {
                let (kind, rule) = match fam {
                    Fam::Decl => (K::ReferenceDeclarator, K::ReferenceDeclarator),
                    Fam::Field => (K::ReferenceDeclarator, K::ReferenceFieldDeclarator),
                    Fam::Type => (K::ReferenceDeclarator, K::ReferenceTypeDeclarator),
                    Fam::Abstract => (K::AbstractReferenceDeclarator, K::AbstractReferenceDeclarator),
                };
                // `T &... args`: a variadic reference, in no supertype.
                let variadic = fam == Fam::Decl && self.kind_at(1) == T::DotDotDot;
                let n = if variadic { self.open(kind, s.field, C_NONE) } else { self.open_in(kind, s) };
                let k = punct(t.kind).expect("a reference");
                self.tk(if variadic { K::VariadicReferenceDeclarator } else { rule }, k)?;
                if variadic {
                    self.variadic_declarator()?;
                } else if fam != Fam::Abstract || self.abstract_continues() {
                    let p = std::mem::replace(&mut self.in_pointer, true);
                    self.declarator(fam, slot(f::NONE, fam.ctx()))?;
                    self.in_pointer = p;
                }
                self.close(n);
                return Ok(n);
            }
            T::DotDotDot if self.cpp && fam == Fam::Decl => {
                let n = self.variadic_declarator_in(s.field)?;
                return Ok(n);
            }
            T::LBracket if self.cpp && fam == Fam::Decl && self.kind_at(1) != T::LBracket => {
                let n = self.open_in(K::StructuredBindingDeclarator, s);
                self.list(K::StructuredBindingDeclarator, T::RBracket, true, false, |p| {
                    if !p.is_ident() {
                        return p.refuse();
                    }
                    p.leaf(K::Identifier, f::NONE, C_NONE)?;
                    Ok(())
                })?;
                self.close(n);
                n
            }
            T::Tilde | T::ColonColon if self.cpp && fam == Fam::Decl => self.qname(cpp::Role::Decl, s)?,
            T::Name if self.cpp && fam == Fam::Decl && (t.word == w::OPERATOR || !self.reserved(t.word) && (self.kind_at(1) == T::ColonColon || self.kind_at(1) == T::Lt && self.angle_end(1).is_some())) => {
                self.qname(cpp::Role::Decl, s)?
            }
            T::Name if self.cpp && fam == Fam::Field && (t.word == w::OPERATOR || !self.reserved(t.word) && self.kind_at(1) == T::Lt && self.angle_end(1).is_some()) => {
                self.qname(cpp::Role::Field, s)?
            }
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
                        // C++'s tapes imply `__restrict` (its node's one word).
                        if self.cpp && k == K::MsRestrictModifier {
                            self.bump()?;
                        } else {
                            self.kw_any(k)?;
                        }
                        self.close(k2);
                        self.close(m);
                    } else if self.is_qualifier_word(t.word) {
                        self.qualifier()?;
                    } else {
                        break;
                    }
                }
                if fam != Fam::Abstract || self.abstract_continues() {
                    let p = std::mem::replace(&mut self.in_pointer, true);
                    self.declarator(fam, inner)?;
                    self.in_pointer = p;
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
                    if self.cpp {
                        self.function_tail(K::AbstractFunctionDeclarator)?;
                    }
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
                T::LParen if self.cpp => {
                    // `T x(a + 1)`: an initializer's arguments, no parameters.
                    if fam == Fam::Decl && !self.params_ahead() {
                        return Ok(d);
                    }
                    let kind = fam.kind(K::FunctionDeclarator);
                    let n = self.wrap(d, kind, s, inner);
                    self.parameter_list()?;
                    self.function_tail(kind)?;
                    self.close(n);
                    d = n;
                }
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

    /// `...` and a name: a variadic declarator (in no supertype).
    fn variadic_declarator(&mut self) -> R<N> {
        self.variadic_declarator_in(f::NONE)
    }

    fn variadic_declarator_in(&mut self, field: u8) -> R<N> {
        let n = self.open(K::VariadicDeclarator, field, C_NONE);
        self.tk(K::VariadicDeclarator, K::TDotDotDot)?;
        if self.is_ident() {
            self.leaf(K::Identifier, f::NONE, C_NONE)?;
        }
        self.close(n);
        Ok(n)
    }

    /// Whether the parentheses ahead hold parameters (a function declarator's), not an
    /// initializer's arguments: each item starts as a type does and holds no operator or literal
    /// outside brackets (before a default's `=`).
    fn params_ahead(&mut self) -> bool {
        let split = self.in_pointer;
        self.params_at_with(0, split)
    }

    /// `params_ahead` for the parentheses at token `k`.
    pub(super) fn params_at(&mut self, k: usize) -> bool {
        self.params_at_with(k, false)
    }

    /// `params_at`, where `split` says the parse has forked before the first word (so a literal's
    /// word reads as a type's name there too).
    fn params_at_with(&mut self, k: usize, split: bool) -> bool {
        let Some(e) = self.paren_end(k) else { return false };
        if e == k + 1 {
            return true;
        }
        let mut j = k + 1;
        let mut item_start = true;
        let mut depth = 0usize;
        let mut default = false;
        // The item's names (a qualified name's parts as one): a `*` or `&` after two is an
        // operator's (`a * b`).
        let mut names = 0usize;
        // Whether the item's first word is a type's keyword: no expression reads it (`void (*f)()`).
        let mut hard = false;
        // Whether an initializer's reading survives to the end (a `;` or `,` after): a tie.
        let tie = matches!(self.kind_at(e + 1), T::Semi | T::Comma);
        while j < e {
            let t = self.peek_at(j);
            if item_start {
                item_start = false;
                hard = t.kind == T::Name && { let c = t.word; self.specifier_word(c) && !is_storage(c) };
                let ok = match t.kind {
                    // A literal's word (`true`, `nullptr`, `this`, `sizeof` …) first: an
                    // initializer's (tree-sitter lexes the first token once, as the keyword); later,
                    // a type's name where the other items are parameters (its parameter reading
                    // lexes it as a name, and wins the tie).
                    T::Name if is_literal_word(t.word) => split || j > k + 1,
                    T::Name => {
                        let c = t.word;
                        !self.reserved(c) || self.specifier_word(c) || is_primitive(c) || is_sized(c) || matches!(c, w::AUTO | w::DECLTYPE | w::TYPENAME | w::CONST | w::VOLATILE)
                    }
                    T::ColonColon | T::DotDotDot => true,
                    T::LBracket => self.kind_at(j + 1) == T::LBracket,
                    _ => false,
                };
                if !ok {
                    return false;
                }
            }
            // A parenthesized group after a type's name at the parameters' depth: a function
            // type's parameters (`T(U, V)`), themselves parameters; `(*f)` is a declarator.
            // A parenthesized declarator (`T (&x)`) loses tree-sitter's tie (its precedence -10).
            if depth == 0 && !default && !hard && tie && t.kind == T::LParen && j > k + 1 && matches!(self.kind_at(j - 1), T::Name | T::Gt) && self.paren_declarator_group(j) {
                return false;
            }
            if depth == 0 && !default && t.kind == T::LParen && j > k + 1 && matches!(self.kind_at(j - 1), T::Name | T::Gt) && !self.paren_declarator_group(j) {
                if !self.params_at_with(j, true) {
                    return false;
                }
                j = self.paren_end(j).expect("balanced") + 1;
                continue;
            }
            // `{` at the parameters' depth is an initializer's (`T x(U{})`).
            if t.kind == T::LBrace && depth == 0 && !default {
                return false;
            }
            match t.kind {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => depth = depth.saturating_sub(1),
                T::Lt if depth == 0 && !default => {
                    // Template arguments.
                    match self.angle_end(j) {
                        Some(g) => {
                            j = g + 1;
                            continue;
                        }
                        None => return false,
                    }
                }
                T::Comma if depth == 0 => {
                    item_start = true;
                    default = false;
                    names = 0;
                }
                T::Name if depth == 0 && !default && !self.reserved(t.word) => names += 1,
                T::ColonColon if depth == 0 && !default => names = names.saturating_sub(1),
                T::Star | T::Amp | T::AmpAmp if depth == 0 && !default && names >= 2 => return false,
                T::Eq if depth == 0 => default = true,
                _ if depth == 0 && !default => {
                    let bad = matches!(
                        t.kind,
                        T::Number | T::Str | T::Char | T::RawStr | T::Dot | T::Arrow | T::Plus | T::Minus | T::Slash | T::Percent | T::Bang | T::Question | T::Pipe | T::PipePipe | T::Caret | T::EqEq | T::Ne | T::Gt | T::Ge | T::Le | T::LtLt | T::GtGt | T::PlusPlus | T::MinusMinus | T::Tilde
                    );
                    if bad {
                        return false;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        true
    }

    /// Whether the parentheses at token `k` hold a declarator alone: `(*f)`, `(&a)[3]`'s `(&a)`,
    /// `(**p)`, `(*)`.
    fn paren_declarator_group(&mut self, k: usize) -> bool {
        if !matches!(self.kind_at(k + 1), T::Star | T::Amp | T::AmpAmp | T::Caret) {
            return false;
        }
        let Some(e) = self.paren_end(k) else { return false };
        let mut j = k + 1;
        while matches!(self.kind_at(j), T::Star | T::Amp | T::AmpAmp | T::Caret) || self.peek_at(j).kind == T::Name && { let c = self.peek_at(j).word; self.is_qualifier_word(c) } {
            j += 1;
        }
        if self.is_ident_at(j) {
            j += 1;
        }
        while self.kind_at(j) == T::LBracket {
            match self.paren_end(j) {
                Some(b) => j = b + 1,
                None => return false,
            }
        }
        j == e
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
        matches!(self.peek(), T::Star | T::LBracket) || self.peek() == T::LParen || self.cpp && matches!(self.peek(), T::Amp | T::AmpAmp)
    }

    /// Whether `(` ahead opens an abstract parenthesized declarator (`(*)`, `(*)[…]`, `([…])`), not
    /// parameters.
    fn abstract_paren_ahead(&mut self) -> bool {
        matches!(self.kind_at(1), T::Star | T::LBracket) || self.cpp && matches!(self.kind_at(1), T::Amp | T::AmpAmp) || self.kind_at(1) == T::LParen && self.kind_at(2) != T::RParen && !self.type_in_parens_ahead(1)
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
        if self.cpp {
            // C++'s `...` is a bare token: a delimiter beside it is stored.
            let dots = |p: &mut Self, k: usize| p.kind_at(k) == T::DotDotDot;
            if dots(self, 1) {
                self.leaf(K::TLParen, f::NONE, C_NONE)?;
            } else {
                self.punct_tk(K::ParameterList, T::LParen, false)?;
            }
            let mut last_dots = false;
            while self.peek() != T::RParen {
                if self.peek() == T::DotDotDot {
                    self.leaf(K::TDotDotDot, f::NONE, C_NONE)?;
                    last_dots = true;
                } else {
                    self.parameter_declaration()?;
                    last_dots = false;
                }
                if self.peek() != T::Comma {
                    break;
                }
                if dots(self, 1) {
                    self.leaf(K::TComma, f::NONE, C_NONE)?;
                } else {
                    self.tk(K::ParameterList, K::TComma)?;
                }
            }
            if self.peek() != T::RParen {
                return self.refuse();
            }
            if last_dots {
                self.leaf(K::TRParen, f::NONE, C_NONE)?;
            } else {
                self.tk(K::ParameterList, K::TRParen)?;
            }
            self.close(n);
            return Ok(false);
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

    pub(super) fn parameter_declaration(&mut self) -> R<()> {
        let n = self.open(K::ParameterDeclaration, f::NONE, C_NONE);
        if self.cpp && self.peek() == T::Name && is_literal_word(self.tok().word) {
            self.leaf(K::TypeIdentifier, f::TYPE, class(K::TypeIdentifier, Ctx::Type))?;
        } else if self.specifiers(K::ParameterDeclaration)?.is_none() {
            return self.refuse();
        }
        let closers = |p: &mut Self| matches!(p.peek(), T::Comma | T::RParen) || p.cpp && matches!(p.peek(), T::Eq | T::Gt | T::GtGt);
        if !closers(self) && !self.is_attribute() {
            if self.cpp && (self.peek() == T::DotDotDot || matches!(self.peek(), T::Amp | T::AmpAmp) && self.kind_at(1) == T::DotDotDot) {
                // `T... args`, `T&&... args`.
                self.rekind(n.at, K::VariadicParameterDeclaration, BARE);
                self.declarator(Fam::Decl, slot(f::DECLARATOR, Ctx::Bare))?;
            } else if self.names_ahead() {
                self.declarator(Fam::Decl, slot(f::DECLARATOR, Ctx::Decl))?;
            } else {
                self.declarator(Fam::Abstract, slot(f::DECLARATOR, Ctx::ADecl))?;
            }
        }
        if self.cpp && self.peek() == T::Eq {
            self.rekind(n.at, K::OptionalParameterDeclaration, BARE);
            self.tk(K::OptionalParameterDeclaration, K::TEq)?;
            if self.peek() == T::LBrace {
                self.initializer_list(slot(f::DEFAULT_VALUE, Ctx::Bare))?;
            } else {
                self.expr(slot(f::DEFAULT_VALUE, Ctx::Expr))?;
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
                // C++: `(a, b)`, `(A::b …`, `(T<U> …`: parameters.
                T::LParen if self.cpp && self.is_ident_at(k + 1) && matches!(self.kind_at(k + 2), T::Comma | T::ColonColon | T::Lt | T::Name | T::Amp | T::AmpAmp | T::LParen) => return false,
                T::Amp | T::AmpAmp if self.cpp => k += 1,
                T::Name if self.cpp && t.word == w::OPERATOR => return true,
                T::ColonColon if self.cpp => return true,
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
        if matches!(self.peek(), T::Star | T::LBracket | T::LParen) || self.cpp && matches!(self.peek(), T::Amp | T::AmpAmp) {
            self.declarator(Fam::Abstract, slot(f::DECLARATOR, Ctx::ADecl))?;
        }
        self.close(n);
        Ok(n)
    }

    /// Whether the parentheses at token `k` hold a type (a cast's, `sizeof`'s): a type's keyword
    /// or qualifier first, or a name then `*`s and `)`, or a name and `)` where `cast` allows.
    pub(super) fn type_in_parens_ahead(&mut self, k: usize) -> bool {
        let t = self.peek_at(k + 1);
        if self.cpp && (t.kind == T::ColonColon || t.kind == T::Name && !self.reserved(t.word) && matches!(self.kind_at(k + 2), T::ColonColon | T::Lt)) {
            // A qualified or template type's name, its declarator's `*`/`&` and qualifiers.
            let Some(e) = self.class_name_end(k + 1) else { return false };
            let mut j = e;
            while matches!(self.kind_at(j), T::Star | T::Amp | T::AmpAmp) || self.peek_at(j).kind == T::Name && { let c = self.peek_at(j).word; self.is_qualifier_word(c) } {
                j += 1;
            }
            // `(A::b)` alone: the caller decides by what follows.
            return j > e && self.kind_at(j) == T::RParen;
        }
        if t.kind != T::Name {
            return false;
        }
        if self.specifier_word(t.word) && !is_storage(t.word) && t.word != w::TYPEDEF && !is_ms_call(t.word) {
            // C++: `(int(x) …`, `(T{…} …` are functional casts.
            if self.cpp && is_primitive(t.word) && (self.kind_at(k + 2) == T::LBrace || self.kind_at(k + 2) == T::LParen && !matches!(self.kind_at(k + 3), T::Star | T::Amp | T::AmpAmp | T::Caret | T::RParen)) {
                return false;
            }
            return true;
        }
        if self.reserved(t.word) || matches!(t.word, w::TRUE | w::FALSE | w::TRUE_C | w::FALSE_C | w::NULL) {
            return false;
        }
        // `(T (*)(…))`: a function pointer's type.
        if self.kind_at(k + 2) == T::LParen && matches!(self.kind_at(k + 3), T::Star | T::Caret) && self.kind_at(k + 4) == T::RParen {
            return true;
        }
        let mut j = k + 2;
        while self.kind_at(j) == T::Star || self.peek_at(j).kind == T::Name && { let c = self.peek_at(j).word; self.is_qualifier_word(c) } {
            j += 1;
        }
        if j > k + 2 {
            return self.kind_at(j) == T::RParen;
        }
        // C++: `(T&)x`, `(T&&)x`.
        if self.cpp && matches!(self.kind_at(k + 2), T::Amp | T::AmpAmp) && self.kind_at(k + 3) == T::RParen {
            return true;
        }
        // `(T(CALL *)(…))`: a function pointer's type, a calling convention's name inside.
        if self.kind_at(k + 2) == T::LParen && self.is_ident_at(k + 3) && matches!(self.kind_at(k + 4), T::Star | T::Caret) && self.kind_at(k + 5) == T::RParen && self.kind_at(k + 6) == T::LParen {
            return true;
        }
        // `(T)`: a type or an expression; the caller decides by what follows.
        false
    }
}
