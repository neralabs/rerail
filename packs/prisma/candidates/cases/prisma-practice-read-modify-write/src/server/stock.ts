import { Prisma } from "@prisma/client";
import { db } from "@/lib/db";

// The row lock is taken in a helper (`SELECT … FOR UPDATE` in its body) before the read.
async function lockProductForUpdate(tx: Prisma.TransactionClient, id: string) {
  await tx.$queryRaw`SELECT id FROM "Product" WHERE id = ${id} FOR UPDATE`;
}

export async function sellLocked(id: string, qty: number) {
  return db.$transaction(async (tx) => {
    await lockProductForUpdate(tx, id);
    const p = await tx.product.findUniqueOrThrow({ where: { id } });
    await tx.product.update({ where: { id }, data: { stock: p.stock - qty } });
  });
}

// A lock taken after the write guards nothing.
export async function sellLockedLate(id: string, qty: number) {
  return db.$transaction(async (tx) => {
    const p = await tx.product.findUniqueOrThrow({ where: { id } });
    await tx.product.update({ where: { id }, data: { stock: p.stock - qty } }); // caught
    await lockProductForUpdate(tx, id);
  });
}

// A compare-and-set claim: the write matches only while the wallet still holds the code it clears.
export async function redeem(id: string, code: string, n: number) {
  const w = await db.wallet.findUniqueOrThrow({ where: { id } });
  await db.wallet.updateMany({ where: { id, armedCode: code }, data: { armedCode: null, credits: w.credits + n } });
}
