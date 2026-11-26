"use server";

import { after } from "next/server";
import { prisma } from "@/lib/db";
import { audit } from "@/lib/audit";

// A callback an array method runs at once is part of the transaction callback: reported.
export async function payItems(orderId: string, ids: string[]) {
  await prisma.$transaction(async (tx) => {
    await tx.order.update({ where: { id: orderId }, data: { status: "PAID" } });
    await Promise.all(ids.map((id) => prisma.lineItem.update({ where: { id }, data: { paid: true } }))); // caught
    ids.forEach((id) => audit("paid", id)); // caught
  });
}

// Functions the callback only defines run after the commit: left alone (on `tx` they would fail with P2028).
export async function payLater(orderId: string) {
  await prisma.$transaction(async (tx) => {
    await tx.order.update({ where: { id: orderId }, data: { status: "PAID" } });
    after(async () => {
      await prisma.auditLog.create({ data: { action: "paid", orderId } });
      await audit("paid", orderId);
    });
  });
}

export async function payOnCommit(orderId: string) {
  const onCommit: Array<() => Promise<unknown>> = [];
  await prisma.$transaction(async (tx) => {
    await tx.order.update({ where: { id: orderId }, data: { status: "PAID" } });
    onCommit.push(() => prisma.auditLog.create({ data: { action: "paid", orderId } }));
  });
  await Promise.all(onCommit.map((f) => f()));
}

export async function loadOrder(orderId: string) {
  return prisma.$transaction(async (tx) => {
    const order = await tx.order.findFirst({ where: { id: orderId } });
    return { order, countAll: () => prisma.order.count() };
  });
}
