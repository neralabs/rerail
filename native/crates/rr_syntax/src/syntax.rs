//! The `.rr` text as written: tokens, and statements before lowering (`lower`) or checking (`check`).
//!
//! Soufflé's grammar, with the additions `docs/rule-syntax/lib/core.rr` lists: patterns (``n ~ `…` ``), named regex
//! groups in `match`, `must`/`may` heads, `.violation` with `fix` lines, `=> edits`, `join … by … sep`, and
//! `ordered by` on a declaration. The policy directives (`.policy`, `.mode`, `.scope`, …) are settings: each is kept
//! as its text, up to the end of its line. Two JSON forms stay for what has no Datalog form yet: the matcher term
//! `ast(n, { … })` and the policy document `.policy { … }`.

use crate::Error;
use crate::tape::{self, Field, Kind, Tree};
use std::cell::Cell;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Id(String),
    Str(String),
    Num(i64),
    /// A decimal number, as written (`0.0`).
    Float(String),
    /// A backtick literal's content: a code pattern or a fix template.
    Tick(String),
    /// `#include` and other `#` lines.
    Hash(String),
    P(&'static str),
    /// What the lexer could not read (`lex`): only a taped parse sees one.
    Bad,
    Eof,
}

const PUNCT: [&str; 31] = [
    ":-", "=>", "!=", "<=", ">=", "<:", "->", "(", ")", ",", ".", ";", ":", "=", "<", ">", "!", "{", "}", "|", "@", "*", "[", "]", "-", "+",
    "/", "%", "~", "^", "&",
];

/// A token, and the byte range it spans.
pub type Spanned = (Tok, usize, usize);

/// A string literal's escapes: `\"`, `\\`, `\n`, `\t`, `\r`, `\u{…}` and JSON's `\uXXXX`. Any other backslash is
/// kept with the character after it, so a regex reads as written (`"\d+\.py"`).
pub(crate) fn unescape(body: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('u') => {
                let hex: String = if chars.peek() == Some(&'{') {
                    chars.next();
                    let h: String = chars.by_ref().take_while(|&c| c != '}').collect();
                    h
                } else {
                    chars.by_ref().take(4).collect()
                };
                let code = u32::from_str_radix(&hex, 16).map_err(|_| format!("bad \\u escape {hex:?}"))?;
                // A JSON surrogate pair.
                if (0xD800..0xDC00).contains(&code) && chars.peek() == Some(&'\\') {
                    let rest: String = chars.clone().take(6).collect();
                    if let Some(low) = rest.strip_prefix("\\u").and_then(|h| u32::from_str_radix(h, 16).ok()) {
                        chars.by_ref().take(6).for_each(drop);
                        let joined = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                        out.push(char::from_u32(joined).ok_or("bad surrogate pair")?);
                        continue;
                    }
                }
                out.push(char::from_u32(code).ok_or_else(|| format!("bad code point {hex:?}"))?);
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    Ok(out)
}

/// A string as an `.rr` literal, read back by `unescape` as the same string.
pub fn quote(s: &str) -> String {
    // Most strings need no escape: copied whole.
    if !s.chars().any(|c| matches!(c, '"' | '\\') || c.is_control()) {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        out.push_str(s);
        out.push('"');
        return out;
    }
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// What the lexer reads: the tokens, the comments' spans (a tape's extras), and the first error. A token the lexer
/// cannot read is `Tok::Bad`; an unterminated string, backtick literal or comment is one `Bad` to the end of the file.
pub struct Lexed {
    pub toks: Vec<Spanned>,
    pub comments: Vec<(usize, usize)>,
    pub error: Option<(usize, Error)>,
}

pub fn tokens(src: &str) -> Result<Vec<Spanned>, Error> {
    let lexed = lex(src);
    match lexed.error {
        Some((_, error)) => Err(error),
        None => Ok(lexed.toks),
    }
}

pub fn lex(src: &str) -> Lexed {
    let bytes = src.as_bytes();
    let (mut pos, mut out, mut comments, mut error) = (0, Vec::new(), Vec::new(), None);
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut bad = |out: &mut Vec<Spanned>, start: usize, end: usize, what: &str| {
        out.push((Tok::Bad, start, end));
        if error.is_none() {
            error = Some((start, Error::Syntax(format!("{what} at line {}", line_of(src, start)))));
        }
    };
    loop {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let start = pos;
        let rest = &src[pos..];
        if rest.starts_with("//") {
            pos += rest.find('\n').unwrap_or(rest.len());
            comments.push((start, pos));
            continue;
        }
        if rest.starts_with("/*") {
            match rest.find("*/") {
                Some(e) => {
                    pos += e + 2;
                    comments.push((start, pos));
                    continue;
                }
                None => {
                    bad(&mut out, start, src.len(), "unterminated comment");
                    break;
                }
            }
        }
        let c = bytes[pos];
        if c == b'"' {
            let mut end = pos + 1;
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            if end >= bytes.len() {
                bad(&mut out, start, src.len(), "unterminated string");
                break;
            }
            match unescape(&src[pos + 1..end]) {
                Ok(value) => out.push((Tok::Str(value), start, end + 1)),
                Err(e) => bad(&mut out, start, end + 1, &e),
            }
            pos = end + 1;
        } else if c == b'`' {
            let n = bytes[pos..].iter().take_while(|&&b| b == b'`').count();
            let fence = &src[pos..pos + n];
            let body_start = pos + n;
            // Two backticks with no closing pair are the empty literal (`replace(c, ``)`).
            let close = match src[body_start..].find(fence) {
                Some(c) => c + body_start,
                None if n == 2 => {
                    out.push((Tok::Tick(String::new()), start, pos + 2));
                    pos += 2;
                    continue;
                }
                None => {
                    bad(&mut out, start, src.len(), "unterminated backtick literal");
                    break;
                }
            };
            let mut body = &src[body_start..close];
            if n > 1 {
                body = body.strip_prefix(' ').unwrap_or(body);
                body = body.strip_suffix(' ').unwrap_or(body);
            }
            out.push((Tok::Tick(body.to_owned()), start, close + n));
            pos = close + n;
        } else if c.is_ascii_digit() {
            let end = pos + bytes[pos..].iter().take_while(|b| b.is_ascii_digit()).count();
            // `1.5`: a `.` between digits; `0.` ends a clause.
            if bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(u8::is_ascii_digit) {
                let end = end + 1 + bytes[end + 1..].iter().take_while(|b| b.is_ascii_digit()).count();
                out.push((Tok::Float(src[pos..end].to_owned()), start, end));
                pos = end;
                continue;
            }
            match src[pos..end].parse() {
                Ok(value) => out.push((Tok::Num(value), start, end)),
                Err(_) => bad(&mut out, start, end, "bad number"),
            }
            pos = end;
        } else if c == b'#' {
            let end = pos + 1 + bytes[pos + 1..].iter().take_while(|&&b| ident(b)).count();
            out.push((Tok::Hash(src[pos + 1..end].to_owned()), start, end));
            pos = end;
        } else if let Some(p) = PUNCT.iter().find(|p| rest.starts_with(**p)) {
            out.push((Tok::P(p), start, pos + p.len()));
            pos += p.len();
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let end = pos + bytes[pos..].iter().take_while(|&&b| ident(b)).count();
            out.push((Tok::Id(src[pos..end].to_owned()), start, end));
            pos = end;
        } else {
            let end = pos + rest.chars().next().map_or(1, char::len_utf8);
            bad(&mut out, start, end, &format!("bad input {:?}", &rest[..rest.char_indices().nth(10).map_or(rest.len(), |(i, _)| i)]));
            pos = end;
        }
    }
    Lexed { toks: out, comments, error }
}

pub fn line_of(src: &str, pos: usize) -> usize {
    src[..pos.min(src.len())].bytes().filter(|&b| b == b'\n').count() + 1
}

/// Where each line of a text starts, so a position's line is a binary search, not a count from the start: a parser
/// asks the line of every statement, and a count from the start made a long file quadratic.
pub struct Lines(Vec<usize>);

impl Lines {
    pub fn of(src: &str) -> Self {
        Lines(src.bytes().enumerate().filter(|&(_, b)| b == b'\n').map(|(i, _)| i).collect())
    }

    /// The line of byte `pos`, from 1, as `line_of` gives it.
    pub fn line(&self, pos: usize) -> usize {
        self.0.partition_point(|&newline| newline < pos) + 1
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    Var(String),
    Wild,
    Num(i64),
    /// A decimal number, as written.
    Float(String),
    Str(String),
    Tick(String),
    /// `@f(…)` (`functor`), or a Soufflé built-in function such as `cat(…)`.
    Call { name: String, functor: bool, args: Vec<Term> },
    Bin { op: &'static str, l: Box<Term>, r: Box<Term> },
    /// `count : { … }`, `min x : { … }`, `join x by k sep "," : { … }`.
    Agg { op: String, over: Option<Box<Term>>, by: Vec<Term>, sep: Option<String>, body: Vec<Lit> },
    /// A JSON object argument, as written: `ast(n, { … })`.
    Json(String),
    /// `NAME: v`, an `ast` capture.
    Named(String, Box<Term>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Lit {
    Atom { rel: String, args: Vec<Term>, neg: bool },
    Cmp { l: Term, op: &'static str, r: Term },
    Pattern { subject: Term, pattern: String, neg: bool },
    /// `( a ; b )`: alternatives, each a conjunction.
    Group { alts: Vec<Vec<Lit>>, neg: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Head {
    pub modal: Option<String>,
    pub rel: String,
    pub args: Vec<Term>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Include(String),
    /// `.decl r(c: t, …)` and the words after it on its line (`widen 64`, `must, may`, `ordered by earlier`).
    Decl { name: String, cols: Vec<(String, String)>, tail: Vec<String> },
    Edit { name: String },
    Functor { name: String },
    Type { name: String },
    /// `.input a, b`; `params`: the relations given with Soufflé's I/O parameters, `.input r(IO=json, …)`, and
    /// the parameters' text.
    Io { output: bool, names: Vec<String>, params: Vec<(String, String)> },
    /// `.lattice L<base> { Op -> term, … }`.
    Lattice { name: String, base: String, ops: Vec<(String, Term)> },
    Pragma(Vec<String>),
    /// `.violation V(at: loc, …) "message"`, its `fix` lines' relations, its `suppress` lines, and `options { … }`.
    /// `code {c}` names the column holding the finding's code (`in "re"`: the codes it may produce), `code "X"` gives
    /// it, and `tag {t}` names an extra key field.
    Violation {
        name: String,
        cols: Vec<(String, String)>,
        code: Option<Code>,
        code_in: Option<String>,
        tag: Option<String>,
        message: String,
        fixes: Vec<String>,
        suppress: Vec<Suppress>,
    },
    /// `.policy { … }`: a policy document in JSON.
    Policy(serde_json::Value),
    /// A setting directive (`.policy "id" revision 1`, `.mode`, `.scope`, …): its name and its text.
    Setting { key: String, text: String },
    Clause { heads: Vec<Head>, body: Vec<Lit>, edits: Vec<Term> },
}

/// A `.violation`'s code: a column's value (`code {c}`), or a constant (`code "E501"`).
#[derive(Clone, Debug, PartialEq)]
pub enum Code {
    Column(String),
    Constant(String),
}

/// A `.violation`'s `suppress TOOL "rule", … [over {c}]` line: the tool comments that silence its findings.
#[derive(Clone, Debug, PartialEq)]
pub struct Suppress {
    pub tool: String,
    pub rules: Vec<String>,
    pub over: Option<String>,
}

/// A statement and the line it starts on.
pub type Statement = (Item, usize);

/// Reads statements; `taped` also builds the file's tree (decision 0061). Each production that makes a node opens it
/// at its first token and closes it after its last; tokens become leaves as they are read. Without a tree every
/// tree step does nothing.
pub struct Parser<'s> {
    src: &'s str,
    lines: Lines,
    toks: Vec<Spanned>,
    i: usize,
    /// Inside an aggregate's value or key, where `x :` ends the term (`agg_part`).
    no_named: bool,
    tree: Option<Tree>,
    comments: Vec<(usize, usize)>,
    /// Where the last `fail` was, in bytes.
    failed_at: Cell<usize>,
    /// The first error, and where it is: the lexer's, else the first statement's that did not parse.
    error: Option<(usize, Error)>,
}

/// Where a parse can go back to: the token, and the tree's state.
struct Save {
    i: usize,
    tree: Option<tape::Mark>,
}

impl<'s> Parser<'s> {
    pub fn new(src: &'s str) -> Result<Self, Error> {
        Ok(Self { src, lines: Lines::of(src), toks: tokens(src)?, i: 0, no_named: false, tree: None, comments: Vec::new(), failed_at: Cell::new(0), error: None })
    }

    /// A parser that builds the file's tree, and reads on past a statement that does not parse (`recover`).
    pub fn taped(src: &'s str) -> Self {
        let lexed = lex(src);
        Self {
            src,
            lines: Lines::of(src),
            toks: lexed.toks,
            i: 0,
            no_named: false,
            tree: Some(Tree::new(src.len())),
            comments: lexed.comments,
            failed_at: Cell::new(0),
            error: lexed.error,
        }
    }

    /// The tree read, in preorder with its comments (`tape::flatten`), and the first error.
    pub fn into_tape(self) -> (Vec<tape::Node>, Option<(usize, Error)>) {
        let nodes = self.tree.as_ref().map_or_else(Vec::new, |tree| tape::flatten(tree, &self.comments));
        (nodes, self.error)
    }

    fn peek(&self) -> &Tok {
        self.peek_at(0)
    }

    fn peek_at(&self, k: usize) -> &Tok {
        self.toks.get(self.i + k).map_or(&Tok::Eof, |t| &t.0)
    }

    fn is_p(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::P(q) if *q == p)
    }

    fn is_p_at(&self, k: usize, p: &str) -> bool {
        matches!(self.peek_at(k), Tok::P(q) if *q == p)
    }

    fn is_id(&self, id: &str) -> bool {
        matches!(self.peek(), Tok::Id(s) if s == id)
    }

    fn start(&self) -> usize {
        self.toks.get(self.i).map_or(self.src.len(), |t| t.1)
    }

    fn line(&self) -> usize {
        self.lines.line(self.start())
    }

    /// The line of the token before the next one.
    fn last_line(&self) -> usize {
        self.i.checked_sub(1).and_then(|i| self.toks.get(i)).map_or(0, |t| self.lines.line(t.2.saturating_sub(1)))
    }

    fn fail<T>(&self, what: &str) -> Result<T, Error> {
        let at = self.start();
        self.failed_at.set(at);
        let near: String = self.src[at..].chars().take(40).collect();
        Err(Error::Syntax(format!("line {}: expected {what}, got {:?} near {near:?}", self.lines.line(at), self.peek())))
    }

    // The tree's steps.

    fn save(&self) -> Save {
        Save { i: self.i, tree: self.tree.as_ref().map(|tree| tree.mark(self.i)) }
    }

    fn restore(&mut self, save: Save) {
        self.i = save.i;
        if let (Some(tree), Some(mark)) = (&mut self.tree, save.tree) {
            tree.reset(mark);
        }
    }

    fn open(&mut self, kind: Kind) {
        let at = self.start();
        if let Some(tree) = &mut self.tree {
            tree.push(kind as u16, at, at, true);
        }
    }

    fn close(&mut self) {
        let end = self.i.checked_sub(1).and_then(|i| self.toks.get(i)).map_or(0, |t| t.2);
        if let Some(tree) = &mut self.tree {
            tree.close(end);
        }
    }

    fn here(&self) -> usize {
        self.tree.as_ref().map_or(0, Tree::here)
    }

    fn wrap(&mut self, from: usize, kind: Kind) {
        if let Some(tree) = &mut self.tree {
            tree.wrap(from, kind as u16);
        }
    }

    /// Gives the open node's last child `field`.
    fn field(&mut self, field: Field) {
        if let Some(tree) = &mut self.tree
            && let Some(last) = tree.last()
        {
            tree.nodes[last as usize].field = field as u16;
        }
    }

    /// A leaf of `kind` over tokens `from..to`.
    fn leaf_over(&mut self, kind: u16, from: usize, to: usize) {
        if let Some(tree) = &mut self.tree
            && from < to
            && to <= self.toks.len()
        {
            tree.push(kind, self.toks[from].1, self.toks[to - 1].2, false);
        }
    }

    /// The next token, as a leaf of `kind`.
    fn bump_as(&mut self, kind: Kind) {
        self.leaf_over(kind as u16, self.i, self.i + 1);
        self.i += 1;
    }

    /// The anonymous token `text` at token `at`.
    fn anon_over(&mut self, text: &str, at: usize) {
        if self.tree.is_some() {
            let kind = tape::anonymous(text).unwrap_or(Kind::Identifier as u16);
            self.leaf_over(kind, at, at + 1);
        }
    }

    /// The next token, as an anonymous leaf (punctuation, a reserved word).
    fn anon(&mut self) {
        if self.tree.is_some() {
            let kind = match self.peek() {
                Tok::P(p) => tape::anonymous(p),
                Tok::Id(s) => tape::anonymous(s),
                Tok::Hash(h) => tape::anonymous(&format!("#{h}")),
                _ => None,
            };
            self.leaf_over(kind.unwrap_or(Kind::Identifier as u16), self.i, self.i + 1);
        }
        self.i += 1;
    }

    /// The next token as one of a line's words (a modifier, a setting's argument), given `field` if it is named.
    fn word(&mut self, field: Field) -> Result<(), Error> {
        let kind = match self.peek() {
            Tok::Id(_) => Kind::Identifier,
            Tok::Num(_) => Kind::Number,
            Tok::Float(_) => Kind::Float,
            Tok::Str(_) => Kind::String,
            Tok::Tick(_) => Kind::CodeLiteral,
            Tok::Bad => return self.fail("a word"),
            Tok::P(_) | Tok::Hash(_) | Tok::Eof => {
                self.anon();
                return Ok(());
            }
        };
        self.bump_as(kind);
        self.field(field);
        Ok(())
    }

    // Tokens.

    fn p(&mut self, p: &str) -> Result<(), Error> {
        if self.is_p(p) {
            self.anon();
            Ok(())
        } else {
            self.fail(&format!("{p:?}"))
        }
    }

    fn id(&mut self) -> Result<String, Error> {
        match self.peek().clone() {
            Tok::Id(s) => {
                self.bump_as(Kind::Identifier);
                Ok(s)
            }
            _ => self.fail("an identifier"),
        }
    }

    fn string(&mut self) -> Result<String, Error> {
        match self.peek().clone() {
            Tok::Str(s) => {
                self.bump_as(Kind::String);
                Ok(s)
            }
            _ => self.fail("a string"),
        }
    }

    /// Items separated by commas, up to `close`, each given `field`.
    fn list<T>(&mut self, close: &str, field: Field, mut item: impl FnMut(&mut Self) -> Result<T, Error>) -> Result<Vec<T>, Error> {
        let mut out = Vec::new();
        if self.is_p(close) {
            return Ok(out);
        }
        out.push(item(self)?);
        self.field(field);
        while self.is_p(",") {
            self.anon();
            out.push(item(self)?);
            self.field(field);
        }
        Ok(out)
    }

    /// The text of a balanced `{ … }`, braces included. Its tokens make no leaves: the caller makes one.
    fn braces(&mut self) -> Result<String, Error> {
        let start = self.start();
        if !self.is_p("{") {
            return self.fail("\"{\"");
        }
        self.i += 1;
        let mut depth = 1;
        while depth > 0 {
            match self.peek() {
                Tok::P("{") => depth += 1,
                Tok::P("}") => depth -= 1,
                Tok::Eof => return self.fail("a closing brace"),
                Tok::Bad => return self.fail("a closing brace"),
                _ => {}
            }
            self.i += 1;
        }
        Ok(self.src[start..self.toks[self.i - 1].2].to_owned())
    }

    /// `{ … }` as one `json_object` leaf, given `field`.
    fn json(&mut self, field: Field) -> Result<String, Error> {
        let from = self.i;
        let text = self.braces()?;
        self.leaf_over(Kind::JsonObject as u16, from, self.i);
        self.field(field);
        Ok(text)
    }

    /// `{c}`: a column named in braces (`code {c}`, `tag {t}`, `over {n}`), its name an identifier given `field`.
    fn brace_column(&mut self, field: Field) -> Result<String, Error> {
        let from = self.i;
        let text = self.braces()?;
        if self.i - from == 3 && matches!(self.toks[from + 1].0, Tok::Id(_)) {
            self.anon_over("{", from);
            self.leaf_over(Kind::Identifier as u16, from + 1, from + 2);
            self.field(field);
            self.anon_over("}", from + 2);
        } else {
            self.leaf_over(Kind::RawText as u16, from, self.i);
            self.field(field);
        }
        Ok(text.trim_start_matches('{').trim_end_matches('}').trim().to_owned())
    }

    /// A type: `number`, `node`, `flat<>`, `shortlex_min<>`.
    fn ty(&mut self) -> Result<String, Error> {
        self.open(Kind::Type);
        let mut t = self.id()?;
        self.field(Field::Name);
        if self.is_p("<") {
            self.anon();
            let inner = if self.is_p(">") {
                String::new()
            } else {
                let inner = self.id()?;
                self.field(Field::Argument);
                inner
            };
            self.p(">")?;
            t = format!("{t}<{inner}>");
        }
        self.close();
        Ok(t)
    }

    fn columns(&mut self) -> Result<Vec<(String, String)>, Error> {
        self.open(Kind::ColumnList);
        self.p("(")?;
        let cols = self.list(")", Field::Column, |p| {
            p.open(Kind::Column);
            let c = p.id()?;
            p.field(Field::Name);
            p.p(":")?;
            let t = p.ty()?;
            p.field(Field::Type);
            p.close();
            Ok((c, t))
        })?;
        self.p(")")?;
        self.close();
        self.field(Field::Columns);
        Ok(cols)
    }

    /// `( term, … )`, given the field `arguments`.
    fn arguments(&mut self) -> Result<Vec<Term>, Error> {
        self.open(Kind::ArgumentList);
        self.p("(")?;
        let args = self.list(")", Field::Argument, Self::term)?;
        self.p(")")?;
        self.close();
        self.field(Field::Arguments);
        Ok(args)
    }

    /// The words left on the current line: a declaration's modifiers, each a leaf given `field`.
    fn rest_of_line(&mut self, line: usize, field: Field) -> Result<Vec<String>, Error> {
        let mut out = Vec::new();
        while !matches!(self.peek(), Tok::Eof) && self.line() == line {
            out.push(match self.peek() {
                Tok::Id(s) => s.clone(),
                Tok::Num(n) => n.to_string(),
                Tok::Float(f) => f.clone(),
                Tok::Str(s) => s.clone(),
                Tok::P(p) => (*p).to_owned(),
                other => format!("{other:?}"),
            });
            self.word(field)?;
        }
        Ok(out)
    }

    /// The tokens left on the current line, as one `raw_text` leaf given `field`.
    fn raw_line(&mut self, line: usize, field: Field) -> Result<(), Error> {
        let from = self.i;
        while !matches!(self.peek(), Tok::Eof) && self.line() == line {
            if *self.peek() == Tok::Bad {
                return self.fail("a word");
            }
            self.i += 1;
        }
        self.leaf_over(Kind::RawText as u16, from, self.i);
        self.field(field);
        Ok(())
    }

    pub fn statements(&mut self) -> Result<Vec<Statement>, Error> {
        let mut out = Vec::new();
        while *self.peek() != Tok::Eof {
            let line = self.line();
            let save = self.save();
            match self.statement(line) {
                Ok(item) => out.push((item, line)),
                Err(error) if self.tree.is_some() => self.recover(save, error),
                Err(error) => return Err(error),
            }
        }
        Ok(out)
    }

    /// After a statement that did not parse: one ERROR leaf from its first token to where parsing resumes, the next
    /// `#include`, a `.word` at the start of a line, or the token after the next `.` that ends a clause.
    fn recover(&mut self, save: Save, error: Error) {
        if self.error.is_none() {
            self.error = Some((self.failed_at.get(), error));
        }
        let first = save.i;
        self.restore(save);
        let line_start = |at: usize| self.src[..at].bytes().rev().take_while(|&b| b != b'\n').all(|b| b.is_ascii_whitespace());
        let mut k = first + 1;
        while k < self.toks.len() {
            match &self.toks[k].0 {
                Tok::Hash(_) => break,
                Tok::P(".") if matches!(self.toks.get(k + 1).map(|t| &t.0), Some(Tok::Id(_))) && line_start(self.toks[k].1) => break,
                Tok::P(".") => {
                    k += 1;
                    break;
                }
                _ => k += 1,
            }
        }
        let k = k.min(self.toks.len());
        self.leaf_over(tape::ERROR, first, k);
        self.i = k;
    }

    fn statement(&mut self, line: usize) -> Result<Item, Error> {
        if let Tok::Hash(h) = self.peek().clone() {
            self.open(Kind::Include);
            self.anon();
            if h != "include" {
                return self.fail("#include");
            }
            let path = self.string()?;
            self.field(Field::Path);
            self.close();
            Ok(Item::Include(path))
        } else if self.is_p(".")
            && let Tok::Id(d) = self.peek_at(1).clone()
        {
            let policy = d == "policy" && self.is_p_at(2, "{");
            let kind = match d.as_str() {
                "decl" => Kind::Declaration,
                "edit" => Kind::EditDeclaration,
                "functor" => Kind::FunctorDeclaration,
                "type" => Kind::TypeDeclaration,
                "input" | "output" => Kind::IoDirective,
                "lattice" => Kind::LatticeDeclaration,
                "pragma" => Kind::Pragma,
                "violation" => Kind::ViolationDeclaration,
                "policy" if policy => Kind::PolicyDocument,
                _ => Kind::Setting,
            };
            self.open(kind);
            match tape::directive(&d).filter(|_| kind != Kind::Setting) {
                Some(keyword) => {
                    self.leaf_over(keyword, self.i, self.i + 2);
                    self.i += 2;
                }
                None => {
                    self.anon();
                    self.bump_as(Kind::Identifier);
                    self.field(Field::Name);
                }
            }
            let item = self.directive(line, d)?;
            self.close();
            Ok(item)
        } else {
            self.clause()
        }
    }

    /// A directive's statement, after its `.word`.
    fn directive(&mut self, line: usize, d: String) -> Result<Item, Error> {
        Ok(match d.as_str() {
            "decl" => {
                let name = self.id()?;
                self.field(Field::Name);
                let cols = self.columns()?;
                let at = self.last_line();
                Item::Decl { name, cols, tail: self.rest_of_line(at, Field::Modifier)? }
            }
            "edit" => {
                let name = self.id()?;
                self.field(Field::Name);
                self.columns()?;
                Item::Edit { name }
            }
            "functor" => {
                let name = self.id()?;
                self.field(Field::Name);
                self.columns()?;
                self.p(":")?;
                self.ty()?;
                self.field(Field::ReturnType);
                let at = self.last_line();
                self.rest_of_line(at, Field::Modifier)?;
                Item::Functor { name }
            }
            "type" => {
                let name = self.id()?;
                self.field(Field::Name);
                let at = self.last_line();
                self.raw_line(at, Field::Definition)?;
                Item::Type { name }
            }
            "input" | "output" => {
                let mut params = Vec::new();
                let names = self.list(".", Field::Relation, |p| {
                    p.open(Kind::IoRelation);
                    let name = p.id()?;
                    p.field(Field::Name);
                    if p.is_p("(") {
                        let start = p.start();
                        p.anon();
                        let from = p.i;
                        let mut depth = 1;
                        while depth > 0 {
                            match p.peek() {
                                Tok::P("(") => depth += 1,
                                Tok::P(")") => depth -= 1,
                                Tok::Eof | Tok::Bad => return p.fail("a closing parenthesis"),
                                _ => {}
                            }
                            p.i += 1;
                        }
                        let close = p.i - 1;
                        p.leaf_over(Kind::RawText as u16, from, close);
                        p.field(Field::Parameters);
                        p.anon_over(")", close);
                        let text = p.src[start + 1..p.toks[close].1].to_owned();
                        params.push((name.clone(), text));
                    }
                    p.close();
                    Ok(name)
                })?;
                Item::Io { output: d == "output", names, params }
            }
            "lattice" => {
                let name = self.id()?;
                self.field(Field::Name);
                self.p("<")?;
                let base = if self.is_p(">") {
                    String::new()
                } else {
                    let base = self.id()?;
                    self.field(Field::Base);
                    base
                };
                self.p(">")?;
                self.p("{")?;
                let ops = self.list("}", Field::Operation, |p| {
                    p.open(Kind::LatticeOperation);
                    let op = p.id()?;
                    p.field(Field::Name);
                    p.p("->")?;
                    // A functor named, not called: `Lub -> @shortlex_min`.
                    if p.is_p("@") && matches!(p.peek_at(1), Tok::Id(_)) && !p.is_p_at(2, "(") {
                        p.open(Kind::FunctorCall);
                        p.anon();
                        let name = p.id()?;
                        p.field(Field::Function);
                        p.close();
                        p.field(Field::Value);
                        p.close();
                        return Ok((op, Term::Call { name, functor: true, args: Vec::new() }));
                    }
                    let t = p.term()?;
                    p.field(Field::Value);
                    p.close();
                    Ok((op, t))
                })?;
                self.p("}")?;
                Item::Lattice { name, base, ops }
            }
            "pragma" => {
                let mut words = Vec::new();
                while let Tok::Str(_) = self.peek() {
                    words.push(self.string()?);
                    self.field(Field::Argument);
                }
                Item::Pragma(words)
            }
            "violation" => {
                let name = self.id()?;
                self.field(Field::Name);
                let cols = self.columns()?;
                // `code {c}`: the finding's code, from a column (`in "re"` after it: the codes it may produce);
                // `code "X"`: a constant code; `tag {t}`: an extra key field.
                let (mut code, mut code_in, mut tag) = (None, None, None);
                loop {
                    if self.is_id("code") && matches!(self.peek_at(1), Tok::Str(_)) {
                        self.anon();
                        code = Some(Code::Constant(self.string()?));
                        self.field(Field::Code);
                    } else if (self.is_id("code") || self.is_id("tag")) && self.is_p_at(1, "{") {
                        let is_code = self.is_id("code");
                        self.open(if is_code { Kind::CodeColumn } else { Kind::TagColumn });
                        self.anon();
                        let column = self.brace_column(Field::Column)?;
                        if is_code {
                            code = Some(Code::Column(column));
                            if self.is_id("in") && matches!(self.peek_at(1), Tok::Str(_)) {
                                self.anon();
                                code_in = Some(self.string()?);
                                self.field(Field::Codes);
                            }
                        } else {
                            tag = Some(column);
                        }
                        self.close();
                        self.field(if is_code { Field::Code } else { Field::Tag });
                    } else {
                        break;
                    }
                }
                let message = self.string()?;
                self.field(Field::Message);
                let (mut fixes, mut suppress) = (Vec::new(), Vec::new());
                // Continuation lines: `fix [safe] name "…"`, `suppress TOOL "rule", …`, `options { … }`.
                loop {
                    if self.is_id("fix") && matches!(self.peek_at(1), Tok::Id(_)) {
                        self.open(Kind::FixLine);
                        self.anon();
                        if self.is_id("safe") && matches!(self.peek_at(1), Tok::Id(_)) {
                            self.anon();
                        }
                        fixes.push(self.id()?);
                        self.field(Field::Relation);
                        self.string()?;
                        self.field(Field::Description);
                        self.close();
                        self.field(Field::Fix);
                    } else if self.is_id("suppress") && matches!(self.peek_at(1), Tok::Id(_)) && matches!(self.peek_at(2), Tok::Str(_)) {
                        self.open(Kind::SuppressLine);
                        self.anon();
                        let tool = self.id()?;
                        self.field(Field::Tool);
                        let mut rules = vec![self.string()?];
                        self.field(Field::Rule);
                        while self.is_p(",") && matches!(self.peek_at(1), Tok::Str(_)) {
                            self.anon();
                            rules.push(self.string()?);
                            self.field(Field::Rule);
                        }
                        // `over {c}`: the comment is read on the lines of column c's node too.
                        let mut over = None;
                        if self.is_id("over") && self.is_p_at(1, "{") {
                            self.anon();
                            over = Some(self.brace_column(Field::Over)?);
                        }
                        self.close();
                        self.field(Field::Suppress);
                        suppress.push(Suppress { tool, rules, over });
                    } else if self.is_id("options") && self.is_p_at(1, "{") {
                        self.anon();
                        self.json(Field::Options)?;
                    } else {
                        break;
                    }
                }
                Item::Violation { name, cols, code, code_in, tag, message, fixes, suppress }
            }
            "policy" if self.is_p("{") => {
                let text = self.json(Field::Document)?;
                Item::Policy(serde_json::from_str(&text).map_err(|e| Error::Syntax(format!("line {line}: bad .policy document: {e}")))?)
            }
            _ => {
                // A setting: its text to the end of the line, and any `{ … }` it opens; a line that ends in `,`
                // goes on to the next.
                let start = self.start();
                let mut end = start;
                let mut line = line;
                while !matches!(self.peek(), Tok::Eof) && (self.line() == line || (self.i > 0 && self.toks[self.i - 1].0 == Tok::P(","))) {
                    line = self.line();
                    if self.is_p("{") {
                        self.json(Field::Argument)?;
                    } else {
                        self.word(Field::Argument)?;
                    }
                    end = self.toks[self.i - 1].2;
                }
                Item::Setting { key: d, text: self.src[start..end.max(start)].to_owned() }
            }
        })
    }

    fn head(&mut self) -> Result<Head, Error> {
        self.open(Kind::Head);
        let modal = if (self.is_id("must") || self.is_id("may")) && matches!(self.peek_at(1), Tok::Id(_)) {
            let Tok::Id(modal) = self.peek().clone() else { unreachable!() };
            self.anon();
            self.field(Field::Modal);
            Some(modal)
        } else {
            None
        };
        let rel = self.id()?;
        self.field(Field::Relation);
        let args = self.arguments()?;
        self.close();
        Ok(Head { modal, rel, args })
    }

    fn clause(&mut self) -> Result<Item, Error> {
        self.open(Kind::Clause);
        let mut heads = vec![self.head()?];
        self.field(Field::Head);
        while self.is_p(",") {
            self.anon();
            heads.push(self.head()?);
            self.field(Field::Head);
        }
        let mut body = Vec::new();
        if self.is_p(":-") {
            self.anon();
            let alts = self.disjunction()?;
            self.field(Field::Body);
            body = if alts.len() == 1 { alts.into_iter().next().expect("one") } else { vec![Lit::Group { alts, neg: false }] };
        }
        let mut edits = Vec::new();
        if self.is_p("=>") {
            self.anon();
            edits = self.list(".", Field::Edit, Self::term)?;
        }
        self.p(".")?;
        self.close();
        Ok(Item::Clause { heads, body, edits })
    }

    /// A body: a `conjunction`, or a `disjunction` of them where a `;` follows the first.
    fn disjunction(&mut self) -> Result<Vec<Vec<Lit>>, Error> {
        let from = self.here();
        let mut alts = vec![self.conjunction()?];
        if self.is_p(";") {
            self.wrap(from, Kind::Disjunction);
            self.field(Field::Alternative);
            while self.is_p(";") {
                self.anon();
                alts.push(self.conjunction()?);
                self.field(Field::Alternative);
            }
            self.close();
        }
        Ok(alts)
    }

    fn conjunction(&mut self) -> Result<Vec<Lit>, Error> {
        self.open(Kind::Conjunction);
        let mut out = vec![self.literal()?];
        self.field(Field::Literal);
        while self.is_p(",") {
            self.anon();
            out.push(self.literal()?);
            self.field(Field::Literal);
        }
        self.close();
        Ok(out)
    }

    fn literal(&mut self) -> Result<Lit, Error> {
        if self.is_p("!") {
            self.open(Kind::Negation);
            self.anon();
            let inner = self.literal()?;
            self.field(Field::Operand);
            self.close();
            return Ok(match inner {
                Lit::Atom { rel, args, neg } => Lit::Atom { rel, args, neg: !neg },
                Lit::Pattern { subject, pattern, neg } => Lit::Pattern { subject, pattern, neg: !neg },
                Lit::Group { alts, neg } => Lit::Group { alts, neg: !neg },
                Lit::Cmp { .. } => return self.fail("an atom after !"),
            });
        }
        if self.is_p("(") {
            // A group `( a ; b )`, or a parenthesized term that a comparison follows: `(a - b) * 10 > o`.
            let save = self.save();
            self.open(Kind::Group);
            self.anon();
            let group = self.disjunction().and_then(|alts| {
                self.field(Field::Body);
                self.p(")").map(|()| alts)
            });
            let term_follows = matches!(self.peek(), Tok::P("=" | "!=" | "<" | "<=" | ">" | ">=" | "+" | "-" | "*" | "/" | "%"));
            match group {
                Ok(alts) if !term_follows => {
                    self.close();
                    return Ok(Lit::Group { alts, neg: false });
                }
                _ => self.restore(save),
            }
        }
        let from = self.here();
        let l = self.term()?;
        if self.is_p("~") {
            self.wrap(from, Kind::PatternMatch);
            self.field(Field::Subject);
            self.anon();
            let pattern = match self.peek().clone() {
                Tok::Tick(t) => t,
                _ => return self.fail("a backtick pattern"),
            };
            self.bump_as(Kind::CodeLiteral);
            self.field(Field::Pattern);
            self.close();
            return Ok(Lit::Pattern { subject: l, pattern, neg: false });
        }
        let op = match self.peek() {
            Tok::P(p @ ("=" | "!=" | "<" | "<=" | ">" | ">=")) => *p,
            _ => {
                return match l {
                    Term::Call { name, functor: false, args } => {
                        self.retag_atom();
                        Ok(Lit::Atom { rel: name, args, neg: false })
                    }
                    _ => self.fail("an atom or a comparison"),
                };
            }
        };
        self.wrap(from, Kind::Comparison);
        self.field(Field::Left);
        self.anon();
        self.field(Field::Operator);
        let r = self.term()?;
        self.field(Field::Right);
        self.close();
        Ok(Lit::Cmp { l, op, r })
    }

    /// The `call` just read is a literal: an `atom`, whose function is its `relation`.
    fn retag_atom(&mut self) {
        if let Some(tree) = &mut self.tree
            && let Some(call) = tree.last()
        {
            tree.nodes[call as usize].kind = Kind::Atom as u16;
            let children = tree.nodes[call as usize].children.clone();
            for child in children {
                let node = &mut tree.nodes[child as usize];
                if node.field == Field::Function as u16 {
                    node.field = Field::Relation as u16;
                }
            }
        }
    }

    pub fn term(&mut self) -> Result<Term, Error> {
        let from = self.here();
        let mut l = self.product()?;
        while let Tok::P(op @ ("+" | "-")) = self.peek() {
            let op = *op;
            self.wrap(from, Kind::BinaryExpression);
            self.field(Field::Left);
            self.anon();
            self.field(Field::Operator);
            let r = self.product()?;
            self.field(Field::Right);
            self.close();
            l = Term::Bin { op, l: Box::new(l), r: Box::new(r) };
        }
        Ok(l)
    }

    fn product(&mut self) -> Result<Term, Error> {
        let from = self.here();
        let mut l = self.primary()?;
        while let Tok::P(op @ ("*" | "/" | "%")) = self.peek() {
            let op = *op;
            self.wrap(from, Kind::BinaryExpression);
            self.field(Field::Left);
            self.anon();
            self.field(Field::Operator);
            let r = self.primary()?;
            self.field(Field::Right);
            self.close();
            l = Term::Bin { op, l: Box::new(l), r: Box::new(r) };
        }
        Ok(l)
    }

    /// An aggregate, from its operator word `op`: `count : { … }`, `min x : { … }`, `join x by k sep "," : { … }`.
    fn aggregate(&mut self, op: String) -> Result<Term, Error> {
        self.open(Kind::Aggregate);
        self.anon();
        self.field(Field::Operator);
        let over = if self.is_p(":") {
            None
        } else {
            let over = self.agg_part()?;
            self.field(Field::Value);
            Some(Box::new(over))
        };
        let (mut by, mut sep) = (Vec::new(), None);
        if self.is_id("by") {
            self.anon();
            if self.is_p("(") {
                self.anon();
                by = self.list(")", Field::Key, Self::term)?;
                self.p(")")?;
            } else {
                by.push(self.agg_part()?);
                self.field(Field::Key);
            }
        }
        if self.is_id("sep") {
            self.anon();
            sep = Some(self.string()?);
            self.field(Field::Separator);
        }
        self.p(":")?;
        self.p("{")?;
        let alts = self.disjunction()?;
        self.field(Field::Body);
        self.p("}")?;
        self.close();
        let body = if alts.len() == 1 { alts.into_iter().next().expect("one") } else { vec![Lit::Group { alts, neg: false }] };
        Ok(Term::Agg { op, over, by, sep, body })
    }

    fn primary(&mut self) -> Result<Term, Error> {
        let (t, kind) = match self.peek().clone() {
            Tok::Num(n) => (Term::Num(n), Kind::Number),
            Tok::Float(f) => (Term::Float(f), Kind::Float),
            Tok::Str(s) => (Term::Str(s), Kind::String),
            Tok::Tick(t) => (Term::Tick(t), Kind::CodeLiteral),
            // A negative literal is one token: `-1` is a `number` that spans the sign.
            Tok::P("-") if matches!(self.peek_at(1), Tok::Num(_) | Tok::Float(_)) => {
                let from = self.i;
                let term = match self.peek_at(1).clone() {
                    Tok::Num(n) => Term::Num(-n),
                    Tok::Float(f) => Term::Float(format!("-{f}")),
                    _ => unreachable!(),
                };
                let kind = if matches!(term, Term::Num(_)) { Kind::Number } else { Kind::Float };
                self.i += 2;
                self.leaf_over(kind as u16, from, from + 2);
                return Ok(term);
            }
            // `-x`, `-f(x)`: zero minus it.
            Tok::P("-") => {
                self.open(Kind::UnaryExpression);
                self.anon();
                self.field(Field::Operator);
                let operand = self.primary()?;
                self.field(Field::Operand);
                self.close();
                return Ok(Term::Bin { op: "-", l: Box::new(Term::Num(0)), r: Box::new(operand) });
            }
            Tok::P("(") => {
                self.open(Kind::ParenthesizedExpression);
                self.anon();
                let t = self.term()?;
                self.field(Field::Expression);
                self.p(")")?;
                self.close();
                return Ok(t);
            }
            Tok::P("{") => return Ok(Term::Json(self.json(Field::None)?)),
            Tok::P("@") => {
                self.open(Kind::FunctorCall);
                self.anon();
                let name = self.id()?;
                self.field(Field::Function);
                let args = self.arguments()?;
                self.close();
                return Ok(Term::Call { name, functor: true, args });
            }
            Tok::Id(s) if s == "_" => (Term::Wild, Kind::Wildcard),
            Tok::Id(s) if s == "count" && self.is_p_at(1, ":") => return self.aggregate(s),
            // `min (a, b)`: a call, unless a `:` follows the parentheses (`min (a + b) : { … }`).
            Tok::Id(s) if matches!(s.as_str(), "min" | "max" | "sum" | "product" | "mean" | "join") && self.is_p_at(1, "(") => return self.call_or_aggregate(s),
            // `sum x : {…}`, `sum strlen(x) - strlen(a) : {…}`, `join x by k sep "," : {…}`: an aggregate over a term,
            // when a `:`, `by` or `sep` follows it; otherwise a variable named like an aggregate.
            Tok::Id(s) if matches!(s.as_str(), "min" | "max" | "sum" | "product" | "mean" | "join") && !matches!(self.peek_at(1), Tok::P("," | ")" | "=" | "!=" | "<" | ">" | "<=" | ">=" | "+" | "-" | "*" | "/" | "%" | "." | ";" | "}")) => {
                let save = self.save();
                self.i += 1;
                let aggregate = self.agg_part().is_ok() && (self.is_p(":") || self.is_id("by") || self.is_id("sep"));
                self.restore(save);
                if aggregate {
                    return self.aggregate(s);
                }
                (Term::Var(s), Kind::Variable)
            }
            Tok::Id(s) if self.is_p_at(1, "(") => {
                self.open(Kind::Call);
                self.bump_as(Kind::Identifier);
                self.field(Field::Function);
                let args = self.arguments()?;
                self.close();
                return Ok(Term::Call { name: s, functor: false, args });
            }
            Tok::Id(s) if self.is_p_at(1, ":") && !self.no_named => {
                self.open(Kind::NamedArgument);
                self.bump_as(Kind::Identifier);
                self.field(Field::Name);
                self.anon();
                let value = self.term()?;
                self.field(Field::Value);
                self.close();
                return Ok(Term::Named(s, Box::new(value)));
            }
            Tok::Id(s) => (Term::Var(s), Kind::Variable),
            _ => return self.fail("a term"),
        };
        self.bump_as(kind);
        Ok(t)
    }

    /// An aggregate's value or key: a term, in which `x :` is a variable followed by the aggregate's `:`, not an
    /// `ast` capture.
    fn agg_part(&mut self) -> Result<Term, Error> {
        let named = std::mem::replace(&mut self.no_named, true);
        let t = self.term();
        self.no_named = named;
        t
    }

    /// `min(a, b)` is a call unless a `:` follows the parentheses (`min (a + b) : { … }`).
    fn call_or_aggregate(&mut self, op: String) -> Result<Term, Error> {
        let save = self.save();
        self.open(Kind::Call);
        self.bump_as(Kind::Identifier);
        self.field(Field::Function);
        let args = self.arguments()?;
        self.close();
        if self.is_p(":") {
            self.restore(save);
            return self.aggregate(op);
        }
        Ok(Term::Call { name: op, functor: false, args })
    }
}

impl Term {
    /// The variables in the term, patterns' and templates' `$X` included.
    pub fn vars(&self, out: &mut Vec<String>) {
        match self {
            Term::Var(v) => out.push(v.clone()),
            Term::Tick(t) => out.extend(tick_vars(t)),
            Term::Call { args, .. } => args.iter().for_each(|a| a.vars(out)),
            Term::Bin { l, r, .. } => {
                l.vars(out);
                r.vars(out);
            }
            Term::Named(_, t) => t.vars(out),
            Term::Agg { over, by, body, .. } => {
                over.iter().for_each(|o| o.vars(out));
                by.iter().for_each(|b| b.vars(out));
                body.iter().for_each(|l| l.vars(out));
            }
            Term::Wild | Term::Num(_) | Term::Float(_) | Term::Str(_) | Term::Json(_) => {}
        }
    }
}

impl Lit {
    /// The variables of this literal's own scope: not those only inside an aggregate's body, which are that
    /// body's own unless they appear outside it too (Soufflé's scoping).
    pub fn scope_vars(&self, out: &mut Vec<String>) {
        fn term(t: &Term, out: &mut Vec<String>) {
            match t {
                Term::Agg { .. } => {}
                Term::Call { args, .. } => args.iter().for_each(|a| term(a, out)),
                Term::Bin { l, r, .. } => {
                    term(l, out);
                    term(r, out);
                }
                Term::Named(_, t) => term(t, out),
                t => t.vars(out),
            }
        }
        match self {
            Lit::Atom { args, .. } => args.iter().for_each(|a| term(a, out)),
            Lit::Cmp { l, r, .. } => {
                term(l, out);
                term(r, out);
            }
            Lit::Group { alts, .. } => alts.iter().flatten().for_each(|l| l.scope_vars(out)),
            l @ Lit::Pattern { .. } => l.vars(out),
        }
    }

    pub fn vars(&self, out: &mut Vec<String>) {
        match self {
            Lit::Atom { args, .. } => args.iter().for_each(|a| a.vars(out)),
            Lit::Cmp { l, r, .. } => {
                l.vars(out);
                r.vars(out);
            }
            Lit::Pattern { subject, pattern, .. } => {
                subject.vars(out);
                out.extend(tick_vars(pattern));
            }
            Lit::Group { alts, .. } => alts.iter().flatten().for_each(|l| l.vars(out)),
        }
    }
}

/// The `$X`, `$$$X` names in a pattern or template.
pub fn tick_vars(t: &str) -> Vec<String> {
    let b = t.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' {
            let mut j = i;
            while j < b.len() && b[j] == b'$' {
                j += 1;
            }
            let k = j + b[j..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == b'_').count();
            if k > j && (b[j].is_ascii_alphabetic() || b[j] == b'_') {
                out.push(t[j..k].to_owned());
            }
            i = k.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}
