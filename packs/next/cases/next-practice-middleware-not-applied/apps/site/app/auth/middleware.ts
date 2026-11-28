import { NextResponse, type NextRequest } from "next/server";

// A helper: ./proxy.ts imports it through the `@/` alias.
export async function requireAdmin(request: NextRequest) {
  if (request.nextUrl.pathname.startsWith("/admin") && !request.cookies.has("admin")) {
    return NextResponse.redirect(new URL("/login", request.url));
  }
  return null;
}

export default requireAdmin;
