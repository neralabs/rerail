//! `formatArgs` (a Rerail extension): which arguments of a format call no field of its template
//! consumes, and which fields no argument fills: for Python as Ruff 0.16.9's Pyflakes rules read
//! them (`rules/pyflakes/rules/strings.rs`: F504, F505, F507, F522, F523, F524), for PHP's
//! printf-family calls as PHP 8.5.11 and PHPStan 2.2.16 read them (`printf`), and for its `sscanf`
//! and `fscanf` calls as PHP 8.5.11 validates them (`scanf`).

use super::AstMatcher;
use super::each::{self, HostRanges};
use super::exported::export;
use super::parse::host_map;
use super::parse::parsed_tree;
use super::util::{THREE_VALUED, UNKNOWN};
use super::value::Reading;
use crate::code_rule_dsl::identifier::python_identifier;
use crate::code_rule_dsl::literal::{Grammar, Parsed, SubDoc, offsets};
use crate::code_rule_dsl::value::Value;
use ast_grep_core::Matcher;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{AstGrep, Doc, Node};
use std::borrow::Cow;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Style {
    /// `str.format` templates (`braceFormat`).
    Brace,
    /// printf-style formats (`percentFormat`).
    Percent,
    /// PHP's printf-family formats (`printfFormat`).
    Printf,
    /// PHP's `sscanf`/`fscanf` formats (`scanfFormat`).
    Scanf,
}

impl Style {
    pub(crate) fn grammar(self) -> Grammar {
        match self {
            Self::Brace => Grammar::Brace,
            Self::Percent => Grammar::Percent,
            Self::Printf => Grammar::Printf,
            Self::Scanf => Grammar::Scanf,
        }
    }
}

#[derive(Clone)]
pub(crate) struct FormatArgs {
    pub(crate) style: Style,
    /// The capture holding the format string.
    pub(crate) string: String,
    /// The capture holding the arguments: a call's argument list (a `$$$` capture), or the right
    /// operand of `%`.
    pub(crate) args: String,
    /// Matched on the host arguments no field consumes; binds into the host environment.
    pub(crate) unused: Option<AstMatcher>,
    /// Matched on the template's fields (placeholders and stars for `%`) no argument fills.
    pub(crate) missing: Option<AstMatcher>,
    /// The captures `missing` may bind, exported to the host.
    pub(crate) exports: Vec<String>,
    /// How a PHP format that is no string literal is read: its value through bindings (`printf`
    /// and `scanf`); `None` for Python's styles, which read a literal only.
    pub(crate) reading: Option<Reading>,
}

/// One side's candidates: nodes and whether each is known to be unused (or missing); `incomplete`
/// when others may be that the analysis cannot name (a `**` splat, a dynamic key).
struct Side<T> {
    candidates: Vec<(T, bool)>,
    incomplete: bool,
}

impl<T> Side<T> {
    fn known(candidates: Vec<(T, bool)>) -> Self {
        Self {
            candidates,
            incomplete: false,
        }
    }

    fn unknown() -> Self {
        Self {
            candidates: Vec::new(),
            incomplete: true,
        }
    }

    fn merge(mut self, other: Self) -> Self {
        self.candidates.extend(other.candidates);
        self.incomplete |= other.incomplete;
        self
    }
}

/// A template's fields: `(node id, key)`, the key an argument index or a keyword.
struct Fields {
    fields: Vec<(usize, Key)>,
}

#[derive(Clone, PartialEq)]
enum Key {
    Index(usize),
    Name(String),
}

fn key_of(value: Option<&Value>) -> Option<Key> {
    match value? {
        Value::Str(name) => Some(Key::Name(name.clone())),
        Value::Int(index) => usize::try_from(index).ok().map(Key::Index),
        _ => None,
    }
}

/// What a format call's arguments and template give before any sub-rule is matched.
enum Prepared<'t, D: Doc> {
    NoMatch,
    Unknown,
    Sides {
        tree: std::sync::Arc<crate::code_rule_dsl::literal::SubTree>,
        unused: Side<Node<'t, D>>,
        missing: Side<usize>,
    },
}

impl FormatArgs {
    /// The call's unused arguments and missing fields, read from the captures in `env`.
    fn prepare<'t, D: Doc>(&self, env: &MetaVarEnv<'t, D>) -> Prepared<'t, D> {
        match (self.style, &self.reading) {
            (Style::Brace, _) => self.prepare_python(env, true),
            (Style::Percent, _) => self.prepare_python(env, false),
            (Style::Printf, Some(reading)) => {
                printf::prepare(&self.string, &self.args, reading, env)
            }
            (Style::Scanf, Some(reading)) => scanf::prepare(&self.string, &self.args, reading, env),
            (Style::Printf | Style::Scanf, None) => {
                unreachable!("PHP's styles compile with a reading")
            }
        }
    }

    /// A Python `.format` call (`brace`) or `%` format: Ruff's AST has no parentheses, and its
    /// F50x and F52x read a plain `str` literal only.
    fn prepare_python<'t, D: Doc>(&self, env: &MetaVarEnv<'t, D>, brace: bool) -> Prepared<'t, D> {
        let Some(string) = env.get_match(&self.string) else {
            return Prepared::NoMatch;
        };
        let string = unparenthesized(string.clone());
        if !plain_literal(&string) {
            if matches!(string.kind().as_ref(), "string" | "concatenated_string") {
                return Prepared::Unknown;
            }
            return Prepared::NoMatch;
        }
        let tree = match parsed_tree(&string, self.style.grammar()) {
            None | Some(Parsed::NotString) => return Prepared::NoMatch,
            Some(Parsed::Unknown) => return Prepared::Unknown,
            Some(Parsed::Tree(tree)) => tree,
        };
        let root = AstGrep::doc(SubDoc { tree: tree.clone() });
        let parsed = root.root();
        // A template Ruff refuses (F501, F521) has no fields to consume anything.
        if parsed.children().any(|part| part.kind() == "error") {
            return Prepared::Unknown;
        }
        let args: Vec<Node<'t, D>> = match env.get_match(&self.args) {
            Some(operand) => vec![operand.clone()],
            None => env
                .get_multiple_matches(&self.args)
                .into_iter()
                .filter(|arg| arg.is_named() && arg.kind() != "comment")
                .collect(),
        };
        let operand = env.get_match(&self.args).is_some();
        let fields = Fields {
            fields: parsed
                .dfs()
                .filter(|part| {
                    if brace {
                        part.kind() == "field"
                    } else {
                        matches!(part.kind().as_ref(), "placeholder" | "star")
                    }
                })
                .filter_map(|part| Some((part.node_id(), key_of(tree.value(part.node_id()))?)))
                .collect(),
        };
        let (unused, missing) = if brace {
            self::brace(&args, &fields)
        } else if operand {
            percent_operand(&args[0], &fields)
        } else {
            percent_list(&args, &fields)
        };
        Prepared::Sides {
            tree,
            unused,
            missing,
        }
    }

    pub(super) fn match_node<'t, D: Doc>(
        &self,
        node: Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        let (tree, unused, missing) = match self.prepare(env) {
            Prepared::NoMatch => return None,
            Prepared::Unknown => {
                UNKNOWN.set(true);
                return None;
            }
            Prepared::Sides {
                tree,
                unused,
                missing,
            } => (tree, unused, missing),
        };
        let root = AstGrep::doc(SubDoc { tree: tree.clone() });
        let parsed = root.root();
        let three = THREE_VALUED.get();
        let saved = three && UNKNOWN.replace(false);
        let mut verdict = Some(true);
        let mut env_after = Cow::Borrowed(env.as_ref());
        if let Some(rule) = &self.unused {
            let found = first(rule, unused.candidates, &mut env_after);
            verdict = and(verdict, open(found, unused.incomplete));
        }
        if let Some(rule) = &self.missing
            && verdict != Some(false)
        {
            let nodes = parsed
                .dfs()
                .filter_map(|part| {
                    missing
                        .candidates
                        .iter()
                        .find(|(id, _)| *id == part.node_id())
                        .map(|(_, known)| (part, *known))
                })
                .collect();
            let mut inner = Cow::Owned(MetaVarEnv::new());
            let hosts = each::active().then(|| HostRanges::enter(host_map(&tree)));
            let found = first(rule, nodes, &mut inner);
            drop(hosts);
            if found == Some(true) {
                for name in &self.exports {
                    if let Some(capture) = inner.get_match(name)
                        && let Some(range) = tree.host_range(capture.node_id())
                    {
                        export(&mut env_after, name, &capture.text(), range);
                    }
                }
            }
            verdict = and(verdict, open(found, missing.incomplete));
        }
        match verdict {
            Some(true) => {
                if three {
                    UNKNOWN.set(saved);
                }
                if let Cow::Owned(bound) = env_after {
                    *env = Cow::Owned(bound);
                }
                Some(node)
            }
            Some(false) => {
                if three {
                    UNKNOWN.set(saved);
                }
                None
            }
            None => {
                UNKNOWN.set(true);
                None
            }
        }
    }
}

/// A search's verdict when more candidates may exist than it saw: a miss is then unknown.
fn open(found: Option<bool>, incomplete: bool) -> Option<bool> {
    match found {
        Some(false) if incomplete => None,
        other => other,
    }
}

/// Three-valued `and`.
fn and(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    match (left, right) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

/// Whether some candidate matches `rule`, binding from the first known one that does: `Some(true)`,
/// `None` when only unknown candidates (or unknown verdicts) could, `Some(false)` otherwise.
fn first<'t, D: Doc>(
    rule: &AstMatcher,
    candidates: Vec<(Node<'t, D>, bool)>,
    env: &mut Cow<MetaVarEnv<'t, D>>,
) -> Option<bool> {
    let mut unknown = false;
    for (candidate, known) in candidates {
        let mut trial = Cow::Borrowed(env.as_ref());
        UNKNOWN.set(false);
        let matched = rule.match_node_with_env(candidate, &mut trial).is_some();
        // `violation.each`: a candidate that binds the enumerated capture to a reported range is
        // passed over.
        if matched && each::active() && each::excluded(&trial) {
            UNKNOWN.set(false);
            continue;
        }
        if matched && known {
            UNKNOWN.set(false);
            if let Cow::Owned(bound) = trial {
                *env = Cow::Owned(bound);
            }
            return Some(true);
        }
        unknown |= matched || UNKNOWN.get();
        UNKNOWN.set(false);
    }
    (!unknown).then_some(false)
}

/// The expression inside parentheses, as Ruff's AST has none.
fn unparenthesized<'t, D: Doc>(mut node: Node<'t, D>) -> Node<'t, D> {
    while node.kind() == "parenthesized_expression" {
        let mut inner = node
            .children()
            .filter(|child| child.is_named() && child.kind() != "comment");
        let (Some(expression), None) = (inner.next(), inner.next()) else {
            break;
        };
        drop(inner);
        node = expression;
    }
    node
}

fn named<'t, D: Doc>(node: &Node<'t, D>) -> Vec<Node<'t, D>> {
    node.children()
        .filter(|child| child.is_named() && child.kind() != "comment")
        .collect()
}

/// `str.format(...)`: positional arguments by index (F523 skips starred ones and reads the rest
/// by position), keywords by name (F522, unknown past `**kw`); fields whose index is past the
/// positional arguments or whose keyword is not passed (F524, unknown past `*args` or `**kw`).
fn brace<'t, D: Doc>(args: &[Node<'t, D>], fields: &Fields) -> (Side<Node<'t, D>>, Side<usize>) {
    let mut positional = Vec::new();
    let mut keywords = Vec::new();
    let mut star_args = false;
    let mut star_kwargs = false;
    for arg in args {
        match arg.kind().as_ref() {
            "keyword_argument" => {
                if let Some(name) = arg.field("name") {
                    // A keyword's name is an identifier, NFKC-normalized; a field's name is
                    // string content, read as written (CPython's `str.format` looks it up raw).
                    keywords.push((arg.clone(), python_identifier(&name).into_owned()));
                }
            }
            "dictionary_splat" => star_kwargs = true,
            "list_splat" => {
                star_args = true;
                positional.push((arg.clone(), true));
            }
            _ => positional.push((arg.clone(), false)),
        }
    }
    let indices: BTreeSet<usize> = fields
        .fields
        .iter()
        .filter_map(|(_, key)| match key {
            Key::Index(index) => Some(*index),
            Key::Name(_) => None,
        })
        .collect();
    let names: BTreeSet<&str> = fields
        .fields
        .iter()
        .filter_map(|(_, key)| match key {
            Key::Name(name) => Some(name.as_str()),
            Key::Index(_) => None,
        })
        .collect();
    let mut unused = Vec::new();
    for (index, (arg, starred)) in positional.iter().enumerate() {
        if !starred && !indices.contains(&index) {
            unused.push((arg.clone(), true));
        }
    }
    for (arg, name) in &keywords {
        if !names.contains(name.as_str()) {
            unused.push((arg.clone(), !star_kwargs));
        }
    }
    let missing = if star_args || star_kwargs {
        Side::unknown()
    } else {
        let passed: BTreeSet<&str> = keywords.iter().map(|(_, name)| name.as_str()).collect();
        Side::known(
            fields
                .fields
                .iter()
                .filter(|(_, key)| match key {
                    Key::Index(index) => *index >= positional.len(),
                    Key::Name(name) => !passed.contains(name.as_str()),
                })
                .map(|(id, _)| (*id, true))
                .collect(),
        )
    };
    (Side::known(unused), missing)
}

/// The positional fields (placeholders and stars) whose index is at or past `count`.
fn positional_past(fields: &Fields, count: usize) -> Side<usize> {
    Side::known(
        fields
            .fields
            .iter()
            .filter(|(_, key)| matches!(key, Key::Index(index) if *index >= count))
            .map(|(id, _)| (*id, true))
            .collect(),
    )
}

/// `%` with its right operand. With no positional value, a dict display is read against the
/// mapping keys (F504, F505); with no mapping key, the operand is the values: a tuple display's
/// elements (F507), another operand of a known type but tuple one value, one Ruff types as a tuple
/// (`(1,) + (2,)`) values it cannot count, and with no placeholder any other operand is unused
/// (Ruff's F507 reads one so). Both readings may apply; a format that mixes keys and positions has
/// neither (F506), so everything is unknown.
fn percent_operand<'t, D: Doc>(
    operand: &Node<'t, D>,
    fields: &Fields,
) -> (Side<Node<'t, D>>, Side<usize>) {
    let operand = unparenthesized(operand.clone());
    let keys: Vec<&str> = fields
        .fields
        .iter()
        .filter_map(|(_, key)| match key {
            Key::Name(name) => Some(name.as_str()),
            Key::Index(_) => None,
        })
        .collect();
    let positional = fields.fields.len() - keys.len();
    let mapping = positional == 0 && operand.kind() == "dictionary";
    let sequence = keys.is_empty();
    if !mapping && !sequence {
        return (Side::unknown(), Side::unknown());
    }
    let mut unused = Side::known(Vec::new());
    let mut missing = Side::known(Vec::new());
    if mapping {
        let (mapped_unused, mapped_missing) = mapping_values(&operand, fields, &keys);
        unused = unused.merge(mapped_unused);
        missing = missing.merge(mapped_missing);
    }
    if sequence {
        let (values_unused, values_missing) = sequence_values(&operand, fields, positional);
        unused = unused.merge(values_unused);
        missing = missing.merge(values_missing);
    }
    (unused, missing)
}

/// A dict display against a format's mapping keys: pairs with a string key the format does not
/// name are unused (unknown past a `**` splat); keys the dict does not pass are missing (unknown
/// past a splat or a key that is no string literal).
fn mapping_values<'t, D: Doc>(
    dict: &Node<'t, D>,
    fields: &Fields,
    keys: &[&str],
) -> (Side<Node<'t, D>>, Side<usize>) {
    let entries = named(dict);
    if entries.iter().any(|entry| entry.kind() != "pair") {
        return (Side::unknown(), Side::unknown());
    }
    let mut unused = Vec::new();
    let mut passed = Vec::new();
    let mut dynamic = false;
    for pair in &entries {
        match pair.field("key").and_then(|key| string_key(&key)) {
            Some(key) => {
                if !keys.contains(&key.as_str()) {
                    unused.push((pair.clone(), true));
                }
                passed.push(key);
            }
            None => dynamic = true,
        }
    }
    let missing = if dynamic {
        Side::unknown()
    } else {
        Side::known(
            fields
                .fields
                .iter()
                .filter(|(_, key)| matches!(key, Key::Name(name) if !passed.contains(name)))
                .map(|(id, _)| (*id, true))
                .collect(),
        )
    };
    (Side::known(unused), missing)
}

/// The operand as a format's positional values.
fn sequence_values<'t, D: Doc>(
    operand: &Node<'t, D>,
    fields: &Fields,
    positional: usize,
) -> (Side<Node<'t, D>>, Side<usize>) {
    if operand.kind() == "tuple" {
        let elements = named(operand);
        if elements
            .iter()
            .any(|element| element.kind() == "list_splat")
        {
            return (Side::unknown(), Side::unknown());
        }
        let unused = elements
            .iter()
            .skip(positional)
            .map(|element| (element.clone(), true))
            .collect();
        return (Side::known(unused), positional_past(fields, elements.len()));
    }
    match resolve(operand) {
        // Ruff's F507 stops at a tuple it cannot count, placeholders or not.
        Type::Atom(Atom::Tuple) => (Side::unknown(), Side::unknown()),
        Type::Atom(_) => {
            let unused = if positional == 0 {
                vec![(operand.clone(), true)]
            } else {
                Vec::new()
            };
            (Side::known(unused), positional_past(fields, 1))
        }
        _ if positional == 0 => (
            Side::known(vec![(operand.clone(), true)]),
            Side::known(Vec::new()),
        ),
        _ => (Side::unknown(), Side::unknown()),
    }
}

/// A printf-style format with its values as an argument list (a logging call's): positional
/// arguments by position; unknown past `*args` or for a format with mapping keys.
fn percent_list<'t, D: Doc>(
    args: &[Node<'t, D>],
    fields: &Fields,
) -> (Side<Node<'t, D>>, Side<usize>) {
    if fields
        .fields
        .iter()
        .any(|(_, key)| matches!(key, Key::Name(_)))
    {
        return (Side::unknown(), Side::unknown());
    }
    let positional: Vec<&Node<'t, D>> = args
        .iter()
        .filter(|arg| !matches!(arg.kind().as_ref(), "keyword_argument" | "dictionary_splat"))
        .collect();
    if positional.iter().any(|arg| arg.kind() == "list_splat") {
        return (Side::unknown(), Side::unknown());
    }
    let unused = positional
        .iter()
        .skip(fields.fields.len())
        .map(|arg| ((*arg).clone(), true))
        .collect();
    (
        Side::known(unused),
        positional_past(fields, positional.len()),
    )
}

/// Whether a node is Ruff's `Expr::StringLiteral`: a string or implicit concatenation with no `f`,
/// `b` or `t` prefix in any part.
fn plain_literal<D: Doc>(node: &Node<'_, D>) -> bool {
    let parts = match node.kind().as_ref() {
        "string" => vec![node.clone()],
        "concatenated_string" => named(node),
        _ => return false,
    };
    parts.iter().all(|part| {
        let text = part.text();
        part.kind() == "string"
            && text.find(['\'', '"']).is_some_and(|quote| {
                !text[..quote]
                    .chars()
                    .any(|c| matches!(c, 'f' | 'F' | 'b' | 'B' | 't' | 'T'))
            })
    })
}

/// A dict key's value when it is a string literal (Ruff's `Expr::StringLiteral`).
fn string_key<D: Doc>(key: &Node<'_, D>) -> Option<String> {
    let key = unparenthesized(key.clone());
    if !plain_literal(&key) {
        return None;
    }
    match offsets::decode_python(&key) {
        offsets::Decode::Known(decoded) => Some(decoded.text),
        _ => None,
    }
}

/// Ruff's `PythonType` (`ruff_python_semantic::analyze::type_inference`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Atom {
    String,
    Bytes,
    Bool,
    Integer,
    Float,
    Complex,
    None,
    Ellipsis,
    Dict,
    List,
    Set,
    Tuple,
    Generator,
}

impl Atom {
    fn number(self) -> Option<u8> {
        match self {
            Self::Bool => Some(0),
            Self::Integer => Some(1),
            Self::Float => Some(2),
            Self::Complex => Some(3),
            _ => None,
        }
    }

    fn from_number(rank: u8) -> Self {
        match rank {
            0 => Self::Bool,
            1 => Self::Integer,
            2 => Self::Float,
            _ => Self::Complex,
        }
    }

    /// Ruff's `is_subtype_of`: the numeric tower, and each type of itself.
    fn subtype_of(self, other: Self) -> bool {
        match (self.number(), other.number()) {
            (Some(left), Some(right)) => left <= right,
            _ => self == other,
        }
    }

    /// Ruff's `NumberLike::coerce`: the higher of two numbers, `bool` read as `int`.
    fn coerce(self, other: Self) -> Self {
        let rank = self
            .number()
            .unwrap_or(1)
            .max(other.number().unwrap_or(1))
            .max(1);
        Self::from_number(rank)
    }
}

/// Ruff's `ResolvedPythonType`.
#[derive(Clone, Debug, PartialEq)]
enum Type {
    Atom(Atom),
    Union(BTreeSet<Atom>),
    Unknown,
    Error,
}

impl Type {
    fn union(self, other: Self) -> Self {
        match (self, other) {
            (Self::Error, _) | (_, Self::Error) => Self::Error,
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            (Self::Atom(a), Self::Atom(b)) => {
                if a.subtype_of(b) {
                    Self::Atom(b)
                } else if b.subtype_of(a) {
                    Self::Atom(a)
                } else {
                    Self::Union([a, b].into_iter().collect())
                }
            }
            (Self::Atom(a), Self::Union(mut set)) | (Self::Union(mut set), Self::Atom(a)) => {
                if !set.iter().any(|b| a.subtype_of(*b)) {
                    set.insert(a);
                }
                Self::Union(set)
            }
            (Self::Union(mut a), Self::Union(b)) => {
                for element in b {
                    if !a.iter().any(|known| element.subtype_of(*known)) {
                        a.insert(element);
                    }
                }
                Self::Union(a)
            }
        }
    }
}

/// Ruff's `ResolvedPythonType::from` over a tree-sitter expression.
fn resolve<D: Doc>(node: &Node<'_, D>) -> Type {
    let node = unparenthesized(node.clone());
    let atom = |atom| Type::Atom(atom);
    match node.kind().as_ref() {
        "dictionary" | "dictionary_comprehension" => atom(Atom::Dict),
        "set" | "set_comprehension" => atom(Atom::Set),
        "list" | "list_comprehension" => atom(Atom::List),
        "tuple" => atom(Atom::Tuple),
        "generator_expression" => atom(Atom::Generator),
        "string" | "concatenated_string" => {
            let parts = if node.kind() == "string" {
                vec![node.clone()]
            } else {
                named(&node)
            };
            let prefixes: String = parts
                .iter()
                .map(|part| {
                    let text = part.text();
                    let end = text.find(['\'', '"']).unwrap_or(0);
                    text[..end].to_ascii_lowercase()
                })
                .collect();
            if prefixes.contains('t') {
                Type::Unknown
            } else if prefixes.contains('f') {
                atom(Atom::String)
            } else if prefixes.contains('b') {
                atom(Atom::Bytes)
            } else {
                atom(Atom::String)
            }
        }
        "integer" => {
            if node.text().ends_with(['j', 'J']) {
                atom(Atom::Complex)
            } else {
                atom(Atom::Integer)
            }
        }
        "float" => {
            if node.text().ends_with(['j', 'J']) {
                atom(Atom::Complex)
            } else {
                atom(Atom::Float)
            }
        }
        "true" | "false" => atom(Atom::Bool),
        "none" => atom(Atom::None),
        "ellipsis" => atom(Atom::Ellipsis),
        "named_expression" => node
            .field("value")
            .map_or(Type::Unknown, |value| resolve(&value)),
        "conditional_expression" => {
            let parts = named(&node);
            match parts.as_slice() {
                [body, _, orelse] => resolve(body).union(resolve(orelse)),
                _ => Type::Unknown,
            }
        }
        "boolean_operator" => match (node.field("left"), node.field("right")) {
            (Some(left), Some(right)) => resolve(&left).union(resolve(&right)),
            _ => Type::Unknown,
        },
        "not_operator" => atom(Atom::Bool),
        "unary_operator" => {
            let (Some(operator), Some(operand)) = (node.field("operator"), node.field("argument"))
            else {
                return Type::Unknown;
            };
            match (operator.text().as_ref(), resolve(&operand)) {
                ("~", Type::Atom(Atom::Bool | Atom::Integer)) => atom(Atom::Integer),
                ("~", Type::Atom(_)) => Type::Error,
                ("+" | "-", Type::Atom(number)) if number.number().is_some() => {
                    atom(if number == Atom::Bool {
                        Atom::Integer
                    } else {
                        number
                    })
                }
                ("+" | "-", Type::Atom(_)) => Type::Error,
                _ => Type::Unknown,
            }
        }
        "binary_operator" => {
            let (Some(left), Some(operator), Some(right)) = (
                node.field("left"),
                node.field("operator"),
                node.field("right"),
            ) else {
                return Type::Unknown;
            };
            binary(operator.text().as_ref(), resolve(&left), resolve(&right))
        }
        _ => Type::Unknown,
    }
}

/// Ruff's type of a binary operation over two operand types.
fn binary(operator: &str, left: Type, right: Type) -> Type {
    // `%` with a string or bytes left side is a string whatever the right side is.
    match (operator, &left) {
        ("%", Type::Atom(Atom::String)) => return Type::Atom(Atom::String),
        ("%", Type::Atom(Atom::Bytes)) => return Type::Atom(Atom::Bytes),
        _ => {}
    }
    let (Type::Atom(left), Type::Atom(right)) = (left, right) else {
        return Type::Unknown;
    };
    let numbers = left.number().is_some() && right.number().is_some();
    let number = || Type::Atom(left.coerce(right));
    match operator {
        "+" => match (left, right) {
            (Atom::String, Atom::String) => Type::Atom(Atom::String),
            (Atom::Bytes, Atom::Bytes) => Type::Atom(Atom::Bytes),
            (Atom::List, Atom::List) => Type::Atom(Atom::List),
            (Atom::Tuple, Atom::Tuple) => Type::Atom(Atom::Tuple),
            _ if numbers => number(),
            _ => Type::Error,
        },
        "-" => match (left, right) {
            _ if numbers => number(),
            (Atom::Set, Atom::Set) => Type::Atom(Atom::Set),
            _ => Type::Error,
        },
        "%" if numbers => number(),
        "*" => match (left, right) {
            _ if numbers => number(),
            (Atom::String, Atom::Integer) | (Atom::Integer, Atom::String) => {
                Type::Atom(Atom::String)
            }
            _ => Type::Error,
        },
        "//" | "**" => {
            if numbers {
                number()
            } else {
                Type::Error
            }
        }
        "/" => {
            if numbers {
                let coerced = left.coerce(right);
                Type::Atom(if coerced == Atom::Integer {
                    Atom::Float
                } else {
                    coerced
                })
            } else {
                Type::Error
            }
        }
        "&" | "|" | "^" | "<<" | ">>" => {
            if numbers {
                if left.coerce(right) == Atom::Integer {
                    Type::Atom(Atom::Integer)
                } else {
                    Type::Error
                }
            } else {
                Type::Error
            }
        }
        _ => Type::Unknown,
    }
}

mod printf;
mod rust;
mod scanf;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_core::tree_sitter::LanguageExt;

    /// The type Ruff's `ResolvedPythonType` gives an expression statement's expression.
    fn type_of(source: &str) -> Type {
        let root = GrammarDialect::Python.ast_grep(source);
        let statement = root.root().child(0).expect("statement");
        let expression = statement.child(0).expect("expression");
        resolve(&expression)
    }

    #[test]
    fn types_are_ruffs() {
        for (source, expected) in [
            ("42", Type::Atom(Atom::Integer)),
            ("3.14", Type::Atom(Atom::Float)),
            ("1j", Type::Atom(Atom::Complex)),
            ("'a' 'b'", Type::Atom(Atom::String)),
            ("b'a'", Type::Atom(Atom::Bytes)),
            ("f'{x}'", Type::Atom(Atom::String)),
            ("t'{x}'", Type::Unknown),
            ("-True", Type::Atom(Atom::Integer)),
            ("~1.0", Type::Error),
            ("not x", Type::Atom(Atom::Bool)),
            ("(1 + 2)", Type::Atom(Atom::Integer)),
            ("1 / 2", Type::Atom(Atom::Float)),
            ("'a' + 1", Type::Error),
            ("'%s' % x", Type::Atom(Atom::String)),
            ("'a' * 2", Type::Atom(Atom::String)),
            ("1 if c else 2", Type::Atom(Atom::Integer)),
            ("1 if c else 2.0", Type::Atom(Atom::Float)),
            ("True or 1", Type::Atom(Atom::Integer)),
            ("(x := 1)", Type::Atom(Atom::Integer)),
            ("[x for x in y]", Type::Atom(Atom::List)),
            ("(1,) + (2,)", Type::Atom(Atom::Tuple)),
            ("(*a,)", Type::Atom(Atom::Tuple)),
            ("[*a] + [1]", Type::Atom(Atom::List)),
            ("(1,) * 2", Type::Error),
            ("(x for x in y)", Type::Atom(Atom::Generator)),
            ("x", Type::Unknown),
            ("f(1)", Type::Unknown),
            ("1 @ 2", Type::Unknown),
        ] {
            assert_eq!(type_of(source), expected, "{source}");
        }
        assert_eq!(
            type_of("1 if c else 'a'"),
            Type::Union([Atom::String, Atom::Integer].into_iter().collect())
        );
    }
}
