/**
 * Which side of a hook's answer each part of a walk falls on, through the engine service, where the answers hooks wait
 * for are counted (`AwaitedAnswers`). The session folder's own walk is part of its answer: a command's write there steers
 * at that command's hook, while the read of the other worktrees waits. The read of the other worktrees, and the take
 * after it (the one `lstat` of each source the session saw in a folder no walk enters), come after: a deletion found
 * there is recorded once the answer is written, and the session's next answer follows it.
 */
import { execFileSync } from "node:child_process";
import type * as FileSystem from "node:fs/promises";
import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative, sep } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CheckoutCodeRuleRouter } from "../../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CodeRuleBridge } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import { codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import type { AdapterEvent } from "../../../../src/engine/adapter-contract.js";
import { type EngineService, ingestEngineEvent, startEngineService } from "../../../../src/runtime/index.js";
import { testEmbedder } from "../../../support/hook-events.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";

/** Every path the walks looked at (`lstat`), and the listings held back until a case lets them go, by folder. */
const counted = vi.hoisted(() => ({ looks: [] as string[], held: new Map<string, () => Promise<void>>() }));
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  return {
    ...actual,
    lstat: async (...args: Parameters<typeof actual.lstat>) => {
      counted.looks.push(String(args[0]));
      return await actual.lstat(...args);
    },
    opendir: async (...args: Parameters<typeof actual.opendir>) => {
      const hold = counted.held.get(String(args[0]));
      counted.held.delete(String(args[0]));
      await hold?.();
      return await actual.opendir(...args);
    },
  };
});

const directories: string[] = [];
afterEach(async () => {
  counted.held.clear();
  counted.looks.length = 0;
  vi.restoreAllMocks();
  await Promise.all(directories.splice(0).map(async path => await removeFolder(path)));
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

const printRule = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:print", revision: 1, mode: "steer", scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "print", violation: { code: "P", anchor: "witness.span", message: "No print" }, forbid: { ast: { language: "python", rule: { pattern: "print($ARG)" } } } }],
});

/** The paths under `folder` the walks looked at so far. */
function looksUnder(folder: string): number {
  return counted.looks.filter(path => path === folder || path.startsWith(`${folder}${sep}`)).length;
}

interface Layout {
  readonly root: string;
  readonly databasePath: string;
  readonly w1: string;
  readonly w2: string;
  readonly w3: string;
}

/** A set-up repository at `<root>/main` with worktrees `w1`, `w2` and `w3`, and 300 files in folders of ten in `w2`. */
async function repository(): Promise<Layout> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-answer-sides-")));
  directories.push(root);
  const main = join(root, "main");
  await mkdir(join(main, "src"), { recursive: true });
  await writeFile(join(main, "src", "x.py"), "x = 1\n");
  git(main, "init", "--quiet");
  git(main, "add", ".");
  git(main, "commit", "--quiet", "-m", "seed");
  const databasePath = join(root, "rerail.sqlite");
  setUpTestRepository(main, { library: [{ kind: "code", rule: printRule }] }, databasePath);
  const [w1, w2, w3] = ["w1", "w2", "w3"].map(name => join(root, name)) as [string, string, string];
  for (const worktree of [w1, w2, w3]) git(main, "worktree", "add", "--quiet", "--detach", worktree);
  for (let index = 0; index < 300; index++) {
    await mkdir(join(w2, "many", `d${Math.floor(index / 10)}`), { recursive: true });
    await writeFile(join(w2, "many", `d${Math.floor(index / 10)}`, `f${index}.py`), `x = ${index}\n`);
  }
  return { root, databasePath, w1, w2, w3 };
}

function socketPathIn(root: string): string {
  return process.platform === "win32" ? `\\\\.\\pipe\\rerail-answer-sides-${process.pid}` : join(root, "engine.sock");
}

interface Running {
  readonly service: EngineService;
  /** The bridge of `session-1`, in `w1`. */
  readonly first: CodeRuleBridge;
  /** Sends `event` as a hook does, over the engine socket, and resolves with the answer's deliveries. */
  readonly hook: (event: AdapterEvent) => ReturnType<typeof ingestEngineEvent>;
}

/** The engine service with `session-1` open in `w1` and `session-2` in `w3`, each after a turn and its walk. */
async function running({ root, databasePath, w1, w3 }: Layout): Promise<Running> {
  const opens = vi.spyOn(CodeRuleBridge.prototype, "openWorkspace");
  const service = await startEngineService({ socketPath: socketPathIn(root), storageRoot: join(root, "sessions"), databasePath, embedder: testEmbedder });
  const sessions = [w1, w3].map((cwd, index) => ({ host: "claude-code" as const, sessionId: `session-${index + 1}`, cwd }));
  for (const session of sessions) await service.ingest({ kind: "session-start", eventId: `${session.sessionId}:start`, session });
  await vi.waitFor(() => expect(opens.mock.results).toHaveLength(sessions.length), { timeout: 30_000, interval: 20 });
  await Promise.all(opens.mock.results.map(result => result.value));
  for (const bridge of opens.mock.contexts as CodeRuleBridge[]) {
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
  }
  const [first] = opens.mock.contexts as CodeRuleBridge[];
  return { service, first: first!, hook: async event => await ingestEngineEvent({ socketPath: service.socketPath }, event) };
}

/**
 * Holds `session-2`'s turn end at its folder's first listing, so its hook waits, then starts the walk of the other
 * worktrees through `session-1` while that answer is awaited. Returns the held answer, the walk, and the release.
 */
async function walkWhileAnswerHeld({ first, hook }: Running, { w2, w3 }: Layout) {
  const held = Promise.withResolvers<void>();
  counted.held.set(w3, async () => await held.promise);
  const answered = hook({ kind: "turn-end", eventId: "held-stop", sessionId: "session-2", text: "" });
  await vi.waitFor(() => expect(counted.held.has(w3)).toBe(false), { timeout: 30_000, interval: 10 });
  counted.looks.length = 0;
  await first.checkpoint("turn");
  const walked = first.settleWorktreeWalk();
  await vi.waitFor(() => expect(looksUnder(w2)).toBeGreaterThan(0), { timeout: 30_000, interval: 10 });
  // Time for many batches: a read that went on would look at all 300 files of `w2` and their 30 folders by now.
  await sleep(1_000);
  return { answered, walked, release: () => held.resolve() };
}

let calls = 0;

/** A tool call of `session-1` through the hook: its call, `write` meanwhile, and its result's answer. */
async function toolCall({ hook }: Running, toolName: "Bash" | "Write", input: Record<string, unknown>, write: () => Promise<void>) {
  const toolCallId = `call-${++calls}`;
  const tool = { sessionId: "session-1", toolCallId, toolName } as const;
  await hook({ kind: "call", eventId: `${toolCallId}:call`, ...tool, input });
  await write();
  return await hook({ kind: "result", eventId: `${toolCallId}:result`, ...tool, output: "ok" });
}

describe("the sides of a hook's answer", () => {
  it("steers a command's write in the session's folder at that command's hook while the read of the other worktrees waits", { timeout: 90_000 }, async () => {
    const layout = await repository();
    const service = await running(layout);
    let release = (): void => undefined;
    try {
      // `w2` changes, so the next walk of the other worktrees reads it whole.
      await writeFile(join(layout.w2, "many", "d0", "f0.py"), "x = 'changed'\n");
      const walk = await walkWhileAnswerHeld(service, layout);
      release = walk.release;
      const paused = looksUnder(layout.w2);

      const deliveries = await toolCall(service, "Bash", { command: "./write.sh" }, async () => {
        await writeFile(join(layout.w1, "src", "x.py"), "print(1)\n");
      });

      // The command's own write is found by its folder's walk, which its answer waits for: it steers here.
      expect(deliveries.flatMap(delivery => delivery.content.split("\n")).some(line => line.includes("No print"))).toBe(true);
      // Meanwhile the read of the other worktrees stayed at its first batch of `w2`, and the look at its top.
      expect({ paused: paused <= 17, after: looksUnder(layout.w2) <= 17 }).toEqual({ paused: true, after: true });

      walk.release();
      await walk.answered;
      await walk.walked;
      expect(looksUnder(layout.w2)).toBeGreaterThanOrEqual(300);
    } finally {
      release();
      await service.service.close();
    }
  });

  it("records the deletion of a source the session saw in a folder no walk enters once the answer is written, before the session's next answer", { timeout: 90_000 }, async () => {
    const layout = await repository();
    const service = await running(layout);
    let release = (): void => undefined;
    try {
      const built = join(layout.w2, "build", "gen.py");
      await toolCall(service, "Write", { file_path: built, content: "g = 1\n" }, async () => {
        await mkdir(join(layout.w2, "build"), { recursive: true });
        await writeFile(built, "g = 1\n");
      });
      await service.hook({ kind: "turn-end", eventId: "stop-1", sessionId: "session-1", text: "" });
      await service.first.settleWorktreeWalk();
      await rm(built);
      await writeFile(join(layout.w2, "many", "d0", "f0.py"), "x = 'changed'\n");
      const audits = vi.spyOn(CheckoutCodeRuleRouter.prototype, "auditGeneratedEdits");
      const deleted = (): string[] => audits.mock.calls.flatMap(([, batch]) => batch.edits.filter(edit => edit.deleted === true).map(edit => edit.path));
      const named = relative(layout.w1, built).split(sep).join("/");

      const walk = await walkWhileAnswerHeld(service, layout);
      release = walk.release;
      // While the answer is awaited the read waits at its first batch, so the take after it has asked nothing yet.
      const whileAwaited = { looks: looksUnder(layout.w2) <= 17, built: counted.looks.filter(path => path === built).length, deleted: deleted() };

      walk.release();
      await walk.answered;
      await walk.walked;
      const afterAnswer = { built: counted.looks.filter(path => path === built).length, deleted: deleted() };
      const next = await service.hook({ kind: "turn-end", eventId: "stop-2", sessionId: "session-1", text: "" });

      expect(whileAwaited).toEqual({ looks: true, built: 0, deleted: [] });
      // Once the answer is written the take asks one `lstat` of the source, finds it gone, and records the deletion.
      expect(afterAnswer).toEqual({ built: 1, deleted: [named] });
      expect(service.first.observedDigest(named)).toBeUndefined();
      expect(next).toBeDefined();
    } finally {
      release();
      await service.service.close();
    }
  });
});
