// A start that finds a daemon still in its start waits for it. Before, set-up's dashboard step launched the daemon and
// gave up on it after its 15 s wait while it was still coming up; the `rerail daemon start` a script ran next probed it
// once for 2 s and exited 1, naming a daemon that does not answer. Two starts at once both exited 1 beside the daemon one
// of them launched. Each daemon here is held in its start, recorded and not answering, until every command says it
// waits for it or one has ended (`daemon-start-gate.mjs`).
import { spawn } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, test } from "vitest";
import { setupThenStart, StartingDaemonHome, twoStartsAtOnce, WAITS_BESIDE_STARTING } from "./support/starting-daemon.js";

/** What `rerail status` says once it removed a daemon.json that named a live process other than the home's daemon. */
const NOT_HOME_DAEMON = "The process daemon.json named is not a Rerail daemon of this home, so Rerail ended nothing and removed that record.";

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

  // `rerail status` named a daemon still in its start as one that does not answer, and told the person to stop it.
  test("rerail status beside a daemon still in its start says it is starting, and names no stop", async () => {
    const place = StartingDaemonHome.make();
    try {
      const start = place.run(["daemon", "start"]);
      const held = await place.held();
      const status = await place.run(["status"]).done;
      place.release();
      expect(status).toMatchObject({ code: 4, stdout: expect.stringContaining(`Rerail daemon (pid ${held}) is still starting and does not answer yet.`) });
      expect(status.stdout).not.toContain("rerail daemon stop");
      expect(await start.done).toMatchObject({ code: 0 });
    } finally {
      await place.end();
    }
  }, 120_000);

  // Beside a daemon still in its start, `rerail status` removed a daemon.json that named another program, and did not
  // say so. Not on Windows: there the lookup spends its 2 s on the engine's proof over the held daemon's pipe, which
  // its blocked thread never answers, so no start time is read in time and the record is kept, as for a busy daemon.
  test.skipIf(process.platform === "win32")("rerail status beside a daemon still in its start says it removed a daemon.json that named another program", async () => {
    const place = StartingDaemonHome.make();
    const other = spawn(process.execPath, ["-e", "setInterval(() => {}, 1e6)"], { stdio: "ignore" });
    try {
      const start = place.run(["daemon", "start"]);
      const held = await place.held();
      // daemon.json as a killed daemon leaves it, written an hour before the program that has its pid now started.
      const record = join(place.home, "daemon.json");
      const state = JSON.parse(readFileSync(record, "utf8")) as { startedAt: number };
      writeFileSync(record, JSON.stringify({ ...state, pid: other.pid, startedAt: state.startedAt - 3_600_000 }));
      const status = await place.run(["status"]).done;
      place.release();
      expect(status).toMatchObject({ code: 4, stdout: expect.stringContaining(`${NOT_HOME_DAEMON}\nRerail daemon (pid ${held}) is still starting and does not answer yet.`) });
      expect(await start.done).toMatchObject({ code: 0 });
    } finally {
      other.kill();
      await place.end();
    }
  }, 120_000);
});
