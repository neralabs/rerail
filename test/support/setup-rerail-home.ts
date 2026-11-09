import { afterAll } from "vitest";
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdtempSync, rmSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { watchRealAgentSettings } from "./user-home.js";

const priorRerailHome = process.env.RERAIL_HOME;
const priorContentStorePath = process.env.RERAIL_CONTENT_STORE_PATH;
const priorUserHome = { HOME: process.env.HOME, USERPROFILE: process.env.USERPROFILE };
const priorGitConfig = join(homedir(), ".gitconfig");
// The person's own agent settings, watched for this file's whole run: a test that writes a temp folder into them
// fails the run rather than leaving the person's coding agents pointing at a folder that is about to be deleted.
const realSettingsLeaks = watchRealAgentSettings(homedir(), tmpdir());
const rerailHome = mkdtempSync(join(tmpdir(), "rerail-vitest-home-"));
const contentStoreDirectory = mkdtempSync(join(tmpdir(), "rerail-vitest-store-"));
const userHome = mkdtempSync(join(tmpdir(), "rerail-vitest-user-"));

process.env.RERAIL_HOME = rerailHome;
// Every test file gets a user home of its own, so code that falls back to os.homedir() (harness registration, the
// Claude status store, git's global config) never reaches the person's profile. os.homedir() reads HOME on POSIX and
// USERPROFILE on Windows, so both are set; a test that moves the home uses stubUserHome, which moves both.
process.env.HOME = userHome;
process.env.USERPROFILE = userHome;
// Git still reads the person's global settings (safe.directory, line endings) through a copy, so tests that run git in
// this checkout behave as before, and a test that writes global settings changes only the copy.
if (existsSync(priorGitConfig)) copyFileSync(priorGitConfig, join(userHome, ".gitconfig"));
// A test run is a dev build, whose store would otherwise be this checkout's `.rerail/content-v1.sqlite`, shared by
// every test file; each file gets its own (`contentStorePath` honours the override).
process.env.RERAIL_CONTENT_STORE_PATH = join(contentStoreDirectory, "content.sqlite");

/**
 * Daemons this test file spawned that are still running. A spawned daemon is detached but keeps this worker as its
 * parent until the worker exits, after which nothing would ever stop it. Windows launches through the native binding
 * and has no `ps`; it is not checked.
 */
function strandedDaemons(): Array<{ pid: number; home: string }> {
  if (process.platform === "win32") return [];
  const stranded: Array<{ pid: number; home: string }> = [];
  for (const line of execFileSync("ps", ["-A", "-o", "pid=,ppid=,command="], { encoding: "utf8" }).split("\n")) {
    const match = /^\s*(\d+)\s+(\d+)\s+.*[\\/]dist[\\/]daemon\.js --state-file (.+?)(?: --|$)/u.exec(line);
    if (match !== null && Number(match[2]) === process.pid) stranded.push({ pid: Number(match[1]), home: dirname(match[3]!) });
  }
  return stranded;
}

/** Sends `signal` to `pid`; false when the process is already gone. */
function signalProcess(pid: number, signal: NodeJS.Signals | 0): boolean {
  try {
    process.kill(pid, signal);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ESRCH") return false;
    throw error;
  }
}

/**
 * Stops the process `ps` found rather than going through `stopDaemon(home)`: that signals whatever pid the home's
 * state file records, which a test may have pointed at another process, and a test file's `vi.mock` can replace it.
 */
async function stopProcess(pid: number): Promise<void> {
  if (!signalProcess(pid, "SIGTERM")) return;
  const deadline = Date.now() + 2_000;
  while (signalProcess(pid, 0) && Date.now() < deadline) await sleep(25);
  signalProcess(pid, "SIGKILL");
}

afterAll(async () => {
  try {
    const leaked = realSettingsLeaks();
    if (leaked.length > 0) {
      throw new Error(`A test wrote temp folders into the real coding agent settings: ${leaked.join(", ")}. Restore them with rerail install --refresh, and move the test's home with stubUserHome.`);
    }
    const stranded = strandedDaemons();
    if (stranded.length > 0) {
      for (const daemon of stranded) await stopProcess(daemon.pid);
      throw new Error(`Test file left ${stranded.length} Rerail daemon(s) running (stopped now): ${stranded.map(daemon => `pid ${daemon.pid} in ${daemon.home}`).join(", ")}`);
    }
  } finally {
    if (priorRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = priorRerailHome;
    if (priorContentStorePath === undefined) delete process.env.RERAIL_CONTENT_STORE_PATH;
    else process.env.RERAIL_CONTENT_STORE_PATH = priorContentStorePath;
    for (const [name, value] of Object.entries(priorUserHome)) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
    rmSync(rerailHome, { recursive: true, force: true });
    rmSync(contentStoreDirectory, { recursive: true, force: true });
    rmSync(userHome, { recursive: true, force: true });
  }
});
