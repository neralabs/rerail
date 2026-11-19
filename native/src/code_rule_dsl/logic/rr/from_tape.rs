//! A `.rr` file's statements, read off its tape (decision 0062). The tape is the lean parser's image
//! (`rerail_rr_syntax::tape`, decision 0140) and what the content store keeps; lowering reads the statements this
//! module gives, so a rule file's program is a function of its stored tape. The image is read where it is
//! (`tape_format::View`): each node's kind, field and span from its columns, its children by their subtree sizes,
//! with no node decoded ahead and no index of children made.
//!
//! Every statement kind and term kind of decision 0061 maps back to the form `lower` takes (`syntax::Item`,
//! `syntax::Lit`, `syntax::Term`). Texts come from the source by the nodes' spans: a string is unescaped as the lexer
//! does (`string_value`), a backtick literal loses its fences (`code_body`).

use super::Error;
use super::syntax;
use super::syntax::{Code, Fix, Head, Item, Lines, Lit, Statement, Suppress, Term};
use super::tape::{ERROR, Field, Kind};
use rerail_rr_syntax::tape_format::{CODE_EXTRA, View};

/// A `.rr` tape read in place: its source and its image's columns.
pub struct Tape<'a> {
    src: &'a str,
    lines: &'a Lines,
    v: View<'a>,
}

/// A `.rr` file as a run holds it: its text and its tape, the lean parser's image of it (decision 0140), checked
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

/// A `.violation`'s declaration: its relation and columns, its code (a constant or a column, and the codes a column
/// may hold), its tag column, its message, its fixes and its suppressions.
#[derive(Clone, Debug)]
pub struct Violation {
    pub name: String,
    pub cols: Vec<(String, String)>,
    pub code: Option<Code>,
    pub code_in: Option<String>,
    pub tag: Option<String>,
    pub message: String,
    pub fixes: Vec<Fix>,
    pub suppress: Vec<Suppress>,
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
fn op(text: &str) -> Result<&'static str, Error> {
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

    fn field(&self, n: u32) -> u16 {
        super::tape::kind_field(self.v.key(n as usize)).1
    }

    fn extra(&self, n: u32) -> bool {
        self.v.key(n as usize) >> 16 & u32::from(CODE_EXTRA) != 0
    }

    pub fn is(&self, n: u32, kind: Kind) -> bool {
        self.kind(n) == kind as u16
    }

    fn end(&self, n: u32) -> usize {
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

    fn one(&self, n: u32, field: Field) -> Result<u32, Error> {
        self.get(n, field).map_or_else(|| bad(&format!("{} has no {field:?}", super::tape::KIND_NAMES.get(usize::from(self.kind(n))).unwrap_or(&"?"))), Ok)
    }

    fn name(&self, n: u32, field: Field) -> Result<String, Error> {
        Ok(self.text(self.one(n, field)?).to_owned())
    }

    /// An anonymous child written `text`.
    fn has_token(&self, n: u32, text: &str) -> bool {
        self.kids(n).any(|c| self.kind(c) >= super::tape::FIRST_ANONYMOUS && self.text(c) == text)
    }

    fn string(&self, n: u32) -> Result<String, Error> {
        string_value(self.text(n)).map_or_else(|| bad("a string that does not read"), Ok)
    }

    /// A column named in braces: an identifier, or the braces' text as written (`raw_text`), trimmed as the parser does.
    fn brace_column(&self, n: u32) -> String {
        self.text(n).trim_start_matches('{').trim_end_matches('}').trim().to_owned()
    }

    #[cfg(any(test, feature = "test-hooks"))]
    fn statements(&self) -> Result<Vec<Statement>, Error> {
        self.kids(0).map(|s| Ok((self.statement(s)?, self.line(s)))).collect()
    }

    fn columns(&self, n: u32) -> Result<Vec<(String, String)>, Error> {
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

    /// A `.lattice`'s relation, base type and operations.
    #[allow(clippy::type_complexity)]
    pub fn lattice(&self, s: u32) -> Result<(String, String, Vec<(String, Term)>), Error> {
        let ops = self
            .all(s, Field::Operation)
            .map(|o| {
                let value = self.one(o, Field::Value)?;
                let term = if self.is(value, Kind::FunctorCall) && self.get(value, Field::Arguments).is_none() {
                    Term::Call { name: self.name(value, Field::Function)?, functor: true, args: Vec::new() }
                } else {
                    self.term(value)?
                };
                Ok((self.name(o, Field::Name)?, term))
            })
            .collect::<Result<_, Error>>()?;
        Ok((self.name(s, Field::Name)?, self.get(s, Field::Base).map_or_else(String::new, |b| self.text(b).to_owned()), ops))
    }

    /// A `.pragma`'s words.
    pub fn pragma(&self, s: u32) -> Result<Vec<String>, Error> {
        self.all(s, Field::Argument).map(|a| self.string(a)).collect()
    }

    /// A `.violation`'s declaration (`syntax::Item::Violation`'s fields).
    pub fn violation(&self, s: u32) -> Result<Violation, Error> {
        let (mut code, mut code_in, mut tag) = (None, None, None);
        for c in self.all(s, Field::Code) {
            if self.is(c, Kind::CodeColumn) {
                code = Some(Code::Column(self.brace_column(self.one(c, Field::Column)?)));
                code_in = self.get(c, Field::Codes).map(|x| self.string(x)).transpose()?;
            } else {
                code = Some(Code::Constant(self.string(c)?));
            }
        }
        if let Some(t) = self.all(s, Field::Tag).last() {
            tag = Some(self.brace_column(self.one(t, Field::Column)?));
        }
        let fixes = self
            .all(s, Field::Fix)
            .map(|f| {
                Ok(Fix {
                    name: self.name(f, Field::Relation)?,
                    safe: self.has_token(f, "safe"),
                    title: self.string(self.one(f, Field::Description)?)?,
                })
            })
            .collect::<Result<_, Error>>()?;
        let suppress = self
            .all(s, Field::Suppress)
            .map(|l| {
                Ok(Suppress {
                    tool: self.name(l, Field::Tool)?,
                    rules: self.all(l, Field::Rule).map(|r| self.string(r)).collect::<Result<_, _>>()?,
                    over: self.get(l, Field::Over).map(|o| self.brace_column(o)),
                })
            })
            .collect::<Result<_, Error>>()?;
        Ok(Violation { name: self.name(s, Field::Name)?, cols: self.columns(s)?, code, code_in, tag, message: self.string(self.one(s, Field::Message)?)?, fixes, suppress })
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

    /// A clause's heads, body and edits, in the statement parser's form, which the lowering takes.
    #[allow(clippy::type_complexity)]
    pub fn clause(&self, s: u32) -> Result<(Vec<Head>, Vec<Lit>, Vec<Term>), Error> {
        Ok((
            self.all(s, Field::Head).map(|h| self.head(h)).collect::<Result<_, _>>()?,
            self.get(s, Field::Body).map_or_else(|| Ok(Vec::new()), |b| self.body(b))?,
            self.all(s, Field::Edit).map(|e| self.term(e)).collect::<Result<_, _>>()?,
        ))
    }

    /// Statement `s` in the statement parser's form (`syntax::Item`), for the tape tests' comparison.
    #[cfg(any(test, feature = "test-hooks"))]
    fn statement(&self, s: u32) -> Result<Item, Error> {
        Ok(match self.what(s)? {
            S::Include(path) => Item::Include(path),
            S::Decl(_) => {
                let (name, cols, tail) = self.decl(s)?;
                Item::Decl { name, cols, tail }
            }
            S::Edit(name) => Item::Edit { name: name.to_owned() },
            S::Functor(name) => Item::Functor { name: name.to_owned() },
            S::Type(name) => Item::Type { name: name.to_owned() },
            S::Io { .. } => {
                let (output, names, params) = self.io(s)?;
                Item::Io { output, names, params }
            }
            S::Lattice(_) => {
                let (name, base, ops) = self.lattice(s)?;
                Item::Lattice { name, base, ops }
            }
            S::Pragma => Item::Pragma(self.pragma(s)?),
            S::Violation(_) => {
                let Violation { name, cols, code, code_in, tag, message, fixes, suppress } = self.violation(s)?;
                Item::Violation { name, cols, code, code_in, tag, message, fixes, suppress }
            }
            S::Policy => Item::Policy(self.policy(s)?),
            S::Test => {
                let (name, exactly, uses, body) = self.block(s)?;
                Item::Test { name, exactly, uses, items: body.into_iter().map(|c| Ok((self.statement(c)?, self.line(c)))).collect::<Result<_, Error>>()? }
            }
            S::Fixtures => {
                let (name, _, _, body) = self.block(s)?;
                Item::Fixtures { name, items: body.into_iter().map(|c| Ok((self.statement(c)?, self.line(c)))).collect::<Result<_, Error>>()? }
            }
            S::Setting { key, text } => Item::Setting { key: key.to_owned(), text: text.to_owned() },
            S::Clause => {
                let (heads, body, edits) = self.clause(s)?;
                Item::Clause { heads, body, edits }
            }
        })
    }

    fn head(&self, h: u32) -> Result<Head, Error> {
        Ok(Head {
            modal: self.get(h, Field::Modal).map(|m| self.text(m).to_owned()),
            rel: self.name(h, Field::Relation)?,
            args: self.arguments(self.one(h, Field::Arguments)?)?,
        })
    }

    fn arguments(&self, list: u32) -> Result<Vec<Term>, Error> {
        self.all(list, Field::Argument).map(|a| self.term(a)).collect()
    }

    /// A body or a group's: its alternatives, each a conjunction.
    fn alternatives(&self, b: u32) -> Result<Vec<Vec<Lit>>, Error> {
        if self.is(b, Kind::Disjunction) {
            self.all(b, Field::Alternative).map(|c| self.conjunction(c)).collect()
        } else {
            Ok(vec![self.conjunction(b)?])
        }
    }

    /// A clause's or an aggregate's body: one conjunction, or a group of the alternatives.
    fn body(&self, b: u32) -> Result<Vec<Lit>, Error> {
        let alts = self.alternatives(b)?;
        Ok(if alts.len() == 1 { alts.into_iter().next().expect("one") } else { vec![Lit::Group { alts, neg: false }] })
    }

    fn conjunction(&self, c: u32) -> Result<Vec<Lit>, Error> {
        self.all(c, Field::Literal).map(|l| self.literal(l)).collect()
    }

    fn literal(&self, l: u32) -> Result<Lit, Error> {
        Ok(match self.kind(l) {
            k if k == Kind::Atom as u16 => Lit::Atom { rel: self.name(l, Field::Relation)?, args: self.arguments(self.one(l, Field::Arguments)?)?, neg: false },
            k if k == Kind::Negation as u16 => match self.literal(self.one(l, Field::Operand)?)? {
                Lit::Atom { rel, args, neg } => Lit::Atom { rel, args, neg: !neg },
                Lit::Pattern { subject, pattern, selector, neg } => Lit::Pattern { subject, pattern, selector, neg: !neg },
                Lit::Group { alts, neg } => Lit::Group { alts, neg: !neg },
                Lit::Cmp { .. } => return bad("a negated comparison"),
            },
            k if k == Kind::Comparison as u16 => Lit::Cmp {
                l: self.term(self.one(l, Field::Left)?)?,
                op: op(self.text(self.one(l, Field::Operator)?))?,
                r: self.term(self.one(l, Field::Right)?)?,
            },
            k if k == Kind::PatternMatch as u16 => Lit::Pattern {
                subject: self.term(self.one(l, Field::Subject)?)?,
                pattern: code_body(self.text(self.one(l, Field::Pattern)?)).to_owned(),
                selector: self.get(l, Field::Selector).map(|s| self.string(s)).transpose()?,
                neg: false,
            },
            k if k == Kind::Group as u16 => Lit::Group { alts: self.alternatives(self.one(l, Field::Body)?)?, neg: false },
            _ => return bad("an unknown literal"),
        })
    }

    fn term(&self, t: u32) -> Result<Term, Error> {
        let compact = || self.text(t).split_whitespace().collect::<String>();
        Ok(match self.kind(t) {
            k if k == Kind::Variable as u16 => Term::Var(self.text(t).to_owned()),
            k if k == Kind::Wildcard as u16 => Term::Wild,
            k if k == Kind::Number as u16 => Term::Num(compact().parse().map_err(|_| Error::Syntax(format!("tape: bad number {:?}", self.text(t))))?),
            k if k == Kind::Float as u16 => Term::Float(compact()),
            k if k == Kind::String as u16 => Term::Str(self.string(t)?),
            k if k == Kind::CodeLiteral as u16 => Term::Tick(code_body(self.text(t)).to_owned()),
            k if k == Kind::JsonObject as u16 => Term::Json(self.text(t).to_owned()),
            k if k == Kind::Call as u16 || k == Kind::FunctorCall as u16 => Term::Call {
                name: self.name(t, Field::Function)?,
                functor: k == Kind::FunctorCall as u16,
                args: self.get(t, Field::Arguments).map_or_else(|| Ok(Vec::new()), |a| self.arguments(a))?,
            },
            k if k == Kind::BinaryExpression as u16 => Term::Bin {
                op: op(self.text(self.one(t, Field::Operator)?))?,
                l: Box::new(self.term(self.one(t, Field::Left)?)?),
                r: Box::new(self.term(self.one(t, Field::Right)?)?),
            },
            k if k == Kind::UnaryExpression as u16 => Term::Bin { op: "-", l: Box::new(Term::Num(0)), r: Box::new(self.term(self.one(t, Field::Operand)?)?) },
            k if k == Kind::ParenthesizedExpression as u16 => self.term(self.one(t, Field::Expression)?)?,
            k if k == Kind::Aggregate as u16 => Term::Agg {
                // `count` is implied: the aggregate with a body alone.
                op: self.get(t, Field::Operator).map_or("count", |o| self.text(o)).to_owned(),
                over: self.get(t, Field::Value).map(|v| self.term(v).map(Box::new)).transpose()?,
                by: self.all(t, Field::Key).map(|k| self.term(k)).collect::<Result<_, _>>()?,
                sep: self.get(t, Field::Separator).map(|s| self.string(s)).transpose()?,
                body: self.body(self.one(t, Field::Body)?)?,
            },
            k if k == Kind::NamedArgument as u16 => Term::Named(self.name(t, Field::Name)?, Box::new(self.term(self.one(t, Field::Value)?)?)),
            _ => return bad("an unknown term"),
        })
    }
}

/// The statements of `src` as the statement parser's form (`syntax::Item`): what the tape tests compare with the
/// parser's statements. The engine reads a file's statements off its tape (`RrFile`).
#[cfg(any(test, feature = "test-hooks"))]
pub fn statements(src: &str) -> Result<Vec<Statement>, Error> {
    let file = RrFile::read(src)?;
    let t = file.tape();
    if t.v.is_empty() {
        return Ok(Vec::new());
    }
    t.statements()
}

/// The first error a `.rr` file's parse gives, as running it would report it: its line and message
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

/// A `.rr` string literal's value, as the rule engine reads it (`"a\nb"` is `a`, newline, `b`); `None` for text
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
