// scripts/check/check-ci-paths.mjs: ci.yml's path filters cover what the jobs they gate import and read.

import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { ciPathProblems, COMMON, e2eInstalledPackReads, expectedOutput } from "../../scripts/check/check-ci-paths.mjs";
import { removeFolderSync } from "../support/platform/remove-folder.ts";

let root;
afterEach(() => {
  if (root !== undefined) removeFolderSync(root);
  root = undefined;
});

/** A directory holding `files` (path to content). */
function repository(files) {
  root = mkdtempSync(join(tmpdir(), "rerail-ci-paths-"));
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
  return root;
}

const indent = (lines, spaces) => lines.map((line) => `${" ".repeat(spaces)}${line}`).join("\n");

/** A ci.yml whose `changes` job has `filters` (the paths-filter YAML) and `outputs`, with one job gated on `score`. */
function workflow({ filters, outputs = { score: expectedOutput("score") }, gate = "needs.changes.outputs.score != 'false'", required = ["changes", "score"] }) {
  return [
    "jobs:",
    "  changes:",
    "    outputs:",
    indent(Object.entries(outputs).map(([name, value]) => `${name}: ${value}`), 6),
    "    steps:",
    "      - id: filter",
    "        with:",
    "          filters: |",
    indent(filters.split("\n"), 12),
    "  score:",
    "    needs: [changes]",
    `    if: ${gate}`,
    "    steps: []",
    "  ci-result:",
    "    needs:",
    indent(required.map((job) => `- ${job}`), 6),
    "    steps: []",
    "",
  ].join("\n");
}

const COMMON_FILTER = ["common: &common", ...COMMON.map((path) => `  - '${path}'`)].join("\n");
const SCORE = { score: { jobs: ["score"], entries: ["tool/cli.ts"], reads: ["data/rules.json"] } };
const SOURCES = {
  "tool/cli.ts": 'import { helper } from "./helper.js";\nimport { shared } from "../lib/shared.ts";\nimport "node:fs";\nimport "yaml";\nhelper(shared);\n',
  "tool/helper.ts": "export const helper = (value: unknown) => value;\n",
  "lib/shared.ts": "export const shared = 1;\n",
  "data/rules.json": "[]\n",
};

describe("check-ci-paths", () => {
  it("passes a filter, shared through an anchor, that covers the closure and the runtime reads", async () => {
    const filters = `${COMMON_FILTER}\nscore:\n  - *common\n  - 'tool/**'\n  - 'lib/*.ts'\n  - 'data/**'`;
    const problems = await ciPathProblems(repository({ ".github/workflows/ci.yml": workflow({ filters }), ...SOURCES }), SCORE, {});
    expect(problems).toEqual([]);
  });

  it("names a file the entry points import outside the filter, with its importer, and a runtime read outside it", async () => {
    const filters = `${COMMON_FILTER}\nscore:\n  - *common\n  - 'tool/**'`;
    const problems = await ciPathProblems(repository({ ".github/workflows/ci.yml": workflow({ filters }), ...SOURCES }), SCORE, {});
    expect(problems).toEqual([
      { group: "score", path: "data/rules.json", message: "read at runtime by score, matches no glob of the score filter" },
      { group: "score", path: "lib/shared.ts", message: "imported by tool/cli.ts, matches no glob of the score filter" },
    ]);
  });

  it("requires the files every job shares, a fail-open output, and the gated job to need changes and test its output", async () => {
    const filters = "score:\n  - 'tool/**'\n  - 'lib/**'\n  - 'data/**'\n  - 'package.json'";
    const text = workflow({ filters, outputs: { score: "${{ steps.filter.outputs.score }}" }, gate: "always()" });
    const problems = await ciPathProblems(repository({ ".github/workflows/ci.yml": text, ...SOURCES }), SCORE, {});
    expect(problems.map(({ path, message }) => path ?? message)).toEqual([
      `changes output score must be ${expectedOutput("score")}, so it fails open`,
      ...COMMON.filter((path) => path !== "package.json"),
      "job score must need changes and be skipped when needs.changes.outputs.score is 'false'",
    ]);
  });

  it("flags an output gated on that the check does not know, a job ci-result does not need, and a missing entry or read", async () => {
    const filters = `${COMMON_FILTER}\nscore:\n  - *common\n  - '**'`;
    const text = workflow({ filters, gate: "needs.changes.outputs.score != 'false' && needs.changes.outputs.other != 'false'", required: ["changes"] });
    const groups = { score: { jobs: ["score"], entries: ["tool/cli.ts", "tool/gone/*.ts"], reads: ["data/gone.json"] } };
    const problems = await ciPathProblems(repository({ ".github/workflows/ci.yml": text, ...SOURCES }), groups, {});
    expect(problems).toEqual([
      { group: "ci-result", message: "ci-result does not need job score, so its failure would not fail CI result" },
      { group: "other", message: ".github/workflows/ci.yml gates on changes output other, which this check does not know: add it to GROUPS" },
      { group: "score", path: "data/gone.json", message: "a runtime read this check lists no longer exists: update reads" },
      { group: "score", path: "tool/gone/*.ts", message: "entry point glob matches no file: update entries" },
    ]);
  });

  it("refuses glob syntax it would read differently from paths-filter, and a missing filter", async () => {
    const filters = `${COMMON_FILTER}\nscore:\n  - *common\n  - 'tool/{a,b}/**'\n  - '!docs/**'\n  - 'lib/**.ts'\n  - added: 'data/**'`;
    const groups = { ...SCORE, absent: { jobs: [], entries: [], reads: [] } };
    const problems = await ciPathProblems(repository({ ".github/workflows/ci.yml": workflow({ filters }), ...SOURCES }), groups, {});
    expect(problems.map(({ message }) => message)).toEqual([
      'filter glob "tool/{a,b}/**" uses glob syntax this check does not read as picomatch does',
      'filter glob "!docs/**" uses glob syntax this check does not read as picomatch does',
      'filter glob "lib/**.ts" uses ** inside a segment',
      "filter glob {\"added\":\"data/**\"} is not a plain glob (change-type filters are not supported)",
      ".github/workflows/ci.yml: the changes job has no absent filter",
    ]);
  });

  it("reads the packs the dashboard browser tests install, and fails when it cannot find them", () => {
    const packs = 'const INSTALLED_PACKS = ["complexity", "python"] as const;\n';
    expect(e2eInstalledPackReads(repository({ "e2e/support/scenarios.ts": packs }))).toEqual(["packs/complexity/package.json", "packs/python/package.json"]);
    removeFolderSync(root);
    expect(() => e2eInstalledPackReads(repository({ "e2e/support/scenarios.ts": "export {};\n" }))).toThrow(/INSTALLED_PACKS not found/u);
  });
});
