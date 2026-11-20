//! A `.rerail` file's statements, read off its tape (decision 0062). The tape is the lean parser's image
//! (`rerail_rr_syntax::tape`, decision 0140) and what the content store keeps; lowering reads the statements this
//! module gives, so a rule file's program is a function of its stored tape. The image is read where it is
//! (`tape_format::View`): each node's kind, field and span from its columns, its children by their subtree sizes,
//! with no node decoded ahead and no index of children made.
//!
//! A statement is a node (`Stmt`), read by what it is (`S`) and its readers here; a clause is read by `clause::Reads`.
//! No statement or term is made into another form (decision 0276, stage 6 A). Texts come from the source by the
//! nodes' spans: a string is unescaped as the lexer does (`string_value`), a backtick literal loses its fences
//! (`code_body`).

use super::Error;
use super::syntax;
use super::syntax::Lines;
use super::tape::{ERROR, Field, Kind};
use rerail_rr_syntax::tape_format::{CODE_EXTRA, View};

/// A `.rerail` tape read in place: its source and its image's columns.
pub struct Tape<'a> {
    src: &'a str,
    lines: &'a Lines,
    v: View<'a>,
}

/// A `.rerail` file as a run holds it: its text and its tape, the lean parser's image of it (decision 0140), checked
/// once when read; and the text's line starts. A statement of it is a node of the tape (`Stmt`), read in place.
pub struct RrFile {
    src: std::sync::Arc<str>,
    image: Box<[u8]>,
    keys: Box<[u32]>,
    lines: Lines,
}

impl std::fmt::Debug for RrFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RrFile({} bytes)", self.src.len())
    }
}

impl RrFile {
    /// `src` and its tape. A tape with an ERROR gives the error running the file gives (`syntax_error`).
    pub fn read(src: impl Into<std::sync::Arc<str>>) -> Result<std::sync::Arc<RrFile>, Error> {
        let src: std::sync::Arc<str> = src.into();
        let image = rerail_rr_syntax::tape(&src).into_boxed_slice();
        let keys = View::new(&image).map_err(|e| Error::Syntax(format!("tape: {e}")))?.into_keys();
        let lines = Lines::of(&src);
        let file = RrFile { src, image, keys, lines };
        if file.tape().has_error() {
            let (_, message) = syntax_error(&file.src).unwrap_or((0, "a statement that does not parse".to_owned()));
            return Err(Error::Syntax(message));
        }
        Ok(std::sync::Arc::new(file))
    }

    /// Its text.
    pub fn src(&self) -> &str {
        &self.src
    }

    /// Its tape, read in place.
    pub fn tape(&self) -> Tape<'_> {
        Tape { src: &self.src, lines: &self.lines, v: View::with_keys(&self.image, &self.keys) }
    }

    /// Its statements, in order.
    pub fn statements(self: &std::sync::Arc<Self>) -> Vec<Stmt> {
        let t = self.tape();
        if t.v.is_empty() {
            return Vec::new();
        }
        t.kids(0).map(|node| Stmt { file: std::sync::Arc::clone(self), node }).collect()
    }
}

/// A statement of a program: a node of its file's tape. A position, not a copy: what it says is read off the tape
/// (`Stmt::read`, `Stmt::tape`) where it is needed.
#[derive(Clone, Debug)]
pub struct Stmt {
    file: std::sync::Arc<RrFile>,
    node: u32,
}

/// What a statement is, read off its node (`Stmt::read`): its kind, with the names its readers match on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S<'a> {
    /// `#include "path"`: the path, unescaped.
    Include(String),
    /// `.decl r(…) …`: its relation.
    Decl(&'a str),
    Edit(&'a str),
    Functor(&'a str),
    Type(&'a str),
    /// `.input`/`.output` (`output`).
    Io { output: bool },
    Lattice(&'a str),
    Pragma,
    /// `.violation R(…) …`: its relation.
    Violation(&'a str),
    Policy,
    Test,
    Fixtures,
    /// `.key text`.
    Setting { key: &'a str, text: &'a str },
    /// A rule or facts: its heads, body and edits are read off the clause (`Tape::heads`, `Tape::clause`).
    Clause,
}

impl Stmt {
    /// Statement `node` of `file`: a statement of its tape, or one inside a block (`Tape::block`).
    pub fn at(file: &std::sync::Arc<RrFile>, node: u32) -> Stmt {
        Stmt { file: std::sync::Arc::clone(file), node }
    }

    /// The tape and the statement's node in it.
    pub fn tape(&self) -> (Tape<'_>, u32) {
        (self.file.tape(), self.node)
    }

    /// Its file.
    pub fn file(&self) -> &std::sync::Arc<RrFile> {
        &self.file
    }

    /// Its node in its file's tape.
    pub fn node(&self) -> u32 {
        self.node
    }

    /// Its line, from 1.
    pub fn line(&self) -> usize {
        self.file.tape().line(self.node)
    }

    /// Its text, as written.
    pub fn text(&self) -> &str {
        self.file.tape().text(self.node)
    }

    /// What it is.
    pub fn read(&self) -> Result<S<'_>, Error> {
        let (t, s) = (self.file.tape(), self.node);
        t.what(s)
    }

    /// What it is; a statement that does not read is none of the kinds a reader matches.
    pub fn what(&self) -> Option<S<'_>> {
        self.read().ok()
    }

    /// The statement's heads' relations, for a clause; none for another statement.
    pub fn heads(&self) -> Vec<&str> {
        let (t, s) = (self.file.tape(), self.node);
        if !t.is(s, Kind::Clause) {
            return Vec::new();
        }
        t.all(s, Field::Head).filter_map(|h| t.get(h, Field::Relation).map(|r| t.text(r))).collect()
    }

    /// Whether it is a clause with a body.
    pub fn has_body(&self) -> bool {
        let (t, s) = (self.file.tape(), self.node);
        t.is(s, Kind::Clause) && t.get(s, Field::Body).is_some()
    }

    /// Whether it is a clause ending in `=> edits` (a fix's).
    pub fn has_edits(&self) -> bool {
        let (t, s) = (self.file.tape(), self.node);
        t.is(s, Kind::Clause) && t.get(s, Field::Edit).is_some()
    }
}

/// An operator's text, as the parser holds it.
pub fn op(text: &str) -> Result<&'static str, Error> {
    const OPS: [&str; 11] = ["=", "!=", "<", "<=", ">", ">=", "+", "-", "*", "/", "%"];
    OPS.iter().find(|o| **o == text).copied().ok_or_else(|| Error::Syntax(format!("tape: unknown operator {text:?}")))
}

fn bad<T>(what: &str) -> Result<T, Error> {
    Err(Error::Syntax(format!("tape: {what}")))
}

impl<'a> Tape<'a> {
    /// `image` (the lean parser's tape of `src`) over `src`.

    pub fn kind(&self, n: u32) -> u16 {
        super::tape::kind_field(self.v.key(n as usize)).0
    }

    pub fn field(&self, n: u32) -> u16 {
        super::tape::kind_field(self.v.key(n as usize)).1
    }

    pub fn extra(&self, n: u32) -> bool {
        self.v.key(n as usize) >> 16 & u32::from(CODE_EXTRA) != 0
    }

    /// How many nodes node `n`'s subtree holds, `n` included.
    pub fn size(&self, n: u32) -> u32 {
        self.v.size(n as usize) as u32
    }

    pub fn is(&self, n: u32, kind: Kind) -> bool {
        self.kind(n) == kind as u16
    }

    pub fn end(&self, n: u32) -> usize {
        self.v.end(n as usize) as usize
    }

    pub fn text(&self, n: u32) -> &'a str {
        &self.src[self.v.start(n as usize) as usize..self.end(n)]
    }

    pub fn line(&self, n: u32) -> usize {
        self.lines.line(self.v.start(n as usize) as usize)
    }

    /// `n`'s children in order, extras (comments) left out.
    pub fn kids(&self, n: u32) -> impl Iterator<Item = u32> + '_ {
        self.v.children(n as usize).map(|c| c as u32).filter(move |&c| !self.extra(c))
    }

    /// The names node `n`'s subtree reads, `n` included: each relation and function named (an atom's, a head's, a
    /// call's), each variable, and each identifier in a string, a JSON object or a backtick literal that is not a
    /// pattern's or a selector's (a relation may be an argument: `nearest(n, p, block_node)`, a set of nodes).
    pub fn names_in(&self, n: u32, out: &mut std::collections::BTreeSet<String>) {
        for i in n..n + self.v.size(n as usize) as u32 {
            let (kind, field) = (self.kind(i), self.field(i));
            if self.extra(i) || field == Field::Pattern as u16 || field == Field::Selector as u16 {
                continue;
            }
            if field == Field::Relation as u16 || field == Field::Function as u16 || kind == Kind::Variable as u16 {
                out.insert(self.text(i).to_owned());
            } else if kind == Kind::String as u16 || kind == Kind::JsonObject as u16 || kind == Kind::CodeLiteral as u16 {
                let text = self.text(i);
                out.extend(text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| w.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')).map(str::to_owned));
            }
        }
    }

    /// Whether a node of the tape is an ERROR: a statement that does not parse.
    fn has_error(&self) -> bool {
        (0..self.v.len() as u32).any(|i| self.kind(i) == ERROR)
    }

    pub fn all(&self, n: u32, field: Field) -> impl Iterator<Item = u32> + '_ {
        self.kids(n).filter(move |&c| self.field(c) == field as u16)
    }

    pub fn get(&self, n: u32, field: Field) -> Option<u32> {
        self.all(n, field).next()
    }

    pub fn one(&self, n: u32, field: Field) -> Result<u32, Error> {
        self.get(n, field).map_or_else(|| bad(&format!("{} has no {field:?}", super::tape::KIND_NAMES.get(usize::from(self.kind(n))).unwrap_or(&"?"))), Ok)
    }

    pub fn name(&self, n: u32, field: Field) -> Result<String, Error> {
        Ok(self.text(self.one(n, field)?).to_owned())
    }

    /// An anonymous child written `text`.
    pub fn has_token(&self, n: u32, text: &str) -> bool {
        self.kids(n).any(|c| self.kind(c) >= super::tape::FIRST_ANONYMOUS && self.text(c) == text)
    }

    pub fn string(&self, n: u32) -> Result<String, Error> {
        string_value(self.text(n)).map_or_else(|| bad("a string that does not read"), Ok)
    }

    /// A column named in braces: an identifier, or the braces' text as written (`raw_text`), trimmed as the parser does.
    /// A `{column}`'s name.
    pub fn brace_column(&self, n: u32) -> String {
        self.text(n).trim_start_matches('{').trim_end_matches('}').trim().to_owned()
    }

    /// A declaration's columns: (name, type).
    pub fn columns(&self, n: u32) -> Result<Vec<(String, String)>, Error> {
        let list = self.one(n, Field::Columns)?;
        self.all(list, Field::Column).map(|c| Ok((self.name(c, Field::Name)?, self.ty(self.one(c, Field::Type)?)?))).collect()
    }

    fn ty(&self, n: u32) -> Result<String, Error> {
        let name = self.name(n, Field::Name)?;
        // `T<a>`: the argument says it (its `<` and `>` are implied); `T<>` stores them.
        if let Some(a) = self.get(n, Field::Argument) {
            return Ok(format!("{name}<{}>", self.text(a)));
        }
        Ok(if self.has_token(n, "<") { format!("{name}<>") } else { name })
    }

    /// What statement `s` is (`S`).
    pub fn what(&self, s: u32) -> Result<S<'a>, Error> {
        let k = self.kind(s);
        if k == ERROR {
            return bad("a statement that does not parse");
        }
        let named = |f| self.one(s, f).map(|n| self.text(n));
        Ok(match k {
            k if k == Kind::Include as u16 => S::Include(self.string(self.one(s, Field::Path)?)?),
            k if k == Kind::Declaration as u16 => S::Decl(named(Field::Name)?),
            k if k == Kind::EditDeclaration as u16 => S::Edit(named(Field::Name)?),
            k if k == Kind::FunctorDeclaration as u16 => S::Functor(named(Field::Name)?),
            k if k == Kind::TypeDeclaration as u16 => S::Type(named(Field::Name)?),
            k if k == Kind::IoDirective as u16 => S::Io { output: self.has_token(s, ".output") },
            k if k == Kind::LatticeDeclaration as u16 => S::Lattice(named(Field::Name)?),
            k if k == Kind::Pragma as u16 => S::Pragma,
            k if k == Kind::ViolationDeclaration as u16 => S::Violation(named(Field::Name)?),
            k if k == Kind::PolicyDocument as u16 => S::Policy,
            k if k == Kind::TestBlock as u16 => S::Test,
            k if k == Kind::FixturesBlock as u16 => S::Fixtures,
            k if k == Kind::Setting as u16 => {
                let (key, text) = self.setting(s)?;
                S::Setting { key, text }
            }
            k if k == Kind::Clause as u16 => S::Clause,
            _ => return bad("an unknown statement"),
        })
    }

    /// A `.decl`'s relation, columns and the words after them on its line (`widen 64`, `must, may`).
    pub fn decl(&self, s: u32) -> Result<(String, Vec<(String, String)>, Vec<String>), Error> {
        // The words after the columns, as `rest_of_line` reads them.
        let list = self.one(s, Field::Columns)?;
        let after = self.kids(s).skip_while(|&c| c != list).skip(1);
        let tail = after
            .map(|c| {
                Ok(match self.kind(c) {
                    k if k == Kind::String as u16 => self.string(c)?,
                    k if k == Kind::Number as u16 => self.text(c).parse::<i64>().map_or_else(|_| self.text(c).to_owned(), |n| n.to_string()),
                    _ => self.text(c).to_owned(),
                })
            })
            .collect::<Result<_, Error>>()?;
        Ok((self.name(s, Field::Name)?, self.columns(s)?, tail))
    }

    /// An `.input`/`.output`'s: whether an output, the relations, and each one's parameters as written.
    #[allow(clippy::type_complexity)]
    pub fn io(&self, s: u32) -> Result<(bool, Vec<String>, Vec<(String, String)>), Error> {
        let output = self.has_token(s, ".output");
        let (mut names, mut params) = (Vec::new(), Vec::new());
        for r in self.all(s, Field::Relation) {
            let name = self.name(r, Field::Name)?;
            // `r(…)`: the parameters' text, from the byte after the name's `(` to its `)`. The parentheses
            // around parameters are implied; `r()` stores them.
            let open = self.end(self.one(r, Field::Name)?);
            if let Some(p) = self.get(r, Field::Parameters) {
                let from = self.src[open..].find('(').map_or(open, |k| open + k + 1);
                let to = self.src[..self.end(r)].rfind(')').unwrap_or(self.end(p));
                params.push((name.clone(), self.src[from..to].to_owned()));
            } else if self.has_token(r, "(") {
                params.push((name.clone(), String::new()));
            }
            names.push(name);
        }
        Ok((output, names, params))
    }

    /// A `.lattice`'s relation, base type and operations: each one's name and its value's node.
    #[allow(clippy::type_complexity)]
    pub fn lattice(&self, s: u32) -> Result<(String, String, Vec<(String, u32)>), Error> {
        let ops = self.all(s, Field::Operation).map(|o| Ok((self.name(o, Field::Name)?, self.one(o, Field::Value)?))).collect::<Result<_, Error>>()?;
        Ok((self.name(s, Field::Name)?, self.get(s, Field::Base).map_or_else(String::new, |b| self.text(b).to_owned()), ops))
    }

    /// A `.pragma`'s words.
    pub fn pragma(&self, s: u32) -> Result<Vec<String>, Error> {
        self.all(s, Field::Argument).map(|a| self.string(a)).collect()
    }

    /// A `.violation`'s name, columns and message. Its code, tag, fixes and suppressions are its nodes' own fields
    /// (`Field::Code`, `Tag`, `Fix`, `Suppress`), read where they are.
    pub fn violation(&self, s: u32) -> Result<(String, Vec<(String, String)>, String), Error> {
        Ok((self.name(s, Field::Name)?, self.columns(s)?, self.string(self.one(s, Field::Message)?)?))
    }

    /// The names of a `.violation`'s fixes.
    pub fn fixes(&self, s: u32) -> Vec<&'a str> {
        self.all(s, Field::Fix).filter_map(|f| self.get(f, Field::Relation).map(|r| self.text(r))).collect()
    }

    /// A `.policy` document.
    pub fn policy(&self, s: u32) -> Result<serde_json::Value, Error> {
        let text = self.text(self.one(s, Field::Document)?);
        serde_json::from_str(text).map_err(|e| Error::Syntax(format!("line {}: bad .policy document: {e}", self.line(s))))
    }

    /// A setting's key and text (`.key text`).
    pub fn setting(&self, s: u32) -> Result<(&'a str, &'a str), Error> {
        let name = self.one(s, Field::Name)?;
        let first = self.kids(s).skip_while(|&c| c != name).nth(1);
        let end = self.end(s);
        let text = first.map_or("", |f| &self.src[self.v.start(f as usize) as usize..end]);
        Ok((self.text(name), text))
    }

    /// A `.test` block's name, `exactly`, what it `uses` and its statements' nodes; a `.fixtures` block's name and
    /// statements (`exactly` false, `uses` none).
    #[allow(clippy::type_complexity)]
    pub fn block(&self, s: u32) -> Result<(String, bool, Vec<String>, Vec<u32>), Error> {
        Ok((
            self.name(s, Field::Name)?,
            self.is(s, Kind::TestBlock) && self.get(s, Field::Modifier).is_some(),
            if self.is(s, Kind::TestBlock) { self.all(s, Field::Argument).map(|c| self.text(c).to_owned()).collect() } else { Vec::new() },
            self.all(s, Field::Body).collect(),
        ))
    }
}

/// The first error a `.rerail` file's parse gives, as running it would report it: its line and message
/// (`syntax_error/3`). `None` when it parses.
pub fn syntax_error(src: &str) -> Option<(usize, String)> {
    let error = match syntax::Parser::new(src).and_then(|mut parser| parser.statements()) {
        Ok(_) => return None,
        Err(error) => error,
    };
    let mut parser = syntax::Parser::taped(src);
    let _ = parser.statements();
    let at = parser.into_tape().1.map_or(0, |(at, _)| at);
    let rerail_rr_syntax::Error::Syntax(message) = error;
    Some((syntax::line_of(src, at), message))
}

/// A `.rerail` string literal's value, as the rule engine reads it (`"a\nb"` is `a`, newline, `b`); `None` for text
/// that is no whole string literal.
pub fn string_value(text: &str) -> Option<String> {
    match syntax::tokens(text).ok()?.as_slice() {
        [(syntax::Tok::Str(value), 0, end)] if *end == text.len() => Some(value.to_string()),
        _ => None,
    }
}

/// A backtick literal's body, without its fences (and, for a fence of two or more, the one space inside each).
pub fn code_body(text: &str) -> &str {
    let n = text.bytes().take_while(|&b| b == b'`').count();
    let body = text.get(n..text.len().saturating_sub(n)).unwrap_or("");
    if n > 1 { body.strip_prefix(' ').unwrap_or(body).strip_suffix(' ').unwrap_or(body) } else { body }
}
