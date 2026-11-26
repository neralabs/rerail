import bcrypt from "bcrypt";

export const runtime = "nodejs";

export async function POST(req: Request) {
  const { password } = await req.json();
  return Response.json({ hash: await bcrypt.hash(password, 12) });
}
