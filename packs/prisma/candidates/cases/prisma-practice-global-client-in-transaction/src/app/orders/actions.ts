"use server";

import { prisma } from "@/lib/db";
import { analytics } from "@/lib/analytics-db";
import { audit, auditWith, formatAction, recordEvent } from "@/lib/audit";

export async function markPaid(orderId: string) {
  await prisma.$transaction(async (tx) => {
    await tx.order.update({ where: { id: orderId }, data: { status: "PAID" } });
    await prisma.lineItem.deleteMany({ where: { orderId } }); // caught
    await audit("paid", orderId); // caught
    await auditWith(tx, formatAction("paid"), orderId);
    await recordEvent("order.paid", tx);
    await analytics.pageEvent.create({ data: { name: "order.paid" } });
  });
  return prisma.order.findUnique({ where: { id: orderId } });
}

export async function refund(orderId: string) {
  const [order] = await prisma.$transaction([
    prisma.order.update({ where: { id: orderId }, data: { status: "REFUNDED" } }),
    prisma.lineItem.updateMany({ where: { orderId }, data: { refunded: true } }),
  ]);
  return order;
}

export async function cancel(orderId: string) {
  await prisma.$transaction(async (prisma) => {
    await prisma.order.update({ where: { id: orderId }, data: { status: "CANCELLED" } });
  });
}

export async function archive(orderId: string) {
  try {
    await prisma.$transaction(async (tx) => {
      await tx.order.update({ where: { id: orderId }, data: { archived: true } });
    });
  } catch (error) {
    await audit("archive-failed", orderId);
    throw error;
  }
}
