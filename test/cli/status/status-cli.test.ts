/** What `rerail status` says of the folder it runs in. */
import { spawnSync, type SpawnSyncReturns } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, symlinkSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { afterAll, describe, expect, it } from "vitest";
import { PACKAGE_ROOT, RERAIL_DATABASE_FILE } from "../../../src/engine/index.js";
import { registerHarnesses } from "../../../src/setup/index.js";
import { removeFolderSync } from "../../support/platform/remove-folder.js";

// By its file URL: the run starts outside the package, where a bare `tsx` does not resolve.
const TSX = pathToFileURL(createRequire(import.meta.url).resolve("tsx")).href;

/** `rerail status` from the source in `cwd`, with `env` over this process's environment and no daemon started. */
function status(cwd: string, env: NodeJS.ProcessEnv): SpawnSyncReturns<string> {
  return spawnSync(process.execPath, ["--import", TSX, join(PACKAGE_ROOT, "src/cli/cli.ts"), "--rerail-managed-child", "status"], {
    cwd,
    encoding: "utf8",
    env: { ...process.env, ...env, RERAIL_AUTOSTART: "0" },
    // The test's own timeout cannot end a synchronous spawn, so a hung child is ended here, before it.
    timeout: 50_000,
  });
}

describe("rerail status", () => {
  const home = mkdtempSync(join(tmpdir(), "rerail-status-home-"));
  // A folder outside every Git repository, as its real path: the message names `process.cwd()`.
  const folder = realpathSync(mkdtempSync(join(tmpdir(), "rerail-status-folder-")));
  const users: string[] = [];

  afterAll(() => {
    for (const directory of [home, folder, ...users]) removeFolderSync(directory);
  });

  it("outside a Git repository, says Rerail works in one and how to start it in one from here", () => {
    // A database makes the install read as set up, so status goes on to the folder instead of saying it is not installed.
    writeFileSync(join(home, RERAIL_DATABASE_FILE), "");
    expect(spawnSync("git", ["-C", folder, "rev-parse", "--git-dir"]).status).not.toBe(0);

    const run = status(folder, { RERAIL_HOME: home, HOME: home });

    expect(run.stdout.split("\n")[0], run.stderr).toBe(
      `No Git repository here (${folder}). Rerail works in a Git repository: run rerail status inside one, or start Rerail in one with rerail start --repo <folder>.`,
    );
  }, 60_000);

  /**
   * A user folder whose Rerail home was set up and used (a database), with Claude Code connected to `<home>/current`,
   * which links to a version folder holding no scripts; then `change` runs on that home. Returns what `rerail status`
   * says of the missing scripts, and the status line's script, which registration writes for this system.
   */
  async function missingScriptsLine(change: (rerailHome: string) => void): Promise<{ line: string | undefined; statusScript: string }> {
    const user = realpathSync(mkdtempSync(join(tmpdir(), "rerail-status-user-")));
    users.push(user);
    const rerailHome = join(user, ".rerail");
    const version = join(rerailHome, "versions", "0.2.2", "node_modules", "@neralabs", "rerail");
    mkdirSync(join(version, "dist"), { recursive: true });
    writeFileSync(join(version, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "0.2.2" }));
    writeFileSync(join(rerailHome, RERAIL_DATABASE_FILE), "");
    const current = join(rerailHome, "current");
    // A folder link a test account can make on every system: a junction on Windows.
    symlinkSync(version, current, process.platform === "win32" ? "junction" : "dir");
    mkdirSync(join(user, ".claude"));
    await registerHarnesses({ root: current, home: user, harnesses: ["claude-code"] });
    change(rerailHome);
    // The coding agents' home is HOME on macOS and Linux and USERPROFILE on Windows.
    const run = status(folder, { RERAIL_HOME: rerailHome, HOME: user, USERPROFILE: user });
    const statusScript = join(current, "dist", "hosts", "claude", process.platform === "win32" ? "claude-status.js" : "claude-status.sh");
    return { line: run.stdout.split("\n").find(text => text.startsWith("Claude Code's Rerail hooks run ")), statusScript };
  }

  it("once the managed install's version folders are gone, names the status line's script and set-up, which installs one", async () => {
    // As a removal by hand or a clean-up tool leaves it: `current` dangles, and `install --refresh` would change nothing.
    const { line, statusScript } = await missingScriptsLine(rerailHome => removeFolderSync(join(rerailHome, "versions")));
    expect(line).toContain(statusScript);
    expect(line).toMatch(/ Run npx @neralabs\/rerail setup\.$/u);
  }, 60_000);

  it("while `current` holds an install that lacks the scripts the settings name, as after an update, names install --refresh", async () => {
    const { line, statusScript } = await missingScriptsLine(() => undefined);
    expect(line).toContain(statusScript);
    expect(line).toMatch(/ Run rerail install --refresh\.$/u);
  }, 60_000);
});
