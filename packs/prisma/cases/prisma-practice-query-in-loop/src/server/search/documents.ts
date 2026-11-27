import { prisma } from "@/lib/db";

export async function indexDocument(id: string) {
  const chunks = await prisma.chunk.findMany({ where: { docId: id } });
  for (const chunk of chunks) {
    await prisma.chunk.update({ where: { id: chunk.id }, data: { indexed: true } });
  }
}
