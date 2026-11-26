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
