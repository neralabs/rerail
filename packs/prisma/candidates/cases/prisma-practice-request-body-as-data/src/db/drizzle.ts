import { drizzle } from "drizzle-orm/node-postgres";

// A second database layer on Drizzle: its `db` is not Prisma's.
export const db: any = drizzle(process.env.ANALYTICS_DATABASE_URL!);
