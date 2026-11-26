import { prisma } from "@acme/db";

const LOCK_KEY = 4242;

// The transaction is only a mutex across replicas; the updates stay on the outer client so that no row lock is
// held while a plugin's hook runs on other connections.
export async function upgradePlugins() {
  await prisma.$transaction(async (tx) => {
    const [{ locked }] = await tx.$queryRaw<{ locked: boolean }[]>`SELECT pg_try_advisory_xact_lock(${LOCK_KEY}) AS locked`;
    if (!locked) return;
    const rows = await tx.installedPlugin.findMany();
    for (const row of rows) {
      await prisma.installedPlugin.update({ where: { id: row.id }, data: { version: row.latest } });
    }
  });
}

// A session-level advisory lock is the same mutex.
export async function reindexPlugins() {
  await prisma.$transaction(async (tx) => {
    await tx.$executeRaw`SELECT pg_advisory_lock(${LOCK_KEY})`;
    const rows = await tx.installedPlugin.findMany();
    for (const row of rows) {
      await prisma.installedPlugin.update({ where: { id: row.id }, data: { indexedAt: new Date() } });
    }
    await tx.$executeRaw`SELECT pg_advisory_unlock(${LOCK_KEY})`;
  });
}

// A row lock is no mutex: the outer client's write waits on the row the transaction locked.
export async function bumpLocked(id: string) {
  await prisma.$transaction(async (tx) => {
    await tx.$queryRaw`SELECT id FROM "InstalledPlugin" WHERE id = ${id} FOR UPDATE`;
    await prisma.installedPlugin.update({ where: { id }, data: { indexedAt: new Date() } }); // caught
  });
}
