import { afterEach, describe, expect, it, vi } from "vitest";
import { AttachRetry, type AttachRetryHost } from "../../src/harness/attach-retry.js";
import { AutostartOffError } from "../../src/runtime/index.js";

const { OtherDaemonKeptError } = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");

const NOT_RUNNING = "The Rerail daemon is not running, and RERAIL_AUTOSTART=0 keeps hooks from starting it, so nothing is recorded until it runs. Start it with rerail daemon start.";
const OTHER_KEPT = "The running Rerail daemon is from another Rerail package or an earlier release, and RERAIL_AUTOSTART=0 keeps hooks from replacing it, so nothing is recorded until a daemon from the active package runs. Run rerail daemon stop, then rerail daemon start.";

afterEach(() => {
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

/**
 * An OMP or Pi extension as the retry sees it: each try of `context`'s attach answers as `answer` says for that try
 * (counted from 1), and each line the host was asked to show is kept beside its context.
 */
function extension(answer: (context: string, attempt: number) => Promise<void>) {
  const tries: string[] = [];
  const said: Array<[string, string]> = [];
  const host: AttachRetryHost<string> = {
    initialize: async context => {
      tries.push(context);
      await answer(context, tries.length);
    },
    sessionId: context => context,
    appendEntry() {},
    reportDropped() {},
    failed() {},
    say: (context, line) => { said.push([context, line]); },
    recorder: () => undefined,
  };
  return { host, tries, said };
}

describe("an attach RERAIL_AUTOSTART=0 kept from a daemon it may use", () => {
  it.each([
    ["no daemon runs", new AutostartOffError(), NOT_RUNNING],
    ["another package's daemon runs", new OtherDaemonKeptError(), OTHER_KEPT],
  ])("where %s, says its line once and attaches once a later try finds this package's daemon", async (_where, error, line) => {
    vi.stubEnv("RERAIL_AUTOSTART", "0");
    vi.useFakeTimers();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const { host, tries, said } = extension(async (_context, attempt) => { if (attempt < 3) throw error; });
    const retry = new AttachRetry(host);

    await retry.attach("session");
    await vi.advanceTimersByTimeAsync(1_000);
    await vi.advanceTimersByTimeAsync(2_000);

    expect(tries).toEqual(["session", "session", "session"]);
    expect(said).toEqual([["session", line]]);
    expect(retry.failing()).toBeUndefined();
    expect(stderr).toHaveBeenCalledWith("Rerail attached after 2 failed attempt(s)\n");
  });
});
