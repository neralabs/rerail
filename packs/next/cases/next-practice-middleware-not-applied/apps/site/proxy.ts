import { NextResponse, type NextRequest } from "next/server";
import { middleware as localeMiddleware } from "./app/middleware";
import { requireAdmin } from "@/app/auth/middleware";

export async function proxy(request: NextRequest) {
  const denied = await requireAdmin(request);
  if (denied) return denied;
  return localeMiddleware(request) ?? NextResponse.next();
}

export const config = { matcher: ["/((?!api|_next/static|_next/image|favicon.ico).*)"] };
