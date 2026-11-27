import type { Prisma } from "@prisma/client";

export async function recordAudit(tx: Prisma.TransactionClient, action: string, orderId: string) {
  await tx.auditLog.create({ data: { action, orderId } });
}
