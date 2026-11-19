// One timed run of the product's path for the shippability bar (scripts/ship/shipbar.ts): a checkout's shipped JSON
// rules of one pack, compiled together, over every file of one corpus directory in one production workspace
// (conformance/languages/runner.ts evaluateWorkspace: the repository audit's batching and paging). The checkout is
// chosen at run time, so main and a candidate run through their own engine and modules; this file imports nothing of
// either until then.
//
//   node --import tsx scripts/ship/product-run.ts <checkout> <corpus dir> <out.json> <rules dir>
//
// SHIPBAR_ENGINE_TMPDIR becomes TMPDIR (and TMP and TEMP, which Windows reads) before the engine loads, so the
// engine's temporary stores start in the run's fresh directory while tsx keeps its transpile cache where it was. Corpus
// paths are '/'-separated on every platform, as the recordings name them. Writes { files, rules, compiled,
// compileErrors, findings: [rule, path, line, code, column][], failures: { rule, code, message }[], loadMs, compileMs,
// evaluateMs }; line and column (bytes into the line) are 1-based, 0 for a finding without a span. The translation suite
// (scripts/ship/translations.ts) runs it too, as the original side of each rule's comparison.

import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { pathToFileURL } from "node:url";

interface Violation {
  ruleId: string;
  path?: string;
  span?: { startLine: number; startColumn: number };
  code: string;
}

interface Failure {
  ruleId?: string;
  code: string;
  message: string;
}

const [checkout, corpus, out, rulesDirectory] = process.argv.slice(2);
if (checkout === undefined || corpus === undefined || out === undefined || rulesDirectory === undefined) {
  console.error("usage: product-run.ts <checkout> <corpus dir> <out.json> <rules dir>");
  process.exit(2);
}
const engineTmp = process.env.SHIPBAR_ENGINE_TMPDIR;
if (engineTmp !== undefined) for (const key of process.platform === "win32" ? ["TMPDIR", "TMP", "TEMP"] : ["TMPDIR"]) process.env[key] = engineTmp;

// Each phase is announced on stderr, so a run that dies says where (shipbar.ts reads the last one).
console.error("shipbar: phase load");
const started = performance.now();
const engine = await import(pathToFileURL(join(checkout, "src/engine/index.ts")).href);
const { capabilitiesJson } = await import(pathToFileURL(join(checkout, "conformance/engine.ts")).href);
const { evaluateWorkspace } = await import(pathToFileURL(join(checkout, "conformance/languages/runner.ts")).href);
const loadMs = performance.now() - started;

console.error("shipbar: phase read");
const files: { path: string; language: string; source: string }[] = [];
const walk = (directory: string): void => {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const full = join(directory, entry.name);
    if (entry.isDirectory()) walk(full);
    else if (entry.isFile()) {
      const path = relative(corpus, full).split(sep).join("/");
      const language = engine.languageFor(path);
      if (language !== undefined) files.push({ path, language, source: readFileSync(full, "utf8") });
    }
  }
};
walk(corpus);
files.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));

console.error("shipbar: phase compile");
const compileStarted = performance.now();
const runtime = engine.loadNativeBinding().CodeRuleDslRuntime;
const documents = readdirSync(rulesDirectory).filter((name) => name.endsWith(".json")).sort();
const policyToRule = new Map<string, string>();
const compiled = [];
const compileErrors: { rule: string; error: string }[] = [];
for (const name of documents) {
  const text = readFileSync(join(rulesDirectory, name), "utf8");
  const rule = name.slice(0, -".json".length);
  try {
    compiled.push(runtime.compile(text, "json", capabilitiesJson()));
    policyToRule.set(JSON.parse(text).id, rule);
  } catch (error) {
    compileErrors.push({ rule, error: (error instanceof Error ? error.message : String(error)).slice(0, 300) });
  }
}
const compileMs = performance.now() - compileStarted;

console.error("shipbar: phase evaluate");
const evaluation = await evaluateWorkspace(compiled, files, corpus);
console.error("shipbar: phase write");
const findings = (evaluation.violations as Violation[]).map((v) => [policyToRule.get(v.ruleId) ?? v.ruleId, v.path ?? "", (v.span?.startLine ?? -1) + 1, v.code, (v.span?.startColumn ?? -1) + 1]);
const failures = (evaluation.failures as Failure[]).map((f) => ({
  rule: (f.ruleId === undefined ? undefined : policyToRule.get(f.ruleId)) ?? f.ruleId ?? "",
  code: f.code,
  message: f.message.slice(0, 300),
}));
writeFileSync(
  out,
  JSON.stringify({
    files: files.length,
    rules: documents.length,
    compiled: compiled.length,
    compileErrors,
    loadMs: Math.round(loadMs),
    compileMs: Math.round(compileMs),
    evaluateMs: Math.round(evaluation.ms),
    findings,
    failures,
  }),
);
