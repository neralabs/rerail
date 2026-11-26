"use server";

import { prisma } from "@/lib/db";
import { auth } from "@/lib/auth";

export async function clearAllDrafts() {
  await prisma.draft.deleteMany(); // caught
}

export async function clearAllDraftsLiteral() {
  await prisma.draft.deleteMany({}); // caught
}

export async function removeEverySession() {
  return prisma.$transaction(async (tx) => {
    await tx.session.deleteMany({ where: {} }); // caught
  });
}

export async function clearMyDrafts() {
  const session = await auth();
  await prisma.draft.deleteMany({ where: { userId: session?.user?.id } }); // caught
}

export async function clearMyDraftsChecked() {
  const session = await auth();
  if (!session?.user?.id) throw new Error("Not signed in");
  await prisma.draft.deleteMany({ where: { userId: session.user.id } });
}

export async function clearMyDraftsGuardedChain() {
  const session = await auth();
  if (!session) throw new Error("Not signed in");
  await prisma.draft.deleteMany({ where: { userId: session?.user?.id } });
}

export async function clearKindOfDrafts(kind: string) {
  const session = await auth();
  await prisma.notification.deleteMany({ where: { userId: session?.user?.id, kind } });
}

export async function deleteDraftsMatching(args: { where: { userId: string } }) {
  await prisma.draft.deleteMany(args);
}

export async function deleteDraftsWhere(userId: string) {
  const where = { userId };
  await prisma.draft.deleteMany({ where });
}

export async function deleteDraftsSpread(base: { where: { userId: string } }) {
  await prisma.draft.deleteMany({ ...base });
}
