import "server-only";
import { cookies } from "next/headers";

import { prisma } from "@/lib/db";

export async function requireUser() {
  const token = (await cookies()).get("session")?.value;
  const session = await prisma.session.findUniqueOrThrow({ where: { token } });
  return { userId: session.userId, orgId: session.orgId };
}

export async function getSession() {
  const token = (await cookies()).get("session")?.value;
  const session = await prisma.session.findUniqueOrThrow({ where: { token }, include: { user: true } });
  return { user: { id: session.userId, orgId: session.activeOrgId } };
}
