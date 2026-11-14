//! Statements and items: a block's, a file's and a declaration list's statements, attributes,
//! visibility, functions and their parameters, generics and `where` clauses, `use`, structs,
//! enums, traits, `impl`s and `macro_rules!`.

use super::*;

/// What a statement that starts with an item's words is, read ahead past its visibility and
/// modifiers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Item {
    Fn,
    Struct,
    Union,
    Enum,
    Trait,
    Impl,
    Type,
    Const,
    Static,
    Mod,
    Use,
    ExternCrate,
    ForeignMod,
}

impl<'a> P<'a> {
    /// Statements to the list's end: a `}` (a block's or a declaration list's) or the file's end.
    pub(super) fn statements(&mut self, mode: Stmts) -> R<()> {
        let saved = self.no_struct;
        self.no_struct = false;
        loop {
            match self.peek() {
                T::RBrace if mode != Stmts::File => break,
                T::Eof => break,
                _ => {}
            }
            if !self.statement(mode)? {
                break;
            }
        }
        self.no_struct = saved;
        Ok(())
    }

    /// One statement. `false` where it was a block's last expression (the block ends after it).
    fn statement(&mut self, mode: Stmts) -> R<bool> {
        self.deeper()?;
        let t = self.tok();
        let r = match t.kind {
            T::Hash => {
                if self.peek_at(1).kind == T::Bang {
                    self.inner_attribute_item()?;
                } else {
                    self.attribute_item()?;
                }
                Ok(true)
            }
            T::Semi => {
                let n = self.open(K::EmptyStatement, f::NONE, C_NONE);
                self.tk(K::EmptyStatement, K::TSemi, f::NONE)?;
                self.close(n);
                Ok(true)
            }
            T::Name if t.word == w::LET => {
                self.let_declaration()?;
                Ok(true)
            }
            T::Name if t.word == w::MACRO_RULES && self.peek_at(1).kind == T::Bang && self.adjacent(0) => {
                self.macro_definition()?;
                Ok(true)
            }
            _ => match self.item_ahead() {
                Some(item) => {
                    self.item(item)?;
                    Ok(true)
                }
                None if mode == Stmts::Decl => self.decl_macro(),
                None => self.expression_statement(mode),
            },
        };
        self.depth -= 1;
        r
    }

    /// A macro invocation in a declaration list (no expression statements there), and a `;` after
    /// it an empty statement of its own.
    fn decl_macro(&mut self) -> R<bool> {
        let n = self.expr(BARE, 0)?;
        if n.kind != K::MacroInvocation {
            return self.refuse();
        }
        Ok(true)
    }

    /// An expression statement, a block's last expression, or a macro invocation standing as a
    /// declaration. A block-like expression (`if`, `match`, a block…) at the start ends the
    /// statement unless the next token can only continue an expression (`.`, `?`, `as`, a binary
    /// operator that cannot start one): tree-sitter's `expression_statement` takes it at
    /// precedence 1 where the token could start the next statement.
    fn expression_statement(&mut self, mode: Stmts) -> R<bool> {
        let n = if self.blocklike_start() {
            let n = self.blocklike(BARE)?;
            if !self.continues_expression() {
                let s = self.wrap(n, K::ExpressionStatement, BARE, BARE);
                self.close(s);
                return Ok(true);
            }
            self.rekey(n.at, EXPR);
            self.infix(n, EXPR, 0)?
        } else {
            self.expr(EXPR, 0)?
        };
        match self.peek() {
            T::Semi => {
                let s = self.wrap(n, K::ExpressionStatement, BARE, EXPR);
                self.tk(K::ExpressionStatement, K::TSemi, f::NONE)?;
                self.close(s);
                Ok(true)
            }
            T::RBrace if mode == Stmts::Block => Ok(false),
            _ if n.kind == K::MacroInvocation => {
                self.rekey(n.at, BARE);
                Ok(true)
            }
            _ => self.refuse(),
        }
    }

    /// Whether the next token can only continue an expression (it cannot start a statement).
    pub(super) fn continues_expression(&mut self) -> bool {
        use T::*;
        let t = self.tok();
        match t.kind {
            Dot | Question | Plus | Slash | Percent | Caret | EqEq | Ne | Le | Ge | Gt | GtGt | Eq | PlusEq | MinusEq | StarEq | SlashEq | PercentEq | CaretEq | AmpEq | PipeEq | LtLtEq | GtGtEq => true,
            Name => t.word == w::AS,
            _ => false,
        }
    }

    // ---- Attributes.

    /// `#[attribute]`.
    pub(super) fn attribute_item(&mut self) -> R<()> {
        let n = self.open(K::AttributeItem, f::NONE, C_NONE);
        self.tk(K::AttributeItem, K::THash, f::NONE)?;
        if self.peek() != T::LBracket {
            return self.refuse();
        }
        self.tk(K::AttributeItem, K::TLBracket, f::NONE)?;
        self.attribute()?;
        if self.peek() != T::RBracket {
            return self.refuse();
        }
        self.tk(K::AttributeItem, K::TRBracket, f::NONE)?;
        self.close(n);
        Ok(())
    }

    /// `#![attribute]`.
    pub(super) fn inner_attribute_item(&mut self) -> R<()> {
        let n = self.open(K::InnerAttributeItem, f::NONE, C_NONE);
        self.tk(K::InnerAttributeItem, K::THash, f::NONE)?;
        self.tk(K::InnerAttributeItem, K::TBang, f::NONE)?;
        if self.peek() != T::LBracket {
            return self.refuse();
        }
        self.tk(K::InnerAttributeItem, K::TLBracket, f::NONE)?;
        self.attribute()?;
        if self.peek() != T::RBracket {
            return self.refuse();
        }
        self.tk(K::InnerAttributeItem, K::TRBracket, f::NONE)?;
        self.close(n);
        Ok(())
    }

    /// `path`, `path = value`, `path(tokens)`.
    fn attribute(&mut self) -> R<()> {
        let n = self.open(K::Attribute, f::NONE, C_NONE);
        self.path(BARE)?;
        match self.peek() {
            T::Eq => {
                self.tk(K::Attribute, K::TEq, f::NONE)?;
                self.expr(slot(f::VALUE, Ctx::Expr), 0)?;
            }
            T::LParen | T::LBracket | T::LBrace => self.token_tree(tt::Tt::Delim, f::ARGUMENTS)?,
            _ => {}
        }
        self.close(n);
        Ok(())
    }

    // ---- Items.

    /// The item the next tokens start, read ahead past a visibility and a function's modifiers;
    /// `None` where they start an expression.
    fn item_ahead(&mut self) -> Option<Item> {
        let mut k = 0;
        // A visibility: `pub`, `pub(crate)`, `pub(in path)`; `crate` before an item.
        if self.is_word_at(0, w::PUB) {
            k = 1;
            if self.peek_at(1).kind == T::LParen && self.restricted(1) {
                k = self.close_paren(1)?;
            }
        } else if self.is_word_at(0, w::CRATE) && self.peek_at(1).kind == T::Name && matches!(self.peek_at(1).word, w::FN | w::STRUCT | w::ENUM | w::MOD | w::USE | w::CONST | w::STATIC | w::TYPE | w::TRAIT | w::UNION | w::UNSAFE | w::ASYNC | w::EXTERN) {
            k = 1;
        }
        let vis = k > 0;
        let t = self.peek_at(k);
        if t.kind != T::Name {
            return None;
        }
        let next = self.peek_at(k + 1);
        Some(match t.word {
            w::FN => Item::Fn,
            w::STRUCT => Item::Struct,
            w::ENUM => Item::Enum,
            w::TRAIT => Item::Trait,
            w::IMPL => Item::Impl,
            w::TYPE => Item::Type,
            w::MOD => Item::Mod,
            w::USE => Item::Use,
            w::UNION if next.kind == T::Name => Item::Union,
            w::STATIC if !(next.kind == T::Pipe || next.kind == T::PipePipe || next.kind == T::Name && matches!(next.word, w::MOVE | w::ASYNC)) => Item::Static,
            w::CONST if next.kind == T::LBrace => return None,
            w::CONST if next.kind == T::Name && matches!(next.word, w::FN | w::UNSAFE | w::ASYNC | w::EXTERN) => Item::Fn,
            w::CONST => Item::Const,
            w::UNSAFE | w::ASYNC | w::DEFAULT | w::EXTERN => {
                // Modifiers, then what they qualify.
                let mut j = k;
                loop {
                    let t = self.peek_at(j);
                    if t.kind != T::Name {
                        return None;
                    }
                    match t.word {
                        w::UNSAFE | w::ASYNC | w::DEFAULT | w::CONST => j += 1,
                        w::EXTERN => {
                            if self.is_word_at(j + 1, w::CRATE) {
                                return Some(Item::ExternCrate);
                            }
                            j += 1;
                            if matches!(self.peek_at(j).kind, T::Str | T::RawStr) {
                                j += 1;
                            }
                            if self.peek_at(j).kind == T::LBrace {
                                return Some(Item::ForeignMod);
                            }
                        }
                        w::FN => return Some(Item::Fn),
                        w::IMPL if j > k => return Some(Item::Impl),
                        w::TRAIT if j > k => return Some(Item::Trait),
                        _ => return None,
                    }
                    if j == k + 1 && self.peek_at(j).kind == T::LBrace && !vis {
                        // `unsafe {`, `async {`: a block.
                        return None;
                    }
                }
            }
            _ => return None,
        })
    }

    /// Whether `pub` at `k - 1` is restricted: `(crate)`, `(self)`, `(super)`, `(in …)` (else the
    /// `(` starts a tuple type: `struct S(pub (u8, u8))`).
    fn restricted(&mut self, k: usize) -> bool {
        let t = self.peek_at(k + 1);
        t.kind == T::Name && (matches!(t.word, w::CRATE | w::SELF | w::SUPER) && self.peek_at(k + 2).kind == T::RParen || t.word == w::IN)
    }

    /// The index past the `)` matching the `(` at `k`.
    fn close_paren(&mut self, k: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut j = k;
        loop {
            match self.peek_at(j).kind {
                T::LParen => depth += 1,
                T::RParen => {
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

    /// `pub`, `pub(crate)`, `pub(self)`, `pub(super)`, `pub(in path)`, or `crate`.
    fn visibility(&mut self) -> R<()> {
        let n = self.open(K::VisibilityModifier, f::NONE, C_NONE);
        if self.is_word(w::CRATE) {
            self.leaf(K::Crate, f::NONE, C_NONE)?;
            self.close(n);
            return Ok(());
        }
        self.tk(K::VisibilityModifier, K::TPub, f::NONE)?;
        if self.peek() == T::LParen && self.restricted(0) {
            // `(` is implied before `self`, `super` and `in`; `crate` is the rule's other choice
            // alone, so its `(` is stored.
            let crate_ = self.is_word_at(1, w::CRATE);
            self.gap_tk(K::VisibilityModifier, K::TLParen, crate_)?;
            let t = self.tok();
            match t.word {
                w::CRATE => drop(self.leaf(K::Crate, f::NONE, C_NONE)?),
                w::SELF => drop(self.leaf(K::SelfK, f::NONE, C_NONE)?),
                w::SUPER => drop(self.leaf(K::Super, f::NONE, C_NONE)?),
                _ => {
                    self.tk(K::VisibilityModifier, K::TIn, f::NONE)?;
                    self.path(BARE)?;
                }
            }
            if self.peek() != T::RParen {
                return self.refuse();
            }
            self.tk(K::VisibilityModifier, K::TRParen, f::NONE)?;
        }
        self.close(n);
        Ok(())
    }

    fn item(&mut self, item: Item) -> R<()> {
        let kind = match item {
            Item::Fn => K::FunctionItem,
            Item::Struct => K::StructItem,
            Item::Union => K::UnionItem,
            Item::Enum => K::EnumItem,
            Item::Trait => K::TraitItem,
            Item::Impl => K::ImplItem,
            Item::Type => K::TypeItem,
            Item::Const => K::ConstItem,
            Item::Static => K::StaticItem,
            Item::Mod => K::ModItem,
            Item::Use => K::UseDeclaration,
            Item::ExternCrate => K::ExternCrateDeclaration,
            Item::ForeignMod => K::ForeignModItem,
        };
        let n = self.open(kind, f::NONE, C_NONE);
        let vis = self.is_word(w::PUB) || self.is_word(w::CRATE) && item != Item::ExternCrate;
        if vis {
            if matches!(item, Item::Impl | Item::ForeignMod) {
                // No visibility on an `impl` or an `extern` block.
                return self.refuse();
            }
            self.visibility()?;
        }
        match item {
            Item::Fn => self.function(n)?,
            Item::Struct => self.struct_item()?,
            Item::Union => {
                self.tk(K::UnionItem, K::TUnion, f::NONE)?;
                self.type_name()?;
                self.type_parameters_opt()?;
                self.where_opt()?;
                self.field_declaration_list(slot(f::BODY, Ctx::Bare))?;
            }
            Item::Enum => {
                self.tk(K::EnumItem, K::TEnum, f::NONE)?;
                self.type_name()?;
                self.type_parameters_opt()?;
                self.where_opt()?;
                self.enum_variant_list()?;
            }
            Item::Trait => {
                if self.is_word(w::UNSAFE) {
                    self.tk(K::TraitItem, K::TUnsafe, f::NONE)?;
                }
                self.tk(K::TraitItem, K::TTrait, f::NONE)?;
                self.type_name()?;
                self.type_parameters_opt()?;
                if self.peek() == T::Colon {
                    self.trait_bounds(f::BOUNDS)?;
                }
                self.where_opt()?;
                self.declaration_list()?;
            }
            Item::Impl => self.impl_item()?,
            Item::Type => {
                self.type_item(n)?;
                if vis && kind_of(unkey(self.w.key(n.at)).0) == K::AssociatedType {
                    // An associated type has no visibility.
                    return self.refuse();
                }
            }
            Item::Const => {
                self.tk(K::ConstItem, K::TConst, f::NONE)?;
                self.value_name()?;
                self.expect_tk(K::ConstItem, T::Colon, K::TColon)?;
                self.ty(slot(f::TYPE, Ctx::Type), true)?;
                if self.peek() == T::Eq {
                    self.tk(K::ConstItem, K::TEq, f::NONE)?;
                    self.expr(slot(f::VALUE, Ctx::Expr), 0)?;
                }
                self.expect_tk(K::ConstItem, T::Semi, K::TSemi)?;
            }
            Item::Static => {
                self.tk(K::StaticItem, K::TStatic, f::NONE)?;
                if self.is_word(w::REF) {
                    self.tk(K::StaticItem, K::TRef, f::NONE)?;
                }
                if self.is_word(w::MUT) {
                    self.leaf(K::MutableSpecifier, f::NONE, C_NONE)?;
                }
                self.value_name()?;
                self.expect_tk(K::StaticItem, T::Colon, K::TColon)?;
                self.ty(slot(f::TYPE, Ctx::Type), true)?;
                if self.peek() == T::Eq {
                    self.tk(K::StaticItem, K::TEq, f::NONE)?;
                    self.expr(slot(f::VALUE, Ctx::Expr), 0)?;
                }
                self.expect_tk(K::StaticItem, T::Semi, K::TSemi)?;
            }
            Item::Mod => {
                self.tk(K::ModItem, K::TMod, f::NONE)?;
                self.value_name()?;
                if self.peek() == T::Semi {
                    self.tk(K::ModItem, K::TSemi, f::NONE)?;
                } else {
                    self.declaration_list()?;
                }
            }
            Item::Use => {
                self.tk(K::UseDeclaration, K::TUse, f::NONE)?;
                self.use_clause(slot(f::ARGUMENT, Ctx::Bare))?;
                self.expect_tk(K::UseDeclaration, T::Semi, K::TSemi)?;
            }
            Item::ExternCrate => {
                self.tk(K::ExternCrateDeclaration, K::TExtern, f::NONE)?;
                if !self.is_word(w::CRATE) {
                    return self.refuse();
                }
                self.leaf(K::Crate, f::NONE, C_NONE)?;
                self.value_name()?;
                if self.is_word(w::AS) {
                    self.tk(K::ExternCrateDeclaration, K::TAs, f::NONE)?;
                    self.identifier(slot(f::ALIAS, Ctx::Bare))?;
                }
                self.expect_tk(K::ExternCrateDeclaration, T::Semi, K::TSemi)?;
            }
            Item::ForeignMod => {
                if self.is_word(w::UNSAFE) {
                    self.tk(K::ForeignModItem, K::TUnsafe, f::NONE)?;
                }
                self.extern_modifier()?;
                if self.peek() == T::Semi {
                    self.tk(K::ForeignModItem, K::TSemi, f::NONE)?;
                } else {
                    self.declaration_list()?;
                }
            }
        }
        self.close(n);
        Ok(())
    }

    /// The next token, which must be `t`, under `parent` (as `tk`).
    pub(super) fn expect_tk(&mut self, parent: K, t: T, k: K) -> R<()> {
        if self.peek() != t {
            return self.refuse();
        }
        self.tk(parent, k, f::NONE)
    }

    /// An item's name: `name: identifier` (any word: a keyword where a name stands is a name).
    fn value_name(&mut self) -> R<()> {
        self.identifier(slot(f::NAME, Ctx::Bare))
    }

    /// A name as an `identifier` leaf.
    pub(super) fn identifier(&mut self, s: Slot) -> R<()> {
        if self.peek() != T::Name {
            return self.refuse();
        }
        self.leaf_in(K::Identifier, s)?;
        Ok(())
    }

    /// `name: type_identifier`.
    fn type_name(&mut self) -> R<()> {
        if self.peek() != T::Name {
            return self.refuse();
        }
        self.leaf(K::TypeIdentifier, f::NAME, C_NONE)?;
        Ok(())
    }

    /// `extern "abi"?`.
    fn extern_modifier(&mut self) -> R<()> {
        let n = self.open(K::ExternModifier, f::NONE, C_NONE);
        self.tk(K::ExternModifier, K::TExtern, f::NONE)?;
        if matches!(self.peek(), T::Str | T::RawStr) {
            // Only a plain string (no raw, byte or C string) names an ABI.
            if self.src[self.tok().start as usize] != b'"' {
                return self.refuse();
            }
            self.literal(BARE)?;
        }
        self.close(n);
        Ok(())
    }

    /// A function or a function's signature (`n`, open as a `function_item`): its modifiers,
    /// name, generics, parameters, return type, `where` clause, then a body or `;`.
    fn function(&mut self, n: N) -> R<()> {
        if !self.is_word(w::FN) {
            self.function_modifiers()?;
        }
        self.tk(K::FunctionItem, K::TFn, f::NONE)?;
        if self.peek() == T::Metavariable {
            self.leaf(K::Metavariable, f::NAME, C_NONE)?;
        } else {
            self.value_name()?;
        }
        self.type_parameters_opt()?;
        self.parameters(f::PARAMETERS)?;
        if self.peek() == T::Arrow {
            self.tk(K::FunctionItem, K::TArrow, f::NONE)?;
            self.ty(slot(f::RETURN_TYPE, Ctx::Type), true)?;
        }
        self.where_opt()?;
        if self.peek() == T::Semi {
            self.rekind(n.at, K::FunctionSignatureItem, BARE);
            self.tk(K::FunctionSignatureItem, K::TSemi, f::NONE)?;
        } else {
            self.block(slot(f::BODY, Ctx::Bare))?;
        }
        Ok(())
    }

    /// `async`, `default`, `const`, `unsafe`, `extern "abi"`: one node.
    pub(super) fn function_modifiers(&mut self) -> R<()> {
        let n = self.open(K::FunctionModifiers, f::NONE, C_NONE);
        loop {
            let t = self.tok();
            if t.kind != T::Name {
                break;
            }
            match t.word {
                w::ASYNC => self.tk(K::FunctionModifiers, K::TAsync, f::NONE)?,
                w::DEFAULT => self.tk(K::FunctionModifiers, K::TDefault, f::NONE)?,
                w::CONST => self.tk(K::FunctionModifiers, K::TConst, f::NONE)?,
                w::UNSAFE => self.tk(K::FunctionModifiers, K::TUnsafe, f::NONE)?,
                w::EXTERN => self.extern_modifier()?,
                _ => break,
            }
        }
        self.close(n);
        Ok(())
    }

    /// `( parameter, … )`: a function's (`parameters`), or a function type's.
    pub(super) fn parameters(&mut self, field: u8) -> R<()> {
        if self.peek() != T::LParen {
            return self.refuse();
        }
        let n = self.open(K::Parameters, field, C_NONE);
        let saved = self.no_struct;
        self.no_struct = false;
        self.list(K::Parameters, T::RParen, true, true, |p| {
            if p.peek() == T::Hash {
                p.attribute_item()?;
            }
            p.parameter()
        })?;
        self.no_struct = saved;
        self.close(n);
        Ok(())
    }

    /// A parameter: `self`, `&'a mut self`, `...`, `pattern: type`, or a type alone.
    fn parameter(&mut self) -> R<()> {
        let t = self.tok();
        match t.kind {
            T::DotDotDot => {
                let n = self.open(K::VariadicParameter, f::NONE, C_NONE);
                self.tk(K::VariadicParameter, K::TDotDotDot, f::NONE)?;
                self.close(n);
                return Ok(());
            }
            T::Amp if self.self_after_amp() => return self.self_parameter(),
            T::Name if t.word == w::SELF && self.peek_at(1).kind != T::Colon && self.peek_at(1).kind != T::ColonColon => return self.self_parameter(),
            T::Name if t.word == w::MUT && self.is_word_at(1, w::SELF) && self.peek_at(2).kind != T::Colon => return self.self_parameter(),
            _ => {}
        }
        if !self.colon_ahead() {
            // `&_`, `&mut _`: tree-sitter reads a reference pattern there (`_` is a pattern's
            // token first), then finds no `:`.
            let mut k = 0;
            while matches!(self.peek_at(k).kind, T::Amp | T::AmpAmp) {
                k += 1;
            }
            if k > 0 && self.is_word_at(k, w::MUT) {
                k += 1;
            }
            let t = self.peek_at(k);
            if k > 0 && self.underscore(t) {
                return self.refuse();
            }
            self.ty(TYPE, true)?;
            return Ok(());
        }
        let n = self.open(K::Parameter, f::NONE, C_NONE);
        if self.is_word(w::MUT) {
            self.leaf(K::MutableSpecifier, f::NONE, C_NONE)?;
        }
        if self.is_word(w::SELF) {
            self.leaf(K::SelfK, f::PATTERN, C_NONE)?;
        } else {
            self.pattern(slot(f::PATTERN, Ctx::Pattern), K::Parameter, true)?;
        }
        if self.peek_at(1).kind == T::DotDotDot {
            // `args: ...`: a variadic parameter with a pattern.
            self.rekind(n.at, K::VariadicParameter, BARE);
            self.expect_tk(K::VariadicParameter, T::Colon, K::TColon)?;
            self.tk(K::VariadicParameter, K::TDotDotDot, f::NONE)?;
        } else {
            self.expect_tk(K::Parameter, T::Colon, K::TColon)?;
            self.ty(slot(f::TYPE, Ctx::Type), true)?;
        }
        self.close(n);
        Ok(())
    }

    /// Whether `&` begins a self parameter: `&self`, `&mut self`, `&'a self`, `&'a mut self`.
    fn self_after_amp(&mut self) -> bool {
        let mut k = 1;
        if self.peek_at(k).kind == T::Lifetime {
            k += 1;
        }
        if self.is_word_at(k, w::MUT) {
            k += 1;
        }
        self.is_word_at(k, w::SELF) && !matches!(self.peek_at(k + 1).kind, T::Colon | T::ColonColon)
    }

    fn self_parameter(&mut self) -> R<()> {
        let n = self.open(K::SelfParameter, f::NONE, C_NONE);
        if self.peek() == T::Amp {
            self.tk(K::SelfParameter, K::TAmp, f::NONE)?;
        }
        if self.peek() == T::Lifetime {
            self.lifetime(BARE)?;
        }
        if self.is_word(w::MUT) {
            self.leaf(K::MutableSpecifier, f::NONE, C_NONE)?;
        }
        self.leaf(K::SelfK, f::NONE, C_NONE)?;
        self.close(n);
        Ok(())
    }

    /// Whether a `:` follows at this depth before the item's end (`,` or `)`): a parameter's
    /// pattern then its type, else a type alone.
    pub(super) fn colon_ahead(&mut self) -> bool {
        let mut depth = 0usize;
        let mut k = 0;
        loop {
            match self.peek_at(k).kind {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace if depth == 0 => return false,
                T::RParen | T::RBracket | T::RBrace => depth -= 1,
                T::Comma | T::Pipe if depth == 0 => return false,
                T::Colon if depth == 0 => return true,
                T::Eof => return false,
                _ => {}
            }
            k += 1;
        }
    }

    // ---- Generics.

    pub(super) fn type_parameters_opt(&mut self) -> R<()> {
        if matches!(self.peek(), T::Lt | T::LtLt) {
            self.type_parameters()?;
        }
        Ok(())
    }

    /// `<'a, T: Bound = Default, const N: usize = 3>`.
    pub(super) fn type_parameters(&mut self) -> R<()> {
        self.type_parameters_in(f::TYPE_PARAMETERS)
    }

    /// `<…>` in `field`.
    pub(super) fn type_parameters_in(&mut self, field: u8) -> R<()> {
        if self.peek() == T::LtLt {
            return self.refuse();
        }
        let n = self.open(K::TypeParameters, field, C_NONE);
        self.tk(K::TypeParameters, K::TLt, f::NONE)?;
        loop {
            self.outer_attributes()?;
            let t = self.tok();
            match t.kind {
                T::Lifetime => {
                    let p = self.open(K::LifetimeParameter, f::NONE, C_NONE);
                    self.lifetime(slot(f::NAME, Ctx::Bare))?;
                    if self.peek() == T::Colon {
                        self.trait_bounds(f::BOUNDS)?;
                    }
                    self.close(p);
                }
                T::Metavariable => drop(self.leaf(K::Metavariable, f::NONE, C_NONE)?),
                T::Name if t.word == w::CONST => {
                    let p = self.open(K::ConstParameter, f::NONE, C_NONE);
                    self.tk(K::ConstParameter, K::TConst, f::NONE)?;
                    self.value_name()?;
                    self.expect_tk(K::ConstParameter, T::Colon, K::TColon)?;
                    self.ty(slot(f::TYPE, Ctx::Type), true)?;
                    if self.peek() == T::Eq {
                        self.tk(K::ConstParameter, K::TEq, f::NONE)?;
                        let v = slot(f::VALUE, Ctx::Literal);
                        match self.peek() {
                            T::LBrace => drop(self.block(v)?),
                            T::Minus => drop(self.negative_literal(v)?),
                            T::Name if !matches!(self.tok().word, w::TRUE | w::FALSE) => drop(self.leaf_in(K::Identifier, v)?),
                            _ => drop(self.literal(v)?),
                        }
                    }
                    self.close(p);
                }
                T::Name => {
                    let p = self.open(K::TypeParameter, f::NONE, C_NONE);
                    self.type_name()?;
                    if self.peek() == T::Colon {
                        self.trait_bounds(f::BOUNDS)?;
                    }
                    if self.peek() == T::Eq {
                        self.tk(K::TypeParameter, K::TEq, f::NONE)?;
                        self.ty(slot(f::DEFAULT_TYPE, Ctx::Type), true)?;
                    }
                    self.close(p);
                }
                _ => return self.refuse(),
            }
            self.split_gt();
            match self.peek() {
                T::Comma => {
                    let trailing = matches!(self.peek_at(1).kind, T::Gt | T::GtGt | T::Ge | T::GtGtEq);
                    self.gap_tk(K::TypeParameters, K::TComma, trailing)?;
                    if trailing {
                        self.split_gt();
                        self.leaf(K::TGt, f::NONE, C_NONE)?;
                        break;
                    }
                }
                T::Gt => {
                    self.tk(K::TypeParameters, K::TGt, f::NONE)?;
                    break;
                }
                _ => return self.refuse(),
            }
        }
        self.close(n);
        Ok(())
    }

    /// The next token, where it starts with `>` (`>>`, `>=`, `>>=`), read as `>` alone.
    pub(super) fn split_gt(&mut self) {
        if matches!(self.peek(), T::GtGt | T::Ge | T::GtGtEq) {
            self.split(T::Gt, 1);
        }
    }

    /// `: bound + bound…`: types (no `+` of their own), lifetimes, `for<'a> …`.
    pub(super) fn trait_bounds(&mut self, field: u8) -> R<()> {
        let n = self.open(K::TraitBounds, field, C_NONE);
        self.tk(K::TraitBounds, K::TColon, f::NONE)?;
        loop {
            match self.peek() {
                T::Lifetime => self.lifetime(BARE)?,
                T::Name if self.tok().word == w::FOR && matches!(self.peek_at(1).kind, T::Lt) => self.higher_ranked(BARE)?,
                _ => drop(self.ty(TYPE, false)?),
            }
            if self.peek() != T::Plus {
                break;
            }
            self.tk(K::TraitBounds, K::TPlus, f::NONE)?;
        }
        self.close(n);
        Ok(())
    }

    /// `for<'a> type`: a `higher_ranked_trait_bound`.
    pub(super) fn higher_ranked(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::HigherRankedTraitBound, s);
        self.tk(K::HigherRankedTraitBound, K::TFor, f::NONE)?;
        self.type_parameters()?;
        self.ty(slot(f::TYPE, Ctx::Type), false)?;
        self.close(n);
        Ok(())
    }

    pub(super) fn where_opt(&mut self) -> R<()> {
        if self.is_word(w::WHERE) {
            self.where_clause()?;
        }
        Ok(())
    }

    /// `where left: bounds, …`; a trailing `,` is stored.
    fn where_clause(&mut self) -> R<()> {
        let n = self.open(K::WhereClause, f::NONE, C_NONE);
        self.tk(K::WhereClause, K::TWhere, f::NONE)?;
        loop {
            if matches!(self.peek(), T::LBrace | T::Semi | T::Eq) {
                break;
            }
            let p = self.open(K::WherePredicate, f::NONE, C_NONE);
            let left = slot(f::LEFT, Ctx::Bare);
            match self.peek() {
                T::Lifetime => self.lifetime(left)?,
                T::Name if self.is_word(w::FOR) && self.peek_at(1).kind == T::Lt => self.higher_ranked(left)?,
                _ => drop(self.ty(left, false)?),
            }
            if self.peek() != T::Colon {
                return self.refuse();
            }
            self.trait_bounds(f::BOUNDS)?;
            self.close(p);
            if self.peek() != T::Comma {
                break;
            }
            let trailing = matches!(self.peek_at(1).kind, T::LBrace | T::Semi | T::Eq);
            self.gap_tk(K::WhereClause, K::TComma, trailing)?;
            if trailing {
                break;
            }
        }
        self.close(n);
        Ok(())
    }

    // ---- Item bodies.

    /// `{ item… }`.
    fn declaration_list(&mut self) -> R<()> {
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        let n = self.open(K::DeclarationList, f::BODY, C_NONE);
        self.tk(K::DeclarationList, K::TLBrace, f::NONE)?;
        self.statements(Stmts::Decl)?;
        self.expect_tk(K::DeclarationList, T::RBrace, K::TRBrace)?;
        self.close(n);
        Ok(())
    }

    fn struct_item(&mut self) -> R<()> {
        self.tk(K::StructItem, K::TStruct, f::NONE)?;
        self.type_name()?;
        self.type_parameters_opt()?;
        match self.peek() {
            T::Semi => self.tk(K::StructItem, K::TSemi, f::NONE)?,
            T::LParen => {
                self.ordered_field_declaration_list(slot(f::BODY, Ctx::Bare))?;
                self.where_opt()?;
                self.expect_tk(K::StructItem, T::Semi, K::TSemi)?;
            }
            _ => {
                self.where_opt()?;
                if self.peek() == T::Semi {
                    self.tk(K::StructItem, K::TSemi, f::NONE)?;
                } else {
                    self.field_declaration_list(slot(f::BODY, Ctx::Bare))?;
                }
            }
        }
        Ok(())
    }

    /// `{ name: type, … }`.
    fn field_declaration_list(&mut self, s: Slot) -> R<()> {
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        let n = self.open_in(K::FieldDeclarationList, s);
        self.list(K::FieldDeclarationList, T::RBrace, false, true, |p| {
            p.outer_attributes()?;
            let d = p.open(K::FieldDeclaration, f::NONE, C_NONE);
            if p.is_word(w::PUB) || p.is_word(w::CRATE) && p.peek_at(1).kind == T::Name {
                p.visibility()?;
            }
            if p.peek() != T::Name {
                return p.refuse();
            }
            p.leaf(K::FieldIdentifier, f::NAME, C_NONE)?;
            p.expect_tk(K::FieldDeclaration, T::Colon, K::TColon)?;
            p.ty(slot(f::TYPE, Ctx::Type), true)?;
            p.close(d);
            Ok(())
        })?;
        self.close(n);
        Ok(())
    }

    /// `( pub type, … )`.
    fn ordered_field_declaration_list(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::OrderedFieldDeclarationList, s);
        self.list(K::OrderedFieldDeclarationList, T::RParen, false, true, |p| {
            p.outer_attributes()?;
            if p.is_word(w::PUB) {
                p.visibility()?;
            }
            p.ty(slot(f::TYPE, Ctx::Type), true)?;
            Ok(())
        })?;
        self.close(n);
        Ok(())
    }

    fn enum_variant_list(&mut self) -> R<()> {
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        let n = self.open(K::EnumVariantList, f::BODY, C_NONE);
        self.list(K::EnumVariantList, T::RBrace, false, true, |p| {
            p.outer_attributes()?;
            let v = p.open(K::EnumVariant, f::NONE, C_NONE);
            if p.is_word(w::PUB) {
                p.visibility()?;
            }
            p.value_name()?;
            match p.peek() {
                T::LBrace => p.field_declaration_list(slot(f::BODY, Ctx::Bare))?,
                T::LParen => p.ordered_field_declaration_list(slot(f::BODY, Ctx::Bare))?,
                _ => {}
            }
            if p.peek() == T::Eq {
                p.tk(K::EnumVariant, K::TEq, f::NONE)?;
                p.expr(slot(f::VALUE, Ctx::Expr), 0)?;
            }
            p.close(v);
            Ok(())
        })?;
        self.close(n);
        Ok(())
    }

    /// `unsafe? impl<T> !Trait for Type where … { … }`.
    fn impl_item(&mut self) -> R<()> {
        if self.is_word(w::UNSAFE) {
            self.tk(K::ImplItem, K::TUnsafe, f::NONE)?;
        }
        self.tk(K::ImplItem, K::TImpl, f::NONE)?;
        // `impl <T>`, but not `impl <T as Tr>::A` (a qualified type).
        if matches!(self.peek(), T::Lt) && !self.qualified_ahead() {
            self.type_parameters()?;
        }
        if self.peek() == T::Bang && !matches!(self.peek_at(1).kind, T::LBrace | T::Semi) && !self.is_word_at(1, w::WHERE) {
            // `impl !Trait for T`; `impl ! {}` is the never type's.
            self.tk(K::ImplItem, K::TBang, f::NONE)?;
        }
        let t = self.ty(slot(f::TYPE, Ctx::Type), true)?;
        if self.is_word(w::FOR) {
            // The type read was the trait.
            self.rekey(t.at, slot(f::TRAIT, Ctx::Bare));
            self.tk(K::ImplItem, K::TFor, f::NONE)?;
            self.ty(slot(f::TYPE, Ctx::Type), true)?;
        }
        self.where_opt()?;
        if self.peek() == T::Semi {
            self.tk(K::ImplItem, K::TSemi, f::NONE)?;
        } else {
            self.declaration_list()?;
        }
        Ok(())
    }

    /// Whether `<` starts a qualified type (`<T as Tr>::A`): type parameters hold no `as`, so
    /// `impl<T> ::a::B` is parameters and a path.
    fn qualified_ahead(&mut self) -> bool {
        match self.angle_end(0) {
            Some(k) => self.peek_at(k).kind == T::ColonColon && (1..k).any(|j| self.is_word_at(j, w::AS)),
            None => false,
        }
    }

    /// `type Name<T> where … = Type where …;`, or an associated type: `type Name: Bounds;`
    /// (`n`, open as a `type_item`).
    fn type_item(&mut self, n: N) -> R<()> {
        self.tk(K::TypeItem, K::TType, f::NONE)?;
        self.type_name()?;
        self.type_parameters_opt()?;
        if self.peek() == T::Colon || self.peek() == T::Semi {
            self.rekind(n.at, K::AssociatedType, BARE);
            if self.peek() == T::Colon {
                self.trait_bounds(f::BOUNDS)?;
            }
            self.where_opt()?;
            if self.peek() == T::Eq {
                return self.refuse();
            }
            return self.expect_tk(K::AssociatedType, T::Semi, K::TSemi);
        }
        self.where_opt()?;
        if self.peek() == T::Semi {
            self.rekind(n.at, K::AssociatedType, BARE);
            return self.tk(K::AssociatedType, K::TSemi, f::NONE);
        }
        self.expect_tk(K::TypeItem, T::Eq, K::TEq)?;
        self.ty(slot(f::TYPE, Ctx::Type), true)?;
        self.where_opt()?;
        self.expect_tk(K::TypeItem, T::Semi, K::TSemi)
    }

    // ---- `let`.

    fn let_declaration(&mut self) -> R<()> {
        let n = self.open(K::LetDeclaration, f::NONE, C_NONE);
        self.tk(K::LetDeclaration, K::TLet, f::NONE)?;
        if self.is_word(w::MUT) && !matches!(self.peek_at(1).kind, T::Colon | T::Eq | T::Semi) {
            self.leaf(K::MutableSpecifier, f::NONE, C_NONE)?;
        }
        self.pattern(slot(f::PATTERN, Ctx::Pattern), K::LetDeclaration, true)?;
        if self.peek() == T::Colon {
            self.tk(K::LetDeclaration, K::TColon, f::NONE)?;
            self.ty(slot(f::TYPE, Ctx::Type), true)?;
        }
        if self.peek() == T::Eq {
            self.tk(K::LetDeclaration, K::TEq, f::NONE)?;
            self.expr(slot(f::VALUE, Ctx::Expr), 0)?;
            if self.is_word(w::ELSE) {
                self.tk(K::LetDeclaration, K::TElse, f::NONE)?;
                self.block(slot(f::ALTERNATIVE, Ctx::Bare))?;
            }
        }
        self.expect_tk(K::LetDeclaration, T::Semi, K::TSemi)?;
        self.close(n);
        Ok(())
    }

    // ---- `use`.

    /// A use clause: a path, `path as alias`, `path::{…}`, `{…}`, `path::*`.
    fn use_clause(&mut self, s: Slot) -> R<()> {
        match self.peek() {
            T::LBrace => return self.use_list(s),
            T::Star => {
                let n = self.open_in(K::UseWildcard, s);
                self.leaf(K::TStar, f::NONE, C_NONE)?;
                self.close(n);
                return Ok(());
            }
            T::ColonColon => {
                // `::{…}`, `::*`, `::a`.
                match self.peek_at(1).kind {
                    T::LBrace => {
                        let n = self.open_in(K::ScopedUseList, s);
                        self.tk(K::ScopedUseList, K::TColonColon, f::NONE)?;
                        self.use_list(slot(f::LIST, Ctx::Bare))?;
                        self.close(n);
                        return Ok(());
                    }
                    T::Star => {
                        let n = self.open_in(K::UseWildcard, s);
                        self.leaf(K::TColonColon, f::NONE, C_NONE)?;
                        self.leaf(K::TStar, f::NONE, C_NONE)?;
                        self.close(n);
                        return Ok(());
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        // A path, read segment by segment: it ends at a `{` (a scoped use list), a `*` (a
        // wildcard), an `as`, or the clause's end.
        let mut n = if self.peek() == T::ColonColon {
            let n = self.open_in(K::ScopedIdentifier, s);
            self.tk(K::ScopedIdentifier, K::TColonColon, f::NONE)?;
            self.identifier(slot(f::NAME, Ctx::Bare))?;
            self.close(n);
            n
        } else {
            self.path_start(s)?
        };
        loop {
            if self.peek() != T::ColonColon {
                break;
            }
            match self.peek_at(1).kind {
                T::LBrace => {
                    let l = self.wrap(n, K::ScopedUseList, s, slot(f::PATH, Ctx::Bare));
                    self.tk(K::ScopedUseList, K::TColonColon, f::NONE)?;
                    self.use_list(slot(f::LIST, Ctx::Bare))?;
                    self.close(l);
                    return Ok(());
                }
                T::Star => {
                    let l = self.wrap(n, K::UseWildcard, s, BARE);
                    self.leaf(K::TColonColon, f::NONE, C_NONE)?;
                    self.leaf(K::TStar, f::NONE, C_NONE)?;
                    self.close(l);
                    return Ok(());
                }
                _ => n = self.scoped_segment(n, s)?,
            }
        }
        if self.is_word(w::AS) {
            let a = self.wrap(n, K::UseAsClause, s, slot(f::PATH, Ctx::Bare));
            self.tk(K::UseAsClause, K::TAs, f::NONE)?;
            self.identifier(slot(f::ALIAS, Ctx::Bare))?;
            self.close(a);
        }
        Ok(())
    }

    fn use_list(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::UseList, s);
        self.list(K::UseList, T::RBrace, false, true, |p| p.use_clause(BARE))?;
        self.close(n);
        Ok(())
    }

    // ---- `macro_rules!`.

    /// `macro_rules! name { (…) => {…}; … }`, or with `( … );`, `[ … ];`. The opener is stored (three
    /// brackets tie); a `;` between rules is implied, one after the last is stored with the closer.
    fn macro_definition(&mut self) -> R<()> {
        let n = self.open(K::MacroDefinition, f::NONE, C_NONE);
        // `macro_rules!` is one token.
        self.bump()?;
        self.bump()?;
        self.value_name()?;
        let (close, close_k) = match self.peek() {
            T::LBrace => (T::RBrace, K::TRBrace),
            T::LParen => (T::RParen, K::TRParen),
            T::LBracket => (T::RBracket, K::TRBracket),
            _ => return self.refuse(),
        };
        let brace = close == T::RBrace;
        if self.peek_at(1).kind == close {
            // No rules: `{}` is the one shortest word; `();` is not.
            self.tk_punct(K::MacroDefinition, !brace)?;
            self.tk_punct(K::MacroDefinition, !brace)?;
            if !brace {
                if self.peek() != T::Semi {
                    return self.refuse();
                }
                self.leaf(K::TSemi, f::NONE, C_NONE)?;
            }
            self.close(n);
            return Ok(());
        }
        self.tk_punct(K::MacroDefinition, true)?;
        loop {
            let r = self.open(K::MacroRule, f::NONE, C_NONE);
            self.token_tree(tt::Tt::Pattern, f::LEFT)?;
            self.expect_tk(K::MacroRule, T::FatArrow, K::TFatArrow)?;
            self.token_tree(tt::Tt::Tree, f::RIGHT)?;
            self.close(r);
            match self.peek() {
                T::Semi if self.peek_at(1).kind == close => {
                    self.leaf(K::TSemi, f::NONE, C_NONE)?;
                    self.leaf(close_k, f::NONE, C_NONE)?;
                    if !brace {
                        if self.peek() != T::Semi {
                            return self.refuse();
                        }
                        self.leaf(K::TSemi, f::NONE, C_NONE)?;
                    }
                    break;
                }
                T::Semi => self.tk(K::MacroDefinition, K::TSemi, f::NONE)?,
                k if k == close => {
                    self.tk(K::MacroDefinition, close_k, f::NONE)?;
                    if !brace {
                        self.expect_tk(K::MacroDefinition, T::Semi, K::TSemi)?;
                    }
                    break;
                }
                _ => return self.refuse(),
            }
        }
        self.close(n);
        Ok(())
    }
}
