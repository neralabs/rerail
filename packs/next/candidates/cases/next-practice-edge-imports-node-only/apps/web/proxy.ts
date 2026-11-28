import { NextResponse, type NextRequest } from "next/server";
import { prisma } from "./lib/db";

export async function proxy(req: NextRequest) {
  const flag = await prisma.flag.findUnique({ where: { key: "maintenance" } });
  return flag?.on ? NextResponse.rewrite(new URL("/maintenance", req.url)) : NextResponse.next();
}
