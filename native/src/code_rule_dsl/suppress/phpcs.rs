//! PHP_CodeSniffer's annotations: `phpcs:ignore`, `phpcs:disable`, `phpcs:enable` and
//! `phpcs:ignoreFile` (also spelled `@phpcs:`), as PHP_CodeSniffer 4.0.4 reads them, and as
//! 3.13.6 reads them together with the `@codingStandardsIgnoreLine`, `-Start`, `-End` and `-File`
//! annotations 4.0 removed.
//!
//! This reproduces `Tokenizer::createPositionMap` over PHP_CodeSniffer's comment tokens (a block
//! comment split per line, a `//` or `#` comment with the line break after it, a doc comment as
//! its star, tag and string tokens) and `File::addMessage`'s line check; `File::process` drops
//! every message of a file with a `phpcs:ignoreFile`.

use super::{names, php_comment_end};
use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
use crate::code_rule_dsl::lines::LineTable;
use ast_grep_core::{Doc, Node};
use serde_json::Value;
use std::collections::HashMap;
use std::rc::Rc;

/// `phpcs`: PHP_CodeSniffer's `phpcs:` annotations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Phpcs {
    /// The sniff or message codes the violation reports as.
    pub(crate) codes: Vec<String>,
    /// `version`: the PHP_CodeSniffer major whose reading applies (4 unless 3).
    pub(crate) version: PhpcsVersion,
}

/// `suppress.phpcs`.
pub(super) fn spec(phpcs: &Value, at: &str) -> napi::Result<Phpcs> {
    reject_unknown(object(phpcs, at)?, &["sniffs", "version"], at)?;
    let codes = names(
        phpcs.get("sniffs"),
        &format!("{at}/sniffs"),
        phpcs_code,
        "PHP_CodeSniffer codes",
        "a PHP_CodeSniffer sniff or message code (Standard.Category.Sniff or Standard.Category.Sniff.Code)",
    )?;
    let version = match phpcs.get("version").map(Value::as_u64) {
        None | Some(Some(4)) => PhpcsVersion::Four,
        Some(Some(3)) => PhpcsVersion::Three,
        Some(_) => return Err(invalid(format!("{at}/version must be 3 or 4"))),
    };
    Ok(Phpcs { codes, version })
}

/// A sniff (`Standard.Category.Sniff`) or message (`Standard.Category.Sniff.Code`) code: parts of
/// letters, digits and `_`, as PHP_CodeSniffer names standards, sniffs and codes.
fn phpcs_code(code: &str) -> bool {
    let parts = code.split('.');
    (3..=4).contains(&parts.clone().count())
        && parts.into_iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}

/// A PHP_CodeSniffer major version whose annotations a violation honours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PhpcsVersion {
    /// PHP_CodeSniffer 3.13.6, with the `@codingStandardsIgnore*` annotations.
    Three,
    /// PHP_CodeSniffer 4.0.4.
    Four,
}

#[derive(Debug, Default)]
pub(crate) struct PhpcsIndex {
    four: Ignores<IgnoreList>,
    three: Ignores<Rules>,
}

/// What a version ignores: the whole file, or per line (zero-based) the rules in force there.
#[derive(Debug)]
struct Ignores<T> {
    file: bool,
    lines: HashMap<usize, Rc<T>>,
}

impl<T> Default for Ignores<T> {
    fn default() -> Self {
        Self {
            file: false,
            lines: HashMap::new(),
        }
    }
}

impl PhpcsIndex {
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>, source: &str, lines: &LineTable) -> Self {
        let pieces = pieces(root, source, lines);
        let mut index = Self::default();
        let all4 = Rc::new(IgnoreList::all());
        let all3 = Rc::new(Rules::all());
        let mut ignoring4: Option<Rc<IgnoreList>> = None;
        let mut ignoring3: Option<Rc<Rules>> = None;
        let mut next = pieces.iter().peekable();
        for line in 0..=lines.line(source.len()) {
            // A token before the line's first annotation (or a line without one) applies the
            // rules in force to the line.
            if !next
                .peek()
                .is_some_and(|piece| piece.line == line && piece.first)
            {
                apply(&mut index.four.lines, line, ignoring4.as_ref());
                apply(&mut index.three.lines, line, ignoring3.as_ref());
            }
            while let Some(piece) = next.next_if(|piece| piece.line == line) {
                let text = piece_text_ltrim(piece.text)
                    .trim_end_matches([' ', '*', '/', '\t', '\r', '\n']);
                index.four.file |= ignores_file(piece.text);
                index.three.file |= ignores_file(piece.text)
                    || piece_text_ltrim(piece.text).contains("@codingStandardsIgnoreFile");
                if let Some((text, annotation)) = annotation(text) {
                    four(
                        &mut index.four.lines,
                        &mut ignoring4,
                        &all4,
                        piece,
                        text,
                        annotation,
                    );
                }
                if text.contains("@codingStandards") {
                    legacy(&mut index.three.lines, &mut ignoring3, &all3, piece, text);
                } else if let Some((text, annotation)) = annotation(text) {
                    three(
                        &mut index.three.lines,
                        &mut ignoring3,
                        &all3,
                        piece,
                        text,
                        annotation,
                    );
                }
                apply(&mut index.four.lines, line, ignoring4.as_ref());
                apply(&mut index.three.lines, line, ignoring3.as_ref());
            }
        }
        index
    }

    /// Whether a message with any of `codes` starting at byte `at` is ignored.
    pub(crate) fn suppresses(
        &self,
        codes: &[String],
        version: PhpcsVersion,
        at: usize,
        lines: &LineTable,
    ) -> bool {
        let line = lines.line(at);
        match version {
            PhpcsVersion::Four => {
                self.four.file
                    || self.four.lines.get(&line).is_some_and(|rules| {
                        rules.ignores_everything()
                            || codes.iter().any(|code| rules.is_ignored(code))
                    })
            }
            PhpcsVersion::Three => {
                self.three.file
                    || self
                        .three
                        .lines
                        .get(&line)
                        .is_some_and(|rules| codes.iter().any(|code| rules.ignores(code)))
            }
        }
    }

    /// Whether the file holds no annotation either version would apply.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        !self.four.file
            && !self.three.file
            && self.four.lines.is_empty()
            && self.three.lines.is_empty()
    }
}

/// `if ($ignoring !== null && isset($this->ignoredLines[$line]) === false)`.
fn apply<T>(lines: &mut HashMap<usize, Rc<T>>, line: usize, ignoring: Option<&Rc<T>>) {
    if let Some(ignoring) = ignoring {
        lines.entry(line).or_insert_with(|| Rc::clone(ignoring));
    }
}

/// `$this->ignoredLines[$line] = $rules`, where a null rule set unsets the line.
fn assign<T>(lines: &mut HashMap<usize, Rc<T>>, line: usize, rules: Option<&Rc<T>>) {
    match rules {
        Some(rules) => {
            lines.insert(line, Rc::clone(rules));
        }
        None => {
            lines.remove(&line);
        }
    }
}

/// `ltrim($content, " \t/*#")`.
fn piece_text_ltrim(text: &str) -> &str {
    text.trim_start_matches([' ', '\t', '/', '*', '#'])
}

/// Whether `text` starts with `prefix`, ASCII case-insensitively (`strtolower` then `substr`).
fn starts_ci(text: &str, prefix: &str) -> bool {
    text.as_bytes()
        .get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix.as_bytes()))
}

/// `File::process`: a comment token that starts `phpcs:ignoreFile`.
fn ignores_file(text: &str) -> bool {
    let text = piece_text_ltrim(text);
    starts_ci(text, "phpcs:ignorefile") || starts_ci(text, "@phpcs:ignorefile")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Annotation {
    Set,
    IgnoreFile,
    Disable,
    Enable,
    Ignore,
}

/// A `phpcs:` annotation: its text without the `@` and `--` note, and which it is (checked in
/// `createPositionMap`'s order, so `phpcs:ignoreFile` is not `phpcs:ignore`).
fn annotation(text: &str) -> Option<(&str, Annotation)> {
    let text = if starts_ci(text, "phpcs:") {
        text
    } else if starts_ci(text, "@phpcs:") {
        &text[1..]
    } else {
        return None;
    };
    let text = text.find(" --").map_or(text, |at| &text[..at]);
    let annotation = [
        ("phpcs:set", Annotation::Set),
        ("phpcs:ignorefile", Annotation::IgnoreFile),
        ("phpcs:disable", Annotation::Disable),
        ("phpcs:enable", Annotation::Enable),
        ("phpcs:ignore", Annotation::Ignore),
    ]
    .into_iter()
    .find(|(prefix, _)| starts_ci(text, prefix))?
    .1;
    Some((text, annotation))
}

/// PHP's `substr($text, $from)`.
fn after(text: &str, from: usize) -> &str {
    text.get(from..).unwrap_or("")
}

/// PHP's `empty()` of a string.
fn empty(text: &str) -> bool {
    text.is_empty() || text == "0"
}

/// The comma-separated codes after an annotation, each trimmed as PHP's `trim`.
fn codes(text: &str) -> impl Iterator<Item = &str> {
    text.split(',')
        .map(|code| code.trim_matches([' ', '\t', '\n', '\r', '\0', '\x0B']))
}

/// PHP_CodeSniffer 4.0.4's `createPositionMap` for one annotation.
fn four(
    lines: &mut HashMap<usize, Rc<IgnoreList>>,
    ignoring: &mut Option<Rc<IgnoreList>>,
    all: &Rc<IgnoreList>,
    piece: &Piece<'_>,
    text: &str,
    annotation: Annotation,
) {
    let line = piece.line;
    let extended = |ignoring: &Option<Rc<IgnoreList>>, additional: &str, ignore: bool| {
        let mut next = ignoring
            .as_deref()
            .cloned()
            .unwrap_or_else(IgnoreList::nothing);
        for code in codes(additional) {
            next.set(code, ignore);
        }
        next
    };
    match annotation {
        Annotation::Set => {
            if !piece.tokens {
                assign(lines, line, Some(all));
            }
        }
        Annotation::IgnoreFile => {}
        Annotation::Disable => {
            if !piece.content {
                assign(lines, line, Some(all));
            }
            let additional = after(text, 14);
            *ignoring = Some(if empty(additional) {
                Rc::clone(all)
            } else {
                Rc::new(extended(ignoring, additional, true))
            });
        }
        Annotation::Enable => {
            if ignoring.is_some() {
                let additional = after(text, 13);
                *ignoring = if empty(additional) {
                    None
                } else {
                    let next = extended(ignoring, additional, false);
                    (!next.ignores_nothing()).then(|| Rc::new(next))
                };
                if !piece.content {
                    assign(lines, line, Some(all));
                } else {
                    assign(lines, line, ignoring.as_ref());
                }
            }
        }
        Annotation::Ignore => {
            let additional = after(text, 13);
            let rules = if empty(additional) {
                Rc::clone(all)
            } else {
                Rc::new(extended(ignoring, additional, true))
            };
            if !piece.content {
                assign(lines, line, Some(all));
                assign(lines, line + 1, Some(&rules));
            } else {
                assign(lines, line, Some(&rules));
            }
        }
    }
}

/// PHP_CodeSniffer 3.13.6's `createPositionMap` for one `phpcs:` annotation.
fn three(
    lines: &mut HashMap<usize, Rc<Rules>>,
    ignoring: &mut Option<Rc<Rules>>,
    all: &Rc<Rules>,
    piece: &Piece<'_>,
    text: &str,
    annotation: Annotation,
) {
    let line = piece.line;
    match annotation {
        Annotation::Set => {
            if !piece.tokens {
                assign(lines, line, Some(all));
            }
        }
        Annotation::IgnoreFile => {}
        Annotation::Disable => {
            if !piece.content {
                assign(lines, line, Some(all));
            }
            let additional = after(text, 14);
            if empty(additional) {
                *ignoring = Some(Rc::clone(all));
            } else {
                let mut next = ignoring.as_deref().cloned().unwrap_or_default();
                for code in codes(additional) {
                    next.add(code);
                    // A newly disabled code ends the exceptions it covers.
                    if let Some(except) = &mut next.except {
                        except.retain(|except| !covers(code, except));
                        if except.is_empty() {
                            next.except = None;
                        }
                    }
                }
                *ignoring = Some(Rc::new(next));
            }
        }
        Annotation::Enable => {
            if let Some(current) = ignoring.as_deref() {
                let additional = after(text, 13);
                if empty(additional) {
                    *ignoring = None;
                } else {
                    let mut next = current.clone();
                    let mut enabled: Vec<&str> = Vec::new();
                    for code in codes(additional) {
                        if !enabled.contains(&code) {
                            enabled.push(code);
                        }
                        // An enabled standard or category removes the codes under it, and the
                        // exceptions under it.
                        next.all &= !covers(code, ".all");
                        next.codes.retain(|ignored| !covers(code, ignored));
                        if covers(code, ".except") {
                            next.except = None;
                        }
                        if let Some(except) = &mut next.except {
                            except.retain(|except| !covers(code, except));
                        }
                    }
                    *ignoring = if next.is_empty() {
                        None
                    } else {
                        let except = next.except.get_or_insert_with(Vec::new);
                        for code in enabled {
                            if !except.iter().any(|except| except == code) {
                                except.push(code.to_owned());
                            }
                        }
                        Some(Rc::new(next))
                    };
                }
                if !piece.content {
                    assign(lines, line, Some(all));
                } else {
                    assign(lines, line, ignoring.as_ref());
                }
            }
        }
        Annotation::Ignore => {
            let additional = after(text, 13);
            let mut rules = if empty(additional) {
                Rules::all()
            } else {
                let mut rules = Rules::default();
                for code in codes(additional) {
                    rules.add(code);
                }
                rules
            };
            // `$ignoreRules += $ignoring`.
            if let Some(ignoring) = ignoring.as_deref() {
                rules.all |= ignoring.all;
                for code in &ignoring.codes {
                    rules.add(code);
                }
                if rules.except.is_none() {
                    rules.except.clone_from(&ignoring.except);
                }
            }
            let rules = Rc::new(rules);
            if !piece.content {
                assign(lines, line, Some(all));
                assign(lines, line + 1, Some(&rules));
            } else {
                assign(lines, line, Some(&rules));
            }
        }
    }
}

/// PHP_CodeSniffer 3.13.6's `@codingStandardsIgnore*` annotations.
fn legacy(
    lines: &mut HashMap<usize, Rc<Rules>>,
    ignoring: &mut Option<Rc<Rules>>,
    all: &Rc<Rules>,
    piece: &Piece<'_>,
    text: &str,
) {
    let line = piece.line;
    if ignoring.is_none() && text.contains("@codingStandardsIgnoreStart") {
        *ignoring = Some(Rc::clone(all));
        if piece.own_line {
            assign(lines, line, Some(all));
        }
    } else if ignoring.is_some() && text.contains("@codingStandardsIgnoreEnd") {
        if piece.own_line {
            assign(lines, line, Some(all));
        } else {
            assign(lines, line, ignoring.as_ref());
        }
        *ignoring = None;
    } else if ignoring.is_none() && text.contains("@codingStandardsIgnoreLine") {
        assign(lines, line, Some(all));
        if piece.own_line {
            assign(lines, line + 1, Some(all));
        }
    }
}

/// `$code === $key || strpos($key, $code.'.') === 0`: `code` names `key` or a prefix of it.
fn covers(code: &str, key: &str) -> bool {
    key.strip_prefix(code)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

/// PHP_CodeSniffer 4.0.4's `IgnoreList`: a tree of code parts, `.default` where a part's
/// children are not named.
#[derive(Clone, Debug)]
struct IgnoreList(Vec<(String, Entry)>);

#[derive(Clone, Debug)]
enum Entry {
    Ignore(bool),
    Parts(Vec<(String, Entry)>),
}

const DEFAULT: &str = ".default";

impl IgnoreList {
    fn nothing() -> Self {
        Self(vec![(DEFAULT.to_owned(), Entry::Ignore(false))])
    }

    fn all() -> Self {
        Self(vec![(DEFAULT.to_owned(), Entry::Ignore(true))])
    }

    fn set(&mut self, code: &str, ignore: bool) {
        let mut data = &mut self.0;
        let mut parts = code.split('.').peekable();
        while let Some(part) = parts.next() {
            let at = match data.iter().position(|(key, _)| key == part) {
                Some(at) => at,
                None => {
                    data.push((part.to_owned(), Entry::Parts(Vec::new())));
                    data.len() - 1
                }
            };
            if parts.peek().is_none() {
                data[at].1 = Entry::Ignore(ignore);
                return;
            }
            if let Entry::Ignore(value) = data[at].1 {
                data[at].1 = Entry::Parts(vec![(DEFAULT.to_owned(), Entry::Ignore(value))]);
            }
            let Entry::Parts(children) = &mut data[at].1 else {
                return;
            };
            data = children;
        }
    }

    fn is_ignored(&self, code: &str) -> bool {
        fn get<'a>(data: &'a [(String, Entry)], key: &str) -> Option<&'a Entry> {
            data.iter()
                .find(|(name, _)| name == key)
                .map(|(_, entry)| entry)
        }
        let mut data: &[(String, Entry)] = &self.0;
        let mut ignored = matches!(get(data, DEFAULT), Some(Entry::Ignore(true)));
        for part in code.split('.') {
            match get(data, part) {
                None => break,
                Some(Entry::Ignore(value)) => {
                    ignored = *value;
                    break;
                }
                Some(Entry::Parts(children)) => {
                    data = children;
                    if let Some(Entry::Ignore(value)) = get(data, DEFAULT) {
                        ignored = *value;
                    }
                }
            }
        }
        ignored
    }

    fn holds(data: &[(String, Entry)], wanted: bool) -> bool {
        data.iter().any(|(_, entry)| match entry {
            Entry::Ignore(value) => *value == wanted,
            Entry::Parts(children) => Self::holds(children, wanted),
        })
    }

    fn ignores_nothing(&self) -> bool {
        !Self::holds(&self.0, true)
    }

    fn ignores_everything(&self) -> bool {
        !Self::holds(&self.0, false)
    }
}

/// PHP_CodeSniffer 3.13.6's ignore rules: `.all`, ignored codes, and `.except` codes.
#[derive(Clone, Debug, Default)]
struct Rules {
    all: bool,
    codes: Vec<String>,
    except: Option<Vec<String>>,
}

impl Rules {
    fn all() -> Self {
        Self {
            all: true,
            ..Self::default()
        }
    }

    fn add(&mut self, code: &str) {
        if code == ".all" {
            self.all = true;
        } else if !self.codes.iter().any(|ignored| ignored == code) {
            self.codes.push(code.to_owned());
        }
    }

    fn is_empty(&self) -> bool {
        !self.all && self.codes.is_empty() && self.except.is_none()
    }

    /// `File::addMessage`: the message code or its sniff, category or standard is ignored and
    /// none of them is excepted.
    fn ignores(&self, code: &str) -> bool {
        if self.all {
            return true;
        }
        let names =
            |list: &[String]| prefixes(code).any(|prefix| list.iter().any(|name| name == prefix));
        names(&self.codes) && !self.except.as_deref().is_some_and(names)
    }
}

/// A code and each shorter dot-separated prefix of it.
fn prefixes(code: &str) -> impl Iterator<Item = &str> {
    std::iter::once(code).chain(code.match_indices('.').map(|(at, _)| &code[..at]))
}

/// An annotation-bearing comment token: a `T_COMMENT` line, or a doc comment's tag or string.
#[derive(Debug)]
struct Piece<'s> {
    text: &'s str,
    line: usize,
    /// No token precedes it on its line.
    first: bool,
    /// `$lineHasOtherContent`: code, another comment, or a doc comment's opener, closer or tag
    /// shares its line (white space, `<?php`, `?>` and a doc comment's star do not).
    content: bool,
    /// `$lineHasOtherTokens`: any token but white space shares its line.
    tokens: bool,
    /// PHP_CodeSniffer 3's `$ownLine`: the token before it (white space aside, outside doc
    /// comments) is on an earlier line.
    own_line: bool,
}

/// The annotation-bearing comment tokens of a file, in source order.
fn pieces<'s, D: Doc>(root: &Node<'_, D>, source: &'s str, lines: &LineTable) -> Vec<Piece<'s>> {
    let mut pieces = Vec::new();
    for comment in root.dfs().filter(|node| node.kind() == "comment") {
        let range = comment.range();
        let text = &source[range.clone()];
        if text.starts_with("/**") && text != "/**/" {
            doc_pieces(source, range.start, range.end, lines, &mut pieces);
        } else if text.starts_with("/*") {
            let mut at = range.start;
            for (index, segment) in text.split_inclusive('\n').enumerate() {
                let line = lines.line(at);
                let (before, own_line) = if index == 0 {
                    before_on_line(source, at, lines)
                } else {
                    (Neighbours::default(), true)
                };
                let next = if segment.ends_with('\n') {
                    Neighbours::default()
                } else {
                    after_on_line(source, range.end, lines)
                };
                pieces.push(Piece {
                    text: segment,
                    line,
                    first: at == lines.start(line),
                    content: before.content || next.content,
                    tokens: before.tokens || next.tokens,
                    own_line,
                });
                at += segment.len();
            }
        } else {
            // PHP_CodeSniffer moves the line break after a `//` or `#` comment into its token.
            let end = php_comment_end(source, &range);
            let rest = &source[end..];
            let newline = if rest.starts_with("\r\n") {
                2
            } else {
                usize::from(rest.starts_with('\n'))
            };
            let line = lines.line(range.start);
            let (before, own_line) = before_on_line(source, range.start, lines);
            let next = if newline > 0 {
                Neighbours::default()
            } else {
                after_on_line(source, end, lines)
            };
            pieces.push(Piece {
                text: &source[range.start..end + newline],
                line,
                first: range.start == lines.start(line),
                content: before.content || next.content,
                tokens: before.tokens || next.tokens,
                own_line,
            });
        }
    }
    pieces
}

#[derive(Clone, Copy, Debug, Default)]
struct Neighbours {
    content: bool,
    tokens: bool,
}

/// The tokens before byte `at` on its line, and PHP_CodeSniffer 3's `$ownLine` (only white space
/// precedes it there).
fn before_on_line(source: &str, at: usize, lines: &LineTable) -> (Neighbours, bool) {
    let start = lines.start(lines.line(at));
    let before = &source[start..at];
    let mut rest = before;
    let mut found = Neighbours::default();
    loop {
        rest = rest.trim_end_matches(is_space);
        if rest.is_empty() {
            break;
        }
        // Compare bytes: the five before the end may start inside a multi-byte character, and a
        // match is ASCII, so cutting it leaves a character boundary.
        if let Some(tail) = rest.len().checked_sub(5)
            && rest.as_bytes()[tail..].eq_ignore_ascii_case(b"<?php")
        {
            found.tokens = true;
            rest = &rest[..tail];
            continue;
        }
        found = Neighbours {
            content: true,
            tokens: true,
        };
        break;
    }
    (found, at > 0 && before.trim_matches(is_space).is_empty())
}

/// The tokens after byte `at` on its line.
fn after_on_line(source: &str, at: usize, lines: &LineTable) -> Neighbours {
    let end = lines.end(lines.line(at)).max(at);
    let mut rest = &source[at..end];
    let mut found = Neighbours::default();
    loop {
        rest = rest.trim_start_matches(is_space);
        if rest.is_empty() {
            return found;
        }
        if let Some(after) = rest.strip_prefix("?>") {
            found.tokens = true;
            rest = after;
            continue;
        }
        return Neighbours {
            content: true,
            tokens: true,
        };
    }
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0B' | '\x0C' | '\0')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DocKind {
    Open,
    Space,
    Star,
    Tag,
    String,
    Close,
}

/// `Comment::tokenizeString`: a doc comment's tokens; its tags and strings become pieces.
fn doc_pieces<'s>(
    source: &'s str,
    start: usize,
    end: usize,
    lines: &LineTable,
    pieces: &mut Vec<Piece<'s>>,
) {
    let comment = &source[start..end];
    let lead = comment.len() - comment.trim_start_matches(['/', '*']).len();
    let open = if lead == comment.len() && comment.ends_with("*/") {
        comment.len() - 2
    } else {
        lead
    };
    let rest = &comment[open..];
    let body = rest.trim_end_matches(['/', '*']);
    let mut tokens: Vec<(DocKind, usize, usize)> = vec![(DocKind::Open, start, start + open)];
    let mut at = start + open;
    'lines: for (index, segment) in body.split_inclusive('\n').enumerate() {
        let bytes = segment.as_bytes();
        let mut c = 0;
        let space = |c: usize| {
            c + bytes[c..]
                .iter()
                .take_while(|byte| matches!(byte, b' ' | b'\t'))
                .count()
        };
        let skipped = space(c);
        if skipped > c {
            tokens.push((DocKind::Space, at + c, at + skipped));
            c = skipped;
            if c == bytes.len() {
                break 'lines;
            }
        }
        if index > 0 && bytes[c] == b'*' {
            tokens.push((DocKind::Star, at + c, at + c + 1));
            c += 1;
        }
        let skipped = space(c);
        if skipped > c {
            tokens.push((DocKind::Space, at + c, at + skipped));
            c = skipped;
        }
        if c < bytes.len() {
            if bytes[c] == b'@' {
                let tag = 1 + bytes[c + 1..]
                    .iter()
                    .take_while(|byte| {
                        !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0B' | b'\x0C')
                    })
                    .count();
                if tag > 1
                    && !segment[c..c + tag]
                        .to_ascii_lowercase()
                        .starts_with("@phpcs:")
                {
                    tokens.push((DocKind::Tag, at + c, at + c + tag));
                    c += tag;
                    let skipped = space(c);
                    if skipped > c {
                        tokens.push((DocKind::Space, at + c, at + skipped));
                        c = skipped;
                    }
                }
            }
            let eol = segment[c..].find('\n').map_or(bytes.len(), |eol| c + eol);
            if eol > c {
                tokens.push((DocKind::String, at + c, at + eol));
            }
            if eol != bytes.len() {
                tokens.push((DocKind::Space, at + eol, at + bytes.len()));
            }
        }
        at += segment.len();
    }
    tokens.push((DocKind::Close, start + open + body.len(), end));
    for (index, &(kind, from, to)) in tokens.iter().enumerate() {
        if !matches!(kind, DocKind::Tag | DocKind::String) {
            continue;
        }
        let line = lines.line(from);
        let mut found = Neighbours::default();
        for &(kind, from, _) in tokens[..index].iter().rev() {
            if lines.line(from) != line {
                break;
            }
            match kind {
                DocKind::Space => {}
                DocKind::Star => found.tokens = true,
                _ => {
                    found = Neighbours {
                        content: true,
                        tokens: true,
                    };
                    break;
                }
            }
        }
        for &(kind, from, to) in &tokens[index + 1..] {
            if kind == DocKind::Space {
                if source[from..to].contains('\n') {
                    break;
                }
                continue;
            }
            found.tokens = true;
            if kind != DocKind::Star {
                found.content = true;
                break;
            }
        }
        let previous = tokens[index - 1].1;
        pieces.push(Piece {
            text: &source[from..to],
            line,
            first: from == lines.start(line),
            content: found.content,
            tokens: found.tokens,
            own_line: lines.line(previous) != line,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{PhpcsIndex, PhpcsVersion, before_on_line};
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::lines::{LineTable, Terminators};
    use crate::code_rule_dsl::oracle;
    use ast_grep_core::tree_sitter::LanguageExt;
    use serde_json::Value;

    const ORACLE: &str = oracle!("php/suppress/phpcs.expected.json");

    fn reports(file: &Value, key: &str) -> Vec<(String, usize)> {
        file[key]
            .as_array()
            .expect("reports")
            .iter()
            .map(|report| {
                (
                    report[0].as_str().expect("code").to_owned(),
                    report[1].as_u64().expect("start") as usize,
                )
            })
            .collect()
    }

    #[test]
    fn phpcs_annotations_ignore_as_php_codesniffer_honours_them() {
        let oracle: Value = serde_json::from_str(ORACLE).expect("oracle JSON");
        assert_eq!(
            oracle["phpcs4"],
            "PHP_CodeSniffer version 4.0.4 (stable) by Squiz and PHPCSStandards"
        );
        assert_eq!(
            oracle["phpcs3"],
            "PHP_CodeSniffer version 3.13.6 (stable) by Squiz and PHPCSStandards"
        );
        for (name, source) in [
            ("disable.php", oracle!("php/suppress/phpcs/disable.php")),
            ("ignore.php", oracle!("php/suppress/phpcs/ignore.php")),
            (
                "ignore_file.php",
                oracle!("php/suppress/phpcs/ignore_file.php"),
            ),
            ("legacy.php", oracle!("php/suppress/phpcs/legacy.php")),
            (
                "legacy_ignore_file.php",
                oracle!("php/suppress/phpcs/legacy_ignore_file.php"),
            ),
            (
                "terminators.php",
                oracle!("php/suppress/phpcs/terminators.php"),
            ),
            ("unicode.php", oracle!("php/suppress/phpcs/unicode.php")),
        ] {
            let root = GrammarDialect::Php.ast_grep(source);
            let lines = LineTable::new(source, Terminators::Newlines);
            let index = PhpcsIndex::read(&root.root(), source, &lines);
            for (key, version) in [
                ("phpcs4", PhpcsVersion::Four),
                ("phpcs3", PhpcsVersion::Three),
            ] {
                let file = &oracle[name][key];
                let ours = reports(file, "all")
                    .into_iter()
                    .filter(|(code, at)| {
                        !index.suppresses(std::slice::from_ref(code), version, *at, &lines)
                    })
                    .collect::<Vec<_>>();
                assert_eq!(ours, reports(file, "kept"), "{name} {key}");
            }
        }
    }

    /// A comment whose line holds code ending in a multibyte character five bytes before it (as
    /// in laravel's tests/Validation/ValidationEmailRuleTest.php, `#[TestWith(['пример@пример.рф'])]
    /// // Cyrillic domain`, where reading the file panicked): PHP_CodeSniffer 4.0.4 reports only
    /// line 3 of this file under suppress/phpcs/ruleset.xml.
    #[test]
    fn phpcs_annotations_after_multibyte_code_on_their_line() {
        let source = "<?php\nsizeof(['рф']); // phpcs:ignore\nsizeof(['рф']); // other\n";
        let root = GrammarDialect::Php.ast_grep(source);
        let lines = LineTable::new(source, Terminators::Newlines);
        let index = PhpcsIndex::read(&root.root(), source, &lines);
        let code = ["Generic.PHP.ForbiddenFunctions.FoundWithAlternative".to_owned()];
        let ignored = |line: &str| {
            let at = source.find(line).expect("line");
            index.suppresses(&code, PhpcsVersion::Four, at, &lines)
        };
        assert!(ignored("sizeof(['рф']); // phpcs:ignore"));
        assert!(!ignored("sizeof(['рф']); // other"));
    }

    #[test]
    fn trailing_comments_after_multi_byte_characters_follow_content() {
        // The five bytes before each comment start inside `é` or `日`.
        for source in ["<?php\necho 'éx'; // c", "<?php\necho '日x'; # c"] {
            let lines = LineTable::new(source, Terminators::Newlines);
            let at = source.find(['/', '#']).expect("comment");
            let (before, own_line) = before_on_line(source, at, &lines);
            assert!(before.content && before.tokens && !own_line, "{source:?}");
        }
    }
}
