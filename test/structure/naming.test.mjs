// Structure gate, naming metrics (#16-#18): each predicate on a tiny in-memory tree.

import { describe, expect, it } from "vitest";
import { memoryRepo } from "../../scripts/check/check-structure/repo.mjs";
import { check as rulesDirs } from "../../scripts/check/check-structure/metrics/16-rules-dirs.mjs";
import { check as supportDirs } from "../../scripts/check/check-structure/metrics/17-test-support-dirs.mjs";
import { check as caseConvention } from "../../scripts/check/check-structure/metrics/18-case-convention.mjs";

const rule = (id) => JSON.stringify({ format: "rerail.policy/v1", id, clauses: [] });
const paths = (result) => result.findings.map((item) => item.path);

describe("#16 rules/ directories", () => {
  it("flags anything but rule documents in a rules/ dir, outside upstream data", () => {
    const result = rulesDirs(
      memoryRepo({
        "packs/example/rules/a.json": rule("example:a"),
        "packs/example/rules/a.verdict.json": "{}",
        "packs/php/rules/README.md": "",
        "packs/php/upstream/phpcs/rules/Foo.php": "",
        "fixtures/rust/upstream/clippy/rules/x.rs": "",
      }),
    );
    expect(paths(result)).toEqual(["packs/example/rules/a.verdict.json", "packs/php/rules/README.md"]);
    expect(result.actual).toBe("2");
  });
});

describe("#17 test helpers in support/", () => {
  it("requires helpers in a support/ dir; tests, declarations and fixture trees are not helpers", () => {
    const result = supportDirs(
      memoryRepo({
        "test/support/oracles.ts": "",
        "test/engine/support/tape.ts": "",
        "test/engine/a.test.ts": "",
        "test/ui-modules.d.ts": "",
        "test/fixtures/complexity/branches.js": "",
        "test/hook-fixtures.ts": "",
        "integration/omp-host.ts": "",
      }),
    );
    expect(paths(result)).toEqual(["integration/omp-host.ts", "test/hook-fixtures.ts"]);
    expect(result.actual).toBe("50% (2/4)");
  });
});

describe("#18 case convention", () => {
  it("wants kebab-case TS/JS files and dirs, snake_case Rust modules and Python files", () => {
    const result = caseConvention(
      memoryRepo({
        "src/engine/code-rule.ts": "",
        "src/engine/code-rule.generated.ts": "",
        "src/hostAdapters/omp.ts": "",
        "scripts/eslint_record.mjs": "",
        "native/src/code_rule_dsl/format_args/mod.rs": "",
        "native/src/code_rule_dsl/formatArgs.rs": "",
        "native/src/code_rule_dsl/value/builtins.generated.rs": "",
        "scripts/generate/model/prepare_model.py": "",
        "scripts/prepare-model.py": "",
      }),
    );
    expect(result.findings.map((item) => `${item.path}: ${item.message}`)).toEqual([
      "native/src/code_rule_dsl/formatArgs.rs: formatArgs.rs is not snake_case",
      "scripts/eslint_record.mjs: eslint_record.mjs is not kebab-case",
      "scripts/prepare-model.py: prepare-model.py is not snake_case",
      "src/hostAdapters/omp.ts: directory hostAdapters/ is not kebab-case",
    ]);
  });

  it("exempts data roots and fixture trees, not React component names elsewhere", () => {
    const result = caseConvention(
      memoryRepo({
        "fixtures/ecma/scope/generate_eslint.mjs": "",
        "packs/next/candidates/probes/labels/fixtures/pages/_app.tsx": "",
        "ui/src/RulesView.tsx": "",
      }),
    );
    expect(paths(result)).toEqual(["ui/src/RulesView.tsx"]);
  });
});
