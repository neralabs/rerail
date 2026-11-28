"use server";

import { PrismaClient } from "@prisma/client";

const prisma = new PrismaClient();

// The same shape as a Next.js Server Function, in a React Router app: not this rule's.
export async function acceptInvite(token: string) {
  return prisma.invite.findFirst({ where: { token, usedAt: null } });
}
