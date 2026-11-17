//! Statements: blocks and statement lists, simple statements (told apart after their first
//! expression list), labels, and the headers of `if`, `for`, `switch` and `select`.

use super::*;

/// A simple statement as read: a statement node, or an expression alone (whose caller decides
/// whether it is an `expression_statement` or a header's condition or value).
enum Simple {
    Stmt(N),
    Expr(N),
}

/// An assignment operator's token kind.
fn assignment(t: T) -> Option<K> {
    use T::*;
    Some(match t {
        Eq => K::TEq,
        PlusEq => K::TPlusEq,
        MinusEq => K::TMinusEq,
        StarEq => K::TStarEq,
        SlashEq => K::TSlashEq,
        PercentEq => K::TPercentEq,
        AmpEq => K::TAmpEq,
        PipeEq => K::TPipeEq,
        CaretEq => K::TCaretEq,
        LtLtEq => K::TLtLtEq,
        GtGtEq => K::TGtGtEq,
        AmpCaretEq => K::TAmpCaretEq,
        _ => return None,
    })
}

impl<'a> P<'a> {
    /// `block` in slot `s`: `{` a statement list `}`.
    pub(super) fn block(&mut self, s: Slot) -> R<N> {
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        let n = self.open_in(K::Block, s);
        self.lines(|p| {
            p.tk(K::Block, K::TLBrace)?;
            p.statement_list(false)?;
            p.punct_tk(K::Block, T::RBrace, false)
        })?;
        self.close(n);
        Ok(n)
    }

    /// A `statement_list`, if any statement is before `}` (or a `case` or `default`, in a case).
    fn statement_list(&mut self, case: bool) -> R<()> {
        let end = |p: &mut Self| match p.peek() {
            T::RBrace | T::Eof => true,
            T::Name if case => p.is_word(w::CASE) || p.is_word(w::DEFAULT),
            _ => false,
        };
        if end(self) {
            return Ok(());
        }
        let n = self.open(K::StatementList, f::NONE, C_NONE);
        loop {
            if self.empty_label_ahead() {
                let l = self.open(K::LabeledStatement, f::NONE, C_NONE);
                self.leaf(K::LabelName, f::LABEL, C_NONE)?;
                self.tk(K::EmptyLabeledStatement, K::TColon)?;
                self.close(l);
                break;
            }
            self.statement(STMT)?;
            let t = self.tok();
            if t.kind == T::Semi || t.nl != NO_NL {
                self.terminator(K::StatementList, T::RBrace)?;
            } else if !end(self) {
                return self.refuse();
            }
            if end(self) {
                break;
            }
        }
        self.close(n);
        Ok(())
    }

    /// Whether a label ends the statement list ahead (`L:` before `}`).
    fn empty_label_ahead(&mut self) -> bool {
        self.is_ident() && self.kind_at(1) == T::Colon && self.kind_at(2) == T::RBrace
    }

    /// A statement in slot `s`.
    pub(super) fn statement(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let r = self.statement_inner(s);
        self.depth -= 1;
        r
    }

    fn statement_inner(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        match t.kind {
            T::LBrace => return self.block(s),
            T::Semi => return self.leaf_in(K::EmptyStatement, s),
            T::Name => match t.word {
                w::CONST | w::VAR | w::TYPE => return self.declaration(s),
                w::RETURN => {
                    let n = self.open_in(K::ReturnStatement, s);
                    self.tk(K::ReturnStatement, K::TReturn)?;
                    if !self.at_nl() && !matches!(self.peek(), T::Semi | T::RBrace | T::Eof) {
                        self.expression_list(BARE)?;
                    }
                    self.close(n);
                    return Ok(n);
                }
                w::GO | w::DEFER => {
                    let (kind, word) = if t.word == w::GO { (K::GoStatement, K::TGo) } else { (K::DeferStatement, K::TDefer) };
                    let n = self.open_in(kind, s);
                    self.tk(kind, word)?;
                    self.expr(EXPR)?;
                    self.close(n);
                    return Ok(n);
                }
                w::IF => return self.if_statement(s),
                w::FOR => return self.for_statement(s),
                w::SWITCH => return self.switch_statement(s),
                w::SELECT => {
                    let n = self.open_in(K::SelectStatement, s);
                    self.tk(K::SelectStatement, K::TSelect)?;
                    self.cases(K::SelectStatement)?;
                    self.close(n);
                    return Ok(n);
                }
                w::FALLTHROUGH => {
                    let n = self.open_in(K::FallthroughStatement, s);
                    self.tk(K::FallthroughStatement, K::TFallthrough)?;
                    self.close(n);
                    return Ok(n);
                }
                w::BREAK | w::CONTINUE | w::GOTO => {
                    let (kind, word) = match t.word {
                        w::BREAK => (K::BreakStatement, K::TBreak),
                        w::CONTINUE => (K::ContinueStatement, K::TContinue),
                        _ => (K::GotoStatement, K::TGoto),
                    };
                    let n = self.open_in(kind, s);
                    self.tk(kind, word)?;
                    if self.is_ident() && !self.at_nl() {
                        self.leaf(K::LabelName, f::NONE, C_NONE)?;
                    } else if kind == K::GotoStatement {
                        return self.refuse();
                    }
                    self.close(n);
                    return Ok(n);
                }
                _ if !reserved(t.word) && self.kind_at(1) == T::Colon => {
                    let n = self.open_in(K::LabeledStatement, s);
                    self.leaf(K::LabelName, f::LABEL, C_NONE)?;
                    self.tk(K::LabeledStatement, K::TColon)?;
                    self.statement(STMT)?;
                    self.close(n);
                    return Ok(n);
                }
                _ => {}
            },
            _ => {}
        }
        match self.simple(s, false)? {
            Simple::Stmt(n) => Ok(n),
            Simple::Expr(e) => Ok(self.expression_statement(e, s)),
        }
    }

    /// `e` wrapped in an `expression_statement` in slot `s`.
    fn expression_statement(&mut self, e: N, s: Slot) -> N {
        let n = self.wrap(e, K::ExpressionStatement, s, EXPR);
        self.close(n);
        n
    }

    /// A simple statement in slot `s`: an expression list, then what follows it says which
    /// (`:=`, an assignment, `<-`, `++`, `--`); an expression alone is left to the caller. Where
    /// `range` (a `for` header), `= range` and `:= range` start a range clause.
    fn simple(&mut self, s: Slot, range: bool) -> R<Simple> {
        let first = self.expr(EXPR)?;
        let t = self.tok();
        let nl = self.at_nl();
        if t.kind == T::Comma || (!nl && (t.kind == T::ColonEq || assignment(t.kind).is_some())) {
            // A list, and what follows it.
            let list = self.wrap(first, K::ExpressionList, slot(f::LEFT, Ctx::Bare), EXPR);
            while self.peek() == T::Comma {
                self.tk(K::ExpressionList, K::TComma)?;
                self.expr(EXPR)?;
            }
            self.close(list);
            let t = self.tok();
            if range && matches!(t.kind, T::ColonEq | T::Eq) && self.is_word_at(1, w::RANGE) {
                let n = self.wrap(list, K::RangeClause, BARE, slot(f::LEFT, Ctx::Bare));
                self.leaf(punct(t.kind).expect("an operator"), f::NONE, C_NONE)?;
                self.tk(K::RangeClause, K::TRange)?;
                self.expr(slot(f::RIGHT, Ctx::Expr))?;
                self.close(n);
                return Ok(Simple::Stmt(n));
            }
            if t.kind == T::ColonEq {
                let n = self.wrap(list, K::ShortVarDeclaration, s, slot(f::LEFT, Ctx::Bare));
                self.tk(K::ShortVarDeclaration, K::TColonEq)?;
                self.expression_list(slot(f::RIGHT, Ctx::Bare))?;
                self.close(n);
                return Ok(Simple::Stmt(n));
            }
            let Some(op) = assignment(t.kind) else { return self.refuse() };
            let n = self.wrap(list, K::AssignmentStatement, s, slot(f::LEFT, Ctx::Bare));
            self.leaf(op, f::OPERATOR, C_NONE)?;
            self.expression_list(slot(f::RIGHT, Ctx::Bare))?;
            self.close(n);
            return Ok(Simple::Stmt(n));
        }
        if nl {
            return Ok(Simple::Expr(first));
        }
        match t.kind {
            T::PlusPlus | T::MinusMinus => {
                let kind = if t.kind == T::PlusPlus { K::IncStatement } else { K::DecStatement };
                let n = self.wrap(first, kind, s, EXPR);
                self.tk(kind, punct(t.kind).expect("an operator"))?;
                self.close(n);
                Ok(Simple::Stmt(n))
            }
            T::LtMinus => {
                let n = self.wrap(first, K::SendStatement, s, slot(f::CHANNEL, Ctx::Expr));
                self.tk(K::SendStatement, K::TLtMinus)?;
                self.expr(slot(f::VALUE, Ctx::Expr))?;
                self.close(n);
                Ok(Simple::Stmt(n))
            }
            _ => Ok(Simple::Expr(first)),
        }
    }

    /// A header's simple statement or expression, read with no composite literal after a type
    /// name and no line end.
    fn header<X>(&mut self, f: impl FnOnce(&mut Self) -> R<X>) -> R<X> {
        let saved = (self.nl_ends, self.no_literal);
        (self.nl_ends, self.no_literal) = (false, true);
        let r = f(self);
        (self.nl_ends, self.no_literal) = saved;
        r
    }

    /// `if`: an initializer and `;`, a condition, a block, and `else` and a block or an `if`.
    fn if_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::IfStatement, s);
        self.tk(K::IfStatement, K::TIf)?;
        self.header(|p| {
            match p.simple(slot(f::INITIALIZER, Ctx::Simple), false)? {
                Simple::Stmt(_) => {
                    p.punct_tk(K::IfStatement, T::Semi, false)?;
                    p.expr(slot(f::CONDITION, Ctx::Expr))?;
                }
                Simple::Expr(e) if p.peek() == T::Semi => {
                    p.expression_statement(e, slot(f::INITIALIZER, Ctx::Simple));
                    p.tk(K::IfStatement, K::TSemi)?;
                    p.expr(slot(f::CONDITION, Ctx::Expr))?;
                }
                Simple::Expr(e) => p.rekey(e.at, slot(f::CONDITION, Ctx::Expr)),
            }
            Ok(())
        })?;
        self.block(slot(f::CONSEQUENCE, Ctx::Bare))?;
        if self.is_word(w::ELSE) && !self.at_nl() {
            self.tk(K::IfStatement, K::TElse)?;
            if self.is_word(w::IF) {
                self.if_statement(slot(f::ALTERNATIVE, Ctx::Bare))?;
            } else {
                self.block(slot(f::ALTERNATIVE, Ctx::Bare))?;
            }
        }
        self.close(n);
        Ok(n)
    }

    /// `for`: nothing, a condition, a `for_clause` or a `range_clause`; then the body.
    fn for_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ForStatement, s);
        self.tk(K::ForStatement, K::TFor)?;
        self.header(|p| {
            if p.peek() == T::LBrace {
                return Ok(());
            }
            if p.is_word(w::RANGE) {
                let r = p.open(K::RangeClause, f::NONE, C_NONE);
                p.tk(K::RangeClause, K::TRange)?;
                p.expr(slot(f::RIGHT, Ctx::Expr))?;
                p.close(r);
                return Ok(());
            }
            if p.peek() == T::Semi {
                let c = p.open(K::ForClause, f::NONE, C_NONE);
                return p.for_clause(c);
            }
            match p.simple(slot(f::INITIALIZER, Ctx::Simple), true)? {
                Simple::Stmt(st) if p.peek() == T::Semi => {
                    if p.kind_of_node(st) == K::RangeClause {
                        return p.refuse();
                    }
                    let c = p.wrap(st, K::ForClause, BARE, slot(f::INITIALIZER, Ctx::Simple));
                    p.for_clause(c)
                }
                Simple::Stmt(st) => {
                    if p.kind_of_node(st) != K::RangeClause {
                        return p.refuse();
                    }
                    Ok(())
                }
                Simple::Expr(e) if p.peek() == T::Semi => {
                    let st = p.expression_statement(e, slot(f::INITIALIZER, Ctx::Simple));
                    let c = p.wrap(st, K::ForClause, BARE, slot(f::INITIALIZER, Ctx::Simple));
                    p.for_clause(c)
                }
                Simple::Expr(_) => Ok(()),
            }
        })?;
        self.block(slot(f::BODY, Ctx::Bare))?;
        self.close(n);
        Ok(n)
    }

    fn kind_of_node(&self, n: N) -> K {
        n.kind
    }

    /// The rest of a `for_clause` (its node `c` open, its initializer read): `;` a condition `;` an
    /// update.
    fn for_clause(&mut self, c: N) -> R<()> {
        self.punct_tk(K::ForClause, T::Semi, false)?;
        if self.peek() != T::Semi {
            self.expr(slot(f::CONDITION, Ctx::Expr))?;
        }
        self.punct_tk(K::ForClause, T::Semi, false)?;
        if self.peek() != T::LBrace {
            match self.simple(slot(f::UPDATE, Ctx::Simple), false)? {
                Simple::Stmt(_) => {}
                Simple::Expr(e) => {
                    self.expression_statement(e, slot(f::UPDATE, Ctx::Simple));
                }
            }
        }
        self.close(c);
        Ok(())
    }

    /// Whether the switch header ahead (after `switch`) is a type switch's: `.(type)` at its
    /// depth before the body's `{`. Where it holds `:=` before that, and a `;`.
    fn type_switch_ahead(&mut self) -> Option<(bool, bool)> {
        let (mut depth, mut j) = (0usize, 0usize);
        let (mut semi, mut alias) = (false, false);
        loop {
            match self.kind_at(j) {
                T::LParen | T::LBracket => depth += 1,
                T::RParen | T::RBracket => depth = depth.checked_sub(1)?,
                // A `{` at the header's depth opens the body, but a type's (`interface{}(x)`).
                T::LBrace if depth == 0 && !(j > 0 && (self.is_word_at(j - 1, w::INTERFACE) || self.is_word_at(j - 1, w::STRUCT))) => {
                    return None
                }
                T::LBrace => depth += 1,
                T::RBrace => depth = depth.checked_sub(1)?,
                T::Semi if depth == 0 => {
                    semi = true;
                    alias = false;
                }
                T::ColonEq if depth == 0 => alias = true,
                T::Dot if depth == 0 && self.kind_at(j + 1) == T::LParen && self.is_word_at(j + 2, w::TYPE) => {
                    return Some((semi, alias));
                }
                T::Eof => return None,
                _ => {}
            }
            j += 1;
        }
    }

    /// `switch`: an expression switch or a type switch.
    fn switch_statement(&mut self, s: Slot) -> R<N> {
        let i = self.i;
        self.i += 1;
        self.ahead();
        let ts = self.type_switch_ahead();
        self.i = i;
        let Some((semi, alias)) = ts else { return self.expression_switch(s) };
        let n = self.open_in(K::TypeSwitchStatement, s);
        self.tk(K::TypeSwitchStatement, K::TSwitch)?;
        self.header(|p| {
            if semi {
                match p.simple(slot(f::INITIALIZER, Ctx::Simple), false)? {
                    Simple::Stmt(_) => {}
                    Simple::Expr(e) => {
                        p.expression_statement(e, slot(f::INITIALIZER, Ctx::Simple));
                    }
                }
                p.punct_tk(K::TypeSwitchStatement, T::Semi, false)?;
            }
            if alias {
                p.expression_list(slot(f::ALIAS, Ctx::Bare))?;
                p.punct_tk(K::TypeSwitchStatement, T::ColonEq, false)?;
            }
            p.expr(slot(f::VALUE, Ctx::Expr))?;
            p.tk(K::TypeSwitchStatement, K::TDot)?;
            p.punct_tk(K::TypeSwitchStatement, T::LParen, false)?;
            p.kw(K::TypeSwitchStatement, w::TYPE)?;
            p.punct_tk(K::TypeSwitchStatement, T::RParen, false)
        })?;
        self.cases(K::TypeSwitchStatement)?;
        self.close(n);
        Ok(n)
    }

    /// An expression switch: an initializer and `;`, a value, then the cases.
    fn expression_switch(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ExpressionSwitchStatement, s);
        self.tk(K::ExpressionSwitchStatement, K::TSwitch)?;
        self.header(|p| {
            if p.peek() == T::LBrace {
                return Ok(());
            }
            if p.peek() == T::Semi {
                p.tk(K::ExpressionSwitchStatement, K::TSemi)?;
            } else {
                match p.simple(slot(f::INITIALIZER, Ctx::Simple), false)? {
                    Simple::Stmt(_) => p.punct_tk(K::ExpressionSwitchStatement, T::Semi, false)?,
                    Simple::Expr(e) if p.peek() == T::Semi => {
                        p.expression_statement(e, slot(f::INITIALIZER, Ctx::Simple));
                        p.tk(K::ExpressionSwitchStatement, K::TSemi)?;
                    }
                    Simple::Expr(e) => {
                        p.rekey(e.at, slot(f::VALUE, Ctx::Expr));
                        return Ok(());
                    }
                }
            }
            if p.peek() != T::LBrace {
                p.expr(slot(f::VALUE, Ctx::Expr))?;
            }
            Ok(())
        })?;
        self.cases(K::ExpressionSwitchStatement)?;
        self.close(n);
        Ok(n)
    }

    /// `{` cases `}` of a switch or select (`parent`).
    fn cases(&mut self, parent: K) -> R<()> {
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        self.lines(|p| {
            p.tk(parent, K::TLBrace)?;
            loop {
                if p.peek() == T::RBrace {
                    break;
                }
                if p.is_word(w::DEFAULT) {
                    let c = p.open(K::DefaultCase, f::NONE, C_NONE);
                    p.tk(K::DefaultCase, K::TDefault)?;
                    p.punct_tk(K::DefaultCase, T::Colon, false)?;
                    p.statement_list(true)?;
                    p.close(c);
                    continue;
                }
                if !p.is_word(w::CASE) {
                    return p.refuse();
                }
                let kind = match parent {
                    K::TypeSwitchStatement => K::TypeCase,
                    K::SelectStatement => K::CommunicationCase,
                    _ => K::ExpressionCase,
                };
                let c = p.open(kind, f::NONE, C_NONE);
                p.tk(kind, K::TCase)?;
                p.nested(|p| match kind {
                    K::TypeCase => loop {
                        p.ty(slot(f::TYPE, Ctx::Type))?;
                        if p.peek() != T::Comma {
                            return Ok(());
                        }
                        p.tk_field(K::TypeCase, K::TComma, f::TYPE)?;
                    },
                    K::CommunicationCase => p.communication(),
                    _ => p.expression_list(slot(f::VALUE, Ctx::Bare)).map(|_| ()),
                })?;
                p.punct_tk(kind, T::Colon, false)?;
                p.statement_list(true)?;
                p.close(c);
            }
            p.tk(parent, K::TRBrace)
        })
    }

    /// A communication case's statement: a send, or a receive (`v := <-ch`, `<-ch`).
    fn communication(&mut self) -> R<()> {
        let first = self.expr(EXPR)?;
        let t = self.tok();
        if t.kind == T::LtMinus {
            let n = self.wrap(first, K::SendStatement, slot(f::COMMUNICATION, Ctx::Bare), slot(f::CHANNEL, Ctx::Expr));
            self.tk(K::SendStatement, K::TLtMinus)?;
            self.expr(slot(f::VALUE, Ctx::Expr))?;
            self.close(n);
            return Ok(());
        }
        if matches!(t.kind, T::Comma | T::Eq | T::ColonEq) {
            let list = self.wrap(first, K::ExpressionList, slot(f::LEFT, Ctx::Bare), EXPR);
            while self.peek() == T::Comma {
                self.tk(K::ExpressionList, K::TComma)?;
                self.expr(EXPR)?;
            }
            self.close(list);
            let n = self.wrap(list, K::ReceiveStatement, slot(f::COMMUNICATION, Ctx::Bare), slot(f::LEFT, Ctx::Bare));
            let t = self.tok();
            if !matches!(t.kind, T::Eq | T::ColonEq) {
                return self.refuse();
            }
            self.leaf(punct(t.kind).expect("an operator"), f::NONE, C_NONE)?;
            self.expr(slot(f::RIGHT, Ctx::Expr))?;
            self.close(n);
            return Ok(());
        }
        let n = self.wrap(first, K::ReceiveStatement, slot(f::COMMUNICATION, Ctx::Bare), slot(f::RIGHT, Ctx::Expr));
        self.close(n);
        Ok(())
    }
}
