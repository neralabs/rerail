import { NextResponse, type NextRequest } from "next/server";

export function proxy(request: NextRequest) {
  return NextResponse.rewrite(new URL(`/docs${request.nextUrl.pathname}`, request.url));
}

export const config = { matcher: ["/guides/:path*"] };
