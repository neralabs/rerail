/**
 * The Merge Assist page (`ui/views/merge.js`) rendered over the fake DOM, for its tests: a status for its checkout, and
 * a harness that gives each test a fresh document, a `fetch` that holds every request until the test answers it, and
 * the page scoped to checkout `/work/a` of repository `A` (storefront).
 */
import { afterEach, beforeEach, vi } from "vitest";
import type { MergeAssistStatus } from "../../../src/engine/index.js";
import { changeScope } from "../../../ui/poll.js";
import { data } from "../../../ui/state.js";
import { actions } from "../../../ui/views/merge.js";
import { sampleStatus } from "../../support/merge-assist/status.js";
import { FakeElement, FakeNode, fakeDocument, type FakeDocument } from "./fake-dom.js";

/** A request the page sent, held until the test answers it. */
export interface Request {
  method: string;
  url: string;
  body: Record<string, unknown> | undefined;
  answer: (status: number, body?: unknown) => void;
}

/**
 * Merge Assist on in checkout `/work/a` of acme/storefront: the agent CLI found, `gh` signed in, no agent call used,
 * scanned two minutes before noon, nothing open. `over` replaces whole fields.
 */
export const storefrontStatus = (over: Partial<MergeAssistStatus> = {}): MergeAssistStatus => sampleStatus("/work/a", {
  repositoryKey: "remote:github.com/acme/storefront",
  readiness: {
    judge: { available: true, runner: "claude", problem: null },
    github: { state: "signed-in", login: "tbelanger", repository: "acme/storefront" },
  },
  budget: { used: 0, limit: 40, windowMinutes: 60 },
  lastScan: { at: "2026-11-02T11:58:00Z", peers: { branches: 2, pullRequests: 1 }, problems: [] },
  ...over,
});

/**
 * The page under test; `root` is fresh for each test, as are the document and the requests behind the helpers. The
 * helpers close over the page, so a test may destructure them.
 */
export interface MergePage {
  /** What the page renders into: pass it to `render`. */
  root: FakeElement;
  /** The requests sent other than reads. */
  readonly writes: () => Request[];
  /** The first button with `data-act` `act` (and `data-i` `id`, when given), or `null`. */
  readonly button: (act: string, id?: string) => FakeElement | null;
  /** What the toast says. */
  readonly toastText: () => string;
  /** What a click on `el` runs, as `app.js` dispatches it; nothing for a disabled button. */
  readonly click: (el: FakeElement | null) => Promise<void> | void;
}

/**
 * Sets up the page for each test of the calling `describe`: call it there. Its `beforeEach` runs before the test's
 * own, which then sets `data.mergeAssist` and renders; its `afterEach` takes the stubs away.
 */
export function mergePage(): MergePage {
  let requests: Request[] = [];
  let document: FakeDocument = fakeDocument();
  const page: MergePage = {
    root: new FakeElement("div"),
    writes: () => requests.filter(r => r.method !== "GET"),
    button: (act, id) => page.root.querySelectorAll(`button[data-act="${act}"]`).find(b => id === undefined || b.dataset.i === id) ?? null,
    toastText: () => document.getElementById("toast")!.textContent,
    click: el => {
      if (el === null) throw new Error("no such button");
      if (el.disabled) return undefined;
      return actions[el.dataset.act!]!(el.dataset.i, el);
    },
  };

  beforeEach(() => {
    requests = [];
    document = fakeDocument();
    vi.stubGlobal("document", document);
    vi.stubGlobal("Node", FakeNode);
    vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
    vi.stubGlobal("fetch", (input: URL, init: { method?: string; body?: string } = {}) => {
      const { promise, resolve } = Promise.withResolvers<Response>();
      requests.push({
        method: init.method ?? "GET",
        url: String(input),
        body: init.body === undefined ? undefined : JSON.parse(init.body) as Record<string, unknown>,
        answer: (status, body) => resolve(new Response(body === undefined ? null : JSON.stringify(body), { status })),
      });
      return promise;
    });
    data.repository = null;
    changeScope({ repository: "A", checkout: "/work/a" });
    data.repository = { setUp: true, repositoryKey: "A", name: "storefront" };
    page.root = new FakeElement("div");
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  return page;
}
