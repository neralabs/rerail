import { prisma } from "../src/lib/db";
import { indexDocument } from "../src/server/search/documents";

// A script by its folder, though it exports its entry: left alone, and so is the module only it imports.
export async function rebuild() {
  const docs = await prisma.doc.findMany();
  for (const doc of docs) {
    await prisma.doc.update({ where: { id: doc.id }, data: { indexedAt: new Date() } });
    await indexDocument(doc.id);
  }
}
