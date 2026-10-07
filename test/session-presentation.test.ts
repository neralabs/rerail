import { mkdtemp, mkdir, realpath, rm, writeFile } from "node:fs/promises";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { formatTerminalStatus, readPresentation, readRulesPage } from "../src/session-presentation.js";
import type { presentationResponseSchema, sessionResponseSchema } from "../src/api/session-contracts.js";
import type { z } from "zod";

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
    databasePath: database, kernelSocketPath: join(home, "kernel.sock"), ui: "bundled", startedAt: 1 };
  await writeFile(join(home, "daemon.json"), JSON.stringify(state));
  requested = [];
  sessionContext = { session: { sessionId, host: "claude-code", cwd, repositoryId: "repository-under-test", startedAt: 1, endedAt: null, eventCount: 0 },
    hud: { activeRuleCount: 3, steeringCount: 2 }, evaluation: null };
  response = { presence: "compact", context: {
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
    expect(result.text).toBe("Rerail | 3 rules | steered 2×");
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

  it("honors silent without session/history reads but exposes on-demand history", async () => {
    response.presence = "silent";
    response.context!.session = null;
    expect((await read()).parts).toEqual([]);
    expect(requested.some(url => url.includes("/api/sessions/"))).toBe(false);
    response.context!.session = structuredClone(sessionContext);
    const onDemand = await read(true);
    expect(onDemand.presence).toBe("silent");
    expect(onDemand.text).toContain("healthy | not evaluated");
  });

  it.each(["host", "cwd", "sessionId"])("refuses navigation and counts for mismatched %s", async dimension => {
    const session = response.context!.session!.session;
    if (dimension === "host") session.host = "omp";
    if (dimension === "cwd") session.cwd = join(cwd, "nested");
    if (dimension === "sessionId") session.sessionId = "another-session";
    await mkdir(join(cwd, "nested"));
    const result = await read();
    expect(result.text).toBe("Rerail | session unavailable");
    expect(result.parts.every(part => part.url === null)).toBe(true);
  });

  it("reads the launch root's database and session from a directory below it", async () => {
    const nested = join(cwd, "nested");
    await mkdir(nested);
    const result = await readPresentation({ sessionId, host: "claude-code", cwd: nested });
    expect(result.text).toBe("Rerail | 3 rules | steered 2×");
    expect(new URL(requested[0]!).searchParams.get("repository")).toBe(await realpath(nested));
  });

  it("does not fabricate links for a missing session or zero counts for unavailable counters", async () => {
    const session = response.context!.session!;
    response.context!.session = null;
    expect((await read()).parts.every(part => part.url === null)).toBe(true);
    response.context!.session = session;
    session.hud = { activeRuleCount: null, steeringCount: null };
    expect((await read()).text).toBe("Rerail | rules unavailable | steering unavailable");
  });

  it("rereads the daemon locator after restart", async () => {
    await read();
    state.url = "http://127.0.0.1:5737/";
    state.port = 5737;
    await writeFile(join(home, "daemon.json"), JSON.stringify(state));
    expect((await read()).url).toContain(":5737/");
  });

  it("uses checkout identity instead of the host daemon and refuses sibling or missing identity before silent preferences", async () => {
    const directory = join(cwd, ".nera");
    await mkdir(directory);
    await writeFile(join(directory, "instance.env"), 'NERA_RERAIL_ENABLED="true"\nNERA_RERAIL_PUBLIC_URL="http://127.0.0.1:35007"\n');
    const locator = join(directory, "rerail-daemon.json");
    await writeFile(locator, JSON.stringify({ ...state, pid: 1, host: "0.0.0.0", url: "http://0.0.0.0:4737/", workspaceRoot: cwd }));
    Object.assign(response.context!.daemon, { pid: 1, workspaceRoot: cwd });
    const result = await read();
    expect(result.text).toBe("Rerail | 3 rules | steered 2×");
    expect(result.url).toContain("http://127.0.0.1:35007/");
    expect(requested).toHaveLength(1);
    vi.stubEnv("RERAIL_WORKSPACE", await realpath(cwd));
    vi.stubEnv("RERAIL_INTERNAL_PORT", "4737");
    requested = [];
    expect((await read()).url).toBe(result.url);
    expect(new URL(requested[0]!).port).toBe("4737");
    expect(requested).toHaveLength(1);

    response.presence = "silent";
    response.context!.daemon.workspaceRoot = home;
    expect((await read()).text).toBe("Rerail | daemon unavailable");
    await rm(locator);
    requested = [];
    expect((await read()).text).toBe("Rerail | daemon unavailable");
    expect(requested).toEqual([]);
  });

  it("rejects replaced daemon identity, remote locators, and oversized responses", async () => {
    response.context!.daemon.pid++;
    expect((await read()).text).toBe("Rerail | daemon unavailable");
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
    const rendered = formatTerminalStatus({ text: "", url: null, presence: "compact", parts: [
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
      response.writeHead(request.url === "/healthz" ? 200 : 404, { "content-type": "application/json" });
      response.end(JSON.stringify(health));
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
    expect(await readRulesPage({ cwd })).toEqual({ url: `http://127.0.0.1:${port}/#rules`, reason: null });
    health.repositoryId = "another-repository";
    expect(await readRulesPage({ cwd })).toEqual({ url: null, reason: "daemon serves another repository" });
    await rm(join(home, "daemon.json"));
    expect(await readRulesPage({ cwd })).toEqual({ url: null, reason: "daemon unavailable" });
  });

  it("refuses to link the rules view unless both the daemon and the checkout name the same repository", async () => {
    const port = await listen();
    await record(join(home, "daemon.json"), { pid: process.pid, port, url: `http://127.0.0.1:${port}/` });
    delete health.repositoryId;
    expect(await readRulesPage({ cwd })).toEqual({ url: null, reason: "daemon serves another repository" });
    await rm(join(cwd, ".rerail", "repository-id"));
    expect(await readRulesPage({ cwd })).toEqual({ url: null, reason: "daemon serves another repository" });
    health.repositoryId = "repository-under-test";
    expect(await readRulesPage({ cwd })).toEqual({ url: null, reason: "daemon serves another repository" });
  });

  it("refuses a recorded daemon whose pid is gone or whose port is vacant", async () => {
    const port = await listen();
    await stop();
    await record(join(home, "daemon.json"), { pid: 999_999, port, url: `http://127.0.0.1:${port}/` });
    expect(await readRulesPage({ cwd })).toEqual({ url: null, reason: "daemon unavailable" });
    await record(join(home, "daemon.json"), { pid: process.pid });
    expect(await readRulesPage({ cwd })).toEqual({ url: null, reason: "daemon unavailable" });
  });

  it("refuses a listener that answers as a different daemon", async () => {
    const port = await listen(999_999);
    await record(join(home, "daemon.json"), { pid: process.pid, port, url: `http://127.0.0.1:${port}/` });
    expect(await readRulesPage({ cwd })).toEqual({ url: null, reason: "daemon unavailable" });
  });

  it("prefers the checkout instance's public address over its internal listener, and drops it once dead", async () => {
    const port = await listen(1);
    const directory = join(cwd, ".nera");
    await mkdir(directory);
    await writeFile(join(directory, "instance.env"), 'NERA_RERAIL_ENABLED="true"\nNERA_RERAIL_PUBLIC_URL="http://127.0.0.1:35007"\n');
    const locator = join(directory, "rerail-daemon.json");
    await record(locator, { pid: 1, host: "0.0.0.0", port, url: `http://0.0.0.0:${port}/`, workspaceRoot: cwd });
    vi.stubEnv("RERAIL_WORKSPACE", await realpath(cwd));
    vi.stubEnv("RERAIL_INTERNAL_PORT", String(port));
    expect(await readRulesPage({ cwd })).toEqual({ url: "http://127.0.0.1:35007/#rules", reason: null });
    await stop();
    await record(locator, { pid: 999_999 });
    expect(await readRulesPage({ cwd })).toEqual({ url: null, reason: "daemon unavailable" });
  });
});
