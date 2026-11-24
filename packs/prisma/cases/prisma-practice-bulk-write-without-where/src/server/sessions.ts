import { prisma } from "@/lib/db";

export async function endSessions(userId?: string) {
  await prisma.session.deleteMany({ where: { userId } }); // caught
}

export async function endSessionsTyped(userId: string | undefined) {
  await prisma.session.deleteMany({ where: { userId: userId } }); // caught
}

export async function endSessionsGuarded(userId?: string) {
  if (!userId) return;
  await prisma.session.deleteMany({ where: { userId } });
}

export async function endSessionsFor(userId: string) {
  await prisma.session.deleteMany({ where: { userId } });
}

export async function markAllRead(userId: string) {
  await prisma.notification.updateManyAndReturn({ data: { read: true } }); // caught
}

// A constant beside a missing owner key filters nothing per owner: every user's unread digests go.
export async function clearDigests(userId?: string) {
  await prisma.notification.deleteMany({ where: { userId, kind: "digest", read: false } }); // caught
}

// A value the caller chose still filters.
export async function clearKind(kind: string, userId?: string) {
  await prisma.notification.deleteMany({ where: { userId, kind } });
}
