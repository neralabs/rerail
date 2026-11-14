//! Types and names: `type` (named, primitive, optional, union, intersection and disjunctive normal
//! form), `qualified_name`, `relative_name`, and `_name`.

use super::*;

impl<'a> P<'a> {
    /// A `type` in slot `s`.
    pub(super) fn ty(&mut self, s: Slot) -> R<N> {
        if self.peek() == T::LParen {
            return self.dnf(s, None);
        }
        let first = self.types_atom(s)?;
        match self.peek() {
            T::Pipe if self.kind_at(1) == T::LParen => self.dnf(s, Some(first)),
            T::Pipe => {
                let n = self.wrap(first, K::UnionType, s, BARE);
                while self.peek() == T::Pipe {
                    if self.kind_at(1) == T::LParen {
                        return self.refuse();
                    }
                    self.tk(K::UnionType, K::TPipe)?;
                    self.types_atom(BARE)?;
                }
                self.close(n);
                Ok(n)
            }
            // `A & B` (not `A &$x`, `A &...$x`: a reference).
            T::Amp if !matches!(self.kind_at(1), T::Dollar | T::DotDotDot) => {
                let n = self.wrap(first, K::IntersectionType, s, BARE);
                while self.peek() == T::Amp && !matches!(self.kind_at(1), T::Dollar | T::DotDotDot) {
                    self.tk(K::IntersectionType, K::TAmp)?;
                    self.types_atom(BARE)?;
                }
                if self.peek() == T::Pipe {
                    return self.refuse();
                }
                self.close(n);
                Ok(n)
            }
            _ => Ok(first),
        }
    }

    /// `disjunctive_normal_form_type`: `(A & B) | C | …`, `first` read already where given.
    fn dnf(&mut self, s: Slot, first: Option<N>) -> R<N> {
        let n = match first {
            Some(f) => self.wrap(f, K::DisjunctiveNormalFormType, s, BARE),
            None => self.open_in(K::DisjunctiveNormalFormType, s)?,
        };
        let mut need = first.is_none();
        loop {
            if !need {
                if self.peek() != T::Pipe {
                    break;
                }
                self.tk(K::DisjunctiveNormalFormType, K::TPipe)?;
            }
            need = false;
            if self.peek() == T::LParen {
                self.tk(K::DisjunctiveNormalFormType, K::TLParen)?;
                let i = self.open(K::IntersectionType, f::NONE, C_NONE)?;
                self.types_atom(BARE)?;
                while self.peek() == T::Amp {
                    self.tk(K::IntersectionType, K::TAmp)?;
                    self.types_atom(BARE)?;
                }
                self.close(i);
                self.tk(K::DisjunctiveNormalFormType, K::TRParen)?;
            } else {
                self.types_atom(BARE)?;
            }
        }
        self.close(n);
        Ok(n)
    }

    /// `_types`: `optional_type`, `named_type` or `primitive_type`.
    fn types_atom(&mut self, s: Slot) -> R<N> {
        if self.peek() == T::Question {
            let n = self.open_in(K::OptionalType, s)?;
            self.tk(K::OptionalType, K::TQuestion)?;
            if !self.primitive_opt(BARE)? {
                self.named_type(BARE)?;
            }
            self.close(n);
            return Ok(n);
        }
        let at = self.w.len();
        if self.primitive_opt(s)? {
            return Ok(N { at, kind: K::PrimitiveType });
        }
        self.named_type(s)
    }

    /// A `primitive_type` where the next word is one; whether it was.
    fn primitive_opt(&mut self, s: Slot) -> R<bool> {
        let t = self.tok();
        if t.kind != T::Name {
            return Ok(false);
        }
        // Lower case alone: `array`, `bool`, `float`, `int`, `null`, `object`, `string` (a token
        // each); any case: `callable`, `false`, `iterable`, `mixed`, `true`, `void` (the kind's
        // own pattern, no token).
        let exact = match t.word {
            w::ARRAY | w::BOOL | w::FLOAT | w::INT | w::NULL | w::OBJECT | w::STRING => true,
            w::CALLABLE | w::FALSE | w::ITERABLE | w::MIXED | w::TRUE | w::VOID => false,
            _ => return Ok(false),
        };
        if exact {
            if !self.is_exact_at(0, t.word) {
                return self.refuse();
            }
            let n = self.open_in(K::PrimitiveType, s)?;
            let k = keyword(t.word).expect("a type word");
            self.leaf(k, f::NONE, C_NONE)?;
            self.close(n);
        } else {
            self.leaf_in(K::PrimitiveType, s)?;
        }
        Ok(true)
    }

    /// Whether token `k` ahead is a `primitive_type`'s word as tree-sitter lexes it (`false` in
    /// any case, `array` in lower case).
    pub(super) fn is_type_word_at(&mut self, k: usize) -> bool {
        match self.word_at(k) {
            c @ (w::ARRAY | w::BOOL | w::FLOAT | w::INT | w::NULL | w::OBJECT | w::STRING) => self.is_exact_at(k, c),
            w::CALLABLE | w::FALSE | w::ITERABLE | w::MIXED | w::TRUE | w::VOID => true,
            _ => false,
        }
    }

    /// `named_type`: a name (its reserved set `classes`), a `qualified_name` or a `relative_name`.
    pub(super) fn named_type(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::NamedType, s)?;
        match self.peek() {
            T::Name if self.is_word(w::NAMESPACE) && self.kind_at(1) == T::Backslash => {
                self.relative_name(BARE)?;
            }
            // `static` (a return type): tree-sitter lexes it as a name there.
            T::Name if self.is_word(w::STATIC) && self.kind_at(1) != T::Backslash => {
                self.leaf(K::Name, f::NONE, C_NONE)?;
            }
            T::Name if self.kind_at(1) != T::Backslash => {
                self.class_name(BARE)?;
            }
            T::Name | T::Backslash => {
                self.qualified_name(BARE)?;
            }
            _ => return self.refuse(),
        }
        self.close(n);
        Ok(n)
    }

    /// `_name`: `static` (a `name`), a name (its reserved set `classes`), a `qualified_name` or a
    /// `relative_name`.
    pub(super) fn class_ref_name(&mut self, s: Slot) -> R<N> {
        match self.peek() {
            T::Name if self.is_word(w::NAMESPACE) && self.kind_at(1) == T::Backslash => self.relative_name(s),
            T::Name if self.is_word(w::STATIC) && self.kind_at(1) != T::Backslash => self.leaf_in(K::Name, s),
            T::Name if self.kind_at(1) != T::Backslash => self.class_name(s),
            T::Name | T::Backslash => self.qualified_name(s),
            _ => self.refuse(),
        }
    }

    /// `qualified_name`: `\`? names between `\`s, the last a name whose reserved set is `classes`.
    pub(super) fn qualified_name(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::QualifiedName, s)?;
        if self.peek() == T::Backslash {
            self.tk_field(K::QualifiedName, K::TBackslash, f::PREFIX)?;
        }
        if self.kind_at(1) == T::Backslash {
            // The names before the last: a `namespace_name`.
            let ns = self.open(K::NamespaceName, f::PREFIX, C_NONE)?;
            self.name_any(BARE)?;
            while self.peek() == T::Backslash && self.kind_at(1) == T::Name && self.kind_at(2) == T::Backslash {
                self.tk(K::NamespaceName, K::TBackslash)?;
                self.name_any(BARE)?;
            }
            self.close(ns);
            if self.peek() != T::Backslash {
                return self.refuse();
            }
            self.tk_field(K::QualifiedName, K::TBackslash, f::PREFIX)?;
        }
        self.class_name(BARE)?;
        self.close(n);
        Ok(n)
    }

    /// `relative_name`: `namespace\` names, the last a name whose reserved set is `classes`.
    pub(super) fn relative_name(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::RelativeName, s)?;
        self.tk_field(K::RelativeName, K::TNamespace, f::PREFIX)?;
        if self.kind_at(1) == T::Name && self.kind_at(2) == T::Backslash {
            self.tk_field(K::RelativeName, K::TBackslash, f::PREFIX)?;
            let ns = self.open(K::NamespaceName, f::PREFIX, C_NONE)?;
            self.name_any(BARE)?;
            while self.peek() == T::Backslash && self.kind_at(1) == T::Name && self.kind_at(2) == T::Backslash {
                self.tk(K::NamespaceName, K::TBackslash)?;
                self.name_any(BARE)?;
            }
            self.close(ns);
        }
        self.tk_field(K::RelativeName, K::TBackslash, f::PREFIX)?;
        self.class_name(BARE)?;
        self.close(n);
        Ok(n)
    }
}
