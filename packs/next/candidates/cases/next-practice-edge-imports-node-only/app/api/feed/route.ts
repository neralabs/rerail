import bcrypt from "bcrypt"; // caught
import { db } from "../../../lib/edge-db";

export const runtime = "edge";

export async function POST(req: Request) {
  const { email, password } = await req.json();
  const user = await db.user.findUnique({ where: { email } });
  const ok = user ? await bcrypt.compare(password, user.passwordHash) : false;
  return Response.json({ ok });
}
