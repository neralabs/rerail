import { NextResponse, type NextRequest } from "next/server";

export function proxy(request: NextRequest) { // caught
  if (request.nextUrl.pathname.startsWith("/admin") && !request.cookies.has("session")) {
    return NextResponse.redirect(new URL("/login", request.url));
  }
  return NextResponse.next();
}

export const config = { matcher: ["/admin/:path*"] };
