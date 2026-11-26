import { db } from "@/lib/db";

export async function seed(req: Request) {
  await db.user.create({ data: await req.json() });
}
