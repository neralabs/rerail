import { Prisma } from "@prisma/client";

// Takes the row lock for the caller's transaction: the helper's body runs `SELECT … FOR UPDATE`.
export async function holdProduct(tx: Prisma.TransactionClient, id: string) {
  await tx.$queryRaw`SELECT id FROM "Product" WHERE id = ${id} FOR UPDATE`;
}
