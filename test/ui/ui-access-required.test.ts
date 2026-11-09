import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { refresh, setNeeds } from "../../ui/poll.js";
import { data, setScope, ui } from "../../ui/state.js";
import { currentNeeds, pageKey } from "../../ui/views/index.js";

/** Whether the test daemon admits the browser; without its access capability every data read answers 401. */
let admitted = false;

function daemon(url: URL): unknown {
  if (!admitted) return new Response(JSON.stringify({ error: "access-required" }), { status: 401 });
  if (url.pathname === "/healthz") return { version: "1", pid: 1 };
  if (url.pathname === "/api/repositories") return { repositories: [] };
  if (url.pathname === "/api/repository") return { repository: { setUp: true, repositoryKey: "A", name: "a" }, checkout: null };
  if (url.pathname === "/api/repository/rules") return { rules: [], unavailable: [] };
  if (url.pathname === "/api/rule-packs") return { packs: [] };
  if (url.pathname === "/api/rules/activity") return { since: 0, rules: {}, sessions: { started: 0, withViolations: 0 } };
  if (url.pathname === "/api/sessions") return { sessions: [], total: 0, nextCursor: null };
  throw new Error(`unexpected ${url.pathname}`);
}

beforeEach(() => {
  admitted = false;
  vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
  vi.stubGlobal("fetch", async (input: URL) => {
    const answered = daemon(new URL(input));
    return answered instanceof Response ? answered : new Response(JSON.stringify(answered), { status: 200 });
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  setScope({ repository: "A" });
  data.repository = null;
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
  it.each(["overview", "pack", "created", "packs", "create", "history", "sessions", "settings"])("shows the access page in place of %s", async view => {
    ui.view = view;

    await refresh();

    expect(data.accessRequired).toBe(true);
    expect(pageKey()).toBe("access");
  });

  it("shows the view again once the daemon admits the browser", async () => {
    await refresh();
    expect(pageKey()).toBe("access");

    admitted = true;
    await refresh();

    expect(data.accessRequired).toBe(false);
    expect(pageKey()).toBe("overview");
  });
});
