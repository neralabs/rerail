import { createRequire } from "node:module";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
const require = createRequire(import.meta.url);
const P = "/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together";
const native = require(process.env.P_ADDON);
const caps = readFileSync(`${P}/native/src/code_rule_dsl/capabilities.json`, "utf8");
const [corpus, out] = process.argv.slice(2);
const pack = "/Users/jeremy/orca/workspaces/rerail/wolfeel/packs/python";
const policies = [...readdirSync(`${pack}/rules`).map((f) => `${pack}/rules/${f}`), ...readdirSync(`${pack}/candidates/rules`).map((f) => `${pack}/candidates/rules/${f}`)].filter((f) => f.endsWith(".json")).sort();
const ws = new native.CodeRuleDslWorkspace();
const accepted = [];
for (const path of policies) {
  try { ws.addCodeRule(native.CodeRuleDslRuntime.compile(readFileSync(path, "utf8"), "json", caps)); accepted.push(path); }
  catch (e) { /* P cannot compile it */ }
}
const files = readdirSync(corpus).sort().map((name) => ({ path: name, language: "python", source: readFileSync(join(corpus, name), "utf8") }));
const t = process.cpuUsage();
const result = JSON.parse(await ws.applyAsync(JSON.stringify({ revision: 1, files, removed: [] })));
const used = process.cpuUsage(t);
ws.close();
writeFileSync(out, accepted.join("\n") + "\n");
console.log(`P: ${accepted.length}/${policies.length} policies, ${files.length} files, ${(result.evaluation?.violations ?? []).length} violations, cpu ${((used.user + used.system) / 1e6).toFixed(2)} s`);
