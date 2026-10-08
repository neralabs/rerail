use super::parser::{self, Prop};
use super::*;
use crate::code_rule_dsl::oracle;
use serde_json::{Value as Json, json};

/// A parsed comment as `phpdoc_tree.php` records it.
fn record(text: &str) -> Json {
    let (parser, root) = parser::parse(text);
    if parser.exhausted() {
        return json!("exhausted");
    }
    match root {
        Ok(root) => render(&parser, root),
        Err(fatal) => {
            let class = match fatal {
                parser::Fatal::Parser(_) => "ParserException",
                parser::Fatal::Logic(_) => "LogicException",
            };
            json!([class, fatal.message()])
        }
    }
}

fn render(parser: &parser::Parser<'_>, id: parser::Id) -> Json {
    let node = &parser.nodes[id];
    let end = parser.tokens.last().map_or(0, |token| token.end);
    let byte = |index: usize| parser.tokens.get(index).map_or(end, |token| token.start);
    let props: serde_json::Map<String, Json> = node
        .props
        .iter()
        .map(|(name, prop)| {
            let value = match prop {
                Prop::Str(value) => json!(php_lossy(value)),
                Prop::Bool(value) => json!(value),
                Prop::Int(value) => json!(value),
                Prop::Strs(values) => json!(values),
            };
            ((*name).to_owned(), value)
        })
        .collect();
    let children: Vec<Json> = node
        .children
        .iter()
        .map(|(name, child)| json!([name, render(parser, *child)]))
        .collect();
    json!([
        node.kind,
        node.start.map(byte),
        node.end.map(|end| byte(end + 1)),
        props,
        children,
    ])
}

/// The text `json_encode` with `JSON_INVALID_UTF8_SUBSTITUTE` writes for bytes: one U+FFFD per
/// sequence `php_next_utf8_char` refuses, skipping as far as it does.
fn php_lossy(bytes: &[u8]) -> String {
    let lead = |b: u8| b < 0x80 || (0xc2..=0xf4).contains(&b);
    let trail = |b: Option<&u8>| b.is_some_and(|b| (0x80..=0xbf).contains(b));
    let mut out = String::new();
    let mut at = 0;
    while at < bytes.len() {
        let c = bytes[at];
        let width = match c {
            0..=0x7f => Ok(1),
            0xc2..=0xdf => {
                if trail(bytes.get(at + 1)) {
                    Ok(2)
                } else {
                    Err(if bytes.get(at + 1).is_none_or(|b| lead(*b)) {
                        1
                    } else {
                        2
                    })
                }
            }
            0xe0..=0xf4 => {
                let width = if c < 0xf0 { 3 } else { 4 };
                match (1..width).find(|offset| !trail(bytes.get(at + offset))) {
                    Some(offset) => Err((1..=offset)
                        .find(|i| bytes.get(at + i).is_none_or(|b| lead(*b)))
                        .unwrap_or(offset + 1)),
                    None => {
                        if std::str::from_utf8(&bytes[at..at + width]).is_ok() {
                            Ok(width)
                        } else {
                            Err(width)
                        }
                    }
                }
            }
            _ => Err(1),
        };
        match width {
            Ok(width) => {
                out.push_str(std::str::from_utf8(&bytes[at..at + width]).expect("UTF-8"));
                at += width;
            }
            Err(skip) => {
                out.push('\u{fffd}');
                at += skip;
            }
        }
    }
    out
}

/// Every case of an oracle file agrees with phpdoc-parser: kinds, spans, properties and children.
fn assert_oracle(file: &str, minimum: usize) {
    let oracle: Json = serde_json::from_str(file).expect("oracle");
    let cases = oracle["cases"].as_array().expect("cases");
    assert!(cases.len() >= minimum, "{} cases", cases.len());
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|case| {
            let text = case[0].as_str().expect("text");
            let ours = record(text);
            (ours != case[1])
                .then(|| format!("{text:?}\n  phpdoc-parser {}\n  Rerail {ours}", case[1]))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn doc_comments_match_phpdoc_parser() {
    assert_oracle(oracle!("php/literal/phpdoc.expected.json"), 5_000);
}

#[test]
fn laravel_doc_comments_match_phpdoc_parser() {
    assert_oracle(oracle!("php/literal/laravel/phpdoc.expected.json"), 1_500);
}

#[test]
fn symfony_doc_comments_match_phpdoc_parser() {
    assert_oracle(oracle!("php/literal/symfony/phpdoc.expected.json"), 1_500);
}

/// Every distinct doc comment of the PHP corpora (`RERAIL_PHPDOC_CORPUS`, written by
/// `rerail-fixtures/php/literal/generate_phpdoc.php --corpus`); the summary goes to
/// `RERAIL_PHPDOC_REPORT`.
#[test]
#[ignore = "phpdoc-parser over every doc comment of the PHP corpora"]
fn benchmark_phpdoc_corpus() {
    let input = std::env::var("RERAIL_PHPDOC_CORPUS").expect("RERAIL_PHPDOC_CORPUS");
    let output = std::env::var("RERAIL_PHPDOC_REPORT").expect("RERAIL_PHPDOC_REPORT");
    let corpus: Json =
        serde_json::from_str(&std::fs::read_to_string(input).expect("corpus")).expect("JSON");
    let cases = corpus["cases"].as_array().expect("cases");
    let started = std::time::Instant::now();
    let mut nodes = 0;
    let mut invalid = 0;
    let mut failures = Vec::new();
    for case in cases {
        let text = case[0].as_str().expect("text");
        let ours = record(text);
        if ours != case[1] {
            failures.push(text.to_owned());
            continue;
        }
        if let Some(root) = parse(text) {
            nodes += root.size();
            let mut stack = vec![&root];
            while let Some(node) = stack.pop() {
                invalid += usize::from(node.error);
                stack.extend(&node.children);
            }
        }
    }
    let report = json!({
        "benchmark": "phpDoc: Rerail's doc comment trees against phpstan/phpdoc-parser",
        "php": corpus["php"],
        "phpstan": corpus["phpstan"],
        "phpdocParser": corpus["phpdocParser"],
        "corpora": corpus["corpora"],
        "distinctDocComments": cases.len(),
        "treeNodes": nodes,
        "errorNodes": invalid,
        "parseMillis": started.elapsed().as_millis() as u64,
        "disagreements": failures,
    });
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).expect("write report");
    assert!(failures.is_empty(), "{} disagreements", failures.len());
}

/// The tree as `kind(value)@text[children]`, one line per node, indented by depth.
fn shape(text: &str) -> Vec<String> {
    let root = parse(text).expect("a tree");
    let mut lines = Vec::new();
    let mut stack = vec![(&root, 0)];
    while let Some((node, depth)) = stack.pop() {
        let value = match &node.value {
            Some(Value::PhpStr(value)) => format!("={:?}", String::from_utf8_lossy(value)),
            _ => String::new(),
        };
        let error = if node.error { "!" } else { "" };
        lines.push(format!(
            "{}{}{error}{value} {:?}",
            "  ".repeat(depth),
            KINDS[usize::from(node.kind) - 1],
            &text[node.start..node.end]
        ));
        stack.extend(node.children.iter().rev().map(|child| (child, depth + 1)));
    }
    lines
}

#[test]
fn tags_hold_types_variables_and_descriptions_in_text_order() {
    let text = "/**\n * Sum.\n *\n * @param array<int, string>|null $names The names\n *   to use.\n * @return mixed\n */";
    assert_eq!(
        shape(text),
        [
            "php_doc \"/**\\n * Sum.\\n *\\n * @param array<int, string>|null $names The names\\n *   to use.\\n * @return mixed\\n */\"",
            "  php_doc_text=\"Sum.\" \"Sum.\"",
            "  php_doc_text=\"\" \"\"",
            "  php_doc_tag=\"@param\" \"@param array<int, string>|null $names The names\\n *   to use.\"",
            "    name=\"@param\" \"@param\"",
            "    param_tag_value \"array<int, string>|null $names The names\\n *   to use.\"",
            "      union_type \"array<int, string>|null\"",
            "        generic_type \"array<int, string>\"",
            "          identifier_type=\"array\" \"array\"",
            "          identifier_type=\"int\" \"int\"",
            "          identifier_type=\"string\" \"string\"",
            "        identifier_type=\"null\" \"null\"",
            "      variable=\"$names\" \"$names\"",
            "      description=\"The names\\n  to use.\" \"The names\\n *   to use.\"",
            "  php_doc_tag=\"@return\" \"@return mixed\"",
            "    name=\"@return\" \"@return\"",
            "    return_tag_value \"mixed\"",
            "      identifier_type=\"mixed\" \"mixed\"",
        ]
    );
}

#[test]
fn a_value_the_parser_refuses_is_an_error_node_with_its_message() {
    assert_eq!(
        shape("/** @param array<int $a */"),
        [
            "php_doc \"/** @param array<int $a */\"",
            "  php_doc_tag=\"@param\" \"@param array<int $a\"",
            "    name=\"@param\" \"@param\"",
            "    invalid_tag_value!=\"Unexpected token \\\"$a\\\", expected '>' at offset 21 on line 1\" \"array<int $a\"",
            "      description=\"array<int $a\" \"array<int $a\"",
        ]
    );
    // An unclosed Doctrine annotation tag throws out of the parser: the whole comment is one error.
    assert_eq!(
        shape("/** @\\Foo( */"),
        [
            "php_doc!=\"Unexpected token \\\"*/\\\", expected type at offset 11 on line 1\" \"/** @\\\\Foo( */\""
        ]
    );
}

#[test]
fn constants_and_methods_carry_their_names() {
    assert_eq!(
        shape("/** @method static Foo::BAR|'x' make<T>(int $a = 1) */"),
        [
            "php_doc \"/** @method static Foo::BAR|'x' make<T>(int $a = 1) */\"",
            "  php_doc_tag=\"@method\" \"@method static Foo::BAR|'x' make<T>(int $a = 1)\"",
            "    name=\"@method\" \"@method\"",
            "    method_tag_value \"static Foo::BAR|'x' make<T>(int $a = 1)\"",
            "      union_type \"Foo::BAR|'x'\"",
            "        const_type \"Foo::BAR\"",
            "          const_fetch=\"Foo::BAR\" \"Foo::BAR\"",
            "        const_type \"'x'\"",
            "          const_expr_string=\"x\" \"'x'\"",
            "      name=\"make\" \"make\"",
            "      template_tag_value \"T\"",
            "        name=\"T\" \"T\"",
            "      method_tag_value_parameter \"int $a = 1\"",
            "        identifier_type=\"int\" \"int\"",
            "        variable=\"$a\" \"$a\"",
            "        const_expr_integer=\"1\" \"1\"",
        ]
    );
}

#[test]
fn only_php_doc_comments_are_parsed() {
    assert!(is_doc_comment("/** x */"));
    assert!(is_doc_comment("/**\n */"));
    assert!(is_doc_comment("/**\r\n*/"));
    // PHP lexes `/**` without white space after it, and `/**/`, as plain comments.
    assert!(!is_doc_comment("/**x */"));
    assert!(!is_doc_comment("/**/"));
    assert!(!is_doc_comment("/**\u{b}*/"));
    assert!(!is_doc_comment("/* x */"));
    assert!(!is_doc_comment("// x"));
    assert!(!is_doc_comment("# x"));
}

#[test]
fn nesting_past_the_bound_is_unknown() {
    let deep = format!("/** @var {}int{} */", "(".repeat(400), ")".repeat(400));
    assert!(parse(&deep).is_none());
    let shallow = format!("/** @var {}int{} */", "(".repeat(40), ")".repeat(40));
    assert!(parse(&shallow).is_some());
}

#[test]
fn a_tag_chain_looked_ahead_past_the_bound_is_unknown() {
    // Each generic tag's description parses the tag after it on its line to see where it ends.
    let long = format!("/** {} */", "@a ".repeat(2_000));
    assert!(parse(&long).is_none());
    let short = format!("/** {} */", "@a ".repeat(40));
    assert!(parse(&short).is_some_and(|root| root.children.len() == 40));
    // The tags a lookahead parsed are not kept: one tag and one value per tag, and the root.
    let (parser, root) = parser::parse(&short);
    assert!(root.is_ok() && !parser.exhausted());
    assert_eq!(parser.nodes.len(), 2 * 40 + 1);
}

#[test]
fn array_and_offset_suffixes_past_the_bound_are_unknown() {
    // Each suffix wraps the type before it in a loop: the tree nests, the parse does not.
    let arrays = format!("/** @var int{} */", "[]".repeat(20_000));
    assert!(parse(&arrays).is_none());
    let offsets = format!("/** @var T{} */", "['a']".repeat(20_000));
    assert!(parse(&offsets).is_none());
    let shallow = format!("/** @var int{} */", "[]".repeat(40));
    assert!(parse(&shallow).is_some());
}
