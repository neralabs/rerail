import { db } from "../db/drizzle";

// The same call shape on a Drizzle `db`: not a Prisma read and write.
export async function sellOnDrizzle(id: string, qty: number) {
  const p = await db.product.findFirst({ where: { id } });
  await db.product.update({ where: { id }, data: { stock: p.stock - qty } });
}
