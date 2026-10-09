import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { captureScope, changeScope, inScope, recommendationsFor, sendInScope } from "../ui/poll.js";
import { data, setScope } from "../ui/state.js";

type Request = { method: string; url: URL; body: unknown; answer: (status: number, body?: unknown) => void };

/**
 * The page's fetch, answered by the test: every request waits until the test answers it, so a scope switch can
 * land while it is in flight. The dashboard's background polls are left unanswered.
 */
let requests: Request[] = [];

beforeEach(() => {
  requests = [];
  vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
  vi.stubGlobal("fetch", (input: URL, init: { method?: string; body?: string } = {}) => {
    const { promise, resolve } = Promise.withResolvers<Response>();
    requests.push({
      method: init.method ?? "GET",
      url: new URL(input),
      body: init.body === undefined ? undefined : JSON.parse(init.body),
      answer: (status, body) => resolve(new Response(body === undefined ? null : JSON.stringify(body), { status })),
    });
    return promise;
  });
  data.repository = null;
  data.recommendations = null;
  setScope({ repository: "A" });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

/** The writes sent so far; `send` calls fetch before its first await, so a write is here as soon as it starts. */
const writes = () => requests.filter(r => r.method === "PUT");
const switchToB = () => changeScope({ repository: "B" });

describe("a write started in one repository", () => {
  it("takes its folders from that repository's recommendations when the person switches while they load", async () => {
    const scope = captureScope();
    const loading = recommendationsFor(scope);
    switchToB();
    const read = requests.find(r => r.url.pathname === "/api/rule-packs/recommendations");
    expect(read!.url.searchParams.get("repository")).toBe("A");
    read!.answer(200, { recommendations: [{ packId: "tslint", folder: "web" }] });

    expect(await loading).toEqual([{ packId: "tslint", folder: "web" }]);
    expect(data.recommendations).toBeNull();
    expect(inScope(scope)).toBe(false);
  });

  it("sends nothing once the page shows another repository", async () => {
    const scope = captureScope();
    switchToB();

    expect(await sendInScope(scope, "PUT", "/api/repository/attachments", { add: [] })).toBe(false);
    expect(writes()).toEqual([]);
  });

  it("goes to the captured repository and reports no rollback when it fails after a switch", async () => {
    const scope = captureScope();
    const sending = sendInScope(scope, "PUT", "/api/repository/attachments", { remove: [] });
    switchToB();
    const [write] = writes();
    expect(write!.body).toEqual({ repository: "A", remove: [] });
    write!.answer(503, { error: "daemon restarting" });

    await expect(sending).resolves.toBe(false);
  });

  it("rejects, so the caller rolls back, when it fails while the page still shows the repository", async () => {
    const sending = sendInScope(captureScope(), "PUT", "/api/repository/attachments", { remove: [] });
    writes()[0]!.answer(503, { error: "daemon restarting" });

    await expect(sending).rejects.toThrow(/answered 503/u);
  });

  it("reports success only while the page still shows the repository", async () => {
    const kept = sendInScope(captureScope(), "PUT", "/api/repository/attachments", { add: [] });
    writes()[0]!.answer(200, {});
    await expect(kept).resolves.toBe(true);

    const left = sendInScope(captureScope(), "PUT", "/api/repository/attachments", { add: [] });
    switchToB();
    writes()[1]!.answer(200, {});
    await expect(left).resolves.toBe(false);
  });
});
