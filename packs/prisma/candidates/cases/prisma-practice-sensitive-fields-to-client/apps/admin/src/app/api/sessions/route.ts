import { prisma } from "@acme/db";

// The admin app's package depends on @acme/db, whose schema gives Session a token column.
export async function GET() {
  return Response.json(await prisma.session.findMany()); // caught
}
