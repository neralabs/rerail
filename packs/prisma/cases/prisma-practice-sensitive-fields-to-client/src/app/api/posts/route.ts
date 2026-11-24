import { NextResponse } from "next/server";
import { prisma } from "@/lib/db";
import { db } from "@/lib/drizzle";

export async function GET() {
  return NextResponse.json(
    await prisma.post.findMany({ include: { author: true } }), // caught
  );
}

export async function POST() {
  const posts = await prisma.post.findMany({
    include: { author: { select: { id: true, name: true } } },
  });
  return NextResponse.json(posts);
}

export async function PUT() {
  return NextResponse.json(await db.query.user.findMany());
}
