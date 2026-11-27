"use server";

import { prisma } from "@notes/db";

// A 'use server' module in a package whose package.json does not list next: no Next.js Server Action is claimed.
export async function savePost(input: { title: string; body: string }) {
  return prisma.post.create({ data: input });
}
