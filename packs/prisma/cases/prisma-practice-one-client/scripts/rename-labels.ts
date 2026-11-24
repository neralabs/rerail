import { PrismaClient } from "@prisma/client";

export async function renameLabels() {
  const prisma = new PrismaClient();
  await prisma.label.updateMany({ where: { name: "todo" }, data: { name: "to do" } });
  await prisma.$disconnect();
}
