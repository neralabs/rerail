import { db } from "../db/drizzle";

// Drizzle's relational query on a `user` table: not Prisma's delegate, whatever the schema.prisma declares.
export async function GET() {
  return Response.json(await db.query.user.findMany());
}

export async function POST() {
  return Response.json(await db.user.findMany());
}
