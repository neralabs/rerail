/**
 * A coding agent's MCP bridge (`rerail mcp-bridge`, started in the agent's folder as its harness starts it) asks a
 * daemon of this build about the folder it works in, a plain folder of a started repository's checkout. The folder is
 * then made a repository of its own (`git init`) whose remote names a repository Rerail has set up and stopped. With no
 * restart, `repository_context_get` with no target names the new repository and the folder as its checkout, from the
 * same bridge and from a bridge started there after, as it does for a folder made a repository the same way that no
 * bridge asked about.
 */
import { execFile, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { promisify } from "node:util";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { ENTRY_POINTS } from "../../src/engine/index.js";
import { DAEMON_STATE_FILE } from "../../src/runtime/index.js";
import { removeFolder } from "../../test/support/platform/remove-folder.js";
import { setUpTestRepository } from "../../test/support/repository-rules.js";

const PACKAGE_ROOT = resolve(".");
const CLI = join(PACKAGE_ROOT, "dist", "cli", "cli.js");
const APP = "https://example.invalid/team/app.git";
const TOOLS = "https://example.invalid/team/tools.git";

let root: string;
let home: string;
let daemon: ChildProcess | undefined;

const environment = (): NodeJS.ProcessEnv => ({ ...process.env, RERAIL_HOME: home, BROWSER: "none" });

async function git(args: readonly string[], cwd: string): Promise<void> {
  await promisify(execFile)("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", ...args], { cwd, windowsHide: true });
}

/** `path` made a repository with one commit and `origin` at `remote`, as `git init` makes one in a folder that has files. */
async function repositoryAt(path: string, remote: string): Promise<string> {
  await git(["init", "-q", "-b", "main"], path);
  await git(["remote", "add", "origin", remote], path);
  await git(["add", "."], path);
  await git(["commit", "-q", "--allow-empty", "-m", "first"], path);
  return path;
}

/** A plain folder at `path` with one file. */
async function folderAt(path: string): Promise<string> {
  await mkdir(path, { recursive: true });
  await writeFile(join(path, "notes.txt"), "notes\n");
  return path;
}

async function startDaemon(): Promise<void> {
  const child = spawn(process.execPath, [join(PACKAGE_ROOT, ENTRY_POINTS.daemon), "--port", "0", "--state-file", join(home, DAEMON_STATE_FILE)], {
    env: environment(), stdio: ["ignore", "pipe", "ignore"], windowsHide: true,
  });
  daemon = child;
  await Promise.race([
    once(child.stdout!, "data"),
    once(child, "close").then(([code]) => { throw new Error(`daemon exited with code ${String(code)}`); }),
  ]);
  child.stdout!.resume();
}

async function stopDaemon(): Promise<void> {
  const child = daemon;
  if (child === undefined || child.exitCode !== null) return;
  const closed = once(child, "close");
  await promisify(execFile)(process.execPath, [CLI, "daemon", "stop"], { cwd: root, env: environment(), windowsHide: true }).catch(() => child.kill());
  await closed;
  daemon = undefined;
}

interface Told {
  readonly repository: string;
  readonly state: string;
  readonly checkout: { readonly path: string; readonly folder: string } | null;
}

/** `rerail mcp-bridge` started in `folder`, as a coding agent's harness starts it, held open for one call after another. */
class Bridge {
  readonly #child: ChildProcess;
  readonly #lines: AsyncIterator<string>;
  #id = 0;

  constructor(folder: string) {
    this.#child = spawn(process.execPath, [CLI, "mcp-bridge"], { cwd: folder, env: environment(), stdio: ["pipe", "pipe", "ignore"], windowsHide: true });
    this.#lines = createInterface({ input: this.#child.stdout!, crlfDelay: Infinity })[Symbol.asyncIterator]();
  }

  static async open(folder: string): Promise<Bridge> {
    const bridge = new Bridge(folder);
    await bridge.#ask("initialize", { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "agent", version: "1" } });
    bridge.#child.stdin!.write(`${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`);
    return bridge;
  }

  async #ask(method: string, params: Record<string, unknown>): Promise<unknown> {
    this.#id += 1;
    this.#child.stdin!.write(`${JSON.stringify({ jsonrpc: "2.0", id: this.#id, method, params })}\n`);
    const line = await this.#lines.next();
    if (line.done === true) throw new Error(`the bridge ended before it answered ${method}`);
    return JSON.parse(line.value);
  }

  /** What `repository_context_get` with no target tells the agent: the repository, its state, and its checkout and folder. */
  async told(): Promise<Told> {
    const answer = await this.#ask("tools/call", { name: "repository_context_get", arguments: {} }) as {
      result: { structuredContent?: { repository: { repositoryKey: string; state: string }; checkout: { checkoutPath: string; folder: string } | null } };
    };
    const context = answer.result.structuredContent;
    expect(context, JSON.stringify(answer)).toBeDefined();
    const checkout = context!.checkout;
    return { repository: context!.repository.repositoryKey, state: context!.repository.state, checkout: checkout === null ? null : { path: checkout.checkoutPath, folder: checkout.folder } };
  }

  async close(): Promise<void> {
    const closed = once(this.#child, "close");
    this.#child.stdin!.end();
    await closed;
  }
}

/** What a bridge started in `folder` is told at its first call. */
async function toldByNewBridge(folder: string): Promise<Told> {
  const bridge = await Bridge.open(folder);
  try {
    return await bridge.told();
  } finally {
    await bridge.close();
  }
}

let app: string;
let tools: string;

beforeAll(async () => {
  root = await realpath(await mkdtemp(join(tmpdir(), "rerail-bridge-folder-repository-")));
  home = join(root, "home");
  const database = join(home, "rerail.sqlite");
  // App is set up and started; tools, a clone elsewhere, is set up and stopped.
  app = setUpTestRepository(await repositoryAt(await folderAt(join(root, "app")), APP), {}, database);
  tools = setUpTestRepository(await repositoryAt(await folderAt(join(root, "tools")), TOOLS), { start: false }, database);
  await startDaemon();
}, 120_000);

afterAll(async () => {
  await stopDaemon();
  await removeFolder(root);
}, 60_000);

describe("an MCP bridge's folder, made a repository of its own while the daemon runs", () => {
  it("is named as the new repository's checkout at the next call, from the same bridge and from a new one", async () => {
    const folder = await folderAt(join(root, "app", "lib"));
    const neverAsked = await folderAt(join(root, "app", "fresh"));
    const bridge = await Bridge.open(folder);
    try {
      expect(await bridge.told()).toEqual({ repository: app, state: "started", checkout: { path: join(root, "app"), folder: "lib" } });

      await repositoryAt(folder, TOOLS);
      await repositoryAt(neverAsked, TOOLS);

      const madeTheSameWay = { repository: tools, state: "stopped", checkout: { path: neverAsked, folder: "" } };
      expect(await toldByNewBridge(neverAsked)).toEqual(madeTheSameWay);
      expect(await bridge.told()).toEqual({ ...madeTheSameWay, checkout: { path: folder, folder: "" } });
      expect(await toldByNewBridge(folder)).toEqual({ ...madeTheSameWay, checkout: { path: folder, folder: "" } });
    } finally {
      await bridge.close();
    }
  }, 90_000);
});
