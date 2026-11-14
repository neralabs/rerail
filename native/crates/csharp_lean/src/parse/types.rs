//! Types, names and type arguments, and the look aheads that find where a type ends
//! (`scan_type`).

use super::expr::starts_operand;
use super::*;

impl<'a> P<'a> {
    /// A `type` in slot `s`, with its suffixes (`?`, `*`, ranks).
    pub(super) fn ty(&mut self, s: Slot) -> R<N> {
        self.ty_with(s, false)
    }

    /// A type after `is` or `as`: a `?` is nullable only where no operand follows it (else it is
    /// a conditional's).
    pub(super) fn ty_in_expr(&mut self, s: Slot) -> R<N> {
        self.ty_with(s, true)
    }

    fn ty_with(&mut self, s: Slot, in_expr: bool) -> R<N> {
        self.deeper()?;
        let t = self.tok();
        let mut n = match t.kind {
            T::Name if t.word == w::REF => {
                let n = self.open_in(K::RefType, s)?;
                self.leaf(K::TRef, f::NONE, C_NONE)?;
                if self.is_word(w::READONLY) {
                    self.leaf(K::TReadonly, f::NONE, C_NONE)?;
                }
                self.ty(slot(f::TYPE, Ctx::Type))?;
                self.close(n);
                self.depth -= 1;
                return Ok(n);
            }
            T::Name if t.word == w::SCOPED && self.scan_type(1).is_some() => {
                let n = self.open_in(K::ScopedType, s)?;
                self.tk(K::ScopedType, K::TScoped)?;
                self.ty(slot(f::TYPE, Ctx::Bare))?;
                self.close(n);
                self.depth -= 1;
                return Ok(n);
            }
            T::Name if t.word == w::DELEGATE && self.kind_at(1) == T::Star => self.function_pointer_type(s)?,
            T::Name if t.word == w::VAR && !matches!(self.kind_at(1), T::Lt | T::Dot | T::ColonColon) => self.leaf_in(K::ImplicitType, s)?,
            T::Name if w::is_predefined(t.word) => self.leaf_in(K::PredefinedType, s)?,
            T::Name if !w::is_reserved(t.word) => self.name(s)?,
            T::LParen => self.tuple_type(s)?,
            _ => return self.refuse(),
        };
        loop {
            match self.peek() {
                T::Question if !n_is(n, &[K::PointerType]) => {
                    if in_expr && (starts_operand(self.peek_at(1)) || self.kind_at(1) == T::LBracket) {
                        break;
                    }
                    n = self.wrap(n, K::NullableType, s, slot(f::TYPE, Ctx::Bare));
                    self.tk(K::NullableType, K::TQuestion)?;
                    self.close(n);
                }
                T::Star => {
                    if in_expr {
                        break;
                    }
                    n = self.wrap(n, K::PointerType, s, slot(f::TYPE, Ctx::Bare));
                    self.tk(K::PointerType, K::TStar)?;
                    self.close(n);
                }
                T::LBracket if self.empty_rank_ahead(0) => {
                    n = self.wrap(n, K::ArrayType, s, slot(f::TYPE, Ctx::Bare));
                    self.rank()?;
                    self.close(n);
                }
                _ => break,
            }
        }
        self.depth -= 1;
        Ok(n)
    }

    /// `function_pointer_type`: `delegate*`, a calling convention, and `<` parameters (each with
    /// `ref`, `out` or `in`) and the return type `>`.
    fn function_pointer_type(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::FunctionPointerType, s)?;
        self.tk(K::FunctionPointerType, K::TDelegate)?;
        self.tk(K::FunctionPointerType, K::TStar)?;
        if self.is_word(w::MANAGED) || self.is_word(w::UNMANAGED) {
            let c = self.open(K::CallingConvention, f::NONE, C_NONE)?;
            let k = if self.is_word(w::MANAGED) { K::TManaged } else { K::TUnmanaged };
            self.leaf(k, f::NONE, C_NONE)?;
            if self.peek() == T::LBracket {
                return self.refuse();
            }
            self.close(c);
        }
        if self.peek() != T::Lt {
            return self.refuse();
        }
        self.tk(K::FunctionPointerType, K::TLt)?;
        loop {
            let k = usize::from(matches!(self.word_at(0), w::REF | w::OUT | w::IN));
            let Some(e) = self.scan_type_base(k) else { return self.refuse() };
            if self.kind_at(e) != T::Comma {
                break;
            }
            let p = self.open(K::FunctionPointerParameter, f::NONE, C_NONE)?;
            if k == 1 {
                let kw = keyword(self.tok().word).expect("a keyword");
                self.leaf(kw, f::NONE, C_NONE)?;
            }
            self.ty(slot(f::TYPE, Ctx::Bare))?;
            self.close(p);
            self.tk(K::FunctionPointerType, K::TComma)?;
        }
        self.ty(slot(f::RETURNS, Ctx::Type))?;
        if self.peek() != T::Gt {
            return self.refuse();
        }
        self.tk(K::FunctionPointerType, K::TGt)?;
        self.close(n);
        Ok(n)
    }

    /// Whether `[` `,`… `]` (no expressions) starts `k` tokens ahead.
    pub(super) fn empty_rank_ahead(&mut self, k: usize) -> bool {
        if self.kind_at(k) != T::LBracket {
            return false;
        }
        let mut j = k + 1;
        while self.kind_at(j) == T::Comma {
            j += 1;
        }
        self.kind_at(j) == T::RBracket
    }

    /// `array_rank_specifier`: `[`, expressions or none between `,`s, `]`.
    pub(super) fn rank(&mut self) -> R<()> {
        let n = self.open(K::ArrayRankSpecifier, f::RANK, C_NONE)?;
        if self.empty_rank_ahead(0) {
            // `[]` is the kind's one shortest word; `[,]` is not, and keeps its tokens.
            let keep = self.kind_at(1) == T::Comma;
            self.punct_tk(K::ArrayRankSpecifier, T::LBracket, keep)?;
            while self.peek() == T::Comma {
                self.leaf(K::TComma, f::NONE, C_NONE)?;
            }
            self.punct_tk(K::ArrayRankSpecifier, T::RBracket, keep)?;
        } else {
            self.tk(K::ArrayRankSpecifier, K::TLBracket)?;
            loop {
                if matches!(self.peek(), T::Comma | T::RBracket) {
                    return self.refuse();
                }
                self.expr(EXPR)?;
                if self.peek() != T::Comma {
                    break;
                }
                self.tk(K::ArrayRankSpecifier, K::TComma)?;
            }
            self.punct_tk(K::ArrayRankSpecifier, T::RBracket, false)?;
        }
        self.close(n);
        Ok(())
    }

    /// An array creation's type: its element type, then ranks (with sizes).
    pub(super) fn array_type_with_ranks(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let t = self.tok();
        let mut n = match t.kind {
            T::Name if w::is_predefined(t.word) => self.leaf(K::PredefinedType, f::TYPE, C_NONE)?,
            T::Name if !w::is_reserved(t.word) => self.name(slot(f::TYPE, Ctx::Bare))?,
            T::LParen => self.tuple_type(slot(f::TYPE, Ctx::Bare))?,
            _ => return self.refuse(),
        };
        loop {
            match self.peek() {
                T::Question => {
                    n = self.wrap(n, K::NullableType, slot(f::TYPE, Ctx::Bare), slot(f::TYPE, Ctx::Bare));
                    self.tk(K::NullableType, K::TQuestion)?;
                    self.close(n);
                }
                T::Star => {
                    n = self.wrap(n, K::PointerType, slot(f::TYPE, Ctx::Bare), slot(f::TYPE, Ctx::Bare));
                    self.tk(K::PointerType, K::TStar)?;
                    self.close(n);
                }
                _ => break,
            }
        }
        if self.peek() != T::LBracket {
            return self.refuse();
        }
        while self.peek() == T::LBracket {
            n = self.wrap(n, K::ArrayType, slot(f::TYPE, Ctx::Bare), slot(f::TYPE, Ctx::Bare));
            self.rank()?;
            self.close(n);
        }
        self.rekey(n.at, s);
        self.depth -= 1;
        Ok(n)
    }

    /// `tuple_type`: `(` elements `)`, two or more, each a type and a name.
    fn tuple_type(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::TupleType, s)?;
        self.list(K::TupleType, T::RParen, true, false, |p| {
            let e = p.open(K::TupleElement, f::NONE, C_NONE)?;
            p.ty(slot(f::TYPE, Ctx::Type))?;
            if p.is_ident() {
                p.ident(slot(f::NAME, Ctx::Bare))?;
            }
            p.close(e);
            Ok(())
        })?;
        self.close(n);
        Ok(n)
    }

    /// A `_name`: an identifier, generic and qualified (`A.B<C>`) and alias-qualified (`a::B`).
    pub(super) fn name(&mut self, s: Slot) -> R<N> {
        let mut n = if self.kind_at(1) == T::ColonColon {
            let n = self.open_in(K::AliasQualifiedName, s)?;
            self.ident(slot(f::ALIAS, Ctx::Bare))?;
            self.tk(K::AliasQualifiedName, K::TColonColon)?;
            self.simple_name(slot(f::NAME, Ctx::Bare), false)?;
            self.close(n);
            n
        } else {
            self.simple_name(s, false)?
        };
        while self.peek() == T::Dot && self.is_ident_at(1) {
            n = self.wrap(n, K::QualifiedName, s, slot(f::QUALIFIER, Ctx::Bare));
            self.tk(K::QualifiedName, K::TDot)?;
            self.simple_name(slot(f::NAME, Ctx::Bare), false)?;
            self.close(n);
        }
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

    /// `type_argument_list`: `<` types `>`, or `<` `,`… `>` (unbound).
    pub(super) fn type_argument_list(&mut self) -> R<()> {
        let n = self.open(K::TypeArgumentList, f::NONE, C_NONE)?;
        if matches!(self.kind_at(1), T::Comma) || self.is_angle_close_at(1) {
            // `<>` is the kind's one shortest word; `<,>` is not, and keeps its tokens.
            let keep = self.kind_at(1) == T::Comma;
            self.punct_tk(K::TypeArgumentList, T::Lt, keep)?;
            while self.peek() == T::Comma {
                self.leaf(K::TComma, f::NONE, C_NONE)?;
            }
            self.close_angle(K::TypeArgumentList, keep)?;
        } else {
            self.tk(K::TypeArgumentList, K::TLt)?;
            loop {
                self.ty(TYPE)?;
                if self.peek() != T::Comma {
                    break;
                }
                self.tk(K::TypeArgumentList, K::TComma)?;
            }
            self.close_angle(K::TypeArgumentList, false)?;
        }
        self.close(n);
        Ok(())
    }

    fn is_angle_close_at(&mut self, k: usize) -> bool {
        matches!(self.kind_at(k), T::Gt | T::GtGt | T::GtGtGt | T::Ge | T::GtGtEq | T::GtGtGtEq)
    }

    // ---- Look aheads.

    /// Where a type starting `k` tokens ahead ends, suffixes included. `None` where none starts.
    pub(super) fn scan_type(&mut self, k: usize) -> Option<usize> {
        let mut owed = 0;
        let e = self.scan_type_owed(k, &mut owed, true)?;
        (owed == 0).then_some(e)
    }

    /// Where a type's base (no `[`… suffix) starting `k` ahead ends.
    pub(super) fn scan_type_base(&mut self, k: usize) -> Option<usize> {
        let mut owed = 0;
        let e = self.scan_type_owed(k, &mut owed, false)?;
        (owed == 0).then_some(e)
    }

    /// A type's end, where a `>>` closing type arguments inside it leaves `owed` closes to the
    /// lists around it (a type then ends at once).
    fn scan_type_owed(&mut self, k: usize, owed: &mut u32, ranks: bool) -> Option<usize> {
        let t = self.peek_at(k);
        let mut k = match t.kind {
            T::Name if t.word == w::REF => {
                let k = if self.is_word_at(k + 1, w::READONLY) { k + 2 } else { k + 1 };
                return self.scan_type_owed(k, owed, ranks);
            }
            T::Name if t.word == w::SCOPED && self.kind_at(k + 1) == T::Name => return self.scan_type_owed(k + 1, owed, ranks),
            T::Name if t.word == w::DELEGATE && self.kind_at(k + 1) == T::Star => {
                // `delegate*`, a calling convention, `<` types between `,`s `>`.
                let mut j = k + 2;
                if matches!(self.word_at(j), w::MANAGED | w::UNMANAGED) {
                    j += 1;
                }
                if self.kind_at(j) != T::Lt {
                    return None;
                }
                j += 1;
                loop {
                    if matches!(self.word_at(j), w::REF | w::OUT | w::IN) {
                        j += 1;
                    }
                    let mut inner = 0;
                    j = self.scan_type_owed(j, &mut inner, true)?;
                    if inner > 0 {
                        return None;
                    }
                    match self.kind_at(j) {
                        T::Comma => j += 1,
                        T::Gt => break j + 1,
                        _ => return None,
                    }
                }
            }
            T::Name if w::is_predefined(t.word) => k + 1,
            T::Name if !w::is_reserved(t.word) => {
                let e = self.scan_name(k, owed)?;
                if *owed > 0 {
                    return Some(e);
                }
                e
            }
            T::LParen => {
                let mut j = k + 1;
                let mut elements = 0;
                loop {
                    j = self.scan_type_owed(j, owed, true)?;
                    if *owed > 0 {
                        return None;
                    }
                    if self.is_ident_at(j) {
                        j += 1;
                    }
                    elements += 1;
                    match self.kind_at(j) {
                        T::Comma => j += 1,
                        T::RParen => break,
                        _ => return None,
                    }
                }
                if elements < 2 {
                    return None;
                }
                j + 1
            }
            _ => return None,
        };
        loop {
            match self.kind_at(k) {
                T::Question => k += 1,
                T::Star => k += 1,
                T::LBracket if ranks && self.empty_rank_ahead(k) => {
                    k += 1;
                    while self.kind_at(k) == T::Comma {
                        k += 1;
                    }
                    k += 1;
                }
                _ => return Some(k),
            }
        }
    }

    /// A `_name` from `k`: its end.
    fn scan_name(&mut self, k: usize, owed: &mut u32) -> Option<usize> {
        let mut k = k;
        if self.kind_at(k + 1) == T::ColonColon {
            if !self.is_ident_at(k + 2) {
                return None;
            }
            k += 2;
        }
        loop {
            if !self.is_ident_at(k) {
                return None;
            }
            k += 1;
            if self.kind_at(k) == T::Lt {
                k = self.scan_targs(k, owed)?;
                if *owed > 0 {
                    return Some(k);
                }
            }
            if self.kind_at(k) == T::Dot && self.is_ident_at(k + 1) {
                k += 1;
                continue;
            }
            return Some(k);
        }
    }

    /// Type arguments from their `<` at `k`: their end (past the `>`), `None` where none close.
    pub(super) fn scan_type_args(&mut self, k: usize) -> Option<usize> {
        let mut owed = 0;
        let e = self.scan_targs(k, &mut owed)?;
        (owed == 0).then_some(e)
    }

    /// Type arguments from their `<` at `k`: their end and the closes a `>>` owes the lists around.
    pub(super) fn scan_type_args_owing(&mut self, k: usize) -> Option<(usize, u32)> {
        let mut owed = 0;
        let e = self.scan_targs(k, &mut owed)?;
        Some((e, owed))
    }

    fn scan_targs(&mut self, k: usize, owed: &mut u32) -> Option<usize> {
        let mut k = k + 1;
        if self.kind_at(k) == T::Comma || self.kind_at(k) == T::Gt {
            while self.kind_at(k) == T::Comma {
                k += 1;
            }
            return (self.kind_at(k) == T::Gt).then_some(k + 1);
        }
        loop {
            k = self.scan_type_owed(k, owed, true)?;
            if *owed > 0 {
                *owed -= 1;
                return Some(k);
            }
            match self.kind_at(k) {
                T::Comma => k += 1,
                T::Gt => return Some(k + 1),
                T::GtGt => {
                    *owed += 1;
                    return Some(k + 1);
                }
                T::GtGtGt => {
                    *owed += 2;
                    return Some(k + 1);
                }
                _ => return None,
            }
        }
    }
}

fn n_is(n: N, kinds: &[K]) -> bool {
    kinds.contains(&n.kind)
}
