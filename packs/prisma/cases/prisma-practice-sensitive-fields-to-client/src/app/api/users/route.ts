import { prisma } from "@/lib/db";

type PublicUser = { id: string; email: string };
const toPublic = (u: { id: string; email: string }): PublicUser => ({ id: u.id, email: u.email });

export async function GET() {
  const users = await prisma.user.findMany({ omit: { passwordHash: true } }); // caught
  return Response.json({ users, total: users.length });
}

export async function HEAD() {
  const users = await prisma.user.findMany({ omit: { passwordHash: true, resetToken: true } });
  return Response.json({ users });
}

export async function OPTIONS() {
  const users = await prisma.user.findMany();
  return Response.json(users.map(toPublic));
}
