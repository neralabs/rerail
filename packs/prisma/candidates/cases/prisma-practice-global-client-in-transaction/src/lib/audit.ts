import type { Prisma } from "@prisma/client";
import { prisma } from "./db";

// Writes through the module client, outside any transaction its caller holds.
export async function audit(action: string, orderId: string) {
  await prisma.auditLog.create({ data: { action, orderId } });
}

// Takes the caller's client: inside a transaction the caller passes `tx`.
export async function auditWith(client: Prisma.TransactionClient, action: string, orderId: string) {
  await client.auditLog.create({ data: { action, orderId } });
}

// Uses the module client unless the caller passes its transaction client.
export async function recordEvent(name: string, client: Prisma.TransactionClient = prisma) {
  await client.event.create({ data: { name } });
}

export const formatAction = (action: string) => action.toUpperCase();
