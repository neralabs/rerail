import { NextResponse, type NextRequest } from "next/server";

export function middleware(request: NextRequest) { // caught
  const res = NextResponse.next();
  res.headers.set("content-security-policy", "default-src 'self'");
  return res;
}
