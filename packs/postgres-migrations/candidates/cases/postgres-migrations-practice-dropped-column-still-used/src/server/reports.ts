import { prisma } from "@/lib/db";

export async function usersByTier() {
  return prisma.$queryRaw`SELECT "tier", count(*) FROM "User" GROUP BY 1`;
}

export async function accountsByProvider() {
  return prisma.$queryRaw`SELECT "provider", count(*) FROM "Account" GROUP BY 1`;
}

export async function tiersWithAccounts() {
  return prisma.$queryRaw`
    SELECT "User"."tier", count("Account"."id")
    FROM "User" JOIN "Account" ON "Account"."id" = "User"."id"
    GROUP BY 1`;
}

export async function flaggedTitles() {
  // oldFlag is gone: its value now lives in the title prefix
  return prisma.$queryRaw`
    SELECT p."title" LIKE 'oldFlag:%' AS "oldFlag" -- was "posts"."oldFlag"
    FROM "posts" p`;
}

export async function copyTiersToTeams() {
  // "tier" here is the Team's column, filled from each account's id
  return prisma.$executeRaw`
    INSERT INTO "Team" ("id", "tier")
    SELECT "id", 'free' FROM "Account"`;
}

export async function prompts() {
  // "prompt" is an output name (no AS) of a JSON lookup, not a column of User
  return prisma.$queryRaw`SELECT u."id", u."meta"->>'prompt' prompt FROM "User" u`;
}
