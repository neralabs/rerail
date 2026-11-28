import { Prisma } from "@prisma/client";
import { prisma } from "@/lib/db";

const ARCHIVED = "archived";
const MAX_ROWS = 500;
const kind = "invoice";

function escapeLiteral(v: string) {
  return `'${v.replace(/'/g, "''")}'`;
}

export async function withTenant<T>(schema: string, run: () => Promise<T>) {
  return prisma.$transaction(async (tx) => {
    // An identifier: it cannot be a parameter.
    await tx.$executeRawUnsafe(`SET search_path TO "${schema}"`);
    await tx.$executeRawUnsafe(`SAVEPOINT ${schema}_start`);
    return run();
  });
}

export async function staleRows(table: string, limit: string, label: string) {
  // Constants pasted in, a number coerced, a literal escaped: none can carry a quote.
  await prisma.$queryRawUnsafe(`SELECT * FROM "Doc" WHERE status = '${ARCHIVED}' LIMIT ${MAX_ROWS}`);
  await prisma.$queryRawUnsafe(`SELECT * FROM "Doc" WHERE kind = '${kind}' AND size > ${10}`);
  await prisma.$queryRawUnsafe(`SELECT * FROM "Doc" LIMIT ${Number(limit)}`);
  await prisma.$queryRawUnsafe(`SELECT * FROM "Doc" WHERE label = ${escapeLiteral(label)}`);
  // A table name from the caller (a name position the rule leaves alone), a parameter for the value.
  return prisma.$queryRawUnsafe(`SELECT count(*) FROM "${table}" WHERE "deletedAt" < $1`, new Date());
}

export const now = Prisma.raw("NOW()");
export const column = (alias: string, name: string) => Prisma.raw(`"${alias}"."${name}"`);

export function notSql(name: string) {
  // String.raw is not Prisma's.
  return String.raw`C:\Users\${name}`;
}

const TABLES = { doc: '"Doc"', user: '"User"' } as const;

export async function counts(table: string, ids: string[]) {
  await prisma.$queryRawUnsafe("SELECT count(*) FROM " + table);
  await prisma.$queryRawUnsafe(`SELECT * FROM ${TABLES.doc} WHERE id = ${TABLES.user}`);
  return prisma.$queryRawUnsafe(`SELECT * FROM "Doc" LIMIT ${ids.length}`);
}

export async function withTimeout(timeoutMs: number, lo: string, hi: string) {
  // Utility statements take no bind parameters: SET, CREATE, … are left alone (a value quoted in a SET is not: see
  // sorting.ts).
  await prisma.$executeRawUnsafe(`SET LOCAL statement_timeout = ${timeoutMs}`);
  await prisma.$executeRawUnsafe(
    `CREATE TABLE IF NOT EXISTS "Event_p" PARTITION OF "Event" FOR VALUES FROM ('${lo}') TO ('${hi}')`,
  );
}

export async function byId(raw: string) {
  const id = Number(raw);
  return prisma.$queryRawUnsafe(`SELECT * FROM "Doc" WHERE id IN (${id})`);
}

enum Reason {
  CSAM = "CSAM",
  TOS = "TOS",
}

export async function restore(ids: number[], userId: number, values: unknown[], keptClause: string) {
  const bind = (v: unknown) => `$${values.push(v)}`;
  await prisma.$queryRawUnsafe(`SELECT id FROM "Image" WHERE "blockedFor" = '${Reason.CSAM}'`);
  await prisma.$queryRawUnsafe(`UPDATE "User" SET muted = true WHERE id = ${bind(userId)}`, ...values);
  return prisma.$queryRawUnsafe(`
    SELECT id FROM "Model"
    -- A model the owner's wipe kept is a moderator's to restore.
    WHERE status = 'Deleted' AND ${keptClause}
  `);
}

export async function setKey(key: string, value: unknown) {
  const json = JSON.stringify(value).replace(/'/g, "''");
  await prisma.$executeRawUnsafe(`INSERT INTO "KeyValue" ("key", "value") VALUES ($1, '${json}'::jsonb)`, key);
}

export const resultField = (field: "promptIds" | "model", since: Date) =>
  Prisma.raw(`result->'${field}' AND "createdAt" > '${since.toISOString()}'`);
