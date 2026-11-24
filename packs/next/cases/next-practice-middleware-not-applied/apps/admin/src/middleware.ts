import { NextResponse, type NextRequest } from "next/server";

// Next.js 16 still loads middleware.ts (deprecated in favour of proxy.ts).
export function middleware(request: NextRequest) {
  return NextResponse.next();
}
