import { prisma } from "../db";

export async function archiveAll(ids: string) {
  const where = { batchId: ids };
  await markArchived(ids);
  return where;
}

async function markArchived(batchId: string) {
  await prisma.project.updateMany({ where: { batchId }, data: { archived: true } });
}
