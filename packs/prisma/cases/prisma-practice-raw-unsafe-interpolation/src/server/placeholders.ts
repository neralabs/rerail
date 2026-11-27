import { prisma } from "@/lib/db";
import { env } from "@/env";

// Placeholder text pasted in: `$1`, `$2`, … with the values passed as parameters.
export async function dependents(fieldIds: string[], type: string) {
  const inClause = fieldIds.map((_, i) => `$${i + 1}`).join(",");
  const typeParam = `$${fieldIds.length + 1}`;
  return prisma.$queryRawUnsafe(
    `SELECT id FROM field WHERE from_id IN (${inClause}) AND type = ${typeParam}`,
    ...fieldIds,
    type,
  );
}

export async function rename(userId: string, name: string) {
  const values: unknown[] = [];
  function param(v: unknown) {
    values.push(v);
    return `$${values.length}`;
  }
  return prisma.$executeRawUnsafe(`UPDATE "User" SET name = ${param(name)} WHERE id = ${param(userId)}`, ...values);
}

// Names that only sound like a placeholder or an escape: the value is pasted.
export async function byRequest(req: { query: Record<string, string>; body: { ID: string } }, q: string, mine: boolean) {
  await prisma.$queryRawUnsafe(`SELECT * FROM "Doc" WHERE id = ${getParam(req)}`); // caught
  await prisma.$queryRawUnsafe(`SELECT * FROM "Doc" WHERE title = ${unescapeHtml(q)}`); // caught
  await prisma.$queryRawUnsafe(`SELECT * FROM "Doc" WHERE id = ${req.body.ID}`); // caught
  await prisma.$queryRawUnsafe(`SELECT * FROM "Doc" WHERE title LIKE '${q + "%"}'`); // caught
  await prisma.$queryRawUnsafe(`SELECT * FROM "Doc" WHERE owner = ${mine ? q : "''"}`); // caught
  await prisma.$queryRawUnsafe(`SELECT * FROM "Doc" WHERE tier = '${env.DEFAULT_TIER}' AND v = ${process.env.SCHEMA_V}`);
}

declare function getParam(req: { query: Record<string, string> }): string;
declare function unescapeHtml(s: string): string;

// MySQL and SQLite `?` marks pasted in, the values bound separately.
export async function byIds(ids: string[], since: Date) {
  const marks = ids.map(() => "?").join(", ");
  await prisma.$queryRawUnsafe(`SELECT id FROM Doc WHERE id IN (${marks}) AND createdAt > ?`, ...ids, since);
  await prisma.$queryRawUnsafe(`SELECT id FROM Doc WHERE id IN (${Array(ids.length).fill("?").join(",")})`, ...ids);
  const slots = "?, ".repeat(ids.length).slice(0, -2);
  await prisma.$executeRawUnsafe(`DELETE FROM Doc WHERE id IN (${slots})`, ...ids);
  // The ids themselves pasted, quoted or not.
  await prisma.$queryRawUnsafe(`SELECT id FROM Doc WHERE id IN (${ids.map((id) => `'${id}'`).join(", ")})`); // caught
  const list = ids.join(", ");
  await prisma.$queryRawUnsafe(`SELECT id FROM Doc WHERE id IN (${list})`); // caught
}
