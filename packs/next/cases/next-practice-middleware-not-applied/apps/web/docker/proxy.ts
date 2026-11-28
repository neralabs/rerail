import { NextResponse, type NextRequest } from "next/server";

// Copied to src/proxy.ts by the Dockerfile; not a place Next.js looks.
export default function middleware(req: NextRequest) {
  const res = NextResponse.next();
  res.headers.set("x-frame-options", "DENY");
  return res;
}

export const config = { matcher: "/:path*" };
