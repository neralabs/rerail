import { prisma } from "@/lib/db";

export async function endSessions(userId?: string) {
  await prisma.session.deleteMany({ where: { userId } });
}

export async function endSessionsTyped(userId: string | undefined) {
  await prisma.session.deleteMany({ where: { userId: userId } });
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
