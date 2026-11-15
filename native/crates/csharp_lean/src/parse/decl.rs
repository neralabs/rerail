//! Declarations: the file's items, `using`s and namespaces, type declarations and their members,
//! attributes, modifiers, parameters and type parameters.

use super::stmt::Items;
use super::*;

/// What a member starts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Member {
    Type(K),
    Namespace,
    Using,
    Event,
    Destructor,
    Conversion,
    Constructor,
    Operator,
    Indexer,
    Method,
    Property,
    Field,
}

/// A modifier's word: `modifier`'s choice.
fn is_modifier_word(code: u8) -> bool {
    matches!(
        code,
        w::ABSTRACT
            | w::ASYNC
            | w::CONST
            | w::EXTERN
            | w::FILE
            | w::FIXED
            | w::INTERNAL
            | w::NEW
            | w::OVERRIDE
            | w::PARTIAL
            | w::PRIVATE
            | w::PROTECTED
            | w::PUBLIC
            | w::READONLY
            | w::REQUIRED
            | w::SEALED
            | w::STATIC
            | w::UNSAFE
            | w::VIRTUAL
            | w::VOLATILE
    )
}

impl<'a> P<'a> {
    /// Whether token `k` is a modifier: a modifier's word, a contextual one (`async`, `partial`,
    /// `file`, `required`) only before a word or `(`.
    pub(super) fn modifier_at(&mut self, k: usize) -> bool {
        let t = self.peek_at(k);
        if t.kind != T::Name || !is_modifier_word(t.word) {
            return false;
        }
        if matches!(t.word, w::ASYNC | w::PARTIAL | w::FILE | w::REQUIRED) {
            let n = self.peek_at(k + 1);
            return n.kind == T::Name && !matches!(n.word, w::AS | w::IS) || n.kind == T::LParen && t.word == w::ASYNC;
        }
        if t.word == w::NEW && self.in_statement {
            return false;
        }
        if t.word == w::FIXED && self.kind_at(k + 1) == T::LParen {
            return false;
        }
        true
    }

    /// `{modifier}`: each a `modifier` holding its word.
    pub(super) fn modifiers(&mut self) -> R<()> {
        while self.modifier_at(0) {
            let n = self.open(K::Modifier, f::NONE, C_NONE)?;
            let k = keyword(self.tok().word).expect("a modifier");
            self.leaf(k, f::NONE, C_NONE)?;
            self.close(n);
        }
        Ok(())
    }

    // ---- The file.

    pub(super) fn toplevel(&mut self) -> R<()> {
        self.toplevel_item(false)
    }

    /// One of the file's items; in an `#if` (`in_if`), a statement is the item itself.
    pub(super) fn toplevel_item(&mut self, in_if: bool) -> R<()> {
        let t = self.tok();
        match t.kind {
            T::PpIf if self.pp_attributes_end(0).is_none() => {
                self.preproc_if(BARE, Items::TopLevel)?;
                return Ok(());
            }
            T::LBracket if (self.is_word_at(1, w::ASSEMBLY) || self.is_word_at(1, w::MODULE)) && self.kind_at(2) == T::Colon => {
                return self.global_attribute();
            }
            T::Name => match t.word {
                w::EXTERN if self.is_word_at(1, w::ALIAS) => {
                    let n = self.open(K::ExternAliasDirective, f::NONE, C_NONE)?;
                    self.tk(K::ExternAliasDirective, K::TExtern)?;
                    self.tk(K::ExternAliasDirective, K::TAlias)?;
                    self.ident(slot(f::NAME, Ctx::Bare))?;
                    self.punct_tk(K::ExternAliasDirective, T::Semi, false)?;
                    self.close(n);
                    return Ok(());
                }
                w::USING if self.using_directive_ahead(0) => {
                    self.using_directive(BARE)?;
                    return Ok(());
                }
                w::GLOBAL if self.is_word_at(1, w::USING) => {
                    self.using_directive(BARE)?;
                    return Ok(());
                }
                w::NAMESPACE => {
                    self.namespace(BARE)?;
                    return Ok(());
                }
                _ => {}
            },
            _ => {}
        }
        if let Some(Member::Type(kind)) = self.member_ahead() {
            self.type_declaration(kind, slot(f::NONE, Ctx::TypeDecl))?;
            return Ok(());
        }
        let saved = std::mem::replace(&mut self.in_statement, true);
        let r = if in_if {
            self.statement(STMT).map(|_| ())
        } else {
            let n = self.open(K::GlobalStatement, f::NONE, C_NONE)?;
            self.statement(STMT)?;
            self.close(n);
            Ok(())
        };
        self.in_statement = saved;
        r
    }

    /// Whether `using` at `k` is a directive's (not a statement's `using (` or `using var`).
    fn using_directive_ahead(&mut self, k: usize) -> bool {
        if self.kind_at(k + 1) == T::LParen {
            return false;
        }
        if self.is_word_at(k + 1, w::STATIC) || self.is_word_at(k + 1, w::UNSAFE) {
            return true;
        }
        // `using A = B;`, `using A.B;`, but not `using var x = …;` or `using T x = …;`.
        if !self.is_ident_at(k + 1) {
            return false;
        }
        if self.kind_at(k + 2) == T::Eq {
            return true;
        }
        match self.scan_type(k + 1) {
            Some(e) => self.kind_at(e) == T::Semi,
            None => false,
        }
    }

    /// `using_directive`: `global`, `using`, `static`/`unsafe`, an alias `A =` or a name, `;`.
    fn using_directive(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::UsingDirective, s)?;
        if self.is_word(w::GLOBAL) {
            self.tk(K::UsingDirective, K::TGlobal)?;
        }
        self.kw(K::UsingDirective, w::USING)?;
        while self.is_word(w::STATIC) || self.is_word(w::UNSAFE) {
            let k = keyword(self.tok().word).expect("a keyword");
            self.tk(K::UsingDirective, k)?;
        }
        if self.is_ident() && self.kind_at(1) == T::Eq {
            self.ident(slot(f::NAME, Ctx::Bare))?;
            self.tk(K::UsingDirective, K::TEq)?;
            self.ty(TYPE)?;
        } else {
            self.name(BARE)?;
        }
        self.punct_tk(K::UsingDirective, T::Semi, false)?;
        self.close(n);
        Ok(())
    }

    /// `namespace N { … }` or `namespace N;` (its items then the file's).
    fn namespace(&mut self, s: Slot) -> R<()> {
        let e = self.scan_type_base(1);
        if e.is_some_and(|e| self.kind_at(e) == T::Semi) {
            let n = self.open(K::FileScopedNamespaceDeclaration, f::NONE, C_NONE)?;
            self.tk(K::FileScopedNamespaceDeclaration, K::TNamespace)?;
            self.name(slot(f::NAME, Ctx::Bare))?;
            self.punct_tk(K::FileScopedNamespaceDeclaration, T::Semi, false)?;
            self.close(n);
            return Ok(());
        }
        let n = self.open_in(K::NamespaceDeclaration, s)?;
        self.tk(K::NamespaceDeclaration, K::TNamespace)?;
        self.name(slot(f::NAME, Ctx::Bare))?;
        self.declaration_list()?;
        if self.peek() == T::Semi {
            self.leaf(K::TSemi, f::NONE, C_NONE)?;
        }
        self.close(n);
        Ok(())
    }

    /// `global_attribute`: `[assembly: A, B]`.
    fn global_attribute(&mut self) -> R<()> {
        let n = self.open(K::GlobalAttribute, f::NONE, C_NONE)?;
        self.leaf(K::TLBracket, f::NONE, C_NONE)?;
        let k = keyword(self.tok().word).expect("a target");
        self.leaf(k, f::NONE, C_NONE)?;
        self.leaf(K::TColon, f::NONE, C_NONE)?;
        self.attributes_to_bracket(K::GlobalAttribute)?;
        self.close(n);
        Ok(())
    }

    /// Attributes between `,`s to the `]` (a trailing `,` stores itself and the `]`).
    fn attributes_to_bracket(&mut self, parent: K) -> R<()> {
        loop {
            self.attribute()?;
            match self.peek() {
                T::Comma if self.kind_at(1) == T::RBracket => {
                    self.leaf(K::TComma, f::NONE, C_NONE)?;
                    self.leaf(K::TRBracket, f::NONE, C_NONE)?;
                    return Ok(());
                }
                T::Comma => self.tk(parent, K::TComma)?,
                T::RBracket => return self.punct_tk(parent, T::RBracket, false),
                _ => return self.refuse(),
            }
        }
    }

    /// `attribute`: a name and its arguments.
    fn attribute(&mut self) -> R<()> {
        let n = self.open(K::Attribute, f::NONE, C_NONE)?;
        self.name(slot(f::NAME, Ctx::Bare))?;
        if self.peek() == T::LParen {
            let a = self.open(K::AttributeArgumentList, f::NONE, C_NONE)?;
            self.list(K::AttributeArgumentList, T::RParen, true, false, |p| {
                let n = p.open(K::AttributeArgument, f::NONE, C_NONE)?;
                if p.is_ident() && matches!(p.kind_at(1), T::Eq | T::Colon) {
                    p.ident(slot(f::NAME, Ctx::Bare))?;
                    let k = punct(p.peek()).expect("a punctuation");
                    p.leaf(k, f::NONE, C_NONE)?;
                }
                p.expr(EXPR)?;
                p.close(n);
                Ok(())
            })?;
            self.close(a);
        }
        self.close(n);
        Ok(())
    }

    /// `{_attribute_list}`: each `[` a target and attributes `]`.
    pub(super) fn attribute_lists(&mut self) -> R<()> {
        loop {
            if self.peek() == T::LBracket {
                self.attribute_list()?;
            } else if self.pp_attributes_end(0).is_some() {
                self.pp_attributes()?;
            } else {
                return Ok(());
            }
        }
    }

    fn attribute_list(&mut self) -> R<()> {
        let n = self.open(K::AttributeList, f::NONE, C_NONE)?;
        self.tk(K::AttributeList, K::TLBracket)?;
        let t = self.tok();
        if t.kind == T::Name && self.kind_at(1) == T::Colon {
            let k = match t.word {
                w::FIELD | w::EVENT | w::METHOD | w::PARAM | w::PROPERTY | w::RETURN | w::TYPE | w::TYPEVAR => keyword(t.word).expect("a target"),
                _ => return self.refuse(),
            };
            let s = self.open(K::AttributeTargetSpecifier, f::NONE, C_NONE)?;
            self.leaf(k, f::NONE, C_NONE)?;
            self.leaf(K::TColon, f::NONE, C_NONE)?;
            self.close(s);
        }
        self.attributes_to_bracket(K::AttributeList)?;
        self.close(n);
        Ok(())
    }

    /// Where attribute lists starting `k` tokens ahead end.
    pub(super) fn skip_attributes(&mut self, mut k: usize) -> Option<usize> {
        loop {
            if self.kind_at(k) == T::LBracket {
                k = self.matching(k)? + 1;
            } else if let Some(e) = self.pp_attributes_end(k) {
                k = e;
            } else {
                return Some(k);
            }
        }
    }

    /// Where an `#if` at `k` that holds attribute lists alone (one at most in each branch, one at
    /// least in all) ends: past its `#endif`. `None` where none starts there.
    pub(super) fn pp_attributes_end(&mut self, k: usize) -> Option<usize> {
        if self.kind_at(k) != T::PpIf {
            return None;
        }
        let mut j = self.pp_line_end(k);
        let mut lists = 0;
        loop {
            if self.kind_at(j) == T::LBracket {
                // A global attribute (`[assembly: …]`) is the file's item, not a declaration's.
                if (self.is_word_at(j + 1, w::ASSEMBLY) || self.is_word_at(j + 1, w::MODULE)) && self.kind_at(j + 2) == T::Colon {
                    return None;
                }
                j = self.matching(j)? + 1;
                lists += 1;
            }
            match self.kind_at(j) {
                T::PpEndif => return (lists > 0).then_some(j + 1),
                T::PpElse => j += 1,
                T::PpElif => j = self.pp_line_end(j),
                _ => return None,
            }
        }
    }

    /// The first token past the line of the directive token at `k`.
    fn pp_line_end(&mut self, k: usize) -> usize {
        let eol = self.lx.blocks.eol(self.peek_at(k).start as usize) as u32;
        let mut j = k + 1;
        while self.peek_at(j).start < eol && self.kind_at(j) != T::Eof {
            j += 1;
        }
        j
    }

    /// `preproc_if_in_attribute_list`: `#if`, a condition, an attribute list, `#elif`s and an
    /// `#else` (each with one), `#endif`.
    fn pp_attributes(&mut self) -> R<()> {
        let n = self.open(K::PreprocIfInAttributeList, f::NONE, C_NONE)?;
        self.tk(K::PreprocIfInAttributeList, K::THashIf)?;
        self.preproc_condition()?;
        self.one_attribute_list()?;
        loop {
            match self.peek() {
                T::PpElif => {
                    let a = self.open(K::PreprocElif, f::ALTERNATIVE, C_NONE)?;
                    self.tk(K::PreprocElifInAttributeList, K::THashElif)?;
                    self.preproc_condition()?;
                    self.one_attribute_list()?;
                    self.close(a);
                    // Further branches nest in this one: not read.
                    if matches!(self.peek(), T::PpElif | T::PpElse) {
                        return self.refuse();
                    }
                    break;
                }
                T::PpElse => {
                    let a = self.open(K::PreprocElse, f::ALTERNATIVE, C_NONE)?;
                    // Its tape kind is `preproc_else`, whose `#else` is stored.
                    self.tk(K::PreprocElse, K::THashElse)?;
                    self.one_attribute_list()?;
                    self.close(a);
                    break;
                }
                _ => break,
            }
        }
        if self.peek() != T::PpEndif {
            return self.refuse();
        }
        self.tk(K::PreprocIfInAttributeList, K::THashEndif)?;
        self.close(n);
        Ok(())
    }

    fn one_attribute_list(&mut self) -> R<()> {
        if self.peek() == T::LBracket {
            self.attribute_list()?;
        }
        Ok(())
    }

    // ---- Members.

    /// What the member (or type declaration) next is, past its attributes and modifiers.
    fn member_ahead(&mut self) -> Option<Member> {
        let mut k = self.skip_attributes(0)?;
        while self.modifier_at(k) {
            k += 1;
        }
        let t = self.peek_at(k);
        if t.kind == T::Tilde {
            return Some(Member::Destructor);
        }
        if t.kind != T::Name && t.kind != T::LParen {
            return None;
        }
        match t.word {
            w::CLASS => return Some(Member::Type(K::ClassDeclaration)),
            w::STRUCT => return Some(Member::Type(K::StructDeclaration)),
            w::REF if self.is_word_at(k + 1, w::STRUCT) => return Some(Member::Type(K::StructDeclaration)),
            w::INTERFACE => return Some(Member::Type(K::InterfaceDeclaration)),
            w::ENUM => return Some(Member::Type(K::EnumDeclaration)),
            w::DELEGATE if self.kind_at(k + 1) != T::Star => return Some(Member::Type(K::DelegateDeclaration)),
            w::RECORD if self.is_ident_at(k + 1) || self.is_word_at(k + 1, w::CLASS) || self.is_word_at(k + 1, w::STRUCT) => {
                return Some(Member::Type(K::RecordDeclaration));
            }
            w::NAMESPACE => return Some(Member::Namespace),
            w::USING => return Some(Member::Using),
            w::EVENT => return Some(Member::Event),
            w::IMPLICIT | w::EXPLICIT => return Some(Member::Conversion),
            _ => {}
        }
        if self.is_ident_at(k) && self.kind_at(k + 1) == T::LParen {
            return Some(Member::Constructor);
        }
        let e = self.scan_type(k)?;
        let t = self.peek_at(e);
        if t.kind == T::Name && t.word == w::OPERATOR {
            return Some(Member::Operator);
        }
        if t.kind == T::Name && t.word == w::THIS {
            return Some(Member::Indexer);
        }
        if !self.is_ident_at(e) {
            return None;
        }
        // The member's name: past an explicit interface (`I.F`, `I<T>.F`, `I.this`).
        let mut j = e;
        loop {
            j += 1;
            if self.kind_at(j) == T::Lt {
                match self.scan_type_args(j) {
                    Some(x) if self.kind_at(x) == T::Dot => j = x,
                    Some(x) => {
                        return (self.kind_at(x) == T::LParen).then_some(Member::Method);
                    }
                    None => return None,
                }
            }
            if self.kind_at(j) == T::Dot {
                if self.is_word_at(j + 1, w::THIS) {
                    return Some(Member::Indexer);
                }
                if self.is_word_at(j + 1, w::OPERATOR) {
                    return Some(Member::Operator);
                }
                if !self.is_ident_at(j + 1) {
                    return None;
                }
                j += 1;
                continue;
            }
            break;
        }
        match self.kind_at(j) {
            T::LParen => Some(Member::Method),
            T::LBrace | T::FatArrow => Some(Member::Property),
            T::Eq | T::Semi | T::Comma | T::LBracket => Some(Member::Field),
            _ => None,
        }
    }

    /// `declaration_list`: `{` members `}`.
    fn declaration_list(&mut self) -> R<()> {
        let n = self.open(K::DeclarationList, f::BODY, C_NONE)?;
        self.punct_tk(K::DeclarationList, T::LBrace, false)?;
        self.members()?;
        self.punct_tk(K::DeclarationList, T::RBrace, false)?;
        self.close(n);
        Ok(())
    }

    /// Members to a `}`, an `#elif`, an `#else` or an `#endif`.
    pub(super) fn members(&mut self) -> R<()> {
        loop {
            match self.peek() {
                T::RBrace | T::PpElif | T::PpElse | T::PpEndif | T::Eof => return Ok(()),
                T::PpIf if self.pp_attributes_end(0).is_none() => {
                    self.preproc_if(DECL, Items::Declarations)?;
                }
                _ => self.member()?,
            }
        }
    }

    fn member(&mut self) -> R<()> {
        let saved = std::mem::replace(&mut self.in_statement, false);
        let r = self.member_in();
        self.in_statement = saved;
        r
    }

    fn member_in(&mut self) -> R<()> {
        let Some(m) = self.member_ahead() else { return self.refuse() };
        match m {
            Member::Type(kind) => self.type_declaration(kind, DECL).map(|_| ()),
            Member::Namespace => self.namespace(DECL),
            Member::Using => self.using_directive(DECL),
            Member::Event => self.event(),
            Member::Destructor => {
                let n = self.open_in(K::DestructorDeclaration, DECL)?;
                self.attribute_lists()?;
                if self.is_word(w::EXTERN) {
                    self.tk(K::DestructorDeclaration, K::TExtern)?;
                }
                if self.peek() != T::Tilde {
                    return self.refuse();
                }
                self.leaf(K::TTilde, f::NONE, C_NONE)?;
                self.ident(slot(f::NAME, Ctx::Bare))?;
                self.parameter_list(K::ParameterList, f::PARAMETERS)?;
                self.function_body(K::DestructorDeclaration)?;
                self.close(n);
                Ok(())
            }
            Member::Conversion => {
                let n = self.open_in(K::ConversionOperatorDeclaration, DECL)?;
                self.attribute_lists()?;
                self.modifiers()?;
                let k = keyword(self.tok().word).expect("implicit or explicit");
                self.leaf(k, f::NONE, C_NONE)?;
                loop {
                    // An interface stands before `operator` (`implicit I.operator T`), the type after.
                    if self.is_word(w::OPERATOR) || self.is_word(w::CHECKED) {
                        let operator = self.is_word(w::OPERATOR);
                        let k = keyword(self.tok().word).expect("a keyword");
                        self.leaf(k, f::NONE, C_NONE)?;
                        if operator && !self.is_word(w::CHECKED) {
                            break;
                        }
                    } else if self.explicit_interface_ahead() {
                        self.explicit_interface()?;
                    } else {
                        break;
                    }
                }
                self.ty(slot(f::TYPE, Ctx::Type))?;
                self.parameter_list(K::ParameterList, f::PARAMETERS)?;
                self.function_body(K::ConversionOperatorDeclaration)?;
                self.close(n);
                Ok(())
            }
            Member::Constructor => {
                let n = self.open_in(K::ConstructorDeclaration, DECL)?;
                self.attribute_lists()?;
                self.modifiers()?;
                self.ident(slot(f::NAME, Ctx::Bare))?;
                self.parameter_list(K::ParameterList, f::PARAMETERS)?;
                if self.peek() == T::Colon {
                    let c = self.open(K::ConstructorInitializer, f::NONE, C_NONE)?;
                    self.leaf(K::TColon, f::NONE, C_NONE)?;
                    if !(self.is_word(w::BASE) || self.is_word(w::THIS)) {
                        return self.refuse();
                    }
                    let k = keyword(self.tok().word).expect("base or this");
                    self.leaf(k, f::NONE, C_NONE)?;
                    self.arguments(f::NONE)?;
                    self.close(c);
                }
                self.function_body(K::ConstructorDeclaration)?;
                self.close(n);
                Ok(())
            }
            Member::Operator => {
                let n = self.open_in(K::OperatorDeclaration, DECL)?;
                self.attribute_lists()?;
                self.modifiers()?;
                self.ty(slot(f::TYPE, Ctx::Type))?;
                if self.explicit_interface_ahead() {
                    self.explicit_interface()?;
                }
                self.kw(K::OperatorDeclaration, w::OPERATOR)?;
                if self.is_word(w::CHECKED) {
                    self.leaf(K::TChecked, f::NONE, C_NONE)?;
                }
                let t = self.tok();
                let op = match t.kind {
                    T::Name if t.word == w::TRUE => K::TTrue,
                    T::Name if t.word == w::FALSE => K::TFalse,
                    T::Bang | T::Tilde | T::PlusPlus | T::MinusMinus | T::Plus | T::Minus | T::Star | T::Slash | T::Percent | T::Caret | T::Pipe | T::Amp | T::LtLt | T::GtGt | T::GtGtGt | T::EqEq | T::Ne | T::Gt | T::Lt | T::Ge | T::Le => punct(t.kind).expect("an operator"),
                    _ => return self.refuse(),
                };
                self.leaf(op, f::OPERATOR, C_NONE)?;
                self.parameter_list(K::ParameterList, f::PARAMETERS)?;
                self.function_body(K::OperatorDeclaration)?;
                self.close(n);
                Ok(())
            }
            Member::Indexer => {
                let n = self.open_in(K::IndexerDeclaration, DECL)?;
                self.attribute_lists()?;
                self.modifiers()?;
                self.ty(slot(f::TYPE, Ctx::Type))?;
                if self.explicit_interface_ahead() {
                    self.explicit_interface()?;
                }
                self.kw(K::IndexerDeclaration, w::THIS)?;
                self.parameter_list(K::BracketedParameterList, f::PARAMETERS)?;
                self.accessors_or_arrow(K::IndexerDeclaration)?;
                self.close(n);
                Ok(())
            }
            Member::Method => {
                let n = self.open_in(K::MethodDeclaration, DECL)?;
                self.attribute_lists()?;
                self.modifiers()?;
                self.ty(slot(f::RETURNS, Ctx::Type))?;
                if self.explicit_interface_ahead() {
                    self.explicit_interface()?;
                }
                self.ident(slot(f::NAME, Ctx::Bare))?;
                if self.peek() == T::Lt {
                    self.type_parameter_list(f::TYPE_PARAMETERS)?;
                }
                self.parameter_list(K::ParameterList, f::PARAMETERS)?;
                self.constraints()?;
                self.function_body(K::MethodDeclaration)?;
                self.close(n);
                Ok(())
            }
            Member::Property => {
                let n = self.open_in(K::PropertyDeclaration, DECL)?;
                self.attribute_lists()?;
                self.modifiers()?;
                self.ty(slot(f::TYPE, Ctx::Type))?;
                if self.explicit_interface_ahead() {
                    self.explicit_interface()?;
                }
                self.ident(slot(f::NAME, Ctx::Bare))?;
                if self.peek() == T::LBrace {
                    self.accessor_list()?;
                    if self.peek() == T::Eq {
                        self.leaf(K::TEq, f::NONE, C_NONE)?;
                        self.expr(slot(f::VALUE, Ctx::Expr))?;
                        self.punct_tk(K::PropertyDeclaration, T::Semi, false)?;
                    }
                } else {
                    self.arrow_clause(f::VALUE)?;
                    self.punct_tk(K::PropertyDeclaration, T::Semi, false)?;
                }
                self.close(n);
                Ok(())
            }
            Member::Field => {
                let n = self.open_in(K::FieldDeclaration, DECL)?;
                self.attribute_lists()?;
                self.modifiers()?;
                self.variable_declaration(f::NONE)?;
                self.punct_tk(K::FieldDeclaration, T::Semi, false)?;
                self.close(n);
                Ok(())
            }
        }
    }

    /// An event: a field-like `event T a, b;` or one with accessors.
    fn event(&mut self) -> R<()> {
        let mut k = self.skip_attributes(0).ok_or_else(|| Refusal::Parse(self.tok().start))?;
        while self.modifier_at(k) {
            k += 1;
        }
        let e = self.scan_type(k + 1).ok_or_else(|| Refusal::Parse(self.tok().start))?;
        let mut j = e;
        while self.is_ident_at(j) && self.kind_at(j + 1) == T::Dot {
            j += 2;
        }
        let accessors = self.is_ident_at(j) && self.kind_at(j + 1) == T::LBrace;
        if !accessors {
            let n = self.open_in(K::EventFieldDeclaration, DECL)?;
            self.attribute_lists()?;
            self.modifiers()?;
            self.kw(K::EventFieldDeclaration, w::EVENT)?;
            self.variable_declaration(f::NONE)?;
            self.punct_tk(K::EventFieldDeclaration, T::Semi, false)?;
            self.close(n);
            return Ok(());
        }
        let n = self.open_in(K::EventDeclaration, DECL)?;
        self.attribute_lists()?;
        self.modifiers()?;
        self.kw(K::EventDeclaration, w::EVENT)?;
        self.ty(slot(f::TYPE, Ctx::Type))?;
        if self.explicit_interface_ahead() {
            self.explicit_interface()?;
        }
        self.ident(slot(f::NAME, Ctx::Bare))?;
        self.accessor_list()?;
        self.close(n);
        Ok(())
    }

    /// Whether an explicit interface (`I.`, `A.I<T>.`) comes before the member's name.
    fn explicit_interface_ahead(&mut self) -> bool {
        if !self.is_ident() {
            return false;
        }
        let mut k = 1;
        if self.kind_at(k) == T::Lt {
            match self.scan_type_args(k) {
                Some(e) => k = e,
                None => return false,
            }
        }
        self.kind_at(k) == T::Dot
    }

    /// `explicit_interface_specifier`: a name and its last `.`.
    fn explicit_interface(&mut self) -> R<()> {
        // The name runs to the last `.` before the member's name (`this`, `operator` or a name and
        // what follows it).
        let mut k = 0;
        let mut last_dot = None;
        loop {
            if !self.is_ident_at(k) {
                break;
            }
            k += 1;
            if self.kind_at(k) == T::Lt {
                match self.scan_type_args(k) {
                    Some(e) => k = e,
                    None => break,
                }
            }
            if self.kind_at(k) != T::Dot {
                break;
            }
            last_dot = Some(k);
            k += 1;
        }
        let Some(dot) = last_dot else { return self.refuse() };
        let stop = self.i + dot;
        let n = self.open(K::ExplicitInterfaceSpecifier, f::NONE, C_NONE)?;
        let mut name = self.simple_name(BARE, false)?;
        while self.i < stop {
            name = self.wrap(name, K::QualifiedName, BARE, slot(f::QUALIFIER, Ctx::Bare));
            self.tk(K::QualifiedName, K::TDot)?;
            self.simple_name(slot(f::NAME, Ctx::Bare), false)?;
            self.close(name);
        }
        self.tk(K::ExplicitInterfaceSpecifier, K::TDot)?;
        self.close(n);
        Ok(())
    }

    /// An accessor list, or `=> value ;` (an indexer's).
    fn accessors_or_arrow(&mut self, parent: K) -> R<()> {
        if self.peek() == T::LBrace {
            self.accessor_list()
        } else {
            self.arrow_clause(f::VALUE)?;
            self.punct_tk(parent, T::Semi, false)
        }
    }

    /// `accessor_list`: `{` accessors `}`.
    fn accessor_list(&mut self) -> R<()> {
        let n = self.open(K::AccessorList, f::ACCESSORS, C_NONE)?;
        self.punct_tk(K::AccessorList, T::LBrace, false)?;
        while self.peek() != T::RBrace {
            let a = self.open(K::AccessorDeclaration, f::NONE, C_NONE)?;
            self.attribute_lists()?;
            self.modifiers()?;
            let t = self.tok();
            let word = t.kind == T::Name && matches!(t.word, w::GET | w::SET | w::ADD | w::REMOVE | w::INIT);
            if word {
                self.leaf(keyword(t.word).expect("an accessor"), f::NAME, C_NONE)?;
            } else {
                self.ident(slot(f::NAME, Ctx::Bare))?;
            }
            if self.peek() == T::Semi {
                // `get;`: a gap of tokens alone, no implied word (`set;` is as short).
                self.punct_tk(K::AccessorDeclaration, T::Semi, word)?;
            } else {
                self.function_body(K::AccessorDeclaration)?;
            }
            self.close(a);
        }
        self.punct_tk(K::AccessorList, T::RBrace, false)?;
        self.close(n);
        Ok(())
    }

    /// `_function_body`: a block, `=> expression ;`, or `;`.
    pub(super) fn function_body(&mut self, parent: K) -> R<()> {
        match self.peek() {
            T::LBrace => {
                self.block(slot(f::BODY, Ctx::Bare))?;
            }
            T::FatArrow => {
                self.arrow_clause(f::BODY)?;
                self.punct_tk(parent, T::Semi, false)?;
            }
            T::Semi => self.punct_tk(parent, T::Semi, false)?,
            _ => return self.refuse(),
        }
        Ok(())
    }

    /// `arrow_expression_clause`: `=>` and an expression.
    fn arrow_clause(&mut self, field: u8) -> R<()> {
        let n = self.open(K::ArrowExpressionClause, field, C_NONE)?;
        if self.peek() != T::FatArrow {
            return self.refuse();
        }
        self.leaf(K::TFatArrow, f::NONE, C_NONE)?;
        let saved = std::mem::replace(&mut self.in_statement, true);
        let r = self.expr(EXPR);
        self.in_statement = saved;
        r?;
        self.close(n);
        Ok(())
    }

    // ---- Type declarations.

    /// A type declaration of `kind` in slot `s`.
    pub(super) fn type_declaration(&mut self, kind: K, s: Slot) -> R<N> {
        let saved = std::mem::replace(&mut self.in_statement, false);
        let n = self.open_in(kind, s)?;
        self.attribute_lists()?;
        self.modifiers()?;
        match kind {
            K::ClassDeclaration => self.kw(kind, w::CLASS)?,
            K::StructDeclaration => {
                if self.is_word(w::REF) {
                    self.tk(kind, K::TRef)?;
                }
                self.kw(kind, w::STRUCT)?;
            }
            K::InterfaceDeclaration => self.kw(kind, w::INTERFACE)?,
            K::EnumDeclaration => self.kw(kind, w::ENUM)?,
            K::RecordDeclaration => {
                self.kw(kind, w::RECORD)?;
                if self.is_word(w::CLASS) || self.is_word(w::STRUCT) {
                    let k = keyword(self.tok().word).expect("a keyword");
                    self.tk(kind, k)?;
                }
            }
            K::DelegateDeclaration => {
                self.kw(kind, w::DELEGATE)?;
                self.ty(slot(f::TYPE, Ctx::Type))?;
                self.ident(slot(f::NAME, Ctx::Bare))?;
                if self.peek() == T::Lt {
                    self.type_parameter_list(f::TYPE_PARAMETERS)?;
                }
                self.parameter_list(K::ParameterList, f::PARAMETERS)?;
                self.constraints()?;
                self.punct_tk(kind, T::Semi, false)?;
                self.close(n);
                self.in_statement = saved;
                return Ok(n);
            }
            _ => unreachable!("a type declaration"),
        }
        self.ident(slot(f::NAME, Ctx::Bare))?;
        if kind == K::EnumDeclaration {
            if self.peek() == T::Colon {
                self.base_list(false)?;
            }
            if self.peek() == T::Semi {
                self.punct_tk(kind, T::Semi, false)?;
            } else {
                self.enum_body()?;
                if self.peek() == T::Semi {
                    self.punct_tk(kind, T::Semi, false)?;
                }
            }
            self.close(n);
            self.in_statement = saved;
            return Ok(n);
        }
        loop {
            match self.peek() {
                T::Lt => self.type_parameter_list(if kind == K::InterfaceDeclaration { f::TYPE_PARAMETERS } else { f::NONE })?,
                T::LParen if kind != K::InterfaceDeclaration => {
                    self.parameter_list(K::ParameterList, f::NONE)?;
                }
                T::Colon => self.base_list(kind == K::RecordDeclaration)?,
                _ => break,
            }
        }
        self.constraints()?;
        if self.peek() == T::Semi {
            self.punct_tk(kind, T::Semi, false)?;
        } else {
            self.declaration_list()?;
            if self.peek() == T::Semi {
                self.punct_tk(kind, T::Semi, false)?;
            }
        }
        self.close(n);
        self.in_statement = saved;
        Ok(n)
    }

    /// `base_list`: `:` types (with arguments) between `,`s; a record's first with arguments is
    /// its `primary_constructor_base_type`.
    /// Whether tokens `from..to` are a name alone (dotted, generic or alias-qualified; no suffix).
    fn name_type_at(&mut self, from: usize, to: usize) -> bool {
        let mut owed = 0;
        self.is_ident_at(from) && self.scan_name(from, &mut owed) == Some(to) && owed == 0
    }

    fn base_list(&mut self, record: bool) -> R<()> {
        let n = self.open(K::BaseList, f::NONE, C_NONE)?;
        self.tk(K::BaseList, K::TColon)?;
        let mut first = true;
        loop {
            let e = self.scan_type(0);
            if record && first && e.is_some_and(|e| self.kind_at(e) == T::LParen) {
                let b = self.open(K::PrimaryConstructorBaseType, f::NONE, C_NONE)?;
                self.name(slot(f::TYPE, Ctx::Bare))?;
                self.arguments(f::NONE)?;
                self.close(b);
            } else {
                // A record's base names are `_name`s to tree-sitter (no `type`).
                if record && e.is_some_and(|e| self.name_type_at(0, e)) {
                    self.name(BARE)?;
                } else {
                    self.ty(TYPE)?;
                }
                if self.peek() == T::LParen {
                    self.arguments(f::NONE)?;
                }
            }
            first = false;
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::BaseList, K::TComma)?;
        }
        self.close(n);
        Ok(())
    }

    /// `enum_member_declaration_list`: `{` members `}`, a trailing `,` storing itself and `}`.
    fn enum_body(&mut self) -> R<()> {
        let n = self.open(K::EnumMemberDeclarationList, f::BODY, C_NONE)?;
        self.list(K::EnumMemberDeclarationList, T::RBrace, true, true, |p| {
            let m = p.open(K::EnumMemberDeclaration, f::NONE, C_NONE)?;
            p.attribute_lists()?;
            p.ident(slot(f::NAME, Ctx::Bare))?;
            if p.peek() == T::Eq {
                p.leaf(K::TEq, f::NONE, C_NONE)?;
                p.expr(slot(f::VALUE, Ctx::Expr))?;
            }
            p.close(m);
            Ok(())
        })?;
        self.close(n);
        Ok(())
    }

    /// `type_parameter_list`: `<` type parameters `>`.
    pub(super) fn type_parameter_list(&mut self, field: u8) -> R<()> {
        let n = self.open(K::TypeParameterList, field, C_NONE)?;
        self.tk(K::TypeParameterList, K::TLt)?;
        loop {
            let p = self.open(K::TypeParameter, f::NONE, C_NONE)?;
            self.attribute_lists()?;
            if (self.is_word(w::IN) || self.is_word(w::OUT)) && self.is_ident_at(1) {
                let k = keyword(self.tok().word).expect("a variance");
                self.leaf(k, f::NONE, C_NONE)?;
            }
            self.ident(slot(f::NAME, Ctx::Bare))?;
            self.close(p);
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::TypeParameterList, K::TComma)?;
        }
        self.close_angle(K::TypeParameterList, false)?;
        self.close(n);
        Ok(())
    }

    /// `{type_parameter_constraints_clause}`: each `where T : constraints`.
    pub(super) fn constraints(&mut self) -> R<()> {
        while self.is_word(w::WHERE) && self.is_ident_at(1) && self.kind_at(2) == T::Colon {
            let n = self.open(K::TypeParameterConstraintsClause, f::NONE, C_NONE)?;
            self.tk(K::TypeParameterConstraintsClause, K::TWhere)?;
            self.ident(BARE)?;
            self.tk(K::TypeParameterConstraintsClause, K::TColon)?;
            loop {
                let c = self.open(K::TypeParameterConstraint, f::NONE, C_NONE)?;
                let t = self.tok();
                match t.kind {
                    T::Name if t.word == w::CLASS => {
                        self.leaf(K::TClass, f::NONE, C_NONE)?;
                        if self.peek() == T::Question {
                            self.leaf(K::TQuestion, f::NONE, C_NONE)?;
                        }
                    }
                    T::Name if matches!(t.word, w::STRUCT | w::NOTNULL | w::UNMANAGED) && (matches!(self.kind_at(1), T::Comma | T::LBrace | T::Semi | T::FatArrow) || self.is_word_at(1, w::WHERE)) || t.word == w::STRUCT => {
                        self.leaf(keyword(t.word).expect("a constraint"), f::NONE, C_NONE)?;
                    }
                    T::Name if t.word == w::NEW && self.kind_at(1) == T::LParen && self.kind_at(2) == T::RParen => {
                        let k = self.open(K::ConstructorConstraint, f::NONE, C_NONE)?;
                        self.tk(K::ConstructorConstraint, K::TNew)?;
                        self.tk(K::ConstructorConstraint, K::TLParen)?;
                        self.tk(K::ConstructorConstraint, K::TRParen)?;
                        self.close(k);
                    }
                    _ => {
                        self.ty(slot(f::TYPE, Ctx::Type))?;
                    }
                }
                self.close(c);
                if self.peek() != T::Comma {
                    break;
                }
                self.tk(K::TypeParameterConstraintsClause, K::TComma)?;
            }
            self.close(n);
        }
        Ok(())
    }

    // ---- Parameters.

    /// A parameter list of `kind` (`parameter_list`, `bracketed_parameter_list`) in `field`.
    pub(super) fn parameter_list(&mut self, kind: K, field: u8) -> R<()> {
        let close = if kind == K::BracketedParameterList { T::RBracket } else { T::RParen };
        let n = self.open(kind, field, C_NONE)?;
        let saved = std::mem::replace(&mut self.in_statement, false);
        let r = self.list(kind, close, true, false, |p| p.parameter(kind));
        self.in_statement = saved;
        r?;
        self.close(n);
        Ok(())
    }

    /// A `parameter` (or a `params` array, its pieces in the list itself).
    fn parameter(&mut self, list: K) -> R<()> {
        let k = self.skip_attributes(0).ok_or_else(|| Refusal::Parse(self.tok().start))?;
        if self.is_word_at(k, w::PARAMS) {
            self.attribute_lists()?;
            self.tk(list, K::TParams)?;
            self.ty(slot(f::TYPE, Ctx::Type))?;
            self.ident(slot(f::NAME, Ctx::Bare))?;
            return Ok(());
        }
        let n = self.open(K::Parameter, f::NONE, C_NONE)?;
        self.attribute_lists()?;
        // The first parameter alone (`(ref` reads as an expression's start too).
        let bare_lambda = std::mem::take(&mut self.bare_lambda);
        loop {
            let word = self.word_at(0);
            if !matches!(word, w::THIS | w::SCOPED | w::REF | w::OUT | w::IN | w::READONLY) {
                break;
            }
            // `scoped` as a name; `scoped` before a name type (a `scoped_type`); `ref` first in a
            // bare lambda's parameter (a `ref_type`), as tree-sitter reads them.
            let next = self.peek_at(1);
            if word == w::SCOPED && (matches!(next.kind, T::Comma | T::RParen | T::Eq) || (next.kind == T::Name && !w::is_reserved(next.word) && !matches!(self.kind_at(2), T::Comma | T::RParen | T::Eq))) {
                break;
            }
            if word == w::REF && bare_lambda {
                break;
            }
            // `(in T x, …) =>` and `(out T x, …) =>`: tree-sitter reads a tuple and fails.
            if matches!(word, w::IN | w::OUT) && bare_lambda {
                return self.refuse();
            }
            self.leaf(K::Modifier, f::NONE, C_NONE)?;
        }
        let typed = !(self.is_ident() && matches!(self.kind_at(1), T::Comma | T::RParen | T::RBracket | T::Eq));
        if typed {
            self.ty(slot(f::TYPE, Ctx::Type))?;
        }
        self.ident(slot(f::NAME, Ctx::Bare))?;
        if self.peek() == T::Eq {
            self.leaf(K::TEq, f::NONE, C_NONE)?;
            self.expr(EXPR)?;
        }
        self.close(n);
        Ok(())
    }
}
