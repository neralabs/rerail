import type { ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { afterEach, expect, it, vi } from "vitest";
import { createOmpPresentation } from "../src/omp-presentation.js";
import { readPresentation, type SessionPresentation } from "../src/session-presentation.js";

vi.mock("../src/session-presentation.js", async () => ({
  readPresentation: vi.fn(),
  formatTerminalStatus: (presentation: SessionPresentation) => presentation.text,
  notOnHerePresentation: (await vi.importActual<typeof import("../src/session-presentation.js")>("../src/session-presentation.js")).notOnHerePresentation,
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
  const status = (text: string): SessionPresentation => ({ text, parts: [], url: null, presence: "compact", repositoryState: "started" });
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
