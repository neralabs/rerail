import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { refreshNow, setNeeds, startPolling } from "../../ui/poll.js";
import { data, setScope, ui } from "../../ui/state.js";
import { currentNeeds } from "../../ui/views/index.js";
import type { MergeAssistStatus } from "../../src/engine/index.js";
import { sampleStatus } from "../support/merge-assist/status.js";

/** The daemon's change stream as the test writes it: `send` one message, `end` it as a stopping daemon does. */
interface Stream { send: (data: string, event?: string) => void; end: () => void; signal: AbortSignal }

let streams: Stream[] = [];
/** The reads the page made, by path, beside the stream. */
let reads: string[] = [];
/** Whether the daemon answers the page's reads; a stopped one refuses the connection. */
let up = true;
/** Whether the daemon serves the change stream; one of an older Rerail answers 404. */
let streams404 = false;
/** Reads the daemon answers with an error (500), by path, while the daemon itself stays up. */
let failing = new Set<string>();
type GithubState = MergeAssistStatus["readiness"]["github"]["state"];
/** Whether the Merge Assist status the daemon answers says a scan runs, and gh's state in it. */
let scanning = false;
let githubState: GithubState = "signed-in";
/**
 * When the daemon read gh's sign-in, on its own clock, which runs `daemonAheadMs` ahead of the page's; none where it
 * ran no gh. A status past five minutes from it reads gh again, which then says `githubNext`.
 */
let githubReading: { at: number } | undefined;
let githubNext: GithubState = "signed-in";
let daemonAheadMs = 0;
let renders: boolean[] = [];
let visibility: (() => void)[] = [];
const encoder = new TextEncoder();

const json = (body: unknown, status = 200): Response => new Response(JSON.stringify(body), { status });

/** gh's readiness as the daemon's status says it: a reading reused for five minutes, then read again. */
function githubShown(): MergeAssistStatus["readiness"]["github"] {
  const shown = { state: githubState, login: null, repository: "acme/app" };
  if (githubReading === undefined) return shown;
  const daemonNow = Date.now() + daemonAheadMs;
  if (daemonNow - githubReading.at >= 5 * 60_000) {
    githubReading = { at: daemonNow };
    githubState = githubNext;
  }
  return { ...shown, state: githubState, readAgainInMs: githubReading.at + 5 * 60_000 - daemonNow };
}

/** Every read a page makes, each empty; Merge Assist with one checkout's status. */
function answer(url: URL): Response {
  const bodies: Record<string, () => unknown> = {
    "/healthz": () => ({ version: "1", pid: 1 }),
    "/api/repositories": () => ({ repositories: [] }),
    "/api/repository": () => ({ repository: { setUp: true, repositoryKey: "A" }, checkout: null }),
    "/api/repository/rules": () => ({ rules: [], unavailable: [] }),
    "/api/rule-packs": () => ({ packs: [] }),
    "/api/merge-assist": () => sampleStatus("/work/a", { scanning, readiness: { github: githubShown() } }),
  };
  const body = bodies[url.pathname];
  if (body === undefined) throw new Error(`unexpected ${url.pathname}`);
  return json(body());
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
/**
 * Lets the page take what the test sent: the stream's bytes, the reads they start and their answers. Each is a chain
 * of promises, and a turn of the event loop runs every promise ready before it; no clock is read or waited on.
 */
const settle = async (): Promise<void> => {
  for (let turn = 0; turn < 20; turn += 1) await new Promise(resolve => setImmediate(resolve));
};
/** Moves the page's clock on by `ms`, then lets what that started run. */
const after = async (ms: number): Promise<void> => {
  await vi.advanceTimersByTimeAsync(ms);
  await settle();
};
const readsOf = (path: string): number => reads.filter(p => p === path).length;

function setHidden(hidden: boolean): void {
  vi.stubGlobal("document", { hidden, addEventListener: (_: string, listener: () => void) => visibility.push(listener) });
  for (const listener of visibility) listener();
}

beforeEach(async () => {
  // Minutes from the next ten-minute step of a sliding activity window, which a beat would read at.
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance", "Date"], now: new Date(2026, 10, 18, 10, 1, 0) });
  streams = [];
  reads = [];
  up = true;
  streams404 = false;
  failing = new Set();
  scanning = false;
  githubState = "signed-in";
  githubReading = undefined;
  githubNext = "signed-in";
  daemonAheadMs = 0;
  renders = [];
  vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
  vi.stubGlobal("fetch", async (input: URL, init: { signal?: AbortSignal } = {}) => {
    const url = new URL(input);
    if (!up) throw new TypeError("fetch failed");
    if (url.pathname === "/api/changes") return streams404 ? json({ error: "not-found" }, 404) : openStream(init.signal!);
    reads.push(url.pathname);
    return failing.has(url.pathname) ? json({ error: "internal" }, 500) : answer(url);
  });
  setScope({ repository: "A", checkout: "/work/a" });
  Object.assign(data, { mergeAssist: null, mergeAssistError: null, mergeAssistExperiment: null, disconnected: false, loaded: false, behindSince: 0 });
  ui.view = "settings";
  ui.range = 7;
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
  it("reads at the stream's first message, then nothing for a minute of beats with nothing new", async () => {
    expect(readsOf("/api/repositories")).toBe(0);

    stream().send("t1");
    await settle();

    expect(readsOf("/api/repositories")).toBe(1);
    expect(data.loaded).toBe(true);
    const readsBefore = reads.length;
    for (let beat = 0; beat < 12; beat += 1) {
      await after(5_000);
      stream().send("t1");
      await settle();
    }
    expect(reads.length).toBe(readsBefore);
    // Only the watch for two missed beats waits; nothing that reads.
    expect(vi.getTimerCount()).toBe(1);
    expect(streams).toHaveLength(1);
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

    await after(5_000);
    expect(readsOf("/api/repositories")).toBe(3);
    // The paced read went out; only the watch for two missed beats waits.
    expect(vi.getTimerCount()).toBe(1);
  });

  it("renders at a beat that brings nothing new and reads nothing, nor moves the poll's time views read more after", async () => {
    stream().send("t1");
    await settle();
    const before = reads.length;
    const polledAt = data.updatedAt;
    renders = [];
    await after(1_000);

    stream().send("t1");
    await settle();

    expect(reads.length).toBe(before);
    expect(renders).toEqual([false]);
    // Settings reads every repository's rule count after each poll, keyed on this time: a beat must not start that.
    expect(data.updatedAt).toBe(polledAt);
    expect(data.heardAt).toBeGreaterThan(polledAt);
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
    await settle();
    expect(streams).toHaveLength(2);
    const before = readsOf("/api/repositories");
    stream().send("t9");
    await settle();

    expect(readsOf("/api/repositories")).toBe(before + 1);
    expect(data.disconnected).toBe(false);
  });

  it("opens the stream again after the pause though the read after its end never answers", async () => {
    stream().send("t1");
    await settle();
    // The daemon takes connections but answers no read, as when its event loop is stuck.
    const unanswered: (() => void)[] = [];
    vi.stubGlobal("fetch", async (input: URL, init: { signal?: AbortSignal } = {}) => {
      const url = new URL(input);
      if (url.pathname === "/api/changes") return openStream(init.signal!);
      reads.push(url.pathname);
      return new Promise<Response>(resolve => unanswered.push(() => resolve(answer(url))));
    });
    stream().end();
    await settle();

    await after(5_000);

    expect(streams).toHaveLength(2);
    // The reads answer at last, so the next test's reads are not held behind them.
    for (const reply of unanswered) reply();
    await settle();
  });

  it("reads the sidebar's count again at a new stream's first message, since a conflict event may have come while none was open", async () => {
    stream().send("t1");
    await settle();
    stream().send("t2");
    await settle();
    expect(readsOf("/api/merge-assist")).toBe(1);

    setHidden(true);
    setHidden(false);
    await settle();
    stream().send("t2");
    await settle();

    expect(readsOf("/api/merge-assist")).toBe(2);
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

  it("opens the stream again once two beats are missed, as when the daemon's loop is stuck or a network dropped it", async () => {
    stream().send("t1");
    await settle();

    await after(12_000);

    expect(streams).toHaveLength(2);
    expect(streams[0]!.signal.aborted).toBe(true);
  });

  it("reads at the pace once a stream that never brought a message misses two beats, as through a proxy that buffers it, until one comes", async () => {
    await after(12_000);
    expect(streams).toHaveLength(2);
    expect(readsOf("/api/repositories")).toBe(1);

    // Main's pace from then, while a new stream is tried every two beats: 10 reads in the first minute.
    for (let second = 12; second < 57; second += 5) await after(5_000);
    expect(readsOf("/api/repositories")).toBe(10);
    expect(streams).toHaveLength(5);

    stream().send("t1");
    await settle();
    const followed = readsOf("/api/repositories");
    for (let beat = 0; beat < 4; beat += 1) {
      await after(5_000);
      stream().send("t1");
      await settle();
    }
    expect(readsOf("/api/repositories")).toBe(followed);
  });

  it("reads nothing at the pace when a stream that already brought a message misses two beats", async () => {
    stream().send("t1");
    await settle();

    await after(12_000);
    await after(12_000);

    expect(streams).toHaveLength(3);
    expect(readsOf("/api/repositories")).toBe(1);
  });

  it("reads every 5 s as before when the daemon serves no stream, as an older Rerail does", async () => {
    setHidden(true);
    streams404 = true;
    setHidden(false);
    await settle();
    const before = readsOf("/api/repositories");

    await after(5_000);
    await after(5_000);

    expect(readsOf("/api/repositories")).toBe(before + 2);
  });

  it("reads every 5 s as before in a tab past the browser's stream slots, so other tabs' requests keep a connection", async () => {
    setHidden(true);
    vi.stubGlobal("navigator", { locks: { request: async (_name: string, _options: object, held: (lock: null) => Promise<boolean>) => held(null) } });
    const opened = streams.length;
    setHidden(false);
    await settle();
    const before = readsOf("/api/repositories");

    await after(5_000);

    expect(streams).toHaveLength(opened);
    expect(readsOf("/api/repositories")).toBe(before + 1);
  });

  it("reads the sidebar's Merge Assist count at a conflict event, and not at every change", async () => {
    stream().send("t1");
    await settle();
    // The count waits for the first read, which draws the page.
    stream().send("t2");
    await settle();
    expect(readsOf("/api/merge-assist")).toBe(1);

    await after(5_000);
    stream().send("t3");
    await settle();
    expect(readsOf("/api/merge-assist")).toBe(1);

    await after(5_000);
    stream().send("t3", "conflicts");
    await settle();
    expect(readsOf("/api/merge-assist")).toBe(2);
  });

  it("reads again at the poll's pace after a read fails while the stream stays open, as the poll read again", async () => {
    stream().send("t1");
    await settle();
    failing.add("/api/repositories");
    stream().send("t2");
    await settle();
    expect(data.disconnected).toBe(true);
    failing.clear();
    // The daemon beats with nothing new, so no message asks for a read: the read that failed is tried again.
    stream().send("t2");
    await settle();
    const before = readsOf("/api/repositories");

    // The pace doubled at the failure, from 5 s to 10 s, as the poll's did.
    await after(10_000);

    expect(readsOf("/api/repositories")).toBe(before + 1);
    expect(data.disconnected).toBe(false);
  });

  it("holds no stream without Web Locks, as on a page that is not a secure context, and reads every 5 s as before", async () => {
    setHidden(true);
    vi.stubGlobal("navigator", {});
    const opened = streams.length;
    setHidden(false);
    await settle();
    const before = readsOf("/api/repositories");

    await after(5_000);
    await after(5_000);

    expect(streams).toHaveLength(opened);
    expect(readsOf("/api/repositories")).toBe(before + 2);
  });

  it("counts the page behind from a new token until a read begun after it answers, whatever the beats say", async () => {
    stream().send("t1");
    await settle();
    expect(data.behindSince).toBe(0);
    const unanswered: (() => void)[] = [];
    vi.stubGlobal("fetch", async (input: URL, init: { signal?: AbortSignal } = {}) => {
      const url = new URL(input);
      if (url.pathname === "/api/changes") return openStream(init.signal!);
      reads.push(url.pathname);
      return new Promise<Response>(resolve => unanswered.push(() => resolve(answer(url))));
    });
    await after(5_000);
    stream().send("t2");
    await settle();
    const behind = data.behindSince;
    expect(behind).toBeGreaterThan(0);

    await after(5_000);
    stream().send("t2");
    await settle();
    expect(data.behindSince).toBe(behind);

    for (const reply of unanswered) reply();
    await settle();
    expect(data.behindSince).toBe(0);
  });

  it("counts the page behind from a read the person asks for until it answers, whatever the beats say", async () => {
    stream().send("t1");
    await settle();
    const unanswered: (() => void)[] = [];
    vi.stubGlobal("fetch", async (input: URL, init: { signal?: AbortSignal } = {}) => {
      const url = new URL(input);
      if (url.pathname === "/api/changes") return openStream(init.signal!);
      reads.push(url.pathname);
      return new Promise<Response>(resolve => unanswered.push(() => resolve(answer(url))));
    });
    await after(1_000);
    const asked = Date.now();
    void refreshNow();
    await settle();

    for (let beat = 0; beat < 3; beat += 1) {
      await after(5_000);
      stream().send("t1");
      await settle();
    }
    expect(data.readingSince).toBe(asked);

    // Answered, it no longer holds the page behind, and the next case's reads join no read of this one.
    for (const reply of unanswered) reply();
    await settle();
    expect(data.readingSince).toBe(0);
  });

  it("reads at the first beat past the activity window's own boundary, as Today's at local midnight", async () => {
    vi.setSystemTime(new Date(2026, 10, 18, 23, 59, 57));
    ui.range = 0;
    stream().send("t1");
    await settle();
    const before = readsOf("/api/repositories");

    await after(2_000);
    stream().send("t1");
    await settle();
    expect(readsOf("/api/repositories")).toBe(before);

    await after(2_000);
    stream().send("t1");
    await settle();
    expect(readsOf("/api/repositories")).toBe(before + 1);
  });

  it("reads at each beat while the Merge Assist page shows a scan running, whose process can end unheard, and not after", async () => {
    ui.view = "merge";
    scanning = true;
    stream().send("t1");
    await settle();
    expect(readsOf("/api/merge-assist")).toBe(1);

    await after(5_000);
    scanning = false;
    stream().send("t1");
    await settle();
    expect(readsOf("/api/merge-assist")).toBe(2);

    await after(5_000);
    stream().send("t1");
    await settle();
    expect(readsOf("/api/merge-assist")).toBe(2);
  });

  it("reads at each beat while the Merge Assist page shows gh signed out, and nothing while Merge Assist is off and runs no gh", async () => {
    ui.view = "merge";
    githubState = "signed-out";
    stream().send("t1");
    await settle();
    await after(5_000);
    stream().send("t1");
    await settle();
    expect(readsOf("/api/merge-assist")).toBe(2);

    githubState = "unchecked";
    await after(5_000);
    stream().send("t1");
    await settle();
    await after(5_000);
    stream().send("t1");
    await settle();

    expect(readsOf("/api/merge-assist")).toBe(3);
  });

  it("asks for the Merge Assist status once when the daemon reads gh's sign-in again, so a sign-out shows, though the browser's clock is 10 minutes behind the daemon's", async () => {
    ui.view = "merge";
    daemonAheadMs = 10 * 60_000;
    githubReading = { at: Date.now() + daemonAheadMs - 60_000 };
    stream().send("t1");
    await settle();
    expect(readsOf("/api/merge-assist")).toBe(1);
    // gh signs out meanwhile; the daemon reads it again for a status past its five minutes, 4 minutes from now.
    githubNext = "signed-out";

    for (let beat = 0; beat < 47; beat += 1) {
      await after(5_000);
      stream().send("t1");
      await settle();
    }
    await after(4_999);
    expect(readsOf("/api/merge-assist")).toBe(1);
    await after(2);

    expect(readsOf("/api/merge-assist")).toBe(2);
    expect(data.mergeAssist?.readiness.github.state).toBe("signed-out");
  });

  it("wakes for gh's sign-in on no page but Merge Assist's, though the sidebar's count holds a reading", async () => {
    githubReading = { at: Date.now() };
    stream().send("t1");
    await settle();
    // The count waits for the first read, which draws the page.
    stream().send("t2");
    await settle();
    expect(data.mergeAssist?.readiness.github.readAgainInMs).toBe(5 * 60_000);
    // Only the watch for two missed beats waits: no wake is set for the reading on this page.
    expect(vi.getTimerCount()).toBe(1);
    const before = reads.length;

    // Past the reading's five minutes, the stream beating with nothing new.
    for (let beat = 0; beat < 61; beat += 1) {
      await after(5_000);
      stream().send("t2");
      await settle();
    }

    expect(reads.length).toBe(before);
  });
});
