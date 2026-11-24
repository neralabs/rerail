import { PrismaClient } from "@prisma/client";

// A second database: its writes were never part of the shop's transactions.
export const analytics = new PrismaClient({ datasourceUrl: process.env.ANALYTICS_DATABASE_URL });
