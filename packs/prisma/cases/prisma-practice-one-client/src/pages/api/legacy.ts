import type { NextApiRequest, NextApiResponse } from "next";
import { PrismaPg } from "@prisma/adapter-pg";
import { PrismaClient } from "@/generated/prisma/client";

export default async function handler(req: NextApiRequest, res: NextApiResponse) {
  const adapter = new PrismaPg({ connectionString: process.env.DATABASE_URL! });
  const db = new PrismaClient({ adapter }); // caught
  try {
    res.json(await db.account.findMany({ where: { ownerId: String(req.query.owner) } }));
  } finally {
    await db.$disconnect();
  }
}
