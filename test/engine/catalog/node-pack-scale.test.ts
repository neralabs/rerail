/**
 * The node pack's n/prefer-promises ports on a large test file. Their aliasing branches read every occurrence of a
 * callee's variable, once per call: on a file that calls imported and const functions thousands of times that was
 * quadratic, and both rules ran out of their utility budget on OpenMausBot's 14,000-line `server/index.test.ts`
 * (reporting nothing for the file). They now skip those branches where no program that runs could reach them.
 */
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { afterEach, describe, expect, it } from "vitest";
import { NativeCodeRuleEnforcer, codeRuleSchema } from "../../../src/engine/index.js";
import { testCodeRuleCatalog } from "../../support/repository-rules.js";

const RULES = ["n-prefer-promises-fs", "n-prefer-promises-dns"];
const directories: string[] = [];
afterEach(async () => {
  await Promise.all(directories.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});

/** A test file in the shape of a large suite, with the callback calls each port reports planted at its end. */
function largeTestFile(calls: number): { source: string; planted: string[] } {
  const lines = [
    'import { expect, it, vi } from "vitest";',
    'import { readFileSync } from "node:fs";',
    "const read = (path) => readFileSync(path, \"utf8\");",
    'it("reads", () => {',
    ...Array.from({ length: calls }, (_, index) => `  expect(read(vi.fn(${index}))).toBe(read("${index}"));`),
    "});",
  ];
  const planted = [
    'const fs = require("fs"); fs.readFile("a", () => {});',
    'const { writeFile } = require("node:fs"); writeFile("b", "", () => {});',
    'let alias; alias = require("fs"); alias.mkdir("c", () => {});',
    'const dns = require("dns"); dns.lookup("d", () => {});',
  ];
  const first = lines.length;
  return {
    source: [...lines, ...planted].join("\n") + "\n",
    planted: [
      `n-prefer-promises-fs:${first}`,
      `n-prefer-promises-fs:${first + 1}`,
      `n-prefer-promises-fs:${first + 2}`,
      `n-prefer-promises-dns:${first + 3}`,
    ],
  };
}

describe("the node pack's n/prefer-promises ports at scale", () => {
  it("finish on a large test file and report the callback calls planted in it", async () => {
    // Candidates until they are promoted.
    const pack = join("packs", "node", "candidates");
    const rules = await Promise.all(RULES.map(async (id) =>
      codeRuleSchema.parse(JSON.parse(await readFile(join(pack, "rules", `${id}.json`), "utf8")))));
    const root = await mkdtemp(join(tmpdir(), "rerail-node-scale-"));
    directories.push(root);
    // The planted calls read `require`, which the ports reach only in a CommonJS file (packs/node/README.md).
    await writeFile(join(root, "package.json"), '{ "type": "commonjs" }\n');
    const enforcer = new NativeCodeRuleEnforcer(testCodeRuleCatalog(rules), {
      workspaceRoot: root, scope: "touched-files", mode: "log", steerDelivery: { async requestSteer() {} },
    });
    try {
      await enforcer.openWorkspace("node-scale");
      const { source, planted } = largeTestFile(3_000);
      await enforcer.auditGeneratedEdits("node-scale", {
        revision: 1, checkpoints: ["edit"],
        edits: [{ path: "server/index.test.ts", language: "typescript", generatedText: source }],
      });
      while (enforcer.snapshotStatus().pending) await sleep(10);
      const status = enforcer.snapshotStatus();
      expect(status.failures).toEqual([]);
      expect(status.violations.map((violation) => `${violation.ruleId}:${violation.span?.startLine}`).toSorted()).toEqual(planted.toSorted());
    } finally {
      await enforcer.close();
    }
  }, 60_000);
});
