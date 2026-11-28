import { prisma } from "../src/lib/db";

export async function resetDb() {
  await prisma.session.deleteMany();
  await prisma.notification.updateMany({ data: { read: false } });
}
