//! Expressions (`expression` and its forms), blocks, strings and literals. Each reader returns the
//! node it wrote (its place), the caller giving it its field and class.
//!
//! An infix expression is read by precedence climbing over the operators' classes (`op_class`:
//! an alphanumeric operator 7, an assignment operator 6, then by the first character, `|` 8 to the
//! other characters 16; right-associative where the operator ends in `:`). Its operands are
//! prefix (17) or simple expressions: a simple expression takes calls, fields, type arguments and
//! `_` after it (18).

use super::{EXPR_CLASS, P, Region};
use crate::kinds::{K, f};
use crate::lex::{R, T, Tok, op, op_class, w};
use crate::scan::{Ext, V, v};

/// The external tokens an operand's state takes after it: an infix operator, a postfix one.
const AFTER_OPERAND: V = v::OPS | v::POSTFIX_OP | v::POSTFIX_STAR;

impl P<'_> {
    // ---- Names.

    /// Whether `t` is a name an identifier reads (a word not reserved, a back-quoted name; `this`
    /// and `super` are identifiers too).
    pub(super) fn is_name(t: Tok) -> bool {
        match t.kind {
            T::Id => !w::is_reserved(t.code) || matches!(t.code, w::THIS | w::SUPER),
            T::BackId => true,
            _ => false,
        }
    }

    /// `_identifier`: a name (`identifier`) or an operator (`operator_identifier`), given `field`.
    pub(super) fn identifier(&mut self, field: u8) -> R<u32> {
        let t = self.tok();
        // `extension` (`_soft_identifier`): the keyword's token in an identifier.
        if t.kind == T::Id && t.code == w::EXTENSION {
            return self.extension_identifier(field);
        }
        if Self::is_name(t) {
            return self.leaf(K::Identifier, field);
        }
        if t.kind == T::Op && !op::is_reserved(t.code) && t.code != op::ARROW {
            return self.leaf(K::OperatorIdentifier, field);
        }
        self.refuse()
    }

    /// A name as a leaf of `kind` (`type_identifier`, …).
    pub(super) fn ident_as(&mut self, kind: K, field: u8) -> R<u32> {
        let t = self.tok();
        if t.kind == T::Id && t.code == w::EXTENSION && kind == K::Identifier && self.keyword_after_dot(t) {
            // `a.extension.B` in a path: an identifier holding the keyword's token.
            return self.extension_identifier(field);
        }
        if t.kind == T::Id && t.code == w::EXTENSION && kind != K::Identifier {
            // A type named `extension`: refused.
            return self.refuse();
        }
        if Self::is_name(t) || t.kind == T::Op && !op::is_reserved(t.code) && t.code != op::ARROW {
            return self.leaf(kind, field);
        }
        self.refuse()
    }

    /// Whether `t` is `extension` after a `.` (`a.extension.B`), where tree-sitter reads its keyword.
    fn keyword_after_dot(&self, t: Tok) -> bool {
        t.kind == T::Id && t.code == w::EXTENSION && self.i > 0 && self.toks[self.i - 1].kind == T::Dot
    }

    /// `extension` read as a name: an `identifier` holding the keyword's token.
    pub(super) fn extension_identifier(&mut self, field: u8) -> R<u32> {
        let n = self.open_field(K::Identifier, field);
        self.tk(K::TExtension)?;
        Ok(self.close(n))
    }

    // ---- Expressions.

    /// `expression`, given `field`, in the `{expression}` class: its place.
    pub(super) fn expression_f(&mut self, field: u8) -> R<u32> {
        self.expression_m(field, false)
    }

    /// `expression_f`, a `match` after it left to the caller where `stop_match` (an assignment's
    /// right side: `a = b match {…}` matches the assignment).
    fn expression_m(&mut self, field: u8, stop_match: bool) -> R<u32> {
        self.deeper()?;
        let saved = std::mem::replace(&mut self.right_operand, false);
        let n = self.expression(stop_match);
        self.right_operand = saved;
        let n = n?;
        self.depth -= 1;
        if field != f::NONE {
            self.t.refield(n, field);
        }
        // `do_while_expression` is `_indentable_expression`'s, not in `expression`: no class.
        if self.t.kind_at(n) != K::DoWhileExpression {
            self.t.set_class(n, EXPR_CLASS);
        }
        Ok(n)
    }

    /// `_indentable_expression`, given `field`: an indented block (or cases) where an indent comes
    /// next, else an expression.
    pub(super) fn indentable_f(&mut self, field: u8) -> R<u32> {
        if self.tok().nl {
            // An expression must come: what may follow the construct (`fol`) is not valid here.
            match self.with_fol(0, |p| p.layout(v::INDENT | v::INLINE | v::XML))? {
                Some(Ext::Indent) => {
                    let at = self.take();
                    return self.indented_block(field, at);
                }
                Some(Ext::Modifier(_) | Ext::Xml) | None => {}
                Some(_) => return self.refuse(),
            }
        }
        self.expression_f(field)
    }

    /// The indent read at `at`: an indented block (`indented_block`) or cases (`indented_cases`),
    /// to its outdent.
    pub(super) fn indented_block(&mut self, field: u8, at: u32) -> R<u32> {
        if self.is_word(w::CASE) && !matches!(self.word_at(1), w::CLASS | w::OBJECT) {
            let n = self.t.open(K::IndentedCases, field, at);
            self.with_fol(v::OUTDENT | v::COMMA_OUTDENT, |p| {
                // To the outdent: a `case` after it is an enclosing match's.
                while p.is_word(w::CASE) && !matches!(p.layout(v::OUTDENT | v::COMMA_OUTDENT)?, Some(Ext::Outdent | Ext::CommaOutdent)) {
                    p.case_clause(Region::Indent)?;
                }
                Ok(())
            })?;
            return self.outdent(n);
        }
        let n = self.t.open(K::IndentedBlock, field, at);
        self.statements(Region::Indent, f::NONE)?;
        self.outdent(n)
    }

    /// The outdent (or a comma's) closing node `n`.
    fn outdent(&mut self, n: crate::tw::Node) -> R<u32> {
        match self.layout(v::OUTDENT | v::COMMA_OUTDENT)? {
            Some(Ext::Outdent | Ext::CommaOutdent) => {
                let at = self.take();
                Ok(self.close_at(n, at))
            }
            _ => self.refuse(),
        }
    }

    /// `expression`.
    fn expression(&mut self, stop_match: bool) -> R<u32> {
        let t = self.tok();
        if t.kind == T::Id {
            match t.code {
                w::IF => return self.if_expression(),
                w::WHILE => return self.while_expression(),
                w::FOR => return self.for_expression(),
                w::TRY => return self.try_expression(),
                w::RETURN => return self.return_expression(),
                w::THROW => return self.throw_expression(),
                w::DO => return self.do_while_expression(),
                w::MACRO => {
                    // `macro a.f` (`macro_body`, its operand unfielded): its precedence is above a
                    // call's and a generic function's, so the operand takes the `.` steps and
                    // `[T]` or `(…)` after it apply over the macro body.
                    let m = self.mark();
                    let n = self.open(K::MacroBody);
                    self.tk(K::TMacro)?;
                    let o = self.mark();
                    let mut x = self.atom()?;
                    while self.peek() == T::Dot && !self.tok().nl {
                        self.t.refield(x, f::VALUE);
                        let fx = self.t.wrap(o, K::FieldExpression, f::NONE);
                        self.tk(K::TDot)?;
                        self.identifier(f::FIELD)?;
                        x = self.close(fx);
                    }
                    self.t.unmark(o);
                    let n = self.close(n);
                    let r = self.suffixes(m, n);
                    self.t.unmark(m);
                    return r;
                }
                _ => {}
            }
        }
        if t.kind == T::Op && t.code == op::NONE && self.text(t) == b"<" && !self.xml_ahead() {
            return self.refuse();
        }
        if self.lambda_ahead() {
            return self.lambda_expression();
        }
        if t.kind == T::LBracket {
            // `[T] => (x: T) => e`: a polymorphic lambda.
            let close = self.matching(0);
            if self.op_at(close + 1) == op::ARROW {
                return self.lambda_expression();
            }
            return self.refuse();
        }
        let m = self.mark();
        let mut n = self.infix(0)?;
        loop {
            let t = self.tok();
            if t.kind == T::Id && t.code == w::MATCH {
                if stop_match {
                    break;
                }
                if t.nl && self.layout(AFTER_OPERAND)?.is_some() {
                    break;
                }
                self.t.refield(n, f::VALUE);
                self.t.set_class(n, EXPR_CLASS);
                let x = self.t.wrap(m, K::MatchExpression, f::NONE);
                self.tk(K::TMatch)?;
                self.match_body()?;
                self.kw_end_marker(w::MATCH, K::TMatch)?;
                n = self.close(x);
                continue;
            }
            if t.kind == T::Op && t.code == op::EQ && !t.nl {
                let k = self.t.kind_at(n);
                if matches!(k, K::InfixExpression | K::MatchExpression | K::AscriptionExpression | K::PostfixExpression) {
                    return self.refuse();
                }
                self.t.refield(n, f::LEFT);
                let x = self.t.wrap(m, K::AssignmentExpression, f::NONE);
                self.tk(K::TEq)?;
                self.indentable_assign()?;
                n = self.close(x);
                // `a = b match {…}`: the match is over the assignment (its value an expression).
                if self.is_word(w::MATCH) {
                    continue;
                }
                break;
            }
            if t.kind == T::Op && t.code == op::COLON && !t.nl {
                if let Some(eol) = self.colon_argument_ahead()? {
                    // A colon argument after an infix expression (`"a" -> b:`): the call's
                    // function is the expression so far.
                    self.t.refield(n, f::FUNCTION);
                    let x = self.t.wrap(m, K::CallExpression, f::NONE);
                    self.colon_argument(eol)?;
                    n = self.close(x);
                    continue;
                }
                self.t.refield(n, f::NONE);
                let x = self.t.wrap(m, K::AscriptionExpression, f::NONE);
                self.tk(K::TColon)?;
                if self.is_op(op::AT) {
                    while self.is_op(op::AT) {
                        self.annotation()?;
                    }
                } else if self.is_word(w::UNDERSCORE) && self.is_op_at(1, op::STAR) {
                    // `a op b: _*` (not a vararg: its expression is no simple one): the type a
                    // repeated wildcard.
                    let r = self.open_field(K::RepeatedParameterType, f::TYPE);
                    let wc = self.open_field(K::Wildcard, f::TYPE);
                    self.tk(K::TUnderscore)?;
                    self.close(wc);
                    self.tk(K::TStar)?;
                    self.close(r);
                } else {
                    self.param_type(f::TYPE)?;
                }
                n = self.close(x);
                continue;
            }
            break;
        }
        self.t.unmark(m);
        Ok(n)
    }

    /// An assignment's right side: an expression, or an indented block.
    fn indentable_assign(&mut self) -> R<u32> {
        if self.tok().nl
            && let Some(Ext::Indent) = self.layout(v::INDENT)? {
                let at = self.take();
                return self.indented_block(f::RIGHT, at);
            }
        self.expression_m(f::RIGHT, true)
    }

    /// An infix expression whose operators bind tighter than `min` (`infix_expression`: `left`,
    /// `operator`, `right`), or its operand.
    fn infix(&mut self, min: u8) -> R<u32> {
        let m = self.mark();
        let mut n = self.prefix()?;
        loop {
            let t = self.tok();
            let (prec, right) = match t.kind {
                T::Op if !op::is_reserved(t.code) && t.code != op::ARROW && t.code != op::QARROW => op_class(self.text(t)),
                T::Id if t.code == w::NONE || !w::is_reserved(t.code) && !matches!(t.code, w::IF | w::ELSE | w::THEN | w::CATCH | w::WITH | w::FORSOME | w::END | w::DERIVES | w::USES) => (7, false),
                T::BackId => (7, false),
                _ => break,
            };
            if prec < min {
                break;
            }
            if t.nl {
                // A line break before the operator: the scanner decides whether it continues.
                if self.layout(AFTER_OPERAND)?.is_some() {
                    break;
                }
            }
            // An operator with no operand after it on its line: postfix (refused).
            let nx = self.peek_at(1);
            if matches!(nx.kind, T::RParen | T::RBracket | T::RBrace | T::Comma | T::Semi | T::Eof) || nx.nl {
                if t.kind == T::Id || t.kind == T::BackId {
                    // `10 seconds`, `a op b c`: a postfix expression (its operand and identifier
                    // unfielded), where the operand is all the expression so far (an
                    // `_infix_operand`, an infix expression too).
                    if min != 0 || self.right_operand {
                        return self.refuse();
                    }
                    self.t.refield(n, f::NONE);
                    let x = self.t.wrap(m, K::PostfixExpression, f::NONE);
                    self.leaf(K::Identifier, f::NONE)?;
                    n = self.close(x);
                    break;
                }
                if self.layout(AFTER_OPERAND)?.is_some_and(|x| matches!(x, Ext::PostfixOp | Ext::PostfixStar)) {
                    return self.refuse();
                }
                if !nx.nl {
                    return self.refuse();
                }
            }
            if t.kind == T::Id && !matches!(nx.kind, T::Id | T::BackId | T::Int | T::Float | T::Str | T::Char | T::LParen | T::LBrace | T::Op | T::Quote) {
                return self.refuse();
            }
            self.t.refield(n, f::LEFT);
            let x = self.t.wrap(m, K::InfixExpression, f::NONE);
            if t.kind == T::Op {
                self.leaf(K::OperatorIdentifier, f::OPERATOR)?;
            } else {
                self.leaf(K::Identifier, f::OPERATOR)?;
            }
            let saved = std::mem::replace(&mut self.right_operand, true);
            let r = self.infix(if right { prec } else { prec + 1 });
            self.right_operand = saved;
            self.t.refield(r?, f::RIGHT);
            n = self.close(x);
            if self.peek() == T::LBracket && !self.right_operand && !self.tok().nl {
                // `a op b[T]`: the type arguments are the infix's, then its calls and fields.
                n = self.suffixes(m, n)?;
            }
        }
        self.t.unmark(m);
        Ok(n)
    }

    /// A prefix expression (`-x`, `!x`, `+x`, `~x`: `prefix_expression`) or a simple one.
    fn prefix(&mut self) -> R<u32> {
        let t = self.tok();
        if t.kind == T::Op && matches!(t.code, op::MINUS | op::PLUS | op::BANG | op::TILDE) {
            let nx = self.peek_at(1);
            if t.code == op::MINUS && matches!(nx.kind, T::Int | T::Float) && nx.start == t.end {
                return self.simple();
            }
            if matches!(nx.kind, T::RParen | T::Comma | T::RBrace | T::RBracket | T::Eof) || nx.kind == T::Op && op::is_reserved(nx.code) {
                // The operator alone (`reduce(_ + _)`'s `+`): an operator name.
                return self.simple();
            }
            // Type arguments after the operand are the prefix expression's (`!x.is[T]`), or an
            // infix expression's around it.
            let m = self.mark();
            let n = self.open(K::PrefixExpression);
            let k = super::op_kind(t.code).expect("a prefix operator");
            self.tk(k)?;
            self.deeper()?;
            let saved = std::mem::replace(&mut self.right_operand, true);
            let r = self.simple();
            self.right_operand = saved;
            r?;
            self.depth -= 1;
            let mut n = self.close(n);
            if self.peek() == T::LBracket && !self.right_operand && !self.tok().nl {
                n = self.suffixes(m, n)?;
            }
            self.t.unmark(m);
            return Ok(n);
        }
        self.simple()
    }

    /// `_simple_expression`: an atom, then calls (`call_expression`), fields (`field_expression`),
    /// type arguments (`generic_function`) and `_` (`method_value`) after it.
    fn simple(&mut self) -> R<u32> {
        let m = self.mark();
        let n = self.atom()?;
        let n = self.suffixes(m, n)?;
        self.t.unmark(m);
        Ok(n)
    }

    /// The suffixes after `n` (read since mark `m`): fields, calls, type arguments. In an infix
    /// expression's right operand type arguments are not its (`a op b[T]` is the infix's).
    fn suffixes(&mut self, m: crate::tw::Mark, mut n: u32) -> R<u32> {
        loop {
            let t = self.tok();
            match t.kind {
                T::Dot | T::LParen | T::LBracket | T::LBrace if t.nl
                    && self.layout(AFTER_OPERAND | v::COLON_EOL)?.is_some() => {
                        break;
                    }
                _ => {}
            }
            match t.kind {
                T::Dot => {
                    let nx = self.peek_at(1);
                    if nx.kind == T::Id && nx.code == w::MATCH {
                        return self.refuse();
                    }
                    if !(Self::is_name(nx) || nx.kind == T::Op && !op::is_reserved(nx.code)) {
                        return self.refuse();
                    }
                    self.t.refield(n, f::VALUE);
                    let x = self.t.wrap(m, K::FieldExpression, f::NONE);
                    self.tk(K::TDot)?;
                    self.identifier(f::FIELD)?;
                    n = self.close(x);
                }
                T::LParen => {
                    self.t.refield(n, f::FUNCTION);
                    let x = self.t.wrap(m, K::CallExpression, f::NONE);
                    self.arguments(f::ARGUMENTS)?;
                    n = self.close(x);
                }
                T::LBrace => {
                    self.t.refield(n, f::FUNCTION);
                    let x = self.t.wrap(m, K::CallExpression, f::NONE);
                    let b = self.block_or_cases()?;
                    self.t.refield(b, f::ARGUMENTS);
                    n = self.close(x);
                }
                T::LBracket if self.right_operand => break,
                T::LBracket => {
                    self.t.refield(n, f::FUNCTION);
                    self.t.set_class(n, EXPR_CLASS);
                    let x = self.t.wrap(m, K::GenericFunction, f::NONE);
                    self.type_arguments(f::TYPE_ARGUMENTS)?;
                    n = self.close(x);
                }
                T::Id if t.code == w::UNDERSCORE && !t.nl => {
                    // `f _`: a method value (the expression and a wildcard, unfielded).
                    self.t.refield(n, f::NONE);
                    let x = self.t.wrap(m, K::MethodValue, f::NONE);
                    let y = self.open(K::Wildcard);
                    self.tk(K::TUnderscore)?;
                    self.close(y);
                    n = self.close(x);
                    break;
                }
                T::Op if t.code == op::COLON && !t.nl && !self.right_operand => {
                    let Some(eol) = self.colon_argument_ahead()? else { break };
                    self.t.refield(n, f::FUNCTION);
                    let x = self.t.wrap(m, K::CallExpression, f::NONE);
                    self.colon_argument(eol)?;
                    n = self.close(x);
                }
                _ => break,
            }
        }
        Ok(n)
    }

    /// Whether a colon argument (fewer braces) comes next: `Some(true)` for `:` at the line's end,
    /// `Some(false)` for `: x =>` (or `_`, `(a, b)`, `?=>`) at the line's end.
    fn colon_argument_ahead(&mut self) -> R<Option<bool>> {
        if self.layout(v::COLON_EOL | AFTER_OPERAND)? == Some(Ext::ColonEol) {
            return Ok(Some(true));
        }
        let t = self.peek_at(1);
        let j = match t.kind {
            T::Id | T::BackId if t.code == w::UNDERSCORE || Self::is_name(t) => 2,
            T::LParen => self.matching(1) + 1,
            _ => return Ok(None),
        };
        let a = self.peek_at(j);
        let arrow = a.kind == T::Op && !a.nl && matches!(a.code, op::ARROW | op::QARROW);
        Ok((arrow && self.peek_at(j + 1).nl).then_some(false))
    }

    /// `:` then a colon argument (`colon_argument`, field `arguments`: a lambda's start in
    /// `lambda_start`, then an indented block or cases); `eol` when the `:` ends the line.
    fn colon_argument(&mut self, eol: bool) -> R<()> {
        if eol {
            self.take();
        }
        self.tk(K::TColon)?;
        let c = self.open_field(K::ColonArgument, f::ARGUMENTS);
        if !eol {
            self.lambda_parameters(f::LAMBDA_START)?;
            let code = self.op_at(0);
            self.tkf(if code == op::QARROW { K::TQFatArrow } else { K::TFatArrow }, f::LAMBDA_START)?;
        }
        match self.layout(v::INDENT)? {
            Some(Ext::Indent) => {
                let at = self.take();
                self.indented_block(f::NONE, at)?;
            }
            _ => return self.refuse(),
        }
        self.close(c);
        Ok(())
    }

    /// A simple expression's first part.
    fn atom(&mut self) -> R<u32> {
        let t = self.tok();
        match t.kind {
            T::Id => match t.code {
                w::UNDERSCORE => {
                    let n = self.open(K::Wildcard);
                    self.tk(K::TUnderscore)?;
                    Ok(self.close(n))
                }
                w::TRUE | w::FALSE | w::NULL => self.literal(),
                w::NEW => self.instance_expression(),
                _ if self.text(t) == b"$" && matches!(self.kind_at(1), T::LBrace | T::LBracket) && self.adjacent(0) => self.splice_expression(),
                _ if Self::is_name(t) => {
                    if self.kind_at(1) == T::Str && self.adjacent(0) {
                        return self.interpolated_string_expression();
                    }
                    if t.code == w::EXTENSION {
                        return self.extension_identifier(f::NONE);
                    }
                    self.leaf(K::Identifier, f::NONE)
                }
                _ => self.refuse(),
            },
            T::BackId => self.leaf(K::Identifier, f::NONE),
            T::Int | T::Float | T::Char | T::Str => self.literal(),
            T::Op if t.code == op::MINUS && matches!(self.kind_at(1), T::Int | T::Float) && self.adjacent(0) => self.literal(),
            T::Op if self.xml_ahead() => self.xml_expression(),
            T::Op if !op::is_reserved(t.code) && t.code != op::ARROW => self.leaf(K::OperatorIdentifier, f::NONE),
            T::LParen => self.paren_expression(),
            T::LBrace => self.block_or_cases(),
            T::Quote => self.quote_expression(),
            _ => self.refuse(),
        }
    }

    /// `'a`, `'{ … }`, `'[T]` (`quote_expression`: a name, statements, or a type; the `'` and
    /// the brackets tokens).
    pub(super) fn quote_expression(&mut self) -> R<u32> {
        let t = self.tok();
        let nx = self.peek_at(1);
        if nx.start != t.end {
            return self.refuse();
        }
        let n = self.open(K::QuoteExpression);
        match nx.kind {
            T::LBrace => {
                self.tk(K::TQuote)?;
                self.tk(K::TLBrace)?;
                self.statements(Region::Brace, f::NONE)?;
                self.expect(T::RBrace)?;
            }
            T::LBracket => {
                self.tk(K::TQuote)?;
                self.tk(K::TLBracket)?;
                if self.is_word(w::TYPE) {
                    // Type definitions before the type: refused.
                    return self.refuse();
                }
                self.with_fol(0, |p| p.typ_x(f::NONE, super::types::Tx::default()))?;
                self.expect(T::RBracket)?;
            }
            T::Id if matches!(nx.code, w::NULL | w::TRUE | w::FALSE) => {
                self.tk(K::TQuote)?;
                self.literal()?;
            }
            _ if Self::is_name(nx) => {
                self.tk(K::TQuote)?;
                self.identifier(f::NONE)?;
            }
            _ => return self.refuse(),
        }
        Ok(self.close(n))
    }

    /// `${ … }`, `$[T]` (`splice_expression`: statements or a type; `${` and `$[` one token
    /// each).
    fn splice_expression(&mut self) -> R<u32> {
        let n = self.open(K::SpliceExpression);
        let d = self.bump()?;
        let b = self.bump()?;
        if b.kind == T::LBrace {
            self.t.token(K::TDollarLBrace, f::NONE, d.start, b.end);
            self.statements(Region::Brace, f::NONE)?;
            self.expect(T::RBrace)?;
        } else {
            self.t.token(K::TX24X5b, f::NONE, d.start, b.end);
            self.with_fol(0, |p| p.typ_x(f::NONE, super::types::Tx::default()))?;
            self.expect(T::RBracket)?;
        }
        Ok(self.close(n))
    }

    /// `()` (`unit`), `(e)` (`parenthesized_expression`), `(a, b)` (`tuple_expression`).
    fn paren_expression(&mut self) -> R<u32> {
        if self.kind_at(1) == T::RParen {
            let n = self.open(K::Unit);
            self.tk(K::TLParen)?;
            self.tk(K::TRParen)?;
            return Ok(self.close(n));
        }
        let m = self.mark();
        let s = self.tok().start;
        let _ = s;
        self.tk(K::TLParen)?;
        let mut count = 0;
        self.with_fol(0, |p| {
            loop {
                p.expression_f(f::NONE)?;
                count += 1;
                if p.peek() == T::Comma {
                    p.tk(K::TComma)?;
                    if p.peek() == T::RParen {
                        break;
                    }
                } else {
                    break;
                }
            }
            p.expect(T::RParen)?;
            Ok(())
        })?;
        let kind = if count == 1 { K::ParenthesizedExpression } else { K::TupleExpression };
        let n = self.t.wrap(m, kind, f::NONE);
        let n = self.close(n);
        self.t.unmark(m);
        Ok(n)
    }

    /// `(a, b)` (`arguments`), given `field`.
    pub(super) fn arguments(&mut self, field: u8) -> R<u32> {
        let n = self.open_field(K::Arguments, field);
        self.tk(K::TLParen)?;
        self.with_fol(0, |p| {
            if p.is_word(w::USING) {
                p.tk(K::TUsing)?;
            }
            while p.peek() != T::RParen {
                // `xs: _*`, `xs*`: varargs.
                match p.vararg_ahead() {
                    Some(0) => {
                        // `xs: _*` (`vararg`): the `_*` left out.
                        let n = p.open(K::Vararg);
                        p.simple()?;
                        if !p.is_op(op::COLON) {
                            return p.refuse();
                        }
                        p.tk(K::TColon)?;
                        // A token of no name: not on the tape, the node over it.
                        let t = p.tok();
                        let e = if p.text(t) == b"_*" { p.bump()? } else { p.bump()?; p.bump()? };
                        p.t.last_end = e.end;
                        p.close(n);
                    }
                    Some(_) => {
                        // `xs*` (`vararg`: the `*` the scanner's, of no name).
                        let n = p.open(K::Vararg);
                        p.simple()?;
                        if !(p.is_op(op::STAR) && matches!(p.kind_at(1), T::RParen | T::Comma)) {
                            return p.refuse();
                        }
                        let e = p.bump()?;
                        p.t.last_end = e.end;
                        p.close(n);
                    }
                    None => {
                        p.expression_f(f::NONE)?;
                    }
                }
                if p.peek() == T::Comma {
                    p.tk(K::TComma)?;
                } else {
                    break;
                }
            }
            p.expect(T::RParen)?;
            Ok(())
        })?;
        Ok(self.close(n))
    }

    /// Whether a vararg is next: `Some(0)` for `xs: _*`, `Some(1)` for a postfix `*` (`xs*`), to
    /// the argument's end.
    fn vararg_ahead(&mut self) -> Option<u8> {
        let (mut d, mut j) = (0i32, 0);
        loop {
            let t = self.peek_at(j);
            match t.kind {
                T::LParen | T::LBracket | T::LBrace => d += 1,
                T::RParen | T::RBracket | T::RBrace if d > 0 => d -= 1,
                T::RParen | T::Comma if d == 0 => {
                    if j == 0 {
                        return None;
                    }
                    let last = self.peek_at(j - 1);
                    if last.kind == T::Id && self.text(last) == b"_*" && j >= 2 && self.op_at(j - 2) == op::COLON {
                        return Some(0);
                    }
                    if self.op_at(j - 1) == op::STAR {
                        let before = self.peek_at(j - 2);
                        return if j >= 3 && self.op_at(j - 3) == op::COLON && before.kind == T::Id && before.code == w::UNDERSCORE && last.start == before.end {
                            // `a op b: _*` is an ascription (`repeated_parameter_type`), not a
                            // vararg: a vararg's expression is a simple one.
                            if self.simple_before(j - 3) { Some(0) } else { None }
                        } else {
                            Some(1)
                        };
                    }
                    return None;
                }
                T::Eof => return None,
                T::RParen | T::RBracket | T::RBrace => return None,
                _ => {}
            }
            j += 1;
            if j > 64 {
                return None;
            }
        }
    }

    /// Whether the tokens before `k` are one simple expression (`_simple_expression`): names and
    /// literals apart by `.` or touching, brackets after them; no operator.
    fn simple_before(&mut self, k: usize) -> bool {
        let (mut d, mut prev) = (0i32, false);
        for j in 0..k {
            let t = self.peek_at(j);
            match t.kind {
                T::LParen | T::LBracket | T::LBrace => d += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    d -= 1;
                    if d == 0 {
                        prev = true;
                    }
                }
                _ if d > 0 => {}
                T::Dot => prev = false,
                T::Op => return false,
                _ => {
                    if prev && !(j > 0 && self.adjacent(j - 1)) {
                        return false;
                    }
                    prev = true;
                }
            }
        }
        true
    }

    // ---- Blocks.

    /// `{ … }`: a block (`block`), or cases (`case_block`).
    pub(super) fn block_or_cases(&mut self) -> R<u32> {
        if self.word_at(1) == w::CASE && !matches!(self.word_at(2), w::CLASS | w::OBJECT) {
            return self.case_block();
        }
        self.block()
    }

    /// `{ … }` (`block`): statements, or a lambda over them (`{ x => … }`).
    pub(super) fn block(&mut self) -> R<u32> {
        let n = self.open(K::Block);
        self.tk(K::TLBrace)?;
        self.i_lambda_block()?;
        self.expect(T::RBrace)?;
        Ok(self.close(n))
    }

    /// A block's inside: a lambda over statements (`{ x => a; b }`), or statements.
    fn i_lambda_block(&mut self) -> R<()> {
        if self.typed_lambda_ahead() {
            // `{ x: T => body }` (`_braced_typed_lambda`, a lambda of no class): the name, `:` and
            // type in `parameters`, then an indentable expression.
            let l = self.open(K::LambdaExpression);
            if self.is_word(w::IMPLICIT) {
                self.tkf(K::TImplicit, f::PARAMETERS)?;
            }
            self.identifier(f::PARAMETERS)?;
            self.tkf(K::TColon, f::PARAMETERS)?;
            self.typ_x(f::PARAMETERS, super::types::Tx { no_arrow: true })?;
            let code = self.op_at(0);
            self.tk(if code == op::QARROW { K::TQFatArrow } else { K::TFatArrow })?;
            self.indentable_f(f::NONE)?;
            self.close(l);
            if self.peek() != T::RBrace {
                return self.refuse();
            }
            return Ok(());
        }
        if self.peek() != T::RBrace && self.lambda_ahead() {
            let l = self.open(K::LambdaExpression);
            self.lambda_parameters(f::PARAMETERS)?;
            let code = self.op_at(0);
            self.tk(if code == op::QARROW { K::TQFatArrow } else { K::TFatArrow })?;
            if self.tok().nl && self.peek() != T::RBrace {
                // An indent after the arrow: the block's statement is a lambda over an indented
                // block (`lambda_expression`'s dynamic precedence), statements after it the block's.
                if let Some(Ext::Indent) = self.layout(v::INDENT)? {
                    let at = self.take();
                    self.indented_block(f::NONE, at)?;
                    let id = self.close(l);
                    self.t.set_class(id, EXPR_CLASS);
                    return self.statements(Region::Brace, f::NONE);
                }
            }
            self.count = 0;
            self.trailing = None;
            if self.peek() != T::RBrace {
                self.statements(Region::Brace, f::NONE)?;
            }
            // One statement: the block's statement is a lambda expression (its dynamic precedence);
            // none, more, or one and a separator after it: the block's lambda
            // (`_block_lambda_expression`), of no class, to the separator's end.
            let one = self.count == 1 && self.trailing.is_none();
            let id = match self.trailing {
                Some(e) if !one => {
                    let e = e.max(self.t.last_end);
                    self.close_at(l, e)
                }
                _ => self.close(l),
            };
            if one {
                self.t.set_class(id, EXPR_CLASS);
            }
            return Ok(());
        }
        self.statements(Region::Brace, f::NONE)
    }

    /// Whether `name: T =>` comes next on the line (a typed lambda in braces).
    fn typed_lambda_ahead(&mut self) -> bool {
        // `implicit x: T =>` too.
        let k = usize::from(self.is_word(w::IMPLICIT));
        let t = self.peek_at(k);
        if !(Self::is_name(t) && t.code != w::UNDERSCORE && self.is_op_at(k + 1, op::COLON)) {
            return false;
        }
        let mut d = 0i32;
        for j in k + 2..k + 64 {
            let t = self.peek_at(j);
            if t.nl || t.kind == T::Eof {
                return false;
            }
            match t.kind {
                T::LParen | T::LBracket | T::LBrace => d += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    d -= 1;
                    if d < 0 {
                        return false;
                    }
                }
                T::Op if d == 0 && matches!(t.code, op::ARROW | op::QARROW) => return true,
                T::Op if d == 0 && t.code == op::EQ => return false,
                _ => {}
            }
        }
        false
    }

    /// `{ case … }` (`case_block`).
    fn case_block(&mut self) -> R<u32> {
        let n = self.open(K::CaseBlock);
        self.tk(K::TLBrace)?;
        self.with_fol(0, |p| {
            while p.is_word(w::CASE) {
                p.case_clause(Region::Brace)?;
            }
            Ok(())
        })?;
        self.expect(T::RBrace)?;
        Ok(self.close(n))
    }

    /// `case p if g => body` (`case_clause`: `pattern`, a guard, `body` on each statement).
    pub(super) fn case_clause(&mut self, region: Region) -> R<()> {
        let n = self.open(K::CaseClause);
        self.tk(K::TCase)?;
        self.pattern_f(f::PATTERN)?;
        if self.is_word(w::IF) {
            self.guard()?;
        }
        if !self.is_op(op::ARROW) {
            return self.refuse();
        }
        self.tk(K::TFatArrow)?;
        self.case_body(region)?;
        self.close(n);
        Ok(())
    }

    /// A case's statements: to the next `case`, the `}` or the outdent.
    fn case_body(&mut self, region: Region) -> R<()> {
        let fol = match region {
            Region::Indent => v::SEMI | v::OUTDENT | v::COMMA_OUTDENT,
            _ => v::SEMI,
        };
        self.with_fol(fol, |p| {
            loop {
                while p.peek() == T::Semi {
                    p.tkf(K::TSemi, f::BODY)?;
                }
                if p.peek() == T::RBrace || p.peek() == T::Eof {
                    return Ok(());
                }
                if p.is_word(w::CASE) && !matches!(p.word_at(1), w::CLASS | w::OBJECT) {
                    return Ok(());
                }
                if region == Region::Indent
                    && let Some(Ext::Outdent | Ext::CommaOutdent) = p.layout(fol)? {
                        return Ok(());
                    }
                p.deeper()?;
                p.statement(f::BODY)?;
                p.depth -= 1;
                match p.layout(fol)? {
                    Some(Ext::Semi) => {
                        // A semicolon the scanner gives is the clause's (before the `}`, or before a
                        // `case` whose arrow is on a later line: its last child), the comments
                        // before it too.
                        let at = p.take();
                        p.t.flush(at);
                        p.t.last_end = p.t.last_end.max(at);
                    }
                    Some(Ext::Outdent | Ext::CommaOutdent) if region == Region::Indent => return Ok(()),
                    Some(_) => return p.refuse(),
                    None => {}
                }
            }
        })
    }

    /// `if c` (`guard`: `condition`).
    fn guard(&mut self) -> R<()> {
        let n = self.open(K::Guard);
        self.tk(K::TIf)?;
        let c = self.postfix_choice()?;
        self.t.refield(c, f::CONDITION);
        self.close(n);
        Ok(())
    }

    /// `_postfix_expression_choice`: an infix operand (or a postfix expression).
    fn postfix_choice(&mut self) -> R<u32> {
        self.infix(0)
    }

    // ---- Control.

    /// `if (c) a else b`, `if c then a else b` (`if_expression`: `condition`, `consequence`,
    /// `alternative`).
    fn if_expression(&mut self) -> R<u32> {
        let n = self.open(K::IfExpression);
        self.tk(K::TIf)?;
        // `if (c) a`: the parenthesized condition's form, unless a `then` follows (the one form
        // that parses: `if (a) - 1 then …` is the other).
        let paren = self.peek() == T::LParen && !self.then_ahead();
        let fol = self.fol;
        if paren {
            let c = self.paren_expression()?;
            if self.t.kind_at(c) != K::ParenthesizedExpression {
                return self.refuse();
            }
            self.t.refield(c, f::CONDITION);
            if self.tok().nl {
                // `_automatic_semicolon?` before the body.
                if let Some(Ext::Semi) = self.layout(v::SEMI | v::INDENT)? {
                    return self.refuse();
                }
            }
        } else {
            self.with_fol(v::SEMI | v::GATE, |p| p.indentable_f(f::CONDITION))?;
            match self.layout(v::SEMI | v::GATE)? {
                Some(Ext::Semi) => {
                    self.take();
                    if self.layout(v::GATE)? != Some(Ext::Gate) {
                        return self.refuse();
                    }
                    self.take();
                }
                Some(Ext::Gate) => {
                    self.take();
                }
                _ => return self.refuse(),
            }
            // `then` is in the condition's field (`condition: seq(…, "then")`).
            if !self.is_word(w::THEN) {
                return self.refuse();
            }
            self.tkf(K::TThen, f::CONDITION)?;
        }
        self.with_fol(fol | v::GATE, |p| p.indentable_f(f::CONSEQUENCE))?;
        if self.peek() == T::Semi && self.word_at(1) == w::ELSE {
            return self.refuse();
        }
        if self.layout(v::GATE)? == Some(Ext::Gate) {
            self.take();
            self.kw(w::ELSE, K::TElse)?;
            self.indentable_f(f::ALTERNATIVE)?;
        }
        self.kw_end_marker(w::IF, K::TIf)?;
        Ok(self.close(n))
    }

    /// Whether a `then` comes after the condition: at its depth, before its line's end or a
    /// closing bracket.
    fn then_ahead(&mut self) -> bool {
        let mut d = 0i32;
        for j in 0..256 {
            let t = self.peek_at(j);
            if j > 0 && t.nl && d == 0 {
                return t.kind == T::Id && t.code == w::THEN;
            }
            match t.kind {
                T::LParen | T::LBracket | T::LBrace => d += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    d -= 1;
                    if d < 0 {
                        return false;
                    }
                }
                T::Id if t.code == w::THEN && d == 0 => return true,
                T::Semi | T::Eof if d == 0 => return false,
                T::Eof => return false,
                _ => {}
            }
        }
        false
    }

    /// The token index of the bracket closing the one `k` ahead.
    pub(super) fn matching(&mut self, k: usize) -> usize {
        let (mut d, mut j) = (0i32, k);
        loop {
            match self.kind_at(j) {
                T::LParen | T::LBracket | T::LBrace => d += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    d -= 1;
                    if d == 0 {
                        return j;
                    }
                }
                T::Eof => return j,
                _ => {}
            }
            j += 1;
        }
    }

    /// `while (c) body`, `while c do body` (`while_expression`: `condition`, `body`).
    /// `end match` (`if`, `while`, `for`, `try`, `new`) on the line after the construct
    /// (`_end_marker_kw_tail`: the semicolon, then an `end_marker`: the scanner's `end` unwritten,
    /// then the keyword's token), inside the construct's node.
    pub(super) fn kw_end_marker(&mut self, word: u8, kind: K) -> R<()> {
        if !(self.tok().nl && self.is_word(w::END)) {
            return Ok(());
        }
        let nx = self.peek_at(1);
        if nx.nl || nx.kind != T::Id || nx.code != word {
            return Ok(());
        }
        if self.with_fol(0, |p| p.layout(v::SEMI))? != Some(Ext::Semi) {
            return Ok(());
        }
        self.take();
        if self.layout(v::END_KW)? != Some(Ext::EndKw) {
            return self.refuse();
        }
        self.take();
        let n = self.open(K::EndMarker);
        let e = self.bump()?;
        self.t.last_end = e.end;
        self.tk(kind)?;
        self.close(n);
        Ok(())
    }

    /// `do body while (c)` (`do_while_expression`: `body`, `condition`).
    fn do_while_expression(&mut self) -> R<u32> {
        let n = self.open(K::DoWhileExpression);
        self.tk(K::TDo)?;
        self.with_fol(0, |p| p.expression_f(f::BODY))?;
        if !self.is_word(w::WHILE) {
            return self.refuse();
        }
        self.tk(K::TWhile)?;
        if self.peek() != T::LParen {
            return self.refuse();
        }
        let c = self.paren_expression()?;
        if self.t.kind_at(c) != K::ParenthesizedExpression {
            return self.refuse();
        }
        self.t.refield(c, f::CONDITION);
        Ok(self.close(n))
    }

    fn while_expression(&mut self) -> R<u32> {
        let n = self.open(K::WhileExpression);
        self.tk(K::TWhile)?;
        if self.peek() != T::LParen || {
            let close = self.matching(0);
            self.word_at(close + 1) == w::DO
        } {
            // `while c do body` (Scala 3): the condition indentable, an automatic semicolon
            // before `do` taken, `do` in the condition's field.
            let fol = self.fol;
            self.with_fol(0, |p| p.indentable_f(f::CONDITION))?;
            if self.with_fol(0, |p| p.layout(v::SEMI))? == Some(Ext::Semi) {
                self.take();
            }
            if !self.is_word(w::DO) {
                return self.refuse();
            }
            self.tkf(K::TDo, f::CONDITION)?;
            self.with_fol(fol, |p| p.indentable_f(f::BODY))?;
            self.kw_end_marker(w::WHILE, K::TWhile)?;
            return Ok(self.close(n));
        }
        let c = self.paren_expression()?;
        if self.t.kind_at(c) != K::ParenthesizedExpression {
            return self.refuse();
        }
        self.t.refield(c, f::CONDITION);
        if self.tok().nl
            && let Some(Ext::Semi) = self.layout(v::SEMI)? {
                return self.refuse();
            }
        self.expression_f(f::BODY)?;
        Ok(self.close(n))
    }

    /// `for (…) body`, `for { … } yield body` (`for_expression`: the brackets' tokens and
    /// `enumerators` given `enumerators`, then `body`). With `(` (named at run time) the engine stores
    /// every token of it: they are written as leaves. `for x <- xs do …` (no brackets) is refused.
    fn for_expression(&mut self) -> R<u32> {
        let n = self.open(K::ForExpression);
        let paren = match self.kind_at(1) {
            T::LParen => true,
            T::LBrace => false,
            _ => return self.bare_for(n),
        };
        self.stored(K::TFor, f::NONE, paren)?;
        self.stored(if paren { K::TLParen } else { K::TLBrace }, f::ENUMERATORS, paren)?;
        let close = if paren { T::RParen } else { T::RBrace };
        self.with_fol(0, |p| p.enumerators(Some(close)))?;
        if self.peek() != close {
            return self.refuse();
        }
        self.stored(if paren { K::TRParen } else { K::TRBrace }, f::ENUMERATORS, paren)?;
        let fol = self.fol;
        if self.is_word(w::YIELD) || self.tok().nl && self.word_at(0) == w::YIELD {
            match self.layout(v::GATE | v::INDENT)? {
                Some(Ext::Gate) => {
                    self.take();
                }
                None => {}
                Some(_) => return self.refuse(),
            }
            if !self.is_word(w::YIELD) {
                return self.refuse();
            }
            self.stored(K::TYield, f::NONE, paren)?;
            self.indentable_f(f::BODY)?;
        } else if self.is_word(w::DO) && !self.tok().nl {
            self.stored(K::TDo, f::NONE, paren)?;
            self.indentable_f(f::BODY)?;
        } else {
            self.with_fol(fol, |p| p.indentable_f(f::BODY))?;
        }
        Ok(self.close(n))
    }

    /// The next token as `kind` given `field`: a leaf where `all` (its parent stores every token).
    pub(super) fn stored(&mut self, kind: K, field: u8, all: bool) -> R<()> {
        if all {
            let t = self.bump()?;
            self.t.leaf(kind, field, t.start, t.end);
        } else {
            self.tkf(kind, field)?;
        }
        Ok(())
    }

    /// `enumerators` to `close`: enumerators apart by semicolons (the scanner's at line ends), in an
    /// indent where the scanner gives one first; a semicolon before `close` is theirs.
    /// `for x <- xs yield e`, `for` and indented enumerators, then `yield` or `do` (the gate before
    /// either at the `for`'s column).
    fn bare_for(&mut self, n: crate::tw::Node) -> R<u32> {
        self.tk(K::TFor)?;
        let fol = self.fol;
        self.with_fol(0, |p| p.enumerators(None))?;
        // After the enumerators only the gate is valid (the `for` is not complete).
        match self.with_fol(0, |p| p.layout(v::GATE))? {
            Some(Ext::Gate) => {
                self.take();
            }
            None => {}
            Some(_) => return self.refuse(),
        }
        match self.word() {
            w::YIELD => self.tk(K::TYield)?,
            w::DO => self.tk(K::TDo)?,
            _ => return self.refuse(),
        };
        self.with_fol(fol, |p| p.indentable_f(f::BODY))?;
        self.kw_end_marker(w::FOR, K::TFor)?;
        Ok(self.close(n))
    }

    /// Whether the enumerators end here: at `close`, or (`None`, a bare `for`) at `yield` or `do`.
    fn enumerators_end(&mut self, close: Option<T>) -> bool {
        match close {
            Some(c) => self.peek() == c,
            None => matches!(self.word(), w::YIELD | w::DO),
        }
    }

    fn enumerators(&mut self, close: Option<T>) -> R<()> {
        let mut indent = false;
        if self.tok().nl {
            match self.layout(v::INDENT)? {
                Some(Ext::Indent) => {
                    self.take();
                    indent = true;
                }
                None => {}
                Some(_) => return self.refuse(),
            }
        }
        let n = self.open_field(K::Enumerators, f::ENUMERATORS);
        let fol = if indent { v::SEMI | v::OUTDENT } else { v::SEMI };
        self.with_fol(fol, |p| {
            loop {
                p.enumerator()?;
                if p.peek() == T::Semi {
                    p.tk(K::TSemi)?;
                    if p.enumerators_end(close) {
                        break;
                    }
                    continue;
                }
                match p.layout(fol)? {
                    Some(Ext::Semi) => {
                        let at = p.take();
                        if p.enumerators_end(close) {
                            p.t.flush(at);
                            p.t.last_end = p.t.last_end.max(at);
                            break;
                        }
                    }
                    Some(Ext::Outdent) if indent => break,
                    None if p.enumerators_end(close) => break,
                    _ => return p.refuse(),
                }
            }
            Ok(())
        })?;
        if indent {
            match self.layout(v::OUTDENT)? {
                Some(Ext::Outdent) => {
                    let at = self.take();
                    self.close_at(n, at);
                }
                _ => return self.refuse(),
            }
        } else {
            self.close(n);
        }
        Ok(())
    }

    /// `p <- e`, `p = e`, `case p <- e`, guards after it; or guards alone (`enumerator`).
    fn enumerator(&mut self) -> R<()> {
        let n = self.open(K::Enumerator);
        if self.is_word(w::IF) {
            // Guards alone: the first may start the line.
            self.guard()?;
        } else {
            if self.is_word(w::CASE) {
                self.tk(K::TCase)?;
            }
            self.pattern_f(f::NONE)?;
            let t = self.tok();
            match (t.kind, t.code) {
                (T::Op, op::LARROW) => self.tk(K::TLArrow)?,
                (T::Op, op::EQ) => self.tk(K::TEq)?,
                _ => return self.refuse(),
            };
            self.indentable_f(f::NONE)?;
        }
        while self.is_word(w::IF) && !self.tok().nl {
            self.guard()?;
        }
        // A guard on the next line is an enumerator of its own (after the scanner's semicolon).
        self.close(n);
        Ok(())
    }

    /// `try body catch … finally …` (`try_expression`: `body`).
    fn try_expression(&mut self) -> R<u32> {
        let n = self.open(K::TryExpression);
        self.tk(K::TTry)?;
        let fol = self.fol;
        self.with_fol(fol | v::GATE, |p| p.indentable_f(f::BODY))?;
        if self.layout(v::GATE)? == Some(Ext::Gate) && self.word_at(0) == w::CATCH {
            self.take();
            let c = self.open(K::CatchClause);
            self.tk(K::TCatch)?;
            if self.is_word(w::CASE) && !self.tok().nl {
                // `catch case p => body` (`_expr_case_clause`: `pattern`, `body`).
                self.tk(K::TCase)?;
                self.pattern_f(f::PATTERN)?;
                if self.is_word(w::IF) {
                    self.guard()?;
                }
                if !self.is_op(op::ARROW) {
                    return self.refuse();
                }
                self.tk(K::TFatArrow)?;
                self.with_fol(fol | v::GATE, |p| p.indentable_f(f::BODY))?;
            } else {
                self.with_fol(fol | v::GATE, |p| p.indentable_f(f::NONE))?;
            }
            self.close(c);
        }
        if self.layout(v::GATE)? == Some(Ext::Gate) && self.word_at(0) == w::FINALLY {
            self.take();
            let c = self.open(K::FinallyClause);
            self.tk(K::TFinally)?;
            self.indentable_f(f::NONE)?;
            self.close(c);
        }
        self.kw_end_marker(w::TRY, K::TTry)?;
        Ok(self.close(n))
    }

    /// `return e` (`return_expression`).
    fn return_expression(&mut self) -> R<u32> {
        let n = self.open(K::ReturnExpression);
        self.tk(K::TReturn)?;
        let t = self.tok();
        if !t.nl && !matches!(t.kind, T::RBrace | T::RParen | T::Semi | T::Eof | T::Comma) {
            self.expression_f(f::NONE)?;
        }
        Ok(self.close(n))
    }

    /// `throw e` (`throw_expression`).
    fn throw_expression(&mut self) -> R<u32> {
        let n = self.open(K::ThrowExpression);
        self.tk(K::TThrow)?;
        self.expression_f(f::NONE)?;
        Ok(self.close(n))
    }

    /// `match { cases }`'s body (`case_block`, field `body`), or indented cases.
    fn match_body(&mut self) -> R<()> {
        if self.peek() == T::LBrace {
            let b = self.case_block()?;
            self.t.refield(b, f::BODY);
            return Ok(());
        }
        if self.tok().nl
            && let Some(Ext::Indent) = self.layout(v::INDENT)? {
                let at = self.take();
                if !self.is_word(w::CASE) {
                    return self.refuse();
                }
                self.indented_block(f::BODY, at)?;
                return Ok(());
            }
        self.refuse()
    }

    // ---- Lambdas.

    /// Whether a lambda starts here: `x =>`, `_ =>`, `implicit x =>`, `(…) =>`.
    pub(super) fn lambda_ahead(&mut self) -> bool {
        let t = self.tok();
        let k = match t.kind {
            T::Id if t.code == w::IMPLICIT => {
                if !Self::is_name(self.peek_at(1)) {
                    return false;
                }
                2
            }
            T::Id | T::BackId if Self::is_name(t) || t.code == w::UNDERSCORE => 1,
            T::LParen => self.matching(0) + 1,
            _ => return false,
        };
        matches!(self.op_at(k), op::ARROW | op::QARROW)
    }

    /// `params => body` (`lambda_expression`: `parameters`).
    fn lambda_expression(&mut self) -> R<u32> {
        let n = self.open(K::LambdaExpression);
        if self.peek() == T::LBracket {
            // `[T] => (x: T) => e`: the type parameters, `=>`, then the lambda's parameters.
            self.type_parameters(f::TYPE_PARAMETERS)?;
            if !self.is_op(op::ARROW) {
                return self.refuse();
            }
            self.tk(K::TFatArrow)?;
        }
        self.lambda_parameters(f::PARAMETERS)?;
        let code = self.op_at(0);
        self.tk(if code == op::QARROW { K::TQFatArrow } else { K::TFatArrow })?;
        self.indentable_f(f::NONE)?;
        Ok(self.close(n))
    }

    /// A lambda's parameters: `x`, `_`, `implicit x`, `(a, b: T)` (`bindings`).
    fn lambda_parameters(&mut self, field: u8) -> R<()> {
        let t = self.tok();
        match t.kind {
            T::Id if t.code == w::UNDERSCORE => {
                let n = self.open_field(K::Wildcard, field);
                self.tk(K::TUnderscore)?;
                self.close(n);
            }
            T::Id if t.code == w::IMPLICIT => {
                self.tkf(K::TImplicit, field)?;
                self.identifier(field)?;
            }
            T::Id | T::BackId => {
                self.identifier(field)?;
            }
            T::LParen => {
                let n = self.open_field(K::Bindings, field);
                self.tk(K::TLParen)?;
                while self.peek() != T::RParen {
                    let b = self.open(K::Binding);
                    if self.is_word(w::UNDERSCORE) {
                        // A wildcard binding has no field (`binding`'s `name` is the identifier's).
                        let x = self.open(K::Wildcard);
                        self.tk(K::TUnderscore)?;
                        self.close(x);
                    } else {
                        self.identifier(f::NAME)?;
                    }
                    if self.is_op(op::COLON) {
                        self.tk(K::TColon)?;
                        self.param_type(f::TYPE)?;
                    }
                    self.close(b);
                    if self.peek() == T::Comma {
                        self.tk(K::TComma)?;
                    } else {
                        break;
                    }
                }
                self.expect(T::RParen)?;
                self.close(n);
            }
            _ => return self.refuse(),
        }
        Ok(())
    }

    // ---- `new`.

    /// `new C(a) { … }` (`instance_expression`).
    fn instance_expression(&mut self) -> R<u32> {
        let n = self.open(K::InstanceExpression);
        self.tk(K::TNew)?;
        if self.peek() == T::LBrace {
            // `new { … }`: a template body alone.
            self.template_body(f::NONE)?;
            return Ok(self.close(n));
        }
        if self.is_op(op::COLON) && !self.tok().nl && self.layout(v::COLON_EOL)? == Some(Ext::ColonEol) {
            // `new:` and an indented body: the template body alone.
            self.indented_template_body(f::NONE)?;
            return Ok(self.close(n));
        }
        let m = self.mark();
        let mut b = self.annotated_type()?;
        if self.peek() == T::LParen && !self.tok().nl && {
            let c = self.matching(0);
            let t = self.peek_at(c + 1);
            t.kind == T::Id && t.code == w::WITH && !t.nl
        } {
            // `new A(x) with B`: the base an `applied_constructor_type` (type, arguments; no fields).
            self.t.refield(b, f::NONE);
            let a = self.t.wrap(m, K::AppliedConstructorType, f::NONE);
            self.arguments(f::NONE)?;
            b = self.close(a);
        }
        // `with` on the next line continues the type where the scanner gives no separator.
        if self.is_word(w::WITH) && self.continues(v::WITH | AFTER_OPERAND)? {
            // `new A with B { … }`: a compound type (`base`, `extra`), its `with`s left out.
            self.t.refield(b, f::BASE);
            let c = self.t.wrap(m, K::CompoundType, f::NONE);
            while self.is_word(w::WITH) && self.continues(v::WITH | AFTER_OPERAND)? {
                self.tk(K::TWith)?;
                let x = self.annotated_type()?;
                self.t.refield(x, f::EXTRA);
            }
            self.close(c);
            self.t.unmark(m);
            if self.peek() == T::LParen {
                return self.refuse();
            }
        } else {
            self.t.unmark(m);
            // One argument list: a second is a call of the instance (`new A(x)(y)`).
            if self.peek() == T::LParen && !self.tok().nl {
                self.arguments(f::ARGUMENTS)?;
            }
            if self.is_word(w::WITH) {
                return self.refuse();
            }
        }
        if self.peek() == T::LBrace && !self.tok().nl {
            self.template_body(f::NONE)?;
        } else if self.is_op(op::COLON) && !self.tok().nl && self.layout(v::COLON_EOL | AFTER_OPERAND)? == Some(Ext::ColonEol) {
            // `new A(x):` and an indented body: the template body's.
            self.indented_template_body(f::NONE)?;
        }
        Ok(self.close(n))
    }

    /// `_constructor_application`: a type, its arguments after it (`arguments`, each).
    pub(super) fn constructor_application(&mut self, field: u8) -> R<()> {
        // In a clause of them (`extends A with B`), `with` is the clause's: an annotated type.
        let t = self.annotated_type()?;
        if field != f::NONE {
            self.t.refield(t, field);
        }
        while self.peek() == T::LParen && !self.tok().nl {
            self.arguments(f::ARGUMENTS)?;
        }
        Ok(())
    }

    // ---- Literals and strings.

    /// A literal: a number (`-` before it), a character, a string, `true`, `false`, `null`.
    pub(super) fn literal(&mut self) -> R<u32> {
        let t = self.tok();
        match t.kind {
            T::Op => {
                // `-1`: the `-` is the literal's.
                let nx = self.peek_at(1);
                let kind = if nx.kind == T::Int { K::IntegerLiteral } else { K::FloatingPointLiteral };
                self.bump()?;
                let e = self.bump()?;
                Ok(self.t.leaf(kind, f::NONE, t.start, e.end))
            }
            T::Int => self.leaf(K::IntegerLiteral, f::NONE),
            T::Float => self.leaf(K::FloatingPointLiteral, f::NONE),
            T::Char => self.leaf(K::CharacterLiteral, f::NONE),
            T::Str => self.string(),
            T::Id if t.code == w::TRUE || t.code == w::FALSE => {
                let n = self.open(K::BooleanLiteral);
                self.tk(if t.code == w::TRUE { K::TTrue } else { K::TFalse })?;
                Ok(self.close(n))
            }
            T::Id if t.code == w::NULL => {
                self.leaf(K::NullLiteral, f::NONE)
            }
            _ => self.refuse(),
        }
    }

    /// A plain string (`string`): its escapes (`escape_sequence`) in it; a multiline one has none.
    fn string(&mut self) -> R<u32> {
        let t = self.tok();
        let s = t.start as usize;
        let n = self.open(K::String);
        let multi = self.src[s..].starts_with(b"\"\"\"");
        let mut i = s + if multi { 3 } else { 1 };
        let end = if multi {
            self.multiline_end(i)?
        } else {
            loop {
                let stop = self.lx.blocks.string_stop(i);
                let eol = self.lx.blocks.eol(i);
                if eol < stop || stop >= self.src.len() {
                    return self.refuse();
                }
                match self.src[stop] {
                    b'"' => break stop + 1,
                    b'\\' => {
                        let e = self.escape_end(stop)?;
                        self.t.leaf(K::EscapeSequence, f::NONE, stop as u32, e as u32);
                        i = e;
                    }
                    _ => i = stop + 1,
                }
            }
        };
        // The lexer's token is the whole string.
        if end != t.end as usize {
            return self.refuse();
        }
        self.bump()?;
        Ok(self.close(n))
    }

    /// A multiline string's end from `i`: past the first `"""` (and the quotes right after it).
    fn multiline_end(&mut self, mut i: usize) -> R<usize> {
        loop {
            let stop = self.lx.blocks.string_stop(i);
            if stop >= self.src.len() {
                return self.refuse();
            }
            if self.src[stop] == b'"' && self.src[stop..].starts_with(b"\"\"\"") {
                let mut e = stop + 3;
                while self.lx.at(e) == b'"' {
                    e += 1;
                }
                return Ok(e);
            }
            i = stop + 1;
        }
    }

    /// The end of the escape at `i` (its `\`): `\` and one of `tbnrf"'\`, `u`s and 4 hex digits,
    /// 1 to 3 octal digits, or any other character but a line end.
    fn escape_end(&mut self, i: usize) -> R<usize> {
        let c = self.lx.at(i + 1);
        Ok(match c {
            b'u' | b'U' => {
                let mut j = i + 1;
                while matches!(self.lx.at(j), b'u' | b'U') {
                    j += 1;
                }
                if (0..4).all(|k| self.lx.at(j + k).is_ascii_hexdigit()) {
                    j + 4
                } else {
                    i + 2
                }
            }
            b'0'..=b'7' => {
                let mut j = i + 2;
                let max = if c <= b'3' { 3 } else { 2 };
                while j < i + 1 + max && matches!(self.lx.at(j), b'0'..=b'7') {
                    j += 1;
                }
                j
            }
            b'\n' | b'\r' | 0 => return self.refuse(),
            c if c >= 0x80 => i + 1 + crate::lex::decode(self.src, i + 1).map_or(1, |(_, n)| n),
            _ => i + 2,
        })
    }

    /// `id"…"` (`interpolated_string_expression`: `interpolator`, then `interpolated_string`).
    pub(super) fn interpolated_string_expression(&mut self) -> R<u32> {
        let n = self.open(K::InterpolatedStringExpression);
        let raw = self.is_word(w::RAW);
        self.leaf(K::Identifier, f::INTERPOLATOR)?;
        let t = self.tok();
        let multi = self.src[t.start as usize..].starts_with(b"\"\"\"");
        let open = if multi { 3 } else { 1 };
        let s = self.open(K::InterpolatedString);
        // The lexer's token is the whole string: its opening quote is written here, its inside
        // read again below.
        self.bump()?;
        if !raw {
            self.t.token(if multi { K::TTripleQuote } else { K::TDQuote }, f::NONE, t.start, t.start + open);
        }
        let mut i = (t.start + open) as usize;
        let mut interpolated = false;
        let end = loop {
            let stop = self.lx.blocks.string_stop(i);
            if stop >= self.src.len() {
                return self.refuse();
            }
            if !multi && self.lx.blocks.eol(i) < stop {
                return self.refuse();
            }
            match self.src[stop] {
                b'"' if !multi => break stop + 1,
                b'"' if self.src[stop..].starts_with(b"\"\"\"") => {
                    let mut e = stop + 3;
                    while self.lx.at(e) == b'"' {
                        e += 1;
                    }
                    break e;
                }
                b'"' => i = stop + 1,
                b'\\' if multi => i = stop + 1,
                b'\\' if raw => i = if matches!(self.lx.at(stop + 1), b'"' | b'\\') { stop + 2 } else { stop + 1 },
                b'\\' => {
                    let e = self.escape_end(stop)?;
                    self.t.leaf(K::EscapeSequence, f::NONE, stop as u32, e as u32);
                    i = e;
                }
                _ => {
                    // `$`.
                    match self.lx.at(stop + 1) {
                        b'$' | b'"' => {
                            self.t.leaf(K::EscapeSequence, f::NONE, stop as u32, stop as u32 + 2);
                            i = stop + 2;
                        }
                        b'{' => {
                            let x = self.t.open(K::Interpolation, f::NONE, stop as u32);
                            self.relex_at(stop + 1);
                            self.t.last_end = stop as u32 + 1;
                            self.sp = stop as u32 + 1;
                            self.with_fol(0, |p| p.block())?;
                            self.close(x);
                            i = self.t.last_end as usize;
                            // What the look ahead lexed past the `}` is the string's.
                            self.relex_at(i);
                            interpolated = true;
                        }
                        c if c.is_ascii_alphabetic() || c == b'_' => {
                            let e = self.lx.blocks.word_end(stop + 1);
                            // `$name`: the name stops at `$` (the interpolation identifier has none).
                            let e = match self.src[stop + 1..e].iter().position(|&b| b == b'$') {
                                Some(k) => stop + 1 + k,
                                None => e,
                            };
                            if self.lx.at(e) >= 0x80 {
                                return self.refuse();
                            }
                            let x = self.t.open(K::Interpolation, f::NONE, stop as u32);
                            self.t.leaf(K::Identifier, f::NONE, stop as u32 + 1, e as u32);
                            self.t.close_at(x, e as u32);
                            i = e;
                        }
                        _ => return self.refuse(),
                    }
                }
            }
        };
        // Without an interpolation's tokens, those read after the string are those lexed from its
        // end already.
        if interpolated || end != t.end as usize {
            self.relex_at(end);
        }
        self.t.last_end = end as u32;
        self.sp = end as u32;
        self.t.close_at(s, end as u32);
        Ok(self.t.close_at(n, end as u32))
    }
}
