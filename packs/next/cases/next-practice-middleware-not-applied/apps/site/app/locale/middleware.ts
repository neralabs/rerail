import { NextResponse, type NextRequest } from "next/server";

export function middleware(request: NextRequest) { // caught
  const locale = request.headers.get("accept-language")?.split(",")[0] ?? "en";
  return NextResponse.rewrite(new URL(`/${locale}${request.nextUrl.pathname}`, request.url));
}

export const config = { matcher: ["/((?!_next).*)"] };
