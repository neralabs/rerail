//! Declarations, their modifiers and attributes, and patterns.

use super::P;
use super::expr::TOP;
use crate::kinds::{K, f};
use crate::lex::{R, T, w};

/// A declaration's keyword, found past its attributes and modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decl {
    Import,
    Property,
    Func,
    Class,
    Protocol,
    Typealias,
    Init,
    Deinit,
    Subscript,
    Associatedtype,
    Operator,
    Precedencegroup,
    Macro,
}

/// Where a declaration is: the file's level, a body's statements, a type's body, a protocol's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Level {
    Top,
    Local,
    Member,
    Protocol,
}

/// Whether `code` is a word a declaration starts with (a modifier, `indirect`, a declaration's
/// keyword).
pub(super) fn starts_declaration(code: u8) -> bool {
    modifier(code).is_some()
        || matches!(
            code,
            w::INDIRECT | w::TYPEALIAS | w::IMPORT | w::FUNC | w::STRUCT | w::ENUM | w::EXTENSION | w::PROTOCOL | w::LET | w::VAR | w::INIT | w::DEINIT | w::SUBSCRIPT | w::ASSOCIATEDTYPE | w::PRECEDENCEGROUP | w::MACRO
        )
}

/// A modifier word's node kind and token (`class` is `property_modifier` only before a member).
fn modifier(code: u8) -> Option<(K, K)> {
    use K::*;
    Some(match code {
        w::PUBLIC => (VisibilityModifier, TPublic),
        w::PRIVATE => (VisibilityModifier, TPrivate),
        w::INTERNAL => (VisibilityModifier, TInternal),
        w::FILEPRIVATE => (VisibilityModifier, TFileprivate),
        w::OPEN => (VisibilityModifier, TOpen),
        w::PACKAGE => (VisibilityModifier, TPackage),
        w::STATIC => (PropertyModifier, TStatic),
        w::DYNAMIC => (PropertyModifier, TDynamic),
        w::OPTIONAL => (PropertyModifier, TOptional),
        w::CLASS => (PropertyModifier, TClass),
        w::DISTRIBUTED => (PropertyModifier, TDistributed),
        w::OVERRIDE => (MemberModifier, TOverride),
        w::CONVENIENCE => (MemberModifier, TConvenience),
        w::REQUIRED => (MemberModifier, TRequired),
        w::NONISOLATED => (MemberModifier, TNonisolated),
        w::MUTATING => (MutationModifier, TMutating),
        w::NONMUTATING => (MutationModifier, TNonmutating),
        w::FINAL => (InheritanceModifier, TFinal),
        w::WEAK => (OwnershipModifier, TWeak),
        w::UNOWNED => (OwnershipModifier, TUnowned),
        w::LAZY => (PropertyBehaviorModifier, TLazy),
        w::INFIX => (FunctionModifier, TInfix),
        w::POSTFIX => (FunctionModifier, TPostfix),
        w::PREFIX => (FunctionModifier, TPrefix),
        w::INOUT => (ParameterModifier, TInout),
        w::BORROWING => (ParameterModifier, TBorrowing),
        w::CONSUMING => (ParameterModifier, TConsuming),
        w::SENDING => (ParameterModifier, TSending),
        w::ISOLATED => (ParameterModifier, TIsolated),
        _ => return None,
    })
}

impl<'a> P<'a> {
    // ---- Attributes.

    /// The index past the attribute at token `k` (its `@`), `None` where it is not one.
    pub(super) fn attribute_end(&mut self, k: usize) -> Option<usize> {
        if self.kind_at(k) != T::At || self.kind_at(k + 1) != T::Name || !self.adjacent(k) {
            return None;
        }
        let mut j = k + 2;
        while self.kind_at(j) == T::Dot && self.kind_at(j + 1) == T::Name {
            j += 2;
        }
        if self.kind_at(j) == T::Lt && self.adjacent(j - 1) {
            j = self.type_args_end(j)? + 1;
        }
        if self.kind_at(j) == T::LParen && self.adjacent(j - 1) {
            let mut d = 0;
            loop {
                match self.kind_at(j) {
                    T::LParen | T::LBracket | T::LBrace => d += 1,
                    T::RParen | T::RBracket | T::RBrace => {
                        d -= 1;
                        if d == 0 {
                            break;
                        }
                    }
                    T::Eof => return None,
                    _ => {}
                }
                j += 1;
            }
            j += 1;
        }
        Some(j)
    }

    /// `attribute`: `@`, a user type, arguments.
    pub(super) fn attribute(&mut self) -> R<()> {
        let n = self.open(K::Attribute);
        self.expect(T::At)?;
        if !self.touching() {
            return self.refuse();
        }
        self.user_type()?;
        if self.peek() == T::LParen && !self.touching() && !matches!(self.t_text_last(), b"escaping" | b"Sendable" | b"autoclosure") {
            // `@A (…)`: a function type's parameters where `->` follows them (seen), else
            // arguments or parameters as the GLR decides (both seen with `async throws ->`).
            if !self.arrow_after_parens(0) {
                return self.refuse();
            }
        }
        if self.peek() == T::LParen && self.touching() {
            self.tk(K::TLParen)?;
            self.level(false, |p| {
                while p.peek() != T::RParen {
                    p.attribute_argument()?;
                    if p.peek() != T::Comma {
                        break;
                    }
                    p.tk(K::TComma)?;
                }
                Ok(())
            })?;
            self.expect(T::RParen)?;
        }
        self.close(n);
        Ok(())
    }

    /// `_attribute_argument` (hidden): `name: expr`, `name:`s, names and a version, an
    /// expression.
    fn attribute_argument(&mut self) -> R<()> {
        if self.peek() == T::Name && self.kind_at(1) == T::Colon {
            self.simple_identifier()?;
            self.tk(K::TColon)?;
            if matches!(self.peek(), T::Comma | T::RParen) {
                return Ok(());
            }
            if self.peek() == T::Name && self.kind_at(1) == T::Colon {
                // `(simple_identifier ":")+`.
                while self.peek() == T::Name && self.kind_at(1) == T::Colon {
                    self.simple_identifier()?;
                    self.tk(K::TColon)?;
                }
                return Ok(());
            }
            return self.expression();
        }
        if self.peek() == T::Name && matches!(self.kind_at(1), T::Name | T::Int | T::Real) && !self.peek_at(1).nl {
            // Names, then a version (none: tree-sitter puts a missing one there).
            while self.peek() == T::Name {
                self.simple_identifier()?;
            }
            if !matches!(self.peek(), T::Int | T::Real) {
                return self.refuse();
            }
            self.version_numbers()?;
            return Ok(());
        }
        if self.peek() == T::Star {
            self.tk(K::TStar)?;
            return Ok(());
        }
        self.expression()
    }

    // ---- Finding a declaration.

    /// The declaration starting at the next token (past attributes and modifiers), if one does.
    pub(super) fn declaration_ahead(&mut self) -> Option<Decl> {
        let mut k = 0;
        loop {
            let t = self.peek_at(k);
            match t.kind {
                T::At => k = self.attribute_end(k)?,
                T::Name if !t.tick => {
                    let next = self.peek_at(k + 1);
                    match t.word {
                        w::IMPORT => return Some(Decl::Import),
                        w::LET | w::VAR => return Some(Decl::Property),
                        w::FUNC => return Some(Decl::Func),
                        w::CLASS if next.kind == T::Name && (next.tick || modifier(next.word).is_none() && !matches!(next.word, w::FUNC | w::VAR | w::LET | w::SUBSCRIPT | w::INIT)) => {
                            return Some(Decl::Class);
                        }
                        w::STRUCT | w::ENUM | w::EXTENSION | w::ACTOR if next.kind == T::Name => return Some(Decl::Class),
                        w::INDIRECT if self.word_at(k + 1) == w::ENUM => return Some(Decl::Class),
                        w::PROTOCOL if next.kind == T::Name => return Some(Decl::Protocol),
                        w::TYPEALIAS => return Some(Decl::Typealias),
                        w::INIT if matches!(next.kind, T::LParen | T::Question | T::Bang | T::Lt) => return Some(Decl::Init),
                        w::DEINIT if next.kind == T::LBrace => return Some(Decl::Deinit),
                        w::SUBSCRIPT if matches!(next.kind, T::LParen | T::Lt) => return Some(Decl::Subscript),
                        w::ASSOCIATEDTYPE => return Some(Decl::Associatedtype),
                        w::PRECEDENCEGROUP => return Some(Decl::Precedencegroup),
                        w::MACRO if next.kind == T::Name => return Some(Decl::Macro),
                        w::PREFIX | w::INFIX | w::POSTFIX if self.word_at(k + 1) == w::OPERATOR => return Some(Decl::Operator),
                        w::ASYNC if matches!(self.word_at(k + 1), w::LET | w::VAR) => return Some(Decl::Property),
                        code if modifier(code).is_some() => {
                            // A modifier only before more modifiers or a declaration.
                            k += 1;
                            if self.kind_at(k) == T::LParen && self.adjacent(k - 1) {
                                if !matches!(self.word_at(k + 1), w::SET | w::UNSAFE | w::NONSENDING) || self.kind_at(k + 2) != T::RParen {
                                    if !matches!(&self.src[self.peek_at(k + 1).start as usize..self.peek_at(k + 1).end as usize], b"safe") {
                                        return None;
                                    }
                                }
                                k += 3;
                            }
                            let n = self.peek_at(k);
                            if n.kind != T::Name && n.kind != T::At {
                                return None;
                            }
                        }
                        _ => return None,
                    }
                }
                _ => return None,
            }
        }
    }

    /// The declaration next, at `level`.
    pub(super) fn declaration(&mut self, d: Decl, level: Level) -> R<()> {
        let kind = match d {
            Decl::Import => K::ImportDeclaration,
            Decl::Property => K::PropertyDeclaration,
            Decl::Func if level == Level::Protocol => K::ProtocolFunctionDeclaration,
            Decl::Func => K::FunctionDeclaration,
            Decl::Class => K::ClassDeclaration,
            Decl::Protocol => K::ProtocolDeclaration,
            Decl::Typealias => K::TypealiasDeclaration,
            Decl::Init => K::InitDeclaration,
            Decl::Deinit => K::DeinitDeclaration,
            Decl::Subscript => K::SubscriptDeclaration,
            Decl::Associatedtype => K::AssociatedtypeDeclaration,
            Decl::Operator => K::OperatorDeclaration,
            Decl::Precedencegroup => K::PrecedenceGroupDeclaration,
            Decl::Macro => K::MacroDeclaration,
        };
        let kind = if d == Decl::Property && level == Level::Protocol { K::ProtocolPropertyDeclaration } else { kind };
        let n = self.open(kind);
        self.modifiers(level, d)?;
        match d {
            Decl::Import => self.import_rest()?,
            Decl::Property if level == Level::Protocol => self.protocol_property_rest()?,
            Decl::Property => self.property_rest()?,
            Decl::Func => self.function_rest(level == Level::Protocol)?,
            Decl::Class => self.class_rest()?,
            Decl::Protocol => self.protocol_rest()?,
            Decl::Typealias => self.typealias_rest()?,
            Decl::Init => self.init_rest(level == Level::Protocol)?,
            Decl::Deinit => {
                self.kw(w::DEINIT, K::TDeinit)?;
                self.function_body()?;
            }
            Decl::Subscript => self.subscript_rest()?,
            Decl::Associatedtype => self.associatedtype_rest()?,
            Decl::Operator => self.operator_rest()?,
            _ => return self.refuse(),
        }
        self.close(n);
        Ok(())
    }

    /// The attributes and modifiers before a declaration: in a `modifiers` node, but in a body's
    /// statements (`_locally_permitted_modifiers`, inlined: no node).
    fn modifiers(&mut self, level: Level, d: Decl) -> R<()> {
        let local = level == Level::Local;
        let mut n = None;
        loop {
            let t = self.tok();
            let is_mod = match t.kind {
                T::At => true,
                T::Name if !t.tick => match t.word {
                    w::CLASS => d != Decl::Class || self.class_is_modifier(),
                    w::ASYNC => false,
                    code => modifier(code).is_some() && !(d == Decl::Operator && matches!(code, w::PREFIX | w::INFIX | w::POSTFIX)),
                },
                _ => false,
            };
            if !is_mod {
                break;
            }
            if local {
                if t.kind == T::Name && !matches!(t.word, w::WEAK | w::UNOWNED | w::FINAL | w::LAZY) {
                    return self.refuse();
                }
            } else if n.is_none() {
                n = Some(self.open(K::Modifiers));
            }
            if t.kind == T::At {
                self.attribute()?;
            } else {
                self.modifier_node()?;
            }
        }
        if let Some(n) = n {
            self.close(n);
        }
        Ok(())
    }

    /// Whether the `class` next is a modifier (before `func`, `var`, …), not the declaration's.
    fn class_is_modifier(&mut self) -> bool {
        let n = self.peek_at(1);
        n.kind == T::Name && !n.tick && (modifier(n.word).is_some() || matches!(n.word, w::FUNC | w::VAR | w::LET | w::SUBSCRIPT | w::INIT | w::CLASS))
    }

    fn modifier_node(&mut self) -> R<()> {
        let (kind, tok) = modifier(self.word()).expect("a modifier");
        let n = self.open(kind);
        if kind == K::OwnershipModifier && self.is_word(w::UNOWNED) && self.kind_at(1) == T::LParen && self.adjacent(0) {
            // `unowned(safe)`, `unowned(unsafe)`: one token.
            let a = self.tok();
            let e = self.peek_at(3);
            let tk = match self.name_text(2) {
                b"safe" => K::TunownedX28safeX29,
                b"unsafe" => K::TunownedX28unsafeX29,
                _ => return self.refuse(),
            };
            for _ in 0..4 {
                self.bump()?;
            }
            self.t.token(tk, f::NONE, a.start, e.end);
            self.close(n);
            return Ok(());
        }
        self.tk(tok)?;
        if self.peek() == T::LParen && self.touching() {
            match kind {
                K::VisibilityModifier if self.is_word_at(1, w::SET) => {
                    self.tk(K::TLParen)?;
                    self.tk(K::TSet)?;
                    self.expect(T::RParen)?;
                }
                K::MemberModifier if matches!(self.word_at(1), w::UNSAFE | w::NONSENDING) => {
                    self.tk(K::TLParen)?;
                    let k = if self.is_word(w::UNSAFE) { K::TUnsafe } else { K::TNonsending };
                    self.tk(k)?;
                    self.expect(T::RParen)?;
                }
                _ => {}
            }
        }
        self.close(n);
        Ok(())
    }

    /// Whether `->` follows the parenthesized list opening `k` tokens ahead.
    fn arrow_after_parens(&mut self, k: usize) -> bool {
        let (mut d, mut j) = (0, k);
        loop {
            match self.kind_at(j) {
                T::LParen | T::LBracket => d += 1,
                T::RParen | T::RBracket => {
                    d -= 1;
                    if d == 0 {
                        return self.kind_at(j + 1) == T::Arrow;
                    }
                }
                T::Eof => return false,
                _ => {}
            }
            j += 1;
        }
    }

    /// The text of the last token read.
    fn t_text_last(&self) -> &'a [u8] {
        let t = self.toks[self.i - 1];
        &self.src[t.start as usize..t.end as usize]
    }

    /// `ownership_modifier` (a capture's).
    pub(super) fn ownership_modifier(&mut self) -> R<()> {
        self.modifier_node()
    }

    // ---- Declarations.

    fn import_rest(&mut self) -> R<()> {
        self.kw(w::IMPORT, K::TImport)?;
        let k = match self.word() {
            w::TYPEALIAS => Some(K::TTypealias),
            w::STRUCT => Some(K::TStruct),
            w::CLASS => Some(K::TClass),
            w::ENUM => Some(K::TEnum),
            w::PROTOCOL => Some(K::TProtocol_),
            w::LET => Some(K::TLet),
            w::VAR => Some(K::TVar),
            w::FUNC => Some(K::TFunc),
            _ => None,
        };
        if let Some(k) = k {
            if self.kind_at(1) == T::Name {
                self.tk(k)?;
            }
        }
        self.identifier(f::NONE)?;
        Ok(())
    }

    /// `value_binding_pattern`: `let` or `var` (`mutability:`), given `field`.
    pub(super) fn value_binding_pattern(&mut self, field: u8) -> R<()> {
        let n = self.open_field(K::ValueBindingPattern, field);
        if self.is_word(w::LET) {
            self.tkf(K::TLet, f::MUTABILITY)?;
        } else {
            if !self.is_word(w::VAR) {
                return self.refuse();
            }
            self.tkf(K::TVar, f::MUTABILITY)?;
        }
        self.close(n);
        Ok(())
    }

    fn property_rest(&mut self) -> R<()> {
        if self.is_word(w::ASYNC) {
            self.tk(K::TAsync)?;
        }
        self.binding_kind()?;
        loop {
            let p = self.open_field(K::Pattern, f::NAME);
            self.no_expr_pattern(true)?;
            self.close(p);
            if self.peek() == T::Colon {
                self.type_annotation()?;
            }
            if self.is_word(w::WHERE) {
                self.type_constraints()?;
            }
            match self.peek() {
                T::Eq => {
                    self.tk(K::TEq)?;
                    let stmt = self.stmt;
                    let willset = self.willset_after_value();
                    let lambda = std::mem::replace(&mut self.no_lambda, willset);
                    let r = self.expr_f(super::expr::VALUE, f::VALUE);
                    self.no_lambda = lambda;
                    self.stmt = stmt;
                    r?;
                    if self.peek() == T::LBrace && self.willset_next() {
                        self.willset_didset_block()?;
                    }
                }
                T::LBrace => {
                    if self.willset_next() {
                        self.willset_didset_block()?;
                    } else {
                        self.computed_property(f::COMPUTED_VALUE)?;
                    }
                }
                _ => {}
            }
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::TComma)?;
        }
        Ok(())
    }

    /// The `let`/`var` of a declaration: `value_binding_pattern` with its `mutability:`.
    fn binding_kind(&mut self) -> R<()> {
        let n = self.open(K::ValueBindingPattern);
        if self.is_word(w::LET) {
            self.tkf(K::TLet, f::MUTABILITY)?;
        } else {
            if !self.is_word(w::VAR) {
                return self.refuse();
            }
            self.tkf(K::TVar, f::MUTABILITY)?;
        }
        self.close(n);
        Ok(())
    }

    /// Whether a `{` and `willSet`/`didSet` come after the value next (its last `{` on its line).
    fn willset_after_value(&mut self) -> bool {
        let mut k = 0;
        let mut d = 0i32;
        loop {
            let t = self.peek_at(k);
            match t.kind {
                T::Eof => return false,
                T::LParen | T::LBracket => d += 1,
                T::RParen | T::RBracket => d -= 1,
                T::LBrace if d == 0 => {
                    let mut j = k + 1;
                    while self.kind_at(j) == T::At {
                        match self.attribute_end(j) {
                            Some(e) => j = e,
                            None => return false,
                        }
                    }
                    return matches!(self.word_at(j), w::WILLSET | w::DIDSET);
                }
                _ if t.nl && d == 0 && k > 0 => return false,
                _ => {}
            }
            k += 1;
            if k > 256 {
                return false;
            }
        }
    }

    /// Whether the `{` next opens a `willSet`/`didSet` block.
    fn willset_next(&mut self) -> bool {
        let mut j = 1;
        while self.kind_at(j) == T::At {
            match self.attribute_end(j) {
                Some(e) => j = e,
                None => return false,
            }
        }
        while modifier(self.word_at(j)).is_some() {
            j += 1;
        }
        matches!(self.word_at(j), w::WILLSET | w::DIDSET)
    }

    fn willset_didset_block(&mut self) -> R<()> {
        let n = self.open(K::WillsetDidsetBlock);
        self.tk(K::TLBrace)?;
        while self.peek() != T::RBrace {
            let mut j = 0;
            while self.kind_at(j) == T::At {
                j = self.attribute_end(j).ok_or(crate::lex::Refusal::Parse(self.tok().start))?;
            }
            while modifier(self.word_at(j)).is_some() {
                j += 1;
            }
            let (kind, tok) = match self.word_at(j) {
                w::WILLSET => (K::WillsetClause, K::TWillSet),
                w::DIDSET => (K::DidsetClause, K::TDidSet),
                _ => return self.refuse(),
            };
            let c = self.open(kind);
            self.modifiers(Level::Member, Decl::Func)?;
            self.tk(tok)?;
            if self.peek() == T::LParen {
                self.tk(K::TLParen)?;
                self.simple_identifier()?;
                self.expect(T::RParen)?;
            }
            self.level(true, |p| p.block())?;
            self.close(c);
        }
        self.expect(T::RBrace)?;
        self.close(n);
        Ok(())
    }

    /// `computed_property`: accessors, or statements.
    fn computed_property(&mut self, field: u8) -> R<()> {
        let n = self.open_field(K::ComputedProperty, field);
        self.tk(K::TLBrace)?;
        if self.accessor_ahead(0) {
            while self.peek() != T::RBrace {
                let mut j = 0;
                while self.kind_at(j) == T::At {
                    j = self.attribute_end(j).ok_or(crate::lex::Refusal::Parse(self.tok().start))?;
                }
                if matches!(self.word_at(j), w::MUTATING | w::NONMUTATING) {
                    j += 1;
                }
                let kind = match self.word_at(j) {
                    w::GET => K::ComputedGetter,
                    w::SET => K::ComputedSetter,
                    w::MODIFY => K::ComputedModify,
                    _ => return self.refuse(),
                };
                let c = self.open(kind);
                while self.peek() == T::At {
                    self.attribute()?;
                }
                self.accessor_specifier(kind)?;
                if kind == K::ComputedSetter && self.peek() == T::LParen {
                    self.tk(K::TLParen)?;
                    self.simple_identifier()?;
                    self.expect(T::RParen)?;
                }
                if self.peek() == T::LBrace {
                    self.level(true, |p| p.block())?;
                }
                self.close(c);
            }
        } else {
            self.level(true, |p| p.statements(false))?;
        }
        self.expect(T::RBrace)?;
        self.close(n);
        Ok(())
    }

    /// Whether an accessor (`get`, `set`, `_modify`, after attributes and a mutation modifier)
    /// starts `k` tokens ahead, followed by its block, `(`, `}` or another accessor.
    fn accessor_ahead(&mut self, k: usize) -> bool {
        let mut j = k;
        while self.kind_at(j) == T::At {
            match self.attribute_end(j) {
                Some(e) => j = e,
                None => return false,
            }
        }
        if matches!(self.word_at(j), w::MUTATING | w::NONMUTATING) {
            j += 1;
        }
        if !matches!(self.word_at(j), w::GET | w::SET | w::MODIFY) {
            return false;
        }
        let next = self.peek_at(j + 1);
        match next.kind {
            T::LBrace | T::RBrace => true,
            T::LParen => self.word_at(j) == w::SET,
            T::Name => matches!(next.word, w::GET | w::SET | w::MODIFY | w::ASYNC | w::THROWS | w::MUTATING | w::NONMUTATING) && !next.tick,
            T::At => true,
            _ => false,
        }
    }

    /// `getter_specifier`, `setter_specifier`, `modify_specifier`: a mutation modifier, the word,
    /// a getter's effects.
    fn accessor_specifier(&mut self, kind: K) -> R<()> {
        let spec = match kind {
            K::ComputedGetter => K::GetterSpecifier,
            K::ComputedSetter => K::SetterSpecifier,
            _ => K::ModifySpecifier,
        };
        let n = self.open(spec);
        if matches!(self.word(), w::MUTATING | w::NONMUTATING) {
            self.modifier_node()?;
        }
        match self.word() {
            w::GET => self.tk(K::TGet)?,
            w::SET => self.tk(K::TSet)?,
            _ => self.tk(K::TModify)?,
        };
        if spec == K::GetterSpecifier {
            loop {
                match self.word() {
                    w::ASYNC => {
                        self.tk(K::TAsync)?;
                    }
                    w::THROWS | w::RETHROWS => self.throws()?,
                    _ => break,
                }
            }
        }
        self.close(n);
        Ok(())
    }

    fn protocol_property_rest(&mut self) -> R<()> {
        let p = self.open_field(K::Pattern, f::NAME);
        self.binding_kind()?;
        self.no_expr_pattern(true)?;
        self.close(p);
        if self.peek() == T::Colon {
            self.type_annotation()?;
        }
        if self.is_word(w::WHERE) {
            self.type_constraints()?;
        }
        let n = self.open(K::ProtocolPropertyRequirements);
        self.expect(T::LBrace)?;
        while self.peek() != T::RBrace {
            let j = if matches!(self.word(), w::MUTATING | w::NONMUTATING) { 1 } else { 0 };
            let kind = match self.word_at(j) {
                w::GET => K::ComputedGetter,
                w::SET => K::ComputedSetter,
                _ => return self.refuse(),
            };
            self.accessor_specifier(kind)?;
        }
        self.expect(T::RBrace)?;
        self.close(n);
        Ok(())
    }

    /// A function's name, generics, parameters, effects, result, constraints and body.
    fn function_rest(&mut self, protocol: bool) -> R<()> {
        self.kw(w::FUNC, K::TFunc)?;
        match self.peek() {
            T::Name => {
                self.simple_identifier_f(f::NAME)?;
            }
            T::Custom => {
                self.leaf(K::CustomOperator, f::NAME)?;
            }
            t if super::punct(t).is_some() && (!matches!(t, T::LParen | T::Lt) || t == T::Lt && self.kind_at(1) == T::LParen) => {
                let k = super::punct(t).expect("an operator");
                if t == T::Bang {
                    self.leaf(K::Bang, f::NAME)?;
                } else {
                    self.tkf(k, f::NAME)?;
                }
            }
            _ => return self.refuse(),
        }
        if self.peek() == T::Lt {
            self.type_parameters()?;
        }
        self.function_value_parameters()?;
        self.effects()?;
        if self.peek() == T::Arrow {
            self.tk(K::TArrow)?;
            if self.parameter_modifier_ahead() {
                return self.refuse();
            }
            self.possibly_unwrapped_type(f::RETURN_TYPE)?;
        }
        if self.is_word(w::WHERE) {
            self.type_constraints()?;
        }
        if self.peek() == T::LBrace {
            self.function_body()?;
        } else if !protocol {
            return self.refuse();
        }
        Ok(())
    }

    /// `async`, `throws`/`rethrows`/`throws(E)`.
    fn effects(&mut self) -> R<()> {
        if self.is_word(w::ASYNC) {
            self.tk(K::TAsync)?;
        }
        if matches!(self.word(), w::THROWS | w::RETHROWS) {
            self.throws()?;
        }
        Ok(())
    }

    /// `_function_value_parameters`: `(` parameters `)` (the parens, defaults and `=`s are the
    /// declaration's children).
    fn function_value_parameters(&mut self) -> R<()> {
        self.expect(T::LParen)?;
        self.level(false, |p| {
            while p.peek() != T::RParen {
                if p.peek() == T::At && !p.parameter_modifier_ahead() {
                    p.attribute()?;
                }
                p.parameter()?;
                if p.peek() == T::Eq {
                    p.tk(K::TEq)?;
                    p.expression_f(f::DEFAULT_VALUE)?;
                }
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
            }
            Ok(())
        })?;
        self.expect(T::RParen)?;
        Ok(())
    }

    fn parameter(&mut self) -> R<()> {
        let n = self.open(K::Parameter);
        if self.peek() == T::Name && self.kind_at(1) == T::Name && self.kind_at(2) == T::Colon {
            self.simple_identifier_f(f::EXTERNAL_NAME)?;
        }
        self.simple_identifier_f(f::NAME)?;
        self.expect(T::Colon)?;
        self.parameter_type()?;
        if self.peek() == T::DotDotDot {
            self.tk(K::TDotDotDot)?;
        }
        self.close(n);
        Ok(())
    }

    fn init_rest(&mut self, protocol: bool) -> R<()> {
        let t = self.tok();
        self.bump()?;
        self.t.token(K::TInit, f::NAME, t.start, t.end);
        match self.peek() {
            T::Question => {
                self.tk(K::TQuestion)?;
            }
            T::Bang => {
                self.leaf(K::Bang, f::NONE)?;
            }
            _ => {}
        }
        if self.peek() == T::Lt {
            self.type_parameters()?;
        }
        self.function_value_parameters()?;
        self.effects()?;
        if self.is_word(w::WHERE) {
            self.type_constraints()?;
        }
        if self.peek() == T::LBrace {
            self.function_body()?;
        } else if !protocol {
            return self.refuse();
        }
        Ok(())
    }

    fn subscript_rest(&mut self) -> R<()> {
        self.kw(w::SUBSCRIPT, K::TSubscript)?;
        if self.peek() == T::Lt {
            self.type_parameters()?;
        }
        self.function_value_parameters()?;
        if self.peek() == T::Arrow {
            self.tk(K::TArrow)?;
            self.possibly_unwrapped_type(f::RETURN_TYPE)?;
        }
        if self.is_word(w::WHERE) {
            self.type_constraints()?;
        }
        self.computed_property(f::NONE)
    }

    /// `prefix|infix|postfix operator OP (: group)?`.
    fn operator_rest(&mut self) -> R<()> {
        let k = match self.word() {
            w::PREFIX => K::TPrefix,
            w::INFIX => K::TInfix,
            _ => K::TPostfix,
        };
        self.tk(k)?;
        self.kw(w::OPERATOR, K::TOperator)?;
        match self.peek() {
            T::Custom => {
                self.leaf(K::CustomOperator, f::NONE)?;
            }
            T::Bang => {
                self.leaf(K::Bang, f::NONE)?;
            }
            t => match super::punct(t) {
                Some(k) if !matches!(t, T::LParen | T::LBrace | T::LBracket | T::Colon | T::Comma) => {
                    self.tk(k)?;
                }
                _ => return self.refuse(),
            },
        }
        if self.peek() == T::Colon {
            self.tk(K::TColon)?;
            self.simple_identifier()?;
        }
        if self.peek() == T::LBrace {
            return self.refuse();
        }
        Ok(())
    }

    fn typealias_rest(&mut self) -> R<()> {
        self.kw(w::TYPEALIAS, K::TTypealias)?;
        self.ident_as(K::TypeIdentifier, f::NAME)?;
        if self.peek() == T::Lt {
            self.type_parameters()?;
        }
        self.expect(T::Eq)?;
        self.ty_f(f::VALUE)
    }

    fn associatedtype_rest(&mut self) -> R<()> {
        self.kw(w::ASSOCIATEDTYPE, K::TAssociatedtype)?;
        self.ident_as(K::TypeIdentifier, f::NAME)?;
        if self.peek() == T::Colon {
            self.tk(K::TColon)?;
            self.ty_f(f::MUST_INHERIT)?;
        }
        if self.is_word(w::WHERE) {
            self.type_constraints()?;
        }
        if self.peek() == T::Eq {
            self.tk(K::TEq)?;
            self.ty_f(f::DEFAULT_VALUE)?;
        }
        Ok(())
    }

    // ---- Types' declarations.

    fn class_rest(&mut self) -> R<()> {
        let word = self.word();
        let enum_ = word == w::ENUM || word == w::INDIRECT;
        if word == w::INDIRECT {
            self.tk(K::TIndirect)?;
        }
        match self.word() {
            w::CLASS => self.tkf(K::TClass, f::DECLARATION_KIND)?,
            w::STRUCT => self.tkf(K::TStruct, f::DECLARATION_KIND)?,
            w::ACTOR => self.tkf(K::TActor, f::DECLARATION_KIND)?,
            w::ENUM => self.tkf(K::TEnum, f::DECLARATION_KIND)?,
            w::EXTENSION => self.tkf(K::TExtension, f::DECLARATION_KIND)?,
            _ => return self.refuse(),
        };
        if word == w::EXTENSION {
            self.unannotated_f(f::NAME)?;
        } else {
            self.ident_as(K::TypeIdentifier, f::NAME)?;
        }
        if self.peek() == T::Lt {
            self.type_parameters()?;
        }
        if self.peek() == T::Colon {
            self.tk(K::TColon)?;
            self.inheritance_specifiers()?;
        }
        if self.is_word(w::WHERE) {
            self.type_constraints()?;
        }
        if enum_ { self.enum_class_body() } else { self.class_body() }
    }

    fn inheritance_specifiers(&mut self) -> R<()> {
        self.level(false, |p| {
            loop {
                while p.peek() == T::At {
                    p.attribute()?;
                }
                let n = p.open(K::InheritanceSpecifier);
                let m = p.mark();
                p.unannotated_f(f::INHERITS_FROM)?;
                p.t.unmark(m);
                p.close(n);
                match p.peek() {
                    T::Comma => {
                        p.tk(K::TComma)?;
                    }
                    T::Amp => {
                        p.tk(K::TAmp)?;
                    }
                    _ => break,
                }
            }
            Ok(())
        })
    }

    fn class_body(&mut self) -> R<()> {
        let n = self.open_field(K::ClassBody, f::BODY);
        self.tk(K::TLBrace)?;
        self.members(Level::Member)?;
        self.expect(T::RBrace)?;
        self.close(n);
        Ok(())
    }

    /// A type's or protocol's members, each followed by a semicolon (or the `}`).
    fn members(&mut self, level: Level) -> R<()> {
        self.level(true, |p| {
            loop {
                match p.peek() {
                    T::RBrace => return Ok(()),
                    // A `;` in a protocol's body: tree-sitter fails there (seen).
                    T::Semi if level == Level::Protocol => return p.refuse(),
                    T::Semi => {
                        p.bump()?;
                        continue;
                    }
                    T::Hash if p.directive_next() => {
                        p.directive()?;
                        continue;
                    }
                    T::Hash if p.diagnostic_next() => {
                        p.diagnostic()?;
                        continue;
                    }
                    _ => {}
                }
                let Some(d) = p.declaration_ahead() else { return p.refuse() };
                p.deeper()?;
                p.declaration(d, level)?;
                p.depth -= 1;
                p.member_comments();
                if level == Level::Protocol && p.peek() == T::Semi {
                    return p.refuse();
                }
                if !p.semi()? && p.peek() != T::RBrace && !p.directive_next() {
                    return p.refuse();
                }
            }
        })
    }

    /// A block comment after a member's line: the scanner's token, not an extra.
    fn member_comments(&mut self) {
        let mut at = self.t.last_end as usize;
        while let crate::lex::Semi::Comment(c) = self.lx.semi(at, crate::lex::SemiCtx::default()) {
            let _ = self.tok();
            for e in self.t.extras[self.t.flushed..].iter_mut() {
                if e.1 == c {
                    e.0 &= !(1 << 16);
                }
            }
            match self.lx.block_comment_end(c as usize) {
                Some(e) => at = e,
                None => return,
            }
        }
    }

    fn enum_class_body(&mut self) -> R<()> {
        let n = self.open_field(K::EnumClassBody, f::BODY);
        self.tk(K::TLBrace)?;
        self.level(false, |p| {
            loop {
                match p.peek() {
                    T::RBrace => return Ok(()),
                    T::Hash if p.directive_next() => {
                        p.directive()?;
                        continue;
                    }
                    _ => {}
                }
                if p.enum_entry_ahead() {
                    p.enum_entry()?;
                    continue;
                }
                let Some(d) = p.declaration_ahead() else { return p.refuse() };
                p.deeper()?;
                p.declaration(d, Level::Member)?;
                p.depth -= 1;
            }
        })?;
        self.expect(T::RBrace)?;
        self.close(n);
        Ok(())
    }

    fn enum_entry_ahead(&mut self) -> bool {
        let mut k = 0;
        loop {
            match self.kind_at(k) {
                T::At => match self.attribute_end(k) {
                    Some(e) => k = e,
                    None => return false,
                },
                T::Name => match self.word_at(k) {
                    w::CASE => return true,
                    w::INDIRECT => k += 1,
                    c if modifier(c).is_some() => k += 1,
                    _ => return false,
                },
                _ => return false,
            }
        }
    }

    fn enum_entry(&mut self) -> R<()> {
        let n = self.open(K::EnumEntry);
        let mut m = None;
        while self.peek() == T::At || (modifier(self.word()).is_some() && !self.is_word(w::INDIRECT)) {
            if m.is_none() {
                m = Some(self.open(K::Modifiers));
            }
            if self.peek() == T::At {
                self.attribute()?;
            } else {
                self.modifier_node()?;
            }
        }
        if let Some(m) = m {
            self.close(m);
        }
        if self.is_word(w::INDIRECT) {
            self.tk(K::TIndirect)?;
        }
        self.kw(w::CASE, K::TCase)?;
        loop {
            self.simple_identifier_f(f::NAME)?;
            match self.peek() {
                T::LParen => self.enum_type_parameters()?,
                T::Eq => {
                    self.tk(K::TEq)?;
                    self.expression_f(f::RAW_VALUE)?;
                }
                _ => {}
            }
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::TComma)?;
        }
        if self.peek() == T::Semi {
            self.tk(K::TSemi)?;
        }
        self.close(n);
        Ok(())
    }

    fn enum_type_parameters(&mut self) -> R<()> {
        let n = self.open_field(K::EnumTypeParameters, f::DATA_CONTENTS);
        self.tk(K::TLParen)?;
        self.level(false, |p| {
            while p.peek() != T::RParen {
                if p.is_word(w::UNDERSCORE) && p.kind_at(1) == T::Name && p.kind_at(2) == T::Colon {
                    p.leaf(K::WildcardPattern, f::NONE)?;
                    p.simple_identifier()?;
                    p.tk(K::TColon)?;
                } else if p.peek() == T::Name && p.kind_at(1) == T::Colon {
                    p.simple_identifier()?;
                    p.tk(K::TColon)?;
                }
                p.ty_f(f::NONE)?;
                if p.peek() == T::Eq {
                    p.tk(K::TEq)?;
                    p.expression()?;
                }
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
            }
            Ok(())
        })?;
        self.expect(T::RParen)?;
        self.close(n);
        Ok(())
    }

    fn protocol_rest(&mut self) -> R<()> {
        self.kw(w::PROTOCOL, K::TProtocol_)?;
        self.ident_as(K::TypeIdentifier, f::NAME)?;
        if self.peek() == T::Lt {
            self.type_parameters()?;
        }
        if self.peek() == T::Colon {
            self.tk(K::TColon)?;
            self.inheritance_specifiers()?;
        }
        if self.is_word(w::WHERE) {
            self.type_constraints()?;
        }
        let n = self.open_field(K::ProtocolBody, f::BODY);
        self.tk(K::TLBrace)?;
        self.members(Level::Protocol)?;
        self.expect(T::RBrace)?;
        self.close(n);
        Ok(())
    }

    // ---- Patterns.

    /// `_no_expr_pattern_already_bound` (hidden): `_`, a tuple pattern, a name (`bound`: given
    /// `bound_identifier`), then a `?`.
    pub(super) fn no_expr_pattern(&mut self, bound: bool) -> R<()> {
        match self.peek() {
            T::Name if self.is_word(w::UNDERSCORE) => {
                self.leaf(K::WildcardPattern, f::NONE)?;
            }
            T::Name => {
                self.simple_identifier_f(if bound { f::BOUND_IDENTIFIER } else { f::NONE })?;
            }
            T::LParen => self.tuple_pattern()?,
            _ => return self.refuse(),
        }
        if self.peek() == T::Question && !self.touching() {
            return self.refuse();
        }
        Ok(())
    }

    /// `_tuple_pattern`: `(` items `)`, each `name: pattern` or a pattern.
    fn tuple_pattern(&mut self) -> R<()> {
        self.tk(K::TLParen)?;
        self.level(false, |p| {
            loop {
                if p.peek() == T::Name && p.kind_at(1) == T::Colon {
                    p.simple_identifier()?;
                    p.tk(K::TColon)?;
                }
                let n = p.open(K::Pattern);
                p.binding_pattern_with_expr()?;
                p.close(n);
                if p.peek() != T::Comma {
                    break;
                }
                p.tk(K::TComma)?;
                if p.peek() == T::RParen {
                    break;
                }
            }
            Ok(())
        })?;
        self.expect(T::RParen)?;
        Ok(())
    }

    /// `_binding_pattern_with_expr` (hidden): a binding, a case pattern, `_`, a tuple, a cast, or
    /// an expression.
    pub(super) fn binding_pattern_with_expr(&mut self) -> R<()> {
        // `let x as T`: the cast's pattern is what follows the binding.
        let binds = matches!(self.word(), w::LET | w::VAR) && self.peek() == T::Name;
        if binds {
            self.value_binding_pattern(f::NONE)?;
        }
        let m = self.mark();
        match self.peek() {
            // `case let .some(x)`.
            T::Dot if binds => self.case_pattern()?,
            _ if binds => {
                self.no_expr_pattern(true)?;
            }
            T::Name if self.is_word(w::UNDERSCORE) && !matches!(self.kind_at(1), T::Dot) => {
                self.leaf(K::WildcardPattern, f::NONE)?;
            }
            T::Name if self.is_word(w::IS) => {
                self.tk(K::TIs)?;
                self.ty_f(f::NONE)?;
            }
            T::Dot => self.case_pattern()?,
            // `(…)`: a tuple pattern, whatever is in it (never a tuple expression, seen).
            T::LParen => self.tuple_pattern()?,
            _ => {
                let lambda = std::mem::replace(&mut self.no_lambda, true);
                let r = self.expr(TOP);
                self.no_lambda = lambda;
                r?;
            }
        }
        if self.is_word(w::AS) {
            // `pattern as Type`: the pattern before it a node.
            let n = self.t.wrap(m, K::Pattern, f::NONE);
            self.close(n);
            self.tk(K::TAs)?;
            self.ty_f(f::NONE)?;
        }
        if self.peek() == T::Question && self.touching() {
            self.tk(K::TQuestion)?;
        }
        self.t.unmark(m);
        Ok(())
    }

    /// `_case_pattern` (hidden): `.name` and a tuple pattern.
    fn case_pattern(&mut self) -> R<()> {
        self.tk(K::TDot)?;
        self.simple_identifier()?;
        if self.peek() == T::LParen && self.touching() {
            self.tuple_pattern()?;
        }
        Ok(())
    }

    /// `_binding_pattern_no_expr` (hidden): a binding, `_`, a tuple, a case pattern, a name
    /// (`bound_identifier`), a cast.
    pub(super) fn binding_pattern_no_expr(&mut self) -> R<()> {
        self.binding_pattern_no_expr_inner(true)
    }

    pub(super) fn binding_pattern_no_expr_inner(&mut self, bound: bool) -> R<()> {
        // `catch let e as T`: the cast's pattern holds the binding (a switch case's does not).
        let m = self.mark();
        match self.peek() {
            T::Name if matches!(self.word(), w::LET | w::VAR) => {
                self.value_binding_pattern(f::NONE)?;
                // `case let .failure(e)`.
                if self.peek() == T::Dot {
                    self.case_pattern()?;
                } else {
                    self.no_expr_pattern(true)?;
                }
            }
            T::Name if self.is_word(w::CASE) => {
                self.tk(K::TCase)?;
                if matches!(self.word(), w::LET | w::VAR) {
                    self.value_binding_pattern(f::NONE)?;
                    // `case let .failure(e)`.
                    if self.peek() == T::Dot {
                        self.case_pattern()?;
                    } else {
                        self.no_expr_pattern(true)?;
                    }
                } else {
                    self.case_pattern()?;
                }
            }
            T::Name if self.is_word(w::IS) => {
                self.tk(K::TIs)?;
                self.ty_f(f::NONE)?;
            }
            T::Name if self.is_word(w::UNDERSCORE) => {
                self.leaf(K::WildcardPattern, f::NONE)?;
            }
            T::Name => {
                self.simple_identifier_f(if bound { f::BOUND_IDENTIFIER } else { f::NONE })?;
            }
            T::Dot => self.case_pattern()?,
            T::LParen => self.tuple_pattern()?,
            _ => return self.refuse(),
        }
        if self.is_word(w::AS) {
            let n = self.t.wrap(m, K::Pattern, f::NONE);
            self.close(n);
            self.tk(K::TAs)?;
            self.ty_f(f::NONE)?;
        }
        if self.peek() == T::Question && self.touching() {
            self.tk(K::TQuestion)?;
        }
        self.t.unmark(m);
        Ok(())
    }
}
