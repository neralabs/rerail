// Converts the Tempo data-layer ports (nera-landing `tempo-oracles/data-layer/<linter>/`) to Rerail's current code-rule
// contract, beside the oracles they are checked against. `just rerail-bench-setup` (setup.sh) runs it; by hand:
//
//   node --import tsx benchmark/data-layer/setup/convert.mts \
//     --rerail <rerail worktree> --addon <rerail_engine.node> \
//     --nera-landing <nera-landing clone> --ref origin/Moti-Velz/tempo-data-layer-benchmark \
//     --out "${RERAIL_BENCH_HOME:-$HOME/rerail-bench}/data-layer/policies"
//
// For each linter: the oracle files (`manifest.json`, `tree.expected.json`, `unsupported.json`, `mutations/`) are copied
// verbatim; `policy.json` is read without duplicate keys and checked exactly as Rerail checks a rule it stores
// (`validateCodeRule`: the strict schema, then the engine compile, with the addon given). The stored form Rerail returns
// is what is written, so a construct the current contract renamed or dropped fails here with the schema's or engine's
// reason instead of loading silently. A rule that does not convert is not written; it and its reason go to
// `conversion-report.json` beside `--out`.
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";

const args = new Map<string, string>();
for (let index = 2; index < process.argv.length; index += 2) args.set(process.argv[index]!.replace(/^--/, ""), process.argv[index + 1]!);
const required = (name: string) => {
  const value = args.get(name);
  if (value === undefined) throw new Error(`--${name} is required`);
  return resolve(value.replace(/^~(?=\/)/, process.env.HOME ?? "~"));
};
const rerail = required("rerail");
const addon = required("addon");
const neraLanding = required("nera-landing");
const ref = args.get("ref") ?? "origin/Moti-Velz/tempo-data-layer-benchmark";
const out = required("out");
process.env.RERAIL_ENGINE_PATH = addon;

const { validateCodeRule } = await import(resolve(rerail, "src/engine/rules.ts"));
const { loadNativeBinding, parseJsonWithoutDuplicateKeys, stableJsonText } = await import(resolve(rerail, "src/engine/index.ts"));

function git(...argv: string[]): Buffer {
  const result = spawnSync("git", ["-C", neraLanding, ...argv], { maxBuffer: 1024 * 1024 * 1024 });
  if (result.status !== 0) throw new Error(`git ${argv.join(" ")} failed: ${result.stderr.toString().trim()}`);
  return result.stdout;
}

const commit = git("rev-parse", `${ref}^{commit}`).toString().trim();
const staging = mkdtempSync(resolve(tmpdir(), "rerail-data-layer-convert-"));
try {
  const archive = resolve(staging, "oracles.tar");
  writeFileSync(archive, git("archive", "--format=tar", commit, "tempo-oracles/data-layer"));
  const untar = spawnSync("tar", ["-xf", archive, "-C", staging]);
  if (untar.status !== 0) throw new Error(`tar failed: ${untar.stderr.toString()}`);
  const source = resolve(staging, "tempo-oracles/data-layer");
  const linters = readdirSync(source, { withFileTypes: true }).filter(entry => entry.isDirectory()).map(entry => entry.name).sort();
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
  // The oracles' recorder (it runs the data-layer linters themselves), kept for provenance.
  cpSync(resolve(source, "generate.py"), resolve(out, "generate.py"));
  const report: Array<Record<string, unknown>> = [];
  const capabilities = readFileSync(resolve(rerail, "native/src/code_rule_dsl/capabilities.json"), "utf8");
  for (const linter of linters) {
    const from = resolve(source, linter);
    const to = resolve(out, linter);
    mkdirSync(to, { recursive: true });
    for (const name of readdirSync(from)) if (name !== "policy.json") cpSync(resolve(from, name), resolve(to, name), { recursive: true });
    if (!existsSync(resolve(from, "policy.json"))) {
      report.push({ linter, status: "no-policy" });
      continue;
    }
    const text = readFileSync(resolve(from, "policy.json"), "utf8");
    let document: unknown;
    try {
      document = parseJsonWithoutDuplicateKeys(text);
    } catch (error) {
      report.push({ linter, status: "failed", stage: "parse", reason: String(error) });
      continue;
    }
    const validation = validateCodeRule(document);
    if (!validation.ok) {
      report.push({ linter, status: "failed", stage: validation.issues === undefined ? "compile" : "schema", reason: validation.reason });
      continue;
    }
    if (validation.inactive !== undefined) {
      report.push({ linter, status: "inactive", reason: validation.inactive.reason });
      continue;
    }
    const stored = `${JSON.stringify(validation.rule, null, 2)}\n`;
    // The written document compiles on its own too: what the benchmark loads is what was checked.
    loadNativeBinding().CodeRuleDslRuntime.compile(stored, "json", capabilities);
    writeFileSync(resolve(to, "policy.json"), stored);
    // `validation.rule` is a `CodeRule`, the schema's parsed output: an id and clauses.
    const clauses = validation.rule.clauses.length;
    report.push({
      linter,
      status: "converted",
      id: validation.rule.id,
      clauses,
      // Whether Rerail's stored form differs from the Tempo document beyond key order and whitespace.
      normalized: stableJsonText(validation.rule) !== stableJsonText(JSON.parse(text)),
    });
  }
  const summary = {
    format: "rerail.data-layer-conversion/v1",
    source: { repository: neraLanding, ref, commit, path: "tempo-oracles/data-layer" },
    rerail: { worktree: rerail, commit: spawnSync("git", ["-C", rerail, "rev-parse", "HEAD"]).stdout.toString().trim(), addon, kernelVersion: loadNativeBinding().health().kernelVersion },
    converted: report.filter(entry => entry.status === "converted").length,
    failed: report.filter(entry => entry.status !== "converted").length,
    linters: report,
  };
  writeFileSync(resolve(dirname(out), "conversion-report.json"), `${JSON.stringify(summary, null, 2)}\n`);
  for (const entry of report) console.log(`${entry.linter}: ${entry.status}${entry.reason === undefined ? "" : ` — ${entry.reason}`}${entry.normalized === true ? " (normalized)" : ""}`);
  console.log(`${summary.converted} converted, ${summary.failed} not; report ${resolve(dirname(out), "conversion-report.json")}`);
  process.exitCode = summary.failed > 0 ? 1 : 0;
} finally {
  rmSync(staging, { recursive: true, force: true });
}
