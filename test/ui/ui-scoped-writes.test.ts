import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { captureScope, changeScope, inScope, queueInScope, recommendationsFor, sendInScope } from "../../ui/poll.js";
import { data, keptPacks, setScope, ui } from "../../ui/state.js";
import { attachmentWrites } from "../../ui/attachments.js";

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

describe("attachment writes clicked in quick succession", () => {
  const attachments = "/api/repository/attachments";
  const setUp = { add: [{ sourceKind: "pack", sourceId: "tslint", folder: "web" }], configure: true };
  const addZod = { add: [{ sourceKind: "pack", sourceId: "zod", folder: "" }] };
  const removeZod = { remove: [{ sourceKind: "pack", sourceId: "zod", folder: "web" }] };
  const recommendationsRead = () => requests.find(r => r.url.pathname === "/api/rule-packs/recommendations")!;

  it("sends the second only once the first is answered, in click order", async () => {
    const scope = captureScope();
    const first = queueInScope(scope, "PUT", attachments, setUp);
    const second = queueInScope(scope, "PUT", attachments, addZod);
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]!.body).toEqual({ repository: "A", ...setUp });

    writes()[0]!.answer(200, {});
    await expect(first).resolves.toBe(true);
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    expect(writes()[1]!.body).toEqual({ repository: "A", ...addZod });
    writes()[1]!.answer(200, {});
    await expect(second).resolves.toBe(true);
  });

  it("never sends a pack added behind a set-up that failed, so the repository is not set up with that pack alone", async () => {
    const scope = captureScope();
    const first = queueInScope(scope, "PUT", attachments, setUp);
    const second = queueInScope(scope, "PUT", attachments, addZod);
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.answer(503, { error: "daemon restarting" });

    await expect(first).rejects.toThrow(/answered 503/u);
    await expect(second).resolves.toBe(false);
    expect(writes()).toHaveLength(1);
  });

  it("keeps an add's place while its folders load, so a remove clicked after it goes out after it", async () => {
    const scope = captureScope();
    const adding = queueInScope(scope, "PUT", attachments, recommendationsFor(scope).then(r => ({
      add: r.map(({ packId, folder }) => ({ sourceKind: "pack", sourceId: packId, folder })),
    })));
    const removing = queueInScope(scope, "PUT", attachments, removeZod);
    recommendationsRead().answer(200, { recommendations: [{ packId: "zod", folder: "web" }] });

    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]!.body).toEqual({ repository: "A", add: [{ sourceKind: "pack", sourceId: "zod", folder: "web" }] });
    writes()[0]!.answer(200, {});
    await expect(adding).resolves.toBe(true);
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    expect(writes()[1]!.body).toEqual({ repository: "A", ...removeZod });
    writes()[1]!.answer(200, {});
    await expect(removing).resolves.toBe(true);
  });

  it("fails an add whose folders cannot be read, and sends nothing clicked after it", async () => {
    const scope = captureScope();
    const adding = queueInScope(scope, "PUT", attachments, recommendationsFor(scope).then(r => ({ add: r })));
    const removing = queueInScope(scope, "PUT", attachments, removeZod);
    recommendationsRead().answer(503, { error: "daemon restarting" });

    await expect(adding).rejects.toThrow(/answered 503/u);
    await expect(removing).resolves.toBe(false);
    expect(writes()).toEqual([]);
  });

  it("sends a write clicked after a failure has been answered", async () => {
    const failed = queueInScope(captureScope(), "PUT", attachments, setUp);
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.answer(503, { error: "daemon restarting" });
    await expect(failed).rejects.toThrow(/answered 503/u);

    const retried = queueInScope(captureScope(), "PUT", attachments, setUp);
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    writes()[1]!.answer(200, {});
    await expect(retried).resolves.toBe(true);
  });

  it("drops the writes queued in a repository once the page shows another, and does not hold the new one's writes", async () => {
    const inA = captureScope();
    const sentInA = queueInScope(inA, "PUT", attachments, setUp);
    const queuedInA = queueInScope(inA, "PUT", attachments, addZod);
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    switchToB();
    const inB = queueInScope(captureScope(), "PUT", attachments, addZod);
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    expect(writes().map(w => w.body)).toEqual([{ repository: "A", ...setUp }, { repository: "B", ...addZod }]);

    writes()[0]!.answer(200, {});
    await expect(sentInA).resolves.toBe(false);
    await expect(queuedInA).resolves.toBe(false);
    writes()[1]!.answer(200, {});
    await expect(inB).resolves.toBe(true);
    expect(writes()).toHaveLength(2);
  });

  it("resolves false, so nothing is rolled back onto the page, when its body fails after a switch", async () => {
    const scope = captureScope();
    const adding = queueInScope(scope, "PUT", attachments, recommendationsFor(scope).then(r => ({ add: r })));
    switchToB();
    recommendationsRead().answer(503, { error: "daemon restarting" });

    await expect(adding).resolves.toBe(false);
    expect(writes()).toEqual([]);
  });
});

describe("the page's rule pack writes", () => {
  const toast = vi.fn();
  const page = () => attachmentWrites({ toast, invalidate: () => {} });
  const pack = (id: string, folder: string) => ({ sourceKind: "pack", sourceId: id, folder });
  const recommended = [{ packId: "tslint", folder: "web" }, { packId: "zod", folder: "" }];
  const recommendationsRead = () => requests.find(r => r.url.pathname === "/api/rule-packs/recommendations")!;

  beforeEach(() => {
    toast.mockReset();
    // A fresh page for repository A: a new write queue, whatever an earlier test left pending.
    changeScope({ repository: "A" });
    data.repository = { setUp: false };
    ui.view = "review";
    ui.settingUp = false;
  });

  it("never puts the repository it started in back on another repository's page when it fails after a switch", async () => {
    const keeping = page().keepPack("zod", true, "Zod kept on.");
    switchToB();
    const shownInB = new Map([["eslint", [""]]]);
    data.attachedPacks = shownInB;
    data.repository = { setUp: true };
    recommendationsRead().answer(503, { error: "daemon restarting" });

    await keeping;
    expect(data.attachedPacks).toBe(shownInB);
    expect(data.repository).toEqual({ setUp: true });
    expect(toast).not.toHaveBeenCalled();
    expect(writes()).toEqual([]);
  });

  it("sends Set up with recommended packs before a review switch clicked after it, so the switch is not undone", async () => {
    data.recommendations = recommended;
    page().setUpRecommended();
    page().keepPack("zod", false, "Zod switched off.");

    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]!.body).toEqual({ repository: "A", add: [pack("tslint", "web"), pack("zod", "")], configure: true });
    writes()[0]!.answer(200, {});
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    expect(writes()[1]!.body).toEqual({ repository: "A", remove: [pack("zod", "")] });
    expect([...data.attachedPacks.keys()]).toEqual(["tslint"]);
  });

  it("shows the recommended packs added once they load, so a switch clicked then is queued behind the set-up", async () => {
    page().setUpRecommended();
    recommendationsRead().answer(200, { recommendations: recommended });
    await vi.waitFor(() => expect(data.repository).toEqual({ setUp: true }));
    page().keepPack("zod", false, "Zod switched off.");

    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]!.body).toEqual({ repository: "A", add: [pack("tslint", "web"), pack("zod", "")], configure: true });
    writes()[0]!.answer(200, {});
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    expect(writes()[1]!.body).toEqual({ repository: "A", remove: [pack("zod", "")] });
  });

  it("keeps the other recommended packs when a pack is added during the review, as from its panel", async () => {
    data.recommendations = recommended;
    page().keepPack("eslint", true, "ESLint kept on.");

    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]!.body).toEqual({ repository: "A", add: [pack("tslint", "web"), pack("zod", ""), pack("eslint", "")], configure: true });
    expect(keptPacks()).toEqual(["tslint", "zod", "eslint"]);
  });
});
