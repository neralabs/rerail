import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { mkdir, realpath, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import type { AdapterDelivery, AdapterEvent, NativeBinding } from "../../../src/engine/index.js";
import { RerailRuntime } from "../../../src/runtime/index.js";
import { sessionDatabaseQuery, temporaryDirectories, testEmbedder } from "../../support/hook-events.js";
import { installPackDirectory } from "../../support/installed-packs.js";
import { setUpTestRepository } from "../../support/repository-rules.js";

/**
 * The engine's rule compiles, as the daemon asks for them: each one run on the JS thread, the daemon's one thread, is
 * counted; each one run off it is counted too, and one of a rule whose id `hold` names waits until `release` is called.
 */
const compiles = vi.hoisted(() => ({
  onThread: 0,
  offThread: 0,
  offThreadIds: [] as string[],
  holding: 0,
  hold: undefined as { id: string; released: Promise<void> } | undefined,
}));

vi.mock("../../../src/engine/native/native.js", async importOriginal => {
  const actual = await importOriginal<{ loadNativeBinding: () => NativeBinding }>();
  let counted: NativeBinding | undefined;
  const count = (binding: NativeBinding): NativeBinding => {
    const engine = binding.CodeRuleDslRuntime;
    const runtime = {
      pythonAstBatch: (documents: string) => engine.pythonAstBatch(documents),
      testRr: (paths: string, filter?: string | null) => engine.testRr(paths, filter),
      compile: (...args: Parameters<typeof engine.compile>) => {
        compiles.onThread += 1;
        return engine.compile(...args);
      },
      compileAsync: async (...args: Parameters<typeof engine.compile>) => {
        compiles.offThread += 1;
        compiles.offThreadIds.push((JSON.parse(args[0]) as { id: string }).id);
        const hold = compiles.hold;
        if (hold !== undefined && args[0].includes(`"id":"${hold.id}"`)) {
          compiles.holding += 1;
          await hold.released;
          compiles.holding -= 1;
        }
        // Absent from an engine built before the daemon compiled rules off its thread: the call fails there.
        const off = (engine as unknown as { compileAsync: typeof engine.compileAsync }).compileAsync;
        return await off(...args);
      },
    };
    return new Proxy(binding, { get: (target, key) => (key === "CodeRuleDslRuntime" ? runtime : Reflect.get(target, key)) });
  };
  return { ...actual, loadNativeBinding: () => (counted ??= count(actual.loadNativeBinding())) };
});

const temporaryDirectory = temporaryDirectories("rerail-rule-compile-");

/** A steer rule that flags `NEEDLE` in any Python file, reporting `code` with `message`. */
function needle(id: string, code: string, message: string): unknown {
  return {
    format: "rerail.policy/v1", id, revision: 1, mode: "steer", description: `Keep NEEDLE out of Python files (${code}).`,
    scope: { include: ["**/*.py"], exclude: [] },
    clauses: [{ id: "needle", violation: { code, anchor: "witness.span", message }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
  };
}

/** Writes and installs in the test's Rerail home the pack `id` at `version`, holding one needle rule `<id>-needle` that says `message`. */
function installNeedlePack(folder: string, id: string, message: string, version = "1.0.0"): void {
  const directory = join(folder, id);
  mkdirSync(join(directory, "rules"), { recursive: true });
  const rule = `${id}-needle`;
  writeFileSync(join(directory, "package.json"), JSON.stringify({
    name: `@neralabs/rerail-pack-${id}`, version, license: "Apache-2.0", files: ["pack.json", "rules"], peerDependencies: { "@neralabs/rerail": ">=0.1.0-0" },
  }));
  writeFileSync(join(directory, "pack.json"), JSON.stringify({
    format: "rerail.rule-pack/v1", id, name: id, category: "language", covers: `Test pack ${id}.`, reason: "matches Python",
    match: { any: [{ files: ["**/*.py"], label: "*.py files" }] },
    rules: [{ id: rule, document: `rules/${rule}.json`, title: rule, group: id }],
  }));
  writeFileSync(join(directory, "rules", `${rule}.json`), JSON.stringify(needle(rule, id.toUpperCase(), message)));
  // Recorded as installed from the registry at `version`: Rerail's own pack from npm, which steers with no approval.
  installPackDirectory(directory, `@neralabs/rerail-pack-${id}`, undefined, version);
}

async function repository(path: string): Promise<string> {
  await mkdir(join(path, "api"), { recursive: true });
  await writeFile(join(path, "README.md"), "seed\n");
  await writeFile(join(path, "api", "x.py"), "def x():\n    return 1\n");
  const git = (...args: string[]) => execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd: path, stdio: "ignore" });
  git("init", "--quiet");
  git("add", "-A");
  git("commit", "--quiet", "-m", "seed");
  return await realpath(path);
}

/**
 * Two repositories, each running a pack of its own: the sessions' folder, and a sibling one of them edits by absolute
 * path. A first daemon opens a session there (`editFirst`: and edits the sibling, so the session's record shows it); the
 * sibling's rule then changes, so the daemon started after it finds no compiled copy of it anywhere, as a restart into a
 * repository no other one shares rules with does.
 */
async function restartedDaemon(editFirst: boolean) {
  const root = await realpath(await temporaryDirectory({ started: false }));
  const databasePath = join(root, "rerail.sqlite");
  const folder = await repository(join(root, "session"));
  const sibling = await repository(join(root, "sibling"));
  installNeedlePack(join(root, "packs"), "rcsession", "No NEEDLE in the session's folder");
  installNeedlePack(join(root, "packs"), "rcsibling", "No NEEDLE in the sibling");
  setUpTestRepository(folder, { packs: ["rcsession"] }, databasePath);
  setUpTestRepository(sibling, { packs: ["rcsibling"] }, databasePath);
  const open = () => new RerailRuntime({ storageRoot: join(root, "sessions"), databasePath, workspaceRoot: folder, embedder: testEmbedder, autostart: false });
  const first = { host: "claude-code" as const, sessionId: "first", cwd: folder };
  const second = { host: "claude-code" as const, sessionId: "second", cwd: folder };
  let calls = 0;
  /** One tool call of `session`: its call, the tool's work, its result; the answers of both. */
  const call = (runtime: RerailRuntime, session: typeof first, path: string, content?: string) => {
    calls += 1;
    const toolCallId = `t${calls}`;
    const input = content === undefined ? { file_path: path } : { file_path: path, content };
    const toolName = content === undefined ? "Read" : "Write";
    const event = (kind: "call" | "result"): AdapterEvent => (kind === "call"
      ? { kind, eventId: `${toolCallId}-call`, sessionId: session.sessionId, session, toolCallId, toolName, input }
      : { kind, eventId: `${toolCallId}-result`, sessionId: session.sessionId, session, toolCallId, toolName, output: "ok" });
    return {
      call: () => runtime.ingest(event("call")),
      result: async () => {
        if (content !== undefined) await writeFile(path, content);
        return await runtime.ingest(event("result"));
      },
    };
  };
  const before = open();
  await before.ingest({ kind: "session-start", eventId: "start", session: first });
  await before.codeRulesOpened(first.sessionId);
  if (editFirst) {
    const earlier = call(before, first, join(sibling, "api", "x.py"), "def x():\n    return 2\n");
    await earlier.call();
    await earlier.result();
  }
  await before.dispose();
  installNeedlePack(join(root, "packs"), "rcsibling", "No NEEDLE in the sibling, still", "1.0.1");
  const runtime = open();
  /** What the sibling's checks recorded, to show beside a steer that did not come. */
  const checks = () => JSON.stringify(sessionDatabaseQuery(root, "SELECT session_id, status, violations_json, failures_json FROM audits ORDER BY started_at"));
  return { folder, sibling, runtime, first, second, call, checks };
}

const ruleIds = (deliveries: readonly AdapterDelivery[]) => deliveries.flatMap(delivery => delivery.ruleIds);

describe("a session's first edit in a repository after the daemon restarts", () => {
  it("compiles none of the repository's rules on the daemon's thread, and another session's call is answered while that edit waits for them", async () => {
    const daemon = await restartedDaemon(false);
    const { runtime, first, second } = daemon;
    try {
      const released = Promise.withResolvers<void>();
      compiles.hold = { id: "rcsibling-needle", released: released.promise };
      await runtime.ingest({ kind: "session-start", eventId: "second-start", session: second });
      await runtime.codeRulesOpened(second.sessionId);
      const opening = daemon.call(runtime, first, join(daemon.folder, "README.md"));
      await opening.call();
      await opening.result();
      await runtime.codeRulesOpened(first.sessionId);
      compiles.onThread = 0;

      const edit = daemon.call(runtime, first, join(daemon.sibling, "api", "x.py"), "NEEDLE = 1\n");
      let answered = false;
      const editCall = edit.call().finally(() => { answered = true; });
      await vi.waitFor(() => expect(answered || compiles.holding > 0).toBe(true), { timeout: 10_000, interval: 10 });
      expect(compiles.onThread).toBe(0);
      expect(answered).toBe(false);

      const other = daemon.call(runtime, second, join(daemon.folder, "README.md"));
      await other.call();
      await other.result();
      expect(answered).toBe(false);

      released.resolve();
      await editCall;
      expect(ruleIds(await edit.result())).toContain("rcsibling-needle");
      expect(compiles.onThread).toBe(0);
    } finally {
      compiles.hold = undefined;
      await runtime.dispose();
    }
  }, 60_000);

  it("steers that edit in its own result, as before", async () => {
    const daemon = await restartedDaemon(true);
    const { runtime, first } = daemon;
    try {
      const edit = daemon.call(runtime, first, join(daemon.sibling, "api", "x.py"), "NEEDLE = 1\n");
      await edit.call();
      const result = await edit.result();
      expect(ruleIds(result), `${JSON.stringify(result)} ${daemon.checks()}`).toContain("rcsibling-needle");
    } finally {
      await runtime.dispose();
    }
  }, 60_000);

  it("has the repository's rules compiled before the session's first edit there, once the session opened again", async () => {
    const daemon = await restartedDaemon(true);
    const { runtime, first } = daemon;
    try {
      const opening = daemon.call(runtime, first, join(daemon.folder, "README.md"));
      await opening.call();
      await opening.result();
      await runtime.codeRulesOpened(first.sessionId);
      await runtime.rulesBuiltAhead(first.sessionId);
      expect(compiles.offThread).toBeGreaterThan(0);
      compiles.onThread = 0;
      compiles.offThread = 0;

      const edit = daemon.call(runtime, first, join(daemon.sibling, "api", "x.py"), "NEEDLE = 1\n");
      await edit.call();
      expect({ onThread: compiles.onThread, offThread: compiles.offThread }).toEqual({ onThread: 0, offThread: 0 });
      expect(ruleIds(await edit.result())).toContain("rcsibling-needle");
    } finally {
      await runtime.dispose();
    }
  }, 60_000);
});

describe("a session's first edit in a started repository kept inside its folder", () => {
  it("finds that repository's rules compiled, built off the daemon's thread as the session opened, and is steered at its result", async () => {
    const root = await realpath(await temporaryDirectory({ started: false }));
    const databasePath = join(root, "rerail.sqlite");
    const folder = await repository(join(root, "session"));
    await writeFile(join(folder, ".gitignore"), ".scratch/\n");
    // A clone kept in a folder the session's repository ignores, started on its own, as an agent's work on a dependency is.
    const nested = await repository(join(folder, ".scratch", "nested"));
    installNeedlePack(join(root, "packs"), "rcsession", "No NEEDLE in the session's folder");
    installNeedlePack(join(root, "packs"), "rcnested", "No NEEDLE in the nested clone");
    setUpTestRepository(folder, { packs: ["rcsession"] }, databasePath);
    setUpTestRepository(nested, { packs: ["rcnested"] }, databasePath);
    const runtime = new RerailRuntime({ storageRoot: join(root, "sessions"), databasePath, workspaceRoot: folder, embedder: testEmbedder, autostart: false });
    const session = { host: "claude-code" as const, sessionId: "nested", cwd: folder };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await runtime.codeRulesOpened(session.sessionId);
      await runtime.rulesBuiltAhead(session.sessionId);
      expect(compiles.offThreadIds).toContain("rcnested-needle");
      compiles.onThread = 0;
      compiles.offThread = 0;

      const path = join(nested, "api", "x.py");
      const input = { file_path: path, content: "NEEDLE = 1\n" };
      await runtime.ingest({ kind: "call", eventId: "w-call", sessionId: session.sessionId, session, toolCallId: "w", toolName: "Write", input });
      expect({ onThread: compiles.onThread, offThread: compiles.offThread }).toEqual({ onThread: 0, offThread: 0 });
      await writeFile(path, input.content);
      const result = await runtime.ingest({ kind: "result", eventId: "w-result", sessionId: session.sessionId, session, toolCallId: "w", toolName: "Write", output: "ok" });
      expect(ruleIds(result)).toContain("rcnested-needle");
    } finally {
      await runtime.dispose();
    }
  }, 60_000);
});

describe("a session that opens", () => {
  it("compiles none of its own repository's rules on the daemon's thread, and records their count as its start is answered", async () => {
    const root = await realpath(await temporaryDirectory({ started: false }));
    const databasePath = join(root, "rerail.sqlite");
    const folder = await repository(join(root, "session"));
    installNeedlePack(join(root, "packs"), "rcopen", "No NEEDLE where the session opens");
    setUpTestRepository(folder, { packs: ["rcopen"] }, databasePath);
    const runtime = new RerailRuntime({ storageRoot: join(root, "sessions"), databasePath, workspaceRoot: folder, embedder: testEmbedder, autostart: false });
    const session = { host: "claude-code" as const, sessionId: "opens", cwd: folder };
    try {
      compiles.onThread = 0;
      compiles.offThreadIds.length = 0;
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      expect({ onThread: compiles.onThread, offThread: compiles.offThreadIds.includes("rcopen-needle") }).toEqual({ onThread: 0, offThread: true });
      const coverage = sessionDatabaseQuery<{ payload: string }>(root, "SELECT payload_json AS payload FROM events WHERE kind = 'rule-coverage'")
        .map(row => (JSON.parse(row.payload) as { activeRuleCount: number }).activeRuleCount);
      expect(coverage).toEqual([1]);
      await runtime.codeRulesOpened(session.sessionId);
      expect(compiles.onThread).toBe(0);
    } finally {
      await runtime.dispose();
    }
  }, 60_000);
});
