import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { resolveNativeBindingPath } from "../src/native.js";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const compareNative = process.env.RERAIL_BENCHMARK_TEST_COMPARE_NATIVE;
interface Identity { path: string; sha256: string }
interface Build extends Identity { loadedPath: string }
interface Sample { native: Identity; initial: { status: string }; edits: Array<{ status: string }> }
interface BaselineReport { build: Build; warmup: Sample; samples: Sample[]; sameSemanticResultsAcrossProcesses: boolean }
interface PairedReport {
  builds: { reference: Build; indexed: Build };
  warmups: { reference: Sample; indexed: Sample };
  pairs: Array<{ order: string[]; reference: Sample; indexed: Sample }>;
  sameSemanticResultsAcrossVariants: boolean;
}

function fingerprint(path: string): Identity {
  return { path: realpathSync(path), sha256: `sha256:${createHash("sha256").update(readFileSync(path)).digest("hex")}` };
}

// These hooks only perturb disposable inputs at deterministic process/load boundaries.
// Every child still runs the real CLI, loader, bundled catalog and native workspace.
const preloadSource = `
const fs = require("node:fs");
const cp = require("node:child_process");
const { syncBuiltinESMExports } = require("node:module");
const mode = process.env.PROVENANCE_TEST_MODE;
function replace(path, bytes) {
  fs.writeFileSync(path + ".replacement", bytes);
  fs.renameSync(path + ".replacement", path);
}
if (!process.argv.includes("--child")) {
  const spawn = cp.spawn;
  let count = 0;
  cp.spawn = function(command, args, options) {
    count += 1;
    const path = options.env.RERAIL_NATIVE_KERNEL_PATH;
    fs.appendFileSync(process.env.PROVENANCE_TEST_EVENTS, JSON.stringify({ path, count }) + "\\n");
    if (mode === "replace-original" && count === Number(process.env.PROVENANCE_TEST_REPLACE_AT)) {
      replace(process.env.PROVENANCE_TEST_ORIGINAL, Buffer.from("rebuilt original no longer contains this addon"));
    }
    if (mode === "tamper-snapshot" && count === 2) replace(path, Buffer.from("tampered private snapshot"));
    if (mode === "redirect-child" && count === 2) options.env.RERAIL_NATIVE_KERNEL_PATH = process.env.PROVENANCE_TEST_ORIGINAL;
    return spawn.call(this, command, args, options);
  };
  syncBuiltinESMExports();
} else if (mode === "tamper-after-load") {
  const dlopen = process.dlopen;
  process.dlopen = function(module, path, ...rest) {
    const result = dlopen.call(this, module, path, ...rest);
    replace(path, Buffer.from("replaced immediately after actual native load"));
    return result;
  };
}
`;

let root: string;
let source: string;
let snapshots: string;
let initialIdentity: Identity;
beforeEach(() => {
  root = mkdtempSync(resolve(tmpdir(), "rerail-benchmark-provenance-test-"));
  mkdirSync(resolve(root, "impl"));
  snapshots = resolve(root, "snapshots");
  mkdirSync(snapshots);
  writeFileSync(resolve(root, "impl/probe.py"), "def run(conn):\n    pass\n");
  writeFileSync(resolve(root, "manifest.json"), JSON.stringify(["impl/probe.py"]));
  writeFileSync(resolve(root, "preload.cjs"), preloadSource);
  source = resolve(root, "reference.node");
  copyFileSync(resolveNativeBindingPath(), source);
  initialIdentity = fingerprint(source);
});
afterEach(() => rmSync(root, { recursive: true, force: true }));

function cli(args: string[] = [], mode = "observe", replaceAt = 2, iterations = 1) {
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    RERAIL_NATIVE_KERNEL_PATH: source,
    TSX_DISABLE_CACHE: "1",
    TMPDIR: snapshots, TMP: snapshots, TEMP: snapshots,
    PROVENANCE_TEST_MODE: mode,
    PROVENANCE_TEST_EVENTS: resolve(root, "events.jsonl"),
    PROVENANCE_TEST_ORIGINAL: source,
    PROVENANCE_TEST_REPLACE_AT: String(replaceAt),
  };
  delete env.RERAIL_BENCHMARK_NATIVE_IDENTITY;
  return spawnSync(process.execPath, [
    "--require", resolve(root, "preload.cjs"), "--import", "tsx",
    resolve(packageRoot, "benchmark/code-rule-workspace.ts"),
    "--corpus", root, "--manifest", resolve(root, "manifest.json"),
    "--iterations", String(iterations), "--edits", "1", ...args,
  ], { cwd: packageRoot, env, encoding: "utf8", timeout: 60_000, maxBuffer: 32 * 1024 * 1024 });
}

function expectSnapshotSamples(build: Build, samples: Sample[], original: Identity) {
  expect({ path: build.path, sha256: build.sha256 }).toEqual(original);
  expect(build.loadedPath).not.toBe(build.path);
  for (const sample of samples) {
    expect(sample.native).toEqual({ path: build.loadedPath, sha256: original.sha256 });
    expect(sample.initial.status).toBe("completed");
    expect(sample.edits.map(edit => edit.status)).toEqual(["completed"]);
  }
  expect(existsSync(build.loadedPath)).toBe(false);
  expect(readdirSync(snapshots)).toEqual([]);
}

describe("workspace benchmark native provenance", () => {
  it.each(["same-path", "copied-bytes"])("rejects %s comparisons before starting children", kind => {
    const indexed = kind === "same-path" ? source : resolve(root, "indexed.node");
    if (indexed !== source) copyFileSync(source, indexed);
    const result = cli(["--compare-native", indexed]);
    expect(result.status, result.stderr).not.toBe(0);
    expect(result.stderr).toContain("requires different native addon bytes");
    expect(existsSync(resolve(root, "events.jsonl"))).toBe(false);
    expect(readdirSync(snapshots)).toEqual([]);
  });

  it("reports the actual canonical binary for a standalone child", () => {
    const result = cli(["--child"]);
    expect(result.status, result.stderr).toBe(0);
    const sample = JSON.parse(result.stdout) as Sample;
    expect(sample.native).toEqual(initialIdentity);
    expect(sample.initial.status).toBe("completed");
    expect(sample.edits.map(edit => edit.status)).toEqual(["completed"]);
  }, 60_000);

  it("keeps baseline warmup and measured samples pinned when the source is replaced", () => {
    const result = cli([], "replace-original");
    expect(result.status, result.stderr).toBe(0);
    const report = JSON.parse(result.stdout) as BaselineReport;
    expect(fingerprint(source).sha256).not.toBe(initialIdentity.sha256);
    expectSnapshotSamples(report.build, [report.warmup, ...report.samples], initialIdentity);
    expect(report.sameSemanticResultsAcrossProcesses).toBe(true);
  }, 60_000);

  it.each(["tamper-snapshot", "redirect-child"])("rejects %s between warmup and measurement and cleans snapshots", mode => {
    const result = cli([], mode);
    expect(result.status, result.stderr).not.toBe(0);
    expect(result.stderr).toContain("Native addon identity mismatch");
    expect(result.stdout).toBe("");
    const events = readFileSync(resolve(root, "events.jsonl"), "utf8").trim().split("\n").map(line => JSON.parse(line) as { path: string });
    expect(events).toHaveLength(2);
    for (const event of events) expect(existsSync(event.path)).toBe(false);
    expect(readdirSync(snapshots)).toEqual([]);
  }, 60_000);

  it.each([false, true])("rejects a binary replaced immediately after loading (standalone=%s)", standalone => {
    const result = cli(standalone ? ["--child"] : [], "tamper-after-load");
    expect(result.status, result.stderr).not.toBe(0);
    expect(result.stderr).toContain("Native addon identity mismatch");
    expect(result.stdout).toBe("");
    expect(readdirSync(snapshots)).toEqual([]);
  }, 60_000);

  // A second actual build is deliberately explicit, never fabricated by modifying addon bytes.
  it.runIf(compareNative !== undefined)("keeps both variants pinned across alternating pairs and an original rebuild", () => {
    const indexed = resolve(root, "indexed.node");
    copyFileSync(compareNative!, indexed);
    const indexedIdentity = fingerprint(indexed);
    expect(indexedIdentity.sha256).not.toBe(initialIdentity.sha256);
    const result = cli(["--compare-native", indexed], "replace-original", 5, 2);
    expect(result.status, result.stderr).toBe(0);
    const report = JSON.parse(result.stdout) as PairedReport;
    expect(fingerprint(source).sha256).not.toBe(initialIdentity.sha256);
    expectSnapshotSamples(report.builds.reference, [report.warmups.reference, ...report.pairs.map(pair => pair.reference)], initialIdentity);
    expectSnapshotSamples(report.builds.indexed, [report.warmups.indexed, ...report.pairs.map(pair => pair.indexed)], indexedIdentity);
    expect(report.pairs.map(pair => pair.order)).toEqual([["reference", "indexed"], ["indexed", "reference"]]);
    expect(report.sameSemanticResultsAcrossVariants).toBe(true);
  }, 60_000);
});
