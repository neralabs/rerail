import { PrismaClient } from "@prisma/client";

// A folder named `latest` is app code: "test" inside a word does not make it a test.
export async function latestPosts() {
  const prisma = new PrismaClient(); // caught
  return prisma.post.findMany({ take: 10 });
}
