import { prisma } from "@/lib/db";

type Row = { sku: string; price: number };

export async function importPrices(rows: Row[]) {
  await Promise.all(rows.map((r) => prisma.product.update({ where: { sku: r.sku }, data: { price: r.price } }))); // caught
}

export async function importInChunks(rows: Row[]) {
  const chunks: Row[][] = [];
  for (let i = 0; i < rows.length; i += 500) chunks.push(rows.slice(i, i + 500));
  for (const chunk of chunks) {
    await prisma.priceHistory.createMany({ data: chunk });
    await prisma.product.updateMany({ where: { sku: { in: chunk.map((c) => c.sku) } }, data: { stale: false } });
  }
}

export async function sweepExpired() {
  let cursor: string | undefined;
  while (true) {
    const page = await prisma.session.findMany({ take: 500, ...(cursor ? { cursor: { id: cursor }, skip: 1 } : {}) });
    if (page.length === 0) break;
    await prisma.session.deleteMany({ where: { id: { in: page.map((s) => s.id) } } });
    cursor = page[page.length - 1].id;
  }
}

export async function logEach(rows: Row[]) {
  for (const r of rows) {
    // a function nested in the loop body runs later, not per iteration here
    setTimeout(() => void prisma.auditLog.create({ data: { sku: r.sku } }), 0);
  }
}
