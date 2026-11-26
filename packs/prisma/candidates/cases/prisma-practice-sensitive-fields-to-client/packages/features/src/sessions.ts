import { prisma } from "@acme/db";

// This package lists no dependency with a schema: it reads the schemas of the root package and of the packages
// other packages depend on (@acme/db), not the example app's.
export async function GET() {
  return Response.json(await prisma.session.findMany()); // caught
}

export async function POST() {
  return Response.json(await prisma.usage.findMany());
}
