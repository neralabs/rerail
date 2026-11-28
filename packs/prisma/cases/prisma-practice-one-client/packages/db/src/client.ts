import { PrismaClient } from "@prisma/client";

// A workspace package whose own package.json does not list next: the rule makes no Next.js claim about it, though
// the Next.js app in apps/web depends on it (a stated gap).
export const prisma = new PrismaClient();
