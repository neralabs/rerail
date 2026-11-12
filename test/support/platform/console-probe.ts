/**
 * A probe of how the child-process helper starts children on Windows, in a process its test starts: a script run under
 * Node with tsx that loads this checkout's `src/engine/child-process.ts`, says whether its process has a console and
 * shares it with its children, starts Git when asked, and waits until it is told to end. `docs/windows.md` ("The
 * daemon's console") says what the tests of it check.
 */
import { execFileSync } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve, win32 } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath, pathToFileURL } from "node:url";

/** The checkout this file is in, where the probe runs, so that `--import tsx` and the helper's source resolve. */
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

/** What the probe saw in its process, and how many console hosts (conhost.exe) ran under each Git it started. */
export interface ProbeSight {
  attached: boolean;
  shares: boolean;
  consoleHosts: number[];
}

interface ProbeAnswer {
  attached: boolean;
  shares: boolean;
  gits: number[];
}

/**
 * The probe script, for `node --import tsx <script> <answer> <done> <gits|none>`. It writes to `<answer>` whether the
 * helper sees a console and shares it, and with `gits` the pids of two Git processes that wait for their input: one
 * started through the helper, one with Node's `windowsHide: true`, as the helper starts a child in a process that does
 * not share its console. It ends them, and itself, once `<done>` exists.
 */
function probeScript(): string {
  const source = (path: string): string => JSON.stringify(pathToFileURL(join(CHECKOUT, "src", "engine", path)).href);
  return `
import { spawn as nodeSpawn } from "node:child_process";
import { existsSync, writeFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import * as helper from ${source("child-process.ts")};
import { gitExecutable } from ${source("git.ts")};
const [answer, done, mode] = process.argv.slice(2);
const gits = [];
if (mode === "gits") {
  // hash-object --stdin runs outside a repository and waits for the end of its input.
  const args = ["hash-object", "--stdin"];
  gits.push(helper.spawn(gitExecutable(), args, { stdio: ["pipe", "ignore", "ignore"] }));
  gits.push(nodeSpawn(gitExecutable(), args, { stdio: ["pipe", "ignore", "ignore"], windowsHide: true }));
}
writeFileSync(answer, JSON.stringify({ attached: helper.hasConsole(), shares: helper.sharesConsole(), gits: gits.map(git => git.pid) }));
while (!existsSync(done)) await sleep(50);
for (const git of gits) git.stdin.end();
`;
}

/** What the probe wrote to `answer`, once it has; its output says why when it never does. */
async function probeAnswer(answer: string, output: () => string | Promise<string>): Promise<ProbeAnswer> {
  const deadline = Date.now() + 40_000;
  while (Date.now() < deadline) {
    try {
      return JSON.parse(await readFile(answer, "utf8")) as ProbeAnswer;
    } catch {
      // Not written yet, or not whole yet.
      await sleep(100);
    }
  }
  throw new Error(`the probe did not answer: ${await output()}`);
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

/** Every process on this machine by pid, with its parent and image name, from one read of the process table. */
function processTable(): Map<number, { parent: number; name: string }> {
  const powershell = win32.join(process.env.SystemRoot ?? String.raw`C:\Windows`, "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
  const rows = execFileSync(powershell, [
    "-NoProfile", "-NonInteractive", "-Command",
    "Get-CimInstance Win32_Process | ForEach-Object { '{0} {1} {2}' -f $_.ProcessId, $_.ParentProcessId, $_.Name }",
  ], { encoding: "utf8", windowsHide: true });
  const processes = new Map<number, { parent: number; name: string }>();
  for (const row of rows.split(/\r?\n/u)) {
    const [pid, parent, ...name] = row.trim().split(" ");
    if (pid !== undefined && pid !== "") processes.set(Number(pid), { parent: Number(parent), name: name.join(" ") });
  }
  return processes;
}

/** `root` and every process under it in `processes`. */
function treeUnder(root: number, processes: ReadonlyMap<number, { parent: number }>): Set<number> {
  const tree = new Set([root]);
  for (let grew = true; grew;) {
    grew = false;
    for (const [pid, { parent }] of processes) {
      if (tree.has(pid) || !tree.has(parent)) continue;
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
 * Writes the probe into `folder`, has `start` run it, and answers what it saw, with the console hosts under the Git it
 * started when `startGit`, counted while that Git still runs. The probe has ended, or been killed, when this returns.
 */
export async function runConsoleProbe(folder: string, startGit: boolean, start: (launch: ProbeLaunch) => StartedProbe): Promise<ProbeSight> {
  const [script, answer, done] = [join(folder, "console-probe.mts"), join(folder, "answer.json"), join(folder, "done")];
  await writeFile(script, probeScript());
  const probe = start({ args: ["--import", "tsx", script, answer, done, startGit ? "gits" : "none"], cwd: CHECKOUT });
  try {
    const { attached, shares, gits } = await probeAnswer(answer, () => probe.output());
    return { attached, shares, consoleHosts: consoleHostsUnder(gits) };
  } finally {
    await endProbe(done, probe);
  }
}
