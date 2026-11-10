import { randomBytes } from "node:crypto";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { runInNewContext } from "node:vm";
import { setFlagsFromString } from "node:v8";
import { describe, expect, it } from "vitest";
import { SqliteSessionStore } from "../../src/runtime/session-store.js";
import { removeFolder } from "../support/platform/remove-folder.js";

setFlagsFromString("--expose-gc");
const gc = runInNewContext("gc") as () => void;

/** Heap bytes in use once full collections have run and their sweeping has finished. */
async function heapInUse(): Promise<number> {
  for (let pass = 0; pass < 3; pass += 1) {
    gc();
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  return process.memoryUsage().heapUsed;
}

describe("the session store", () => {
  it("keeps nothing of a session in memory once it closes, since one daemon opens sessions for weeks", { timeout: 60_000 }, async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-session-store-memory-"));
    const store = new SqliteSessionStore(join(root, "rerail.sqlite"));
    try {
      // Each id a string of its own, 4 KB long: whatever the store keeps of a session holds at least its id.
      const openAndClose = (count: number): void => {
        for (let index = 0; index < count; index += 1) {
          const session = { host: "claude-code" as const, sessionId: randomBytes(2_048).toString("hex"), cwd: root };
          store.openSession(session);
          store.closeSession(session.sessionId);
        }
      };
      // The first sessions pay for the store's prepared statements and the compiled code.
      openAndClose(50);
      const before = await heapInUse();
      const count = 500;
      openAndClose(count);
      expect((await heapInUse() - before) / count).toBeLessThan(1_024);
    } finally {
      store.close();
      await removeFolder(root);
    }
  });
});
