"use cache";

import { prisma } from "@/lib/db";

export default async function BlogPage({
  searchParams, // caught
}: {
  searchParams: Promise<{ page?: string }>;
}) {
  const { page } = await searchParams;
  const posts = await prisma.post.findMany({ skip: Number(page ?? 0) * 10, take: 10 });
  return <ul>{posts.map((p) => <li key={p.id}>{p.title}</li>)}</ul>;
}
