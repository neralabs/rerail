use super::*;
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::oracle;
use crate::policy_dsl::value::tests::agrees;
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::Value as Json;

const ORACLE: &str = oracle!("ecma/value/typescript.expected.json");

/// Whether `source` has no syntax errors under `language`.
fn parses(language: GrammarDialect, source: &str) -> bool {
    !language
        .ast_grep(source)
        .root()
        .get_doc()
        .tree
        .root_node()
        .has_error()
}

/// The value of `(<source>);` and the root kind of the expression inside the parentheses.
fn decode_kind(language: GrammarDialect, source: &str) -> (Value, String) {
    let text = format!("({source});\n");
    let root = language.ast_grep(&text);
    let kind = root
        .root()
        .children()
        .find(|child| child.kind() == "expression_statement")
        .and_then(|statement| {
            statement
                .children()
                .find(|child| child.kind() == "parenthesized_expression")
        })
        .and_then(|parenthesized| operand(&parenthesized))
        .map(|expression| expression.kind().to_string())
        .unwrap_or_default();
    (decode_source(language, source), kind)
}

/// The value of `(<source>);` as an ECMAScript statement: the expression inside the parentheses.
pub(crate) fn decode_source(language: GrammarDialect, source: &str) -> Value {
    let text = format!("({source});\n");
    let root = language.ast_grep(&text);
    let statement = root
        .root()
        .children()
        .find(|child| child.kind() == "expression_statement")
        .unwrap_or_else(|| panic!("{source:?} is an expression statement"));
    let parenthesized = statement
        .children()
        .find(|child| child.kind() == "parenthesized_expression")
        .unwrap_or_else(|| panic!("{source:?} is parenthesized"));
    let Some(expression) = operand(&parenthesized) else {
        return Value::Unknown;
    };
    let mut spend = || true;
    // The cases bind no global name, so every builtin is unshadowed.
    let mut global = |_: std::ops::Range<usize>, _: &str| Some(true);
    decode(
        &expression,
        &mut Context::with_globals(&mut spend, &mut global),
    )
}

/// Every case in the getStaticValue oracle (`rerail-fixtures/ecma/value/generate_typescript.mjs`):
/// - where a parser refuses the source, Rerail knows nothing;
/// - where getStaticValue knows nothing, Rerail knows at most a container's shape;
/// - where getStaticValue knows the value, Rerail knows the same, unless the case says why not.
#[test]
fn typescript_literals_decode_as_get_static_value_reads_them() {
    let oracle: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
    let mut known = 0;
    let mut wrong = Vec::new();
    for case in oracle["cases"].as_array().expect("cases") {
        if case.get("jsx").is_some() {
            continue;
        }
        let source = case["src"].as_str().expect("src");
        for language in [GrammarDialect::TypeScript, GrammarDialect::JavaScript] {
            if language == GrammarDialect::JavaScript
                && !parses(language, &format!("({source});\n"))
            {
                continue;
            }
            let value = decode_source(language, source);
            let expected = &case["value"];
            if expected.get("error").is_some() {
                if value != Value::Unknown {
                    wrong.push(format!(
                        "{language:?} {source:?} does not parse: {expected}"
                    ));
                }
            } else if expected.get("unknown").is_some() {
                if value.is_constant() {
                    wrong.push(format!(
                        "{language:?} {source:?}: getStaticValue knows nothing, Rerail reads {value:?}"
                    ));
                }
            } else if let Some(reason) = case.get("unknown") {
                if value != Value::Unknown {
                    wrong.push(format!(
                        "{language:?} {source:?} is unknown by design ({reason}), Rerail reads {value:?}"
                    ));
                }
            } else {
                if language == GrammarDialect::TypeScript {
                    known += 1;
                }
                if !agrees(&value, expected) {
                    wrong.push(format!(
                        "{language:?} {source:?}: Rerail reads {value:?}, getStaticValue {expected}"
                    ));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(known > 180, "{known} cases known");
}

/// Every kind `decode` knows a value for is listed for the anchor plan: a known value's root kind
/// is in `VALUE_KINDS`.
#[test]
fn value_kinds_cover_every_decoded_root() {
    let oracle: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
    for case in oracle["cases"].as_array().expect("cases") {
        if case.get("jsx").is_some() {
            continue;
        }
        let source = case["src"].as_str().expect("src");
        for language in [GrammarDialect::TypeScript, GrammarDialect::JavaScript] {
            if language == GrammarDialect::JavaScript
                && !parses(language, &format!("({source});\n"))
            {
                continue;
            }
            let (value, kind) = decode_kind(language, source);
            if value.is_known() {
                assert!(
                    VALUE_KINDS.contains(&kind.as_str()),
                    "{language:?} {source:?} has kind {kind}"
                );
            }
        }
    }
}

const DESTRUCTURE_ORACLE: &str = oracle!("ecma/value/destructure.expected.json");

/// Every name a declaration in the destructuring oracle (`rerail-fixtures/ecma/value/generate_destructure.mjs`,
/// TypeScript and Node.js) binds: where the table pairs it with a literal element, that element's
/// value is the name's.
#[test]
fn destructured_names_take_the_elements_javascript_gives_them() {
    use crate::policy_dsl::scope::ScopeTable;
    let oracle: Json = serde_json::from_str(DESTRUCTURE_ORACLE).expect("oracle JSON");
    let (mut paired, mut unpaired) = (0, 0);
    for case in oracle["cases"].as_array().expect("cases") {
        let declaration = case["src"].as_str().expect("src");
        let names = case["names"].as_array().expect("names");
        let uses = names
            .iter()
            .map(|entry| format!("use({});", entry["name"].as_str().expect("name")))
            .collect::<String>();
        let source = format!("{declaration}\n{uses}\n");
        for language in [GrammarDialect::TypeScript, GrammarDialect::JavaScript] {
            if language == GrammarDialect::JavaScript && !parses(language, &source) {
                continue;
            }
            let root = language.ast_grep(&source);
            let table = ScopeTable::build(root.root(), language, usize::MAX, &|| false)
                .expect("table builds");
            let mut at = declaration.len() + 1;
            for entry in names {
                let name = entry["name"].as_str().expect("name");
                at += "use(".len();
                let reference = table.reference(&(at..at + name.len())).unwrap_or_else(|| {
                    panic!("{language:?} {declaration:?}: `{name}` is a reference")
                });
                at += name.len() + ");".len();
                let (_, value) = table
                    .binding(reference, name)
                    .unwrap_or_else(|| panic!("{language:?} {declaration:?}: `{name}` is bound"));
                let Some(value) = value else {
                    unpaired += 1;
                    continue;
                };
                let node = value.find(root.root()).expect("the element is in the tree");
                let mut spend = || true;
                let decoded = decode(&node, &mut Context::new(&mut spend));
                assert!(
                    agrees(&decoded, &entry["value"]),
                    "{language:?} {declaration:?}: `{name}` takes {} ({decoded:?}), JavaScript gives {}",
                    node.text(),
                    entry["value"]
                );
                paired += 1;
            }
        }
    }
    assert!(paired > 15, "{paired} names paired, {unpaired} not");
}

const FOLLOW_ORACLE: &str = oracle!("ecma/value/follow.expected.json");

/// The value of the argument of the last `probe(...)` call in a TypeScript module, following names
/// through the scope table's bindings (`value.follow`).
fn decode_following(language: GrammarDialect, source: &str) -> Value {
    use crate::policy_dsl::scope::ScopeTable;
    let root = language.ast_grep(source);
    let table =
        ScopeTable::build(root.root(), language, usize::MAX, &|| false).expect("table builds");
    let argument = root
        .root()
        .dfs()
        .filter(|node| {
            node.kind() == "call_expression"
                && node
                    .field("function")
                    .is_some_and(|callee| callee.text() == "probe")
        })
        .last()
        .and_then(|call| call.field("arguments"))
        .and_then(|arguments| arguments.children().find(|child| child.is_named()))
        .unwrap_or_else(|| panic!("{source:?} calls probe"));
    let mut spend = || true;
    let mut global = |range: std::ops::Range<usize>, name: &str| {
        let reference = table.reference(&range)?;
        Some(table.resolve(reference, name).is_none())
    };
    let mut binding = |range: std::ops::Range<usize>, name: &str| {
        let reference = table.reference(&range)?;
        let (binding, value) = table.binding(reference, name)?;
        Some(binding.followed(value?))
    };
    let mut context = Context::with_globals(&mut spend, &mut global).following(&mut binding);
    decode(&argument, &mut context)
}

/// Every program in the `value.follow` oracle (`rerail-fixtures/ecma/value/generate_follow.mjs`): where getStaticValue
/// with the program's scope knows nothing, Rerail knows at most a container's shape; where it knows
/// the value, Rerail knows the same, unless the case says why not.
#[test]
fn followed_names_read_as_get_static_value_reads_them_with_a_scope() {
    let oracle: Json = serde_json::from_str(FOLLOW_ORACLE).expect("oracle JSON");
    let mut known = 0;
    for case in oracle["cases"].as_array().expect("cases") {
        let source = case["src"].as_str().expect("src");
        for language in [GrammarDialect::TypeScript, GrammarDialect::JavaScript] {
            if language == GrammarDialect::JavaScript && !parses(language, source) {
                continue;
            }
            let value = decode_following(language, source);
            let expected = &case["value"];
            if expected.get("unknown").is_some() {
                assert!(
                    !value.is_constant(),
                    "{language:?} {source:?}: getStaticValue knows nothing, Rerail reads {value:?}"
                );
            } else if let Some(reason) = case.get("unknown") {
                assert_eq!(
                    value,
                    Value::Unknown,
                    "{language:?} {source:?} is unknown by design: {reason}"
                );
            } else {
                if language == GrammarDialect::TypeScript {
                    known += 1;
                }
                assert!(
                    agrees(&value, expected),
                    "{language:?} {source:?}: Rerail reads {value:?}, getStaticValue {expected}"
                );
            }
        }
    }
    assert!(known > 20, "{known} cases known");
}

/// `compare`'s JavaScript orders (`text: "value"`, `lt`/`le`/`gt`/`ge`) over the two operands of
/// every relational case in the getStaticValue oracle give what getStaticValue folds the whole
/// expression to: IsLessThan with its conversions, and false where it is undefined.
#[test]
fn value_orders_are_get_static_values_relational_operators() {
    let oracle: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
    let mut compared = 0;
    for case in oracle["cases"].as_array().expect("cases") {
        let source = case["src"].as_str().expect("src");
        // Cases unknown by design (an object's conversion) stay unknown.
        let Some(expected) = case["value"]["bool"].as_bool() else {
            continue;
        };
        if case.get("unknown").is_some() {
            continue;
        }
        let text = format!("({source});\n");
        for language in [GrammarDialect::TypeScript, GrammarDialect::JavaScript] {
            if language == GrammarDialect::JavaScript && !parses(language, &text) {
                continue;
            }
            let root = language.ast_grep(&text);
            let Some(expression) = root
                .root()
                .dfs()
                .find(|node| node.kind() == "parenthesized_expression")
                .and_then(|parenthesized| operand(&parenthesized))
            else {
                continue;
            };
            let (Some(left), Some(operator), Some(right)) = (
                expression.field("left"),
                expression.field("operator"),
                expression.field("right"),
            ) else {
                continue;
            };
            let operator = operator.text();
            if expression.kind() != "binary_expression"
                || !matches!(operator.as_ref(), "<" | "<=" | ">" | ">=")
            {
                continue;
            }
            let (left, right) = (
                decode_source(language, &left.text()),
                decode_source(language, &right.text()),
            );
            assert_eq!(
                ops::relation(&operator, &left, &right),
                Some(expected),
                "{language:?} {source:?}"
            );
            if language == GrammarDialect::TypeScript {
                compared += 1;
            }
        }
    }
    assert!(compared > 35, "{compared} relational cases compared");
}

/// Every JSX case in the getStaticValue oracle: the first attribute string's value, as
/// typescript-estree gives it (`Literal.value`), is Rerail's, unless the case says why not. Where
/// tree-sitter-typescript's JSX string grammar refuses a reference it has no rule for (`&sup2;`,
/// an entity name with a digit), the file has a syntax error, and the string still decodes.
#[test]
fn jsx_attribute_strings_decode_as_typescript_estree_reads_them() {
    let oracle: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
    let mut compared = 0;
    for case in oracle["cases"].as_array().expect("cases") {
        if case.get("jsx").is_none() {
            continue;
        }
        let source = case["src"].as_str().expect("src");
        let text = format!("({source});\n");
        for language in [GrammarDialect::Tsx, GrammarDialect::JavaScript] {
            if language == GrammarDialect::JavaScript && !parses(language, &text) {
                continue;
            }
            let root = language.ast_grep(&text);
            let string = root
                .root()
                .dfs()
                .find(|node| {
                    node.kind() == "string"
                        && node
                            .parent()
                            .is_some_and(|parent| parent.kind() == "jsx_attribute")
                })
                .unwrap_or_else(|| panic!("{language:?} {source:?} has an attribute string"));
            let mut spend = || true;
            let value = decode(&string, &mut Context::new(&mut spend));
            if let Some(reason) = case.get("unknown") {
                assert_eq!(
                    value,
                    Value::Unknown,
                    "{language:?} {source:?} is unknown by design: {reason}"
                );
            } else {
                assert!(
                    agrees(&value, &case["value"]),
                    "{language:?} {source:?}: Rerail reads {value:?}, typescript-estree {}",
                    case["value"]
                );
                if language == GrammarDialect::Tsx {
                    compared += 1;
                }
            }
        }
    }
    assert!(compared > 15, "{compared} JSX cases compared");
}

/// Every function a call folds through is on eslint-utils' allowlists (`builtins.rs`, generated
/// from its source), and every getter and constant read is on its lists too.
#[test]
fn folded_builtins_are_on_eslint_utils_allowlists() {
    use super::builtins::{CALL_ALLOWED, CALL_PASS_THROUGH};
    for name in calls::IMPLEMENTED {
        assert!(
            CALL_ALLOWED.contains(name) || CALL_PASS_THROUGH.contains(name),
            "{name} is on no allowlist"
        );
    }
    // Sorted and without repeats, so a regenerated list diffs cleanly.
    assert!(calls::IMPLEMENTED.windows(2).all(|pair| pair[0] < pair[1]));
}
