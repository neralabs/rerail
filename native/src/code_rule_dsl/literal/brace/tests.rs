use super::*;
use crate::code_rule_dsl::oracle;

/// A node as `kind=value:"text"[children]`.
fn describe(text: &str, node: &PNode) -> String {
    let kind = KINDS[usize::from(node.kind) - 1];
    let value = match &node.value {
        Some(Value::Str(value)) => format!("={value:?}"),
        Some(Value::Int(value)) => format!("={value}"),
        _ => String::new(),
    };
    let children = node
        .children
        .iter()
        .map(|child| describe(text, child))
        .collect::<Vec<_>>();
    let inner = if children.is_empty() {
        String::new()
    } else {
        format!("[{}]", children.join(" "))
    };
    format!("{kind}{value}:{:?}{inner}", &text[node.start..node.end])
}

/// The tree's parts, described.
fn shape(text: &str) -> Vec<String> {
    parse(text)
        .children
        .iter()
        .map(|part| describe(text, part))
        .collect()
}

fn error(text: &str) -> Option<String> {
    parse(text)
        .children
        .iter()
        .find(|part| part.error)
        .and_then(|part| match &part.value {
            Some(Value::Str(message)) => Some(message.clone()),
            _ => None,
        })
}

#[test]
fn ruffs_format_string_examples() {
    assert_eq!(
        shape("abcd{1}:{key}"),
        [
            "literal=\"abcd\":\"abcd\"",
            "field=1:\"{1}\"[field_name=1:\"1\"[index=1:\"1\"]]",
            "literal=\":\":\":\"",
            "field=\"key\":\"{key}\"[field_name=\"key\":\"key\"[name=\"key\":\"key\"]]",
        ]
    );
    assert_eq!(
        shape("abcd{1:{a}}"),
        [
            "literal=\"abcd\":\"abcd\"",
            "field=1:\"{1:{a}}\"[field_name=1:\"1\"[index=1:\"1\"] format_spec=\"{a}\":\"{a}\"[field=\"a\":\"{a}\"[field_name=\"a\":\"a\"[name=\"a\":\"a\"]]]]",
        ]
    );
    assert_eq!(error("{a:%ЫйЯЧ}"), None);
    assert_eq!(
        error("{s"),
        Some("Single '{' encountered in format string".into())
    );
    assert_eq!(
        shape("{{{key}}}ddfe"),
        [
            "literal=\"{\":\"{{\"",
            "field=\"key\":\"{key}\"[field_name=\"key\":\"key\"[name=\"key\":\"key\"]]",
            "literal=\"}ddfe\":\"}}ddfe\"",
        ]
    );
    assert_eq!(
        shape("I am a \\N{snowman}"),
        ["literal=\"I am a \\\\N{snowman}\":\"I am a \\\\N{snowman}\""]
    );
    assert_eq!(
        shape("I am a \\N{snowman}{snowman}"),
        [
            "literal=\"I am a \\\\N{snowman}\":\"I am a \\\\N{snowman}\"",
            "field=\"snowman\":\"{snowman}\"[field_name=\"snowman\":\"snowman\"[name=\"snowman\":\"snowman\"]]",
        ]
    );
    assert_eq!(
        shape("I am a \\\\N{snowman}"),
        [
            "literal=\"I am a \\\\\\\\N\":\"I am a \\\\\\\\N\"",
            "field=\"snowman\":\"{snowman}\"[field_name=\"snowman\":\"snowman\"[name=\"snowman\":\"snowman\"]]",
        ]
    );
}

#[test]
fn field_names_and_automatic_numbering() {
    // Ruff's FormatSummary: autos count in order, a field before the fields in its spec.
    fn walk(node: &PNode, out: &mut Vec<String>) {
        if KINDS[usize::from(node.kind) - 1] == "field_name" {
            out.push(match &node.value {
                Some(Value::Int(value)) => value.to_string(),
                Some(Value::Str(value)) => value.clone(),
                _ => "?".into(),
            });
        }
        for child in &node.children {
            walk(child, out);
        }
    }
    let values = |text: &str| -> Vec<String> {
        let mut out = Vec::new();
        walk(&parse(text), &mut out);
        out
    };
    assert_eq!(
        values("foo{foo}a{}b{2}c{2}d{1}{}{}e{bar}{foo}f{spam}"),
        ["foo", "0", "2", "2", "1", "1", "2", "bar", "foo", "spam"]
    );
    assert_eq!(
        values("foo{foo}a{:{}{}}b{2:{3}{4}}c{2}d{1}{}e{bar:{spam}{eggs}}"),
        [
            "foo", "0", "1", "2", "2", "3", "4", "2", "1", "3", "bar", "spam", "eggs"
        ]
    );
    assert_eq!(
        shape("{key.attr[0][string]!r}"),
        [
            "field=\"key\":\"{key.attr[0][string]!r}\"[field_name=\"key\":\"key.attr[0][string]\"[name=\"key\":\"key\" attribute=\"attr\":\"attr\" element_index=0:\"0\" element_index=\"string\":\"string\"] conversion=\"r\":\"r\"]"
        ]
    );
    for (text, message) in [
        ("{key..}", "Empty attribute in format string"),
        ("{key[]}", "Empty attribute in format string"),
        ("{key[}", "Expected '}' before end of string"),
        (
            "{key[0]after}",
            "Only '.' or '[' may follow ']' in format field specifier",
        ),
        ("{", "Single '{' encountered in format string"),
        ("}", "Single '}' encountered in format string"),
        ("a}b", "Single '}' encountered in format string"),
        ("{0!}", "Unexpected error parsing format string"),
        ("{0!rr}", "Unexpected error parsing format string"),
        ("{:{:{}}}", "Max format placeholder recursion exceeded"),
        ("{foo}a{}b{bar..}", "Empty attribute in format string"),
        (
            "{foo}a{}b{bar:{spam..}}",
            "Empty attribute in format string",
        ),
    ] {
        assert_eq!(error(text).as_deref(), Some(message), "{text}");
        assert_eq!(parse(text).children.len(), 1, "{text}");
    }
    // Ruff accepts any single conversion character.
    assert_eq!(error("{0!x}"), None);
}

#[test]
fn the_format_spec_mini_language() {
    assert_eq!(
        shape("{:<>-#23,.11b}"),
        [
            "field=0:\"{:<>-#23,.11b}\"[field_name=0:\"\"[auto=0:\"\"] format_spec=\"<>-#23,.11b\":\"<>-#23,.11b\"[fill=\"<\":\"<\" align=\">\":\">\" sign=\"-\":\"-\" alternate:\"#\" width=23:\"23\" grouping=\",\":\",\" precision=11:\".11\" type=\"b\":\"b\"]]"
        ]
    );
    assert_eq!(
        shape("{:033}"),
        [
            "field=0:\"{:033}\"[field_name=0:\"\"[auto=0:\"\"] format_spec=\"033\":\"033\"[zero:\"0\" width=33:\"33\"]]"
        ]
    );
    let spec_error = |spec: &str| -> String {
        let root = parse(&format!("{{:{spec}}}"));
        let field = &root.children[0];
        let spec = field.children.last().expect("spec");
        match spec.children.as_slice() {
            [node] if node.error => match &node.value {
                Some(Value::Str(message)) => message.clone(),
                _ => "?".into(),
            },
            _ => "ok".into(),
        }
    };
    for (spec, expected) in [
        ("%3", "InvalidFormatSpecifier"),
        (".2fa", "InvalidFormatSpecifier"),
        ("ds", "InvalidFormatSpecifier"),
        ("x+", "InvalidFormatSpecifier"),
        ("b4", "InvalidFormatSpecifier"),
        ("o!", "InvalidFormatSpecifier"),
        ("d ", "InvalidFormatSpecifier"),
        ("z", "InvalidFormatType('z')"),
        ("33", "ok"),
        ("<>33", "ok"),
        (".", "InvalidFormatType('.')"),
        ("99999999999999999999999", "DecimalDigitsTooMany"),
        (".99999999999", "PrecisionTooBig"),
        ("!r>10", "ok"),
    ] {
        assert_eq!(spec_error(spec), expected, "{spec}");
    }
    // A brace in a spec opens a field, so a spec the mini-language reads has none.
    assert_eq!(
        error("{:{{x}}}").as_deref(),
        Some("Max format placeholder recursion exceeded")
    );
}

fn summary(node: &PNode, autos: &mut Vec<i64>, indices: &mut Vec<i64>, keywords: &mut Vec<String>) {
    if KINDS[usize::from(node.kind) - 1] == "field_name" {
        let head = &node.children[0];
        let small =
            |value: &crate::code_rule_dsl::value::BigInt| i64::try_from(value).expect("small");
        match (KINDS[usize::from(head.kind) - 1], &head.value) {
            ("auto", Some(Value::Int(value))) => autos.push(small(value)),
            ("index", Some(Value::Int(value))) => indices.push(small(value)),
            ("name", Some(Value::Str(value))) => keywords.push(value.clone()),
            other => panic!("{other:?}"),
        }
    }
    for child in &node.children {
        summary(child, autos, indices, keywords);
    }
}

/// Every case of `rerail-fixtures/python/literal/brace.expected.json`: errors and messages as Ruff 0.16.9 reports them
/// (F521), fields as CPython 3.13's `_string.formatter_parser` reads them where it reads them too.
#[test]
fn templates_match_ruff_and_cpython() {
    let oracle: serde_json::Value =
        serde_json::from_str(oracle!("python/literal/brace.expected.json")).expect("oracle");
    let cases = oracle["cases"].as_array().expect("cases");
    assert!(cases.len() > 1_500);
    let mut compared = 0;
    for case in cases {
        let text = case[0].as_str().expect("text");
        let (python, ruff) = (&case[1], &case[2]);
        assert_eq!(error(text).as_deref(), ruff.as_str(), "{text:?}");
        // Ruff reads `\N{...}` in the decoded value as an escape, braces and all, where CPython
        // reads a field (a raw string's or an escaped backslash's `\N`): Ruff is the reference.
        // Ruff reads an index with Rust's `usize` parser: `+1` is index 1 (a keyword to CPython)
        // and `\u{661}` a keyword (an index to CPython).
        let index_quirk = matches!(text, "{+1}" | "{\u{661}}");
        if ruff.is_string() || python.get("error").is_some() || text.contains("\\N{") || index_quirk
        {
            continue;
        }
        let (mut autos, mut indices, mut keywords) = (Vec::new(), Vec::new(), Vec::new());
        summary(&parse(text), &mut autos, &mut indices, &mut keywords);
        let numbers = |key: &str| -> Vec<i64> {
            python[key]
                .as_array()
                .expect("list")
                .iter()
                .map(|value| value.as_i64().expect("int"))
                .collect()
        };
        assert_eq!(autos, numbers("autos"), "{text:?}");
        assert_eq!(indices, numbers("indices"), "{text:?}");
        let expected: Vec<String> = python["keywords"]
            .as_array()
            .expect("list")
            .iter()
            .map(|value| value.as_str().expect("str").to_owned())
            .collect();
        assert_eq!(keywords, expected, "{text:?}");
        compared += 1;
    }
    assert!(compared > 500, "{compared}");
}
