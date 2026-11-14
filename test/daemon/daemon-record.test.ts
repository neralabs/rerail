import { existsSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { afterEach, expect, it, vi } from "vitest";
import { RerailDaemon } from "../../src/daemon/daemon.js";
import { DAEMON_STATE_FILE } from "../../src/runtime/index.js";
import { removeFolder } from "../support/platform/remove-folder.js";

/** What the next file replace waits for before it writes, once; a test sets it. */
const gate = vi.hoisted(() => ({ next: undefined as (() => Promise<void>) | undefined }));

vi.mock("../../src/engine/file-replace.js", async () => {
  const real = await vi.importActual<typeof import("../../src/engine/file-replace.js")>("../../src/engine/file-replace.js");
  return {
    ...real,
    replaceFile: async (...args: Parameters<typeof real.replaceFile>): Promise<void> => {
      const wait = gate.next;
      gate.next = undefined;
      await wait?.();
      await real.replaceFile(...args);
    },
  };
});

const originalRerailHome = process.env.RERAIL_HOME;
const homes: string[] = [];

afterEach(async () => {
  gate.next = undefined;
  if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = originalRerailHome;
  await Promise.all(homes.splice(0).map(async home => await removeFolder(home)));
});

it("lets a write of daemon.json its record check has in flight land before its close removes the file, never after", async () => {
  const home = await mkdtemp(join(tmpdir(), "rerail-record-"));
  homes.push(home);
  process.env.RERAIL_HOME = home;
  const stateFile = join(home, DAEMON_STATE_FILE);
  const daemon = await RerailDaemon.start({
    version: "0.0.0-test",
    databasePath: join(home, "rerail.sqlite"),
    workspaceRoot: home,
    port: 0,
    stateFile,
    recordCheckMs: 20,
    nativeHealth: () => ({ ok: true, kernelVersion: "1" }),
  });
  const started = Promise.withResolvers<void>();
  const release = Promise.withResolvers<void>();
  // The check's next write of daemon.json waits for `release`, as a write held up by a slow disk or a scanner does.
  gate.next = async () => {
    started.resolve();
    await release.promise;
  };
  await rm(stateFile);
  await started.promise;
  const closing = daemon.close();
  const closedBeforeTheWrite = await Promise.race([closing.then(() => true), delay(1_000).then(() => false)]);
  release.resolve();
  await closing;
  expect(closedBeforeTheWrite).toBe(false);
  // Landing after the close, the write would leave a daemon.json that names a daemon that is gone.
  expect(existsSync(stateFile)).toBe(false);
}, 20_000);
