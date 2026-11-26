import { prisma } from "@acme/db";
import { reserve, reserveWith } from "./stock";

export async function placeOrder(productId: string, qty: number) {
  await prisma.$transaction(async (tx) => {
    await tx.orderLine.create({ data: { productId, qty } });
    await reserve(productId, qty); // caught
    await reserveWith(tx, productId, qty);
  });
}
