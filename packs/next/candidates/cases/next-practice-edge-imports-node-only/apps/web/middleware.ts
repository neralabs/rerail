import { NextResponse, type NextRequest } from "next/server";
import { prisma } from "./lib/db";

export async function middleware(req: NextRequest) {
  const blocked = await prisma.blockedIp.findFirst({ where: { ip: req.headers.get("x-forwarded-for") ?? "" } });
  return blocked ? new NextResponse(null, { status: 403 }) : NextResponse.next();
}

export const config = { runtime: "nodejs", matcher: ["/api/:path*"] };
