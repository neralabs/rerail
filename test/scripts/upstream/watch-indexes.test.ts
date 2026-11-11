/**
 * The Ruff and Clippy source files the nightly report reads to find a ported rule's code and the tests upstream adds for
 * it (scripts/upstream/watch-indexes.ts), on excerpts in the shape the pinned sources have.
 */
import assert from "node:assert/strict";
import { test } from "vitest";

import { clippyAnnotatedLints, clippyDeclaredLints, ruffCodes, ruffTestCases } from "../../../scripts/upstream/watch-indexes.js";

const REGISTRY = `pub enum Linter {
    /// [flake8-bugbear](https://pypi.org/project/flake8-bugbear/)
    #[prefix = "B"]
    Flake8Bugbear,
    /// [pycodestyle](https://pypi.org/project/pycodestyle/)
    #[prefix = "E"]
    #[prefix = "W"]
    Pycodestyle,
}
`;

const CODES = `        (Flake8Bugbear, "005") => rules::flake8_bugbear::rules::StripWithMultiCharacters,
        // (Flake8Bugbear, "999") => rules::flake8_bugbear::rules::Commented,
        (Pycodestyle, "E711") => (RuleGroup::Stable, rules::pycodestyle::rules::NoneComparison),
        (Pycodestyle, "W605") => rules::pycodestyle::rules::InvalidEscapeSequence,
`;

test("Ruff's codes.rs and registry.rs give each full code's plugin and struct", () => {
  assert.deepEqual(Object.fromEntries(ruffCodes(CODES, REGISTRY)), {
    B005: { plugin: "flake8_bugbear", struct: "StripWithMultiCharacters" },
    E711: { plugin: "pycodestyle", struct: "NoneComparison" },
    W605: { plugin: "pycodestyle", struct: "InvalidEscapeSequence" },
  });
});

test("a Ruff plugin's mod.rs lists each test_case's rule and fixture", () => {
  const modRs = `    #[test_case(Rule::StripWithMultiCharacters, Path::new("B005.py"))]
    #[test_case(
        Rule::UnaryPrefixIncrementDecrement,
        Path::new("B002.py")
    )]
    #[test_case(Rule::LoopVariableOverridesIterator, Path::new("B020.py"); "named")]
    fn rules(rule_code: Rule, path: &Path) -> Result<()> {
`;
  assert.deepEqual(ruffTestCases(modRs), [
    { struct: "StripWithMultiCharacters", file: "B005.py" },
    { struct: "UnaryPrefixIncrementDecrement", file: "B002.py" },
    { struct: "LoopVariableOverridesIterator", file: "B020.py" },
  ]);
});

test("Clippy's declared_lints.rs gives each lint's module, nested modules as paths", () => {
  const text = `pub static LINTS: &[&::declare_clippy_lint::LintInfo] = &[
    crate::double_parens::DOUBLE_PARENS_INFO,
    crate::misc_early::ZERO_PREFIXED_LITERAL_INFO,
    crate::operators::arithmetic::ARITHMETIC_SIDE_EFFECTS_INFO,
];
`;
  assert.deepEqual(Object.fromEntries(clippyDeclaredLints(text)), {
    double_parens: "double_parens",
    zero_prefixed_literal: "misc_early",
    arithmetic_side_effects: "operators/arithmetic",
  });
});

test("a Clippy UI test's //~ annotations name its lints, whatever their marker or level", () => {
  const text = `fn main() {
    let _ = ((1));
    //~^ double_parens
    let x = 0123; //~ ERROR: clippy::zero_prefixed_literal
    //~| precedence_bits
    // ~ not_an_annotation
}
`;
  assert.deepEqual([...clippyAnnotatedLints(text)], ["double_parens", "zero_prefixed_literal", "precedence_bits"]);
});
