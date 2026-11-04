import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { changeScope } from "../../ui/poll.js";
import { removeRule, restoreRule } from "../../ui/rule-remove.js";
import { data } from "../../ui/state.js";
import type * as Poll from "../../ui/poll.js";

const { toast } = vi.hoisted(() => ({ toast: vi.fn() }));
vi.mock("../../ui/shell.js", () => ({ closePanel: () => {}, toast }));
// The page's poll is not under test: a refresh after a write finishes at once.
vi.mock("../../ui/poll.js", async importOriginal => ({ ...(await importOriginal<typeof Poll>()), refreshNow: async () => {} }));

type Request = { method: string; url: string; body: unknown; answer: (status: number, body?: unknown) => void };

const RULE = "lib:money-as-cents";

describe("Undo on a deleted rule", () => {
  let requests: Request[] = [];
  const writes = () => requests.filter(r => r.method === "PUT");

  beforeEach(() => {
    requests = [];
    toast.mockReset();
    vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
    vi.stubGlobal("fetch", (input: URL, init: { method?: string; body?: string } = {}) => {
      const { promise, resolve } = Promise.withResolvers<Response>();
      requests.push({
        method: init.method ?? "GET",
        url: String(input),
        body: init.body === undefined ? undefined : JSON.parse(init.body),
        answer: (status, body) => resolve(new Response(body === undefined ? null : JSON.stringify(body), { status })),
      });
      return promise;
    });
    changeScope({ repository: "A", checkout: "/work/a" });
    data.repository = { setUp: true, repositoryKey: "A", name: "storefront" };
    data.rules = new Map([[RULE, {
      origin: "user",
      kind: "code",
      rule: { id: RULE, clauses: [{ violation: { message: "Money as cents" } }] },
      folderStates: [{ folder: "", source: { kind: "library_rule", id: RULE } }],
    }]]);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("adds the rule back to the repository it was deleted from, after the page switched to another", async () => {
    const deleting = removeRule(RULE);
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.answer(200, {});
    await deleting;
    expect(toast).toHaveBeenLastCalledWith("Deleted from storefront.", { action: { label: "Undo", act: "ruleundo", id: RULE } });

    changeScope({ repository: "B", checkout: "/work/b" });
    const restoring = restoreRule(RULE);

    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    expect(writes()[1]).toMatchObject({
      url: expect.stringMatching(/\/api\/repository\/attachments$/u),
      body: { repository: "A", checkout: "/work/a", add: [{ sourceKind: "library_rule", sourceId: RULE, folder: "" }] },
    });
    writes()[1]!.answer(200, {});
    await restoring;
    expect(toast).toHaveBeenLastCalledWith("Restored to storefront.");
  });
});
