import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { loadNativeBinding } from "../../../../src/engine/index.js";

const native = loadNativeBinding();
const capabilities = readFileSync("native/src/code_rule_dsl/capabilities.json", "utf8");
const rule = () => native.CodeRuleDslRuntime.compile(JSON.stringify({
  format: "rerail.policy/v1", id: "owner", revision: 1, mode: "log", clauses: [{
    id: "calls", forbid: { ast: { language: "python", syntax: "valid", rule: {
      all: [{ kind: "call" }, { has: { kind: "identifier", regex: "^target$", stopBy: "end" } }],
    } } }, violation: { code: "TARGET", message: "Target", anchor: "witness.span" },
  }],
}), "json", capabilities);
const file = (path: string, source: string) => ({ path, source, language: "python" });
const snapshot = (files: ReturnType<typeof file>[]) => JSON.stringify({
  format: "rerail.repository-snapshot/v1", revision: 1, files,
});

it("shares immutable facts while snapshots and workspaces keep separate membership and versions", async () => {
  const owner = new native.CodeRuleDslExecution();
  const compiled = rule();
  const a = owner.createWorkspace(), b = owner.createWorkspace();
  const results = async (workspace: typeof a) => JSON.parse(await workspace.resultPageAsync(0, 256 * 1024));
  try {
    a.addCodeRule(compiled); b.addCodeRule(compiled);
    const positive = file("same.py", "target()\n"), clean = file("same.py", "other()\n");
    const run = (files: ReturnType<typeof file>[]) => JSON.parse(owner.evaluate(compiled, snapshot(files), "report"));
    expect(run([positive])).toMatchObject({ failures: [], violations: [expect.anything()] });
    await a.applyAsync(JSON.stringify({ revision: 1, files: [positive], removed: [] }));
    await b.applyAsync(JSON.stringify({ revision: 1, files: [clean], removed: [] }));
    expect((await results(a)).violations).toHaveLength(1);
    expect((await results(b)).violations).toHaveLength(0);
    expect(run([clean])).toMatchObject({ failures: [], violations: [] });
    expect(run([])).toMatchObject({ failures: [], violations: [] });
    await b.applyAsync(JSON.stringify({ revision: 2, files: [], removed: ["same.py"] }));
    expect((await results(a)).violations).toHaveLength(1);
    expect(run([positive]).violations).toHaveLength(1);
  } finally { a.close(); b.close(); owner.close(); }
  expect(() => owner.createWorkspace()).toThrow(/closed/);
});
