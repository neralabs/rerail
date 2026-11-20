//! Types: names, qualified and generic types, pointers, arrays and slices, maps, channels, function
//! types, structs (their fields, named or embedded) and interfaces (methods and type elements).

use super::*;

impl<'a> P<'a> {
    /// Whether the next token can start a type.
    pub(super) fn type_starts(&mut self) -> bool {
        let t = self.tok();
        match t.kind {
            T::Name => !reserved(t.word) || matches!(t.word, w::FUNC | w::MAP | w::CHAN | w::STRUCT | w::INTERFACE),
            T::Star | T::LBracket | T::LtMinus | T::LParen => true,
            _ => false,
        }
    }

    /// A type in slot `s`.
    pub(super) fn ty(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let r = self.ty_inner(s);
        self.depth -= 1;
        r
    }

    fn ty_inner(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Name if !reserved(t.word) => self.named_type(s),
            T::Name => match t.word {
                w::MAP => {
                    let n = self.open_in(K::MapType, s);
                    self.tk(K::MapType, K::TMap)?;
                    self.nested(|p| {
                        p.punct_tk(K::MapType, T::LBracket, false)?;
                        p.ty(slot(f::KEY, Ctx::Type))?;
                        p.punct_tk(K::MapType, T::RBracket, false)
                    })?;
                    self.ty(slot(f::VALUE, Ctx::Type))?;
                    self.close(n);
                    Ok(n)
                }
                w::CHAN => {
                    let n = self.open_in(K::ChannelType, s);
                    self.tk(K::ChannelType, K::TChan)?;
                    if self.peek() == T::LtMinus {
                        self.tk(K::ChannelType, K::TLtMinus)?;
                    }
                    self.ty(slot(f::VALUE, Ctx::Type))?;
                    self.close(n);
                    Ok(n)
                }
                w::FUNC => {
                    let n = self.open_in(K::FunctionType, s);
                    self.tk(K::FunctionType, K::TFunc)?;
                    self.signature()?;
                    self.close(n);
                    Ok(n)
                }
                w::STRUCT => self.struct_type(s),
                w::INTERFACE => self.interface_type(s),
                _ => self.refuse(),
            },
            T::LtMinus => {
                let n = self.open_in(K::ChannelType, s);
                self.tk(K::ChannelType, K::TLtMinus)?;
                self.kw(K::ChannelType, w::CHAN)?;
                self.ty(slot(f::VALUE, Ctx::Type))?;
                self.close(n);
                Ok(n)
            }
            T::Star => {
                let n = self.open_in(K::PointerType, s);
                self.tk(K::PointerType, K::TStar)?;
                self.ty(TYPE)?;
                self.close(n);
                Ok(n)
            }
            T::Tilde => {
                let n = self.open_in(K::NegatedType, s);
                self.tk(K::NegatedType, K::TTilde)?;
                self.ty(TYPE)?;
                self.close(n);
                Ok(n)
            }
            T::LParen => {
                let n = self.open_in(K::ParenthesizedType, s);
                self.nested(|p| {
                    p.tk(K::ParenthesizedType, K::TLParen)?;
                    p.ty(TYPE)?;
                    p.punct_tk(K::ParenthesizedType, T::RParen, false)
                })?;
                self.close(n);
                Ok(n)
            }
            T::LBracket => self.array_type(s),
            _ => self.refuse(),
        }
    }

    /// `[]T`, `[N]T` or `[...]T`.
    pub(super) fn array_type(&mut self, s: Slot) -> R<N> {
        let kind = match self.kind_at(1) {
            T::RBracket => K::SliceType,
            T::DotDotDot if self.kind_at(2) == T::RBracket => K::ImplicitLengthArrayType,
            _ => K::ArrayType,
        };
        let n = self.open_in(kind, s);
        self.nested(|p| {
            p.tk(kind, K::TLBracket)?;
            match kind {
                K::ArrayType => {
                    p.expr(slot(f::LENGTH, Ctx::Expr))?;
                }
                K::ImplicitLengthArrayType => p.tk(kind, K::TDotDotDot)?,
                _ => {}
            }
            p.punct_tk(kind, T::RBracket, false)
        })?;
        self.ty(slot(f::ELEMENT, Ctx::Type))?;
        self.close(n);
        Ok(n)
    }

    /// A type name: `T`, `pkg.T`, and type arguments after either (`T[int]`).
    fn named_type(&mut self, s: Slot) -> R<N> {
        let n = self.type_name(s)?;
        if self.peek() == T::LBracket && !self.at_nl() {
            let g = self.wrap(n, K::GenericType, s, slot(f::TYPE, Ctx::Bare));
            self.type_arguments()?;
            self.close(g);
            return Ok(g);
        }
        Ok(n)
    }

    /// `T` (a `type_identifier`) or `pkg.T` (a `qualified_type`).
    pub(super) fn type_name(&mut self, s: Slot) -> R<N> {
        if !self.is_ident() {
            return self.refuse();
        }
        if self.kind_at(1) == T::Dot && self.is_ident_at(2) {
            let n = self.open_in(K::QualifiedType, s);
            self.leaf(K::PackageIdentifier, f::PACKAGE, C_NONE)?;
            self.tk(K::QualifiedType, K::TDot)?;
            self.leaf(K::TypeIdentifier, f::NAME, C_NONE)?;
            self.close(n);
            return Ok(n);
        }
        self.leaf_in(K::TypeIdentifier, s)
    }

    /// `type_arguments`: `[` type elements `]`.
    pub(super) fn type_arguments(&mut self) -> R<()> {
        let n = self.open(K::TypeArguments, f::TYPE_ARGUMENTS, C_NONE);
        self.list(K::TypeArguments, T::RBracket, false, true, |p| p.type_elem(K::TypeElem, BARE).map(|_| ()))?;
        self.close(n);
        Ok(())
    }

    /// A `type_elem` (as `kind`: `type_constraint` in type parameters): types between `|`s.
    pub(super) fn type_elem(&mut self, kind: K, s: Slot) -> R<N> {
        let n = self.open_in(kind, s);
        loop {
            self.ty(TYPE)?;
            if self.peek() != T::Pipe || self.at_nl() {
                break;
            }
            self.tk(kind, K::TPipe)?;
        }
        self.close(n);
        Ok(n)
    }

    /// `struct` and its fields.
    pub(super) fn struct_type(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::StructType, s);
        self.tk(K::StructType, K::TStruct)?;
        let l = self.open(K::FieldDeclarationList, f::NONE, C_NONE);
        self.braced_lines(K::FieldDeclarationList, |p| p.field_declaration())?;
        self.close(l);
        self.close(n);
        Ok(n)
    }

    /// `{`, items each ended by a terminator (the last one's optional), `}`, under `parent`.
    pub(super) fn braced_lines(&mut self, parent: K, mut item: impl FnMut(&mut Self) -> R<()>) -> R<()> {
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        self.lines(|p| {
            p.tk(parent, K::TLBrace)?;
            while p.peek() != T::RBrace {
                item(p)?;
                p.terminator(parent, T::RBrace)?;
            }
            p.tk(parent, K::TRBrace)
        })
    }

    /// `field_declaration`: names and a type, or an embedded type (`T`, `*T`, `pkg.T`, `T[A]`);
    /// then a tag.
    fn field_declaration(&mut self) -> R<()> {
        let n = self.open(K::FieldDeclaration, f::NONE, C_NONE);
        if self.embedded_ahead() {
            if self.peek() == T::Star {
                self.tk(K::FieldDeclaration, K::TStar)?;
            }
            self.named_type(slot(f::TYPE, Ctx::Bare))?;
        } else {
            loop {
                if !self.is_ident() {
                    return self.refuse();
                }
                self.leaf(K::FieldIdentifier, f::NAME, C_NONE)?;
                if self.peek() != T::Comma {
                    break;
                }
                self.tk(K::FieldDeclaration, K::TComma)?;
            }
            self.ty(slot(f::TYPE, Ctx::Type))?;
        }
        if matches!(self.peek(), T::Quote | T::Raw) && !self.at_nl() {
            self.string(slot(f::TAG, Ctx::Bare))?;
        }
        self.close(n);
        Ok(())
    }

    /// Whether the field ahead is an embedded type: `*T`, or a name not followed by a type.
    fn embedded_ahead(&mut self) -> bool {
        if self.peek() == T::Star {
            return true;
        }
        let u = self.peek_at(1);
        if u.nl != NO_NL {
            return true;
        }
        match u.kind {
            T::Dot | T::Quote | T::Raw | T::Semi | T::RBrace => true,
            T::LBracket => self.kind_at(2) != T::RBracket && self.bracket_end(1).is_some_and(|e| {
                let t = self.peek_at(e);
                t.nl != NO_NL || matches!(t.kind, T::Quote | T::Raw | T::Semi | T::RBrace)
            }),
            _ => false,
        }
    }

    /// `interface` and its elements: methods and type elements.
    fn interface_type(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::InterfaceType, s);
        self.tk(K::InterfaceType, K::TInterface)?;
        self.braced_lines(K::InterfaceType, |p| {
            if p.is_ident() && p.kind_at(1) == T::LParen {
                let m = p.open(K::MethodElem, f::NONE, C_NONE);
                p.leaf(K::FieldIdentifier, f::NAME, C_NONE)?;
                p.signature()?;
                p.close(m);
            } else {
                p.type_elem(K::TypeElem, BARE)?;
            }
            Ok(())
        })?;
        self.close(n);
        Ok(n)
    }
}
