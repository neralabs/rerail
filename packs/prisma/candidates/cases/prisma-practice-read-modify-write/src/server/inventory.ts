import { db } from "@/lib/db";
import { holdProduct } from "./locks";

// The transaction's client goes to a helper (another file) that locks the row: the read and write are serialised.
export async function restock(id: string, qty: number) {
  return db.$transaction(async (tx) => {
    await holdProduct(tx, id);
    const p = await tx.product.findUniqueOrThrow({ where: { id } });
    await tx.product.update({ where: { id }, data: { stock: p.stock + qty } });
  });
}

// Calls named for a lock that do not lock this row: a user flag, an in-process mutex.
export async function sellAsUser(userId: string, id: string, qty: number) {
  await lockUser(userId);
  await mutex.lock();
  const p = await db.product.findUniqueOrThrow({ where: { id } });
  await db.product.update({ where: { id }, data: { stock: p.stock - qty } }); // caught
}

declare function lockUser(userId: string): Promise<void>;
declare const mutex: { lock(): Promise<void> };
