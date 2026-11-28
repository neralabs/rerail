import { PrismaClient } from "@prisma/client";

const globalForPrisma = globalThis as unknown as { prisma?: PrismaClient };

// Every query leaves out the API key hashes unless it selects them.
export const prisma =
  globalForPrisma.prisma ?? new PrismaClient({ omit: { apiKey: { hashedKey: true } } });

if (process.env.NODE_ENV !== "production") globalForPrisma.prisma = prisma;
