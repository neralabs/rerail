//! `parse` extensions: the `identifierWords` grammar, the pythonFString `interpolation` marker, the
//! text grammars' tree-rule compilation, and `host` (a text grammar's tree rule testing the host
//! node its `parse` holds).
//!
//! Ported from native (the source of truth):
//! - `literal/identifier_words.rs` (L9-30): `[A-Za-z_][A-Za-z0-9_]*` words of the decoded text;
//!   root `words` (no value), children `word` (value: the word as a str).
//! - `literal/offsets.rs` `decode_python` (L69-129) + `Decoded::host_range` (L40-57): the decoded
//!   text and each node's host range (`crate::fstring::{Decoded, decode_body}` is the shared port).
//! - `literal/mod.rs` `parse_string` (L819-845): a text grammar over a `string` /
//!   `concatenated_string`; `Decode::Unknown` → `Parsed::Unknown`, `NotString` → no tree.
//! - `literal/python_fstring.rs` + `compile/parse.rs` L364-368: `interpolation` (default `{}`) is
//!   each slot's text; `crate::fstring::parse(f, n, marker)` builds the pieces.
//! - `compile/parse.rs` `ParseTest` host kinds (L376-384): pythonFString `string`,
//!   `concatenated_string`, `format_specifier`; identifierWords `string`, `concatenated_string`.
//! - `compile/parse.rs` `validate_sub_rule` (L109-280) and `SUB_RULE_KEYS` (L32-47): a text
//!   grammar's tree rule takes `kind regex all any not inside has precedes follows count compare
//!   value capture host`; its kinds are the grammar's; relations take no `field`; count no `as`.
//! - `compile/parse.rs` `compile_sub_rule` L596-614: `host` compiles as a host-language rule and is
//!   the sub-rule's last term; `compile/ast_rule.rs` L705-711: `host` outside a parse's tree rule is
//!   an error (no embedded documents in Python).
//! - `matcher/parse.rs` `HostTest::match_node` (L110-134), `host_exports` (L138-161),
//!   `ParseTest::match_tree` (L207-250) and the `HOOKS` stack (L64-108): the host rule runs on the
//!   host node in the host's environment as the `parse` began, with the tree's captures bound so
//!   far exported into it; it binds nothing; `violation.each` is suspended. A node of no tree being
//!   matched under a `parse` (or a file node) does not match.
//! - `matcher/exported.rs` `export` (L35-60) + `matcher/compare.rs` L112-125: an export does not
//!   shadow a capture bound to a node (reads take the node match first).
//! - Captures (`matcher/captures/semantic.rs`): `Host` binds, reads and mentions nothing (L47, L92,
//!   L143) and capture analysis does not enter its rule; `Semantic::reads_values` enters it
//!   (`matcher/semantic.rs` L79).
//!
//! Refusals (named):
//! - `identifierWords: \N{...} escape` (via `fstring::decode_body`): the Unicode name table is not
//!   available; native decodes the name or reads the literal as unknown.
//! - `parse: tree rule key` / `parse: kind` / `parse: field` / `parse: count as` / `host outside
//!   parse`: native compile errors, reported as `Err`.
use super::*;
use crate::fstring::{Decoded, decode_body};
use crate::pyvalue::{MAX_STRING_BYTES, prefix};
use std::cell::{Cell, RefCell};

/// The kinds of pythonFString trees (`literal/python_fstring.rs` `KINDS`).
pub(super) const FSTRING_KINDS: &[&str] = &["template", "literal", "interpolation"];
/// The kinds of identifierWords trees (`literal/identifier_words.rs` `KINDS`).
pub(super) const WORDS_KINDS: &[&str] = &["words", "word"];

/// Keys a text grammar's tree rule may use (`SUB_RULE_KEYS` plus `host`).
const TREE_RULE_KEYS: &[&str] = &[
    "kind", "regex", "all", "any", "not", "inside", "has", "precedes", "follows", "count", "compare",
    "value", "capture", "host",
];
const RELATION_KEYS: &[&str] = &["inside", "has", "precedes", "follows"];

/// A text grammar's parse of one host node: its nodes (index 0 the root, whose `parent` is
/// `NONE`; other parents index this vector), not a string the grammar reads, or unknown.
pub(super) enum TextTree {
    Tree(Vec<ParsedNode>),
    NotString,
    Unknown,
}

/// `host` inside a text grammar's tree rule: `rule` (a host-language rule) on the host node.
#[derive(Clone)]
pub struct Host {
    pub rule: Rule,
}

impl Host {
    /// Every nested rule, for `reads_values` and `parses` only: native capture analysis
    /// (binds/reads/mentions) does not enter a host rule — use `capture_sub_rules` there.
    pub fn sub_rules(&self) -> Vec<&Rule> {
        vec![&self.rule]
    }
    /// The nested rules capture analysis enters: none.
    pub fn capture_sub_rules(&self) -> Vec<&Rule> {
        Vec::new()
    }
    pub fn binds(&self) -> Vec<Name> {
        Vec::new()
    }
    pub fn reads(&self) -> Vec<Name> {
        Vec::new()
    }
}

thread_local! {
    /// Compiling a text grammar's tree rule (outside its host rules): `host` is valid.
    static IN_TREE: Cell<bool> = const { Cell::new(false) };
    /// `host` terms compiled so far (whether a tree rule holds one).
    static HOSTS_COMPILED: Cell<u64> = const { Cell::new(0) };
    /// The `parse` terms being matched whose tree rules hold `host`, innermost last (native
    /// `HOOKS`).
    static FRAMES: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

struct Frame {
    /// The evaluation the frame belongs to (its address): parsed indices are per evaluation.
    eval: usize,
    /// The tree's parsed-node range `[root, end)`.
    tree: (u32, u32),
    /// The host node the `parse` holds.
    host: u32,
    /// The host environment as the `parse` began.
    before: Env,
}

/// Pops the frame `with_host` pushed, whatever happens.
struct Pushed;
impl Drop for Pushed {
    fn drop(&mut self) {
        FRAMES.with_borrow_mut(|frames| frames.pop());
    }
}

/// Exports a tree capture into a host environment (`exported.rs` `export`): a capture bound to a
/// host node keeps its node (reads take it first); otherwise the export binds the name.
pub(super) fn export(env: &mut Env, name: Name, b: Bound) {
    if !matches!(env.get(name), Some(Bound::Node(_))) {
        env.set(name, b);
    }
}

/// The value a text grammar gives a parsed node (`SubTree::value`): pythonFString's `template`
/// (the decoded text), `literal` (its text) and `interpolation` (the marker), identifierWords'
/// `word`; `words` has none. Only for nodes of text-grammar trees: an annotation tree's Python
/// kinds (`interpolation` among them) have values of their own.
pub(super) fn text_node_value(kind: u32, text: &str) -> Option<PyValue> {
    let kind = facts::name(kind);
    matches!(&*kind, "template" | "literal" | "interpolation" | "word").then(|| PyValue::Str(text.to_owned()))
}

enum Decode {
    Known(Decoded),
    Unknown,
    NotString,
}

/// `literal/offsets.rs` `decode_python`: a `string` or `concatenated_string` decoded as `str`.
fn decode_python(f: &Facts, n: u32) -> Result<Decode, String> {
    let is = |c: u32, kind: &str| &*facts::name(f.kind[c as usize]) == kind;
    let parts: Vec<u32> = if is(n, "string") {
        vec![n]
    } else if is(n, "concatenated_string") {
        f.children(n).iter().copied().filter(|&c| is(c, "string")).collect()
    } else {
        return Ok(Decode::NotString);
    };
    let mut d = Decoded { text: String::new(), spans: Vec::new(), host_start: f.start[n as usize] as usize };
    for &part in &parts {
        if f.children(part).iter().any(|&c| is(c, "interpolation")) {
            return Ok(Decode::Unknown);
        }
        let text = f.text(part);
        let base = f.start[part as usize] as usize;
        let Some(quote_at) = text.find(['\'', '"']) else { return Ok(Decode::Unknown) };
        let Some(pref) = prefix(&text[..quote_at]) else { return Ok(Decode::Unknown) };
        if pref.template || pref.bytes {
            return Ok(Decode::Unknown);
        }
        let rest = &text[quote_at..];
        let quote = if rest.starts_with("'''") || rest.starts_with("\"\"\"") { 3 } else { 1 };
        if rest.len() < 2 * quote || !rest.ends_with(&rest[..quote]) {
            return Ok(Decode::Unknown);
        }
        let body = &rest[quote..rest.len() - quote];
        let decoded = decode_body(body, base + quote_at + quote, &pref, &mut d)
            .map_err(|why| format!("identifierWords: {why}"))?;
        if !decoded || d.text.len() > MAX_STRING_BYTES {
            return Ok(Decode::Unknown);
        }
    }
    Ok(Decode::Known(d))
}

/// The identifierWords tree of `n` (`literal/mod.rs` `parse_string` with
/// `identifier_words::parse`).
pub(super) fn identifier_words_tree(f: &Facts, n: u32) -> Result<TextTree, String> {
    let d = match decode_python(f, n)? {
        Decode::Known(d) => d,
        Decode::Unknown => return Ok(TextTree::Unknown),
        Decode::NotString => return Ok(TextTree::NotString),
    };
    let text = &d.text;
    let (rs, re) = d.host_range(0, text.len());
    let mut nodes = vec![ParsedNode { kind: facts::intern("words"), start: rs, end: re, text: text.clone(), parent: NONE, named: true }];
    let word = facts::intern("word");
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if !c.is_ascii_alphabetic() && c != '_' {
            continue;
        }
        let mut end = start + 1;
        while let Some(&(at, next)) = chars.peek() {
            if !next.is_ascii_alphanumeric() && next != '_' {
                break;
            }
            end = at + 1;
            chars.next();
        }
        let (hs, he) = d.host_range(start, end);
        nodes.push(ParsedNode { kind: word, start: hs, end: he, text: text[start..end].to_owned(), parent: 0, named: true });
    }
    Ok(TextTree::Tree(nodes))
}

/// The pythonFString tree of `n` with interpolation slots reading `marker`
/// (`literal/python_fstring.rs` `parse`): a `template` root over `literal`/`interpolation`
/// children. The root spans the first piece's start to the last one's end, which is
/// `host_range(0, len)` (empty text: the host's start).
pub(super) fn fstring_tree(f: &Facts, n: u32, marker: &str) -> Result<TextTree, String> {
    let pieces = match crate::fstring::parse(f, n, marker)? {
        crate::fstring::FParse::Tree(pieces) => pieces,
        crate::fstring::FParse::NotString => return Ok(TextTree::NotString),
        crate::fstring::FParse::Unknown => return Ok(TextTree::Unknown),
    };
    let text: String = pieces.iter().map(|p| p.text.as_str()).collect();
    let (s, e) = match (pieces.first(), pieces.last()) {
        (Some(a), Some(b)) => (a.start, b.end),
        _ => (f.start[n as usize], f.start[n as usize]),
    };
    let mut nodes = vec![ParsedNode { kind: facts::intern("template"), start: s, end: e, text, parent: NONE, named: true }];
    for piece in pieces {
        nodes.push(ParsedNode { kind: facts::intern(piece.kind), start: piece.start, end: piece.end, text: piece.text, parent: 0, named: true });
    }
    Ok(TextTree::Tree(nodes))
}

/// Native `validate_sub_rule` for a text grammar's tree rule (`hosted`): keys, kinds, relations
/// without `field`, count without `as`. A `host` rule is the host's and is not walked.
fn validate_tree_rule(v: &Json, path: &str, grammar: &str, kinds: &[&str]) -> Result<(), String> {
    let rule = obj(v, path)?;
    for key in rule.keys() {
        if !TREE_RULE_KEYS.contains(&key.as_str()) {
            return Err(format!("{path}/{key}: parse: tree rule key `{key}` is not supported inside parse ({grammar} trees)"));
        }
    }
    if rule.is_empty() {
        return Err(format!("{path} must contain at least one matcher of the parsed tree"));
    }
    if let Some(kind) = rule.get("kind") {
        let kind = kind.as_str().filter(|k| !k.is_empty()).ok_or_else(|| format!("{path}/kind must be a non-empty string"))?;
        if !kinds.contains(&kind) {
            return Err(format!("{path}/kind: parse: kind `{kind}` is not a {grammar} kind; expected one of {}", kinds.join(", ")));
        }
    }
    if rule.get("value").and_then(|value| value.get("follow")).is_some() {
        return Err(format!("{path}/value/follow: a parsed node has no binding to follow"));
    }
    for key in ["all", "any"] {
        if let Some(items) = rule.get(key) {
            let items = items.as_array().ok_or_else(|| format!("{path}/{key} must be an array"))?;
            if items.is_empty() {
                return Err(format!("{path}/{key} must not be empty"));
            }
            for (index, item) in items.iter().enumerate() {
                validate_tree_rule(item, &format!("{path}/{key}/{index}"), grammar, kinds)?;
            }
        }
    }
    if let Some(not) = rule.get("not") {
        validate_tree_rule(not, &format!("{path}/not"), grammar, kinds)?;
    }
    let relation = |value: &Json, path: &str| -> Result<(), String> {
        let o = obj(value, path)?;
        if o.contains_key("field") {
            return Err(format!("{path}/field: parse: field is not supported in a parsed tree's rule"));
        }
        if let Some(stop @ Json::Object(_)) = o.get("stopBy") {
            validate_tree_rule(stop, &format!("{path}/stopBy"), grammar, kinds)?;
        }
        let mut nested = o.clone();
        nested.remove("stopBy");
        validate_tree_rule(&Json::Object(nested), path, grammar, kinds)
    };
    for key in RELATION_KEYS {
        if let Some(value) = rule.get(*key) {
            relation(value, &format!("{path}/{key}"))?;
        }
    }
    if let Some(count) = rule.get("count") {
        let count_path = format!("{path}/count");
        let o = obj(count, &count_path)?;
        if o.contains_key("as") {
            return Err(format!(
                "{count_path}/as: parse: count as — a parsed tree's count binds nothing; count in the ast query's own rule to report its value"
            ));
        }
        for key in RELATION_KEYS {
            if let Some(value) = o.get(*key) {
                relation(value, &format!("{count_path}/{key}"))?;
            }
        }
    }
    Ok(())
}

impl Compiler<'_> {
    /// A text grammar's (pythonFString, identifierWords) tree rule: validated as native's
    /// `validate_sub_rule`, compiled with `host` allowed. Returns the rule and whether it holds a
    /// `host` (native `host_sound.is_some()`), which `match_text_tree` takes as `hosted`.
    pub(super) fn compile_tree_rule(&mut self, v: &Json, at: &str, grammar: ParseGrammar) -> Result<(Rule, bool), String> {
        let (name, kinds) = match grammar {
            ParseGrammar::PythonFString => ("pythonFString", FSTRING_KINDS),
            ParseGrammar::IdentifierWords => ("identifierWords", WORDS_KINDS),
            ParseGrammar::Annotation => return Err(format!("{at}: annotation trees are not text-grammar trees")),
        };
        validate_tree_rule(v, at, name, kinds)?;
        let before = HOSTS_COMPILED.get();
        let outer = IN_TREE.replace(true);
        let rule = self.rule(v, at);
        IN_TREE.set(outer);
        Ok((rule?, HOSTS_COMPILED.get() != before))
    }

    /// `host: RULE` inside a text grammar's tree rule. The glue pushes it as the rule's last term
    /// (after `value`), as `compile_sub_rule` does.
    pub(super) fn compile_host(&mut self, v: &Json, at: &str) -> Result<Rule, String> {
        let path = format!("{at}/host");
        if !IN_TREE.get() {
            return Err(format!("{path}: host outside parse — host tests the literal a parse holds; use it inside parse"));
        }
        HOSTS_COMPILED.set(HOSTS_COMPILED.get() + 1);
        // A host rule is the host language's: `host` is invalid inside it.
        let outer = IN_TREE.replace(false);
        let rule = self.rule(v, &path);
        IN_TREE.set(outer);
        Ok(Rule::Host(Box::new(Host { rule: rule? })))
    }
}

impl Eval<'_> {
    fn host_eval_id(&self) -> usize {
        std::ptr::from_ref(self).cast::<()>() as usize
    }

    /// Appends a text tree's nodes to the parsed side table: its `[root, end)` range.
    pub(super) fn append_tree(&self, nodes: Vec<ParsedNode>) -> (u32, u32) {
        let mut parsed = self.parsed.borrow_mut();
        let base = parsed.len() as u32;
        for mut node in nodes {
            if node.parent != NONE {
                node.parent += base;
            }
            parsed.push(node);
        }
        (base, parsed.len() as u32)
    }

    /// The text grammar's tree at host node `n` (`ParseTest::tree_at`, uncached: the glue caches
    /// by (node, grammar, marker)): `Some((root, end))` a tree, `Some((NONE, NONE))` unknown,
    /// `None` no tree (not a host kind, not a string the grammar reads, or refused).
    pub(super) fn text_tree(&self, n: u32, grammar: ParseGrammar, marker: &str) -> Option<(u32, u32)> {
        let hosts: &[&str] = match grammar {
            ParseGrammar::PythonFString => &["string", "concatenated_string", "format_specifier"],
            ParseGrammar::IdentifierWords => &["string", "concatenated_string"],
            ParseGrammar::Annotation => return None,
        };
        if !self.f.named(n) || !hosts.contains(&&*facts::name(self.f.kind[n as usize])) {
            return None;
        }
        let built = match grammar {
            ParseGrammar::PythonFString => fstring_tree(self.f, n, marker),
            _ => identifier_words_tree(self.f, n),
        };
        match built {
            Ok(TextTree::Tree(nodes)) => Some(self.append_tree(nodes)),
            Ok(TextTree::NotString) => None,
            Ok(TextTree::Unknown) => Some((NONE, NONE)),
            Err(why) => {
                self.refuse(why);
                None
            }
        }
    }

    /// `ParseTest::match_node` past `tree_at` for a text grammar: `tree` from `text_tree` (unknown
    /// → unknown verdict); the rule runs on the root in a fresh environment, with this `parse`'s
    /// host frame entered when `hosted`; on a match its captures are exported into `env`.
    pub(super) fn match_text_tree(&self, host: u32, tree: (u32, u32), rule: &Rule, hosted: bool, env: &mut Env) -> bool {
        if tree.0 == NONE {
            self.unknown.set(true);
            return false;
        }
        let mut inner = Env::default();
        let matched = if hosted {
            self.with_host(host, tree, env, || self.m(rule, Bound::Parsed(tree.0), &mut inner))
        } else {
            self.m(rule, Bound::Parsed(tree.0), &mut inner)
        };
        if !matched {
            return false;
        }
        for (name, b) in inner.vars {
            export(env, name, b);
        }
        true
    }

    /// Runs `f` with the `parse` of `host` (tree `tree`) as the innermost host frame (native
    /// `Hosted::enter`), the host environment `env` as it stands now.
    pub(super) fn with_host<T>(&self, host: u32, tree: (u32, u32), env: &Env, f: impl FnOnce() -> T) -> T {
        FRAMES.with_borrow_mut(|frames| frames.push(Frame { eval: self.host_eval_id(), tree, host, before: env.clone() }));
        let _pop = Pushed;
        f()
    }

    /// Native `HostTest::match_node`: on a node of a tree whose `parse` is being matched, the host
    /// rule on the host node in the host environment as the `parse` began, with this tree's
    /// captures bound so far in `env` exported into it. Binds nothing.
    pub(super) fn match_host(&self, x: &Host, node: Bound, env: &mut Env) -> bool {
        let Bound::Parsed(p) = node else { return false };
        let eval = self.host_eval_id();
        let frame = FRAMES.with_borrow(|frames| {
            frames
                .iter()
                .rev()
                .find(|fr| fr.eval == eval && fr.tree.0 <= p && p < fr.tree.1)
                .map(|fr| (fr.host, fr.tree, fr.before.clone()))
        });
        let Some((host, tree, mut host_env)) = frame else { return false };
        for &(name, b) in &env.vars {
            if matches!(b, Bound::Parsed(q) if tree.0 <= q && q < tree.1) {
                export(&mut host_env, name, b);
            }
        }
        self.suspended(|| self.m(&x.rule, Bound::Node(host), &mut host_env))
    }
}
