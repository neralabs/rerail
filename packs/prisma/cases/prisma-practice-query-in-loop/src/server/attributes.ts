import type { Prisma } from "@prisma/client";
import { prisma } from "@/lib/db";

type Row = { id: string; version: number; body: string; key: string };

// Compare-and-set: each write holds only if the row still has the version read, and moves it on.
export async function bumpAll(rows: Row[]) {
  for (const row of rows) {
    const res = await prisma.doc.updateMany({
      where: { id: row.id, version: row.version },
      data: { body: row.body.trim(), version: row.version + 1 },
    });
    if (res.count === 0) console.warn("changed meanwhile", row.id);
  }
  for (const { id, version } of rows) {
    await prisma.doc.updateMany({ where: { id, version }, data: { version: version + 1 } });
  }
}

// The where pins the version, but the write leaves it as it is: no lock to keep.
export async function retitle(rows: Row[]) {
  for (const row of rows) {
    await prisma.doc.updateMany({ where: { id: row.id, version: row.version }, data: { body: row.body } }); // caught
  }
}

// One lock order for every transaction: sorted first, then written one at a time.
export async function revive(tx: Prisma.TransactionClient, rows: Row[]) {
  rows.sort((a, b) => a.key.localeCompare(b.key));
  for (const row of rows) {
    await tx.tag.update({ where: { id: row.id }, data: { deleted: false } });
  }
  for (const row of rows.toSorted((a, b) => a.key.localeCompare(b.key))) {
    await tx.tag.update({ where: { id: row.id }, data: { deleted: false } });
  }
}

export async function reviveInTransaction(rows: Row[]) {
  const ordered = rows.toSorted((a, b) => a.key.localeCompare(b.key));
  await prisma.$transaction(async (t) => {
    for (const row of ordered) await t.tag.update({ where: { id: row.id }, data: { deleted: false } });
  });
}

// Sorted, but no transaction holds the locks: one write per row.
export async function reviveSorted(rows: Row[]) {
  rows.sort((a, b) => a.key.localeCompare(b.key));
  for (const row of rows) {
    await prisma.tag.update({ where: { id: row.id }, data: { deleted: false } }); // caught
  }
}

// In a transaction, but in the caller's order.
export async function reviveUnsorted(tx: Prisma.TransactionClient, rows: Row[]) {
  for (const row of rows) {
    await tx.tag.update({ where: { id: row.id }, data: { deleted: false } }); // caught
  }
}

// Each field is checked, and the whole save abandoned on a bad one, before its update; the create comes first.
export async function claimFields(tx: Prisma.TransactionClient, fields: { local: string; remote: string }[]) {
  const existing = new Map((await tx.attribute.findMany()).map((a) => [a.code, a]));
  for (const field of fields) {
    const current = existing.get(field.local);
    if (!current) {
      await tx.attribute.create({ data: { code: field.local, sourceId: field.remote } }); // caught
      continue;
    }
    if (current.owner !== "internal") throw new Error(`${field.local} is owned elsewhere`);
    await tx.attribute.update({ where: { id: current.id }, data: { sourceId: field.remote } });
  }
}

// A throw guards the update in its own branch; the delete in the other branch is not behind it.
export async function moveOrDrop(tx: Prisma.TransactionClient, lines: { id: string; to: string | null; kind: string }[]) {
  for (const line of lines) {
    if (line.kind === "move") {
      if (!line.to) throw new Error("no target");
      await tx.line.update({ where: { id: line.id }, data: { location: line.to } });
    } else {
      await tx.line.delete({ where: { id: line.id } }); // caught
    }
  }
}

type Field = { local: string; remote: string };

// Checked first, but no transaction holds the writes, or the data is the same for every row, or a create:
// `$transaction([...])`, `updateMany` or `createMany` fits.
export async function checkedNoTransaction(fields: Field[]) {
  for (const field of fields) {
    if (!field.remote) throw new Error(`${field.local} is not mapped`);
    await prisma.attribute.update({ where: { code: field.local }, data: { sourceId: field.remote } }); // caught
  }
}

export async function checkedSameData(tx: Prisma.TransactionClient, fields: Field[]) {
  for (const field of fields) {
    if (!field.remote) throw new Error(`${field.local} is not mapped`);
    await tx.attribute.update({ where: { code: field.local }, data: { deleted: false } }); // caught
    await tx.attributeLog.create({ data: { code: field.local, remote: field.remote } }); // caught
  }
}

// An error passed on from a catch is no check of the item.
export async function pushEach(tx: Prisma.TransactionClient, fields: Field[]) {
  for (const field of fields) {
    try {
      await pushRemote(field.remote);
    } catch (e) {
      throw e;
    }
    await tx.attribute.update({ where: { code: field.local }, data: { syncedAs: field.remote } }); // caught
  }
}

declare function pushRemote(remote: string): Promise<void>;

// A compare-and-set on a JSON column: the write holds only if the column still `equals` the content read.
export async function claimNotifications(trackers: { id: string; notes: unknown }[]) {
  for (const tracker of trackers) {
    await prisma.tracker.updateMany({
      where: { id: tracker.id, notes: { equals: tracker.notes as object } },
      data: { notes: [] },
    });
  }
}

// The item's previous value under another name, and data that depends on the outcome: still a compare-and-set.
type Pending = { id: string; previousDataId: string; dataId: string; retry: boolean };
export async function commitAll(tx: Prisma.TransactionClient, items: Pending[]) {
  for (const { id, previousDataId, dataId, retry } of items) {
    await tx.item.updateMany({
      where: { id, dataId: previousDataId },
      data: retry ? { dataId, retries: 1 } : { dataId: null },
    });
  }
}

// The old value is the only item value in the where: rows looked up by it, not a lock on the item's row.
export async function remapAll(tx: Prisma.TransactionClient, envelopeId: string, pairs: { oldId: string; newId: string }[]) {
  for (const { oldId, newId } of pairs) {
    await tx.envelopeItem.updateMany({ where: { envelopeId, dataId: oldId }, data: { dataId: newId } }); // caught
  }
}
