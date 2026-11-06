//! The `.rr` text as written: tokens, and statements before lowering (`lower`) or checking (`check`).
//!
//! Soufflé's grammar, with the additions `docs/rule-syntax/lib/core.rr` lists: patterns (``n ~ `…` ``), named regex
//! groups in `match`, `must`/`may` heads, `.violation` with `fix` lines, `=> edits`, `join … by … sep`, and
//! `ordered by` on a declaration. The policy directives (`.policy`, `.mode`, `.scope`, …) are settings: each is kept
//! as its text, up to the end of its line. Two JSON forms stay for what has no Datalog form yet: the matcher term
//! `ast(n, { … })` and the policy document `.policy { … }`.

use super::Error;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Tok {
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
    Eof,
}

const PUNCT: [&str; 31] = [
    ":-", "=>", "!=", "<=", ">=", "<:", "->", "(", ")", ",", ".", ";", ":", "=", "<", ">", "!", "{", "}", "|", "@", "*", "[", "]", "-", "+",
    "/", "%", "~", "^", "&",
];

/// A token, and the byte range it spans.
pub(super) type Spanned = (Tok, usize, usize);

/// A string literal's escapes: `\"`, `\\`, `\n`, `\t`, `\r`, `\u{…}` and JSON's `\uXXXX`. Any other backslash is
/// kept with the character after it, so a regex reads as written (`"\d+\.py"`).
fn unescape(body: &str) -> Result<String, String> {
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
pub(in crate::code_rule_dsl) fn quote(s: &str) -> String {
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

pub(super) fn tokens(src: &str) -> Result<Vec<Spanned>, Error> {
    let bytes = src.as_bytes();
    let (mut pos, mut out) = (0, Vec::new());
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let fail = |pos: usize, what: &str| Error::Syntax(format!("{what} at line {}", line_of(src, pos)));
    loop {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            return Ok(out);
        }
        let start = pos;
        let rest = &src[pos..];
        if rest.starts_with("//") {
            pos += rest.find('\n').unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("/*") {
            pos += rest.find("*/").map(|e| e + 2).ok_or_else(|| fail(pos, "unterminated comment"))?;
            continue;
        }
        let c = bytes[pos];
        if c == b'"' {
            let mut end = pos + 1;
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            if end >= bytes.len() {
                return Err(fail(pos, "unterminated string"));
            }
            let value = unescape(&src[pos + 1..end]).map_err(|e| fail(pos, &e))?;
            out.push((Tok::Str(value), start, end + 1));
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
                None => return Err(fail(pos, "unterminated backtick literal")),
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
            let value = src[pos..end].parse().map_err(|_| fail(pos, "bad number"))?;
            out.push((Tok::Num(value), start, end));
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
            return Err(fail(pos, &format!("bad input {:?}", &rest[..rest.char_indices().nth(10).map_or(rest.len(), |(i, _)| i)])));
        }
    }
}

pub(super) fn line_of(src: &str, pos: usize) -> usize {
    src[..pos.min(src.len())].bytes().filter(|&b| b == b'\n').count() + 1
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Term {
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
pub(super) enum Lit {
    Atom { rel: String, args: Vec<Term>, neg: bool },
    Cmp { l: Term, op: &'static str, r: Term },
    Pattern { subject: Term, pattern: String, neg: bool },
    /// `( a ; b )`: alternatives, each a conjunction.
    Group { alts: Vec<Vec<Lit>>, neg: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Head {
    pub(super) modal: Option<String>,
    pub(super) rel: String,
    pub(super) args: Vec<Term>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Item {
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
    /// `.violation V(at: node, …) "message"`, its `fix` lines' relations, and `options { … }`.
    /// `code {c}` and `tag {t}` name the columns holding the finding's code and an extra key field.
    Violation { name: String, cols: Vec<(String, String)>, code: Option<String>, tag: Option<String>, message: String, fixes: Vec<String> },
    /// `.policy { … }`: a policy document in JSON.
    Policy(serde_json::Value),
    /// A setting directive (`.policy "id" revision 1`, `.mode`, `.scope`, …): its name and its text.
    Setting { key: String, text: String },
    Clause { heads: Vec<Head>, body: Vec<Lit>, edits: Vec<Term> },
}

/// A statement and the line it starts on.
pub(super) type Statement = (Item, usize);

pub(super) struct Parser<'s> {
    src: &'s str,
    toks: Vec<Spanned>,
    i: usize,
}

impl<'s> Parser<'s> {
    pub(super) fn new(src: &'s str) -> Result<Self, Error> {
        Ok(Self { src, toks: tokens(src)?, i: 0 })
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
        line_of(self.src, self.start())
    }

    /// The line of the token before the next one.
    fn last_line(&self) -> usize {
        self.i.checked_sub(1).and_then(|i| self.toks.get(i)).map_or(0, |t| line_of(self.src, t.2.saturating_sub(1)))
    }

    fn fail<T>(&self, what: &str) -> Result<T, Error> {
        let at = self.start();
        let near: String = self.src[at..].chars().take(40).collect();
        Err(Error::Syntax(format!("line {}: expected {what}, got {:?} near {near:?}", line_of(self.src, at), self.peek())))
    }

    fn p(&mut self, p: &str) -> Result<(), Error> {
        if self.is_p(p) {
            self.i += 1;
            Ok(())
        } else {
            self.fail(&format!("{p:?}"))
        }
    }

    fn id(&mut self) -> Result<String, Error> {
        match self.peek().clone() {
            Tok::Id(s) => {
                self.i += 1;
                Ok(s)
            }
            _ => self.fail("an identifier"),
        }
    }

    fn string(&mut self) -> Result<String, Error> {
        match self.peek().clone() {
            Tok::Str(s) => {
                self.i += 1;
                Ok(s)
            }
            _ => self.fail("a string"),
        }
    }

    fn list<T>(&mut self, close: &str, mut item: impl FnMut(&mut Self) -> Result<T, Error>) -> Result<Vec<T>, Error> {
        let mut out = Vec::new();
        if self.is_p(close) {
            return Ok(out);
        }
        out.push(item(self)?);
        while self.is_p(",") {
            self.i += 1;
            out.push(item(self)?);
        }
        Ok(out)
    }

    /// The text of a balanced `{ … }`, braces included.
    fn braces(&mut self) -> Result<String, Error> {
        let start = self.start();
        self.p("{")?;
        let mut depth = 1;
        while depth > 0 {
            match self.peek() {
                Tok::P("{") => depth += 1,
                Tok::P("}") => depth -= 1,
                Tok::Eof => return self.fail("a closing brace"),
                _ => {}
            }
            self.i += 1;
        }
        Ok(self.src[start..self.toks[self.i - 1].2].to_owned())
    }

    /// A type: `number`, `node`, `flat<>`, `shortlex_min<>`.
    fn ty(&mut self) -> Result<String, Error> {
        let mut t = self.id()?;
        if self.is_p("<") {
            self.i += 1;
            let inner = if self.is_p(">") { String::new() } else { self.id()? };
            self.p(">")?;
            t = format!("{t}<{inner}>");
        }
        Ok(t)
    }

    fn columns(&mut self) -> Result<Vec<(String, String)>, Error> {
        self.p("(")?;
        let cols = self.list(")", |p| {
            let c = p.id()?;
            p.p(":")?;
            Ok((c, p.ty()?))
        })?;
        self.p(")")?;
        Ok(cols)
    }

    /// The words left on the current line: a declaration's modifiers.
    fn rest_of_line(&mut self, line: usize) -> Vec<String> {
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
            self.i += 1;
        }
        out
    }

    pub(super) fn statements(&mut self) -> Result<Vec<Statement>, Error> {
        let mut out = Vec::new();
        while *self.peek() != Tok::Eof {
            let line = self.line();
            let item = if let Tok::Hash(h) = self.peek().clone() {
                self.i += 1;
                if h != "include" {
                    return self.fail("#include");
                }
                Item::Include(self.string()?)
            } else if self.is_p(".") && matches!(self.peek_at(1), Tok::Id(_)) {
                self.i += 1;
                self.directive(line)?
            } else {
                self.clause()?
            };
            out.push((item, line));
        }
        Ok(out)
    }

    fn directive(&mut self, line: usize) -> Result<Item, Error> {
        let d = self.id()?;
        Ok(match d.as_str() {
            "decl" => {
                let name = self.id()?;
                let cols = self.columns()?;
                let at = self.last_line();
                Item::Decl { name, cols, tail: self.rest_of_line(at) }
            }
            "edit" => {
                let name = self.id()?;
                self.columns()?;
                Item::Edit { name }
            }
            "functor" => {
                let name = self.id()?;
                self.columns()?;
                self.p(":")?;
                self.ty()?;
                let at = self.last_line();
                self.rest_of_line(at);
                Item::Functor { name }
            }
            "type" => {
                let name = self.id()?;
                let at = self.last_line();
                self.rest_of_line(at);
                Item::Type { name }
            }
            "input" | "output" => {
                let mut params = Vec::new();
                let names = self.list(".", |p| {
                    let name = p.id()?;
                    if p.is_p("(") {
                        let start = p.start();
                        p.i += 1;
                        let mut depth = 1;
                        while depth > 0 {
                            match p.peek() {
                                Tok::P("(") => depth += 1,
                                Tok::P(")") => depth -= 1,
                                Tok::Eof => return p.fail("a closing parenthesis"),
                                _ => {}
                            }
                            p.i += 1;
                        }
                        let text = p.src[start + 1..p.toks[p.i - 1].1].to_owned();
                        params.push((name.clone(), text));
                    }
                    Ok(name)
                })?;
                Item::Io { output: d == "output", names, params }
            }
            "lattice" => {
                let name = self.id()?;
                self.p("<")?;
                let base = if self.is_p(">") { String::new() } else { self.id()? };
                self.p(">")?;
                self.p("{")?;
                let ops = self.list("}", |p| {
                    let op = p.id()?;
                    p.p("->")?;
                    // A functor named, not called: `Lub -> @shortlex_min`.
                    if p.is_p("@") && matches!(p.peek_at(1), Tok::Id(_)) && !p.is_p_at(2, "(") {
                        p.i += 1;
                        return Ok((op, Term::Call { name: p.id()?, functor: true, args: Vec::new() }));
                    }
                    Ok((op, p.term()?))
                })?;
                self.p("}")?;
                Item::Lattice { name, base, ops }
            }
            "pragma" => {
                let mut words = Vec::new();
                while let Tok::Str(_) = self.peek() {
                    words.push(self.string()?);
                }
                Item::Pragma(words)
            }
            "violation" => {
                let name = self.id()?;
                let cols = self.columns()?;
                // `code {c}`: the finding's code, from a column; `tag {t}`: an extra key field.
                let (mut code, mut tag) = (None, None);
                while (self.is_id("code") || self.is_id("tag")) && self.is_p_at(1, "{") {
                    let which = if self.is_id("code") { &mut code } else { &mut tag };
                    self.i += 1;
                    let text = self.braces()?;
                    *which = Some(text.trim_start_matches('{').trim_end_matches('}').trim().to_owned());
                }
                let message = self.string()?;
                let mut fixes = Vec::new();
                // Continuation lines: `fix [safe] name "…"`, `options { … }`.
                loop {
                    if self.is_id("fix") && matches!(self.peek_at(1), Tok::Id(_)) {
                        self.i += 1;
                        if self.is_id("safe") && matches!(self.peek_at(1), Tok::Id(_)) {
                            self.i += 1;
                        }
                        fixes.push(self.id()?);
                        self.string()?;
                    } else if self.is_id("options") && self.is_p_at(1, "{") {
                        self.i += 1;
                        self.braces()?;
                    } else {
                        break;
                    }
                }
                Item::Violation { name, cols, code, tag, message, fixes }
            }
            "policy" if self.is_p("{") => {
                let text = self.braces()?;
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
                        self.braces()?;
                    } else {
                        self.i += 1;
                    }
                    end = self.toks[self.i - 1].2;
                }
                Item::Setting { key: d, text: self.src[start..end.max(start)].to_owned() }
            }
        })
    }

    fn head(&mut self) -> Result<Head, Error> {
        let modal = if (self.is_id("must") || self.is_id("may")) && matches!(self.peek_at(1), Tok::Id(_)) { Some(self.id()?) } else { None };
        let rel = self.id()?;
        self.p("(")?;
        let args = self.list(")", Self::term)?;
        self.p(")")?;
        Ok(Head { modal, rel, args })
    }

    fn clause(&mut self) -> Result<Item, Error> {
        let mut heads = vec![self.head()?];
        while self.is_p(",") {
            self.i += 1;
            heads.push(self.head()?);
        }
        let mut body = Vec::new();
        if self.is_p(":-") {
            self.i += 1;
            let alts = self.disjunction()?;
            body = if alts.len() == 1 { alts.into_iter().next().expect("one") } else { vec![Lit::Group { alts, neg: false }] };
        }
        let mut edits = Vec::new();
        if self.is_p("=>") {
            self.i += 1;
            edits = self.list(".", Self::term)?;
        }
        self.p(".")?;
        Ok(Item::Clause { heads, body, edits })
    }

    fn disjunction(&mut self) -> Result<Vec<Vec<Lit>>, Error> {
        let mut alts = vec![self.conjunction()?];
        while self.is_p(";") {
            self.i += 1;
            alts.push(self.conjunction()?);
        }
        Ok(alts)
    }

    fn conjunction(&mut self) -> Result<Vec<Lit>, Error> {
        let mut out = vec![self.literal()?];
        while self.is_p(",") {
            self.i += 1;
            out.push(self.literal()?);
        }
        Ok(out)
    }

    fn literal(&mut self) -> Result<Lit, Error> {
        if self.is_p("!") {
            self.i += 1;
            return Ok(match self.literal()? {
                Lit::Atom { rel, args, neg } => Lit::Atom { rel, args, neg: !neg },
                Lit::Pattern { subject, pattern, neg } => Lit::Pattern { subject, pattern, neg: !neg },
                Lit::Group { alts, neg } => Lit::Group { alts, neg: !neg },
                Lit::Cmp { .. } => return self.fail("an atom after !"),
            });
        }
        if self.is_p("(") {
            // A group `( a ; b )`, or a parenthesized term that a comparison follows: `(a - b) * 10 > o`.
            let save = self.i;
            self.i += 1;
            let group = self.disjunction().and_then(|alts| self.p(")").map(|()| alts));
            let term_follows = matches!(self.peek(), Tok::P("=" | "!=" | "<" | "<=" | ">" | ">=" | "+" | "-" | "*" | "/" | "%"));
            match group {
                Ok(alts) if !term_follows => return Ok(Lit::Group { alts, neg: false }),
                _ => self.i = save,
            }
        }
        let l = self.term()?;
        if self.is_p("~") {
            self.i += 1;
            let pattern = match self.peek().clone() {
                Tok::Tick(t) => t,
                _ => return self.fail("a backtick pattern"),
            };
            self.i += 1;
            return Ok(Lit::Pattern { subject: l, pattern, neg: false });
        }
        let op = match self.peek() {
            Tok::P(p @ ("=" | "!=" | "<" | "<=" | ">" | ">=")) => *p,
            _ => {
                return match l {
                    Term::Call { name, functor: false, args } => Ok(Lit::Atom { rel: name, args, neg: false }),
                    _ => self.fail("an atom or a comparison"),
                };
            }
        };
        self.i += 1;
        Ok(Lit::Cmp { l, op, r: self.term()? })
    }

    pub(super) fn term(&mut self) -> Result<Term, Error> {
        let mut l = self.product()?;
        while let Tok::P(op @ ("+" | "-")) = self.peek() {
            let op = *op;
            self.i += 1;
            l = Term::Bin { op, l: Box::new(l), r: Box::new(self.product()?) };
        }
        Ok(l)
    }

    fn product(&mut self) -> Result<Term, Error> {
        let mut l = self.primary()?;
        while let Tok::P(op @ ("*" | "/" | "%")) = self.peek() {
            let op = *op;
            self.i += 1;
            l = Term::Bin { op, l: Box::new(l), r: Box::new(self.primary()?) };
        }
        Ok(l)
    }

    fn aggregate(&mut self, op: String) -> Result<Term, Error> {
        let over = if self.is_p(":") { None } else { Some(Box::new(self.agg_part()?)) };
        let (mut by, mut sep) = (Vec::new(), None);
        if self.is_id("by") {
            self.i += 1;
            if self.is_p("(") {
                self.i += 1;
                by = self.list(")", Self::term)?;
                self.p(")")?;
            } else {
                by.push(self.agg_part()?);
            }
        }
        if self.is_id("sep") {
            self.i += 1;
            sep = Some(self.string()?);
        }
        self.p(":")?;
        self.p("{")?;
        let alts = self.disjunction()?;
        self.p("}")?;
        let body = if alts.len() == 1 { alts.into_iter().next().expect("one") } else { vec![Lit::Group { alts, neg: false }] };
        Ok(Term::Agg { op, over, by, sep, body })
    }

    fn primary(&mut self) -> Result<Term, Error> {
        let t = match self.peek().clone() {
            Tok::Num(n) => Term::Num(n),
            Tok::Float(f) => Term::Float(f),
            Tok::Str(s) => Term::Str(s),
            Tok::Tick(t) => Term::Tick(t),
            Tok::P("-") if matches!(self.peek_at(1), Tok::Num(_)) => {
                self.i += 1;
                let Tok::Num(n) = self.peek().clone() else { unreachable!() };
                Term::Num(-n)
            }
            Tok::P("-") if matches!(self.peek_at(1), Tok::Float(_)) => {
                self.i += 1;
                let Tok::Float(f) = self.peek().clone() else { unreachable!() };
                Term::Float(format!("-{f}"))
            }
            // `-x`, `-f(x)`: zero minus it.
            Tok::P("-") => {
                self.i += 1;
                return Ok(Term::Bin { op: "-", l: Box::new(Term::Num(0)), r: Box::new(self.primary()?) });
            }
            Tok::P("(") => {
                self.i += 1;
                let t = self.term()?;
                self.p(")")?;
                return Ok(t);
            }
            Tok::P("{") => return Ok(Term::Json(self.braces()?)),
            Tok::P("@") => {
                self.i += 1;
                let name = self.id()?;
                self.p("(")?;
                let args = self.list(")", Self::term)?;
                self.p(")")?;
                return Ok(Term::Call { name, functor: true, args });
            }
            Tok::Id(s) if s == "_" => Term::Wild,
            Tok::Id(s) if s == "count" && self.is_p_at(1, ":") => {
                self.i += 1;
                return self.aggregate(s);
            }
            // `sum strlen(v) : {…}`: an aggregate over a call, when a `:` follows it.
            Tok::Id(s) if matches!(s.as_str(), "min" | "max" | "sum" | "mean" | "join") && matches!(self.peek_at(1), Tok::Id(_)) && self.is_p_at(2, "(") => {
                let save = self.i;
                self.i += 1;
                if self.primary().is_ok() && (self.is_p(":") || self.is_id("by") || self.is_id("sep")) {
                    self.i = save + 1;
                    return self.aggregate(s);
                }
                self.i = save;
                Term::Var(s)
            }
            Tok::Id(s)
                if matches!(s.as_str(), "min" | "max" | "sum" | "mean" | "join")
                    && (self.is_p_at(2, ":") || matches!(self.peek_at(2), Tok::Id(w) if w == "by" || w == "sep") || self.is_p_at(1, "(")) =>
            {
                // `min x : {…}`, `min (a, b) : {…}` would be a call `min(a, b)` if no `:` follows; see below.
                if self.is_p_at(1, "(") {
                    return self.call_or_aggregate(s);
                }
                self.i += 1;
                return self.aggregate(s);
            }
            Tok::Id(s) if self.is_p_at(1, "(") => {
                self.i += 2;
                let args = self.list(")", Self::term)?;
                self.p(")")?;
                return Ok(Term::Call { name: s, functor: false, args });
            }
            Tok::Id(s) if self.is_p_at(1, ":") => {
                self.i += 2;
                return Ok(Term::Named(s, Box::new(self.term()?)));
            }
            Tok::Id(s) => Term::Var(s),
            _ => return self.fail("a term"),
        };
        self.i += 1;
        Ok(t)
    }

    /// An aggregate's value or key: a variable (followed by `:`, `by` or `sep`, so not an `ast` capture) or a term.
    fn agg_part(&mut self) -> Result<Term, Error> {
        if let Tok::Id(v) = self.peek().clone()
            && v != "_"
            && !self.is_p_at(1, "(")
        {
            self.i += 1;
            return Ok(Term::Var(v));
        }
        self.primary()
    }

    /// `min(a, b)` is a call unless a `:` follows the parentheses (`min (a + b) : { … }`).
    fn call_or_aggregate(&mut self, op: String) -> Result<Term, Error> {
        let save = self.i;
        self.i += 2;
        let args = self.list(")", Self::term)?;
        self.p(")")?;
        if self.is_p(":") {
            self.i = save + 1;
            return self.aggregate(op);
        }
        Ok(Term::Call { name: op, functor: false, args })
    }
}

impl Term {
    /// The variables in the term, patterns' and templates' `$X` included.
    pub(super) fn vars(&self, out: &mut Vec<String>) {
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
    pub(super) fn scope_vars(&self, out: &mut Vec<String>) {
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

    pub(super) fn vars(&self, out: &mut Vec<String>) {
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
pub(super) fn tick_vars(t: &str) -> Vec<String> {
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
