import { describe, expect, it } from "vitest";
import { daemonExit } from "../../src/daemon/daemon-exit.js";

/** A daemon process whose close waits for `finish`, recording what its way out does; `fails` names a step that throws. */
function process_(fails?: "release" | "relaunch") {
  const events: string[] = [];
  let finish: (failure?: Error) => void = () => undefined;
  const step = (name: "release" | "relaunch") => () => {
    events.push(name);
    if (fails === name) throw new Error(`${name} refused`);
  };
  const exit = daemonExit({
    close: async () => {
      events.push("close");
      await new Promise<void>((resolve, reject) => {
        finish = failure => (failure === undefined ? resolve() : reject(failure));
      });
    },
    release: step("release"),
    relaunch: step("relaunch"),
    exit: code => events.push(`exit ${code}`),
    log: line => events.push(line),
  });
  return { events, exit, finish: (failure?: Error) => finish(failure) };
}

const settled = async (): Promise<void> => await new Promise(resolve => setImmediate(resolve));

describe("the daemon's way out", () => {
  it("stops, or restarts through the next daemon, with one close", async () => {
    const stopped = process_();
    stopped.exit.stop();
    stopped.finish();
    await settled();
    expect(stopped.events).toEqual(["close", "release", "exit 0"]);

    const restarted = process_();
    restarted.exit.restart();
    restarted.finish();
    await settled();
    expect(restarted.events).toEqual(["close", "release", "relaunch", "exit 0"]);
  });

  it("starts no next daemon once a stop is under way, and closes once when a stop comes during a restart", async () => {
    // `rerail daemon stop` while the heap ceiling restarts the daemon, or the other way round.
    const stopFirst = process_();
    stopFirst.exit.stop();
    stopFirst.exit.restart();
    stopFirst.finish();
    await settled();
    expect(stopFirst.events).toEqual(["close", "release", "exit 0"]);

    const restartFirst = process_();
    restartFirst.exit.restart();
    restartFirst.exit.stop();
    restartFirst.finish();
    await settled();
    expect(restartFirst.events).toEqual(["close", "release", "relaunch", "exit 0"]);
  });

  it("says why and exits 1 when the close fails, and starts nothing", async () => {
    const failed = process_();
    failed.exit.restart();
    failed.finish(new Error("socket busy"));
    await settled();
    expect(failed.events).toEqual(["close", "rerail daemon: restart failed: socket busy\n", "exit 1"]);
  });

  // The main-instance lock's release and the next daemon's launch are native calls that can throw after a clean close.
  it("says why and exits 1 when the release or the relaunch throws after the close", async () => {
    const unreleased = process_("release");
    unreleased.exit.stop();
    unreleased.finish();
    await settled();
    expect(unreleased.events).toEqual(["close", "release", "rerail daemon: shutdown failed: release refused\n", "exit 1"]);

    const unlaunched = process_("relaunch");
    unlaunched.exit.restart();
    unlaunched.finish();
    await settled();
    expect(unlaunched.events).toEqual(["close", "release", "relaunch", "rerail daemon: restart failed: relaunch refused\n", "exit 1"]);
  });
});
