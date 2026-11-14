import { mkdtemp, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RerailDaemon } from "../../src/daemon/daemon.js";
import type { AuditRecorder, CodeRuleAuditor } from "../../src/engine/index.js";
import { ingestEngineEvent, type EngineService, type startEngineService } from "../../src/runtime/index.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { startTestRepository } from "../support/started-repository.js";

/**
 * The engine service each daemon here starts: its sessions are suspended after a minute of `clock` without an event,
 * and its evaluator checks nothing and completes each scan at once, recorded as a real scan is.
 */
const engines = vi.hoisted(() => ({
  clock: { now: 1_000_000 },
  started: [] as EngineService[],
  createAuditor: ({ workspaceRoot, audits }: { workspaceRoot: string; audits: AuditRecorder }): CodeRuleAuditor => ({
    openWorkspace: async () => undefined,
    acceptsPath: () => true,
    auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
    audit: async (sessionId, input) => {
      const auditId = audits.start({
        sessionId, workspaceRoot, scope: input.scope, mode: input.mode, revision: 0, catalogRevision: "test", ruleRevisions: [], paths: [],
      });
      audits.finish(auditId, { status: "completed", violations: [], failures: [] });
      return [auditId];
    },
    close: async () => undefined,
  }),
}));
vi.mock("../../src/runtime/engine-service.js", async importOriginal => {
  const actual = await importOriginal<{ startEngineService: typeof startEngineService }>();
  return {
    ...actual,
    startEngineService: async (options: Parameters<typeof startEngineService>[0]) => {
      const service = await actual.startEngineService({
        ...options, idleSessionMs: 60_000, idleRuntimeMs: 0, now: () => engines.clock.now, createAuditor: engines.createAuditor,
      });
      engines.started.push(service);
      return service;
    },
  };
});

const ACCESS = "test-access-capability";

describe("the daemon and a session it let go of while quiet", () => {
  let root: string | undefined;
  const daemons: RerailDaemon[] = [];

  afterEach(async () => {
    await Promise.all(daemons.splice(0).map(daemon => daemon.close()));
    engines.started.splice(0);
    vi.unstubAllEnvs();
    if (root !== undefined) await removeFolder(root);
    root = undefined;
  });

  /** `path` of the daemon's API, with its capability, and the JSON it answers. */
  async function api<T>(daemon: RerailDaemon, path: string, body?: unknown): Promise<{ status: number; body: T }> {
    const response = await fetch(new URL(path, daemon.url), {
      method: body === undefined ? "GET" : "POST",
      headers: { "x-rerail-access": ACCESS, ...(body === undefined ? {} : { "content-type": "application/json" }) },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    // Test boundary: the daemon's JSON is asserted against expectations right after.
    return { status: response.status, body: await response.json() as T };
  }

  // `rerail scan --session` on a session whose agent sends no more events (closed, or gone): the scan queued for the
  // session, and nothing opened it to run the scan, so it stayed queued.
  it("runs a scan asked through its API for a session it suspended while quiet", async () => {
    root = await realpath(await mkdtemp(join(tmpdir(), "rerail-daemon-quiet-")));
    const databasePath = join(root, "rerail.sqlite");
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    startTestRepository(root, databasePath);
    const daemon = await RerailDaemon.start({
      version: "0.0.0-test", databasePath, workspaceRoot: root, port: 0, nativeHealth: () => ({ ok: true, kernelVersion: "1" }), accessToken: ACCESS,
    });
    daemons.push(daemon);
    await ingestEngineEvent({ socketPath: daemon.kernelSocketPath, accessToken: ACCESS }, {
      kind: "session-start", eventId: "a:start", session: { host: "claude-code", sessionId: "a", cwd: root },
    });
    const [engine] = engines.started;
    engines.clock.now += 60_000;
    await engine!.closeIdleSessions();
    expect(engine!.openRuntimes()).toBe(0);
    const queued = await api<{ id: string; status: string }>(daemon, "/api/sessions/a/audit-requests", { scope: "repository" });
    expect(queued).toMatchObject({ status: 200, body: { status: "queued" } });
    await vi.waitFor(async () => {
      const listed = await api<{ auditRequests: { id: string; status: string }[] }>(daemon, "/api/sessions/a/audit-requests");
      expect(listed.body.auditRequests.find(request => request.id === queued.body.id)).toMatchObject({ status: "completed" });
    }, { timeout: 10_000, interval: 100 });
  });
});
