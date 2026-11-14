//! Statements, blocks, local declarations and functions, and `#if` among statements.

use super::*;

/// What a statement that starts with a type is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Local {
    /// `local_declaration_statement`.
    Declaration,
    /// `local_function_statement`.
    Function,
}

/// Where an `#if`'s items stand: what each item is read as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Items {
    /// Statements (a block's).
    Statements,
    /// Declarations (a declaration list's).
    Declarations,
    /// The file's items.
    TopLevel,
}

impl<'a> P<'a> {
    /// A `block` in slot `s`: `{` statements `}`.
    pub(super) fn block(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::Block, s)?;
        self.tk(K::Block, K::TLBrace)?;
        let saved = std::mem::replace(&mut self.in_statement, true);
        let r = self.statements();
        self.in_statement = saved;
        r?;
        self.punct_tk(K::Block, T::RBrace, false)?;
        self.close(n);
        Ok(n)
    }

    /// Statements to a `}`, an `#elif`, an `#else` or an `#endif`.
    pub(super) fn statements(&mut self) -> R<()> {
        loop {
            match self.peek() {
                T::RBrace | T::PpElif | T::PpElse | T::PpEndif | T::Eof => return Ok(()),
                _ => {
                    self.statement(STMT)?;
                }
            }
        }
    }

    /// A statement in slot `s`.
    pub(super) fn statement(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let t = self.tok();
        let n = match t.kind {
            T::LBrace => self.block(s)?,
            T::Semi => {
                let n = self.open_in(K::EmptyStatement, s)?;
                self.leaf(K::TSemi, f::NONE, C_NONE)?;
                self.close(n);
                n
            }
            T::PpIf => self.preproc_if(s, Items::Statements)?,
            T::Name => match t.word {
                w::IF => self.if_statement(s)?,
                w::WHILE => {
                    let n = self.open_in(K::WhileStatement, s)?;
                    self.leaf(K::TWhile, f::NONE, C_NONE)?;
                    self.condition(K::WhileStatement)?;
                    self.statement(slot(f::BODY, Ctx::Stmt))?;
                    self.close(n);
                    n
                }
                w::DO => {
                    let n = self.open_in(K::DoStatement, s)?;
                    self.tk(K::DoStatement, K::TDo)?;
                    self.statement(slot(f::BODY, Ctx::Stmt))?;
                    self.kw(K::DoStatement, w::WHILE)?;
                    self.condition(K::DoStatement)?;
                    self.punct_tk(K::DoStatement, T::Semi, false)?;
                    self.close(n);
                    n
                }
                w::FOR => self.for_statement(s)?,
                w::FOREACH => self.foreach_statement(s, false)?,
                w::AWAIT if self.is_word_at(1, w::FOREACH) => self.foreach_statement(s, true)?,
                w::AWAIT if self.is_word_at(1, w::USING) && self.kind_at(2) == T::LParen => self.using_statement(s, true)?,
                w::USING if self.kind_at(1) == T::LParen => self.using_statement(s, false)?,
                w::SWITCH => self.switch_statement(s)?,
                w::TRY => self.try_statement(s)?,
                w::RETURN | w::THROW => {
                    let kind = if t.word == w::RETURN { K::ReturnStatement } else { K::ThrowStatement };
                    let n = self.open_in(kind, s)?;
                    self.leaf(keyword(t.word).expect("a keyword"), f::NONE, C_NONE)?;
                    let mut bare = false;
                    if self.peek() != T::Semi {
                        let e = self.expr(EXPR)?;
                        bare = matches!(e.kind, K::TThis | K::TBase);
                    }
                    self.punct_tk(kind, T::Semi, bare)?;
                    self.close(n);
                    n
                }
                w::BREAK | w::CONTINUE => {
                    let kind = if t.word == w::BREAK { K::BreakStatement } else { K::ContinueStatement };
                    let n = self.open_in(kind, s)?;
                    self.tk(kind, keyword(t.word).expect("a keyword"))?;
                    self.punct_tk(kind, T::Semi, false)?;
                    self.close(n);
                    n
                }
                w::GOTO => {
                    let n = self.open_in(K::GotoStatement, s)?;
                    self.tk(K::GotoStatement, K::TGoto)?;
                    let mut word = false;
                    if self.is_word(w::CASE) || self.is_word(w::DEFAULT) {
                        let k = keyword(self.tok().word).expect("a keyword");
                        self.tk(K::GotoStatement, k)?;
                        word = true;
                    }
                    let mut expr = false;
                    if self.peek() != T::Semi {
                        self.expr(EXPR)?;
                        expr = true;
                    }
                    // `goto default;` is no implied word (`goto ;` is shorter).
                    self.punct_tk(K::GotoStatement, T::Semi, word && !expr)?;
                    self.close(n);
                    n
                }
                w::LOCK => {
                    let n = self.open_in(K::LockStatement, s)?;
                    self.tk(K::LockStatement, K::TLock)?;
                    let bare = self.is_word_at(1, w::THIS) && self.kind_at(2) == T::RParen;
                    self.punct_tk(K::LockStatement, T::LParen, bare)?;
                    self.expr(EXPR)?;
                    self.punct_tk(K::LockStatement, T::RParen, bare)?;
                    self.statement(STMT)?;
                    self.close(n);
                    n
                }
                w::FIXED if self.kind_at(1) == T::LParen => {
                    let n = self.open_in(K::FixedStatement, s)?;
                    self.tk(K::FixedStatement, K::TFixed)?;
                    self.punct_tk(K::FixedStatement, T::LParen, false)?;
                    self.variable_declaration(f::NONE)?;
                    self.punct_tk(K::FixedStatement, T::RParen, false)?;
                    self.statement(STMT)?;
                    self.close(n);
                    n
                }
                w::CHECKED | w::UNCHECKED if self.kind_at(1) == T::LBrace => {
                    let n = self.open_in(K::CheckedStatement, s)?;
                    self.leaf(keyword(t.word).expect("a keyword"), f::NONE, C_NONE)?;
                    self.block(BARE)?;
                    self.close(n);
                    n
                }
                w::UNSAFE if self.kind_at(1) == T::LBrace => {
                    let n = self.open_in(K::UnsafeStatement, s)?;
                    self.tk(K::UnsafeStatement, K::TUnsafe)?;
                    self.block(BARE)?;
                    self.close(n);
                    n
                }
                w::YIELD if self.is_word_at(1, w::RETURN) || self.is_word_at(1, w::BREAK) => {
                    let n = self.open_in(K::YieldStatement, s)?;
                    self.tk(K::YieldStatement, K::TYield)?;
                    if self.is_word(w::RETURN) {
                        self.tk(K::YieldStatement, K::TReturn)?;
                        self.expr(EXPR)?;
                    } else {
                        self.tk(K::YieldStatement, K::TBreak)?;
                    }
                    self.punct_tk(K::YieldStatement, T::Semi, false)?;
                    self.close(n);
                    n
                }
                _ if self.is_ident() && self.kind_at(1) == T::Colon => {
                    let n = self.open_in(K::LabeledStatement, s)?;
                    self.ident(BARE)?;
                    self.tk(K::LabeledStatement, K::TColon)?;
                    self.statement(STMT)?;
                    self.close(n);
                    n
                }
                _ => self.declaration_or_expression(s)?,
            },
            _ => self.declaration_or_expression(s)?,
        };
        self.depth -= 1;
        Ok(n)
    }

    /// `(` condition `)` of a `while`, a `do`, an `if`.
    fn condition(&mut self, parent: K) -> R<()> {
        self.punct_tk(parent, T::LParen, false)?;
        self.expr(slot(f::CONDITION, Ctx::Expr))?;
        self.punct_tk(parent, T::RParen, false)
    }

    fn if_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::IfStatement, s)?;
        self.leaf(K::TIf, f::NONE, C_NONE)?;
        self.condition(K::IfStatement)?;
        self.statement(slot(f::CONSEQUENCE, Ctx::Stmt))?;
        if self.is_word(w::ELSE) {
            self.tk(K::IfStatement, K::TElse)?;
            self.statement(slot(f::ALTERNATIVE, Ctx::Stmt))?;
        }
        self.close(n);
        Ok(n)
    }

    /// Whether a local declaration or function starts here (its modifiers, a type and a name).
    pub(super) fn local_ahead(&mut self) -> Option<Local> {
        let mut k = 0;
        if self.is_word_at(k, w::AWAIT) && self.is_word_at(k + 1, w::USING) {
            k += 1;
        }
        if self.is_word_at(k, w::USING) {
            k += 1;
        }
        while self.modifier_at(k) {
            k += 1;
        }
        let e = self.scan_type(k)?;
        // `await f();`, `await t;`: an `await` expression.
        if e == k + 1 && self.is_word_at(k, w::AWAIT) {
            return None;
        }
        if self.is_ident_at(e) {
            return match self.kind_at(e + 1) {
                T::Eq | T::Semi | T::Comma | T::LBracket => Some(Local::Declaration),
                T::LParen => Some(Local::Function),
                T::Lt => {
                    let close = self.scan_type_args(e + 1)?;
                    (self.kind_at(close) == T::LParen).then_some(Local::Function)
                }
                _ => None,
            };
        }
        // `var (a, b) = t;`
        if self.is_word_at(k, w::VAR) && e == k + 1 && self.kind_at(e) == T::LParen {
            let close = self.matching(e)?;
            return (self.kind_at(close + 1) == T::Eq).then_some(Local::Declaration);
        }
        None
    }

    /// A statement that is a local declaration, a local function, or an expression.
    fn declaration_or_expression(&mut self, s: Slot) -> R<N> {
        match self.local_ahead() {
            Some(Local::Declaration) => {
                let n = self.open_in(K::LocalDeclarationStatement, s)?;
                if self.is_word(w::AWAIT) {
                    self.tk(K::LocalDeclarationStatement, K::TAwait)?;
                }
                if self.is_word(w::USING) {
                    self.tk(K::LocalDeclarationStatement, K::TUsing)?;
                }
                self.modifiers()?;
                self.variable_declaration(f::NONE)?;
                self.punct_tk(K::LocalDeclarationStatement, T::Semi, false)?;
                self.close(n);
                Ok(n)
            }
            Some(Local::Function) => {
                let n = self.open_in(K::LocalFunctionStatement, s)?;
                self.modifiers()?;
                self.ty(slot(f::TYPE, Ctx::Type))?;
                self.ident(slot(f::NAME, Ctx::Bare))?;
                if self.peek() == T::Lt {
                    self.type_parameter_list(f::TYPE_PARAMETERS)?;
                }
                self.parameter_list(K::ParameterList, f::PARAMETERS)?;
                self.constraints()?;
                self.function_body(K::LocalFunctionStatement)?;
                self.close(n);
                Ok(n)
            }
            None => {
                let n = self.open_in(K::ExpressionStatement, s)?;
                let e = self.expr(BARE)?;
                if !matches!(
                    e.kind,
                    K::AssignmentExpression | K::InvocationExpression | K::PostfixUnaryExpression | K::PrefixUnaryExpression | K::AwaitExpression | K::ObjectCreationExpression | K::ParenthesizedExpression
                ) {
                    return self.refuse();
                }
                self.punct_tk(K::ExpressionStatement, T::Semi, false)?;
                self.close(n);
                Ok(n)
            }
        }
    }

    /// `variable_declaration`: a type and declarators between `,`s.
    pub(super) fn variable_declaration(&mut self, field: u8) -> R<N> {
        let n = self.open(K::VariableDeclaration, field, C_NONE)?;
        self.ty(slot(f::TYPE, Ctx::Type))?;
        loop {
            self.variable_declarator()?;
            if self.peek() != T::Comma {
                break;
            }
            self.tk(K::VariableDeclaration, K::TComma)?;
        }
        self.close(n);
        Ok(n)
    }

    /// `variable_declarator`: a name (or a tuple pattern), a bracketed size, and `= value`.
    fn variable_declarator(&mut self) -> R<()> {
        let n = self.open(K::VariableDeclarator, f::NONE, C_NONE)?;
        if self.peek() == T::LParen {
            self.tuple_pattern(f::NONE)?;
        } else {
            self.ident(slot(f::NAME, Ctx::Bare))?;
        }
        if self.peek() == T::LBracket {
            self.bracketed_arguments(K::BracketedArgumentList, f::NONE, C_NONE)?;
        }
        if self.peek() == T::Eq {
            self.tk(K::VariableDeclarator, K::TEq)?;
            self.expr(EXPR)?;
        }
        self.close(n);
        Ok(())
    }

    /// `tuple_pattern`: `(` names, `_`s and tuple patterns `)`.
    pub(super) fn tuple_pattern(&mut self, field: u8) -> R<()> {
        let n = self.open(K::TuplePattern, field, C_NONE)?;
        self.list(K::TuplePattern, T::RParen, true, false, |p| {
            if p.peek() == T::LParen {
                p.tuple_pattern(f::NONE)
            } else if p.is_word(w::UNDERSCORE) {
                p.leaf(K::Discard, f::NONE, C_NONE).map(|_| ())
            } else {
                p.ident(slot(f::NAME, Ctx::Bare)).map(|_| ())
            }
        })?;
        self.close(n);
        Ok(())
    }

    fn for_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ForStatement, s)?;
        self.leaf(K::TFor, f::NONE, C_NONE)?;
        self.punct_tk(K::ForStatement, T::LParen, false)?;
        if self.peek() != T::Semi {
            if self.local_ahead() == Some(Local::Declaration) {
                self.variable_declaration(f::INITIALIZER)?;
            } else {
                loop {
                    self.expr(slot(f::INITIALIZER, Ctx::Expr))?;
                    if self.peek() != T::Comma {
                        break;
                    }
                    self.tk_field(K::ForStatement, K::TComma, f::INITIALIZER)?;
                }
            }
        }
        self.punct_tk(K::ForStatement, T::Semi, false)?;
        if self.peek() != T::Semi {
            self.expr(slot(f::CONDITION, Ctx::Expr))?;
        }
        self.punct_tk(K::ForStatement, T::Semi, false)?;
        if self.peek() != T::RParen {
            loop {
                self.expr(slot(f::UPDATE, Ctx::Expr))?;
                if self.peek() != T::Comma {
                    break;
                }
                self.tk_field(K::ForStatement, K::TComma, f::UPDATE)?;
            }
        }
        self.punct_tk(K::ForStatement, T::RParen, false)?;
        self.statement(slot(f::BODY, Ctx::Stmt))?;
        self.close(n);
        Ok(n)
    }

    fn foreach_statement(&mut self, s: Slot, awaited: bool) -> R<N> {
        let n = self.open_in(K::ForeachStatement, s)?;
        if awaited {
            self.leaf(K::TAwait, f::NONE, C_NONE)?;
        }
        self.leaf(K::TForeach, f::NONE, C_NONE)?;
        self.punct_tk(K::ForeachStatement, T::LParen, awaited)?;
        // `type name in`, `var (a, b) in`, or an expression.
        let typed = self.scan_type(0).is_some_and(|e| {
            (self.is_ident_at(e) && self.is_word_at(e + 1, w::IN)) || (self.kind_at(e) == T::LParen && self.matching(e).is_some_and(|c| self.is_word_at(c + 1, w::IN)))
        });
        // `A<B> x in`, one plain name as the type argument: to tree-sitter, `A < B > x`.
        let compared = self.is_ident_at(0)
            && self.kind_at(1) == T::Lt
            && self.is_ident_at(2)
            && self.kind_at(3) == T::Gt
            && self.is_ident_at(4)
            && self.is_word_at(5, w::IN);
        if compared {
            let left = slot(f::LEFT, Ctx::Expr);
            let outer = self.open_in(K::BinaryExpression, left)?;
            let inner = self.open_in(K::BinaryExpression, left)?;
            self.ident(left)?;
            self.leaf(K::TLt, f::OPERATOR, C_NONE)?;
            self.ident(slot(f::RIGHT, Ctx::Expr))?;
            self.close(inner);
            self.leaf(K::TGt, f::OPERATOR, C_NONE)?;
            self.ident(slot(f::RIGHT, Ctx::Expr))?;
            self.close(outer);
        } else if typed {
            self.ty(slot(f::TYPE, Ctx::Type))?;
            if self.peek() == T::LParen {
                self.tuple_pattern(f::LEFT)?;
            } else {
                self.ident(slot(f::LEFT, Ctx::Bare))?;
            }
        } else {
            self.expr(slot(f::LEFT, Ctx::Expr))?;
        }
        self.leaf(K::TIn, f::NONE, C_NONE)?;
        self.expr(slot(f::RIGHT, Ctx::Expr))?;
        self.punct_tk(K::ForeachStatement, T::RParen, false)?;
        self.statement(slot(f::BODY, Ctx::Stmt))?;
        self.close(n);
        Ok(n)
    }

    fn using_statement(&mut self, s: Slot, awaited: bool) -> R<N> {
        let n = self.open_in(K::UsingStatement, s)?;
        if awaited {
            self.leaf(K::TAwait, f::NONE, C_NONE)?;
        }
        self.leaf(K::TUsing, f::NONE, C_NONE)?;
        self.punct_tk(K::UsingStatement, T::LParen, awaited)?;
        let declaration = self.scan_type(0).is_some_and(|e| self.is_ident_at(e) && matches!(self.kind_at(e + 1), T::Eq | T::Comma | T::RParen));
        if declaration {
            self.variable_declaration(f::NONE)?;
        } else {
            self.expr(EXPR)?;
        }
        self.punct_tk(K::UsingStatement, T::RParen, false)?;
        self.statement(slot(f::BODY, Ctx::Stmt))?;
        self.close(n);
        Ok(n)
    }

    fn switch_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::SwitchStatement, s)?;
        self.leaf(K::TSwitch, f::NONE, C_NONE)?;
        if self.peek() != T::LParen {
            return self.refuse();
        }
        let close = self.matching(0).ok_or_else(|| Refusal::Parse(self.tok().start))?;
        if self.top_comma_pub(1, close) {
            self.expr(slot(f::VALUE, Ctx::Bare))?;
        } else {
            self.punct_tk(K::SwitchStatement, T::LParen, false)?;
            self.expr(slot(f::VALUE, Ctx::Expr))?;
            self.punct_tk(K::SwitchStatement, T::RParen, false)?;
        }
        let b = self.open(K::SwitchBody, f::BODY, C_NONE)?;
        self.punct_tk(K::SwitchBody, T::LBrace, false)?;
        while self.peek() != T::RBrace {
            let sec = self.open(K::SwitchSection, f::NONE, C_NONE)?;
            if self.is_word(w::CASE) {
                self.leaf(K::TCase, f::NONE, C_NONE)?;
                self.pattern(PATTERN)?;
                if self.is_word(w::WHEN) {
                    self.when_clause()?;
                }
            } else if self.is_word(w::DEFAULT) {
                self.leaf(K::TDefault, f::NONE, C_NONE)?;
            } else {
                return self.refuse();
            }
            if self.peek() != T::Colon {
                return self.refuse();
            }
            self.leaf(K::TColon, f::NONE, C_NONE)?;
            while !(self.is_word(w::CASE) || (self.is_word(w::DEFAULT) && self.kind_at(1) == T::Colon) || matches!(self.peek(), T::RBrace | T::Eof)) {
                self.statement(STMT)?;
            }
            self.close(sec);
        }
        self.punct_tk(K::SwitchBody, T::RBrace, false)?;
        self.close(b);
        self.close(n);
        Ok(n)
    }

    fn try_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::TryStatement, s)?;
        self.tk(K::TryStatement, K::TTry)?;
        self.block(slot(f::BODY, Ctx::Bare))?;
        while self.is_word(w::CATCH) {
            let c = self.open(K::CatchClause, f::NONE, C_NONE)?;
            self.tk(K::CatchClause, K::TCatch)?;
            if self.peek() == T::LParen {
                let d = self.open(K::CatchDeclaration, f::NONE, C_NONE)?;
                self.punct_tk(K::CatchDeclaration, T::LParen, false)?;
                self.ty(slot(f::TYPE, Ctx::Type))?;
                if self.is_ident() {
                    self.ident(slot(f::NAME, Ctx::Bare))?;
                }
                self.punct_tk(K::CatchDeclaration, T::RParen, false)?;
                self.close(d);
            }
            if self.is_word(w::WHEN) {
                let w_ = self.open(K::CatchFilterClause, f::NONE, C_NONE)?;
                self.tk(K::CatchFilterClause, K::TWhen)?;
                self.punct_tk(K::CatchFilterClause, T::LParen, false)?;
                self.expr(EXPR)?;
                self.punct_tk(K::CatchFilterClause, T::RParen, false)?;
                self.close(w_);
            }
            self.block(slot(f::BODY, Ctx::Bare))?;
            self.close(c);
        }
        if self.is_word(w::FINALLY) {
            let c = self.open(K::FinallyClause, f::NONE, C_NONE)?;
            self.tk(K::FinallyClause, K::TFinally)?;
            self.block(BARE)?;
            self.close(c);
        }
        self.close(n);
        Ok(n)
    }

    // ---- `#if`.

    /// `preproc_if` in slot `s`, its items read as `items`: `#if` condition, items, and an `#elif`
    /// or `#else`, to `#endif`.
    pub(super) fn preproc_if(&mut self, s: Slot, items: Items) -> R<N> {
        let n = self.open_in(K::PreprocIf, s)?;
        self.tk(K::PreprocIf, K::THashIf)?;
        self.preproc_condition()?;
        self.preproc_items(items)?;
        self.preproc_alternative(items)?;
        if self.peek() != T::PpEndif {
            return self.refuse();
        }
        self.tk(K::PreprocIf, K::THashEndif)?;
        self.close(n);
        Ok(n)
    }

    fn preproc_alternative(&mut self, items: Items) -> R<()> {
        match self.peek() {
            T::PpElif => {
                let n = self.open(K::PreprocElif, f::ALTERNATIVE, C_NONE)?;
                self.tk(K::PreprocElif, K::THashElif)?;
                self.preproc_condition()?;
                self.preproc_items(items)?;
                self.preproc_alternative(items)?;
                self.close(n);
            }
            T::PpElse => {
                let n = self.open(K::PreprocElse, f::ALTERNATIVE, C_NONE)?;
                self.tk(K::PreprocElse, K::THashElse)?;
                self.preproc_items(items)?;
                self.close(n);
            }
            _ => {}
        }
        Ok(())
    }

    fn preproc_items(&mut self, items: Items) -> R<()> {
        match items {
            Items::Statements => self.statements(),
            Items::Declarations => self.members(),
            Items::TopLevel => {
                while !matches!(self.peek(), T::PpElif | T::PpElse | T::PpEndif | T::Eof) {
                    self.toplevel_item(true)?;
                }
                Ok(())
            }
        }
    }

    /// An `#if`'s or `#elif`'s condition, to its line's end.
    fn preproc_condition(&mut self) -> R<()> {
        let eol = self.lx.blocks.eol(self.last_end as usize) as u32;
        self.pp_binary(slot(f::CONDITION, Ctx::Bare), 0, eol)?;
        if self.tok().start < eol && self.peek() != T::Eof {
            return self.refuse();
        }
        Ok(())
    }

    fn pp_binary(&mut self, s: Slot, min: u8, eol: u32) -> R<N> {
        let mut left = self.pp_unary(s, eol)?;
        loop {
            let t = self.tok();
            if t.start >= eol {
                break;
            }
            let (p, op) = match t.kind {
                T::PipePipe => (4, K::TPipePipe),
                T::AmpAmp => (5, K::TAmpAmp),
                T::EqEq => (9, K::TEqEq),
                T::Ne => (9, K::TNe),
                _ => break,
            };
            if p < min {
                break;
            }
            left = self.wrap(left, K::BinaryExpression, s, slot(f::LEFT, Ctx::Bare));
            self.leaf(op, f::OPERATOR, C_NONE)?;
            self.pp_binary(slot(f::RIGHT, Ctx::Bare), p + 1, eol)?;
            self.close(left);
        }
        Ok(left)
    }

    fn pp_unary(&mut self, s: Slot, eol: u32) -> R<N> {
        self.deeper()?;
        let t = self.tok();
        if t.start >= eol {
            return self.refuse();
        }
        let n = match t.kind {
            T::Bang => {
                let n = self.open_in(K::UnaryExpression, s)?;
                self.tk_field(K::UnaryExpression, K::TBang, f::OPERATOR)?;
                self.pp_unary(slot(f::ARGUMENT, Ctx::Bare), eol)?;
                self.close(n);
                n
            }
            T::LParen => {
                let n = self.open_in(K::ParenthesizedExpression, s)?;
                self.tk(K::ParenthesizedExpression, K::TLParen)?;
                self.pp_binary(BARE, 0, eol)?;
                self.punct_tk(K::ParenthesizedExpression, T::RParen, false)?;
                self.close(n);
                n
            }
            T::Int => self.leaf_in(K::IntegerLiteral, s)?,
            T::Name if matches!(t.word, w::TRUE | w::FALSE) => {
                let n = self.open_in(K::BooleanLiteral, s)?;
                self.leaf(keyword(t.word).expect("a boolean"), f::NONE, C_NONE)?;
                self.close(n);
                n
            }
            T::Name if !w::is_reserved(t.word) => self.ident(s)?,
            _ => return self.refuse(),
        };
        self.depth -= 1;
        Ok(n)
    }
}
