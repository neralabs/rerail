import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/index.js";
import { readConflicts, writeConflicts, type StoredConflict } from "../../../src/runtime/merge-assist/conflict-store.js";
import { STATE_KEYS, writeState } from "../../../src/runtime/merge-assist/state.js";
import { sampleConflict } from "../../support/merge-assist/status.js";

const roots: string[] = [];
const databases: RerailDatabase[] = [];
afterEach(async () => {
  for (const database of databases.splice(0)) database.$client.close();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

async function database(): Promise<RerailDatabase> {
  const root = await mkdtemp(join(tmpdir(), "rerail-conflict-store-"));
  roots.push(root);
  const opened = openRerailDatabase(join(root, "rerail.sqlite"));
  databases.push(opened);
  return opened;
}

function stored(overrides: Parameters<typeof sampleConflict>[0] = {}): StoredConflict {
  return { conflict: sampleConflict(overrides), files: ["native/src/model.rs"], anchor: "0123456789abcdef01234567" };
}

const DISMISSED = { id: "conflict:overlap:fedcba9876543210fedcba98:01234567", status: "dismissed", dismissal: { name: "Tim Belanger", at: "2026-11-03T14:10:00Z" } } as const;

describe("Merge Assist's conflict store", () => {
  it("reads every conflict it can when one entry is not one this build reads", async () => {
    const db = await database();
    const open = stored();
    const dismissed = stored(DISMISSED);
    writeState(db, STATE_KEYS.conflicts("/work/app"), [open, { conflict: { id: "from another version" } }, dismissed], 1);
    expect(readConflicts(db, "/work/app").map(entry => entry.conflict.id)).toEqual([open.conflict.id, dismissed.conflict.id]);
  });

  it("keeps a dismissed conflict without its hunks and evidence, and an open one whole", async () => {
    const db = await database();
    const open = stored();
    const dismissed = stored(DISMISSED);
    writeConflicts(db, "/work/app", [open, dismissed], 1);
    const [readOpen, readDismissed] = readConflicts(db, "/work/app");
    expect(readOpen).toEqual(open);
    expect(readDismissed?.conflict).toMatchObject({ id: dismissed.conflict.id, status: "dismissed", dismissal: dismissed.conflict.dismissal, evidence: [], mine: { hunk: null }, theirs: { hunk: null } });
  });
});
