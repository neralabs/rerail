import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { refresh, setNeeds } from "../../ui/poll.js";
import { data, setScope, ui } from "../../ui/state.js";
import { currentNeeds, pageKey } from "../../ui/views/index.js";

/** Whether the test daemon admits the browser; without its access capability every data read answers 401. */
let admitted = false;
/** Paths the daemon refuses even when it admits the browser. */
let refused: ReadonlySet<string> = new Set();
/** Every path the page read, in order. */
let requested: string[] = [];

const REPOSITORY = { setUp: true, repositoryKey: "A", name: "a" };

function daemon(url: URL): unknown {
  requested.push(url.pathname);
  if (!admitted || refused.has(url.pathname)) return new Response(JSON.stringify({ error: "access-required" }), { status: 401 });
  if (url.pathname === "/healthz") return { version: "1", pid: 1 };
  if (url.pathname === "/api/repositories") return { repositories: [] };
  if (url.pathname === "/api/repository") return { repository: REPOSITORY, checkout: null };
  if (url.pathname === "/api/repository/rules") return { rules: [], unavailable: [] };
  if (url.pathname === "/api/rule-packs") return { packs: [] };
  if (url.pathname === "/api/rules/activity") return { since: 0, rules: {}, sessions: { started: 0, withViolations: 0 } };
  if (url.pathname === "/api/sessions") return { sessions: [], total: 0, nextCursor: null };
  throw new Error(`unexpected ${url.pathname}`);
}

beforeEach(() => {
  admitted = false;
  refused = new Set();
  requested = [];
  vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
  vi.stubGlobal("fetch", async (input: URL) => {
    const answered = daemon(new URL(input));
    return answered instanceof Response ? answered : new Response(JSON.stringify(answered), { status: 200 });
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  setScope({ repository: "A" });
  data.repository = null;
  Object.assign(data, { accessRequired: false, missing: false, loaded: false, disconnected: false });
  Object.assign(data, { mergeAssist: null, mergeAssistError: null, mergeAssistExperiment: null });
  ui.panel = null;
  ui.view = "overview";
  setNeeds(currentNeeds);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

// A dashboard link copied from a coding agent's session, opened in a browser that never opened the link
// `rerail manage` opens. The daemon answers 401 to every read, and each view drew its own nothing-loaded state:
// Overview's zero tiles, a pack page's "Loading…" that never ended. The poll's reads are refused together; one left
// unhandled fails this file's run (vitest reports unhandled rejections), as it logged an error in the browser.
describe("a browser without the dashboard's access capability", () => {
  it("shows the access page before the repository is known", async () => {
    await refresh();

    expect(data.accessRequired).toBe(true);
    expect(pageKey()).toBe("access");
  });

  // The repository is known (read before the capability was lost), so each view's own reads go out alongside the
  // context and the sidebar's, and every one is refused: a refused browser, not a lost connection.
  it.each([
    ["overview", ["/api/rules/activity", "/api/sessions"]],
    ["pack", ["/api/rules/activity"]],
    ["created", ["/api/rules/activity"]],
    ["packs", ["/api/rules/activity"]],
    ["history", ["/api/rules/activity"]],
    ["sessions", ["/api/sessions"]],
    ["merge", []],
    ["create", []],
    ["settings", []],
  ])("shows the access page in place of %s once its reads are refused", async (view, own) => {
    data.repository = REPOSITORY;
    ui.view = view;

    await refresh();

    expect(requested).toEqual(expect.arrayContaining(["/api/repository", "/api/repository/rules", "/api/rule-packs", ...own]));
    if (view === "merge") expect(requested).not.toContain("/api/merge-assist");
    expect(data.accessRequired).toBe(true);
    expect(data.disconnected).toBe(false);
    expect(pageKey()).toBe("access");
    // An expected state, polled every 5 s: no warning each time.
    expect(console.warn).not.toHaveBeenCalled();
  });

  it("takes a refused Merge Assist read as a refused browser, not as Merge Assist failing", async () => {
    admitted = true;
    refused = new Set(["/api/merge-assist"]);
    data.repository = REPOSITORY;
    ui.view = "merge";

    await refresh();

    expect(requested).toContain("/api/merge-assist");
    expect(data.accessRequired).toBe(true);
    expect(data.mergeAssistError).toBeNull();
    expect(pageKey()).toBe("access");
  });

  it("shows the view again once the daemon admits the browser", async () => {
    await refresh();
    expect(pageKey()).toBe("access");

    admitted = true;
    await refresh();

    expect(data.accessRequired).toBe(false);
    expect(data.disconnected).toBe(false);
    expect(pageKey()).toBe("overview");
  });
});

describe("a daemon that does not answer", () => {
  it("is a lost connection, and the page warns", async () => {
    vi.stubGlobal("fetch", async () => { throw new TypeError("fetch failed"); });

    await refresh();

    expect(data.disconnected).toBe(true);
    expect(data.accessRequired).toBe(false);
    expect(console.warn).toHaveBeenCalledWith("poll failed", expect.any(Error));
  });
});
