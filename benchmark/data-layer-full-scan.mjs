#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, realpathSync, writeFileSync } from "node:fs";
import { cpus, release, totalmem } from "node:os";
import { dirname, join, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const expected = {
  dataLayerCommit: "fce4dc7208c0b892c11a549c8be43fc285580d6b",
  policies: 12,
  clauses: 829,
  files: 5_725,
  sourceBytes: 95_697_853,
};

function usage() {
  return `Usage: npm run benchmark:data-layer-full-scan -- \\
  --repository <nera-data-layer checkout> \\
  --policies <archived data-layer policy directory> [options]

Options:
  --native <addon>  no-file-deadline addon (default: native/target/no-file-deadline)
  --output <file>   also write the result JSON to this file

Build the required addon first:
  npm run build:native:no-file-deadline

This benchmark requires the historical 12-policy, 829-clause catalog and the pinned
5,725-file data-layer checkout. It refuses production-budget and substitute inputs.`;
}

function options(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!["--repository", "--policies", "--native", "--output"].includes(key) || value === undefined) {
      throw new Error(`${usage()}\n\nUnknown or incomplete argument: ${key ?? "(none)"}`);
    }
    if (values.has(key)) throw new Error(`Duplicate argument: ${key}`);
    values.set(key, value);
  }
  if (!values.has("--repository") || !values.has("--policies")) throw new Error(usage());
  const suffix = `${process.platform}-${process.arch}`;
  return {
    repository: realpathSync(resolve(values.get("--repository"))),
    policies: realpathSync(resolve(values.get("--policies"))),
    native: realpathSync(resolve(values.get("--native") ?? join(root, "native", "target", "no-file-deadline", `rerail_engine.${suffix}.node`))),
    output: values.has("--output") ? resolve(values.get("--output")) : undefined,
  };
}

function git(repository, ...args) {
  const result = spawnSync("/usr/bin/git", ["-C", repository, ...args], {
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
  });
  if (result.status !== 0) throw new Error(`/usr/bin/git -C ${repository} ${args.join(" ")} failed: ${result.stderr.trim()}`);
  return result.stdout;
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function measured(before, after) {
  return {
    userMs: (after.userCPUTime - before.userCPUTime) / 1_000,
    systemMs: (after.systemCPUTime - before.systemCPUTime) / 1_000,
  };
}

const input = options(process.argv.slice(2));
const require = createRequire(import.meta.url);
const binding = require(input.native);
const kernelVersion = binding.health().kernelVersion;
if (!kernelVersion.endsWith("+no-file-deadline")) {
  throw new Error(`Refusing ${input.native}: kernel ${kernelVersion} enforces the production file budget`);
}

const dataLayerCommit = git(input.repository, "rev-parse", "HEAD").trim();
if (dataLayerCommit !== expected.dataLayerCommit) {
  throw new Error(`Data-layer commit is ${dataLayerCommit}; expected ${expected.dataLayerCommit}`);
}
const dataLayerChanges = git(input.repository, "status", "--porcelain", "--untracked-files=no")
  .split("\n")
  .filter(Boolean);
if (dataLayerChanges.length !== 0) throw new Error(`Data-layer checkout is dirty:\n${dataLayerChanges.join("\n")}`);

const capabilitiesPath = join(root, "native", "src", "code_rule_dsl", "capabilities.json");
const capabilities = readFileSync(capabilitiesPath, "utf8");
const policyFiles = readdirSync(input.policies, { withFileTypes: true })
  .filter(entry => entry.isDirectory() && existsSync(join(input.policies, entry.name, "policy.json")))
  .map(entry => ({ name: entry.name, path: join(input.policies, entry.name, "policy.json") }))
  .sort((left, right) => left.name.localeCompare(right.name));

const compileBefore = process.resourceUsage();
const compileStarted = performance.now();
let clauses = 0;
const policies = policyFiles.map(policy => {
  const source = readFileSync(policy.path, "utf8");
  const document = JSON.parse(source);
  clauses += document.clauses.length;
  const compiled = binding.CodeRuleDslRuntime.compile(source, "json", capabilities);
  return {
    ...policy,
    sourceSha256: sha256(source),
    clauses: document.clauses.length,
    digest: compiled.digest(),
    compiled,
  };
});
const compileWallMs = performance.now() - compileStarted;
const compileCpu = measured(compileBefore, process.resourceUsage());
if (policies.length !== expected.policies || clauses !== expected.clauses) {
  throw new Error(`Policy catalog has ${policies.length} policies and ${clauses} clauses; expected ${expected.policies} and ${expected.clauses}`);
}

const labels = git(input.repository, "ls-files", "-z", "--", "*.py").split("\0").filter(Boolean).sort();
let sourceBytes = 0;
const files = labels.map(label => {
  const absolute = join(input.repository, label);
  const source = readFileSync(absolute, "utf8");
  sourceBytes += Buffer.byteLength(source);
  const module = binding.modulePath("python", absolute);
  return module === null ? { path: label, language: "python", source } : { path: label, language: "python", source, module };
});
if (files.length !== expected.files || sourceBytes !== expected.sourceBytes) {
  throw new Error(`Corpus has ${files.length} files and ${sourceBytes} bytes; expected ${expected.files} and ${expected.sourceBytes}`);
}

const workspace = new binding.CodeRuleDslWorkspace();
for (const policy of policies) workspace.addCodeRule(policy.compiled);
const accepted = files.filter(file => workspace.acceptsPath(file.path));
if (accepted.length !== expected.files) {
  workspace.close();
  throw new Error(`Policies accept ${accepted.length} files; expected ${expected.files}`);
}

const request = JSON.stringify({ revision: 1, files: accepted, removed: [] });
const rssBefore = process.memoryUsage().rss;
const applyBefore = process.resourceUsage();
const applyStarted = performance.now();
const serialized = await workspace.applyAsync(request);
const applyWallMs = performance.now() - applyStarted;
const applyAfter = process.resourceUsage();
const rssAfter = process.memoryUsage().rss;
const memoryProfile = JSON.parse(workspace.memoryProfile());
workspace.close();

const result = JSON.parse(serialized).evaluation;
const failuresByCode = {};
for (const failure of result.failures) failuresByCode[failure.code] = (failuresByCode[failure.code] ?? 0) + 1;
const output = {
  benchmark: "rerail-data-layer-full-scan-v1",
  inputs: {
    dataLayerCommit,
    policies: policies.map(({ name, path, sourceSha256, clauses, digest }) => ({ name, path, sourceSha256, clauses, digest })),
    capabilities: { path: capabilitiesPath, sha256: sha256(capabilities) },
    totals: { policies: policies.length, clauses, files: accepted.length, sourceBytes, requestBytes: Buffer.byteLength(request) },
  },
  engine: {
    addon: input.native,
    addonSha256: sha256(readFileSync(input.native)),
    kernelVersion,
    executionEngine: "sqlite",
    rayonThreads: process.env.RAYON_NUM_THREADS ?? null,
    memoryProfile,
  },
  machine: {
    cpu: cpus()[0]?.model ?? "unknown",
    logicalCpus: cpus().length,
    memoryBytes: totalmem(),
    platform: process.platform,
    arch: process.arch,
    osRelease: release(),
    node: process.version,
  },
  compile: { wallMs: compileWallMs, ...compileCpu },
  apply: {
    wallMs: applyWallMs,
    ...measured(applyBefore, applyAfter),
    rssBefore,
    rssAfter,
    maxRssBytes: applyAfter.maxRSS * 1_024,
  },
  result: {
    sha256: sha256(serialized),
    status: result.status,
    violations: result.violations.length,
    failures: result.failures.length,
    failuresByCode,
    metrics: result.metrics,
    ...(result.profile === undefined ? {} : { profile: result.profile }),
  },
};
const json = `${JSON.stringify(output, null, 2)}\n`;
if (input.output !== undefined) writeFileSync(input.output, json);
process.stdout.write(json);
