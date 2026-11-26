import { Prisma } from "@prisma/client";
import { prisma } from "@/lib/db";
import { sendReceipt } from "./mail";

type Line = { sku: string; qty: number };

// A helper that is handed the transaction client.
export async function reserveStock(tx: Prisma.TransactionClient, lines: Line[]) {
  for (const l of lines) {
    tx.reservation.create({ data: { sku: l.sku, qty: l.qty } }); // caught
  }
  return tx.stock.findMany({ where: { sku: { in: lines.map((l) => l.sku) } } });
}

// Everything below is awaited, returned or collected: none of it is reported.
export async function placeOrder(userId: string, lines: Line[]) {
  const order = await prisma.$transaction(async (tx) => {
    const created = await tx.order.create({ data: { userId } });
    for (const l of lines) {
      await tx.orderLine.create({ data: { orderId: created.id, sku: l.sku, qty: l.qty } });
    }
    await Promise.all(lines.map((l) => tx.stock.update({ where: { sku: l.sku }, data: { qty: { decrement: l.qty } } })));
    const writes: Promise<unknown>[] = [];
    lines.forEach((l) => writes.push(tx.reservation.delete({ where: { sku_userId: { sku: l.sku, userId } } })));
    await Promise.all(writes);
    const pending = tx.auditLog.create({ data: { action: "order", userId } });
    const skus: string[] = [];
    lines.forEach((l) => skus.push(l.sku));
    await pending;
    await tx.user.update({ where: { id: userId }, data: { lastOrderAt: new Date() } }).then(() => undefined);
    await reserveStock(tx, lines);
    return tx.order.findUniqueOrThrow({ where: { id: created.id }, include: { lines: true } });
  });
  lines.forEach(async (l) => {
    await prisma.stockAlert.deleteMany({ where: { sku: l.sku } });
  });
  setTimeout(() => sendReceipt(order.id), 0);
  return order;
}

export function settle(orderId: string) {
  return prisma.$transaction(async (tx) => Promise.all([
    tx.payment.updateMany({ where: { orderId }, data: { settled: true } }),
    tx.order.update({ where: { id: orderId }, data: { status: "SETTLED" } }),
  ]));
}

export function bulkClose(ids: string[]) {
  return prisma.$transaction(ids.map((id) => prisma.order.update({ where: { id }, data: { status: "CLOSED" } })));
}

export async function totals(lines: Line[]) {
  return prisma.$transaction(async (tx) => {
    const tally = new Map<string, number>();
    lines.forEach((l) => tally.set(l.sku, (tally.get(l.sku) ?? 0) + l.qty));
    const counts = lines.map(async (l) => tx.stock.count({ where: { sku: l.sku } }));
    return { tally, counts: await Promise.all(counts) };
  });
}
