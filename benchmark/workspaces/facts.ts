// `run --engine facts`: the memory47 workspaces through one facts daemon (PHASE-3.md § P3c, docs/facts/PROTOCOL.md).
// Per workspace, in turn: a bulk sync from the Git object database (the first workspace evaluates; later ones are new
// worktrees over a warm store), the unchanged re-apply (the same commit synced again, then every file edited to its own
// Git blob id: both must do no work), and `--edits` edits appending a comment to the Python files at the minimum,
// median, p95 and maximum size in turn, each hash-first (bytes only on `need_bytes`). Each sync and edit is timed to its
// first findings (its first partial frame, or the reply when the daemon sends none) and to its reply (complete). Then
// the membership rebalance, the daemon's resident set at steady state, and its memory attribution at shutdown
// (`shutdown` with `attribute`). `--findings-out` writes each workspace's per-file findings for `compare-findings`.
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { setTimeout as sleep } from "node:timers/promises";
import { z } from "zod";
import { type DaemonMessage, FactsDaemon, gitBlobId } from "../../conformance/data-layer/facts-daemon.js";

export interface FactsRunOptions {
  daemon: string;
  policies: string;
  store: string;
  threads: number | undefined;
  cross: string;
  roots: ReadonlyArray<{ name: string; root: string; commit: string }>;
  edits: number;
  reapply: boolean;
  output: string;
  findingsOut: string | undefined;
}

const MIB = 1024 * 1024;

function gitLines(root: string, ...args: string[]): string[] {
  const result = spawnSync("git", ["-C", root, ...args], { encoding: "utf8", maxBuffer: 1 << 28 });
  if (result.status !== 0) throw new Error(`git -C ${root} ${args.join(" ")}: ${result.stderr.trim()}`);
  return result.stdout.split("\n").filter(line => line !== "");
}

function percentile(values: readonly number[], fraction: number): number {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)] ?? Number.NaN;
}

function distribution(values: readonly number[]) {
  return { n: values.length, p50: percentile(values, 0.5), p95: percentile(values, 0.95), p99: percentile(values, 0.99), max: percentile(values, 1) };
}

const count = (rows: unknown) => (rows as unknown[]).length;

/** Applies a `findings` frame (partial or reply) to the rows a client holds, as multiset counts by row. */
function applyFrame(held: Map<string, number>, message: DaemonMessage) {
  for (const [rows, sign] of [[message.introduced, 1], [message.failures_introduced, 1], [message.removed, -1], [message.failures_removed, -1]] as const) {
    for (const row of (rows as unknown[] | undefined) ?? []) {
      const key = JSON.stringify(row);
      const n = (held.get(key) ?? 0) + sign;
      if (n === 0) held.delete(key);
      else held.set(key, n);
    }
  }
}

/** Rows the client holds that the daemon's `all_findings` lacks, and the reverse (refused rules' failures excluded). */
function heldMismatches(held: Map<string, number>, all: DaemonMessage): number {
  const expected = new Map<string, number>();
  const failures = (all.failures as unknown[][]).filter(row => row[2] !== "");
  for (const row of [...(all.findings as unknown[]), ...failures]) applyFrame(expected, { op: "findings", introduced: [row] });
  let mismatches = 0;
  for (const key of new Set([...held.keys(), ...expected.keys()])) mismatches += Math.abs((held.get(key) ?? 0) - (expected.get(key) ?? 0));
  return mismatches;
}

/**
 * A sync or edit timed to its first findings (its first partial frame, else its reply) and to its reply (complete),
 * with the findings introduced and removed over every frame; every frame is applied to `held`.
 */
async function timed<T>(held: Map<string, number>, run: (partial: (message: DaemonMessage) => void) => Promise<T>, replyOf: (result: T) => DaemonMessage) {
  const t = performance.now();
  let firstMs: number | undefined;
  let frames = 0;
  let introduced = 0;
  let removed = 0;
  const result = await run(message => {
    firstMs ??= performance.now() - t;
    frames += 1;
    introduced += count(message.introduced);
    removed += count(message.removed);
    applyFrame(held, message);
  });
  const completeMs = performance.now() - t;
  const reply = replyOf(result);
  applyFrame(held, reply);
  return { result, reply, firstMs: firstMs ?? completeMs, completeMs, frames, introduced: introduced + count(reply.introduced), removed: removed + count(reply.removed) };
}

export async function runFacts(options: FactsRunOptions): Promise<void> {
  const started = performance.now();
  const daemon = await FactsDaemon.start({ binary: options.daemon, policies: options.policies, store: options.store, threads: options.threads, cross: options.cross });
  const pushes: DaemonMessage[] = [];
  daemon.onPush(message => pushes.push({ ...message, introduced: (message.introduced as unknown[]).length, removed: (message.removed as unknown[]).length }));
  const hello = await daemon.request({ op: "hello" });
  const stats = async () => (await daemon.request({ op: "stats" })) as DaemonMessage & { rss: number; state: unknown; work: Record<string, number> };
  const workspaces = [];
  for (const { name, root, commit } of options.roots) {
    const gitDir = resolve(root, gitLines(root, "rev-parse", "--git-common-dir")[0]!);
    let t: number;
    // The per-file rows a client holds after applying every frame of this worktree's requests.
    const held = new Map<string, number>();
    const sync = await timed(held, partial => daemon.request({ op: "sync", worktree: name, git_dir: gitDir, commit }, undefined, partial), reply => reply);
    const afterSync = await stats();
    const scanned = await daemon.request({ op: "findings", worktree: name });
    const syncMismatches = heldMismatches(held, scanned);
    if (options.findingsOut !== undefined) writeFileSync(join(options.findingsOut, `${name}.scan.json`), `${JSON.stringify(scanned)}\n`);
    // Tracked files the per-file engine reads, by size (`ls-files -s` gives the blob ids the re-apply sends).
    const tracked = gitLines(root, "ls-files", "-s").map(line => {
      const [meta, path] = line.split("\t") as [string, string];
      return { path, git: meta.split(" ")[1]! };
    }).filter(file => file.path.endsWith(".py") || file.path.endsWith(".rs"));
    let reapply;
    if (options.reapply) {
      t = performance.now();
      const again = await daemon.request({ op: "sync", worktree: name, git_dir: gitDir, commit });
      applyFrame(held, again);
      const syncAgainMs = performance.now() - t;
      t = performance.now();
      const work = { evaluated_clauses: 0, memoised_clauses: 0, evaluated_pairs: 0, changed_paths: 0, need_bytes: 0 };
      for (const file of tracked) {
        const reply = await daemon.request({ op: "edit", worktree: name, path: file.path, git: file.git });
        if (reply.op === "need_bytes") work.need_bytes += 1;
        else {
          applyFrame(held, reply);
          const w = reply.work as Record<string, number>;
          work.evaluated_clauses += w.evaluated_clauses!;
          work.memoised_clauses += w.memoised_clauses!;
          work.evaluated_pairs += w.evaluated_pairs!;
          work.changed_paths += reply.changed_paths as number;
        }
      }
      reapply = { syncAgainMs, syncAgainChangedPaths: again.changed_paths, syncAgainWork: again.work, editsMs: performance.now() - t, files: tracked.length, work };
    }
    const python = tracked.filter(file => file.path.endsWith(".py")).map(file => ({ ...file, bytes: statSync(join(root, file.path)).size }));
    python.sort((a, b) => a.bytes - b.bytes || (a.path < b.path ? -1 : 1));
    const at = (fraction: number) => python[Math.floor((python.length - 1) * fraction)]!;
    const targets = [{ bucket: "minimum", ...at(0) }, { bucket: "p50", ...at(0.5) }, { bucket: "p95", ...at(0.95) }, { bucket: "maximum", ...at(1) }];
    const edits = [];
    for (let iteration = 0; iteration < options.edits; iteration += 1) {
      const target = targets[iteration % targets.length]!;
      const bytes = Buffer.concat([readFileSync(join(root, target.path)), Buffer.from(`\n# rerail benchmark edit ${iteration}\n`)]);
      const edit = await timed(held, partial => daemon.edit(name, target.path, bytes, partial), r => r.reply);
      const { reply } = edit;
      edits.push({ bucket: target.bucket, path: target.path, sourceBytes: bytes.length, git: gitBlobId(bytes), sentBytes: edit.result.sentBytes, firstMs: edit.firstMs, wallMs: edit.completeMs, frames: edit.frames, daemonMs: reply.ms, work: reply.work, tasks: reply.tasks, workers: reply.workers, introduced: edit.introduced, removed: edit.removed });
    }
    const edited = await daemon.request({ op: "findings", worktree: name });
    const editMismatches = heldMismatches(held, edited);
    if (options.findingsOut !== undefined && options.edits > 0) writeFileSync(join(options.findingsOut, `${name}.edits.json`), `${JSON.stringify(edited)}\n`);
    const after = await stats();
    const wall = edits.map(edit => edit.wallMs);
    const measured = {
      name, root, commit,
      sync: { firstMs: sync.firstMs, wallMs: sync.completeMs, daemonMs: sync.reply.ms, frames: sync.frames, changedPaths: sync.reply.changed_paths, work: sync.reply.work, tasks: sync.reply.tasks, findings: sync.introduced },
      // Rows a client applying every frame holds that differ from the daemon's `findings` (0: the deltas are exact).
      heldMismatches: { afterSync: syncMismatches, afterEdits: editMismatches },
      reapply,
      edits: { count: edits.length, wallP50Ms: percentile(wall, 0.5), wallP95Ms: percentile(wall, 0.95), samples: edits },
      rss: { afterSync: afterSync.rss, afterEdits: after.rss },
      state: after.state,
    };
    workspaces.push(measured);
    process.stderr.write(`rerail-workspaces: facts ${name}: sync first ${Math.round(sync.firstMs)} / complete ${Math.round(sync.completeMs)} ms (${JSON.stringify(sync.reply.work)}), ${measured.sync.findings} findings; ` +
      (reapply === undefined ? "" : `re-apply ${Math.round(reapply.syncAgainMs + reapply.editsMs)} ms, ${reapply.work.evaluated_clauses} clauses evaluated; `) +
      `edits p50 ${measured.edits.wallP50Ms.toFixed(1)} / p95 ${measured.edits.wallP95Ms.toFixed(1)} ms; held mismatches ${syncMismatches}/${editMismatches}; rss ${(after.rss / MIB).toFixed(0)} MiB\n`);
  }
  const beforeRebalance = await stats();
  const rebalance = await daemon.request({ op: "rebalance" });
  // Cross-file pushes (with --cross) come from the daemon's cross-file thread; a pause lets the last ones land.
  if (options.cross !== "none") await sleep(2_000);
  const steady = await stats();
  const bye = await daemon.close(true);
  const evaluating = workspaces.flatMap(w => w.edits.samples).filter(e => (e.work as Record<string, number>).evaluated_clauses! > 0);
  const allEdits = workspaces.flatMap(w => w.edits.samples);
  const buckets = [...new Set(allEdits.map(e => e.bucket))];
  const newWorktrees = workspaces.slice(1).map(w => w.sync);
  const summary = {
    newWorktree: { first: distribution(newWorktrees.map(s => s.firstMs)), complete: distribution(newWorktrees.map(s => s.wallMs)) },
    edits: {
      all: { first: distribution(allEdits.map(e => e.firstMs)), complete: distribution(allEdits.map(e => e.wallMs)) },
      evaluating: { first: distribution(evaluating.map(e => e.firstMs)), complete: distribution(evaluating.map(e => e.wallMs)) },
      byBucket: Object.fromEntries(buckets.map(b => {
        const of = allEdits.filter(e => e.bucket === b);
        return [b, { first: distribution(of.map(e => e.firstMs)), complete: distribution(of.map(e => e.wallMs)), evaluating: of.filter(e => (e.work as Record<string, number>).evaluated_clauses! > 0).length }];
      })),
    },
  };
  const result = { benchmark: "rerail-workspaces/v1", engine: "facts", hello, cross: options.cross, store: options.store, wallMs: performance.now() - started, summary, workspaces, rebalance, rss: { beforeRebalance: beforeRebalance.rss, steady: steady.rss }, steady, memory: bye.memory, pushes };
  writeFileSync(options.output, `${JSON.stringify(result, null, 2)}\n`);
  process.stderr.write(`rerail-workspaces: facts: ${workspaces.length} workspaces in ${(result.wallMs / 1000).toFixed(0)} s; steady RSS ${(steady.rss / MIB).toFixed(0)} MiB; ${options.output}\n${JSON.stringify(summary)}\n`);
}

const helloSchema = z.object({ hello: z.object({ per_file_clauses: z.array(z.tuple([z.string(), z.string()])) }) });
const nativeFindingsSchema = z.object({
  violations: z.array(z.object({ ruleId: z.string().optional(), policyId: z.string().optional(), clauseId: z.string(), path: z.string(), code: z.string(), span: z.object({ startLine: z.number() }) })),
  failures: z.array(z.object({ ruleId: z.string().optional(), policyId: z.string().optional(), clauseId: z.string().optional(), code: z.string().optional(), path: z.string().optional() })).optional(),
});
const factsFindingsSchema = z.object({
  findings: z.array(z.tuple([z.string(), z.string(), z.string(), z.number(), z.string(), z.number().nullable()])),
  failures: z.array(z.tuple([z.string(), z.string(), z.string(), z.string()])),
});

/** Native's rule id as the facts engine's check name (`eval::Rule::compile`: every leading `data-layer-` goes). */
const checkOf = (ruleId: string) => ruleId.replace(/^(data-layer-)+/, "").replaceAll("-", "_");

/**
 * `compare-findings <native findings dir> <facts findings dir> [<facts.json>]`: per workspace (`<name>.scan.json`, and
 * `<name>.edits.json` when both have it), native's violations of the clauses the facts daemon answers per file (its
 * `hello`'s `per_file_clauses`, from facts.json) against the daemon's per-file findings, as multisets of
 * `[check, clause, path, line, code]`. Prints one row per file compared (with each side's per-file failures by check and
 * clause) and up to 5 differences each, then the differences summed by check; returns whether every workspace is equal.
 */
export function compareFindings(nativeDir: string, factsDir: string, factsJson: string): boolean {
  const run = helloSchema.parse(JSON.parse(readFileSync(factsJson, "utf8")));
  const perFile = new Set(run.hello.per_file_clauses.map(([check, clause]) => `${check}\u0000${clause}`));
  const byCheck = new Map<string, { onlyNative: number; onlyFacts: number }>();
  let equal = true;
  let files = 0;
  for (const file of readdirSync(factsDir).filter(name => name.endsWith(".json")).sort()) {
    const nativePath = join(nativeDir, file);
    if (!existsSync(nativePath)) continue;
    files += 1;
    const nativeRun = nativeFindingsSchema.parse(JSON.parse(readFileSync(nativePath, "utf8")));
    const native = nativeRun.violations
      .map(v => [checkOf(v.ruleId ?? v.policyId ?? ""), v.clauseId, v.path, v.span.startLine + 1, v.code] as const)
      .filter(([check, clause]) => perFile.has(`${check}\u0000${clause}`))
      .map(key => JSON.stringify(key));
    const factsRun = factsFindingsSchema.parse(JSON.parse(readFileSync(join(factsDir, file), "utf8")));
    const facts = factsRun.findings.map(([check, clause, path, line, code]) => JSON.stringify([check, clause, path, line, code]));
    const minus = (left: string[], right: string[]) => {
      const counts = new Map<string, number>();
      for (const key of right) counts.set(key, (counts.get(key) ?? 0) + 1);
      return left.filter(key => {
        const n = counts.get(key) ?? 0;
        if (n > 0) counts.set(key, n - 1);
        return n === 0;
      });
    };
    const onlyNative = minus(native, facts);
    const onlyFacts = minus(facts, native);
    for (const [keys, side] of [[onlyNative, "onlyNative"], [onlyFacts, "onlyFacts"]] as const) {
      for (const key of keys) {
        const check = (JSON.parse(key) as string[])[0]!;
        const row = byCheck.get(check) ?? { onlyNative: 0, onlyFacts: 0 };
        row[side] += 1;
        byCheck.set(check, row);
      }
    }
    equal &&= onlyNative.length === 0 && onlyFacts.length === 0;
    const nativeFailures = (nativeRun.failures ?? []).map(f => `${checkOf(f.ruleId ?? f.policyId ?? "")}/${f.clauseId ?? ""} ${f.code ?? ""}`);
    const factsFailures = factsRun.failures.filter(([, , path]) => path !== "").map(([check, clause, , message]) => `${check}/${clause} ${message}`);
    process.stdout.write(`${file}: native ${native.length}, facts ${facts.length}, only native ${onlyNative.length}, only facts ${onlyFacts.length}; ` +
      `failures native [${nativeFailures.join("; ")}], facts per-file [${factsFailures.join("; ")}]\n`);
    for (const key of [...onlyNative.slice(0, 5).map(k => `  native-only ${k}`), ...onlyFacts.slice(0, 5).map(k => `  facts-only ${k}`)]) process.stdout.write(`${key}\n`);
  }
  process.stdout.write(`compare-findings: ${files} files; differences by check: ${byCheck.size === 0 ? "none" : JSON.stringify(Object.fromEntries(byCheck))}\n`);
  return equal;
}
