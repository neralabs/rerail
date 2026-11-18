import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

// The JS realpath counted, the one the key Rerail 0.1.4 recorded a folder under goes through; the native one, which
// every key of this build goes through, is passed on as it is.
const jsRealpaths = vi.hoisted(() => ({ calls: 0, inWrite: 0, writing: false }));
vi.mock("node:fs", async importOriginal => {
  const original = await importOriginal<typeof import("node:fs")>();
  const realpathSync = Object.assign((...parameters: Parameters<typeof original.realpathSync>) => {
    jsRealpaths.calls += 1;
    if (jsRealpaths.writing) jsRealpaths.inWrite += 1;
    return original.realpathSync(...parameters);
  }, { native: original.realpathSync.native });
  return { ...original, realpathSync };
});

import { checkoutStatus, openRerailDatabase, recordCheckout, type RerailDatabase } from "../../../src/engine/index.js";
import { createCheckoutResolver } from "../../../src/engine/catalog/repository-registry.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const cleanups: Array<() => unknown> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

/** A database in a new folder, and a folder outside Git there spelt as the disk stores it and as VS Code can (`c:\…`, lower case). */
async function plainFolder(name: string): Promise<{ database: RerailDatabase; stored: string; spelled: string }> {
  const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-earlier-key-")));
  cleanups.push(() => removeFolder(root));
  const database = openRerailDatabase(join(root, "rerail.sqlite"));
  cleanups.push(() => database.$client.close());
  const stored = join(root, name);
  mkdirSync(stored);
  const spelled = join(dirname(stored).replace(/^[A-Z]:/u, drive => drive.toLowerCase()), name.toLowerCase());
  return { database, stored, spelled };
}

describe.runIf(process.platform === "win32")("On Windows, the key Rerail 0.1.4 recorded a folder outside Git under", () => {
  it("is resolved once per spelling, not on each start check a daemon makes for a session's event", async () => {
    const { database, stored, spelled } = await plainFolder("Notes");
    recordCheckout(database, stored);
    const resolve = createCheckoutResolver(database);
    jsRealpaths.calls = 0;
    expect(checkoutStatus(database, resolve(spelled)).started).toBe(false);
    const first = jsRealpaths.calls;
    for (let check = 0; check < 5; check++) checkoutStatus(database, resolve(spelled));
    expect({ first, later: jsRealpaths.calls - first }).toEqual({ first: 1, later: 0 });
  });

  it("is resolved after recording the folder writes its row, not while the write holds the database", async () => {
    const { database, spelled } = await plainFolder("Drafts");
    const write = database.transaction.bind(database);
    vi.spyOn(database, "transaction").mockImplementation(((work, config) => write(tx => {
      jsRealpaths.writing = true;
      try {
        return work(tx);
      } finally {
        jsRealpaths.writing = false;
      }
    }, config)) as typeof database.transaction);
    jsRealpaths.inWrite = 0;
    expect(recordCheckout(database, spelled).earlierCheckoutPath).toBe(realpathSync(spelled));
    expect(jsRealpaths.inWrite).toBe(0);
  });
});
