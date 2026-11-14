import { readFileSync, writeFileSync } from "node:fs";
import { loadNativeBinding } from "/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/src/engine/index.js";
import { capabilitiesJson } from "/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/conformance/engine.js";
const M = "/private/tmp/claude-501/-Users-jeremy-orca-workspaces-rerail-Pulling-it-all-together/3adb237a-22e7-4e0c-9a7a-33e109d2ab63/scratchpad/m2m7";
const idx = JSON.parse(readFileSync(M + "/fx/index.json", "utf8"));
const native = loadNativeBinding();
const out: any[] = [];
for (const e of idx) {
  const pol = readFileSync(`${M}/fx/${e.i}.policy.json`, "utf8");
  const rule = native.CodeRuleDslRuntime.compile(pol, "json", capabilitiesJson());
  const paths = readFileSync(`${M}/fx/${e.i}.files`, "utf8").split("\n").filter(Boolean);
  const files = paths.map(p => ({ path: p, source: readFileSync(`${M}/fx/${e.i}/${p}`, "utf8"), language: "python" }));
  const input = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files });
  const r = JSON.parse(rule.evaluate(input, "report"));
  const id = JSON.parse(pol).id;
  out.push({ i: e.i, keys: [...new Set(r.violations.map((v: any) => JSON.stringify([id, v.path, v.span.startLine + 1, v.code])))].sort(), failures: r.failures });
}
writeFileSync(M + "/fx/native.json", JSON.stringify(out));
console.log("done", out.length);
