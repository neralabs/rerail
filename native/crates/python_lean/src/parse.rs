//! The parser: recursive descent over the lexer's tokens, writing each node to the tape as it is
//! found (`Writer`), with tree-sitter-python's shapes, fields and supertypes.
//!
//! - **A slot** (`Slot`) is where an expression goes: its field and its context (`Ctx`), which
//!   give its supertypes by its kind (`class`). An expression found to be the first child of a
//!   larger one (an operand before its operator) gets the larger one put before it
//!   (`Writer::insert`) and its own slot changed (`rekey`).
//! - **A target** parsed as an expression (`a, b = …`) is made a pattern at the `=` (`to_pattern`),
//!   its kinds and supertypes rewritten in place.
//! - **Extras** are written before the next node opened or token read (`flush`): into the node
//!   then open, the lowest that holds the tokens on either side.
//! - **A token is stored** only where tree-sitter's tape stores it (the ones the grammar does not
//!   imply: operators, `def`, a trailing comma and its closing bracket, …).

use crate::lex::{Extra, Lexed, Refusal, T, Tok, soft};
use crate::{K, Tape, f, key, unkey};
use rerail_tape_format::{Redo, Widths, Writer};

type R<X> = Result<X, Refusal>;

const C_NONE: u8 = 0;
const C_EP: u8 = 1;
const C_E: u8 = 2;
const C_P: u8 = 3;
const C_PAT: u8 = 4;
const C_PARAM: u8 = 5;

/// The context a node stands in: which supertypes its kind is in there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctx {
    /// An `expression`: a primary one is in both supertypes, another in `expression`.
    Expr,
    /// A `primary_expression` (an operand, a call's function, an attribute's object).
    Primary,
    /// A `pattern` (an assignment's or a `for`'s target).
    Pattern,
    /// A `parameter`.
    Param,
    /// None: a kind named directly by its parent's rule.
    Bare,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    field: u8,
    ctx: Ctx,
}

const fn slot(field: u8, ctx: Ctx) -> Slot {
    Slot { field, ctx }
}

const EXPR: Slot = slot(f::NONE, Ctx::Expr);

/// A node written: its index and kind.
#[derive(Clone, Copy, Debug)]
struct N {
    at: usize,
    kind: K,
}

fn is_primary(k: K) -> bool {
    use K::*;
    matches!(
        k,
        Identifier
            | String
            | ConcatenatedString
            | Integer
            | Float
            | True
            | False
            | None
            | UnaryOperator
            | BinaryOperator
            | Attribute
            | Subscript
            | Call
            | List
            | ListComprehension
            | Dictionary
            | DictionaryComprehension
            | Set
            | SetComprehension
            | Tuple
            | ParenthesizedExpression
            | GeneratorExpression
            | Ellipsis
            | ListSplat
            | Await
    )
}

fn is_expression_only(k: K) -> bool {
    use K::*;
    matches!(k, ComparisonOperator | NotOperator | BooleanOperator | Lambda | ConditionalExpression | NamedExpression | AsPattern)
}

fn class(k: K, ctx: Ctx) -> u8 {
    match ctx {
        Ctx::Expr if is_primary(k) => C_EP,
        Ctx::Expr if is_expression_only(k) => C_E,
        Ctx::Primary if is_primary(k) => C_P,
        Ctx::Pattern if matches!(k, K::Identifier | K::Attribute | K::Subscript | K::ListSplatPattern | K::TuplePattern | K::ListPattern) => C_PAT,
        Ctx::Param => C_PARAM,
        _ => C_NONE,
    }
}

/// What a `*` makes in a list of elements.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Splat {
    /// A collection's or an argument list's: `list_splat` around an expression.
    Collection,
    /// An expression list's or a subscript's: the primary `list_splat` (`*name`, then postfixes).
    Alias,
}

const MAX_DEPTH: u32 = 400;

struct P<'a> {
    src: &'a [u8],
    toks: &'a [Tok],
    extras: &'a [Extra],
    i: usize,
    e: usize,
    w: Writer<Widths>,
    last_end: u32,
    depth: u32,
}

pub fn parse(src: &[u8], lexed: &Lexed) -> R<Tape> {
    let mut widths = Widths::guess(src.len());
    loop {
        let mut p = P {
            src,
            toks: &lexed.toks,
            extras: &lexed.extras,
            i: 0,
            e: 0,
            w: Writer::new(0, widths, lexed.toks.len() + lexed.extras.len() + 1),
            last_end: 0,
            depth: 0,
        };
        p.module()?;
        match p.w.finish_columns() {
            Ok(columns) => return Ok(columns),
            Err(Redo(w)) => {
                if w == widths {
                    return Err(Refusal::TooLarge);
                }
                widths = w;
            }
        }
    }
}

fn can_start_expr(t: T) -> bool {
    use T::*;
    matches!(t, Name | Int | Float | StrStart | True | False | None | Ellipsis | LParen | LBracket | LBrace | Minus | Plus | Tilde | Not | Lambda | Star | Yield)
}

fn binop(t: T) -> Option<(u8, K)> {
    use T::*;
    Some(match t {
        Pipe => (14, K::TPipe),
        Caret => (16, K::TCaret),
        Amp => (15, K::TAmp),
        LShift => (17, K::TLShift),
        RShift => (17, K::TRShift),
        Plus => (18, K::TPlus),
        Minus => (18, K::TMinus),
        Star => (19, K::TStar),
        At => (19, K::TAt),
        Slash => (19, K::TSlash),
        Percent => (19, K::TPercent),
        DSlash => (19, K::TDSlash),
        Pow => (21, K::TPow),
        _ => return Option::None,
    })
}

fn augmented(t: T) -> Option<K> {
    use T::*;
    Some(match t {
        PlusEq => K::TPlusEq,
        MinusEq => K::TMinusEq,
        StarEq => K::TStarEq,
        SlashEq => K::TSlashEq,
        AtEq => K::TAtEq,
        DSlashEq => K::TDSlashEq,
        PercentEq => K::TPercentEq,
        PowEq => K::TPowEq,
        RShiftEq => K::TRShiftEq,
        LShiftEq => K::TLShiftEq,
        AmpEq => K::TAmpEq,
        CaretEq => K::TCaretEq,
        PipeEq => K::TPipeEq,
        _ => return Option::None,
    })
}

fn comparison(t: T) -> Option<K> {
    use T::*;
    Some(match t {
        Lt => K::TLt,
        Gt => K::TGt,
        Le => K::TLe,
        Ge => K::TGe,
        EqEq => K::TEqEq,
        Ne => K::TNe,
        LtGt => K::TLtGt,
        In => K::TIn,
        _ => return Option::None,
    })
}

impl<'a> P<'a> {
    // ---- Tokens and the writer.

    #[inline(always)]
    fn peek(&self) -> T {
        self.toks[self.i].kind
    }

    #[inline(always)]
    fn peek_at(&self, k: usize) -> T {
        self.toks.get(self.i + k).map_or(T::Eof, |t| t.kind)
    }

    #[inline(always)]
    fn tok(&self) -> Tok {
        self.toks[self.i]
    }

    fn text(&self, t: Tok) -> &'a [u8] {
        &self.src[t.start as usize..t.end as usize]
    }

    /// Whether the token `k` ahead is a name that is the soft keyword `soft` (`lex::soft`).
    fn is_soft(&self, k: usize, soft: u8) -> bool {
        self.toks.get(self.i + k).is_some_and(|t| t.soft == soft)
    }

    /// Whether the token `k` ahead is the name `name`.
    fn is_name(&self, k: usize, name: &[u8]) -> bool {
        self.toks.get(self.i + k).is_some_and(|t| t.kind == T::Name && self.text(*t) == name)
    }

    fn refuse<X>(&self) -> R<X> {
        Err(Refusal::Parse(self.tok().start))
    }

    /// The extras before `before`, written into the node open.
    #[inline(always)]
    fn flush(&mut self, before: u32) {
        while let Some(x) = self.extras.get(self.e) {
            if x.start >= before {
                break;
            }
            let kind = if x.comment { K::Comment } else { K::LineContinuation };
            let n = self.w.push(key(kind, f::NONE, C_NONE, true), x.start);
            self.w.close(n, x.end);
            self.e += 1;
        }
    }

    /// The next token read, its extras before it written.
    #[inline(always)]
    fn bump(&mut self) -> Tok {
        let t = self.tok();
        self.flush(t.start);
        if t.kind != T::Eof {
            self.i += 1;
        }
        self.last_end = t.end;
        t
    }

    fn expect(&mut self, kind: T) -> R<Tok> {
        if self.peek() != kind {
            return self.refuse();
        }
        Ok(self.bump())
    }

    /// The next token as a leaf.
    #[inline(always)]
    fn leaf(&mut self, kind: K, field: u8, class: u8) -> N {
        let t = self.bump();
        let at = self.w.push(key(kind, field, class, false), t.start);
        self.w.close(at, t.end);
        N { at, kind }
    }

    /// A token read before, as a leaf now (a trailing comma, written once its list is known).
    fn leaf_of(&mut self, t: Tok, kind: K) {
        let at = self.w.push(key(kind, f::NONE, C_NONE, false), t.start);
        self.w.close(at, t.end);
    }

    /// A node opened at the next token.
    #[inline(always)]
    fn open(&mut self, kind: K, field: u8, class: u8) -> N {
        let s = self.tok().start;
        self.flush(s);
        N { at: self.w.push(key(kind, field, class, false), s), kind }
    }

    #[inline(always)]
    fn open_in(&mut self, kind: K, s: Slot) -> N {
        self.open(kind, s.field, class(kind, s.ctx))
    }

    #[inline(always)]
    fn close(&mut self, n: N) {
        self.w.close(n.at, self.last_end);
    }

    /// `kind` put before `n` (wrapping it and what follows), in slot `s`; `n` moves up one.
    fn wrap(&mut self, n: N, kind: K, s: Slot) -> N {
        let start = self.w.start(n.at);
        self.w.insert(n.at, key(kind, s.field, class(kind, s.ctx), false), start);
        N { at: n.at, kind }
    }

    /// `n`'s slot changed.
    fn rekey(&mut self, at: usize, s: Slot) {
        let (kind, _, _, extra) = unkey(self.w.key(at));
        let k = kind_of(kind);
        self.w.set_key(at, key(k, s.field, class(k, s.ctx), extra));
    }

    fn kind_at(&self, at: usize) -> K {
        kind_of(unkey(self.w.key(at)).0)
    }

    /// The named, non-extra children of `at` (a closed node).
    fn children(&self, at: usize) -> Vec<usize> {
        let end = at + self.w.size(at) as usize;
        let mut c = at + 1;
        let mut out = Vec::new();
        while c < end {
            let (kind, _, _, extra) = unkey(self.w.key(c));
            if !extra && crate::KINDS[usize::from(kind)].1 {
                out.push(c);
            }
            c += self.w.size(c) as usize;
        }
        out
    }

    fn deeper(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.refuse();
        }
        Ok(())
    }

    // ---- The module and statements.

    fn module(&mut self) -> R<()> {
        let len = self.src.len() as u32;
        let first = self.toks.first().filter(|t| t.kind != T::Eof).map_or(len, |t| t.start);
        let start = self.extras.first().map_or(first, |x| x.start.min(first));
        let m = self.w.push(key(K::Module, f::NONE, C_NONE, false), start);
        while self.peek() != T::Eof {
            self.statement_or_recovered(None)?;
            self.w.commit();
        }
        self.flush(u32::MAX);
        self.w.close(m, len);
        Ok(())
    }

    /// A statement of a module or of `block`. A syntax error anywhere, the text's end included, is
    /// refused: tree-sitter-python reads the file (decision 0163, amended 2026-11-13). Tree-sitter's
    /// recovery is its GLR search, and the end-of-text recovery here (`recover_at_end`) missed it
    /// twice (`def broken(:`, a block's first statement `return (`): the rules saw other errors.
    /// `recover_at_end` is kept, unused, for the Bitlex parser's work on recovery.
    #[inline(always)]
    fn statement_or_recovered(&mut self, block: Option<&mut N>) -> R<()> {
        let _ = block;
        self.statement()
    }

    /// A statement the text ends in, as tree-sitter-python recovers it: one `ERROR`, an extra of the
    /// block (or module) the statement is in, holding what tree-sitter's stack holds at the end.
    ///
    /// - A subtree tree-sitter reduced (one the parser closed) is kept whole, without its field
    ///   (but for an import's names, which a hidden rule gives theirs); its supertypes are its
    ///   context's.
    /// - A node still open is not made: its pieces are the `ERROR`'s, each by the same rule.
    /// - Every token not in a kept subtree is a leaf of its own: an `ERROR` implies none.
    /// - Comments before the statement stay outside the `ERROR`.
    /// - A block's first statement: the `ERROR` is the compound statement's, before its block,
    ///   which is empty, at the text's end (tree-sitter had no block yet to put it in).
    ///
    /// Only where the statement failed for want of tokens: what is left is layout tokens and the
    /// end. A syntax error before the end is refused (tree-sitter-python reads the file).
    #[allow(dead_code)]
    #[cold]
    #[inline(never)]
    fn recover_at_end(&mut self, mark: Mark, block: Option<&mut N>) -> R<()> {
        // The token the statement failed at, the text's last: the `ERROR`'s too (tree-sitter skips
        // it into it). Past it, only layout tokens and the end.
        let layout = |t: &Tok| matches!(t.kind, T::Newline | T::Indent | T::Dedent | T::Eof);
        // (Its line as the text has it: inside brackets the lexer gives no line ends.)
        let from = self.toks[self.i].start as usize;
        let line_end = self.src[from.min(self.src.len())..].iter().position(|&c| c == b'\n').map_or(u32::MAX, |k| (from + k) as u32);
        let rest = self.toks[self.i..].iter().take_while(|t| !layout(t) && t.start < line_end).count();
        // (One at most: past it tree-sitter may take up the statement again, as `)` after `f(a b`.
        // And only a word being typed, `impor` of `from os.path impor`: before punctuation tree-sitter
        // may put in a missing token and read on, as `)` before the `:` of `def broken(:`, an error
        // of one empty node where an `ERROR` here would hold the whole statement.)
        if rest > 1 || (rest == 1 && self.toks[self.i].kind != T::Name) || !self.toks[self.i + rest..].iter().all(layout) {
            return self.refuse();
        }
        let Some(last) = self.toks[mark.i..self.i].iter().rev().find(|t| !layout(t)).copied() else { return self.refuse() };
        let toks: Vec<Tok> = self.toks[mark.i..self.i + rest].iter().copied().filter(|t| !layout(t)).collect();
        let consumed = toks.len() - rest;
        self.i += rest;
        // Whether tree-sitter reduced the last token into what the parser closed around it: by the
        // token before it (measured over cut texts). After an opening bracket, `=`, `->` or a
        // keyword it did not (the last token's subtrees are pieces; its leaf has no supertypes);
        // after `,`, `.`, `:`, an operator, a closing bracket it did, up to a primary expression.
        let reduced = consumed < 2
            || !matches!(
                toks[consumed - 2].kind,
                T::LParen | T::LBracket | T::LBrace | T::Eq | T::Arrow | T::From | T::If | T::In | T::Not | T::With | T::Elif | T::While | T::For | T::Return | T::Import | T::Del | T::Assert | T::Raise | T::Yield | T::Lambda | T::Global | T::Nonlocal | T::Except | T::As | T::Class | T::Def
            );
        // The kept subtrees, in order (first node, size, its key in the `ERROR`); the open nodes
        // are the ancestors of all after them, so a kept node's parent is the last open one.
        let mut kept: Vec<(usize, usize, u32)> = Vec::new();
        let mut parent = None;
        let mut j = mark.n;
        while j < self.w.len() {
            let size = self.w.size(j) as usize;
            let at_end = !self.w.is_open(j) && self.w.end(j) >= last.end;
            if self.w.is_open(j) || (!reduced && at_end && size > 1) {
                parent = Some(self.kind_at(j));
                j += 1;
                continue;
            }
            let (kind, field, class, extra) = unkey(self.w.key(j));
            let import = matches!(parent, Some(K::ImportStatement | K::ImportFromStatement | K::FutureImportStatement));
            let field = if import && field == f::NAME { field } else { f::NONE };
            let class = match (at_end, reduced) {
                (true, false) => C_NONE,
                (true, true) if size > 1 && class == C_EP => C_P,
                _ => class,
            };
            kept.push((j, size, key(kind_of(kind), field, class, extra)));
            j += size;
        }
        // The comments before the statement's first token, outside the `ERROR`.
        let lead = kept.iter().take_while(|&&(j, _, k)| unkey(k).3 && self.w.start(j) < toks[0].start).count();
        let pieces = self.pieces(&kept, &toks)?;
        // A block's first statement: the block goes, and comes back empty after the `ERROR`.
        let first = block.as_ref().map(|b| (b.at + 1 == mark.n, self.w.key(b.at)));
        let from = match first {
            Some((true, _)) => mark.n - 1,
            _ => mark.n,
        };
        self.w.truncate(from);
        self.depth = mark.depth;
        for nodes in &pieces[..lead] {
            self.replay(nodes);
        }
        let error = self.w.push(key(K::Error, f::NONE, C_NONE, true), pieces[lead][0].1);
        for nodes in &pieces[lead..] {
            self.replay(nodes);
        }
        let end = toks.last().map_or(last.end, |t| t.end);
        self.w.close(error, end);
        self.last_end = end;
        if let (Some((true, key)), Some(b)) = (first, block) {
            b.at = self.w.push(key, end);
        }
        // The statement's line ends are the `ERROR`'s; the dedents close the blocks around it.
        while self.peek() == T::Newline {
            self.i += 1;
        }
        Ok(())
    }

    /// The pieces of an `ERROR` by start: each kept subtree's nodes (key, start, size, end), and a
    /// leaf for each token not in one.
    #[allow(dead_code)]
    fn pieces(&self, kept: &[(usize, usize, u32)], toks: &[Tok]) -> R<Vec<Vec<(u32, u32, u32, u32)>>> {
        let node = |w: &Writer<Widths>, n: usize| (w.key(n), w.start(n), w.size(n), w.end(n));
        let mut pieces: Vec<Vec<(u32, u32, u32, u32)>> = Vec::new();
        let (mut k, mut t) = (0, 0);
        while k < kept.len() || t < toks.len() {
            let ks = kept.get(k).map(|&(j, _, _)| self.w.start(j));
            match (ks, toks.get(t).copied()) {
                (Some(ks), Some(tok)) if tok.start >= ks => {
                    let (j, size, key) = kept[k];
                    let end = self.w.end(j);
                    let mut nodes: Vec<_> = (j..j + size).map(|n| node(&self.w, n)).collect();
                    nodes[0].0 = key;
                    pieces.push(nodes);
                    // The tokens inside it are its own.
                    while toks.get(t).is_some_and(|x| x.start < end) {
                        t += 1;
                    }
                    k += 1;
                }
                (Some(_), None) => {
                    let (j, size, key) = kept[k];
                    let mut nodes: Vec<_> = (j..j + size).map(|n| node(&self.w, n)).collect();
                    nodes[0].0 = key;
                    pieces.push(nodes);
                    k += 1;
                }
                (_, Some(tok)) => {
                    pieces.push(vec![(key(token_kind(tok)?, f::NONE, C_NONE, false), tok.start, 1, tok.end)]);
                    t += 1;
                }
                (None, None) => unreachable!(),
            }
        }
        Ok(pieces)
    }

    /// Nodes written again, in preorder: (key, start, size, end) each.
    #[allow(dead_code)]
    fn replay(&mut self, nodes: &[(u32, u32, u32, u32)]) {
        let mut open: Vec<(usize, usize, u32)> = Vec::new();
        for (k, &(key, start, size, end)) in nodes.iter().enumerate() {
            let at = self.w.push(key, start);
            open.push((at, k + size as usize, end));
            while let Some(&(at, until, end)) = open.last() {
                if until != k + 1 {
                    break;
                }
                self.w.close(at, end);
                open.pop();
            }
        }
    }

    fn statement(&mut self) -> R<()> {
        self.deeper()?;
        match self.peek() {
            T::If => self.if_statement()?,
            T::For => self.for_statement()?,
            T::While => self.while_statement()?,
            T::Try => self.try_statement()?,
            T::With => self.with_statement()?,
            T::Def => self.function_definition(f::NONE)?,
            T::Class => self.class_definition(f::NONE)?,
            T::At => self.decorated()?,
            T::Name if self.is_soft(0, soft::ASYNC) && matches!(self.peek_at(1), T::Def | T::For | T::With) => match self.peek_at(1) {
                T::Def => self.function_definition(f::NONE)?,
                T::For => self.for_statement()?,
                _ => self.with_statement()?,
            },
            T::Name if self.is_soft(0, soft::MATCH) && self.is_match() => self.match_statement()?,
            _ => self.simple_statements()?,
        }
        self.depth -= 1;
        Ok(())
    }

    /// Statements on one line, `;` between, the line's NEWLINE.
    fn simple_statements(&mut self) -> R<()> {
        self.simple_statement()?;
        while self.peek() == T::Semi {
            self.leaf(K::TSemi, f::NONE, C_NONE);
            if self.peek() == T::Newline {
                break;
            }
            self.simple_statement()?;
        }
        self.expect(T::Newline)?;
        Ok(())
    }

    fn simple_statement(&mut self) -> R<()> {
        match self.peek() {
            T::Import => self.import_statement(),
            T::From => self.from_import(),
            T::Return => {
                let n = self.open(K::ReturnStatement, f::NONE, C_NONE);
                self.bump();
                if can_start_expr(self.peek()) && self.peek() != T::Yield {
                    self.exprs(EXPR)?;
                }
                self.close(n);
                Ok(())
            }
            T::Del => {
                let n = self.open(K::DeleteStatement, f::NONE, C_NONE);
                self.bump();
                self.exprs(EXPR)?;
                self.close(n);
                Ok(())
            }
            T::Raise => {
                let n = self.open(K::RaiseStatement, f::NONE, C_NONE);
                self.bump();
                if can_start_expr(self.peek()) {
                    self.exprs(EXPR)?;
                }
                if self.peek() == T::From {
                    self.bump();
                    self.expr(slot(f::CAUSE, Ctx::Expr))?;
                }
                self.close(n);
                Ok(())
            }
            T::Pass => self.keyword_statement(K::PassStatement),
            T::Break => self.keyword_statement(K::BreakStatement),
            T::Continue => self.keyword_statement(K::ContinueStatement),
            T::Global => self.names_statement(K::GlobalStatement),
            T::Nonlocal => self.names_statement(K::NonlocalStatement),
            T::Assert => {
                let n = self.open(K::AssertStatement, f::NONE, C_NONE);
                self.bump();
                self.expr(EXPR)?;
                while self.peek() == T::Comma {
                    self.bump();
                    self.expr(EXPR)?;
                }
                self.close(n);
                Ok(())
            }
            T::Name if self.is_soft(0, soft::PRINT) && self.is_print() => self.print_statement(),
            // `print(a, *b)`: tree-sitter's parse as a print statement of a tuple lives past the
            // splat and changes its shape; left to it.
            T::Name if self.is_soft(0, soft::PRINT) && self.peek_at(1) == T::LParen && self.late_splat() => self.refuse(),
            T::Name if self.is_soft(0, soft::EXEC) && matches!(self.peek_at(1), T::StrStart | T::Name) && !matches!(self.peek_at(2), T::Dot | T::LParen | T::LBracket) => {
                self.exec_statement()
            }
            T::Name if self.is_soft(0, soft::TYPE) && self.is_type_alias() => self.type_alias(),
            _ => self.expression_statement(),
        }
    }

    /// Whether `type` starts a type alias: an expression follows it, and an `=` this line (as
    /// tree-sitter takes `type(x).y = z`, by the alias's dynamic precedence).
    fn is_type_alias(&self) -> bool {
        if !matches!(self.peek_at(1), T::Name | T::LParen | T::StrStart | T::None | T::Int) {
            return false;
        }
        let (mut depth, mut k) = (0u32, self.i + 1);
        while let Some(t) = self.toks.get(k) {
            match t.kind {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => depth = depth.saturating_sub(1),
                T::Eq if depth == 0 => return true,
                T::Newline | T::Semi | T::Eof | T::Colon if depth == 0 => return false,
                _ if depth == 0 && augmented(t.kind).is_some() => return false,
                _ => {}
            }
            k += 1;
        }
        false
    }

    fn keyword_statement(&mut self, kind: K) -> R<()> {
        self.leaf(kind, f::NONE, C_NONE);
        Ok(())
    }

    fn names_statement(&mut self, kind: K) -> R<()> {
        let n = self.open(kind, f::NONE, C_NONE);
        self.bump();
        loop {
            self.name(f::NONE)?;
            if self.peek() != T::Comma {
                break;
            }
            self.bump();
        }
        self.close(n);
        Ok(())
    }

    /// An identifier, from a name token.
    fn name(&mut self, field: u8) -> R<N> {
        if self.peek() != T::Name {
            return self.refuse();
        }
        Ok(self.leaf(K::Identifier, field, C_NONE))
    }

    /// Whether the parentheses from the token after next hold a `*` after a comma, at their depth.
    fn late_splat(&self) -> bool {
        let (mut depth, mut k, mut comma) = (0u32, self.i + 1, false);
        while let Some(t) = self.toks.get(k) {
            match t.kind {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return false;
                    }
                }
                T::Comma if depth == 1 => comma = true,
                T::Star if depth == 1 && comma => return true,
                T::Eof => return false,
                _ => {}
            }
            k += 1;
        }
        false
    }

    fn is_print(&self) -> bool {
        matches!(self.peek_at(1), T::Name | T::Int | T::Float | T::StrStart | T::True | T::False | T::None | T::Lambda | T::Not | T::Tilde | T::LBrace | T::RShift)
    }

    fn print_statement(&mut self) -> R<()> {
        let n = self.open(K::PrintStatement, f::NONE, C_NONE);
        self.bump();
        if self.peek() == T::RShift {
            let c = self.open(K::Chevron, f::NONE, C_NONE);
            self.bump();
            self.expr(EXPR)?;
            self.close(c);
        } else {
            self.expr(slot(f::ARGUMENT, Ctx::Expr))?;
        }
        while self.peek() == T::Comma {
            let comma = self.bump();
            if can_start_expr(self.peek()) {
                self.expr(slot(f::ARGUMENT, Ctx::Expr))?;
            } else {
                self.leaf_of(comma, K::TComma);
                break;
            }
        }
        self.close(n);
        Ok(())
    }

    fn exec_statement(&mut self) -> R<()> {
        let n = self.open(K::ExecStatement, f::NONE, C_NONE);
        self.bump();
        if self.peek() == T::StrStart {
            self.one_string(slot(f::CODE, Ctx::Bare))?;
        } else {
            self.name(f::CODE)?;
        }
        if self.peek() == T::In {
            self.bump();
            self.expr(EXPR)?;
            while self.peek() == T::Comma {
                self.bump();
                self.expr(EXPR)?;
            }
        }
        if !matches!(self.peek(), T::Newline | T::Semi) {
            return self.refuse();
        }
        self.close(n);
        Ok(())
    }

    fn type_alias(&mut self) -> R<()> {
        let n = self.open(K::TypeAliasStatement, f::NONE, C_NONE);
        self.bump();
        self.ty(f::LEFT)?;
        self.expect(T::Eq)?;
        self.ty(f::RIGHT)?;
        self.close(n);
        Ok(())
    }

    fn import_statement(&mut self) -> R<()> {
        let n = self.open(K::ImportStatement, f::NONE, C_NONE);
        self.bump();
        self.import_list(false)?;
        self.close(n);
        Ok(())
    }

    /// `_import_list`: dotted names, aliased or not; in parentheses where `parens`.
    fn import_list(&mut self, parens: bool) -> R<()> {
        loop {
            let d = self.dotted_name(f::NAME)?;
            if self.peek() == T::As {
                let a = self.wrap(d, K::AliasedImport, slot(f::NAME, Ctx::Bare));
                self.bump();
                self.name(f::ALIAS)?;
                self.close(a);
            }
            if self.peek() != T::Comma {
                break;
            }
            let comma = self.bump();
            if self.peek() != T::Name {
                self.leaf_of(comma, K::TComma);
                if parens {
                    if self.peek() != T::RParen {
                        return self.refuse();
                    }
                    self.leaf(K::TRParen, f::NONE, C_NONE);
                    return Ok(());
                }
                break;
            }
        }
        if parens {
            self.expect(T::RParen)?;
        }
        Ok(())
    }

    fn dotted_name(&mut self, field: u8) -> R<N> {
        let n = self.open(K::DottedName, field, C_NONE);
        self.name(f::NONE)?;
        while self.peek() == T::Dot {
            self.bump();
            self.name(f::NONE)?;
        }
        self.close(n);
        Ok(n)
    }

    fn from_import(&mut self) -> R<()> {
        if self.is_name(1, b"__future__") {
            let n = self.open(K::FutureImportStatement, f::NONE, C_NONE);
            self.leaf(K::TFrom, f::NONE, C_NONE);
            self.leaf(K::TFuture, f::NONE, C_NONE);
            if self.peek() != T::Import {
                return self.refuse();
            }
            self.leaf(K::TImport, f::NONE, C_NONE);
            self.import_names()?;
            self.close(n);
            return Ok(());
        }
        let n = self.open(K::ImportFromStatement, f::NONE, C_NONE);
        self.bump();
        if matches!(self.peek(), T::Dot | T::Ellipsis) {
            let r = self.open(K::RelativeImport, f::MODULE_NAME, C_NONE);
            let p = self.open(K::ImportPrefix, f::NONE, C_NONE);
            while matches!(self.peek(), T::Dot | T::Ellipsis) {
                let t = self.bump();
                for s in t.start..t.end {
                    let d = self.w.push(key(K::TDot, f::NONE, C_NONE, false), s);
                    self.w.close(d, s + 1);
                }
            }
            self.close(p);
            if self.peek() == T::Name {
                self.dotted_name(f::NONE)?;
            }
            self.close(r);
        } else {
            self.dotted_name(f::MODULE_NAME)?;
        }
        if self.peek() != T::Import {
            return self.refuse();
        }
        self.leaf(K::TImport, f::NONE, C_NONE);
        if self.peek() == T::Star {
            let w = self.open(K::WildcardImport, f::NONE, C_NONE);
            self.leaf(K::TStar, f::NONE, C_NONE);
            self.close(w);
        } else {
            self.import_names()?;
        }
        self.close(n);
        Ok(())
    }

    fn import_names(&mut self) -> R<()> {
        if self.peek() == T::LParen {
            self.leaf(K::TLParen, f::NONE, C_NONE);
            self.import_list(true)
        } else {
            self.import_list(false)
        }
    }

    fn expression_statement(&mut self) -> R<()> {
        let st = self.open(K::ExpressionStatement, f::NONE, C_NONE);
        if self.peek() == T::Yield {
            self.yield_expr(f::NONE)?;
            self.close(st);
            return Ok(());
        }
        let first = self.element(EXPR, Splat::Alias)?;
        let mut count = 1;
        let mut trailing = Option::None;
        while self.peek() == T::Comma {
            let comma = self.bump();
            if can_start_expr(self.peek()) && self.peek() != T::Yield {
                self.element(EXPR, Splat::Alias)?;
                count += 1;
            } else {
                trailing = Some(comma);
                break;
            }
        }
        let list = count > 1 || trailing.is_some();
        match self.peek() {
            T::Eq | T::Colon => {
                let kind = self.peek();
                if list {
                    if kind == T::Colon {
                        return self.refuse();
                    }
                    let pl = self.wrap(first, K::PatternList, slot(f::LEFT, Ctx::Bare));
                    if let (Some(comma), true) = (trailing, count > 1) {
                        self.leaf_of(comma, K::TComma);
                    }
                    self.close(pl);
                }
                let a = self.wrap(first, K::Assignment, slot(f::NONE, Ctx::Bare));
                self.to_pattern(a.at + 1, f::LEFT)?;
                self.bump();
                if kind == T::Colon {
                    self.ty(f::TYPE)?;
                    if self.peek() == T::Eq {
                        self.bump();
                        self.rhs(f::RIGHT)?;
                    }
                } else {
                    self.rhs(f::RIGHT)?;
                }
                self.close(a);
            }
            t if augmented(t).is_some() => {
                if list {
                    let pl = self.wrap(first, K::PatternList, slot(f::LEFT, Ctx::Bare));
                    if let (Some(comma), true) = (trailing, count > 1) {
                        self.leaf_of(comma, K::TComma);
                    }
                    self.close(pl);
                }
                let a = self.wrap(first, K::AugmentedAssignment, slot(f::NONE, Ctx::Bare));
                self.to_pattern(a.at + 1, f::LEFT)?;
                self.leaf(augmented(t).expect("an operator"), f::OPERATOR, C_NONE);
                self.rhs(f::RIGHT)?;
                self.close(a);
            }
            _ => {
                if let Some(comma) = trailing {
                    self.leaf_of(comma, K::TComma);
                }
            }
        }
        self.close(st);
        Ok(())
    }

    /// `_right_hand_side`: an expression or a list of them, a yield, or an assignment again.
    fn rhs(&mut self, field: u8) -> R<N> {
        if self.peek() == T::Yield {
            return self.yield_expr(field);
        }
        let e = self.exprs(slot(field, Ctx::Expr))?;
        let t = self.peek();
        if t == T::Eq || augmented(t).is_some() {
            let kind = if t == T::Eq { K::Assignment } else { K::AugmentedAssignment };
            let a = self.wrap(e, kind, slot(field, Ctx::Bare));
            self.to_pattern(a.at + 1, f::LEFT)?;
            if t == T::Eq {
                self.bump();
            } else {
                self.leaf(augmented(t).expect("an operator"), f::OPERATOR, C_NONE);
            }
            self.rhs(f::RIGHT)?;
            self.close(a);
            return Ok(a);
        }
        Ok(e)
    }

    /// `_expressions`: an expression, or an `expression_list` of them.
    fn exprs(&mut self, s: Slot) -> R<N> {
        let first = self.element(s, Splat::Alias)?;
        if self.peek() != T::Comma {
            return Ok(first);
        }
        let l = self.wrap(first, K::ExpressionList, slot(s.field, Ctx::Bare));
        self.rekey(first.at + 1, EXPR);
        let mut count = 1;
        while self.peek() == T::Comma {
            let comma = self.bump();
            if can_start_expr(self.peek()) && self.peek() != T::Yield {
                self.element(EXPR, Splat::Alias)?;
                count += 1;
            } else {
                if count > 1 {
                    self.leaf_of(comma, K::TComma);
                }
                break;
            }
        }
        self.close(l);
        Ok(l)
    }

    fn yield_expr(&mut self, field: u8) -> R<N> {
        let n = self.open(K::Yield, field, C_NONE);
        self.leaf(K::TYield, f::NONE, C_NONE);
        if self.peek() == T::From {
            self.leaf(K::TFrom, f::NONE, C_NONE);
            self.expr(EXPR)?;
        } else if can_start_expr(self.peek()) && self.peek() != T::Yield {
            self.exprs(EXPR)?;
        }
        self.close(n);
        Ok(n)
    }

    /// The expression at `at` made a pattern (an assignment's target), in `field`.
    fn to_pattern(&mut self, at: usize, field: u8) -> R<()> {
        let kind = self.kind_at(at);
        let pat = |k: K| key(k, field, C_PAT, false);
        match kind {
            K::Identifier | K::Attribute | K::Subscript => self.w.set_key(at, pat(kind)),
            K::Tuple | K::ParenthesizedExpression => {
                self.w.set_key(at, pat(K::TuplePattern));
                for c in self.children(at) {
                    self.to_pattern(c, f::NONE)?;
                }
            }
            K::List => {
                self.w.set_key(at, pat(K::ListPattern));
                for c in self.children(at) {
                    self.to_pattern(c, f::NONE)?;
                }
            }
            K::ExpressionList | K::PatternList => {
                self.w.set_key(at, key(K::PatternList, field, C_NONE, false));
                for c in self.children(at) {
                    self.to_pattern(c, f::NONE)?;
                }
            }
            K::ListSplat => {
                self.w.set_key(at, pat(K::ListSplatPattern));
                for c in self.children(at) {
                    let k = self.kind_at(c);
                    if !matches!(k, K::Identifier | K::Attribute | K::Subscript) {
                        return Err(Refusal::Parse(self.w.start(c)));
                    }
                    self.w.set_key(c, key(k, f::NONE, C_NONE, false));
                }
            }
            _ => return Err(Refusal::Parse(self.w.start(at))),
        }
        Ok(())
    }

    // ---- Compound statements.

    /// A suite after its `:`: an indented block, the rest of the line, or nothing.
    fn suite(&mut self, field: u8) -> R<()> {
        match self.peek() {
            T::Indent => {
                self.bump();
                let mut b = self.open(K::Block, field, C_NONE);
                while !matches!(self.peek(), T::Dedent | T::Eof) {
                    self.statement_or_recovered(Some(&mut b))?;
                }
                // The text's end closes a block too: one recovered inside brackets, where the
                // lexer gives no dedents.
                if self.peek() != T::Eof {
                    self.expect(T::Dedent)?;
                }
                self.close(b);
            }
            T::Newline => {
                let b = self.open(K::Block, field, C_NONE);
                self.bump();
                self.close(b);
            }
            _ => {
                let b = self.open(K::Block, field, C_NONE);
                self.simple_statements()?;
                self.close(b);
            }
        }
        Ok(())
    }

    fn colon_suite(&mut self, field: u8) -> R<()> {
        self.expect(T::Colon)?;
        self.suite(field)
    }

    fn if_statement(&mut self) -> R<()> {
        let n = self.open(K::IfStatement, f::NONE, C_NONE);
        self.bump();
        self.expr(slot(f::CONDITION, Ctx::Expr))?;
        self.colon_suite(f::CONSEQUENCE)?;
        while self.peek() == T::Elif {
            let e = self.open(K::ElifClause, f::ALTERNATIVE, C_NONE);
            self.bump();
            self.expr(slot(f::CONDITION, Ctx::Expr))?;
            self.colon_suite(f::CONSEQUENCE)?;
            self.close(e);
        }
        self.else_clause(f::ALTERNATIVE)?;
        self.close(n);
        Ok(())
    }

    fn else_clause(&mut self, field: u8) -> R<()> {
        if self.peek() == T::Else {
            let e = self.open(K::ElseClause, field, C_NONE);
            self.bump();
            self.colon_suite(f::BODY)?;
            self.close(e);
        }
        Ok(())
    }

    fn while_statement(&mut self) -> R<()> {
        let n = self.open(K::WhileStatement, f::NONE, C_NONE);
        self.bump();
        self.expr(slot(f::CONDITION, Ctx::Expr))?;
        self.colon_suite(f::BODY)?;
        self.else_clause(f::ALTERNATIVE)?;
        self.close(n);
        Ok(())
    }

    fn for_statement(&mut self) -> R<()> {
        let n = self.open(K::ForStatement, f::NONE, C_NONE);
        if self.peek() == T::Name {
            self.leaf(K::TAsync, f::NONE, C_NONE);
        }
        self.leaf(K::TFor, f::NONE, C_NONE);
        self.targets(f::LEFT)?;
        self.expect(T::In)?;
        self.exprs(slot(f::RIGHT, Ctx::Expr))?;
        self.colon_suite(f::BODY)?;
        self.else_clause(f::ALTERNATIVE)?;
        self.close(n);
        Ok(())
    }

    fn try_statement(&mut self) -> R<()> {
        let n = self.open(K::TryStatement, f::NONE, C_NONE);
        self.bump();
        self.colon_suite(f::BODY)?;
        while self.peek() == T::Except {
            let c = self.open(K::ExceptClause, f::NONE, C_NONE);
            self.leaf(K::TExcept, f::NONE, C_NONE);
            if self.peek() == T::Star {
                self.leaf(K::TStar, f::NONE, C_NONE);
            }
            if self.peek() != T::Colon {
                let e = self.expr(slot(f::VALUE, Ctx::Expr))?;
                if self.peek() == T::As {
                    self.as_pattern(e, slot(f::VALUE, Ctx::Expr))?;
                } else {
                    while self.peek() == T::Comma {
                        self.bump();
                        self.expr(slot(f::VALUE, Ctx::Expr))?;
                    }
                }
            }
            if self.peek() != T::Colon {
                return self.refuse();
            }
            self.leaf(K::TColon, f::NONE, C_NONE);
            self.suite(f::NONE)?;
            self.close(c);
        }
        self.else_clause(f::NONE)?;
        if self.peek() == T::Finally {
            let c = self.open(K::FinallyClause, f::NONE, C_NONE);
            self.bump();
            self.colon_suite(f::NONE)?;
            self.close(c);
        }
        self.close(n);
        Ok(())
    }

    /// `e as target`: an `as_pattern` around `e`, in `s`.
    fn as_pattern(&mut self, e: N, s: Slot) -> R<N> {
        if e.kind == K::ConditionalExpression {
            // `as` binds tighter than a conditional: its last operand is the pattern's.
            let last = *self.children(e.at).last().expect("an operand");
            let inner = N { at: last, kind: self.kind_at(last) };
            self.as_pattern(inner, EXPR)?;
            self.w.close(e.at, self.last_end);
            return Ok(e);
        }
        let a = self.wrap(e, K::AsPattern, s);
        self.rekey(e.at + 1, EXPR);
        self.bump();
        let t = self.open(K::AsPatternTarget, f::ALIAS, C_NONE);
        self.expr(slot(f::NONE, Ctx::Primary))?;
        self.close(t);
        self.close(a);
        Ok(a)
    }

    fn with_statement(&mut self) -> R<()> {
        let n = self.open(K::WithStatement, f::NONE, C_NONE);
        if self.peek() == T::Name {
            self.leaf(K::TAsync, f::NONE, C_NONE);
        }
        self.leaf(K::TWith, f::NONE, C_NONE);
        let c = self.open(K::WithClause, f::NONE, C_NONE);
        let parens = self.peek() == T::LParen && self.paren_is_clause();
        if parens {
            self.leaf(K::TLParen, f::NONE, C_NONE);
        }
        loop {
            let item = self.open(K::WithItem, f::NONE, C_NONE);
            let e = self.expr(slot(f::VALUE, Ctx::Expr))?;
            if self.peek() == T::As {
                self.as_pattern(e, slot(f::VALUE, Ctx::Expr))?;
            }
            self.close(item);
            if self.peek() != T::Comma {
                if parens {
                    self.expect(T::RParen)?;
                }
                break;
            }
            let comma = self.bump();
            if (parens && self.peek() == T::RParen) || (!parens && self.peek() == T::Colon) {
                self.leaf_of(comma, K::TComma);
                if parens {
                    self.leaf(K::TRParen, f::NONE, C_NONE);
                }
                break;
            }
        }
        self.close(c);
        self.colon_suite(f::BODY)?;
        self.close(n);
        Ok(())
    }

    /// Whether the `(` after `with` opens the clause's parentheses (items in them) rather than an
    /// expression: tree-sitter's choice, by what is inside and what follows.
    fn paren_is_clause(&self) -> bool {
        let (mut depth, mut k) = (0i32, self.i);
        let (mut has_as, mut commas, mut last) = (false, 0, T::Eof);
        loop {
            let Some(t) = self.toks.get(k) else { return false };
            match t.kind {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                T::As if depth == 1 => has_as = true,
                T::Comma if depth == 1 => commas += 1,
                T::Eof | T::Newline => return false,
                _ => {}
            }
            if depth >= 1 {
                last = t.kind;
            }
            k += 1;
        }
        if self.toks.get(k + 1).map(|t| t.kind) != Some(T::Colon) {
            return false;
        }
        // One item in them is an expression (an `as` in it a pattern), one and a comma a tuple.
        (has_as && commas > 0) || commas > 1 || (commas == 1 && last != T::Comma)
    }

    fn function_definition(&mut self, field: u8) -> R<()> {
        let n = self.open(K::FunctionDefinition, field, C_NONE);
        if self.peek() == T::Name {
            self.leaf(K::TAsync, f::NONE, C_NONE);
        }
        self.leaf(K::TDef, f::NONE, C_NONE);
        self.name(f::NAME)?;
        if self.peek() == T::LBracket {
            self.type_parameter(f::TYPE_PARAMETERS)?;
        }
        if self.peek() != T::LParen {
            return self.refuse();
        }
        let p = self.open(K::Parameters, f::PARAMETERS, C_NONE);
        self.bump();
        self.parameters(true)?;
        self.close(p);
        if self.peek() == T::Arrow {
            self.bump();
            self.ty(f::RETURN_TYPE)?;
        }
        self.colon_suite(f::BODY)?;
        self.close(n);
        Ok(())
    }

    /// Parameters to their closer: `)` (read here) where `def`, the lambda's `:` (left) otherwise.
    fn parameters(&mut self, def: bool) -> R<()> {
        let closer = if def { T::RParen } else { T::Colon };
        while self.peek() != closer {
            self.parameter(def)?;
            if self.peek() != T::Comma {
                break;
            }
            let comma = self.bump();
            if self.peek() == closer {
                self.leaf_of(comma, K::TComma);
                if def {
                    self.leaf(K::TRParen, f::NONE, C_NONE);
                    return Ok(());
                }
                return Ok(());
            }
        }
        if def {
            self.expect(T::RParen)?;
        }
        Ok(())
    }

    fn parameter(&mut self, def: bool) -> R<()> {
        let param = slot(f::NONE, Ctx::Param);
        match self.peek() {
            T::Star | T::Pow => {
                let star = self.peek() == T::Star;
                if star && matches!(self.peek_at(1), T::Comma | T::RParen | T::Colon) {
                    let n = self.open_in(K::KeywordSeparator, param);
                    self.leaf(K::TStar, f::NONE, C_NONE);
                    self.close(n);
                    return Ok(());
                }
                let kind = if star { K::ListSplatPattern } else { K::DictionarySplatPattern };
                let n = self.open_in(kind, param);
                self.bump();
                self.name(f::NONE)?;
                self.close(n);
                if def && self.peek() == T::Colon {
                    let t = self.wrap(n, K::TypedParameter, param);
                    self.w.set_key(n.at + 1, key(kind, f::NONE, C_NONE, false));
                    self.bump();
                    self.ty(f::TYPE)?;
                    self.close(t);
                }
            }
            T::Slash => {
                let n = self.open_in(K::PositionalSeparator, param);
                self.leaf(K::TSlash, f::NONE, C_NONE);
                self.close(n);
            }
            T::LParen => {
                let t = self.pattern(param)?;
                if self.peek() == T::Eq {
                    let d = self.wrap(t, K::DefaultParameter, param);
                    self.w.set_key(t.at + 1, key(K::TuplePattern, f::NAME, C_NONE, false));
                    self.bump();
                    self.expr(slot(f::VALUE, Ctx::Expr))?;
                    self.close(d);
                }
            }
            T::Name => {
                if def && self.peek_at(1) == T::Colon {
                    let n = self.open_in(K::TypedParameter, param);
                    self.name(f::NONE)?;
                    self.bump();
                    self.ty(f::TYPE)?;
                    if self.peek() == T::Eq {
                        self.w.set_key(n.at, key(K::TypedDefaultParameter, f::NONE, C_PARAM, false));
                        self.w.set_key(n.at + 1, key(K::Identifier, f::NAME, C_NONE, false));
                        self.bump();
                        self.expr(slot(f::VALUE, Ctx::Expr))?;
                    }
                    self.close(n);
                } else if self.peek_at(1) == T::Eq {
                    let n = self.open_in(K::DefaultParameter, param);
                    self.name(f::NAME)?;
                    self.bump();
                    self.expr(slot(f::VALUE, Ctx::Expr))?;
                    self.close(n);
                } else {
                    self.leaf(K::Identifier, f::NONE, C_PARAM);
                }
            }
            _ => return self.refuse(),
        }
        Ok(())
    }

    fn class_definition(&mut self, field: u8) -> R<()> {
        let n = self.open(K::ClassDefinition, field, C_NONE);
        self.bump();
        self.name(f::NAME)?;
        if self.peek() == T::LBracket {
            self.type_parameter(f::TYPE_PARAMETERS)?;
        }
        if self.peek() == T::LParen {
            self.arguments(f::SUPERCLASSES, false)?;
        }
        self.colon_suite(f::BODY)?;
        self.close(n);
        Ok(())
    }

    fn decorated(&mut self) -> R<()> {
        let d = self.open(K::DecoratedDefinition, f::NONE, C_NONE);
        while self.peek() == T::At {
            let n = self.open(K::Decorator, f::NONE, C_NONE);
            self.bump();
            self.expr(EXPR)?;
            self.expect(T::Newline)?;
            self.close(n);
        }
        match self.peek() {
            T::Def => self.function_definition(f::DEFINITION)?,
            T::Class => self.class_definition(f::DEFINITION)?,
            T::Name if self.is_soft(0, soft::ASYNC) && self.peek_at(1) == T::Def => self.function_definition(f::DEFINITION)?,
            _ => return self.refuse(),
        }
        self.close(d);
        Ok(())
    }

    /// Whether `match` starts a match statement: its line ends in a header's `:`.
    fn is_match(&self) -> bool {
        if matches!(self.peek_at(1), T::Eq | T::Dot | T::Colon | T::Comma | T::RParen | T::Newline | T::Semi) || augmented(self.peek_at(1)).is_some() {
            return false;
        }
        let mut k = self.i + 1;
        while let Some(t) = self.toks.get(k) {
            match t.kind {
                T::Indent => return self.toks[k - 1].kind == T::Colon,
                T::Newline | T::Eof => return false,
                _ => k += 1,
            }
        }
        false
    }

    fn match_statement(&mut self) -> R<()> {
        let n = self.open(K::MatchStatement, f::NONE, C_NONE);
        self.bump();
        self.expr(slot(f::SUBJECT, Ctx::Expr))?;
        while self.peek() == T::Comma {
            let comma = self.bump();
            if self.peek() == T::Colon {
                self.leaf_of(comma, K::TComma);
                break;
            }
            self.expr(slot(f::SUBJECT, Ctx::Expr))?;
        }
        if self.peek() != T::Colon {
            return self.refuse();
        }
        self.leaf(K::TColon, f::NONE, C_NONE);
        if self.peek() != T::Indent {
            return self.refuse();
        }
        let b = self.open(K::Block, f::BODY, C_NONE);
        self.bump();
        while self.is_soft(0, soft::CASE) {
            self.case_clause()?;
        }
        self.expect(T::Dedent)?;
        self.close(b);
        self.close(n);
        Ok(())
    }

    fn case_clause(&mut self) -> R<()> {
        let c = self.open(K::CaseClause, f::ALTERNATIVE, C_NONE);
        self.bump();
        self.case_pattern(f::NONE)?;
        while self.peek() == T::Comma {
            let comma = self.bump();
            if matches!(self.peek(), T::If | T::Colon) {
                self.leaf_of(comma, K::TComma);
                break;
            }
            self.case_pattern(f::NONE)?;
        }
        if self.peek() == T::If {
            let g = self.open(K::IfClause, f::GUARD, C_NONE);
            self.bump();
            self.expr(EXPR)?;
            self.close(g);
        }
        if self.peek() != T::Colon {
            return self.refuse();
        }
        self.leaf(K::TColon, f::NONE, C_NONE);
        self.suite(f::CONSEQUENCE)?;
        self.close(c);
        Ok(())
    }

    fn case_pattern(&mut self, field: u8) -> R<N> {
        self.deeper()?;
        let cp = self.open(K::CasePattern, field, C_NONE);
        if self.is_soft(0, soft::WILD) && !matches!(self.peek_at(1), T::Dot | T::LParen) {
            self.bump();
        } else if self.peek() == T::Name && self.peek_at(1) == T::Eq {
            let k = self.open(K::KeywordPattern, f::NONE, C_NONE);
            self.name(f::NONE)?;
            self.leaf(K::TEq, f::NONE, C_NONE);
            self.simple_pattern(f::NONE)?;
            self.close(k);
        } else {
            self.simple_pattern(f::NONE)?;
        }
        self.close(cp);
        let mut out = cp;
        while self.peek() == T::As {
            let a = self.wrap(out, K::AsPattern, slot(f::NONE, Ctx::Bare));
            let outer = self.wrap(a, K::CasePattern, slot(field, Ctx::Bare));
            self.w.set_key(outer.at + 2, key(K::CasePattern, f::NONE, C_NONE, false));
            self.bump();
            self.name(f::NONE)?;
            self.w.close(outer.at + 1, self.last_end);
            self.close(outer);
            out = outer;
        }
        self.depth -= 1;
        Ok(out)
    }

    /// `_simple_pattern`, or a union of them, into the node open; the first child in `field`.
    fn simple_pattern(&mut self, field: u8) -> R<()> {
        let first = self.pattern_atom(field)?;
        if self.peek() == T::Pipe {
            let u = self.wrap(first, K::UnionPattern, slot(field, Ctx::Bare));
            if field != f::NONE {
                self.rekey(first.at + 1, slot(f::NONE, Ctx::Bare));
            }
            while self.peek() == T::Pipe {
                self.bump();
                self.pattern_atom(f::NONE)?;
            }
            self.close(u);
        }
        Ok(())
    }

    fn pattern_atom(&mut self, field: u8) -> R<N> {
        let bare = slot(field, Ctx::Bare);
        Ok(match self.peek() {
            T::Name => {
                let d = self.dotted_name(field)?;
                if self.peek() == T::LParen {
                    let c = self.wrap(d, K::ClassPattern, bare);
                    self.rekey(d.at + 1, slot(f::NONE, Ctx::Bare));
                    self.bump();
                    let mut closed = false;
                    while self.peek() != T::RParen {
                        self.case_pattern(f::NONE)?;
                        if self.peek() != T::Comma {
                            break;
                        }
                        let comma = self.bump();
                        if self.peek() == T::RParen {
                            self.leaf_of(comma, K::TComma);
                            self.leaf(K::TRParen, f::NONE, C_NONE);
                            closed = true;
                            break;
                        }
                    }
                    if !closed {
                        self.expect(T::RParen)?;
                    }
                    self.close(c);
                    return Ok(c);
                }
                d
            }
            T::Minus | T::Int | T::Float => {
                // `-`? number, and a complex pattern where `+`/`-` and a number follow.
                let neg = self.peek() == T::Minus;
                let o = usize::from(neg);
                if !matches!(self.peek_at(o), T::Int | T::Float) {
                    return self.refuse();
                }
                let complex = matches!(self.peek_at(o + 1), T::Plus | T::Minus) && matches!(self.peek_at(o + 2), T::Int | T::Float);
                let c = complex.then(|| self.open(K::ComplexPattern, field, C_NONE));
                let first = if neg { self.leaf(K::TMinus, if complex { f::NONE } else { field }, C_NONE) } else { N { at: usize::MAX, kind: K::TMinus } };
                let num = self.number(if neg || complex { f::NONE } else { field });
                if let Some(c) = c {
                    let op = if self.peek() == T::Plus { K::TPlus } else { K::TMinus };
                    self.leaf(op, f::NONE, C_NONE);
                    self.number(f::NONE);
                    self.close(c);
                    return Ok(c);
                }
                if neg { first } else { num }
            }
            T::StrStart => self.string(bare)?,
            T::True => self.leaf(K::True, field, C_NONE),
            T::False => self.leaf(K::False, field, C_NONE),
            T::None => self.leaf(K::None, field, C_NONE),
            T::LParen | T::LBracket => {
                let (kind, closer, closing) = if self.peek() == T::LParen { (K::TuplePattern, T::RParen, K::TRParen) } else { (K::ListPattern, T::RBracket, K::TRBracket) };
                let n = self.open(kind, field, C_NONE);
                self.bump();
                let mut closed = false;
                while self.peek() != closer {
                    self.case_pattern(f::NONE)?;
                    if self.peek() != T::Comma {
                        break;
                    }
                    let comma = self.bump();
                    if self.peek() == closer {
                        self.leaf_of(comma, K::TComma);
                        self.leaf(closing, f::NONE, C_NONE);
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    self.expect(closer)?;
                }
                self.close(n);
                n
            }
            T::LBrace => {
                let n = self.open(K::DictPattern, field, C_NONE);
                self.bump();
                let mut closed = false;
                while self.peek() != T::RBrace {
                    if matches!(self.peek(), T::Star | T::Pow) {
                        self.splat_pattern()?;
                    } else {
                        self.simple_pattern(f::KEY)?;
                        self.expect(T::Colon)?;
                        self.case_pattern(f::VALUE)?;
                    }
                    if self.peek() != T::Comma {
                        break;
                    }
                    let comma = self.bump();
                    if self.peek() == T::RBrace {
                        self.leaf_of(comma, K::TComma);
                        self.leaf(K::TRBrace, f::NONE, C_NONE);
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    self.expect(T::RBrace)?;
                }
                self.close(n);
                n
            }
            T::Star | T::Pow => self.splat_pattern()?,
            _ => return self.refuse(),
        })
    }

    fn number(&mut self, field: u8) -> N {
        let kind = if self.peek() == T::Int { K::Integer } else { K::Float };
        self.leaf(kind, field, C_NONE)
    }

    fn splat_pattern(&mut self) -> R<N> {
        let n = self.open(K::SplatPattern, f::NONE, C_NONE);
        let op = if self.peek() == T::Star { K::TStar } else { K::TPow };
        self.leaf(op, f::NONE, C_NONE);
        if self.is_soft(0, soft::WILD) {
            self.bump();
        } else {
            self.name(f::NONE)?;
        }
        self.close(n);
        Ok(n)
    }

    // ---- Targets.

    /// `_left_hand_side` read as patterns (a `for`'s target), in `field`.
    fn targets(&mut self, field: u8) -> R<N> {
        let first = self.pattern(slot(field, Ctx::Pattern))?;
        if self.peek() != T::Comma {
            return Ok(first);
        }
        let l = self.wrap(first, K::PatternList, slot(field, Ctx::Bare));
        self.rekey(first.at + 1, slot(f::NONE, Ctx::Pattern));
        let mut count = 1;
        while self.peek() == T::Comma {
            let comma = self.bump();
            if matches!(self.peek(), T::Name | T::LParen | T::LBracket | T::Star) {
                self.pattern(slot(f::NONE, Ctx::Pattern))?;
                count += 1;
            } else {
                if count > 1 {
                    self.leaf_of(comma, K::TComma);
                }
                break;
            }
        }
        self.close(l);
        Ok(l)
    }

    /// A `pattern` (or, in a parameter's slot, a tuple pattern).
    fn pattern(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let n = match self.peek() {
            T::Star => {
                let n = self.open_in(K::ListSplatPattern, s);
                self.bump();
                let e = self.postfix(slot(f::NONE, Ctx::Bare))?;
                if !matches!(e.kind, K::Identifier | K::Attribute | K::Subscript) {
                    return self.refuse();
                }
                self.close(n);
                n
            }
            T::LParen | T::LBracket => {
                let (kind, closer, closing) = if self.peek() == T::LParen { (K::TuplePattern, T::RParen, K::TRParen) } else { (K::ListPattern, T::RBracket, K::TRBracket) };
                let n = self.open_in(kind, s);
                self.bump();
                let mut closed = false;
                while self.peek() != closer {
                    self.pattern(slot(f::NONE, Ctx::Pattern))?;
                    if self.peek() != T::Comma {
                        break;
                    }
                    let comma = self.bump();
                    if self.peek() == closer {
                        self.leaf_of(comma, K::TComma);
                        self.leaf(closing, f::NONE, C_NONE);
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    self.expect(closer)?;
                }
                self.close(n);
                n
            }
            _ => {
                let e = self.postfix(s)?;
                if !matches!(e.kind, K::Identifier | K::Attribute | K::Subscript) {
                    return self.refuse();
                }
                e
            }
        };
        self.depth -= 1;
        Ok(n)
    }

    // ---- Expressions.

    /// An element of a list: an expression, or a splat as `mode` makes it.
    fn element(&mut self, s: Slot, mode: Splat) -> R<N> {
        if self.peek() != T::Star {
            return self.expr(s);
        }
        if mode == Splat::Alias {
            if self.peek_at(1) != T::Name {
                return self.refuse();
            }
            let n = self.open_in(K::ListSplat, s);
            self.bump();
            self.leaf(K::Identifier, f::NONE, C_NONE);
            self.close(n);
            let e = self.postfix_from(n, s)?;
            if binop(self.peek()).is_some() || comparison(self.peek()).is_some() || matches!(self.peek(), T::If | T::And | T::Or | T::Not | T::Is | T::Walrus) {
                return self.refuse();
            }
            return Ok(e);
        }
        let n = self.open(K::ListSplat, s.field, C_NONE);
        self.bump();
        self.expr(EXPR)?;
        self.close(n);
        Ok(n)
    }

    fn expr(&mut self, s: Slot) -> R<N> {
        self.expr_x(s, false)
    }

    /// `expression`; where `nocond`, without a conditional (a comprehension's `in` and `if`).
    fn expr_x(&mut self, s: Slot, nocond: bool) -> R<N> {
        self.deeper()?;
        let n = if self.peek() == T::Lambda {
            self.lambda(s, nocond)?
        } else {
            let left = self.disjunction(s)?;
            if self.peek() == T::Walrus {
                if left.kind != K::Identifier {
                    return self.refuse();
                }
                let n = self.wrap(left, K::NamedExpression, s);
                self.w.set_key(left.at + 1, key(K::Identifier, f::NAME, C_NONE, false));
                self.bump();
                self.expr(slot(f::VALUE, Ctx::Expr))?;
                self.close(n);
                n
            } else if self.peek() == T::If && !nocond {
                let n = self.wrap(left, K::ConditionalExpression, s);
                self.rekey(left.at + 1, EXPR);
                self.bump();
                self.expr(EXPR)?;
                self.expect(T::Else)?;
                self.expr(EXPR)?;
                self.close(n);
                n
            } else {
                left
            }
        };
        self.depth -= 1;
        Ok(n)
    }

    fn lambda(&mut self, s: Slot, nocond: bool) -> R<N> {
        let n = self.open_in(K::Lambda, s);
        self.bump();
        if self.peek() != T::Colon {
            let p = self.open(K::LambdaParameters, f::PARAMETERS, C_NONE);
            self.parameters(false)?;
            self.close(p);
        }
        self.expect(T::Colon)?;
        self.expr_x(slot(f::BODY, Ctx::Expr), nocond)?;
        self.close(n);
        Ok(n)
    }

    fn disjunction(&mut self, s: Slot) -> R<N> {
        let mut left = self.conjunction(s)?;
        while self.peek() == T::Or {
            let n = self.wrap(left, K::BooleanOperator, s);
            self.rekey(left.at + 1, slot(f::LEFT, Ctx::Expr));
            self.leaf(K::TOr, f::OPERATOR, C_NONE);
            self.conjunction(slot(f::RIGHT, Ctx::Expr))?;
            self.close(n);
            left = n;
        }
        Ok(left)
    }

    fn conjunction(&mut self, s: Slot) -> R<N> {
        let mut left = self.inversion(s)?;
        while self.peek() == T::And {
            let n = self.wrap(left, K::BooleanOperator, s);
            self.rekey(left.at + 1, slot(f::LEFT, Ctx::Expr));
            self.leaf(K::TAnd, f::OPERATOR, C_NONE);
            self.inversion(slot(f::RIGHT, Ctx::Expr))?;
            self.close(n);
            left = n;
        }
        Ok(left)
    }

    fn inversion(&mut self, s: Slot) -> R<N> {
        if self.peek() != T::Not {
            return self.comparison(s);
        }
        self.deeper()?;
        let n = self.open_in(K::NotOperator, s);
        self.bump();
        self.inversion(slot(f::ARGUMENT, Ctx::Expr))?;
        self.close(n);
        self.depth -= 1;
        Ok(n)
    }

    fn is_comparison(&self) -> bool {
        comparison(self.peek()).is_some() || self.peek() == T::Is || (self.peek() == T::Not && self.peek_at(1) == T::In)
    }

    fn comparison(&mut self, s: Slot) -> R<N> {
        let left = self.binary(s, 14)?;
        if !self.is_comparison() {
            return Ok(left);
        }
        let n = self.wrap(left, K::ComparisonOperator, s);
        self.rekey(left.at + 1, slot(f::NONE, Ctx::Primary));
        while self.is_comparison() {
            match self.peek() {
                T::Not => {
                    let o = self.open(K::NotIn, f::OPERATORS, C_NONE);
                    self.leaf(K::TNot, f::NONE, C_NONE);
                    self.leaf(K::TIn, f::NONE, C_NONE);
                    self.close(o);
                }
                T::Is if self.peek_at(1) == T::Not => {
                    let o = self.open(K::IsNot, f::OPERATORS, C_NONE);
                    self.leaf(K::TIs, f::NONE, C_NONE);
                    self.leaf(K::TNot, f::NONE, C_NONE);
                    self.close(o);
                }
                T::Is => {
                    self.leaf(K::TIs, f::OPERATORS, C_NONE);
                }
                t => {
                    self.leaf(comparison(t).expect("a comparison"), f::OPERATORS, C_NONE);
                }
            }
            self.binary(slot(f::NONE, Ctx::Primary), 14)?;
        }
        self.close(n);
        Ok(n)
    }

    /// Binary operators from precedence `min` up, by precedence climbing.
    fn binary(&mut self, s: Slot, min: u8) -> R<N> {
        self.deeper()?;
        let mut left = self.unary(s)?;
        while let Some((prec, op)) = binop(self.peek()) {
            if prec < min {
                break;
            }
            let n = self.wrap(left, K::BinaryOperator, s);
            self.rekey(left.at + 1, slot(f::LEFT, Ctx::Primary));
            self.leaf(op, f::OPERATOR, C_NONE);
            self.binary(slot(f::RIGHT, Ctx::Primary), if op == K::TPow { prec } else { prec + 1 })?;
            self.close(n);
            left = n;
        }
        self.depth -= 1;
        Ok(left)
    }

    fn unary(&mut self, s: Slot) -> R<N> {
        match self.peek() {
            T::Plus | T::Minus | T::Tilde => {
                let op = match self.peek() {
                    T::Plus => K::TPlus,
                    T::Minus => K::TMinus,
                    _ => K::TTilde,
                };
                let n = self.open_in(K::UnaryOperator, s);
                self.leaf(op, f::OPERATOR, C_NONE);
                self.binary(slot(f::ARGUMENT, Ctx::Primary), 21)?;
                self.close(n);
                Ok(n)
            }
            T::Name if self.is_soft(0, soft::AWAIT) && matches!(self.peek_at(1), T::Name | T::Int | T::Float | T::StrStart | T::LParen | T::LBracket | T::LBrace | T::True | T::False | T::None | T::Ellipsis) => {
                let n = self.open_in(K::Await, s);
                self.bump();
                self.binary(slot(f::NONE, Ctx::Primary), 21)?;
                self.close(n);
                Ok(n)
            }
            _ => self.postfix(s),
        }
    }

    fn postfix(&mut self, s: Slot) -> R<N> {
        let a = self.atom(s)?;
        self.postfix_from(a, s)
    }

    /// Attributes, calls and subscripts after `left`, in `s`.
    fn postfix_from(&mut self, mut left: N, s: Slot) -> R<N> {
        loop {
            match self.peek() {
                T::Dot => {
                    let n = self.wrap(left, K::Attribute, s);
                    self.rekey(left.at + 1, slot(f::OBJECT, Ctx::Primary));
                    self.bump();
                    self.name(f::ATTRIBUTE)?;
                    self.close(n);
                    left = n;
                }
                T::LParen => {
                    let n = self.wrap(left, K::Call, s);
                    self.rekey(left.at + 1, slot(f::FUNCTION, Ctx::Primary));
                    self.arguments(f::ARGUMENTS, true)?;
                    self.close(n);
                    left = n;
                }
                T::LBracket => {
                    let n = self.wrap(left, K::Subscript, s);
                    self.rekey(left.at + 1, slot(f::VALUE, Ctx::Primary));
                    self.subscript()?;
                    self.close(n);
                    left = n;
                }
                _ => return Ok(left),
            }
        }
    }

    /// An argument list (or, where `generator` and its one argument is followed by `for`, a
    /// generator expression), in `field`.
    fn arguments(&mut self, field: u8, generator: bool) -> R<()> {
        self.deeper()?;
        let n = self.open(K::ArgumentList, field, C_NONE);
        self.bump();
        let mut first = true;
        while self.peek() != T::RParen {
            match self.peek() {
                T::Star => {
                    self.element(EXPR, Splat::Collection)?;
                }
                T::Pow => {
                    let d = self.open(K::DictionarySplat, f::NONE, C_NONE);
                    self.bump();
                    self.expr(EXPR)?;
                    self.close(d);
                }
                T::Name if self.peek_at(1) == T::Eq => {
                    let k = self.open(K::KeywordArgument, f::NONE, C_NONE);
                    self.name(f::NAME)?;
                    self.bump();
                    self.expr(slot(f::VALUE, Ctx::Expr))?;
                    self.close(k);
                }
                _ => {
                    let e = self.expr(EXPR)?;
                    if first && generator && self.starts_for() {
                        self.w.set_key(n.at, key(K::GeneratorExpression, field, C_NONE, false));
                        self.rekey(e.at, slot(f::BODY, Ctx::Expr));
                        self.comprehension()?;
                        self.expect(T::RParen)?;
                        self.close(n);
                        self.depth -= 1;
                        return Ok(());
                    }
                }
            }
            first = false;
            if self.peek() != T::Comma {
                break;
            }
            let comma = self.bump();
            if self.peek() == T::RParen {
                self.leaf_of(comma, K::TComma);
                self.leaf(K::TRParen, f::NONE, C_NONE);
                self.close(n);
                self.depth -= 1;
                return Ok(());
            }
        }
        self.expect(T::RParen)?;
        self.close(n);
        self.depth -= 1;
        Ok(())
    }

    fn starts_for(&self) -> bool {
        self.peek() == T::For || (self.is_soft(0, soft::ASYNC) && self.peek_at(1) == T::For)
    }

    /// `_comprehension_clauses`: `for … in …`, then more of those and `if`s.
    fn comprehension(&mut self) -> R<()> {
        loop {
            if self.starts_for() {
                let c = self.open(K::ForInClause, f::NONE, C_NONE);
                if self.peek() == T::Name {
                    self.leaf(K::TAsync, f::NONE, C_NONE);
                }
                self.leaf(K::TFor, f::NONE, C_NONE);
                self.targets(f::LEFT)?;
                self.expect(T::In)?;
                self.expr_x(slot(f::RIGHT, Ctx::Expr), true)?;
                while self.peek() == T::Comma {
                    let comma = self.bump();
                    if can_start_expr(self.peek()) {
                        self.expr_x(slot(f::RIGHT, Ctx::Expr), true)?;
                    } else {
                        self.leaf_of(comma, K::TComma);
                        break;
                    }
                }
                self.close(c);
            } else if self.peek() == T::If {
                let c = self.open(K::IfClause, f::NONE, C_NONE);
                self.bump();
                self.expr_x(EXPR, true)?;
                self.close(c);
            } else {
                return Ok(());
            }
        }
    }

    fn subscript(&mut self) -> R<()> {
        self.bump();
        let sub = slot(f::SUBSCRIPT, Ctx::Expr);
        loop {
            if self.peek() == T::Colon {
                let n = self.open(K::Slice, f::SUBSCRIPT, C_NONE);
                self.slice_rest()?;
                self.close(n);
            } else {
                let e = self.element(sub, Splat::Alias)?;
                if self.peek() == T::Colon {
                    let n = self.wrap(e, K::Slice, slot(f::SUBSCRIPT, Ctx::Bare));
                    self.rekey(e.at + 1, EXPR);
                    self.slice_rest()?;
                    self.close(n);
                }
            }
            if self.peek() != T::Comma {
                break;
            }
            let comma = self.bump();
            if self.peek() == T::RBracket {
                self.leaf_of(comma, K::TComma);
                self.leaf(K::TRBracket, f::NONE, C_NONE);
                return Ok(());
            }
        }
        self.expect(T::RBracket)?;
        Ok(())
    }

    /// A slice from its first `:`: every `:` stored, the bounds that are there.
    fn slice_rest(&mut self) -> R<()> {
        self.leaf(K::TColon, f::NONE, C_NONE);
        if can_start_expr(self.peek()) && self.peek() != T::Star {
            self.expr(EXPR)?;
        }
        if self.peek() == T::Colon {
            self.leaf(K::TColon, f::NONE, C_NONE);
            if can_start_expr(self.peek()) && self.peek() != T::Star {
                self.expr(EXPR)?;
            }
        }
        Ok(())
    }

    fn atom(&mut self, s: Slot) -> R<N> {
        let leaf = |p: &mut Self, kind: K| Ok(p.leaf(kind, s.field, class(kind, s.ctx)));
        match self.peek() {
            T::Name => leaf(self, K::Identifier),
            T::Int => leaf(self, K::Integer),
            T::Float => leaf(self, K::Float),
            T::True => leaf(self, K::True),
            T::False => leaf(self, K::False),
            T::None => leaf(self, K::None),
            T::Ellipsis => leaf(self, K::Ellipsis),
            T::StrStart => self.string(s),
            T::LParen => self.paren(s),
            T::LBracket => self.list(s),
            T::LBrace => self.brace(s),
            _ => self.refuse(),
        }
    }

    /// `(`: a tuple, a parenthesized expression or yield, or a generator expression.
    fn paren(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let n = self.open_in(K::ParenthesizedExpression, s);
        self.bump();
        let set = |p: &mut Self, kind: K| p.w.set_key(n.at, key(kind, s.field, class(kind, s.ctx), false));
        if self.peek() == T::RParen {
            set(self, K::Tuple);
            self.bump();
            self.close(n);
            self.depth -= 1;
            return Ok(N { at: n.at, kind: K::Tuple });
        }
        if self.peek() == T::Yield {
            self.yield_expr(f::NONE)?;
            if self.peek() == T::Comma {
                return self.refuse();
            }
            self.expect(T::RParen)?;
            self.close(n);
            self.depth -= 1;
            return Ok(n);
        }
        let first = self.element(EXPR, Splat::Collection)?;
        let kind = if self.starts_for() {
            set(self, K::GeneratorExpression);
            self.rekey(first.at, slot(f::BODY, Ctx::Expr));
            self.comprehension()?;
            self.expect(T::RParen)?;
            K::GeneratorExpression
        } else if self.peek() == T::As {
            self.as_pattern(first, EXPR)?;
            self.expect(T::RParen)?;
            K::ParenthesizedExpression
        } else if self.peek() == T::RParen {
            if first.kind == K::ListSplat && self.kind_at(first.at) == K::ListSplat && unkey(self.w.key(first.at)).2 == C_NONE {
                return self.refuse();
            }
            self.bump();
            K::ParenthesizedExpression
        } else {
            set(self, K::Tuple);
            self.collection_rest(T::RParen, K::TRParen, Splat::Collection)?;
            K::Tuple
        };
        self.close(n);
        self.depth -= 1;
        Ok(N { at: n.at, kind })
    }

    /// A collection's elements after its first, and its closer (stored after a trailing comma).
    fn collection_rest(&mut self, closer: T, closing: K, mode: Splat) -> R<()> {
        while self.peek() == T::Comma {
            let comma = self.bump();
            if self.peek() == closer {
                self.leaf_of(comma, K::TComma);
                self.leaf(closing, f::NONE, C_NONE);
                return Ok(());
            }
            if self.peek() == T::Yield {
                self.yield_expr(f::NONE)?;
            } else {
                self.element(EXPR, mode)?;
            }
        }
        self.expect(closer)?;
        Ok(())
    }

    /// Whether the collection from the token next (after its `[` or `{`) is one splat of a name
    /// and its postfixes (`[*a.b]`, `{*f(x)}`), closed by `closer`: where tree-sitter-python reads
    /// the splat as the primary `list_splat` (the parse of a `[…]` as a `list_pattern`, and of a set
    /// as its alias, live to the closer). With more elements it is not.
    fn lone_splat(&self, closer: T) -> bool {
        let at = |k: usize| self.toks.get(k).map_or(T::Eof, |t| t.kind);
        let mut k = self.i;
        if at(k) != T::Star || at(k + 1) != T::Name {
            return false;
        }
        k += 2;
        loop {
            match at(k) {
                T::Dot if at(k + 1) == T::Name => k += 2,
                T::LBracket | T::LParen => {
                    let mut depth = 0u32;
                    loop {
                        match at(k) {
                            T::LParen | T::LBracket | T::LBrace => depth += 1,
                            T::RParen | T::RBracket | T::RBrace => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            T::Eof => return false,
                            _ => {}
                        }
                        k += 1;
                    }
                    k += 1;
                }
                _ => break,
            }
        }
        at(k) == closer
    }

    fn list(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let n = self.open_in(K::List, s);
        self.bump();
        let mut kind = K::List;
        if self.peek() == T::RBracket {
            self.bump();
        } else {
            let mode = if self.lone_splat(T::RBracket) { Splat::Alias } else { Splat::Collection };
            let first = if self.peek() == T::Yield { self.yield_expr(f::NONE)? } else { self.element(EXPR, mode)? };
            if self.starts_for() {
                kind = K::ListComprehension;
                self.w.set_key(n.at, key(kind, s.field, class(kind, s.ctx), false));
                self.rekey(first.at, slot(f::BODY, Ctx::Expr));
                self.comprehension()?;
                self.expect(T::RBracket)?;
            } else {
                self.collection_rest(T::RBracket, K::TRBracket, mode)?;
            }
        }
        self.close(n);
        self.depth -= 1;
        Ok(N { at: n.at, kind })
    }

    /// `{`: a dictionary, a set, or a comprehension of either.
    fn brace(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let n = self.open_in(K::Dictionary, s);
        self.bump();
        let set = |p: &mut Self, kind: K| p.w.set_key(n.at, key(kind, s.field, class(kind, s.ctx), false));
        let kind;
        if self.peek() == T::RBrace {
            self.bump();
            kind = K::Dictionary;
        } else if self.peek() == T::Pow {
            self.dict_rest()?;
            kind = K::Dictionary;
        } else {
            let mode = if self.lone_splat(T::RBrace) { Splat::Alias } else { Splat::Collection };
            let first = self.element(EXPR, mode)?;
            if self.peek() == T::Colon {
                let pair = self.wrap(first, K::Pair, slot(f::NONE, Ctx::Bare));
                self.rekey(first.at + 1, slot(f::KEY, Ctx::Expr));
                self.bump();
                self.expr(slot(f::VALUE, Ctx::Expr))?;
                self.close(pair);
                if self.starts_for() {
                    kind = K::DictionaryComprehension;
                    set(self, kind);
                    self.rekey(pair.at, slot(f::BODY, Ctx::Bare));
                    self.comprehension()?;
                    self.expect(T::RBrace)?;
                } else {
                    kind = K::Dictionary;
                    if self.peek() == T::Comma {
                        let comma = self.bump();
                        if self.peek() == T::RBrace {
                            self.leaf_of(comma, K::TComma);
                            self.leaf(K::TRBrace, f::NONE, C_NONE);
                        } else {
                            self.dict_rest()?;
                        }
                    } else {
                        self.expect(T::RBrace)?;
                    }
                }
            } else if self.starts_for() {
                kind = K::SetComprehension;
                set(self, kind);
                self.rekey(first.at, slot(f::BODY, Ctx::Expr));
                self.comprehension()?;
                self.expect(T::RBrace)?;
            } else {
                kind = K::Set;
                set(self, kind);
                self.collection_rest(T::RBrace, K::TRBrace, Splat::Collection)?;
            }
        }
        self.close(n);
        self.depth -= 1;
        Ok(N { at: n.at, kind })
    }

    /// A dictionary's items from the next (a pair or a `**` splat) to its `}`.
    fn dict_rest(&mut self) -> R<()> {
        loop {
            if self.peek() == T::Pow {
                let d = self.open(K::DictionarySplat, f::NONE, C_NONE);
                self.bump();
                self.expr(EXPR)?;
                self.close(d);
            } else {
                let pair = self.open(K::Pair, f::NONE, C_NONE);
                self.expr(slot(f::KEY, Ctx::Expr))?;
                self.expect(T::Colon)?;
                self.expr(slot(f::VALUE, Ctx::Expr))?;
                self.close(pair);
            }
            if self.peek() != T::Comma {
                break;
            }
            let comma = self.bump();
            if self.peek() == T::RBrace {
                self.leaf_of(comma, K::TComma);
                self.leaf(K::TRBrace, f::NONE, C_NONE);
                return Ok(());
            }
        }
        self.expect(T::RBrace)?;
        Ok(())
    }

    // ---- Strings.

    /// A string, or strings side by side (`concatenated_string`), in `s`.
    fn string(&mut self, s: Slot) -> R<N> {
        let first = self.one_string(s)?;
        if self.peek() != T::StrStart {
            return Ok(first);
        }
        let n = self.wrap(first, K::ConcatenatedString, s);
        self.rekey(first.at + 1, slot(f::NONE, Ctx::Bare));
        while self.peek() == T::StrStart {
            self.one_string(slot(f::NONE, Ctx::Bare))?;
        }
        self.close(n);
        Ok(n)
    }

    fn one_string(&mut self, s: Slot) -> R<N> {
        self.deeper()?;
        let n = self.open_in(K::String, s);
        self.leaf(K::StringStart, f::NONE, C_NONE);
        loop {
            match self.peek() {
                T::Text | T::Escape | T::EscInterp | T::Backslash => {
                    let c = self.open(K::StringContent, f::NONE, C_NONE);
                    loop {
                        match self.peek() {
                            T::Text => {
                                self.bump();
                            }
                            T::Escape => {
                                self.leaf(K::EscapeSequence, f::NONE, C_NONE);
                            }
                            T::EscInterp => {
                                self.leaf(K::EscapeInterpolation, f::NONE, C_NONE);
                            }
                            T::Backslash => {
                                self.leaf(K::TBackslash, f::NONE, C_NONE);
                            }
                            _ => break,
                        }
                    }
                    self.close(c);
                }
                T::IOpen => self.interpolation(K::Interpolation)?,
                T::StrEnd => {
                    self.leaf(K::StringEnd, f::NONE, C_NONE);
                    break;
                }
                _ => return self.refuse(),
            }
        }
        self.close(n);
        self.depth -= 1;
        Ok(n)
    }

    fn interpolation(&mut self, kind: K) -> R<()> {
        self.deeper()?;
        let n = self.open(kind, f::NONE, C_NONE);
        self.bump();
        let expression = slot(f::EXPRESSION, Ctx::Expr);
        if self.peek() == T::Yield {
            self.yield_expr(f::EXPRESSION)?;
        } else {
            self.exprs(expression)?;
        }
        let mut bare_eq = false;
        if self.peek() == T::Eq {
            self.leaf(K::TEq, f::NONE, C_NONE);
            bare_eq = true;
        }
        if self.peek() == T::Conversion {
            self.leaf(K::TypeConversion, f::TYPE_CONVERSION, C_NONE);
            bare_eq = false;
        }
        if self.peek() == T::Spec {
            bare_eq = false;
            let fs = self.open(K::FormatSpecifier, f::FORMAT_SPECIFIER, C_NONE);
            self.bump();
            while self.peek() == T::IOpen {
                self.interpolation(K::FormatExpression)?;
            }
            let end = self.tok().start;
            self.w.close(fs.at, end);
        }
        if self.peek() != T::IClose {
            return self.refuse();
        }
        if bare_eq {
            self.leaf(K::TRBrace, f::NONE, C_NONE);
        } else {
            self.bump();
        }
        self.close(n);
        self.depth -= 1;
        Ok(())
    }

    // ---- Types.

    /// `type`, with unions over its operands (left to right), in `field`.
    fn ty(&mut self, field: u8) -> R<N> {
        let mut t = self.type_operand(field)?;
        while self.peek() == T::Pipe {
            let u = self.wrap(t, K::UnionType, slot(f::NONE, Ctx::Bare));
            let outer = self.wrap(u, K::Type, slot(field, Ctx::Bare));
            self.w.set_key(outer.at + 2, key(K::Type, f::NONE, C_NONE, false));
            self.bump();
            self.type_operand(f::NONE)?;
            self.w.close(outer.at + 1, self.last_end);
            self.close(outer);
            t = outer;
        }
        Ok(t)
    }

    /// A `type` without a union at its top: a splat, a generic (and member types after it), or an
    /// expression.
    fn type_operand(&mut self, field: u8) -> R<N> {
        self.deeper()?;
        let n = self.open(K::Type, field, C_NONE);
        let generic = self.peek() == T::Name && self.peek_at(1) == T::LBracket;
        match self.peek() {
            T::Star | T::Pow => {
                let sp = self.open(K::SplatType, f::NONE, C_NONE);
                let op = if self.peek() == T::Star { K::TStar } else { K::TPow };
                self.leaf(op, f::NONE, C_NONE);
                self.name(f::NONE)?;
                self.close(sp);
            }
            _ if generic => {
                let g = self.open(K::GenericType, f::NONE, C_NONE);
                self.name(f::NONE)?;
                self.type_parameter(f::NONE)?;
                self.close(g);
                if self.peek() == T::LBracket {
                    return self.refuse();
                }
            }
            _ => {
                self.expr(EXPR)?;
            }
        }
        self.close(n);
        let mut out = n;
        while generic && self.peek() == T::Dot {
            let m = self.wrap(out, K::MemberType, slot(f::NONE, Ctx::Bare));
            let outer = self.wrap(m, K::Type, slot(field, Ctx::Bare));
            self.w.set_key(outer.at + 2, key(K::Type, f::NONE, C_NONE, false));
            self.bump();
            self.name(f::NONE)?;
            self.w.close(outer.at + 1, self.last_end);
            self.close(outer);
            out = outer;
        }
        self.depth -= 1;
        Ok(out)
    }

    /// `[` types `]`: a generic's arguments, a definition's type parameters (constrained, with
    /// defaults).
    fn type_parameter(&mut self, field: u8) -> R<()> {
        let n = self.open(K::TypeParameter, field, C_NONE);
        self.bump();
        while self.peek() != T::RBracket {
            let mut t = self.ty(f::NONE)?;
            if self.peek() == T::Colon {
                let c = self.wrap(t, K::ConstrainedType, slot(f::NONE, Ctx::Bare));
                let outer = self.wrap(c, K::Type, slot(f::NONE, Ctx::Bare));
                self.bump();
                self.ty(f::NONE)?;
                self.w.close(outer.at + 1, self.last_end);
                self.close(outer);
                t = outer;
            }
            if self.peek() == T::Eq {
                let d = self.wrap(t, K::TypeDefault, slot(f::NONE, Ctx::Bare));
                self.w.set_key(d.at + 1, key(K::Type, f::NAME, C_NONE, false));
                self.bump();
                self.ty(f::VALUE)?;
                self.close(d);
            }
            if self.peek() != T::Comma {
                break;
            }
            let comma = self.bump();
            if self.peek() == T::RBracket {
                self.leaf_of(comma, K::TComma);
                self.leaf(K::TRBracket, f::NONE, C_NONE);
                self.close(n);
                return Ok(());
            }
        }
        self.expect(T::RBracket)?;
        self.close(n);
        Ok(())
    }
}

/// Where a statement began: the writer's nodes, the tokens read, the depth.
#[derive(Clone, Copy)]
struct Mark {
    n: usize,
    i: usize,
    depth: u32,
}

/// A token's leaf in an `ERROR`: its kind as tree-sitter names the token.
fn token_kind(t: Tok) -> R<K> {
    use T::*;
    Ok(match t.kind {
        Name => match t.soft {
            soft::ASYNC => K::TAsync,
            soft::AWAIT => K::TAwait,
            soft::MATCH => K::TMatch,
            soft::CASE => K::TCase,
            soft::TYPE => K::TType,
            soft::PRINT => K::TPrint,
            soft::EXEC => K::TExec,
            _ => K::Identifier,
        },
        Int => K::Integer,
        Float => K::Float,
        False => K::False,
        None => K::None,
        True => K::True,
        Ellipsis => K::Ellipsis,
        And => K::TAnd,
        As => K::TAs,
        Assert => K::TAssert,
        Break => K::TBreak,
        Class => K::TClass,
        Continue => K::TContinue,
        Def => K::TDef,
        Del => K::TDel,
        Elif => K::TElif,
        Else => K::TElse,
        Except => K::TExcept,
        Finally => K::TFinally,
        For => K::TFor,
        From => K::TFrom,
        Global => K::TGlobal,
        If => K::TIf,
        Import => K::TImport,
        In => K::TIn,
        Is => K::TIs,
        Lambda => K::TLambda,
        Nonlocal => K::TNonlocal,
        Not => K::TNot,
        Or => K::TOr,
        Pass => K::TPass,
        Raise => K::TRaise,
        Return => K::TReturn,
        Try => K::TTry,
        While => K::TWhile,
        With => K::TWith,
        Yield => K::TYield,
        LParen => K::TLParen,
        RParen => K::TRParen,
        LBracket => K::TLBracket,
        RBracket => K::TRBracket,
        LBrace | IOpen => K::TLBrace,
        RBrace | IClose => K::TRBrace,
        Comma => K::TComma,
        Colon | Spec => K::TColon,
        Semi => K::TSemi,
        Dot => K::TDot,
        Eq => K::TEq,
        Walrus => K::TWalrus,
        Arrow => K::TArrow,
        At => K::TAt,
        Plus => K::TPlus,
        Minus => K::TMinus,
        Star => K::TStar,
        Pow => K::TPow,
        Slash => K::TSlash,
        DSlash => K::TDSlash,
        Percent => K::TPercent,
        Amp => K::TAmp,
        Pipe => K::TPipe,
        Caret => K::TCaret,
        Tilde => K::TTilde,
        LShift => K::TLShift,
        RShift => K::TRShift,
        Lt => K::TLt,
        Gt => K::TGt,
        Le => K::TLe,
        Ge => K::TGe,
        EqEq => K::TEqEq,
        Ne => K::TNe,
        LtGt => K::TLtGt,
        PlusEq => K::TPlusEq,
        MinusEq => K::TMinusEq,
        StarEq => K::TStarEq,
        SlashEq => K::TSlashEq,
        AtEq => K::TAtEq,
        DSlashEq => K::TDSlashEq,
        PercentEq => K::TPercentEq,
        PowEq => K::TPowEq,
        RShiftEq => K::TRShiftEq,
        LShiftEq => K::TLShiftEq,
        AmpEq => K::TAmpEq,
        CaretEq => K::TCaretEq,
        PipeEq => K::TPipeEq,
        StrStart => K::StringStart,
        StrEnd => K::StringEnd,
        Text => K::StringContent,
        Escape => K::EscapeSequence,
        EscInterp => K::EscapeInterpolation,
        Backslash => K::TBackslash,
        Eof | Newline | Indent | Dedent | Conversion => return Err(Refusal::Parse(t.start)),
    })
}

/// A key's kind as `K`.
fn kind_of(k: u8) -> K {
    debug_assert!(usize::from(k) < crate::KINDS.len());
    // SAFETY: `K` is `repr(u8)` with variants 0 to `KINDS.len() - 1`, and every key's kind is one.
    unsafe { std::mem::transmute::<u8, K>(k) }
}
