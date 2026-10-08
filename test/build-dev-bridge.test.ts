import { execFile, spawn } from "node:child_process";
import { once } from "node:events";
import { chmod, cp, mkdir, mkdtemp, readFile, readlink, realpath, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { RerailDaemon } from "../src/daemon.js";
import { pendingPath, persistPending } from "../src/hook-host.js";

const execute = promisify(execFile);
const packageRoot = resolve(import.meta.dirname, "..");
const bridgeCli = join(packageRoot, "dist", "host-bridge", "dist", "cli.js");
const bridgeRoot = dirname(dirname(bridgeCli));
const installer = join(packageRoot, "..", "scripts", "rerail-container", "install-bridge.mjs");
const temporaryDirectories: string[] = [];

beforeAll(async () => {
  await execute(process.execPath, [join(packageRoot, "scripts", "build-dev-bridge.mjs")], { cwd: packageRoot });
}, 60_000);

afterAll(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

/** Runs the built bridge's registration module the way the installer does, from a plain Node process. */
async function register(mode: "registerHarnesses" | "unregisterHarnesses", root: string, options: object): Promise<void> {
  await execute(process.execPath, ["--input-type=module", "-e", `
    import { ${mode} } from ${JSON.stringify(pathToFileURL(join(root, "dist", "registration.js")).href)};
    await ${mode}(${JSON.stringify({ root, ...options })});
  `]);
}

describe("Rerail development bridge CLI", () => {
  it("loads and invokes every connected bundle without host-installed Rerail dependencies", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-bridge-isolated-"));
    temporaryDirectories.push(home);
    const isolatedBridge = join(home, "dev-bridge");
    const workspaceDirectory = join(home, "checkout");
    await cp(bridgeRoot, isolatedBridge, { recursive: true });
    await mkdir(join(workspaceDirectory, ".nera"), { recursive: true });
    const workspace = await realpath(workspaceDirectory);
    const isolatedDaemon = await RerailDaemon.start({
      version: "test",
      databasePath: join(workspace, ".rerail", "rerail.sqlite"),
      workspaceRoot: workspace,
      port: 0,
    });
    try {
      await writeFile(
        join(workspace, ".nera", "instance.env"),
        `NERA_RERAIL_ENABLED="true"\nNERA_RERAIL_PUBLIC_URL="${isolatedDaemon.url}"\n`,
      );
      const bundleUrl = (name: string) => pathToFileURL(join(isolatedBridge, "dist", name)).href;
      // Runtime-selected copied artifacts are the boundary under test; source-level static imports would resolve repo dependencies.
      await expect(execute(process.execPath, ["--input-type=module", "-e", `
        const pi = await import(${JSON.stringify(bundleUrl("bundled-pi.js"))});
        const cursor = await import(${JSON.stringify(bundleUrl("bundled-cursor.js"))});
        const claude = await import(${JSON.stringify(bundleUrl("bundled-claude.js"))});
        const codex = await import(${JSON.stringify(bundleUrl("bundled-codex.js"))});
        if (typeof pi.default !== "function") throw new Error("Pi bridge did not export an extension");
        await cursor.runCursorHook({ hook_event_name: "sessionStart", conversation_id: "cursor-isolated", cwd: process.cwd() });
        await claude.runClaudeHook({ hook_event_name: "SessionStart", session_id: "claude-isolated", cwd: process.cwd(), source: "startup" });
        await codex.runCodexHook({ hook_event_name: "SessionStart", session_id: "codex-isolated", cwd: process.cwd(), source: "startup" });
      `], { cwd: workspace, encoding: "utf8", env: { ...process.env, CLAUDE_PROJECT_DIR: workspace } })).resolves.toMatchObject({ stderr: "" });
    } finally {
      await isolatedDaemon.close();
    }
  }, 60_000);

  it("installs the packaged Codex skill with its working bridge command", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-bridge-install-"));
    temporaryDirectories.push(home);
    await register("registerHarnesses", bridgeRoot, { home, harnesses: ["codex"] });
    expect(await readFile(join(home, ".codex", "skills", "rerail", "SKILL.md"), "utf8")).toContain(bridgeCli);
  });

  it("composes and byte-exactly restores a project Claude status command through the bridge installer", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-bridge-project-"));
    temporaryDirectories.push(home);
    const workspace = join(home, "checkout");
    const localPath = join(workspace, ".claude", "settings.local.json");
    const original = `${JSON.stringify({ statusLine: { type: "command", command: "printf project" }, theme: "dark" }, null, 2)}\n`;
    await mkdir(join(home, ".claude"), { recursive: true });
    await mkdir(dirname(localPath), { recursive: true });
    await writeFile(localPath, original);
    await execute(process.execPath, [installer, bridgeRoot, workspace], { env: { ...process.env, HOME: home, RERAIL_HOME: join(home, ".rerail") } });
    const installed = await readlink(join(home, ".rerail", "dev-bridge"));
    const composed = JSON.parse(await readFile(localPath, "utf8")) as { statusLine: { command: string }; theme: string };
    expect(composed.theme).toBe("dark");
    expect(composed.statusLine.command).toContain(join(installed, "dist", "claude-status.js"));
    await register("unregisterHarnesses", installed, { home, workspaceDir: workspace });
    expect(await readFile(localPath, "utf8")).toBe(original);
  });

  it("persists exactly one steer receipt through the checkout daemon for one bridged hook delivery", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-bridge-hook-"));
    temporaryDirectories.push(root);
    const daemon = await RerailDaemon.start({ version: "test", databasePath: join(root, ".rerail", "rerail.sqlite"), workspaceRoot: root, port: 0 });
    try {
      await mkdir(join(root, ".nera"));
      await writeFile(join(root, ".nera", "instance.env"), `NERA_RERAIL_ENABLED="true"\nNERA_RERAIL_PUBLIC_URL="${daemon.url}"\n`);
      await persistPending(pendingPath(join(root, ".rerail", "sessions"), "claude-code", "bridge-session"), [
        { sessionId: "bridge-session", requested: "steer", mechanism: "additional-context", content: "Mind the API.", ruleIds: [] },
      ]);
      const hook = spawn(process.execPath, [join(bridgeRoot, "dist", "claude-hook.js")], { env: { ...process.env, RERAIL_HOME: join(root, "home") } });
      let stdout = "";
      let stderr = "";
      hook.stdout.setEncoding("utf8").on("data", chunk => { stdout += chunk; });
      hook.stderr.setEncoding("utf8").on("data", chunk => { stderr += chunk; });
      hook.stdin.end(JSON.stringify({ hook_event_name: "SessionStart", session_id: "bridge-session", cwd: root, source: "startup" }));
      const [code] = await once(hook, "exit");
      expect(code, stderr).toBe(0);
      expect(JSON.parse(stdout)).toEqual({ hookSpecificOutput: { hookEventName: "SessionStart", additionalContext: "Mind the API." } });
      const session = await (await fetch(new URL("/api/sessions/bridge-session", daemon.url))).json() as { hud: { steeringCount: number } };
      expect(session.hud.steeringCount).toBe(1);
    } finally {
      await daemon.close();
    }
  }, 60_000);

  it("keeps the installed bridge inert in a temporary worktree below a disabled workspace", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-bridge-disabled-"));
    const worktree = join(root, "tmp", "baseline-gate-audit");
    temporaryDirectories.push(root);
    await mkdir(join(root, ".nera"));
    await mkdir(worktree, { recursive: true });
    await writeFile(join(root, ".nera", "instance.env"), 'NERA_RERAIL_ENABLED="false"\n');

    const hook = spawn(process.execPath, [join(bridgeRoot, "dist", "codex-hook.js")], {
      cwd: worktree,
      env: { ...process.env, RERAIL_HOME: join(root, "home") },
    });
    let stdout = "";
    let stderr = "";
    hook.stdout.setEncoding("utf8").on("data", chunk => { stdout += chunk; });
    hook.stderr.setEncoding("utf8").on("data", chunk => { stderr += chunk; });
    hook.stdin.end(JSON.stringify({
      hook_event_name: "SessionStart",
      session_id: "disabled-worktree",
      cwd: worktree,
      source: "startup",
    }));

    const [code] = await once(hook, "exit");
    expect(code, stderr).toBe(0);
    expect(stdout).toBe("{}\n");
    expect(stderr).toBe("");
    await expect(stat(join(worktree, ".rerail"))).rejects.toMatchObject({ code: "ENOENT" });
  });

  it("runs the disabled checkout operation when PATH supplies a malicious just", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-bridge-cli-"));
    temporaryDirectories.push(root);
    const bin = join(root, "bin");
    const hijackMarker = join(root, "just-was-run");
    await mkdir(join(root, ".nera"), { recursive: true });
    await mkdir(join(root, "scripts"));
    await mkdir(bin);
    await writeFile(join(root, ".nera", "instance.env"), "NERA_RERAIL_ENABLED=false\n");
    await writeFile(join(root, "scripts", "ops.sh"), [
      "set -eu",
      "[ \"$1\" = rerail-command ]",
      "[ \"$2\" = status ]",
      "printf 'checkout status is available\\n'",
      "",
    ].join("\n"));
    const fakeJust = join(bin, "just");
    await writeFile(fakeJust, "#!/bin/sh\nprintf hijacked > \"$HIJACK_MARKER\"\n");
    await chmod(fakeJust, 0o755);

    const { stdout } = await execute(process.execPath, [bridgeCli, "status"], {
      cwd: root,
      env: { ...process.env, PATH: bin, HIJACK_MARKER: hijackMarker },
    });

    expect(stdout).toBe("checkout status is available\n");
    await expect(stat(hijackMarker)).rejects.toMatchObject({ code: "ENOENT" });
  });

  it("reports a missing checkout launcher on one stderr line instead of a stack trace", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-bridge-cli-"));
    temporaryDirectories.push(root);
    await mkdir(join(root, ".nera"), { recursive: true });
    await writeFile(join(root, ".nera", "instance.env"), "NERA_RERAIL_ENABLED=true\nNERA_RERAIL_PUBLIC_URL=http://127.0.0.1:4737\n");

    const failure = await execute(process.execPath, [bridgeCli, "status"], { cwd: root, env: process.env }).catch(error => error);
    expect(failure.code).toBe(127);
    expect(failure.stderr).toMatch(/^\/bin\/bash: .*scripts\/ops\.sh: No such file or directory\n$/u);
  });
});
