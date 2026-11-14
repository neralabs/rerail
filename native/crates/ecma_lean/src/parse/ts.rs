//! TypeScript's part of the grammar (tree-sitter-typescript's `define-grammar.js`, patched): types,
//! annotations, parameters, signatures, and the declarations JavaScript lacks. The same parser; a
//! TypeScript file is read where `P::ts`.
//!
//! A type's supertypes are the tape's: a primary type (`primary_type`) where a `type` stands is in
//! both, `{type,primary_type}`; where a `primary_type` alone stands (an array's element, a lookup's
//! object, `keyof`'s operand), in that one; a function, constructor, `readonly` or `infer` type, in
//! `type` alone.

use super::*;

/// TypeScript's reserved words: never a type's name.
fn reserved(code: u8) -> bool {
    matches!(
        code,
        w::BREAK
            | w::CASE
            | w::CATCH
            | w::CLASS
            | w::CONST
            | w::CONTINUE
            | w::DEBUGGER
            | w::DEFAULT
            | w::DELETE
            | w::DO
            | w::ELSE
            | w::ENUM
            | w::EXPORT
            | w::EXTENDS
            | w::FINALLY
            | w::FOR
            | w::FUNCTION
            | w::IF
            | w::IMPORT
            | w::IN
            | w::INSTANCEOF
            | w::NEW
            | w::RETURN
            | w::SUPER
            | w::SWITCH
            | w::THROW
            | w::TRY
            | w::VAR
            | w::WHILE
            | w::WITH
    )
}

/// The predefined type a word names, if it is one.
fn predefined(code: u8) -> Option<K> {
    Some(match code {
        w::ANY => K::TAny,
        w::NUMBER => K::TNumber,
        w::BOOLEAN => K::TBoolean,
        w::STRING => K::TString,
        w::SYMBOL => K::TSymbol,
        w::VOID => K::TVoid,
        w::UNKNOWN => K::TUnknown,
        w::NEVER => K::TNever,
        w::OBJECT => K::TObject,
        _ => return None,
    })
}

/// The accessibility modifier a word is, if it is one.
fn accessibility(code: u8) -> Option<K> {
    Some(match code {
        w::PUBLIC => K::TPublic,
        w::PRIVATE => K::TPrivate,
        w::PROTECTED => K::TProtected,
        _ => return None,
    })
}

pub(super) fn is_primary_type(k: K) -> bool {
    use K::*;
    matches!(
        k,
        ParenthesizedType
            | PredefinedType
            | TypeIdentifier
            | NestedTypeIdentifier
            | GenericType
            | ObjectType
            | ArrayType
            | TupleType
            | FlowMaybeType
            | TypeQuery
            | IndexTypeQuery
            | ThisType
            | ExistentialType
            | LiteralType
            | LookupType
            | ConditionalType
            | MemberExpression
            | TemplateLiteralType
            | IntersectionType
            | UnionType
    )
}

pub(super) fn is_type_only(k: K) -> bool {
    matches!(k, K::FunctionType | K::ReadonlyType | K::ConstructorType | K::InferType)
}

const TYPE: Slot = slot(f::NONE, Ctx::Type);

impl<'a> P<'a> {
    // ---- Types.

    /// A `type` in slot `s`.
    pub(super) fn ty(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let r = self.ty_inner(s, true);
        self.depth -= 1;
        r
    }

    fn ty_inner(&mut self, s: Slot, conditional: bool) -> R<N> {
        let t = self.tok();
        if t.kind == T::Name {
            let next = self.peek_at(1);
            match t.word {
                w::INFER if next.kind == T::Name => {
                    let n = self.open_in(K::InferType, s);
                    self.tk(K::InferType, K::TInfer, f::NONE)?;
                    self.leaf(K::TypeIdentifier, f::NONE, C_NONE)?;
                    if self.is_word(w::EXTENDS) && !self.ty_stops() {
                        self.leaf(K::TExtends, f::NONE, C_NONE)?;
                        self.ty_inner(TYPE, false)?;
                    }
                    self.close(n);
                    return Ok(n);
                }
                w::NEW => return self.constructor_type(s),
                w::ABSTRACT if self.is_word_at(1, w::NEW) => return self.constructor_type(s),
                _ => {}
            }
        }
        if matches!(t.kind, T::LParen | T::Lt) {
            let m = self.mark();
            match self.function_type(s) {
                Ok(n) => return Ok(n),
                Err(_) if t.kind == T::Lt => return self.refuse(),
                Err(_) => self.restore(m),
            }
        }
        let start = self.here();
        let left = self.union_ty(s)?;
        if !conditional || !self.is_word(w::EXTENDS) || self.ty_stops() {
            return Ok(left);
        }
        // `left extends right ? consequence : alternative`.
        let n = self.wrap(N { at: start, kind: left.kind }, K::ConditionalType, s, slot(f::LEFT, Ctx::Type));
        self.tk(K::ConditionalType, K::TExtends, f::NONE)?;
        self.ty_inner(slot(f::RIGHT, Ctx::Type), false)?;
        if self.peek() != T::Question {
            return self.refuse();
        }
        self.bump()?;
        let saved = self.asi;
        self.asi = false;
        self.ty(slot(f::CONSEQUENCE, Ctx::Type))?;
        self.asi = saved;
        self.expect(T::Colon)?;
        self.ty(slot(f::ALTERNATIVE, Ctx::Type))?;
        self.close(n);
        Ok(n)
    }

    /// Whether a type stops before the next token: at a statement's or a member's level, a line
    /// break the scanner ends it at (after a type, no binary operator is valid).
    fn ty_stops(&mut self) -> bool {
        self.asi && self.tok().nl && self.asi(0)
    }

    /// `(params) => type`, `<T>(params) => type`.
    fn function_type(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::FunctionType, s);
        if self.peek() == T::Lt {
            self.type_parameters()?;
        }
        self.formal_parameters()?;
        if self.peek() != T::Arrow {
            return self.refuse();
        }
        self.tk(K::FunctionType, K::TArrow, f::NONE)?;
        self.return_type_after_arrow()?;
        self.close(n);
        Ok(n)
    }

    /// A function type's return: a type, a type predicate or `asserts`.
    fn return_type_after_arrow(&mut self) -> R<()> {
        let t = self.tok();
        if t.kind == T::Name && (t.word == w::ASSERTS || self.is_word_at(1, w::IS)) && matches!(self.peek_at(1).kind, T::Name) {
            return self.refuse();
        }
        self.ty(slot(f::RETURN_TYPE, Ctx::Type))?;
        Ok(())
    }

    fn constructor_type(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ConstructorType, s);
        if self.is_word(w::ABSTRACT) {
            self.tk(K::ConstructorType, K::TAbstract, f::NONE)?;
        }
        self.tk(K::ConstructorType, K::TNew, f::NONE)?;
        if self.peek() == T::Lt {
            self.type_parameters()?;
        }
        self.formal_parameters()?;
        if self.peek() != T::Arrow {
            return self.refuse();
        }
        self.tk(K::ConstructorType, K::TArrow, f::NONE)?;
        // `new () => A & B` is `(new () => A) & B` (both `prec.left(0)`, the reduction taken):
        // the type one operand; an operator after it refused.
        self.postfix_ty(slot(f::TYPE, Ctx::Type))?;
        if matches!(self.peek(), T::Amp | T::Pipe) {
            return self.refuse();
        }
        self.close(n);
        Ok(n)
    }

    /// Unions over intersections (left to right; a leading `|` is a union of one).
    fn union_ty(&mut self, s: Slot) -> R<N> {
        self.list_ty(s, T::Pipe, K::UnionType, K::TPipe)
    }

    fn intersection_ty(&mut self, s: Slot) -> R<N> {
        self.list_ty(s, T::Amp, K::IntersectionType, K::TAmp)
    }

    /// `a op b op c` as `(a op b) op c`; `op a` alone, the operator stored.
    fn list_ty(&mut self, s: Slot, op: T, kind: K, tok: K) -> R<N> {
        let start = self.here();
        let operand = |p: &mut Self, s: Slot| if op == T::Pipe { p.intersection_ty(s) } else { p.postfix_ty(s) };
        let mut left = if self.peek() == op {
            let n = self.open_in(kind, s);
            self.leaf(tok, f::NONE, C_NONE)?;
            operand(self, TYPE)?;
            self.close(n);
            n
        } else {
            operand(self, s)?
        };
        while self.peek() == op && !self.ty_stops() {
            let n = self.wrap(N { at: start, kind: left.kind }, kind, s, TYPE);
            if op == T::Pipe {
                self.bump()?;
            } else {
                self.tk(kind, tok, f::NONE)?;
            }
            operand(self, TYPE)?;
            self.close(n);
            left = n;
        }
        Ok(left)
    }

    /// A primary type and its postfixes (`T[]`, `T[K]`); `keyof T`.
    fn postfix_ty(&mut self, s: Slot) -> R<N> {
        if self.is_word(w::KEYOF) && !matches!(self.peek_at(1).kind, T::Comma | T::RParen | T::RBracket | T::Gt | T::Semi | T::Eq | T::Pipe | T::Amp) {
            self.deeper()?;
            let n = self.open_in(K::IndexTypeQuery, s);
            self.tk(K::IndexTypeQuery, K::TKeyof, f::NONE)?;
            self.postfix_ty(slot(f::NONE, Ctx::PrimaryType))?;
            self.close(n);
            self.depth -= 1;
            return Ok(n);
        }
        if self.is_word(w::READONLY) && matches!(self.peek_at(1).kind, T::Name | T::LBracket | T::LParen | T::LBrace) {
            // `readonly T`: over a whole type (`readonly A[] | B` is `readonly (A[] | B)`).
            self.deeper()?;
            let n = self.open_in(K::ReadonlyType, s);
            self.tk(K::ReadonlyType, K::TReadonly, f::NONE)?;
            self.ty(TYPE)?;
            self.close(n);
            self.depth -= 1;
            return Ok(n);
        }
        let start = self.here();
        let mut e = self.atom_ty(s)?;
        while self.peek() == T::LBracket && !self.ty_stops() {
            let inner = slot(f::NONE, Ctx::PrimaryType);
            if self.peek_at(1).kind == T::RBracket {
                let n = self.wrap(N { at: start, kind: e.kind }, K::ArrayType, s, inner);
                self.bump()?;
                self.bump()?;
                self.close(n);
                e = n;
            } else {
                let n = self.wrap(N { at: start, kind: e.kind }, K::LookupType, s, inner);
                self.bump()?;
                let saved = self.asi;
                self.asi = false;
                self.ty(TYPE)?;
                self.asi = saved;
                self.expect(T::RBracket)?;
                self.close(n);
                e = n;
            }
        }
        Ok(e)
    }

    fn atom_ty(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::LParen => {
                let n = self.open_in(K::ParenthesizedType, s);
                self.bump()?;
                let saved = self.asi;
                self.asi = false;
                self.ty(TYPE)?;
                self.asi = saved;
                self.expect(T::RParen)?;
                self.close(n);
                Ok(n)
            }
            T::LBrace => {
                let n = self.open_in(K::ObjectType, s);
                self.object_type_body(K::ObjectType)?;
                self.close(n);
                Ok(n)
            }
            T::LBracket => self.tuple_type(s),
            T::Str => {
                let n = self.open_in(K::LiteralType, s);
                self.string(BARE)?;
                self.close(n);
                Ok(n)
            }
            T::Number => {
                let n = self.open_in(K::LiteralType, s);
                self.leaf(K::Number, f::NONE, C_NONE)?;
                self.close(n);
                Ok(n)
            }
            T::Minus | T::Plus if self.peek_at(1).kind == T::Number => {
                let n = self.open_in(K::LiteralType, s);
                let u = self.open(K::UnaryExpression, f::NONE, C_NONE);
                let op = if t.kind == T::Minus { K::TMinus } else { K::TPlus };
                self.leaf(op, f::OPERATOR, C_NONE)?;
                self.leaf(K::Number, f::ARGUMENT, C_NONE)?;
                self.close(u);
                self.close(n);
                Ok(n)
            }
            T::Backtick => self.template_literal_type(s),
            T::Name => {
                let next = self.peek_at(1).kind;
                if next != T::Dot {
                    if let Some(k) = predefined(t.word) {
                        let n = self.open_in(K::PredefinedType, s);
                        self.tk(K::PredefinedType, k, f::NONE)?;
                        self.close(n);
                        return Ok(n);
                    }
                    let literal = match t.word {
                        w::TRUE => Some(K::True),
                        w::FALSE => Some(K::False),
                        w::NULL => Some(K::Null),
                        w::UNDEFINED => Some(K::Undefined),
                        _ => None,
                    };
                    if let Some(k) = literal {
                        let n = self.open_in(K::LiteralType, s);
                        self.leaf(k, f::NONE, C_NONE)?;
                        self.close(n);
                        return Ok(n);
                    }
                    if t.word == w::THIS {
                        return self.leaf_in(K::ThisType, s);
                    }
                }
                match t.word {
                    w::TYPEOF => self.type_query(s),
                    w::IMPORT if next == T::LParen => {
                        // `import("a").B.C`: member expressions over the call.
                        let start = self.here();
                        self.import_call(f::OBJECT)?;
                        if self.peek() != T::Dot {
                            return self.refuse();
                        }
                        let e = self.import_members(start, s)?;
                        if matches!(self.peek(), T::Lt | T::Dot) {
                            return self.refuse();
                        }
                        Ok(e)
                    }
                    w::UNIQUE | w::IMPORT | w::INFER | w::READONLY | w::ASSERTS if next == T::Name || next == T::LParen => self.refuse(),
                    _ if reserved(t.word) => self.refuse(),
                    _ => self.type_reference(s),
                }
            }
            _ => self.refuse(),
        }
    }

    /// `A`, `A.B.C`, `A<T>`, `A.B<T>`.
    pub(super) fn type_reference(&mut self, s: Slot) -> R<N> {
        // The names: `n0 . n1 . … nk`.
        let k = self.dotted_names();
        self.type_reference_k(s, k)
    }

    /// The number of `. name` after the next name.
    pub(super) fn dotted_names(&mut self) -> usize {
        let mut k = 0;
        while self.peek_at(2 * k + 1).kind == T::Dot && self.peek_at(2 * k + 2).kind == T::Name {
            k += 1;
        }
        k
    }

    /// A type reference over the next name and `k` dotted names after it.
    pub(super) fn type_reference_k(&mut self, s: Slot, k: usize) -> R<N> {
        let start = self.here();
        let kind = if k == 0 {
            self.leaf_in(K::TypeIdentifier, s)?;
            K::TypeIdentifier
        } else {
            let n = self.open_in(K::NestedTypeIdentifier, s);
            self.nested_identifier(k - 1, f::MODULE, K::NestedIdentifier)?;
            self.tk(K::NestedTypeIdentifier, K::TDot, f::NONE)?;
            self.leaf(K::TypeIdentifier, f::NAME, C_NONE)?;
            self.close(n);
            K::NestedTypeIdentifier
        };
        if self.peek() == T::Lt && !self.ty_stops() {
            let g = self.wrap(N { at: start, kind }, K::GenericType, s, slot(f::NAME, Ctx::Bare));
            self.type_arguments(f::TYPE_ARGUMENTS)?;
            self.close(g);
            return Ok(g);
        }
        Ok(N { at: start, kind })
    }

    /// `n0 . … . nm` (`m` dots) as an identifier (`m` 0) or a `kind` (a `nested_identifier`, whose
    /// inner ones are `member_expression`s), in `field`.
    pub(super) fn nested_identifier(&mut self, m: usize, field: u8, kind: K) -> R<()> {
        if m == 0 {
            self.leaf(K::Identifier, field, C_NONE)?;
            return Ok(());
        }
        let n = self.open(kind, field, C_NONE);
        self.nested_identifier(m - 1, f::OBJECT, K::MemberExpression)?;
        self.tk(kind, K::TDot, f::NONE)?;
        self.leaf(K::PropertyIdentifier, f::PROPERTY, C_NONE)?;
        self.close(n);
        Ok(())
    }

    /// `typeof x`, `typeof a.b`, `typeof f<T>`.
    fn type_query(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::TypeQuery, s);
        self.tk(K::TypeQuery, K::TTypeof, f::NONE)?;
        if self.is_word(w::IMPORT) && self.peek_at(1).kind == T::LParen {
            let start = self.here();
            self.import_call(f::NONE)?;
            self.import_members(start, BARE)?;
            if matches!(self.peek(), T::Lt | T::Dot | T::LBracket) {
                return self.refuse();
            }
            self.close(n);
            return Ok(n);
        }
        let mut k = 0;
        while matches!(self.peek_at(2 * k + 1).kind, T::Dot) && matches!(self.peek_at(2 * k + 2).kind, T::Name | T::Private) {
            k += 1;
        }
        if self.peek_at(2 * k + 1).kind == T::Lt {
            return self.refuse();
        }
        if !matches!(self.peek(), T::Name) || matches!(self.peek_at(2 * k + 1).kind, T::LParen | T::QDot) {
            return self.refuse();
        }
        if self.peek_at(2 * k + 1).kind == T::LBracket && self.peek_at(2 * k + 2).kind != T::RBracket {
            return self.refuse();
        }
        self.type_query_operand(k, f::NONE)?;
        self.close(n);
        Ok(n)
    }

    /// `import("a")` in a type, a `call_expression` in `field`.
    fn import_call(&mut self, field: u8) -> R<()> {
        let c = self.open(K::CallExpression, field, C_NONE);
        let i = self.open(K::Import, f::FUNCTION, C_NONE);
        self.tk(K::Import, K::TImport, f::NONE)?;
        self.close(i);
        self.arguments()?;
        self.close(c);
        Ok(())
    }

    /// `. name` after an import type's call (written at `start`), the outermost in slot `s`.
    fn import_members(&mut self, start: usize, s: Slot) -> R<N> {
        let mut e = N { at: start, kind: K::CallExpression };
        while self.peek() == T::Dot && self.peek_at(1).kind == T::Name {
            let m = self.wrap(e, K::MemberExpression, s, slot(f::OBJECT, Ctx::Bare));
            self.tk(K::MemberExpression, K::TDot, f::NONE)?;
            self.leaf(K::PropertyIdentifier, f::PROPERTY, C_NONE)?;
            self.close(m);
            e = m;
        }
        Ok(e)
    }

    /// `n0 . … . nm` in a type query: an identifier (or `this`), or member expressions.
    fn type_query_operand(&mut self, m: usize, field: u8) -> R<()> {
        if m == 0 {
            let kind = if self.is_word(w::THIS) { K::This } else { K::Identifier };
            self.leaf(kind, field, C_NONE)?;
            return Ok(());
        }
        let n = self.open(K::MemberExpression, field, C_NONE);
        self.type_query_operand(m - 1, f::OBJECT)?;
        self.tk(K::MemberExpression, K::TDot, f::NONE)?;
        let kind = if self.peek() == T::Private { K::PrivatePropertyIdentifier } else { K::PropertyIdentifier };
        self.leaf(kind, f::PROPERTY, C_NONE)?;
        self.close(n);
        Ok(())
    }

    fn tuple_type(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::TupleType, s);
        self.bump()?;
        let saved = self.asi;
        self.asi = false;
        loop {
            if self.peek() == T::RBracket {
                self.bump()?;
                break;
            }
            self.tuple_member()?;
            match self.peek() {
                T::Comma => {
                    let c = self.bump()?;
                    if self.peek() == T::RBracket {
                        self.leaf_tok(c, K::TComma);
                        self.leaf(K::TRBracket, f::NONE, C_NONE)?;
                        break;
                    }
                }
                T::RBracket => {
                    self.bump()?;
                    break;
                }
                _ => return self.refuse(),
            }
        }
        self.asi = saved;
        self.close(n);
        Ok(n)
    }

    /// A tuple's member: `name: T`, `name?: T`, `...name: T`, `...T`, `T?`, `T`.
    fn tuple_member(&mut self) -> R<()> {
        let t = self.tok();
        let n1 = self.peek_at(1).kind;
        if t.kind == T::Name && (n1 == T::Colon || (n1 == T::Question && self.peek_at(2).kind == T::Colon)) {
            let kind = if n1 == T::Colon { K::RequiredParameter } else { K::OptionalParameter };
            let n = self.open(kind, f::NONE, C_NONE);
            self.leaf(K::Identifier, f::NAME, C_NONE)?;
            if n1 == T::Question {
                self.tk(kind, K::TQuestion, f::NONE)?;
            }
            self.type_annotation(f::TYPE)?;
            self.close(n);
            return Ok(());
        }
        if t.kind == T::Ellipsis {
            if self.peek_at(1).kind == T::Name && self.peek_at(2).kind == T::Colon {
                let n = self.open(K::RequiredParameter, f::NONE, C_NONE);
                let r = self.open(K::RestPattern, f::NAME, C_NONE);
                self.bump()?;
                self.leaf(K::Identifier, f::NONE, C_NONE)?;
                self.close(r);
                self.type_annotation(f::TYPE)?;
                self.close(n);
                return Ok(());
            }
            let n = self.open(K::RestType, f::NONE, C_NONE);
            self.bump()?;
            self.ty(TYPE)?;
            self.close(n);
            return Ok(());
        }
        let start = self.here();
        let e = self.ty(TYPE)?;
        if self.peek() == T::Question {
            let n = self.wrap(N { at: start, kind: e.kind }, K::OptionalType, BARE, TYPE);
            self.bump()?;
            self.close(n);
        }
        Ok(())
    }

    fn template_literal_type(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::TemplateLiteralType, s);
        let t = self.bump()?;
        let mut pos = t.end as usize;
        self.relex_at(pos);
        loop {
            let (piece, end) = self.lx.template_piece(pos)?;
            match piece {
                Piece::Chars => self.leaf_span(K::StringFragment, f::NONE, pos as u32, end as u32),
                // An escape is no piece of a template literal type's.
                Piece::Escape => return self.refuse_at(pos),
                Piece::Close => {
                    self.last_end = end as u32;
                    self.relex_at(end);
                    break;
                }
                Piece::Open => {
                    let sub = self.open_span(K::TemplateType, f::NONE, pos as u32);
                    self.last_end = end as u32;
                    self.relex_at(end);
                    let saved = self.asi;
                    self.asi = false;
                    if self.is_word(w::INFER) {
                        return self.refuse();
                    }
                    self.ty(slot(f::NONE, Ctx::PrimaryType))?;
                    self.asi = saved;
                    let r = self.tok();
                    if r.kind != T::RBrace {
                        return self.refuse();
                    }
                    self.flush(r.start);
                    self.last_end = r.end;
                    self.close(sub);
                    self.relex_at(r.end as usize);
                    pos = r.end as usize;
                    continue;
                }
            }
            pos = end;
        }
        self.close(n);
        Ok(n)
    }

    /// `{ members }` of an object type or an interface body (the node open): its braces and
    /// separators stored.
    pub(super) fn object_type_body(&mut self, kind: K) -> R<()> {
        self.leaf(K::TLBrace, f::NONE, C_NONE)?;
        let saved = self.asi;
        self.asi = true;
        loop {
            match self.peek() {
                T::RBrace => break,
                T::Semi | T::Comma => return self.refuse(),
                _ => self.type_member()?,
            }
            match self.peek() {
                T::Semi => {
                    self.leaf(K::TSemi, f::NONE, C_NONE)?;
                }
                T::Comma => {
                    self.leaf(K::TComma, f::NONE, C_NONE)?;
                }
                T::RBrace => break,
                _ if self.asi_end(0) => {}
                _ => return self.refuse(),
            }
        }
        self.asi = saved;
        let _ = kind;
        self.leaf(K::TRBrace, f::NONE, C_NONE)?;
        Ok(())
    }

    /// An object type's member.
    fn type_member(&mut self) -> R<()> {
        let t = self.tok();
        let n1 = self.peek_at(1);
        // Index signatures and mapped types.
        if t.kind == T::LBracket && self.index_signature_ahead(0) {
            return self.index_signature(false);
        }
        if t.kind == T::Name && t.word == w::READONLY && n1.kind == T::LBracket && self.index_signature_ahead(1) {
            return self.index_signature(false);
        }
        if matches!(t.kind, T::Minus | T::Plus) && self.is_word_at(1, w::READONLY) {
            return self.index_signature(false);
        }
        if matches!(t.kind, T::LParen | T::Lt) {
            let n = self.open(K::CallSignature, f::NONE, C_NONE);
            self.call_signature()?;
            self.close(n);
            return Ok(());
        }
        if t.kind == T::Name && t.word == w::NEW && matches!(n1.kind, T::LParen | T::Lt) {
            let n = self.open(K::ConstructSignature, f::NONE, C_NONE);
            self.tk(K::ConstructSignature, K::TNew, f::NONE)?;
            if self.peek() == T::Lt {
                self.type_parameters()?;
            }
            self.formal_parameters()?;
            if self.peek() == T::Colon {
                self.type_annotation(f::TYPE)?;
            }
            self.close(n);
            return Ok(());
        }
        let n = self.open(K::PropertySignature, f::NONE, C_NONE);
        let mut method = false;
        // Modifiers: a word a member's name follows.
        loop {
            let t = self.tok();
            if t.kind != T::Name || !self.member_modifier() || self.peek_at(1).nl {
                break;
            }
            if let Some(k) = accessibility(t.word) {
                let a = self.open(K::AccessibilityModifier, f::NONE, C_NONE);
                self.leaf(k, f::NONE, C_NONE)?;
                self.close(a);
            } else if t.word == w::READONLY {
                self.leaf(K::TReadonly, f::NONE, C_NONE)?;
            } else if matches!(t.word, w::GET | w::SET) && !method {
                let k = if t.word == w::GET { K::TGet } else { K::TSet };
                self.leaf(k, f::NONE, C_NONE)?;
                method = true;
            } else {
                return self.refuse();
            }
        }
        self.property_name(f::NAME)?;
        let mut kind = K::PropertySignature;
        if self.peek() == T::Question {
            self.leaf(K::TQuestion, f::NONE, C_NONE)?;
        }
        if matches!(self.peek(), T::LParen | T::Lt) {
            kind = K::MethodSignature;
            self.w.set_key(n.at, key(kind, f::NONE, C_NONE, false));
            self.call_signature()?;
        } else if method {
            return self.refuse();
        } else if self.peek() == T::Colon {
            self.type_annotation(f::TYPE)?;
        }
        let _ = kind;
        self.close(n);
        Ok(())
    }

    /// Whether the `[` `k` tokens ahead opens an index signature or a mapped type (`[name:`,
    /// `[name in`), not a computed name.
    fn index_signature_ahead(&mut self, k: usize) -> bool {
        self.peek_at(k + 1).kind == T::Name && (self.peek_at(k + 2).kind == T::Colon || self.is_word_at(k + 2, w::IN))
    }

    /// `[name: T]: U`, `[K in T as A]?: U` with their modifiers (`readonly`, `-readonly`).
    pub(super) fn index_signature(&mut self, class: bool) -> R<()> {
        let n = self.open(K::IndexSignature, f::NONE, C_NONE);
        let mut readonly = false;
        if class && self.is_word(w::STATIC) {
            self.leaf(K::TStatic, f::NONE, C_NONE)?;
        }
        if matches!(self.peek(), T::Minus | T::Plus) {
            let k = if self.peek() == T::Minus { K::TMinus } else { K::TPlus };
            self.leaf(k, f::SIGN, C_NONE)?;
        }
        if self.is_word(w::READONLY) {
            self.leaf(K::TReadonly, f::NONE, C_NONE)?;
            readonly = true;
        }
        if self.peek() != T::LBracket {
            return self.refuse();
        }
        // The `[` is stored after `readonly` (the shape does not say where it stands then).
        if readonly {
            self.leaf(K::TLBracket, f::NONE, C_NONE)?;
        } else {
            self.bump()?;
        }
        let saved = self.asi;
        self.asi = false;
        if self.peek_at(1).kind == T::Colon {
            self.leaf(K::Identifier, f::NAME, C_NONE)?;
            self.bump()?;
            self.ty(slot(f::INDEX_TYPE, Ctx::Type))?;
        } else {
            let m = self.open(K::MappedTypeClause, f::NONE, C_NONE);
            self.leaf(K::TypeIdentifier, f::NAME, C_NONE)?;
            self.tk(K::MappedTypeClause, K::TIn, f::NONE)?;
            self.ty(slot(f::TYPE, Ctx::Type))?;
            if self.is_word(w::AS) {
                self.tk(K::MappedTypeClause, K::TAs, f::NONE)?;
                self.ty(slot(f::ALIAS, Ctx::Type))?;
            }
            self.close(m);
        }
        self.expect(T::RBracket)?;
        self.asi = saved;
        // `: T`, `?: T`, `-?: T`, `+?: T` (each one token: no space inside).
        let t = self.tok();
        let kind = match t.kind {
            T::Colon => None,
            T::Question => Some(K::OptingTypeAnnotation),
            T::Minus => Some(K::OmittingTypeAnnotation),
            T::Plus => Some(K::AddingTypeAnnotation),
            _ => return self.refuse(),
        };
        match kind {
            None => self.type_annotation(f::TYPE)?,
            Some(kind) => {
                let a = self.open(kind, f::TYPE, C_NONE);
                let mut end = t.end;
                if t.kind != T::Question {
                    self.bump()?;
                    let q = self.tok();
                    if q.kind != T::Question || q.start != end {
                        return self.refuse();
                    }
                    end = q.end;
                }
                self.bump()?;
                let c = self.tok();
                if c.kind != T::Colon || c.start != end {
                    return self.refuse();
                }
                self.bump()?;
                self.ty(TYPE)?;
                self.close(a);
            }
        }
        self.close(n);
        Ok(())
    }

    /// `: T` as a `type_annotation` in `field`.
    pub(super) fn type_annotation(&mut self, field: u8) -> R<()> {
        let n = self.open(K::TypeAnnotation, field, C_NONE);
        self.tk(K::TypeAnnotation, K::TColon, f::NONE)?;
        self.ty(TYPE)?;
        self.close(n);
        Ok(())
    }

    /// A function's return type after its parameters: `: T`, `: x is T`, `: asserts x is T`.
    pub(super) fn return_type(&mut self) -> R<()> {
        let n1 = self.peek_at(1);
        let n2 = self.peek_at(2);
        let predicate_name = |t: Tok| t.kind == T::Name;
        if n1.kind == T::Name && n1.word == w::ASSERTS && predicate_name(n2) && !n2.nl {
            let n = self.open(K::AssertsAnnotation, f::RETURN_TYPE, C_NONE);
            self.bump()?;
            let a = self.open(K::Asserts, f::NONE, C_NONE);
            self.tk(K::Asserts, K::TAsserts, f::NONE)?;
            if self.is_word_at(1, w::IS) {
                self.type_predicate()?;
            } else {
                let kind = if self.is_word(w::THIS) { K::This } else { K::Identifier };
                self.leaf(kind, f::NONE, C_NONE)?;
            }
            self.close(a);
            self.close(n);
            return Ok(());
        }
        if predicate_name(n1) && n2.kind == T::Name && n2.word == w::IS && !n2.nl {
            let n = self.open(K::TypePredicateAnnotation, f::RETURN_TYPE, C_NONE);
            self.bump()?;
            self.type_predicate()?;
            self.close(n);
            return Ok(());
        }
        self.type_annotation(f::RETURN_TYPE)
    }

    /// `name is T`.
    fn type_predicate(&mut self) -> R<()> {
        let n = self.open(K::TypePredicate, f::NONE, C_NONE);
        let t = self.tok();
        if let Some(k) = predefined(t.word).filter(|_| t.kind == T::Name) {
            let i = self.open(K::Identifier, f::NAME, C_NONE);
            self.leaf(k, f::NONE, C_NONE)?;
            self.close(i);
        } else {
            let kind = if self.is_word(w::THIS) { K::This } else { K::Identifier };
            self.leaf(kind, f::NAME, C_NONE)?;
        }
        self.tk(K::TypePredicate, K::TIs, f::NONE)?;
        self.ty(slot(f::TYPE, Ctx::Type))?;
        self.close(n);
        Ok(())
    }

    /// `<T, U extends V = W>` in `type_parameters`.
    pub(super) fn type_parameters(&mut self) -> R<()> {
        let n = self.open(K::TypeParameters, f::TYPE_PARAMETERS, C_NONE);
        self.bump()?;
        let saved = self.asi;
        self.asi = false;
        loop {
            self.type_parameter()?;
            match self.peek() {
                T::Comma => {
                    let c = self.bump()?;
                    if self.peek() == T::Gt {
                        self.leaf_tok(c, K::TComma);
                        self.leaf(K::TGt, f::NONE, C_NONE)?;
                        break;
                    }
                }
                _ => {
                    self.close_angle()?;
                    break;
                }
            }
        }
        self.asi = saved;
        self.close(n);
        Ok(())
    }

    fn type_parameter(&mut self) -> R<()> {
        let n = self.open(K::TypeParameter, f::NONE, C_NONE);
        if self.is_word(w::CONST) && self.peek_at(1).kind == T::Name {
            self.tk(K::TypeParameter, K::TConst, f::NONE)?;
        }
        if (self.is_word(w::IN) || self.is_word(w::OUT)) && self.peek_at(1).kind == T::Name {
            return self.refuse();
        }
        if self.peek() != T::Name {
            return self.refuse();
        }
        self.leaf(K::TypeIdentifier, f::NAME, C_NONE)?;
        if self.is_word(w::EXTENDS) {
            let c = self.open(K::Constraint, f::CONSTRAINT, C_NONE);
            self.tk(K::Constraint, K::TExtends, f::NONE)?;
            self.ty(TYPE)?;
            self.close(c);
        }
        if self.peek() == T::Eq {
            let d = self.open(K::DefaultType, f::VALUE, C_NONE);
            self.tk(K::DefaultType, K::TEq, f::NONE)?;
            self.ty(TYPE)?;
            self.close(d);
        }
        self.close(n);
        Ok(())
    }

    /// `<T, U>` in `field` (a call's, a generic type's).
    pub(super) fn type_arguments(&mut self, field: u8) -> R<()> {
        let n = self.open(K::TypeArguments, field, C_NONE);
        self.bump()?;
        let saved = self.asi;
        self.asi = false;
        loop {
            self.ty(TYPE)?;
            match self.peek() {
                T::Comma => {
                    let c = self.bump()?;
                    if self.peek() == T::Gt {
                        self.leaf_tok(c, K::TComma);
                        self.leaf(K::TGt, f::NONE, C_NONE)?;
                        break;
                    }
                }
                _ => {
                    self.close_angle()?;
                    break;
                }
            }
        }
        self.asi = saved;
        self.close(n);
        Ok(())
    }

    /// The `>` closing type arguments or parameters: a `>`, or the first byte of a token it starts
    /// (`>>`, `>=`, `>>=`, …), the rest lexed again.
    fn close_angle(&mut self) -> R<()> {
        let t = self.tok();
        match t.kind {
            T::Gt => {
                self.bump()?;
            }
            T::GtGt | T::GtGtGt | T::Ge | T::GtGtEq | T::GtGtGtEq => {
                self.flush(t.start);
                self.last_end = t.start + 1;
                self.relex_at(t.start as usize + 1);
            }
            _ => return self.refuse(),
        }
        Ok(())
    }

    /// `<T>(params): R` (the type parameters and return type TypeScript's).
    pub(super) fn call_signature(&mut self) -> R<()> {
        if self.peek() == T::Lt {
            self.type_parameters()?;
        }
        self.formal_parameters()?;
        if self.peek() == T::Colon {
            self.return_type()?;
        }
        Ok(())
    }

    // ---- Parameters.

    /// A TypeScript parameter: `required_parameter` or `optional_parameter`, with its modifiers,
    /// pattern, `?`, type and default.
    pub(super) fn ts_parameter(&mut self) -> R<()> {
        let n = self.open(K::RequiredParameter, f::NONE, C_NONE);
        if self.peek() == T::At {
            return self.refuse();
        }
        // Modifiers: a word a pattern follows.
        loop {
            let t = self.tok();
            let next = self.peek_at(1).kind;
            if t.kind != T::Name || !matches!(next, T::Name | T::LBrace | T::LBracket | T::Ellipsis) {
                break;
            }
            if let Some(k) = accessibility(t.word) {
                let a = self.open(K::AccessibilityModifier, f::NONE, C_NONE);
                self.leaf(k, f::NONE, C_NONE)?;
                self.close(a);
            } else if t.word == w::OVERRIDE {
                let a = self.open(K::OverrideModifier, f::NONE, C_NONE);
                self.leaf(K::TOverride, f::NONE, C_NONE)?;
                self.close(a);
            } else if t.word == w::READONLY {
                self.leaf(K::TReadonly, f::NONE, C_NONE)?;
            } else {
                break;
            }
        }
        let pat = slot(f::PATTERN, Ctx::Pattern);
        if self.is_word(w::THIS) {
            self.leaf(K::This, f::PATTERN, C_NONE)?;
        } else if self.peek() == T::Ellipsis {
            self.rest_pattern(pat)?;
        } else {
            self.open_pattern_target(pat)?;
        }
        let mut kind = K::RequiredParameter;
        if self.peek() == T::Question {
            kind = K::OptionalParameter;
            self.w.set_key(n.at, key(kind, f::NONE, C_NONE, false));
            self.tk(kind, K::TQuestion, f::NONE)?;
        }
        if self.peek() == T::Colon {
            self.type_annotation(f::TYPE)?;
        }
        if self.peek() == T::Eq {
            self.tk(kind, K::TEq, f::NONE)?;
            self.assignment(slot(f::VALUE, Ctx::Expr))?;
        }
        self.close(n);
        Ok(())
    }

    // ---- Declarations.

    /// TypeScript's statements and declarations at `t` (a name): `None` where `t` starts none.
    pub(super) fn ts_statement(&mut self, s: Slot, t: Tok) -> Option<R<()>> {
        let n1 = self.peek_at(1);
        let same_line = !n1.nl;
        Some(match t.word {
            w::INTERFACE if n1.kind == T::Name && same_line => self.interface_declaration(s),
            w::TYPE if n1.kind == T::Name && same_line && matches!(self.peek_at(2).kind, T::Eq | T::Lt) => self.type_alias(s),
            w::ENUM if n1.kind == T::Name => self.enum_declaration(s),
            w::CONST if n1.kind == T::Name && n1.word == w::ENUM => self.enum_declaration(s),
            w::ABSTRACT if n1.kind == T::Name && n1.word == w::CLASS && same_line => self.class_declaration(s),
            w::DECLARE if same_line && self.declares_at(1) => self.ambient(s),
            w::NAMESPACE if same_line && n1.kind == T::Name => {
                // At a statement's place, `namespace N {}` is an expression statement.
                let e = self.open_in(K::ExpressionStatement, s);
                let r = self.internal_module(EXPR);
                if let Err(r) = r {
                    return Some(Err(r));
                }
                self.after = V_EXPR;
                let r = self.semicolon();
                self.close(e);
                r
            }
            w::MODULE if same_line && matches!(n1.kind, T::Name | T::Str) => self.module(s),
            _ => return None,
        })
    }

    /// Whether the token `k` ahead starts a declaration `declare` may precede.
    fn declares_at(&mut self, k: usize) -> bool {
        let t = self.peek_at(k);
        t.kind == T::Name
            && matches!(t.word, w::FUNCTION | w::CONST | w::LET | w::VAR | w::CLASS | w::ABSTRACT | w::ENUM | w::INTERFACE | w::TYPE | w::NAMESPACE | w::MODULE | w::GLOBAL | w::ASYNC)
    }

    /// A declaration where a `declaration` stands (an export's, `declare`'s): TypeScript's or
    /// JavaScript's. `None` where the next token starts none.
    pub(super) fn ts_declaration(&mut self, d: Slot) -> Option<R<()>> {
        let t = self.tok();
        if !self.ts || t.kind != T::Name {
            return None;
        }
        let n1 = self.peek_at(1);
        Some(match t.word {
            w::INTERFACE => self.interface_declaration(d),
            w::TYPE if n1.kind == T::Name => self.type_alias(d),
            w::ENUM => self.enum_declaration(d),
            w::CONST if n1.word == w::ENUM && n1.kind == T::Name => self.enum_declaration(d),
            w::ABSTRACT if n1.word == w::CLASS => self.class_declaration(d),
            w::DECLARE if self.declares_at(1) => self.ambient(d),
            w::NAMESPACE if n1.kind == T::Name => self.internal_module(d).map(|_| ()),
            w::MODULE if matches!(n1.kind, T::Name | T::Str) => self.module(d),
            _ => return None,
        })
    }

    fn interface_declaration(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::InterfaceDeclaration, s);
        self.tk(K::InterfaceDeclaration, K::TInterface, f::NONE)?;
        self.leaf(K::TypeIdentifier, f::NAME, C_NONE)?;
        if self.peek() == T::Lt {
            self.type_parameters()?;
        }
        if self.is_word(w::EXTENDS) {
            let e = self.open(K::ExtendsTypeClause, f::NONE, C_NONE);
            self.tk(K::ExtendsTypeClause, K::TExtends, f::NONE)?;
            loop {
                if self.peek() != T::Name {
                    return self.refuse();
                }
                self.type_reference(slot(f::TYPE, Ctx::Bare))?;
                if self.peek() != T::Comma {
                    break;
                }
                self.bump()?;
            }
            self.close(e);
        }
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        let b = self.open(K::InterfaceBody, f::BODY, C_NONE);
        self.object_type_body(K::InterfaceBody)?;
        self.close(b);
        self.close(n);
        self.after = V_EXPR;
        Ok(())
    }

    fn type_alias(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::TypeAliasDeclaration, s);
        self.tk(K::TypeAliasDeclaration, K::TType, f::NONE)?;
        self.leaf(K::TypeIdentifier, f::NAME, C_NONE)?;
        if self.peek() == T::Lt {
            self.type_parameters()?;
        }
        if self.peek() != T::Eq {
            return self.refuse();
        }
        self.tk(K::TypeAliasDeclaration, K::TEq, f::NONE)?;
        let saved = self.asi;
        self.asi = true;
        self.ty(slot(f::VALUE, Ctx::Type))?;
        self.asi = saved;
        self.after = 0;
        self.semicolon()?;
        self.close(n);
        Ok(())
    }

    fn enum_declaration(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::EnumDeclaration, s);
        if self.is_word(w::CONST) {
            self.tk(K::EnumDeclaration, K::TConst, f::NONE)?;
        }
        self.tk(K::EnumDeclaration, K::TEnum, f::NONE)?;
        if self.peek() != T::Name {
            return self.refuse();
        }
        self.leaf(K::Identifier, f::NAME, C_NONE)?;
        let b = self.open(K::EnumBody, f::BODY, C_NONE);
        self.expect(T::LBrace)?;
        let saved = self.asi;
        self.asi = false;
        loop {
            if self.peek() == T::RBrace {
                self.bump()?;
                break;
            }
            let start = self.here();
            self.property_name(f::NAME)?;
            if self.peek() == T::Eq {
                let kind = kind_of(unkey(self.w.key(start)).0);
                let a = self.wrap(N { at: start, kind }, K::EnumAssignment, BARE, slot(f::NAME, Ctx::Bare));
                self.tk(K::EnumAssignment, K::TEq, f::NONE)?;
                self.assignment(slot(f::VALUE, Ctx::Expr))?;
                self.close(a);
            }
            match self.peek() {
                T::Comma => {
                    let c = self.bump()?;
                    if self.peek() == T::RBrace {
                        self.leaf_tok(c, K::TComma);
                        self.leaf(K::TRBrace, f::NONE, C_NONE)?;
                        break;
                    }
                }
                T::RBrace => {
                    self.bump()?;
                    break;
                }
                _ => return self.refuse(),
            }
        }
        self.asi = saved;
        self.close(b);
        self.close(n);
        self.after = V_EXPR;
        Ok(())
    }

    /// `declare` and a declaration.
    fn ambient(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::AmbientDeclaration, s);
        self.tk(K::AmbientDeclaration, K::TDeclare, f::NONE)?;
        if self.is_word(w::GLOBAL) {
            if self.peek_at(1).kind != T::LBrace {
                return self.refuse();
            }
            self.tk(K::AmbientDeclaration, K::TGlobal, f::NONE)?;
            self.statement_block(BARE, false)?;
            self.close(n);
            return Ok(());
        }
        if self.is_word(w::MODULE) && self.peek_at(1).kind == T::Dot {
            return self.refuse();
        }
        self.declaration(slot(f::NONE, Ctx::Decl))?;
        self.close(n);
        Ok(())
    }

    /// A declaration in slot `d` (an export's, `declare`'s).
    pub(super) fn declaration(&mut self, d: Slot) -> R<()> {
        if let Some(r) = self.ts_declaration(d) {
            return r;
        }
        let t = self.tok();
        if t.kind != T::Name {
            return self.refuse();
        }
        match t.word {
            w::VAR => self.variable_declaration(d),
            w::CONST => self.lexical_declaration(d, K::TConst),
            w::LET => self.lexical_declaration(d, K::TLet),
            w::FUNCTION => self.function(d, true, false),
            w::ASYNC if self.is_word_at(1, w::FUNCTION) => self.function(d, true, true),
            w::CLASS => self.class_declaration(d),
            _ => self.refuse(),
        }
    }

    /// `namespace N {}`, `namespace A.B {}`.
    fn internal_module(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::InternalModule, s);
        self.tk(K::InternalModule, K::TNamespace, f::NONE)?;
        self.module_rest()?;
        self.close(n);
        Ok(n)
    }

    fn module(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::Module, s);
        self.tk(K::Module, K::TModule, f::NONE)?;
        self.module_rest()?;
        self.close(n);
        Ok(())
    }

    /// A module's name and its body (optional).
    fn module_rest(&mut self) -> R<()> {
        match self.peek() {
            T::Str => self.string(slot(f::NAME, Ctx::Bare))?,
            T::Name => {
                let mut k = 0;
                while self.peek_at(2 * k + 1).kind == T::Dot && self.peek_at(2 * k + 2).kind == T::Name {
                    k += 1;
                }
                self.nested_identifier(k, f::NAME, K::NestedIdentifier)?;
            }
            _ => return self.refuse(),
        }
        if self.peek() == T::LBrace {
            self.statement_block(slot(f::BODY, Ctx::Bare), false)?;
        } else {
            return self.refuse();
        }
        self.after = V_EXPR;
        Ok(())
    }

    // ---- Classes.

    /// `extends A<T>, B` and `implements C, D` (TypeScript's `class_heritage`).
    pub(super) fn ts_class_heritage(&mut self) -> R<()> {
        let h = self.open(K::ClassHeritage, f::NONE, C_NONE);
        if self.is_word(w::EXTENDS) {
            let e = self.open(K::ExtendsClause, f::NONE, C_NONE);
            self.tk(K::ExtendsClause, K::TExtends, f::NONE)?;
            loop {
                let saved = self.in_extends;
                self.in_extends = true;
                let r = self.postfix(slot(f::VALUE, Ctx::Expr));
                self.in_extends = saved;
                r?;
                if self.peek() == T::Lt {
                    self.type_arguments(f::TYPE_ARGUMENTS)?;
                }
                if self.peek() != T::Comma {
                    break;
                }
                self.bump()?;
            }
            self.close(e);
        }
        if self.is_word(w::IMPLEMENTS) {
            let i = self.open(K::ImplementsClause, f::NONE, C_NONE);
            self.tk(K::ImplementsClause, K::TImplements, f::NONE)?;
            loop {
                self.ty(TYPE)?;
                if self.peek() != T::Comma {
                    break;
                }
                self.bump()?;
            }
            self.close(i);
        }
        self.close(h);
        Ok(())
    }

    /// A TypeScript class member: a method (or its signature), a field, an index signature, a
    /// static block.
    pub(super) fn ts_class_member(&mut self) -> R<()> {
        if self.peek() == T::At {
            return self.refuse();
        }
        if self.is_word(w::STATIC) && self.peek_at(1).kind == T::LBrace {
            let n = self.open(K::ClassStaticBlock, f::NONE, C_NONE);
            self.tk(K::ClassStaticBlock, K::TStatic, f::NONE)?;
            self.statement_block(slot(f::BODY, Ctx::Bare), true)?;
            self.close(n);
            return Ok(());
        }
        if self.peek() == T::LBracket && self.index_signature_ahead(0) {
            self.index_signature(true)?;
            return self.ts_member_end(K::IndexSignature);
        }
        let n = self.open(K::MethodDefinition, f::NONE, C_NONE);
        let mut is_abstract = false;
        let mut accessor = false;
        let mut star = false;
        loop {
            let t = self.tok();
            if t.kind == T::Star {
                self.leaf(K::TStar, f::NONE, C_NONE)?;
                star = true;
                break;
            }
            if t.kind != T::Name || !self.member_modifier() {
                break;
            }
            let next = self.peek_at(1);
            if let Some(k) = accessibility(t.word) {
                let a = self.open(K::AccessibilityModifier, f::NONE, C_NONE);
                self.leaf(k, f::NONE, C_NONE)?;
                self.close(a);
                continue;
            }
            match t.word {
                w::OVERRIDE => {
                    let a = self.open(K::OverrideModifier, f::NONE, C_NONE);
                    self.leaf(K::TOverride, f::NONE, C_NONE)?;
                    self.close(a);
                }
                w::STATIC => {
                    self.leaf(K::TStatic, f::NONE, C_NONE)?;
                }
                w::READONLY => {
                    self.leaf(K::TReadonly, f::NONE, C_NONE)?;
                }
                w::ABSTRACT => {
                    self.leaf(K::TAbstract, f::NONE, C_NONE)?;
                    is_abstract = true;
                }
                w::DECLARE => {
                    self.leaf(K::TDeclare, f::NONE, C_NONE)?;
                }
                w::ACCESSOR => {
                    self.leaf(K::TAccessor, f::NONE, C_NONE)?;
                    accessor = true;
                }
                w::ASYNC if !next.nl => {
                    self.leaf(K::TAsync, f::NONE, C_NONE)?;
                }
                w::GET | w::SET if !next.nl => {
                    let k = if t.word == w::GET { K::TGet } else { K::TSet };
                    self.leaf(k, f::NONE, C_NONE)?;
                    break;
                }
                _ => break,
            }
        }
        if self.peek() == T::LBracket && self.index_signature_ahead(0) {
            return self.refuse();
        }
        self.property_name(f::NAME)?;
        if self.peek() == T::Question {
            self.leaf(K::TQuestion, f::NONE, C_NONE)?;
        }
        if matches!(self.peek(), T::LParen | T::Lt) {
            if accessor {
                return self.refuse();
            }
            self.call_signature()?;
            if self.peek() == T::LBrace && !is_abstract {
                self.statement_block(slot(f::BODY, Ctx::Bare), true)?;
                self.close(n);
                if self.peek() == T::Semi {
                    self.leaf(K::TSemi, f::NONE, C_NONE)?;
                }
                return Ok(());
            }
            let kind = if is_abstract { K::AbstractMethodSignature } else { K::MethodSignature };
            self.w.set_key(n.at, key(kind, f::NONE, C_NONE, false));
            self.close(n);
            return self.ts_member_end(kind);
        }
        if star {
            return self.refuse();
        }
        // A field.
        self.w.set_key(n.at, key(K::PublicFieldDefinition, f::NONE, C_NONE, false));
        if self.peek() == T::Bang {
            self.leaf(K::TBang, f::NONE, C_NONE)?;
        }
        self.after = V_LPAREN;
        if self.peek() == T::Colon {
            let saved = self.asi;
            self.asi = true;
            self.type_annotation(f::TYPE)?;
            self.asi = saved;
            self.after = 0;
        }
        if self.peek() == T::Eq {
            self.tk(K::PublicFieldDefinition, K::TEq, f::NONE)?;
            let saved = self.asi;
            self.asi = true;
            self.assignment(slot(f::VALUE, Ctx::Expr))?;
            self.asi = saved;
        }
        self.close(n);
        self.ts_member_end(K::PublicFieldDefinition)
    }

    /// A member's end in a class body: `;` or `,` (the body's), or an automatic semicolon.
    fn ts_member_end(&mut self, kind: K) -> R<()> {
        match self.peek() {
            T::Semi => {
                self.leaf(K::TSemi, f::NONE, C_NONE)?;
                Ok(())
            }
            T::Comma => {
                self.leaf(K::TComma, f::NONE, C_NONE)?;
                Ok(())
            }
            _ => {
                let v = if kind == K::MethodSignature { V_FN_SIG } else { self.after };
                if self.asi_end(v) {
                    return Ok(());
                }
                self.refuse()
            }
        }
    }
}
