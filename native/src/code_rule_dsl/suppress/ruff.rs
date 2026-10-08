//! Ruff 0.16.9's `# noqa` matrix: every diagnostic Ruff reports with `--ignore-noqa` on the
//! fixtures in `rerail-fixtures/python/suppress/ruff`, read through `NoqaIndex`, against the ones Ruff keeps when it honours
//! the comments (`rerail-fixtures/python/suppress/generate_ruff.py`).

use super::noqa::{NoqaIndex, parent_statement};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::oracle;
use crate::code_rule_dsl::test_support::*;
use ast_grep_language::LanguageExt;
use serde_json::{Value, json};

const ORACLE: &str = oracle!("python/suppress/ruff.expected.json");

const FIXTURES: &[(&str, &str)] = &[
    ("file_all.py", oracle!("python/suppress/ruff/file_all.py")),
    (
        "file_codes.py",
        oracle!("python/suppress/ruff/file_codes.py"),
    ),
    (
        "file_invalid.py",
        oracle!("python/suppress/ruff/file_invalid.py"),
    ),
    ("inline.py", oracle!("python/suppress/ruff/inline.py")),
    ("mapping.py", oracle!("python/suppress/ruff/mapping.py")),
    ("shebang.py", oracle!("python/suppress/ruff/shebang.py")),
    (
        "start_blank.py",
        oracle!("python/suppress/ruff/start_blank.py"),
    ),
    (
        "start_formfeed.py",
        oracle!("python/suppress/ruff/start_formfeed.py"),
    ),
    (
        "start_indent.py",
        oracle!("python/suppress/ruff/start_indent.py"),
    ),
    (
        "start_noqa_first.py",
        oracle!("python/suppress/ruff/start_noqa_first.py"),
    ),
    (
        "start_noqa_later.py",
        oracle!("python/suppress/ruff/start_noqa_later.py"),
    ),
    (
        "terminators.py",
        oracle!("python/suppress/ruff/terminators.py"),
    ),
];

/// The rules that give their diagnostics a parent offset (Ruff's `set_parent`), as the Ruff
/// harness's `noqa.ts` lists them.
const PARENT_RULES: &[&str] = &[
    "F401", "F811", "F822", "N811", "N812", "N813", "N814", "N817", "TC001", "TC002", "TC003",
    "TC004", "TC008",
];

/// Each diagnostic of a fixture's `all` or `kept` list, as `CODE@start`.
fn diagnostics(oracle: &Value, fixture: &str, list: &str) -> Vec<(String, usize)> {
    oracle[fixture][list]
        .as_array()
        .unwrap_or_else(|| panic!("{fixture} {list}"))
        .iter()
        .map(|diagnostic| {
            (
                diagnostic[0].as_str().expect("code").to_owned(),
                diagnostic[1].as_u64().expect("start") as usize,
            )
        })
        .collect()
}

#[test]
fn noqa_comments_suppress_as_ruff_honours_them() {
    let oracle: Value = serde_json::from_str(ORACLE).expect("oracle JSON");
    assert_eq!(oracle["ruff"], "0.16.9");
    let mut report = Vec::new();
    let mut counted = (0, 0);
    for (fixture, source) in FIXTURES {
        let root = GrammarDialect::Python.ast_grep(*source);
        let root = root.root();
        let lines = LineTable::new(source, Terminators::Newlines);
        let index = NoqaIndex::read(&root, source, &lines);
        let kept = diagnostics(&oracle, fixture, "kept");
        let ours = diagnostics(&oracle, fixture, "all")
            .into_iter()
            .filter(|(code, start)| {
                let parent = PARENT_RULES
                    .contains(&code.as_str())
                    .then(|| parent_statement(&root, *start))
                    .flatten();
                !index.suppresses(std::slice::from_ref(code), *start, parent)
            })
            .collect::<Vec<_>>();
        counted.0 += diagnostics(&oracle, fixture, "all").len();
        counted.1 += kept.len();
        if ours != kept {
            report.push(format!("{fixture}: we keep {ours:?}, Ruff keeps {kept:?}"));
        }
    }
    assert_eq!(report, Vec::<String>::new(), "\n{}", report.join("\n"));
    assert_eq!(counted, (87, 33));
}

/// Ports with `suppress.noqa` drop what Ruff drops, end to end: E741 (an ambiguous name assigned)
/// and INP001 (a file-level diagnostic at byte 0, which a shebang's line maps to the next line),
/// anchored at `file.start`: the module node starts at its first token, which blank lines, white
/// space or a form feed can move off byte 0 (the fix round after the suites scored M12).
#[test]
fn ports_with_suppress_noqa_keep_what_ruff_keeps() {
    let oracle: Value = serde_json::from_str(ORACLE).expect("oracle JSON");
    let ports = [
        (
            "E741",
            "witness.start",
            json!({ "kind": "identifier", "regex": "^[lIO]$", "inside": { "kind": "assignment", "field": "left" } }),
        ),
        (
            "INP001",
            "file.start",
            json!({ "kind": "module",
                "not": { "has": { "kind": "comment", "regex": "^#!", "layout": { "start": { "eq": 0 } } } } }),
        ),
    ];
    for (code, anchor, rule) in ports {
        let document = clause_document(
            json!({ "id": "port", "forbid": { "ast": { "language": "python", "rule": rule } },
            "violation": { "anchor": anchor, "code": code, "message": "port", "suppress": { "noqa": { "codes": [code] } } } }),
        );
        let compiled = compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        for (fixture, source) in FIXTURES {
            let result = evaluate_file(&compiled, "python", fixture, source);
            let ours = result["violations"]
                .as_array()
                .expect("findings")
                .iter()
                .map(|violation| violation["span"]["startByte"].as_u64().expect("start") as usize)
                .collect::<Vec<_>>();
            let kept = diagnostics(&oracle, fixture, "kept")
                .into_iter()
                .filter(|(kept, _)| kept == code)
                .map(|(_, start)| start)
                .collect::<Vec<_>>();
            assert_eq!(ours, kept, "{code} {fixture}");
        }
    }
}
