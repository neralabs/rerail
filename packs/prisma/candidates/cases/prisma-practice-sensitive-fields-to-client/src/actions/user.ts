"use server";

import { prisma } from "@/lib/db";
import { currentUserId } from "@/lib/auth";

export async function getProfile() {
  const id = await currentUserId();
  return prisma.user.findUniqueOrThrow({ where: { id } }); // caught
}

async function loadAccounts(userId: string) {
  return prisma.account.findMany({ where: { userId } });
}

export async function countAccounts() {
  const accounts = await loadAccounts(await currentUserId());
  return accounts.length;
}
