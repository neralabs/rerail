/**
 * The facts engine behind the data-layer harness (`replay.ts`, `suite.ts`, `incremental.ts` with `--engine facts`): a CLI
 * bridge to the facts binary (`FACTS_ENGINE_BIN`, default `target/release/facts-a`). The harness resolves every call's
 * policy exactly as it does for the native engines; the binary evaluates it and returns findings with the keys the harness
 * compares (check, clause, path, line, code). A policy the engine cannot compile, or a construct it refuses, comes back as
 * a failure of that call; nothing falls back to the native engine.
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { z } from "zod";
import type { CodeRuleDslEngine } from "../../src/engine/index.js";

export type HarnessEngine = CodeRuleDslEngine | "facts";

export interface FactsCall { id: number; policy: unknown; files: { path: string; source: string }[] }

/** A call's result in the shape of the native engine's report pages; spans carry the line only. */
export interface FactsResult {
  violations: { code: string; path: string; message: string; clauseId: string; span: { startByte: number; startLine: number; startColumn: number } }[];
  failures: { code: string; message: string; path?: string }[];
}

const findingSchema = z.object({ check: z.string(), clause: z.string(), path: z.string(), line: z.number(), code: z.string() });
const failureSchema = z.object({ check: z.string(), clause: z.string(), path: z.string(), message: z.string() });
const responseSchema = z.union([
  z.object({ id: z.number(), compile_error: z.string() }),
  z.object({ id: z.number(), ms: z.number(), findings: z.array(findingSchema), failures: z.array(failureSchema) }),
]);

export function factsBinaryPath(): string {
  return resolve(process.env.FACTS_ENGINE_BIN ?? "target/release/facts-a");
}

export function factsBinarySha256(): string {
  return createHash("sha256").update(readFileSync(factsBinaryPath())).digest("hex");
}

function facts(args: string[]): void {
  const run = spawnSync(factsBinaryPath(), args, { stdio: ["ignore", "inherit", "inherit"], maxBuffer: 1 << 30 });
  if (run.error !== undefined) throw run.error;
  if (run.status !== 0) throw new Error(`facts engine exited with ${run.status ?? run.signal}: ${factsBinaryPath()} ${args.join(" ")}`);
}

export function toResult(findings: z.infer<typeof findingSchema>[], failures: z.infer<typeof failureSchema>[]): FactsResult {
  return {
    violations: findings.map(f => ({ code: f.code, path: f.path, message: "", clauseId: f.clause, span: { startByte: -1, startLine: f.line - 1, startColumn: -1 } })),
    failures: failures.map(f => ({ code: "facts-failure", message: `${f.check}/${f.clause}: ${f.message}`, path: f.path })),
  };
}

/** Evaluates every call in one facts-engine process (`facts-a calls`); results by call id, with the engine's time per call. */
export function runFactsCalls(calls: FactsCall[]): Map<number, { result: FactsResult; ms: number }> {
  const dir = mkdtempSync(join(tmpdir(), "facts-calls-"));
  try {
    const input = join(dir, "calls.jsonl"), output = join(dir, "results.jsonl");
    writeFileSync(input, calls.map(call => JSON.stringify(call)).join("\n") + "\n");
    facts(["calls", "--input", input, "--output", output]);
    const results = new Map<number, { result: FactsResult; ms: number }>();
    for (const line of readFileSync(output, "utf8").split("\n")) {
      if (line.trim() === "") continue;
      const response = responseSchema.parse(JSON.parse(line));
      results.set(response.id, "compile_error" in response
        ? { result: { violations: [], failures: [{ code: "facts-compile-error", message: `facts engine: ${response.compile_error}` }] }, ms: 0 }
        : { result: toResult(response.findings, response.failures), ms: response.ms });
    }
    const missing = calls.filter(call => !results.has(call.id));
    if (missing.length > 0) throw new Error(`facts engine returned no result for ${missing.length} calls`);
    return results;
  } finally { rmSync(dir, { recursive: true, force: true }); }
}

/**
 * The repository suite through the facts engine's parallel repository run (`facts-a repo`): the recording's files are
 * written under a temporary root and evaluated with the policies' production configuration. Returns findings keyed as
 * `suite.ts` keys them ([ruleId, path, line, code]) and the failures.
 */
export function runFactsRepository(files: { path: string; source: string }[], policies: string, threads = 4): { keys: string[]; failures: unknown[]; results: unknown } {
  const dir = mkdtempSync(join(tmpdir(), "facts-repo-"));
  try {
    const root = join(dir, "root"), out = join(dir, "out");
    for (const file of files) {
      const path = join(root, file.path);
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, file.source);
    }
    facts(["repo", "--root", root, "--threads", String(threads), "--policies", resolve(policies), "--out-dir", out]);
    const results = z.object({ findings: z.array(z.tuple([z.string(), z.string(), z.number(), z.string()])), failures: z.array(failureSchema), files: z.number() })
      .passthrough().parse(JSON.parse(readFileSync(join(out, "results.json"), "utf8")));
    if (results.files !== files.filter(file => file.path.endsWith(".py")).length) throw new Error(`facts engine walked ${results.files} files of ${files.length}`);
    const keys = results.findings.map(([check, path, line, code]) => JSON.stringify([`data-layer-${check.replaceAll("_", "-")}`, path, line, code])).sort();
    return { keys, failures: results.failures, results: { files: results.files, wallNs: (results as Record<string, unknown>).wall_ns, maxRssBytes: (results as Record<string, unknown>).max_rss_bytes } };
  } finally { rmSync(dir, { recursive: true, force: true }); }
}
