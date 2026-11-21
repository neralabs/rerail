// A start that finds a daemon still in its start waits for it. Before, set-up's dashboard step launched the daemon and
// gave up on it after its 15 s wait while it was still coming up; the `rerail daemon start` a script ran next probed it
// once for 2 s and exited 1, naming a daemon that does not answer. Two starts at once both exited 1 beside the daemon one
// of them launched. Each daemon here is held in its start, recorded and not answering, until every command says it
// waits for it or one has ended (`daemon-start-gate.mjs`).
import { describe, expect, test } from "vitest";
import { setupThenStart, StartingDaemonHome, twoStartsAtOnce, WAITS_BESIDE_STARTING } from "./support/starting-daemon.js";

describe("a start beside a daemon still in its start", () => {
  test("rerail setup --answer start, then rerail daemon start: set-up waits for the daemon it launched, and the start finds it running", async () => {
    const place = StartingDaemonHome.make();
    try {
      const seen = await setupThenStart(place);
      expect(seen.setup).toMatchObject({ code: 0, stderr: expect.stringMatching(WAITS_BESIDE_STARTING) });
      expect(seen.startBesideHeld).toBe(false);
      expect(seen.start).toMatchObject({ code: 0, stdout: expect.stringContaining("The Rerail daemon is already running.") });
      expect(seen.daemons).toBe(1);
    } finally {
      await place.end();
    }
  }, 120_000);

  test("two rerail daemon start at once: both wait for the daemon one of them launched and exit 0 with one daemon running", async () => {
    const place = StartingDaemonHome.make();
    try {
      const seen = await twoStartsAtOnce(place);
      for (const start of seen.starts) expect(start).toMatchObject({ code: 0, stderr: expect.stringMatching(WAITS_BESIDE_STARTING) });
      expect(seen.starts.filter(start => start.stdout.includes("The Rerail daemon is already running."))).toHaveLength(1);
      expect(seen.daemons).toBe(1);
    } finally {
      await place.end();
    }
  }, 120_000);
});
