import { execFile } from "node:child_process";
import { existsSync, realpathSync } from "node:fs";
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

async function rerail(cli: string, args: readonly string[], env: NodeJS.ProcessEnv): Promise<{ code: number | null; stdout: string }> {
  const { promise, resolve: settle } = Promise.withResolvers<{ code: number | null; stdout: string }>();
  const child = execFile(process.execPath, [cli, ...args], { env }, (_error, stdout) => settle({ code: child.exitCode, stdout }));
  return await promise;
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
      // Test boundary: the state file the start wrote, read before any assertion for the pid the clean-up ends.
      const state = join(home, "daemon.json");
      if (existsSync(state)) daemonPid = (JSON.parse(await readFile(state, "utf8")) as { pid: number }).pid;
      expect(started.stdout).toContain(`Engine: unavailable: ${why}\n`);
      expect(await rerail(cli, ["status"], env)).toMatchObject({ code: 5, stdout: expect.stringContaining(`Engine: unavailable: ${why}\n`) });
      expect(await readFile(join(home, "daemon.log"), "utf8")).toContain(`native kernel: unavailable: ${why}\n`);
      const bundle = join(root, "bundle.json");
      expect((await rerail(cli, ["support-bundle", "--output", bundle], env)).code).toBe(0);
      // The bundle replaces paths under the user's home and the Rerail home, so this matches the reason around them.
      const redacted = /^no code rule can run: Rerail at .+ is neither the managed installation under .+ nor in a Git checkout, so it has no content store$/u;
      expect(JSON.parse(await readFile(bundle, "utf8"))).toMatchObject({ daemon: { status: "running", engine: { ok: false, error: expect.stringMatching(redacted) } } });
    } finally {
      try {
        if (cli !== undefined) await rerail(cli, ["daemon", "stop"], env);
        // The daemon runs in the Rerail home, inside the folder removed next: its exit comes first.
        if (daemonPid !== undefined) await endProcess(daemonPid);
      } finally {
        await removeFolder(root);
      }
    }
  }, 120_000);
});
