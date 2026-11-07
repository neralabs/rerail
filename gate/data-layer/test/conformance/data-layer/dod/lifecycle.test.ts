import { readFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";
import { prepareDataLayerAudit } from "../../../../src/cli/index.js";
import { loadNativeBinding, RESULT_PAGE_BYTES } from "../../../../src/engine/index.js";
const oracle = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-lifecycle.json", "utf8"));
type File = { path: string; source: string };
type Row = { path: string; line: number; code: string; message: string };
type State = { name: string; files: File[]; expected: Row[] };
const canonical = (rows: Row[]) => rows.map(row => JSON.stringify([row.path, row.line, row.code, row.message])).sort();
for (const record of [...oracle.cases, ...oracle.dependencyCases] as { check: string; id?: string; states: State[] }[]) it(`preserves ${record.id ?? record.check} across clean, positive, restored, removed and readded revisions`, async () => {
  const root = await mkdtemp(join(tmpdir(), "rerail-lifecycle-"));
  const workspace = new (loadNativeBinding().CodeRuleDslWorkspace)();
  try {
    const files = (values: File[]) => values.map(file => ({ ...file, language: file.path.endsWith(".py") ? "python" : "text" }));
    const { compiled } = await prepareDataLayerAudit({ repositoryRoot: root, packDirectory: "packs/data-layer", ruleIds: [`data-layer-${record.check.replaceAll("_", "-")}`],
      snapshot: { format: "rerail.repository-snapshot/v1", revision: 0, files: files(record.states[0]!.files) } });
    for (const rule of compiled) workspace.addCodeRule(rule);
    let previous = new Set<string>();
    let previousSources = new Map<string, string>();
    for (const [index, state] of record.states.entries()) {
      const current = new Set(state.files.map(file => file.path));
      const changed = state.files.filter(file => previousSources.get(file.path) !== file.source);
      await workspace.applyAsync(JSON.stringify({ revision: index + 1, files: files(changed), removed: [...previous].filter(path => !current.has(path)), quiet: true }));
      const rows: Row[] = [], failures: unknown[] = [];
      for (let cursor = 0; ;) {
        const page = JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES));
        rows.push(...page.violations.map((row: { path: string; code: string; message: string; span: { startLine: number } }) => ({ path: row.path, line: row.span.startLine + 1, code: row.code, message: row.message })));
        failures.push(...page.failures);
        if (page.done) break;
        if (page.cursor <= cursor) throw new Error("Result cursor did not advance");
        cursor = page.cursor;
      }
      expect(failures, state.name).toEqual([]);
      expect(canonical(rows), state.name).toEqual(canonical(state.expected));
      previous = current;
      previousSources = new Map(state.files.map(file => [file.path, file.source]));
    }
  } finally { workspace.close(); await rm(root, { recursive: true, force: true }); }
}, 90_000);
it("retains all ten production programs and every lifecycle state", () => {
  expect(oracle.cases).toHaveLength(10);
  for (const record of oracle.cases) {
    expect(record.states.map((s: State) => s.name)).toEqual(["clean", "positive", "restored", "removed", "readded"]);
    expect(record.states[0].expected).toEqual([]); expect(record.states[1].expected.length).toBeGreaterThan(0);
  }
});
