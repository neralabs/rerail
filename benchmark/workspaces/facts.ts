// `run --engine facts`: the memory47 workspaces through one facts daemon (PHASE-3.md § P3c, docs/facts/PROTOCOL.md).
// Per workspace, in turn: a bulk sync from the Git object database (the first workspace evaluates; later ones are new
// worktrees over a warm store), the unchanged re-apply (the same commit synced again, then every file edited to its own
// Git blob id: both must do no work), and `--edits` edits appending a comment to the Python files at the minimum,
// median, p95 and maximum size in turn, each hash-first (bytes only on `need_bytes`). Then the membership rebalance and
// the daemon's resident set at steady state. `--findings-out` writes each workspace's per-file findings for
// `compare-findings`.
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { performance } from "node:perf_hooks";
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
    let t = performance.now();
    const sync = await daemon.request({ op: "sync", worktree: name, git_dir: gitDir, commit });
    const syncMs = performance.now() - t;
    const afterSync = await stats();
    if (options.findingsOut !== undefined) writeFileSync(join(options.findingsOut, `${name}.scan.json`), `${JSON.stringify(await daemon.request({ op: "findings", worktree: name }))}\n`);
    // Tracked files the per-file engine reads, by size (`ls-files -s` gives the blob ids the re-apply sends).
    const tracked = gitLines(root, "ls-files", "-s").map(line => {
      const [meta, path] = line.split("\t") as [string, string];
      return { path, git: meta.split(" ")[1]! };
    }).filter(file => file.path.endsWith(".py") || file.path.endsWith(".rs"));
    let reapply;
    if (options.reapply) {
      t = performance.now();
      const again = await daemon.request({ op: "sync", worktree: name, git_dir: gitDir, commit });
      const syncAgainMs = performance.now() - t;
      t = performance.now();
      const work = { evaluated_clauses: 0, memoised_clauses: 0, evaluated_pairs: 0, changed_paths: 0, need_bytes: 0 };
      for (const file of tracked) {
        const reply = await daemon.request({ op: "edit", worktree: name, path: file.path, git: file.git });
        if (reply.op === "need_bytes") work.need_bytes += 1;
        else {
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
      t = performance.now();
      const { reply, sentBytes } = await daemon.edit(name, target.path, bytes);
      edits.push({ bucket: target.bucket, path: target.path, sourceBytes: bytes.length, git: gitBlobId(bytes), sentBytes, wallMs: performance.now() - t, daemonMs: reply.ms, work: reply.work, introduced: (reply.introduced as unknown[]).length, removed: (reply.removed as unknown[]).length });
    }
    if (options.findingsOut !== undefined && options.edits > 0) writeFileSync(join(options.findingsOut, `${name}.edits.json`), `${JSON.stringify(await daemon.request({ op: "findings", worktree: name }))}\n`);
    const after = await stats();
    const wall = edits.map(edit => edit.wallMs);
    const measured = {
      name, root, commit,
      sync: { wallMs: syncMs, daemonMs: sync.ms, changedPaths: sync.changed_paths, work: sync.work, findings: (sync.introduced as unknown[]).length },
      reapply,
      edits: { count: edits.length, wallP50Ms: percentile(wall, 0.5), wallP95Ms: percentile(wall, 0.95), samples: edits },
      rss: { afterSync: afterSync.rss, afterEdits: after.rss },
      state: after.state,
    };
    workspaces.push(measured);
    process.stderr.write(`rerail-workspaces: facts ${name}: sync ${Math.round(syncMs)} ms (${JSON.stringify(sync.work)}), ${measured.sync.findings} findings; ` +
      (reapply === undefined ? "" : `re-apply ${Math.round(reapply.syncAgainMs + reapply.editsMs)} ms, ${reapply.work.evaluated_clauses} clauses evaluated; `) +
      `edits p50 ${measured.edits.wallP50Ms.toFixed(1)} / p95 ${measured.edits.wallP95Ms.toFixed(1)} ms; rss ${(after.rss / MIB).toFixed(0)} MiB\n`);
  }
  const beforeRebalance = await stats();
  const rebalance = await daemon.request({ op: "rebalance" });
  // Cross-file pushes (with --cross) run when the daemon is idle; a stats request after a pause collects them.
  await new Promise(resolveIdle => setTimeout(resolveIdle, options.cross === "none" ? 0 : 2_000));
  const steady = await stats();
  await daemon.close();
  const result = { benchmark: "rerail-workspaces/v1", engine: "facts", hello, cross: options.cross, store: options.store, wallMs: performance.now() - started, workspaces, rebalance, rss: { beforeRebalance: beforeRebalance.rss, steady: steady.rss }, steady, pushes };
  writeFileSync(options.output, `${JSON.stringify(result, null, 2)}\n`);
  process.stderr.write(`rerail-workspaces: facts: ${workspaces.length} workspaces in ${(result.wallMs / 1000).toFixed(0)} s; steady RSS ${(steady.rss / MIB).toFixed(0)} MiB; ${options.output}\n`);
}

const helloSchema = z.object({ hello: z.object({ per_file_clauses: z.array(z.tuple([z.string(), z.string()])) }) });
const nativeFindingsSchema = z.object({
  violations: z.array(z.object({ ruleId: z.string().optional(), policyId: z.string().optional(), clauseId: z.string(), path: z.string(), code: z.string(), span: z.object({ startLine: z.number() }) })),
});
const factsFindingsSchema = z.object({ findings: z.array(z.tuple([z.string(), z.string(), z.string(), z.number(), z.string(), z.number().nullable()])) });

/**
 * `compare-findings <native findings dir> <facts findings dir> [<facts.json>]`: per workspace (`<name>.scan.json`, and
 * `<name>.edits.json` when both have it), native's violations of the clauses the facts daemon answers per file (its
 * `hello`'s `per_file_clauses`, from facts.json) against the daemon's per-file findings, as multisets of
 * `[check, clause, path, line, code]` (native's rule id `data-layer-a-b` is check `a_b`). Prints one row per file
 * compared and up to 5 differences each; returns whether every workspace is equal.
 */
export function compareFindings(nativeDir: string, factsDir: string, factsJson: string): boolean {
  const run = helloSchema.parse(JSON.parse(readFileSync(factsJson, "utf8")));
  const perFile = new Set(run.hello.per_file_clauses.map(([check, clause]) => `${check}\u0000${clause}`));
  let equal = true;
  for (const file of readdirSync(factsDir).filter(name => name.endsWith(".json")).sort()) {
    const nativePath = join(nativeDir, file);
    if (!existsSync(nativePath)) continue;
    const native = nativeFindingsSchema.parse(JSON.parse(readFileSync(nativePath, "utf8"))).violations
      .map(v => [(v.ruleId ?? v.policyId ?? "").replace(/^data-layer-/, "").replaceAll("-", "_"), v.clauseId, v.path, v.span.startLine + 1, v.code] as const)
      .filter(([check, clause]) => perFile.has(`${check}\u0000${clause}`))
      .map(key => JSON.stringify(key));
    const facts = factsFindingsSchema.parse(JSON.parse(readFileSync(join(factsDir, file), "utf8"))).findings
      .map(([check, clause, path, line, code]) => JSON.stringify([check, clause, path, line, code]));
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
    equal &&= onlyNative.length === 0 && onlyFacts.length === 0;
    process.stdout.write(`${file}: native ${native.length}, facts ${facts.length}, only native ${onlyNative.length}, only facts ${onlyFacts.length}\n`);
    for (const key of [...onlyNative.slice(0, 5).map(k => `  native-only ${k}`), ...onlyFacts.slice(0, 5).map(k => `  facts-only ${k}`)]) process.stdout.write(`${key}\n`);
  }
  return equal;
}
