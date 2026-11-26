"use server";

import { revalidatePath } from "next/cache";
import { prisma } from "@/lib/db";

type Line = { sku: string; qty: number };

export async function checkout(cartId: string) {
  await prisma.$transaction(async (tx) => {
    const cart = await tx.cart.findUniqueOrThrow({ where: { id: cartId }, include: { items: true } });
    cart.items.forEach(async (i: Line) => { // caught
      await tx.stock.update({ where: { sku: i.sku }, data: { qty: { decrement: i.qty } } });
    });
    tx.cart.delete({ where: { id: cartId } }); // caught
  });
  revalidatePath("/cart");
}

export async function restock(lines: Line[]) {
  return prisma.$transaction(async (tx) => {
    lines.forEach((l) => tx.stock.update({ where: { sku: l.sku }, data: { qty: { increment: l.qty } } })); // caught
    Promise.all(lines.map((l) => tx.restockLog.create({ data: { sku: l.sku, qty: l.qty } }))); // caught
    lines.map(async (l) => { // caught
      await tx.supplierOrder.updateMany({ where: { sku: l.sku, open: true }, data: { open: false } });
    });
    void tx.auditLog.create({ data: { action: "restock" } }); // caught
    return lines.length;
  });
}

export async function archiveOrder(orderId: string) {
  await prisma.$transaction(async (tx) => {
    await tx.order.update({ where: { id: orderId }, data: { archived: true } });
    tx.$executeRaw`UPDATE "Invoice" SET "archived" = true WHERE "orderId" = ${orderId}`; // caught
    tx.orderEvent.create({ data: { orderId, kind: "archived" } }).then(() => console.log("archived")); // caught
    setTimeout(() => tx.searchIndex.deleteMany({ where: { orderId } }), 0); // caught
  });
}

export function cancelOrder(orderId: string) {
  return prisma.$transaction(function (tx) {
    tx.order.update({ where: { id: orderId }, data: { status: "CANCELLED" } }); // caught
    return tx.payment.updateMany({ where: { orderId }, data: { refundable: false } });
  });
}
