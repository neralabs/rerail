use super::*;
use crate::policy_dsl::oracle;
use num_traits::ToPrimitive;

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
fn placeholders_number_the_values_they_consume() {
    assert_eq!(
        shape("Hello %s, 100%% %1$'*-10.3f"),
        [
            "literal=\"Hello \":\"Hello \"",
            "placeholder=0:\"%s\"[conversion=\"s\":\"s\"]",
            "literal=\", 100% \":\", 100%% \"",
            "placeholder=0:\"%1$'*-10.3f\"[argnum=0:\"1$\" flags=\"'*-\":\"'*-\" width=10:\"10\" precision=3:\".3\" conversion=\"f\":\"f\"]",
        ]
    );
    // Stars consume values before the placeholder's own; `*N$` names one.
    assert_eq!(
        shape("%*.*f %s %*3$ld"),
        [
            "placeholder=2:\"%*.*f\"[width:\"*\"[star=0:\"*\"] precision:\".*\"[star=1:\"*\"] conversion=\"f\":\"f\"]",
            "literal=\" \":\" \"",
            "placeholder=3:\"%s\"[conversion=\"s\":\"s\"]",
            "literal=\" \":\" \"",
            "placeholder=4:\"%*3$ld\"[width:\"*3$\"[star=2:\"*3$\"] length:\"l\" conversion=\"d\":\"d\"]",
        ]
    );
    // `%5%` consumes a value; a letter right after `%` takes no modifiers.
    assert_eq!(
        shape("%5%%ls%.e"),
        [
            "placeholder=0:\"%5%\"[width=5:\"5\" conversion=\"%\":\"%\"]",
            "placeholder=1:\"%ls\"[length:\"l\" conversion=\"s\":\"s\"]",
            "placeholder=2:\"%.e\"[precision:\".\" conversion=\"e\":\"e\"]",
        ]
    );
}

#[test]
fn formats_php_refuses_are_one_error() {
    assert_eq!(
        shape("a %"),
        ["error=\"Missing format specifier at end of string\":\"\""]
    );
    assert_eq!(
        shape("%0$s"),
        [
            "error=\"Argument number specifier must be greater than zero and less than 2147483647\":\"0$\""
        ]
    );
    assert_eq!(shape("%'"), ["error=\"Missing padding character\":\"'\""]);
    assert_eq!(
        shape("%2147483647d"),
        ["error=\"Width must be between 0 and 2147483647\":\"2147483647\""]
    );
    assert_eq!(
        shape("%.99999999999f"),
        ["error=\"Precision must be between 0 and 2147483647\":\"99999999999\""]
    );
    assert_eq!(
        shape("%s %y"),
        ["error=\"Unknown format specifier \\\"y\\\"\":\"y\""]
    );
    // PHP names the specifier's first byte; the node covers its character.
    assert_eq!(
        shape("%é"),
        ["error=\"Unknown format specifier \\\"\u{fffd}\\\"\":\"é\""]
    );
}

/// The argument uses of a tree in the order PHP consumes them: each placeholder's width star,
/// precision star, then its value.
fn uses(root: &PNode) -> Vec<(i64, &'static str)> {
    let mut uses = Vec::new();
    for part in root
        .children
        .iter()
        .filter(|part| kind(part) == "placeholder")
    {
        for child in &part.children {
            if matches!(kind(child), "width" | "precision")
                && let Some(Value::Int(index)) =
                    child.children.first().and_then(|s| s.value.as_ref())
            {
                uses.push((index.to_i64().expect("index"), kind(child)));
            }
        }
        if let Some(Value::Int(index)) = &part.value {
            uses.push((index.to_i64().expect("index"), "value"));
        }
    }
    uses
}

/// Every case of `fixtures/php/literal/printf.expected.json`. A format PHP 8.5.11 refuses (a
/// `ValueError` with every value passed) is one `error` with PHP's message, and one PHPStan 2.2.16
/// reads as invalid; any other has PHPStan's argument uses, and a call passes PHP exactly when it
/// passes every value a use asks for. Where no missing value can shift PHP's scan (no `*` and no
/// `%` conversion), its `ArgumentCountError` names the count the uses ask for.
#[test]
fn printf_formats_match_php_and_phpstan() {
    let oracle: serde_json::Value =
        serde_json::from_str(oracle!("php/literal/printf.expected.json")).expect("oracle");
    let counts: Vec<u64> = oracle["runs"]
        .as_array()
        .expect("runs")
        .iter()
        .map(|count| count.as_u64().expect("count"))
        .collect();
    let cases = oracle["cases"].as_array().expect("cases");
    assert!(cases.len() >= 5_000);
    let mut failures = Vec::new();
    for case in cases {
        let text = case[0].as_str().expect("format");
        let runs = case[1].as_array().expect("runs");
        let root = parse(text);
        let error = root
            .children
            .iter()
            .find(|part| part.error)
            .map(|part| match &part.value {
                Some(Value::PhpStr(message)) => String::from_utf8_lossy(message).into_owned(),
                _ => String::new(),
            });
        if let Some(error) = error {
            if !case[2].is_null() {
                failures.push(format!(
                    "{text:?}: PHPStan reads it, Rerail refuses it: {error}"
                ));
            }
            let last = &runs[runs.len() - 1];
            if last[0] != "ValueError" || last[1] != error.as_str() {
                failures.push(format!("{text:?}: PHP {last}, Rerail {error}"));
            }
            continue;
        }
        let ours: Vec<(i64, &str)> = uses(&root);
        let phpstan: Option<Vec<(i64, &str)>> = case[2].as_array().map(|uses| {
            uses.iter()
                .map(|r#use| {
                    (
                        r#use[0].as_i64().expect("index"),
                        r#use[1].as_str().expect("kind"),
                    )
                })
                .collect()
        });
        if phpstan.as_ref() != Some(&ours) {
            failures.push(format!("{text:?}: PHPStan {phpstan:?}, Rerail {ours:?}"));
            continue;
        }
        let required = ours.iter().map(|(index, _)| index + 1).max().unwrap_or(0) as u64;
        let shifts = text.contains('*')
            || root.children.iter().any(|part| {
                part.children.iter().any(|child| {
                    kind(child) == "conversion" && child.value == Some(Value::PhpStr(b"%".to_vec()))
                })
            });
        for (count, run) in counts.iter().zip(runs) {
            let passes = run == "ok";
            if passes != (*count >= required) {
                failures.push(format!(
                    "{text:?} with {count}: PHP {run}, Rerail needs {required}"
                ));
            } else if !passes && !shifts {
                let expected = format!(
                    "{} arguments are required, {} given",
                    required + 1,
                    count + 1
                );
                if run[0] != "ArgumentCountError" || run[1] != expected.as_str() {
                    failures.push(format!(
                        "{text:?} with {count}: PHP {run}, Rerail {expected}"
                    ));
                }
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
