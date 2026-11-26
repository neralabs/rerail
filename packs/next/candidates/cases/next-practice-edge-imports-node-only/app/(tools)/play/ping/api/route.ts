import { prisma } from "../../../../../lib/db"; // caught

export const runtime = "edge";

export async function GET() {
  await prisma.$queryRaw`select 1`;
  return Response.json({ pong: true });
}
