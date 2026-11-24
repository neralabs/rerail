"use server";

import { prisma } from "@/lib/db";

// A (public) route group: the poll is open to signed-out visitors by design.
export async function vote(pollId: string, option: string) {
  await prisma.vote.create({ data: { pollId, option } });
}
