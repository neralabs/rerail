import { chmod, mkdir, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";
import { launcherRecordedNode, olderNodeStep } from "../../src/harness/index.js";
import { launcherShim } from "../../src/setup/updater.js";
import { temporaryDirectories } from "../support/hook-events.js";

const WINDOWS = process.platform === "win32";
const NODE = WINDOWS ? "node.exe" : "node";
const temporaryDirectory = temporaryDirectories("rerail-older-node-step-");
const GONE_STEP = "Put Node 24 or newer first on PATH, then run rerail install --refresh.";

interface Recorded {
  /** The Rerail home, `<folder>/<home>`, whose launcher records `node`. */
  home: string;
  node: string;
}

/**
 * A Rerail home in a folder of its own whose launcher, as set-up writes it, records `node` (a path in that folder);
 * the file is made there with the execute bit, or without it, or not at all.
 */
async function homeRecording(node: string[], made: "runs" | "no execute bit" | "gone" = "runs", homeName = "home"): Promise<Recorded> {
  const folder = await temporaryDirectory({ started: false });
  const home = join(folder, homeName);
  const recorded = join(home, "..", ...node);
  await mkdir(join(home, "bin"), { recursive: true });
  await writeFile(join(home, "bin", WINDOWS ? "rerail.cmd" : "rerail"), launcherShim(join(home, "current"), recorded));
  if (made !== "gone") {
    await mkdir(dirname(recorded), { recursive: true });
    await writeFile(recorded, "#!/bin/sh\n");
    await chmod(recorded, made === "runs" ? 0o755 : 0o644);
  }
  return { home, node: recorded };
}

/**
 * The hook's line on an older Node names the step that gets Rerail back. `rerail install --refresh` starts through the
 * launcher, which runs the node set-up recorded while that file is there, whatever PATH holds: there the line names
 * the launcher's own command with `node` from PATH in that node's place.
 */
describe("the hook's step on a Node older than Rerail runs on", () => {
  it("names the launcher's command with node from PATH while the node set-up recorded is still there", async () => {
    const { home, node } = await homeRecording(["system", "bin", NODE]);
    const cli = join(home, "current", "dist", "cli", "cli.js");
    // The path as a shell takes it: for a POSIX shell bare, or single-quoted where it holds a sign; on Windows in double
    // quotes, which cmd.exe and PowerShell both take.
    const steps = (WINDOWS ? [`"${cli}"`] : [cli, `'${cli}'`]).map(word => `Put Node 24 or newer first on PATH, then run node ${word} install --refresh: the rerail command runs ${node} while that file is there, whatever PATH holds.`);
    expect(steps).toContain(olderNodeStep(home));
  });

  it("names rerail install --refresh once the node set-up recorded is gone, since the launcher then runs node from PATH", async () => {
    const { home } = await homeRecording(["gone", "bin", NODE], "gone");
    expect(olderNodeStep(home)).toBe(GONE_STEP);
  });

  it("names rerail install --refresh where there is no launcher", async () => {
    expect(olderNodeStep(join(await temporaryDirectory({ started: false }), "home"))).toBe(GONE_STEP);
  });

  it.runIf(!WINDOWS)("takes a recorded node without its execute bit for gone, as the launcher's [ -x ] does", async () => {
    const { home } = await homeRecording(["system", "bin", NODE], "no execute bit");
    expect(olderNodeStep(home)).toBe(GONE_STEP);
  });

  it.runIf(!WINDOWS)("quotes the launcher's command for the shell a person types it in", async () => {
    const { home } = await homeRecording(["system", NODE], "runs", "home with space");
    const cli = join(home, "current", "dist", "cli", "cli.js");
    expect(olderNodeStep(home)).toContain(`then run node '${cli}' install --refresh: `);
  });

  it.runIf(WINDOWS).each(["home with space", "RUNNER~1"])("double-quotes the launcher's command on Windows for a home such as %s, as cmd.exe and PowerShell both take it", async homeName => {
    const { home } = await homeRecording(["system", NODE], "runs", homeName);
    const cli = join(home, "current", "dist", "cli", "cli.js");
    expect(olderNodeStep(home)).toContain(`then run node "${cli}" install --refresh: `);
  });

  it.runIf(WINDOWS).each(["100%", "a!b", "a$b", "a`b", "a“b"])("names PowerShell on Windows for a home such as %s, which double quotes cannot carry in both shells", async homeName => {
    const { home } = await homeRecording(["system", NODE], "runs", homeName);
    const cli = join(home, "current", "dist", "cli", "cli.js");
    expect(olderNodeStep(home)).toContain(`then in PowerShell run node '${cli}' install --refresh: `);
  });
});

describe("the node the launcher records", () => {
  it.runIf(!WINDOWS)("is read back from the launcher set-up writes, a quote in its path included", async () => {
    const { home, node } = await homeRecording(["it's", "bin", NODE]);
    expect(launcherRecordedNode(home)).toBe(node);
  });

  it.runIf(WINDOWS)("is read back from the batch launcher set-up writes, a % in its path and a non-ASCII path in the Rerail home included", async () => {
    const percent = await homeRecording(["100%", NODE]);
    expect(launcherRecordedNode(percent.home)).toBe(percent.node);
    const inHome = await homeRecording(["home", "node", "José", NODE]);
    expect(launcherRecordedNode(inHome.home)).toBe(inHome.node);
  });

  it.runIf(WINDOWS)("is none where the batch launcher could not spell it, so it runs node.exe from PATH", async () => {
    const { home } = await homeRecording(["José", NODE]);
    expect(launcherRecordedNode(home)).toBeUndefined();
    expect(olderNodeStep(home)).toBe(GONE_STEP);
  });
});
