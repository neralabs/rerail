import { prisma } from "@/lib/db";

// Imported by the seed and by the app's cron (./cron.ts): app code.
export async function recountAll() {
  const lists = await prisma.list.findMany();
  for (const list of lists) {
    const size = await prisma.item.count({ where: { listId: list.id } }); // caught
    await prisma.list.update({ where: { id: list.id }, data: { size } }); // caught
  }
}
