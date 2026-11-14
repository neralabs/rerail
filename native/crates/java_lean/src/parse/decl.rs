//! Declarations: the file's (`package`, `import`, `module`, a method at the top level), types
//! (classes, interfaces, enums, records, annotation types) and their bodies' members, modifiers,
//! and parameters.

use super::*;

/// What a declaration ahead declares.
#[derive(Clone, Copy, Debug)]
pub(super) enum Decl {
    /// A local variable (a `local_variable_declaration`).
    Local,
    /// A type, of this kind.
    Type(K),
}

/// What a member of a class body ahead is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Member {
    Type(K),
    Method,
    Constructor,
    Compact,
    Field,
}

fn is_modifier_word(code: u8) -> bool {
    matches!(
        code,
        w::PUBLIC | w::PROTECTED | w::PRIVATE | w::ABSTRACT | w::STATIC | w::FINAL | w::STRICTFP | w::DEFAULT | w::SYNCHRONIZED | w::NATIVE | w::TRANSIENT | w::VOLATILE
    )
}

impl<'a> P<'a> {
    // ---- Look aheads.

    /// Whether `non-sealed` (three tokens that touch) starts `k` ahead.
    fn non_sealed_at(&mut self, k: usize) -> bool {
        self.is_word_at(k, w::NON)
            && self.kind_at(k + 1) == T::Minus
            && self.is_word_at(k + 2, w::SEALED)
            && self.peek_at(k).end == self.peek_at(k + 1).start
            && self.peek_at(k + 1).end == self.peek_at(k + 2).start
    }

    /// Whether `sealed` `k` ahead is a modifier: before a declaration's word or another modifier.
    fn sealed_at(&mut self, k: usize) -> bool {
        if !self.is_word_at(k, w::SEALED) {
            return false;
        }
        let t = self.peek_at(k + 1);
        t.kind == T::At || t.kind == T::AtInterface || (t.kind == T::Name && (is_modifier_word(t.word) || matches!(t.word, w::CLASS | w::INTERFACE | w::SEALED | w::RECORD | w::ENUM | w::NON)))
    }

    /// Past modifiers (annotations and modifier words) from `k` ahead: where they end.
    pub(super) fn scan_modifiers(&mut self, mut k: usize) -> Option<usize> {
        loop {
            let t = self.peek_at(k);
            match t.kind {
                T::At => k = self.scan_annotations(k)?,
                T::Name if is_modifier_word(t.word) => {
                    // `synchronized (…)` is a statement.
                    if t.word == w::SYNCHRONIZED && self.kind_at(k + 1) == T::LParen {
                        return Some(k);
                    }
                    k += 1;
                }
                T::Name if self.sealed_at(k) => k += 1,
                T::Name if self.non_sealed_at(k) => k += 3,
                _ => return Some(k),
            }
        }
    }

    /// A type declaration's kind, where its word is `k` ahead (`record` before a name and its
    /// parameters).
    fn type_word_at(&mut self, k: usize) -> Option<K> {
        let t = self.peek_at(k);
        match t.kind {
            T::AtInterface => Some(K::AnnotationTypeDeclaration),
            T::Name => match t.word {
                w::CLASS => Some(K::ClassDeclaration),
                w::INTERFACE => Some(K::InterfaceDeclaration),
                w::ENUM => Some(K::EnumDeclaration),
                w::RECORD if self.is_ident_at(k + 1) && matches!(self.kind_at(k + 2), T::LParen | T::Lt) => Some(K::RecordDeclaration),
                _ => None,
            },
            _ => None,
        }
    }

    /// A declaration in a statement's place `k` ahead: a type's, or a local variable's (modifiers,
    /// a type, a name and what follows a declarator's name).
    pub(super) fn declaration_ahead(&mut self, k: usize) -> Option<Decl> {
        let j = self.scan_modifiers(k)?;
        if let Some(kind) = self.type_word_at(j) {
            return Some(Decl::Type(kind));
        }
        let e = self.scan_type(j, true)?;
        let t = self.peek_at(e);
        if !(self.is_ident_at(e) || self.underscore(t)) {
            return None;
        }
        if j > k || matches!(self.kind_at(e + 1), T::Eq | T::Semi | T::Comma | T::LBracket | T::Colon | T::At) {
            return Some(Decl::Local);
        }
        None
    }

    pub(super) fn declaration_ahead_at(&mut self, k: usize) -> Option<Decl> {
        self.declaration_ahead(k)
    }

    /// A class body's member ahead.
    fn member_ahead(&mut self) -> Option<Member> {
        let j = self.scan_modifiers(0)?;
        if let Some(kind) = self.type_word_at(j) {
            return Some(Member::Type(kind));
        }
        let mut j = j;
        if self.kind_at(j) == T::Lt {
            j = self.scan_type_arguments(j)?;
            j = self.scan_annotations(j)?;
            if self.is_ident_at(j) && self.kind_at(j + 1) == T::LParen {
                return Some(Member::Constructor);
            }
            return Some(Member::Method);
        }
        if self.is_ident_at(j) && self.kind_at(j + 1) == T::LParen {
            return Some(Member::Constructor);
        }
        if self.is_ident_at(j) && self.kind_at(j + 1) == T::LBrace {
            return Some(Member::Compact);
        }
        let e = self.scan_type(j, true)?;
        if self.kind_at(e + 1) == T::LParen {
            return Some(Member::Method);
        }
        Some(Member::Field)
    }

    // ---- The file's statements.

    /// A `_toplevel_statement`: a statement, or a method declaration.
    pub(super) fn toplevel(&mut self) -> R<()> {
        let j = self.scan_annotations(0).unwrap_or(0);
        if self.is_word_at(j, w::PACKAGE) {
            return self.package_declaration();
        }
        if self.is_word(w::IMPORT) {
            return self.import_declaration();
        }
        if (self.is_word_at(j, w::OPEN) && self.is_word_at(j + 1, w::MODULE)) || (self.is_word_at(j, w::MODULE) && self.is_ident_at(j + 1)) {
            return self.module_declaration();
        }
        if let Some(j) = self.scan_modifiers(0) {
            let method = if self.kind_at(j) == T::Lt {
                true
            } else if self.type_word_at(j).is_none() {
                self.scan_type(j, true).is_some_and(|e| self.is_ident_at(e) && self.kind_at(e + 1) == T::LParen)
            } else {
                false
            };
            if method {
                self.method_declaration(BARE)?;
                return Ok(());
            }
        }
        self.statement(STMT)?;
        Ok(())
    }

    fn package_declaration(&mut self) -> R<()> {
        let n = self.open_in(K::PackageDeclaration, STMT);
        while self.peek() == T::At {
            self.annotation(BARE)?;
        }
        self.kw(K::PackageDeclaration, w::PACKAGE)?;
        self.name(BARE)?;
        self.punct_tk(K::PackageDeclaration, T::Semi, false)?;
        self.close(n);
        Ok(())
    }

    fn import_declaration(&mut self) -> R<()> {
        let n = self.open_in(K::ImportDeclaration, STMT);
        self.kw(K::ImportDeclaration, w::IMPORT)?;
        if self.is_word(w::STATIC) {
            self.kw(K::ImportDeclaration, w::STATIC)?;
        }
        self.name(BARE)?;
        if self.peek() == T::Dot && self.kind_at(1) == T::Star {
            self.tk(K::ImportDeclaration, K::TDot)?;
            let a = self.open(K::Asterisk, f::NONE, C_NONE);
            self.leaf(K::TStar, f::NONE, C_NONE)?;
            self.close(a);
        }
        self.punct_tk(K::ImportDeclaration, T::Semi, false)?;
        self.close(n);
        Ok(())
    }

    fn module_declaration(&mut self) -> R<()> {
        let n = self.open_in(K::ModuleDeclaration, STMT);
        while self.peek() == T::At {
            self.annotation(BARE)?;
        }
        if self.is_word(w::OPEN) {
            self.kw(K::ModuleDeclaration, w::OPEN)?;
        }
        self.kw(K::ModuleDeclaration, w::MODULE)?;
        self.name(slot(f::NAME, Ctx::Bare))?;
        let b = self.open(K::ModuleBody, f::BODY, C_NONE);
        self.punct_tk(K::ModuleBody, T::LBrace, false)?;
        while self.peek() != T::RBrace {
            self.module_directive()?;
        }
        self.punct_tk(K::ModuleBody, T::RBrace, false)?;
        self.close(b);
        self.close(n);
        Ok(())
    }

    fn module_directive(&mut self) -> R<()> {
        let d = slot(f::NONE, Ctx::Directive);
        let t = self.tok();
        if t.kind != T::Name {
            return self.refuse();
        }
        match t.word {
            w::REQUIRES => {
                let n = self.open_in(K::RequiresModuleDirective, d);
                self.kw(K::RequiresModuleDirective, w::REQUIRES)?;
                while (self.is_word(w::TRANSITIVE) || self.is_word(w::STATIC)) && self.is_ident_at(1) {
                    let m = self.open(K::RequiresModifier, f::MODIFIERS, C_NONE);
                    let k = if self.is_word(w::STATIC) { K::TStatic } else { K::TTransitive };
                    self.leaf(k, f::NONE, C_NONE)?;
                    self.close(m);
                }
                self.name(slot(f::MODULE, Ctx::Bare))?;
                self.punct_tk(K::RequiresModuleDirective, T::Semi, false)?;
                self.close(n);
            }
            w::EXPORTS | w::OPENS => {
                let (kind, word) = if t.word == w::EXPORTS { (K::ExportsModuleDirective, w::EXPORTS) } else { (K::OpensModuleDirective, w::OPENS) };
                let n = self.open_in(kind, d);
                self.kw(kind, word)?;
                self.name(slot(f::PACKAGE, Ctx::Bare))?;
                if self.is_word(w::TO) {
                    self.kw(kind, w::TO)?;
                    loop {
                        self.name(slot(f::MODULES, Ctx::Bare))?;
                        if self.peek() != T::Comma {
                            break;
                        }
                        self.tk(kind, K::TComma)?;
                    }
                }
                self.punct_tk(kind, T::Semi, false)?;
                self.close(n);
            }
            w::USES => {
                let n = self.open_in(K::UsesModuleDirective, d);
                self.kw(K::UsesModuleDirective, w::USES)?;
                self.name(slot(f::TYPE, Ctx::Bare))?;
                self.punct_tk(K::UsesModuleDirective, T::Semi, false)?;
                self.close(n);
            }
            w::PROVIDES => {
                let n = self.open_in(K::ProvidesModuleDirective, d);
                self.kw(K::ProvidesModuleDirective, w::PROVIDES)?;
                self.name(slot(f::PROVIDED, Ctx::Bare))?;
                self.kw(K::ProvidesModuleDirective, w::WITH)?;
                self.name(BARE)?;
                while self.peek() == T::Comma {
                    self.tk(K::ProvidesModuleDirective, K::TComma)?;
                    self.name(slot(f::PROVIDER, Ctx::Bare))?;
                }
                self.punct_tk(K::ProvidesModuleDirective, T::Semi, false)?;
                self.close(n);
            }
            _ => return self.refuse(),
        }
        Ok(())
    }

    // ---- Modifiers.

    /// `modifiers`, where any stand ahead: annotations and modifier words.
    pub(super) fn modifiers(&mut self) -> R<()> {
        if self.scan_modifiers(0) == Some(0) {
            return Ok(());
        }
        let n = self.open(K::Modifiers, f::NONE, C_NONE);
        loop {
            let t = self.tok();
            match t.kind {
                T::At => {
                    self.annotation(BARE)?;
                }
                T::Name if is_modifier_word(t.word) && !(t.word == w::SYNCHRONIZED && self.kind_at(1) == T::LParen) => {
                    self.leaf(keyword(t.word).expect("a modifier"), f::NONE, C_NONE)?;
                }
                T::Name if self.sealed_at(0) => {
                    self.leaf(K::TSealed, f::NONE, C_NONE)?;
                }
                T::Name if self.non_sealed_at(0) => {
                    let start = self.bump()?.start;
                    self.bump()?;
                    let end = self.bump()?.end;
                    let at = self.w.push(key(K::TNonSealed, f::NONE, C_NONE, false), start);
                    self.w.close(at, end);
                }
                _ => break,
            }
        }
        self.close(n);
        Ok(())
    }

    // ---- Type declarations.

    /// A type's declaration, of `kind`, in slot `s`.
    pub(super) fn type_declaration(&mut self, kind: K, s: Slot) -> R<N> {
        self.deeper()?;
        let n = self.open_in(kind, s);
        self.modifiers()?;
        match kind {
            K::ClassDeclaration => {
                self.kw(kind, w::CLASS)?;
                self.type_name()?;
                if self.peek() == T::Lt {
                    self.type_parameters(f::TYPE_PARAMETERS)?;
                }
                if self.is_word(w::EXTENDS) {
                    self.type_clause(K::Superclass, f::SUPERCLASS, K::TExtends, false)?;
                }
                if self.is_word(w::IMPLEMENTS) {
                    self.type_clause(K::SuperInterfaces, f::INTERFACES, K::TImplements, true)?;
                }
                if self.is_word(w::PERMITS) {
                    self.type_clause(K::Permits, f::PERMITS, K::TPermits, true)?;
                }
                self.class_body(K::ClassBody, f::BODY)?;
            }
            K::InterfaceDeclaration => {
                self.kw(kind, w::INTERFACE)?;
                self.type_name()?;
                if self.peek() == T::Lt {
                    self.type_parameters(f::TYPE_PARAMETERS)?;
                }
                if self.is_word(w::EXTENDS) {
                    self.type_clause(K::ExtendsInterfaces, f::NONE, K::TExtends, true)?;
                }
                if self.is_word(w::PERMITS) {
                    self.type_clause(K::Permits, f::PERMITS, K::TPermits, true)?;
                }
                self.class_body(K::InterfaceBody, f::BODY)?;
            }
            K::EnumDeclaration => {
                self.kw(kind, w::ENUM)?;
                self.type_name()?;
                if self.is_word(w::IMPLEMENTS) {
                    self.type_clause(K::SuperInterfaces, f::INTERFACES, K::TImplements, true)?;
                }
                self.enum_body()?;
            }
            K::RecordDeclaration => {
                self.kw(kind, w::RECORD)?;
                self.type_name()?;
                if self.peek() == T::Lt {
                    self.type_parameters(f::TYPE_PARAMETERS)?;
                }
                self.formal_parameters(f::PARAMETERS)?;
                if self.is_word(w::IMPLEMENTS) {
                    self.type_clause(K::SuperInterfaces, f::INTERFACES, K::TImplements, true)?;
                }
                self.class_body(K::ClassBody, f::BODY)?;
            }
            _ => {
                if self.peek() != T::AtInterface {
                    return self.refuse();
                }
                self.tk(kind, K::TAtInterface)?;
                self.type_name()?;
                self.class_body(K::AnnotationTypeBody, f::BODY)?;
            }
        }
        self.close(n);
        self.depth -= 1;
        Ok(n)
    }

    /// A declared type's name (an `identifier`).
    fn type_name(&mut self) -> R<()> {
        if !self.is_ident() {
            return self.refuse();
        }
        self.leaf(K::Identifier, f::NAME, C_NONE)?;
        Ok(())
    }

    /// A class, interface or annotation type body (`kind`): `{` members `}`.
    pub(super) fn class_body(&mut self, kind: K, field: u8) -> R<()> {
        let n = self.open(kind, field, C_NONE);
        self.braced(kind, |p| p.members(kind, T::RBrace))?;
        self.close(n);
        Ok(())
    }

    /// Members to `close`; an empty member's `;` stored. Whether the last gap holds one.
    fn members(&mut self, body: K, close: T) -> R<bool> {
        let mut keep = false;
        loop {
            match self.peek() {
                t if t == close || t == T::Eof => return Ok(keep),
                T::Semi => {
                    self.leaf(K::TSemi, f::NONE, C_NONE)?;
                    keep = true;
                }
                _ => {
                    self.member(body)?;
                    keep = false;
                }
            }
        }
    }

    /// A member of a body of `body`'s kind.
    fn member(&mut self, body: K) -> R<()> {
        if self.peek() == T::LBrace && body == K::ClassBody {
            self.block(BARE)?;
            return Ok(());
        }
        if self.is_word(w::STATIC) && self.kind_at(1) == T::LBrace && body == K::ClassBody {
            let n = self.open(K::StaticInitializer, f::NONE, C_NONE);
            self.tk(K::StaticInitializer, K::TStatic)?;
            self.block(BARE)?;
            self.close(n);
            return Ok(());
        }
        let Some(m) = self.member_ahead() else { return self.refuse() };
        match m {
            Member::Type(kind) => {
                self.type_declaration(kind, BARE)?;
            }
            Member::Method if body == K::AnnotationTypeBody => self.annotation_element()?,
            Member::Method => {
                self.method_declaration(BARE)?;
            }
            Member::Constructor if body == K::ClassBody => self.constructor_declaration()?,
            Member::Compact if body == K::ClassBody => {
                let n = self.open(K::CompactConstructorDeclaration, f::NONE, C_NONE);
                self.modifiers()?;
                self.leaf(K::Identifier, f::NAME, C_NONE)?;
                self.block(slot(f::BODY, Ctx::Bare))?;
                self.close(n);
            }
            Member::Field => {
                let kind = if body == K::ClassBody { K::FieldDeclaration } else { K::ConstantDeclaration };
                let n = self.open(kind, f::NONE, C_NONE);
                self.modifiers()?;
                self.unannotated(slot(f::TYPE, Ctx::UType), true)?;
                self.declarators(kind)?;
                self.punct_tk(kind, T::Semi, false)?;
                self.close(n);
            }
            _ => return self.refuse(),
        }
        Ok(())
    }

    /// `method_declaration`: modifiers, type parameters and annotations, a type, a name, its
    /// parameters, dimensions and `throws`, then a body or `;`.
    pub(super) fn method_declaration(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let n = self.open_in(K::MethodDeclaration, s);
        self.modifiers()?;
        if self.peek() == T::Lt {
            self.type_parameters(f::TYPE_PARAMETERS)?;
            while self.peek() == T::At {
                self.annotation(BARE)?;
            }
        }
        self.unannotated(slot(f::TYPE, Ctx::UType), true)?;
        if !self.is_ident() {
            return self.refuse();
        }
        self.leaf(K::Identifier, f::NAME, C_NONE)?;
        self.formal_parameters(f::PARAMETERS)?;
        if self.dims_ahead(0) {
            self.dimensions(f::DIMENSIONS)?;
        }
        if self.is_word(w::THROWS) {
            self.type_clause(K::Throws, f::NONE, K::TThrows, false)?;
        }
        if self.peek() == T::LBrace {
            self.block(slot(f::BODY, Ctx::Bare))?;
        } else {
            self.punct_tk(K::MethodDeclaration, T::Semi, false)?;
        }
        self.close(n);
        self.depth -= 1;
        Ok(n)
    }

    fn annotation_element(&mut self) -> R<()> {
        let kind = K::AnnotationTypeElementDeclaration;
        let n = self.open(kind, f::NONE, C_NONE);
        self.modifiers()?;
        self.unannotated(slot(f::TYPE, Ctx::UType), true)?;
        if !self.is_ident() {
            return self.refuse();
        }
        self.leaf(K::Identifier, f::NAME, C_NONE)?;
        self.punct_tk(kind, T::LParen, false)?;
        self.punct_tk(kind, T::RParen, false)?;
        if self.dims_ahead(0) {
            self.dimensions(f::DIMENSIONS)?;
        }
        if self.is_word(w::DEFAULT) {
            self.kw(kind, w::DEFAULT)?;
            self.element_value(slot(f::VALUE, Ctx::Expr))?;
        }
        self.punct_tk(kind, T::Semi, false)?;
        self.close(n);
        Ok(())
    }

    fn constructor_declaration(&mut self) -> R<()> {
        let n = self.open(K::ConstructorDeclaration, f::NONE, C_NONE);
        self.modifiers()?;
        if self.peek() == T::Lt {
            self.type_parameters(f::TYPE_PARAMETERS)?;
        }
        self.leaf(K::Identifier, f::NAME, C_NONE)?;
        self.formal_parameters(f::PARAMETERS)?;
        if self.is_word(w::THROWS) {
            self.type_clause(K::Throws, f::NONE, K::TThrows, false)?;
        }
        let b = self.open(K::ConstructorBody, f::BODY, C_NONE);
        self.braced(K::ConstructorBody, |p| {
            if p.explicit_ahead() {
                p.explicit_constructor_invocation()?;
            }
            p.statements_in_body()
        })?;
        self.close(b);
        self.close(n);
        Ok(())
    }

    fn statements_in_body(&mut self) -> R<bool> {
        let mut keep = false;
        loop {
            match self.peek() {
                T::RBrace | T::Eof => return Ok(keep),
                T::Semi => {
                    self.leaf_in(K::TSemi, STMT)?;
                    keep = true;
                }
                _ => {
                    self.statement(STMT)?;
                    keep = false;
                }
            }
        }
    }

    /// Whether a constructor's body starts with a call of another (`this(…)`, `super(…)`,
    /// `<T>this(…)`, `outer.super(…)`).
    fn explicit_ahead(&mut self) -> bool {
        let mut k = 0;
        if self.kind_at(0) == T::Lt {
            match self.scan_type_arguments(0) {
                Some(e) => k = e,
                None => return false,
            }
        }
        if (self.is_word_at(k, w::THIS) || self.is_word_at(k, w::SUPER)) && self.kind_at(k + 1) == T::LParen {
            return true;
        }
        // `primary . [<…>] super (`, before the statement's `;`.
        let mut depth = 0u32;
        let mut k = 0;
        loop {
            match self.kind_at(k) {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => match depth.checked_sub(1) {
                    Some(d) => depth = d,
                    None => return false,
                },
                T::Semi | T::Eof if depth == 0 => return false,
                T::Eof => return false,
                T::Dot if depth == 0 => {
                    let mut j = k + 1;
                    if self.kind_at(j) == T::Lt {
                        match self.scan_type_arguments(j) {
                            Some(e) => j = e,
                            None => return false,
                        }
                    }
                    if self.is_word_at(j, w::SUPER) && self.kind_at(j + 1) == T::LParen {
                        return true;
                    }
                }
                _ => {}
            }
            k += 1;
        }
    }

    fn explicit_constructor_invocation(&mut self) -> R<()> {
        let kind = K::ExplicitConstructorInvocation;
        let n = self.open(kind, f::NONE, C_NONE);
        let mut k = 0;
        if self.peek() == T::Lt {
            k = self.scan_type_arguments(0).unwrap_or(0);
        }
        if (self.is_word_at(k, w::THIS) || self.is_word_at(k, w::SUPER)) && self.kind_at(k + 1) == T::LParen {
            if self.peek() == T::Lt {
                self.type_arguments(f::TYPE_ARGUMENTS)?;
            }
            let c = if self.is_word(w::THIS) { K::This } else { K::Super };
            self.leaf(c, f::CONSTRUCTOR, C_NONE)?;
        } else {
            self.primary(slot(f::OBJECT, Ctx::Primary))?;
            self.punct_tk(kind, T::Dot, false)?;
            if self.peek() == T::Lt {
                self.type_arguments(f::TYPE_ARGUMENTS)?;
            }
            if !self.is_word(w::SUPER) {
                return self.refuse();
            }
            self.leaf(K::Super, f::CONSTRUCTOR, C_NONE)?;
        }
        self.arguments()?;
        self.punct_tk(kind, T::Semi, false)?;
        self.close(n);
        Ok(())
    }

    /// `enum_body`: `{` constants between `,`s, a trailing `,`, and declarations after `;` `}`.
    fn enum_body(&mut self) -> R<()> {
        let n = self.open(K::EnumBody, f::BODY, C_NONE);
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        let bare_comma = self.kind_at(1) == T::Comma;
        self.gap_tk(K::EnumBody, K::TLBrace, bare_comma)?;
        let mut constants = false;
        while self.peek() != T::Semi && self.peek() != T::RBrace && !(self.peek() == T::Comma && !constants) {
            self.enum_constant()?;
            constants = true;
            if self.peek() != T::Comma {
                break;
            }
            let after = self.kind_at(1);
            if after == T::RBrace || after == T::Semi {
                break;
            }
            self.tk(K::EnumBody, K::TComma)?;
        }
        // A trailing `,`: stored, with the `}` in its gap.
        let trailing = self.peek() == T::Comma;
        if trailing {
            self.leaf(K::TComma, f::NONE, C_NONE)?;
        }
        if self.peek() == T::Semi {
            let d = self.open(K::EnumBodyDeclarations, f::NONE, C_NONE);
            let first_semi = self.kind_at(1) == T::Semi;
            self.gap_tk(K::EnumBodyDeclarations, K::TSemi, first_semi)?;
            self.members(K::ClassBody, T::RBrace)?;
            self.close(d);
            self.punct_tk(K::EnumBody, T::RBrace, false)?;
        } else {
            self.punct_tk(K::EnumBody, T::RBrace, trailing)?;
        }
        self.close(n);
        Ok(())
    }

    fn enum_constant(&mut self) -> R<()> {
        let n = self.open(K::EnumConstant, f::NONE, C_NONE);
        self.modifiers()?;
        if !self.is_ident() {
            return self.refuse();
        }
        self.leaf(K::Identifier, f::NAME, C_NONE)?;
        if self.peek() == T::LParen {
            self.arguments()?;
        }
        if self.peek() == T::LBrace {
            self.class_body(K::ClassBody, f::BODY)?;
        }
        self.close(n);
        Ok(())
    }

    // ---- Parameters.

    /// `formal_parameters`: `(` a receiver and parameters between `,`s `)`.
    pub(super) fn formal_parameters(&mut self, field: u8) -> R<()> {
        if self.peek() != T::LParen {
            return self.refuse();
        }
        let n = self.open(K::FormalParameters, field, C_NONE);
        self.list(K::FormalParameters, T::RParen, true, false, |p| p.parameter())?;
        self.close(n);
        Ok(())
    }

    fn parameter(&mut self) -> R<()> {
        // A receiver: annotations, a type, names and `.`s, `this`.
        let receiver = (|| {
            let j = self.scan_annotations(0)?;
            let mut k = self.scan_type(j, true)?;
            while self.is_ident_at(k) && self.kind_at(k + 1) == T::Dot {
                k += 2;
            }
            self.is_word_at(k, w::THIS).then_some(())
        })()
        .is_some();
        if receiver {
            let n = self.open(K::ReceiverParameter, f::NONE, C_NONE);
            while self.peek() == T::At {
                self.annotation(BARE)?;
            }
            self.unannotated(UTYPE, true)?;
            while self.is_ident() {
                self.leaf(K::Identifier, f::NONE, C_NONE)?;
                self.punct_tk(K::ReceiverParameter, T::Dot, false)?;
            }
            self.leaf(K::This, f::NONE, C_NONE)?;
            self.close(n);
            return Ok(());
        }
        let spread = self.scan_modifiers(0).and_then(|j| self.scan_type(j, true)).is_some_and(|e| self.kind_at(e) == T::DotDotDot);
        if spread {
            let n = self.open(K::SpreadParameter, f::NONE, C_NONE);
            self.modifiers()?;
            self.unannotated(UTYPE, true)?;
            self.tk(K::SpreadParameter, K::TDotDotDot)?;
            while self.peek() == T::At {
                self.annotation(BARE)?;
            }
            self.variable_declarator(f::NONE)?;
            self.close(n);
            return Ok(());
        }
        let n = self.open(K::FormalParameter, f::NONE, C_NONE);
        self.modifiers()?;
        self.unannotated(slot(f::TYPE, Ctx::UType), true)?;
        self.declarator_id()?;
        self.close(n);
        Ok(())
    }
}
