/** `rerail start --cursor-cloud`: a Cursor cloud agent's VM gets project hooks kept out of Git, the marker and a daemon. */
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { z } from "zod";
import { startForCursorCloud, type CloudDaemon } from "../../src/cli/cursor-cloud-start.js";
import { startRerail } from "../../src/cli/repository-setup.js";
import { openRerailDatabase, recordCheckout, setUpRepository } from "../../src/engine/index.js";
import { cursorCloudMarkerPath } from "../../src/hosts/cursor/cursor-cloud.js";
import { CURSOR_HOOK_EVENTS } from "../../src/setup/harness-registration.js";
import { fakeSetupEnvironment } from "../support/fake-setup-environment.js";
import { agentIo, fixture, removeFixtures, SETUP_TIMEOUT_MS, type Fixture, type Recorded } from "../support/repository-setup.js";

afterEach(async () => {
  await removeFixtures();
});

const NODE = "/opt/node/bin/node";
const DAEMON: CloudDaemon = { url: "http://127.0.0.1:4737", pid: 4242 };
/** A hook of the repository's own: the payload logger the PRD's cloud test commits. */
const OWN_HOOK = { command: "sh -c 'cat >> /tmp/cursor-hook-payloads.jsonl; echo {}'" };

interface CloudVm extends Fixture {
  /** This install's package root, whose Cursor hook the entries run. */
  install: string;
  hooksFile: string;
  daemon: ReturnType<typeof vi.fn<() => Promise<CloudDaemon>>>;
}

async function cloudVm(): Promise<CloudVm> {
  const setup = await fixture();
  const install = join(setup.root, "rerail", "current");
  await mkdir(join(install, "dist"), { recursive: true });
  await writeFile(join(install, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "1.0.0" }));
  return { ...setup, install, hooksFile: join(setup.repo, ".cursor", "hooks.json"), daemon: vi.fn(async () => DAEMON) };
}

/** What `rerail setup` leaves in the database: the repository set up, with no pack. */
function setUp(vm: CloudVm): void {
  const database = openRerailDatabase(vm.databasePath);
  try {
    setUpRepository(database, recordCheckout(database, vm.repo).repositoryKey, { packs: [] });
  } finally {
    database.$client.close();
  }
}

async function startInCloud(vm: CloudVm, platform?: NodeJS.Platform): Promise<Recorded> {
  const io = agentIo();
  await startForCursorCloud({
    workspace: vm.repo, databasePath: vm.databasePath, root: vm.install, nodePath: NODE, say: text => io.say(text), daemon: vm.daemon,
    start: () => startRerail(vm.repo, io, { databasePath: vm.databasePath, environment: fakeSetupEnvironment() }),
    ...(platform === undefined ? {} : { platform }),
  });
  return io;
}

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", args, { cwd, encoding: "utf8" });
}

/** The hook line registration writes for Cursor: the recorded node, with node from PATH once it is gone. */
function rerailHook(vm: CloudVm): { command: string } {
  return { command: `sh -c 'if [ -x "$0" ]; then exec "$0" "$1"; fi; exec node "$1"' ${NODE} ${join(vm.install, "dist", "hosts", "cursor", "cursor-hook.js")}` };
}

/** The hooks file Cursor reads: each event's entries. */
const CursorHooksFile = z.object({ hooks: z.record(z.string(), z.array(z.unknown())) });

async function hooksOf(vm: CloudVm): Promise<Record<string, unknown[]>> {
  return CursorHooksFile.parse(JSON.parse(await readFile(vm.hooksFile, "utf8"))).hooks;
}

async function excludeLines(vm: CloudVm): Promise<string[]> {
  return (await readFile(join(vm.repo, ".git", "info", "exclude"), "utf8")).split("\n").filter(line => line === ".cursor/hooks.json");
}

describe("rerail start --cursor-cloud", { timeout: SETUP_TIMEOUT_MS }, () => {
  it("writes Rerail's hooks for every Cursor event into the checkout beside its own, excludes the file once, and changes nothing when run again", async () => {
    const vm = await cloudVm();
    setUp(vm);
    await mkdir(join(vm.repo, ".cursor"));
    await writeFile(vm.hooksFile, `${JSON.stringify({ version: 1, hooks: { stop: [OWN_HOOK] } }, null, 2)}\n`);

    const first = await startInCloud(vm);
    const hooks = await hooksOf(vm);
    expect(Object.keys(hooks).sort()).toEqual([...CURSOR_HOOK_EVENTS].sort());
    for (const event of CURSOR_HOOK_EVENTS) expect(hooks[event]?.at(-1)).toEqual(rerailHook(vm));
    expect(hooks.stop).toEqual([OWN_HOOK, rerailHook(vm)]);
    expect(await excludeLines(vm)).toHaveLength(1);
    expect(git(vm.repo, "status", "--porcelain")).toBe("");
    expect(existsSync(cursorCloudMarkerPath(vm.databasePath))).toBe(true);
    expect(vm.daemon).toHaveBeenCalledTimes(1);
    expect(first.text()).toContain(`Hooks: ${vm.hooksFile}, for 8 Cursor events (Rerail's entries added).`);
    expect(first.text()).toContain(`Daemon: running at ${DAEMON.url} (pid ${DAEMON.pid})`);

    const written = await readFile(vm.hooksFile, "utf8");
    const marker = await readFile(cursorCloudMarkerPath(vm.databasePath), "utf8");
    const again = await startInCloud(vm);
    expect(await readFile(vm.hooksFile, "utf8")).toBe(written);
    expect(await readFile(cursorCloudMarkerPath(vm.databasePath), "utf8")).toBe(marker);
    expect(await excludeLines(vm)).toHaveLength(1);
    expect(git(vm.repo, "status", "--porcelain")).toBe("");
    expect(vm.daemon).toHaveBeenCalledTimes(2);
    expect(again.text()).toContain("(already in place)");
    expect(again.text()).toContain("is listed in");
  });

  it("marks a hooks file the repository commits skip-worktree, so its own hooks stay and git status stays clean", async () => {
    const vm = await cloudVm();
    setUp(vm);
    await mkdir(join(vm.repo, ".cursor"));
    await writeFile(vm.hooksFile, `${JSON.stringify({ version: 1, hooks: { preToolUse: [OWN_HOOK] } }, null, 2)}\n`);
    git(vm.repo, "add", "--", ".cursor/hooks.json");
    git(vm.repo, "-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "-m", "cursor hooks");

    const io = await startInCloud(vm);
    expect((await hooksOf(vm)).preToolUse).toEqual([OWN_HOOK, rerailHook(vm)]);
    expect(git(vm.repo, "ls-files", "-v", "--", ".cursor/hooks.json")).toBe("S .cursor/hooks.json\n");
    expect(git(vm.repo, "status", "--porcelain")).toBe("");
    expect(await excludeLines(vm)).toEqual([]);
    expect(io.text()).toContain("--skip-worktree");

    const written = await readFile(vm.hooksFile, "utf8");
    await startInCloud(vm);
    expect(await readFile(vm.hooksFile, "utf8")).toBe(written);
    expect(git(vm.repo, "status", "--porcelain")).toBe("");
  });

  it("fails in a repository that is not set up, writing no hooks, no marker, and starting no daemon", async () => {
    const vm = await cloudVm();
    await expect(startInCloud(vm)).rejects.toThrow(/not set up.*--allow-sessions/u);
    expect(existsSync(join(vm.repo, ".cursor"))).toBe(false);
    expect(existsSync(cursorCloudMarkerPath(vm.databasePath))).toBe(false);
    expect(vm.daemon).not.toHaveBeenCalled();
    expect(git(vm.repo, "status", "--porcelain")).toBe("");
  });

  it("refuses on Windows before starting anything", async () => {
    const vm = await cloudVm();
    setUp(vm);
    await expect(startInCloud(vm, "win32")).rejects.toThrow(/does not run on Windows/u);
    expect(existsSync(vm.hooksFile)).toBe(false);
    expect(existsSync(cursorCloudMarkerPath(vm.databasePath))).toBe(false);
    expect(vm.daemon).not.toHaveBeenCalled();
  });
});
