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
            w::CASE => matches!(self.word_at(1), w::CLASS | w::OBJECT),
            w::EXTENSION => matches!(self.kind_at(1), T::LParen | T::LBracket),
            w::INLINE | w::OPAQUE | w::OPEN | w::INFIX | w::TRANSPARENT | w::ERASED | w::INTO | w::TRACKED | w::UPDATE | w::CONSUME => {
                matches!(self.layout(v::MODS)?, Some(Ext::Modifier(_)))
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
            _ => return self.refuse(),
        };
        let s = self.tok().start;
        let n = self.t.wrap_from(m, kind, field, s);
        rest(self)?;
        if matches!(kind, K::PackageObject | K::ClassDefinition | K::ObjectDefinition | K::TraitDefinition | K::FunctionDefinition | K::ValDefinition | K::VarDefinition | K::TypeDefinition) {
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
                    if !matches!(self.layout(v::MODS)?, Some(Ext::Modifier(_))) {
                        break;
                    }
                    // The scanner's token: the word, a named leaf of its own.
                    return self.refuse();
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
    fn import_rest(&mut self) -> R<()> {
        self.tk(K::TImport)?;
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
            // `extension` in a path: tree-sitter takes its keyword (refused).
            if !(Self::is_name(t) || t.kind == T::Id && matches!(t.code, w::ENUM | w::EXPORT)) || t.kind == T::Id && t.code == w::EXTENSION {
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
                T::Id | T::BackId => {
                    if self.word_at(2) == w::AS {
                        return self.refuse();
                    }
                    self.tkf(K::TDot, f::PATH)?;
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
        if self.peek() == T::LBrace || self.is_op(op::COLON) {
            return self.refuse();
        }
        Ok(())
    }

    // ---- Classes, objects, traits.

    /// `class C[T](a: A) extends B with D { … }`, a trait alike (`name`, `type_parameters`,
    /// `class_parameters`, `extend`, `body`).
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
        if self.is_op(op::AT) {
            return self.refuse();
        }
        if matches!(self.word(), w::PRIVATE | w::PROTECTED) {
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
            let n = self.open_field(K::ExtendsClause, f::EXTEND);
            self.tk(K::TExtends)?;
            self.constructor_application(f::TYPE)?;
            loop {
                if self.is_word(w::WITH) && self.continues(v::WITH | HEAD)? {
                    self.tkf(K::TWith, f::TYPE)?;
                    self.constructor_application(f::TYPE)?;
                } else if self.peek() == T::Comma {
                    return self.refuse();
                } else {
                    break;
                }
            }
            self.close(n);
        }
        if self.is_word(w::DERIVES) && self.continues(HEAD)? {
            return self.refuse();
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
            self.indented_template_body()?;
        }
        Ok(())
    }

    /// Whether the token next is the construct's where it starts a line: the scanner's choice where
    /// the state takes `own` (a semicolon or an outdent there ends the construct).
    fn continues(&mut self, own: crate::scan::V) -> R<bool> {
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
    fn indented_template_body(&mut self) -> R<()> {
        if self.layout(v::COLON_EOL)? != Some(Ext::ColonEol) {
            return self.refuse();
        }
        let n = self.open_field(K::TemplateBody, f::BODY);
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

    /// `{ … }` (`template_body`), given `field`: a self type, then statements.
    pub(super) fn template_body(&mut self, field: u8) -> R<u32> {
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
        }
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
        // `name: T =>` on the line.
        let mut j = 2;
        loop {
            let t = self.peek_at(j);
            if t.nl || t.kind == T::Eof || j > 40 {
                return false;
            }
            if t.kind == T::Op && t.code == op::ARROW {
                return true;
            }
            j += 1;
        }
    }

    /// `(a: A, val b: B = 1)` (`class_parameters`).
    fn class_parameters(&mut self) -> R<()> {
        let n = self.open_field(K::ClassParameters, f::CLASS_PARAMETERS);
        self.tk(K::TLParen)?;
        if self.is_word(w::USING) {
            return self.refuse();
        }
        if self.is_word(w::IMPLICIT) {
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
            // `def extension`: tree-sitter reads the keyword.
            return self.refuse();
        }
        self.identifier(f::NAME)?;
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
        let n = self.open_field(K::Parameters, f::PARAMETERS);
        self.tk(K::TLParen)?;
        if self.is_word(w::USING) {
            // `(using c: C)`; `(using C)` (types alone) is refused.
            if !(Self::is_name(self.peek_at(1)) && self.is_op_at(2, op::COLON)) {
                return self.refuse();
            }
            self.tk(K::TUsing)?;
        } else if self.is_word(w::IMPLICIT) {
            self.tk(K::TImplicit)?;
        }
        while self.peek() != T::RParen {
            let c = self.open(K::Parameter);
            while self.is_op(op::AT) {
                self.annotation()?;
            }
            if matches!(self.word(), w::INLINE | w::ERASED | w::CONSUME | w::TRACKED) && Self::is_name(self.peek_at(1)) {
                return self.refuse();
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
            if j > 1 && t.nl && d == 0 || t.kind == T::Eof {
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
            return self.refuse();
        }
        let p = self.def_pattern()?;
        self.t.refield(p, f::PATTERN);
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
        self.pattern_f(f::NONE)
    }

    /// `type T[A] = U` (`type_definition`: `name`, `type_parameters`, `type`).
    fn type_rest(&mut self) -> R<()> {
        self.tk(K::TType)?;
        self.ident_as(K::TypeIdentifier, f::NAME)?;
        if self.peek() == T::LBracket {
            self.type_parameters(f::TYPE_PARAMETERS)?;
        }
        if self.is_op(op::SUPERTYPE) || self.is_op(op::SUBTYPE) || self.is_op(op::COLON) {
            return self.refuse();
        }
        if self.is_op(op::EQ) {
            self.tk(K::TEq)?;
            self.typ_f(f::TYPE)?;
        }
        Ok(())
    }
}
