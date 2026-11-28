import "server-only"; // caught
import { cookies } from "next/headers";
import { verifyPassword } from "./password";

export type SessionUser = { id: string; email: string };

export const SESSION_COOKIE = "session";

export async function getSessionUser(): Promise<SessionUser | null> {
  const token = (await cookies()).get(SESSION_COOKIE)?.value;
  if (!token) return null;
  const res = await fetch(`${process.env.AUTH_URL}/session`, { headers: { authorization: `Bearer ${token}` } });
  return res.ok ? ((await res.json()) as SessionUser) : null;
}

export { verifyPassword };
