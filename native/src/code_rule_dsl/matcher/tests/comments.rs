//! Comments in rules: every production grammar keeps its comments as nodes a rule can match by
//! `kind` and `regex`, and a comment is a sibling of the code around it, never a child of the
//! declaration it documents.

use crate::code_rule_dsl::test_support::{
    CAPABILITIES, clause_document, compile_document_with, evaluate_file, violation,
};
use serde_json::{Value, json};

/// The texts a rule of `language` matches in one file the kernel reads as `label`.
fn found(language: &str, label: &str, path: &str, rule: Value, source: &str) -> Vec<String> {
    let capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities are JSON");
    let compiled = compile_document_with(
        clause_document(json!({
            "id": "comments",
            "forbid": { "ast": { "language": language, "rule": rule } },
            "violation": violation(),
        })),
        &capabilities,
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, label, path, source);
    assert_eq!(result["status"], "completed", "{result}");
    result["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .map(|violation| {
            let span = &violation["span"];
            let start = span["startByte"].as_u64().expect("start") as usize;
            let end = span["endByte"].as_u64().expect("end") as usize;
            source[start..end].to_owned()
        })
        .collect()
}

fn comment(regex: &str) -> Value {
    json!({ "kind": "comment", "regex": regex })
}

#[test]
fn comment_text_and_style_are_matched_by_regex_over_the_whole_comment() {
    let ecma = "// line\n/* block */\n/** doc */\nconst a = 1; // trailing\n";
    for (label, path) in [
        ("typescript", "a.ts"),
        ("tsx", "a.tsx"),
        ("javascript", "a.js"),
    ] {
        let line = found("typescript", label, path, comment("^//"), ecma);
        assert_eq!(line, ["// line", "// trailing"], "{label}");
        let block = found("typescript", label, path, comment(r"^/\*([^*]|\*/)"), ecma);
        assert_eq!(block, ["/* block */"], "{label}");
        let doc = found("typescript", label, path, comment(r"^/\*\*[^/]"), ecma);
        assert_eq!(doc, ["/** doc */"], "{label}");
    }
    let python = "# lead\nx = 1  # trailing\n";
    assert_eq!(
        found("python", "python", "a.py", comment("^#"), python),
        ["# lead", "# trailing"]
    );
    // PHP's `#` and `//` are line comments; `#[` opens an attribute, not a comment.
    let php = "<?php\n# hash\n// slashes\n/* block */\n/** doc */\n#[Attr]\nfunction f() {}\n";
    assert_eq!(
        found("php", "php", "a.php", comment("^(#|//)"), php),
        ["# hash", "// slashes"]
    );
    assert_eq!(
        found("php", "php", "a.php", comment(r"^/\*\*\s"), php),
        ["/** doc */"]
    );
}

#[test]
fn a_leading_comment_precedes_its_declaration_and_is_not_inside_it() {
    let leading = |kind: &str| json!({ "kind": "comment", "precedes": { "stopBy": "neighbor", "kind": kind } });
    let inside =
        |kind: &str| json!({ "kind": "comment", "inside": { "stopBy": "end", "kind": kind } });
    let ts = "/** Doc. */\nexport function a() {}\n\nexport function b() {}\n";
    assert_eq!(
        found(
            "typescript",
            "typescript",
            "a.ts",
            leading("export_statement"),
            ts
        ),
        ["/** Doc. */"]
    );
    assert!(
        found(
            "typescript",
            "typescript",
            "a.ts",
            inside("export_statement"),
            ts
        )
        .is_empty()
    );
    // The declaration side: an export with no doc comment directly before it.
    let undocumented = json!({
        "kind": "export_statement",
        "not": { "follows": { "stopBy": "neighbor", "kind": "comment", "regex": r"^/\*\*" } },
    });
    assert_eq!(
        found("typescript", "typescript", "a.ts", undocumented, ts),
        ["export function b() {}"]
    );

    // Python: a comment after a function at module indentation belongs to the module, before the
    // next definition; one indented after the body's last statement belongs to the body.
    let python = "# lead\ndef f():\n    pass\n    # tail of f\n# before g\ndef g():\n    pass\n";
    assert_eq!(
        found(
            "python",
            "python",
            "a.py",
            leading("function_definition"),
            python
        ),
        ["# lead", "# before g"]
    );
    assert_eq!(
        found(
            "python",
            "python",
            "a.py",
            inside("function_definition"),
            python
        ),
        ["# tail of f"]
    );

    let php = "<?php\n/** Doc */\nfunction f() {}\n# note\nfunction g() {}\nfunction h() {}\n";
    assert_eq!(
        found("php", "php", "a.php", leading("function_definition"), php),
        ["/** Doc */", "# note"]
    );
    assert!(found("php", "php", "a.php", inside("function_definition"), php).is_empty());
}

#[test]
fn a_blank_line_or_code_on_the_same_line_is_read_through_layout() {
    // Attached: no blank line between the comment and the declaration after it.
    let attached = json!({
        "kind": "comment",
        "precedes": { "stopBy": "neighbor", "kind": "lexical_declaration" },
        "layout": { "after": { "to": "token", "blank": { "eq": 0 } } },
    });
    let source =
        "// attached\nconst a = 1;\n// detached\n\nconst b = 2;\nconst c = 3; // trailing\n";
    assert_eq!(
        found("typescript", "typescript", "a.ts", attached, source),
        ["// attached"]
    );
    let trailing = json!({ "kind": "comment", "layout": { "before": { "to": "token", "breaks": { "eq": 0 } } } });
    assert_eq!(
        found("typescript", "typescript", "a.ts", trailing, source),
        ["// trailing"]
    );
}

#[test]
fn blade_comment_names_the_templates_comments_and_its_embedded_phps() {
    let source = "{{-- blade --}}\n<!-- html -->\n@php\n    // note\n    $x = 1; /** @var int */\n@endphp\n<?php # tag ?>\n";
    assert_eq!(
        found(
            "blade",
            "blade",
            "a.blade.php",
            json!({ "kind": "comment" }),
            source
        ),
        [
            "{{-- blade --}}",
            "<!-- html -->",
            "// note",
            "/** @var int */",
            "# tag "
        ]
    );
    // A doc comment's tree reads the embedded PHP's comments only, now named by `kind`.
    let doc = json!({ "kind": "comment", "parse": { "grammar": "phpDoc", "has": { "stopBy": "end", "kind": "php_doc_tag" } } });
    assert_eq!(
        found("blade", "blade", "a.blade.php", doc, source),
        ["/** @var int */"]
    );
}
