//! `.rdl` text to `.rr` text, comments and line breaks kept: how the engine's earlier dialect moved to Soufflé form
//! (`docs/rule-syntax/ALIGNMENT.md`, the wolfeel row). Each statement is rewritten in place:
//!
//! | `.rdl`                                   | `.rr`                                                      |
//! |------------------------------------------|------------------------------------------------------------|
//! | `$x`                                     | `x` (`count_` where the name is a keyword)                 |
//! | `int`, `sym`                             | `number`, `symbol`                                         |
//! | `.lattice r(k: t; v: flat)`              | `.decl r(k: t, v: flat<>)`                                 |
//! | `.decl r(k: t; v: sym) merge @f widen 8` | `.lattice f<symbol> { Lub -> @f }` and `.decl r(k: t, v: f<>) widen 8` |
//! | `.demand a, b`                           | `.pragma "magic-transform" "a, b"`                         |
//! | `.kindset k = "a" \| "b"`                | `.decl k(kind: symbol)` and `k("a"). k("b").`              |
//! | `@prefix($s, "_")`, `!@prefix(…)`        | `prefix("_", s)`, `!prefix("_", s)` (pattern first)        |
//! | `@glob($s, {"a", "b"})`                  | `(glob("a", s) ; glob("b", s))`                            |
//! | `!@glob($s, {"a", "b"})`                 | `!glob("a", s), !glob("b", s)`                             |
//! | `$s in {"a", "b"}`                       | `(s = "a" ; s = "b")`                                      |
//! | `@iequals($s, "a")`                      | `iglob("a", s)`, the pattern's glob characters escaped     |
//! | `@rx($s, re)`                            | `match("(?s).*(?:re).*", s)` (`match` is whole-string)     |
//! | `@cat($a, $b)`                           | `@dot(a, b)`                                               |
//! | `$n = count { a(…) }`                    | `n = count : { a(…) }`                                     |
//! | `ast($d, $n, { … }, X: $v)`              | `ast(d, n, { … }, X: v)`                                   |
//!
//! In comments, `$x` (a lower-case variable) becomes `x`, outside backticks, as `site/souffle.py` does.
//! `logic::rr::tests` checks that every translated program lowers to the program the `.rdl` parsed to.

use super::syntax::quote;
use crate::code_rule_dsl::logic::front::Error;
use crate::code_rule_dsl::logic::strings::is_test;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Var(String),
    Str(String),
    Int(i64),
    P(&'static str),
    Id(String),
    Eof,
}

const PUNCT: [&str; 22] = [":-", "!=", "<=", ">=", "(", ")", ",", ".", ";", ":", "=", "<", ">", "!", "{", "}", "|", "@", "*", "[", "]", "-"];

/// Names that are words of `.rr` where a term may stand.
const KEYWORDS: [&str; 12] = ["count", "sum", "min", "max", "mean", "join", "top", "by", "sep", "must", "may", "nil"];

/// The `.rdl` tokens, as `front` reads them, with their byte ranges.
fn tokens(src: &str) -> Result<Vec<(Tok, usize, usize)>, Error> {
    let bytes = src.as_bytes();
    let (mut pos, mut out) = (0, Vec::new());
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
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
        let c = bytes[pos];
        if c == b'$' && bytes.get(pos + 1).is_some_and(|&b| b.is_ascii_alphabetic() || b == b'_') {
            let end = pos + 1 + bytes[pos + 1..].iter().take_while(|&&b| ident(b)).count();
            out.push((Tok::Var(src[pos + 1..end].to_owned()), start, end));
            pos = end;
        } else if c == b'"' {
            let mut end = pos + 1;
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            if end >= bytes.len() {
                return Err(Error::Syntax(format!("unterminated string at byte {pos}")));
            }
            let value = serde_json::from_str(&src[pos..=end]).map_err(|e| Error::Syntax(format!("bad string at byte {pos}: {e}")))?;
            out.push((Tok::Str(value), start, end + 1));
            pos = end + 1;
        } else if c.is_ascii_digit() || (c == b'-' && bytes.get(pos + 1).is_some_and(u8::is_ascii_digit)) {
            let end = pos + 1 + bytes[pos + 1..].iter().take_while(|b| b.is_ascii_digit()).count();
            out.push((Tok::Int(src[pos..end].parse().map_err(|_| Error::Syntax(format!("bad integer at byte {pos}")))?), start, end));
            pos = end;
        } else if let Some(p) = PUNCT.iter().find(|p| rest.starts_with(**p)) {
            out.push((Tok::P(p), start, pos + p.len()));
            pos += p.len();
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let end = pos + bytes[pos..].iter().take_while(|&&b| ident(b)).count();
            out.push((Tok::Id(src[pos..end].to_owned()), start, end));
            pos = end;
        } else {
            return Err(Error::Syntax(format!("bad input at byte {pos}")));
        }
    }
}

/// A comment's `$x` as `x`, outside backticks.
fn comment_vars(gap: &str) -> String {
    let mut out = String::new();
    for line in gap.split_inclusive('\n') {
        let Some(at) = line.find("//") else {
            out.push_str(line);
            continue;
        };
        out.push_str(&line[..at]);
        let mut tick = false;
        let b = line[at..].as_bytes();
        let text = &line[at..];
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'`' {
                tick = !tick;
            }
            if !tick && b[i] == b'$' && b.get(i + 1).is_some_and(|c| c.is_ascii_lowercase() || *c == b'_') {
                i += 1;
                continue;
            }
            let ch = text[i..].chars().next().expect("a char");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
enum T {
    Var(String),
    Wild,
    Int(i64),
    Str(String),
    Top,
    Call(String, Vec<T>),
    Set(Vec<T>),
    /// A bare name: a kind set.
    Name(String),
}

struct Translator<'s> {
    src: &'s str,
    toks: Vec<(Tok, usize, usize)>,
    i: usize,
    out: String,
    lattices: BTreeSet<String>,
}

/// The built-in constraints of `.rr`: a relation of the same name is renamed with a trailing `_`.
pub(super) const RESERVED: [&str; 13] =
    ["glob", "iglob", "prefix", "iprefix", "suffix", "isuffix", "contains", "icontains", "word", "iword", "match", "match_all", "split_dot"];

/// A relation's `.rr` name.
pub(super) fn relation(r: &str) -> String {
    if RESERVED.contains(&r) { format!("{r}_") } else { r.to_owned() }
}

pub(super) fn var(v: &str) -> String {
    if KEYWORDS.contains(&v) { format!("{v}_") } else { v.to_owned() }
}

fn ty(t: &str) -> &str {
    match t {
        "int" => "number",
        "sym" => "symbol",
        other => other,
    }
}

impl T {
    fn print(&self) -> Result<String, Error> {
        Ok(match self {
            T::Var(v) => var(v),
            T::Wild => "_".to_owned(),
            T::Int(n) => n.to_string(),
            T::Str(s) => quote(s),
            T::Top => "top".to_owned(),
            T::Name(n) => n.clone(),
            T::Call(f, _) if is_test(f) => return Err(Error::Syntax(format!("the text test @{f} as a value: no .rr form"))),
            T::Call(f, args) => format!("@{}({})", if f == "cat" { "dot" } else { f }, list(args)?),
            T::Set(_) => return Err(Error::Syntax("a set outside a text test: no .rr form".to_owned())),
        })
    }
}

fn list(args: &[T]) -> Result<String, Error> {
    Ok(args.iter().map(T::print).collect::<Result<Vec<_>, _>>()?.join(", "))
}

/// A text test `@op(s, pattern)`, holding (`neg` false) or failing, in `.rr`.
fn test(op: &str, s: &T, pattern: &T, neg: bool) -> Result<String, Error> {
    let s = s.print()?;
    let one = |p: &T| -> Result<String, Error> {
        let T::Str(p) = p else {
            let p = p.print()?;
            return Ok(match op {
                "equals" => format!("{s} {} {p}", if neg { "!=" } else { "=" }),
                _ => format!("{}{}({p}, {s})", if neg { "!" } else { "" }, op),
            });
        };
        let bang = if neg { "!" } else { "" };
        Ok(match op {
            "equals" => format!("{s} {} {}", if neg { "!=" } else { "=" }, quote(p)),
            "iequals" => format!("{bang}iglob({}, {s})", quote(&super::print::glob_literal(p))),
            "rx" => format!("{bang}match({}, {s})", quote(&format!("(?s).*(?:{p}).*"))),
            "irx" => format!("{bang}match({}, {s})", quote(&format!("(?is).*(?:{p}).*"))),
            _ => format!("{bang}{op}({}, {s})", quote(p)),
        })
    };
    match pattern {
        T::Set(items) if items.len() == 1 => one(&items[0]),
        T::Set(items) if neg => Ok(items.iter().map(one).collect::<Result<Vec<_>, _>>()?.join(", ")),
        T::Set(items) => Ok(format!("({})", items.iter().map(one).collect::<Result<Vec<_>, _>>()?.join(" ; "))),
        p => one(p),
    }
}

impl Translator<'_> {
    fn peek(&self) -> &Tok {
        self.peek_at(0)
    }

    fn peek_at(&self, k: usize) -> &Tok {
        self.toks.get(self.i + k).map_or(&Tok::Eof, |t| &t.0)
    }

    fn is_p(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::P(q) if *q == p)
    }

    fn is_id(&self, id: &str) -> bool {
        matches!(self.peek(), Tok::Id(s) if s == id)
    }

    /// The text between the previous token and the next one: whitespace and comments.
    fn gap(&self) -> String {
        let from = self.i.checked_sub(1).map_or(0, |k| self.toks[k].2);
        let to = self.toks.get(self.i).map_or(self.src.len(), |t| t.1);
        comment_vars(&self.src[from..to])
    }

    fn fail<T>(&self, what: &str) -> Result<T, Error> {
        let at = self.toks.get(self.i).map_or(self.src.len(), |t| t.1);
        let line = self.src[..at].bytes().filter(|&b| b == b'\n').count() + 1;
        Err(Error::Syntax(format!("line {line}: expected {what}, got {:?}", self.peek())))
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

    fn var(&mut self) -> Result<String, Error> {
        match self.peek().clone() {
            Tok::Var(s) => {
                self.i += 1;
                Ok(var(&s))
            }
            _ => self.fail("a variable"),
        }
    }

    fn terms(&mut self) -> Result<Vec<T>, Error> {
        let mut out = vec![self.term()?];
        while self.is_p(",") {
            self.i += 1;
            out.push(self.term()?);
        }
        Ok(out)
    }

    fn term(&mut self) -> Result<T, Error> {
        let t = match self.peek().clone() {
            Tok::Var(v) => T::Var(v),
            Tok::Str(s) => T::Str(s),
            Tok::Int(n) => T::Int(n),
            Tok::Id(s) if s == "_" => T::Wild,
            Tok::Id(s) if s == "top" => T::Top,
            Tok::P("{") => {
                self.i += 1;
                let items = if self.is_p("}") { Vec::new() } else { self.terms()? };
                self.p("}")?;
                return Ok(T::Set(items));
            }
            Tok::P("@") => {
                self.i += 1;
                let f = self.id()?;
                self.p("(")?;
                let args = self.terms()?;
                self.p(")")?;
                return Ok(T::Call(f, args));
            }
            _ => return self.fail("a term"),
        };
        self.i += 1;
        Ok(t)
    }

    /// The source text of a balanced `{ … }`, as written.
    fn braces(&mut self) -> Result<String, Error> {
        let start = self.toks.get(self.i).map_or(self.src.len(), |t| t.1);
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

    fn atom(&mut self) -> Result<String, Error> {
        let rel = self.id()?;
        self.p("(")?;
        let mut args = Vec::new();
        loop {
            // A kind set: `inside($n, $a, set)`, `has(…)`.
            if let Tok::Id(name) = self.peek().clone()
                && name != "_"
                && name != "top"
            {
                self.i += 1;
                args.push(T::Name(name));
            } else {
                args.push(self.term()?);
            }
            if !self.is_p(",") {
                break;
            }
            self.i += 1;
        }
        self.p(")")?;
        Ok(format!("{}({})", relation(&rel), list(&args)?))
    }

    fn literal(&mut self) -> Result<String, Error> {
        if self.is_p("!") {
            self.i += 1;
            if self.is_p("@") {
                let T::Call(op, args) = self.term()? else { unreachable!("a call") };
                let [s, p] = args.as_slice() else { return self.fail("a text test of two arguments") };
                return test(&op, s, p, true);
            }
            return Ok(format!("!{}", self.atom()?));
        }
        if self.is_id("nearest") && matches!(self.peek_at(1), Tok::P("(")) {
            self.i += 2;
            let n = self.var()?;
            self.p(",")?;
            let a = self.var()?;
            self.p(",")?;
            let set = self.id()?;
            self.p(")")?;
            return Ok(format!("nearest({n}, {a}, {set})"));
        }
        if self.is_id("ast") && matches!(self.peek_at(1), Tok::P("(")) {
            self.i += 2;
            let d = self.var()?;
            self.p(",")?;
            let n = self.var()?;
            self.p(",")?;
            let matcher = self.braces()?;
            let mut out = format!("ast({d}, {n}, {matcher}");
            while self.is_p(",") {
                self.i += 1;
                let name = self.id()?;
                self.p(":")?;
                out.push_str(&format!(", {name}: {}", self.var()?));
            }
            self.p(")")?;
            return Ok(out + ")");
        }
        if matches!(self.peek(), Tok::Id(_)) {
            return self.atom();
        }
        let l = self.term()?;
        if self.is_id("in") {
            self.i += 1;
            let set = self.term()?;
            return test("equals", &l, &set, false);
        }
        let op = match self.peek() {
            Tok::P(p @ ("=" | "!=" | "<" | "<=" | ">" | ">=")) => *p,
            _ => {
                if let T::Call(f, args) = &l
                    && is_test(f)
                    && let [s, p] = args.as_slice()
                {
                    return test(f, s, p, false);
                }
                return self.fail("a comparison");
            }
        };
        self.i += 1;
        if let T::Call(f, args) = &l
            && is_test(f)
            && let [s, p] = args.as_slice()
            && op == "="
            && let Tok::Int(b @ (0 | 1)) = self.peek().clone()
        {
            self.i += 1;
            return test(f, s, p, b == 0);
        }
        if op == "="
            && let T::Var(v) = &l
            && let Tok::Id(agg) = self.peek().clone()
            && matches!(agg.as_str(), "count" | "min" | "max")
            && matches!(self.peek_at(1), Tok::P("{") | Tok::Var(_) | Tok::P(":"))
        {
            self.i += 1;
            let over = if let Tok::Var(_) = self.peek() { format!(" {}", self.var()?) } else { String::new() };
            if self.is_p(":") {
                self.i += 1;
            }
            self.p("{")?;
            let atom = self.atom()?;
            self.p("}")?;
            return Ok(format!("{} = {agg}{over} : {{ {atom} }}", var(v)));
        }
        let r = self.term()?;
        Ok(format!("{} {op} {}", l.print()?, r.print()?))
    }

    fn directive(&mut self) -> Result<(), Error> {
        let d = self.id()?;
        match d.as_str() {
            "decl" | "lattice" => {
                let name = self.id()?;
                self.p("(")?;
                let (mut cols, mut keys) = (Vec::new(), false);
                loop {
                    let c = self.id()?;
                    self.p(":")?;
                    cols.push((c, self.id()?));
                    if self.is_p(";") {
                        self.i += 1;
                        keys = true;
                    } else if self.is_p(",") {
                        self.i += 1;
                    } else {
                        break;
                    }
                }
                self.p(")")?;
                let (mut merge, mut widen) = (None, None);
                if self.is_id("merge") {
                    self.i += 1;
                    self.p("@")?;
                    merge = Some(self.id()?);
                }
                if self.is_id("widen") {
                    self.i += 1;
                    let Tok::Int(k) = self.peek().clone() else { return self.fail("a widening count") };
                    self.i += 1;
                    widen = Some(k);
                }
                let lattice = d == "lattice" || keys || merge.is_some();
                let n = cols.len();
                let mut printed = Vec::new();
                for (i, (c, t)) in cols.iter().enumerate() {
                    if lattice && i + 1 == n {
                        let column = match &merge {
                            Some(f) => {
                                if self.lattices.insert(f.clone()) {
                                    self.out.push_str(&format!(".lattice {f}<{}> {{ Lub -> @{f} }}\n", ty(t)));
                                }
                                format!("{f}<>")
                            }
                            None => format!("{t}<>"),
                        };
                        printed.push(format!("{c}: {column}"));
                    } else {
                        printed.push(format!("{c}: {}", ty(t)));
                    }
                }
                self.out.push_str(&format!(".decl {}({})", relation(&name), printed.join(", ")));
                if let Some(k) = widen {
                    self.out.push_str(&format!(" widen {k}"));
                }
            }
            "input" | "output" => {
                let mut names = vec![relation(&self.id()?)];
                while self.is_p(",") {
                    self.i += 1;
                    names.push(relation(&self.id()?));
                }
                self.out.push_str(&format!(".{d} {}", names.join(", ")));
            }
            "demand" => {
                let names = if self.is_p("*") {
                    self.i += 1;
                    vec!["*".to_owned()]
                } else {
                    let mut names = vec![relation(&self.id()?)];
                    while self.is_p(",") {
                        self.i += 1;
                        names.push(relation(&self.id()?));
                    }
                    names
                };
                self.out.push_str(&format!(".pragma \"magic-transform\" {}", quote(&names.join(", "))));
            }
            // `.pragma "key" "value"` is `.rr` already.
            "pragma" => {
                let (Tok::Str(key), Tok::Str(value)) = (self.peek().clone(), self.peek_at(1).clone()) else {
                    return self.fail("a pragma's two strings");
                };
                self.i += 2;
                self.out.push_str(&format!(".pragma {} {}", quote(&key), quote(&value)));
            }
            "kindset" => {
                let name = self.id()?;
                self.p("=")?;
                let mut kinds = Vec::new();
                loop {
                    match self.peek().clone() {
                        Tok::Str(s) => kinds.push(s),
                        _ => return self.fail("a kind"),
                    }
                    self.i += 1;
                    if !self.is_p("|") {
                        break;
                    }
                    self.i += 1;
                }
                let facts = kinds.iter().map(|k| format!("{name}({}).", quote(k))).collect::<Vec<_>>().join(" ");
                self.out.push_str(&format!(".decl {name}(kind: symbol)\n{facts}"));
            }
            "policy" => {
                let text = self.braces()?;
                self.out.push_str(&format!(".policy {text}"));
            }
            _ => return self.fail("a directive"),
        }
        Ok(())
    }

    fn statement(&mut self) -> Result<(), Error> {
        if self.is_p(".") {
            self.i += 1;
            return self.directive();
        }
        let head = self.atom()?;
        self.out.push_str(&head);
        if self.is_p(".") {
            self.i += 1;
            self.out.push('.');
            return Ok(());
        }
        self.p(":-")?;
        self.out.push_str(" :-");
        loop {
            let gap = self.gap();
            self.out.push_str(if gap.is_empty() { " " } else { &gap });
            let literal = self.literal()?;
            self.out.push_str(&literal);
            if !self.is_p(",") {
                break;
            }
            self.i += 1;
            self.out.push(',');
        }
        self.p(".")?;
        self.out.push('.');
        Ok(())
    }
}

/// The `.rr` text of an `.rdl` program.
pub(in crate::code_rule_dsl) fn translate(src: &str) -> Result<String, Error> {
    let mut t = Translator { src, toks: tokens(src)?, i: 0, out: String::new(), lattices: BTreeSet::new() };
    loop {
        let gap = t.gap();
        t.out.push_str(&gap);
        if *t.peek() == Tok::Eof {
            break;
        }
        t.statement()?;
    }
    Ok(t.out)
}
