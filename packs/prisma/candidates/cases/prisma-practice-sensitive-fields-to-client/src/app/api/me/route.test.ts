import { NextResponse } from "next/server";
import { prisma } from "@/lib/db";

export async function fixture() {
  return NextResponse.json(await prisma.user.findMany());
}
