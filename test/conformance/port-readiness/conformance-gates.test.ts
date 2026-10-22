// conformance/port-readiness/conformance-gates.ts: the conformance gates over every rule no gated tool-pack judges.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, it } from "vitest";

import { conformanceBlockingFailures, runConformanceGates } from "../../../conformance/port-readiness/conformance-gates.js";
import { loadEngine } from "../../../conformance/port-readiness/engine.js";
import { PACKAGE_ROOT } from "../../../src/engine/index.js";

const temporary: string[] = [];
afterEach(() => {
  for (const directory of temporary.splice(0)) rmSync(directory, { recursive: true, force: true });
});

function write(root: string, path: string, value: unknown): void {
  mkdirSync(join(root, path, ".."), { recursive: true });
  writeFileSync(join(root, path), typeof value === "string" ? value : `${JSON.stringify(value, null, 2)}\n`);
}

/** A tree of two packs: `demo`, with no conformance.json, and `ported`, whose one tool has a gates.yaml. */
function tree(): string {
  const root = mkdtempSync(join(tmpdir(), "rerail-conformance-gates-"));
  temporary.push(root);
  const complexity = readFileSync(join(PACKAGE_ROOT, "packs/complexity/rules/complexity-cyclomatic-python.json"), "utf8");
  write(root, "packs/demo/pack.json", { rules: [{ id: "complexity-cyclomatic-python", document: "rules/complexity-cyclomatic-python.json" }] });
  write(root, "packs/demo/rules/complexity-cyclomatic-python.json", complexity);
  write(root, "packs/demo/candidates/rules/broken.json", "{");
  write(root, "packs/ported/pack.json", { rules: [] });
  write(root, "packs/ported/conformance.json", { tools: [{ name: "lint" }], rules: [{ id: "x", policy: "rules/x.json", upstream: { tool: "lint", rule: "x" } }] });
  write(root, "packs/ported/upstream/lint/gates.yaml", "");
  write(root, "packs/ported/rules/x.json", complexity);
  return root;
}

describe("the conformance gates", () => {
  // Alone it takes under a second, but as the file's first case, with the whole suite sharing the machine, it has taken
  // 14 s, past vitest's 5 s default.
  it("judge the rules no gated tool-pack judges, and leave the gated tool-pack's to its own run", async () => {
    const root = tree();
    const report = await runConformanceGates(root, { engine: loadEngine(), cacheRoot: join(root, ".cache") });
    assert.deepEqual(report.rules.map((rule) => [rule.file, rule.location, rule.pass, rule.stoppedAt]), [
      ["packs/demo/candidates/rules/broken.json", "candidate", false, "loads"],
      ["packs/demo/rules/complexity-cyclomatic-python.json", "shipped", true, null],
    ]);
    assert.deepEqual(report.packs.map((entry) => [entry.pack, entry.checks.pass, entry.judgedByToolPacks]), [["demo", true, 0], ["ported", true, 1]]);
    assert.equal(conformanceBlockingFailures(report), 0);
  }, 30_000);

  it("make no in-scope check, and say passes-conformance does not apply to a pack with no conformance.json", async () => {
    const root = tree();
    const report = await runConformanceGates(root, { engine: loadEngine(), cacheRoot: join(root, ".cache") });
    const shipped = report.rules.find((rule) => rule.location === "shipped")!;
    assert.match(shipped.gates.loads.checks.find((check) => check.id === "in-scope")!.detail, /^not checked: /u);
    assert.equal(shipped.gates["passes-conformance"].applies, false);
    assert.match(shipped.gates["passes-conformance"].checks[0]!.detail, /packs\/demo has no conformance\.json/u);
    assert.equal(shipped.gates.compact.pass, true);
  });

  it("block on a shipped rule that fails, and on a pack whose shipped rules do not load together", async () => {
    const root = tree();
    write(root, "packs/demo/rules/broken.json", "{");
    write(root, "packs/demo/pack.json", { rules: [{ id: "complexity-cyclomatic-python", document: "rules/complexity-cyclomatic-python.json" }, { id: "broken", document: "rules/broken.json" }] });
    const report = await runConformanceGates(root, { engine: loadEngine(), cacheRoot: join(root, ".cache") });
    assert.equal(report.rules.find((rule) => rule.file === "packs/demo/rules/broken.json")?.pass, false);
    assert.equal(report.packs.find((entry) => entry.pack === "demo")?.checks.pass, false);
    assert.equal(conformanceBlockingFailures(report), 2);
  });
});
