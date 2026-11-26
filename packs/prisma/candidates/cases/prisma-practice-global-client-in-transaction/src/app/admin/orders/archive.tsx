import { prisma } from "@/lib/db";

export async function archiveLines(orderId: string) {
  await prisma.lineItem.updateMany({ where: { orderId }, data: { archived: true } });
}

export function ArchivedBadge() {
  return <span>archived</span>;
}
