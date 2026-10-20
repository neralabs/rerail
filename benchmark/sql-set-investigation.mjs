// Alternating, fresh-process comparisons. Configuration and raw results stay outside Git.
// Usage: node benchmark/sql-set-investigation.mjs /absolute/path/config.json
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { cpus, loadavg } from "node:os";
import { resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(import.meta.url);
if (process.argv[2] === undefined) {
  throw new Error("Usage: node benchmark/sql-set-investigation.mjs /absolute/path/config.json");
}
const configPath = resolve(process.argv[2]);
const config = JSON.parse(readFileSync(configPath, "utf8"));
const hash = value => createHash("sha256").update(value).digest("hex");
const canonical = value => {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])]));
  }
  return value;
};
const sorted = rows => rows.map(row => JSON.stringify(canonical(row))).sort();
const cpuMs = before => {
  const used = process.cpuUsage(before);
  return (used.user + used.system) / 1000;
};

if (process.argv[3] === "--child") {
  const engine = process.argv[4];
  const binding = createRequire(import.meta.url)(resolve(config.addon));
  const kernel = binding.health();
  if (!kernel.kernelVersion.endsWith("+unbounded-work")) {
    throw new Error(`Benchmark requires unbounded-work, addon reports ${kernel.kernelVersion}`);
  }
  const workerThreads = JSON.parse(binding.nativeMemoryProfile()).workerPool.threads;
  if (workerThreads !== (config.threads ?? 4)) {
    throw new Error(`Addon has ${workerThreads} workers, configuration requires ${config.threads ?? 4}`);
  }
  const capabilities = readFileSync(config.capabilities, "utf8");
  const beforeCompile = process.cpuUsage();
  const compileStarted = performance.now();
  const policies = config.policies.map(path => {
    const text = readFileSync(path, "utf8");
    return binding.CodeRuleDslRuntime.compile(text, "json", capabilities);
  });
  const compile = { wallMs: performance.now() - compileStarted, cpuMs: cpuMs(beforeCompile) };
  const beforeSetup = process.cpuUsage();
  const setupStarted = performance.now();
  const workspace = new binding.CodeRuleDslWorkspace({ engine, sqlBatchSize: config.batchSize ?? 64 });
  for (const policy of policies) workspace.addCodeRule(policy);
  if (typeof workspace.engineReport !== "function") {
    throw new Error("Addon cannot report its execution engine. Refusing an unverified engine selection.");
  }
  const initialReport = JSON.parse(workspace.engineReport());
  if (initialReport.engine !== engine) {
    throw new Error(`Requested ${engine}, addon reports ${initialReport.engine}`);
  }
  const setup = { wallMs: performance.now() - setupStarted, cpuMs: cpuMs(beforeSetup) };
  const files = config.files.map(path => {
    const absolute = resolve(config.corpus, path);
    const source = readFileSync(absolute, "utf8");
    const language = config.language ?? "python";
    const module = binding.modulePath(language, absolute);
    return { path, language, source, ...(module == null ? {} : { module }) };
  }).filter(file => workspace.acceptsPath(file.path));
  const request = JSON.stringify({ revision: 1, files, removed: [] });
  const load = loadavg();
  const beforeApply = process.cpuUsage();
  const applyStarted = performance.now();
  const serialized = await workspace.applyAsync(request);
  const apply = { wallMs: performance.now() - applyStarted, cpuMs: cpuMs(beforeApply) };
  const response = JSON.parse(serialized);
  const evaluation = response.evaluation;
  const outcomes = {
    status: evaluation.status,
    violations: sorted(evaluation.violations),
    failures: sorted(evaluation.failures),
    blindFiles: sorted(response.blindFiles ?? []),
  };
  const report = JSON.parse(workspace.engineReport());
  const result = {
    engine, workerThreads, load, compile, setup, apply,
    files: files.length,
    bytes: files.reduce((total, file) => total + Buffer.byteLength(file.source), 0),
    kernel,
    digest: hash(JSON.stringify(outcomes)),
    counts: { violations: outcomes.violations.length, failures: outcomes.failures.length, blindFiles: outcomes.blindFiles.length },
    failureCodes: evaluation.failures.reduce((counts, failure) => {
      counts[failure.code] = (counts[failure.code] ?? 0) + 1;
      return counts;
    }, {}),
    metrics: evaluation.metrics,
    maxRssBytes: process.resourceUsage().maxRSS * 1024,
    report,
  };
  writeFileSync(process.argv[5], JSON.stringify(outcomes, null, 2) + "\n");
  workspace.close();
  process.stdout.write(JSON.stringify(result));
} else {
  const runs = [];
  const median = values => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
  const fingerprint = {
    config,
    runnerSha256: hash(readFileSync(script)),
    machine: { cpu: cpus()[0]?.model, logicalCpus: cpus().length, node: process.version },
    addonSha256: hash(readFileSync(config.addon)),
    capabilitiesSha256: hash(readFileSync(config.capabilities)),
    policies: config.policies.map(path => ({ path, sha256: hash(readFileSync(path)), clauses: JSON.parse(readFileSync(path, "utf8")).clauses.length })),
    corpusSha256: hash(config.files.map(path => `${path}\0${hash(readFileSync(resolve(config.corpus, path)))}`).join("\n")),
  };
  for (let round = 0; round < (config.rounds ?? 3); round++) {
    for (const engine of round % 2 === 0 ? ["native", "sql"] : ["sql", "native"]) {
      const child = spawnSync(process.execPath, [script, configPath, "--child", engine, `${config.output}.${round}.${engine}.outcomes.json`], {
        encoding: "utf8", maxBuffer: 32 * 1024 * 1024, timeout: config.timeoutMs ?? 300_000,
        env: { ...process.env, RAYON_NUM_THREADS: String(config.threads ?? 4) },
      });
      const run = child.status === 0
        ? { round, ...JSON.parse(child.stdout) }
        : { round, engine, error: child.error?.message ?? child.stderr, exitCode: child.status };
      runs.push(run);
      console.error(JSON.stringify({ round, engine, apply: run.apply, counts: run.counts, error: run.error, digest: run.digest }));
      writeFileSync(config.output, JSON.stringify({
        ...fingerprint,
        runs,
      }, null, 2) + "\n");
    }
  }
  const successful = runs.filter(run => run.error === undefined);
  const medians = Object.fromEntries(["native", "sql"].map(engine => {
    const selected = successful.filter(run => run.engine === engine);
    return [engine, selected.length === 0 ? null : {
      applyWallMs: median(selected.map(run => run.apply.wallMs)),
      applyCpuMs: median(selected.map(run => run.apply.cpuMs)),
      totalCpuMs: median(selected.map(run => run.compile.cpuMs + run.setup.cpuMs + run.apply.cpuMs)),
    }];
  }));
  const parity = successful.length === runs.length && new Set(successful.map(run => run.digest)).size === 1;
  const summary = {
    parity, medians,
    applyCpuRatio: medians.sql && medians.native ? medians.sql.applyCpuMs / medians.native.applyCpuMs : null,
    applyWallRatio: medians.sql && medians.native ? medians.sql.applyWallMs / medians.native.applyWallMs : null,
    totalCpuRatio: medians.sql && medians.native ? medians.sql.totalCpuMs / medians.native.totalCpuMs : null,
  };
  writeFileSync(config.output, JSON.stringify({ ...fingerprint, runs, summary }, null, 2) + "\n");
  console.log(JSON.stringify(summary, null, 2));
  if (!parity) process.exitCode = 1;
}
