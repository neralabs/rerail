import { NextResponse, type NextRequest } from "next/server";
import { createHash } from "node:crypto"; // caught
import type { JwtPayload } from "jsonwebtoken";
import { Role } from "@prisma/client";
import type { User } from "./lib/db";
import { auth } from "@/auth"; // caught
import { readSession } from "./lib/session";

export default auth(async (req: NextRequest) => {
  const session = await readSession(req.cookies.get("session")?.value);
  if (req.nextUrl.pathname.startsWith("/admin") && session?.role !== Role.ADMIN) {
    return NextResponse.redirect(new URL("/login", req.url));
  }
  const res = NextResponse.next();
  res.headers.set("x-request-id", createHash("sha256").update(req.url).digest("hex"));
  return res;
});

export function claims(p: JwtPayload, u: User) {
  return { sub: p.sub, email: u.email };
}

export const config = { matcher: ["/((?!_next/static|_next/image|favicon.ico).*)"] };
