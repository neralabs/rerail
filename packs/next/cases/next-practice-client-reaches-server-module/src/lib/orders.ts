import type { Order } from "@prisma/client";
import { prisma } from "./db";

export function formatOrderTotal(order: Pick<Order, "totalCents" | "currency">): string {
  return new Intl.NumberFormat("en-US", { style: "currency", currency: order.currency }).format(
    order.totalCents / 100,
  );
}

export async function listOrders(customerId: string) {
  return prisma.order.findMany({ where: { customerId }, orderBy: { createdAt: "desc" } });
}
