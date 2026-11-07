import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { changeScope, saveFollowGitignore } from "../../ui/poll.js";
import { data } from "../../ui/state.js";

type Request = { method: string; url: string; body: unknown; answer: (status: number, body?: unknown) => void };

describe("the Follow .gitignore setting on the page", () => {
  let requests: Request[] = [];
  const writes = () => requests.filter(r => r.method === "PUT");
  const toast = vi.fn();

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
    data.repository = null;
    changeScope({ repository: "A", checkout: "/work/a" });
    data.followGitignore = true;
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("shows the change at once and writes it for the repository the page shows", async () => {
    const saving = saveFollowGitignore(false, { toast, invalidate: () => {} });

    expect(data.followGitignore).toBe(false);
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]).toMatchObject({
      url: expect.stringMatching(/\/api\/repository\/follow-gitignore$/u),
      body: { repository: "A", checkout: "/work/a", followGitignore: false },
    });
    writes()[0]!.answer(200, { repository: { followGitignore: false } });
    await saving;
    expect(data.followGitignore).toBe(false);
    expect(toast).toHaveBeenCalledWith("Files Git ignores are checked in sessions started from now on.");
  });

  it("puts the setting back and says why when the write fails", async () => {
    const saving = saveFollowGitignore(false, { toast, invalidate: () => {} });
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.answer(403, { error: "agent-not-permitted", message: "not yours" });
    await saving;

    expect(data.followGitignore).toBe(true);
    expect(toast).toHaveBeenCalledWith(expect.stringMatching(/^Could not save: /u));
  });

  it("leaves another repository's page alone when the answer comes after a switch", async () => {
    const saving = saveFollowGitignore(false, { toast, invalidate: () => {} });
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    changeScope({ repository: "B" });
    writes()[0]!.answer(500, { error: "disk full" });
    await saving;

    expect(data.followGitignore).toBeNull();
    expect(toast).not.toHaveBeenCalled();
  });
});
