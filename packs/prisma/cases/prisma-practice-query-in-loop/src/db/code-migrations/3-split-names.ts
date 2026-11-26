import { db } from "~/server/db";

// A data migration module: the migration runner calls `up` once per database.
export async function up() {
  const users = await db.user.findMany({ select: { id: true, name: true } });
  for (const u of users) {
    const [first, ...rest] = (u.name ?? "").split(" ");
    await db.user.update({ where: { id: u.id }, data: { firstName: first, lastName: rest.join(" ") } });
  }
}
