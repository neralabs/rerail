//! A `.rerail` file's statements, read off its tape (decision 0062). The tape is the one every document has: the
//! `.rerail` parser's (`rerail_rr_syntax::lean`, decision 0140), written in the run's format as the parse goes
//! (`TapeImage::encode_rr`), and what the content store keeps. Lowering reads the statements this module gives, so a
//! rule file's program is a function of its tape. The tape is read where it is, through the tape API every reader
//! uses (`rerail_tape_format::tape::Tape`, decision 0293): each node's code, span and subtree from its columns, its
//! children by their subtree ends, a byte's line from the tape's newline index, with no node decoded ahead and no
//! index of children made. A 16z tape's blocks are decoded into its batch's cache: a file is kept for the process
//! by itself (`rr::read_shared`), so it is read in a batch of its own (`TapeDoc::alone`'s).
//!
//! A statement is a node (`Stmt`), read by what it is (`S`) and its readers here; a clause is read by `clause::Reads`.
//! No statement or term is made into another form (decision 0276, stage 6 A). Texts come from the source by the
//! nodes' spans: a string is unescaped as the lexer does (`string_value`), a backtick literal loses its fences
//! (`code_body`).

use rerail_blocks::vector::Vector;
use super::Error;
use super::syntax;
use super::tape::{ERROR, Field, Kind};
use crate::code_rule_dsl::tape::formats::DocTape;
use rerail_rr_syntax::tape_format::tape::{AnyTape, EntryRef, Tape as _};
use rerail_rr_syntax::tape_format::{CODE_ERROR, CODE_EXTRA};

/// A `.rerail` tape read in place: its source and its reader.
pub struct Tape<'a> {
    src: &'a str,
    t: AnyTape<'a>,
}

/// A `.rerail` file as a run holds it: its text and its tape, as any document holds them (`DocTape`: the image, its
/// reader made once, and its batch's cache for a 16z tape), checked once when read. A statement of it is a node of
/// the tape (`Stmt`), read in place.
pub struct RrFile {
    doc: DocTape,
}

impl std::fmt::Debug for RrFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RrFile({} bytes)", self.doc.source().len())
    }
}

impl RrFile {
    /// `src` and its tape. A tape with an ERROR gives the error running the file gives (`syntax_error`).
    pub fn read(src: impl Into<String>) -> Result<std::sync::Arc<RrFile>, Error> {
        let file = RrFile::parsed(src)?;
        if file.tape().has_error(file.doc.image_arc().faults()) {
            let (_, message) = syntax_error(file.src()).unwrap_or((0, "a statement that does not parse".to_owned()));
            return Err(Error::Syntax(message));
        }
        Ok(std::sync::Arc::new(file))
    }

    /// `src` and its tape, ERRORs and all: a reader that reads a file that may not parse (`rr::message_masked`).
    pub fn parsed(src: impl Into<String>) -> Result<RrFile, Error> {
        use crate::code_rule_dsl::tape::{Grammar, TapeImage};
        let src = std::sync::Arc::new(src.into());
        let grammar = Grammar::embedded(crate::code_rule_dsl::language::GrammarDialect::Rr).map_err(Error::Syntax)?;
        let image = TapeImage::encode_rr(&grammar, &src, &|| false).map_err(|e| Error::Syntax(format!("tape: {e}")))?;
        // Its own batch's cache: the file is read alone (`TapeDoc::alone`'s).
        let cache = image.own_cache().map(std::sync::Arc::new);
        Ok(RrFile { doc: DocTape::in_batch(std::sync::Arc::new(image), src, cache) })
    }

    /// Its text.
    pub fn src(&self) -> &str {
        self.doc.source()
    }

    /// Its tape, read in place.
    pub fn tape(&self) -> Tape<'_> {
        Tape { src: self.doc.source(), t: *self.doc.reader() }
    }

    /// Its statements, in order.
    pub fn statements(self: &std::sync::Arc<Self>) -> Statements {
        let t = self.tape();
        if t.t.is_empty() {
            return Statements::default();
        }
        Statements { files: vec![std::sync::Arc::clone(self)], at: t.kids(0).map(|node| [0, node]).collect() }
    }
}

/// A statement of a program: a node of its file's tape. A position, not a copy: what it says is read off the tape
/// (`Stmt::read`, `Stmt::tape`) where it is needed.
#[derive(Clone, Debug)]
pub struct Stmt {
    file: std::sync::Arc<RrFile>,
    node: u32,
}

/// A program's statements, in its order: each a node of one of its files' tapes (`rr::Files`, its includes resolved,
/// its own after). The list is a column in a block (`Vector`: each statement its file's index and its node), read in
/// place; the files are held once each. A statement read off it (`Stmt`) is a position, made as it is read.
#[derive(Clone, Default)]
pub struct Statements {
    files: Vec<std::sync::Arc<RrFile>>,
    at: Vector<[u32; 2]>,
}

impl std::fmt::Debug for Statements {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Statements({} of {} files)", self.at.len(), self.files.len())
    }
}

impl Statements {
    pub fn len(&self) -> usize {
        self.at.len()
    }

    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
    }

    /// Statement `i`.
    pub fn get(&self, i: usize) -> Option<Stmt> {
        self.at.as_slice().get(i).map(|&[f, node]| Stmt { file: std::sync::Arc::clone(&self.files[f as usize]), node })
    }

    pub fn first(&self) -> Option<Stmt> {
        self.get(0)
    }

    /// Its statements in order, each read off the column as it is reached.
    pub fn iter(&self) -> impl Iterator<Item = Stmt> + '_ {
        (0..self.len()).map(|i| self.get(i).expect("a statement of the list"))
    }

    /// Its statements in order, each with its file as the list holds it: a reader that keeps what a statement's tape
    /// reads (a clause's names) borrows the list's file, not the statement read off it.
    pub fn places(&self) -> impl Iterator<Item = (&std::sync::Arc<RrFile>, Stmt)> + '_ {
        self.at.as_slice().iter().map(|&[f, node]| (&self.files[f as usize], Stmt { file: std::sync::Arc::clone(&self.files[f as usize]), node }))
    }

    /// Those `keep` keeps, in order, over the same files.
    pub fn filtered(&self, mut keep: impl FnMut(&Stmt) -> bool) -> Statements {
        let at = self.at.as_slice().iter().copied().filter(|&[f, node]| keep(&Stmt { file: std::sync::Arc::clone(&self.files[f as usize]), node })).collect();
        Statements { files: self.files.clone(), at }
    }

    /// `s` after the list's statements: its file held once (the last one's first, as a list is made file by file).
    pub fn push(&mut self, s: Stmt) {
        let f = match self.files.iter().rposition(|f| std::sync::Arc::ptr_eq(f, &s.file)) {
            Some(f) => f,
            None => {
                self.files.push(s.file);
                self.files.len() - 1
            }
        };
        self.at.push([f as u32, s.node]);
    }
}

impl FromIterator<Stmt> for Statements {
    fn from_iter<I: IntoIterator<Item = Stmt>>(items: I) -> Self {
        let mut out = Statements::default();
        out.extend(items);
        out
    }
}

impl Extend<Stmt> for Statements {
    fn extend<I: IntoIterator<Item = Stmt>>(&mut self, items: I) {
        items.into_iter().for_each(|s| self.push(s));
    }
}

impl<'a> IntoIterator for &'a Statements {
    type Item = Stmt;
    type IntoIter = Box<dyn Iterator<Item = Stmt> + 'a>;
    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

impl IntoIterator for Statements {
    type Item = Stmt;
    type IntoIter = Owned;
    fn into_iter(self) -> Owned {
        Owned { list: self, next: 0 }
    }
}

/// A list's statements, the list held (`Statements::into_iter`).
pub struct Owned {
    list: Statements,
    next: usize,
}

impl Iterator for Owned {
    type Item = Stmt;
    fn next(&mut self) -> Option<Stmt> {
        let s = self.list.get(self.next)?;
        self.next += 1;
        Some(s)
    }
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
    /// Code `code`, as the language's table (or format 15's own) holds it: the parser's kind, field and flags.
    fn entry(&self, code: u32) -> EntryRef<'_> {
        self.t.table().entry(code)
    }

    /// A code's kind and field: ERROR (no field) for a node flagged as one (`tape::kind_field`).
    fn kind_field(&self, code: u32) -> (u16, u16) {
        let e = self.entry(code);
        if e.flags & CODE_ERROR != 0 { (ERROR, 0) } else { (e.kind, e.field) }
    }

    pub fn kind(&self, n: u32) -> u16 {
        self.kind_field(self.t.code(n as usize)).0
    }

    pub fn field(&self, n: u32) -> u16 {
        self.kind_field(self.t.code(n as usize)).1
    }

    pub fn extra(&self, n: u32) -> bool {
        self.entry(self.t.code(n as usize)).flags & CODE_EXTRA != 0
    }

    /// How many nodes node `n`'s subtree holds, `n` included.
    pub fn size(&self, n: u32) -> u32 {
        (self.t.subtree_end(n as usize) - n as usize) as u32
    }

    pub fn is(&self, n: u32, kind: Kind) -> bool {
        self.kind(n) == kind as u16
    }

    /// How many nodes the tape holds.
    pub fn len(&self) -> u32 {
        self.t.len() as u32
    }

    pub fn start(&self, n: u32) -> usize {
        self.t.start(n as usize) as usize
    }

    pub fn end(&self, n: u32) -> usize {
        self.t.end(n as usize) as usize
    }

    pub fn text(&self, n: u32) -> &'a str {
        let (start, end) = self.t.span(n as usize);
        &self.src[start as usize..end as usize]
    }

    /// Its line, from 1: the newlines before its first byte, by the tape's newline index. Format 15 holds none (a run
    /// that stores it, a measurement's), so there they are counted.
    pub fn line(&self, n: u32) -> usize {
        let x = self.start(n);
        1 + match self.t {
            AnyTape::F16(t) => t.line(x as u32) as usize,
            AnyTape::F16Z(t) => t.line(x as u32) as usize,
            AnyTape::F15(_) => self.src.as_bytes()[..x].iter().filter(|&&b| b == b'\n').count(),
        }
    }

    /// `n`'s children in order, each with its code, extras (comments) left out: a child's code and subtree end are
    /// one read (`code_and_end`).
    fn coded_kids(&self, n: u32) -> impl Iterator<Item = (u32, u32)> + '_ {
        let end = self.t.subtree_end(n as usize);
        let mut c = n as usize + 1;
        std::iter::from_fn(move || {
            while c < end {
                let (at, (code, next)) = (c, self.t.code_and_end(c));
                c = next;
                if self.entry(code).flags & CODE_EXTRA == 0 {
                    return Some((at as u32, code));
                }
            }
            None
        })
    }

    /// `n`'s children in order, extras (comments) left out.
    pub fn kids(&self, n: u32) -> impl Iterator<Item = u32> + '_ {
        self.coded_kids(n).map(|(c, _)| c)
    }

    /// The names node `n`'s subtree reads, `n` included: each relation and function named (an atom's, a head's, a
    /// call's), each variable, and each identifier in a string, a JSON object or a backtick literal that is not a
    /// pattern's or a selector's (a relation may be an argument: `nearest(n, p, block_node)`, a set of nodes).
    pub fn names_in(&self, n: u32, out: &mut std::collections::BTreeSet<String>) {
        for i in n..n + self.size(n) {
            let code = self.t.code(i as usize);
            let (kind, field) = self.kind_field(code);
            if self.entry(code).flags & CODE_EXTRA != 0 || field == Field::Pattern as u16 || field == Field::Selector as u16 {
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

    /// Whether a node of the tape is an ERROR: a statement that does not parse. Format 16's writer lists its faults
    /// in the tape's home as it writes (`TapeImage::faults`), so only those are read; format 15's are looked for.
    fn has_error(&self, faults: Option<&[u32]>) -> bool {
        match faults {
            Some(faults) => faults.iter().any(|&i| self.kind(i) == ERROR),
            None => (0..self.t.len() as u32).any(|i| self.kind(i) == ERROR),
        }
    }

    pub fn all(&self, n: u32, field: Field) -> impl Iterator<Item = u32> + '_ {
        self.coded_kids(n).filter(move |&(_, code)| self.kind_field(code).1 == field as u16).map(|(c, _)| c)
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
        self.coded_kids(n).any(|(c, code)| self.kind_field(code).0 >= super::tape::FIRST_ANONYMOUS && self.text(c) == text)
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
        let text = first.map_or("", |f| &self.src[self.start(f)..end]);
        Ok((self.text(name), text))
    }

    /// A `.test` block's name, `exactly`, what it `uses` and its statements' nodes; a `.fixtures` block's name and
    /// statements (`exactly` false, `uses` none).
    #[allow(clippy::type_complexity)]
    pub fn block(&self, s: u32) -> Result<(String, bool, Vec<String>, Vector<u32>), Error> {
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
