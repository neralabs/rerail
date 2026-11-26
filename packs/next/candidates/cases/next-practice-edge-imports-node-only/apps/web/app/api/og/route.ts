import { prisma } from "../../../lib/db";

export const runtime = "edge";

export async function GET() {
  return Response.json({ products: await prisma.product.count() });
}
