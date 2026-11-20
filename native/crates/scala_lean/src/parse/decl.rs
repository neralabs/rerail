//! Definitions (`_definition`): packages, imports, classes, objects, traits, `def`, `val`, `var`,
//! `type`; their annotations, modifiers, parameters and template bodies.

use super::{P, Region};
use crate::kinds::{K, f};
use crate::lex::{R, T, op, w};
use crate::scan::{Ext, v};

impl P<'_> {
    /// Whether a definition starts here (its annotations and modifiers first).
    pub(super) fn definition_ahead(&mut self) -> R<bool> {
        let t = self.tok();
        if t.kind == T::Op && t.code == op::AT {
            return Ok(true);
        }
        if t.kind != T::Id {
            return Ok(false);
        }
        Ok(match t.code {
            w::ABSTRACT | w::FINAL | w::SEALED | w::IMPLICIT | w::LAZY | w::OVERRIDE | w::PRIVATE | w::PROTECTED => true,
            w::CLASS | w::OBJECT | w::TRAIT | w::DEF | w::VAL | w::VAR | w::TYPE | w::IMPORT | w::EXPORT | w::PACKAGE | w::ENUM | w::GIVEN => true,
            w::CASE => self.enum_cases || matches!(self.word_at(1), w::CLASS | w::OBJECT),
            w::EXTENSION => matches!(self.kind_at(1), T::LParen | T::LBracket),
            w::INLINE | w::OPAQUE | w::OPEN | w::INFIX | w::TRANSPARENT | w::ERASED | w::INTO | w::TRACKED | w::UPDATE | w::CONSUME => {
                // At a statement's start no separator is valid (the one before it is read).
                matches!(self.with_fol(0, |p| p.layout(v::MODS))?, Some(Ext::Modifier(_)))
            }
            _ => false,
        })
    }

    /// A definition, given `field`.
    pub(super) fn definition(&mut self, field: u8) -> R<()> {
        let m = self.mark();
        while self.is_op(op::AT) {
            self.annotation()?;
        }
        self.modifiers()?;
        let t = self.tok();
        if t.kind != T::Id {
            return self.refuse();
        }
        let (kind, rest): (K, fn(&mut Self) -> R<()>) = match t.code {
            w::IMPORT => (K::ImportDeclaration, Self::import_rest),
            w::EXPORT => (K::ExportDeclaration, Self::import_rest),
            w::PACKAGE if self.word_at(1) == w::OBJECT => (K::PackageObject, Self::package_object_rest),
            w::PACKAGE => (K::PackageClause, Self::package_rest),
            w::CLASS => (K::ClassDefinition, Self::class_rest),
            w::OBJECT => (K::ObjectDefinition, Self::object_rest),
            w::TRAIT => (K::TraitDefinition, Self::class_rest),
            w::CASE if self.word_at(1) == w::CLASS => (K::ClassDefinition, Self::class_rest),
            w::CASE if self.word_at(1) == w::OBJECT => (K::ObjectDefinition, Self::object_rest),
            w::DEF => (K::FunctionDefinition, Self::def_rest),
            w::VAL | w::VAR => {
                let kind = self.val_kind()?;
                (kind, Self::val_rest)
            }
            w::TYPE => (K::TypeDefinition, Self::type_rest),
            w::CASE if self.enum_cases => (K::EnumCaseDefinitions, Self::enum_case_rest),
            w::ENUM => (K::EnumDefinition, Self::enum_rest),
            w::GIVEN => (K::GivenDefinition, Self::given_rest),
            w::EXTENSION => (K::ExtensionDefinition, Self::extension_rest),
            _ => return self.refuse(),
        };
        let s = self.tok().start;
        let n = self.t.wrap_from(m, kind, field, s);
        rest(self)?;
        if matches!(kind, K::EnumDefinition | K::GivenDefinition | K::PackageObject | K::ClassDefinition | K::ObjectDefinition | K::TraitDefinition | K::FunctionDefinition | K::ValDefinition | K::VarDefinition | K::TypeDefinition) {
            self.end_marker()?;
        }
        let id = self.close(n);
        if let Some(k) = self.retype.take() {
            self.t.retype(id, k);
        }
        self.t.unmark(m);
        Ok(())
    }

    /// `end name` on the line after a definition (`_end_marker_named_tail`: the semicolon, then
    /// `end_marker`: the `end` the scanner's, unwritten, then the name as `_end_ident`).
    fn end_marker(&mut self) -> R<()> {
        if !(self.tok().nl && self.is_word(w::END)) {
            return Ok(());
        }
        let nx = self.peek_at(1);
        if nx.nl || !(nx.kind == T::Id || nx.kind == T::BackId) {
            return Ok(());
        }
        if self.layout(v::SEMI)? != Some(Ext::Semi) {
            return Ok(());
        }
        self.take();
        if self.layout(v::END_KW)? != Some(Ext::EndKw) {
            return self.refuse();
        }
        self.take();
        let n = self.open(K::EndMarker);
        let e = self.bump()?;
        self.t.last_end = e.end;
        match nx.code {
            w::VAL | w::GIVEN | w::THIS | w::EXTENSION if nx.kind == T::Id => return self.refuse(),
            _ if w::is_reserved(nx.code) && nx.kind == T::Id => return self.refuse(),
            _ => {
                self.tk(K::TEndIdent)?;
            }
        }
        self.close(n);
        Ok(())
    }

    /// `@T(args)` (`annotation`: `name`, `arguments`).
    pub(super) fn annotation(&mut self) -> R<()> {
        let n = self.open(K::Annotation);
        self.tk(K::TAt)?;
        let t = self.simple_type()?;
        self.t.refield(t, f::NAME);
        while self.peek() == T::LParen && self.touching() {
            self.arguments(f::ARGUMENTS)?;
        }
        if self.peek() == T::LParen && !self.tok().nl {
            return self.refuse();
        }
        self.close(n);
        Ok(())
    }

    /// `modifiers`: the run of modifiers, in one node.
    /// The node of a soft modifier's word (`inline` → `inline_modifier`, …).
    fn soft_modifier(code: u8) -> K {
        match code {
            w::INLINE => K::InlineModifier,
            w::OPAQUE => K::OpaqueModifier,
            w::OPEN => K::OpenModifier,
            w::INFIX => K::InfixModifier,
            w::TRANSPARENT => K::TransparentModifier,
            w::ERASED => K::ErasedModifier,
            w::INTO => K::IntoModifier,
            w::TRACKED => K::TrackedModifier,
            w::UPDATE => K::UpdateModifier,
            _ => K::ConsumeModifier,
        }
    }

    fn modifiers(&mut self) -> R<()> {
        let mut n = None;
        loop {
            let t = self.tok();
            let kind = match t.code {
                _ if t.kind != T::Id => break,
                w::ABSTRACT => K::TAbstract,
                w::FINAL => K::TFinal,
                w::SEALED => K::TSealed,
                w::IMPLICIT => K::TImplicit,
                w::LAZY => K::TLazy,
                w::OVERRIDE => K::TOverride,
                w::PRIVATE | w::PROTECTED => {
                    if n.is_none() {
                        n = Some(self.open(K::Modifiers));
                    }
                    let a = self.open(K::AccessModifier);
                    self.tk(if t.code == w::PRIVATE { K::TPrivate } else { K::TProtected })?;
                    if self.peek() == T::LBracket && self.touching() {
                        let q = self.open(K::AccessQualifier);
                        self.tk(K::TLBracket)?;
                        self.identifier(f::NONE)?;
                        self.expect(T::RBracket)?;
                        self.close(q);
                    }
                    self.close(a);
                    continue;
                }
                w::INLINE | w::OPAQUE | w::OPEN | w::INFIX | w::TRANSPARENT | w::ERASED | w::INTO | w::TRACKED | w::UPDATE | w::CONSUME => {
                    // Among modifiers no separator is valid.
                    if !matches!(self.with_fol(0, |p| p.layout(v::MODS))?, Some(Ext::Modifier(_))) {
                        break;
                    }
                    // The scanner's token (`_inline_modifier`, …): the word, a named leaf.
                    self.take();
                    let kind = Self::soft_modifier(t.code);
                    if t.code == w::OPAQUE {
                        // `opaque` is the type definition's own, after its modifiers.
                        if let Some(n) = n.take() {
                            self.close(n);
                        }
                        self.leaf(kind, f::NONE)?;
                        if !self.is_word(w::TYPE) {
                            return self.refuse();
                        }
                        return Ok(());
                    }
                    if n.is_none() {
                        n = Some(self.open(K::Modifiers));
                    }
                    self.leaf(kind, f::NONE)?;
                    continue;
                }
                _ => break,
            };
            if n.is_none() {
                n = Some(self.open(K::Modifiers));
            }
            self.tk(kind)?;
        }
        if let Some(n) = n {
            self.close(n);
        }
        Ok(())
    }

    // ---- Imports and packages.

    /// `import a.b.{c, d => e}, f._` (`import_declaration`).
    /// `import` or `export` (`export_declaration`, the same shape), then its paths.
    fn import_rest(&mut self) -> R<()> {
        let kind = if self.is_word(w::EXPORT) { K::TExport } else { K::TImport };
        self.tk(kind)?;
        loop {
            self.namespace_expression()?;
            if self.peek() == T::Comma {
                self.tk(K::TComma)?;
            } else {
                return Ok(());
            }
        }
    }

    /// `_namespace_expression`: a path (`path` on each name and `.`), then `._`, `.given`,
    /// `.{…}` or `.x as y` after it.
    fn namespace_expression(&mut self) -> R<()> {
        if self.word_at(1) == w::AS {
            return self.refuse();
        }
        loop {
            let t = self.tok();
            // `extension` in a path: tree-sitter takes its keyword (`identifier` reads it so).
            if !(Self::is_name(t) || t.kind == T::Id && matches!(t.code, w::ENUM | w::EXPORT | w::EXTENSION)) {
                return self.refuse();
            }
            if t.kind == T::Id && matches!(t.code, w::ENUM | w::EXPORT) {
                self.leaf(K::Identifier, f::PATH)?;
            } else {
                self.identifier(f::PATH)?;
            }
            if self.peek() != T::Dot {
                return Ok(());
            }
            let nx = self.peek_at(1);
            match nx.kind {
                T::Id if nx.code == w::UNDERSCORE || nx.code == w::GIVEN => {
                    self.tk(K::TDot)?;
                    if nx.code == w::GIVEN && !matches!(self.kind_at(1), T::Comma | T::Semi | T::Eof) && !self.peek_at(1).nl {
                        return self.refuse();
                    }
                    let n = self.open(K::NamespaceWildcard);
                    self.bump().map(|t| self.t.token(if nx.code == w::GIVEN { K::TGiven } else { K::TUnderscore }, f::NONE, t.start, t.end))?;
                    self.close(n);
                    return Ok(());
                }
                T::Op if nx.code == op::STAR => {
                    self.tk(K::TDot)?;
                    let n = self.open(K::NamespaceWildcard);
                    self.tk(K::TStar)?;
                    self.close(n);
                    return Ok(());
                }
                T::LBrace => {
                    self.tk(K::TDot)?;
                    return self.namespace_selectors();
                }
                T::Id | T::BackId if self.word_at(2) == w::AS => {
                    // `a.B as C` (`as_renamed_identifier`; its `.` of no field).
                    self.tk(K::TDot)?;
                    let x = self.open(K::AsRenamedIdentifier);
                    self.identifier(f::NAME)?;
                    self.tk(K::TAs)?;
                    if self.is_word(w::UNDERSCORE) {
                        let y = self.open_field(K::Wildcard, f::ALIAS);
                        self.tk(K::TUnderscore)?;
                        self.close(y);
                    } else {
                        self.identifier(f::ALIAS)?;
                    }
                    self.close(x);
                    return Ok(());
                }
                T::Id | T::BackId => {
                    self.tkf(K::TDot, f::PATH)?;
                }
                // `cats.~>`: an operator's name.
                T::Op if !op::is_reserved(nx.code) => {
                    self.tkf(K::TDot, f::PATH)?;
                    self.leaf(K::OperatorIdentifier, f::PATH)?;
                    return Ok(());
                }
                _ => return self.refuse(),
            }
        }
    }

    /// `{a, b => c, _}` (`namespace_selectors`).
    fn namespace_selectors(&mut self) -> R<()> {
        let n = self.open(K::NamespaceSelectors);
        self.tk(K::TLBrace)?;
        loop {
            let t = self.tok();
            if t.kind == T::Id && (t.code == w::UNDERSCORE || t.code == w::GIVEN) || t.kind == T::Op && t.code == op::STAR {
                if t.code == w::GIVEN && t.kind == T::Id && !matches!(self.kind_at(1), T::Comma | T::RBrace) {
                    return self.refuse();
                }
                let x = self.open(K::NamespaceWildcard);
                let k = if t.kind == T::Op { K::TStar } else if t.code == w::GIVEN { K::TGiven } else { K::TUnderscore };
                self.tk(k)?;
                self.close(x);
            } else if self.is_op_at(1, op::ARROW) {
                let x = self.open(K::ArrowRenamedIdentifier);
                self.identifier(f::NAME)?;
                self.tk(K::TFatArrow)?;
                if self.is_word(w::UNDERSCORE) {
                    let y = self.open_field(K::Wildcard, f::ALIAS);
                    self.tk(K::TUnderscore)?;
                    self.close(y);
                } else {
                    self.identifier(f::ALIAS)?;
                }
                self.close(x);
            } else if self.word_at(1) == w::AS {
                let x = self.open(K::AsRenamedIdentifier);
                self.identifier(f::NAME)?;
                self.tk(K::TAs)?;
                if self.is_word(w::UNDERSCORE) {
                    let y = self.open_field(K::Wildcard, f::ALIAS);
                    self.tk(K::TUnderscore)?;
                    self.close(y);
                } else {
                    self.identifier(f::ALIAS)?;
                }
                self.close(x);
            } else {
                self.identifier(f::NONE)?;
            }
            if self.peek() == T::Comma {
                self.tk(K::TComma)?;
                if self.peek() == T::RBrace {
                    break;
                }
            } else {
                break;
            }
        }
        self.expect(T::RBrace)?;
        self.close(n);
        Ok(())
    }

    /// `package a.b` (`package_clause`: `name` a `package_identifier`), a body after it in braces.
    fn package_rest(&mut self) -> R<()> {
        self.tk(K::TPackage)?;
        let n = self.open_field(K::PackageIdentifier, f::NAME);
        loop {
            self.identifier(f::NONE)?;
            if self.peek() == T::Dot {
                self.tk(K::TDot)?;
            } else {
                break;
            }
        }
        self.close(n);
        // `package a { … }` and `package a:` then indented statements: a template body, `body`.
        if self.peek() == T::LBrace && !self.tok().nl {
            self.template_body(f::BODY)?;
        } else if self.is_op(op::COLON) && !self.tok().nl {
            self.indented_template_body(f::BODY)?;
        }
        Ok(())
    }

    // ---- Classes, objects, traits.

    /// `class C[T](a: A) extends B with D { … }`, a trait alike (`name`, `type_parameters`,
    /// `class_parameters`, `extend`, `body`).
    /// `class A @Inject() (…)` (`_constructor_annotation`, an `annotation`: `@` kept, the type
    /// `name`, then `(` and `)`, each an `arguments`: the alias is over the bracket pair). An
    /// annotation with arguments is refused.
    fn constructor_annotation(&mut self) -> R<()> {
        let a = self.open(K::Annotation);
        // Kept: the annotation's automaton (the plain annotation's) drops it.
        self.stored(K::TAt, f::NONE, true)?;
        let t = self.simple_type()?;
        self.t.refield(t, f::NAME);
        if self.peek() == T::LParen && self.touching() {
            if self.kind_at(1) != T::RParen {
                return self.refuse();
            }
            for _ in 0..2 {
                let p = self.bump()?;
                self.t.leaf(K::Arguments, f::NONE, p.start, p.end);
            }
        }
        self.close(a);
        Ok(())
    }

    fn class_rest(&mut self) -> R<()> {
        if self.is_word(w::CASE) {
            self.tk(K::TCase)?;
        }
        match self.word() {
            w::CLASS => self.tk(K::TClass)?,
            _ => self.tk(K::TTrait)?,
        };
        self.identifier(f::NAME)?;
        if self.peek() == T::LBracket {
            self.type_parameters(f::TYPE_PARAMETERS)?;
        }
        if self.is_op(op::AT) && !self.tok().nl {
            self.constructor_annotation()?;
        }
        // A constructor's access modifier (`class A private (…)`), on the header's line.
        if matches!(self.word(), w::PRIVATE | w::PROTECTED) && !self.tok().nl {
            let a = self.open(K::AccessModifier);
            let code = self.word();
            self.tk(if code == w::PRIVATE { K::TPrivate } else { K::TProtected })?;
            if self.peek() == T::LBracket && self.touching() {
                let q = self.open(K::AccessQualifier);
                self.tk(K::TLBracket)?;
                self.identifier(f::NONE)?;
                self.expect(T::RBracket)?;
                self.close(q);
            }
            self.close(a);
        }
        while self.peek() == T::LParen {
            if self.tok().nl {
                return self.refuse();
            }
            self.class_parameters()?;
        }
        self.template_tail()
    }

    /// `package object p extends B { … }` (`package_object`: `name`, `extend`, `body`).
    fn package_object_rest(&mut self) -> R<()> {
        self.tk(K::TPackage)?;
        self.tk(K::TObject)?;
        self.identifier(f::NAME)?;
        self.template_tail()
    }

    /// `object O extends B { … }` (`name`, `extend`, `body`).
    fn object_rest(&mut self) -> R<()> {
        if self.is_word(w::CASE) {
            self.tk(K::TCase)?;
        }
        self.tk(K::TObject)?;
        self.identifier(f::NAME)?;
        self.template_tail()
    }

    /// A template's `extends` clause, `derives` clause and body.
    fn template_tail(&mut self) -> R<()> {
        // What the header's state takes after it: `extends`, `derives`, a body.
        const HEAD: crate::scan::V = v::EXTENDS | v::DERIVES | v::COLON_EOL;
        if self.is_word(w::EXTENDS) && self.continues(HEAD)? {
            self.extends_clause()?;
        }
        if self.is_word(w::DERIVES) && self.continues(HEAD)? {
            self.derives_clause()?;
        }
        if self.peek() == T::LBrace {
            // `_definition_body`: a semicolon the scanner gives before the `{` is its (its dynamic
            // precedence: the braces are the body).
            if self.tok().nl {
                match self.layout(HEAD)? {
                    Some(Ext::Semi) => {
                        self.take();
                    }
                    None => {}
                    Some(_) => return self.refuse(),
                }
            }
            self.template_body(f::BODY)?;
        } else if self.is_op(op::COLON) && !self.tok().nl {
            self.indented_template_body(f::BODY)?;
        }
        Ok(())
    }

    /// `derives A, b.C` (`derives_clause`, field `derive`: each name `type`).
    fn derives_clause(&mut self) -> R<()> {
        let n = self.open_field(K::DerivesClause, f::DERIVE);
        self.tk(K::TDerives)?;
        loop {
            let t = self.type_path()?;
            self.t.refield(t, f::TYPE);
            if self.peek() == T::Comma {
                self.tk(K::TComma)?;
            } else {
                break;
            }
        }
        self.close(n);
        Ok(())
    }

    /// `extends A(x) with B` (`extends_clause`, field `extend`: each type `type`).
    fn extends_clause(&mut self) -> R<()> {
        const HEAD: crate::scan::V = v::EXTENDS | v::DERIVES | v::COLON_EOL;
        let n = self.open_field(K::ExtendsClause, f::EXTEND);
        self.tk(K::TExtends)?;
        self.constructor_application(f::TYPE)?;
        while self.is_word(w::WITH) && self.continues(v::WITH | HEAD)? {
            self.tkf(K::TWith, f::TYPE)?;
            self.constructor_application(f::TYPE)?;
        }
        if self.peek() == T::Comma {
            return self.refuse();
        }
        self.close(n);
        Ok(())
    }

    /// `enum E[T](p) extends B: cases` (`enum_definition`: `name`, `type_parameters`,
    /// `class_parameters`, `extend`, `body` an `enum_body`).
    fn enum_rest(&mut self) -> R<()> {
        self.tk(K::TEnum)?;
        self.identifier(f::NAME)?;
        if self.peek() == T::LBracket {
            self.type_parameters(f::TYPE_PARAMETERS)?;
        }
        while self.peek() == T::LParen && !self.tok().nl {
            self.class_parameters()?;
        }
        if self.is_word(w::EXTENDS) && !self.tok().nl {
            self.extends_clause()?;
        }
        if self.is_word(w::DERIVES) && !self.tok().nl {
            self.derives_clause()?;
        }
        let saved = std::mem::replace(&mut self.enum_cases, true);
        let r = if self.is_op(op::COLON) && !self.tok().nl {
            self.indented_body(K::EnumBody, f::BODY)
        } else if self.peek() == T::LBrace && !self.tok().nl {
            let n = self.open_field(K::EnumBody, f::BODY);
            self.tk(K::TLBrace)?;
            let r = self.statements(Region::Brace, f::NONE);
            self.enum_cases = saved;
            r?;
            self.expect(T::RBrace)?;
            self.close(n);
            Ok(())
        } else {
            self.refuse()
        };
        self.enum_cases = saved;
        r
    }

    /// `case A, B extends C` (`simple_enum_case`s: `name`, `extend`), or `case A(x) extends B`
    /// (`full_enum_case`: `name`, `class_parameters`, `extend`); `case` and the commas left out.
    fn enum_case_rest(&mut self) -> R<()> {
        self.tk(K::TCase)?;
        if matches!(self.kind_at(1), T::LParen | T::LBracket) && self.adjacent(0) {
            let n = self.open(K::FullEnumCase);
            self.identifier(f::NAME)?;
            if self.peek() == T::LBracket {
                self.type_parameters(f::TYPE_PARAMETERS)?;
            }
            while self.peek() == T::LParen && !self.tok().nl {
                self.class_parameters()?;
            }
            if self.is_word(w::EXTENDS) && !self.tok().nl {
                self.extends_clause()?;
            }
            self.close(n);
            return Ok(());
        }
        loop {
            let n = self.open(K::SimpleEnumCase);
            self.identifier(f::NAME)?;
            if self.is_word(w::EXTENDS) && !self.tok().nl {
                self.extends_clause()?;
            }
            self.close(n);
            if self.peek() == T::Comma {
                self.tk(K::TComma)?;
            } else {
                return Ok(());
            }
        }
    }

    /// `given T = e`, `given n: T = e`, `given n: T with` and an indented body (`given_definition`:
    /// `name`, `return_type`, `body`). Other forms are refused.
    fn given_rest(&mut self) -> R<()> {
        self.tk(K::TGiven)?;
        let named = Self::is_name(self.tok()) && !self.tok().nl;
        if named && self.is_op_at(1, op::COLON) {
            self.identifier(f::NAME)?;
            self.tk(K::TColon)?;
        } else if let Some(colon) = self.given_constructor_ahead(named) {
            if !colon {
                // Conditional signatures (`_given_sig`: `[T: C] =>`, `(x: A) =>`): the type
                // parameters unfielded, the parameters a `given_conditional`, each before `=>`.
                if named {
                    return self.refuse();
                }
                while matches!(self.peek(), T::LBracket | T::LParen) {
                    if self.peek() == T::LBracket {
                        self.type_parameters(f::NONE)?;
                    } else {
                        self.parameters_as(K::GivenConditional, f::NONE)?;
                    }
                    if !self.is_op(op::ARROW) {
                        return self.refuse();
                    }
                    self.tk(K::TFatArrow)?;
                }
                return self.given_type_and_body();
            }
            // `_given_constructor`: name, type parameters, parameter lists, `:`.
            if named {
                self.identifier(f::NAME)?;
            }
            if self.peek() == T::LBracket {
                self.type_parameters(f::TYPE_PARAMETERS)?;
            }
            while self.peek() == T::LParen && !self.tok().nl {
                self.parameters()?;
            }
            if !self.is_op(op::COLON) {
                return self.refuse();
            }
            self.tk(K::TColon)?;
        }
        self.given_type_and_body()
    }

    /// `_given_type_and_body`: the return type, then `= e`, `with` and a body, or none.
    fn given_type_and_body(&mut self) -> R<()> {
        let t = self.annotated_type()?;
        self.t.refield(t, f::RETURN_TYPE);
        if self.is_op(op::EQ) {
            self.tk(K::TEq)?;
            self.indentable_f(f::BODY)?;
        } else if self.is_word(w::WITH) && !self.tok().nl {
            // `_structural_instance`: its `with` in the return type's field, then the body.
            self.tkf(K::TWith, f::RETURN_TYPE)?;
            if self.peek() == T::LBrace && !self.tok().nl {
                // `with { … }` (`with_template_body`, braced).
                let n = self.open_field(K::WithTemplateBody, f::BODY);
                self.tk(K::TLBrace)?;
                let saved = std::mem::replace(&mut self.enum_cases, false);
                let r = self.statements(Region::Brace, f::NONE);
                self.enum_cases = saved;
                r?;
                self.expect(T::RBrace)?;
                self.close(n);
                return Ok(());
            }
            match self.layout(v::INDENT)? {
                Some(Ext::Indent) => {}
                _ => return self.refuse(),
            }
            let at = self.take();
            let n = self.t.open(K::WithTemplateBody, f::BODY, at);
            let saved = std::mem::replace(&mut self.enum_cases, false);
            let r = self.statements(Region::Indent, f::NONE);
            self.enum_cases = saved;
            r?;
            match self.layout(v::OUTDENT | v::COMMA_OUTDENT)? {
                Some(Ext::Outdent | Ext::CommaOutdent) => {
                    let at = self.take();
                    self.close_at(n, at);
                }
                _ => return self.refuse(),
            }
        }
        // Else a declaration (`_given_body` is optional).
        Ok(())
    }

    /// After `given`: whether `name[…](…)` (or the brackets alone) then `:` (`Some(true)`) or `=>`
    /// (`Some(false)`) comes next, rather than a type `M[T]`.
    fn given_constructor_ahead(&mut self, named: bool) -> Option<bool> {
        let mut j = usize::from(named);
        if !matches!(self.kind_at(j), T::LParen | T::LBracket) {
            return None;
        }
        while matches!(self.kind_at(j), T::LParen | T::LBracket) {
            j = self.matching(j) + 1;
        }
        let t = self.peek_at(j);
        if t.kind != T::Op {
            return None;
        }
        match t.code {
            op::COLON => Some(true),
            op::ARROW | op::QARROW => Some(false),
            _ => None,
        }
    }

    /// `extension (x: T) def …`, or its definitions indented (`extension_definition`:
    /// `type_parameters`, `parameters`, each definition `body`).
    fn extension_rest(&mut self) -> R<()> {
        self.tk(K::TExtension)?;
        if self.peek() == T::LBracket {
            self.type_parameters(f::TYPE_PARAMETERS)?;
        }
        while self.peek() == T::LParen {
            if self.tok().nl {
                return self.refuse();
            }
            self.parameters()?;
        }
        if self.tok().nl {
            match self.layout(v::INDENT)? {
                Some(Ext::Indent) => {
                    self.take();
                }
                _ => return self.refuse(),
            }
            self.statements(Region::Indent, f::BODY)?;
            match self.layout(v::OUTDENT | v::COMMA_OUTDENT)? {
                Some(Ext::Outdent | Ext::CommaOutdent) => {
                    let at = self.take();
                    self.t.flush(at);
                    self.t.last_end = self.t.last_end.max(at);
                }
                _ => return self.refuse(),
            }
            return Ok(());
        }
        if self.peek() == T::LBrace {
            // `{ … }` (`_extension_template_body`, field `body` on all of it).
            self.tkf(K::TLBrace, f::BODY)?;
            self.statements(Region::Brace, f::BODY)?;
            if self.peek() != T::RBrace {
                return self.refuse();
            }
            self.tkf(K::TRBrace, f::BODY)?;
            return Ok(());
        }
        if !self.definition_ahead()? {
            return self.refuse();
        }
        self.definition(f::BODY)
    }

    /// Whether the token next is the construct's where it starts a line: the scanner's choice where
    /// the state takes `own` (a semicolon or an outdent there ends the construct).
    pub(super) fn continues(&mut self, own: crate::scan::V) -> R<bool> {
        if !self.tok().nl {
            return Ok(true);
        }
        Ok(match self.layout(own)? {
            None => true,
            Some(Ext::Semi | Ext::Outdent | Ext::CommaOutdent) => false,
            Some(_) => return self.refuse(),
        })
    }

    /// `:` at a line's end, then indented statements (`template_body`, field `body`: from the `:` to
    /// the outdent).
    pub(super) fn indented_template_body(&mut self, field: u8) -> R<()> {
        let saved = std::mem::replace(&mut self.enum_cases, false);
        let r = self.indented_body(K::TemplateBody, field);
        self.enum_cases = saved;
        r
    }

    /// `:` at a line's end, an indent, statements, the outdent: a node of `kind`, field `body`.
    fn indented_body(&mut self, kind: K, field: u8) -> R<()> {
        if self.layout(v::COLON_EOL)? != Some(Ext::ColonEol) {
            return self.refuse();
        }
        let n = self.open_field(kind, field);
        self.take();
        // The lexer's token for the same `:`.
        self.tk(K::TColon)?;
        match self.layout(v::INDENT)? {
            Some(Ext::Indent) => {
                self.take();
            }
            _ => return self.refuse(),
        }
        if self.self_type_ahead() {
            self.self_type()?;
            if self.layout(v::SEMI | v::OUTDENT)? == Some(Ext::Semi) {
                self.take();
            }
        }
        self.statements(Region::Indent, f::NONE)?;
        match self.layout(v::OUTDENT | v::COMMA_OUTDENT)? {
            Some(Ext::Outdent | Ext::CommaOutdent) => {
                let at = self.take();
                self.close_at(n, at);
                Ok(())
            }
            _ => self.refuse(),
        }
    }

    /// `{ type T <: A; def f: B }` (`refinement`): declarations, as a braced body reads them.
    pub(super) fn refinement(&mut self) -> R<()> {
        self.refinement_as(K::Refinement).map(|_| ())
    }

    /// `{ type T = … }` as a node of `kind` (`refinement`, `structural_type`): its place.
    pub(super) fn refinement_as(&mut self, kind: K) -> R<u32> {
        let saved = (std::mem::replace(&mut self.in_bound, false), std::mem::replace(&mut self.enum_cases, false));
        let n = self.open(kind);
        let r = (|| {
            self.tk(K::TLBrace)?;
            let indent = self.braced_indent()?;
            self.statements(if indent { Region::Indent } else { Region::Brace }, f::NONE)?;
            if indent {
                match self.layout(v::OUTDENT)? {
                    Some(Ext::Outdent) => {
                        self.take();
                    }
                    _ => return self.refuse(),
                }
            }
            self.expect(T::RBrace)?;
            Ok(())
        })();
        (self.in_bound, self.enum_cases) = saved;
        r?;
        Ok(self.close(n))
    }

    /// `{ … }` (`template_body`), given `field`: a self type, then statements.
    pub(super) fn template_body(&mut self, field: u8) -> R<u32> {
        let saved = std::mem::replace(&mut self.enum_cases, false);
        let r = self.template_body_in(field);
        self.enum_cases = saved;
        r
    }

    fn template_body_in(&mut self, field: u8) -> R<u32> {
        let n = self.open_field(K::TemplateBody, field);
        self.tk(K::TLBrace)?;
        // An indent inside the braces (`_braced_template_body2`: an indent before or after the self
        // type, statements, the outdent at the `}`).
        let mut indent = self.braced_indent()?;
        if self.self_type_ahead() {
            self.self_type()?;
            if !indent {
                indent = self.braced_indent()?;
            }
            // `_block` may open with semicolons: the line's end after the self type.
            if self.peek() != T::RBrace && self.layout(v::SEMI)? == Some(Ext::Semi) {
                self.take();
            }
        }
        self.statements(if indent { Region::Indent } else { Region::Brace }, f::NONE)?;
        if indent {
            match self.layout(v::OUTDENT)? {
                Some(Ext::Outdent) => {
                    self.take();
                }
                _ => return self.refuse(),
            }
            // `_braced_template_body2`: statements after the outdent are the body's too (an indent
            // the scanner measured past a line's leading comment ends before the `}`).
            if self.peek() != T::RBrace {
                self.statements(Region::Brace, f::NONE)?;
            }
        }
        self.expect(T::RBrace)?;
        Ok(self.close(n))
    }

    /// The indent the scanner gives at a line's start inside braces: whether it did.
    fn braced_indent(&mut self) -> R<bool> {
        if self.tok().nl && self.peek() != T::RBrace && self.layout(v::INDENT)? == Some(Ext::Indent) {
            self.take();
            return Ok(true);
        }
        Ok(false)
    }

    /// `name: T =>` (`self_type`: the name, its type; the `:` and `=>` left out).
    fn self_type(&mut self) -> R<()> {
        let n = self.open(K::SelfType);
        if self.is_word(w::UNDERSCORE) {
            let x = self.open(K::Wildcard);
            self.tk(K::TUnderscore)?;
            self.close(x);
        } else {
            self.identifier(f::NONE)?;
        }
        if self.is_op(op::COLON) {
            self.tk(K::TColon)?;
            self.typ_x(f::NONE, super::types::Tx { no_arrow: true })?;
        }
        if !self.is_op(op::ARROW) {
            return self.refuse();
        }
        self.tk(K::TFatArrow)?;
        self.close(n);
        Ok(())
    }

    /// Whether a self type starts the body (`self =>`, `self: T =>`, `this: T =>`).
    /// Whether the line break before the token `j` ahead continues a type: an operator or `with`
    /// ends the line before, or `with`, `&` or `|` starts this one (no automatic semicolon is
    /// valid inside a type).
    fn type_continues_at(&mut self, j: usize) -> bool {
        let t = self.peek_at(j);
        let before = self.peek_at(j - 1);
        before.kind == T::Op && before.code != op::COLON && before.code != op::EQ
            || before.kind == T::Id && before.code == w::WITH
            || t.kind == T::Id && t.code == w::WITH
            || t.kind == T::Op && (t.code == op::PIPE || self.text(t) == b"&")
    }

    fn self_type_ahead(&mut self) -> bool {
        let t = self.tok();
        if !(Self::is_name(t) || t.kind == T::Id && t.code == w::UNDERSCORE) {
            return false;
        }
        if self.is_op_at(1, op::ARROW) {
            return true;
        }
        if !self.is_op_at(1, op::COLON) {
            return false;
        }
        // `name: T =>` on the line, or on the next after an operator that continues the type
        // (`A &` then `B =>`).
        let mut j = 2;
        loop {
            let t = self.peek_at(j);
            if t.nl && !self.type_continues_at(j) || t.kind == T::Eof || j > 400 {
                return false;
            }
            if t.kind == T::Op && t.code == op::ARROW {
                return true;
            }
            j += 1;
        }
    }

    /// `(a: A, val b: B = 1)` (`class_parameters`).
    /// The rest of `(using A, B)` in node `n`: types alone, each `type`, their commas stored.
    fn using_types(&mut self, n: crate::tw::Node) -> R<()> {
        loop {
            self.param_type(f::TYPE)?;
            if self.peek() == T::Comma {
                self.tk(K::TComma)?;
            } else {
                break;
            }
        }
        self.expect(T::RParen)?;
        self.close(n);
        Ok(())
    }

    fn class_parameters(&mut self) -> R<()> {
        let n = self.open_field(K::ClassParameters, f::CLASS_PARAMETERS);
        self.tk(K::TLParen)?;
        if self.is_word(w::USING) {
            self.tk(K::TUsing)?;
            // `(using x: T)`, `(using inline x: T)`: parameters; else types alone.
            let k = usize::from(matches!(self.word(), w::INLINE | w::ERASED | w::CONSUME | w::TRACKED));
            if !(Self::is_name(self.peek_at(k)) && self.is_op_at(k + 1, op::COLON)) {
                return self.using_types(n);
            }
        } else if self.is_word(w::IMPLICIT) {
            self.tk(K::TImplicit)?;
        }
        while self.peek() != T::RParen {
            let c = self.open(K::ClassParameter);
            while self.is_op(op::AT) {
                self.annotation()?;
            }
            self.modifiers()?;
            match self.word() {
                w::VAL => {
                    self.tk(K::TVal)?;
                }
                w::VAR => {
                    self.tk(K::TVar)?;
                }
                _ => {}
            }
            self.identifier(f::NAME)?;
            if self.is_op(op::COLON) {
                self.tk(K::TColon)?;
                self.param_type(f::TYPE)?;
            }
            if self.is_op(op::EQ) {
                self.tk(K::TEq)?;
                self.expression_f(f::DEFAULT_VALUE)?;
            }
            self.close(c);
            if self.peek() == T::Comma {
                self.tk(K::TComma)?;
            } else {
                break;
            }
        }
        self.expect(T::RParen)?;
        self.close(n);
        Ok(())
    }

    // ---- def, val, var, type.

    /// `def f[T](a: A): R = body` (`function_definition`, or `function_declaration` without a
    /// body: the kind is set when the body is known).
    fn def_rest(&mut self) -> R<()> {
        self.tk(K::TDef)?;
        if self.is_word(w::EXTENSION) {
            // `def extension`: tree-sitter reads the keyword, in an identifier.
            self.extension_identifier(f::NAME)?;
        } else {
            self.identifier(f::NAME)?;
        }
        loop {
            match self.peek() {
                T::LParen if !self.tok().nl => self.parameters()?,
                T::LBracket if !self.tok().nl => {
                    self.type_parameters(f::PARAMETERS)?;
                }
                _ => break,
            }
        }
        if self.tok().nl && self.peek() == T::LParen {
            return self.refuse();
        }
        if self.is_op(op::COLON) {
            self.tk(K::TColon)?;
            self.typ_f(f::RETURN_TYPE)?;
        }
        if self.is_op(op::EQ) {
            self.tk(K::TEq)?;
            self.indentable_f(f::BODY)?;
        } else if self.peek() == T::LBrace && !self.tok().nl {
            let b = self.block()?;
            self.t.refield(b, f::BODY);
        } else {
            // A declaration: the node is retyped when it closes.
            self.retype = Some(K::FunctionDeclaration);
        }
        Ok(())
    }

    /// `(a: A, b: B = 1)` (`parameters`, field `parameters`).
    fn parameters(&mut self) -> R<()> {
        self.parameters_as(K::Parameters, f::PARAMETERS)
    }

    /// `(a: A, …)` as a node of `kind` (`given_conditional` too), given `field`.
    fn parameters_as(&mut self, kind: K, field: u8) -> R<()> {
        let n = self.open_field(kind, field);
        self.tk(K::TLParen)?;
        if self.is_word(w::USING) {
            self.tk(K::TUsing)?;
            // `(using x: T)`, `(using inline x: T)`: parameters; else types alone.
            let k = usize::from(matches!(self.word(), w::INLINE | w::ERASED | w::CONSUME | w::TRACKED));
            if !(Self::is_name(self.peek_at(k)) && self.is_op_at(k + 1, op::COLON)) {
                // `(using A, B)`: types alone, each `type`, their commas stored.
                return self.using_types(n);
            }
        } else if self.is_word(w::IMPLICIT) {
            self.tk(K::TImplicit)?;
        }
        while self.peek() != T::RParen {
            let c = self.open(K::Parameter);
            while self.is_op(op::AT) {
                self.annotation()?;
            }
            // `inline x: T` (and `erased`, …): the scanner's modifier token, a named leaf of the
            // parameter's.
            while matches!(self.word(), w::INLINE | w::ERASED | w::CONSUME | w::TRACKED) && Self::is_name(self.peek_at(1)) {
                if !matches!(self.layout(v::MODS)?, Some(Ext::Modifier(_))) {
                    return self.refuse();
                }
                self.take();
                let kind = Self::soft_modifier(self.word());
                self.leaf(kind, f::NONE)?;
            }
            match self.word() {
                w::VAL => {
                    self.tk(K::TVal)?;
                }
                w::VAR => {
                    self.tk(K::TVar)?;
                }
                _ => {}
            }
            self.identifier(f::NAME)?;
            if !self.is_op(op::COLON) {
                return self.refuse();
            }
            self.tk(K::TColon)?;
            self.param_type(f::TYPE)?;
            if self.is_op(op::EQ) {
                self.tk(K::TEq)?;
                self.expression_f(f::DEFAULT_VALUE)?;
            }
            self.close(c);
            if self.peek() == T::Comma {
                self.tk(K::TComma)?;
            } else {
                break;
            }
        }
        self.expect(T::RParen)?;
        self.close(n);
        Ok(())
    }

    /// Whether the `val`/`var` next is a definition (`= value`) or a declaration (`: T` alone).
    fn val_kind(&mut self) -> R<K> {
        let var = self.is_word(w::VAR);
        // To the `=` or the line's end, brackets balanced.
        let mut j = 1;
        let mut d = 0i32;
        loop {
            let t = self.peek_at(j);
            // A `:` starting a line continues the definition (`val x` then `: T = v` under it).
            let colon = t.kind == T::Op && t.code == op::COLON;
            if j > 1 && t.nl && d == 0 && !colon && !self.type_continues_at(j) || t.kind == T::Eof {
                break;
            }
            match t.kind {
                T::LParen | T::LBracket | T::LBrace => d += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    d -= 1;
                    if d < 0 {
                        break;
                    }
                }
                T::Semi if d == 0 => break,
                T::Op if t.code == op::EQ && d == 0 => {
                    return Ok(if var { K::VarDefinition } else { K::ValDefinition });
                }
                _ => {}
            }
            j += 1;
        }
        Ok(if var { K::VarDeclaration } else { K::ValDeclaration })
    }

    /// `val p: T = v` (`pattern`, `type`, `value`), `val a, b: T` (declaration: `name`, `type`).
    fn val_rest(&mut self) -> R<()> {
        let var = self.is_word(w::VAR);
        let def = matches!(self.val_kind()?, K::ValDefinition | K::VarDefinition);
        self.tk(if var { K::TVar } else { K::TVal })?;
        if !def {
            loop {
                self.identifier(f::NAME)?;
                if self.peek() == T::Comma {
                    self.tk(K::TComma)?;
                } else {
                    break;
                }
            }
            if !self.is_op(op::COLON) {
                return self.refuse();
            }
            self.tk(K::TColon)?;
            self.typ_f(f::TYPE)?;
            return Ok(());
        }
        // The pattern: names (`identifiers`) or a definition pattern.
        if Self::is_name(self.tok()) && self.kind_at(1) == T::Comma {
            let n = self.open_field(K::Identifiers, f::PATTERN);
            loop {
                self.identifier(f::NONE)?;
                if self.peek() == T::Comma {
                    self.tk(K::TComma)?;
                } else {
                    break;
                }
            }
            self.close(n);
        } else {
            let p = self.def_pattern()?;
            self.t.refield(p, f::PATTERN);
        }
        if self.is_op(op::COLON) {
            self.tk(K::TColon)?;
            self.typ_f(f::TYPE)?;
        }
        self.expect_op(op::EQ)?;
        self.indentable_f(f::VALUE)?;
        Ok(())
    }

    /// `_definition_pattern`: a pattern without `|`, `: T` or `*` at its top.
    fn def_pattern(&mut self) -> R<u32> {
        let t = self.tok();
        if Self::is_name(t) && t.code != w::UNDERSCORE && (self.is_op_at(1, op::COLON) || self.is_op_at(1, op::EQ)) {
            return self.identifier(f::NONE);
        }
        if t.kind == T::Id && t.code == w::UNDERSCORE && self.is_op_at(1, op::COLON) {
            // `val _: T = …`: the wildcard alone, the type the definition's.
            let n = self.open(K::Wildcard);
            self.tk(K::TUnderscore)?;
            return Ok(self.close(n));
        }
        self.pattern_f(f::NONE)
    }

    /// `type T[A] = U` (`type_definition`: `name`, `type_parameters`, `type`).
    fn type_rest(&mut self) -> R<()> {
        self.tk(K::TType)?;
        self.ident_as(K::TypeIdentifier, f::NAME)?;
        if self.peek() == T::LBracket {
            self.type_parameters(f::TYPE_PARAMETERS)?;
        }
        if self.is_op(op::CARET) {
            return self.refuse();
        }
        self.bounds(false)?;
        if self.is_op(op::EQ) {
            self.tk(K::TEq)?;
            self.typ_f(f::TYPE)?;
        }
        Ok(())
    }
}
