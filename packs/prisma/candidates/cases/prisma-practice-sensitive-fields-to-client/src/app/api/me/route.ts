import { NextResponse } from "next/server";
import { compare } from "bcryptjs";
import { prisma } from "@/lib/db";
import { currentUserId } from "@/lib/auth";

export async function GET() {
  const user = await prisma.user.findUnique({ where: { id: await currentUserId() } }); // caught
  return NextResponse.json(user);
}

export async function PATCH(req: Request) {
  const { name } = await req.json();
  const user = await prisma.user.update({
    where: { id: await currentUserId() },
    data: { name },
    select: { id: true, email: true, name: true },
  });
  return NextResponse.json(user);
}

export async function POST(req: Request) {
  const { email, password } = await req.json();
  const user = await prisma.user.findUnique({ where: { email } });
  const ok = user ? await compare(password, user.passwordHash) : false;
  return NextResponse.json({ ok, id: user?.id });
}
