//! Directives: `#include`, `#define` (an object's and a function's), the conditionals (`#if`,
//! `#ifdef`, `#elif`, `#else`, `#endif`) around items of the level they stand at, and any other
//! directive as a `preproc_call` with its argument.
//!
//! A conditional's rule depends on its level (`preproc_if_in_field_declaration_list` …): its node
//! is written as `preproc_if`, `preproc_else` …, and its tokens are stored by its own rule's
//! implied tokens.

use super::*;

/// A conditional's rules at a level: `#if`, `#ifdef`, `#else`, `#elif`, `#elifdef`.
struct Rules {
    r#if: K,
    ifdef: K,
    r#else: K,
    elif: K,
    elifdef: K,
}

fn rules(level: Level, no_comma: bool) -> Rules {
    use K::*;
    match level {
        Level::Top | Level::Block => Rules { r#if: PreprocIf, ifdef: PreprocIfdef, r#else: PreprocElse, elif: PreprocElif, elifdef: PreprocElifdef },
        Level::Field => Rules {
            r#if: PreprocIfInFieldDeclarationList,
            ifdef: PreprocIfdefInFieldDeclarationList,
            r#else: PreprocElseInFieldDeclarationList,
            elif: PreprocElifInFieldDeclarationList,
            elifdef: PreprocElifdefInFieldDeclarationList,
        },
        Level::Enum if no_comma => Rules {
            r#if: PreprocIfInEnumeratorListNoComma,
            ifdef: PreprocIfdefInEnumeratorListNoComma,
            r#else: PreprocElseInEnumeratorListNoComma,
            elif: PreprocElifInEnumeratorListNoComma,
            elifdef: PreprocElifdefInEnumeratorListNoComma,
        },
        Level::Enum => Rules {
            r#if: PreprocIfInEnumeratorList,
            ifdef: PreprocIfdefInEnumeratorList,
            r#else: PreprocElseInEnumeratorList,
            elif: PreprocElifInEnumeratorList,
            elifdef: PreprocElifdefInEnumeratorList,
        },
    }
}

impl<'a> P<'a> {
    /// The tokens from `pos` lexed again (where the parser read a piece the lexer does not: an
    /// argument, `<…>`): the buffer from the next token on dropped.
    pub(super) fn relex(&mut self, pos: usize) {
        self.relex_over(pos, pos);
    }

    /// The tokens from `pos` lexed again, the comments lexed from `from` on dropped (a piece over
    /// `from..pos` holds them).
    fn relex_over(&mut self, from: usize, pos: usize) {
        self.relexes += 1;
        self.toks.truncate(self.i);
        self.next = pos;
        let keep = self.extras[self.e..].iter().position(|x| x.start as usize >= from).map_or(self.extras.len(), |k| self.e + k);
        self.extras.truncate(keep);
    }

    /// A directive's line end: the `\n` in its span, no node (a comment before it is the
    /// directive's). A directive the file ends in without one is refused.
    pub(super) fn line_end(&mut self) -> R<()> {
        let t = self.tok();
        if t.nl == NO_NL {
            return self.refuse();
        }
        self.flush(t.nl);
        self.last_end = t.nl + 1;
        Ok(())
    }

    /// A leaf over `start..end` (a piece the parser read from the bytes).
    pub(super) fn span_leaf(&mut self, kind: K, field: u8, class: u8, start: usize, end: usize) {
        self.flush(start as u32);
        let n = self.w.push(key(kind, field, class, false), start as u32);
        self.w.close(n, end as u32);
        self.last_end = end as u32;
    }

    /// A directive's argument (`preproc_arg`) from where the last token ended, in `field`, if the
    /// line holds one; the tokens after it lexed again.
    fn argument(&mut self, field: u8) -> R<()> {
        let from = self.last_end as usize;
        if let Some((s, e)) = self.lx.arg(from) {
            self.relex_over(s, e);
            self.span_leaf(K::PreprocArg, field, C_NONE, s, e);
        }
        Ok(())
    }

    /// A directive at `level`, under `parent`.
    pub(super) fn directive(&mut self, level: Level) -> R<()> {
        let t = self.tok();
        match t.word {
            w::INCLUDE => self.include(),
            w::DEFINE => self.define(),
            w::IF | w::IFDEF | w::IFNDEF => self.conditional(level),
            _ if self.is_branch_word(t.word) => self.refuse(),
            _ => {
                // Any other directive: `preproc_call`, its directive and argument.
                let n = self.open(K::PreprocCall, f::NONE, C_NONE);
                self.leaf(K::PreprocDirective, f::DIRECTIVE, C_NONE)?;
                self.argument(f::ARGUMENT)?;
                self.line_end()?;
                self.close(n);
                Ok(())
            }
        }
    }

    fn is_branch_word(&self, word: u8) -> bool {
        matches!(word, w::ELSE | w::ELIF | w::ELIFDEF | w::ELIFNDEF | w::ENDIF)
    }

    /// `#include` and its path: a string, `<…>`, a name or a call.
    fn include(&mut self) -> R<()> {
        let n = self.open(K::PreprocInclude, f::NONE, C_NONE);
        self.tk(K::PreprocInclude, K::THashInclude)?;
        let t = self.tok();
        if t.nl != NO_NL {
            return self.refuse();
        }
        match t.kind {
            T::Str => {
                self.string(slot(f::PATH, Ctx::Bare))?;
            }
            T::Lt => {
                let Some(e) = self.lx.system_lib(t.start as usize) else { return self.refuse() };
                self.relex(e);
                self.span_leaf(K::SystemLibString, f::PATH, C_NONE, t.start as usize, e);
            }
            T::Name if self.kind_at(1) == T::LParen && self.peek_at(1).nl == NO_NL => {
                self.pre_expr(slot(f::PATH, Ctx::Bare))?;
            }
            T::Name => {
                self.leaf(K::Identifier, f::PATH, C_NONE)?;
            }
            _ => return self.refuse(),
        }
        self.line_end()?;
        self.close(n);
        Ok(())
    }

    /// `#define` a name, its parameters where a `(` follows it at once, and its value.
    fn define(&mut self) -> R<()> {
        let n = self.open(K::PreprocDef, f::NONE, C_NONE);
        self.tk(K::PreprocDef, K::THashDefine)?;
        let name = self.tok();
        if name.nl != NO_NL || name.kind != T::Name {
            return self.refuse();
        }
        self.leaf(K::Identifier, f::NAME, C_NONE)?;
        let t = self.tok();
        if t.kind == T::LParen && t.start == name.end && t.nl == NO_NL {
            self.rekind(n.at, K::PreprocFunctionDef, BARE);
            let ps = self.open(K::PreprocParams, f::PARAMETERS, C_NONE);
            let mut first = true;
            // `(...`: the `(` stored; `...)`: the `)` stored.
            let open_kept = self.kind_at(1) == T::DotDotDot;
            self.gap_tk(K::PreprocParams, K::TLParen, open_kept)?;
            let mut last = T::LParen;
            loop {
                let t = self.tok();
                if t.nl != NO_NL {
                    return self.refuse();
                }
                match t.kind {
                    T::RParen => break,
                    // A `,` before `...` is stored.
                    T::Comma if !first => {
                        let kept = self.kind_at(1) == T::DotDotDot;
                        self.gap_tk(K::PreprocParams, K::TComma, kept)?
                    }
                    _ if !first => return self.refuse(),
                    _ => {}
                }
                first = false;
                last = self.peek();
                match last {
                    T::Name => {
                        self.leaf(K::Identifier, f::NONE, C_NONE)?;
                    }
                    T::DotDotDot => {
                        self.leaf(K::TDotDotDot, f::NONE, C_NONE)?;
                    }
                    _ => return self.refuse(),
                }
            }
            self.gap_tk(K::PreprocParams, K::TRParen, last == T::DotDotDot)?;
            self.close(ps);
        }
        self.argument(f::VALUE)?;
        self.line_end()?;
        self.close(n);
        Ok(())
    }

    /// `#if`, `#ifdef` or `#ifndef`, its branch's items at `level`, its alternatives, `#endif`.
    fn conditional(&mut self, level: Level) -> R<()> {
        let no_comma = level == Level::Enum && self.enum_branch_no_comma();
        let r = rules(level, no_comma);
        let t = self.tok();
        let (kind, rule) = if t.word == w::IF { (K::PreprocIf, r.r#if) } else { (K::PreprocIfdef, r.ifdef) };
        let n = self.open(kind, f::NONE, C_NONE);
        self.head(rule, t.word)?;
        self.branch(level, rule)?;
        self.alternatives(level, &r)?;
        let t = self.tok();
        if t.kind != T::Directive || t.word != w::ENDIF {
            return self.refuse();
        }
        // `#endif` is stored at every level (the expected tapes key it by the written kind).
        self.tk(kind, K::THashEndif)?;
        self.close(n);
        Ok(())
    }

    /// A conditional's directive and its condition (`#if`, `#elif`: an expression and the line's
    /// `\n`) or name (`#ifdef` …).
    fn head(&mut self, rule: K, word: u8) -> R<()> {
        let tok = match word {
            w::IF => K::THashIf,
            w::ELIF => K::THashElif,
            w::IFDEF => K::THashIfdef,
            w::IFNDEF => K::THashIfndef,
            w::ELIFDEF => K::THashElifdef,
            w::ELIFNDEF => K::THashElifndef,
            _ => return self.refuse(),
        };
        self.tk(rule, tok)?;
        if self.tok().nl != NO_NL {
            return self.refuse();
        }
        if matches!(word, w::IF | w::ELIF) {
            self.pre_expr(slot(f::CONDITION, Ctx::Bare))?;
            // The `\n` after the condition: a token of the rule's; stored where the dialect's
            // tapes keep it (C++'s, over the blank lines after it).
            let t = self.tok();
            if t.nl == NO_NL {
                return self.refuse();
            }
            self.flush(t.nl);
            if self.stores(rule, K::TNewline, f::NONE) {
                let mut e = t.nl as usize + 1;
                // The run of `\n`s (a `\r` ends it: `\r\n` lines give one `\n`).
                while e < self.src.len() && self.src[e] == b'\n' {
                    e += 1;
                }
                let n = self.w.push(key(K::TNewline, f::NONE, C_NONE, false), t.nl);
                self.w.close(n, e as u32);
                self.last_end = e as u32;
            } else {
                self.last_end = t.nl + 1;
            }
        } else {
            if self.peek() != T::Name {
                return self.refuse();
            }
            self.leaf(K::Identifier, f::NAME, C_NONE)?;
        }
        Ok(())
    }

    /// A branch's items, to the directive that ends it.
    fn branch(&mut self, level: Level, rule: K) -> R<()> {
        loop {
            let t = self.tok();
            if t.kind == T::Eof {
                return self.refuse();
            }
            if t.kind == T::Directive && self.is_branch_word(t.word) {
                return Ok(());
            }
            match level {
                Level::Enum => self.enum_item(rule)?,
                // A conditional's items are `_block_item`s, at the top too: its statements are
                // in `statement`.
                Level::Top => self.item(Level::Block, rule)?,
                _ => self.item(level, rule)?,
            }
        }
    }

    /// `#elif`, `#elifdef` and `#else` branches, each the alternative of the one before.
    fn alternatives(&mut self, level: Level, r: &Rules) -> R<()> {
        let t = self.tok();
        if t.kind != T::Directive {
            return self.refuse();
        }
        let (kind, rule) = match t.word {
            w::ENDIF => return Ok(()),
            w::ELSE => (K::PreprocElse, r.r#else),
            w::ELIF => (K::PreprocElif, r.elif),
            w::ELIFDEF | w::ELIFNDEF => (K::PreprocElifdef, r.elifdef),
            _ => return self.refuse(),
        };
        let n = self.open(kind, f::ALTERNATIVE, C_NONE);
        if t.word == w::ELSE {
            self.tk(rule, K::THashElse)?;
        } else {
            self.head(rule, t.word)?;
        }
        self.branch(level, rule)?;
        if t.word != w::ELSE {
            self.alternatives(level, r)?;
        }
        self.close(n);
        Ok(())
    }

    /// Whether an enumerator list's conditional ahead holds an enumerator without its `,`
    /// (`preproc_if_in_enumerator_list_no_comma`).
    fn enum_branch_no_comma(&mut self) -> bool {
        let mut depth = 0usize;
        let mut j = 1;
        let mut last = T::Comma;
        loop {
            let t = self.peek_at(j);
            match t.kind {
                T::Eof => return false,
                T::Directive if matches!(t.word, w::IF | w::IFDEF | w::IFNDEF) => depth += 1,
                T::Directive if t.word == w::ENDIF => {
                    if depth == 0 {
                        return last != T::Comma;
                    }
                    depth -= 1;
                }
                T::Directive if depth == 0 && self.is_branch_word(t.word) => {
                    if last != T::Comma {
                        return true;
                    }
                }
                T::Directive => {}
                k => last = k,
            }
            j += 1;
        }
    }

    /// An enumerator list's item at a conditional's level: an enumerator and its `,`, or a
    /// directive.
    fn enum_item(&mut self, rule: K) -> R<()> {
        if self.peek() == T::Directive {
            return self.directive(Level::Enum);
        }
        self.enumerator()?;
        if self.peek() == T::Comma {
            // A `,` before the branch's end is stored.
            if self.kind_at(1) == T::Directive && { let c = self.peek_at(1).word; self.is_branch_word(c) } {
                self.leaf(K::TComma, f::NONE, C_NONE)?;
            } else {
                self.tk(rule, K::TComma)?;
            }
        }
        Ok(())
    }

    // ---- Directive expressions.

    /// A directive's expression (`_preproc_expression`), to its line's end.
    pub(super) fn pre_expr(&mut self, s: Slot) -> R<N> {
        self.pre_binary(s, 1)
    }

    fn pre_binary(&mut self, s: Slot, min: u8) -> R<N> {
        self.deeper()?;
        let mut left = self.pre_unary(s)?;
        loop {
            let t = self.tok();
            if t.nl != NO_NL {
                break;
            }
            let Some((p, op)) = super::expr::binary(t.kind) else { break };
            if p < min {
                break;
            }
            let n = self.wrap(left, K::BinaryExpression, s, slot(f::LEFT, Ctx::Bare));
            self.leaf(op, f::OPERATOR, C_NONE)?;
            self.pre_binary(slot(f::RIGHT, Ctx::Bare), p + 1)?;
            self.close(n);
            left = n;
        }
        self.depth -= 1;
        Ok(left)
    }

    fn pre_unary(&mut self, s: Slot) -> R<N> {
        let t = self.tok();
        if t.nl != NO_NL {
            return self.refuse();
        }
        match t.kind {
            T::Bang | T::Tilde | T::Minus | T::Plus => {
                if matches!(t.kind, T::Minus | T::Plus) && self.kind_at(1) == T::Number && self.peek_at(1).start == t.end {
                    return self.signed_number(s);
                }
                let n = self.open_in(K::UnaryExpression, s);
                self.leaf(punct(t.kind).expect("an operator"), f::OPERATOR, C_NONE)?;
                self.pre_unary(slot(f::ARGUMENT, Ctx::Bare))?;
                self.close(n);
                Ok(n)
            }
            T::Number => self.leaf_in(K::NumberLiteral, s),
            T::Char => self.char_literal(s),
            T::LParen => {
                let n = self.open_in(K::ParenthesizedExpression, s);
                self.tk(K::PreprocParenthesizedExpression, K::TLParen)?;
                self.pre_expr(BARE)?;
                if self.peek() != T::RParen || self.tok().nl != NO_NL {
                    return self.refuse();
                }
                self.tk(K::PreprocParenthesizedExpression, K::TRParen)?;
                self.close(n);
                Ok(n)
            }
            T::Name if t.word == w::DEFINED => {
                let n = self.open_in(K::PreprocDefined, s);
                self.leaf(K::Tdefined, f::NONE, C_NONE)?;
                if self.peek() == T::LParen && self.tok().nl == NO_NL {
                    self.tk(K::PreprocDefined, K::TLParen)?;
                    if self.peek() != T::Name || self.tok().nl != NO_NL {
                        return self.refuse();
                    }
                    self.leaf(K::Identifier, f::NONE, C_NONE)?;
                    if self.peek() != T::RParen || self.tok().nl != NO_NL {
                        return self.refuse();
                    }
                    self.tk(K::PreprocDefined, K::TRParen)?;
                } else {
                    if self.peek() != T::Name || self.tok().nl != NO_NL {
                        return self.refuse();
                    }
                    self.leaf(K::Identifier, f::NONE, C_NONE)?;
                }
                self.close(n);
                Ok(n)
            }
            T::Name => {
                let u = self.peek_at(1);
                if u.kind == T::LParen && u.nl == NO_NL {
                    let n = self.open_in(K::CallExpression, s);
                    self.leaf(K::Identifier, f::FUNCTION, C_NONE)?;
                    let a = self.open(K::ArgumentList, f::ARGUMENTS, C_NONE);
                    self.tk(K::PreprocArgumentList, K::TLParen)?;
                    if self.peek() != T::RParen {
                        loop {
                            self.pre_expr(BARE)?;
                            if self.tok().nl != NO_NL {
                                return self.refuse();
                            }
                            match self.peek() {
                                T::Comma => self.tk(K::PreprocArgumentList, K::TComma)?,
                                T::RParen => break,
                                _ => return self.refuse(),
                            }
                        }
                    }
                    if self.tok().nl != NO_NL {
                        return self.refuse();
                    }
                    self.tk(K::PreprocArgumentList, K::TRParen)?;
                    self.close(a);
                    self.close(n);
                    return Ok(n);
                }
                self.leaf_in(K::Identifier, s)
            }
            _ => self.refuse(),
        }
    }
}
