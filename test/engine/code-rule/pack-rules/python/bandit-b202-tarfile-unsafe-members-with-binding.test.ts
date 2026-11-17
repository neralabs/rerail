/**
 * bandit-b202-tarfile-unsafe-members must report `extractall` on an archive bound by `with tarfile.open(...) as tar`
 * (#252).
 *
 * Bandit 1.9.4's B202 (bandit/plugins/tarfile_unsafe_members.py) does not trace the receiver. It reports a call whose
 * function name contains `extractall` once the file has run `import tarfile` (an exact module match, so
 * `from tarfile import open` does not count), unless the call passes `filter="data"`. The port used to report only a
 * receiver assigned from `tarfile.open(...)`, so a `with` binding, `tarfile.TarFile.open` and `tarfile.extractall`
 * were missed and `filter="data"` was still reported. Each case's expected positions are what Bandit 1.9.4 reports on
 * it (line, then its column plus one).
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_ID = "bandit-b202-tarfile-unsafe-members";
const document = await readFile(repositoryPath("packs", "python", "candidates", "rules", `${RULE_ID}.json`), "utf8");

const evaluationSchema = z.object({
  violations: z
    .array(z.object({ span: z.object({ startLine: z.number().int().nonnegative(), startColumn: z.number().int().nonnegative() }) }))
    .default([]),
  failures: z.array(z.unknown()).default([]),
});

if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
const rule = runtime.compile(document, "json", capabilities);

/** The `line:column` (both 1-based) of each violation the rule reports in `source`. */
function reported(source: string): string[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "python", path: "app/archive.py", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(rule.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => `${violation.span.startLine + 1}:${violation.span.startColumn + 1}`);
}

const cases: readonly { name: string; source: string; expected: readonly string[] }[] = [
  {
    name: "an archive bound by with ... as",
    source: "import tarfile\n\ndef unpack(p):\n    with tarfile.open(p) as tar:\n        tar.extractall()\n",
    expected: ["5:9"],
  },
  {
    name: "a with binding from tarfile.TarFile.open, among other items",
    source:
      "import os, tarfile\n\ndef unpack(p):\n    with tarfile.TarFile.open(p) as t, open(p) as other:\n" +
      '        t.extractall(path="d")\n        other.extractall()\n',
    expected: ["5:9", "6:9"],
  },
  {
    name: "an archive assigned from tarfile.open, as before",
    source: "import tarfile\ntar = tarfile.open(p)\ntar.extractall()\ntar.extractall(members=[])\n",
    expected: ["3:1", "4:1"],
  },
  {
    name: "tarfile imported under an alias, extractall called on the module",
    source: "import tarfile as tf\ntf.extractall(members=tar)\n",
    expected: ["2:1"],
  },
  {
    name: "a filter other than the literal data",
    source: 'import tarfile\nt.extractall(filter="fully_trusted")\nt.extractall(filter=choice)\nt.extractall(filter=f"data")\n',
    expected: ["2:1", "3:1", "4:1"],
  },
  {
    name: "filter=data, however the literal is written",
    source:
      "import tarfile\n\ndef unpack(p):\n    with tarfile.open(p) as tar:\n        tar.extractall(filter=\"data\")\n" +
      "        tar.extractall(filter='data', members=m)\n        tar.extractall(filter=(\"da\" \"ta\"))\n",
    expected: [],
  },
  {
    name: "a file that only imports from tarfile",
    source: "from tarfile import open\nt = open(p)\nt.extractall()\n",
    expected: [],
  },
  {
    name: "a call before import tarfile",
    source: "t.extractall()\nimport tarfile\n",
    expected: [],
  },
  {
    name: "a submodule named tarfile",
    source: "import os.tarfile\nt.extractall()\n",
    expected: [],
  },
  {
    name: "a function whose name does not contain extractall",
    source: "import tarfile\nt.extract_all()\nt.extract(member)\n",
    expected: [],
  },
];

describe(RULE_ID, () => {
  it.each(cases)("$name", ({ source, expected }) => {
    expect(reported(source)).toEqual(expected);
  });
});
