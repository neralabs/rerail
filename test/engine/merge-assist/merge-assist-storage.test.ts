import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { eq } from "drizzle-orm";
import { afterEach, describe, expect, it } from "vitest";
import { mergeAssistEnabled } from "../../../src/engine/catalog/repository-registry.js";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { mergeAssistItems, repositories, type NewMergeAssistItemRow } from "../../../src/engine/storage/schema.js";

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

function item(overrides: Partial<NewMergeAssistItemRow> = {}): NewMergeAssistItemRow {
  return {
    id: "item-1",
    checkoutPath: "/work/web",
    repositoryKey: REPOSITORY,
    candidateId: "candidate:retries-3f9a1c",
    candidateHash: "hash",
    origin: "peer",
    peerKind: "branch",
    peerRef: "jbarnes/session-retry",
    status: "open",
    tier: "review",
    candidate: {
      format: "rerail.candidate/v1",
      id: "candidate:retries-3f9a1c",
      decision: "Retries live in the transport layer.",
      generality: "repository",
      scope: ["src/**"],
      author: "jbarnes",
      createdAt: "2026-11-03T14:02:11Z",
      state: "active",
      supersedes: null,
    },
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  };
}

describe("Merge Assist storage", () => {
  it("is off for a repository until it is turned on, and off for a repository Rerail does not know", async () => {
    const db = await database();
    db.insert(repositories).values({ repositoryKey: REPOSITORY, displayName: "web", createdAt: 1, updatedAt: 1 }).run();
    expect(mergeAssistEnabled(db, REPOSITORY)).toBe(false);
    db.update(repositories).set({ mergeAssist: true }).where(eq(repositories.repositoryKey, REPOSITORY)).run();
    expect(mergeAssistEnabled(db, REPOSITORY)).toBe(true);
    expect(mergeAssistEnabled(db, "remote:github.com/acme/unknown")).toBe(false);
  });

  it("keeps one item per checkout, origin, peer and candidate, refuses values outside its enums, and goes with its repository", async () => {
    const db = await database();
    db.insert(repositories).values({ repositoryKey: REPOSITORY, displayName: "web", createdAt: 1, updatedAt: 1 }).run();
    db.insert(mergeAssistItems).values(item()).run();
    expect(db.select().from(mergeAssistItems).get()).toMatchObject({ evidence: {}, trusted: false, candidate: { id: "candidate:retries-3f9a1c" } });

    expect(() => db.insert(mergeAssistItems).values(item({ id: "item-2" })).run()).toThrow(/UNIQUE/u);
    db.insert(mergeAssistItems).values(item({ id: "own", origin: "own", peerKind: null, peerRef: "", tier: null, status: "rejected" })).run();
    expect(() => db.insert(mergeAssistItems).values(item({ id: "item-3", peerRef: "x", status: "closed" as "open" })).run()).toThrow(/CHECK/u);
    expect(() => db.insert(mergeAssistItems).values(item({ id: "item-4", peerRef: "y", tier: "merge" as "talk" })).run()).toThrow(/CHECK/u);

    db.delete(repositories).where(eq(repositories.repositoryKey, REPOSITORY)).run();
    expect(db.select().from(mergeAssistItems).all()).toEqual([]);
  });
});
