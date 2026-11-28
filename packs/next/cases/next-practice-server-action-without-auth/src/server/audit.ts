import { prisma } from "@/lib/db";

export async function audit(action: string) {
  await prisma.auditLog.create({ data: { action } });
}
