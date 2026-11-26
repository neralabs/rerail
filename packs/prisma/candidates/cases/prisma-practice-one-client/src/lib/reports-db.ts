import { PrismaClient } from "@prisma/client";

export const reportsDb = new PrismaClient({ log: ["warn", "error"] }); // caught
