import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

// The JS realpath counted, the one the key Rerail 0.1.4 recorded a folder under goes through, and each lstat; the
// native realpath, which every key of this build goes through, is passed on as it is. With `failing`, the JS realpath
// throws, standing for a realpath that throws for any reason, while the native one still resolves the folder.
const jsRealpaths = vi.hoisted(() => ({ calls: 0, inWrite: 0, writing: false, lstats: 0, failing: false }));
vi.mock("node:fs", async importOriginal => {
  const original = await importOriginal<typeof import("node:fs")>();
  const realpathSync = Object.assign((...parameters: Parameters<typeof original.realpathSync>) => {
    jsRealpaths.calls += 1;
    if (jsRealpaths.writing) jsRealpaths.inWrite += 1;
    if (jsRealpaths.failing) throw Object.assign(new Error("EISDIR: illegal operation on a directory, realpath"), { code: "EISDIR" });
    return original.realpathSync(...parameters);
  }, { native: original.realpathSync.native });
  const lstatSync = ((...parameters: Parameters<typeof original.lstatSync>) => {
    jsRealpaths.lstats += 1;
    return original.lstatSync(...parameters);
  }) as typeof original.lstatSync;
  return { ...original, realpathSync, lstatSync };
});

import { execFileSync } from "node:child_process";
import { auditSettings, checkoutStatus, openRerailDatabase, recordCheckout, type RerailDatabase } from "../../../src/engine/index.js";
import { SqliteAuditControlStore } from "../../../src/engine/audit/audit-control.js";
import { createCheckoutResolver, EARLIER_KEYS_HELD, spelledCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { SqliteSessionStore } from "../../../src/runtime/index.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const cleanups: Array<() => unknown> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

/** A database in a new folder, and a folder outside Git there spelt as the disk stores it and as VS Code can (`c:\…`, lower case). */
async function plainFolder(name: string): Promise<{ database: RerailDatabase; databasePath: string; stored: string; spelled: string }> {
  const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-earlier-key-")));
  cleanups.push(() => removeFolder(root));
  const databasePath = join(root, "rerail.sqlite");
  const database = openRerailDatabase(databasePath);
  cleanups.push(() => database.$client.close());
  const stored = join(root, name);
  mkdirSync(stored);
  const spelled = join(dirname(stored).replace(/^[A-Z]:/u, drive => drive.toLowerCase()), name.toLowerCase());
  return { database, databasePath, stored, spelled };
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

  it("is held for at most EARLIER_KEYS_HELD spellings, the first added dropped first and resolved again once when asked about", async () => {
    const { database, stored } = await plainFolder("Notebooks");
    const checkout = recordCheckout(database, stored);
    // The folder's name in each mix of letter cases but the one the disk stores: one folder, as many spellings.
    const name = "notebooks";
    const spellings = Array.from({ length: 2 ** name.length }, (_, mix) => join(dirname(stored), [...name].map((letter, at) => (mix >> at) & 1 ? letter.toUpperCase() : letter).join("")))
      .filter(spelling => spelling !== stored).slice(0, EARLIER_KEYS_HELD + 1);
    expect(spellings).toHaveLength(EARLIER_KEYS_HELD + 1);
    const [first, last] = [spellings[0]!, spellings.at(-1)!];
    const [firstKey, lastKey] = [realpathSync(first), realpathSync(last)];
    for (const spelling of spellings) spelledCheckout(checkout, spelling);
    jsRealpaths.calls = 0;
    expect(spelledCheckout(checkout, last)?.earlierCheckoutPath).toBe(lastKey);
    const latest = jsRealpaths.calls;
    expect(spelledCheckout(checkout, first)?.earlierCheckoutPath).toBe(firstKey);
    expect({ latest, first: jsRealpaths.calls - latest }).toEqual({ latest: 0, first: 1 });
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

  it("is left out where the JS realpath throws on the folder, so recording and looking it up answer as without that key", async () => {
    const { database, stored, spelled } = await plainFolder("Scratch");
    jsRealpaths.failing = true;
    try {
      const recorded = recordCheckout(database, spelled);
      expect(recorded).toMatchObject({ checkoutPath: stored, git: false });
      expect(recorded).not.toHaveProperty("earlierCheckoutPath");
      const found = createCheckoutResolver(database)(spelled);
      expect(found).toEqual(recorded);
      expect(checkoutStatus(database, found).started).toBe(false);
    } finally {
      jsRealpaths.failing = false;
    }
  });

  it("is looked for in a Git checkout's audit settings once per spelling, not on each read", async () => {
    const { database, databasePath, stored, spelled } = await plainFolder("Factory");
    execFileSync("git", ["init", "-q", stored]);
    const { repositoryKey } = recordCheckout(database, spelled);
    database.insert(auditSettings).values({ workspaceRoot: realpathSync(spelled), settings: { scope: "ai-output", mode: "log" }, updatedAt: 1 }).run();
    const sessions = new SqliteSessionStore(databasePath);
    cleanups.push(() => sessions.close());
    sessions.openSession({ host: "claude-code", sessionId: "spelled", cwd: spelled }, 1, repositoryKey);
    const controls = new SqliteAuditControlStore(databasePath);
    cleanups.push(() => controls.close());
    expect(controls.getSettings("spelled").settings.mode).toBe("log");
    jsRealpaths.lstats = 0;
    for (let read = 0; read < 5; read++) expect(controls.getSettings("spelled").settings.mode).toBe("log");
    expect(jsRealpaths.lstats).toBe(0);
  });
});
