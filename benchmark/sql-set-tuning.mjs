// Interleaved parameter sweeps with an explicit output oracle. Raw artifacts stay outside Git.
// Usage: node benchmark/sql-set-tuning.mjs /absolute/path/sweep.json
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { cpus, loadavg } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const definition = JSON.parse(readFileSync(resolve(process.argv[2]), "utf8"));
const base = JSON.parse(readFileSync(definition.baseConfig, "utf8"));
const runner = fileURLToPath(new URL("./sql-set-investigation.mjs", import.meta.url));
const hash = value => createHash("sha256").update(value).digest("hex");
const median = values => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
mkdirSync(dirname(definition.output), { recursive: true });
const records = [];
const metadata = {
  definition, base,
  machine: { cpu: cpus()[0]?.model, logicalCpus: cpus().length, node: process.version },
  runnerSha256: hash(readFileSync(runner)),
  capabilitiesSha256: hash(readFileSync(base.capabilities)),
  sourceHashes: ["emit.rs", "emit/keyed.rs", "emit/guards.rs", "emit/fusion.rs", "emit/shared_search.rs", "emit/inline.rs", "emit/three.rs", "exec.rs", "frames.rs", "diagnostics.rs", "terminals/pattern.rs", "../../lib.rs"].map(name => {
    const path = fileURLToPath(new URL(`../native/src/code_rule_dsl/sql/${name}`, import.meta.url));
    return { path, sha256: hash(readFileSync(path)) };
  }),
  policyHashes: base.policies.map(path => ({ path, sha256: hash(readFileSync(path)) })),
  corpusSha256: hash(base.files.map(path => `${path}\0${hash(readFileSync(resolve(base.corpus, path)))}`).join("\n")),
};
let oracle = definition.expectedDigest;
for (let round = 0; round < (definition.rounds ?? 1); round++) {
  const variants = round % 2 ? [...definition.variants].reverse() : definition.variants;
  for (const variant of variants) {
    const stem = `${definition.output}.${round}.${variant.name}`;
    const config = { ...base, ...variant, output: stem };
    writeFileSync(`${stem}.config.json`, JSON.stringify(config, null, 2) + "\n");
    const environment = { ...process.env };
    for (const key of Object.keys(environment)) {
      if (key.startsWith("RERAIL_SQL_") || key === "RERAIL_BENCH_WORKERS") delete environment[key];
    }
    Object.assign(environment, variant.environment ?? {});
    const startedLoad = loadavg();
    const child = spawnSync(process.execPath, [runner, `${stem}.config.json`, "--child", variant.engine ?? "sql", `${stem}.outcomes.json`], {
      env: environment, encoding: "utf8", maxBuffer: 32 * 1024 * 1024,
      timeout: variant.timeoutMs ?? definition.timeoutMs ?? 180_000,
    });
    let result;
    if (child.status !== 0) {
      result = { error: child.error?.message ?? child.stderr, exitCode: child.status };
    } else {
      result = JSON.parse(child.stdout);
      if (oracle === undefined && result.engine === "native") oracle = result.digest;
      result.parity = oracle !== undefined && result.digest === oracle && result.counts.failures === 0;
    }
    records.push({ round, variant, startedLoad, addonSha256: hash(readFileSync(config.addon)),
      policyHashes: config.policies.map(path => ({ path, sha256: hash(readFileSync(path)) })), ...result });
    writeFileSync(definition.output, JSON.stringify({ ...metadata, oracle, records }, null, 2) + "\n");
    console.error(JSON.stringify({ round, variant: variant.name, apply: result.apply, parity: result.parity, error: result.error }));
  }
}
const summary = definition.variants.map(variant => {
  const selected = records.filter(run => run.variant.name === variant.name);
  const good = selected.filter(run => run.error === undefined);
  return {
    name: variant.name, parity: selected.every(run => run.parity), runs: good.length,
    ...(good.length ? {
      cpuMs: median(good.map(run => run.apply.cpuMs)),
      wallMs: median(good.map(run => run.apply.wallMs)),
      totalCpuMs: median(good.map(run => run.compile.cpuMs + run.setup.cpuMs + run.apply.cpuMs)),
      maxRssBytes: Math.max(...good.map(run => run.maxRssBytes)),
    } : {}),
  };
});
writeFileSync(definition.output, JSON.stringify({ ...metadata, oracle, records, summary }, null, 2) + "\n");
console.log(JSON.stringify(summary, null, 2));
if (summary.some(run => !run.parity)) process.exitCode = 1;
