import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { eq } from "drizzle-orm";
import { afterEach, describe, expect, it } from "vitest";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { mergeAssistEnabled, pruneCheckouts, recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { openRerailDatabase, resetRepositoryHistoryIn, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { mergeAssistState, repositories, sessions } from "../../../src/engine/storage/schema.js";
import { git } from "../../support/merge-assist/git.js";

const roots: string[] = [];
const databases: RerailDatabase[] = [];
afterEach(async () => {
  for (const database of databases.splice(0)) database.$client.close();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

async function folder(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-merge-assist-storage-"));
  roots.push(root);
  return root;
}

async function database(): Promise<RerailDatabase> {
  const opened = openRerailDatabase(join(await folder(), "rerail.sqlite"));
  databases.push(opened);
  return opened;
}

const keys = (db: RerailDatabase): string[] => db.select({ key: mergeAssistState.key }).from(mergeAssistState).all().map(row => row.key).sort();

const REPOSITORY = "remote:github.com/acme/web";

describe("Merge Assist storage", () => {
  it("is off for a repository until it is turned on, and off for a repository Rerail does not know", async () => {
    const db = await database();
    db.insert(repositories).values({ repositoryKey: REPOSITORY, displayName: "web", createdAt: 1, updatedAt: 1 }).run();
    expect(mergeAssistEnabled(db, REPOSITORY)).toBe(false);
    db.update(repositories).set({ mergeAssist: true }).where(eq(repositories.repositoryKey, REPOSITORY)).run();
    expect(mergeAssistEnabled(db, REPOSITORY)).toBe(true);
    expect(mergeAssistEnabled(db, "remote:github.com/acme/unknown")).toBe(false);
  });

  it("forgets what Merge Assist kept for a checkout Rerail forgets", async () => {
    const db = await database();
    const checkout = join(realpathSync(await folder()), "app");
    mkdirSync(checkout);
    git(checkout, "init", "-q", "-b", "main");
    recordCheckout(db, checkout);
    db.insert(mergeAssistState).values([`conflicts:${checkout}`, `scan:${checkout}`, `scanning:${checkout}`, `settings:${REPOSITORY}`]
      .map(key => ({ key, valueJson: "[]", updatedAt: 1 }))).run();
    await removeFolder(checkout);

    expect(pruneCheckouts(db)).toEqual([checkout]);
    expect(keys(db)).toEqual([`settings:${REPOSITORY}`]);
  });

  it("drops the record of notes handed to the sessions a repository reset deletes", async () => {
    const db = await database();
    db.insert(sessions).values([
      { sessionId: "ended", host: "omp", cwd: "/work/app", repositoryId: REPOSITORY, startedAt: 1, endedAt: 2 },
      { sessionId: "running", host: "omp", cwd: "/work/app", repositoryId: REPOSITORY, startedAt: 1 },
      { sessionId: "elsewhere", host: "omp", cwd: "/work/lib", repositoryId: "remote:github.com/acme/lib", startedAt: 1, endedAt: 2 },
    ]).run();
    db.insert(mergeAssistState).values(["ended", "running", "elsewhere"].map(session => ({ key: `delivered:${session}`, valueJson: "[]", updatedAt: 1 }))).run();

    resetRepositoryHistoryIn(db, [REPOSITORY]);
    expect(keys(db)).toEqual(["delivered:elsewhere", "delivered:running"]);
  });
});
