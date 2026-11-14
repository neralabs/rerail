//! Patterns: or-patterns (left-associative), literals and ranges, paths with their tuple, struct
//! and macro forms, bindings, references, tuples and slices.

use super::*;

impl<'a> P<'a> {
    /// A pattern in slot `s` under `parent`. With `or`, `a | b` is an `or_pattern` (not in a
    /// closure's parameters, where `|` ends them). A `_` alone is a token: stored where `parent`
    /// does not imply it.
    pub(super) fn pattern(&mut self, s: Slot, parent: K, or: bool) -> R<()> {
        self.deeper()?;
        let first = self.tok();
        let lead = or && first.kind == T::Pipe;
        let mut n = if lead {
            // `| a`: an `or_pattern` with its `|` stored (the rule's other choice starts with a
            // pattern).
            let n = self.open_in(K::OrPattern, s);
            self.leaf(K::TPipe, f::NONE, C_NONE)?;
            self.pattern_atom(PAT, K::OrPattern)?;
            self.close(n);
            Some(n)
        } else if self.underscore(first) && !(or && self.peek_at(1).kind == T::Pipe) {
            if self.stores(parent, K::TUnderscore, s.field) {
                self.leaf(K::TUnderscore, s.field, class(K::TUnderscore, s.ctx))?;
            } else {
                self.bump()?;
            }
            None
        } else if self.underscore(first) {
            // `_ | b`: the `_` and the `|` are stored (the gap is not the implied word).
            let n = self.open_in(K::OrPattern, s);
            self.leaf(K::TUnderscore, f::NONE, C_PAT)?;
            self.leaf(K::TPipe, f::NONE, C_NONE)?;
            self.or_right()?;
            self.close(n);
            Some(n)
        } else {
            Some(self.pattern_atom(s, parent)?)
        };
        while or && self.peek() == T::Pipe {
            let Some(left) = n else { return self.refuse() };
            let o = self.wrap(left, K::OrPattern, s, PAT);
            self.tk(K::OrPattern, K::TPipe, f::NONE)?;
            self.or_right()?;
            self.close(o);
            n = Some(o);
        }
        self.depth -= 1;
        Ok(())
    }

    /// An or-pattern's right side: a pattern, or `_` (stored: the `_` is no implied token there).
    fn or_right(&mut self) -> R<()> {
        if self.is_underscore() {
            self.leaf(K::TUnderscore, f::NONE, C_PAT)?;
            return Ok(());
        }
        self.pattern_atom(PAT, K::OrPattern)?;
        Ok(())
    }

    /// A pattern without `|`.
    fn pattern_atom(&mut self, s: Slot, parent: K) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Int | T::Float | T::Char | T::Str | T::RawStr => {
                let n = self.literal(s)?;
                self.range_after(n, s)
            }
            T::Minus => {
                let n = self.negative_literal(s)?;
                self.range_after(n, s)
            }
            T::DotDot if !self.range_side_ahead(1) => {
                let n = self.open_in(K::RemainingFieldPattern, s);
                self.tk(K::RemainingFieldPattern, K::TDotDot, f::NONE)?;
                self.close(n);
                Ok(n)
            }
            T::DotDot | T::DotDotEq => {
                let n = self.open_in(K::RangePattern, s);
                self.tk_punct(K::RangePattern, false)?;
                self.range_side(slot(f::RIGHT, Ctx::LitPat))?;
                self.close(n);
                Ok(n)
            }
            T::Amp | T::AmpAmp => {
                if t.kind == T::AmpAmp {
                    self.split(T::Amp, 1);
                }
                let n = self.open_in(K::ReferencePattern, s);
                self.tk(K::ReferencePattern, K::TAmp, f::NONE)?;
                if self.is_word(w::MUT) {
                    self.leaf(K::MutableSpecifier, f::NONE, C_NONE)?;
                }
                self.pattern(PAT, K::ReferencePattern, false)?;
                self.close(n);
                Ok(n)
            }
            T::LParen => {
                let n = self.open_in(K::TuplePattern, s);
                self.list(K::TuplePattern, T::RParen, true, true, |p| p.pattern(PAT, K::TuplePattern, true))?;
                self.close(n);
                Ok(n)
            }
            T::LBracket => {
                let n = self.open_in(K::SlicePattern, s);
                self.list(K::SlicePattern, T::RBracket, true, true, |p| p.pattern(PAT, K::SlicePattern, true))?;
                self.close(n);
                Ok(n)
            }
            T::Name => match t.word {
                w::REF => {
                    let n = self.open_in(K::RefPattern, s);
                    self.tk(K::RefPattern, K::TRef, f::NONE)?;
                    self.pattern(PAT, K::RefPattern, false)?;
                    self.close(n);
                    Ok(n)
                }
                w::MUT => {
                    let n = self.open_in(K::MutPattern, s);
                    self.leaf(K::MutableSpecifier, f::NONE, C_NONE)?;
                    self.pattern(PAT, K::MutPattern, false)?;
                    self.close(n);
                    Ok(n)
                }
                w::CONST if self.peek_at(1).kind == T::LBrace => self.blocklike(s),
                w::TRUE | w::FALSE => {
                    let n = self.literal(s)?;
                    self.range_after(n, s)
                }
                _ => self.path_pattern(s, parent),
            },
            T::Lt | T::ColonColon => self.path_pattern(s, parent),
            _ => self.refuse(),
        }
    }

    /// `-1`, `-1.0`.
    pub(super) fn negative_literal(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::NegativeLiteral, s);
        self.tk(K::NegativeLiteral, K::TMinus, f::NONE)?;
        match self.peek() {
            T::Int => drop(self.leaf(K::IntegerLiteral, f::NONE, C_NONE)?),
            T::Float => drop(self.leaf(K::FloatLiteral, f::NONE, C_NONE)?),
            _ => return self.refuse(),
        }
        self.close(n);
        Ok(n)
    }

    /// Whether a range's side starts at token `k`: a literal, `-`, or a path.
    fn range_side_ahead(&mut self, k: usize) -> bool {
        let t = self.peek_at(k);
        match t.kind {
            T::Int | T::Float | T::Char | T::Str | T::RawStr | T::Minus | T::ColonColon | T::Lt => true,
            T::Name => !matches!(t.word, w::IF) && !self.underscore(t),
            _ => false,
        }
    }

    /// A range's side: a literal pattern or a path.
    fn range_side(&mut self, s: Slot) -> R<()> {
        match self.peek() {
            T::Minus => drop(self.negative_literal(s)?),
            T::Int | T::Float | T::Char | T::Str | T::RawStr => drop(self.literal(s)?),
            T::Name if matches!(self.tok().word, w::TRUE | w::FALSE) => drop(self.literal(s)?),
            _ => drop(self.pattern_path(s)?),
        }
        Ok(())
    }

    /// After a literal or path pattern `n`: a range (`a..=b`, `a..`, `a...b`) where one follows.
    fn range_after(&mut self, n: N, s: Slot) -> R<N> {
        if !matches!(self.peek(), T::DotDot | T::DotDotEq | T::DotDotDot) {
            return Ok(n);
        }
        let r = self.wrap(n, K::RangePattern, s, slot(f::LEFT, Ctx::LitPat));
        let open = self.peek() == T::DotDot;
        self.tk_punct(K::RangePattern, false)?;
        if !open || self.range_side_ahead(0) {
            self.range_side(slot(f::RIGHT, Ctx::LitPat))?;
        }
        self.close(r);
        Ok(r)
    }

    /// A path in a pattern: `a`, `a::b`, `<T>::A`, `A::<T>` (generic), with no supertype of its own
    /// yet.
    fn pattern_path(&mut self, s: Slot) -> R<N> {
        let mut n = match self.peek() {
            T::Lt => {
                let b = self.bracketed_type(slot(f::PATH, Ctx::Bare))?;
                if self.peek() != T::ColonColon {
                    return self.refuse();
                }
                self.scoped_segment(b, s)?
            }
            T::ColonColon => {
                let n = self.open_in(K::ScopedIdentifier, s);
                self.tk(K::ScopedIdentifier, K::TColonColon, f::NONE)?;
                self.identifier(slot(f::NAME, Ctx::Bare))?;
                self.close(n);
                n
            }
            _ => self.path_start(s)?,
        };
        loop {
            if self.peek() != T::ColonColon {
                break;
            }
            match self.peek_at(1).kind {
                T::Name => n = self.scoped_segment(n, s)?,
                T::Lt | T::LtLt => {
                    let Some(end) = self.angle_end(1) else { return self.refuse() };
                    let inner = if n.kind == K::Identifier { K::TypeIdentifier } else { n.kind };
                    match self.peek_at(end).kind {
                        T::ColonColon | T::LParen => {
                            // `A::<T>::B`, `A::<T>(…)`: a `generic_type` (its `::` stored).
                            self.rekind(n.at, inner, slot(f::TYPE, Ctx::Bare));
                            let g = self.wrap(N { at: n.at, kind: inner }, K::GenericType, s, slot(f::TYPE, Ctx::Bare));
                            self.leaf(K::TColonColon, f::NONE, C_NONE)?;
                            self.type_arguments()?;
                            self.close(g);
                            n = g;
                        }
                        _ => {
                            let g = self.wrap(n, K::GenericPattern, s, BARE);
                            self.tk(K::GenericPattern, K::TColonColon, f::NONE)?;
                            self.type_arguments()?;
                            self.close(g);
                            return Ok(g);
                        }
                    }
                }
                _ => break,
            }
        }
        Ok(n)
    }

    /// A pattern that starts with a path: a tuple struct, a struct, a macro, a binding (`x @ p`), a
    /// range, or the path alone.
    fn path_pattern(&mut self, s: Slot, _parent: K) -> R<N> {
        let first = self.tok();
        let mut n = self.pattern_path(s)?;
        match self.peek() {
            T::LParen if n.kind != K::GenericPattern => {
                let p = self.wrap(n, K::TupleStructPattern, s, slot(f::TYPE, Ctx::Bare));
                self.list(K::TupleStructPattern, T::RParen, true, true, |p| p.pattern(PAT, K::TupleStructPattern, true))?;
                self.close(p);
                Ok(p)
            }
            T::LBrace if matches!(n.kind, K::Identifier | K::ScopedIdentifier) => {
                if n.kind == K::Identifier {
                    self.rekind(n.at, K::TypeIdentifier, slot(f::TYPE, Ctx::Bare));
                } else {
                    self.rekind(n.at, K::ScopedTypeIdentifier, slot(f::TYPE, Ctx::Bare));
                    // The name, the path's last node.
                    let at = self.w.len() - 1;
                    self.rekind(at, K::TypeIdentifier, slot(f::NAME, Ctx::Bare));
                }
                n.kind = K::TypeIdentifier;
                let p = self.wrap(n, K::StructPattern, s, slot(f::TYPE, Ctx::Bare));
                self.struct_pattern_fields()?;
                self.close(p);
                Ok(p)
            }
            T::Bang if matches!(n.kind, K::Identifier | K::ScopedIdentifier) => {
                let m = self.wrap(n, K::MacroInvocation, s, slot(f::MACRO, Ctx::Bare));
                self.tk(K::MacroInvocation, K::TBang, f::NONE)?;
                self.token_tree(tt::Tt::Delim, f::NONE)?;
                self.close(m);
                Ok(m)
            }
            T::At if n.kind == K::Identifier && first.kind == T::Name => {
                let c = self.wrap(n, K::CapturedPattern, s, BARE);
                self.tk(K::CapturedPattern, K::TAt, f::NONE)?;
                self.pattern(PAT, K::CapturedPattern, false)?;
                self.close(c);
                Ok(c)
            }
            T::DotDot | T::DotDotEq | T::DotDotDot => self.range_after(n, s),
            _ => Ok(n),
        }
    }

    /// `{ a, ref mut b, c: p, .. }`.
    fn struct_pattern_fields(&mut self) -> R<()> {
        let saved = self.no_struct;
        self.no_struct = false;
        self.list(K::StructPattern, T::RBrace, false, true, |p| {
            if p.peek() == T::DotDot {
                let n = p.open(K::RemainingFieldPattern, f::NONE, C_NONE);
                p.tk(K::RemainingFieldPattern, K::TDotDot, f::NONE)?;
                p.close(n);
                return Ok(());
            }
            let n = p.open(K::FieldPattern, f::NONE, C_NONE);
            if p.is_word(w::REF) && p.peek_at(1).kind == T::Name {
                p.tk(K::FieldPattern, K::TRef, f::NONE)?;
            }
            if p.is_word(w::MUT) && p.peek_at(1).kind == T::Name {
                p.leaf(K::MutableSpecifier, f::NONE, C_NONE)?;
            }
            if p.peek() != T::Name {
                return p.refuse();
            }
            if p.peek_at(1).kind == T::Colon {
                p.leaf(K::FieldIdentifier, f::NAME, C_NONE)?;
                p.tk(K::FieldPattern, K::TColon, f::NONE)?;
                p.pattern(slot(f::PATTERN, Ctx::Pattern), K::FieldPattern, true)?;
            } else {
                p.leaf(K::ShorthandFieldIdentifier, f::NAME, C_NONE)?;
            }
            p.close(n);
            Ok(())
        })?;
        self.no_struct = saved;
        Ok(())
    }
}
