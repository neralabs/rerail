"use server";
import { prisma } from "@/lib/db";

export async function ordersWithItems(userId: string) {
  const orders = await prisma.order.findMany({ where: { userId } });
  for (const o of orders) {
    o.items = await prisma.orderItem.findMany({ where: { orderId: o.id } }); // caught
  }
  return orders;
}

export async function itemCounts(orderIds: string[]) {
  return Promise.all(orderIds.map((orderId) => prisma.orderItem.count({ where: { orderId } }))); // caught
}

export async function ownersOf(ids: string[]) {
  // findUnique calls made in one tick are batched into one query by Prisma's dataloader.
  return Promise.all(ids.map((id) => prisma.user.findUnique({ where: { id } })));
}

export async function ownerOfEach(ids: string[]) {
  const owners = [];
  for (const id of ids) {
    owners.push(await prisma.user.findUniqueOrThrow({ where: { id } })); // caught
  }
  return owners;
}

export async function addLines(orderId: string, lines: { sku: string; qty: number }[]) {
  for (const { sku, qty } of lines) {
    await prisma.orderItem.create({ data: { orderId, sku, qty } }); // caught
  }
}

export async function settingsFor(keys: string[]) {
  const out: Record<string, unknown> = {};
  for (const k of keys) {
    // the query ignores the item: one query per iteration, but not the n+1 this rule reports
    out[k] = await prisma.setting.findFirst({ where: { key: "defaults" } });
  }
  return out;
}

export async function archiveStatuses() {
  for (const status of ["CANCELLED", "REFUNDED"] as const) {
    await prisma.order.updateMany({ where: { status }, data: { archived: true } });
  }
}
