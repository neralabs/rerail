import { NextResponse } from "next/server";
import { prisma } from "@/lib/db";

export async function POST(request: Request) {
  const { userId } = (await request.json()) as { userId: string };
  await prisma.notification.updateMany({ data: { read: true } }); // caught
  await prisma.notification.updateMany({ where: { userId }, data: { kind: "seen" } });
  return NextResponse.json({ ok: true });
}

export async function DELETE() {
  const now = new Date();
  await prisma.session.deleteMany({ where: { expiresAt: { lt: now } } });
  await prisma.notification.updateManyAndReturn({ where: { read: false, kind: "digest" }, data: { read: true } });
  return NextResponse.json({ ok: true });
}
