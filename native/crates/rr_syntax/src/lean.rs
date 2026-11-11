//! A lean `.rr` parser for the tape alone: the baseline the compiled grammar is measured against
//! (docs/parser-generators/30-a-lean-rust-baseline.md). It is not on the product's path.
//!
//! It reads the grammar `syntax::Parser` reads and gives the same tape (`crate::tape`), node for
//! node, with none of the rest:
//!
//! - **Tokens are a kind and a span.** No token holds a `String`; a word is compared in the text.
//!   Each token's line is counted as it is read, for the grammar's few line rules.
//! - **No statements.** Only the tree is built, and only as the tape: each node is written in
//!   preorder as it opens. The open nodes and their children are a stack.
//! - **A wrap** (an operator found after its left operand: `a - b`, `x <= y`) puts its node before
//!   its first operand: those are the tape's last few nodes.
//! - **Comments** are placed as the parse passes them, by `tape::flatten`'s rules: before a child,
//!   a comment that starts before it is its parent's; a leaf holds the comments inside it.
//! - **Backtracking** puts the tape, the stack and the comments read back to a mark.

use crate::tape::{ERROR, FIRST_ANONYMOUS, Field, Kind, Node};

// Token kinds; punctuation is `P0` plus its place in `syntax::PUNCT` (and in the anonymous kinds).
const ID: u8 = 0;
const STR: u8 = 1;
const NUM: u8 = 2;
const FLOAT: u8 = 3;
const TICK: u8 = 4;
const HASH: u8 = 5;
const BAD: u8 = 6;
const EOF: u8 = 7;
const P0: u8 = 16;

// Punctuation, by its place in `syntax::PUNCT`.
const TURNSTILE: u8 = 0;
const FAT_ARROW: u8 = 1;
const NE: u8 = 2;
const LE: u8 = 3;
const GE: u8 = 4;
const ARROW: u8 = 6;
const LPAREN: u8 = 7;
const RPAREN: u8 = 8;
const COMMA: u8 = 9;
const DOT: u8 = 10;
const SEMI: u8 = 11;
const COLON: u8 = 12;
const EQ: u8 = 13;
const LT: u8 = 14;
const GT: u8 = 15;
const BANG: u8 = 16;
const LBRACE: u8 = 17;
const RBRACE: u8 = 18;
const AT: u8 = 20;
const STAR: u8 = 21;
const MINUS: u8 = 24;
const PLUS: u8 = 25;
const SLASH: u8 = 26;
const PERCENT: u8 = 27;
const TILDE: u8 = 28;

/// The tokens: kind, span, the line of the first byte and of the last.
struct Tokens {
    kind: Vec<u8>,
    start: Vec<u32>,
    end: Vec<u32>,
    line: Vec<u32>,
    last_line: Vec<u32>,
    /// The line of the file's end (a token past the last).
    eof_line: u32,
    comments: Vec<(u32, u32)>,
}

fn newlines(b: &[u8]) -> u32 {
    b.iter().filter(|&&c| c == b'\n').count() as u32
}

/// The punctuation at `b[pos..]`, longest first, as `syntax::PUNCT` orders it.
fn punct(b: &[u8], pos: usize) -> Option<(u8, usize)> {
    let c = b[pos];
    let d = b.get(pos + 1).copied().unwrap_or(0);
    let two = match (c, d) {
        (b':', b'-') => Some(TURNSTILE),
        (b'=', b'>') => Some(FAT_ARROW),
        (b'!', b'=') => Some(NE),
        (b'<', b'=') => Some(LE),
        (b'>', b'=') => Some(GE),
        (b'<', b':') => Some(5),
        (b'-', b'>') => Some(ARROW),
        _ => None,
    };
    if let Some(p) = two {
        return Some((p, 2));
    }
    let one = match c {
        b'(' => 7,
        b')' => 8,
        b',' => 9,
        b'.' => 10,
        b';' => 11,
        b':' => 12,
        b'=' => 13,
        b'<' => 14,
        b'>' => 15,
        b'!' => 16,
        b'{' => 17,
        b'}' => 18,
        b'|' => 19,
        b'@' => 20,
        b'*' => 21,
        b'[' => 22,
        b']' => 23,
        b'-' => 24,
        b'+' => 25,
        b'/' => 26,
        b'%' => 27,
        b'~' => 28,
        b'^' => 29,
        b'&' => 30,
        _ => return None,
    };
    Some((one, 1))
}

/// `syntax::lex`'s tokens, without their text.
fn lex(src: &str) -> Tokens {
    let b = src.as_bytes();
    let n = b.len();
    let mut t = Tokens { kind: Vec::new(), start: Vec::new(), end: Vec::new(), line: Vec::new(), last_line: Vec::new(), eof_line: 0, comments: Vec::new() };
    let (mut pos, mut line) = (0usize, 1u32);
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let push = |t: &mut Tokens, kind: u8, start: usize, end: usize, line: u32, inner: u32| {
        t.kind.push(kind);
        t.start.push(start as u32);
        t.end.push(end as u32);
        t.line.push(line);
        t.last_line.push(line + inner);
    };
    loop {
        while pos < n && b[pos].is_ascii_whitespace() {
            line += u32::from(b[pos] == b'\n');
            pos += 1;
        }
        if pos >= n {
            break;
        }
        let start = pos;
        let c = b[pos];
        if c == b'/' && b.get(pos + 1) == Some(&b'/') {
            pos = b[pos..].iter().position(|&x| x == b'\n').map_or(n, |e| pos + e);
            t.comments.push((start as u32, pos as u32));
            continue;
        }
        if c == b'/' && b.get(pos + 1) == Some(&b'*') {
            // As `str::find("*/")` from the comment's first byte.
            match b[pos + 1..].windows(2).position(|w| w == b"*/") {
                Some(e) => {
                    let end = pos + 1 + e + 2;
                    line += newlines(&b[pos..end]);
                    pos = end;
                    t.comments.push((start as u32, pos as u32));
                    continue;
                }
                None => {
                    push(&mut t, BAD, start, n, line, newlines(&b[start..n.saturating_sub(1).max(start)]));
                    break;
                }
            }
        }
        if c == b'"' {
            let mut end = pos + 1;
            while end < n && b[end] != b'"' {
                end += if b[end] == b'\\' { 2 } else { 1 };
            }
            if end >= n {
                push(&mut t, BAD, start, n, line, newlines(&b[start..n - 1]));
                break;
            }
            let body = &src[pos + 1..end];
            // Only a `\u` escape can be refused.
            let ok = !body.contains("\\u") || crate::syntax::unescape(body).is_ok();
            let inner = newlines(&b[start..end]);
            push(&mut t, if ok { STR } else { BAD }, start, end + 1, line, inner);
            line += inner;
            pos = end + 1;
        } else if c == b'`' {
            let k = b[pos..].iter().take_while(|&&x| x == b'`').count();
            let fence = &src[pos..pos + k];
            let body_start = pos + k;
            match src[body_start..].find(fence) {
                Some(close) => {
                    let end = body_start + close + k;
                    let inner = newlines(&b[start..end - 1]);
                    push(&mut t, TICK, start, end, line, inner);
                    line += newlines(&b[start..end]);
                    pos = end;
                }
                None if k == 2 => {
                    push(&mut t, TICK, start, pos + 2, line, 0);
                    pos += 2;
                }
                None => {
                    push(&mut t, BAD, start, n, line, newlines(&b[start..n - 1]));
                    break;
                }
            }
        } else if c.is_ascii_digit() {
            let end = pos + b[pos..].iter().take_while(|x| x.is_ascii_digit()).count();
            if b.get(end) == Some(&b'.') && b.get(end + 1).is_some_and(u8::is_ascii_digit) {
                let end = end + 1 + b[end + 1..].iter().take_while(|x| x.is_ascii_digit()).count();
                push(&mut t, FLOAT, start, end, line, 0);
                pos = end;
                continue;
            }
            let ok = end - pos < 19 || src[pos..end].parse::<i64>().is_ok();
            push(&mut t, if ok { NUM } else { BAD }, start, end, line, 0);
            pos = end;
        } else if c == b'#' {
            let end = pos + 1 + b[pos + 1..].iter().take_while(|&&x| ident(x)).count();
            push(&mut t, HASH, start, end, line, 0);
            pos = end;
        } else if let Some((p, len)) = punct(b, pos) {
            push(&mut t, P0 + p, start, pos + len, line, 0);
            pos += len;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let end = pos + b[pos..].iter().take_while(|&&x| ident(x)).count();
            push(&mut t, ID, start, end, line, 0);
            pos = end;
        } else {
            let end = pos + src[pos..].chars().next().map_or(1, char::len_utf8);
            push(&mut t, BAD, start, end, line, 0);
            pos = end;
        }
    }
    t.eof_line = newlines(b) + 1;
    t
}

/// The anonymous kind of a word the grammar reserves, else `identifier`.
fn reserved(word: &str) -> u16 {
    (match word {
        "must" => Kind::Must,
        "may" => Kind::May,
        "code" => Kind::CodeWord,
        "in" => Kind::In,
        "tag" => Kind::Tag,
        "fix" => Kind::Fix,
        "safe" => Kind::Safe,
        "suppress" => Kind::Suppress,
        "over" => Kind::Over,
        "options" => Kind::Options,
        "by" => Kind::By,
        "sep" => Kind::Sep,
        "count" => Kind::Count,
        "min" => Kind::Min,
        "max" => Kind::Max,
        "sum" => Kind::Sum,
        "mean" => Kind::Mean,
        "join" => Kind::Join,
        _ => Kind::Identifier,
    }) as u16
}

/// What a literal read was (`syntax::Lit`'s variant), for `!`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lit {
    Atom,
    Cmp,
    Other,
}

/// Whether a term read is a call that is not a functor's (`syntax::Term::Call { functor: false }`).
type Call = bool;

type R<T> = Result<T, ()>;

/// A place to go back to: the token, the tape, the stack and the comments placed.
#[derive(Clone, Copy)]
struct Mark {
    i: usize,
    out: usize,
    kids: usize,
    open: usize,
    comment: usize,
}

struct Parser<'s> {
    src: &'s str,
    t: Tokens,
    i: usize,
    no_named: bool,
    /// The tape so far, in preorder.
    out: Vec<Node>,
    /// The open nodes: each its place in `out` and where its children start in `kids`.
    open: Vec<(u32, u32)>,
    /// The open nodes' children, by place in `out`.
    kids: Vec<u32>,
    /// The next comment to place.
    comment: usize,
}

/// The lexer alone (its tokens' count), to measure apart from the parse.
pub fn token_count(src: &str) -> usize {
    lex(src).kind.len()
}

/// `src`'s tape: `crate::tape(src)`, node for node.
pub fn tape(src: &str) -> Vec<Node> {
    let t = lex(src);
    let mut out = Vec::with_capacity(t.kind.len() + t.comments.len() + 1);
    out.push(Node { kind: Kind::SourceFile as u16, field: 0, start: 0, end: src.len() as u32, parent: None, extra: false });
    let mut p = Parser { src, t, i: 0, no_named: false, out, open: vec![(0, 0)], kids: Vec::new(), comment: 0 };
    while p.kind() != EOF {
        let save = p.mark();
        if p.statement().is_err() {
            p.recover(save);
        }
    }
    // Comments after the last statement are the file's.
    p.comments_before(0, u32::MAX);
    p.out
}

impl Parser<'_> {
    // Tokens.

    fn kind_at(&self, k: usize) -> u8 {
        self.t.kind.get(self.i + k).copied().unwrap_or(EOF)
    }

    fn kind(&self) -> u8 {
        self.kind_at(0)
    }

    fn is_p(&self, p: u8) -> bool {
        self.kind() == P0 + p
    }

    fn is_p_at(&self, k: usize, p: u8) -> bool {
        self.kind_at(k) == P0 + p
    }

    fn text(&self, j: usize) -> &str {
        &self.src[self.t.start[j] as usize..self.t.end[j] as usize]
    }

    fn is_id(&self, word: &str) -> bool {
        self.kind() == ID && self.text(self.i) == word
    }

    fn is_id_at(&self, k: usize) -> bool {
        self.kind_at(k) == ID
    }

    fn start(&self) -> u32 {
        self.t.start.get(self.i).copied().unwrap_or(self.src.len() as u32)
    }

    fn line(&self) -> u32 {
        self.t.line.get(self.i).copied().unwrap_or(self.t.eof_line)
    }

    fn last_line(&self) -> u32 {
        self.i.checked_sub(1).and_then(|j| self.t.last_line.get(j)).copied().unwrap_or(0)
    }

    // The tape.

    fn comments_before(&mut self, parent: u32, before: u32) {
        while let Some(&(start, end)) = self.t.comments.get(self.comment) {
            if start >= before {
                break;
            }
            self.comment += 1;
            self.out.push(Node { kind: Kind::Comment as u16, field: 0, start, end, parent: Some(parent), extra: true });
        }
    }

    fn skip_comments(&mut self, before: u32) {
        while self.t.comments.get(self.comment).is_some_and(|c| c.0 < before) {
            self.comment += 1;
        }
    }

    /// A child of the open node; it stays open when `open`.
    fn push(&mut self, kind: u16, start: u32, end: u32, open: bool) {
        let parent = self.open.last().expect("source_file stays open").0;
        self.comments_before(parent, start);
        let id = self.out.len() as u32;
        self.out.push(Node { kind, field: 0, start, end, parent: Some(parent), extra: false });
        self.kids.push(id);
        if open {
            self.open.push((id, self.kids.len() as u32));
        } else {
            // A leaf's text is its own: a comment inside it is not a node.
            self.skip_comments(end);
        }
    }

    fn open(&mut self, kind: Kind) {
        let at = self.start();
        self.push(kind as u16, at, at, true);
    }

    fn close(&mut self) {
        let end = self.i.checked_sub(1).and_then(|j| self.t.end.get(j)).copied().unwrap_or(0);
        let (id, base) = self.open.pop().expect("an open node");
        let node = &mut self.out[id as usize];
        node.end = end.max(node.start);
        let end = node.end;
        if self.kids.len() > base as usize {
            self.comments_before(id, end);
        } else {
            self.skip_comments(end);
        }
        self.kids.truncate(base as usize);
    }

    fn here(&self) -> usize {
        self.kids.len() - self.open.last().expect("open").1 as usize
    }

    fn last(&self) -> Option<u32> {
        (self.here() > 0).then(|| *self.kids.last().expect("a child"))
    }

    fn field(&mut self, field: Field) {
        if let Some(last) = self.last() {
            self.out[last as usize].field = field as u16;
        }
    }

    /// A node of `kind` around the open node's children from `from` on, open, before the first.
    fn wrap(&mut self, from: usize, kind: Kind) {
        let (x, base) = *self.open.last().expect("open");
        let at = base as usize + from;
        let (k, start, field) = match self.kids.get(at) {
            Some(&k) => {
                let first = &mut self.out[k as usize];
                let (start, field) = (first.start, first.field);
                first.field = 0;
                (k, start, field)
            }
            None => (self.out.len() as u32, 0, 0),
        };
        // The nodes from the first moved on shift by one; the open node's children there are
        // the new node's.
        for node in &mut self.out[k as usize..] {
            if let Some(p) = node.parent.as_mut() {
                if *p >= k {
                    *p += 1;
                } else if *p == x {
                    *p = k;
                }
            }
        }
        self.out.insert(k as usize, Node { kind: kind as u16, field, start, end: start, parent: Some(x), extra: false });
        for c in &mut self.kids[at..] {
            *c += 1;
        }
        self.kids.insert(at, k);
        self.open.push((k, at as u32 + 1));
    }

    fn mark(&self) -> Mark {
        Mark { i: self.i, out: self.out.len(), kids: self.kids.len(), open: self.open.len(), comment: self.comment }
    }

    fn reset(&mut self, m: Mark) {
        self.i = m.i;
        self.open.truncate(m.open);
        self.kids.truncate(m.kids);
        self.out.truncate(m.out);
        self.comment = m.comment;
    }

    fn leaf_over(&mut self, kind: u16, from: usize, to: usize) {
        if from < to && to <= self.t.kind.len() {
            self.push(kind, self.t.start[from], self.t.end[to - 1], false);
        }
    }

    fn bump_as(&mut self, kind: Kind) {
        self.leaf_over(kind as u16, self.i, self.i + 1);
        self.i += 1;
    }

    fn anon(&mut self) {
        let kind = match self.kind() {
            ID => reserved(self.text(self.i)),
            HASH if self.text(self.i) == "#include" => Kind::HashInclude as u16,
            k if k >= P0 => FIRST_ANONYMOUS + u16::from(k - P0),
            _ => Kind::Identifier as u16,
        };
        self.leaf_over(kind, self.i, self.i + 1);
        self.i += 1;
    }

    fn anon_over(&mut self, p: u8, at: usize) {
        self.leaf_over(FIRST_ANONYMOUS + u16::from(p), at, at + 1);
    }

    fn word(&mut self, field: Field) -> R<()> {
        let kind = match self.kind() {
            ID => Kind::Identifier,
            NUM => Kind::Number,
            FLOAT => Kind::Float,
            STR => Kind::String,
            TICK => Kind::CodeLiteral,
            BAD => return Err(()),
            _ => {
                self.anon();
                return Ok(());
            }
        };
        self.bump_as(kind);
        self.field(field);
        Ok(())
    }

    fn p(&mut self, p: u8) -> R<()> {
        if self.is_p(p) {
            self.anon();
            Ok(())
        } else {
            Err(())
        }
    }

    fn id(&mut self) -> R<()> {
        if self.kind() == ID {
            self.bump_as(Kind::Identifier);
            Ok(())
        } else {
            Err(())
        }
    }

    fn string(&mut self) -> R<()> {
        if self.kind() == STR {
            self.bump_as(Kind::String);
            Ok(())
        } else {
            Err(())
        }
    }

    fn list(&mut self, close: u8, field: Field, item: fn(&mut Self) -> R<()>) -> R<()> {
        if self.is_p(close) {
            return Ok(());
        }
        item(self)?;
        self.field(field);
        while self.is_p(COMMA) {
            self.anon();
            item(self)?;
            self.field(field);
        }
        Ok(())
    }

    fn braces(&mut self) -> R<()> {
        if !self.is_p(LBRACE) {
            return Err(());
        }
        self.i += 1;
        let mut depth = 1;
        while depth > 0 {
            match self.kind() {
                k if k == P0 + LBRACE => depth += 1,
                k if k == P0 + RBRACE => depth -= 1,
                EOF | BAD => return Err(()),
                _ => {}
            }
            self.i += 1;
        }
        Ok(())
    }

    fn json(&mut self, field: Field) -> R<()> {
        let from = self.i;
        self.braces()?;
        self.leaf_over(Kind::JsonObject as u16, from, self.i);
        self.field(field);
        Ok(())
    }

    fn brace_column(&mut self, field: Field) -> R<()> {
        let from = self.i;
        self.braces()?;
        if self.i - from == 3 && self.t.kind[from + 1] == ID {
            self.anon_over(LBRACE, from);
            self.leaf_over(Kind::Identifier as u16, from + 1, from + 2);
            self.field(field);
            self.anon_over(RBRACE, from + 2);
        } else {
            self.leaf_over(Kind::RawText as u16, from, self.i);
            self.field(field);
        }
        Ok(())
    }

    fn ty(&mut self) -> R<()> {
        self.open(Kind::Type);
        self.id()?;
        self.field(Field::Name);
        if self.is_p(LT) {
            self.anon();
            if !self.is_p(GT) {
                self.id()?;
                self.field(Field::Argument);
            }
            self.p(GT)?;
        }
        self.close();
        Ok(())
    }

    fn column(&mut self) -> R<()> {
        self.open(Kind::Column);
        self.id()?;
        self.field(Field::Name);
        self.p(COLON)?;
        self.ty()?;
        self.field(Field::Type);
        self.close();
        Ok(())
    }

    fn columns(&mut self) -> R<()> {
        self.open(Kind::ColumnList);
        self.p(LPAREN)?;
        self.list(RPAREN, Field::Column, Self::column)?;
        self.p(RPAREN)?;
        self.close();
        self.field(Field::Columns);
        Ok(())
    }

    fn arguments(&mut self) -> R<()> {
        self.open(Kind::ArgumentList);
        self.p(LPAREN)?;
        self.list(RPAREN, Field::Argument, |p| p.term().map(|_| ()))?;
        self.p(RPAREN)?;
        self.close();
        self.field(Field::Arguments);
        Ok(())
    }

    fn rest_of_line(&mut self, line: u32, field: Field) -> R<()> {
        while self.kind() != EOF && self.line() == line {
            self.word(field)?;
        }
        Ok(())
    }

    fn raw_line(&mut self, line: u32, field: Field) -> R<()> {
        let from = self.i;
        while self.kind() != EOF && self.line() == line {
            if self.kind() == BAD {
                return Err(());
            }
            self.i += 1;
        }
        self.leaf_over(Kind::RawText as u16, from, self.i);
        self.field(field);
        Ok(())
    }

    /// `syntax::Parser::recover`: one ERROR leaf from the statement's first token to where
    /// parsing resumes.
    fn recover(&mut self, save: Mark) {
        let first = save.i;
        self.reset(save);
        let b = self.src.as_bytes();
        let line_start = |at: usize| b[..at].iter().rev().take_while(|&&c| c != b'\n').all(|c| c.is_ascii_whitespace());
        let n = self.t.kind.len();
        let mut k = first + 1;
        while k < n {
            match self.t.kind[k] {
                HASH => break,
                x if x == P0 + DOT && self.t.kind.get(k + 1) == Some(&ID) && line_start(self.t.start[k] as usize) => break,
                x if x == P0 + DOT => {
                    k += 1;
                    break;
                }
                _ => k += 1,
            }
        }
        let k = k.min(n);
        self.leaf_over(ERROR, first, k);
        self.i = k;
    }

    fn statement(&mut self) -> R<()> {
        let line = self.line();
        if self.kind() == HASH {
            let include = self.text(self.i) == "#include";
            self.open(Kind::Include);
            self.anon();
            if !include {
                return Err(());
            }
            self.string()?;
            self.field(Field::Path);
            self.close();
            Ok(())
        } else if self.is_p(DOT) && self.is_id_at(1) {
            let d = self.text(self.i + 1);
            let policy = d == "policy" && self.is_p_at(2, LBRACE);
            let (kind, keyword) = match d {
                "decl" => (Kind::Declaration, Kind::DotDecl),
                "edit" => (Kind::EditDeclaration, Kind::DotEdit),
                "functor" => (Kind::FunctorDeclaration, Kind::DotFunctor),
                "type" => (Kind::TypeDeclaration, Kind::DotType),
                "input" => (Kind::IoDirective, Kind::DotInput),
                "output" => (Kind::IoDirective, Kind::DotOutput),
                "lattice" => (Kind::LatticeDeclaration, Kind::DotLattice),
                "pragma" => (Kind::Pragma, Kind::DotPragma),
                "violation" => (Kind::ViolationDeclaration, Kind::DotViolation),
                "policy" if policy => (Kind::PolicyDocument, Kind::DotPolicy),
                _ => (Kind::Setting, Kind::Identifier),
            };
            let word = self.i + 1;
            self.open(kind);
            if kind == Kind::Setting {
                self.anon();
                self.bump_as(Kind::Identifier);
                self.field(Field::Name);
            } else {
                self.leaf_over(keyword as u16, self.i, self.i + 2);
                self.i += 2;
            }
            self.directive(line, word, kind)?;
            self.close();
            Ok(())
        } else {
            self.clause()
        }
    }

    fn io_relation(&mut self) -> R<()> {
        self.open(Kind::IoRelation);
        self.id()?;
        self.field(Field::Name);
        if self.is_p(LPAREN) {
            self.anon();
            let from = self.i;
            let mut depth = 1;
            while depth > 0 {
                match self.kind() {
                    k if k == P0 + LPAREN => depth += 1,
                    k if k == P0 + RPAREN => depth -= 1,
                    EOF | BAD => return Err(()),
                    _ => {}
                }
                self.i += 1;
            }
            let close = self.i - 1;
            self.leaf_over(Kind::RawText as u16, from, close);
            self.field(Field::Parameters);
            self.anon_over(RPAREN, close);
        }
        self.close();
        Ok(())
    }

    fn lattice_operation(&mut self) -> R<()> {
        self.open(Kind::LatticeOperation);
        self.id()?;
        self.field(Field::Name);
        self.p(ARROW)?;
        if self.is_p(AT) && self.is_id_at(1) && !self.is_p_at(2, LPAREN) {
            self.open(Kind::FunctorCall);
            self.anon();
            self.id()?;
            self.field(Field::Function);
            self.close();
            self.field(Field::Value);
            self.close();
            return Ok(());
        }
        self.term()?;
        self.field(Field::Value);
        self.close();
        Ok(())
    }

    fn directive(&mut self, line: u32, word: usize, kind: Kind) -> R<()> {
        let d = self.text(word);
        match d {
            "decl" => {
                self.id()?;
                self.field(Field::Name);
                self.columns()?;
                let at = self.last_line();
                self.rest_of_line(at, Field::Modifier)?;
            }
            "edit" => {
                self.id()?;
                self.field(Field::Name);
                self.columns()?;
            }
            "functor" => {
                self.id()?;
                self.field(Field::Name);
                self.columns()?;
                self.p(COLON)?;
                self.ty()?;
                self.field(Field::ReturnType);
                let at = self.last_line();
                self.rest_of_line(at, Field::Modifier)?;
            }
            "type" => {
                self.id()?;
                self.field(Field::Name);
                let at = self.last_line();
                self.raw_line(at, Field::Definition)?;
            }
            "input" | "output" => self.list(DOT, Field::Relation, Self::io_relation)?,
            "lattice" => {
                self.id()?;
                self.field(Field::Name);
                self.p(LT)?;
                if !self.is_p(GT) {
                    self.id()?;
                    self.field(Field::Base);
                }
                self.p(GT)?;
                self.p(LBRACE)?;
                self.list(RBRACE, Field::Operation, Self::lattice_operation)?;
                self.p(RBRACE)?;
            }
            "pragma" => {
                while self.kind() == STR {
                    self.string()?;
                    self.field(Field::Argument);
                }
            }
            "violation" => self.violation()?,
            "policy" if kind == Kind::PolicyDocument => {
                let from = self.i;
                self.json(Field::Document)?;
                // The document must be JSON.
                let text = &self.src[self.t.start[from] as usize..self.t.end[self.i - 1] as usize];
                serde_json::from_str::<serde_json::Value>(text).map_err(|_| ())?;
            }
            _ => {
                let mut line = line;
                while self.kind() != EOF && (self.line() == line || (self.i > 0 && self.t.kind[self.i - 1] == P0 + COMMA)) {
                    line = self.line();
                    if self.is_p(LBRACE) {
                        self.json(Field::Argument)?;
                    } else {
                        self.word(Field::Argument)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn violation(&mut self) -> R<()> {
        self.id()?;
        self.field(Field::Name);
        self.columns()?;
        loop {
            if self.is_id("code") && self.kind_at(1) == STR {
                self.anon();
                self.string()?;
                self.field(Field::Code);
            } else if (self.is_id("code") || self.is_id("tag")) && self.is_p_at(1, LBRACE) {
                let is_code = self.is_id("code");
                self.open(if is_code { Kind::CodeColumn } else { Kind::TagColumn });
                self.anon();
                self.brace_column(Field::Column)?;
                if is_code && self.is_id("in") && self.kind_at(1) == STR {
                    self.anon();
                    self.string()?;
                    self.field(Field::Codes);
                }
                self.close();
                self.field(if is_code { Field::Code } else { Field::Tag });
            } else {
                break;
            }
        }
        self.string()?;
        self.field(Field::Message);
        loop {
            if self.is_id("fix") && self.is_id_at(1) {
                self.open(Kind::FixLine);
                self.anon();
                if self.is_id("safe") && self.is_id_at(1) {
                    self.anon();
                }
                self.id()?;
                self.field(Field::Relation);
                self.string()?;
                self.field(Field::Description);
                self.close();
                self.field(Field::Fix);
            } else if self.is_id("suppress") && self.is_id_at(1) && self.kind_at(2) == STR {
                self.open(Kind::SuppressLine);
                self.anon();
                self.id()?;
                self.field(Field::Tool);
                self.string()?;
                self.field(Field::Rule);
                while self.is_p(COMMA) && self.kind_at(1) == STR {
                    self.anon();
                    self.string()?;
                    self.field(Field::Rule);
                }
                if self.is_id("over") && self.is_p_at(1, LBRACE) {
                    self.anon();
                    self.brace_column(Field::Over)?;
                }
                self.close();
                self.field(Field::Suppress);
            } else if self.is_id("options") && self.is_p_at(1, LBRACE) {
                self.anon();
                self.json(Field::Options)?;
            } else {
                break;
            }
        }
        Ok(())
    }

    fn head(&mut self) -> R<()> {
        self.open(Kind::Head);
        if (self.is_id("must") || self.is_id("may")) && self.is_id_at(1) {
            self.anon();
            self.field(Field::Modal);
        }
        self.id()?;
        self.field(Field::Relation);
        self.arguments()?;
        self.close();
        Ok(())
    }

    fn clause(&mut self) -> R<()> {
        self.open(Kind::Clause);
        self.head()?;
        self.field(Field::Head);
        while self.is_p(COMMA) {
            self.anon();
            self.head()?;
            self.field(Field::Head);
        }
        if self.is_p(TURNSTILE) {
            self.anon();
            self.disjunction()?;
            self.field(Field::Body);
        }
        if self.is_p(FAT_ARROW) {
            self.anon();
            self.list(DOT, Field::Edit, |p| p.term().map(|_| ()))?;
        }
        self.p(DOT)?;
        self.close();
        Ok(())
    }

    fn disjunction(&mut self) -> R<()> {
        let from = self.here();
        self.conjunction()?;
        if self.is_p(SEMI) {
            self.wrap(from, Kind::Disjunction);
            self.field(Field::Alternative);
            while self.is_p(SEMI) {
                self.anon();
                self.conjunction()?;
                self.field(Field::Alternative);
            }
            self.close();
        }
        Ok(())
    }

    fn conjunction(&mut self) -> R<()> {
        self.open(Kind::Conjunction);
        self.literal()?;
        self.field(Field::Literal);
        while self.is_p(COMMA) {
            self.anon();
            self.literal()?;
            self.field(Field::Literal);
        }
        self.close();
        Ok(())
    }

    fn literal(&mut self) -> R<Lit> {
        if self.is_p(BANG) {
            self.open(Kind::Negation);
            self.anon();
            let inner = self.literal()?;
            self.field(Field::Operand);
            self.close();
            return if inner == Lit::Cmp { Err(()) } else { Ok(inner) };
        }
        if self.is_p(LPAREN) {
            let save = self.mark();
            self.open(Kind::Group);
            self.anon();
            let group = self.disjunction().and_then(|()| {
                self.field(Field::Body);
                self.p(RPAREN)
            });
            let k = self.kind();
            let term_follows = k >= P0 && matches!(k - P0, EQ | NE | LT | LE | GT | GE | PLUS | MINUS | STAR | SLASH | PERCENT);
            if group.is_ok() && !term_follows {
                self.close();
                return Ok(Lit::Other);
            }
            self.reset(save);
        }
        let from = self.here();
        let call = self.term()?;
        if self.is_p(TILDE) {
            self.wrap(from, Kind::PatternMatch);
            self.field(Field::Subject);
            self.anon();
            if self.kind() != TICK {
                return Err(());
            }
            self.bump_as(Kind::CodeLiteral);
            self.field(Field::Pattern);
            self.close();
            return Ok(Lit::Other);
        }
        let k = self.kind();
        if !(k >= P0 && matches!(k - P0, EQ | NE | LT | LE | GT | GE)) {
            if !call {
                return Err(());
            }
            self.retag_atom();
            return Ok(Lit::Atom);
        }
        self.wrap(from, Kind::Comparison);
        self.field(Field::Left);
        self.anon();
        self.field(Field::Operator);
        self.term()?;
        self.field(Field::Right);
        self.close();
        Ok(Lit::Cmp)
    }

    /// The call just read is an atom: its function is its relation.
    fn retag_atom(&mut self) {
        let Some(call) = self.last() else { return };
        self.out[call as usize].kind = Kind::Atom as u16;
        // Its children: the nodes after it whose parent it is.
        for node in &mut self.out[call as usize + 1..] {
            if node.parent == Some(call) && !node.extra && node.field == Field::Function as u16 {
                node.field = Field::Relation as u16;
            }
        }
    }

    fn term(&mut self) -> R<Call> {
        let from = self.here();
        let mut call = self.product()?;
        while self.is_p(PLUS) || self.is_p(MINUS) {
            self.wrap(from, Kind::BinaryExpression);
            self.field(Field::Left);
            self.anon();
            self.field(Field::Operator);
            self.product()?;
            self.field(Field::Right);
            self.close();
            call = false;
        }
        Ok(call)
    }

    fn product(&mut self) -> R<Call> {
        let from = self.here();
        let mut call = self.primary()?;
        while self.is_p(STAR) || self.is_p(SLASH) || self.is_p(PERCENT) {
            self.wrap(from, Kind::BinaryExpression);
            self.field(Field::Left);
            self.anon();
            self.field(Field::Operator);
            self.primary()?;
            self.field(Field::Right);
            self.close();
            call = false;
        }
        Ok(call)
    }

    fn aggregate(&mut self) -> R<Call> {
        self.open(Kind::Aggregate);
        self.anon();
        self.field(Field::Operator);
        if !self.is_p(COLON) {
            self.agg_part()?;
            self.field(Field::Value);
        }
        if self.is_id("by") {
            self.anon();
            if self.is_p(LPAREN) {
                self.anon();
                self.list(RPAREN, Field::Key, |p| p.term().map(|_| ()))?;
                self.p(RPAREN)?;
            } else {
                self.agg_part()?;
                self.field(Field::Key);
            }
        }
        if self.is_id("sep") {
            self.anon();
            self.string()?;
            self.field(Field::Separator);
        }
        self.p(COLON)?;
        self.p(LBRACE)?;
        self.disjunction()?;
        self.field(Field::Body);
        self.p(RBRACE)?;
        self.close();
        Ok(false)
    }

    fn agg_part(&mut self) -> R<Call> {
        let named = std::mem::replace(&mut self.no_named, true);
        let t = self.term();
        self.no_named = named;
        t
    }

    fn call_or_aggregate(&mut self) -> R<Call> {
        let save = self.mark();
        self.open(Kind::Call);
        self.bump_as(Kind::Identifier);
        self.field(Field::Function);
        self.arguments()?;
        self.close();
        if self.is_p(COLON) {
            self.reset(save);
            return self.aggregate();
        }
        Ok(true)
    }

    fn primary(&mut self) -> R<Call> {
        let kind = match self.kind() {
            NUM => Kind::Number,
            FLOAT => Kind::Float,
            STR => Kind::String,
            TICK => Kind::CodeLiteral,
            k if k == P0 + MINUS && matches!(self.kind_at(1), NUM | FLOAT) => {
                let from = self.i;
                let kind = if self.kind_at(1) == NUM { Kind::Number } else { Kind::Float };
                self.i += 2;
                self.leaf_over(kind as u16, from, from + 2);
                return Ok(false);
            }
            k if k == P0 + MINUS => {
                self.open(Kind::UnaryExpression);
                self.anon();
                self.field(Field::Operator);
                self.primary()?;
                self.field(Field::Operand);
                self.close();
                return Ok(false);
            }
            k if k == P0 + LPAREN => {
                // `(f(x))` is still a call: as a literal, an atom.
                self.open(Kind::ParenthesizedExpression);
                self.anon();
                let call = self.term()?;
                self.field(Field::Expression);
                self.p(RPAREN)?;
                self.close();
                return Ok(call);
            }
            k if k == P0 + LBRACE => {
                self.json(Field::None)?;
                return Ok(false);
            }
            k if k == P0 + AT => {
                self.open(Kind::FunctorCall);
                self.anon();
                self.id()?;
                self.field(Field::Function);
                self.arguments()?;
                self.close();
                return Ok(false);
            }
            ID => {
                let s = self.text(self.i);
                let aggregate_word = matches!(s, "min" | "max" | "sum" | "product" | "mean" | "join");
                if s == "_" {
                    Kind::Wildcard
                } else if s == "count" && self.is_p_at(1, COLON) {
                    return self.aggregate();
                } else if aggregate_word && self.is_p_at(1, LPAREN) {
                    return self.call_or_aggregate();
                } else if aggregate_word
                    && !(self.kind_at(1) >= P0 && matches!(self.kind_at(1) - P0, COMMA | RPAREN | EQ | NE | LT | GT | LE | GE | PLUS | MINUS | STAR | SLASH | PERCENT | DOT | SEMI | RBRACE))
                {
                    let save = self.mark();
                    self.i += 1;
                    let aggregate = self.agg_part().is_ok() && (self.is_p(COLON) || self.is_id("by") || self.is_id("sep"));
                    self.reset(save);
                    if aggregate {
                        return self.aggregate();
                    }
                    Kind::Variable
                } else if self.is_p_at(1, LPAREN) {
                    self.open(Kind::Call);
                    self.bump_as(Kind::Identifier);
                    self.field(Field::Function);
                    self.arguments()?;
                    self.close();
                    return Ok(true);
                } else if self.is_p_at(1, COLON) && !self.no_named {
                    self.open(Kind::NamedArgument);
                    self.bump_as(Kind::Identifier);
                    self.field(Field::Name);
                    self.anon();
                    self.term()?;
                    self.field(Field::Value);
                    self.close();
                    return Ok(false);
                } else {
                    Kind::Variable
                }
            }
            _ => return Err(()),
        };
        self.bump_as(kind);
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    /// Every `.rr` file of the repository, and every prefix of each at a few cut points (to run
    /// the recovery paths), gives `crate::tape`'s tape.
    #[test]
    fn lean_tape_is_the_engines() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let (mut files, mut work) = (Vec::new(), vec![root]);
        while let Some(dir) = work.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref()));
                if p.is_dir() && !skip {
                    work.push(p);
                } else if p.extension().is_some_and(|x| x == "rr") {
                    files.extend(std::fs::read_to_string(&p).ok().map(|t| (p.display().to_string(), t)));
                }
            }
        }
        assert!(files.len() > 1000, "{} files", files.len());
        let mut checked = 0;
        for (path, text) in &files {
            let mut cuts = vec![text.len()];
            for k in 1..8 {
                let mut at = text.len() * k / 8;
                while !text.is_char_boundary(at) {
                    at -= 1;
                }
                cuts.push(at);
            }
            for at in cuts {
                let src = &text[..at];
                assert_eq!(super::tape(src), crate::tape(src), "{path} cut at {at}");
                checked += 1;
            }
        }
        eprintln!("{} files, {checked} texts", files.len());
    }
}
