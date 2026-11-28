import { NextResponse } from "next/server";
import { prisma } from "@/lib/db";

export async function GET(req: Request) {
  const keys = await prisma.apiKey.findMany({ where: { userId: req.headers.get("x-user") ?? "" } });
  const usage = await prisma.usage.findMany();
  return NextResponse.json({ keys, usage });
}

export async function POST(req: Request) {
  const args = await req.json();
  return NextResponse.json(await prisma.user.findMany(args));
}
