"use server";

import { prisma } from "@/lib/db";
import { sendReceipt } from "@/lib/email";
import { sendWelcome } from "@/emails/welcome";

export async function createAccount(email: string, name: string) {
  return prisma.$transaction(async (tx) => {
    const user = await tx.user.create({ data: { email, name } });
    await sendWelcome(email, name); // caught
    // cross-lane: missed until the engine joins .ts and .tsx
    await sendReceipt(email, user.id, 0);
    return user;
  });
}
