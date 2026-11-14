//! Statements.
//!
//! `_semicolon` is `;` or the automatic one before `?>`, which is no token: a gap that ends in a
//! `;` is never its kind's one shortest word (the gap without it is shorter), so every token of
//! such a gap is stored (`exit();`, `unset($a);`, `declare(ticks=1);`).

use super::*;

impl<'a> P<'a> {
    /// A statement in slot `s`.
    pub(super) fn statement(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let n = self.statement_inner(s);
        self.depth -= 1;
        n
    }

    fn statement_inner(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::Semi => {
                let n = self.open_in(K::EmptyStatement, s)?;
                self.tk(K::EmptyStatement, K::TSemi)?;
                self.close(n);
                return Ok(n);
            }
            T::LBrace => return self.compound(s),
            T::HashLBracket => return self.attributed_statement(s),
            T::Name => {}
            _ => return self.expression_statement(s),
        }
        let next = self.kind_at(1);
        match t.word {
            w::IF => self.if_statement(s),
            w::SWITCH => self.switch_statement(s),
            w::WHILE => self.while_statement(s),
            w::DO => self.do_statement(s),
            w::FOR => self.for_statement(s),
            w::FOREACH => self.foreach_statement(s),
            w::GOTO => {
                let n = self.open_in(K::GotoStatement, s)?;
                self.kw(K::GotoStatement, w::GOTO)?;
                self.name(BARE)?;
                self.semi(K::GotoStatement)?;
                self.close(n);
                Ok(n)
            }
            w::CONTINUE | w::BREAK | w::RETURN => {
                let kind = match t.word {
                    w::CONTINUE => K::ContinueStatement,
                    w::BREAK => K::BreakStatement,
                    _ => K::ReturnStatement,
                };
                let n = self.open_in(kind, s)?;
                self.kw(kind, t.word)?;
                if !matches!(self.peek(), T::Semi | T::CloseTag) {
                    self.expr(EXPR)?;
                }
                self.semi(kind)?;
                self.close(n);
                Ok(n)
            }
            w::TRY => self.try_statement(s),
            w::DECLARE => self.declare_statement(s),
            w::ECHO => {
                let n = self.open_in(K::EchoStatement, s)?;
                self.kw(K::EchoStatement, w::ECHO)?;
                self.expressions(slot(f::NONE, Ctx::Expr))?;
                self.semi(K::EchoStatement)?;
                self.close(n);
                Ok(n)
            }
            w::EXIT => self.exit_statement(s),
            w::UNSET if next == T::LParen && self.is_exact_at(0, w::UNSET) => self.unset_statement(s),
            w::CONST => self.const_declaration(s),
            w::FUNCTION if next == T::Name || (next == T::Amp && self.kind_at(2) == T::Name) => self.function_definition(s),
            w::ABSTRACT | w::FINAL | w::READONLY | w::CLASS if self.class_ahead() => self.class_declaration(s),
            w::INTERFACE if next == T::Name => self.interface_declaration(s),
            w::TRAIT if next == T::Name => self.trait_declaration(s),
            w::ENUM if next == T::Name && matches!(self.kind_at(2), T::LBrace | T::Colon) || (next == T::Name && self.is_word_at(2, w::IMPLEMENTS)) => {
                self.enum_declaration(s)
            }
            w::NAMESPACE if next != T::Backslash => self.namespace_definition(s),
            w::USE => self.use_declaration(s),
            w::GLOBAL => {
                let n = self.open_in(K::GlobalDeclaration, s)?;
                self.kw(K::GlobalDeclaration, w::GLOBAL)?;
                loop {
                    self.simple_variable(BARE)?;
                    if self.peek() != T::Comma {
                        break;
                    }
                    self.tk(K::GlobalDeclaration, K::TComma)?;
                }
                self.semi(K::GlobalDeclaration)?;
                self.close(n);
                Ok(n)
            }
            w::STATIC if next == T::Dollar => {
                let n = self.open_in(K::FunctionStaticDeclaration, s)?;
                self.kw(K::FunctionStaticDeclaration, w::STATIC)?;
                loop {
                    let d = self.open(K::StaticVariableDeclaration, f::NONE, C_NONE)?;
                    self.variable_name(slot(f::NAME, Ctx::Bare))?;
                    if self.peek() == T::Eq {
                        self.tk(K::StaticVariableDeclaration, K::TEq)?;
                        self.expr(slot(f::VALUE, Ctx::Expr))?;
                    }
                    self.close(d);
                    if self.peek() != T::Comma {
                        break;
                    }
                    self.tk(K::FunctionStaticDeclaration, K::TComma)?;
                }
                self.semi(K::FunctionStaticDeclaration)?;
                self.close(n);
                Ok(n)
            }
            _ if next == T::Colon && !w::is_reserved(t.word) => {
                let n = self.open_in(K::NamedLabelStatement, s)?;
                self.name(BARE)?;
                self.tk(K::NamedLabelStatement, K::TColon)?;
                self.close(n);
                Ok(n)
            }
            _ => self.expression_statement(s),
        }
    }

    /// Whether `abstract`, `final`, `readonly` or `class` start a class declaration here: its
    /// modifiers, then `class` and a name.
    fn class_ahead(&mut self) -> bool {
        let mut k = 0;
        while matches!(self.word_at(k), w::ABSTRACT | w::FINAL | w::READONLY) {
            k += 1;
        }
        self.is_word_at(k, w::CLASS) && self.kind_at(k + 1) == T::Name
    }

    pub(super) fn expression_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ExpressionStatement, s)?;
        self.expr(EXPR)?;
        self.semi(K::ExpressionStatement)?;
        self.close(n);
        Ok(n)
    }

    /// `{` statements `}`.
    pub(super) fn compound(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::CompoundStatement, s)?;
        self.expect(K::CompoundStatement, T::LBrace)?;
        while self.peek() != T::RBrace {
            if self.peek() == T::Eof {
                return self.refuse();
            }
            self.statement(STMT)?;
        }
        self.expect(K::CompoundStatement, T::RBrace)?;
        self.close(n);
        Ok(n)
    }

    /// Statements in a colon block (`:` …), to one of the words in `ends`.
    fn colon_block(&mut self, s: Slot, ends: &[u8]) -> R<N> {
        let n = self.open_in(K::ColonBlock, s)?;
        self.expect(K::ColonBlock, T::Colon)?;
        self.statements_until(ends)?;
        self.close(n);
        Ok(n)
    }

    /// Statements up to one of the words `ends` (or the end).
    fn statements_until(&mut self, ends: &[u8]) -> R<()> {
        loop {
            let t = self.tok();
            if t.kind == T::Eof || t.kind == T::RBrace || (t.kind == T::Name && ends.contains(&t.word)) {
                return Ok(());
            }
            self.statement(STMT)?;
        }
    }

    /// `( expression )` in slot `s`.
    pub(super) fn parenthesized(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ParenthesizedExpression, s)?;
        self.expect(K::ParenthesizedExpression, T::LParen)?;
        self.expr(EXPR)?;
        self.expect(K::ParenthesizedExpression, T::RParen)?;
        self.close(n);
        Ok(n)
    }

    fn if_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::IfStatement, s)?;
        self.kw(K::IfStatement, w::IF)?;
        self.parenthesized(slot(f::CONDITION, Ctx::Bare))?;
        if self.peek() == T::Colon {
            self.colon_block(slot(f::BODY, Ctx::Bare), &[w::ELSEIF, w::ELSE, w::ENDIF])?;
            loop {
                if self.is_word(w::ELSEIF) {
                    let c = self.open(K::ElseIfClause, f::ALTERNATIVE, C_NONE)?;
                    self.tk(K::ElseIfClause2, K::TElseif)?;
                    self.parenthesized(slot(f::CONDITION, Ctx::Bare))?;
                    self.colon_block(slot(f::BODY, Ctx::Bare), &[w::ELSEIF, w::ELSE, w::ENDIF])?;
                    self.close(c);
                } else if self.is_word(w::ELSE) {
                    if self.is_word_at(1, w::IF) {
                        return self.refuse();
                    }
                    let c = self.open(K::ElseClause, f::ALTERNATIVE, C_NONE)?;
                    self.tk(K::ElseClause2, K::TElse)?;
                    self.colon_block(slot(f::BODY, Ctx::Bare), &[w::ENDIF])?;
                    self.close(c);
                } else {
                    break;
                }
            }
            self.kw(K::IfStatement, w::ENDIF)?;
            self.semi(K::IfStatement)?;
            self.close(n);
            return Ok(n);
        }
        self.statement(slot(f::BODY, Ctx::Stmt))?;
        loop {
            if self.is_word(w::ELSEIF) {
                let c = self.open(K::ElseIfClause, f::ALTERNATIVE, C_NONE)?;
                self.tk(K::ElseIfClause, K::TElseif)?;
                self.parenthesized(slot(f::CONDITION, Ctx::Bare))?;
                self.statement(slot(f::BODY, Ctx::Stmt))?;
                self.close(c);
            } else if self.is_word(w::ELSE) {
                let c = self.open(K::ElseClause, f::ALTERNATIVE, C_NONE)?;
                self.tk(K::ElseClause, K::TElse)?;
                self.statement(slot(f::BODY, Ctx::Stmt))?;
                self.close(c);
                break;
            } else {
                break;
            }
        }
        self.close(n);
        Ok(n)
    }

    fn switch_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::SwitchStatement, s)?;
        self.kw(K::SwitchStatement, w::SWITCH)?;
        self.parenthesized(slot(f::CONDITION, Ctx::Bare))?;
        let b = self.open(K::SwitchBlock, f::BODY, C_NONE)?;
        let colon = match self.peek() {
            T::LBrace => {
                self.tk(K::SwitchBlock, K::TLBrace)?;
                false
            }
            T::Colon => {
                self.tk(K::SwitchBlock, K::TColon)?;
                true
            }
            _ => return self.refuse(),
        };
        loop {
            if self.is_word(w::CASE) || self.is_word(w::DEFAULT) {
                let case = self.is_word(w::CASE);
                let kind = if case { K::CaseStatement } else { K::DefaultStatement };
                let c = self.open(kind, f::NONE, C_NONE)?;
                if case {
                    self.kw(kind, w::CASE)?;
                    self.expr(slot(f::VALUE, Ctx::Expr))?;
                } else {
                    self.kw(kind, w::DEFAULT)?;
                }
                match self.peek() {
                    T::Colon => self.tk(kind, K::TColon)?,
                    T::Semi => self.tk(kind, K::TSemi)?,
                    _ => return self.refuse(),
                }
                self.statements_until(&[w::CASE, w::DEFAULT, w::ENDSWITCH])?;
                self.close(c);
            } else {
                break;
            }
        }
        if colon {
            self.kw(K::SwitchBlock, w::ENDSWITCH)?;
            self.semi(K::SwitchBlock)?;
        } else {
            self.expect(K::SwitchBlock, T::RBrace)?;
        }
        self.close(b);
        self.close(n);
        Ok(n)
    }

    fn while_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::WhileStatement, s)?;
        self.kw(K::WhileStatement, w::WHILE)?;
        self.parenthesized(slot(f::CONDITION, Ctx::Bare))?;
        if self.peek() == T::Colon {
            self.colon_block(slot(f::BODY, Ctx::Bare), &[w::ENDWHILE])?;
            self.kw(K::WhileStatement, w::ENDWHILE)?;
            self.semi(K::WhileStatement)?;
        } else {
            self.statement(slot(f::BODY, Ctx::Stmt))?;
        }
        self.close(n);
        Ok(n)
    }

    fn do_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::DoStatement, s)?;
        self.kw(K::DoStatement, w::DO)?;
        self.statement(slot(f::BODY, Ctx::Stmt))?;
        self.kw(K::DoStatement, w::WHILE)?;
        self.parenthesized(slot(f::CONDITION, Ctx::Bare))?;
        self.semi(K::DoStatement)?;
        self.close(n);
        Ok(n)
    }

    fn for_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ForStatement, s)?;
        // The gaps: `for (` init `;` cond `;` update `)` body. A gap that ends in the statement's
        // `;` (no body) is stored whole.
        let close = self.matching(1).ok_or_else(|| Refusal::Parse(self.tok().start))?;
        let empty_body = self.kind_at(close + 1) == T::Semi;
        self.kw(K::ForStatement, w::FOR)?;
        let keep = empty_body && self.tail_gap(1);
        self.punct_tk(K::ForStatement, T::LParen, keep)?;
        let fields = [f::INITIALIZE, f::CONDITION, f::UPDATE];
        for (j, field) in fields.into_iter().enumerate() {
            if !matches!(self.peek(), T::Semi | T::RParen) {
                self.expressions(slot(field, Ctx::Expr))?;
            }
            let keep = empty_body && (j == 2 || self.tail_gap(1));
            if j < 2 {
                self.punct_tk(K::ForStatement, T::Semi, keep)?;
            } else {
                self.punct_tk(K::ForStatement, T::RParen, keep)?;
            }
        }
        match self.peek() {
            T::Semi => self.tok_leaf(K::TSemi).map(|_| ())?,
            T::CloseTag => {}
            T::Colon => {
                self.tk(K::ForStatement, K::TColon)?;
                loop {
                    let t = self.tok();
                    if t.kind == T::Eof || (t.kind == T::Name && t.word == w::ENDFOR) {
                        break;
                    }
                    self.statement(slot(f::BODY, Ctx::Stmt))?;
                }
                self.kw(K::ForStatement, w::ENDFOR)?;
                self.semi(K::ForStatement)?;
            }
            _ => {
                self.statement(slot(f::BODY, Ctx::Stmt))?;
            }
        }
        self.close(n);
        Ok(n)
    }

    /// Whether the tokens from `k` ahead are `;`s to a `)`: no named child before the header's end.
    fn tail_gap(&mut self, mut k: usize) -> bool {
        while self.kind_at(k) == T::Semi {
            k += 1;
        }
        self.kind_at(k) == T::RParen
    }

    /// The token index (from here) of the bracket closing the one `k` ahead.
    pub(super) fn matching(&mut self, k: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut j = k;
        loop {
            match self.kind_at(j) {
                T::LParen | T::LBracket | T::LBrace | T::HashLBracket => depth += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(j);
                    }
                }
                T::Eof => return None,
                _ => {}
            }
            j += 1;
        }
    }

    fn foreach_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ForeachStatement, s)?;
        let close = self.matching(1).ok_or_else(|| Refusal::Parse(self.tok().start))?;
        let empty_body = self.kind_at(close + 1) == T::Semi;
        self.kw(K::ForeachStatement, w::FOREACH)?;
        self.expect(K::ForeachStatement, T::LParen)?;
        self.expr(EXPR)?;
        self.kw(K::ForeachStatement, w::AS)?;
        // `k => v` (a `pair`) or a value.
        let pair = {
            let mut k = 0;
            let mut depth = 0usize;
            loop {
                match self.kind_at(k) {
                    T::LParen | T::LBracket | T::LBrace => depth += 1,
                    T::RParen if depth == 0 => break false,
                    T::RParen | T::RBracket | T::RBrace => depth -= 1,
                    T::FatArrow if depth == 0 => break true,
                    T::Eof => break false,
                    _ => {}
                }
                k += 1;
            }
        };
        if pair {
            let p = self.open(K::Pair, f::NONE, C_NONE)?;
            self.expr(EXPR)?;
            self.tk(K::Pair, K::TFatArrow)?;
            self.foreach_value()?;
            self.close(p);
        } else {
            self.foreach_value()?;
        }
        self.punct_tk(K::ForeachStatement, T::RParen, empty_body)?;
        match self.peek() {
            T::Semi => self.tok_leaf(K::TSemi).map(|_| ())?,
            T::CloseTag => {}
            T::Colon => {
                self.colon_block(slot(f::BODY, Ctx::Bare), &[w::ENDFOREACH])?;
                self.kw(K::ForeachStatement, w::ENDFOREACH)?;
                self.semi(K::ForeachStatement)?;
            }
            _ => {
                self.statement(slot(f::BODY, Ctx::Stmt))?;
            }
        }
        self.close(n);
        Ok(n)
    }

    /// `_foreach_value`: `&$v`, a list, or an expression.
    fn foreach_value(&mut self) -> R<()> {
        match self.peek() {
            T::Amp => {
                self.by_ref(BARE)?;
            }
            T::LBracket if self.list_ahead_in_foreach() => {
                self.list_literal(BARE)?;
            }
            _ if self.is_word(w::LIST) && self.kind_at(1) == T::LParen => {
                self.list_literal(BARE)?;
            }
            _ => {
                self.expr(EXPR)?;
            }
        }
        Ok(())
    }

    /// Whether the `[` here is a list (`foreach (… as [$a, $b])`): its `]` is followed by `)`.
    fn list_ahead_in_foreach(&mut self) -> bool {
        self.matching(0).is_some_and(|e| self.kind_at(e + 1) == T::RParen)
    }

    fn try_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::TryStatement, s)?;
        self.kw(K::TryStatement, w::TRY)?;
        self.compound(slot(f::BODY, Ctx::Bare))?;
        let mut any = false;
        loop {
            if self.is_word(w::CATCH) {
                let c = self.open(K::CatchClause, f::NONE, C_NONE)?;
                self.kw(K::CatchClause, w::CATCH)?;
                self.expect(K::CatchClause, T::LParen)?;
                let l = self.open(K::TypeList, f::TYPE, C_NONE)?;
                loop {
                    self.named_type(BARE)?;
                    if self.peek() != T::Pipe {
                        break;
                    }
                    self.tk(K::TypeList, K::TPipe)?;
                }
                self.close(l);
                if self.peek() == T::Dollar {
                    self.variable_name(slot(f::NAME, Ctx::Bare))?;
                }
                self.expect(K::CatchClause, T::RParen)?;
                self.compound(slot(f::BODY, Ctx::Bare))?;
                self.close(c);
            } else if self.is_word(w::FINALLY) {
                let c = self.open(K::FinallyClause, f::NONE, C_NONE)?;
                self.kw(K::FinallyClause, w::FINALLY)?;
                self.compound(slot(f::BODY, Ctx::Bare))?;
                self.close(c);
            } else {
                break;
            }
            any = true;
        }
        if !any {
            return self.refuse();
        }
        self.close(n);
        Ok(n)
    }

    fn declare_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::DeclareStatement, s)?;
        self.kw(K::DeclareStatement, w::DECLARE)?;
        self.expect(K::DeclareStatement, T::LParen)?;
        let d = self.open(K::DeclareDirective, f::NONE, C_NONE)?;
        let k = match self.word_at(0) {
            w::TICKS if self.is_exact_at(0, w::TICKS) => K::TTicks,
            w::ENCODING if self.is_exact_at(0, w::ENCODING) => K::TEncoding,
            w::STRICT_TYPES if self.is_exact_at(0, w::STRICT_TYPES) => K::TStrictTypes,
            _ => return self.refuse(),
        };
        self.tk(K::DeclareDirective, k)?;
        self.tk(K::DeclareDirective, K::TEq)?;
        self.literal(slot(f::NONE, Ctx::Lit))?;
        self.close(d);
        // `)` then `;`: one gap with the end, stored.
        let semi = self.kind_at(1) == T::Semi;
        self.punct_tk(K::DeclareStatement, T::RParen, semi)?;
        match self.peek() {
            T::Semi => {
                self.tok_leaf(K::TSemi)?;
            }
            T::CloseTag => {}
            T::Colon => {
                self.tk(K::DeclareStatement, K::TColon)?;
                self.statements_until(&[w::ENDDECLARE])?;
                self.kw(K::DeclareStatement, w::ENDDECLARE)?;
                self.semi(K::DeclareStatement)?;
            }
            _ => {
                self.statement(STMT)?;
            }
        }
        self.close(n);
        Ok(n)
    }

    fn exit_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ExitStatement, s)?;
        // `exit` `(` expression? `)` `;`: the gap ending in `;` is stored.
        let semi = match self.kind_at(1) {
            T::LParen => {
                let e = self.matching(1).ok_or_else(|| Refusal::Parse(self.tok().start))?;
                self.kind_at(e + 1) == T::Semi
            }
            _ => self.kind_at(1) == T::Semi,
        };
        let empty = self.kind_at(1) == T::LParen && self.kind_at(2) == T::RParen;
        self.kw(K::ExitStatement, w::EXIT)?;
        if self.peek() == T::LParen {
            self.punct_tk(K::ExitStatement, T::LParen, semi && empty)?;
            if self.peek() != T::RParen {
                self.expr(EXPR)?;
            }
            self.punct_tk(K::ExitStatement, T::RParen, semi)?;
        }
        self.semi(K::ExitStatement)?;
        self.close(n);
        Ok(n)
    }

    fn unset_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::UnsetStatement, s)?;
        let e = self.matching(1).ok_or_else(|| Refusal::Parse(self.tok().start))?;
        let semi = self.kind_at(e + 1) == T::Semi;
        let trailing = self.kind_at(e - 1) == T::Comma;
        self.tk(K::UnsetStatement, K::TUnset)?;
        self.expect(K::UnsetStatement, T::LParen)?;
        loop {
            self.variable(BARE)?;
            if self.peek() != T::Comma {
                break;
            }
            if self.kind_at(1) == T::RParen {
                self.tok_leaf(K::TComma)?;
                break;
            }
            self.tk(K::UnsetStatement, K::TComma)?;
        }
        self.punct_tk(K::UnsetStatement, T::RParen, semi || trailing)?;
        self.semi(K::UnsetStatement)?;
        self.close(n);
        Ok(n)
    }

    fn namespace_definition(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::NamespaceDefinition, s)?;
        self.kw(K::NamespaceDefinition, w::NAMESPACE)?;
        if self.peek() == T::Name {
            self.namespace_name(slot(f::NAME, Ctx::Bare))?;
        }
        if self.peek() == T::LBrace {
            self.compound(slot(f::BODY, Ctx::Bare))?;
        } else {
            self.semi(K::NamespaceDefinition)?;
        }
        self.close(n);
        Ok(n)
    }

    /// `namespace_name`: names between `\`s, its reserved set `nothing`; to the last name where
    /// `upto` names are given.
    pub(super) fn namespace_name(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::NamespaceName, s)?;
        self.name_any(BARE)?;
        while self.peek() == T::Backslash && self.kind_at(1) == T::Name {
            self.tk(K::NamespaceName, K::TBackslash)?;
            self.name_any(BARE)?;
        }
        self.close(n);
        Ok(n)
    }

    fn use_declaration(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::NamespaceUseDeclaration, s)?;
        self.kw(K::NamespaceUseDeclaration, w::USE)?;
        // `use function A\{…}` / `use A\{…}`: a group.
        let typed = (self.is_word(w::FUNCTION) || self.is_word(w::CONST)) && self.kind_at(1) == T::Name;
        let at = usize::from(typed);
        let mut k = at;
        let mut group = false;
        loop {
            match self.kind_at(k) {
                T::Name | T::Backslash => k += 1,
                T::LBrace => {
                    group = self.kind_at(k - 1) == T::Backslash;
                    break;
                }
                _ => break,
            }
        }
        if group {
            if typed {
                self.use_type()?;
            }
            if self.peek() == T::Backslash {
                return self.refuse();
            }
            self.namespace_name(BARE)?;
            self.tk(K::NamespaceUseDeclaration, K::TBackslash)?;
            let g = self.open(K::NamespaceUseGroup, f::BODY, C_NONE)?;
            self.list(K::NamespaceUseGroup, T::RBrace, false, true, |p| p.use_clause().map(|_| ()))?;
            self.close(g);
        } else {
            loop {
                self.use_clause()?;
                if self.peek() != T::Comma {
                    break;
                }
                self.tk(K::NamespaceUseDeclaration, K::TComma)?;
            }
        }
        self.semi(K::NamespaceUseDeclaration)?;
        self.close(n);
        Ok(n)
    }

    /// `function` or `const` in a use's `type` field.
    fn use_type(&mut self) -> R<()> {
        let k = if self.is_word(w::FUNCTION) { K::TFunction } else { K::TConst };
        self.leaf(k, f::TYPE, C_NONE)?;
        Ok(())
    }

    fn use_clause(&mut self) -> R<N> {
        let n = self.open(K::NamespaceUseClause, f::NONE, C_NONE)?;
        if (self.is_word(w::FUNCTION) || self.is_word(w::CONST)) && matches!(self.kind_at(1), T::Name | T::Backslash) {
            self.use_type()?;
        }
        self.qualified_or_name(BARE)?;
        if self.is_word(w::AS) {
            self.tk(K::NamespaceUseClause, K::TAs)?;
            self.name(slot(f::ALIAS, Ctx::Bare))?;
        }
        self.close(n);
        Ok(n)
    }

    /// `name` or `qualified_name` (a use clause's): `\`? names between `\`s.
    fn qualified_or_name(&mut self, s: Slot) -> R<N> {
        if self.peek() == T::Name && self.kind_at(1) != T::Backslash {
            return self.class_name(s);
        }
        self.qualified_name(s)
    }

    /// `_expressions`: an expression, or a `sequence_expression` of them (`a, b, c`).
    pub(super) fn expressions(&mut self, s: Slot) -> R<N> {
        let first = self.expr(s)?;
        if self.peek() != T::Comma {
            return Ok(first);
        }
        // `a, b, c` is `sequence(a, sequence(b, c))`.
        let n = self.wrap(first, K::SequenceExpression, s, EXPR);
        self.tk(K::SequenceExpression, K::TComma)?;
        let mut opened = vec![n];
        loop {
            let e = self.expr(EXPR)?;
            if self.peek() != T::Comma {
                break;
            }
            let inner = self.wrap(e, K::SequenceExpression, BARE, EXPR);
            self.tk(K::SequenceExpression, K::TComma)?;
            opened.push(inner);
        }
        for m in opened.into_iter().rev() {
            self.close(m);
        }
        Ok(n)
    }
}
