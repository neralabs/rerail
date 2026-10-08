//! phpdoc-parser 2.3.5's `TokenIterator`, `PhpDocParser`, `TypeParser` and `ConstExprParser`,
//! ported one method to one function. A node's span is its `startIndex` and `endIndex` attributes
//! (token indices), which the parser sets each time it "enriches" a node and which a later
//! enrichment of the same node overwrites, as the PHP objects' attributes are; savepoints are a
//! stack the parser pushes and pops as phpdoc-parser does, unbalanced where it is.

use super::lexer::{T, Token, tokenize};
use std::ops::Range;

pub(super) type Id = usize;

/// A scalar property of a node, as phpdoc-parser holds it.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Prop {
    Str(Vec<u8>),
    Bool(bool),
    Int(i64),
    Strs(Vec<&'static str>),
}

/// A string property located in the text (a name, a variable, a description).
#[derive(Clone, Debug)]
pub(super) struct Leaf {
    pub(super) kind: &'static str,
    /// Bytes of the text.
    pub(super) span: Range<usize>,
    pub(super) value: Vec<u8>,
}

/// A node of phpdoc-parser's AST.
#[derive(Clone, Debug)]
pub(super) struct Node {
    /// The class without its namespace and `Node` suffix, in snake case.
    pub(super) kind: &'static str,
    /// Token indices (`startIndex`, `endIndex`); the end is before the start for no token.
    pub(super) start: Option<usize>,
    pub(super) end: Option<usize>,
    pub(super) props: Vec<(&'static str, Prop)>,
    /// Node-valued properties in declaration order, one entry per element of a list.
    pub(super) children: Vec<(&'static str, Id)>,
    pub(super) leaves: Vec<Leaf>,
}

/// A `ParserException`'s arguments.
#[derive(Clone, Debug)]
pub(super) struct Error {
    value: Range<usize>,
    offset: usize,
    expected: T,
    expected_value: Option<&'static str>,
    line: u32,
}

/// An exception `PhpDocParser::parse` throws: its class and message.
#[derive(Clone, Debug)]
pub(super) enum Fatal {
    /// A `ParserException` no production catches.
    Parser(String),
    /// A `LogicException`.
    Logic(&'static str),
}

impl Fatal {
    pub(super) fn message(&self) -> &str {
        match self {
            Self::Parser(message) => message,
            Self::Logic(message) => message,
        }
    }
}

type R<X> = Result<X, Error>;

/// `json_encode` with `JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES`, as parse errors quote.
fn json(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Error {
    /// `ParserException::getMessage`.
    pub(super) fn message(&self, text: &str) -> String {
        let expected_value = self
            .expected_value
            .map_or(String::new(), |value| format!(" ({})", json(value)));
        format!(
            "Unexpected token {}, expected {}{} at offset {} on line {}",
            json(&text[self.value.clone()]),
            self.expected.label(),
            expected_value,
            self.offset,
            self.line
        )
    }
}

/// A description and the bytes it was read from.
struct Text {
    value: String,
    span: Range<usize>,
}

const SHAPES: &[&str] = &[
    "array",
    "list",
    "non-empty-array",
    "non-empty-list",
    "object",
];

/// Nesting of types, constant expressions, Doctrine values and the tags a description looks ahead
/// at the parser follows.
const MAX_DEPTH: usize = crate::code_rule_dsl::value::MAX_DEPTH;

/// Tokens advanced and nodes built per token of the comment, backtracking included, before the
/// parse gives up.
const WORK_PER_TOKEN: usize = 64;

pub(super) struct Parser<'a> {
    pub(super) text: &'a str,
    pub(super) tokens: Vec<Token>,
    index: usize,
    /// How many comments `skipNewLineTokensAndConsumeComments` collected and no type took.
    comments: usize,
    saves: Vec<(usize, usize)>,
    /// `TOKEN_PHPDOC_EOL` is among the skipped tokens (inside a Doctrine annotation's arguments).
    skip_eol: bool,
    newline: Option<&'static str>,
    /// Nesting of the recursive productions, the work done, and its bound.
    depth: usize,
    work: usize,
    limit: usize,
    /// Nesting or work passed its bound: every production fails and the tree is unknown.
    exhausted: bool,
    pub(super) nodes: Vec<Node>,
}

/// Parses a doc comment: the parser, holding the nodes, and the root or what it threw.
pub(super) fn parse(text: &str) -> (Parser<'_>, Result<Id, Fatal>) {
    let mut parser = Parser {
        text,
        tokens: tokenize(text),
        index: 0,
        comments: 0,
        saves: Vec::new(),
        skip_eol: false,
        newline: None,
        depth: 0,
        work: 0,
        limit: 0,
        exhausted: false,
        nodes: Vec::new(),
    };
    parser.limit = WORK_PER_TOKEN * parser.tokens.len() + 4096;
    parser.skip_irrelevant();
    let root = parser.php_doc();
    (parser, root)
}

impl<'a> Parser<'a> {
    // `TokenIterator`.

    fn token(&self) -> Option<&Token> {
        self.tokens.get(self.index)
    }

    fn is(&self, ty: T) -> bool {
        self.token().is_some_and(|token| token.ty == ty)
    }

    fn is_any(&self, types: &[T]) -> bool {
        self.token().is_some_and(|token| types.contains(&token.ty))
    }

    fn value(&self) -> &'a str {
        let text = self.text;
        self.token()
            .map_or("", |token| &text[token.start..token.end])
    }

    fn offset(&self) -> usize {
        self.token().map_or(self.text.len(), |token| token.start)
    }

    fn error(&self, expected: T, expected_value: Option<&'static str>) -> Error {
        let (value, line) = self
            .token()
            .map_or((self.text.len()..self.text.len(), 0), |token| {
                (token.start..token.end, token.line)
            });
        Error {
            value,
            offset: self.offset(),
            expected,
            expected_value,
            line,
        }
    }

    fn skipped(&self, ty: T) -> bool {
        ty == T::HorizontalWs || (self.skip_eol && ty == T::PhpdocEol)
    }

    fn skip_irrelevant(&mut self) {
        if self.index >= self.tokens.len() {
            return;
        }
        while self.skipped(self.tokens[self.index].ty) {
            if self.index + 1 >= self.tokens.len() {
                break;
            }
            self.index += 1;
        }
    }

    fn next(&mut self) {
        self.work += 1;
        self.index += 1;
        self.skip_irrelevant();
    }

    /// Enters a recursive production: an error once nesting or work passed its bound.
    fn enter(&mut self) -> R<()> {
        if self.exhausted || self.depth >= MAX_DEPTH || self.work > self.limit {
            self.exhausted = true;
            return Err(self.error(T::End, None));
        }
        self.depth += 1;
        Ok(())
    }

    fn leave<X>(&mut self, result: R<X>) -> R<X> {
        self.depth -= 1;
        result
    }

    /// Whether nesting or work passed its bound, so the tree is not phpdoc-parser's.
    pub(super) fn exhausted(&self) -> bool {
        self.exhausted
    }

    fn detect_newline(&mut self) {
        let value = self.value();
        if value.starts_with("\r\n") {
            self.newline = Some("\r\n");
        } else if value.starts_with('\n') {
            self.newline = Some("\n");
        }
    }

    fn consume(&mut self, ty: T) -> R<()> {
        if !self.is(ty) {
            return Err(self.error(ty, None));
        }
        if ty == T::PhpdocEol && self.newline.is_none() {
            self.detect_newline();
        }
        self.next();
        Ok(())
    }

    fn consume_value(&mut self, ty: T, value: &'static str) -> R<()> {
        if !self.is(ty) || self.value() != value {
            return Err(self.error(ty, Some(value)));
        }
        self.next();
        Ok(())
    }

    fn try_consume_value(&mut self, value: &str) -> bool {
        if self.value() != value {
            return false;
        }
        self.next();
        true
    }

    fn try_consume(&mut self, ty: T) -> bool {
        if !self.is(ty) {
            return false;
        }
        if ty == T::PhpdocEol && self.newline.is_none() {
            self.detect_newline();
        }
        self.next();
        true
    }

    /// `skipNewLineTokensAndConsumeComments`.
    fn skip_lines(&mut self) {
        if self.is(T::Comment) {
            self.comments += 1;
            self.next();
        }
        if !self.is(T::PhpdocEol) {
            return;
        }
        loop {
            let found = self.try_consume(T::PhpdocEol);
            if self.is(T::Comment) {
                self.comments += 1;
                self.next();
            }
            if !found {
                break;
            }
        }
    }

    /// `getSkippedHorizontalWhiteSpaceIfAny`.
    fn skipped_space(&self) -> &'a str {
        let text = self.text;
        match self.index.checked_sub(1).and_then(|at| self.tokens.get(at)) {
            Some(token) if token.ty == T::HorizontalWs => &text[token.start..token.end],
            _ => "",
        }
    }

    fn join_until(&mut self, stops: &[T]) -> &'a str {
        let start = self.offset();
        while !self.is_any(stops) {
            self.index += 1;
            self.work += 1;
        }
        &self.text[start..self.offset()]
    }

    /// `endIndexOfLastRelevantToken`.
    fn end_index(&self) -> usize {
        let mut end = self.index.saturating_sub(1);
        while self
            .tokens
            .get(end)
            .is_some_and(|token| self.skipped(token.ty))
        {
            if end == 0 {
                break;
            }
            end -= 1;
        }
        end
    }

    fn preceded_by_space(&self) -> bool {
        self.index
            .checked_sub(1)
            .and_then(|at| self.tokens.get(at))
            .is_some_and(|token| token.ty == T::HorizontalWs)
    }

    fn push(&mut self) {
        self.saves.push((self.index, self.comments));
    }

    fn drop_save(&mut self) {
        self.saves.pop();
    }

    fn rollback(&mut self) {
        if let Some((index, comments)) = self.saves.pop() {
            self.index = index;
            self.comments = comments;
        }
    }

    // Nodes.

    fn add(&mut self, kind: &'static str) -> Id {
        self.work += 1;
        self.nodes.push(Node {
            kind,
            start: None,
            end: None,
            props: Vec::new(),
            children: Vec::new(),
            leaves: Vec::new(),
        });
        self.nodes.len() - 1
    }

    fn prop(&mut self, id: Id, name: &'static str, prop: Prop) {
        self.nodes[id].props.push((name, prop));
    }

    fn str_prop(&mut self, id: Id, name: &'static str, value: &str) {
        self.prop(id, name, Prop::Str(value.as_bytes().to_vec()));
    }

    fn child(&mut self, id: Id, name: &'static str, child: Id) {
        self.nodes[id].children.push((name, child));
    }

    fn opt_child(&mut self, id: Id, name: &'static str, child: Option<Id>) {
        if let Some(child) = child {
            self.child(id, name, child);
        }
    }

    /// `PhpDocParser::enrichWithAttributes` and `ConstExprParser::enrichWithAttributes`.
    fn enrich(&mut self, id: Id, start: usize) -> Id {
        let end = self.end_index();
        let node = &mut self.nodes[id];
        node.start = Some(start);
        node.end = Some(end);
        id
    }

    /// `TypeParser::enrichWithAttributes`, which also takes the comments read so far.
    fn enrich_type(&mut self, id: Id, start: usize) -> Id {
        self.comments = 0;
        self.enrich(id, start)
    }

    /// A token as a leaf of `id`.
    fn leaf(&mut self, id: Id, kind: &'static str, token: usize, value: &str) {
        let span = self.tokens[token].start..self.tokens[token].end;
        self.nodes[id].leaves.push(Leaf {
            kind,
            span,
            value: value.as_bytes().to_vec(),
        });
    }

    /// A description as a leaf of `id`, and its property.
    fn description(&mut self, id: Id, name: &'static str, text: Text) {
        if !text.value.is_empty() {
            let bytes = self.text.as_bytes();
            let mut span = text.span;
            while span.start < span.end && bytes[span.start].is_ascii_whitespace() {
                span.start += 1;
            }
            while span.end > span.start && bytes[span.end - 1].is_ascii_whitespace() {
                span.end -= 1;
            }
            self.nodes[id].leaves.push(Leaf {
                kind: "description",
                span,
                value: text.value.as_bytes().to_vec(),
            });
        }
        self.prop(id, name, Prop::Str(text.value.into_bytes()));
    }

    fn name_of(&self, id: Id) -> Option<&[u8]> {
        let node = &self.nodes[id];
        if node.kind != "identifier_type" {
            return None;
        }
        node.props.iter().find_map(|(name, prop)| match prop {
            Prop::Str(value) if *name == "name" => Some(value.as_slice()),
            _ => None,
        })
    }

    fn identifier(&mut self, name: &str) -> Id {
        let id = self.add("identifier_type");
        self.str_prop(id, "name", name);
        id
    }

    // `PhpDocParser`.

    fn php_doc(&mut self) -> Result<Id, Fatal> {
        let fatal = |parser: &Self, error: Error| Fatal::Parser(error.message(parser.text));
        if let Err(error) = self.consume(T::OpenPhpdoc) {
            return Err(fatal(self, error));
        }
        self.try_consume(T::PhpdocEol);
        let mut children = Vec::new();
        if !self.is(T::ClosePhpdoc) {
            let mut last = self.parse_child().map_err(|error| fatal(self, error))?;
            children.push(last);
            while !self.is(T::ClosePhpdoc) {
                if self.nodes[last].kind == "php_doc_tag"
                    && matches!(
                        self.nodes[self.nodes[last].children[0].1].kind,
                        "doctrine_tag_value" | "generic_tag_value"
                    )
                {
                    self.try_consume(T::PhpdocEol);
                    if self.is(T::ClosePhpdoc) {
                        break;
                    }
                    last = self.parse_child().map_err(|error| fatal(self, error))?;
                    children.push(last);
                    continue;
                }
                if !self.try_consume(T::PhpdocEol) {
                    break;
                }
                if self.is(T::ClosePhpdoc) {
                    break;
                }
                last = self.parse_child().map_err(|error| fatal(self, error))?;
                children.push(last);
            }
        }
        let flushed = || Fatal::Logic("Comments should already be flushed");
        if let Err(error) = self.consume(T::ClosePhpdoc) {
            let start = self.index;
            let name = children
                .last()
                .filter(|last| self.nodes[**last].kind == "php_doc_tag")
                .and_then(|last| {
                    self.nodes[*last]
                        .props
                        .iter()
                        .find_map(|(name, prop)| match prop {
                            Prop::Str(value) if *name == "name" => Some(value.clone()),
                            _ => None,
                        })
                })
                .unwrap_or_default();
            let message = error.message(self.text);
            let invalid = self.add("invalid_tag_value");
            self.str_prop(invalid, "value", &message);
            self.str_prop(invalid, "message", &message);
            self.enrich(invalid, start);
            let tag = self.add("php_doc_tag");
            self.prop(tag, "name", Prop::Str(name));
            self.child(tag, "value", invalid);
            self.index = self.tokens.len() - 1;
            self.enrich(tag, start);
            if self.comments != 0 {
                return Err(flushed());
            }
            let root = self.add("php_doc");
            self.child(root, "children", tag);
            return Ok(self.enrich(root, 0));
        }
        if self.comments != 0 {
            return Err(flushed());
        }
        let root = self.add("php_doc");
        for child in children {
            self.child(root, "children", child);
        }
        Ok(self.enrich(root, 0))
    }

    fn parse_child(&mut self) -> R<Id> {
        let start = self.index;
        if self.is(T::PhpdocTag) {
            let tag = self.parse_tag();
            return Ok(self.enrich(tag, start));
        }
        if self.is(T::DoctrineTag) {
            let name = self.value();
            self.next();
            let tag_start = self.index;
            let value = self.parse_doctrine_tag_value(name)?;
            let value = self.enrich(value, tag_start);
            let tag = self.add("php_doc_tag");
            self.str_prop(tag, "name", name);
            self.leaf(tag, "name", start, name);
            self.child(tag, "value", value);
            return Ok(self.enrich(tag, start));
        }
        let text = self.parse_text();
        let node = self.add("php_doc_text");
        self.str_prop(node, "text", &text.value);
        Ok(self.enrich(node, start))
    }

    fn parse_text(&mut self) -> Text {
        let begin = self.offset();
        let mut text = String::new();
        let mut savepoint = false;
        loop {
            let space = self.skipped_space();
            let joined = self.join_until(&[T::PhpdocEol, T::ClosePhpdoc, T::End]);
            text.push_str(space);
            text.push_str(joined);
            if !self.is_any(&[T::PhpdocEol, T::ClosePhpdoc]) {
                break;
            }
            if !savepoint {
                self.push();
                savepoint = true;
            } else if !space.is_empty() || !joined.is_empty() {
                self.drop_save();
                self.push();
            }
            self.push();
            self.next();
            if self.is_any(&[T::PhpdocTag, T::DoctrineTag, T::ClosePhpdoc, T::End]) {
                self.rollback();
                break;
            }
            self.drop_save();
            text.push_str(self.newline.unwrap_or("\n"));
        }
        if savepoint {
            self.rollback();
            let newline = self.newline.unwrap_or("\n");
            let kept = text.trim_end_matches(|c| newline.contains(c)).len();
            text.truncate(kept);
        }
        Text {
            value: text.trim_matches([' ', '\t']).to_owned(),
            span: begin..self.offset(),
        }
    }

    fn parse_description_after_doctrine_tag(&mut self) -> Text {
        let begin = self.offset();
        let mut text = String::new();
        let mut savepoint = false;
        let ends = [T::ClosePhpdoc, T::End];
        loop {
            let space = self.skipped_space();
            let joined = self.join_until(&[
                T::PhpdocTag,
                T::DoctrineTag,
                T::PhpdocEol,
                T::ClosePhpdoc,
                T::End,
            ]);
            text.push_str(space);
            text.push_str(joined);
            if !self.is_any(&[T::PhpdocEol, T::ClosePhpdoc]) {
                let rest = |parser: &mut Self, mut text: String| {
                    text.push_str(&parser.parse_text().value);
                    Text {
                        value: text.trim_matches([' ', '\t']).to_owned(),
                        span: begin..parser.offset(),
                    }
                };
                if !self.preceded_by_space() {
                    return rest(self, text);
                }
                if self.is(T::PhpdocTag) {
                    // phpdoc-parser parses the next tag to see whether it ends the description,
                    // and that tag's description the tag after it: the chain is nesting, and
                    // the nodes it builds are dropped with it.
                    if self.enter().is_err() {
                        break;
                    }
                    let nodes = self.nodes.len();
                    self.push();
                    let start = self.index;
                    let tag = self.parse_tag();
                    let child = self.enrich(tag, start);
                    let value = self.nodes[self.nodes[child].children[0].1].kind;
                    self.nodes.truncate(nodes);
                    self.depth -= 1;
                    if matches!(value, "generic_tag_value" | "doctrine_tag_value") {
                        self.rollback();
                        break;
                    }
                    if value == "invalid_tag_value" {
                        self.rollback();
                        self.push();
                        self.next();
                        if self.is(T::OpenParentheses) {
                            self.rollback();
                            break;
                        }
                        self.rollback();
                        return rest(self, text);
                    }
                    self.rollback();
                    return rest(self, text);
                }
                break;
            }
            if !savepoint {
                self.push();
                savepoint = true;
            } else if !space.is_empty() || !joined.is_empty() {
                self.drop_save();
                self.push();
            }
            self.push();
            self.next();
            if self.is_any(&[T::PhpdocTag, T::DoctrineTag]) || self.is_any(&ends) {
                self.rollback();
                break;
            }
            self.drop_save();
            text.push_str(self.newline.unwrap_or("\n"));
        }
        if savepoint {
            self.rollback();
            let newline = self.newline.unwrap_or("\n");
            let kept = text.trim_end_matches(|c| newline.contains(c)).len();
            text.truncate(kept);
        }
        Text {
            value: text.trim_matches([' ', '\t']).to_owned(),
            span: begin..self.offset(),
        }
    }

    fn parse_tag(&mut self) -> Id {
        let name = self.value();
        let name_token = self.index;
        self.next();
        let value = self.parse_tag_value(name);
        let tag = self.add("php_doc_tag");
        self.str_prop(tag, "name", name);
        self.leaf(tag, "name", name_token, name);
        self.child(tag, "value", value);
        tag
    }

    fn parse_tag_value(&mut self, tag: &str) -> Id {
        let start = self.index;
        self.push();
        let value = match tag {
            "@param" | "@phpstan-param" | "@psalm-param" | "@phan-param" => self.parse_param(),
            "@param-immediately-invoked-callable"
            | "@phpstan-param-immediately-invoked-callable" => {
                self.parse_named("param_immediately_invoked_callable_tag_value")
            }
            "@param-later-invoked-callable" | "@phpstan-param-later-invoked-callable" => {
                self.parse_named("param_later_invoked_callable_tag_value")
            }
            "@param-closure-this" | "@phpstan-param-closure-this" => {
                self.parse_typed_named("param_closure_this_tag_value", "parameterName")
            }
            "@pure-unless-callable-is-impure" | "@phpstan-pure-unless-callable-is-impure" => {
                self.parse_named("pure_unless_callable_is_impure_tag_value")
            }
            "@pure-unless-parameter-passed" | "@phpstan-pure-unless-parameter-passed" => {
                self.parse_named("pure_unless_parameter_is_passed_tag_value")
            }
            "@var" | "@phpstan-var" | "@psalm-var" | "@phan-var" => self.parse_var(),
            "@return" | "@phpstan-return" | "@psalm-return" | "@phan-return"
            | "@phan-real-return" => self.parse_typed("return_tag_value"),
            "@throws" | "@phpstan-throws" => self.parse_typed("throws_tag_value"),
            "@mixin" | "@phan-mixin" => self.parse_typed("mixin_tag_value"),
            "@psalm-require-extends" | "@phpstan-require-extends" => {
                self.parse_typed("require_extends_tag_value")
            }
            "@psalm-require-implements" | "@phpstan-require-implements" => {
                self.parse_typed("require_implements_tag_value")
            }
            "@psalm-inheritors" | "@phpstan-sealed" => self.parse_typed("sealed_tag_value"),
            "@deprecated" => self.parse_optional_description(false).map(|text| {
                let id = self.add("deprecated_tag_value");
                self.description(id, "description", text);
                id
            }),
            "@property"
            | "@property-read"
            | "@property-write"
            | "@phpstan-property"
            | "@phpstan-property-read"
            | "@phpstan-property-write"
            | "@psalm-property"
            | "@psalm-property-read"
            | "@psalm-property-write"
            | "@phan-property"
            | "@phan-property-read"
            | "@phan-property-write" => {
                self.parse_typed_named("property_tag_value", "propertyName")
            }
            "@method" | "@phpstan-method" | "@psalm-method" | "@phan-method" => self.parse_method(),
            "@template"
            | "@phpstan-template"
            | "@psalm-template"
            | "@phan-template"
            | "@template-covariant"
            | "@phpstan-template-covariant"
            | "@psalm-template-covariant"
            | "@template-contravariant"
            | "@phpstan-template-contravariant"
            | "@psalm-template-contravariant" => self.parse_template_tag_value(true),
            "@extends" | "@phpstan-extends" | "@phan-extends" | "@phan-inherits"
            | "@template-extends" => self.parse_extends("extends_tag_value"),
            "@implements" | "@phpstan-implements" | "@template-implements" => {
                self.parse_extends("implements_tag_value")
            }
            "@use" | "@phpstan-use" | "@template-use" => self.parse_extends("uses_tag_value"),
            "@phpstan-type" | "@psalm-type" | "@phan-type" => self.parse_type_alias(),
            "@phpstan-import-type" | "@psalm-import-type" => self.parse_type_alias_import(),
            "@phpstan-assert"
            | "@phpstan-assert-if-true"
            | "@phpstan-assert-if-false"
            | "@psalm-assert"
            | "@psalm-assert-if-true"
            | "@psalm-assert-if-false"
            | "@phan-assert"
            | "@phan-assert-if-true"
            | "@phan-assert-if-false" => self.parse_assert(),
            "@phpstan-this-out" | "@phpstan-self-out" | "@psalm-this-out" | "@psalm-self-out" => {
                self.parse_typed("self_out_tag_value")
            }
            "@param-out" | "@phpstan-param-out" | "@psalm-param-out" => {
                self.parse_typed_named("param_out_tag_value", "parameterName")
            }
            _ => {
                if self.is(T::OpenParentheses) {
                    self.parse_doctrine_tag_value(tag)
                } else {
                    let text = self.parse_description_after_doctrine_tag();
                    let id = self.add("generic_tag_value");
                    self.description(id, "value", text);
                    Ok(id)
                }
            }
        };
        let value = match value {
            Ok(value) => {
                self.drop_save();
                value
            }
            Err(error) => {
                self.rollback();
                let message = error.message(self.text);
                let text = self.parse_text();
                let id = self.add("invalid_tag_value");
                self.description(id, "value", text);
                self.str_prop(id, "message", &message);
                id
            }
        };
        self.enrich(value, start)
    }

    fn parse_doctrine_tag_value(&mut self, tag: &str) -> R<Id> {
        let start = self.index;
        let arguments = self.parse_doctrine_arguments(false)?;
        let annotation = self.add("doctrine_annotation");
        self.str_prop(annotation, "name", tag);
        for argument in arguments {
            self.child(annotation, "arguments", argument);
        }
        let annotation = self.enrich(annotation, start);
        let text = self.parse_description_after_doctrine_tag();
        let id = self.add("doctrine_tag_value");
        self.child(id, "annotation", annotation);
        self.description(id, "description", text);
        Ok(id)
    }

    fn parse_doctrine_arguments(&mut self, deep: bool) -> R<Vec<Id>> {
        if !self.is(T::OpenParentheses) {
            return Ok(Vec::new());
        }
        if !deep {
            self.skip_eol = true;
        }
        let mut arguments = Vec::new();
        let listed = (|| {
            self.consume(T::OpenParentheses)?;
            loop {
                if self.is(T::CloseParentheses) {
                    break;
                }
                arguments.push(self.parse_doctrine_argument()?);
                if !self.try_consume(T::Comma) {
                    break;
                }
            }
            Ok::<_, Error>(())
        })();
        if !deep {
            self.skip_eol = false;
        }
        listed?;
        self.consume(T::CloseParentheses)?;
        Ok(arguments)
    }

    fn parse_doctrine_argument(&mut self) -> R<Id> {
        let start = self.index;
        if !self.is(T::Identifier) {
            let value = self.parse_doctrine_argument_value()?;
            let id = self.add("doctrine_argument");
            self.child(id, "value", value);
            return Ok(self.enrich(id, start));
        }
        self.push();
        let keyed = (|| {
            let name = self.value();
            self.consume(T::Identifier)?;
            let key = self.identifier(name);
            let key = self.enrich(key, start);
            self.consume(T::Equal)?;
            let value = self.parse_doctrine_argument_value()?;
            Ok::<_, Error>((key, value))
        })();
        let id = self.add("doctrine_argument");
        match keyed {
            Ok((key, value)) => {
                self.drop_save();
                self.child(id, "key", key);
                self.child(id, "value", value);
            }
            Err(_) => {
                self.rollback();
                let value = self.parse_doctrine_argument_value()?;
                self.child(id, "value", value);
            }
        }
        Ok(self.enrich(id, start))
    }

    fn parse_doctrine_argument_value(&mut self) -> R<Id> {
        self.enter()?;
        let result = self.doctrine_argument_value();
        self.leave(result)
    }

    fn doctrine_argument_value(&mut self) -> R<Id> {
        let start = self.index;
        if self.is_any(&[T::PhpdocTag, T::DoctrineTag]) {
            let name = self.value();
            self.next();
            let arguments = self.parse_doctrine_arguments(true)?;
            let id = self.add("doctrine_annotation");
            self.str_prop(id, "name", name);
            for argument in arguments {
                self.child(id, "arguments", argument);
            }
            return Ok(self.enrich(id, start));
        }
        if self.try_consume(T::OpenCurlyBracket) {
            let mut items = Vec::new();
            loop {
                if self.is(T::CloseCurlyBracket) {
                    break;
                }
                items.push(self.parse_doctrine_array_item()?);
                if !self.try_consume(T::Comma) {
                    break;
                }
            }
            self.consume(T::CloseCurlyBracket)?;
            let id = self.add("doctrine_array");
            for item in items {
                self.child(id, "items", item);
            }
            return Ok(self.enrich(id, start));
        }
        let name = self.value();
        self.push();
        if self.try_consume(T::Identifier) {
            let identifier = self.identifier(name);
            let identifier = self.enrich(identifier, start);
            if !self.is(T::DoubleColon) {
                self.drop_save();
                return Ok(identifier);
            }
            self.rollback();
        } else {
            self.drop_save();
        }
        let before = self.error(T::Identifier, None);
        let expression = self.parse_const(true)?;
        if self.nodes[expression].kind == "const_expr_array" {
            return Err(before);
        }
        Ok(expression)
    }

    fn parse_doctrine_array_item(&mut self) -> R<Id> {
        let start = self.index;
        self.push();
        let keyed = (|| {
            let key = self.parse_doctrine_array_key()?;
            if !self.try_consume(T::Equal) && !self.try_consume(T::Colon) {
                self.consume(T::Equal)?;
            }
            let value = self.parse_doctrine_argument_value()?;
            Ok::<_, Error>((key, value))
        })();
        let id = self.add("doctrine_array_item");
        match keyed {
            Ok((key, value)) => {
                self.drop_save();
                self.child(id, "key", key);
                self.child(id, "value", value);
            }
            Err(_) => {
                self.rollback();
                let value = self.parse_doctrine_argument_value()?;
                self.child(id, "value", value);
            }
        }
        Ok(self.enrich(id, start))
    }

    fn parse_doctrine_array_key(&mut self) -> R<Id> {
        let start = self.index;
        let key = if self.is(T::Integer) {
            let key = self.add("const_expr_integer");
            let value = self.value().replace('_', "");
            self.str_prop(key, "value", &value);
            self.next();
            key
        } else if self.is(T::DoctrineAnnotationString) {
            let value = self.value();
            let key = self.parse_doctrine_string(value);
            self.next();
            key
        } else if self.is(T::SingleQuotedString) {
            let key = self.add("const_expr_string");
            let value = unescape_string(self.value());
            self.prop(key, "value", Prop::Str(value));
            self.prop(key, "quoteType", Prop::Int(1));
            self.next();
            key
        } else if self.is(T::DoubleQuotedString) {
            let value = self.value();
            self.next();
            self.parse_doctrine_string(value)
        } else {
            let name = self.value();
            self.push();
            if !self.try_consume(T::Identifier) {
                self.drop_save();
                return Err(self.error(T::Identifier, None));
            }
            if !self.is(T::DoubleColon) {
                self.drop_save();
                let key = self.identifier(name);
                return Ok(self.enrich(key, start));
            }
            self.rollback();
            let expression = self.parse_const(true)?;
            if self.nodes[expression].kind != "const_fetch" {
                return Err(self.error(T::Identifier, None));
            }
            return Ok(expression);
        };
        Ok(self.enrich(key, start))
    }

    fn parse_param(&mut self) -> R<Id> {
        let ty = if self.is_any(&[T::Reference, T::Variadic, T::Variable]) {
            None
        } else {
            Some(self.parse_type()?)
        };
        let reference = self.try_consume(T::Reference);
        let variadic = self.try_consume(T::Variadic);
        let (name, token) = self.parse_required_variable()?;
        let text = self.parse_optional_description(false)?;
        let id = self.add(if ty.is_some() {
            "param_tag_value"
        } else {
            "typeless_param_tag_value"
        });
        self.opt_child(id, "type", ty);
        self.prop(id, "isReference", Prop::Bool(reference));
        self.prop(id, "isVariadic", Prop::Bool(variadic));
        self.str_prop(id, "parameterName", name);
        self.leaf(id, "variable", token, name);
        self.description(id, "description", text);
        Ok(id)
    }

    /// A tag of a parameter name and a description.
    fn parse_named(&mut self, kind: &'static str) -> R<Id> {
        let (name, token) = self.parse_required_variable()?;
        let text = self.parse_optional_description(false)?;
        let id = self.add(kind);
        self.str_prop(id, "parameterName", name);
        self.leaf(id, "variable", token, name);
        self.description(id, "description", text);
        Ok(id)
    }

    /// A tag of a type, a variable and a description.
    fn parse_typed_named(&mut self, kind: &'static str, property: &'static str) -> R<Id> {
        let ty = self.parse_type()?;
        let (name, token) = self.parse_required_variable()?;
        let text = self.parse_optional_description(false)?;
        let id = self.add(kind);
        self.child(id, "type", ty);
        self.str_prop(id, property, name);
        self.leaf(id, "variable", token, name);
        self.description(id, "description", text);
        Ok(id)
    }

    /// A tag of a type and a description.
    fn parse_typed(&mut self, kind: &'static str) -> R<Id> {
        let ty = self.parse_type()?;
        let text = self.parse_optional_description(true)?;
        let id = self.add(kind);
        self.child(id, "type", ty);
        self.description(id, "description", text);
        Ok(id)
    }

    fn parse_var(&mut self) -> R<Id> {
        let ty = self.parse_type()?;
        let variable = self.parse_optional_variable();
        let text = self.parse_optional_description(variable.is_none())?;
        let id = self.add("var_tag_value");
        self.child(id, "type", ty);
        let name = variable.map_or("", |(name, token)| {
            self.leaf(id, "variable", token, name);
            name
        });
        self.str_prop(id, "variableName", name);
        self.description(id, "description", text);
        Ok(id)
    }

    fn parse_method(&mut self) -> R<Id> {
        let first = self.parse_type()?;
        let (mut is_static, second) = if self.name_of(first) == Some(b"static") {
            (true, self.parse_type()?)
        } else {
            (false, first)
        };
        let (return_type, name, name_span) = if self.is(T::Identifier) {
            let name = self.value().as_bytes().to_vec();
            let token = &self.tokens[self.index];
            let span = token.start..token.end;
            self.next();
            (Some(second), name, span)
        } else if let Some(name) = self.name_of(second) {
            let name = name.to_vec();
            let node = &self.nodes[second];
            let span = node.start.zip(node.end).map_or(0..0, |(start, end)| {
                self.tokens[start].start..self.tokens[end].end
            });
            let return_type = is_static.then_some(first);
            is_static = false;
            (return_type, name, span)
        } else {
            return Err(self.error(T::Identifier, None));
        };
        let mut templates = Vec::new();
        if self.try_consume(T::OpenAngleBracket) {
            loop {
                let start = self.index;
                let template = self.parse_template_tag_value(false)?;
                templates.push(self.enrich(template, start));
                if !self.try_consume(T::Comma) {
                    break;
                }
            }
            self.consume(T::CloseAngleBracket)?;
        }
        let mut parameters = Vec::new();
        self.consume(T::OpenParentheses)?;
        if !self.is(T::CloseParentheses) {
            parameters.push(self.parse_method_parameter()?);
            while self.try_consume(T::Comma) {
                parameters.push(self.parse_method_parameter()?);
            }
        }
        self.consume(T::CloseParentheses)?;
        let text = self.parse_optional_description(false)?;
        let id = self.add("method_tag_value");
        self.prop(id, "isStatic", Prop::Bool(is_static));
        self.opt_child(id, "returnType", return_type);
        self.nodes[id].leaves.push(Leaf {
            kind: "name",
            span: name_span,
            value: name.clone(),
        });
        self.prop(id, "methodName", Prop::Str(name));
        for template in templates {
            self.child(id, "templateTypes", template);
        }
        for parameter in parameters {
            self.child(id, "parameters", parameter);
        }
        self.description(id, "description", text);
        Ok(id)
    }

    fn parse_method_parameter(&mut self) -> R<Id> {
        let start = self.index;
        let ty = if self.is_any(&[T::Identifier, T::OpenParentheses, T::Nullable]) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let reference = self.try_consume(T::Reference);
        let variadic = self.try_consume(T::Variadic);
        let name = self.value();
        let token = self.index;
        self.consume(T::Variable)?;
        let default = if self.try_consume(T::Equal) {
            Some(self.parse_const(false)?)
        } else {
            None
        };
        let id = self.add("method_tag_value_parameter");
        self.opt_child(id, "type", ty);
        self.prop(id, "isReference", Prop::Bool(reference));
        self.prop(id, "isVariadic", Prop::Bool(variadic));
        self.str_prop(id, "parameterName", name);
        self.leaf(id, "variable", token, name);
        self.opt_child(id, "defaultValue", default);
        Ok(self.enrich(id, start))
    }

    fn parse_extends(&mut self, kind: &'static str) -> R<Id> {
        let start = self.index;
        let name = self.value();
        self.consume(T::Identifier)?;
        let base = self.identifier(name);
        let base = self.enrich_type(base, start);
        let ty = self.parse_generic(base)?;
        let text = self.parse_optional_description(true)?;
        let id = self.add(kind);
        self.child(id, "type", ty);
        self.description(id, "description", text);
        Ok(id)
    }

    fn parse_type_alias(&mut self) -> R<Id> {
        let alias = self.value();
        let alias_token = self.index;
        self.consume(T::Identifier)?;
        self.try_consume(T::Equal);
        let start = self.index;
        let parsed = (|| {
            let ty = self.parse_type()?;
            if !self.is(T::ClosePhpdoc) && !self.is(T::PhpdocEol) {
                return Err(self.error(T::PhpdocEol, None));
            }
            Ok::<_, Error>(ty)
        })();
        let ty = match parsed {
            Ok(ty) => ty,
            Err(error) => {
                let _ = self.parse_optional_description(false);
                let invalid = self.add("invalid_type");
                self.str_prop(invalid, "message", &error.message(self.text));
                self.enrich(invalid, start)
            }
        };
        let id = self.add("type_alias_tag_value");
        self.str_prop(id, "alias", alias);
        self.leaf(id, "name", alias_token, alias);
        self.child(id, "type", ty);
        Ok(id)
    }

    fn parse_type_alias_import(&mut self) -> R<Id> {
        let alias = self.value();
        let alias_token = self.index;
        self.consume(T::Identifier)?;
        self.consume_value(T::Identifier, "from")?;
        let from_start = self.index;
        let from = self.value();
        self.consume(T::Identifier)?;
        let from = self.identifier(from);
        let from = self.enrich(from, from_start);
        let mut alias_as = None;
        if self.try_consume_value("as") {
            alias_as = Some((self.value(), self.index));
            self.consume(T::Identifier)?;
        }
        let id = self.add("type_alias_import_tag_value");
        self.str_prop(id, "importedAlias", alias);
        self.leaf(id, "name", alias_token, alias);
        self.child(id, "importedFrom", from);
        if let Some((name, token)) = alias_as {
            self.str_prop(id, "importedAs", name);
            self.leaf(id, "name", token, name);
        }
        Ok(id)
    }

    fn parse_assert(&mut self) -> R<Id> {
        let negated = self.try_consume(T::Negated);
        let equality = self.try_consume(T::Equal);
        let ty = self.parse_type()?;
        let parameter_token = self.index;
        let parameter = if self.is(T::ThisVariable) {
            self.next();
            "$this"
        } else {
            let parameter = self.value();
            self.consume(T::Variable)?;
            parameter
        };
        let mut member = None;
        if self.is(T::Arrow) {
            self.consume(T::Arrow)?;
            let name = self.value();
            let token = self.index;
            self.consume(T::Identifier)?;
            let method = if self.try_consume(T::OpenParentheses) {
                self.consume(T::CloseParentheses)?;
                true
            } else {
                false
            };
            member = Some((name, token, method));
        }
        let text = self.parse_optional_description(false)?;
        let id = self.add(match member {
            Some((_, _, true)) => "assert_tag_method_value",
            Some((_, _, false)) => "assert_tag_property_value",
            None => "assert_tag_value",
        });
        self.child(id, "type", ty);
        self.str_prop(id, "parameter", parameter);
        self.leaf(id, "variable", parameter_token, parameter);
        if let Some((name, token, method)) = member {
            self.str_prop(id, if method { "method" } else { "property" }, name);
            self.leaf(id, "name", token, name);
        }
        self.prop(id, "isNegated", Prop::Bool(negated));
        self.prop(id, "isEquality", Prop::Bool(equality));
        self.description(id, "description", text);
        Ok(id)
    }

    fn parse_optional_variable(&mut self) -> Option<(&'a str, usize)> {
        let token = self.index;
        if self.is(T::Variable) {
            let name = self.value();
            self.next();
            Some((name, token))
        } else if self.is(T::ThisVariable) {
            self.next();
            Some(("$this", token))
        } else {
            None
        }
    }

    fn parse_required_variable(&mut self) -> R<(&'a str, usize)> {
        let name = self.value();
        let token = self.index;
        self.consume(T::Variable)?;
        Ok((name, token))
    }

    fn parse_optional_description(&mut self, limit_start: bool) -> R<Text> {
        if limit_start {
            if self.is_any(&[T::Union, T::Intersection]) {
                return Err(self.error(T::Other, None));
            }
            if !self.is_any(&[T::PhpdocEol, T::ClosePhpdoc, T::End]) && !self.preceded_by_space() {
                return Err(self.error(T::HorizontalWs, None));
            }
        }
        Ok(self.parse_text())
    }

    // `TypeParser`.

    fn parse_type(&mut self) -> R<Id> {
        self.enter()?;
        let result = self.type_inner();
        self.leave(result)
    }

    fn type_inner(&mut self) -> R<Id> {
        let start = self.index;
        let ty = if self.is(T::Nullable) {
            self.parse_nullable()?
        } else {
            let ty = self.parse_atomic()?;
            self.push();
            self.skip_lines();
            match self.union_or_intersection(ty) {
                Ok(Some(enriched)) => {
                    self.drop_save();
                    enriched
                }
                Ok(None) | Err(_) => {
                    self.rollback();
                    self.union_or_intersection(ty)?.unwrap_or(ty)
                }
            }
        };
        Ok(self.enrich_type(ty, start))
    }

    fn union_or_intersection(&mut self, ty: Id) -> R<Option<Id>> {
        if self.is(T::Union) {
            return self.parse_list(ty, T::Union, "union_type").map(Some);
        }
        if self.is(T::Intersection) {
            return self
                .parse_list(ty, T::Intersection, "intersection_type")
                .map(Some);
        }
        Ok(None)
    }

    /// `parseUnion`, `parseIntersection`.
    fn parse_list(&mut self, first: Id, separator: T, kind: &'static str) -> R<Id> {
        let mut types = vec![first];
        while self.try_consume(separator) {
            types.push(self.parse_atomic()?);
            self.push();
            self.skip_lines();
            if !self.is(separator) {
                self.rollback();
                break;
            }
            self.drop_save();
        }
        let id = self.add(kind);
        for ty in types {
            self.child(id, "types", ty);
        }
        Ok(id)
    }

    /// `subParseUnion`, `subParseIntersection`.
    fn sub_parse_list(&mut self, first: Id, separator: T, kind: &'static str) -> R<Id> {
        let mut types = vec![first];
        while self.try_consume(separator) {
            self.skip_lines();
            types.push(self.parse_atomic()?);
            self.skip_lines();
        }
        let id = self.add(kind);
        for ty in types {
            self.child(id, "types", ty);
        }
        Ok(id)
    }

    fn sub_parse(&mut self) -> R<Id> {
        self.enter()?;
        let result = self.sub_parse_inner();
        self.leave(result)
    }

    fn sub_parse_inner(&mut self) -> R<Id> {
        let start = self.index;
        let ty = if self.is(T::Nullable) {
            self.parse_nullable()?
        } else if self.is(T::Variable) {
            self.parse_conditional_for_parameter()?
        } else {
            let ty = self.parse_atomic()?;
            if self.value() == "is" {
                self.parse_conditional(ty)?
            } else {
                self.skip_lines();
                if self.is(T::Union) {
                    self.sub_parse_list(ty, T::Union, "union_type")?
                } else if self.is(T::Intersection) {
                    self.sub_parse_list(ty, T::Intersection, "intersection_type")?
                } else {
                    ty
                }
            }
        };
        Ok(self.enrich_type(ty, start))
    }

    fn parse_atomic(&mut self) -> R<Id> {
        self.enter()?;
        let result = self.atomic();
        self.leave(result)
    }

    fn atomic(&mut self) -> R<Id> {
        let start = self.index;
        if self.try_consume(T::OpenParentheses) {
            self.skip_lines();
            let mut ty = self.sub_parse()?;
            self.skip_lines();
            self.consume(T::CloseParentheses)?;
            if self.is(T::OpenSquareBracket) {
                ty = self.try_parse_array_or_offset(ty);
            }
            return Ok(self.enrich_type(ty, start));
        }
        if self.try_consume(T::ThisVariable) {
            let this = self.add("this_type");
            let mut ty = self.enrich_type(this, start);
            if self.is(T::OpenSquareBracket) {
                ty = self.try_parse_array_or_offset(ty);
            }
            return Ok(self.enrich_type(ty, start));
        }
        let name = self.value();
        self.push();
        if self.try_consume(T::Identifier) {
            let identifier = self.identifier(name);
            let mut ty = self.enrich_type(identifier, start);
            if !self.is(T::DoubleColon) {
                self.drop_save();
                if self.is(T::OpenAngleBracket) {
                    self.push();
                    let html = self.is_html();
                    self.rollback();
                    if html {
                        return Ok(ty);
                    }
                    let original = ty;
                    ty = self.try_parse_callable(ty, true);
                    if ty == original {
                        ty = self.parse_generic(ty)?;
                        if self.is(T::OpenSquareBracket) {
                            ty = self.try_parse_array_or_offset(ty);
                        }
                    }
                } else if self.is(T::OpenParentheses) {
                    ty = self.try_parse_callable(ty, false);
                } else if self.is(T::OpenSquareBracket) {
                    ty = self.try_parse_array_or_offset(ty);
                } else if SHAPES.contains(&name)
                    && self.is(T::OpenCurlyBracket)
                    && !self.preceded_by_space()
                {
                    ty = if name == "object" {
                        self.parse_object_shape()?
                    } else {
                        self.parse_array_shape(name)?
                    };
                    if self.is(T::OpenSquareBracket) {
                        let shape = self.enrich_type(ty, start);
                        ty = self.try_parse_array_or_offset(shape);
                    }
                }
                return Ok(self.enrich_type(ty, start));
            }
            self.rollback();
        } else {
            self.drop_save();
        }
        self.parse_const_type(start)
    }

    /// A constant expression as a type (`ConstTypeNode`), from `start`.
    fn parse_const_type(&mut self, start: usize) -> R<Id> {
        let before = self.error(T::Identifier, None);
        let expression = self.parse_const(false)?;
        if self.nodes[expression].kind == "const_expr_array" {
            return Err(before);
        }
        let id = self.add("const_type");
        self.child(id, "constExpr", expression);
        let mut ty = self.enrich_type(id, start);
        if self.is(T::OpenSquareBracket) {
            ty = self.try_parse_array_or_offset(ty);
        }
        Ok(ty)
    }

    fn parse_conditional(&mut self, subject: Id) -> R<Id> {
        self.consume(T::Identifier)?;
        let id = self.add("conditional_type");
        self.child(id, "subjectType", subject);
        self.parse_condition(id)
    }

    fn parse_conditional_for_parameter(&mut self) -> R<Id> {
        let name = self.value();
        let token = self.index;
        self.consume(T::Variable)?;
        self.consume_value(T::Identifier, "is")?;
        let id = self.add("conditional_type_for_parameter");
        self.str_prop(id, "parameterName", name);
        self.leaf(id, "variable", token, name);
        self.parse_condition(id)
    }

    /// The part of a conditional type after `is`.
    fn parse_condition(&mut self, id: Id) -> R<Id> {
        let mut negated = false;
        if self.value() == "not" {
            negated = true;
            self.consume(T::Identifier)?;
        }
        let target = self.parse_type()?;
        self.skip_lines();
        self.consume(T::Nullable)?;
        self.skip_lines();
        let if_type = self.parse_type()?;
        self.skip_lines();
        self.consume(T::Colon)?;
        self.skip_lines();
        let else_type = self.sub_parse()?;
        self.child(id, "targetType", target);
        self.child(id, "if", if_type);
        self.child(id, "else", else_type);
        self.prop(id, "negated", Prop::Bool(negated));
        Ok(id)
    }

    fn parse_nullable(&mut self) -> R<Id> {
        self.consume(T::Nullable)?;
        let ty = self.parse_atomic()?;
        let id = self.add("nullable_type");
        self.child(id, "type", ty);
        Ok(id)
    }

    fn is_html(&mut self) -> bool {
        if self.consume(T::OpenAngleBracket).is_err() || !self.is(T::Identifier) {
            return false;
        }
        let name = self.value();
        self.next();
        if !self.try_consume(T::CloseAngleBracket) {
            return false;
        }
        let end_tag = format!("</{name}>");
        let close = format!("/{name}>");
        while !self.is(T::End) {
            if (self.try_consume(T::OpenAngleBracket) && self.value().contains(close.as_str()))
                || self.value().ends_with(end_tag.as_str())
            {
                return true;
            }
            self.next();
        }
        false
    }

    pub(super) fn parse_generic(&mut self, base: Id) -> R<Id> {
        self.consume(T::OpenAngleBracket)?;
        self.skip_lines();
        let start = self.nodes[base].start;
        let mut types = Vec::new();
        let mut variances = Vec::new();
        let mut first = true;
        while first || self.try_consume(T::Comma) {
            self.skip_lines();
            if !first && self.is(T::CloseAngleBracket) {
                break;
            }
            first = false;
            let (ty, variance) = self.parse_generic_argument()?;
            types.push(ty);
            variances.push(variance);
            self.skip_lines();
        }
        let id = self.add("generic_type");
        self.child(id, "type", base);
        for ty in types {
            self.child(id, "genericTypes", ty);
        }
        self.prop(id, "variances", Prop::Strs(variances));
        match start {
            Some(start) => {
                self.enrich_type(id, start);
            }
            None => self.comments = 0,
        }
        self.consume(T::CloseAngleBracket)?;
        Ok(id)
    }

    fn parse_generic_argument(&mut self) -> R<(Id, &'static str)> {
        let start = self.index;
        if self.try_consume(T::Wildcard) {
            let mixed = self.identifier("mixed");
            return Ok((self.enrich_type(mixed, start), "bivariant"));
        }
        let variance = if self.try_consume_value("contravariant") {
            "contravariant"
        } else if self.try_consume_value("covariant") {
            "covariant"
        } else {
            "invariant"
        };
        Ok((self.parse_type()?, variance))
    }

    fn parse_template_tag_value(&mut self, described: bool) -> R<Id> {
        let name = self.value();
        let name_token = self.index;
        self.consume(T::Identifier)?;
        let mut bound = None;
        let mut lower = None;
        if self.try_consume_value("of") || self.try_consume_value("as") {
            bound = Some(self.parse_type()?);
        }
        if self.try_consume_value("super") {
            lower = Some(self.parse_type()?);
        }
        let default = if self.try_consume_value("=") {
            Some(self.parse_type()?)
        } else {
            None
        };
        let text = if described {
            self.parse_optional_description(true)?
        } else {
            Text {
                value: String::new(),
                span: 0..0,
            }
        };
        let id = self.add("template_tag_value");
        self.str_prop(id, "name", name);
        self.leaf(id, "name", name_token, name);
        self.opt_child(id, "bound", bound);
        self.opt_child(id, "default", default);
        self.opt_child(id, "lowerBound", lower);
        self.description(id, "description", text);
        Ok(id)
    }

    fn parse_callable(&mut self, identifier: Id, templated: bool) -> R<Id> {
        let templates = if templated {
            self.parse_callable_templates()?
        } else {
            Vec::new()
        };
        self.consume(T::OpenParentheses)?;
        self.skip_lines();
        let mut parameters = Vec::new();
        if !self.is(T::CloseParentheses) {
            parameters.push(self.parse_callable_parameter()?);
            self.skip_lines();
            while self.try_consume(T::Comma) {
                self.skip_lines();
                if self.is(T::CloseParentheses) {
                    break;
                }
                parameters.push(self.parse_callable_parameter()?);
                self.skip_lines();
            }
        }
        self.consume(T::CloseParentheses)?;
        self.consume(T::Colon)?;
        let start = self.index;
        let returned = self.parse_callable_return_type()?;
        let returned = self.enrich_type(returned, start);
        let id = self.add("callable_type");
        self.child(id, "identifier", identifier);
        for template in templates {
            self.child(id, "templateTypes", template);
        }
        for parameter in parameters {
            self.child(id, "parameters", parameter);
        }
        self.child(id, "returnType", returned);
        Ok(id)
    }

    fn parse_callable_templates(&mut self) -> R<Vec<Id>> {
        self.consume(T::OpenAngleBracket)?;
        let mut templates = Vec::new();
        let mut first = true;
        while first || self.try_consume(T::Comma) {
            self.skip_lines();
            if !first && self.is(T::CloseAngleBracket) {
                break;
            }
            first = false;
            let start = self.index;
            let template = self.parse_template_tag_value(false)?;
            templates.push(self.enrich_type(template, start));
            self.skip_lines();
        }
        self.consume(T::CloseAngleBracket)?;
        Ok(templates)
    }

    fn parse_callable_parameter(&mut self) -> R<Id> {
        let start = self.index;
        let ty = self.parse_type()?;
        let reference = self.try_consume(T::Reference);
        let variadic = self.try_consume(T::Variadic);
        let mut variable = None;
        if self.is(T::Variable) {
            variable = Some((self.value(), self.index));
            self.consume(T::Variable)?;
        }
        let optional = self.try_consume(T::Equal);
        let id = self.add("callable_type_parameter");
        self.child(id, "type", ty);
        self.prop(id, "isReference", Prop::Bool(reference));
        self.prop(id, "isVariadic", Prop::Bool(variadic));
        let name = variable.map_or("", |(name, token)| {
            self.leaf(id, "variable", token, name);
            name
        });
        self.str_prop(id, "parameterName", name);
        self.prop(id, "isOptional", Prop::Bool(optional));
        Ok(self.enrich_type(id, start))
    }

    fn parse_callable_return_type(&mut self) -> R<Id> {
        let start = self.index;
        if self.is(T::Nullable) {
            return self.parse_nullable();
        }
        if self.try_consume(T::OpenParentheses) {
            let mut ty = self.sub_parse()?;
            self.consume(T::CloseParentheses)?;
            if self.is(T::OpenSquareBracket) {
                ty = self.try_parse_array_or_offset(ty);
            }
            return Ok(ty);
        }
        if self.try_consume(T::ThisVariable) {
            let mut ty = self.add("this_type");
            if self.is(T::OpenSquareBracket) {
                let this = self.enrich_type(ty, start);
                ty = self.try_parse_array_or_offset(this);
            }
            return Ok(ty);
        }
        let name = self.value();
        // phpdoc-parser keeps this savepoint when the identifier is the type.
        self.push();
        if self.try_consume(T::Identifier) {
            let mut ty = self.identifier(name);
            if !self.is(T::DoubleColon) {
                if self.is(T::OpenAngleBracket) {
                    let base = self.enrich_type(ty, start);
                    ty = self.parse_generic(base)?;
                    if self.is(T::OpenSquareBracket) {
                        let generic = self.enrich_type(ty, start);
                        ty = self.try_parse_array_or_offset(generic);
                    }
                } else if self.is(T::OpenSquareBracket) {
                    let identifier = self.enrich_type(ty, start);
                    ty = self.try_parse_array_or_offset(identifier);
                } else if SHAPES.contains(&name)
                    && self.is(T::OpenCurlyBracket)
                    && !self.preceded_by_space()
                {
                    ty = if name == "object" {
                        self.parse_object_shape()?
                    } else {
                        self.enrich_type(ty, start);
                        self.parse_array_shape(name)?
                    };
                    if self.is(T::OpenSquareBracket) {
                        let shape = self.enrich_type(ty, start);
                        ty = self.try_parse_array_or_offset(shape);
                    }
                }
                return Ok(ty);
            }
            self.rollback();
        } else {
            self.drop_save();
        }
        self.parse_const_type(start)
    }

    fn try_parse_callable(&mut self, identifier: Id, templated: bool) -> Id {
        self.push();
        match self.parse_callable(identifier, templated) {
            Ok(ty) => {
                self.drop_save();
                ty
            }
            Err(_) => {
                self.rollback();
                identifier
            }
        }
    }

    fn try_parse_array_or_offset(&mut self, mut ty: Id) -> Id {
        let start = self.nodes[ty].start;
        while self.is(T::OpenSquareBracket) {
            self.push();
            let offset_access = !self.preceded_by_space();
            self.next();
            if offset_access && !self.is(T::CloseSquareBracket) {
                let offset = (|| {
                    let offset = self.parse_type()?;
                    self.consume(T::CloseSquareBracket)?;
                    Ok::<_, Error>(offset)
                })();
                let Ok(offset) = offset else {
                    self.rollback();
                    return ty;
                };
                self.drop_save();
                let id = self.add("offset_access_type");
                self.child(id, "type", ty);
                self.child(id, "offset", offset);
                ty = id;
            } else {
                if self.consume(T::CloseSquareBracket).is_err() {
                    self.rollback();
                    return ty;
                }
                self.drop_save();
                let id = self.add("array_type");
                self.child(id, "type", ty);
                ty = id;
            }
            if let Some(start) = start {
                self.enrich_type(ty, start);
            }
        }
        ty
    }

    fn parse_array_shape(&mut self, kind: &'a str) -> R<Id> {
        self.consume(T::OpenCurlyBracket)?;
        let mut items = Vec::new();
        let mut sealed = true;
        let mut unsealed = None;
        let mut closed = false;
        let mut done = false;
        loop {
            self.skip_lines();
            if self.try_consume(T::CloseCurlyBracket) {
                closed = true;
                break;
            }
            if self.try_consume(T::Variadic) {
                sealed = false;
                self.skip_lines();
                if self.is(T::OpenAngleBracket) {
                    unsealed = Some(self.parse_unsealed(kind == "array")?);
                    self.skip_lines();
                }
                self.try_consume(T::Comma);
                break;
            }
            items.push(self.parse_array_shape_item()?);
            self.skip_lines();
            if !self.try_consume(T::Comma) {
                done = true;
            }
            if self.is(T::Comment) {
                self.next();
            }
            if done {
                break;
            }
        }
        if !closed {
            self.skip_lines();
            self.consume(T::CloseCurlyBracket)?;
        }
        let id = self.add("array_shape");
        for item in items {
            self.child(id, "items", item);
        }
        self.prop(id, "sealed", Prop::Bool(sealed));
        self.str_prop(id, "kind", kind);
        self.opt_child(id, "unsealedType", unsealed);
        Ok(id)
    }

    fn parse_array_shape_item(&mut self) -> R<Id> {
        let start = self.index;
        self.skip_lines();
        self.push();
        let keyed = (|| {
            let key = self.parse_array_shape_key()?;
            let optional = self.try_consume(T::Nullable);
            self.consume(T::Colon)?;
            let value = self.parse_type()?;
            Ok::<_, Error>((key, optional, value))
        })();
        let id = match keyed {
            Ok((key, optional, value)) => {
                self.drop_save();
                let id = self.add("array_shape_item");
                self.child(id, "keyName", key);
                self.prop(id, "optional", Prop::Bool(optional));
                self.child(id, "valueType", value);
                id
            }
            Err(_) => {
                self.rollback();
                let value = self.parse_type()?;
                let id = self.add("array_shape_item");
                self.prop(id, "optional", Prop::Bool(false));
                self.child(id, "valueType", value);
                id
            }
        };
        Ok(self.enrich_type(id, start))
    }

    fn parse_array_shape_key(&mut self) -> R<Id> {
        let start = self.index;
        let key = if self.is(T::Integer) {
            let key = self.add("const_expr_integer");
            let value = self.value().replace('_', "");
            self.str_prop(key, "value", &value);
            self.next();
            key
        } else if self.is_any(&[T::SingleQuotedString, T::DoubleQuotedString]) {
            self.quoted_key()
        } else {
            let name = self.value();
            self.consume(T::Identifier)?;
            if self.try_consume(T::DoubleColon) {
                let constant = self.value();
                self.consume(T::Identifier)?;
                let key = self.add("const_fetch");
                self.str_prop(key, "className", name);
                self.str_prop(key, "name", constant);
                key
            } else {
                self.identifier(name)
            }
        };
        Ok(self.enrich_type(key, start))
    }

    /// A quoted array or object shape key (`ConstExprStringNode`), consumed.
    fn quoted_key(&mut self) -> Id {
        let quote = if self.is(T::SingleQuotedString) { 1 } else { 2 };
        let key = self.add("const_expr_string");
        let value = unescape_string(self.value());
        self.prop(key, "value", Prop::Str(value));
        self.prop(key, "quoteType", Prop::Int(quote));
        self.next();
        key
    }

    /// `parseArrayShapeUnsealedType` (`keyed`) and `parseListShapeUnsealedType`.
    fn parse_unsealed(&mut self, keyed: bool) -> R<Id> {
        let start = self.index;
        self.consume(T::OpenAngleBracket)?;
        self.skip_lines();
        let mut value = self.parse_type()?;
        self.skip_lines();
        let mut key = None;
        if keyed && self.try_consume(T::Comma) {
            self.skip_lines();
            key = Some(value);
            value = self.parse_type()?;
            self.skip_lines();
        }
        self.consume(T::CloseAngleBracket)?;
        let id = self.add("array_shape_unsealed_type");
        self.child(id, "valueType", value);
        self.opt_child(id, "keyType", key);
        Ok(self.enrich_type(id, start))
    }

    fn parse_object_shape(&mut self) -> R<Id> {
        self.consume(T::OpenCurlyBracket)?;
        let mut items = Vec::new();
        let mut closed = false;
        loop {
            self.skip_lines();
            if self.try_consume(T::CloseCurlyBracket) {
                closed = true;
                break;
            }
            items.push(self.parse_object_shape_item()?);
            self.skip_lines();
            if !self.try_consume(T::Comma) {
                break;
            }
        }
        if !closed {
            self.skip_lines();
            self.consume(T::CloseCurlyBracket)?;
        }
        let id = self.add("object_shape");
        for item in items {
            self.child(id, "items", item);
        }
        Ok(id)
    }

    fn parse_object_shape_item(&mut self) -> R<Id> {
        let start = self.index;
        self.skip_lines();
        let key_start = self.index;
        let key = if self.is_any(&[T::SingleQuotedString, T::DoubleQuotedString]) {
            self.quoted_key()
        } else {
            let name = self.value();
            self.consume(T::Identifier)?;
            self.identifier(name)
        };
        let key = self.enrich_type(key, key_start);
        let optional = self.try_consume(T::Nullable);
        self.consume(T::Colon)?;
        let value = self.parse_type()?;
        let id = self.add("object_shape_item");
        self.child(id, "keyName", key);
        self.prop(id, "optional", Prop::Bool(optional));
        self.child(id, "valueType", value);
        Ok(self.enrich_type(id, start))
    }

    // `ConstExprParser` (`doctrine`: the one `toDoctrine` makes).

    fn parse_const(&mut self, doctrine: bool) -> R<Id> {
        self.enter()?;
        let result = self.constant(doctrine);
        self.leave(result)
    }

    fn constant(&mut self, doctrine: bool) -> R<Id> {
        let start = self.index;
        if self.is_any(&[T::Float, T::Integer]) {
            let kind = if self.is(T::Float) {
                "const_expr_float"
            } else {
                "const_expr_integer"
            };
            let value = self.value().replace('_', "");
            self.next();
            let id = self.add(kind);
            self.str_prop(id, "value", &value);
            return Ok(self.enrich(id, start));
        }
        if doctrine && self.is(T::DoctrineAnnotationString) {
            let value = self.value();
            self.next();
            let id = self.add("doctrine_const_expr_string");
            self.prop(id, "value", Prop::Str(unescape_doctrine(value.as_bytes())));
            self.prop(id, "quoteType", Prop::Int(2));
            return Ok(self.enrich(id, start));
        }
        if self.is_any(&[T::SingleQuotedString, T::DoubleQuotedString]) {
            if doctrine {
                if self.is(T::SingleQuotedString) {
                    return Err(self.error(T::DoubleQuotedString, None));
                }
                let value = self.value();
                self.next();
                let id = self.parse_doctrine_string(value);
                return Ok(self.enrich(id, start));
            }
            let id = self.quoted_key();
            return Ok(self.enrich(id, start));
        }
        if self.is(T::Identifier) {
            let identifier = self.value();
            self.next();
            let constant = match identifier.to_ascii_lowercase().as_str() {
                "true" => Some("const_expr_true"),
                "false" => Some("const_expr_false"),
                "null" => Some("const_expr_null"),
                "array" => {
                    self.consume(T::OpenParentheses)?;
                    return self.parse_const_array(T::CloseParentheses, start, doctrine);
                }
                _ => None,
            };
            if let Some(kind) = constant {
                let id = self.add(kind);
                return Ok(self.enrich(id, start));
            }
            let id = self.add("const_fetch");
            if self.try_consume(T::DoubleColon) {
                let mut name = String::new();
                let mut last = None;
                loop {
                    if last != Some(T::Identifier) && self.is(T::Identifier) {
                        name.push_str(self.value());
                        self.consume(T::Identifier)?;
                        last = Some(T::Identifier);
                        continue;
                    }
                    if last != Some(T::Wildcard) && self.try_consume(T::Wildcard) {
                        name.push('*');
                        last = Some(T::Wildcard);
                        if !self.skipped_space().is_empty() {
                            break;
                        }
                        continue;
                    }
                    if last.is_none() {
                        self.consume(T::Wildcard)?;
                    }
                    break;
                }
                self.str_prop(id, "className", identifier);
                self.str_prop(id, "name", &name);
            } else {
                self.str_prop(id, "className", "");
                self.str_prop(id, "name", identifier);
            }
            return Ok(self.enrich(id, start));
        }
        if self.try_consume(T::OpenSquareBracket) {
            return self.parse_const_array(T::CloseSquareBracket, start, doctrine);
        }
        Err(self.error(T::Identifier, None))
    }

    fn parse_const_array(&mut self, end: T, start: usize, doctrine: bool) -> R<Id> {
        let mut items = Vec::new();
        if !self.try_consume(end) {
            loop {
                items.push(self.parse_const_array_item(doctrine)?);
                if !self.try_consume(T::Comma) || self.is(end) {
                    break;
                }
            }
            self.consume(end)?;
        }
        let id = self.add("const_expr_array");
        for item in items {
            self.child(id, "items", item);
        }
        Ok(self.enrich(id, start))
    }

    fn parse_const_array_item(&mut self, doctrine: bool) -> R<Id> {
        let start = self.index;
        let expression = self.parse_const(doctrine)?;
        let (key, value) = if self.try_consume(T::DoubleArrow) {
            (Some(expression), self.parse_const(doctrine)?)
        } else {
            (None, expression)
        };
        let id = self.add("const_expr_array_item");
        self.opt_child(id, "key", key);
        self.child(id, "value", value);
        Ok(self.enrich(id, start))
    }

    /// `parseDoctrineString`: `text` and the double-quoted strings after it, as one.
    fn parse_doctrine_string(&mut self, text: &str) -> Id {
        let mut joined = text.as_bytes().to_vec();
        while self.is_any(&[T::DoubleQuotedString, T::DoctrineAnnotationString]) {
            joined.extend_from_slice(self.value().as_bytes());
            self.next();
        }
        let id = self.add("doctrine_const_expr_string");
        self.prop(id, "value", Prop::Str(unescape_doctrine(&joined)));
        self.prop(id, "quoteType", Prop::Int(2));
        id
    }
}

/// `str_replace($search, $replace, $subject)` for one pair, left to right.
fn replace(subject: &[u8], search: &[u8], replacement: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(subject.len());
    let mut at = 0;
    while at < subject.len() {
        if subject[at..].starts_with(search) {
            out.extend_from_slice(replacement);
            at += search.len();
        } else {
            out.push(subject[at]);
            at += 1;
        }
    }
    out
}

/// `DoctrineConstExprStringNode::unescape`.
fn unescape_doctrine(value: &[u8]) -> Vec<u8> {
    let inner = value
        .get(1..value.len().saturating_sub(1))
        .unwrap_or_default();
    replace(inner, b"\"\"", b"\"")
}

/// `StringUnescaper::unescapeString` over a quoted token.
fn unescape_string(token: &str) -> Vec<u8> {
    let bytes = token.as_bytes();
    let inner = &bytes[1..bytes.len() - 1];
    if bytes[0] == b'\'' {
        return replace(&replace(inner, b"\\\\", b"\\"), b"\\'", b"'");
    }
    let s = replace(inner, b"\\\"", b"\"");
    let mut out = Vec::with_capacity(s.len());
    let mut at = 0;
    while at < s.len() {
        if s[at] != b'\\' || at + 1 >= s.len() {
            out.push(s[at]);
            at += 1;
            continue;
        }
        let next = s[at + 1];
        let simple = match next {
            b'\\' => Some(b'\\'),
            b'n' => Some(b'\n'),
            b'r' => Some(b'\r'),
            b't' => Some(b'\t'),
            b'f' => Some(0x0c),
            b'v' => Some(0x0b),
            b'e' => Some(0x1b),
            _ => None,
        };
        if let Some(byte) = simple {
            out.push(byte);
            at += 2;
            continue;
        }
        let hex = |from: usize, max: usize| {
            s[from..]
                .iter()
                .take(max)
                .take_while(|b| b.is_ascii_hexdigit())
                .count()
        };
        if matches!(next, b'x' | b'X') {
            let digits = hex(at + 2, 2);
            if digits > 0 {
                let text = std::str::from_utf8(&s[at + 2..at + 2 + digits]).unwrap_or("0");
                out.push(u8::from_str_radix(text, 16).unwrap_or(0));
                at += 2 + digits;
                continue;
            }
        }
        let octal = s[at + 1..]
            .iter()
            .take(3)
            .take_while(|b| (b'0'..=b'7').contains(*b))
            .count();
        if octal > 0 {
            let value = s[at + 1..at + 1 + octal]
                .iter()
                .fold(0u32, |value, digit| value * 8 + u32::from(digit - b'0'));
            out.push((value & 0xff) as u8);
            at += 1 + octal;
            continue;
        }
        if next == b'u' && s.get(at + 2) == Some(&b'{') {
            let digits = hex(at + 3, usize::MAX);
            if digits > 0 && s.get(at + 3 + digits) == Some(&b'}') {
                let text = std::str::from_utf8(&s[at + 3..at + 3 + digits]).unwrap_or("");
                let code = u64::from_str_radix(text, 16).unwrap_or(u64::MAX);
                code_point(code, &mut out);
                at += 4 + digits;
                continue;
            }
        }
        out.push(s[at]);
        at += 1;
    }
    out
}

/// `StringUnescaper::codePointToUtf8`.
fn code_point(code: u64, out: &mut Vec<u8>) {
    let byte = |value: u64| (value & 0xff) as u8;
    if code <= 0x7f {
        out.push(byte(code));
    } else if code <= 0x7ff {
        out.extend([byte((code >> 6) + 0xc0), byte((code & 0x3f) + 0x80)]);
    } else if code <= 0xffff {
        out.extend([
            byte((code >> 12) + 0xe0),
            byte(((code >> 6) & 0x3f) + 0x80),
            byte((code & 0x3f) + 0x80),
        ]);
    } else if code <= 0x1f_ffff {
        out.extend([
            byte((code >> 18) + 0xf0),
            byte(((code >> 12) & 0x3f) + 0x80),
            byte(((code >> 6) & 0x3f) + 0x80),
            byte((code & 0x3f) + 0x80),
        ]);
    } else {
        out.extend_from_slice("\u{fffd}".as_bytes());
    }
}
