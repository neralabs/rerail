import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MergeAssistStatus } from "../../src/engine/index.js";
import { captureScope, refresh, setNeeds, writeMergeAssist } from "../../ui/poll.js";
import { data, setScope, ui } from "../../ui/state.js";
import { currentNeeds } from "../../ui/views/index.js";

/** A request the test answers itself: every Merge Assist write, and its reads while `holding`. */
interface Held { method: string; url: URL; resolve: (response: Response) => void }

let requested: URL[] = [];
let held: Held[] = [];
let holding = false;
/** How the daemon answers a Merge Assist read the test does not hold. */
let mergeRead: () => Response;

const json = (body: unknown, code = 200): Response => new Response(JSON.stringify(body), { status: code });

/** Merge Assist on, with one finding needing review (`open`) or adopted (`applied`). */
function status(findingStatus: "open" | "applied"): MergeAssistStatus {
  return {
    repositoryKey: "A", checkout: "/work/a", branch: "mine", baseBranch: "main", enabled: true,
    readiness: {
      judge: { available: true, runner: "claude", problem: null },
      github: { state: "not-github", login: null, repository: null },
      candidatesIgnored: false, rulesActive: true,
    },
    budget: { used: 0, limit: 40, windowMinutes: 60 },
    scanning: false,
    lastScan: null,
    own: [],
    findings: [{
      id: "f1", tier: "review", status: findingStatus,
      candidate: { id: "candidate:x-0a0b0c", decision: "Do x.", scope: [], level: "note", author: "jbarnes", createdAt: "2026-11-02T10:00:00Z" },
      peer: { kind: "branch", ref: "jbarnes/x", commit: "abc", number: null, title: null, url: null, author: null, trusted: true },
      reason: "r", evidence: { violations: [], conflicts: [], mine: null, judgment: null }, message: null, updatedAt: "2026-11-02T11:00:00Z",
    }],
    conflicts: [],
    decisionLog: { questions: [], problems: [] },
  };
}

/** Every other read a poll makes, each empty. */
function emptyDaemon(url: URL): Response {
  if (url.pathname === "/healthz") return json({ version: "1", pid: 1 });
  if (url.pathname === "/api/repositories") return json({ repositories: [] });
  if (url.pathname === "/api/repository") return json({ repository: data.repository, checkout: null });
  if (url.pathname === "/api/repository/rules") return json({ rules: [], unavailable: [] });
  if (url.pathname === "/api/rule-packs") return json({ packs: [] });
  throw new Error(`unexpected ${url.pathname}`);
}

const reads = () => held.filter(r => r.method === "GET");
const writes = () => held.filter(r => r.method !== "GET");
const adopt = () => writeMergeAssist(captureScope(), "PUT", "/api/merge-assist/findings", { findingId: "f1", action: "adopt" });

beforeEach(() => {
  requested = [];
  held = [];
  holding = false;
  mergeRead = () => json(status("open"));
  vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
  vi.stubGlobal("fetch", async (input: URL, init: { method?: string } = {}) => {
    const url = new URL(input);
    const method = init.method ?? "GET";
    requested.push(url);
    if (!url.pathname.startsWith("/api/merge-assist")) return emptyDaemon(url);
    if (method === "GET" && !holding) return mergeRead();
    const { promise, resolve } = Promise.withResolvers<Response>();
    held.push({ method, url, resolve });
    return promise;
  });
  setScope({ repository: "A", checkout: "/work/a" });
  data.repository = { setUp: true, repositoryKey: "A" };
  Object.assign(data, { mergeAssist: null, mergeAssistError: null });
  ui.view = "merge";
  ui.panel = null;
  setNeeds(currentNeeds);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("Merge Assist reads and writes", () => {
  it("drops a read sent before a write once the write has answered: it may hold the state from before it", async () => {
    holding = true;
    const polling = refresh();
    await vi.waitFor(() => expect(reads()).toHaveLength(1));

    const adopting = adopt();
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.resolve(json(status("applied")));
    expect((await adopting)!.findings[0]!.status).toBe("applied");

    reads()[0]!.resolve(json(status("open")));
    await polling;
    expect(data.mergeAssist!.findings[0]!.status).toBe("applied");
  });

  it("drops a read sent while a write is out, though it answers after the write", async () => {
    const adopting = adopt();
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    holding = true;
    const polling = refresh();
    await vi.waitFor(() => expect(reads()).toHaveLength(1));

    writes()[0]!.resolve(json(status("applied")));
    await adopting;
    reads()[0]!.resolve(json(status("open")));
    await polling;
    expect(data.mergeAssist!.findings[0]!.status).toBe("applied");
  });

  it("keeps the answer of the latest write when an earlier one answers last", async () => {
    const first = adopt();
    const second = adopt();
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    writes()[1]!.resolve(json(status("applied")));
    await second;
    writes()[0]!.resolve(json(status("open")));
    await first;
    expect(data.mergeAssist!.findings[0]!.status).toBe("applied");
  });

  it("takes a read sent after the last write answered", async () => {
    const adopting = adopt();
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.resolve(json(status("applied")));
    await adopting;
    await refresh();
    expect(data.mergeAssist!.findings[0]!.status).toBe("open");
  });

  it("reads the sidebar's count at most every 30 s while Merge Assist cannot be read, not on every poll", async () => {
    ui.view = "settings";
    mergeRead = () => json({ error: "internal" }, 500);

    await refresh();
    await refresh();

    expect(data.mergeAssistError).toMatch(/answered 500$/u);
    expect(requested.filter(url => url.pathname === "/api/merge-assist")).toHaveLength(1);
  });
});
