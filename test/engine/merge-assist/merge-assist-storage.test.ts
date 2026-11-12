import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { eq } from "drizzle-orm";
import { afterEach, describe, expect, it } from "vitest";
import { mergeAssistEnabled } from "../../../src/engine/catalog/repository-registry.js";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { repositories } from "../../../src/engine/storage/schema.js";

const roots: string[] = [];
const databases: RerailDatabase[] = [];
afterEach(async () => {
  for (const database of databases.splice(0)) database.$client.close();
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

async function database(): Promise<RerailDatabase> {
  const root = await mkdtemp(join(tmpdir(), "rerail-merge-assist-storage-"));
  roots.push(root);
  const opened = openRerailDatabase(join(root, "rerail.sqlite"));
  databases.push(opened);
  return opened;
}

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
});
