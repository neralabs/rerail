import { prisma } from "./client";

export async function getStats() {
  return { orders: await prisma.order.count() };
}
