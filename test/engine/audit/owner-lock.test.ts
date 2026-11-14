import { randomUUID } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, unlinkSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { forgetFreedOwnerLocks, holdOwnerLock, ownerLockFreed } from "../../../src/engine/audit/owner-lock.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

// These answers decide whether a daemon's sessions are ended as no one's: a lock judged free while its service runs
// would end that service's live sessions, and one that never reads free would leave a dead one's open for good.

interface TestLock {
  release(): void;
}

/** Stands in for each lock the engine takes while set: given the lock file and the lock, it answers the lock taken. */
const taking = vi.hoisted(() => ({ lock: undefined as ((path: string, lock: TestLock) => TestLock) | undefined }));
vi.mock("../../../src/engine/native/native.js", async importOriginal => {
  const actual = await importOriginal<{ loadNativeBinding: () => { tryLockExclusive(path: string): TestLock | null } }>();
  return {
    ...actual,
    loadNativeBinding: () => {
      const binding = actual.loadNativeBinding();
      return {
        ...binding,
        tryLockExclusive: (path: string) => {
          const lock = binding.tryLockExclusive(path);
          return lock === null || taking.lock === undefined ? lock : taking.lock(path, lock);
        },
      };
    },
  };
});

const homes: string[] = [];
afterEach(async () => {
  taking.lock = undefined;
  await Promise.all(homes.splice(0).map(home => removeFolder(home)));
});

/** A database's path in a home of its own, and the folder its owner locks go in. */
async function home(): Promise<{ databasePath: string; owners: string }> {
  const root = await mkdtemp(join(tmpdir(), "rerail-owner-lock-"));
  homes.push(root);
  return { databasePath: join(root, "rerail.sqlite"), owners: join(root, "locks", "owners") };
}

describe("an engine service's owner lock", () => {
  it("removes the file of a free lock no open session names, and leaves a held one and a free one a session names", async () => {
    const { databasePath, owners } = await home();
    const held = holdOwnerLock(databasePath);
    const [unnamed, named] = [randomUUID(), randomUUID()];
    for (const id of [unnamed, named]) writeFileSync(join(owners, `${id}.lock`), "");
    writeFileSync(join(owners, "notes.txt"), "");
    const asked: string[] = [];
    forgetFreedOwnerLocks(databasePath, lockId => {
      asked.push(lockId);
      return lockId === named;
    });
    // Only a free lock is asked about, and only under it.
    expect(asked.sort()).toEqual([unnamed, named].sort());
    expect(readdirSync(owners).sort()).toEqual([`${held.id}.lock`, `${named}.lock`, "notes.txt"].sort());
    // Once the service lets its lock go and no session names either, both go; the lock kept for the named one was let go.
    held.release();
    forgetFreedOwnerLocks(databasePath, () => false);
    expect(readdirSync(owners)).toEqual(["notes.txt"]);
  });

  it("holds no lock when another start removed its file before it locked it, and lets go of the lock it took", async () => {
    const { databasePath, owners } = await home();
    let released = 0;
    taking.lock = (path, lock) => {
      unlinkSync(path);
      return { release: () => { released += 1; lock.release(); } };
    };
    const lock = holdOwnerLock(databasePath);
    taking.lock = undefined;
    expect(lock.id).toBeUndefined();
    expect(released).toBe(1);
    expect(readdirSync(owners)).toEqual([]);
  });

  it("reads a lock this process holds as held, one let go as freed whenever asked, and one with no file as held", async () => {
    const { databasePath, owners } = await home();
    const held = holdOwnerLock(databasePath);
    const id = held.id!;
    expect(ownerLockFreed(databasePath, id)).toBe(false);
    held.release();
    // Its file stays for a sweep to judge the rows that name it; asking takes the lock and lets it go again.
    expect(existsSync(join(owners, `${id}.lock`))).toBe(true);
    expect([ownerLockFreed(databasePath, id), ownerLockFreed(databasePath, id)]).toEqual([true, true]);
    const unknown = randomUUID();
    expect(ownerLockFreed(databasePath, unknown)).toBe(false);
    expect(existsSync(join(owners, `${unknown}.lock`)), "a file made by asking").toBe(false);
  });

  it("holds none where the owners' folder cannot be made", async () => {
    const { databasePath, owners } = await home();
    mkdirSync(join(owners, ".."), { recursive: true });
    writeFileSync(owners, "");
    expect(holdOwnerLock(databasePath).id).toBeUndefined();
  });
});
