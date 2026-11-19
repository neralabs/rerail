// Run selected data-layer rules on the native engine (cold workspace) over a corpus directory; dump finding keys.
import { readFileSync, writeFileSync } from "node:fs";
import { encodeCodeRuleWorkspaceRequest, loadNativeBinding, RESULT_PAGE_BYTES } from "/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/src/engine/index.js";
import { capabilitiesJson } from "/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/conformance/engine.js";
const [corpus, list, out, ...rules] = process.argv.slice(2);
const paths = readFileSync(list, "utf8").split("\n").filter(Boolean);
const files = paths.map(p => ({ path: p, source: readFileSync(`${corpus}/${p}`, "utf8"), language: "python" }));
const native = loadNativeBinding();
const result: any = { rules: {} };
for (const rule of rules) {
  const ws = new native.CodeRuleDslWorkspace();
  try {
    const c0 = performance.now();
    const compiled = native.CodeRuleDslRuntime.compile(readFileSync(`/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/packs/data-layer/candidates/rules/${rule}.json`, "utf8"), "json", capabilitiesJson());
    ws.addCodeRule(compiled);
    const compileMs = performance.now() - c0;
    const cpu = process.cpuUsage(), t0 = performance.now();
    let rev = 0, batch: typeof files = [], bytes = 0;
    const send = async (b: typeof files, deferGlobal: boolean) => { await ws.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: ++rev, files: b, removed: [], quiet: true, deferGlobal })); };
    for (const f of files) { batch.push(f); bytes += f.source.length; if (batch.length >= 32 || bytes >= 1 << 20) { await send(batch, true); batch = []; bytes = 0; } }
    if (batch.length) await send(batch, true);
    await send([], false);
    const violations: any[] = [], failures: any[] = [];
    for (let cursor = 0; ;) {
      const page = JSON.parse(await ws.resultPageAsync(cursor, RESULT_PAGE_BYTES));
      violations.push(...page.violations); failures.push(...page.failures);
      if (page.done) break; cursor = page.cursor;
    }
    const wallMs = performance.now() - t0, c = process.cpuUsage(cpu);
    const keys = [...new Set(violations.map(v => JSON.stringify([v.ruleId, v.path, v.span.startLine + 1, v.code])))].sort();
    result.rules[rule] = { compileMs, wallMs, cpuMs: (c.user + c.system) / 1000, violations: violations.length, keys, failures: failures.slice(0, 50), nFailures: failures.length, peakRssBytes: process.resourceUsage().maxRSS * 1024 };
    console.error(rule, JSON.stringify({ compileMs, wallMs, keys: keys.length, failures: failures.length }));
  } finally { ws.close(); }
}
writeFileSync(out, JSON.stringify(result, null, 1));
