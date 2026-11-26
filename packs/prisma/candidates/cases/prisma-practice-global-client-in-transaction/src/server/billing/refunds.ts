import { db } from "../db";
import { audit } from "@/lib/audit";

export async function refundOrder(orderId: string) {
  await db.$transaction(async (tx) => {
    await tx.order.update({ where: { id: orderId }, data: { status: "REFUNDED" } });
    await audit("refunded", orderId); // caught
  });
}
