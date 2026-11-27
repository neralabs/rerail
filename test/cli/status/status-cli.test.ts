/** What `rerail status` says of the folder it runs in. */
import { spawnSync, type SpawnSyncReturns } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
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

/** The folder under the user's home whose presence makes each coding agent found on this computer. */
const AGENT_FOLDERS = { "claude-code": ".claude", codex: ".codex", cursor: ".cursor", pi: join(".pi", "agent") } as const;

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
   * A user folder whose Rerail home was set up and used (a database), with `agent` connected to `<home>/current`, which
   * links to a version folder holding no scripts; then `change` runs on that home. Returns the lines `rerail status`
   * prints, and `current`.
   */
  async function statusOfConnected(agent: keyof typeof AGENT_FOLDERS, change: (rerailHome: string) => void): Promise<{ lines: string[]; current: string }> {
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
    mkdirSync(join(user, AGENT_FOLDERS[agent]), { recursive: true });
    await registerHarnesses({ root: current, home: user, harnesses: [agent] });
    change(rerailHome);
    // The coding agents' home is HOME on macOS and Linux and USERPROFILE on Windows.
    const run = status(folder, { RERAIL_HOME: rerailHome, HOME: user, USERPROFILE: user });
    return { lines: run.stdout.split("\n").filter(line => line !== ""), current };
  }

  // As a removal by hand or a clean-up tool leaves it: `current` dangles, and `install --refresh` would change nothing.
  const removeVersions = (rerailHome: string): void => removeFolderSync(join(rerailHome, "versions"));

  it.each([["codex", "Codex"], ["cursor", "Cursor"], ["pi", "Pi"], ["claude-code", "Claude Code"]] as const)(
    "once the managed install's version folders are gone and only %s is connected, says so first and once, and names set-up through npx",
    async (agent, name) => {
      const { lines, current } = await statusOfConnected(agent, removeVersions);

      expect(lines[0], lines.join("\n")).toContain(`${current} is gone: Rerail does not run in ${name}, so nothing is recorded`);
      expect(lines[0]).toMatch(/ Run npx @neralabs\/rerail setup first\.$/u);
      expect(lines.filter(line => line.includes("npx @neralabs/rerail setup")), lines.join("\n")).toHaveLength(1);
    },
    60_000,
  );

  it("once the managed install's version folders are gone in a home with no database yet, names the install, not a computer without Rerail", async () => {
    const { lines, current } = await statusOfConnected("codex", rerailHome => {
      removeVersions(rerailHome);
      rmSync(join(rerailHome, RERAIL_DATABASE_FILE));
    });

    expect(lines[0], lines.join("\n")).toContain(`${current} is gone: Rerail does not run in Codex, so nothing is recorded`);
  }, 60_000);

  it("with `current` itself removed, names Claude Code's missing scripts and set-up, since a refresh would register this package", async () => {
    let current = "";
    const { lines } = await statusOfConnected("claude-code", rerailHome => {
      current = join(rerailHome, "current");
      removeFolderSync(join(rerailHome, "versions"));
      removeFolderSync(current);
    });
    const line = lines.find(text => text.startsWith("Claude Code's Rerail hooks run "));

    expect(line, lines.join("\n")).toContain(join(current, "dist", "hosts", "claude"));
    expect(line).toMatch(/ Run npx @neralabs\/rerail setup\.$/u);
  }, 60_000);

  it("while `current` holds an install that lacks the scripts the settings name, as after an update, names the status line's script and install --refresh", async () => {
    const { lines, current } = await statusOfConnected("claude-code", () => undefined);
    const line = lines.find(text => text.startsWith("Claude Code's Rerail hooks run "));

    expect(line, lines.join("\n")).toContain(join(current, "dist", "hosts", "claude", process.platform === "win32" ? "claude-status.js" : "claude-status.sh"));
    expect(line).toMatch(/ Run rerail install --refresh\.$/u);
    expect(lines.join("\n")).not.toContain(" is gone");
  }, 60_000);
});
