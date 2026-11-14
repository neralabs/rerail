//! Types, type arguments and parameters, dimensions, annotations and names, and the look aheads
//! that find where a type ends (`scan_type`).

use super::*;

impl<'a> P<'a> {
    /// A type in slot `s` (a `_type` or an `_unannotated_type`, by `s.ctx`): annotations make it an
    /// `annotated_type` (a `_type`'s alone); `dims` reads array dimensions after it.
    pub(super) fn ty(&mut self, s: Slot, dims: bool) -> R<N> {
        if self.peek() == T::At {
            if s.ctx != Ctx::Type {
                return self.refuse();
            }
            let n = self.open_in(K::AnnotatedType, s);
            while self.peek() == T::At {
                self.annotation(BARE)?;
            }
            self.unannotated(UTYPE, dims)?;
            self.close(n);
            return Ok(n);
        }
        self.unannotated(s, dims)
    }

    /// An `_unannotated_type` (or a `_simple_type`, without `dims`).
    pub(super) fn unannotated(&mut self, s: Slot, dims: bool) -> R<N> {
        self.deeper()?;
        let t = self.tok();
        let mut n = if t.kind == T::Name && w::is_primitive(t.word) {
            self.primitive(s)?
        } else if t.kind == T::Name && !reserved(t.word) {
            self.class_type(s)?
        } else {
            return self.refuse();
        };
        if dims && self.dims_ahead(0) {
            n = self.wrap(n, K::ArrayType, s, slot(f::ELEMENT, Ctx::UType));
            self.dimensions(f::DIMENSIONS)?;
            self.close(n);
        }
        self.depth -= 1;
        Ok(n)
    }

    /// A primitive type (`void` included): its node, the word stored where the kind is a choice.
    fn primitive(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        let kind = match t.word {
            w::BYTE | w::SHORT | w::INT | w::LONG | w::CHAR => K::IntegralType,
            w::FLOAT | w::DOUBLE => K::FloatingPointType,
            w::BOOLEAN => K::BooleanType,
            _ => K::VoidType,
        };
        if matches!(kind, K::IntegralType | K::FloatingPointType) {
            let n = self.open_in(kind, s);
            self.leaf(keyword(t.word).expect("a primitive"), f::NONE, C_NONE)?;
            self.close(n);
            Ok(n)
        } else {
            self.leaf_in(kind, s)
        }
    }

    /// A class type: a `type_identifier`, scoped (`a.B`) and generic (`B<C>`) as far as they go.
    fn class_type(&mut self, s: Slot) -> R<N> {
        let mut n = self.leaf_in(K::TypeIdentifier, s)?;
        loop {
            match self.peek() {
                T::Lt => {
                    n = self.wrap(n, K::GenericType, s, BARE);
                    self.type_arguments(f::NONE)?;
                    self.close(n);
                }
                T::Dot if self.kind_at(1) == T::At || self.is_ident_at(1) => {
                    n = self.wrap(n, K::ScopedTypeIdentifier, s, BARE);
                    self.tk(K::ScopedTypeIdentifier, K::TDot)?;
                    while self.peek() == T::At {
                        self.annotation(BARE)?;
                    }
                    if !self.is_ident() {
                        return self.refuse();
                    }
                    self.leaf(K::TypeIdentifier, f::NONE, C_NONE)?;
                    self.close(n);
                }
                _ => return Ok(n),
            }
        }
    }

    /// Whether array dimensions start `k` tokens ahead: annotations and `[` `]`.
    pub(super) fn dims_ahead(&mut self, k: usize) -> bool {
        let Some(k) = self.scan_annotations(k) else { return false };
        self.kind_at(k) == T::LBracket && self.kind_at(k + 1) == T::RBracket
    }

    /// `dimensions`: `[` `]` pairs, each after its annotations. A gap of more than one pair is not
    /// its kind's implied word (`[ ]`): its brackets are stored.
    pub(super) fn dimensions(&mut self, field: u8) -> R<N> {
        let n = self.open(K::Dimensions, field, C_NONE);
        while self.dims_ahead(0) {
            while self.peek() == T::At {
                self.annotation(BARE)?;
            }
            let mut pairs = 1;
            while self.kind_at(2 * pairs) == T::LBracket && self.kind_at(2 * pairs + 1) == T::RBracket {
                pairs += 1;
            }
            for _ in 0..pairs {
                self.gap_tk(K::Dimensions, K::TLBracket, pairs > 1)?;
                self.gap_tk(K::Dimensions, K::TRBracket, pairs > 1)?;
            }
        }
        self.close(n);
        Ok(n)
    }

    /// The `>` closing an angle list: a `>>`, `>>>`, `>=`… read as its first `>`.
    pub(super) fn close_angle(&mut self, parent: K, keep: bool) -> R<()> {
        match self.peek() {
            T::Gt => {}
            T::GtGt | T::GtGtGt | T::Ge | T::GtGtEq | T::GtGtGtEq => self.split_at(0, T::Gt, 1),
            _ => return self.refuse(),
        }
        self.gap_tk(parent, K::TGt, keep)
    }

    /// `type_arguments`: `<` types and wildcards `>` (`<>` empty).
    pub(super) fn type_arguments(&mut self, field: u8) -> R<N> {
        let n = self.open(K::TypeArguments, field, C_NONE);
        self.tk(K::TypeArguments, K::TLt)?;
        if !self.is_angle_close() {
            loop {
                if self.wildcard_ahead() {
                    self.wildcard()?;
                } else {
                    self.ty(TYPE, true)?;
                }
                if self.peek() != T::Comma {
                    break;
                }
                self.tk(K::TypeArguments, K::TComma)?;
            }
        }
        self.close_angle(K::TypeArguments, false)?;
        self.close(n);
        Ok(n)
    }

    fn is_angle_close(&mut self) -> bool {
        matches!(self.peek(), T::Gt | T::GtGt | T::GtGtGt | T::Ge | T::GtGtEq | T::GtGtGtEq)
    }

    fn wildcard_ahead(&mut self) -> bool {
        self.scan_annotations(0).is_some_and(|k| self.kind_at(k) == T::Question)
    }

    /// `wildcard`: annotations, `?`, and `extends T` or `super T`.
    fn wildcard(&mut self) -> R<()> {
        let n = self.open(K::Wildcard, f::NONE, C_NONE);
        while self.peek() == T::At {
            self.annotation(BARE)?;
        }
        self.tk(K::Wildcard, K::TQuestion)?;
        if self.is_word(w::EXTENDS) {
            self.tk(K::Wildcard, K::TExtends)?;
            self.ty(TYPE, true)?;
        } else if self.is_word(w::SUPER) {
            self.leaf(K::Super, f::NONE, C_NONE)?;
            self.ty(TYPE, true)?;
        }
        self.close(n);
        Ok(())
    }

    /// `type_parameters`: `<` parameters `>`, each annotations, a name and its bound.
    pub(super) fn type_parameters(&mut self, field: u8) -> R<()> {
        let n = self.open(K::TypeParameters, field, C_NONE);
        self.tk(K::TypeParameters, K::TLt)?;
        loop {
            let p = self.open(K::TypeParameter, f::NONE, C_NONE);
            while self.peek() == T::At {
                self.annotation(BARE)?;
            }
            if !self.is_ident() {
                return self.refuse();
            }
            self.leaf(K::TypeIdentifier, f::NONE, C_NONE)?;
            if self.is_word(w::EXTENDS) {
                let b = self.open(K::TypeBound, f::NONE, C_NONE);
                self.tk(K::TypeBound, K::TExtends)?;
                self.ty(TYPE, true)?;
                while self.peek() == T::Amp {
                    self.tk(K::TypeBound, K::TAmp)?;
                    self.ty(TYPE, true)?;
                }
                self.close(b);
            }
            self.close(p);
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::TypeParameters, K::TComma)?;
        }
        self.close_angle(K::TypeParameters, false)?;
        self.close(n);
        Ok(())
    }

    /// `kind` (`superclass`, `throws`…): its keyword, then types between `,`s (in a `type_list`
    /// where `list`).
    pub(super) fn type_clause(&mut self, kind: K, field: u8, word: K, list: bool) -> R<()> {
        let n = self.open(kind, field, C_NONE);
        self.tk(kind, word)?;
        let l = list.then(|| self.open(K::TypeList, f::NONE, C_NONE));
        let parent = if list { K::TypeList } else { kind };
        loop {
            self.ty(TYPE, true)?;
            if kind == K::Superclass || self.peek() != T::Comma {
                break;
            }
            self.tk(parent, K::TComma)?;
        }
        if let Some(l) = l {
            self.close(l);
        }
        self.close(n);
        Ok(())
    }

    // ---- Annotations and names.

    /// An annotation: `@`, a name, and an argument list where `(` follows (a `marker_annotation`
    /// without).
    pub(super) fn annotation(&mut self, s: Slot) -> R<N> {
        let mut k = 1;
        while self.is_ident_at(k) && self.kind_at(k + 1) == T::Dot {
            k += 2;
        }
        let args = self.kind_at(k + 1) == T::LParen;
        let kind = if args { K::Annotation } else { K::MarkerAnnotation };
        let n = self.open_in(kind, s);
        self.tk(kind, K::TAt)?;
        self.name(slot(f::NAME, Ctx::Bare))?;
        if args {
            self.annotation_arguments()?;
        }
        self.close(n);
        Ok(n)
    }

    /// `annotation_argument_list`: one element value, or `key = value` pairs.
    fn annotation_arguments(&mut self) -> R<()> {
        let n = self.open(K::AnnotationArgumentList, f::ARGUMENTS, C_NONE);
        let pairs = self.is_ident_at(1) && self.kind_at(2) == T::Eq;
        if pairs || self.kind_at(1) == T::RParen {
            self.list(K::AnnotationArgumentList, T::RParen, true, false, |p| {
                let e = p.open(K::ElementValuePair, f::NONE, C_NONE);
                if !p.is_ident() {
                    return p.refuse();
                }
                p.leaf(K::Identifier, f::KEY, C_NONE)?;
                p.punct_tk(K::ElementValuePair, T::Eq, false)?;
                p.element_value(slot(f::VALUE, Ctx::Expr))?;
                p.close(e);
                Ok(())
            })?;
        } else {
            self.tk(K::AnnotationArgumentList, K::TLParen)?;
            self.element_value(EXPR)?;
            self.punct_tk(K::AnnotationArgumentList, T::RParen, false)?;
        }
        self.close(n);
        Ok(())
    }

    /// An `_element_value`: an expression, an array of element values, or an annotation.
    pub(super) fn element_value(&mut self, s: Slot) -> R<()> {
        match self.peek() {
            T::LBrace => {
                let n = self.open(K::ElementValueArrayInitializer, s.field, C_NONE);
                self.list(K::ElementValueArrayInitializer, T::RBrace, true, true, |p| p.element_value(EXPR))?;
                self.close(n);
            }
            T::At => {
                self.annotation(slot(s.field, Ctx::Bare))?;
            }
            _ => {
                self.expr(s)?;
            }
        }
        Ok(())
    }

    /// A `_name`: an identifier, or `scoped_identifier`s (`a.b.c`).
    pub(super) fn name(&mut self, s: Slot) -> R<N> {
        if !self.is_ident() {
            return self.refuse();
        }
        let mut n = self.leaf_in(K::Identifier, s)?;
        while self.peek() == T::Dot && self.is_ident_at(1) {
            n = self.wrap(n, K::ScopedIdentifier, s, slot(f::SCOPE, Ctx::Bare));
            self.tk(K::ScopedIdentifier, K::TDot)?;
            self.leaf(K::Identifier, f::NAME, C_NONE)?;
            self.close(n);
        }
        Ok(n)
    }

    // ---- Look aheads.

    /// Past annotations from `k` ahead: where they end. An annotation's arguments are skipped by
    /// their brackets.
    pub(super) fn scan_annotations(&mut self, mut k: usize) -> Option<usize> {
        while self.kind_at(k) == T::At {
            k += 1;
            if !self.is_ident_at(k) {
                return None;
            }
            k += 1;
            while self.kind_at(k) == T::Dot && self.is_ident_at(k + 1) {
                k += 2;
            }
            if self.kind_at(k) == T::LParen {
                k = self.skip_brackets(k)?;
            }
        }
        Some(k)
    }

    /// From a bracket `k` ahead: just past its match. `None` at the end of the file.
    pub(super) fn skip_brackets(&mut self, mut k: usize) -> Option<usize> {
        let mut depth = 0u32;
        loop {
            match self.kind_at(k) {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(k + 1);
                    }
                }
                T::Eof => return None,
                _ => {}
            }
            k += 1;
        }
    }

    /// A type from `k` ahead (an `_unannotated_type`, with dimensions where `dims`): where it
    /// ends. `None` where no type starts there.
    pub(super) fn scan_type(&mut self, k: usize, dims: bool) -> Option<usize> {
        let t = self.peek_at(k);
        if t.kind != T::Name || (reserved(t.word) && !w::is_primitive(t.word)) {
            return None;
        }
        let mut k = k + 1;
        if !w::is_primitive(t.word) {
            loop {
                match self.kind_at(k) {
                    T::Lt => k = self.scan_type_arguments(k)?,
                    T::Dot => {
                        let j = self.scan_annotations(k + 1)?;
                        if !self.is_ident_at(j) {
                            break;
                        }
                        k = j + 1;
                    }
                    _ => break,
                }
            }
        }
        if dims {
            while self.dims_ahead(k) {
                k = self.scan_annotations(k)? + 2;
            }
        }
        Some(k)
    }

    /// Type arguments from the `<` `k` ahead: just past their `>`. `None` where a token no type
    /// holds comes first, or a `>>` closes past them.
    pub(super) fn scan_type_arguments(&mut self, k: usize) -> Option<usize> {
        let mut depth = 0i32;
        let mut k = k;
        loop {
            let t = self.peek_at(k);
            match t.kind {
                T::Lt => depth += 1,
                T::Gt => depth -= 1,
                T::GtGt => depth -= 2,
                T::GtGtGt => depth -= 3,
                T::Name if !reserved(t.word) || w::is_primitive(t.word) || matches!(t.word, w::EXTENDS | w::SUPER) => {}
                T::Dot | T::Comma | T::Question | T::Amp | T::LBracket | T::RBracket => {}
                T::At => {
                    k = self.scan_annotations(k)?;
                    continue;
                }
                _ => return None,
            }
            k += 1;
            if depth == 0 {
                return Some(k);
            }
            if depth < 0 {
                return None;
            }
        }
    }
}
