import { mkdtemp, mkdir, realpath, rm, writeFile } from "node:fs/promises";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { dashboardOrigin, formatTerminalStatus, linkDeliveries, readPresentation, readRepositoryState, readRulesDashboard, readSummaryState } from "../src/session-presentation.js";
import type { RepositoryStartState } from "../src/repository-model.js";
import { steerBlock } from "../src/rerail-steer.js";
import type { presentationResponseSchema, sessionResponseSchema } from "../src/api/session-contracts.js";
import type { z } from "zod";
import { hookRerailStarted } from "../src/repository-start.js";
import { startTestRepository } from "./support/started-repository.js";

let home: string;
let cwd: string;
let database: string;
let state: Record<string, unknown>;
let response: z.infer<typeof presentationResponseSchema>;
let sessionContext: z.infer<typeof sessionResponseSchema>;
const sessionId = "session /?&one";
let requested: string[];

beforeEach(async () => {
  home = await mkdtemp(join(tmpdir(), "rerail-presentation-"));
  cwd = join(home, "workspace");
  database = join(home, "rerail.sqlite");
  await mkdir(join(cwd, ".rerail"), { recursive: true });
  await writeFile(database, "");
  await writeFile(join(cwd, ".rerail", "repository-id"), "repository-under-test\n");
  state = { pid: process.pid, host: "127.0.0.1", port: 4737, url: "http://127.0.0.1:4737/", version: "test",
    databasePath: database, kernelSocketPath: join(home, "engine.sock"), ui: "bundled", startedAt: 1, packageRoot: home };
  await writeFile(join(home, "daemon.json"), JSON.stringify(state));
  requested = [];
  sessionContext = { session: { sessionId, host: "claude-code", cwd, repositoryId: "repository-under-test", startedAt: 1, endedAt: null, eventCount: 0 },
    hud: { activeRuleCount: 3, steeringCount: 2 }, audit: null };
  response = { presence: "compact", repositoryState: "started", context: {
    daemon: { pid: process.pid, database: { path: database, exists: true }, native: { ok: true } },
    session: structuredClone(sessionContext),
  } };
  vi.stubEnv("RERAIL_HOME", home);
  vi.stubGlobal("fetch", vi.fn(async (input: URL | string) => {
    const url = new URL(input);
    requested.push(url.href);
    if (url.pathname === "/api/presentation") return Response.json(response);
    throw new Error(`Unexpected request ${url.href}`);
  }));
});

afterEach(async () => {
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  await rm(home, { recursive: true, force: true });
});

const read = (onDemand = false) => readPresentation({ sessionId: "session /?&one", host: "claude-code", cwd, onDemand });

describe("installed session presentation", () => {
  it("renders exact-session counters and three separate navigation links", async () => {
    const result = await read();
    expect(result).toMatchObject({ status: "on", text: "Rerail ● 3 rules  ·  steered 2×", counts: { rules: 3, steered: 2 } });
    expect(requested).toHaveLength(1);
    const request = new URL(requested[0]!);
    expect(request.pathname).toBe("/api/presentation");
    expect(request.searchParams.get("sessionId")).toBe(sessionId);
    const session = new URL(result.parts[0]!.url!);
    expect(session.searchParams.get("sessionId")).toBe("session /?&one");
    expect(session.searchParams.get("host")).toBe("claude-code");
    expect(session.hash).toBe("#sessions");
    expect(result.parts[2]!.url).toBe(session.href);
    // Without a scope from the daemon the rules link opens the repository picker; with one it opens that repository.
    expect(new URL(result.parts[1]!.url!).hash).toBe("#rules");
    expect(new URL(result.parts[1]!.url!).search).toBe("");
    response.scope = { repositoryKey: "remote:github.com/acme/app", checkoutPath: cwd };
    const rules = new URL((await read()).parts[1]!.url!);
    expect(Object.fromEntries(rules.searchParams)).toEqual({ repository: "remote:github.com/acme/app", checkout: cwd });
    expect(rules.hash).toBe("#rules");
    const rendered = formatTerminalStatus(result);
    expect(rendered.split("\u001b]8;;http")).toHaveLength(4);
  });

  it("shows the steered count only once Rerail has steered, and names Engine and audit trouble", async () => {
    response.context!.session!.hud.steeringCount = 0;
    expect((await read()).text).toBe("Rerail ● 3 rules");
    response.context!.daemon.native.ok = false;
    response.context!.session!.audit = { workspaceRoot: cwd, scope: "touched-files", mode: "steer", status: "failed", violationCount: 0, failureCount: 1 };
    expect((await read()).text).toBe("Rerail ● 3 rules  ·  engine unavailable  ·  audit failed");
  });

  it("honors silent without session/history reads but exposes on-demand counts", async () => {
    response.presence = "silent";
    response.context!.session = null;
    expect((await read()).parts).toEqual([]);
    expect(requested.some(url => url.includes("/api/sessions/"))).toBe(false);
    response.context!.session = structuredClone(sessionContext);
    expect(await read(true)).toMatchObject({ presence: "silent", status: "on", counts: { rules: 3, steered: 2 } });
  });

  it.each(["host", "cwd", "sessionId"])("refuses navigation and counts for mismatched %s", async dimension => {
    const session = response.context!.session!.session;
    if (dimension === "host") session.host = "omp";
    if (dimension === "cwd") session.cwd = join(cwd, "nested");
    if (dimension === "sessionId") session.sessionId = "another-session";
    await mkdir(join(cwd, "nested"));
    const result = await read();
    expect(result).toMatchObject({ status: "unavailable", text: "Rerail ● session unavailable" });
    expect(result.parts.every(part => part.url === null)).toBe(true);
  });

  it("reads the launch root's database and session from a directory below it", async () => {
    const nested = join(cwd, "nested");
    await mkdir(nested);
    const result = await readPresentation({ sessionId, host: "claude-code", cwd: nested });
    expect(result.text).toBe("Rerail ● 3 rules  ·  steered 2×");
    expect(new URL(requested[0]!).searchParams.get("repository")).toBe(await realpath(nested));
  });

  it("does not fabricate links for a missing session or zero counts for unavailable counters", async () => {
    const session = response.context!.session!;
    response.context!.session = null;
    expect((await read()).parts.every(part => part.url === null)).toBe(true);
    response.context!.session = session;
    session.hud = { activeRuleCount: null, steeringCount: null };
    expect((await read()).text).toBe("Rerail ● rules unavailable  ·  steering unavailable");
  });

  it("reports not on here only where Rerail is stopped, and Not set up with the start command where no rules are configured", async () => {
    response.repositoryState = "stopped";
    const stopped = await read();
    expect(stopped).toMatchObject({ status: "not-on-here", text: "Rerail ○ not on here", repositoryState: "stopped" });
    expect(stopped.parts.every(part => part.url === null)).toBe(true);
    response.presence = "silent";
    expect(await read()).toMatchObject({ parts: [], repositoryState: "stopped" });
    response.presence = "compact";
    response.repositoryState = "not-set-up";
    const notSetUp = await read();
    expect(notSetUp).toMatchObject({ status: "not-set-up", text: "Rerail ○ not set up  ·  rerail start", repositoryState: "not-set-up" });
    // Recorded or not, the footer says why nothing runs, linking the scoped rules page.
    response.context!.session = null;
    response.scope = { repositoryKey: "remote:github.com/acme/app", checkoutPath: cwd };
    const unrecorded = await read();
    expect(unrecorded.text).toBe(notSetUp.text);
    expect(new URL(unrecorded.parts[1]!.url!).searchParams.get("repository")).toBe("remote:github.com/acme/app");
  });

  it("with no daemon, follows the caller's start state: not on here, not set up, or not running where started", async () => {
    await rm(join(home, "daemon.json"));
    const withState = (startState: () => RepositoryStartState) => readPresentation({ sessionId, host: "claude-code", cwd, startState });
    expect(await withState(() => "stopped")).toMatchObject({ status: "not-on-here", text: "Rerail ○ not on here", repositoryState: "stopped" });
    expect(await withState(() => "not-set-up")).toMatchObject({ status: "not-set-up", text: "Rerail ○ not set up  ·  rerail start" });
    expect(await withState(() => "started")).toMatchObject({ status: "not-running", text: "Rerail ✕ not running" });
    expect(await withState(() => { throw new Error("database unreadable"); })).toMatchObject({ status: "not-running" });
    expect(await read()).toMatchObject({ status: "not-running" });
    expect(requested).toEqual([]);
  });

  it("summarizes a repository without a session from the daemon first, else the local state", async () => {
    await rm(join(home, "daemon.json"));
    expect(await readSummaryState(cwd, "cursor", () => "stopped")).toBe("stopped");
    expect(await readSummaryState(cwd, "cursor", () => "started")).toBeNull();
  });

  it("rereads the daemon locator after restart", async () => {
    await read();
    state.url = "http://127.0.0.1:5737/";
    state.port = 5737;
    await writeFile(join(home, "daemon.json"), JSON.stringify(state));
    expect((await read()).url).toContain(":5737/");
  });

  it("rejects replaced daemon identity, remote locators, and oversized responses", async () => {
    response.context!.daemon.pid++;
    expect(await read()).toMatchObject({ status: "not-running", text: "Rerail ✕ not running" });
    response.context!.daemon.pid = process.pid;
    response.context!.session!.session.sessionId = "x".repeat(1_048_576);
    expect((await read()).url).toBeNull();
    state.url = "http://example.com/";
    await writeFile(join(home, "daemon.json"), JSON.stringify(state));
    requested = [];
    expect((await read()).url).toBeNull();
    expect(requested).toEqual([]);
  });

  it("strips terminal control injection from labels and link targets", () => {
    const rendered = formatTerminalStatus({ status: "on", text: "", url: null, presence: "compact", repositoryState: "started", counts: null, parts: [
      { text: "Rerail\u001b\n\u202e", url: "http://127.0.0.1/\u001b\u0007\u202e" },
      { text: "2 rules", url: "http://127.0.0.1/#rules" },
    ] });
    expect(rendered).not.toContain("\u202e");
    expect(rendered).not.toContain("\n");
    // A BEL smuggled into the target is stripped, so it cannot close the sequence early.
    expect(rendered).toContain("\u001b]8;;http://127.0.0.1/\u0007\u001b[38;2;77;95;218mRerail\u001b[39m\u001b]8;;\u0007");
    expect(rendered).toContain("\u001b]8;;http://127.0.0.1/#rules\u00072 rules\u001b]8;;\u0007");
    expect(rendered).not.toContain("\u001b\\");
  });
});

describe("dashboard links", () => {
  const steer = { sessionId: "s", requested: "steer" as const, mechanism: "additional-context" as const, content: steerBlock("a:b", "Fix it."), ruleIds: ["a:b"] };

  it("links steers to the Rerail home daemon when it is bound to loopback", async () => {
    expect(await dashboardOrigin(cwd)).toBe("http://127.0.0.1:4737/");
    const [linked] = await linkDeliveries([steer], cwd);
    expect(linked?.content).toContain("Why? → http://127.0.0.1:4737/#rule/a%3Ab");
  });

  it("keeps links relative for a daemon bound to 0.0.0.0, which no browser should be sent to", async () => {
    await writeFile(join(home, "daemon.json"), JSON.stringify({ ...state, host: "0.0.0.0", url: "http://0.0.0.0:4737/" }));
    expect(await dashboardOrigin(cwd)).toBeUndefined();
    const [linked] = await linkDeliveries([steer], cwd);
    expect(linked?.content).toContain("Why? → #rule/a%3Ab");
  });
});

describe("rules page", () => {
  let stub: Server | undefined;
  let health: Record<string, unknown>;
  /** Whether the stub daemon has recorded the checkout, so `GET /api/repository?checkout=` answers. */
  let knownCheckout: boolean;
  const repositoryKey = "remote:github.com/acme/app";

  const stop = async (): Promise<void> => {
    const server = stub;
    stub = undefined;
    if (server !== undefined) await new Promise<void>(resolve => server.close(() => resolve()));
  };

  const repositoryContext = (checkoutPath: string) => ({
    repository: { repositoryKey, name: "app", aliases: [], setUp: true, configuredAt: 1, started: true, state: "started", checkoutCount: 1 },
    clones: [],
    checkout: { checkoutPath, folder: "", state: "started" },
  });

  /** Liveness is only provable over the wire, so these fixtures answer `/healthz` from a real listener. */
  const listen = async (pid: number = process.pid): Promise<number> => {
    vi.unstubAllGlobals();
    knownCheckout = true;
    health = { status: "ok", pid, database: { path: database, exists: true }, native: { ok: true } };
    const checkoutPath = await realpath(cwd);
    stub = createServer((request, response) => {
      const url = new URL(request.url ?? "/", "http://127.0.0.1");
      const answers: Record<string, unknown> = {
        "/healthz": health,
        "/api/presentation": { presence: "compact", repositoryState: "stopped" },
        ...(knownCheckout && url.searchParams.get("checkout") === checkoutPath ? { "/api/repository": repositoryContext(checkoutPath) } : {}),
      };
      const body = answers[url.pathname];
      response.writeHead(body === undefined ? 404 : 200, { "content-type": "application/json" });
      response.end(JSON.stringify(body ?? { error: "checkout-not-found" }));
    });
    await new Promise<void>(resolve => stub!.listen(0, "127.0.0.1", () => resolve()));
    return (stub!.address() as AddressInfo).port;
  };

  const record = async (file: string, fields: Record<string, unknown>): Promise<void> => {
    Object.assign(state, fields);
    await writeFile(file, JSON.stringify(state));
  };

  const scoped = async (origin: string): Promise<string> =>
    `${origin}/?${new URLSearchParams({ repository: repositoryKey, checkout: await realpath(cwd) })}#rules`;

  afterEach(stop);

  it("links the rules view scoped to the checkout's repository, from the daemon that answers as itself", async () => {
    const port = await listen();
    await record(join(home, "daemon.json"), { pid: process.pid, port, url: `http://127.0.0.1:${port}/` });
    expect(await readRulesDashboard({ cwd })).toEqual({ url: await scoped(`http://127.0.0.1:${port}`), reason: null });
    await rm(join(home, "daemon.json"));
    expect(await readRulesDashboard({ cwd })).toEqual({ url: null, reason: "daemon unavailable" });
  });

  it("links the repository picker when the daemon has not seen the checkout, whichever repository it started in", async () => {
    const port = await listen();
    await record(join(home, "daemon.json"), { pid: process.pid, port, url: `http://127.0.0.1:${port}/` });
    knownCheckout = false;
    expect(await readRulesDashboard({ cwd })).toEqual({ url: `http://127.0.0.1:${port}/#rules`, reason: null });
  });

  it("scopes steers' Why? links to the checkout's repository when the daemon knows it", async () => {
    const port = await listen();
    await record(join(home, "daemon.json"), { pid: process.pid, port, url: `http://127.0.0.1:${port}/` });
    const steer = { sessionId: "s", requested: "steer" as const, mechanism: "additional-context" as const, content: steerBlock("a:b", "Fix it."), ruleIds: ["a:b"] };
    const rulePage = (await scoped(`http://127.0.0.1:${port}`)).replace("#rules", "#rule/a%3Ab");
    expect((await linkDeliveries([steer], cwd))[0]?.content).toContain(`Why? → ${rulePage}`);
    knownCheckout = false;
    expect((await linkDeliveries([steer], cwd))[0]?.content).toContain(`Why? → http://127.0.0.1:${port}/#rule/a%3Ab`);
  });

  it("reads the repository's start state from a live daemon, and nothing from a dead one", async () => {
    const port = await listen();
    await record(join(home, "daemon.json"), { pid: process.pid, port, url: `http://127.0.0.1:${port}/` });
    expect(await readRepositoryState(cwd, "omp")).toBe("stopped");
    await record(join(home, "daemon.json"), { pid: 999_999 });
    expect(await readRepositoryState(cwd, "omp")).toBeNull();
  });

  it("lets a live daemon's answer decide the hook start check, and reads the database only once none answers", async () => {
    startTestRepository(cwd, database);
    const port = await listen();
    await record(join(home, "daemon.json"), { pid: process.pid, port, url: `http://127.0.0.1:${port}/` });
    // The daemon says stopped; this process's database, which says started, is not read.
    expect(await hookRerailStarted("claude-code", cwd)).toBe(false);
    await record(join(home, "daemon.json"), { pid: 999_999 });
    expect(await hookRerailStarted("claude-code", cwd)).toBe(true);
  });

  it("refuses a recorded daemon whose pid is gone or whose port is vacant", async () => {
    const port = await listen();
    await stop();
    await record(join(home, "daemon.json"), { pid: 999_999, port, url: `http://127.0.0.1:${port}/` });
    expect(await readRulesDashboard({ cwd })).toEqual({ url: null, reason: "daemon unavailable" });
    await record(join(home, "daemon.json"), { pid: process.pid });
    expect(await readRulesDashboard({ cwd })).toEqual({ url: null, reason: "daemon unavailable" });
  });

  it("refuses a listener that answers as a different daemon", async () => {
    const port = await listen(999_999);
    await record(join(home, "daemon.json"), { pid: process.pid, port, url: `http://127.0.0.1:${port}/` });
    expect(await readRulesDashboard({ cwd })).toEqual({ url: null, reason: "daemon unavailable" });
  });

});
