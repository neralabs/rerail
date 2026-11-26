import { execFile } from "node:child_process";
import { realpathSync } from "node:fs";
import { cp, mkdir, mkdtemp, readdir, readFile, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { describe, expect, test } from "vitest";
import { removeFolder } from "../support/platform/remove-folder.js";

/** This checkout, whose built package (`npm run build:js`) the test copies. */
const checkout = resolve(import.meta.dirname, "..", "..");

/**
 * The built package laid out as `npm install <tarball> --prefix <root>` lays it out, in a folder outside any Git
 * repository: `dist/`, the native binaries, the model, the manifest and the engine's own files copied, its dependencies
 * beside it. Such a package is neither the managed installation nor in a checkout, so it has no content store. Returns
 * its real root, as `realpathSync` gives it: the path Node runs the package's modules from, so the one a message names.
 */
async function installOutsideGit(root: string): Promise<string> {
  const packageRoot = join(root, "node_modules", "@neralabs", "rerail");
  const capabilities = join("native", "src", "code_rule_dsl", "capabilities.json");
  await mkdir(join(packageRoot, "native", "src", "code_rule_dsl"), { recursive: true });
  for (const entry of ["dist", "model", "package.json", capabilities]) {
    await cp(join(checkout, entry), join(packageRoot, entry), { recursive: true });
  }
  for (const name of await readdir(join(checkout, "native"))) {
    if (name.endsWith(".node")) await cp(join(checkout, "native", name), join(packageRoot, "native", name));
  }
  await symlink(join(checkout, "node_modules"), join(packageRoot, "node_modules"), "junction");
  return realpathSync(packageRoot);
}

/** How a `rerail` command exited, and what it wrote. */
interface Ran {
  code: number | null;
  stdout: string;
  stderr: string;
}

async function rerail(cli: string, args: readonly string[], env: NodeJS.ProcessEnv): Promise<Ran> {
  const { promise, resolve: settle } = Promise.withResolvers<Ran>();
  const child = execFile(process.execPath, [cli, ...args], { env }, (_error, stdout, stderr) => settle({ code: child.exitCode, stdout, stderr }));
  return await promise;
}

/** The case's own limit, from its start. */
const CASE_LIMIT_MS = 120_000;
/**
 * The part of the case's limit kept for what follows the daemon's answer: a second start, the status, the bundle and the
 * clean-up, whose wait for the daemon's exit (`endProcess`) takes up to 30 s twice. A daemon slow to answer is waited
 * for until the rest of the limit is spent.
 */
const AFTER_ANSWER_MS = 60_000;
/** `rerail status`'s exit codes when no daemon answers it (docs/reference.md): 3 when none runs, 4 when it does not answer. */
const NO_ANSWER = [3, 4];

/**
 * The pid the daemon's state file names, or `undefined` when the file is not written, is gone or cannot be read or
 * parsed. Never throws, so the clean-up's `daemon stop` and `endProcess` still run and the case's own failure stays the
 * one reported.
 */
async function recordedPid(home: string): Promise<number | undefined> {
  try {
    // Test boundary: the daemon's own record of itself.
    return (JSON.parse(await readFile(join(home, "daemon.json"), "utf8")) as { pid: number }).pid;
  } catch {
    return undefined;
  }
}

/**
 * The lines `rerail daemon start` prints for the daemon that runs. A start whose daemon answers within the start's own
 * wait printed them. One whose daemon has not, as on a busy machine, exits 1 and prints nothing on stdout, saying on
 * stderr that the daemon may still be starting and that it leaves it running: the case then waits, until `answerBy`,
 * for `rerail status` to find the daemon answering, and takes the lines a second start prints for a daemon that runs,
 * after "The Rerail daemon is already running.".
 */
async function runningDaemonLines(cli: string, env: NodeJS.ProcessEnv, started: Ran, answerBy: number): Promise<string> {
  if (started.code === 0) return started.stdout;
  expect(started).toMatchObject({ code: 1, stdout: "", stderr: expect.stringContaining("may still be starting; this start leaves it running") });
  await expect.poll(async () => (await rerail(cli, ["status"], env)).code, { timeout: answerBy - Date.now(), interval: 250 }).not.toBeOneOf(NO_ANSWER);
  const again = await rerail(cli, ["daemon", "start", "--port", "0"], env);
  expect(again).toMatchObject({ code: 0, stdout: expect.stringMatching(/^The Rerail daemon is already running\.\n/u) });
  return again.stdout;
}

/** Whether a process this account may signal runs as `pid`: signal 0 asks and sends nothing. */
function running(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/**
 * Waits for `pid` to exit and, when the wait runs out (a `daemon stop` that failed), ends it and waits again, so the
 * folder it runs in is removed after it. Never throws: the case's own failure stays the one reported.
 */
async function endProcess(pid: number): Promise<void> {
  const exited = async (): Promise<boolean> =>
    await expect.poll(() => running(pid), { timeout: 30_000, interval: 50 }).toBe(false).then(() => true, () => false);
  if (await exited()) return;
  try {
    process.kill(pid, "SIGKILL");
  } catch {
    // It exited between the wait and the signal.
  }
  await exited();
}

describe("a Rerail package with no content store", () => {
  // Every code rule audit of such a Rerail fails to open its workspace; each place that reports the engine says so.
  test("says on each surface that no code rule can run, and why", async () => {
    const answerBy = Date.now() + CASE_LIMIT_MS - AFTER_ANSWER_MS;
    const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-no-store-")));
    const home = join(root, "rerail-home");
    const env: NodeJS.ProcessEnv = { ...process.env, RERAIL_HOME: home };
    delete env.RERAIL_CONTENT_STORE_PATH;
    let cli: string | undefined;
    let daemonPid: number | undefined;
    try {
      const packageRoot = await installOutsideGit(root);
      cli = join(packageRoot, "dist", "cli", "cli.js");
      const why = `no code rule can run: Rerail at ${packageRoot} is neither the managed installation under ` +
        `${join(home, "versions")} nor in a Git checkout, so it has no content store`;
      const started = await rerail(cli, ["daemon", "start", "--port", "0"], env);
      // Read before any assertion, for the clean-up to end that daemon.
      daemonPid = await recordedPid(home);
      expect(await runningDaemonLines(cli, env, started, answerBy)).toContain(`Engine: unavailable: ${why}\n`);
      expect(await rerail(cli, ["status"], env)).toMatchObject({ code: 5, stdout: expect.stringContaining(`Engine: unavailable: ${why}\n`) });
      expect(await readFile(join(home, "daemon.log"), "utf8")).toContain(`native kernel: unavailable: ${why}\n`);
      const bundle = join(root, "bundle.json");
      expect((await rerail(cli, ["support-bundle", "--output", bundle], env)).code).toBe(0);
      // The bundle replaces paths under the user's home and the Rerail home, so this matches the reason around them.
      const redacted = /^no code rule can run: Rerail at .+ is neither the managed installation under .+ nor in a Git checkout, so it has no content store$/u;
      expect(JSON.parse(await readFile(bundle, "utf8"))).toMatchObject({ daemon: { status: "running", engine: { ok: false, error: expect.stringMatching(redacted) } } });
    } finally {
      try {
        // A daemon still in its start when the start returned writes its state file later.
        daemonPid ??= await recordedPid(home);
        if (cli !== undefined) await rerail(cli, ["daemon", "stop"], env);
        // The daemon runs in the Rerail home, inside the folder removed next: its exit comes first.
        if (daemonPid !== undefined) await endProcess(daemonPid);
      } finally {
        await removeFolder(root);
      }
    }
  }, CASE_LIMIT_MS);
});
