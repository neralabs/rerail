use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
enum TokenKind {
    Open,
    Close,
    AltOpen,
    AltClose,
    Dot,
    Quant(char),
    Word(String),
    String(String),
    Capture(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Token {
    kind: TokenKind,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug)]
enum Form {
    Kind { name: String, named: bool },
    Named,
    Any,
    Missing(Option<(String, bool)>),
    Alternative(Vec<Pattern>),
    Group,
}

#[derive(Clone, Debug)]
struct Pattern {
    form: Form,
    children: Vec<Item>,
    captures: Vec<String>,
    quantifier: Option<char>,
    branch_field: Option<String>,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug)]
struct Item {
    pattern: Pattern,
    field: Option<String>,
    negated_field: Option<String>,
    anchor_before: bool,
    anchor_after: bool,
}

#[derive(Clone, Debug)]
enum Argument {
    Capture(String),
    Text(String),
}

#[derive(Clone, Debug)]
struct Predicate {
    name: String,
    arguments: Vec<Argument>,
}

#[derive(Clone, Debug)]
pub(super) struct QueryPattern {
    pattern: Pattern,
    predicates: Vec<Predicate>,
    source: String,
}

impl QueryPattern {
    pub(super) fn source(&self) -> &str {
        &self.source
    }
}

fn parse_error(offset: usize, message: impl Into<String>) -> Error {
    Error::ScmParse {
        offset,
        message: message.into(),
    }
}

fn lex(source: &str) -> Result<Vec<Token>, Error> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            byte if byte.is_ascii_whitespace() => at += 1,
            b';' => {
                at += 1;
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
            }
            b'(' | b')' | b'[' | b']' | b'.' | b'?' | b'*' | b'+' => {
                let start = at;
                let kind = match bytes[at] {
                    b'(' => TokenKind::Open,
                    b')' => TokenKind::Close,
                    b'[' => TokenKind::AltOpen,
                    b']' => TokenKind::AltClose,
                    b'.' => TokenKind::Dot,
                    byte => TokenKind::Quant(byte as char),
                };
                at += 1;
                tokens.push(Token {
                    kind,
                    start,
                    end: at,
                });
            }
            b'"' => {
                let start = at;
                at += 1;
                let mut value = String::new();
                let mut closed = false;
                while at < bytes.len() {
                    let byte = bytes[at];
                    at += 1;
                    match byte {
                        b'"' => {
                            closed = true;
                            break;
                        }
                        b'\\' => {
                            let Some(&escaped) = bytes.get(at) else {
                                return Err(parse_error(at, "truncated string escape"));
                            };
                            at += 1;
                            value.push(match escaped {
                                b'n' => '\n',
                                b'r' => '\r',
                                b't' => '\t',
                                b'0' => '\0',
                                b'"' => '"',
                                b'\\' => '\\',
                                other if other.is_ascii() => other as char,
                                _ => return Err(parse_error(at - 1, "non-ASCII escaped byte")),
                            });
                        }
                        other if other.is_ascii() => value.push(other as char),
                        _ => {
                            let rest = &source[at - 1..];
                            let character = rest
                                .chars()
                                .next()
                                .ok_or_else(|| parse_error(at - 1, "invalid UTF-8 string"))?;
                            value.push(character);
                            at += character.len_utf8() - 1;
                        }
                    }
                }
                if !closed {
                    return Err(parse_error(start, "unterminated string"));
                }
                tokens.push(Token {
                    kind: TokenKind::String(value),
                    start,
                    end: at,
                });
            }
            _ => {
                let start = at;
                while at < bytes.len()
                    && !bytes[at].is_ascii_whitespace()
                    && !matches!(bytes[at], b'(' | b')' | b'[' | b']' | b';')
                    && (bytes[start] == b'#' || !matches!(bytes[at], b'?' | b'*' | b'+'))
                {
                    at += 1;
                }
                if at == start {
                    return Err(parse_error(at, "unexpected query byte"));
                }
                let text = source[start..at].to_owned();
                let kind = text.strip_prefix('@').map_or_else(
                    || TokenKind::Word(text.clone()),
                    |name| TokenKind::Capture(name.to_owned()),
                );
                tokens.push(Token {
                    kind,
                    start,
                    end: at,
                });
            }
        }
    }
    Ok(tokens)
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    at: usize,
    predicates: Vec<Predicate>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Result<Token, Error> {
        let token = self
            .tokens
            .get(self.at)
            .cloned()
            .ok_or_else(|| parse_error(self.source.len(), "unexpected end of query"))?;
        self.at += 1;
        Ok(token)
    }

    fn parse_all(mut self) -> Result<Vec<QueryPattern>, Error> {
        let mut patterns = Vec::new();
        while self.at < self.tokens.len() {
            let predicate_start = self.predicates.len();
            let pattern = self.pattern()?;
            let predicates = self.predicates.split_off(predicate_start);
            let source = self.source[pattern.start..pattern.end].trim().to_owned();
            patterns.push(QueryPattern {
                pattern,
                predicates,
                source,
            });
        }
        Ok(patterns)
    }

    fn predicate(&mut self) -> Result<(), Error> {
        let open = self.next()?;
        debug_assert_eq!(open.kind, TokenKind::Open);
        let name_token = self.next()?;
        let TokenKind::Word(name) = name_token.kind else {
            return Err(parse_error(name_token.start, "expected predicate name"));
        };
        if !name.starts_with('#') {
            return Err(parse_error(name_token.start, "expected predicate name"));
        }
        let mut arguments = Vec::new();
        loop {
            let token = self.next()?;
            match token.kind {
                TokenKind::Close => break,
                TokenKind::Capture(name) => arguments.push(Argument::Capture(name)),
                TokenKind::String(text) | TokenKind::Word(text) => {
                    arguments.push(Argument::Text(text));
                }
                _ => return Err(parse_error(token.start, "invalid predicate argument")),
            }
        }
        self.predicates.push(Predicate { name, arguments });
        Ok(())
    }

    fn body(&mut self) -> Result<(Vec<Item>, usize), Error> {
        let mut children: Vec<Item> = Vec::new();
        let mut anchor_before = false;
        loop {
            let Some(token) = self.peek().cloned() else {
                return Err(parse_error(self.source.len(), "unterminated list"));
            };
            match token.kind {
                TokenKind::Close => {
                    self.at += 1;
                    return Ok((children, token.end));
                }
                TokenKind::Dot => {
                    self.at += 1;
                    anchor_before = true;
                    if let Some(previous) = children.last_mut() {
                        previous.anchor_after = true;
                    }
                }
                TokenKind::Open
                    if matches!(
                        self.tokens.get(self.at + 1).map(|token| &token.kind),
                        Some(TokenKind::Word(name)) if name.starts_with('#')
                    ) =>
                {
                    self.predicate()?
                }
                _ => {
                    let mut field = None;
                    let mut negated_field = None;
                    if let TokenKind::Word(word) = &token.kind {
                        if let Some(name) = word.strip_suffix(':') {
                            field = Some(name.to_owned());
                            self.at += 1;
                        } else if let Some(name) = word.strip_prefix('!') {
                            if name.is_empty() {
                                return Err(parse_error(token.start, "empty negated field"));
                            }
                            negated_field = Some(name.to_owned());
                            self.at += 1;
                        }
                    }
                    let pattern = if negated_field.is_some() {
                        Pattern {
                            form: Form::Group,
                            children: Vec::new(),
                            captures: Vec::new(),
                            quantifier: None,
                            branch_field: None,
                            start: token.start,
                            end: token.end,
                        }
                    } else {
                        self.pattern()?
                    };
                    children.push(Item {
                        pattern,
                        field,
                        negated_field,
                        anchor_before,
                        anchor_after: false,
                    });
                    anchor_before = false;
                }
            }
        }
    }

    fn pattern(&mut self) -> Result<Pattern, Error> {
        let first = self.next()?;
        let start = first.start;
        let (form, children, mut end) = match first.kind {
            TokenKind::String(name) => (Form::Kind { name, named: false }, Vec::new(), first.end),
            TokenKind::Word(name) if name == "_" => (Form::Any, Vec::new(), first.end),
            TokenKind::Open => {
                if matches!(self.peek().map(|token| &token.kind), Some(TokenKind::Word(name)) if name == "MISSING")
                {
                    self.at += 1;
                    let token = self.next()?;
                    let (kind, end) = match token.kind {
                        TokenKind::Close => (None, token.end),
                        TokenKind::Word(name) | TokenKind::String(name) => {
                            let named = matches!(self.tokens[self.at - 1].kind, TokenKind::Word(_));
                            let close = self.next()?;
                            if close.kind != TokenKind::Close {
                                return Err(parse_error(
                                    close.start,
                                    "expected end of missing-node pattern",
                                ));
                            }
                            (Some((name, named)), close.end)
                        }
                        _ => {
                            return Err(parse_error(
                                token.start,
                                "expected missing-node kind or closing parenthesis",
                            ));
                        }
                    };
                    (Form::Missing(kind), Vec::new(), end)
                } else {
                    let form = match self.peek().map(|token| &token.kind) {
                        Some(TokenKind::Word(name)) if name == "_" => {
                            self.at += 1;
                            Form::Named
                        }
                        Some(TokenKind::Word(name))
                            if !name.starts_with('#')
                                && !name.ends_with(':')
                                && !name.starts_with('!') =>
                        {
                            let name = name.clone();
                            self.at += 1;
                            Form::Kind { name, named: true }
                        }
                        _ => Form::Group,
                    };
                    let (children, end) = self.body()?;
                    (form, children, end)
                }
            }
            TokenKind::AltOpen => {
                let mut alternatives = Vec::new();
                let close_end = loop {
                    let Some(token) = self.peek() else {
                        return Err(parse_error(start, "unterminated alternative"));
                    };
                    if token.kind == TokenKind::AltClose {
                        let close_end = token.end;
                        self.at += 1;
                        break close_end;
                    }
                    let branch_field = match &token.kind {
                        TokenKind::Word(word) => word.strip_suffix(':').map(str::to_owned),
                        _ => None,
                    };
                    if branch_field.is_some() {
                        self.at += 1;
                    }
                    let mut alternative = self.pattern()?;
                    alternative.branch_field = branch_field;
                    alternatives.push(alternative);
                };
                (Form::Alternative(alternatives), Vec::new(), close_end)
            }
            _ => return Err(parse_error(start, "expected query pattern")),
        };
        let mut captures = Vec::new();
        let mut quantifier = None;
        loop {
            match self.peek().cloned() {
                Some(Token {
                    kind: TokenKind::Capture(name),
                    end: capture_end,
                    ..
                }) => {
                    self.at += 1;
                    captures.push(name);
                    end = capture_end;
                }
                Some(Token {
                    kind: TokenKind::Quant(value),
                    end: quantifier_end,
                    ..
                }) => {
                    self.at += 1;
                    if quantifier.replace(value).is_some() {
                        return Err(parse_error(start, "multiple quantifiers on one pattern"));
                    }
                    end = quantifier_end;
                }
                _ => break,
            }
        }
        Ok(Pattern {
            form,
            children,
            captures,
            quantifier,
            branch_field: None,
            start,
            end,
        })
    }
}

pub(super) fn parse(source: &str) -> Result<Vec<QueryPattern>, Error> {
    Parser {
        source,
        tokens: lex(source)?,
        at: 0,
        predicates: Vec::new(),
    }
    .parse_all()
}

struct Compiler<'a> {
    grammar: &'a tree_sitter::Language,
    words: Vec<u32>,
    strings: Vec<Vec<u8>>,
    string_indexes: BTreeMap<Vec<u8>, u16>,
    labels: Vec<u16>,
    label_indexes: BTreeMap<String, u16>,
    predicates: Vec<TextPredicate>,
    capture_predicates: Vec<CapturePredicate>,
    directives: Vec<Directive>,
    repeat_indexes: BTreeMap<(usize, usize), u16>,
    next_repeat: u16,
    next_cursor: u16,
    sequence_depth: usize,
}

impl<'a> Compiler<'a> {
    fn new(grammar: &'a tree_sitter::Language) -> Self {
        Self {
            grammar,
            words: Vec::new(),
            strings: Vec::new(),
            string_indexes: BTreeMap::new(),
            labels: Vec::new(),
            label_indexes: BTreeMap::new(),
            predicates: Vec::new(),
            capture_predicates: Vec::new(),
            directives: Vec::new(),
            repeat_indexes: BTreeMap::new(),
            next_repeat: 0,
            next_cursor: 0,
            sequence_depth: 0,
        }
    }

    fn string(&mut self, value: &[u8]) -> Result<u16, Error> {
        if let Some(index) = self.string_indexes.get(value) {
            return Ok(*index);
        }
        let index = u16::try_from(self.strings.len()).map_err(|_| Error::UnsupportedPattern)?;
        self.strings.push(value.to_vec());
        self.string_indexes.insert(value.to_vec(), index);
        Ok(index)
    }

    fn label(&mut self, name: &str) -> Result<u16, Error> {
        if let Some(index) = self.label_indexes.get(name) {
            return Ok(*index);
        }
        let string = self.string(name.as_bytes())?;
        let index = u16::try_from(self.labels.len()).map_err(|_| Error::UnsupportedPattern)?;
        self.labels.push(string);
        self.label_indexes.insert(name.to_owned(), index);
        Ok(index)
    }

    fn emit(&mut self, opcode: u8, mode: u8, operand: u16) -> usize {
        let index = self.words.len();
        self.words.push(word(opcode, mode, operand));
        index
    }

    fn patch(&mut self, index: usize, target: usize) -> Result<(), Error> {
        let (opcode, mode, _) = split(self.words[index]);
        let target = u16::try_from(target).map_err(|_| Error::UnsupportedPattern)?;
        self.words[index] = word(opcode, mode, target);
        Ok(())
    }

    fn kind(&self, name: &str, named: bool) -> Result<u16, Error> {
        if name == "ERROR" {
            return Ok(u16::MAX);
        }
        let kind = self.grammar.id_for_node_kind(name, named);
        if kind == 0 {
            return Err(Error::UnknownNodeKind(name.to_owned()));
        }
        Ok(kind)
    }

    fn predicate_kind(&self, name: &str) -> Option<u16> {
        self.kind(name, true)
            .or_else(|_| self.kind(name, false))
            .ok()
    }

    fn field(&self, name: &str) -> Result<u16, Error> {
        self.grammar
            .field_id_for_name(name)
            .map(|field| field.get())
            .ok_or_else(|| Error::UnknownField(name.to_owned()))
    }

    fn pattern(&mut self, pattern: &Pattern) -> Result<(), Error> {
        if pattern.quantifier.is_some() {
            return self.quantified_pattern(pattern);
        }
        self.pattern_atom(pattern)
    }

    fn pattern_atom(&mut self, pattern: &Pattern) -> Result<(), Error> {
        if let Some(field) = &pattern.branch_field {
            self.emit(FIELD, 0, self.field(field)?);
        }
        match &pattern.form {
            Form::Kind { name, named } => {
                if let Some((supertype, subtype)) = named.then(|| name.split_once('/')).flatten() {
                    if supertype.is_empty() || subtype.is_empty() {
                        return Err(Error::UnsupportedPattern);
                    }
                    let kind = self.kind(subtype, true)?;
                    let supertype = self.kind(supertype, true)?;
                    self.emit(KIND, 0, kind);
                    self.emit(SUPERTYPE, 0, supertype);
                } else {
                    let kind = self.kind(name, *named)?;
                    self.emit(KIND, 0, kind);
                }
            }
            Form::Missing(kind) => {
                if let Some((name, named)) = kind {
                    let kind = self.kind(name, *named)?;
                    self.emit(KIND, 0, kind);
                }
                self.emit(FLAGS, MISSING, 0);
            }
            Form::Named => {
                self.emit(FLAGS, NAMED, u16::from(ERROR));
            }
            Form::Any => {
                self.emit(FLAGS, 0, u16::from(ERROR));
            }
            Form::Alternative(alternatives) => self.alternatives(alternatives)?,
            Form::Group => {
                self.sequence(&pattern.children, !pattern.captures.is_empty())?;
                self.captures(&pattern.captures)?;
                return Ok(());
            }
        }
        if !pattern.children.is_empty() {
            let parent = self.cursor_slot()?;
            let last_child = if pattern
                .children
                .iter()
                .any(|child| child.negated_field.is_some())
            {
                self.cursor_slot()?
            } else {
                u16::MAX
            };
            self.emit(SAVE_CURSOR, 0, parent);
            self.nested_sequence_from(
                &pattern.children,
                false,
                false,
                Some((parent, last_child)),
                !pattern.captures.is_empty(),
            )?;
            self.emit(RESTORE_CURSOR, 0, parent);
        }
        self.captures(&pattern.captures)
    }

    fn alternatives(&mut self, alternatives: &[Pattern]) -> Result<(), Error> {
        let Some((first, rest)) = alternatives.split_first() else {
            return Err(Error::UnsupportedPattern);
        };
        if rest.is_empty() {
            return self.pattern(first);
        }
        let fork = self.emit(FORK, 0, 0);
        self.pattern(first)?;
        let done = self.emit(GOTO, 0, 0);
        self.patch(fork, self.words.len())?;
        self.alternatives(rest)?;
        self.patch(done, self.words.len())
    }

    fn sequence(&mut self, items: &[Item], captures_after: bool) -> Result<(), Error> {
        self.nested_sequence_from(items, false, false, None, captures_after)
    }

    fn nested_sequence_from(
        &mut self,
        items: &[Item],
        matched: bool,
        anchor_after: bool,
        child_cursors: Option<(u16, u16)>,
        captures_after: bool,
    ) -> Result<(), Error> {
        self.sequence_depth += 1;
        let result =
            self.sequence_from(items, matched, anchor_after, child_cursors, captures_after);
        self.sequence_depth -= 1;
        result
    }

    fn sequence_from(
        &mut self,
        items: &[Item],
        matched: bool,
        anchor_after: bool,
        child_cursors: Option<(u16, u16)>,
        captures_after: bool,
    ) -> Result<(), Error> {
        let Some((item, rest)) = items.split_first() else {
            if anchor_after {
                self.emit(LAST, NAMED, 0);
            }
            return Ok(());
        };
        if let Some(field) = &item.negated_field {
            if let Some((parent, last_child)) = child_cursors {
                self.emit(RESTORE_CURSOR, 0, parent);
                self.emit(ABSENT_FIELD, 0, self.field(field)?);
                if matched && last_child != u16::MAX {
                    self.emit(RESTORE_CURSOR, 0, last_child);
                }
            } else {
                self.emit(ABSENT_FIELD, 0, self.field(field)?);
            }
            return self.sequence_from(rest, matched, anchor_after, child_cursors, captures_after);
        }
        if let Some(quantifier) = item.pattern.quantifier {
            return self.quantified_item(
                item,
                rest,
                matched,
                anchor_after,
                quantifier,
                child_cursors,
                captures_after,
            );
        }
        self.item_prefix(item, matched, anchor_after, child_cursors)?;
        self.pattern_atom(&item.pattern)?;
        if let Some((_, last_child)) = child_cursors
            && last_child != u16::MAX
        {
            self.emit(SAVE_CURSOR, 0, last_child);
        }
        self.sequence_from(rest, true, item.anchor_after, child_cursors, captures_after)
    }

    fn item_prefix(
        &mut self,
        item: &Item,
        matched: bool,
        anchor_after: bool,
        child_cursors: Option<(u16, u16)>,
    ) -> Result<(), Error> {
        if matched {
            self.emit(
                if item.anchor_before || anchor_after {
                    NEXT
                } else {
                    FOLLOWING
                },
                NAMED,
                0,
            );
        } else if child_cursors.is_some() {
            self.emit(CHILD, 0, 0);
        }
        if let Some(field) = &item.field {
            self.emit(FIELD, 0, self.field(field)?);
        }
        if item.anchor_before && !matched {
            self.emit(FIRST, NAMED, 0);
        }
        Ok(())
    }

    fn cursor_slot(&mut self) -> Result<u16, Error> {
        let slot = self.next_cursor;
        self.next_cursor = self
            .next_cursor
            .checked_add(1)
            .ok_or(Error::UnsupportedPattern)?;
        Ok(slot)
    }

    fn pattern_has_captures(pattern: &Pattern) -> bool {
        !pattern.captures.is_empty()
            || pattern.children.iter().any(|item| {
                item.negated_field.is_none() && Self::pattern_has_captures(&item.pattern)
            })
            || matches!(&pattern.form, Form::Alternative(alternatives) if alternatives.iter().any(Self::pattern_has_captures))
    }

    fn repeat_id(&mut self, pattern: &Pattern) -> Result<u16, Error> {
        let key = (pattern.start, pattern.end);
        if let Some(id) = self.repeat_indexes.get(&key) {
            return Ok(*id);
        }
        let id = self.next_repeat;
        self.next_repeat = self
            .next_repeat
            .checked_add(1)
            .ok_or(Error::UnsupportedPattern)?;
        self.repeat_indexes.insert(key, id);
        Ok(id)
    }

    fn quantified_pattern(&mut self, pattern: &Pattern) -> Result<(), Error> {
        let quantifier = pattern.quantifier.ok_or(Error::UnsupportedPattern)?;
        let id = self.repeat_id(pattern)?;
        self.emit(REPEAT_BEGIN, 0, id);
        let zero = matches!(quantifier, '?' | '*').then(|| self.emit(FORK, 0, 0));
        self.pattern_atom(pattern)?;
        self.emit(REPEAT_BUMP, 0, id);
        if matches!(quantifier, '*' | '+') {
            let loop_start = self.words.len();
            let exit = self.emit(FORK, 0, 0);
            // Native quantifier loop-back seeks the immediate sibling, including anonymous nodes.
            self.emit(NEXT, 0, 0);
            self.pattern_atom(pattern)?;
            self.emit(REPEAT_BUMP, 0, id);
            self.emit(
                GOTO,
                0,
                u16::try_from(loop_start).map_err(|_| Error::UnsupportedPattern)?,
            );
            self.patch(exit, self.words.len())?;
        }
        self.emit(REPEAT_END, 0, id);
        let done = self.emit(GOTO, 0, 0);
        if let Some(zero) = zero {
            self.patch(zero, self.words.len())?;
            self.emit(REPEAT_END, 0, id);
        }
        self.patch(done, self.words.len())
    }

    #[allow(clippy::too_many_arguments)]
    fn quantified_item(
        &mut self,
        item: &Item,
        rest: &[Item],
        matched: bool,
        anchor_after: bool,
        quantifier: char,
        child_cursors: Option<(u16, u16)>,
        captures_after: bool,
    ) -> Result<(), Error> {
        let id = self.repeat_id(&item.pattern)?;
        self.emit(REPEAT_BEGIN, 0, id);
        let zero = matches!(quantifier, '?' | '*').then(|| self.emit(FORK, 0, 0));

        self.item_prefix(item, matched, anchor_after, child_cursors)?;
        self.pattern_atom(&item.pattern)?;
        if let Some((_, last_child)) = child_cursors
            && last_child != u16::MAX
        {
            self.emit(SAVE_CURSOR, 0, last_child);
        }
        if rest.is_empty() && item.anchor_after {
            self.emit(LAST, NAMED, 0);
        }
        self.emit(REPEAT_BUMP, 0, id);

        let exit = if matches!(quantifier, '*' | '+') {
            let loop_start = self.words.len();
            let exit_mode = if self.sequence_depth == 1
                && rest.is_empty()
                && !captures_after
                && !Self::pattern_has_captures(&item.pattern)
            {
                GREEDY
            } else {
                0
            };
            let exit = self.emit(FORK, exit_mode, 0);
            // Repetition advances immediately. Initial sibling selection remains separate.
            self.emit(NEXT, 0, 0);
            self.pattern_atom(&item.pattern)?;
            if let Some((_, last_child)) = child_cursors
                && last_child != u16::MAX
            {
                self.emit(SAVE_CURSOR, 0, last_child);
            }
            if rest.is_empty() && item.anchor_after {
                self.emit(LAST, NAMED, 0);
            }
            self.emit(REPEAT_BUMP, 0, id);
            self.emit(
                GOTO,
                0,
                u16::try_from(loop_start).map_err(|_| Error::UnsupportedPattern)?,
            );
            Some(exit)
        } else {
            None
        };

        if let Some(exit) = exit {
            self.patch(exit, self.words.len())?;
        }
        self.emit(REPEAT_END, 0, id);
        self.sequence_from(rest, true, item.anchor_after, child_cursors, captures_after)?;
        let done = self.emit(GOTO, 0, 0);

        if let Some(zero) = zero {
            self.patch(zero, self.words.len())?;
            self.emit(REPEAT_END, 0, id);
            let mut zero_rest = rest.to_vec();
            if let Some(first) = zero_rest.first_mut() {
                if item.anchor_after {
                    // `Q* . B` is vacuous when Q is empty. `A . Q* . B` keeps B adjacent to A.
                    first.anchor_before = item.anchor_before;
                }
                if !matched && item.anchor_before {
                    // A leading boundary on an empty first-child quantifier transfers to B.
                    first.anchor_before = true;
                }
            }
            // An anchor between the previous item and an empty quantifier is vacuous. A trailing
            // anchor on an empty run has no node to constrain.
            self.sequence_from(&zero_rest, matched, false, child_cursors, captures_after)?;
        }
        self.patch(done, self.words.len())
    }

    fn captures(&mut self, captures: &[String]) -> Result<(), Error> {
        for capture in captures {
            let label = self.label(capture)?;
            self.emit(CAPTURE, 0, label);
        }
        Ok(())
    }

    fn predicates(&mut self, predicates: &[Predicate]) -> Result<(), Error> {
        for predicate in predicates {
            if predicate.name == "#set!" {
                let (capture, key, value) = match predicate.arguments.as_slice() {
                    [Argument::Text(key)] => (None, key, None),
                    [Argument::Text(key), Argument::Text(value)] => (None, key, Some(value)),
                    [Argument::Capture(capture), Argument::Text(key)] => (Some(capture), key, None),
                    [
                        Argument::Capture(capture),
                        Argument::Text(key),
                        Argument::Text(value),
                    ] => (Some(capture), key, Some(value)),
                    _ => return Err(Error::UnsupportedPattern),
                };
                let key = self.string(key.as_bytes())?;
                let value = value
                    .map(|value| self.string(value.as_bytes()))
                    .transpose()?;
                if let Some(capture) = capture {
                    let capture = *self
                        .label_indexes
                        .get(capture)
                        .ok_or_else(|| Error::UnknownCapture(capture.clone()))?;
                    self.directives.push(Directive::SetCapture {
                        capture,
                        key,
                        value,
                    });
                } else {
                    self.directives.push(Directive::Set { key, value });
                }
                continue;
            }
            if predicate.name == "#offset!" {
                let Some(Argument::Capture(capture)) = predicate.arguments.first() else {
                    return Err(Error::UnsupportedPattern);
                };
                let capture = *self
                    .label_indexes
                    .get(capture)
                    .ok_or_else(|| Error::UnknownCapture(capture.clone()))?;
                let offsets = predicate
                    .arguments
                    .iter()
                    .skip(1)
                    .map(|argument| match argument {
                        Argument::Text(value) => {
                            value.parse::<i32>().map_err(|_| Error::UnsupportedPattern)
                        }
                        Argument::Capture(_) => Err(Error::UnsupportedPattern),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let offsets: [i32; 4] =
                    offsets.try_into().map_err(|_| Error::UnsupportedPattern)?;
                self.directives.push(Directive::Offset { capture, offsets });
                continue;
            }
            if predicate.name == "#strip!" {
                let [Argument::Capture(capture), Argument::Text(source)] =
                    predicate.arguments.as_slice()
                else {
                    return Err(Error::UnsupportedPattern);
                };
                let capture = *self
                    .label_indexes
                    .get(capture)
                    .ok_or_else(|| Error::UnknownCapture(capture.clone()))?;
                let source_index = self.string(source.as_bytes())?;
                self.directives.push(Directive::Strip {
                    capture,
                    source: source_index,
                    compiled: Regex::new(source).map_err(|_| Error::UnsupportedPattern)?,
                });
                continue;
            }
            if matches!(
                predicate.name.as_str(),
                "#select-adjacent!" | "#set-adjacent!"
            ) {
                let [Argument::Capture(capture), Argument::Capture(target)] =
                    predicate.arguments.as_slice()
                else {
                    return Err(Error::UnsupportedPattern);
                };
                let capture = *self
                    .label_indexes
                    .get(capture)
                    .ok_or_else(|| Error::UnknownCapture(capture.clone()))?;
                let target = *self
                    .label_indexes
                    .get(target)
                    .ok_or_else(|| Error::UnknownCapture(target.clone()))?;
                self.directives
                    .push(Directive::SelectAdjacent { capture, target });
                continue;
            }
            if predicate.name == "#downcase!" {
                let [Argument::Capture(capture)] = predicate.arguments.as_slice() else {
                    return Err(Error::UnsupportedPattern);
                };
                let capture = *self
                    .label_indexes
                    .get(capture)
                    .ok_or_else(|| Error::UnknownCapture(capture.clone()))?;
                self.directives.push(Directive::Downcase { capture });
                continue;
            }
            if predicate.name == "#set-lang-from-mimetype!" {
                let [Argument::Capture(capture)] = predicate.arguments.as_slice() else {
                    return Err(Error::UnsupportedPattern);
                };
                let capture = *self
                    .label_indexes
                    .get(capture)
                    .ok_or_else(|| Error::UnknownCapture(capture.clone()))?;
                self.directives
                    .push(Directive::SetLanguageFromMimeType { capture });
                continue;
            }
            if matches!(predicate.name.as_str(), "#is?" | "#is-not?") {
                if predicate.arguments.is_empty() || predicate.arguments.len() > 3 {
                    return Err(Error::UnsupportedPattern);
                }
                let capture_count = predicate
                    .arguments
                    .iter()
                    .filter(|argument| matches!(argument, Argument::Capture(_)))
                    .count();
                let text_count = predicate
                    .arguments
                    .iter()
                    .filter(|argument| matches!(argument, Argument::Text(_)))
                    .count();
                if capture_count > 1 || !(1..=2).contains(&text_count) {
                    return Err(Error::UnsupportedPattern);
                }
                for argument in &predicate.arguments {
                    if let Argument::Capture(capture) = argument
                        && !self.label_indexes.contains_key(capture)
                    {
                        return Err(Error::UnknownCapture(capture.clone()));
                    }
                }
                eprintln!(
                    "Tempo ignored SCM property predicate {} for ast-index-spike compatibility; exact host semantics are tracked in NER-1690 (https://linear.app/nera/issue/NER-1690)",
                    predicate.name
                );
                continue;
            }
            let Some(Argument::Capture(capture)) = predicate.arguments.first() else {
                return Err(Error::UnsupportedPattern);
            };
            let capture = *self
                .label_indexes
                .get(capture)
                .ok_or_else(|| Error::UnknownCapture(capture.clone()))?;
            let any = predicate.name.starts_with("#any-");
            let negated = predicate.name.contains("not-");
            let compiled = match predicate.name.as_str() {
                "#eq?" | "#not-eq?" | "#any-eq?" | "#any-not-eq?" => {
                    match predicate.arguments.get(1) {
                        Some(Argument::Capture(right)) => CapturePredicate::Equal {
                            left: capture,
                            right: *self
                                .label_indexes
                                .get(right)
                                .ok_or_else(|| Error::UnknownCapture(right.clone()))?,
                            negated,
                            any,
                        },
                        Some(Argument::Text(text)) => {
                            let string = self.string(text.as_bytes())?;
                            let predicate_index = u16::try_from(self.predicates.len())
                                .map_err(|_| Error::UnsupportedPattern)?;
                            self.predicates.push(TextPredicate::Equal(string));
                            CapturePredicate::Text {
                                capture,
                                predicate: predicate_index,
                                negated,
                                any,
                            }
                        }
                        None => return Err(Error::UnsupportedPattern),
                    }
                }
                "#match?" | "#not-match?" | "#any-match?" | "#any-not-match?" => {
                    let Some(Argument::Text(source)) = predicate.arguments.get(1) else {
                        return Err(Error::UnsupportedPattern);
                    };
                    let string = self.string(source.as_bytes())?;
                    let predicate_index = u16::try_from(self.predicates.len())
                        .map_err(|_| Error::UnsupportedPattern)?;
                    self.predicates.push(TextPredicate::Regex {
                        source: string,
                        compiled: Regex::new(source).map_err(|_| Error::UnsupportedPattern)?,
                    });
                    CapturePredicate::Text {
                        capture,
                        predicate: predicate_index,
                        negated,
                        any,
                    }
                }
                "#any-of?" | "#not-any-of?" => CapturePredicate::AnyOf {
                    capture,
                    strings: predicate
                        .arguments
                        .iter()
                        .skip(1)
                        .map(|argument| match argument {
                            Argument::Text(text) => self.string(text.as_bytes()),
                            Argument::Capture(_) => Err(Error::UnsupportedPattern),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    negated,
                    any: false,
                },
                "#contains?" | "#not-contains?" | "#any-contains?" | "#any-not-contains?" => {
                    if predicate.arguments.len() < 2 {
                        return Err(Error::UnsupportedPattern);
                    }
                    CapturePredicate::Contains {
                        capture,
                        strings: predicate
                            .arguments
                            .iter()
                            .skip(1)
                            .map(|argument| match argument {
                                Argument::Text(text) => self.string(text.as_bytes()),
                                Argument::Capture(_) => Err(Error::UnsupportedPattern),
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                        negated,
                        any,
                    }
                }
                "#kind-eq?" | "#not-kind-eq?" => {
                    if predicate.arguments.len() < 2 {
                        return Err(Error::UnsupportedPattern);
                    }
                    let kinds = predicate
                        .arguments
                        .iter()
                        .skip(1)
                        .map(|argument| match argument {
                            Argument::Text(kind) => Ok(self.predicate_kind(kind)),
                            Argument::Capture(_) => Err(Error::UnsupportedPattern),
                        })
                        .collect::<Result<Vec<_>, _>>()?
                        .into_iter()
                        .flatten()
                        .collect();
                    CapturePredicate::Kinds {
                        capture,
                        kinds,
                        negated,
                    }
                }
                "#has-parent?" | "#not-has-parent?" | "#has-ancestor?" | "#not-has-ancestor?" => {
                    if predicate.arguments.len() < 2 {
                        return Err(Error::UnsupportedPattern);
                    }
                    let kinds = predicate
                        .arguments
                        .iter()
                        .skip(1)
                        .map(|argument| match argument {
                            Argument::Text(kind) => Ok(self.predicate_kind(kind)),
                            Argument::Capture(_) => Err(Error::UnsupportedPattern),
                        })
                        .collect::<Result<Vec<_>, _>>()?
                        .into_iter()
                        .flatten()
                        .collect();
                    CapturePredicate::Relation {
                        capture,
                        kinds,
                        ancestor: predicate.name.contains("ancestor"),
                        negated,
                    }
                }
                "#same-line?" | "#not-same-line?" => {
                    if predicate.arguments.len() != 2 {
                        return Err(Error::UnsupportedPattern);
                    }
                    let Some(Argument::Capture(right)) = predicate.arguments.get(1) else {
                        return Err(Error::UnsupportedPattern);
                    };
                    CapturePredicate::SameLine {
                        left: capture,
                        right: *self
                            .label_indexes
                            .get(right)
                            .ok_or_else(|| Error::UnknownCapture(right.clone()))?,
                        negated,
                    }
                }
                "#lua-match?" | "#not-lua-match?" | "#any-lua-match?" | "#any-not-lua-match?" => {
                    let Some(Argument::Text(source)) = predicate.arguments.get(1) else {
                        return Err(Error::UnsupportedPattern);
                    };
                    let string = self.string(source.as_bytes())?;
                    let predicate_index = u16::try_from(self.predicates.len())
                        .map_err(|_| Error::UnsupportedPattern)?;
                    self.predicates.push(TextPredicate::LuaPattern {
                        source: string,
                        compiled: LuaPattern::parse(source.as_bytes())
                            .map_err(|_| Error::UnsupportedFeature("Lua pattern syntax"))?,
                    });
                    CapturePredicate::Text {
                        capture,
                        predicate: predicate_index,
                        negated,
                        any,
                    }
                }
                _ => return Err(Error::UnsupportedFeature("predicate")),
            };
            let index = u16::try_from(self.capture_predicates.len())
                .map_err(|_| Error::UnsupportedPattern)?;
            self.capture_predicates.push(compiled);
            self.emit(ASSERT, 0, index);
        }
        Ok(())
    }

    fn compile(mut self, query: &QueryPattern) -> Result<Program, Error> {
        self.pattern(&query.pattern)?;
        self.predicates(&query.predicates)?;
        let output = self.label("tree-sitter-query")?;
        self.emit(EMIT, 0, output);
        Program::new_with_capture_predicates(
            self.words,
            self.strings,
            self.predicates,
            self.capture_predicates,
            self.labels,
            Vec::new(),
        )?
        .with_directives(self.directives)?
        .with_tree_sitter_query_semantics()
    }
}

pub(super) fn compile(
    query: &QueryPattern,
    grammar: &tree_sitter::Language,
) -> Result<Program, Error> {
    Compiler::new(grammar).compile(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_builds_full_query_syntax_before_lowering() {
        let source = r#"
; comment with unmatched delimiters ([)
((function_definition
   name: (identifier) @name
   !type_parameters
   body: (block . [(return_statement) (pass_statement)]* @body .)) @definition
 (#match? @name "^[a-z_]+$")
 (#set! role "function"))
"#;
        let parsed = parse(source).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].predicates.len(), 2);
        assert_eq!(parsed[0].pattern.children.len(), 1);
        let definition = &parsed[0].pattern.children[0].pattern;
        assert!(
            matches!(definition.form, Form::Kind { ref name, named: true } if name == "function_definition")
        );
        assert_eq!(definition.captures, ["definition"]);
        assert_eq!(definition.children.len(), 3);
        assert_eq!(definition.children[0].field.as_deref(), Some("name"));
        assert_eq!(
            definition.children[1].negated_field.as_deref(),
            Some("type_parameters")
        );
        let body = &definition.children[2].pattern;
        assert!(body.children[0].anchor_before);
        assert!(body.children[0].anchor_after);
        assert_eq!(body.children[0].pattern.quantifier, Some('*'));
    }

    #[test]
    #[ignore = "all pinned Python SCM files"]
    fn parser_accepts_every_pinned_python_query_file() {
        let root = std::env::var("TEMPO_PYTHON_SCM_QUERY_DIR")
            .expect("TEMPO_PYTHON_SCM_QUERY_DIR must name the original Python SCM directory");
        let mut paths = std::fs::read_dir(&root)
            .unwrap_or_else(|error| {
                panic!(
                    "cannot read pinned Python SCM input at {root}: {error}; run tempo/scripts/fetch-pinned-python-scm.sh"
                )
            })
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "scm"))
            .collect::<Vec<_>>();
        paths.sort();
        let mut patterns = 0_usize;
        for path in &paths {
            let source = std::fs::read_to_string(path).unwrap();
            patterns += parse(&source)
                .unwrap_or_else(|error| panic!("{}: {error:?}", path.display()))
                .len();
        }
        assert_eq!(paths.len(), 15);
        assert_eq!(patterns, 335);
        println!(
            "TEMPO_PYTHON_SCM_PARSE {{\"files\":{},\"patterns\":{patterns}}}",
            paths.len()
        );
    }

    #[test]
    #[ignore = "all pinned Python SCM patterns through the Program lowerer"]
    fn lowerer_audits_every_pinned_python_pattern_without_approximation() {
        use ast_grep_language::LanguageExt;
        let root = std::env::var("TEMPO_PYTHON_SCM_QUERY_DIR")
            .expect("TEMPO_PYTHON_SCM_QUERY_DIR must name the original Python SCM directory");
        let mut paths = std::fs::read_dir(&root)
            .unwrap_or_else(|error| {
                panic!(
                    "cannot read pinned Python SCM input at {root}: {error}; run tempo/scripts/fetch-pinned-python-scm.sh"
                )
            })
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "scm"))
            .collect::<Vec<_>>();
        paths.sort();
        let language = crate::code_rule_dsl::language::GrammarDialect::Python.get_ts_language();
        let pinned_nvim = BTreeMap::from([
            (
                "nvim-treesitter__python__folds.scm",
                "7ea0eb74ffee66415a3b6fd51b47873b411bc80d8eb4dfb7aa6ffa0cf7fad1f3",
            ),
            (
                "nvim-treesitter__python__highlights.scm",
                "ab8b66cbe6a7382e0ed7693c194101f1900c39f9ec0085a1749a8817e3e6895c",
            ),
            (
                "nvim-treesitter__python__indents.scm",
                "50781d783a18a566581d8280c53e150454d1e3523cfcd4f1401bb3a228a98e80",
            ),
            (
                "nvim-treesitter__python__injections.scm",
                "8558299709200b21d496890e4799153dc069bf302ce1545d23f70a0869c8b65d",
            ),
            (
                "nvim-treesitter__python__locals.scm",
                "370635358588e6d66e6065dd9fb1e21d864e8eb125abbf4728e573bc06ef4921",
            ),
        ]);
        let mut compiled = Vec::new();
        let mut incompatible = BTreeMap::<String, usize>::new();
        let mut rejected = BTreeMap::<String, usize>::new();
        for path in &paths {
            let source = std::fs::read_to_string(path).unwrap();
            if let Some(expected) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| pinned_nvim.get(name))
            {
                assert_eq!(
                    format!("{:x}", Sha256::digest(source.as_bytes())),
                    *expected
                );
            }
            for pattern in parse(&source).unwrap() {
                if let Err(error) = tree_sitter::Query::new(&language, pattern.source()) {
                    *incompatible.entry(error.message.clone()).or_default() += 1;
                    continue;
                }
                match compile(&pattern, &language) {
                    Ok(program) => compiled.push(program),
                    Err(error) => {
                        *rejected.entry(format!("{error:?}")).or_default() += 1;
                    }
                }
            }
        }
        let incompatible_count = incompatible.values().sum::<usize>();
        let rejected_count = rejected.values().sum::<usize>();
        assert_eq!(compiled.len(), 335);
        assert_eq!(incompatible_count, 0);
        assert_eq!(rejected_count, 0, "compatible patterns must all lower");
        let mut opcode_counts = BTreeMap::<&str, usize>::new();
        let mut value_program_counts = BTreeMap::<&str, usize>::new();
        let mut capture_program_counts = BTreeMap::<&str, usize>::new();
        let mut output_instruction_counts = BTreeMap::<&str, usize>::new();
        for program in &compiled {
            for value in &program.value_programs {
                let name = match value {
                    super::super::value::ValueProgram::Equal { .. } => "equal",
                    super::super::value::ValueProgram::Prefix { .. } => "prefix",
                    super::super::value::ValueProgram::Regex(_) => "regex",
                    super::super::value::ValueProgram::Lua(_) => "lua",
                };
                *value_program_counts.entry(name).or_default() += 1;
            }
            for capture in &program.capture_programs {
                let name = match capture {
                    super::super::value::CaptureProgram::Equal { .. } => "equal",
                    super::super::value::CaptureProgram::Text { .. } => "text",
                    super::super::value::CaptureProgram::AnyOf { .. } => "any_of",
                    super::super::value::CaptureProgram::Kind { .. } => "kind",
                    super::super::value::CaptureProgram::Contains { .. } => "contains",
                    super::super::value::CaptureProgram::Relation { .. } => "relation",
                    super::super::value::CaptureProgram::SameLine { .. } => "same_line",
                };
                *capture_program_counts.entry(name).or_default() += 1;
            }
            for output in &program.output_program.instructions {
                let name = match output {
                    super::super::output::OutputInstruction::MatchProperty { .. } => {
                        "match_property"
                    }
                    super::super::output::OutputInstruction::CaptureProperty { .. } => {
                        "capture_property"
                    }
                    super::super::output::OutputInstruction::Offset { .. } => "offset",
                    super::super::output::OutputInstruction::Strip { .. } => "strip",
                    super::super::output::OutputInstruction::SelectAdjacent { .. } => {
                        "select_adjacent"
                    }
                    super::super::output::OutputInstruction::Downcase { .. } => "downcase",
                    super::super::output::OutputInstruction::LanguageFromMime { .. } => {
                        "language_from_mime"
                    }
                };
                *output_instruction_counts.entry(name).or_default() += 1;
            }
            for word in &program.words {
                let name = match super::super::split(*word).0 {
                    super::super::STOP => "stop",
                    super::super::KIND => "kind",
                    super::super::FIELD => "field",
                    super::super::FLAGS => "flags",
                    super::super::TEXT => "text",
                    super::super::SIZE => "size",
                    super::super::ASSERT => "assert",
                    super::super::FIRST => "first",
                    super::super::LAST => "last",
                    super::super::ABSENT_FIELD => "absent_field",
                    super::super::SUPERTYPE => "supertype",
                    super::super::CHILD => "child",
                    super::super::DESCEND => "descend",
                    super::super::NEXT => "next",
                    super::super::FOLLOWING => "following",
                    super::super::PARENT => "parent",
                    super::super::SAVE_CURSOR => "save_cursor",
                    super::super::RESTORE_CURSOR => "restore_cursor",
                    super::super::CAPTURE => "capture",
                    super::super::REPEAT_BEGIN => "repeat_begin",
                    super::super::REPEAT_BUMP => "repeat_bump",
                    super::super::REPEAT_END => "repeat_end",
                    super::super::FORK => "fork",
                    super::super::GOTO => "goto",
                    super::super::EMIT => "emit",
                    opcode => panic!("unrecognized compiled opcode {opcode}"),
                };
                *opcode_counts.entry(name).or_default() += 1;
            }
        }

        let native_modules = compiled
            .iter()
            .map(|program| {
                super::super::native_ir::lower(program)
                    .expect("the native IR must lower every pinned Program")
            })
            .collect::<Vec<_>>();
        let native_cpu_started = std::time::Instant::now();
        for native in &native_modules {
            super::super::native_cpu::compile_straight_line(native)
                .expect("the callback-free CPU JIT must compile every pinned Program");
        }
        let native_cpu_millis = native_cpu_started.elapsed().as_secs_f64() * 1_000.0;
        let metal_compile_millis: Option<f64> = None;
        println!(
            "TEMPO_PYTHON_SCM_COMPILATION {{\"programs\":{},\"nativeIrLowered\":{},\"callbackFreeCpuJitCompiled\":{},\"callbackFreeCpuJitCompileMillis\":{native_cpu_millis},\"metalJitCompiled\":{},\"metalJitCompileMillis\":{},\"structuralOpcodeCounts\":{},\"valueProgramCounts\":{},\"captureProgramCounts\":{},\"outputInstructionCounts\":{}}}",
            compiled.len(),
            compiled.len(),
            compiled.len(),
            if metal_compile_millis.is_some() {
                compiled.len()
            } else {
                0
            },
            serde_json::to_string(&metal_compile_millis).unwrap(),
            serde_json::to_string(&opcode_counts).unwrap(),
            serde_json::to_string(&value_program_counts).unwrap(),
            serde_json::to_string(&capture_program_counts).unwrap(),
            serde_json::to_string(&output_instruction_counts).unwrap(),
        );
        println!(
            "TEMPO_PYTHON_SCM_LOWERING {{\"compiled\":{},\"grammarIncompatible\":{incompatible_count},\"loweringRejected\":{rejected_count},\"incompatibilities\":{},\"rejections\":{}}}",
            compiled.len(),
            serde_json::to_string(&incompatible).unwrap(),
            serde_json::to_string(&rejected).unwrap()
        );
    }

    #[test]
    fn lowerer_accepts_extensions_and_ignores_deferred_property_semantics() {
        use ast_grep_language::LanguageExt;
        let language = crate::code_rule_dsl::language::GrammarDialect::Python.get_ts_language();
        let quantified = parse("(parameters (identifier)+ @name)").unwrap();
        assert!(compile(&quantified[0], &language).is_ok());
        let directed = parse("((identifier) @name (#set! role \"name\"))").unwrap();
        assert!(compile(&directed[0], &language).is_ok());
        let property = parse("((identifier) @name (#is? @name local))").unwrap();
        assert!(compile(&property[0], &language).is_ok());
        let malformed = parse("((identifier) @name (#is? @name))").unwrap();
        assert!(matches!(
            compile(&malformed[0], &language),
            Err(Error::UnsupportedPattern)
        ));
    }
}
