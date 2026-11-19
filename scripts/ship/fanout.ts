// The shippability bar spread over several Linux hosts (`shipbar.ts fanout`, docs in shipbar.ts's header). This
// module holds what the fan-out decides and moves, apart from the measuring itself (shipbar.ts's `host` and `work`):
//
//   plan     every unit of the three slices, sized so no unit outlasts about UNIT_BUDGET_S on one worker (slice A by
//            repetition: main and every candidate, cold then warm, one repetition per unit; slice B by chunks of
//            rules, each rule timed on main and on every candidate that has it; slice C by file, every candidate),
//            assigned to hosts longest first onto the least loaded (LPT). An A cell's repetitions share a host, so its
//            confirmation runs there with all of its runs at hand.
//   merge    each host's out/ pulled home, merged into one runs/ (logs concatenated, other files copied once), from
//            which `verdict` reports.
//   remote   ssh and rsync through one remote shell (`ssh` unless --ssh or SHIPBAR_SSH names another command, a
//            wrapper that reaches the hosts its own way, say), each call with a timeout: everything long runs
//            detached on the hosts (setsid nohup), and the Mac only starts it and polls.

import { spawn } from "node:child_process";
import { appendFileSync, copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import type { Candidate, Corpus } from "./shipbar.js";
import type { FileLabel } from "./verdict.js";

/** A unit's planned length on one worker, at most (seconds): the longest unit bounds the measure phase. */
export const UNIT_BUDGET_S = 240;

// Planning estimates, seconds, from the 2026-11-19 four-host run on c7i.8xlarge (4 cores per worker): one cold
// whole-pack run per side (A), one rule-alone run (B), one one-file run (C). They order and spread the work; a wrong
// one costs balance, never a result.
const A_COLD_S: Record<string, number> = { "python/real": 140, "python/fixtures": 17, "react/real": 27, "javascript/real": 17, "node/real": 15, "typescript/real": 10, "node/fixtures": 7, "rust/real": 8, "javascript/fixtures": 5 };
const B_RUN_S: Record<string, number> = { "python/real": 1.7, "python/fixtures": 1.0, "javascript/fixtures": 1.3, "javascript/real": 1.3, "node/real": 1.5, "react/real": 2.1, "typescript/real": 1.4, "typescript/fixtures": 0.9 };
const C_RUN_S: Record<string, number> = { python: 3 };
/** The empty rule set's run: start-up. */
const STARTUP_S = 0.8;

export interface PackInput {
  pack: string;
  /** The corpora the pack has (a pack without an upstream recording has no fixtures). */
  corpora: Corpus[];
  /** Slice B's rules: "(none)" (the empty rule set) first, then every rule main ships that a candidate has. */
  rules: string[];
  /** Slice C's files. */
  files: FileLabel[];
}

export interface Unit {
  id: string;
  slice: "A" | "B" | "C";
  pack: string;
  corpus: Corpus;
  /** A: the repetition. */
  rep?: number;
  /** B: the rules timed alone, in order. */
  rules?: string[];
  /** C: the file. */
  label?: FileLabel;
  /** Planned seconds on one worker. */
  est: number;
  /** Units of one group go to one host: an A cell's repetitions. */
  group: string;
}

export interface PlannedUnit extends Unit {
  host: string;
}

export interface Plan {
  format: 1;
  /** The candidates every unit measures beside main. */
  candidates: Candidate[];
  hosts: string[];
  /** Each host's workers: one CPU set each. */
  workers: string[];
  units: PlannedUnit[];
  /** Slice A's confirmation, per cell, on the host its repetitions ran on, after every worker there is done. */
  confirm: { id: string; pack: string; corpus: Corpus; host: string }[];
  /** Planned seconds per host: its units' sum over its workers, or its longest unit where that is longer. */
  load: Record<string, number>;
}

export interface PlanOptions {
  runs: number;
  ruleRuns: number;
  fileRuns: number;
  /** The candidates measured beside main: each unit runs every side. */
  candidates: readonly Candidate[];
  unitBudgetS?: number;
}

/** Every unit of the three slices: A by repetition, B by chunks of rules within the budget, C by file. */
export function planUnits(packs: readonly PackInput[], o: PlanOptions): Unit[] {
  const budget = o.unitBudgetS ?? UNIT_BUDGET_S;
  const sides = 1 + o.candidates.length;
  // The product sides' empty rule set beside each A repetition (fixed.jsonl): the `.rerail` side has none.
  const productSides = 1 + o.candidates.filter((c) => c !== "rr").length;
  const units: Unit[] = [];
  for (const p of packs) {
    for (const corpus of p.corpora) {
      const key = `${p.pack}/${corpus}`;
      const a = A_COLD_S[key] ?? 1.2;
      for (let rep = 1; rep <= o.runs; rep += 1) {
        // Each side cold then warm, and the empty rule set's cold then warm beside it.
        units.push({ id: `A|${p.pack}|${corpus}|r${rep}`, slice: "A", pack: p.pack, corpus, rep, est: sides * 2 * a + productSides * 2 * STARTUP_S, group: `A|${p.pack}|${corpus}` });
      }
      const perRule = o.ruleRuns * sides * (B_RUN_S[key] ?? 0.8);
      const size = Math.max(1, Math.floor(budget / perRule));
      for (let i = 0; i < p.rules.length; i += size) {
        const rules = p.rules.slice(i, i + size);
        const id = `B|${p.pack}|${corpus}|${String(i / size).padStart(3, "0")}`;
        units.push({ id, slice: "B", pack: p.pack, corpus, rules, est: rules.length * perRule, group: id });
      }
    }
    for (const label of p.files) {
      const id = `C|${p.pack}|${label}`;
      units.push({ id, slice: "C", pack: p.pack, corpus: "real", label, est: o.fileRuns * sides * 2 * (C_RUN_S[p.pack] ?? 1.5), group: id });
    }
  }
  return units;
}

/**
 * The units over the hosts, longest group first onto the host that would finish it soonest (LPT): a host's load is
 * its units' sum over its workers, or its longest unit where that is longer.
 */
export function planFanout(packs: readonly PackInput[], hosts: readonly string[], workers: readonly string[], o: PlanOptions): Plan {
  if (hosts.length === 0 || workers.length === 0) throw new Error("a fan-out needs at least one host and one worker");
  const units = planUnits(packs, o);
  const groups = new Map<string, Unit[]>();
  for (const u of units) groups.set(u.group, [...(groups.get(u.group) ?? []), u]);
  const sum = new Map(hosts.map((h) => [h, 0]));
  const longest = new Map(hosts.map((h) => [h, 0]));
  const finish = (h: string, extra = 0, unit = 0): number => Math.max((sum.get(h)! + extra) / workers.length, Math.max(longest.get(h)!, unit));
  const planned: PlannedUnit[] = [];
  const ordered = [...groups.values()].sort((a, b) => b.reduce((s, u) => s + u.est, 0) - a.reduce((s, u) => s + u.est, 0) || (a[0]!.id < b[0]!.id ? -1 : 1));
  for (const group of ordered) {
    const total = group.reduce((s, u) => s + u.est, 0);
    const longestUnit = Math.max(...group.map((u) => u.est));
    const host = hosts.reduce((best, h) => (finish(h, total, longestUnit) < finish(best, total, longestUnit) ? h : best));
    sum.set(host, sum.get(host)! + total);
    longest.set(host, Math.max(longest.get(host)!, longestUnit));
    for (const u of group) planned.push({ ...u, host });
  }
  planned.sort((a, b) => b.est - a.est || (a.id < b.id ? -1 : 1));
  const cells = new Map<string, { id: string; pack: string; corpus: Corpus; host: string }>();
  for (const u of planned) if (u.slice === "A") cells.set(u.group, { id: `A-confirm|${u.pack}|${u.corpus}`, pack: u.pack, corpus: u.corpus, host: u.host });
  return { format: 1, candidates: [...o.candidates], hosts: [...hosts], workers: [...workers], units: planned, confirm: [...cells.values()].sort((a, b) => (a.id < b.id ? -1 : 1)), load: Object.fromEntries(hosts.map((h) => [h, Math.round(finish(h))])) };
}

/** A unit's claim: a directory name, made atomically by the worker that takes the unit. */
export function claimName(id: string): string {
  return id.replace(/[^A-Za-z0-9._-]/g, "_");
}

// ---------------------------------------------------------------- merge

/**
 * Each host's out/ merged into one: a .jsonl log's lines appended where the merged log lacks them (a cell's rule
 * chunks may have run on several hosts), any other file copied where the merged tree lacks it. Returns the files
 * that differed between hosts and were kept from the first.
 */
export function mergeTrees(sources: readonly string[], into: string): string[] {
  const conflicts: string[] = [];
  for (const source of sources) {
    const walk = (directory: string): void => {
      for (const entry of readdirSync(directory, { withFileTypes: true })) {
        const from = join(directory, entry.name);
        if (entry.isDirectory()) {
          walk(from);
          continue;
        }
        if (!entry.isFile()) continue;
        const to = join(into, relative(source, from));
        mkdirSync(dirname(to), { recursive: true });
        if (entry.name.endsWith(".jsonl")) {
          const have = new Set(existsSync(to) ? readFileSync(to, "utf8").split("\n").filter((l) => l !== "") : []);
          // Only whole lines: a host's last line still being written has no newline yet.
          const lines = readFileSync(from, "utf8").split("\n").slice(0, -1).filter((l) => l !== "" && !have.has(l));
          if (lines.length > 0) appendFileSync(to, `${lines.join("\n")}\n`);
        } else if (!existsSync(to)) {
          copyFileSync(from, to);
        } else if (!readFileSync(to).equals(readFileSync(from))) {
          conflicts.push(relative(into, to));
        }
      }
    };
    if (existsSync(source)) walk(source);
  }
  return conflicts;
}

// ---------------------------------------------------------------- remote

export interface Done {
  status: number;
  stdout: string;
  stderr: string;
}

/**
 * A process with a deadline: at `timeoutS` its whole process group is killed (an ssh through SSM leaves helpers
 * behind otherwise), its status then 124. Never throws.
 */
export function exec(command: string, args: readonly string[], timeoutS: number): Promise<Done> {
  const { promise, resolve: done } = Promise.withResolvers<Done>();
  const child = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"], detached: true });
  let stdout = "";
  let stderr = "";
  let timedOut = false;
  child.stdout.on("data", (d: Buffer) => (stdout += d.toString()));
  child.stderr.on("data", (d: Buffer) => (stderr += d.toString()));
  const timer = setTimeout(() => {
    timedOut = true;
    try {
      process.kill(-child.pid!, "SIGKILL");
    } catch {
      child.kill("SIGKILL");
    }
  }, timeoutS * 1000);
  child.on("error", (error) => {
    clearTimeout(timer);
    done({ status: 127, stdout, stderr: `${stderr}${String(error)}` });
  });
  child.on("close", (code) => {
    clearTimeout(timer);
    done({ status: timedOut ? 124 : (code ?? 1), stdout, stderr: timedOut ? `${stderr}timed out after ${timeoutS} s` : stderr });
  });
  return promise;
}

/** The remote shell every call goes through: `--ssh`, else $SHIPBAR_SSH, else plain `ssh`, as words. */
export function remoteShell(option: string | undefined, env: NodeJS.ProcessEnv = process.env): string[] {
  const words = (option ?? env.SHIPBAR_SSH ?? "ssh").trim().split(/\s+/).filter((w) => w !== "");
  if (words.length === 0) throw new Error("the remote shell command is empty (--ssh or SHIPBAR_SSH)");
  return words;
}

/** One command on a host through the remote shell, run from the remote home directory, with a short deadline. */
export function ssh(shell: readonly string[], host: string, command: string, timeoutS = 90): Promise<Done> {
  return exec(shell[0]!, [...shell.slice(1), host, command], timeoutS);
}

/** rsync over the remote shell; `from` and `to` as rsync takes them (`<host>:<path relative to ~>` remote). */
export function rsync(shell: readonly string[], from: string, to: string, extra: readonly string[], timeoutS: number): Promise<Done> {
  return exec("rsync", ["-az", "-e", shell.join(" "), ...extra, from, to], timeoutS);
}

/**
 * The remote command that starts `script` (a host script under <dir>/fanout/) detached from the remote shell: in its own
 * session, immune to hangups, its output in fanout/<script>.out, so the shell returns at once and a dropped connection
 * leaves it running. A login shell, so the host's toolchains are on its PATH.
 */
export function detached(dir: string, script: string, args: readonly string[]): string {
  const quote = (arg: string): string => `'${arg.replaceAll("'", "'\\''")}'`;
  return `cd ${dir} && (setsid nohup bash -l fanout/${script} ${args.map(quote).join(" ")} > fanout/${script}.out 2>&1 < /dev/null &)`;
}

/** `attempt` until it succeeds or `tries` run out (a stalled call is killed by its own deadline, then tried again). */
export async function retry(label: string, tries: number, attempt: () => Promise<Done>, log: (line: string) => void): Promise<Done> {
  let last: Done = { status: 1, stdout: "", stderr: "not tried" };
  for (let i = 1; i <= tries; i += 1) {
    last = await attempt();
    if (last.status === 0) return last;
    log(`${label}: attempt ${i} of ${tries} failed (${last.status}): ${last.stderr.trim().split("\n").at(-1) ?? ""}`);
  }
  return last;
}
