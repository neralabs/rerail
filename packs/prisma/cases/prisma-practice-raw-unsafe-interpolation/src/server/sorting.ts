import { Prisma } from "@prisma/client";
import { prisma } from "@/lib/db";

const ORDER = { newest: Prisma.sql`"createdAt" DESC`, name: Prisma.sql`"name"` } as const;

// A name cannot be a parameter, so a sort key from the caller is pasted: from an unchecked string, it is SQL.
export async function listProducts(sort: string, dir: string) {
  await prisma.$queryRawUnsafe(`SELECT * FROM "Product" ORDER BY ${sort}`); // caught
  await prisma.$queryRaw`SELECT * FROM "Product" ORDER BY ${Prisma.raw(sort)}`; // caught
  return prisma.$queryRawUnsafe(`SELECT * FROM "Product" ORDER BY "name" ${dir}`); // caught
}

// Checked against a list, looked up in a fixed map, switched on, or typed as a union of literals: left alone.
export async function listChecked(sort: string, key: keyof typeof ORDER) {
  if (!["name", "createdAt"].includes(sort)) throw new Error("bad sort");
  await prisma.$queryRawUnsafe(`SELECT * FROM "Product" ORDER BY "${sort}"`);
  return prisma.$queryRaw`SELECT * FROM "Product" ORDER BY ${ORDER[key]}`;
}

export async function listSwitched(sort: string) {
  switch (sort) {
    case "name":
    case "createdAt":
      return prisma.$queryRaw`SELECT * FROM "Product" ORDER BY ${Prisma.raw(sort)}`;
    default:
      throw new Error("bad sort");
  }
}

export async function listTyped(sort: "name" | "createdAt") {
  return prisma.$queryRawUnsafe(`SELECT * FROM "Product" ORDER BY ${sort}`);
}

// SET takes no parameters, but a value quoted in it is still SQL; set_config takes one.
export async function setTenant(tenantId: string, ms: number) {
  await prisma.$executeRawUnsafe(`SET LOCAL app.tenant = '${tenantId}'`); // caught
  await prisma.$executeRawUnsafe(`SET LOCAL statement_timeout = ${ms}`);
  await prisma.$executeRaw`SELECT set_config('app.tenant', ${tenantId}, true)`;
}

// A sort key turned into SQL by name: reported wherever it is pasted.
export const orderSql = (sortBy: string) => Prisma.raw(sortBy); // caught

// An alias helper: Prisma.raw of a name every caller hands as a constant, not in a sort position.
export const ownedBy = (alias: string, userId: string) => Prisma.sql`${Prisma.raw(alias)}."userId" = ${userId}`;
