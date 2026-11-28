import { prisma } from "~/lib/db";

// App code in a folder named prisma: queries run per request like any other.
export async function websitesWithOwners(websites: { id: string; userId: string }[]) {
  return Promise.all(
    websites.map(async (w) => ({ ...w, owner: await prisma.user.findFirst({ where: { id: w.userId } }) })), // caught
  );
}

export async function publishPages(pages: { slug: string; title: string }[]) {
  for (const page of pages) {
    await prisma.page.create({ data: { slug: page.slug, title: page.title } }); // caught
  }
}

export async function retireByReason(byReason: Map<string, string[]>) {
  // Each iteration writes every row grouped under one key: already one query per group.
  await Promise.all(
    Array.from(byReason.entries()).map(([reason, ids]) =>
      prisma.job.updateMany({ where: { id: { in: ids } }, data: { status: "FAILED", reason } }),
    ),
  );
}

export async function purgeInChunks(chunks: string[][]) {
  for (const chunk of chunks) {
    await prisma.session.deleteMany({ where: { userId: { in: chunk } } });
  }
}

export async function refreshBatches(batchJobs: { id: string }[]) {
  for (const job of batchJobs) {
    await prisma.batchJob.update({ where: { id: job.id }, data: { refreshedAt: new Date() } }); // caught
  }
}
