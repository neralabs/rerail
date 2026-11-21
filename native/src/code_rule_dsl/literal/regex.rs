//! Regular expressions in the dialect the engine matches with, the Rust `regex` crate's (decision 0495 §2): a `regex`
//! grammar, hand-written, whose tree a rule set reads as it reads code. A rule file's `match("…", s)` regexes are its
//! programs (`match-regex`), and any string parses as one (`parsed(s, "regex", r)`).
//!
//! ```text
//! a|b*            regex > alternation > (literal "a", repetition "*" > literal "b")
//! (?P<y>\d{4})    regex > group > (name "y", repetition "{4}" > escape "\d")
//! [^a-z[:digit:]] regex > class "^" > (range > (literal, literal), class_name "digit")
//! (?<=a)b         regex > concatenation > (unsupported "(?<=a)", literal "b"): the dialect refuses look-around
//! ```
//!
//! - Kinds: `regex` (the root); `alternation` and `concatenation` of two or more parts (one part stands alone);
//!   `group`, its `name` and `flags` first where it has them; `repetition` over its one part, its value the operator
//!   (`*`, `+?`, `{2,5}`); `class`, its value `^` where it is negated, over its `range`s, `class_name`s, `literal`s,
//!   `escape`s and nested `class`es; `literal`, its value the character; `dot`; `anchor` (`^`, `$`, `\b`, `\B`,
//!   `\A`, `\z`, `\<`, `\>`); `escape` (`\d`, `\pL`, `\x{7f}`, …), its value its text; `flags` (`(?i)`, the `i` of
//!   `(?i:…)`).
//! - `unsupported`: a construct the dialect refuses (a look-around, a back-reference `\1` or `\k<n>`, an atomic or
//!   possessive form), over its bytes, its value what it is.
//! - `error`: a text the parser cannot read at all (an unclosed group or class, a stray `)`, a repetition of
//!   nothing) is a `regex` root over one `error` node, its value the message.

use super::PNode;
use crate::code_rule_dsl::value::Value;

/// The kinds of the `regex` grammar's trees.
pub(crate) const KINDS: &[&str] = &[
    "regex",
    "alternation",
    "concatenation",
    "group",
    "name",
    "repetition",
    "class",
    "range",
    "class_name",
    "literal",
    "dot",
    "anchor",
    "escape",
    "flags",
    "unsupported",
    "error",
];

/// The `regex` grammar (`literal::LiteralGrammar`).
pub(crate) struct RegexGrammar;

/// Its handle, which the engine registers (`literal::install`).
pub const GRAMMAR: super::Grammar = super::Grammar::new(&RegexGrammar);

impl super::LiteralGrammar for RegexGrammar {
    fn name(&self) -> &'static str {
        "regex"
    }
    fn kinds(&self) -> &'static [&'static str] {
        KINDS
    }
    fn host_language(&self) -> crate::code_rule_dsl::language::GrammarDialect {
        crate::code_rule_dsl::language::GrammarDialect::Rr
    }
    fn host(&self) -> &'static str {
        "rule-file strings"
    }
    fn host_kinds(&self) -> &'static [&'static str] {
        &["string"]
    }
    fn parse(&self, grammar: super::Grammar, text: &str) -> PNode {
        tree(grammar, text)
    }
}

/// The tree of `text`: a `regex` root over its alternation, or over one `error` node.
pub fn tree(grammar: super::Grammar, text: &str) -> PNode {
    let mut root = PNode::new(grammar, "regex", 0, text.len());
    let mut p = Parser { g: grammar, s: text, i: 0 };
    match p.alternation(0) {
        Ok(node) if p.i == text.len() => {
            if let Some(node) = node {
                root.children.push(node);
            }
        }
        Ok(_) => root.children.push(error(grammar, text.len(), "a `)` closes no group")),
        Err(message) => root.children.push(error(grammar, text.len(), message)),
    }
    root
}

fn error(grammar: super::Grammar, len: usize, message: &str) -> PNode {
    PNode::new(grammar, "error", 0, len).with_value(Value::Str(message.into()))
}

/// The deepest groups nest: deeper is an `error`, as the dialect's own limit refuses it.
const NEST: usize = 250;

struct Parser<'s> {
    g: super::Grammar,
    s: &'s str,
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }

    fn at(&self, prefix: &str) -> bool {
        self.s[self.i..].starts_with(prefix)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        Some(c)
    }

    fn node(&self, kind: &str, start: usize) -> PNode {
        PNode::new(self.g, kind, start, self.i)
    }

    fn valued(&self, kind: &str, start: usize, value: &str) -> PNode {
        self.node(kind, start).with_value(Value::Str(value.into()))
    }

    /// Parts split by `|`, to the end or a `)`. `None` for an empty text.
    fn alternation(&mut self, depth: usize) -> Result<Option<PNode>, &'static str> {
        if depth > NEST {
            return Err("groups nest too deep");
        }
        let start = self.i;
        let mut parts = vec![self.concatenation(depth)?];
        while self.at("|") {
            self.i += 1;
            parts.push(self.concatenation(depth)?);
        }
        if parts.len() == 1 {
            return Ok(parts.pop().flatten());
        }
        let mut node = self.node("alternation", start);
        // An empty part (`a|`) is an empty concatenation at its place.
        node.children = parts.into_iter().map(|p| p.unwrap_or_else(|| PNode::new(self.g, "concatenation", self.i, self.i))).collect();
        Ok(Some(node))
    }

    /// The items to a `|`, a `)` or the end; one item stands alone, none is `None`.
    fn concatenation(&mut self, depth: usize) -> Result<Option<PNode>, &'static str> {
        let start = self.i;
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let item = self.atom(depth)?;
            items.push(self.repetition(item)?);
        }
        Ok(match items.len() {
            0 => None,
            1 => items.pop(),
            _ => {
                let mut node = self.node("concatenation", start);
                node.children = items;
                Some(node)
            }
        })
    }

    /// `item` and the repetition operators after it (`a*?`, `a{2}`); a second operator repeats the first's node.
    fn repetition(&mut self, mut item: PNode) -> Result<PNode, &'static str> {
        loop {
            let start = self.i;
            let op = match self.peek() {
                Some('*' | '+' | '?') => {
                    self.bump();
                    true
                }
                Some('{') => self.counted(),
                _ => false,
            };
            if !op {
                return Ok(item);
            }
            if self.at("?") {
                self.i += 1;
            } else if self.at("+") {
                // A possessive repetition (`a++`): the dialect has none.
                self.i += 1;
                let mut node = self.valued("unsupported", item.start, "possessive repetition");
                node.children.push(item);
                item = node;
                continue;
            }
            let value = self.s[start..self.i].to_owned();
            let mut node = self.valued("repetition", item.start, &value);
            node.children.push(item);
            item = node;
        }
    }

    /// `{n}`, `{n,}` or `{n,m}` at a `{`, consumed where it is one; a `{` that starts none is left (a literal).
    fn counted(&mut self) -> bool {
        let rest = &self.s[self.i + 1..];
        let Some(close) = rest.find('}') else { return false };
        let body: String = rest[..close].chars().filter(|c| !c.is_whitespace()).collect();
        let mut parts = body.splitn(2, ',');
        let low = parts.next().unwrap_or("");
        let high = parts.next();
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        if digits(low) && high.is_none_or(|h| h.is_empty() || digits(h)) {
            self.i += close + 2;
            return true;
        }
        false
    }

    fn atom(&mut self, depth: usize) -> Result<PNode, &'static str> {
        let start = self.i;
        match self.peek() {
            Some('(') => self.group(depth),
            Some('[') => self.class(depth),
            Some('.') => {
                self.bump();
                Ok(self.node("dot", start))
            }
            Some(c @ ('^' | '$')) => {
                self.bump();
                Ok(self.valued("anchor", start, &c.to_string()))
            }
            Some('\\') => self.escape(),
            Some('*' | '+' | '?') => Err("a repetition repeats nothing"),
            Some(c) => {
                self.bump();
                Ok(self.valued("literal", start, &c.to_string()))
            }
            None => Err("the regex ends early"),
        }
    }

    /// `(…)`, `(?:…)`, `(?flags)`, `(?flags:…)`, `(?P<n>…)`, `(?<n>…)`; a look-around is `unsupported`.
    fn group(&mut self, depth: usize) -> Result<PNode, &'static str> {
        let start = self.i;
        self.i += 1;
        let mut children = Vec::new();
        let mut unsupported = None;
        if self.at("?") {
            self.i += 1;
            if self.at("P<") || (self.at("<") && !self.at("<=") && !self.at("<!")) {
                self.i += if self.at("P<") { 2 } else { 1 };
                let from = self.i;
                let Some(len) = self.s[self.i..].find('>') else { return Err("a group's name is not closed") };
                self.i += len;
                let name = self.s[from..self.i].to_owned();
                children.push(PNode::new(self.g, "name", from, self.i).with_value(Value::Str(name.as_str().into())));
                self.i += 1;
            } else if ["=", "!", "<=", "<!", ">"].iter().any(|p| self.at(p)) {
                unsupported = Some(if self.at(">") { "atomic group" } else { "look-around" });
                self.i += if self.at("<") { 2 } else { 1 };
            } else {
                let from = self.i;
                while self.peek().is_some_and(|c| c.is_ascii_alphabetic() || c == '-') {
                    self.bump();
                }
                if self.i > from {
                    children.push(PNode::new(self.g, "flags", from, self.i).with_value(Value::Str(self.s[from..self.i].into())));
                }
                if self.at(")") {
                    // `(?i)`: the flags alone, for the rest of the group around them.
                    self.i += 1;
                    let mut node = self.node("group", start);
                    node.children = children;
                    return Ok(node);
                }
                if !self.at(":") {
                    return Err("a group's flags end in `:` or `)`");
                }
                self.i += 1;
            }
        }
        if let Some(inner) = self.alternation(depth + 1)? {
            children.push(inner);
        }
        if !self.at(")") {
            return Err("a group is not closed");
        }
        self.i += 1;
        let mut node = match unsupported {
            Some(what) => self.valued("unsupported", start, what),
            None => self.node("group", start),
        };
        node.children = children;
        Ok(node)
    }

    /// `[…]`: an optional `^`, then ranges, `[:name:]`s, escapes, literals and nested classes; `&&`, `--` and `~~`
    /// between them are the class's operators (anonymous).
    fn class(&mut self, depth: usize) -> Result<PNode, &'static str> {
        if depth > NEST {
            return Err("classes nest too deep");
        }
        let start = self.i;
        self.i += 1;
        let negated = self.at("^");
        if negated {
            self.i += 1;
        }
        let mut children: Vec<PNode> = Vec::new();
        let mut first = true;
        loop {
            let from = self.i;
            match self.peek() {
                None => return Err("a class is not closed"),
                Some(']') if !first => {
                    self.i += 1;
                    break;
                }
                Some('[') if self.at("[:") => {
                    let Some(len) = self.s[self.i + 2..].find(":]") else { return Err("a class name is not closed") };
                    let name = self.s[self.i + 2..self.i + 2 + len].trim_start_matches('^').to_owned();
                    self.i += len + 4;
                    children.push(self.valued("class_name", from, &name));
                }
                Some('[') => children.push(self.class(depth + 1)?),
                _ if ["&&", "--", "~~"].iter().any(|op| self.at(op)) => self.i += 2,
                _ => {
                    let item = self.class_item()?;
                    // `a-z`: a range, unless the `-` ends the class.
                    if self.at("-") && !self.at("-]") && !self.at("--") {
                        self.i += 1;
                        let high = self.class_item()?;
                        let mut range = self.node("range", from);
                        range.children = vec![item, high];
                        children.push(range);
                    } else {
                        children.push(item);
                    }
                }
            }
            first = false;
        }
        let mut node = if negated { self.valued("class", start, "^") } else { self.node("class", start) };
        node.children = children;
        Ok(node)
    }

    /// A class's one character: a literal or an escape.
    fn class_item(&mut self) -> Result<PNode, &'static str> {
        let start = self.i;
        match self.peek() {
            Some('\\') => self.escape(),
            Some(c) => {
                self.bump();
                Ok(self.valued("literal", start, &c.to_string()))
            }
            None => Err("a class is not closed"),
        }
    }

    /// `\…`: an anchor (`\b`, `\A`, …), an escaped literal (`\.`), a class (`\d`, `\pL`, `\p{Greek}`), a code point
    /// (`\x7f`, `\u{1F600}`), or a back-reference, which the dialect refuses.
    fn escape(&mut self) -> Result<PNode, &'static str> {
        let start = self.i;
        self.i += 1;
        let Some(c) = self.bump() else { return Err("a `\\` ends the regex") };
        match c {
            'b' | 'B' | 'A' | 'z' | '<' | '>' => Ok(self.valued("anchor", start, &self.s[start..self.i])),
            '1'..='9' => {
                while self.peek().is_some_and(|d| d.is_ascii_digit()) {
                    self.bump();
                }
                Ok(self.valued("unsupported", start, "back-reference"))
            }
            'k' if self.at("<") => {
                let Some(len) = self.s[self.i..].find('>') else { return Err("a back-reference's name is not closed") };
                self.i += len + 1;
                Ok(self.valued("unsupported", start, "back-reference"))
            }
            'p' | 'P' | 'x' | 'u' | 'U' => {
                if self.at("{") {
                    let Some(len) = self.s[self.i..].find('}') else { return Err("a `{` in an escape is not closed") };
                    self.i += len + 1;
                } else {
                    let n = match c {
                        'p' | 'P' => 1,
                        'x' => 2,
                        'u' => 4,
                        _ => 8,
                    };
                    for _ in 0..n {
                        if self.bump().is_none() {
                            return Err("an escape ends early");
                        }
                    }
                }
                Ok(self.valued("escape", start, &self.s[start..self.i]))
            }
            c if c.is_ascii_alphanumeric() => Ok(self.valued("escape", start, &self.s[start..self.i])),
            // `\.`, `\\`, `\[`: the character itself.
            c => Ok(self.valued("literal", start, &c.to_string())),
        }
    }
}
