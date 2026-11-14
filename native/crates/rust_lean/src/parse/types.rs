//! Types: paths (their last segment a `type_identifier`), generics, references and pointers,
//! tuples and arrays, function types, `impl` and `dyn` traits, bounded types (`A + 'a`).

use super::*;

impl<'a> P<'a> {
    /// A type in slot `s`. With `plus`, `A + B` is a `bounded_type` (left-associative): where the
    /// rule the type ends is done at it (a reference's type, a cast's, a bound's), a `+` is not
    /// the type's (tree-sitter's `bounded_type` is at precedence -1).
    pub(super) fn ty(&mut self, s: Slot, plus: bool) -> R<N> {
        self.deeper()?;
        let mut n = self.type_atom(s)?;
        while plus && self.peek() == T::Plus {
            n = self.wrap(n, K::BoundedType, s, TYPE);
            self.tk(K::BoundedType, K::TPlus, f::NONE)?;
            match self.peek() {
                T::Lifetime => drop(self.lifetime_node(BARE)?),
                T::Name if self.is_word(w::USE) => self.use_bounds()?,
                _ => drop(self.type_atom(TYPE)?),
            }
            self.close(n);
        }
        self.depth -= 1;
        Ok(n)
    }

    /// `'a + Tr + …`, a `bounded_type` that starts with a lifetime or `use<…>`, in slot `s`.
    fn bounds_type(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::BoundedType, s);
        if self.peek() == T::Lifetime {
            self.lifetime_node(BARE)?;
        } else {
            self.use_bounds()?;
        }
        if self.peek() != T::Plus {
            return self.refuse();
        }
        let mut n = n;
        loop {
            self.tk(K::BoundedType, K::TPlus, f::NONE)?;
            match self.peek() {
                T::Lifetime => drop(self.lifetime_node(BARE)?),
                T::Name if self.is_word(w::USE) => self.use_bounds()?,
                _ => drop(self.type_atom(TYPE)?),
            }
            self.close(n);
            if self.peek() != T::Plus {
                return Ok(n);
            }
            n = self.wrap(n, K::BoundedType, s, TYPE);
        }
    }

    fn lifetime_node(&mut self, s: Slot) -> R<N> {
        self.quoted(K::Lifetime, s)
    }

    /// `use<'a, T>`.
    fn use_bounds(&mut self) -> R<()> {
        let n = self.open(K::UseBounds, f::NONE, C_NONE);
        self.tk(K::UseBounds, K::TUse, f::NONE)?;
        if self.peek() != T::Lt {
            return self.refuse();
        }
        self.tk(K::UseBounds, K::TLt, f::NONE)?;
        loop {
            self.split_gt();
            match self.peek() {
                T::Gt => break,
                T::Lifetime => self.lifetime(BARE)?,
                T::Name => drop(self.leaf(K::TypeIdentifier, f::NONE, C_NONE)?),
                _ => return self.refuse(),
            }
            self.split_gt();
            if self.peek() == T::Comma {
                let trailing = matches!(self.peek_at(1).kind, T::Gt | T::GtGt | T::Ge | T::GtGtEq);
                self.gap_tk(K::UseBounds, K::TComma, trailing)?;
                if trailing {
                    self.split_gt();
                    self.leaf(K::TGt, f::NONE, C_NONE)?;
                    self.close(n);
                    return Ok(());
                }
            }
        }
        self.tk(K::UseBounds, K::TGt, f::NONE)?;
        self.close(n);
        Ok(())
    }

    /// A type without `+`.
    fn type_atom(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Amp | T::AmpAmp => {
                if t.kind == T::AmpAmp {
                    self.split(T::Amp, 1);
                }
                let n = self.open_in(K::ReferenceType, s);
                self.tk(K::ReferenceType, K::TAmp, f::NONE)?;
                if self.peek() == T::Lifetime {
                    self.lifetime(BARE)?;
                }
                if self.is_word(w::MUT) {
                    self.leaf(K::MutableSpecifier, f::NONE, C_NONE)?;
                }
                self.ty(slot(f::TYPE, Ctx::Type), false)?;
                self.close(n);
                Ok(n)
            }
            T::Star => {
                let n = self.open_in(K::PointerType, s);
                self.tk(K::PointerType, K::TStar, f::NONE)?;
                if self.is_word(w::CONST) {
                    self.tk(K::PointerType, K::TConst, f::NONE)?;
                } else if self.is_word(w::MUT) {
                    self.leaf(K::MutableSpecifier, f::NONE, C_NONE)?;
                } else {
                    return self.refuse();
                }
                self.ty(slot(f::TYPE, Ctx::Type), false)?;
                self.close(n);
                Ok(n)
            }
            T::LParen if self.peek_at(1).kind == T::RParen => {
                let n = self.open_in(K::UnitType, s);
                self.tk(K::UnitType, K::TLParen, f::NONE)?;
                self.tk(K::UnitType, K::TRParen, f::NONE)?;
                self.close(n);
                Ok(n)
            }
            T::LParen => {
                let n = self.open_in(K::TupleType, s);
                self.list(K::TupleType, T::RParen, false, false, |p| {
                    p.ty(TYPE, true)?;
                    Ok(())
                })?;
                self.close(n);
                Ok(n)
            }
            T::LBracket => {
                let n = self.open_in(K::ArrayType, s);
                self.tk(K::ArrayType, K::TLBracket, f::NONE)?;
                self.ty(slot(f::ELEMENT, Ctx::Type), true)?;
                if self.peek() == T::Semi {
                    self.tk(K::ArrayType, K::TSemi, f::NONE)?;
                    let saved = self.no_struct;
                    self.no_struct = false;
                    self.expr(slot(f::LENGTH, Ctx::Expr), 0)?;
                    self.no_struct = saved;
                }
                self.expect_tk(K::ArrayType, T::RBracket, K::TRBracket)?;
                self.close(n);
                Ok(n)
            }
            T::Bang => {
                let n = self.open_in(K::NeverType, s);
                self.tk(K::NeverType, K::TBang, f::NONE)?;
                self.close(n);
                Ok(n)
            }
            T::Question => {
                let n = self.open_in(K::RemovedTraitBound, s);
                self.tk(K::RemovedTraitBound, K::TQuestion, f::NONE)?;
                self.ty(TYPE, false)?;
                self.close(n);
                Ok(n)
            }
            T::Lifetime => self.lifetime_node(s),
            T::Metavariable => self.leaf_in(K::Metavariable, s),
            T::Lt | T::LtLt | T::ColonColon => self.type_path(s),
            T::Name => match t.word {
                w::IMPL => {
                    let n = self.open_in(K::AbstractType, s);
                    self.tk(K::AbstractType, K::TImpl, f::NONE)?;
                    if self.is_word(w::FOR) && self.peek_at(1).kind == T::Lt {
                        self.tk(K::AbstractType, K::TFor, f::NONE)?;
                        self.type_parameters_in(f::NONE)?;
                    }
                    // `impl 'a + Tr`: a bound list that starts with a lifetime (or `use<…>`)
                    // is no type, so the whole list is the trait.
                    if self.peek() == T::Lifetime || self.is_word(w::USE) && self.peek_at(1).kind == T::Lt {
                        self.bounds_type(slot(f::TRAIT, Ctx::Bare))?;
                    } else {
                        self.ty(slot(f::TRAIT, Ctx::Bare), false)?;
                    }
                    self.close(n);
                    Ok(n)
                }
                w::DYN => {
                    let n = self.open_in(K::DynamicType, s);
                    self.tk(K::DynamicType, K::TDyn, f::NONE)?;
                    if self.is_word(w::FOR) && self.peek_at(1).kind == T::Lt {
                        self.higher_ranked(slot(f::TRAIT, Ctx::Bare))?;
                    } else {
                        self.ty(slot(f::TRAIT, Ctx::Bare), false)?;
                    }
                    self.close(n);
                    Ok(n)
                }
                w::FN | w::UNSAFE | w::EXTERN | w::FOR => self.function_type(s),
                w::ASYNC | w::CONST if self.is_word_at(1, w::FN) || self.is_word_at(1, w::UNSAFE) || self.is_word_at(1, w::EXTERN) => self.function_type(s),
                c if w::is_primitive(c) && self.peek_at(1).kind != T::ColonColon => self.leaf_in(K::PrimitiveType, s),
                _ => self.type_path(s),
            },
            _ => self.refuse(),
        }
    }

    /// `for<'a> unsafe extern "C" fn(…) -> T`.
    fn function_type(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::FunctionType, s);
        if self.is_word(w::FOR) {
            let l = self.open(K::ForLifetimes, f::NONE, C_NONE);
            self.tk(K::ForLifetimes, K::TFor, f::NONE)?;
            self.expect_tk(K::ForLifetimes, T::Lt, K::TLt)?;
            loop {
                self.lifetime(BARE)?;
                self.split_gt();
                match self.peek() {
                    T::Comma => {
                        let trailing = matches!(self.peek_at(1).kind, T::Gt | T::GtGt | T::Ge | T::GtGtEq);
                        self.gap_tk(K::ForLifetimes, K::TComma, trailing)?;
                        if trailing {
                            self.split_gt();
                            self.leaf(K::TGt, f::NONE, C_NONE)?;
                            break;
                        }
                    }
                    T::Gt => {
                        self.tk(K::ForLifetimes, K::TGt, f::NONE)?;
                        break;
                    }
                    _ => return self.refuse(),
                }
            }
            self.close(l);
            if !matches!(self.tok().word, w::FN | w::UNSAFE | w::EXTERN | w::ASYNC | w::CONST) || self.peek() != T::Name {
                return self.refuse();
            }
        }
        if !self.is_word(w::FN) {
            self.function_modifiers()?;
        }
        self.expect_word(w::FN)?;
        self.parameters(f::PARAMETERS)?;
        if self.peek() == T::Arrow {
            self.tk(K::FunctionType, K::TArrow, f::NONE)?;
            self.ty(slot(f::RETURN_TYPE, Ctx::Type), false)?;
        }
        self.close(n);
        Ok(n)
    }

    /// The node `n` (a path written as an expression's: `identifier`, `scoped_identifier`) as a
    /// type's, in slot `s`: an `identifier` is a `type_identifier`, a `scoped_identifier` a
    /// `scoped_type_identifier` whose name (at `name_at`) is one.
    fn type_name_of(&mut self, n: N, s: Slot, name_at: Option<usize>) -> N {
        match n.kind {
            K::Identifier => {
                self.rekind(n.at, K::TypeIdentifier, s);
                N { at: n.at, kind: K::TypeIdentifier }
            }
            K::ScopedIdentifier => {
                self.rekind(n.at, K::ScopedTypeIdentifier, s);
                if let Some(at) = name_at {
                    self.rekind(at, K::TypeIdentifier, slot(f::NAME, Ctx::Bare));
                }
                N { at: n.at, kind: K::ScopedTypeIdentifier }
            }
            _ => {
                self.rekey(n.at, s);
                n
            }
        }
    }

    /// A type's path: `A`, `a::B`, `<T as Tr>::A`, `A<T>`, `A<T>::B`, `Fn(A) -> B`, `m!(…)`.
    fn type_path(&mut self, s: Slot) -> R<N> {
        let mut name_at = None;
        let mut n = match self.peek() {
            T::Lt | T::LtLt => {
                let b = self.bracketed_type(slot(f::PATH, Ctx::Bare))?;
                if self.peek() != T::ColonColon || self.peek_at(1).kind != T::Name {
                    return self.refuse();
                }
                let n = self.scoped_segment(b, s)?;
                name_at = Some(self.w.len() - 1);
                n
            }
            T::ColonColon => {
                let n = self.open_in(K::ScopedIdentifier, s);
                self.tk(K::ScopedIdentifier, K::TColonColon, f::NONE)?;
                self.identifier(slot(f::NAME, Ctx::Bare))?;
                name_at = Some(self.w.len() - 1);
                self.close(n);
                n
            }
            _ => self.path_start(s)?,
        };
        loop {
            match self.peek() {
                T::ColonColon => match self.peek_at(1).kind {
                    T::Name => {
                        n = self.scoped_segment(n, s)?;
                        name_at = Some(self.w.len() - 1);
                    }
                    T::Lt => {
                        // `A::<T>`: a turbofish's `generic_type` (its `::` stored).
                        let inner = if n.kind == K::Identifier { K::TypeIdentifier } else { n.kind };
                        self.rekind(n.at, inner, slot(f::TYPE, Ctx::Bare));
                        let g = self.wrap(N { at: n.at, kind: inner }, K::GenericType, s, slot(f::TYPE, Ctx::Bare));
                        self.leaf(K::TColonColon, f::NONE, C_NONE)?;
                        self.type_arguments()?;
                        self.close(g);
                        n = g;
                        name_at = None;
                    }
                    _ => break,
                },
                T::Lt | T::LtLt => {
                    let t = self.type_name_of(n, slot(f::TYPE, Ctx::Bare), name_at);
                    let g = self.wrap(t, K::GenericType, s, slot(f::TYPE, Ctx::Bare));
                    self.type_arguments()?;
                    self.close(g);
                    if self.peek() == T::ColonColon && self.peek_at(1).kind == T::Name {
                        // `A<T>::B`: a scoped type's path.
                        let sc = self.wrap(g, K::ScopedTypeIdentifier, s, slot(f::PATH, Ctx::Bare));
                        self.tk(K::ScopedTypeIdentifier, K::TColonColon, f::NONE)?;
                        self.leaf(K::TypeIdentifier, f::NAME, C_NONE)?;
                        self.close(sc);
                        n = sc;
                        name_at = None;
                        continue;
                    }
                    return Ok(g);
                }
                T::LParen if matches!(n.kind, K::Identifier | K::ScopedIdentifier) => {
                    // `Fn(A) -> B`.
                    let t = self.type_name_of(n, slot(f::TRAIT, Ctx::Bare), name_at);
                    let ft = self.wrap(t, K::FunctionType, s, slot(f::TRAIT, Ctx::Bare));
                    self.parameters(f::PARAMETERS)?;
                    if self.peek() == T::Arrow {
                        self.tk(K::FunctionType, K::TArrow, f::NONE)?;
                        self.ty(slot(f::RETURN_TYPE, Ctx::Type), false)?;
                    }
                    self.close(ft);
                    return Ok(ft);
                }
                T::Bang if matches!(n.kind, K::Identifier | K::ScopedIdentifier) && matches!(self.peek_at(1).kind, T::LParen | T::LBracket | T::LBrace) => {
                    let m = self.wrap(n, K::MacroInvocation, s, slot(f::MACRO, Ctx::Bare));
                    self.tk(K::MacroInvocation, K::TBang, f::NONE)?;
                    self.token_tree(tt::Tt::Delim, f::NONE)?;
                    self.close(m);
                    return Ok(m);
                }
                _ => break,
            }
        }
        Ok(self.type_name_of(n, s, name_at))
    }

    /// `<T>`, `<T as Trait>`: a `bracketed_type`, a qualified path's start.
    pub(super) fn bracketed_type(&mut self, s: Slot) -> R<N> {
        if self.peek() == T::LtLt {
            self.split(T::Lt, 1);
        }
        let n = self.open_in(K::BracketedType, s);
        self.tk(K::BracketedType, K::TLt, f::NONE)?;
        let t = self.ty(TYPE, true)?;
        if self.is_word(w::AS) {
            let q = self.wrap(t, K::QualifiedType, BARE, slot(f::TYPE, Ctx::Type));
            self.tk(K::QualifiedType, K::TAs, f::NONE)?;
            self.ty(slot(f::ALIAS, Ctx::Type), true)?;
            self.close(q);
        }
        self.split_gt();
        self.expect_tk(K::BracketedType, T::Gt, K::TGt)?;
        self.close(n);
        Ok(n)
    }

    /// `<T, 'a, Item = U, 3, {N}, T: Bound>`.
    pub(super) fn type_arguments(&mut self) -> R<()> {
        if self.peek() == T::LtLt {
            self.split(T::Lt, 1);
        }
        let n = self.open(K::TypeArguments, f::TYPE_ARGUMENTS, C_NONE);
        let saved = self.no_struct;
        self.no_struct = false;
        self.tk(K::TypeArguments, K::TLt, f::NONE)?;
        loop {
            let t = self.tok();
            match t.kind {
                T::Lifetime => self.lifetime(BARE)?,
                T::LBrace => drop(self.block(BARE)?),
                T::Int | T::Float | T::Char | T::Str | T::RawStr => drop(self.literal(slot(f::NONE, Ctx::Literal))?),
                T::Name if matches!(t.word, w::TRUE | w::FALSE) => drop(self.literal(slot(f::NONE, Ctx::Literal))?),
                T::Name if self.binding_ahead() => {
                    let b = self.open(K::TypeBinding, f::NONE, C_NONE);
                    self.leaf(K::TypeIdentifier, f::NAME, C_NONE)?;
                    if matches!(self.peek(), T::Lt | T::LtLt) {
                        self.type_arguments()?;
                    }
                    self.tk(K::TypeBinding, K::TEq, f::NONE)?;
                    self.ty(slot(f::TYPE, Ctx::Type), true)?;
                    self.close(b);
                }
                _ => drop(self.ty(TYPE, true)?),
            }
            if self.peek() == T::Colon {
                self.trait_bounds(f::NONE)?;
            }
            self.split_gt();
            match self.peek() {
                T::Comma => {
                    let trailing = matches!(self.peek_at(1).kind, T::Gt | T::GtGt | T::Ge | T::GtGtEq);
                    self.gap_tk(K::TypeArguments, K::TComma, trailing)?;
                    if trailing {
                        self.split_gt();
                        self.leaf(K::TGt, f::NONE, C_NONE)?;
                        break;
                    }
                }
                T::Gt => {
                    self.tk(K::TypeArguments, K::TGt, f::NONE)?;
                    break;
                }
                _ => return self.refuse(),
            }
        }
        self.no_struct = saved;
        self.close(n);
        Ok(())
    }

    /// Whether a type binding starts here: `Name = …`, `Name<…> = …`.
    fn binding_ahead(&mut self) -> bool {
        match self.peek_at(1).kind {
            T::Eq => true,
            T::Lt => self.angle_end(1).is_some_and(|k| self.peek_at(k).kind == T::Eq),
            _ => false,
        }
    }

    /// The index past the `>` that closes the `<` at token `k` (counting `<<` and `>>` as two),
    /// `None` where a bracket or `;` closes first, or a `>>` closes it and one more (`C<D>> =`
    /// in `B<C<D>> = e` is no binding).
    pub(super) fn angle_end(&mut self, k: usize) -> Option<usize> {
        let mut angle = 0i32;
        let mut brackets = 0i32;
        let mut j = k;
        loop {
            let t = self.peek_at(j);
            match t.kind {
                T::Lt => angle += 1,
                T::LtLt => angle += 2,
                T::Gt | T::Ge => angle -= 1,
                T::GtGt | T::GtGtEq => angle -= 2,
                T::LParen | T::LBracket | T::LBrace => brackets += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    brackets -= 1;
                    if brackets < 0 {
                        return None;
                    }
                }
                T::Semi if brackets == 0 => return None,
                T::Eof => return None,
                _ => {}
            }
            j += 1;
            if angle < 0 {
                // `>>` closed this `<` and one before it: the end is within that token.
                return None;
            }
            if angle == 0 && j > k {
                return Some(j);
            }
        }
    }
}
