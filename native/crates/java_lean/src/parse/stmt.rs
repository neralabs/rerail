//! Statements: blocks, control flow, `switch` and its labels and patterns, `try`, local
//! declarations, and the look ahead that tells a declaration from an expression statement.

use super::*;
use super::decl::Decl;

impl<'a> P<'a> {
    /// A statement in slot `s` (a `statement`'s, with a field), the only child of its kind there
    /// (`parent`): an empty statement's `;` is a token in `parent`'s gap.
    pub(super) fn statement_in(&mut self, parent: K, s: Slot) -> R<()> {
        if self.peek() == T::Semi {
            return self.tk_field(parent, K::TSemi, s.field);
        }
        self.statement(s).map(|_| ())
    }

    /// Statements to `}` (or `case`/`default`, in a switch group), each a `statement`. An empty
    /// statement's `;` is stored, and so is the gap it stands in. Whether the last gap holds one.
    fn statements(&mut self, group: bool) -> R<bool> {
        let mut keep = false;
        loop {
            match self.peek() {
                T::RBrace | T::Eof => return Ok(keep),
                T::Semi => {
                    self.leaf_in(K::TSemi, STMT)?;
                    keep = true;
                }
                T::Name if group && (self.is_word(w::CASE) || (self.is_word(w::DEFAULT) && matches!(self.kind_at(1), T::Colon | T::Arrow))) => return Ok(keep),
                _ => {
                    self.statement(STMT)?;
                    keep = false;
                }
            }
        }
    }

    /// `block`: `{` statements `}`.
    pub(super) fn block(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::Block, s);
        self.braced(K::Block, |p| p.statements(false))?;
        self.close(n);
        Ok(n)
    }

    /// `{`, a body read by `body`, `}`: the braces stored where the gaps they stand in hold an
    /// empty statement (`body` says whether the last one does).
    pub(super) fn braced(&mut self, kind: K, body: impl FnOnce(&mut Self) -> R<bool>) -> R<()> {
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        let first_semi = self.kind_at(1) == T::Semi;
        self.gap_tk(kind, K::TLBrace, first_semi)?;
        let keep = body(self)?;
        self.punct_tk(kind, T::RBrace, keep)
    }

    /// A statement in slot `s`.
    pub(super) fn statement(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let t = self.tok();
        let n = match t.kind {
            T::LBrace => self.block(s)?,
            T::Semi => self.leaf_in(K::TSemi, s)?,
            T::Name => match t.word {
                w::IF => self.if_statement(s)?,
                w::WHILE => {
                    let n = self.open_in(K::WhileStatement, s);
                    self.tk(K::WhileStatement, K::TWhile)?;
                    self.parenthesized(f::CONDITION)?;
                    self.statement_in(K::WhileStatement, slot(f::BODY, Ctx::Stmt))?;
                    self.close(n);
                    n
                }
                w::DO => {
                    let n = self.open_in(K::DoStatement, s);
                    self.tk(K::DoStatement, K::TDo)?;
                    self.statement_in(K::DoStatement, slot(f::BODY, Ctx::Stmt))?;
                    self.kw(K::DoStatement, w::WHILE)?;
                    self.parenthesized(f::CONDITION)?;
                    self.punct_tk(K::DoStatement, T::Semi, false)?;
                    self.close(n);
                    n
                }
                w::FOR => self.for_statement(s)?,
                w::TRY => self.try_statement(s)?,
                w::RETURN | w::THROW => {
                    let (kind, word) = if t.word == w::RETURN { (K::ReturnStatement, K::TReturn) } else { (K::ThrowStatement, K::TThrow) };
                    let n = self.open_in(kind, s);
                    self.tk(kind, word)?;
                    if kind == K::ThrowStatement || self.peek() != T::Semi {
                        self.expr(EXPR)?;
                    }
                    self.punct_tk(kind, T::Semi, false)?;
                    self.close(n);
                    n
                }
                w::BREAK | w::CONTINUE => {
                    let (kind, word) = if t.word == w::BREAK { (K::BreakStatement, K::TBreak) } else { (K::ContinueStatement, K::TContinue) };
                    let n = self.open_in(kind, s);
                    self.tk(kind, word)?;
                    if self.is_ident() {
                        self.leaf(K::Identifier, f::NONE, C_NONE)?;
                    }
                    self.punct_tk(kind, T::Semi, false)?;
                    self.close(n);
                    n
                }
                w::ASSERT => {
                    let n = self.open_in(K::AssertStatement, s);
                    self.tk(K::AssertStatement, K::TAssert)?;
                    self.expr(EXPR)?;
                    if self.peek() == T::Colon {
                        self.tk(K::AssertStatement, K::TColon)?;
                        self.expr(EXPR)?;
                    }
                    self.punct_tk(K::AssertStatement, T::Semi, false)?;
                    self.close(n);
                    n
                }
                w::SWITCH => self.switch_expression(s)?,
                w::SYNCHRONIZED if self.kind_at(1) == T::LParen => {
                    let n = self.open_in(K::SynchronizedStatement, s);
                    self.tk(K::SynchronizedStatement, K::TSynchronized)?;
                    self.parenthesized(f::NONE)?;
                    self.block(slot(f::BODY, Ctx::Bare))?;
                    self.close(n);
                    n
                }
                w::YIELD if self.yield_ahead() => {
                    let n = self.open_in(K::YieldStatement, s);
                    self.tk(K::YieldStatement, K::TYield)?;
                    self.expr(EXPR)?;
                    self.punct_tk(K::YieldStatement, T::Semi, false)?;
                    self.close(n);
                    n
                }
                _ if self.is_ident() && self.kind_at(1) == T::Colon => {
                    let n = self.open_in(K::LabeledStatement, s);
                    self.leaf(K::Identifier, f::NONE, C_NONE)?;
                    self.tk(K::LabeledStatement, K::TColon)?;
                    self.statement_in(K::LabeledStatement, STMT)?;
                    self.close(n);
                    n
                }
                _ => self.declaration_or_expression(s)?,
            },
            T::At | T::AtInterface => self.declaration_or_expression(s)?,
            _ => self.expression_statement(s)?,
        };
        self.depth -= 1;
        Ok(n)
    }

    /// Whether `yield` starts a `yield` statement: an expression follows it that does not
    /// continue `yield` as a name.
    fn yield_ahead(&mut self) -> bool {
        let t = self.peek_at(1);
        if super::expr::continues_name(t.kind) {
            return false;
        }
        true
    }

    /// A statement starting with a name or `@`: a declaration (local class, local variable) where
    /// one starts here, else an expression statement.
    fn declaration_or_expression(&mut self, s: Slot) -> R<N> {
        match self.declaration_ahead(0) {
            Some(Decl::Local) => self.local_variable_declaration(s),
            Some(Decl::Type(kind)) => self.type_declaration(kind, s),
            None => self.expression_statement(s),
        }
    }

    fn expression_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::ExpressionStatement, s);
        self.expr(EXPR)?;
        self.punct_tk(K::ExpressionStatement, T::Semi, false)?;
        self.close(n);
        Ok(n)
    }

    /// `local_variable_declaration`: modifiers, a type, declarators, `;`.
    pub(super) fn local_variable_declaration(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::LocalVariableDeclaration, s);
        self.modifiers()?;
        self.unannotated(slot(f::TYPE, Ctx::UType), true)?;
        self.declarators(K::LocalVariableDeclaration)?;
        self.punct_tk(K::LocalVariableDeclaration, T::Semi, false)?;
        self.close(n);
        Ok(n)
    }

    /// `variable_declarator`s between `,`s.
    pub(super) fn declarators(&mut self, parent: K) -> R<()> {
        loop {
            self.variable_declarator(f::DECLARATOR)?;
            if self.peek() != T::Comma {
                return Ok(());
            }
            self.tk(parent, K::TComma)?;
        }
    }

    /// `variable_declarator`: a name, dimensions, and `= value`.
    pub(super) fn variable_declarator(&mut self, field: u8) -> R<()> {
        let n = self.open(K::VariableDeclarator, field, C_NONE);
        self.declarator_id()?;
        if self.peek() == T::Eq {
            self.tk(K::VariableDeclarator, K::TEq)?;
            self.variable_initializer(slot(f::VALUE, Ctx::Expr))?;
        }
        self.close(n);
        Ok(())
    }

    /// `_variable_declarator_id`: a name (`_` an `underscore_pattern`) and dimensions.
    pub(super) fn declarator_id(&mut self) -> R<()> {
        let t = self.tok();
        if self.underscore(t) {
            self.leaf(K::UnderscorePattern, f::NAME, C_NONE)?;
        } else if self.is_ident() {
            self.leaf(K::Identifier, f::NAME, C_NONE)?;
        } else {
            return self.refuse();
        }
        if self.dims_ahead(0) {
            self.dimensions(f::DIMENSIONS)?;
        }
        Ok(())
    }

    /// `parenthesized_expression` (a condition).
    fn parenthesized(&mut self, field: u8) -> R<()> {
        if self.peek() != T::LParen {
            return self.refuse();
        }
        let n = self.open(K::ParenthesizedExpression, field, C_NONE);
        self.tk(K::ParenthesizedExpression, K::TLParen)?;
        self.expr(EXPR)?;
        self.punct_tk(K::ParenthesizedExpression, T::RParen, false)?;
        self.close(n);
        Ok(())
    }

    fn if_statement(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::IfStatement, s);
        self.tk(K::IfStatement, K::TIf)?;
        self.parenthesized(f::CONDITION)?;
        if self.peek() == T::Semi && self.is_word_at(1, w::ELSE) && self.kind_at(2) == T::Semi {
            // `if (c) ; else ;`: one gap, not its kind's implied word.
            self.leaf(K::TSemi, f::CONSEQUENCE, C_NONE)?;
            self.leaf(K::TElse, f::NONE, C_NONE)?;
            self.leaf(K::TSemi, f::ALTERNATIVE, C_NONE)?;
            self.close(n);
            return Ok(n);
        }
        self.statement_in(K::IfStatement, slot(f::CONSEQUENCE, Ctx::Stmt))?;
        if self.is_word(w::ELSE) {
            self.tk(K::IfStatement, K::TElse)?;
            self.statement_in(K::IfStatement, slot(f::ALTERNATIVE, Ctx::Stmt))?;
        }
        self.close(n);
        Ok(n)
    }

    fn for_statement(&mut self, s: Slot) -> R<N> {
        // `for (T x : e)`: past `(`, modifiers, a type and a name, a `:`.
        let enhanced = (|| {
            let k = self.scan_modifiers(2)?;
            let k = self.scan_type(k, true)?;
            let t = self.peek_at(k);
            if !(self.is_ident_at(k) || self.underscore(t)) {
                return None;
            }
            let mut k = k + 1;
            while self.dims_ahead(k) {
                k = self.scan_annotations(k)? + 2;
            }
            (self.kind_at(k) == T::Colon).then_some(())
        })()
        .is_some();
        if enhanced {
            let n = self.open_in(K::EnhancedForStatement, s);
            self.tk(K::EnhancedForStatement, K::TFor)?;
            self.punct_tk(K::EnhancedForStatement, T::LParen, false)?;
            self.modifiers()?;
            self.unannotated(slot(f::TYPE, Ctx::UType), true)?;
            self.declarator_id()?;
            self.punct_tk(K::EnhancedForStatement, T::Colon, false)?;
            self.expr(slot(f::VALUE, Ctx::Expr))?;
            self.punct_tk(K::EnhancedForStatement, T::RParen, false)?;
            self.statement_in(K::EnhancedForStatement, slot(f::BODY, Ctx::Stmt))?;
            self.close(n);
            return Ok(n);
        }
        let n = self.open_in(K::ForStatement, s);
        self.tk(K::ForStatement, K::TFor)?;
        self.punct_tk(K::ForStatement, T::LParen, false)?;
        if matches!(self.declaration_ahead_at(0), Some(Decl::Local)) {
            self.local_variable_declaration(slot(f::INIT, Ctx::Bare))?;
        } else {
            if self.peek() != T::Semi {
                loop {
                    self.expr(slot(f::INIT, Ctx::Expr))?;
                    if self.peek() != T::Comma {
                        break;
                    }
                    self.tk(K::ForStatement, K::TComma)?;
                }
            }
            self.punct_tk(K::ForStatement, T::Semi, false)?;
        }
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
                self.tk(K::ForStatement, K::TComma)?;
            }
        }
        self.punct_tk(K::ForStatement, T::RParen, false)?;
        self.statement_in(K::ForStatement, slot(f::BODY, Ctx::Stmt))?;
        self.close(n);
        Ok(n)
    }

    fn try_statement(&mut self, s: Slot) -> R<N> {
        let resources = self.kind_at(1) == T::LParen;
        let kind = if resources { K::TryWithResourcesStatement } else { K::TryStatement };
        let n = self.open_in(kind, s);
        self.tk(kind, K::TTry)?;
        if resources {
            self.resources()?;
        }
        self.block(slot(f::BODY, Ctx::Bare))?;
        let mut any = false;
        while self.is_word(w::CATCH) {
            let c = self.open(K::CatchClause, f::NONE, C_NONE);
            self.tk(K::CatchClause, K::TCatch)?;
            self.punct_tk(K::CatchClause, T::LParen, false)?;
            let p = self.open(K::CatchFormalParameter, f::NONE, C_NONE);
            self.modifiers()?;
            let t = self.open(K::CatchType, f::NONE, C_NONE);
            loop {
                self.unannotated(UTYPE, true)?;
                if self.peek() != T::Pipe {
                    break;
                }
                self.tk(K::CatchType, K::TPipe)?;
            }
            self.close(t);
            self.declarator_id()?;
            self.close(p);
            self.punct_tk(K::CatchClause, T::RParen, false)?;
            self.block(slot(f::BODY, Ctx::Bare))?;
            self.close(c);
            any = true;
        }
        if self.is_word(w::FINALLY) {
            let c = self.open(K::FinallyClause, f::NONE, C_NONE);
            self.tk(K::FinallyClause, K::TFinally)?;
            self.block(BARE)?;
            self.close(c);
            any = true;
        }
        if !resources && !any {
            return self.refuse();
        }
        self.close(n);
        Ok(n)
    }

    /// `resource_specification`: `(` resources between `;`s, a trailing `;` allowed `)`.
    fn resources(&mut self) -> R<()> {
        let n = self.open(K::ResourceSpecification, f::RESOURCES, C_NONE);
        self.tk(K::ResourceSpecification, K::TLParen)?;
        loop {
            let r = self.open(K::Resource, f::NONE, C_NONE);
            if matches!(self.declaration_ahead_at(0), Some(Decl::Local)) {
                self.modifiers()?;
                self.unannotated(slot(f::TYPE, Ctx::UType), true)?;
                self.declarator_id()?;
                self.punct_tk(K::Resource, T::Eq, false)?;
                self.expr(slot(f::VALUE, Ctx::Expr))?;
            } else {
                let e = self.primary(BARE)?;
                if !matches!(e.kind, K::Identifier | K::FieldAccess) {
                    return self.refuse();
                }
            }
            self.close(r);
            if self.peek() != T::Semi {
                break;
            }
            if self.kind_at(1) == T::RParen {
                self.leaf(K::TSemi, f::NONE, C_NONE)?;
                self.leaf(K::TRParen, f::NONE, C_NONE)?;
                self.close(n);
                return Ok(());
            }
            self.tk(K::ResourceSpecification, K::TSemi)?;
        }
        self.punct_tk(K::ResourceSpecification, T::RParen, false)?;
        self.close(n);
        Ok(())
    }

    // ---- Switch.

    /// `switch_expression`: `switch`, its condition, and its block of groups or rules.
    pub(super) fn switch_expression(&mut self, s: Slot) -> R<N> {
        let n = self.open_in(K::SwitchExpression, s);
        self.tk(K::SwitchExpression, K::TSwitch)?;
        self.parenthesized(f::CONDITION)?;
        let b = self.open(K::SwitchBlock, f::BODY, C_NONE);
        if self.peek() != T::LBrace {
            return self.refuse();
        }
        self.tk(K::SwitchBlock, K::TLBrace)?;
        let mut rules: Option<bool> = None;
        while self.peek() != T::RBrace {
            if !(self.is_word(w::CASE) || self.is_word(w::DEFAULT)) {
                return self.refuse();
            }
            let rule = self.rule_ahead();
            if *rules.get_or_insert(rule) != rule {
                return self.refuse();
            }
            if rule {
                let r = self.open(K::SwitchRule, f::NONE, C_NONE);
                self.switch_label()?;
                self.tk(K::SwitchRule, K::TArrow)?;
                match self.tok() {
                    t if t.kind == T::LBrace => {
                        self.block(BARE)?;
                    }
                    t if t.kind == T::Name && t.word == w::THROW => {
                        self.statement(BARE)?;
                    }
                    _ => {
                        self.expression_statement(BARE)?;
                    }
                }
                self.close(r);
            } else {
                // A group per label: tree-sitter reduces a group at each next label.
                let g = self.open(K::SwitchBlockStatementGroup, f::NONE, C_NONE);
                self.switch_label()?;
                if self.peek() != T::Colon {
                    return self.refuse();
                }
                self.tk(K::SwitchBlockStatementGroup, K::TColon)?;
                self.statements(true)?;
                self.close(g);
            }
        }
        self.tk(K::SwitchBlock, K::TRBrace)?;
        self.close(b);
        self.close(n);
        Ok(n)
    }

    /// Whether the label ahead ends in `->` (a rule) rather than `:` (a group): the first `:` or
    /// `->` outside brackets.
    fn rule_ahead(&mut self) -> bool {
        let mut k = 1;
        let mut depth = 0u32;
        // Conditionals open (`?` read, its `:` not yet).
        let mut open = 0u32;
        loop {
            match self.kind_at(k) {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => match depth.checked_sub(1) {
                    Some(d) => depth = d,
                    None => return false,
                },
                T::Arrow if depth == 0 => return true,
                T::Question if depth == 0 => open += 1,
                T::Colon if depth == 0 && open > 0 => open -= 1,
                T::Colon if depth == 0 => return false,
                T::Eof => return false,
                _ => {}
            }
            k += 1;
        }
    }

    /// `switch_label`: `case` a pattern or expressions and a guard, or `default`.
    fn switch_label(&mut self) -> R<()> {
        let n = self.open(K::SwitchLabel, f::NONE, C_NONE);
        if self.is_word(w::DEFAULT) {
            self.tk(K::SwitchLabel, K::TDefault)?;
            self.close(n);
            return Ok(());
        }
        self.tk(K::SwitchLabel, K::TCase)?;
        if self.record_pattern_ahead(0) || self.type_pattern_ahead(0) {
            let p = self.open(K::Pattern, f::NONE, C_NONE);
            if self.record_pattern_ahead(0) {
                self.record_pattern(BARE)?;
            } else {
                self.type_pattern()?;
            }
            self.close(p);
        } else {
            let saved = std::mem::replace(&mut self.no_lambda, true);
            let r = (|| {
                loop {
                    self.expr(EXPR)?;
                    if self.peek() != T::Comma {
                        return Ok(());
                    }
                    self.tk(K::SwitchLabel, K::TComma)?;
                }
            })();
            self.no_lambda = saved;
            r?;
        }
        if self.is_word(w::WHEN) {
            let g = self.open(K::Guard, f::NONE, C_NONE);
            self.tk(K::Guard, K::TWhen)?;
            let saved = std::mem::replace(&mut self.no_lambda, true);
            let r = self.expr(EXPR);
            self.no_lambda = saved;
            r?;
            self.close(g);
        }
        self.close(n);
        Ok(())
    }

    /// Whether a type pattern starts `k` ahead: a type, then a name.
    fn type_pattern_ahead(&mut self, k: usize) -> bool {
        self.scan_type(k, true).is_some_and(|e| self.is_ident_at(e) && !self.is_word_at(e, w::WHEN) || self.is_word_at(e, w::WHEN) && matches!(self.kind_at(e + 1), T::Arrow | T::Colon))
    }

    /// `type_pattern`: a type and a name.
    fn type_pattern(&mut self) -> R<()> {
        let n = self.open(K::TypePattern, f::NONE, C_NONE);
        self.unannotated(UTYPE, true)?;
        if !self.is_ident() {
            return self.refuse();
        }
        self.leaf(K::Identifier, f::NONE, C_NONE)?;
        self.close(n);
        Ok(())
    }

    /// Whether a record pattern starts `k` ahead: a name (or a generic type) and parentheses
    /// that hold patterns alone.
    pub(super) fn record_pattern_ahead(&mut self, k: usize) -> bool {
        if !self.is_ident_at(k) {
            return false;
        }
        let mut k = k + 1;
        if self.kind_at(k) == T::Lt {
            let Some(e) = self.scan_type_arguments(k) else { return false };
            k = e;
        }
        if self.kind_at(k) != T::LParen {
            return false;
        }
        self.pattern_list_ahead(k).is_some()
    }

    /// From a `(` `k` ahead: patterns between `,`s and `)` (`_`, a type and a name, a record
    /// pattern); just past the `)`.
    fn pattern_list_ahead(&mut self, k: usize) -> Option<usize> {
        let mut k = k + 1;
        if self.kind_at(k) == T::RParen {
            return Some(k + 1);
        }
        loop {
            let t = self.peek_at(k);
            if self.underscore(t) && matches!(self.kind_at(k + 1), T::Comma | T::RParen) {
                k += 1;
            } else if self.record_pattern_ahead(k) {
                let mut j = k + 1;
                if self.kind_at(j) == T::Lt {
                    j = self.scan_type_arguments(j)?;
                }
                k = self.pattern_list_ahead(j)?;
            } else {
                let e = self.scan_type(k, true)?;
                if !self.is_ident_at(e) {
                    return None;
                }
                k = e + 1;
            }
            match self.kind_at(k) {
                T::Comma => k += 1,
                T::RParen => return Some(k + 1),
                _ => return None,
            }
        }
    }

    /// `record_pattern`: a name (or generic type) and its body of patterns.
    pub(super) fn record_pattern(&mut self, s: Slot) -> R<()> {
        let n = self.open_in(K::RecordPattern, s);
        if self.kind_at(1) == T::Lt {
            let g = self.open(K::GenericType, f::NONE, C_NONE);
            self.leaf(K::TypeIdentifier, f::NONE, C_NONE)?;
            self.type_arguments(f::NONE)?;
            self.close(g);
        } else {
            self.leaf(K::Identifier, f::NONE, C_NONE)?;
        }
        let b = self.open(K::RecordPatternBody, f::NONE, C_NONE);
        self.list(K::RecordPatternBody, T::RParen, true, false, |p| {
            let t = p.tok();
            if p.underscore(t) && matches!(p.kind_at(1), T::Comma | T::RParen) {
                let c = p.open(K::RecordPatternComponent, f::NONE, C_NONE);
                p.leaf(K::UnderscorePattern, f::NONE, C_NONE)?;
                p.close(c);
            } else if p.record_pattern_ahead(0) {
                p.record_pattern(BARE)?;
            } else {
                let c = p.open(K::RecordPatternComponent, f::NONE, C_NONE);
                p.unannotated(UTYPE, true)?;
                if !p.is_ident() {
                    return p.refuse();
                }
                p.leaf(K::Identifier, f::NONE, C_NONE)?;
                p.close(c);
            }
            Ok(())
        })?;
        self.close(b);
        self.close(n);
        Ok(())
    }
}
