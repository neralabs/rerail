//! Declarations: the package clause, imports, `const`, `var` and `type` specs, functions and
//! methods, and parameter lists (whose names are told from types by the shape of their items).

use super::*;

impl<'a> P<'a> {
    /// A top-level item: a package clause, an import, a function or method, or a declaration.
    pub(super) fn toplevel(&mut self) -> R<()> {
        let t = self.tok();
        if t.kind != T::Name {
            return self.refuse();
        }
        match t.word {
            w::PACKAGE => {
                let n = self.open(K::PackageClause, f::NONE, C_NONE);
                self.tk(K::PackageClause, K::TPackage)?;
                if !self.is_ident() {
                    return self.refuse();
                }
                self.leaf(K::PackageIdentifier, f::NONE, C_NONE)?;
                self.close(n);
            }
            w::IMPORT => self.import_declaration()?,
            w::FUNC => self.function()?,
            w::CONST | w::VAR | w::TYPE => {
                self.declaration(STMT)?;
            }
            _ => return self.refuse(),
        }
        Ok(())
    }

    /// `import` and a spec, or `(` specs `)`.
    fn import_declaration(&mut self) -> R<()> {
        let n = self.open(K::ImportDeclaration, f::NONE, C_NONE);
        self.tk(K::ImportDeclaration, K::TImport)?;
        if self.peek() == T::LParen {
            let l = self.open(K::ImportSpecList, f::NONE, C_NONE);
            self.line_list(K::ImportSpecList, |p| p.import_spec())?;
            self.close(l);
        } else {
            self.import_spec()?;
        }
        self.close(n);
        Ok(())
    }

    /// `import_spec`: a name (`.`, `_` or a package's) and a path.
    fn import_spec(&mut self) -> R<()> {
        let n = self.open(K::ImportSpec, f::NONE, C_NONE);
        let t = self.tok();
        match t.kind {
            T::Dot => {
                // `dot`: a named node over its `.` token.
                let d = self.open(K::Dot, f::NAME, C_NONE);
                self.leaf(K::TDot, f::NONE, C_NONE)?;
                self.close(d);
            }
            T::Name if self.underscore(t) => {
                self.leaf(K::BlankIdentifier, f::NAME, C_NONE)?;
            }
            T::Name if !reserved(t.word) => {
                self.leaf(K::PackageIdentifier, f::NAME, C_NONE)?;
            }
            _ => {}
        }
        self.string(slot(f::PATH, Ctx::Bare))?;
        self.close(n);
        Ok(())
    }

    /// Whether `t` is `_` alone.
    pub(super) fn underscore(&self, t: Tok) -> bool {
        t.kind == T::Name && t.end == t.start + 1 && self.src[t.start as usize] == b'_'
    }

    /// A list of lines in parentheses under `parent` (its node open): `(`, items read by `item`,
    /// each ended by a terminator (the last one's optional), `)`.
    pub(super) fn line_list(&mut self, parent: K, mut item: impl FnMut(&mut Self) -> R<()>) -> R<()> {
        // In a declaration's own node (`type (…)`), the `(` is no gap's shortest word: a
        // declaration of one spec has none. The `)` after the last spec is implied.
        let keep = parent == K::TypeDeclaration;
        self.lines(|p| {
            p.punct_tk(parent, T::LParen, keep)?;
            while p.peek() != T::RParen {
                item(p)?;
                p.terminator(parent, T::RParen)?;
            }
            p.punct_tk(parent, T::RParen, false)
        })
    }

    /// A list of specs in parentheses whose every spec takes a terminator (`const`'s, `var`'s).
    fn spec_list(&mut self, parent: K, mut item: impl FnMut(&mut Self) -> R<()>) -> R<()> {
        let keep = parent == K::ConstDeclaration;
        self.lines(|p| {
            p.punct_tk(parent, T::LParen, keep)?;
            while p.peek() != T::RParen {
                item(p)?;
                let t = p.tok();
                if t.kind == T::Semi {
                    p.tk(parent, K::TSemi)?;
                } else if t.nl == NO_NL {
                    return p.refuse();
                } else {
                    p.flush(t.nl_last);
                    p.last_end = t.nl_last + 1;
                }
            }
            p.punct_tk(parent, T::RParen, false)
        })
    }

    /// A `const`, `var` or `type` declaration in slot `s`.
    pub(super) fn declaration(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.word {
            w::CONST => {
                let n = self.open_in(K::ConstDeclaration, s);
                self.tk(K::ConstDeclaration, K::TConst)?;
                if self.peek() == T::LParen {
                    self.spec_list(K::ConstDeclaration, |p| p.const_spec())?;
                } else {
                    self.const_spec()?;
                }
                self.close(n);
                Ok(n)
            }
            w::VAR => {
                let n = self.open_in(K::VarDeclaration, s);
                self.tk(K::VarDeclaration, K::TVar)?;
                if self.peek() == T::LParen {
                    let l = self.open(K::VarSpecList, f::NONE, C_NONE);
                    self.spec_list(K::VarSpecList, |p| p.var_spec())?;
                    self.close(l);
                } else {
                    self.var_spec()?;
                }
                self.close(n);
                Ok(n)
            }
            w::TYPE => {
                let n = self.open_in(K::TypeDeclaration, s);
                self.tk(K::TypeDeclaration, K::TType)?;
                if self.peek() == T::LParen {
                    self.line_list(K::TypeDeclaration, |p| p.type_spec())?;
                } else {
                    self.type_spec()?;
                }
                self.close(n);
                Ok(n)
            }
            _ => self.refuse(),
        }
    }

    /// Names: `identifier (, identifier)*`, each in field `field` (the `,`s too) under `parent`.
    fn names(&mut self, parent: K, field: u8) -> R<()> {
        loop {
            if !self.is_ident() {
                return self.refuse();
            }
            self.leaf(K::Identifier, field, C_NONE)?;
            if self.peek() != T::Comma {
                return Ok(());
            }
            // Only `const_spec`'s names are one field with their `,`s (`name: seq(…)`).
            self.tk_field(parent, K::TComma, if parent == K::ConstSpec { field } else { f::NONE })?;
        }
    }

    /// `const_spec`: names, then a type and `=` values, or `=` values, or nothing.
    fn const_spec(&mut self) -> R<()> {
        let n = self.open(K::ConstSpec, f::NONE, C_NONE);
        self.names(K::ConstSpec, f::NAME)?;
        if !self.at_nl() && !matches!(self.peek(), T::Semi | T::RParen | T::Eof) {
            if self.peek() != T::Eq {
                self.ty(slot(f::TYPE, Ctx::Type))?;
            }
            self.punct_tk(K::ConstSpec, T::Eq, false)?;
            self.expression_list(slot(f::VALUE, Ctx::Bare))?;
        }
        self.close(n);
        Ok(())
    }

    /// `var_spec`: names, then a type (and `=` values) or `=` values.
    fn var_spec(&mut self) -> R<()> {
        let n = self.open(K::VarSpec, f::NONE, C_NONE);
        self.names(K::VarSpec, f::NAME)?;
        if self.peek() != T::Eq {
            self.ty(slot(f::TYPE, Ctx::Type))?;
            if self.peek() == T::Eq && !self.at_nl() {
                self.tk(K::VarSpec, K::TEq)?;
                self.expression_list(slot(f::VALUE, Ctx::Bare))?;
            }
        } else {
            self.tk(K::VarSpec, K::TEq)?;
            self.expression_list(slot(f::VALUE, Ctx::Bare))?;
        }
        self.close(n);
        Ok(())
    }

    /// `type_spec` or `type_alias`: a name, type parameters, (`=`) and a type.
    fn type_spec(&mut self) -> R<()> {
        if !self.is_ident() {
            return self.refuse();
        }
        let alias = self.type_params_ahead(1).map_or(self.kind_at(1) == T::Eq, |k| self.kind_at(k) == T::Eq);
        let kind = if alias { K::TypeAlias } else { K::TypeSpec };
        let n = self.open(kind, f::NONE, C_NONE);
        self.leaf(K::TypeIdentifier, f::NAME, C_NONE)?;
        if self.type_params_ahead(0).is_some() {
            self.type_parameter_list()?;
        }
        if alias {
            self.tk(kind, K::TEq)?;
        }
        self.ty(slot(f::TYPE, Ctx::Type))?;
        self.close(n);
        Ok(())
    }

    /// Whether the tokens from `k` ahead are type parameters (`[T any]`, `[K comparable, V any]`,
    /// `[T, U any]`), not an array type's length (`[N]int`): a name, then names after `,`s, then a
    /// constraint before `]`. The token after their `]`, ahead.
    pub(super) fn type_params_ahead(&mut self, k: usize) -> Option<usize> {
        if self.kind_at(k) != T::LBracket || !self.is_ident_at(k + 1) {
            return None;
        }
        let mut j = k + 2;
        while self.kind_at(j) == T::Comma {
            if !self.is_ident_at(j + 1) {
                return None;
            }
            j += 2;
        }
        // A constraint starts here: no `]` (an array's length), and no operator after a name
        // (`[N * M]`, `[a.b]`: an expression).
        let t = self.peek_at(j);
        let starts = match t.kind {
            T::Name => !reserved(t.word) || matches!(t.word, w::INTERFACE | w::FUNC | w::MAP | w::CHAN | w::STRUCT),
            T::Star | T::LBracket | T::Tilde | T::LParen | T::LtMinus => true,
            _ => false,
        };
        if !starts {
            return None;
        }
        self.bracket_end(k)
    }

    /// The token after the bracket closing the one at `k` ahead (`(`, `[` or `{`).
    pub(super) fn bracket_end(&mut self, k: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut j = k;
        loop {
            match self.kind_at(j) {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j + 1);
                    }
                }
                T::Eof => return None,
                _ => {}
            }
            j += 1;
        }
    }

    /// `type_parameter_list`: `[` declarations `]`, each names and a constraint.
    pub(super) fn type_parameter_list(&mut self) -> R<()> {
        let n = self.open(K::TypeParameterList, f::TYPE_PARAMETERS, C_NONE);
        self.list(K::TypeParameterList, T::RBracket, false, true, |p| {
            let d = p.open(K::TypeParameterDeclaration, f::NONE, C_NONE);
            loop {
                p.leaf(K::Identifier, f::NAME, C_NONE)?;
                // Names until one is followed by its constraint: `[K, V any]` is one declaration.
                if p.peek() == T::Comma {
                    p.tk(K::TypeParameterDeclaration, K::TComma)?;
                    continue;
                }
                break;
            }
            p.type_elem(K::TypeConstraint, slot(f::TYPE, Ctx::Bare))?;
            p.close(d);
            Ok(())
        })?;
        self.close(n);
        Ok(())
    }

    /// A function or method declaration.
    fn function(&mut self) -> R<()> {
        let method = self.kind_at(1) == T::LParen;
        let kind = if method { K::MethodDeclaration } else { K::FunctionDeclaration };
        let n = self.open(kind, f::NONE, C_NONE);
        self.tk(kind, K::TFunc)?;
        if method {
            self.parameter_list(f::RECEIVER)?;
            if !self.is_ident() {
                return self.refuse();
            }
            self.leaf(K::FieldIdentifier, f::NAME, C_NONE)?;
        } else {
            if !self.is_ident() {
                return self.refuse();
            }
            self.leaf(K::Identifier, f::NAME, C_NONE)?;
            if self.peek() == T::LBracket {
                self.type_parameter_list()?;
            }
        }
        self.signature()?;
        if self.peek() == T::LBrace && !self.at_nl() {
            self.block(slot(f::BODY, Ctx::Bare))?;
        }
        self.close(n);
        Ok(())
    }

    /// Parameters and a result (`parameter_list` or a simple type), as a function's.
    pub(super) fn signature(&mut self) -> R<()> {
        self.parameter_list(f::PARAMETERS)?;
        if self.at_nl() {
            return Ok(());
        }
        if self.peek() == T::LParen {
            self.parameter_list(f::RESULT)?;
        } else if self.type_starts() {
            self.ty(slot(f::RESULT, Ctx::SType))?;
        }
        Ok(())
    }

    /// `parameter_list` in `field`: `(` declarations `)`. Where some item is a name and a type
    /// (`a int`), the items that are a name alone are names of the next declaration (`a, b int`);
    /// otherwise every item is a type.
    pub(super) fn parameter_list(&mut self, field: u8) -> R<()> {
        if self.peek() != T::LParen {
            return self.refuse();
        }
        let named = self.named_parameters();
        let n = self.open(K::ParameterList, field, C_NONE);
        self.list(K::ParameterList, T::RParen, true, true, |p| {
            if p.peek() == T::DotDotDot {
                let d = p.open(K::VariadicParameterDeclaration, f::NONE, C_NONE);
                p.tk(K::VariadicParameterDeclaration, K::TDotDotDot)?;
                p.ty(slot(f::TYPE, Ctx::Type))?;
                p.close(d);
                return Ok(());
            }
            if named {
                if !p.is_ident() {
                    return p.refuse();
                }
                if p.kind_at(1) == T::DotDotDot {
                    let d = p.open(K::VariadicParameterDeclaration, f::NONE, C_NONE);
                    p.leaf(K::Identifier, f::NAME, C_NONE)?;
                    p.tk(K::VariadicParameterDeclaration, K::TDotDotDot)?;
                    p.ty(slot(f::TYPE, Ctx::Type))?;
                    p.close(d);
                    return Ok(());
                }
                let d = p.open(K::ParameterDeclaration, f::NONE, C_NONE);
                loop {
                    p.leaf(K::Identifier, f::NAME, C_NONE)?;
                    if p.peek() == T::Comma {
                        p.tk(K::ParameterDeclaration, K::TComma)?;
                        if !p.is_ident() {
                            return p.refuse();
                        }
                        continue;
                    }
                    break;
                }
                p.ty(slot(f::TYPE, Ctx::Type))?;
                p.close(d);
                return Ok(());
            }
            let d = p.open(K::ParameterDeclaration, f::NONE, C_NONE);
            p.ty(slot(f::TYPE, Ctx::Type))?;
            p.close(d);
            Ok(())
        })?;
        self.close(n);
        Ok(())
    }

    /// Whether the parameter list ahead (at its `(`) names its parameters: some item is a name
    /// followed by a type (or `...`).
    fn named_parameters(&mut self) -> bool {
        let mut j = 1;
        loop {
            // An item from `j`.
            let t = self.peek_at(j);
            if t.kind == T::Name && !reserved(t.word) {
                let u = self.peek_at(j + 1);
                let named = match u.kind {
                    T::Comma | T::RParen | T::Dot => false,
                    T::LBracket => {
                        // `a []T` and `a [N]T` name; `T[int]` is a generic type.
                        self.kind_at(j + 2) == T::RBracket || self.bracket_end(j + 1).is_some_and(|e| !matches!(self.kind_at(e), T::Comma | T::RParen))
                    }
                    T::Eof => false,
                    _ => true,
                };
                if named {
                    return true;
                }
            }
            // To the item's end: its `,` at depth 0, or the list's `)`.
            let mut depth = 0usize;
            loop {
                match self.kind_at(j) {
                    T::LParen | T::LBracket | T::LBrace => depth += 1,
                    T::RParen | T::RBracket | T::RBrace => {
                        if depth == 0 {
                            return false;
                        }
                        depth -= 1;
                    }
                    T::Comma if depth == 0 => break,
                    T::Eof => return false,
                    _ => {}
                }
                j += 1;
            }
            j += 1;
        }
    }
}
