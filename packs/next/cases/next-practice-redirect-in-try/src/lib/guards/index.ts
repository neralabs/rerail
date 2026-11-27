import * as nav from "next/navigation";
import { getSession } from "../auth";

export async function requireOwner(teamId: string) {
  const session = await getSession();
  if (session?.user.teamId !== teamId) nav.forbidden();
  return session.user;
}

export function isOwner(user: { role: string }) {
  return user.role === "OWNER";
}
