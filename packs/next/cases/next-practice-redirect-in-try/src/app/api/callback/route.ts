import { NextResponse, type NextRequest } from "next/server";
import { redirect as remixRedirect } from "@remix-run/node";
import { prisma } from "@/lib/db";

export async function GET(request: NextRequest) {
  const code = request.nextUrl.searchParams.get("code");
  try {
    const account = await prisma.account.findFirstOrThrow({ where: { code: code ?? "" } });
    return NextResponse.redirect(new URL(`/accounts/${account.id}`, request.url));
  } catch {
    return NextResponse.redirect(new URL("/login?error=callback", request.url));
  }
}

export async function POST() {
  try {
    return remixRedirect("/done");
  } catch {
    return new Response("failed", { status: 500 });
  }
}
