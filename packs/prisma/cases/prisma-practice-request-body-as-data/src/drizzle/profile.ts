import { db } from "../db/drizzle";

// The same call shape on a Drizzle `db`: not a Prisma write.
export async function PATCH(req: Request) {
  const body = await req.json();
  return db.user.update({ where: { id: body.id }, data: body });
}
