// The waits a user experiences (PRD §5.3), measured on the baseline and the candidate through the product's own entry
// points, never the engine binary: the Claude Code hook as Claude Code runs it on Linux (`node dist/hosts/claude/
// claude-hook.js`, one process per event, the event JSON on stdin), the CLI (`node dist/cli/cli.js`) and the daemon the
// CLI and the hooks start. `shipbar.ts waits` drives it; waits-model.ts turns its rows into the report's waits section.
//
// Each side runs in its own throwaway HOME and Rerail home (RERAIL_HOME), with its own database and content store
// (RERAIL_CONTENT_STORE_PATH: the checkout is no managed installation, so it names the store a release keeps in
// ~/.cache/rerail, under the file name the side's engine gives it), set up as a person sets a repository up: each pack
// added from the side's own packs/ and approved, then `rerail start --packs …` answered as the agent lane asks. The
// repository is a fresh git repository of a pinned real-code corpus, with a branch `switch` holding the replayed
// session's edits (W5's branch switch). Every process runs on the pinned CPUs (taskset); the daemon inherits them.
//
// Groups, each run per side, the side order alternating per repetition:
//   replay         W1, W2: a warm home (set up and scanned once); per repetition the daemon starts, the session is
//                  replayed with its pauses, and the database says when each edit's check finished.
//   first-session  W3, W6 autostart: a fresh home, no daemon, page cache dropped; SessionStart (which starts the
//                  daemon), the prompt and the session's first edit, until that edit's check finishes.
//   lifecycle      W5, W6, W7: a fresh home; page cache dropped; `daemon start`; scans first ever, warm and after
//                  `git checkout switch`; the CLI commands; the daemon stopped and started warm; a SessionStart.
//   upgrade        W4: a home the baseline wrote (a session and a scan) restored before each run, read by each side.
//   busy           W8: while repository scans run (each after a branch switch), `rerail status` and a hook pair.

import { spawnSync } from "node:child_process";
import { appendFileSync, cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { hostname } from "node:os";
import { dirname, extname, join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { buildWaitsSection, failingScenarios, SCENARIOS, scenarioId, type ScenarioSpec, type WaitId, type WaitRun, type WaitsSection, type WaitSide } from "./waits-model.js";
import type { RepoSpec, Session } from "./waits-session.js";
import { editRuleDocument, editSource, FILE_EDITS, oneRulePack, pickStepFile, type Shape, SHAPES } from "./waits-steps.js";

export type CpuTimes = Map<number, { busy: number; total: number }>;

/** The machine probes shipbar.ts measures with, shared so a wait's load is recorded as a pack run's is. */
export interface Machine {
  cpuTimes: () => CpuTimes;
  busiest: (before: CpuTimes, after: CpuTimes, cpus: readonly number[]) => number;
}

export interface WaitsSettings {
  checkouts: Record<WaitSide, { dir: string; sha: string }>;
  out: string;
  repoName: string;
  repo: RepoSpec;
  corpusDir: string;
  corpus: { files: number; sha256: string };
  /** The repository's replayed session; null for a repository that runs the step benchmarks only. */
  sessionName: string | null;
  session: Session | null;
  sessionSha256: string | null;
  cpus: number[];
  siblings: number[];
  /** Runs per side of each single wait (at least 5: they are under 2 s, or near it). */
  runs: number;
  /** Replays per side of each distribution wait. */
  replays: number;
  /** Confirmation repetitions of a failing scenario: runs for a single wait, replays for a distribution. */
  confirm: number;
  /** Scenario ids requested. */
  scenarios: string[];
  /** A short scratch root (Unix socket paths are short). */
  work: string;
  limits: Record<string, number | null>;
  reportOnly: string[];
  command: string;
}

type Group = "replay" | "first-session" | "lifecycle" | "upgrade" | "busy" | `steps:${Shape}`;

const GROUP_OF: Record<WaitId, Group> = { W1: "replay", W2: "replay", W3: "first-session", W4: "upgrade", W5: "lifecycle", W6: "lifecycle", W7: "lifecycle", W8: "busy" };

/** A step scenario (W1 and W5 `steps-<shape>-<step>`) in its shape's group; W6's autostart in the first-session group; every other scenario in its wait's group. */
function groupOf(spec: ScenarioSpec): Group {
  if (spec.shape !== undefined) return `steps:${spec.shape}`;
  return spec.wait === "W6" && spec.scenario === "autostart" ? "first-session" : GROUP_OF[spec.wait];
}

/** What the report says is not measured, beyond the scenarios a run did not request. */
export const NOT_MEASURED: Record<string, string> = {
  "wait/W3/fully-indexed": "time until fully indexed: the product exposes no signal for the end of indexing that a run could wait for",
  "wait/W1/other-hosts": "W1 on Codex, Cursor and OMP: only the Claude Code hook is replayed (Codex's and Cursor's hooks share its daemon path; OMP runs in process)",
  "wait/resources": "§5.4 resources (daemon RSS over a session, CPU-seconds, store size)",
};

// ---------------------------------------------------------------- processes

function sleep(ms: number): void {
  if (ms > 0) Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

interface Result {
  status: number;
  stdout: string;
  stderr: string;
  ms: number;
  start: number;
  end: number;
}

function lastLine(text: string): string {
  return (text.split("\n").map((line) => line.trim()).filter((line) => line !== "").at(-1) ?? "").slice(0, 300);
}

/** The OS page cache dropped (needs passwordless sudo); whether it was. */
function dropPageCache(): boolean {
  return spawnSync("sudo", ["-n", "sh", "-c", "sync; echo 3 > /proc/sys/vm/drop_caches"], { stdio: "ignore" }).status === 0;
}

/** One side's throwaway home: its environment, its checkout's entry points, and the repository it works in. */
class Home {
  readonly repo: string;
  readonly env: Record<string, string>;

  constructor(
    readonly settings: WaitsSettings,
    readonly side: WaitSide,
    readonly dir: string,
    /** The checkout whose entry points run here (W4 runs either side on a home the baseline wrote). */
    readonly checkout: string = settings.checkouts[side].dir,
  ) {
    this.repo = join(dir, "repo");
    const env: Record<string, string> = {};
    for (const [key, value] of Object.entries(process.env)) if (value !== undefined && !key.startsWith("RERAIL_") && !key.startsWith("SHIPBAR_") && !key.startsWith("CLAUDE_")) env[key] = value;
    env.HOME = dir;
    env.RERAIL_HOME = join(dir, ".rerail");
    env.TMPDIR = join(dir, "t");
    env.BROWSER = "none";
    env.RERAIL_DEVICE_SECRET_STORE = "file";
    env.RERAIL_CONTENT_STORE_PATH = join(dir, "store", contentStoreFile(this.checkout));
    this.env = env;
  }

  private spawn(program: string, args: readonly string[], input: string | undefined, extraEnv: Record<string, string>, timeoutMs: number): Result {
    const start = Date.now();
    const t0 = performance.now();
    const result = spawnSync("taskset", ["-c", this.settings.cpus.join(","), program, ...args], {
      cwd: existsSync(this.repo) ? this.repo : this.dir, env: { ...this.env, ...extraEnv }, input, encoding: "utf8", maxBuffer: 1 << 28, timeout: timeoutMs,
    });
    const ms = performance.now() - t0;
    return { status: result.status ?? (result.signal === null ? 1 : 124), stdout: result.stdout ?? "", stderr: result.stderr ?? String(result.error ?? ""), ms, start, end: Date.now() };
  }

  cli(args: readonly string[], timeoutMs = 120_000): Result {
    return this.spawn(process.execPath, [join(this.checkout, "dist", "cli", "cli.js"), ...args], undefined, {}, timeoutMs);
  }

  /** One Claude Code hook event, in its own process, as Claude Code runs it. */
  hook(input: Record<string, unknown>): Result & { quiet: boolean; steered: boolean } {
    const result = this.spawn(process.execPath, [join(this.checkout, "dist", "hosts", "claude", "claude-hook.js")], JSON.stringify({ cwd: this.repo, ...input }), { CLAUDE_PROJECT_DIR: this.repo }, 120_000);
    const out = result.stdout.trim();
    return { ...result, quiet: out === "{}", steered: /"additionalContext"|"systemMessage"/.test(out) };
  }

  git(args: readonly string[]): void {
    const result = spawnSync("git", ["-c", "user.name=shipbar", "-c", "user.email=shipbar@example.invalid", "-c", "commit.gpgsign=false", ...args], { cwd: this.repo, env: this.env, encoding: "utf8" });
    if (result.status !== 0) throw new Error(`git ${args.join(" ")}: ${lastLine(result.stderr)}`);
  }

  query<T>(sql: string, ...params: (string | number)[]): T[] {
    const path = join(this.dir, ".rerail", "rerail.sqlite");
    if (!existsSync(path)) return [];
    const database = new DatabaseSync(path, { readOnly: true });
    try {
      return database.prepare(sql).all(...params) as T[];
    } finally {
      database.close();
    }
  }

  /** The side's copy of a pack, which the home installs from (the rule step edits it, never the checkout). */
  packSource(pack: string): string {
    return join(this.dir, "packs-src", pack);
  }

  /** `rerail packs add` from the home's copy of `pack`, then `rerail rules approve` of it; its version. */
  addPack(pack: string): string {
    const folder = this.packSource(pack);
    const added = this.cli(["packs", "add", folder]);
    if (added.status !== 0) throw new Error(`rerail packs add ${pack}: ${lastLine(added.stderr || added.stdout)}`);
    const manifest = JSON.parse(readFileSync(join(folder, "package.json"), "utf8")) as { name: string; version: string };
    const approved = this.cli(["rules", "approve", manifest.name]);
    if (approved.status !== 0) throw new Error(`rerail rules approve ${manifest.name}: ${lastLine(approved.stderr || approved.stdout)}`);
    return manifest.version;
  }

  /**
   * A fresh home: the repository (the whole corpus, or only `files`; with the session's edits on branch `switch`
   * unless `switchBranch` is false), each pack copied from the side's checkout, added and approved (with `oneRule`,
   * only the pack holding that rule, every other rule of it off), and `rerail start` answered as asked.
   */
  create(options: { files?: readonly string[]; switchBranch?: boolean; oneRule?: string } = {}): void {
    rmSync(this.dir, { recursive: true, force: true });
    mkdirSync(join(this.dir, "t"), { recursive: true });
    mkdirSync(join(this.dir, "store"), { recursive: true });
    if (options.files === undefined) cpSync(this.settings.corpusDir, this.repo, { recursive: true });
    else for (const file of options.files) writeFile(join(this.repo, file), readFileSync(join(this.settings.corpusDir, file), "utf8"));
    this.git(["init", "-q", "-b", "main"]);
    this.git(["add", "-A"]);
    this.git(["commit", "-qm", "base"]);
    if (options.switchBranch !== false && this.settings.session !== null) {
      this.git(["checkout", "-qb", "switch"]);
      for (const turn of this.settings.session.turns) for (const edit of turn.edits) writeFile(join(this.repo, edit.path), edit.content);
      this.git(["add", "-A"]);
      this.git(["commit", "-qm", "the replayed session's edits"]);
      this.git(["checkout", "-q", "main"]);
    }
    const packs = options.oneRule === undefined ? this.settings.repo.packs : this.settings.repo.packs.filter((pack) => readFileSync(join(this.checkout, "packs", pack, "pack.json"), "utf8").includes(`"id": "${options.oneRule}"`));
    if (packs.length === 0) throw new Error(`no pack of ${this.settings.repo.packs.join(", ")} holds rule ${options.oneRule}`);
    const versions: string[] = [];
    for (const pack of packs) {
      const folder = this.packSource(pack);
      cpSync(join(this.checkout, "packs", pack), folder, { recursive: true });
      if (options.oneRule !== undefined) writeFileSync(join(folder, "pack.json"), oneRulePack(readFileSync(join(folder, "pack.json"), "utf8"), options.oneRule));
      versions.push(`${pack}@${this.addPack(pack)}`);
    }
    let answer: string | undefined;
    for (let step = 0; ; step += 1) {
      const started = this.cli(["start", "--repo", this.repo, "--packs", versions.join(","), "--json", ...(answer === undefined ? [] : ["--answer", answer])]);
      let parsed: { step?: string; screen?: string; question?: { options: { answer: string }[] } | null };
      try {
        parsed = JSON.parse(started.stdout) as typeof parsed;
      } catch {
        throw new Error(`rerail start: ${lastLine(started.stderr || started.stdout)}`);
      }
      if (parsed.step === "done") break;
      if (parsed.step === "error" || parsed.question == null || step > 6) throw new Error(`rerail start stopped at ${parsed.step}: ${(parsed.screen ?? "").slice(0, 200)}`);
      const offered = parsed.question.options.map((o) => o.answer);
      answer = ["allow", "yes", "start"].find((a) => offered.includes(a)) ?? offered[0];
    }
    this.stopDaemon();
  }

  /** The repository back on `main` as committed: the replay's edits undone. */
  resetRepo(): void {
    this.git(["checkout", "-qf", "main"]);
    this.git(["clean", "-fdq"]);
  }

  startDaemon(): Result {
    const result = this.cli(["daemon", "start"]);
    if (result.status !== 0) throw new Error(`rerail daemon start: ${lastLine(result.stderr || result.stdout)}`);
    return result;
  }

  /** `rerail daemon stop`, and a kill of its process if it is still there: no daemon outlives its run. */
  stopDaemon(): void {
    this.cli(["daemon", "stop"], 60_000);
    const state = join(this.dir, ".rerail", "daemon.json");
    if (!existsSync(state)) return;
    try {
      const { pid } = JSON.parse(readFileSync(state, "utf8")) as { pid?: number };
      if (pid !== undefined) process.kill(pid, "SIGKILL");
    } catch {
      // Gone already.
    }
  }

  /** The running daemon's CPU time so far (user + sys) and peak RSS (VmHWM), from /proc; null without a daemon. */
  daemonUsage(): { cpuMs: number; peakMb: number } | null {
    try {
      const { pid } = JSON.parse(readFileSync(join(this.dir, ".rerail", "daemon.json"), "utf8")) as { pid: number };
      // Fields after the command's closing parenthesis: utime and stime are the 12th and 13th, in clock ticks (100 Hz).
      const fields = readFileSync(`/proc/${pid}/stat`, "utf8").split(")").at(-1)!.trim().split(/\s+/);
      const hwm = /VmHWM:\s+(\d+) kB/.exec(readFileSync(`/proc/${pid}/status`, "utf8"))?.[1];
      return { cpuMs: (Number(fields[11]) + Number(fields[12])) * 10, peakMb: Math.round(Number(hwm ?? 0) / 1024) };
    } catch {
      return null;
    }
  }

  /** This home's edit checks (`ai-output` audits) begun at or after `since`, in order. */
  editChecks(since: number): { paths: string[]; started: number; finished: number | null; status: string; failure: string | null }[] {
    return this.query<{ paths_json: string; started_at: number; finished_at: number | null; status: string; failures_json: string | null }>(
      "SELECT paths_json, started_at, finished_at, status, failures_json FROM audits WHERE scope = 'ai-output' AND started_at >= ? ORDER BY started_at",
      since,
    ).map((row) => ({ paths: JSON.parse(row.paths_json) as string[], started: row.started_at, finished: row.finished_at, status: row.status, failure: row.status === "failed" ? (JSON.parse(row.failures_json ?? "[]") as { message?: string }[])[0]?.message ?? null : null }));
  }
}

/** The replayed session, which every group but the step benchmarks needs (shipbar.ts runs only those without one). */
function sessionOf(settings: WaitsSettings): Session {
  if (settings.session === null) throw new Error(`repo ${settings.repoName} has no replayed session (waits.json)`);
  return settings.session;
}

function writeFile(path: string, content: string): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
}

const storeFiles = new Map<string, string>();

/** The content store's file name the checkout's engine uses (it names its store floor: content-v<N>.sqlite). */
function contentStoreFile(checkout: string): string {
  let name = storeFiles.get(checkout);
  if (name === undefined) {
    const addon = readdirSync(join(checkout, "native")).find((file) => /^rerail_engine\..+\.node$/.test(file));
    if (addon === undefined) throw new Error(`${checkout} has no native engine (native/rerail_engine.*.node): build it`);
    const result = spawnSync(process.execPath, ["-e", `process.stdout.write(require(${JSON.stringify(join(checkout, "native", addon))}).contentStoreFileName())`], { encoding: "utf8" });
    if (result.status !== 0 || result.stdout === "") throw new Error(`${checkout}'s engine names no content store: ${lastLine(result.stderr)}`);
    name = result.stdout;
    storeFiles.set(checkout, name);
  }
  return name;
}

// ---------------------------------------------------------------- the session's events

/** A Write of `path` as Claude Code sends it: PreToolUse, the file written, PostToolUse. */
function edit(home: Home, sessionId: string, toolUseId: string, path: string, content: string): { pre: Result & { quiet: boolean }; post: Result & { quiet: boolean; steered: boolean } } {
  const file = join(home.repo, path);
  const tool = { session_id: sessionId, tool_name: "Write", tool_use_id: toolUseId, tool_input: { file_path: file, content } };
  const pre = home.hook({ ...tool, hook_event_name: "PreToolUse" });
  writeFile(file, content);
  const post = home.hook({ ...tool, hook_event_name: "PostToolUse", tool_response: { type: "update", filePath: file } });
  return { pre, post };
}

function sessionStart(home: Home, sessionId: string): Result & { quiet: boolean; steered: boolean } {
  return home.hook({ session_id: sessionId, hook_event_name: "SessionStart", source: "startup" });
}

/** A partial row: what one group measured of one scenario. */
interface Measured {
  wait: WaitId;
  scenario: string;
  ok: boolean;
  reason: string | null;
  samples: Record<string, number[]>;
  notes: WaitRun["notes"];
  cacheDropped: boolean | null;
}

function measured(wait: WaitId, scenario: string, samples: Record<string, number[]>, notes: WaitRun["notes"] = {}, cacheDropped: boolean | null = null, failure: string | null = null): Measured {
  return { wait, scenario, ok: failure === null, reason: failure, samples: failure === null ? samples : {}, notes, cacheDropped };
}

interface EditTiming {
  path: string;
  preStart: number;
  postStart: number;
  postEnd: number;
  postMs: number;
  steered: boolean;
}

/**
 * Each edit's verdict: its check's finish (the edit check, `ai-output` audit, of its path that began after its
 * PreToolUse and before the path's next edit), or its PostToolUse answer if that came later; late when the check
 * finished after the PostToolUse answered (the steer waits for a later event) or past the hook timeout limit when one
 * is set. A check that fails is no verdict: `failure` says so.
 */
function editVerdicts(home: Home, edits: readonly EditTiming[], since: number): { verdict: number[]; late: number[]; unmatched: number; failure: string | null } {
  const hookTimeout = home.settings.limits["W1.hook_timeout_ms"] ?? null;
  const checks = home.editChecks(since);
  const verdict: number[] = [];
  const late: number[] = [];
  let unmatched = 0;
  let failed = 0;
  let reason: string | null = null;
  edits.forEach((e, i) => {
    const next = edits.slice(i + 1).find((other) => other.path === e.path)?.preStart ?? Number.POSITIVE_INFINITY;
    const check = checks.find((c) => c.paths.includes(e.path) && c.started >= e.preStart && c.started < next && c.finished !== null);
    if (check === undefined) {
      unmatched += 1;
      verdict.push(e.postMs);
      late.push(hookTimeout !== null && e.postMs > hookTimeout ? 1 : 0);
      return;
    }
    if (check.status === "failed") {
      failed += 1;
      reason ??= check.failure;
    }
    // In the answer: the hook's own wall time; after it: until the check finished, from the hook's start.
    const ms = check.finished! <= e.postEnd ? e.postMs : check.finished! - e.postStart;
    verdict.push(ms);
    late.push(check.finished! > e.postEnd || (hookTimeout !== null && ms > hookTimeout) ? 1 : 0);
  });
  return { verdict, late, unmatched, failure: failed > 0 ? `${failed} of ${edits.length} edit checks failed: ${reason ?? "no reason recorded"}` : null };
}

function timing(path: string, pre: Result, post: Result & { steered: boolean }): EditTiming {
  return { path, preStart: pre.start, postStart: post.start, postEnd: post.end, postMs: post.ms, steered: post.steered };
}

/** W1 and W2: the session replayed with its pauses on a running daemon (`editVerdicts` for each edit's verdict). */
function replay(home: Home, sessionId: string): Measured[] {
  const session = sessionOf(home.settings);
  const { edit_ms: editPause, turn_ms: turnPause } = session.pauses;
  home.resetRepo();
  home.startDaemon();
  const since = Date.now();
  const quiet: number[] = [];
  const edits: EditTiming[] = [];
  const count = (r: Result & { quiet: boolean }): void => {
    if (r.quiet) quiet.push(r.ms);
  };
  try {
    const opened = sessionStart(home, sessionId);
    if (opened.quiet) throw new Error(`SessionStart answered {} (${lastLine(opened.stderr)})`);
    sleep(turnPause);
    session.turns.forEach((turn, t) => {
      count(home.hook({ session_id: sessionId, hook_event_name: "UserPromptSubmit", prompt_id: `p${t}`, prompt: `Make the change of commit ${turn.commit}.` }));
      sleep(editPause);
      turn.edits.forEach((e, i) => {
        const { pre, post } = edit(home, sessionId, `toolu_${t}_${i}`, e.path, e.content);
        count(pre);
        count(post);
        edits.push(timing(e.path, pre, post));
        sleep(editPause);
      });
      count(home.hook({ session_id: sessionId, hook_event_name: "Stop", prompt_id: `p${t}`, last_assistant_message: "Done." }));
      sleep(turnPause);
    });
    home.hook({ session_id: sessionId, hook_event_name: "SessionEnd" });
    const { verdict, late, unmatched, failure } = editVerdicts(home, edits, since);
    const notes = { edits: edits.length, steered: edits.filter((e) => e.steered).length, unmatched, late: late.filter((l) => l === 1).length, quiet_hooks: quiet.length };
    return [measured("W1", "replay", { verdict_ms: verdict, late }, notes, null, failure), measured("W2", "quiet-hook", { hook_ms: quiet }, { hooks: quiet.length }, null, failure)];
  } finally {
    home.stopDaemon();
  }
}

/**
 * From a SessionStart to the first edit's verdict, on whatever home and store `home` holds and with no daemon running:
 * the hook starts one. The edit is the session's first; its check is waited for (a Stop is sent after 15 s, as an
 * agent ending its turn settles every check), up to 60 s.
 */
function firstVerdict(home: Home, sessionId: string): { firstMs: number | null; startMs: number; reason: string | null } {
  const first = sessionOf(home.settings).turns[0]!.edits[0]!;
  const t0 = Date.now();
  const opened = sessionStart(home, sessionId);
  if (opened.quiet) return { firstMs: null, startMs: opened.ms, reason: `SessionStart answered {} (${lastLine(opened.stderr) || "no daemon"})` };
  home.hook({ session_id: sessionId, hook_event_name: "UserPromptSubmit", prompt_id: "p0", prompt: "Make the change." });
  const { post } = edit(home, sessionId, "toolu_first", first.path, first.content);
  let stopped = false;
  for (;;) {
    const check = home.editChecks(t0).find((c) => c.paths.includes(first.path) && c.finished !== null);
    if (check !== undefined) {
      if (check.status === "failed") return { firstMs: null, startMs: opened.ms, reason: `the first edit's check failed: ${check.failure ?? "no reason recorded"}` };
      return { firstMs: Math.max(post.end, check.finished!) - t0, startMs: opened.ms, reason: null };
    }
    const waited = Date.now() - t0;
    if (waited > 60_000) return { firstMs: null, startMs: opened.ms, reason: "the first edit had no finished check 60 s after SessionStart" };
    if (!stopped && waited > 15_000) {
      home.hook({ session_id: sessionId, hook_event_name: "Stop", prompt_id: "p0", last_assistant_message: "Done." });
      stopped = true;
    }
    sleep(50);
  }
}

/**
 * One `rerail scan --scope repository --wait`: its wall time, or, past --wait's 10 s, the request's finish; its
 * findings; and the daemon's CPU time over it and its peak RSS after it (reported, as the step benchmarks report them).
 */
function scan(home: Home, sessionId: string): { ms: number | null; withinWait: boolean; findings: number | null; reason: string | null; daemon?: { cpu_ms: number | null; peak_rss_mb: number | null } } {
  const before = home.daemonUsage();
  const result = home.cli(["scan", "--session", sessionId, "--scope", "repository", "--wait", "--json"], 3_600_000);
  const usage = (): { cpu_ms: number | null; peak_rss_mb: number | null } => {
    const after = home.daemonUsage();
    return { cpu_ms: before === null || after === null ? null : after.cpuMs - before.cpuMs, peak_rss_mb: after?.peakMb ?? null };
  };
  if (result.status !== 0 && result.status !== 2) return { ms: null, withinWait: false, findings: null, reason: `rerail scan exited ${result.status}: ${lastLine(result.stderr || result.stdout)}` };
  for (;;) {
    const [request] = home.query<{ status: string; finished_at: number | null; audit_ids_json: string }>("SELECT status, finished_at, audit_ids_json FROM audit_requests WHERE created_at >= ? ORDER BY created_at DESC LIMIT 1", result.start);
    if (request?.finished_at != null) {
      if (request.status !== "completed") return { ms: null, withinWait: false, findings: null, reason: `the scan ${request.status}` };
      const ids = JSON.parse(request.audit_ids_json) as string[];
      const [counted] = ids.length === 0 ? [] : home.query<{ n: number }>(`SELECT coalesce(sum(json_array_length(violations_json)), 0) AS n FROM audits WHERE audit_id IN (${ids.map(() => "?").join(",")})`, ...ids);
      return { ms: result.status === 0 ? result.ms : request.finished_at - result.start, withinWait: result.status === 0, findings: counted?.n ?? 0, reason: null, daemon: usage() };
    }
    if (Date.now() - result.start > 3_600_000) return { ms: null, withinWait: false, findings: null, reason: "the scan did not finish in an hour" };
    sleep(100);
  }
}

// ---------------------------------------------------------------- groups

function firstSessionGroup(settings: WaitsSettings, side: WaitSide, rep: number, phase: string): Measured[] {
  const home = new Home(settings, side, join(settings.work, `f${side[0]}`));
  home.create();
  const dropped = dropPageCache();
  try {
    const { firstMs, startMs, reason } = firstVerdict(home, `w3-${side}-${phase}-${rep}`);
    return [
      measured("W3", "first-session", { first_verdict_ms: firstMs === null ? [] : [firstMs] }, { session_start_ms: Math.round(startMs) }, dropped, reason),
      measured("W6", "autostart", { wall_ms: [startMs] }, {}, dropped, reason?.startsWith("SessionStart") === true ? reason : null),
    ];
  } finally {
    home.stopDaemon();
  }
}

function lifecycleGroup(settings: WaitsSettings, side: WaitSide, rep: number, phase: string): Measured[] {
  const home = new Home(settings, side, join(settings.work, `l${side[0]}`));
  home.create();
  const out: Measured[] = [];
  try {
    let dropped = dropPageCache();
    out.push(measured("W6", "daemon-start-first", { wall_ms: [home.startDaemon().ms] }, {}, dropped));
    const sessionId = `w5-${side}-${phase}-${rep}`;
    if (sessionStart(home, sessionId).quiet) throw new Error("SessionStart answered {}");
    dropped = dropPageCache();
    for (const [scenario, before] of [["scan-first", () => undefined], ["scan-warm", () => undefined], ["scan-switch", () => home.git(["checkout", "-q", "switch"])]] as const) {
      before();
      const { ms, withinWait, findings, reason, daemon } = scan(home, sessionId);
      out.push(measured("W5", scenario, { wall_ms: ms === null ? [] : [ms] }, { within_scan_wait: withinWait, findings, daemon_cpu_ms: daemon?.cpu_ms ?? null, daemon_peak_rss_mb: daemon?.peak_rss_mb ?? null }, scenario === "scan-first" ? dropped : null, reason));
    }
    home.git(["checkout", "-q", "main"]);
    for (const [scenario, args] of [["status", ["status"]], ["packs-list", ["packs", "list"]], ["rules-show", ["rules", "show", settings.repo.rule, "--repo", home.repo]]] as const) {
      const result = home.cli(args);
      out.push(measured("W7", scenario, { wall_ms: [result.ms] }, {}, null, result.status === 0 ? null : `rerail ${args.join(" ")} exited ${result.status}: ${lastLine(result.stderr || result.stdout)}`));
    }
    home.stopDaemon();
    out.push(measured("W6", "daemon-start-warm", { wall_ms: [home.startDaemon().ms] }));
    const opened = sessionStart(home, `${sessionId}-again`);
    out.push(measured("W6", "session-open", { wall_ms: [opened.ms] }, {}, null, opened.quiet ? "SessionStart answered {}" : null));
    return out;
  } finally {
    home.stopDaemon();
  }
}

/** The home the baseline wrote, kept at <work>/u.snapshot: one session with its edits checked, and a scan. */
function upgradeSnapshot(settings: WaitsSettings): string {
  const snapshot = join(settings.work, "u.snapshot");
  if (existsSync(join(snapshot, ".ready"))) return snapshot;
  const home = new Home(settings, "baseline", join(settings.work, "u"));
  home.create();
  try {
    const { reason } = firstVerdict(home, "w4-previous-release");
    if (reason !== null) throw new Error(`the baseline's session for W4's store: ${reason}`);
    const scanned = scan(home, "w4-previous-release");
    if (scanned.reason !== null) throw new Error(`the baseline's scan for W4's store: ${scanned.reason}`);
  } finally {
    home.stopDaemon();
  }
  rmSync(snapshot, { recursive: true, force: true });
  cpSync(home.dir, snapshot, { recursive: true });
  writeFileSync(join(snapshot, ".ready"), "");
  return snapshot;
}

function upgradeGroup(settings: WaitsSettings, side: WaitSide, rep: number, phase: string): Measured[] {
  const snapshot = upgradeSnapshot(settings);
  // Both sides read the home at the path the baseline wrote it at: the database records the repository's path.
  const dir = join(settings.work, "u");
  rmSync(dir, { recursive: true, force: true });
  cpSync(snapshot, dir, { recursive: true });
  rmSync(join(dir, ".ready"), { force: true });
  const home = new Home(settings, side, dir);
  const dropped = dropPageCache();
  try {
    const { firstMs, startMs, reason } = firstVerdict(home, `w4-${side}-${phase}-${rep}`);
    return [measured("W4", "upgrade", { first_verdict_ms: firstMs === null ? [] : [firstMs] }, { session_start_ms: Math.round(startMs), store_file: contentStoreFile(home.checkout) }, dropped, reason)];
  } finally {
    home.stopDaemon();
  }
}

// Each scan follows a branch switch so its audit has work; a warm one lasts a second or two, so a few probes fit in it.
const BUSY_STATUS_SAMPLES = 100;
const BUSY_MAX_SCANS = 100;

function busyGroup(settings: WaitsSettings, side: WaitSide, rep: number, phase: string): Measured[] {
  const home = new Home(settings, side, join(settings.work, `x${side[0]}`));
  home.create();
  const status: number[] = [];
  const notAnswering: number[] = [];
  const hook: number[] = [];
  let scans = 0;
  try {
    home.startDaemon();
    const sessionId = `w8-${side}-${phase}-${rep}`;
    if (sessionStart(home, sessionId).quiet) throw new Error("SessionStart answered {}");
    const warm = scan(home, sessionId);
    if (warm.reason !== null) throw new Error(`warm-up scan: ${warm.reason}`);
    const probe = sessionOf(settings).turns[0]!.edits[0]!.path;
    while (status.length < BUSY_STATUS_SAMPLES && scans < BUSY_MAX_SCANS) {
      home.git(["checkout", "-q", scans % 2 === 0 ? "switch" : "main"]);
      const requested = home.cli(["scan", "--session", sessionId, "--scope", "repository", "--json"]);
      if (requested.status !== 0 && requested.status !== 2) throw new Error(`rerail scan exited ${requested.status}: ${lastLine(requested.stderr || requested.stdout)}`);
      scans += 1;
      for (let i = 0; i < 400; i += 1) {
        const [request] = home.query<{ finished_at: number | null }>("SELECT finished_at FROM audit_requests WHERE created_at >= ? ORDER BY created_at DESC LIMIT 1", requested.start);
        if (request === undefined || request.finished_at !== null) break;
        const answered = home.cli(["status"], 60_000);
        status.push(answered.ms);
        notAnswering.push(answered.status === 4 || answered.status === 124 ? 1 : 0);
        const tool = { session_id: sessionId, tool_name: "Read", tool_use_id: `toolu_busy_${scans}_${i}`, tool_input: { file_path: join(home.repo, probe) } };
        hook.push(home.hook({ ...tool, hook_event_name: "PreToolUse" }).ms);
        hook.push(home.hook({ ...tool, hook_event_name: "PostToolUse", tool_response: { type: "text", file: { filePath: join(home.repo, probe) } } }).ms);
      }
    }
    const notes = { scans, samples_during_audits: status.length };
    const tooFew = status.length === 0 ? `no audit lasted long enough to probe (${scans} scans)` : null;
    return [measured("W8", "status-during-audit", { status_ms: status, not_answering: notAnswering }, notes, null, tooFew), measured("W8", "hook-during-audit", { hook_ms: hook }, notes, null, tooFew)];
  } finally {
    home.stopDaemon();
  }
}

function replayGroup(settings: WaitsSettings, side: WaitSide, rep: number, phase: string): Measured[] {
  const home = new Home(settings, side, join(settings.work, `r${side[0]}`));
  // A warm home: set up and scanned once, then every replay of this side reads the same database and store.
  if (!existsSync(join(home.dir, ".warm"))) {
    home.create();
    try {
      home.startDaemon();
      if (sessionStart(home, "warm-up").quiet) throw new Error("SessionStart answered {}");
      const warmed = scan(home, "warm-up");
      if (warmed.reason !== null) throw new Error(`warm-up scan: ${warmed.reason}`);
    } finally {
      home.stopDaemon();
    }
    writeFileSync(join(home.dir, ".warm"), "");
  }
  return replay(home, `w1-${side}-${phase}-${rep}`);
}

const stepFiles = new Map<string, string>();

/** The file the steps edit (waits-steps.ts pickStepFile), among the corpus's files of the session's language. */
function stepFile(settings: WaitsSettings): string {
  let file = stepFiles.get(settings.corpusDir);
  if (file === undefined) {
    // Any file the step edits apply to (waits-steps.ts names the languages): the corpus's own language decides.
    const paths = (readdirSync(settings.corpusDir, { recursive: true }) as string[]).filter((path) => extname(path) !== "");
    file = pickStepFile(paths.map((path) => ({ path, size: readFileSync(join(settings.corpusDir, path)).length, source: () => readFileSync(join(settings.corpusDir, path), "utf8") })));
    stepFiles.set(settings.corpusDir, file);
  }
  return file;
}

/**
 * The step benchmarks' sequence (waits-steps.ts) in one shape, on a fresh home with the page cache dropped: each step,
 * then a repository scan (W5, its wall time and findings); a file edit is made as the agent's Write through the hooks
 * and its verdict waited for (W1).
 */
function stepsGroup(settings: WaitsSettings, side: WaitSide, rep: number, phase: string, shape: Shape): Measured[] {
  const spec = SHAPES[shape];
  const file = stepFile(settings);
  const rule = settings.repo.rule;
  const home = new Home(settings, side, join(settings.work, `s${side[0]}`));
  home.create({ switchBranch: false, ...(spec.corpus === "one" ? { files: [file] } : {}), ...(spec.rules === "one" ? { oneRule: rule } : {}) });
  const out: Measured[] = [];
  const dropped = dropPageCache();
  try {
    home.startDaemon();
    const sessionId = `steps-${shape}-${side}-${phase}-${rep}`;
    if (sessionStart(home, sessionId).quiet) throw new Error("SessionStart answered {}");
    for (const step of spec.steps) {
      const scenario = `steps-${shape}-${step}`;
      const notes: WaitRun["notes"] = { file };
      if (FILE_EDITS.includes(step)) {
        const since = Date.now();
        const { pre, post } = edit(home, sessionId, `toolu_${step}`, file, editSource(step, readFileSync(join(home.repo, file), "utf8"), file));
        while (!home.editChecks(since).some((c) => c.paths.includes(file) && c.finished !== null) && Date.now() - since < 60_000) sleep(50);
        const { verdict, late, unmatched, failure } = editVerdicts(home, [timing(file, pre, post)], since);
        out.push(measured("W1", scenario, { verdict_ms: unmatched > 0 ? [] : verdict }, { late: late[0] === 1, steered: post.steered }, null, failure ?? (unmatched > 0 ? "the edit had no finished check within 60 s" : null)));
      } else if (step === "rule") {
        const pack = readdirSync(join(home.dir, "packs-src")).find((p) => existsSync(join(home.packSource(p), "rules", `${rule}.json`)));
        if (pack === undefined) throw new Error(`no installed pack holds rule ${rule}`);
        const document = join(home.packSource(pack), "rules", `${rule}.json`);
        writeFileSync(document, editRuleDocument(readFileSync(document, "utf8")));
        const t0 = performance.now();
        home.addPack(pack);
        notes.add_and_approve_ms = Math.round(performance.now() - t0);
      }
      const scanned = scan(home, sessionId);
      const usage = { daemon_cpu_ms: scanned.daemon?.cpu_ms ?? null, daemon_peak_rss_mb: scanned.daemon?.peak_rss_mb ?? null };
      out.push(measured("W5", scenario, { wall_ms: scanned.ms === null ? [] : [scanned.ms] }, { ...notes, findings: scanned.findings, within_scan_wait: scanned.withinWait, ...usage }, step === "cold" ? dropped : null, scanned.reason));
    }
    return out;
  } finally {
    home.stopDaemon();
  }
}

const GROUPS: Record<Exclude<Group, `steps:${string}`>, (settings: WaitsSettings, side: WaitSide, rep: number, phase: string) => Measured[]> = {
  replay: replayGroup,
  "first-session": firstSessionGroup,
  lifecycle: lifecycleGroup,
  upgrade: upgradeGroup,
  busy: busyGroup,
};

// ---------------------------------------------------------------- running

function runsPath(out: string): string {
  return join(out, "waits", "runs.jsonl");
}

export function readWaitRuns(out: string): WaitRun[] {
  const path = runsPath(out);
  return existsSync(path) ? readFileSync(path, "utf8").split("\n").filter((line) => line !== "").map((line) => JSON.parse(line) as WaitRun) : [];
}

/** The group's runs of one phase, the side order alternating per repetition; repetitions already logged are kept. */
function runGroup(settings: WaitsSettings, machine: Machine, group: Group, phase: "runs" | "confirm", reps: number): void {
  const donePath = join(settings.out, "waits", "done.txt");
  const done = new Set(existsSync(donePath) ? readFileSync(donePath, "utf8").split("\n") : []);
  const scenarios = SCENARIOS.filter((spec) => groupOf(spec) === group);
  for (let rep = 1; rep <= reps; rep += 1) {
    const sides: WaitSide[] = rep % 2 === 1 ? ["baseline", "candidate"] : ["candidate", "baseline"];
    for (const side of sides) {
      const tag = `${group}/${phase}/${rep}/${side}`;
      if (done.has(tag)) continue;
      const load = Number(readFileSync("/proc/loadavg", "utf8").split(" ")[0]);
      const before = machine.cpuTimes();
      const started = new Date().toISOString();
      let rows: Measured[];
      try {
        rows = group.startsWith("steps:") ? stepsGroup(settings, side, rep, phase, group.slice("steps:".length) as Shape) : GROUPS[group as keyof typeof GROUPS](settings, side, rep, phase);
      } catch (error) {
        const reason = error instanceof Error ? error.message : String(error);
        rows = scenarios.map((spec) => measured(spec.wait, spec.scenario, {}, {}, null, reason));
      }
      const after = machine.cpuTimes();
      const common = { side, phase, rep, load, pinnedBusy: machine.busiest(before, after, settings.cpus), siblingBusy: machine.busiest(before, after, settings.siblings), started, host: hostname(), cpus: settings.cpus.join(",") };
      for (const row of rows) {
        const run: WaitRun = { ...row, ...common };
        appendFileSync(runsPath(settings.out), `${JSON.stringify(run)}\n`);
        const figure = Object.entries(row.samples).map(([name, values]) => `${name} ${values.length === 1 ? Math.round(values[0]!) : `n=${values.length}`}`).join(", ");
        console.log(`shipbar: ${scenarioId(row)} ${tag}: ${row.ok ? figure : `FAILED (${row.reason})`} (load ${load})`);
      }
      appendFileSync(donePath, `${tag}\n`);
    }
  }
}

/** Every requested scenario's group, then confirmation repetitions of the scenarios that fail. */
export function runWaits(settings: WaitsSettings, machine: Machine): void {
  mkdirSync(join(settings.out, "waits"), { recursive: true });
  writeFileSync(join(settings.out, "waits", "settings.json"), JSON.stringify(recordedSettings(settings), null, 1));
  const requested = SCENARIOS.filter((spec) => settings.scenarios.includes(scenarioId(spec)));
  const groups = [...new Set(requested.map(groupOf))];
  const repsOf = (group: Group): number => (group === "replay" || group === "busy" ? settings.replays : settings.runs);
  for (const group of groups) runGroup(settings, machine, group, "runs", repsOf(group));
  if (settings.confirm === 0) return;
  const failing = failingScenarios(readWaitRuns(settings.out), { limits: settings.limits }).filter((spec) => settings.scenarios.includes(scenarioId(spec)));
  for (const group of [...new Set(failing.map(groupOf))]) runGroup(settings, machine, group, "confirm", group === "replay" || group === "busy" ? settings.replays : settings.confirm);
}

/** What `runWaits` records of its settings in <out>/waits/settings.json, for the report. */
interface RecordedSettings {
  baseline: { dir: string; sha: string };
  candidate: { dir: string; sha: string };
  repo: { name: string; corpus: string; packs: string[]; files: number; sha256: string };
  session: { name: string; source: string; edits: number; sha256: string } | null;
  cpus: string;
  runs: number;
  replays: number;
  confirm: number;
  scenarios: string[];
  limits: Record<string, number | null>;
  reportOnly: string[];
  command: string;
}

function recordedSettings(s: WaitsSettings): RecordedSettings {
  return {
    baseline: s.checkouts.baseline,
    candidate: s.checkouts.candidate,
    repo: { name: s.repoName, corpus: s.repo.corpus, packs: s.repo.packs, ...s.corpus },
    session: s.session === null ? null : { name: s.sessionName!, source: `${s.session.source} ${s.session.from.slice(0, 10)}..${s.session.to.slice(0, 10)}`, edits: s.session.turns.reduce((sum, t) => sum + t.edits.length, 0), sha256: s.sessionSha256! },
    cpus: s.cpus.join(","),
    runs: s.runs,
    replays: s.replays,
    confirm: s.confirm,
    scenarios: s.scenarios,
    limits: s.limits,
    reportOnly: s.reportOnly,
    command: s.command,
  };
}

/** The report's waits section from <out>/waits, or null when no waits were run there. */
export function waitsSection(out: string): WaitsSection | null {
  const path = join(out, "waits", "settings.json");
  if (!existsSync(path)) return null;
  const s = JSON.parse(readFileSync(path, "utf8")) as RecordedSettings;
  const notRequested = Object.fromEntries(SCENARIOS.filter((spec) => !s.scenarios.includes(scenarioId(spec))).map((spec) => [scenarioId(spec), "not requested in this run (--waits)"]));
  return buildWaitsSection(readWaitRuns(out), {
    limits: s.limits,
    reportOnly: s.reportOnly,
    requested: s.scenarios,
    skipped: { ...notRequested, ...NOT_MEASURED },
    inputs: {
      sessions: s.session === null ? {} : { [s.session.name]: { edits: s.session.edits, sha256: s.session.sha256, source: s.session.source } },
      repos: { [s.repo.name]: { corpus: s.repo.corpus, packs: s.repo.packs, files: s.repo.files, sha256: s.repo.sha256 } },
    },
    method: [
      `each side in its own fresh HOME, Rerail home and content store, set up through \`rerail packs add\`, \`rerail rules approve\` and \`rerail start --packs ${s.repo.packs.join(",")}\``,
      `entry points: the Claude Code hook (\`node dist/hosts/claude/claude-hook.js\`, one process per event), \`node dist/cli/cli.js\`, and the daemon they start; all on CPUs ${s.cpus}`,
      `single waits: median of ${s.runs} runs per side, interleaved; distributions: ${s.replays} replays per side pooled, p50 and p95 fail only when the bootstrap 95% CI of the ratio lies above the bar`,
      `confirmation: a failing scenario runs ${s.confirm} more repetitions (distributions: ${s.replays} more replays)`,
    ],
    reproduce: (spec) => `${s.command.replace(/--waits\s+\S+/, "")} --waits ${scenarioId(spec).replace(/^wait\//, "")}`.replace(/\s+/g, " "),
  });
}
