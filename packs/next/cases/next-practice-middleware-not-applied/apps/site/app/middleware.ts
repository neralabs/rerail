import { NextResponse, type NextRequest } from "next/server";

// A helper: ./proxy.ts imports it and calls it.
export function middleware(request: NextRequest) {
  if (!request.cookies.has("locale")) {
    return NextResponse.redirect(new URL(`/en${request.nextUrl.pathname}`, request.url));
  }
  return undefined;
}
