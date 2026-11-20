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
 * `withoutAsync` stands in an engine built before `compileAsync`, which lacks it; `offThreadDone` names each compile off
 * the thread that ended.
 */
const compiles = vi.hoisted(() => ({
  onThread: 0,
  offThread: 0,
  offThreadIds: [] as string[],
  offThreadDone: [] as string[],
  holding: 0,
  hold: undefined as { id: string; released: Promise<void> } | undefined,
  withoutAsync: false,
}));

vi.mock("../../../src/engine/native/native.js", async importOriginal => {
  const actual = await importOriginal<{ loadNativeBinding: () => NativeBinding }>();
  let counted: NativeBinding | undefined;
  const count = (binding: NativeBinding): NativeBinding => {
    const engine = binding.CodeRuleDslRuntime;
    const compileAsync = async (...args: Parameters<typeof engine.compile>) => {
      const id = (JSON.parse(args[0]) as { id: string }).id;
      compiles.offThread += 1;
      compiles.offThreadIds.push(id);
      const hold = compiles.hold;
      if (hold?.id === id) {
        compiles.holding += 1;
        await hold.released;
        compiles.holding -= 1;
      }
      try {
        return await engine.compileAsync!(...args);
      } finally {
        compiles.offThreadDone.push(id);
      }
    };
    const runtime = {
      pythonAstBatch: (documents: string) => engine.pythonAstBatch(documents),
      testRr: (paths: string, filter?: string | null) => engine.testRr(paths, filter),
      compile: (...args: Parameters<typeof engine.compile>) => {
        compiles.onThread += 1;
        return engine.compile(...args);
      },
      get compileAsync() {
        return compiles.withoutAsync ? undefined : compileAsync;
      },
    };
    return new Proxy(binding, { get: (target, key) => (key === "CodeRuleDslRuntime" ? runtime : Reflect.get(target, key)) });
  };
  return { ...actual, loadNativeBinding: () => (counted ??= count(actual.loadNativeBinding())) };
});

/** Settles once every task the JS thread has queued now has run: what a compile that ended starts next has started. */
const queuedWorkRan = () => new Promise<void>(resolve => setImmediate(resolve));

const temporaryDirectory = temporaryDirectories("rerail-rule-compile-");

/** A steer rule that flags `NEEDLE` in any Python file, reporting `code` with `message`, its title. */
function needle(id: string, code: string, message: string): unknown {
  return {
    format: "rerail.policy/v1", id, revision: 1, mode: "steer", title: message, description: `Keep NEEDLE out of Python files (${code}).`,
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
 * repository no other one shares rules with does. `siblingPack` names the sibling's pack, whose rule is `<id>-needle`.
 */
async function restartedDaemon(editFirst: boolean, siblingPack = "rcsibling") {
  const root = await realpath(await temporaryDirectory({ started: false }));
  const databasePath = join(root, "rerail.sqlite");
  const folder = await repository(join(root, "session"));
  const sibling = await repository(join(root, "sibling"));
  installNeedlePack(join(root, "packs"), "rcsession", "No NEEDLE in the session's folder");
  installNeedlePack(join(root, "packs"), siblingPack, "No NEEDLE in the sibling");
  setUpTestRepository(folder, { packs: ["rcsession"] }, databasePath);
  setUpTestRepository(sibling, { packs: [siblingPack] }, databasePath);
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
  installNeedlePack(join(root, "packs"), siblingPack, "No NEEDLE in the sibling, still", "1.0.1");
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

  it("names the rule by its own title in the person's line, whether it was compiled ahead or at that edit", async () => {
    for (const ahead of [true, false]) {
      const pack = ahead ? "rctitleahead" : "rctitlecall";
      const daemon = await restartedDaemon(ahead, pack);
      const { runtime, first } = daemon;
      try {
        compiles.offThreadIds.length = 0;
        const opening = daemon.call(runtime, first, join(daemon.folder, "README.md"));
        await opening.call();
        await opening.result();
        await runtime.codeRulesOpened(first.sessionId);
        await runtime.rulesBuiltAhead(first.sessionId);
        expect(compiles.offThreadIds.includes(`${pack}-needle`)).toBe(ahead);
        compiles.onThread = 0;

        const edit = daemon.call(runtime, first, join(daemon.sibling, "api", "x.py"), "NEEDLE = 1\n");
        await edit.call();
        expect(compiles.offThreadIds).toContain(`${pack}-needle`);
        const result = await edit.result();
        const rules = result.flatMap(delivery => delivery.rules ?? []);
        expect(rules, `${JSON.stringify(result)} ${daemon.checks()}`)
          .toContainEqual(expect.objectContaining({ id: `${pack}-needle`, title: "No NEEDLE in the sibling, still" }));
        expect(compiles.onThread).toBe(0);
      } finally {
        await runtime.dispose();
      }
    }
  }, 120_000);

  it("compiles them on the daemon's thread with an engine built before it could compile off it, and steers that edit as before", async () => {
    const daemon = await restartedDaemon(true, "rcolder");
    const { runtime, first } = daemon;
    compiles.withoutAsync = true;
    try {
      compiles.onThread = 0;
      const edit = daemon.call(runtime, first, join(daemon.sibling, "api", "x.py"), "NEEDLE = 1\n");
      await edit.call();
      const result = await edit.result();
      expect(ruleIds(result), `${JSON.stringify(result)} ${daemon.checks()}`).toContain("rcolder-needle");
      expect(compiles.onThread).toBeGreaterThan(0);
    } finally {
      compiles.withoutAsync = false;
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

/**
 * A session folder with a started repository kept in a folder it ignores (`nested`), and a sibling repository beside it,
 * each running the packs named, every pack holding one needle rule `<pack>-needle`.
 */
async function nestedAndSibling(packs: { session: string; nested: readonly string[]; sibling: readonly string[] }) {
  const root = await realpath(await temporaryDirectory({ started: false }));
  const databasePath = join(root, "rerail.sqlite");
  const folder = await repository(join(root, "session"));
  await writeFile(join(folder, ".gitignore"), ".scratch/\n");
  const nested = await repository(join(folder, ".scratch", "nested"));
  const sibling = await repository(join(root, "sibling"));
  for (const id of [packs.session, ...packs.nested, ...packs.sibling]) installNeedlePack(join(root, "packs"), id, `No NEEDLE (${id})`);
  setUpTestRepository(folder, { packs: [packs.session] }, databasePath);
  setUpTestRepository(nested, { packs: [...packs.nested] }, databasePath);
  setUpTestRepository(sibling, { packs: [...packs.sibling] }, databasePath);
  const open = () => new RerailRuntime({ storageRoot: join(root, "sessions"), databasePath, workspaceRoot: folder, embedder: testEmbedder, autostart: false });
  /** `pack`'s rule changed, so no compiled copy of it is left in this process. */
  const change = (pack: string) => installNeedlePack(join(root, "packs"), pack, `No NEEDLE (${pack}), still`, "1.0.1");
  return { folder, nested, sibling, open, change };
}

let writes = 0;

/** A Write of `content` to `path` in `session`: its call and its result. */
function write(runtime: RerailRuntime, session: { host: "claude-code"; sessionId: string; cwd: string }, path: string, content: string) {
  writes += 1;
  const toolCallId = `w${writes}`;
  const common = { sessionId: session.sessionId, session, toolCallId, toolName: "Write" } as const;
  return {
    call: () => runtime.ingest({ kind: "call", eventId: `${toolCallId}-call`, ...common, input: { file_path: path, content } }),
    result: async () => {
      await writeFile(path, content);
      return await runtime.ingest({ kind: "result", eventId: `${toolCallId}-result`, ...common, output: "ok" });
    },
  };
}

describe("a tool call while rules are built ahead", () => {
  it("is answered while one rule built ahead compiles, when the rule it needs waits on that line behind it", async () => {
    const setup = await nestedAndSibling({ session: "rcjsession", nested: ["rcjheld"], sibling: ["rcjbehind"] });
    const holds = { host: "claude-code" as const, sessionId: "holds", cwd: setup.folder };
    const edits = { host: "claude-code" as const, sessionId: "edits", cwd: setup.folder };
    // In an earlier daemon the second session edited the sibling: its record names the sibling first to build ahead.
    const before = setup.open();
    await before.ingest({ kind: "session-start", eventId: "before-start", session: edits });
    await before.codeRulesOpened(edits.sessionId);
    const earlier = write(before, edits, join(setup.sibling, "api", "x.py"), "def x():\n    return 2\n");
    await earlier.call();
    await earlier.result();
    await before.dispose();
    setup.change("rcjheld");
    setup.change("rcjbehind");
    const runtime = setup.open();
    const released = Promise.withResolvers<void>();
    compiles.hold = { id: "rcjheld-needle", released: released.promise };
    compiles.offThreadIds.length = 0;
    try {
      // The first session builds the nested repository's rules ahead: its compile is held.
      await runtime.ingest({ kind: "session-start", eventId: "holds-start", session: holds });
      await runtime.codeRulesOpened(holds.sessionId);
      await vi.waitFor(() => expect(compiles.holding).toBe(1), { timeout: 10_000, interval: 10 });
      // The second session builds the sibling's rules ahead: its rule waits on the same line, behind the held one.
      await runtime.ingest({ kind: "session-start", eventId: "edits-start", session: edits });
      await runtime.codeRulesOpened(edits.sessionId);
      expect(compiles.offThreadIds).not.toContain("rcjbehind-needle");

      const edit = write(runtime, edits, join(setup.sibling, "api", "x.py"), "NEEDLE = 1\n");
      let answered = false;
      const call = edit.call().finally(() => { answered = true; });
      await vi.waitFor(() => expect(answered).toBe(true), { timeout: 10_000, interval: 10 });
      await call;
      expect(compiles.holding).toBe(1);
      expect(ruleIds(await edit.result())).toContain("rcjbehind-needle");
    } finally {
      released.resolve();
      compiles.hold = undefined;
      await runtime.dispose();
    }
  }, 60_000);
});

describe("a session that ends while it builds rules ahead", () => {
  it("compiles none of the rules still waiting to be built for it alone", async () => {
    const setup = await nestedAndSibling({ session: "rcwsession", nested: ["rcwa", "rcwb"], sibling: [] });
    const runtime = setup.open();
    const session = { host: "claude-code" as const, sessionId: "ends", cwd: setup.folder };
    const released = Promise.withResolvers<void>();
    compiles.hold = { id: "rcwa-needle", released: released.promise };
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await runtime.codeRulesOpened(session.sessionId);
      // The nested repository's two rules are on the line of rules built ahead: the first compiles, held; the second waits.
      await vi.waitFor(() => expect(compiles.holding).toBe(1), { timeout: 10_000, interval: 10 });
      expect(compiles.offThreadIds).not.toContain("rcwb-needle");

      await runtime.ingest({ kind: "session-end", eventId: "end", sessionId: session.sessionId });
      await runtime.closeSession(session.sessionId);
      released.resolve();
      await vi.waitFor(() => expect(compiles.offThreadDone).toContain("rcwa-needle"), { timeout: 10_000, interval: 10 });
      await queuedWorkRan();
      expect(compiles.offThreadIds).not.toContain("rcwb-needle");
    } finally {
      released.resolve();
      compiles.hold = undefined;
      await runtime.dispose();
    }
  }, 60_000);
});
