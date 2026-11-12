import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, realpathSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer as createHttpServer } from "node:http";
import { createServer as createSocketServer, type AddressInfo, type Server } from "node:net";
import { homedir, hostname, tmpdir, userInfo } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { checkouts, defaultRerailDatabasePath, events, openRerailDatabase, PACKAGE_ROOT, policySessionOwners, sessions, SqliteAuditStore } from "../../src/engine/index.js";
import { collectSupportBundle, runSupportBundleCommand, scrubText, type SupportBundle } from "../../src/cli/support-bundle.js";
import { startTestRepository } from "../support/started-repository.js";

const directories: string[] = [];
const daemons: Array<() => Promise<void>> = [];
const fileRerailHome = process.env.RERAIL_HOME;

afterEach(async () => {
  if (fileRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = fileRerailHome;
  await Promise.all(daemons.splice(0).map(close => close()));
  await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true })));
});

const SESSION = "session-under-report";
const PROMPT = "PROMPT-SENTINEL fix the billing page";
const REPLY = "REPLY-SENTINEL done";
const TOOL_INPUT = "TOOL-INPUT-SENTINEL cat .env";
const TOOL_OUTPUT = "TOOL-OUTPUT-SENTINEL DATABASE_URL=postgres://db";
const CAPTURED = "CAPTURED-SENTINEL const password";
const FIX = "FIX-SENTINEL replacement source";
const RULE_MESSAGE = "RULE-MESSAGE-SENTINEL quoting matched code";
const SAMPLE = "SAMPLE-SENTINEL sampled content";
const PREIMAGE = "PREIMAGE-SENTINEL file as it was";
const ACCESS_TOKEN = "access-token-value-0123456789";
const NATIVE_TOKEN = "native-token-value-0123456789";
const OWNER_TOKEN = "owner-token-value-0123456789";
const STEER = "Rerail: eslint-eqeqeq at src/app.ts:3. Use === instead of ==.";

interface Fixture {
  root: string;
  repo: string;
  home: string;
  repositoryKey: string;
  output: string;
}

/** A started git repository with a Rerail home of its own; nothing recorded in it yet. */
async function fixture(): Promise<Fixture> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-support-bundle-")));
  directories.push(root);
  execFileSync("git", ["init", "-q", "-b", "main", "repo"], { cwd: root });
  const home = join(root, "home");
  mkdirSync(home);
  process.env.RERAIL_HOME = home;
  const repo = join(root, "repo");
  return { root, repo, home, repositoryKey: startTestRepository(repo), output: join(root, "bundle.json") };
}

function recordSession(world: Fixture, sessionId: string, startedAt: number): void {
  const database = openRerailDatabase(defaultRerailDatabasePath());
  try {
    database.insert(sessions).values({ sessionId, host: "claude-code", cwd: world.repo, repositoryId: world.repositoryKey, startedAt }).run();
    const event = (eventId: string, kind: string, payload: unknown): void => {
      database.insert(events).values({ sessionId, eventId, kind, observedAt: startedAt + 1, payloadJson: JSON.stringify(payload) }).run();
    };
    event("e1", "session-start", { kind: "session-start", session: { sessionId, cwd: world.repo } });
    event("e2", "user-prompt", { kind: "user-prompt", text: PROMPT });
    event("e3", "call", { kind: "call", toolName: "Bash", input: { command: TOOL_INPUT } });
    event("e4", "result", { kind: "result", toolName: "Bash", output: TOOL_OUTPUT, failed: true });
    event("e5", "assistant-message", { kind: "assistant-message", text: REPLY });
    event("e6", "turn-end", { kind: "turn-end", text: REPLY });
    event("e7", "steering-delivered", { ruleIds: ["eslint-eqeqeq"] });
    event("e8", "events-dropped", { count: 4 });
    event("e9", "code-rules-pending", { boundary: "turn-end" });
    event("e10", "a-kind-from-the-future", { secret: PROMPT });
    database.insert(policySessionOwners).values({ sessionId, delivery: "background", active: true, processId: 1, token: OWNER_TOKEN, updatedAt: startedAt }).run();
  } finally {
    database.$client.close();
  }
}

function recordAudit(world: Fixture, sessionId: string): void {
  const store = new SqliteAuditStore(defaultRerailDatabasePath());
  try {
    const auditId = store.start({ sessionId, workspaceRoot: world.repo, scope: "ai-output", mode: "steer", revision: 1, catalogRevision: "digest", ruleRevisions: [{ ruleId: "eslint-eqeqeq", revision: 1 }], paths: ["src/app.ts", "config/secrets.yaml"] });
    const span = { startByte: 10, endByte: 20, startLine: 3, startColumn: 5, endLine: 3, endColumn: 15 };
    const violation = { ruleId: "eslint-eqeqeq", clauseId: "eqeqeq", code: "eqeqeq", mode: "steer" as const, message: RULE_MESSAGE, span, captures: {}, variables: { MATCH: CAPTURED }, fix: { span, text: FIX } };
    store.finish(auditId, {
      status: "completed",
      violations: [
        { ...violation, id: "v1", path: "src/app.ts" },
        { ...violation, id: "v2", path: "config/secrets.yaml" },
      ],
      failures: [{ ruleId: "eslint-eqeqeq", clauseId: "eqeqeq", code: "adapter-error", stage: "parse", message: `parser died reading ${world.repo}/src/app.ts`, path: "src/app.ts" }],
    });
  } finally {
    store.close();
  }
}

function writeLedger(world: Fixture, sessionId: string): void {
  const directory = join(world.repo, ".rerail", "sessions", createHash("sha256").update(sessionId).digest("hex"));
  mkdirSync(directory, { recursive: true });
  writeFileSync(join(directory, "events.jsonl"), [
    { type: "hud-tracking-start" },
    { type: "sample", item: { text: SAMPLE } },
    { type: "policy-call", toolCallId: "c1", toolName: "Edit", paths: ["src/app.ts"], preimages: { "src/app.ts": { text: PREIMAGE } } },
    { type: "delivery", delivery: { sessionId, requested: "steer", mechanism: "message", content: STEER, ruleIds: ["eslint-eqeqeq"], rules: [] } },
  ].map(entry => JSON.stringify(entry)).join("\n") + "\n");
}

/** Runs the command with its output kept, and reads back the file it wrote. */
async function run(world: Fixture, ...args: string[]): Promise<{ exit: number; stdout: string; stderr: string; text: string; bundle: SupportBundle }> {
  return await runFrom(world, undefined, ...args);
}

/** `run`, as the package at `packageRoot` runs it; undefined runs it as this checkout's build. */
async function runFrom(world: Fixture, packageRoot: string | undefined, ...args: string[]): Promise<{ exit: number; stdout: string; stderr: string; text: string; bundle: SupportBundle }> {
  let stdout = "";
  let stderr = "";
  const exit = await runSupportBundleCommand(
    ["--repo", world.repo, "--output", world.output, ...args],
    () => "usage",
    {
      packageVersion: "9.9.9",
      ...(packageRoot === undefined ? {} : { packageRoot }),
      stdout: { write: text => { stdout += text; return true; } },
      stderr: { write: text => { stderr += text; return true; } },
    },
  );
  if (exit !== 0) return { exit, stdout, stderr, text: "", bundle: {} as SupportBundle };
  const text = readFileSync(world.output, "utf8");
  return { exit, stdout, stderr, text, bundle: JSON.parse(text) as SupportBundle };
}

/** A Rerail package folder at `path`, with nothing in it: the bundle only asks where it is. */
function packageAt(path: string): string {
  mkdirSync(path, { recursive: true });
  writeFileSync(join(path, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "9.9.9" }));
  return path;
}

/** `world`'s Rerail home pointing `current` at `target`, as set-up activates a version or a checkout. */
function activate(world: Fixture, target: string): string {
  const current = join(world.home, "current");
  // A junction needs no privilege on Windows; elsewhere the type is ignored.
  symlinkSync(target, current, "junction");
  return current;
}

/**
 * `count` more checkouts recorded for `world`'s repository, and a session with a thousand events and fifty failed
 * audits of twenty failures each: about what a few weeks of worktrees and one busy day leave in the database.
 */
function busyHistory(world: Fixture, count: number): void {
  const database = openRerailDatabase(defaultRerailDatabasePath());
  try {
    database.$client.transaction(() => {
      for (let index = 0; index < count; index += 1) {
        const checkoutPath = join(world.root, "worktrees", `feature-${index}`);
        database.insert(checkouts).values({ checkoutPath, repositoryKey: world.repositoryKey, cloneId: `clone-${index}`, started: null, createdAt: 1, lastSeenAt: 1 }).run();
      }
      database.insert(sessions).values({ sessionId: SESSION, host: "claude-code", cwd: world.repo, repositoryId: world.repositoryKey, startedAt: 1 }).run();
      for (let index = 0; index < 1_000; index += 1) {
        const kind = ["call", "result", "turn-end", "code-rules-pending"][index % 4]!;
        database.insert(events).values({ sessionId: SESSION, eventId: `e${index}`, kind, observedAt: index, payloadJson: JSON.stringify({ kind, toolName: "Edit", boundary: "turn-end" }) }).run();
      }
    })();
  } finally {
    database.$client.close();
  }
  const store = new SqliteAuditStore(defaultRerailDatabasePath());
  try {
    for (let index = 0; index < 50; index += 1) {
      const auditId = store.start({ sessionId: SESSION, workspaceRoot: world.repo, scope: "ai-output", mode: "steer", revision: 1, catalogRevision: "digest", ruleRevisions: [{ ruleId: "eslint-eqeqeq", revision: 1 }], paths: ["src/app.ts"] });
      store.finish(auditId, {
        status: "failed",
        violations: [],
        failures: Array.from({ length: 20 }, (_, failure) => ({
          ruleId: `rule-${failure}`, clauseId: "clause", code: "adapter-error", stage: "evaluate", path: `src/file-${index}-${failure}.ts`,
          message: `evaluate failed in ${world.repo} for src/file-${index}-${failure}.ts`,
        })),
      });
    }
  } finally {
    store.close();
  }
}

/**
 * A daemon recorded in `home` that answers its health probe `delayMs` late, or never when `delayMs` is undefined.
 * `onProbe` runs as each probe arrives.
 */
async function slowDaemon(home: string, delayMs: number | undefined, onProbe?: () => void): Promise<{ close: () => Promise<void> }> {
  const health = {
    status: "ok", pid: process.pid, version: "9.9.9", ui: "bundled", uptimeMs: 60_000, database: { path: join(home, "rerail.sqlite"), exists: true },
    native: { ok: true, kernelVersion: "1.0.0" }, model: { ok: true }, healthy: true, workspaceRoot: home, installedAt: null,
  };
  const http = createHttpServer((_request, response) => {
    onProbe?.();
    if (delayMs === undefined) return;
    setTimeout(() => response.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(health)), delayMs);
  });
  await new Promise<void>(resolve => http.listen(0, "127.0.0.1", () => resolve()));
  const { port } = http.address() as AddressInfo;
  // A live daemon holds its engine socket; Rerail may check that it is there before it probes the daemon. One per
  // port, so a second daemon can replace the first in one home; as short as `engine.sock`, for macOS's 103 bytes.
  const kernelSocketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-bundle-test-${process.pid}-${port}` : join(home, `e${port}.sock`);
  const engine = createSocketServer(socket => socket.destroy());
  await new Promise<void>(resolve => engine.listen(kernelSocketPath, () => resolve()));
  writeFileSync(join(home, "daemon.json"), JSON.stringify({
    pid: process.pid, host: "127.0.0.1", port, url: `http://127.0.0.1:${port}/`, version: "9.9.9", databasePath: join(home, "rerail.sqlite"),
    kernelSocketPath, ui: "bundled", startedAt: 1, packageRoot: home, accessToken: "fake-daemon-capability-0123456789",
  }));
  const closed = async (server: Server): Promise<void> => await new Promise(resolve => server.close(() => resolve()));
  return {
    close: async () => {
      http.closeAllConnections();
      await Promise.all([closed(http), closed(engine)]);
    },
  };
}

async function reportedWorld(): Promise<Fixture> {
  const world = await fixture();
  recordSession(world, SESSION, 1_000);
  recordAudit(world, SESSION);
  writeLedger(world, SESSION);
  return world;
}

describe("rerail support-bundle", () => {
  it("leaves out everything the person or the agent wrote, read or ran", async () => {
    const { text, exit } = await run(await reportedWorld());
    expect(exit).toBe(0);
    for (const sentinel of [PROMPT, REPLY, TOOL_INPUT, TOOL_OUTPUT, CAPTURED, FIX, RULE_MESSAGE, SAMPLE, PREIMAGE]) {
      expect(text).not.toContain(sentinel);
    }
  });

  it("keeps what a maintainer diagnoses from: the timeline, the gaps, the steers and the violations' places", async () => {
    const { bundle } = await run(await reportedWorld());
    expect(bundle.rerail.version).toBe("9.9.9");
    expect(bundle.session?.host).toBe("claude-code");
    expect(bundle.session?.eventCounts).toMatchObject({ call: 1, result: 1, "steering-delivered": 1 });
    expect(bundle.session?.gaps).toEqual({ droppedEvents: 4, uncheckedBoundaries: 0, codeRulesPendingBoundaries: 1, ignoredEdits: 0, ignoreRuleChanges: 0 });
    expect(bundle.session?.timeline).toContainEqual(expect.objectContaining({ kind: "result", tool: "Bash", failed: true }));
    expect(bundle.session?.timeline).toContainEqual(expect.objectContaining({ kind: "steering-delivered", ruleIds: ["eslint-eqeqeq"] }));
    expect(bundle.session?.steers).toEqual([{ mechanism: "message", requested: "steer", ruleIds: ["eslint-eqeqeq"], text: STEER }]);
    const audit = bundle.session?.audits[0];
    expect(audit).toMatchObject({ scope: "ai-output", mode: "steer", status: "completed", pathCount: 2, ruleCount: 1, failuresOmitted: 0 });
    expect(audit?.violations[0]).toEqual({ ruleId: "eslint-eqeqeq", clauseId: "eqeqeq", code: "eqeqeq", mode: "steer", path: "src/app.ts", line: 3, column: 5 });
    expect(audit?.failures[0]).toMatchObject({ code: "adapter-error", stage: "parse", path: "src/app.ts" });
  });

  it("records a kind it does not know by its name alone", async () => {
    const { bundle } = await run(await reportedWorld());
    expect(bundle.session?.timeline.find(entry => entry.kind === "a-kind-from-the-future")).toEqual({ at: expect.any(Number), kind: "a-kind-from-the-future" });
  });

  it("withholds a path named like a secret, and keeps the finding by rule and line", async () => {
    const { bundle, text } = await run(await reportedWorld());
    expect(text).not.toContain("secrets.yaml");
    expect(bundle.session?.audits[0]?.violations[1]).toMatchObject({ ruleId: "eslint-eqeqeq", path: "<withheld: named like a secret>", line: 3 });
  });

  it("takes the capability tokens, the checkout path and the repository's name out of the daemon log", async () => {
    const world = await reportedWorld();
    writeFileSync(join(world.home, "daemon.json"), JSON.stringify({ pid: 999_999_999, url: "http://127.0.0.1:1", accessToken: ACCESS_TOKEN, nativeToken: NATIVE_TOKEN }));
    writeFileSync(join(world.home, "daemon.log"), [
      `Rerail dashboard http://127.0.0.1:4737/?rerail-access=${ACCESS_TOKEN}`,
      `native session ${SESSION} token ${NATIVE_TOKEN} owner ${OWNER_TOKEN}`,
      `error reading ${world.repo}/src/app.ts in ${world.repositoryKey}`,
      `home is ${homedir()}/work, Rerail home is ${world.home}`,
    ].join("\n"));
    const { bundle, text } = await run(world);
    for (const secret of [ACCESS_TOKEN, NATIVE_TOKEN, OWNER_TOKEN, SESSION, world.repo, world.repositoryKey, world.home, homedir()]) {
      expect(text).not.toContain(secret);
    }
    expect(bundle.daemonLog).toContain("error reading <checkout>/src/app.ts in <repository>");
  });

  it("takes the computer's name out of the log by its first label too, as macOS logs it without .local", async () => {
    const world = await reportedWorld();
    const label = hostname().split(".")[0]!;
    if (label.length < 4) return;
    writeFileSync(join(world.home, "daemon.log"), `listening on ${label} and on ${hostname()}\n`);
    const { bundle } = await run(world);
    expect(bundle.daemonLog[0]).toBe("listening on <host> and on <host>");
  });

  it("scrubs the session's own text the same way as the log", async () => {
    const world = await reportedWorld();
    const { bundle, text } = await run(world);
    expect(text).not.toContain(world.repo);
    expect(bundle.session?.audits[0]?.failures[0]?.message).toBe("parser died reading <checkout>/src/app.ts");
  });

  it("keeps the end of a long log, and says how much it left out", async () => {
    const world = await fixture();
    writeFileSync(join(world.home, "daemon.log"), Array.from({ length: 1_000 }, (_, line) => `line ${line}`).join("\n"));
    const { bundle } = await run(world);
    expect(bundle.daemonLog).toHaveLength(200);
    expect(bundle.daemonLog.at(-1)).toBe("line 999");
    expect(bundle.daemonLogOmittedLines).toBe(800);
  });

  it("reports the latest session of this repository, or the one --session names", async () => {
    const world = await fixture();
    recordSession(world, "earlier", 1_000);
    recordSession(world, "later", 2_000);
    expect((await run(world)).bundle.session?.startedAt).toBe(2_000);
    expect((await run(world, "--session", "earlier")).bundle.session?.startedAt).toBe(1_000);
  });

  it("never reports another repository's session", async () => {
    const world = await reportedWorld();
    const other = await fixture();
    // Each fixture has its own Rerail home, so the other repository is recorded in the same database by hand.
    process.env.RERAIL_HOME = world.home;
    const database = openRerailDatabase(defaultRerailDatabasePath());
    try {
      database.insert(sessions).values({ sessionId: "elsewhere", host: "codex", cwd: other.repo, repositoryId: "another-repository", startedAt: 9_000 }).run();
    } finally {
      database.$client.close();
    }
    expect((await run(world)).bundle.session?.host).toBe("claude-code");
    const refused = await run(world, "--session", "elsewhere");
    expect(refused.exit).toBe(1);
    expect(refused.stderr).toContain("No session elsewhere in this repository");
  });

  it("refuses a session it cannot find, and writes nothing", async () => {
    const world = await fixture();
    const { exit, stderr } = await run(world, "--session", "nope");
    expect(exit).toBe(1);
    expect(stderr).toContain("No session nope in this repository");
    expect(() => statSync(world.output)).toThrow();
  });

  it("still writes a bundle where Rerail was never set up: no database, no daemon, no repository", async () => {
    const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-support-bundle-")));
    directories.push(root);
    process.env.RERAIL_HOME = join(root, "home");
    const world = { root, repo: root, home: process.env.RERAIL_HOME, repositoryKey: "", output: join(root, "bundle.json") };
    const { exit, bundle } = await run(world);
    expect(exit).toBe(0);
    expect(bundle.rerail.version).toBe("9.9.9");
    expect(bundle.daemon.status).toBe("absent");
    expect(bundle.session).toBeNull();
    expect(bundle.platform.os).toBe(process.platform);
  });

  it("takes about as long where the database records a thousand checkouts as where it records none", { timeout: 180_000 }, async () => {
    const elapsed = async (checkoutCount: number): Promise<number> => {
      const world = await fixture();
      busyHistory(world, checkoutCount);
      const started = performance.now();
      const { exit, bundle } = await run(world);
      expect(exit).toBe(0);
      expect(bundle.session?.audits).toHaveLength(50);
      return performance.now() - started;
    };
    const none = await elapsed(0);
    const thousand = await elapsed(1_000);
    expect(thousand, `${none.toFixed(0)} ms with none, ${thousand.toFixed(0)} ms with a thousand`).toBeLessThan(3 * none + 2_000);
  });

  it("says Rerail is not the managed install when it runs from a package outside the home's versions, and why it has no content store", async () => {
    const world = await fixture();
    const elsewhere = activate(world, packageAt(join(world.root, "install", "node_modules", "@neralabs", "rerail")));
    const { bundle } = await runFrom(world, elsewhere);
    expect(bundle.rerail.managedInstall).toBe(false);
    expect(bundle.problems).toEqual([expect.stringMatching(/^install: Rerail at .+ is neither the managed installation .+ so it has no content store$/u)]);
  });

  it("says Rerail is the managed install when it runs from a version under the home", async () => {
    const world = await fixture();
    const current = activate(world, packageAt(join(world.home, "versions", "9.9.9", "node_modules", "@neralabs", "rerail")));
    const { bundle } = await runFrom(world, current);
    expect(bundle.rerail.managedInstall).toBe(true);
    expect(bundle.problems).toEqual([]);
  });

  it("waits out a busy daemon that misses the first health probe, and says how long it took to answer", { timeout: 30_000 }, async () => {
    const world = await fixture();
    const daemon = await slowDaemon(world.home, 2_500);
    daemons.push(daemon.close);
    const { bundle } = await run(world);
    expect(bundle.daemon).toMatchObject({ status: "running", version: "9.9.9", engine: { ok: true, version: "1.0.0" } });
    expect(bundle.daemon.answeredInMs).toBeGreaterThanOrEqual(2_500);
  });

  it("still calls a daemon that never answers unresponsive once the wait is over", { timeout: 30_000 }, async () => {
    const world = await fixture();
    const daemon = await slowDaemon(world.home, undefined);
    daemons.push(daemon.close);
    const { bundle } = await collectSupportBundle({
      repo: world.repo, home: world.home, databasePath: defaultRerailDatabasePath(), sessionId: undefined, packageVersion: "9.9.9",
      packageRoot: PACKAGE_ROOT, busyDaemonWaitMs: 500,
    });
    expect(bundle.daemon).toEqual({ status: "unresponsive" });
  });

  it("reports the daemon that took a busy one's place while the bundle waited, as a restart does", { timeout: 30_000 }, async () => {
    const world = await fixture();
    let replaced: Promise<void> | undefined;
    // The first probe reaches a daemon that never answers; while the bundle waits, another records itself in daemon.json.
    const stuck = await slowDaemon(world.home, undefined, () => {
      replaced ??= slowDaemon(world.home, 0).then(daemon => {
        daemons.push(daemon.close);
      });
    });
    daemons.push(stuck.close);
    const { bundle } = await collectSupportBundle({
      repo: world.repo, home: world.home, databasePath: defaultRerailDatabasePath(), sessionId: undefined, packageVersion: "9.9.9",
      packageRoot: PACKAGE_ROOT, busyDaemonWaitMs: 3_000,
    });
    await replaced;
    expect(bundle.daemon).toMatchObject({ status: "running", version: "9.9.9" });
  });

  describe("where set-up never finished: a database, and this checkout not recorded in it", () => {
    async function unrecordedWorld(): Promise<Fixture> {
      const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-support-bundle-")));
      directories.push(root);
      execFileSync("git", ["init", "-q", "-b", "main", "repo"], { cwd: root });
      const home = join(root, "home");
      mkdirSync(home);
      process.env.RERAIL_HOME = home;
      openRerailDatabase(defaultRerailDatabasePath()).$client.close();
      return { root, repo: join(root, "repo"), home, repositoryKey: "", output: join(root, "bundle.json") };
    }

    it("writes the bundle with no session, and calls that no problem", async () => {
      const { exit, bundle } = await run(await unrecordedWorld());
      expect(exit).toBe(0);
      expect(bundle.session).toBeNull();
      expect(bundle.problems).toEqual([]);
    });

    it("refuses a session it was asked for", async () => {
      const { exit, stderr } = await run(await unrecordedWorld(), "--session", "nope");
      expect(exit).toBe(1);
      expect(stderr).toContain("No session nope in this repository");
    });
  });

  it("names an installed pack that failed to load, with the reason and no path of this computer", async () => {
    const world = await fixture();
    const pack = join(world.home, "packs", "node_modules", "@neralabs", "rerail-pack-broken");
    mkdirSync(pack, { recursive: true });
    writeFileSync(join(pack, "package.json"), JSON.stringify({ name: "@neralabs/rerail-pack-broken", version: "1.2.3" }));
    const { bundle, text } = await run(world);
    expect(bundle.rerail.skippedPacks).toEqual([expect.objectContaining({ package: "@neralabs/rerail-pack-broken", version: "1.2.3", reason: expect.any(String) })]);
    expect(text).not.toContain(world.home);
  });

  it("finds the steers from the checkout's root when the session ran in a folder inside it", async () => {
    const world = await fixture();
    recordSession(world, SESSION, 1_000);
    writeLedger(world, SESSION);
    const database = openRerailDatabase(defaultRerailDatabasePath());
    database.update(sessions).set({ cwd: join(world.repo, "packages", "web") }).run();
    database.$client.close();
    expect((await run(world)).bundle.session?.steers).toHaveLength(1);
  });

  it("says so when the session steered and the steers' ledger is gone", async () => {
    const world = await fixture();
    recordSession(world, SESSION, 1_000);
    const { bundle } = await run(world);
    expect(bundle.session?.steers).toEqual([]);
    expect(bundle.problems).toEqual([expect.stringContaining("steered 1 time")]);
  });

  it("writes the file for the person's user only", async () => {
    const world = await reportedWorld();
    await run(world);
    expect(statSync(world.output).mode & 0o777).toBe(0o600);
  });

  it("--json names the file and what to check in it, and carries none of the bundle's text", async () => {
    const world = await reportedWorld();
    const { stdout } = await run(world, "--json");
    const summary = JSON.parse(stdout) as { path: string; steers: number; violations: number; logLines: number };
    expect(summary).toMatchObject({ path: world.output, steers: 1, violations: 2 });
    expect(stdout).not.toContain(STEER);
  });

  it("refuses an unknown option", async () => {
    await expect(runSupportBundleCommand(["--nope"], () => "usage", { packageVersion: "9.9.9" })).rejects.toThrow("Unknown support-bundle option: --nope");
  });
});

describe("scrubText", () => {
  const scrub = (text: string): string => scrubText(text, { knownValues: [] });

  it.each([
    ["a GitHub token", "ghp_abcdefghijklmnopqrstuvwxyz0123456789"],
    ["a fine-grained GitHub token", "github_pat_11ABCDEFG0abcdefghijkl_mnopqrstuvwxyz"],
    ["an API key", "sk-ant-api03-abcdefghijklmnopqrstuvwxyz"],
    ["an AWS access key", "AKIAIOSFODNN7EXAMPLE"],
    ["a Slack token", "xoxb-123456789012-abcdefghijkl"],
    ["an npm token", "npm_abcdefghijklmnopqrstuvwxyz0123456789"],
    ["a JWT", "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.abcDEF123-_x"],
    ["an email address", "someone@example.com"],
  ])("takes out %s", (_name, secret) => {
    expect(scrub(`before ${secret} after`)).toBe("before <redacted> after");
  });

  it.each([
    ["a bearer header", "Authorization: Bearer abc.def-123", "abc.def-123"],
    ["a password in a URL", "postgres://admin:hunter2@db.internal/app", "hunter2"],
    ["an assignment", "API_KEY=abc123", "abc123"],
    ["a quoted assignment", 'password: "hunter2"', "hunter2"],
    ["an npm auth line", "//registry.npmjs.org/:_authToken=abc123", "abc123"],
    ["the dashboard's capability", "http://127.0.0.1:4737/?rerail-access=abc123", "abc123"],
  ])("takes the value out of %s", (_name, line, value) => {
    expect(scrub(line)).not.toContain(value);
  });

  it.each([
    // Built at run time: GitHub's push protection reads a key-shaped literal as a key.
    ["a Stripe key", `sk_live_${"51H".padEnd(24, "x")}`],
    ["a Stripe test key", `sk_test_${"51H".padEnd(24, "x")}`],
    ["a Google API key", "AIzaSyA-abcdefghijklmnopqrstuvwxyz01234"],
  ])("takes out %s", (_name, secret) => {
    expect(scrub(`before ${secret} after`)).toBe("before <redacted> after");
  });

  it.each([
    ["a basic auth header", "Authorization: Basic dXNlcjpwYXNz", "dXNlcjpwYXNz"],
    ["a digest auth header", "authorization: Digest abc.def", "abc.def"],
    ["a token auth header", "Authorization: Token abc123def", "abc123def"],
    ["a URL password with no user", "postgres://:hunter2@db:5432/app", "hunter2"],
  ])("takes the credential out of %s", (_name, line, value) => {
    expect(scrub(line)).not.toContain(value);
  });

  it("takes out a private key across lines", () => {
    const key = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\ndef\n-----END OPENSSH PRIVATE KEY-----";
    expect(scrub(`before ${key} after`)).toBe("before <redacted> after");
  });

  it.each([
    ["macOS", "/Users/somebody/work/app.ts", "~/work/app.ts"],
    ["Linux", "/home/somebody/work/app.ts", "~/work/app.ts"],
    ["Windows", "C:\\Users\\somebody\\work\\app.ts", "~\\work\\app.ts"],
  ])("shortens a %s home directory, whoever's it is", (_name, path, expected) => {
    expect(scrub(path)).toBe(expected);
  });

  it.each([
    ["forward slashes", "C:/Users/somebody/work/app.ts", "~/work/app.ts"],
    ["JSON-escaped backslashes", String.raw`{"cwd":"C:\\Users\\somebody\\work\\app.ts"}`, String.raw`{"cwd":"~\\work\\app.ts"}`],
    ["WSL's mount of a Windows home", "/mnt/c/Users/somebody/work/app.ts", "~/work/app.ts"],
  ])("shortens a Windows home directory written with %s", (_name, path, expected) => {
    expect(scrub(path)).toBe(expected);
  });

  it.each([
    ["as recorded", String.raw`C:\Users\somebody\src\acme-app\x.ts`],
    ["with forward slashes", "C:/Users/somebody/src/acme-app/x.ts"],
    ["JSON-escaped", String.raw`C:\\Users\\somebody\\src\\acme-app\\x.ts`],
  ])("replaces a Windows checkout path written %s", (_name, text) => {
    const knownValues = [{ value: String.raw`C:\Users\somebody\src\acme-app`, label: "<checkout>" }];
    expect(scrubText(text, { knownValues })).not.toContain("acme-app");
    expect(scrubText(text, { knownValues })).toContain("<checkout>");
  });

  it("replaces a POSIX checkout path written JSON-escaped, as a logged JSON string holds it", () => {
    const knownValues = [{ value: "/home/somebody/src/acme-app", label: "<checkout>" }];
    expect(scrubText(String.raw`{"cwd":"\/home\/somebody\/src\/acme-app"}`, { knownValues })).toBe(String.raw`{"cwd":"<checkout>"}`);
  });

  it("takes out this computer's user name wherever it appears", () => {
    const { username } = userInfo();
    expect(scrubText(`owned by ${username}.`, { knownValues: [{ value: username, label: "<user>" }] })).toBe("owned by <user>.");
  });

  it("takes a short user name out of a path only, where it would otherwise eat a common word", () => {
    const knownValues = [{ value: "dev", label: "<user>", word: true, pathOnly: true }];
    expect(scrubText("/Volumes/data/dev/app.ts on the dev build", { knownValues })).toBe("/Volumes/data/<user>/app.ts on the dev build");
    expect(scrubText(String.raw`D:\dev\app.ts`, { knownValues })).toBe(String.raw`D:\<user>\app.ts`);
    expect(scrubText("/srv/devices/x", { knownValues })).toBe("/srv/devices/x");
  });

  it("replaces a longer literal before a shorter one inside it", () => {
    const knownValues = [{ value: "/work", label: "<short>" }, { value: "/work/app", label: "<checkout>" }];
    expect(scrubText("/work/app/src", { knownValues })).toBe("<checkout>/src");
  });

  it("ignores a home or checkout of /, as a container can have, which would eat every path", () => {
    expect(scrubText("src/app.ts", { knownValues: [{ value: "/", label: "~" }] })).toBe("src/app.ts");
  });

  it("leaves rule ids, versions, digests and ordinary words alone", () => {
    const line = "no-hardcoded-password fired in pack javascript@0.1.4 at 3f2a9c1d, a token was expected";
    expect(scrub(line)).toBe(line);
  });
});
