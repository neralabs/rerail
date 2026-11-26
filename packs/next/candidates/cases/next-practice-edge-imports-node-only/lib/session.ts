import { auditToFile } from "./audit";
import { jwtVerify } from "jose";

const secret = new TextEncoder().encode(process.env.SESSION_SECRET);

export async function readSession(token: string | undefined) {
  if (!token) return null;
  const { payload } = await jwtVerify(token, secret);
  return payload as { sub: string; role: string };
}

// Called from the Node.js login route only.
export function recordFailedLogin(email: string) {
  auditToFile(`failed login ${email}`);
}
