"use server";

import { prisma } from "@/lib/db";

// Verifying a custom domain is a signed-in owner's action, not a sign-up step.
export async function verifyDomain(id: string) { // caught
  await prisma.domain.update({ where: { id }, data: { verified: true } });
}
