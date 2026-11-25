//! A `.rerail` file's tokens classed for an editor (`rerail lsp`'s semantic tokens), and the statements that do not
//! parse: what the engine's own lexer reads, each token given the class its place in the taped parse's tree says.
//!
//! The tree is named-only (decision 0161): a token the grammar implies (`.decl`'s word, `:-`, `code` in a
//! `.violation`) has no leaf, so a word without one is a keyword, and punctuation is classed by its text. Inside a
//! statement that does not parse (an ERROR node), tokens are classed by what they are alone: strings, numbers,
//! comments, directives.

use crate::syntax::{self, Tok};
use crate::tape::{self, Field, Kind, Node};

/// What a token is, for an editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Comment,
    /// A directive (`.decl`, `.mode`, `#include`), a reserved word (`must`, `count`), a declaration's modifier.
    Keyword,
    String,
    /// The pattern of `match(pattern, n)`.
    Regexp,
    /// A backtick literal: a code pattern or a fix template.
    Code,
    Number,
    Operator,
    /// A relation where a clause names it (`kind(n, "x")`, a head, a `fix` line).
    Relation,
    /// A relation where `.decl`, `.violation`, `.edit`, `.lattice`, `.input`/`.output` declares it.
    RelationDeclaration,
    /// A functor or function called (`@f(x)`, `strlen(s)`).
    Function,
    /// A functor declared (`.functor f(…)`).
    FunctionDeclaration,
    Variable,
    Type,
    /// A type declared (`.type T …`).
    TypeDeclaration,
    /// A column of a declaration (`x` in `.decl r(x: node)`).
    Parameter,
    /// A named argument's name (`at: n`).
    Property,
    /// A setting's word (`log` in `.mode log`, `include` in `.scope include "…"`).
    Value,
    /// A `.test` or `.fixtures` block's name.
    Test,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Class::Comment => "comment",
            Class::Keyword => "keyword",
            Class::String => "string",
            Class::Regexp => "regexp",
            Class::Code => "code",
            Class::Number => "number",
            Class::Operator => "operator",
            Class::Relation => "relation",
            Class::RelationDeclaration => "relation-declaration",
            Class::Function => "function",
            Class::FunctionDeclaration => "function-declaration",
            Class::Variable => "variable",
            Class::Type => "type",
            Class::TypeDeclaration => "type-declaration",
            Class::Parameter => "parameter",
            Class::Property => "property",
            Class::Value => "value",
            Class::Test => "test",
        }
    }
}

/// A classed token: its byte span and class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub class: Class,
}

/// A statement that does not parse, or the place the first error is: its byte span and what the parser says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    pub start: usize,
    pub end: usize,
    pub message: String,
}

/// A file's tokens, in source order and not overlapping, and its syntax errors.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Highlighted {
    pub tokens: Vec<Token>,
    pub errors: Vec<SyntaxError>,
}

/// The punctuation that is an operator; the rest (brackets, `,`, `.`, `:`) is left unclassed.
fn operator(p: &str) -> bool {
    matches!(p, ":-" | "=>" | "!=" | "<=" | ">=" | "<:" | "->" | "=" | "<" | ">" | "!" | "|" | "*" | "-" | "+" | "/" | "%" | "~" | "^" | "&")
}

/// `src`'s tokens and comments classed, and its syntax errors: the first error at the token it is at, with the
/// parser's message, and each other statement that does not parse over its span.
pub fn highlight(src: &str) -> Highlighted {
    let lexed = syntax::lex(src);
    let mut parser = syntax::Parser::taped(src);
    let _ = parser.statements();
    let (nodes, first) = parser.into_tape();
    Highlighted { tokens: classed(src, &lexed.toks, &lexed.comments, &nodes), errors: errors(src, &lexed.toks, &nodes, first) }
}

fn classed(src: &str, toks: &[syntax::Spanned<'_>], comments: &[(usize, usize)], nodes: &[Node]) -> Vec<Token> {
    let mut out = Vec::with_capacity(toks.len() + comments.len());
    let mut comments = comments.iter().copied().peekable();
    // The nodes holding the token, outermost first; `next` is the first node not yet entered.
    let mut open: Vec<usize> = Vec::new();
    let mut next = 0;
    let mut i = 0;
    while i < toks.len() {
        let (tok, start, end) = (&toks[i].0, toks[i].1, toks[i].2);
        if matches!(tok, Tok::Eof) {
            break;
        }
        while let Some(&(c_start, c_end)) = comments.peek().filter(|&&(c_start, _)| c_start < start) {
            out.push(Token { start: c_start, end: c_end, class: Class::Comment });
            comments.next();
        }
        while next < nodes.len() && nodes[next].start as usize <= start {
            let node = &nodes[next];
            while open.last().is_some_and(|&top| node.parent != Some(top as u32)) {
                open.pop();
            }
            open.push(next);
            next += 1;
        }
        while open.last().is_some_and(|&top| (nodes[top].end as usize) <= start) {
            open.pop();
        }
        let in_error = open.iter().any(|&n| nodes[n].is_error());
        let leaf = open.last().copied().filter(|&n| is_leaf(nodes, n) && nodes[n].start as usize <= start && end <= nodes[n].end as usize).map(|n| &nodes[n]);
        let parent = |n: &Node| n.parent.map(|p| &nodes[p as usize]);
        // A directive's `.` and its word, written together: one keyword. The word has no leaf (implied by the
        // statement's kind), or is a setting's name.
        if let Tok::P(".") = tok
            && let Some((Tok::Id(_), w_start, w_end)) = toks.get(i + 1)
            && *w_start == end
            && !in_error
        {
            let directive = match leaf_at(nodes, *w_start) {
                None => true,
                Some(w) => w.kind == Kind::Identifier as u16 && w.field == Field::Name as u16 && parent(w).is_some_and(|p| p.kind == Kind::Setting as u16),
            };
            if directive {
                out.push(Token { start, end: *w_end, class: Class::Keyword });
                i += 2;
                continue;
            }
        }
        let class = match tok {
            Tok::Str(_) => Some(if leaf.is_some_and(|l| pattern_of_match(src, nodes, l)) { Class::Regexp } else { Class::String }),
            Tok::Num(_) | Tok::Float(_) => Some(Class::Number),
            Tok::Tick(_) => Some(Class::Code),
            Tok::Hash(_) => Some(Class::Keyword),
            Tok::P(p) => operator(p).then_some(Class::Operator),
            Tok::Id(_) if in_error => None,
            Tok::Id(_) => match leaf {
                None => Some(Class::Keyword),
                Some(l) if l.kind >= tape::FIRST_ANONYMOUS => Some(Class::Keyword),
                Some(l) if l.kind == Kind::Variable as u16 || l.kind == Kind::Wildcard as u16 => Some(Class::Variable),
                Some(l) if l.kind == Kind::Identifier as u16 => identifier(l, parent(l)),
                Some(_) => None,
            },
            Tok::Bad | Tok::Eof => None,
        };
        if let Some(class) = class
            && out.last().is_none_or(|t: &Token| t.end <= start)
        {
            out.push(Token { start, end, class });
        }
        i += 1;
    }
    out.extend(comments.map(|(start, end)| Token { start, end, class: Class::Comment }));
    out
}

/// Whether node `n` has no child but comments: a token's leaf (or a `raw_text`, a `json_object` over several).
fn is_leaf(nodes: &[Node], n: usize) -> bool {
    nodes[n + 1..].iter().take_while(|c| c.parent == Some(n as u32)).all(|c| c.extra)
}

/// The innermost leaf starting at byte `at`, if a leaf starts there.
fn leaf_at(nodes: &[Node], at: usize) -> Option<&Node> {
    let from = nodes.partition_point(|n| (n.start as usize) < at);
    (from..nodes.len()).take_while(|&n| nodes[n].start as usize == at).filter(|&n| is_leaf(nodes, n)).last().map(|n| &nodes[n])
}

/// An identifier leaf's class, by the node it is in and its field there.
fn identifier(leaf: &Node, parent: Option<&Node>) -> Option<Class> {
    let field = leaf.field;
    let Some(parent) = parent else { return None };
    let kind = parent.kind;
    let is = |k: Kind| kind == k as u16;
    let f = |x: Field| field == x as u16;
    Some(if f(Field::Modifier) {
        Class::Keyword
    } else if is(Kind::Setting) {
        if f(Field::Name) { Class::Keyword } else { Class::Value }
    } else if f(Field::Relation) {
        Class::Relation
    } else if f(Field::Function) {
        Class::Function
    } else if f(Field::Name)
        && (is(Kind::Declaration) || is(Kind::EditDeclaration) || is(Kind::ViolationDeclaration) || is(Kind::LatticeDeclaration) || is(Kind::IoRelation))
    {
        Class::RelationDeclaration
    } else if f(Field::Name) && is(Kind::FunctorDeclaration) {
        Class::FunctionDeclaration
    } else if f(Field::Name) && is(Kind::TypeDeclaration) {
        Class::TypeDeclaration
    } else if is(Kind::Type) {
        Class::Type
    } else if f(Field::Name) && is(Kind::Column) {
        Class::Parameter
    } else if f(Field::Name) && is(Kind::NamedArgument) {
        Class::Property
    } else if f(Field::Name) && (is(Kind::TestBlock) || is(Kind::FixturesBlock)) {
        Class::Test
    } else if f(Field::Base) {
        Class::Type
    } else if f(Field::Name) && is(Kind::LatticeOperation) {
        Class::Function
    } else if f(Field::Column) || f(Field::Over) || f(Field::Key) || f(Field::Argument) && (is(Kind::TestBlock) || is(Kind::Type)) {
        Class::Variable
    } else if f(Field::Tool) {
        Class::Value
    } else {
        return None;
    })
}

/// Whether string leaf `leaf` is the first argument of `match(…)`: a regex.
fn pattern_of_match(src: &str, nodes: &[Node], leaf: &Node) -> bool {
    let Some(list) = leaf.parent.map(|p| &nodes[p as usize]) else { return false };
    if list.kind != Kind::ArgumentList as u16 {
        return false;
    }
    let Some(list_at) = leaf.parent else { return false };
    let Some(call_at) = list.parent else { return false };
    let first = nodes[list_at as usize + 1..].iter().find(|n| n.parent == Some(list_at) && !n.extra);
    if first.is_none_or(|first| first.start != leaf.start) {
        return false;
    }
    nodes[call_at as usize + 1..]
        .iter()
        .take_while(|n| n.start < nodes[call_at as usize].end)
        .find(|n| n.parent == Some(call_at) && (n.field == Field::Relation as u16 || n.field == Field::Function as u16))
        .is_some_and(|name| src.get(name.start as usize..name.end as usize) == Some("match"))
}

fn errors(src: &str, toks: &[syntax::Spanned<'_>], nodes: &[Node], first: Option<(usize, crate::Error)>) -> Vec<SyntaxError> {
    let mut out = Vec::new();
    let first = first.map(|(at, crate::Error::Syntax(message))| {
        // The token the parse stopped at, or the rest of its line at the end of the file.
        let end = toks.iter().find(|(tok, start, _)| *start >= at && !matches!(tok, Tok::Eof)).map_or_else(|| line_end(src, at), |&(_, start, end)| if start == at { end } else { line_end(src, at) });
        let message = strip_line(&message).to_owned();
        out.push(SyntaxError { start: at, end: end.max(at), message });
        at
    });
    for node in nodes.iter().filter(|n| n.is_error()) {
        let (start, end) = (node.start as usize, node.end as usize);
        if first.is_some_and(|at| start <= at && at <= end) {
            continue;
        }
        out.push(SyntaxError { start, end, message: "this statement does not parse".to_owned() });
    }
    out.sort_by_key(|e| e.start);
    out
}

fn line_end(src: &str, at: usize) -> usize {
    src[at.min(src.len())..].find('\n').map_or(src.len(), |n| at + n)
}

/// A message without the `line N: ` it starts with: an editor shows where it is.
fn strip_line(message: &str) -> &str {
    message
        .strip_prefix("line ")
        .and_then(|rest| rest.split_once(": ").filter(|(n, _)| n.bytes().all(|b| b.is_ascii_digit())))
        .map_or(message, |(_, rest)| rest)
}

#[cfg(test)]
mod tests;
