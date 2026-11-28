import { cookies } from "next/headers";
import { prisma } from "@/lib/db";

export async function auth() {
  const sid = (await cookies()).get("sid")?.value;
  const session = await prisma.session.findUnique({ where: { id: sid ?? "" }, include: { user: true } });
  if (!session) throw new Error("unauthenticated");
  return session;
}
