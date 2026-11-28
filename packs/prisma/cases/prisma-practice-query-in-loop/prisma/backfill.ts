import type { PrismaClient } from "@prisma/client";

// Imported only by prisma/seed.ts: a backfill run once per deploy, left alone.
export async function backfillSlugs(prisma: PrismaClient) {
  const posts = await prisma.post.findMany({ where: { slug: null } });
  for (const post of posts) {
    await prisma.post.update({ where: { id: post.id }, data: { slug: post.title.toLowerCase() } });
  }
}
