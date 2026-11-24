import { PrismaClient } from "@prisma/client";

export async function GET() {
  const prisma = new PrismaClient(); // caught
  const count = await prisma.user.count();
  return Response.json({ count });
}
