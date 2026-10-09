import { mkdtemp, mkdir, realpath, rm, writeFile } from "node:fs/promises";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { dashboardOrigin, formatTerminalStatus, linkDeliveries, readPresentation, readRepositoryState, readRulesDashboard, readSummaryState } from "../../src/presentation/session-presentation.js";
import type { RepositoryStartState } from "../../src/runtime/index.js";
import { steerBlock, openRerailDatabase } from "../../src/engine/index.js";
import type { presentationResponseSchema, sessionResponseSchema } from "../../src/api/index.js";
import type { z } from "zod";
import { startRepository } from "../../src/runtime/index.js";
import { hookRerailStarted } from "../../src/harness/index.js";

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
    expect(new URL(result.parts[1]!.url!).hash).toBe("#rules");
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

  it("reports not on here, with the state the daemon gave, where Rerail is stopped or was never started", async () => {
    response.repositoryState = "stopped";
    const stopped = await read();
    expect(stopped).toMatchObject({ status: "not-on-here", text: "Rerail ○ not on here", repositoryState: "stopped" });
    expect(stopped.parts.every(part => part.url === null)).toBe(true);
    // A new session in a repository that was never started has no recorded session at all.
    response.repositoryState = "never";
    response.context!.session = null;
    expect(await read()).toMatchObject({ text: "Rerail ○ not on here", repositoryState: "never" });
    expect((await read(true)).repositoryState).toBe("never");
    response.presence = "silent";
    expect(await read()).toMatchObject({ parts: [], repositoryState: "never" });
  });

  it("with no daemon, is not on here unless the caller's start state says the checkout is started", async () => {
    await rm(join(home, "daemon.json"));
    const withState = (startState: () => RepositoryStartState) => readPresentation({ sessionId, host: "claude-code", cwd, startState });
    expect(await withState(() => "never")).toMatchObject({ status: "not-on-here", text: "Rerail ○ not on here", repositoryState: "never" });
    expect(await withState(() => "stopped")).toMatchObject({ status: "not-on-here", repositoryState: "stopped" });
    expect(await withState(() => "started")).toMatchObject({ status: "not-running", text: "Rerail ✕ not running" });
    expect(await withState(() => { throw new Error("database unreadable"); })).toMatchObject({ status: "not-running" });
    expect(await read()).toMatchObject({ status: "not-running" });
    expect(requested).toEqual([]);
  });

  it("summarizes a repository without a session from the daemon first, else the local state", async () => {
    await rm(join(home, "daemon.json"));
    expect(await readSummaryState(cwd, "cursor", () => "never")).toBe("never");
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

  const stop = async (): Promise<void> => {
    const server = stub;
    stub = undefined;
    if (server !== undefined) await new Promise<void>(resolve => server.close(() => resolve()));
  };

  /** Liveness is only provable over the wire, so these fixtures answer `/healthz` from a real listener. */
  const listen = async (pid: number = process.pid): Promise<number> => {
    vi.unstubAllGlobals();
    health = { status: "ok", pid, database: { path: database, exists: true }, native: { ok: true }, repositoryId: "repository-under-test" };
    stub = createServer((request, response) => {
      const path = new URL(request.url ?? "/", "http://127.0.0.1").pathname;
      response.writeHead(path === "/healthz" || path === "/api/presentation" ? 200 : 404, { "content-type": "application/json" });
      response.end(JSON.stringify(path === "/api/presentation" ? { presence: "compact", repositoryState: "stopped" } : health));
    });
    await new Promise<void>(resolve => stub!.listen(0, "127.0.0.1", () => resolve()));
    return (stub!.address() as AddressInfo).port;
  };

  const record = async (file: string, fields: Record<string, unknown>): Promise<void> => {
    Object.assign(state, fields);
    await writeFile(file, JSON.stringify(state));
  };

  afterEach(stop);

  it("links the rules view of the daemon that answers as itself", async () => {
    const port = await listen();
    await record(join(home, "daemon.json"), { pid: process.pid, port, url: `http://127.0.0.1:${port}/` });
    expect(await readRulesDashboard({ cwd })).toEqual({ url: `http://127.0.0.1:${port}/#rules`, reason: null });
    health.repositoryId = "another-repository";
    expect(await readRulesDashboard({ cwd })).toEqual({ url: null, reason: "daemon serves another repository" });
    await rm(join(home, "daemon.json"));
    expect(await readRulesDashboard({ cwd })).toEqual({ url: null, reason: "daemon unavailable" });
  });

  it("reads the repository's start state from a live daemon, and nothing from a dead one", async () => {
    const port = await listen();
    await record(join(home, "daemon.json"), { pid: process.pid, port, url: `http://127.0.0.1:${port}/` });
    expect(await readRepositoryState(cwd, "omp")).toBe("stopped");
    await record(join(home, "daemon.json"), { pid: 999_999 });
    expect(await readRepositoryState(cwd, "omp")).toBeNull();
  });

  it("lets a live daemon's answer decide the hook start check, and reads the database only once none answers", async () => {
    const store = openRerailDatabase(database);
    startRepository(store, "repository-under-test");
    store.$client.close();
    const port = await listen();
    await record(join(home, "daemon.json"), { pid: process.pid, port, url: `http://127.0.0.1:${port}/` });
    // The daemon says stopped; this process's database, which says started, is not read.
    expect(await hookRerailStarted("claude-code", cwd)).toBe(false);
    await record(join(home, "daemon.json"), { pid: 999_999 });
    expect(await hookRerailStarted("claude-code", cwd)).toBe(true);
  });

  it("refuses to link the rules view unless both the daemon and the checkout name the same repository", async () => {
    const port = await listen();
    await record(join(home, "daemon.json"), { pid: process.pid, port, url: `http://127.0.0.1:${port}/` });
    delete health.repositoryId;
    expect(await readRulesDashboard({ cwd })).toEqual({ url: null, reason: "daemon serves another repository" });
    await rm(join(cwd, ".rerail", "repository-id"));
    expect(await readRulesDashboard({ cwd })).toEqual({ url: null, reason: "daemon serves another repository" });
    health.repositoryId = "repository-under-test";
    expect(await readRulesDashboard({ cwd })).toEqual({ url: null, reason: "daemon serves another repository" });
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
