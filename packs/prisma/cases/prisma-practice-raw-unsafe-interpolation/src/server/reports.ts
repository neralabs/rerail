import { Prisma } from "@prisma/client";
import { prisma } from "@/lib/db";

export async function salesReport(teamId: string, status?: string, from?: string) {
  let sql = `SELECT date_trunc('day', "createdAt") AS day, sum(total) FROM "Order" WHERE 1 = 1`;
  sql += ` AND "teamId" = ${teamId}`; // caught
  if (status) sql += ` AND status = '${status}'`; // caught
  sql += from ? ` AND "createdAt" >= '${from}'` : ""; // caught
  sql += ` GROUP BY 1 ORDER BY 1`;
  return prisma.$queryRawUnsafe<{ day: Date; sum: number }[]>(sql);
}

export function statusFilter(status: string) {
  return Prisma.raw(`status = '${status}'`); // caught
}

export function ownerFilter(ownerId: string) {
  // The safe fragment: Prisma.sql sends ownerId as a parameter.
  return Prisma.sql`"ownerId" = ${ownerId}`;
}

export async function archive(ids: string[]) {
  return prisma.$transaction(async (tx) => {
    await tx.$executeRawUnsafe(
      `UPDATE "Order" SET status = 'archived' WHERE id IN (${ids.map((id) => `'${id}'`).join(",")})`, // caught
    );
  });
}

export function statusIn(statuses: string[]) {
  return Prisma.sql`"status" IN (${Prisma.raw(statuses.map((s) => `'${s}'`).join(", "))})`; // caught
}

export function statusInSafe(statuses: string[]) {
  return Prisma.sql`"status" IN (${Prisma.join(statuses)})`;
}

enum OrderStatus {
  OPEN = "OPEN",
  PAID = "PAID",
  VOID = "VOID",
}

// The enum's own names, inlined on purpose: no input reaches the list.
export const NOT_VOID = Prisma.raw(
  Object.values(OrderStatus)
    .filter((s) => s !== OrderStatus.VOID)
    .map((s) => `'${s}'`)
    .join(", "),
);

const SEARCH_PATHS = ["name", "sku"] as const;

// Each path is a constant, inlined so the expression matches a functional index.
export const searchable = (like: string) =>
  Prisma.join(SEARCH_PATHS.map((path) => Prisma.sql`data#>>${Prisma.raw(`'{${path}}'`)} ILIKE ${like}`), " OR ");
