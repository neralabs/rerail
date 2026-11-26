import { Pool } from "@neondatabase/serverless";
import { PrismaNeon } from "@prisma/adapter-neon";
import { PrismaClient } from "@prisma/client";

const adapter = new PrismaNeon(new Pool({ connectionString: process.env.DATABASE_URL }));

export const db = new PrismaClient({ adapter });
