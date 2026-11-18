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
 * of a folder on a drive that does not answer does (a message port that is never closed stands for it).
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

  it("ends once its answer is written, while a stat of gh's hosts file has not answered", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const env = { ...process.env, RERAIL_HOME: ws.folder, RERAIL_AUTOSTART: "0", RERAIL_EXPERIMENTAL_MERGE_ASSIST: "1", GH_CONFIG_DIR: join(ws.folder, "gh") };
    // Turned on a minute ago under the command's own gh environment, so its status reads no gh.
    const github = new FakeGithub({ signInEnvironment: ghSignInStamps(env)().environment });
    await ws.service({ github, now: () => Date.now() - 60_000 }).setEnabled(ws.repo, true);
    const preload = join(ws.folder, "stalled-stat.cjs");
    writeFileSync(preload, STALLED_STAT);
    const cli = join(PACKAGE_ROOT, "src", "cli", "cli.ts");
    const args = ["--require", preload, "--import", "tsx", cli, "--rerail-managed-child", "merge-assist", "status", "--repo", ws.repo];
    const started = spawn(process.execPath, args, { cwd: PACKAGE_ROOT, env, stdio: ["ignore", "pipe", "pipe"] });
    child = started;
    let stdout = "";
    let stderr = "";
    started.stdout.on("data", (chunk: Buffer) => { stdout += chunk.toString(); });
    started.stderr.on("data", (chunk: Buffer) => { stderr += chunk.toString(); });

    // Without the end, the process waits for the stat, and this case for its time limit.
    const [code] = await once(started, "close");
    expect({ code, stderr }).toEqual({ code: 0, stderr: "stat of hosts.yml asked\n" });
    expect(stdout).toContain("  GitHub: gh signed in as tbelanger (acme/app), read ");
  }, 60_000);
});
