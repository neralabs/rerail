import type { ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { afterEach, expect, it, vi } from "vitest";
import { createOmpPresentation } from "../src/omp-presentation.js";
import { readPresentation, type SessionPresentation } from "../src/session-presentation.js";

vi.mock("../src/session-presentation.js", () => ({
  readPresentation: vi.fn(),
  formatTerminalStatus: (presentation: SessionPresentation) => presentation.text,
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
  const status = (text: string): SessionPresentation => ({ text, parts: [], url: null, presence: "compact" });
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
