use super::*;
use crate::code_rule_dsl::oracle;
use num_traits::ToPrimitive;
use std::collections::BTreeSet;

fn kind(node: &PNode) -> &'static str {
    KINDS[usize::from(node.kind) - 1]
}

/// The tree as `kind(value)[children]`, one line per part.
fn shape(text: &str) -> Vec<String> {
    fn node(text: &str, node: &PNode) -> String {
        let value = match &node.value {
            Some(Value::PhpStr(value)) => format!("={:?}", String::from_utf8_lossy(value)),
            Some(Value::Int(value)) => format!("={value}"),
            _ => String::new(),
        };
        let children = node
            .children
            .iter()
            .map(|child| node_fn(text, child))
            .collect::<Vec<_>>();
        let inner = if children.is_empty() {
            String::new()
        } else {
            format!("[{}]", children.join(" "))
        };
        format!(
            "{}{value}:{:?}{inner}",
            kind(node),
            &text[node.start..node.end]
        )
    }
    fn node_fn(text: &str, child: &PNode) -> String {
        node(text, child)
    }
    parse(text)
        .children
        .iter()
        .map(|part| node(text, part))
        .collect()
}

#[test]
fn placeholders_number_the_variables_they_assign() {
    assert_eq!(
        shape("%d,%*s %5[^]a]%ld"),
        [
            "placeholder=0:\"%d\"[conversion=\"d\":\"d\"]",
            "literal=\",\":\",\"",
            "placeholder:\"%*s\"[suppress:\"*\" conversion=\"s\":\"s\"]",
            "literal=\" \":\" \"",
            "placeholder=1:\"%5[^]a]\"[width=5:\"5\" conversion=\"[\":\"[^]a]\"[set=\"^]a\":\"^]a\"]]",
            "placeholder=2:\"%ld\"[length:\"l\" conversion=\"d\":\"d\"]",
        ]
    );
    assert_eq!(
        shape("%2$s %1$d"),
        [
            "placeholder=1:\"%2$s\"[argnum=1:\"2$\" conversion=\"s\":\"s\"]",
            "literal=\" \":\" \"",
            "placeholder=0:\"%1$d\"[argnum=0:\"1$\" conversion=\"d\":\"d\"]",
        ]
    );
    // PHP reads no further than a NUL.
    assert_eq!(
        shape("%d\0%q"),
        [
            "placeholder=0:\"%d\"[conversion=\"d\":\"d\"]",
            "literal=\"\\0%q\":\"\\0%q\"",
        ]
    );
}

#[test]
fn formats_php_refuses_are_one_error() {
    for (text, message) in [
        (
            "%d %1$s",
            "cannot mix \"%\" and \"%n$\" conversion specifiers",
        ),
        (
            "%1$s %1$d",
            "Variable is assigned by multiple \"%n$\" conversion specifiers",
        ),
        ("%256$s", "\"%n$\" argument index out of range"),
        ("%[abc", "Unmatched [ in format string"),
        ("%q", "Bad scan conversion character \"q\""),
        ("abc %", "Bad scan conversion character \""),
    ] {
        let root = parse(text);
        let [error] = root.children.as_slice() else {
            panic!("{text:?}: one error");
        };
        assert_eq!(
            error.value,
            Some(Value::PhpStr(message.as_bytes().to_vec())),
            "{text:?}"
        );
    }
}

/// Every case of `fixtures/php/literal/scanf.expected.json`. A format PHP 8.5.11 refuses with no
/// variable passed is one `error` with PHP's message; any other assigns the variables its
/// placeholders number, and a call with variables throws exactly where one of them is passed no
/// placeholder assigns ("not assigned") or a placeholder numbers one past the last passed (the
/// index error, which PHP raises first).
#[test]
fn scanf_formats_match_php() {
    let oracle: serde_json::Value =
        serde_json::from_str(oracle!("php/literal/scanf.expected.json")).expect("oracle");
    let counts: Vec<i64> = oracle["runs"]
        .as_array()
        .expect("runs")
        .iter()
        .map(|count| count.as_i64().expect("count"))
        .collect();
    let cases = oracle["cases"].as_array().expect("cases");
    assert!(cases.len() >= 5_000);
    let mut failures = Vec::new();
    for case in cases {
        let text = case[0].as_str().expect("format");
        let runs = case[1].as_array().expect("runs");
        let root = parse(text);
        if let Some(error) = root.children.iter().find(|part| part.error) {
            let Some(Value::PhpStr(message)) = &error.value else {
                unreachable!()
            };
            let message = String::from_utf8_lossy(message);
            if runs[0] != message.as_ref() {
                failures.push(format!("{text:?}: PHP {}, Rerail {message}", runs[0]));
            }
            continue;
        }
        let fields: BTreeSet<i64> = root
            .children
            .iter()
            .filter_map(|part| match &part.value {
                Some(Value::Int(index)) if kind(part) == "placeholder" => index.to_i64(),
                _ => None,
            })
            .collect();
        let numbered = root
            .children
            .iter()
            .any(|part| part.children.iter().any(|child| kind(child) == "argnum"));
        let assigns = fields.last().map_or(0, |last| last + 1);
        if runs[0] != serde_json::json!(["ok", assigns]) {
            failures.push(format!(
                "{text:?}: PHP {}, Rerail assigns {assigns}",
                runs[0]
            ));
        }
        for (count, run) in counts.iter().zip(runs).skip(1) {
            let missing = fields.iter().any(|index| index >= count);
            let unused = (0..*count).any(|index| !fields.contains(&index));
            let expected = if missing && numbered {
                "\"%n$\" argument index out of range"
            } else if missing {
                "Different numbers of variable names and field specifiers"
            } else if unused {
                "Variable is not assigned by any conversion specifiers"
            } else {
                "ok"
            };
            if run != expected {
                failures.push(format!(
                    "{text:?} with {count}: PHP {run}, Rerail {expected}"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
