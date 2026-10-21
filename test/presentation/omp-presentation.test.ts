import type { ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { afterEach, expect, it, vi } from "vitest";
import { createOmpPresentation } from "../../src/presentation/omp-presentation.js";
import { readPresentation, type SessionPresentation } from "../../src/presentation/session-presentation.js";

vi.mock("../../src/presentation/session-presentation.js", async () => ({
  readPresentation: vi.fn(),
  formatTerminalStatus: (presentation: SessionPresentation) => presentation.text,
  notOnHerePresentation: (await vi.importActual<typeof import("../../src/presentation/session-presentation.js")>("../../src/presentation/session-presentation.js")).notOnHerePresentation,
}));
afterEach(() => vi.clearAllMocks());

it("never repaints a previous session or a closed widget after an outstanding read completes", async () => {
  const first = Promise.withResolvers<SessionPresentation>();
  const second = Promise.withResolvers<SessionPresentation>();
  const third = Promise.withResolvers<SessionPresentation>();
  vi.mocked(readPresentation).mockReturnValueOnce(first.promise)
    .mockReturnValueOnce(second.promise).mockReturnValueOnce(third.promise);
  let visible: string[] | undefined;
  let tick: (() => void) | undefined;
  const context = (id: string) => ({
    cwd: "/workspace", hasUI: true,
    sessionManager: { getSessionId: () => id },
    ui: { setWidget: (_key: string, lines: string[] | undefined) => { visible = lines; } },
    setInterval: (callback: () => void) => { tick = callback; return 1; },
    clearTimer: () => { tick = undefined; },
  }) as unknown as ExtensionContext;
  const status = (text: string): SessionPresentation => ({ status: "on", text, parts: [], url: null, presence: "compact", repositoryState: "started", counts: null });
  const hud = createOmpPresentation();
  hud.start(context("old"));
  hud.start(context("current"));
  second.resolve(status("Current session"));
  await second.promise;
  expect(visible).toEqual(["Current session"]);
  first.resolve(status("Old session"));
  await first.promise;
  expect(visible).toEqual(["Current session"]);
  tick?.();
  hud.stop();
  third.resolve(status("Closed session"));
  await third.promise;
  expect(visible).toBeUndefined();
});

it("says Rerail is not on here, without asking a daemon, where the host knows it is off", () => {
  let visible: string[] | undefined;
  let polling = false;
  const context = {
    cwd: "/workspace", hasUI: true,
    sessionManager: { getSessionId: () => "off" },
    ui: { setWidget: (_key: string, lines: string[] | undefined) => { visible = lines; } },
    setInterval: () => { polling = true; return 1; },
    clearTimer: () => undefined,
  } as unknown as ExtensionContext;
  createOmpPresentation().start(context, { off: true });
  expect(visible).toEqual([expect.stringContaining("not on here")]);
  expect(readPresentation).not.toHaveBeenCalled();
  expect(polling).toBe(false);
});

it("says session events are being dropped while the daemon's status still reads, and clears it when they stop", async () => {
  const read = Promise.withResolvers<SessionPresentation>();
  vi.mocked(readPresentation).mockReturnValueOnce(read.promise);
  let visible: string[] | undefined;
  const context = {
    cwd: "/workspace", hasUI: true,
    sessionManager: { getSessionId: () => "dropping" },
    ui: { setWidget: (_key: string, lines: string[] | undefined) => { visible = lines; } },
    setInterval: () => 1,
    clearTimer: () => undefined,
  } as unknown as ExtensionContext;
  const hud = createOmpPresentation();
  hud.start(context);
  hud.dropped(1);
  expect(visible).toEqual(["Rerail: 1 session event dropped — daemon not answering"]);
  // `/healthz` may answer while the session's own requests do not: both lines show.
  read.resolve({ status: "on", text: "Rerail ● 3 rules", parts: [], url: null, presence: "compact", repositoryState: "started", counts: null });
  await read.promise;
  hud.dropped(4);
  expect(visible).toEqual(["Rerail ● 3 rules", "Rerail: 4 session events dropped — daemon not answering"]);
  hud.dropped(0);
  expect(visible).toEqual(["Rerail ● 3 rules"]);
  hud.stop();
});

it("keeps a silent footer silent while session events are being dropped", async () => {
  vi.mocked(readPresentation).mockResolvedValueOnce({ status: "on", text: "Rerail ● 3 rules", parts: [], url: null, presence: "silent", repositoryState: "started", counts: null });
  let visible: string[] | undefined;
  let painted = false;
  const context = {
    cwd: "/workspace", hasUI: true,
    sessionManager: { getSessionId: () => "silent" },
    ui: { setWidget: (_key: string, lines: string[] | undefined) => { visible = lines; painted = true; } },
    setInterval: () => 1,
    clearTimer: () => undefined,
  } as unknown as ExtensionContext;
  const hud = createOmpPresentation();
  hud.start(context);
  await vi.waitFor(() => expect(painted).toBe(true));
  hud.dropped(2);
  expect(visible).toBeUndefined();
  hud.stop();
});
