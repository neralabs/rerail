//! `formatArgs` for Python: `str.format` (`brace`) and `%` formats, as Ruff 0.16.9's Pyflakes rules read them
//! (`rules/pyflakes/rules/strings.rs`: F504, F505, F507, F522, F523, F524).

use super::*;
use crate::code_rule_dsl::identifier::python_identifier;

/// A Python `.format` call (`brace`) or `%` format: Ruff's AST has no parentheses, and its
/// F50x and F52x read a plain `str` literal only.
pub(super) fn prepare<'t, D: Doc>(this: &FormatArgs, env: &MetaVarEnv<'t, D>, brace: bool) -> Prepared<'t, D> {
    let Some(string) = env.get_match(&this.string) else {
        return Prepared::NoMatch;
    };
    let string = unparenthesized(string.clone());
    if !plain_literal(&string) {
        if matches!(string.kind().as_ref(), "string" | "concatenated_string") {
            return Prepared::Unknown;
        }
        return Prepared::NoMatch;
    }
    let tree = match parsed_tree(&string, this.style.grammar()) {
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
    let args: Vec<Node<'t, D>> = match env.get_match(&this.args) {
        Some(operand) => vec![operand.clone()],
        None => env
            .get_multiple_matches(&this.args)
            .into_iter()
            .filter(|arg| arg.is_named() && arg.kind() != "comment")
            .collect(),
    };
    let operand = env.get_match(&this.args).is_some();
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
    let entries = dict.named_code().collect::<Vec<_>>();
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
        let elements = operand.named_code().collect::<Vec<_>>();
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
        "concatenated_string" => node.named_code().collect::<Vec<_>>(),
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
    match crate::code_rule_dsl::literal::python_string::decode_python(&key) {
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
                node.named_code().collect::<Vec<_>>()
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
            let parts = node.named_code().collect::<Vec<_>>();
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
            let (Some(operator), Some(operand)) = (node.operator_token(), node.field("argument"))
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
                node.operator_token(),
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

#[cfg(test)]
pub mod tests {
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

/// Python's format styles (`FormatPreparer`): `brace` (`str.format`) or `%`.
pub struct Python {
    brace: bool,
}

/// `str.format`'s preparer.
pub const BRACE: Python = Python { brace: true };
/// `%`'s preparer.
pub const PERCENT: Python = Python { brace: false };

impl super::FormatPreparer for Python {
    fn prepare_tape<'t>(&self, format: &super::FormatArgs, env: &MetaVarEnv<'t, crate::code_rule_dsl::tape::TapeDoc>) -> Prepared<'t, crate::code_rule_dsl::tape::TapeDoc> {
        prepare(format, env, self.brace)
    }
    fn prepare_sub<'t>(&self, format: &super::FormatArgs, env: &MetaVarEnv<'t, crate::code_rule_dsl::literal::SubDoc>) -> Prepared<'t, crate::code_rule_dsl::literal::SubDoc> {
        prepare(format, env, self.brace)
    }
    fn prepare_str<'t>(&self, format: &super::FormatArgs, env: &MetaVarEnv<'t, ast_grep_core::tree_sitter::StrDoc<crate::code_rule_dsl::language::GrammarDialect>>) -> Prepared<'t, ast_grep_core::tree_sitter::StrDoc<crate::code_rule_dsl::language::GrammarDialect>> {
        prepare(format, env, self.brace)
    }
}
