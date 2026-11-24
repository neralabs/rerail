import { PrismaClient } from "@prisma/client";

function makeClient() {
  return new PrismaClient({ datasourceUrl: process.env.ANALYTICS_URL }); // caught
}

export const analytics = makeClient();
