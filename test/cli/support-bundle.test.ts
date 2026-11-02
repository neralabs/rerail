import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { homedir, tmpdir, userInfo } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { defaultRerailDatabasePath, events, openRerailDatabase, policySessionOwners, sessions, SqliteAuditStore } from "../../src/engine/index.js";
import { runSupportBundleCommand, scrubText, type SupportBundle } from "../../src/cli/support-bundle.js";
import { startTestRepository } from "../support/started-repository.js";

const directories: string[] = [];
const fileRerailHome = process.env.RERAIL_HOME;

afterEach(async () => {
  if (fileRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = fileRerailHome;
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
  let stdout = "";
  let stderr = "";
  const exit = await runSupportBundleCommand(
    ["--repo", world.repo, "--output", world.output, ...args],
    () => "usage",
    { packageVersion: "9.9.9", stdout: { write: text => { stdout += text; return true; } }, stderr: { write: text => { stderr += text; return true; } } },
  );
  if (exit !== 0) return { exit, stdout, stderr, text: "", bundle: {} as SupportBundle };
  const text = readFileSync(world.output, "utf8");
  return { exit, stdout, stderr, text, bundle: JSON.parse(text) as SupportBundle };
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

  it("keeps what a maintainer diagnoses from: the timeline, the gaps, the steers and the findings' places", async () => {
    const { bundle } = await run(await reportedWorld());
    expect(bundle.rerail.version).toBe("9.9.9");
    expect(bundle.session?.host).toBe("claude-code");
    expect(bundle.session?.eventCounts).toMatchObject({ call: 1, result: 1, "steering-delivered": 1 });
    expect(bundle.session?.gaps).toEqual({ droppedEvents: 4, uncheckedBoundaries: 0, codeRulesPendingBoundaries: 1 });
    expect(bundle.session?.timeline).toContainEqual(expect.objectContaining({ kind: "result", tool: "Bash", failed: true }));
    expect(bundle.session?.timeline).toContainEqual(expect.objectContaining({ kind: "steering-delivered", ruleIds: ["eslint-eqeqeq"] }));
    expect(bundle.session?.steers).toEqual([{ mechanism: "message", requested: "steer", ruleIds: ["eslint-eqeqeq"], text: STEER }]);
    const audit = bundle.session?.audits[0];
    expect(audit).toMatchObject({ scope: "ai-output", mode: "steer", status: "completed", paths: 2, rules: 1 });
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
    const summary = JSON.parse(stdout) as { path: string; steers: number; findings: number; logLines: number };
    expect(summary).toMatchObject({ path: world.output, steers: 1, findings: 2 });
    expect(stdout).not.toContain(STEER);
  });

  it("refuses an unknown option", async () => {
    await expect(runSupportBundleCommand(["--nope"], () => "usage", { packageVersion: "9.9.9" })).rejects.toThrow("Unknown support-bundle option: --nope");
  });
});

describe("scrubText", () => {
  const scrub = (text: string): string => scrubText(text, { literals: [] });

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

  it("takes out this computer's user name wherever it appears", () => {
    const { username } = userInfo();
    expect(scrubText(`owned by ${username}.`, { literals: [{ value: username, label: "<user>" }] })).toBe("owned by <user>.");
  });

  it("replaces a longer literal before a shorter one inside it", () => {
    const literals = [{ value: "/work", label: "<short>" }, { value: "/work/app", label: "<checkout>" }];
    expect(scrubText("/work/app/src", { literals })).toBe("<checkout>/src");
  });

  it("ignores a home or checkout of /, as a container can have, which would eat every path", () => {
    expect(scrubText("src/app.ts", { literals: [{ value: "/", label: "~" }] })).toBe("src/app.ts");
  });

  it("leaves rule ids, versions, digests and ordinary words alone", () => {
    const line = "no-hardcoded-password fired in pack javascript@0.1.4 at 3f2a9c1d, a token was expected";
    expect(scrub(line)).toBe(line);
  });
});
