import { Client } from "@planetscale/database";
import { PrismaPlanetScale } from "@prisma/adapter-planetscale";
import { PrismaClient } from "@prisma/client";

// An HTTP driver adapter keeps no connection pool: a copy per hot reload opens no connections.
const adapter = new PrismaPlanetScale(new Client({ url: process.env.DATABASE_URL }));

export const edgeDb = new PrismaClient({ adapter });
