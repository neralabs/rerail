"use cache";

import { prisma } from "@/lib/db";

// params is allowed in a cache scope: it becomes part of the cache key.
export default async function PostPage({ params }: { params: Promise<{ slug: string }> }) {
  const { slug } = await params;
  const post = await prisma.post.findUnique({ where: { slug } });
  return <article>{post?.title}</article>;
}
