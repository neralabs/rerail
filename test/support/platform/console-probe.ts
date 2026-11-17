/**
 * A probe of the console a process has on Windows when its test starts it as Rerail starts the daemon: a script run
 * under Node with tsx that says whether its process has a console, starts two Git processes, and waits until it is told
 * to end. `docs/windows.md` ("The daemon's console") says what the test of it checks.
 */
import { execFileSync } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve, win32 } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath, pathToFileURL } from "node:url";

/** The checkout this file is in, where the probe runs, so that `--import tsx` and the Git module's source resolve. */
const CHECKOUT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");

/** How a test starts the probe: Node with `args`, in `cwd`. */
export interface ProbeLaunch {
  args: string[];
  cwd: string;
}

/** The probe as its test started it. */
export interface StartedProbe {
  ended(): boolean;
  /** Ends it, when it has not ended 10 s after it was told to. */
  kill(): void;
  /** What it wrote to its output, for the failure when it never answers. */
  output(): string | Promise<string>;
}

/**
 * Whether the probe's process had a console, and how many console hosts (conhost.exe) ran under each Git it started:
 * what Windows did, read without asking Rerail's child-process helper.
 */
export interface ProbeSight {
  attached: boolean;
  consoleHosts: number[];
}

interface ProbeAnswer {
  attached: boolean;
  gits: number[];
}

/**
 * The probe script, for `node --import tsx <script> <answer> <done>`. It writes to `<answer>` whether its process has
 * a console, which it reads by opening `CONOUT$`, and the pids of two Git processes that wait for their input: one
 * started with no `windowsHide`, as a library Rerail does not control could start one, and one with Node's
 * `windowsHide: true`, as Rerail's helper starts every child. It ends them, and itself, once `<done>` exists.
 */
function probeScript(): string {
  const git = JSON.stringify(pathToFileURL(join(CHECKOUT, "src", "engine", "git.ts")).href);
  return `
import { spawn } from "node:child_process";
import { closeSync, existsSync, openSync, writeFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { gitExecutable } from ${git};
const [answer, done] = process.argv.slice(2);
const attached = () => {
  try {
    closeSync(openSync(${JSON.stringify(String.raw`\\.\CONOUT$`)}, "r+"));
    return true;
  } catch {
    return false;
  }
};
// hash-object --stdin runs outside a repository and waits for the end of its input.
const args = ["hash-object", "--stdin"];
const gits = [
  spawn(gitExecutable(), args, { stdio: ["pipe", "ignore", "ignore"] }),
  spawn(gitExecutable(), args, { stdio: ["pipe", "ignore", "ignore"], windowsHide: true }),
];
writeFileSync(answer, JSON.stringify({ attached: attached(), gits: gits.map(git => git.pid) }));
while (!existsSync(done)) await sleep(50);
for (const git of gits) git.stdin.end();
`;
}

/** What the probe wrote to `answer`, once it has; its output says why when it never does. */
async function probeAnswer(answer: string, output: () => string | Promise<string>): Promise<ProbeAnswer> {
  const deadline = Date.now() + 40_000;
  while (Date.now() < deadline) {
    try {
      // The probe writes this shape; the test's expectation checks it.
      return JSON.parse(await readFile(answer, "utf8")) as ProbeAnswer;
    } catch {
      // Not written yet, or not whole yet.
      await sleep(100);
    }
  }
  throw new Error(`the probe did not answer in 40 s: ${await output()}`);
}

/** Tells the probe to end and gives it 10 s, then kills it: a probe that hangs never outlives its test. */
async function endProbe(done: string, probe: StartedProbe): Promise<void> {
  await writeFile(done, "");
  const deadline = Date.now() + 10_000;
  while (!probe.ended() && Date.now() < deadline) await sleep(50);
  if (probe.ended()) return;
  try {
    probe.kill();
  } catch { /* It ended meanwhile. */ }
}

/** A row of the process table: the parent's pid, when the process started (Unix time in ms), and its image name. */
interface ProcessRow {
  parent: number;
  started: number;
  name: string;
}

/** Every process on this machine by pid, from one read of the process table. */
function processTable(): Map<number, ProcessRow> {
  const powershell = win32.join(process.env.SystemRoot ?? String.raw`C:\Windows`, "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
  const rows = execFileSync(powershell, [
    "-NoProfile", "-NonInteractive", "-Command",
    "Get-CimInstance Win32_Process | ForEach-Object { '{0} {1} {2} {3}' -f $_.ProcessId, $_.ParentProcessId, $(if ($_.CreationDate) { ([DateTimeOffset]$_.CreationDate).ToUnixTimeMilliseconds() } else { 0 }), $_.Name }",
  ], { encoding: "utf8", windowsHide: true });
  const processes = new Map<number, ProcessRow>();
  for (const row of rows.split(/\r?\n/u)) {
    const [pid, parent, started, ...name] = row.trim().split(" ");
    if (pid !== undefined && pid !== "") processes.set(Number(pid), { parent: Number(parent), started: Number(started), name: name.join(" ") });
  }
  return processes;
}

/**
 * Whether `pid` is a child of `parent` in `processes`. Windows reuses pids, and a process keeps its parent's pid after
 * that parent has gone, so a process names as its parent one that started after it only when its own parent ended and
 * the pid went to another: a console host that outlived the process that made it, for one.
 */
function childOf(pid: number, parent: number, processes: ReadonlyMap<number, ProcessRow>): boolean {
  const row = processes.get(pid);
  const parentRow = processes.get(parent);
  return row !== undefined && parentRow !== undefined && row.parent === parent && row.started >= parentRow.started;
}

/** `root` and every process under it in `processes`. */
function treeUnder(root: number, processes: ReadonlyMap<number, ProcessRow>): Set<number> {
  const tree = new Set([root]);
  for (let grew = true; grew;) {
    grew = false;
    for (const [pid, { parent }] of processes) {
      if (tree.has(pid) || !tree.has(parent) || !childOf(pid, parent, processes)) continue;
      tree.add(pid);
      grew = true;
    }
  }
  return tree;
}

/** How many console hosts (conhost.exe) run under each of `pids`; Git for Windows' launcher runs the real Git below it. */
function consoleHostsUnder(pids: readonly number[]): number[] {
  if (pids.length === 0) return [];
  const processes = processTable();
  return pids.map(root => [...treeUnder(root, processes)].filter(pid => processes.get(pid)?.name === "conhost.exe").length);
}

/**
 * Writes the probe into `folder`, has `start` run it, and answers whether it had a console and the console hosts under
 * each Git it started, counted while those Gits still run. The probe has ended, or been killed, when this returns.
 */
export async function runConsoleProbe(folder: string, start: (launch: ProbeLaunch) => StartedProbe): Promise<ProbeSight> {
  const [script, answer, done] = [join(folder, "console-probe.mts"), join(folder, "answer.json"), join(folder, "done")];
  await writeFile(script, probeScript());
  const probe = start({ args: ["--import", "tsx", script, answer, done], cwd: CHECKOUT });
  try {
    const { attached, gits } = await probeAnswer(answer, () => probe.output());
    return { attached, consoleHosts: consoleHostsUnder(gits) };
  } finally {
    await endProbe(done, probe);
  }
}
