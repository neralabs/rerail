import { NextResponse, type NextRequest } from "next/server";

// A helper the API routes wrap their handlers with; not a place Next.js looks for middleware.
export default function middleware(handler: (req: NextRequest) => Promise<Response>) {
  return async (req: NextRequest) => {
    try {
      return await handler(req);
    } catch {
      return NextResponse.json({ error: "internal" }, { status: 500 });
    }
  };
}
