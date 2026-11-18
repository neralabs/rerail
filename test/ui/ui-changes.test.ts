import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setNeeds, startPolling } from "../../ui/poll.js";
import { data, setScope, ui } from "../../ui/state.js";
import { currentNeeds } from "../../ui/views/index.js";
import { sampleStatus } from "../support/merge-assist/status.js";

/** The daemon's change stream as the test writes it: `send` one message, `end` it as a stopping daemon does. */
interface Stream { send: (data: string, event?: string) => void; end: () => void; signal: AbortSignal }

let streams: Stream[] = [];
/** The reads the page made, by path, beside the stream. */
let reads: string[] = [];
/** Whether the daemon answers the page's reads; a stopped one refuses the connection. */
let up = true;
let renders: boolean[] = [];
let visibility: (() => void)[] = [];
const encoder = new TextEncoder();

const json = (body: unknown, status = 200): Response => new Response(JSON.stringify(body), { status });

/** Every read a page makes, each empty; Merge Assist with one checkout's status. */
function answer(url: URL): Response {
  const bodies: Record<string, unknown> = {
    "/healthz": { version: "1", pid: 1 },
    "/api/repositories": { repositories: [] },
    "/api/repository": { repository: { setUp: true, repositoryKey: "A" }, checkout: null },
    "/api/repository/rules": { rules: [], unavailable: [] },
    "/api/rule-packs": { packs: [] },
    "/api/merge-assist": sampleStatus("/work/a"),
  };
  const body = bodies[url.pathname];
  if (body === undefined) throw new Error(`unexpected ${url.pathname}`);
  return json(body);
}

function openStream(signal: AbortSignal): Response {
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  const body = new ReadableStream<Uint8Array>({ start: c => { controller = c; } });
  streams.push({
    send: (token, event) => controller.enqueue(encoder.encode(`${event === undefined ? "" : `event: ${event}\n`}data: ${token}\n\n`)),
    end: () => controller.close(),
    signal,
  });
  signal.addEventListener("abort", () => controller.error(signal.reason));
  return new Response(body, { status: 200, headers: { "content-type": "text/event-stream" } });
}

const stream = (): Stream => streams.at(-1)!;
/** Lets the page take what the test sent: the stream's bytes, the reads they start and their answers. */
const settle = async (): Promise<void> => { await vi.advanceTimersByTimeAsync(0); };
const readsOf = (path: string): number => reads.filter(p => p === path).length;

function setHidden(hidden: boolean): void {
  vi.stubGlobal("document", { hidden, addEventListener: (_: string, listener: () => void) => visibility.push(listener) });
  for (const listener of visibility) listener();
}

beforeEach(async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance", "Date"] });
  streams = [];
  reads = [];
  up = true;
  renders = [];
  vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
  vi.stubGlobal("fetch", async (input: URL, init: { signal?: AbortSignal } = {}) => {
    const url = new URL(input);
    if (!up) throw new TypeError("fetch failed");
    if (url.pathname === "/api/changes") return openStream(init.signal!);
    reads.push(url.pathname);
    return answer(url);
  });
  setScope({ repository: "A", checkout: "/work/a" });
  Object.assign(data, { mergeAssist: null, mergeAssistError: null, mergeAssistExperiment: null, disconnected: false });
  ui.view = "settings";
  ui.panel = null;
  setNeeds(currentNeeds);
  setHidden(false);
  startPolling(changed => renders.push(changed));
  await settle();
});

afterEach(() => {
  // A hidden tab ends its stream and every timer the page holds, so no test sees the last one's.
  setHidden(true);
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("the dashboard following the daemon's change stream", () => {
  it("reads at the stream's first message, then sends nothing and holds no timer while no message comes", async () => {
    expect(readsOf("/api/repositories")).toBe(0);

    stream().send("t1");
    await settle();

    expect(readsOf("/api/repositories")).toBe(1);
    expect(data.loaded).toBe(true);
    const after = reads.length;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(reads.length).toBe(after);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("reads a new token at once after a quiet spell, and at most every 5 s while changes keep coming", async () => {
    stream().send("t1");
    await settle();
    stream().send("t2");
    await settle();
    expect(readsOf("/api/repositories")).toBe(2);

    stream().send("t3");
    stream().send("t4");
    await settle();
    expect(readsOf("/api/repositories")).toBe(2);

    await vi.advanceTimersByTimeAsync(5_000);
    expect(readsOf("/api/repositories")).toBe(3);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("renders at a beat that brings nothing new, and reads nothing", async () => {
    stream().send("t1");
    await settle();
    const before = reads.length;
    renders = [];

    stream().send("t1");
    await settle();

    expect(reads.length).toBe(before);
    expect(renders).toEqual([false]);
  });

  it("reads once when the stream ends, and a daemon gone shows as disconnected in that render", async () => {
    stream().send("t1");
    await settle();
    let disconnectedWhenRendered: boolean | undefined;
    renders = [];
    const listened = renders.push.bind(renders);
    renders.push = (changed: boolean) => { disconnectedWhenRendered = data.disconnected; return listened(changed); };

    up = false;
    stream().end();
    await settle();

    expect(renders).toHaveLength(1);
    expect(disconnectedWhenRendered).toBe(true);
    // The pause before the stream is opened again.
    expect(vi.getTimerCount()).toBe(1);
  });

  it("opens the stream again after the pause once the daemon answers, and reads at its first message", async () => {
    stream().send("t1");
    await settle();
    up = false;
    stream().end();
    await settle();

    up = true;
    await vi.advanceTimersToNextTimerAsync();
    expect(streams).toHaveLength(2);
    const before = readsOf("/api/repositories");
    stream().send("t9");
    await settle();

    expect(readsOf("/api/repositories")).toBe(before + 1);
    expect(data.disconnected).toBe(false);
  });

  it("ends the stream while the tab is hidden, and follows a new one when it shows again", async () => {
    stream().send("t1");
    await settle();

    setHidden(true);
    expect(stream().signal.aborted).toBe(true);
    expect(vi.getTimerCount()).toBe(0);

    setHidden(false);
    await settle();
    expect(streams).toHaveLength(2);
  });

  it("reads the sidebar's Merge Assist count at a conflict event, and not at every change", async () => {
    stream().send("t1");
    await settle();
    // The count waits for the first read, which draws the page.
    stream().send("t2");
    await settle();
    expect(readsOf("/api/merge-assist")).toBe(1);

    await vi.advanceTimersByTimeAsync(5_000);
    stream().send("t3");
    await settle();
    expect(readsOf("/api/merge-assist")).toBe(1);

    await vi.advanceTimersByTimeAsync(5_000);
    stream().send("t3", "conflicts");
    await settle();
    expect(readsOf("/api/merge-assist")).toBe(2);
  });
});
