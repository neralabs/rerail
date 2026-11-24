import type { NextApiRequest, NextApiResponse } from "next";
import { db } from "@/lib/db";

export default async function handler(req: NextApiRequest, res: NextApiResponse) {
  if (req.method === "PUT") {
    const user = await db.user.upsert({
      where: { id: String(req.query.id) },
      create: req.body, // caught
      update: { name: req.body.name },
    });
    return res.json(user);
  }
  res.status(405).end();
}
