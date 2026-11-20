/** `rerail merge-assist` run as the person runs it, in a process of its own, beside a stat of gh's hosts file. */
import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { ghSignInStamps } from "../../../src/runtime/merge-assist/gh-config.js";
import { FakeGithub, mergeAssistWorkspaces } from "../../support/merge-assist/workspace.js";
import { withExperiments } from "../../support/environment/experiments.js";

withExperiments("MERGE_ASSIST");

/**
 * Loaded into the command's process: a stat of gh's hosts file never answers and holds Node's loop meanwhile, as a stat
 * of a folder on a drive that does not answer does (a message port that is never closed stands for it). With
 * `CLOSE_FAILS` set, closing the database fails from then on, as an error Rerail did not foresee would.
 */
const STALLED_STAT = `
const fsp = require("node:fs/promises");
const { syncBuiltinESMExports } = require("node:module");
const { MessageChannel } = require("node:worker_threads");
const stat = fsp.stat;
fsp.stat = function (file, ...rest) {
  if (!String(file).endsWith("hosts.yml")) return stat.call(fsp, file, ...rest);
  process.stderr.write("stat of hosts.yml asked\\n");
  new MessageChannel().port1.on("message", () => undefined);
  if (process.env.CLOSE_FAILS === "1") {
    require(require.resolve("better-sqlite3", { paths: [process.cwd()] })).prototype.close = () => { throw new Error("the database did not close"); };
  }
  return new Promise(() => undefined);
};
syncBuiltinESMExports();
`;

describe("rerail merge-assist as its own process", () => {
  const workspace = mergeAssistWorkspaces();
  let child: ChildProcess | undefined;
  afterEach(() => {
    child?.kill();
    child = undefined;
  });

  /**
   * `rerail merge-assist status` in its own process (`--json` with `json`), Merge Assist on a minute ago with the hook
   * command `hook` (none by default), beside a stat that never answers.
   */
  async function status(options: { closeFails?: boolean; hook?: string[]; json?: boolean }): Promise<{ code: unknown; stdout: string; stderr: string }> {
    const { closeFails = false, hook, json = false } = options;
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const env = { ...process.env, RERAIL_HOME: ws.folder, RERAIL_AUTOSTART: "0", RERAIL_EXPERIMENTAL_MERGE_ASSIST: "1", GH_CONFIG_DIR: join(ws.folder, "gh"), CLOSE_FAILS: closeFails ? "1" : "0" };
    // Turned on under the command's own gh environment, so its status reads no gh.
    const github = new FakeGithub({ signInEnvironment: ghSignInStamps(env)().environment });
    const service = ws.service({ github, now: () => Date.now() - 60_000 });
    await service.setEnabled(ws.repo, true);
    if (hook !== undefined) await service.updateSettings(ws.repo, { hook: { command: hook, timeoutSeconds: 10 } });
    const preload = join(ws.folder, "stalled-stat.cjs");
    writeFileSync(preload, STALLED_STAT);
    const cli = join(PACKAGE_ROOT, "src", "cli", "cli.ts");
    const args = ["--require", preload, "--import", "tsx", cli, "--rerail-managed-child", "merge-assist", "status", "--repo", ws.repo, ...(json ? ["--json"] : [])];
    const started = spawn(process.execPath, args, { cwd: PACKAGE_ROOT, env, stdio: ["ignore", "pipe", "pipe"] });
    child = started;
    let stdout = "";
    let stderr = "";
    started.stdout.on("data", (chunk: Buffer) => { stdout += chunk.toString(); });
    started.stderr.on("data", (chunk: Buffer) => { stderr += chunk.toString(); });
    // Without the end, the process waits for the stat: a bound of its own, under the case's, so such a process fails the
    // case's assertion on its code, not the case's time limit.
    const ended = once(started, "close").then(([code]) => code as unknown);
    const running = new Promise(resolve => setTimeout(resolve, 30_000, "still running after 30 s"));
    const code = await Promise.race([ended, running]);
    if (code === "still running after 30 s") started.kill();
    return { code, stdout, stderr };
  }

  it("ends once its answer is written, while a stat of gh's hosts file has not answered", async () => {
    const { code, stdout, stderr } = await status({});
    expect({ code, stderr }).toEqual({ code: 0, stderr: "stat of hosts.yml asked\n" });
    expect(stdout).toContain("  GitHub: gh signed in as akim (acme/app), read ");
  }, 60_000);

  it("ends only once all of a long answer is written", async () => {
    // A hook command of 40 arguments of 4,000 characters: an answer longer than a pipe holds, so part of it is still
    // being written when the command returns.
    const hook = Array.from({ length: 40 }, (_, index) => `${String(index).padStart(4, "0")}${"x".repeat(3_996)}`);
    const { code, stdout } = await status({ hook, json: true });
    expect(code).toBe(0);
    expect(stdout.endsWith("\n")).toBe(true);
    expect((JSON.parse(stdout) as { settings: { hook: { command: string[] } } }).settings.hook.command).toEqual(hook);
  }, 60_000);

  it("ends once its failure is written, while a stat of gh's hosts file has not answered", async () => {
    const { code, stdout, stderr } = await status({ closeFails: true });
    expect({ code, stdout, stderr }).toEqual({ code: 1, stdout: "", stderr: "stat of hosts.yml asked\nrerail: the database did not close\n" });
  }, 60_000);
});
