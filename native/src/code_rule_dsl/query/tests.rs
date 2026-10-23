use super::regex_matches;
use crate::code_rule_dsl::budget::{MatchBudget, set_shortcuts_off};
use crate::code_rule_dsl::test_support::{CAPABILITIES, evaluate_file, pack_test_data};
use crate::code_rule_dsl::{CodeRuleDslRuntime, CompiledCodeRuleDsl};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Matches keep their byte ranges past multibyte text; a match past the limit leaves the scan
/// unfinished.
#[test]
fn regex_matches_keep_byte_ranges_and_stop_past_the_limit() {
    let regex = Regex::new("é").unwrap();
    let (first, finished) = regex_matches(&regex, "éxé.ts", 1);
    assert_eq!((first.len(), first[0].clone(), finished), (1, 0..2, false));
    let all = regex_matches(&regex, "éxé.ts", 2);
    assert_eq!((all.0.as_slice(), all.1), (&[0..2, 3..5][..], true));
}

/// An exhausted match deadline leaves the scan unfinished before it keeps any match.
#[test]
fn regex_matches_stop_at_the_deadline() {
    let _budget = crate::code_rule_dsl::budget::MatchBudget::expired_for_test();
    let regex = Regex::new("needle").unwrap();
    assert_eq!(regex_matches(&regex, "needle\n", 2), (Vec::new(), false));
}

/// A Python pack code rule (a path under `packs/python/`: `rules/` or `candidates/rules/`), as
/// the conformance runner registers it.
fn python_pack_code_rule(rule: &str) -> CompiledCodeRuleDsl {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../packs/python")
        .join(rule);
    CodeRuleDslRuntime::compile(
        std::fs::read_to_string(&path).expect("pack policy"),
        "json".to_owned(),
        CAPABILITIES.to_owned(),
    )
    .unwrap_or_else(|error| panic!("{}: {}", path.display(), error.reason))
}

/// A code rule's result on one file, compiled and evaluated with the work-skipping shortcuts on and
/// off (`set_shortcuts_off`): they must agree, violations and failures alike.
fn agreeing(compile: &dyn Fn() -> CompiledCodeRuleDsl, path: &str, source: &str) -> Value {
    set_shortcuts_off(true);
    let without = evaluate_file(&compile(), "python", path, source);
    set_shortcuts_off(false);
    let with = evaluate_file(&compile(), "python", path, source);
    assert_eq!(with, without, "{path}: the shortcuts changed the result");
    with
}

/// The shortcuts that only skip work (the anchor-set shortcuts) change no result of F811, F821 or
/// F841: on every Ruff fixture, and on a module that mixes unpacking, assignment chains, walruses,
/// `with` targets, redefinitions and dotted imports.
#[test]
fn python_shortcuts_change_no_pack_result() {
    let fixtures = pack_test_data("packs/python/upstream/ruff/fixtures");
    let mut files = Vec::new();
    let mut pending = vec![fixtures];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("fixtures") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "py" || extension == "pyi")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    assert!(files.len() > 50, "{} fixtures", files.len());
    let mixed = "\
import os.path
import os
from typing import overload
import json as json
def f(items):
    a, b = c = (1, 2)
    d = e = f = [3, 4]
    (g, h) = i = j = items
    k = (l := 5)
    m, n = o = p = 6, 7
    with open('x') as (q, r), open('y') as s:
        pass
    t = 1
    t = 2
    u: int = 3
    v += 1
    for w in items:
        pass
    import json
    def os():
        pass
    return [x for x in items if (y := x)]
class C:
    import sys
    z = sys
";
    for rule in [
        "candidates/rules/ruff-f811-redefined-while-unused.json",
        "candidates/rules/ruff-f821-undefined-name.json",
        "candidates/rules/ruff-f841-unused-variable.json",
    ] {
        let compile = || python_pack_code_rule(rule);
        for file in &files {
            let source = std::fs::read_to_string(file).expect("fixture");
            let path = if file.extension().is_some_and(|extension| extension == "pyi") {
                "fixture.pyi"
            } else {
                "fixture.py"
            };
            agreeing(&compile, path, &source);
        }
        let result = agreeing(&compile, "mixed.py", mixed);
        assert_eq!(result["failures"], json!([]), "{rule}: {result}");
    }
}

/// F841's work grows linearly with a function's assignments, each an unpacked chain whose
/// identifiers the capture-free utilities are asked about: doubling them at most about doubles
/// the budget units spent.
#[test]
fn f841_work_grows_linearly_with_assignments() {
    let code_rule = python_pack_code_rule("candidates/rules/ruff-f841-unused-variable.json");
    let mut units = Vec::new();
    for assignments in [500, 1000, 2000] {
        let mut source = "def f(items):\n".to_owned();
        for index in 0..assignments {
            source.push_str(&format!(
                "    a{index}, b{index} = c{index} = d{index} = (items, {index}, (e{index}, items))\n"
            ));
        }
        source.push_str("    return a0, b0, c0, d0\n");
        let budget = MatchBudget::work(&Arc::new(AtomicBool::new(false)), u64::MAX);
        let result = evaluate_file(&code_rule, "python", "chains.py", &source);
        let spent = budget.spent();
        drop(budget);
        assert_eq!(result["failures"], json!([]), "{result}");
        // `c` and `d` of every assignment but the first are unused; `a` and `b` are unpacked.
        assert_eq!(
            result["violations"].as_array().map(Vec::len),
            Some(2 * (assignments - 1)),
            "{assignments} assignments"
        );
        units.push(spent);
    }
    assert!(units[0] > 0, "the evaluation polled its budget: {units:?}");
    for pair in units.windows(2) {
        assert!(
            pair[1] * 4 <= pair[0] * 9,
            "units {units:?}: doubling the assignments more than doubled the work"
        );
    }
}
