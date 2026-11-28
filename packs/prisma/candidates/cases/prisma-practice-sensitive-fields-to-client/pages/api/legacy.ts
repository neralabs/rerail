import type { NextApiRequest, NextApiResponse } from "next";
import { prisma } from "../../src/lib/db";

export default async function handler(req: NextApiRequest, res: NextApiResponse) {
  const userId = String(req.query.userId);
  res.status(200).json(await prisma.account.findMany({ where: { userId } })); // caught
}
