import { prisma } from "@/lib/db";

export async function GET() {
  const count = await prisma.order.count();
  return Response.json({ count });
}
