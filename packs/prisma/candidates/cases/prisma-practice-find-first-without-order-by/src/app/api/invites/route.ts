import { prisma } from "@/lib/db";
import { mongo } from "@/lib/mongo";

export async function GET(req: Request) {
  const teamId = new URL(req.url).searchParams.get("team") ?? "";
  const where = { teamId };
  const invite = await prisma.invite.findFirst({ where });
  const legacy = await mongo.legacyInvite.findFirst({ where: { teamId } });
  return Response.json({ invite, legacy });
}
