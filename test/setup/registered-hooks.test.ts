import { mkdirSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { missingClaudeHookScripts, runRegisteredClaudeHook } from "../../src/setup/registered-hooks.js";

const homes: string[] = [];
afterEach(async () => {
  await Promise.all(homes.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

describe("Claude Code hook scripts that are gone", () => {
  it("names the Rerail scripts the settings run that are missing, and none that exist or are not Rerail's", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-registered-"));
    homes.push(home);
    const live = join(home, "live", "dist");
    mkdirSync(join(live, "harness"), { recursive: true });
    mkdirSync(join(live, "hosts", "claude"), { recursive: true });
    writeFileSync(join(live, "harness", "hook-launcher.js"), "");
    writeFileSync(join(live, "hosts", "claude", "claude-hook.js"), "");
    const gone = join(home, "deleted-test-folder", "dist");
    const hook = (root: string) => ({ type: "command", command: "node", args: [join(root, "harness", "hook-launcher.js"), process.execPath, join(root, "hosts", "claude", "claude-hook.js")] });
    mkdirSync(join(home, ".claude"));
    writeFileSync(join(home, ".claude", "settings.json"), JSON.stringify({
      hooks: {
        SessionStart: [{ hooks: [hook(live), { type: "command", command: "python -I /opt/other/hook.py" }] }],
        Stop: [{ hooks: [hook(gone)] }],
      },
      statusLine: { type: "command", command: `"${process.execPath}" "${join(gone, "hosts", "claude", "claude-status.js")}"` },
    }));
    expect(missingClaudeHookScripts(home).sort()).toEqual([
      join(gone, "harness", "hook-launcher.js"),
      join(gone, "hosts", "claude", "claude-hook.js"),
      join(gone, "hosts", "claude", "claude-status.js"),
    ].sort());
  });

  it("names nothing when the settings are missing or unreadable", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-registered-"));
    homes.push(home);
    expect(missingClaudeHookScripts(home)).toEqual([]);
    mkdirSync(join(home, ".claude"));
    writeFileSync(join(home, ".claude", "settings.json"), "{not json");
    expect(missingClaudeHookScripts(home)).toEqual([]);
  });
});

describe("running the registered Claude hook as Claude Code will", () => {
  /**
   * A home whose Claude settings run `script` (written to dist/hosts/claude/claude-hook.js) with `command`, in exec form
   * as Rerail registers it.
   */
  async function homeRunning(script: string | undefined, command: string = process.execPath): Promise<string> {
    const home = await mkdtemp(join(tmpdir(), "rerail-registered-run-"));
    homes.push(home);
    const hookScript = join(home, "install", "dist", "hosts", "claude", "claude-hook.js");
    if (script !== undefined) {
      mkdirSync(join(home, "install", "dist", "hosts", "claude"), { recursive: true });
      writeFileSync(hookScript, script);
    }
    mkdirSync(join(home, ".claude"));
    writeFileSync(join(home, ".claude", "settings.json"), JSON.stringify({
      hooks: { SessionStart: [{ hooks: [{ type: "command", command, args: [hookScript] }] }] },
    }));
    return home;
  }

  it("passes a hook that reads the session start and answers a JSON object", async () => {
    const home = await homeRunning("let t='';process.stdin.on('data',c=>t+=c).on('end',()=>{JSON.parse(t);process.stdout.write('{}');});");
    expect(await runRegisteredClaudeHook(home)).toBeUndefined();
  });

  it("reads an answer written after the hook process exited, as one a process it started writes on a loaded machine", async () => {
    // The hook hands its stdout to a process of its own that answers 300 ms later, and exits at once: Node reports the
    // hook's exit before its stdout ends.
    const home = await homeRunning([
      "const { spawn } = require('node:child_process');",
      "spawn(process.execPath, ['-e', \"setTimeout(() => process.stdout.write('{}'), 300)\"],",
      "  { cwd: require('node:os').tmpdir(), detached: true, stdio: ['ignore', 'inherit', 'ignore'], windowsHide: true }).unref();",
    ].join("\n"));
    expect(await runRegisteredClaudeHook(home)).toBeUndefined();
  });

  it("gives up on a hook whose stdout stays open past the time limit, and says so", async () => {
    const home = await homeRunning([
      "const { spawn } = require('node:child_process');",
      "spawn(process.execPath, ['-e', 'setTimeout(() => {}, 5000)'],",
      "  { cwd: require('node:os').tmpdir(), detached: true, stdio: ['ignore', 'inherit', 'ignore'], windowsHide: true }).unref();",
    ].join("\n"));
    const started = Date.now();
    expect(await runRegisteredClaudeHook(home, process.env, 300)).toBe("Claude Code's Rerail hook did not answer within 0.3 s");
    expect(Date.now() - started).toBeLessThan(4_000);
  });

  it("finds a bare node on PATH, as the exec form Rerail registers on Windows names it, and never in the current folder", async () => {
    const home = await homeRunning("let t='';process.stdin.on('data',c=>t+=c).on('end',()=>{JSON.parse(t);process.stdout.write('{}');});", "node");
    expect(await runRegisteredClaudeHook(home)).toBeUndefined();
  });

  it("names a registration whose hook script is gone, the failure that leaves every session unrecorded", async () => {
    expect(await runRegisteredClaudeHook(await homeRunning(undefined))).toMatch(/^Claude Code's Rerail hook exited 1: .*Cannot find module/su);
  });

  it.each(["[]", "null", "not json"])("names a hook whose answer %s is not a JSON object", async answer => {
    const home = await homeRunning(`process.stdin.resume().on('end',()=>process.stdout.write(${JSON.stringify(answer)}));`);
    expect(await runRegisteredClaudeHook(home)).toBe("Claude Code's Rerail hook answered with no JSON object");
  });

  it("names a hook that answers but reports a failure, as one rejecting Claude Code's payload does", async () => {
    const home = await homeRunning("process.stdin.resume().on('end',()=>{process.stderr.write('rerail hook: Claude hook omitted index');process.stdout.write('{}');});");
    expect(await runRegisteredClaudeHook(home)).toBe("Claude Code's Rerail hook failed: rerail hook: Claude hook omitted index");
  });

  it("checks nothing where no Rerail hook is registered", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-registered-run-"));
    homes.push(home);
    expect(await runRegisteredClaudeHook(home)).toBeUndefined();
  });
});
