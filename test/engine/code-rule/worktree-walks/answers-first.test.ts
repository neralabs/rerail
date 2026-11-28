/**
 * The engine service's walk of the other worktrees, which no answer waits for, asks the disk nothing more while a hook
 * waits for an answer: its batches of looks would fill the thread pool and the event loop the answer's own reads wait
 * in. It reads the rest once the answer is written.
 */
import { execFileSync } from "node:child_process";
import type * as FileSystem from "node:fs/promises";
import { mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, sep } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { afterEach, describe, expect, it, vi } from "vitest";
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

/** A set-up repository at `<root>/main` with worktrees `w1`, `w2` and `w3`, and `files` files in folders of ten in `w2`. */
async function repository(files: number): Promise<{ root: string; databasePath: string; w1: string; w2: string; w3: string }> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-answers-first-")));
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
  for (let index = 0; index < files; index++) {
    await mkdir(join(w2, "many", `d${Math.floor(index / 10)}`), { recursive: true });
    await writeFile(join(w2, "many", `d${Math.floor(index / 10)}`, `f${index}.py`), `x = ${index}\n`);
  }
  return { root, databasePath, w1, w2, w3 };
}

function socketPathIn(root: string): string {
  return process.platform === "win32" ? `\\\\.\\pipe\\rerail-answers-first-${process.pid}` : join(root, "engine.sock");
}

/** Who waits for the answer: a hook, over the engine socket, or an OMP or Pi session, through the service itself. */
const askers: ReadonlyArray<readonly [string, (service: EngineService, event: AdapterEvent) => Promise<unknown>]> = [
  ["a hook", async (service, event) => await ingestEngineEvent({ socketPath: service.socketPath }, event)],
  ["an OMP or Pi session", async (service, event) => await service.ingest(event)],
];

describe("the walk of the other worktrees while an answer is awaited", () => {
  it.each(askers)("looks at no more than one batch of another worktree while %s waits, then at the rest", { timeout: 90_000 }, async (_asker, ask) => {
    const { root, databasePath, w1, w2, w3 } = await repository(300);
    const opens = vi.spyOn(CodeRuleBridge.prototype, "openWorkspace");
    const service = await startEngineService({ socketPath: socketPathIn(root), storageRoot: join(root, "sessions"), databasePath, embedder: testEmbedder });
    const sessions = [w1, w3].map((cwd, index) => ({ host: "claude-code" as const, sessionId: `session-${index + 1}`, cwd }));
    const held = Promise.withResolvers<void>();
    try {
      for (const session of sessions) await service.ingest({ kind: "session-start", eventId: `${session.sessionId}:start`, session });
      await vi.waitFor(() => expect(opens.mock.results).toHaveLength(sessions.length), { timeout: 30_000, interval: 20 });
      await Promise.all(opens.mock.results.map(result => result.value));
      const [first] = opens.mock.contexts as CodeRuleBridge[];
      for (const bridge of opens.mock.contexts as CodeRuleBridge[]) {
        await bridge.checkpoint("turn");
        await bridge.settleWorktreeWalk();
      }
      // `w2` changes, so the next walk of the other worktrees reads it whole.
      await writeFile(join(w2, "many", "d0", "f0.py"), "x = 'changed'\n");

      // The second session's turn end waits on its folder's walk, held at its first listing: its hook waits meanwhile.
      counted.held.set(w3, async () => await held.promise);
      const answered = ask(service, { kind: "turn-end", eventId: "stop", sessionId: "session-2", text: "" });
      await vi.waitFor(() => expect(counted.held.has(w3)).toBe(false), { timeout: 30_000, interval: 10 });
      counted.looks.length = 0;
      // The first session's turn asks for a walk of the other worktrees, which starts now, while that answer is awaited.
      await first!.checkpoint("turn");
      const walked = first!.settleWorktreeWalk();
      await vi.waitFor(() => expect(looksUnder(join(root, "main")) + looksUnder(w2) + looksUnder(w3)).toBeGreaterThan(0), { timeout: 30_000, interval: 10 });
      // Time for many batches: a walk that went on would look at all 300 files of `w2` and their 30 folders by now.
      await sleep(1_000);
      // One batch at most of `w2` (16 looks), and the look at its top that began it.
      expect(looksUnder(w2)).toBeLessThanOrEqual(17);

      held.resolve();
      await answered;
      await walked;
      expect(looksUnder(w2)).toBeGreaterThanOrEqual(300);
    } finally {
      held.resolve();
      await service.close();
    }
  });
});
