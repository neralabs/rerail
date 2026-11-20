//! Items and statements: an item list's items (a directive, a declaration or a definition, a
//! statement), and C's statements.

use super::*;

impl<'a> P<'a> {
    /// One item at `level` (not an enumerator list's), under `parent` (whose rule stores its
    /// tokens).
    pub(super) fn item(&mut self, level: Level, parent: K) -> R<()> {
        self.deeper()?;
        let r = self.item_inner(level, parent);
        self.depth -= 1;
        r
    }

    fn item_inner(&mut self, level: Level, parent: K) -> R<()> {
        let t = self.tok();
        if t.kind == T::Directive {
            return self.directive(level);
        }
        if self.cpp && level != Level::Enum && self.cpp_item(level, parent)? {
            return Ok(());
        }
        match level {
            Level::Field => return self.field_item(parent),
            Level::Enum => return self.refuse(),
            Level::Top | Level::Block => {}
        }
        if t.kind == T::Semi && level == Level::Top {
            // A lone `;` at the top: a top-level expression statement with no expression.
            let n = self.open(K::ExpressionStatement, f::NONE, C_NONE);
            self.tk(K::ExpressionStatement, K::TSemi)?;
            self.close(n);
            return Ok(());
        }
        if self.declaration_ahead() {
            return self.declaration_item(level, parent);
        }
        let s = if level == Level::Top { BARE } else { STMT };
        self.statement(s, level == Level::Top)?;
        Ok(())
    }

    /// A statement in slot `s` (`top`: a file's, whose expression statement is
    /// `_top_level_expression_statement`).
    pub(super) fn statement(&mut self, s: Slot, top: bool) -> R<N> {
        self.deeper()?;
        let r = self.statement_inner(s, top);
        self.depth -= 1;
        r
    }

    fn statement_inner(&mut self, s: Slot, top: bool) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::LBrace => return self.compound(s),
            T::Semi => {
                let n = self.open_in(K::ExpressionStatement, s);
                self.tk(K::ExpressionStatement, K::TSemi)?;
                self.close(n);
                return Ok(n);
            }
            T::LBracket if self.kind_at(1) == T::LBracket => {
                let n = self.open_in(K::AttributedStatement, s);
                while self.peek() == T::LBracket && self.kind_at(1) == T::LBracket {
                    self.attribute_declaration()?;
                }
                self.statement(STMT, false)?;
                self.close(n);
                return Ok(n);
            }
            T::Name => {}
            _ => return self.expression_statement(s, top),
        }
        if self.cpp
            && let Some(n) = self.cpp_statement(s)? {
                return Ok(n);
            }
        match t.word {
            w::IF => self.if_statement(s),
            w::SWITCH => {
                let n = self.open_in(K::SwitchStatement, s);
                self.kw(K::SwitchStatement, w::SWITCH)?;
                self.paren_condition()?;
                self.compound(slot(f::BODY, Ctx::Bare))?;
                self.close(n);
                Ok(n)
            }
            w::WHILE => {
                let n = self.open_in(K::WhileStatement, s);
                self.kw(K::WhileStatement, w::WHILE)?;
                self.paren_condition()?;
                self.statement(slot(f::BODY, Ctx::Stmt), false)?;
                self.close(n);
                Ok(n)
            }
            w::DO => {
                let n = self.open_in(K::DoStatement, s);
                self.kw(K::DoStatement, w::DO)?;
                self.statement(slot(f::BODY, Ctx::Stmt), false)?;
                self.kw(K::DoStatement, w::WHILE)?;
                self.paren_expr(slot(f::CONDITION, Ctx::Bare))?;
                self.punct_tk(K::DoStatement, T::Semi, false)?;
                self.close(n);
                Ok(n)
            }
            w::FOR => self.for_statement(s),
            w::RETURN => {
                let n = self.open_in(K::ReturnStatement, s);
                self.kw(K::ReturnStatement, w::RETURN)?;
                if self.cpp && self.peek() == T::LBrace {
                    self.initializer_list(BARE)?;
                } else if self.peek() != T::Semi {
                    self.comma_expr(EXPR)?;
                }
                self.punct_tk(K::ReturnStatement, T::Semi, false)?;
                self.close(n);
                Ok(n)
            }
            w::BREAK | w::CONTINUE | w::U_LEAVE => {
                let kind = match t.word {
                    w::BREAK => K::BreakStatement,
                    w::CONTINUE => K::ContinueStatement,
                    _ => K::SehLeaveStatement,
                };
                let n = self.open_in(kind, s);
                self.kw_any(kind)?;
                self.punct_tk(kind, T::Semi, false)?;
                self.close(n);
                Ok(n)
            }
            w::GOTO => {
                let n = self.open_in(K::GotoStatement, s);
                self.kw(K::GotoStatement, w::GOTO)?;
                if !self.is_ident() {
                    return self.refuse();
                }
                self.leaf(K::StatementIdentifier, f::LABEL, C_NONE)?;
                self.punct_tk(K::GotoStatement, T::Semi, false)?;
                self.close(n);
                Ok(n)
            }
            w::CASE | w::DEFAULT => self.case_statement(s),
            w::U_TRY => {
                let n = self.open_in(K::SehTryStatement, s);
                self.kw(K::SehTryStatement, w::U_TRY)?;
                self.compound(slot(f::BODY, Ctx::Bare))?;
                if self.is_word(w::U_EXCEPT) {
                    let c = self.open(K::SehExceptClause, f::NONE, C_NONE);
                    self.kw(K::SehExceptClause, w::U_EXCEPT)?;
                    self.paren_expr(slot(f::FILTER, Ctx::Bare))?;
                    self.compound(slot(f::BODY, Ctx::Bare))?;
                    self.close(c);
                } else if self.is_word(w::U_FINALLY) {
                    let c = self.open(K::SehFinallyClause, f::NONE, C_NONE);
                    self.kw(K::SehFinallyClause, w::U_FINALLY)?;
                    self.compound(slot(f::BODY, Ctx::Bare))?;
                    self.close(c);
                } else {
                    return self.refuse();
                }
                self.close(n);
                Ok(n)
            }
            _ if self.kind_at(1) == T::Colon && self.is_ident() => {
                let n = self.open_in(K::LabeledStatement, s);
                self.leaf(K::StatementIdentifier, f::LABEL, C_NONE)?;
                self.tk(K::LabeledStatement, K::TColon)?;
                if self.peek() == T::RBrace {
                    return self.refuse();
                }
                if self.declaration_ahead() {
                    self.declaration_item(Level::Block, K::LabeledStatement)?;
                } else {
                    self.statement(STMT, false)?;
                }
                self.close(n);
                Ok(n)
            }
            _ => self.expression_statement(s, top),
        }
    }

    /// `(expression | comma_expression)? ;` (`top`: `_expression_not_binary? ;`, inlined: its
    /// expression in no supertype).
    fn expression_statement(&mut self, s: Slot, top: bool) -> R<N> {
        let n = self.open_in(K::ExpressionStatement, s);
        if self.peek() != T::Semi {
            if top {
                let e = self.expr(BARE)?;
                if e.kind == K::BinaryExpression {
                    return self.refuse();
                }
            } else {
                self.comma_expr(EXPR)?;
            }
        }
        self.punct_tk(K::ExpressionStatement, T::Semi, false)?;
        self.close(n);
        Ok(n)
    }

    /// `{` block items `}` in slot `s`.
    pub(super) fn compound(&mut self, s: Slot) -> R<N> {
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        let n = self.open_in(K::CompoundStatement, s);
        self.tk(K::CompoundStatement, K::TLBrace)?;
        // A block's first item reads as a declaration where tree-sitter's GLR ties; later items
        // read as expressions (its tie-break compares the block's repeat nodes, whose shape
        // differs after the first item).
        // C++'s tapes show no such tie: every item reads as an expression.
        let top = std::mem::replace(&mut self.top, !self.cpp);
        let later = std::mem::replace(&mut self.later_item, false);
        while self.peek() != T::RBrace {
            if self.peek() == T::Eof {
                return self.refuse();
            }
            self.item(Level::Block, K::CompoundStatement)?;
            self.top = false;
            self.later_item = true;
        }
        self.top = top;
        self.later_item = later;
        self.tk(K::CompoundStatement, K::TRBrace)?;
        self.close(n);
        Ok(n)
    }

    /// A `condition:` `parenthesized_expression` (C++: a `condition_clause`).
    fn paren_condition(&mut self) -> R<N> {
        if self.cpp {
            return self.condition_clause();
        }
        self.paren_expr(slot(f::CONDITION, Ctx::Bare))
    }

    fn if_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::IfStatement, s);
        self.kw(K::IfStatement, w::IF)?;
        if self.cpp && self.is_word(w::CONSTEXPR) {
            self.kw(K::IfStatement, w::CONSTEXPR)?;
        }
        self.paren_condition()?;
        self.statement(slot(f::CONSEQUENCE, Ctx::Stmt), false)?;
        if self.is_word(w::ELSE) {
            let e = self.open(K::ElseClause, f::ALTERNATIVE, C_NONE);
            self.kw(K::ElseClause, w::ELSE)?;
            self.statement(STMT, false)?;
            self.close(e);
        }
        self.close(n);
        Ok(n)
    }

    fn for_statement(&mut self, s: Slot) -> R<N> {
        if self.cpp && self.range_for_ahead() {
            return self.range_for(s);
        }
        let n = self.open_in(K::ForStatement, s);
        self.kw(K::ForStatement, w::FOR)?;
        self.punct_tk(K::ForStatement, T::LParen, false)?;
        if self.peek() != T::Semi && self.declaration_ahead() {
            self.declaration_in(slot(f::INITIALIZER, Ctx::Bare))?;
        } else {
            if self.peek() != T::Semi {
                self.comma_expr(slot(f::INITIALIZER, Ctx::Expr))?;
            }
            self.punct_tk(K::ForStatement, T::Semi, false)?;
        }
        if self.peek() != T::Semi {
            self.comma_expr(slot(f::CONDITION, Ctx::Expr))?;
        }
        self.punct_tk(K::ForStatement, T::Semi, false)?;
        if self.peek() != T::RParen {
            self.comma_expr(slot(f::UPDATE, Ctx::Expr))?;
        }
        self.punct_tk(K::ForStatement, T::RParen, false)?;
        self.statement(slot(f::BODY, Ctx::Stmt), false)?;
        self.close(n);
        Ok(n)
    }

    /// `case value:` or `default:`, and the statements after it to the next label or the block's
    /// end.
    fn case_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::CaseStatement, s);
        if self.is_word(w::CASE) {
            self.kw(K::CaseStatement, w::CASE)?;
            self.expr(slot(f::VALUE, Ctx::Expr))?;
        } else {
            self.kw(K::CaseStatement, w::DEFAULT)?;
        }
        self.punct_tk(K::CaseStatement, T::Colon, false)?;
        loop {
            let t = self.tok();
            match t.kind {
                T::RBrace | T::Eof => break,
                T::Directive => break,
                T::Name if matches!(t.word, w::CASE | w::DEFAULT) => break,
                _ => {}
            }
            // `_non_case_statement` is inlined: a case's statements are in no supertype.
            if self.declaration_ahead() {
                self.declaration_item(Level::Block, K::CaseStatement)?;
            } else {
                self.statement(BARE, false)?;
            }
        }
        self.close(n);
        Ok(n)
    }
}
