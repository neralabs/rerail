import { prisma } from "@/lib/db";

// `previewToken` and `prefixedSecret` are credentials: "preview" and "prefixed" are not display words.
export async function GET() {
  return Response.json(await prisma.payout.findMany()); // caught
}
