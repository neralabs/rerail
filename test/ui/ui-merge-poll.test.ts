import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MergeAssistStatus } from "../../src/engine/index.js";
import { captureScope, refresh, setNeeds, writeMergeAssist } from "../../ui/poll.js";
import { data, setScope, ui } from "../../ui/state.js";
import { currentNeeds } from "../../ui/views/index.js";
import { sampleStatus } from "../support/merge-assist/status.js";
import { conflict } from "./support/merge-conflicts.js";

/** A request the test answers itself: every Merge Assist write, and its reads while `holding`. */
interface Held { method: string; url: URL; resolve: (response: Response) => void }

let requested: URL[] = [];
let held: Held[] = [];
let holding = false;
/** How the daemon answers a Merge Assist read the test does not hold. */
let mergeRead: () => Response;

const json = (body: unknown, code = 200): Response => new Response(JSON.stringify(body), { status: code });

/** Merge Assist on, with one conflict open (`open`) or dismissed (`dismissed`). */
const status = (conflictStatus: "open" | "dismissed"): MergeAssistStatus => sampleStatus("/work/a", {
  conflicts: [conflict("c1", {
    status: conflictStatus,
    dismissal: conflictStatus === "dismissed" ? { name: "tbelanger", at: "2026-11-02T11:00:00Z" } : null,
  })],
});

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
const dismiss = () => writeMergeAssist(captureScope(), "POST", "/api/merge-assist/conflicts/dismiss", { conflictId: "c1" });

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

    const dismissing = dismiss();
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.resolve(json(status("dismissed")));
    expect((await dismissing)!.conflicts[0]!.status).toBe("dismissed");

    reads()[0]!.resolve(json(status("open")));
    await polling;
    expect(data.mergeAssist!.conflicts[0]!.status).toBe("dismissed");
  });

  it("drops a read sent while a write is out, though it answers after the write", async () => {
    const dismissing = dismiss();
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    holding = true;
    const polling = refresh();
    await vi.waitFor(() => expect(reads()).toHaveLength(1));

    writes()[0]!.resolve(json(status("dismissed")));
    await dismissing;
    reads()[0]!.resolve(json(status("open")));
    await polling;
    expect(data.mergeAssist!.conflicts[0]!.status).toBe("dismissed");
  });

  it("keeps the answer of the latest write when an earlier one answers last", async () => {
    const first = dismiss();
    const second = dismiss();
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    writes()[1]!.resolve(json(status("dismissed")));
    await second;
    writes()[0]!.resolve(json(status("open")));
    await first;
    expect(data.mergeAssist!.conflicts[0]!.status).toBe("dismissed");
  });

  it("takes a read sent after the last write answered", async () => {
    const dismissing = dismiss();
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.resolve(json(status("dismissed")));
    await dismissing;
    await refresh();
    expect(data.mergeAssist!.conflicts[0]!.status).toBe("open");
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
