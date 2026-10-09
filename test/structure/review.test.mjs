// Structure gate, metrics found during the review (#22-#24): each predicate on a tiny in-memory tree.

import { describe, expect, it } from "vitest";
import { memoryRepo } from "../../scripts/check/check-structure/repo.mjs";
import { check as stalePaths } from "../../scripts/check/check-structure/metrics/22-stale-paths.mjs";
import { check as shippedRules } from "../../scripts/check/check-structure/metrics/23-shipped-rules.mjs";
import { check as harnessesInCi } from "../../scripts/check/check-structure/metrics/24-harnesses-in-ci.mjs";

const rule = (id) => JSON.stringify({ format: "rerail.policy/v1", id, clauses: [] });
const paths = (result) => result.findings.map((item) => item.path);
const OLD = ["rerail", "fixtures/"].join("-");

describe("#22 stale paths", () => {
  it("flags each retired path at its line, except in the changelog", () => {
    const result = stalePaths(
      memoryRepo({
        "docs/a.md": `intro\nsee ${OLD}php/x.json and ${OLD}python\n`,
        "CHANGELOG.md": `moved ${OLD} to fixtures/\n`,
      }),
    );
    expect(result.findings).toEqual([
      { path: "docs/a.md", line: 2, message: `stale path ${OLD}` },
      { path: "docs/a.md", line: 2, message: `stale path ${OLD}` },
    ]);
  });
});

describe("#23 shipped rules", () => {
  it("accepts pack entries and flags entries shipping rules from elsewhere, even an empty rules dir", () => {
    const result = shippedRules(
      memoryRepo({
        "package.json": JSON.stringify({
          files: ["dist", "packs/tslint/pack.json", "packs/tslint/rules", "code-rule-dsl/rules", "builtin"],
        }),
        "packs/tslint/pack.json": "{}",
        "packs/tslint/rules/a.json": rule("tslint:a"),
        "builtin/b.json": rule("core:b"),
      }),
    );
    expect(result.findings.map((item) => item.message)).toEqual([
      'files entry "builtin" ships rules from outside packs/<id>/',
      'files entry "code-rule-dsl/rules" ships rules from outside packs/<id>/',
    ]);
    expect(result.actual).toBe("builtin, code-rule-dsl/rules, packs/tslint");
  });
});

describe("#24 harnesses in CI", () => {
  const ci = (script) => `name: CI\njobs:\n  unit:\n    steps:\n      - name: Tests\n        run: |\n${script
    .split("\n")
    .map((line) => `          ${line}`)
    .join("\n")}\n`;

  it("follows npm scripts transitively, including --prefix packages, to the test files they run", () => {
    const result = harnessesInCi(
      memoryRepo({
        ".github/workflows/ci.yml": ci("npm test -- --shard=${{ matrix.shard }}/2\nnpm --prefix playground test"),
        "package.json": JSON.stringify({
          scripts: {
            test: "vitest run --dir test && npm run test:scripts",
            "test:scripts": "node --test test/scripts/ensure-sqlite.test.mjs",
            "test:watch": "vitest --dir test",
            "test:conformance": "node --import tsx test/conformance/cli.ts --check",
          },
        }),
        "test/a.test.ts": "",
        "test/scripts/ensure-sqlite.test.mjs": "",
        "test/conformance/cli.ts": "",
        "integration/b.test.ts": "",
        "playground/package.json": JSON.stringify({ scripts: { test: "vitest run --dir test" } }),
        "playground/test/c.test.ts": "",
        "playground/ui/d.test.tsx": "",
      }),
    );
    expect(result.findings.map((item) => `${item.path}: ${item.message}`)).toEqual([
      "integration/b.test.ts: test file no CI step runs",
      'package.json: script "test:conformance" runs test/conformance/cli.ts, which no CI step runs',
      "playground/ui/d.test.tsx: test file no CI step runs",
    ]);
  });

  it("honours vitest filters and the config's exclude list", () => {
    const result = harnessesInCi(
      memoryRepo({
        ".github/workflows/ci.yml": ci("npm run test:scripts\nnpx vitest run --dir test"),
        "package.json": JSON.stringify({ scripts: { "test:scripts": "vitest run scripts/a.test.mjs" } }),
        "vitest.config.ts": 'export default { test: { exclude: ["**/node_modules/**", "test/slow/**"] } };',
        "scripts/a.test.mjs": "",
        "scripts/b.test.mjs": "",
        "test/slow/c.test.ts": "",
        "test/d.test.ts": "",
        "fixtures/python/value/test_e.py": "",
      }),
    );
    expect(paths(result)).toEqual(["scripts/b.test.mjs", "test/slow/c.test.ts"]);
  });

  it("counts a test file run directly as an entry point", () => {
    const result = harnessesInCi(
      memoryRepo({
        ".github/workflows/ci.yml": ci("cd test/scripts && python3 test_design.py"),
        "test/scripts/test_design.py": "",
      }),
    );
    expect(result.findings).toEqual([]);
  });
});
