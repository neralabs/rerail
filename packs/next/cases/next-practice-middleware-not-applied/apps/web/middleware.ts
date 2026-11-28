import { NextResponse, type NextRequest } from "next/server";

// Never runs: this app keeps its routes in src/app, so Next.js only reads src/middleware.ts.
export async function middleware(request: NextRequest) { // caught
  if (!request.cookies.has("session")) {
    return NextResponse.redirect(new URL("/login", request.url));
  }
  return NextResponse.next();
}

export const config = { matcher: ["/dashboard/:path*", "/settings/:path*"] };
