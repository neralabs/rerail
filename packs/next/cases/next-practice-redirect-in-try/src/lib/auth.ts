import "server-only";
import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import { prisma } from "@/lib/db";

export async function getSession() {
  const token = (await cookies()).get("session")?.value;
  if (!token) return null;
  return prisma.session.findUnique({ where: { token }, include: { user: true } });
}

// Sends the visitor to the login page when there is no session.
export async function requireUser() {
  const session = await getSession();
  if (!session) {
    redirect("/login");
  }
  return session.user;
}

export const requireAdmin = async () => {
  const user = await requireUser();
  if (user.role !== "ADMIN") {
    redirect("/");
  }
  return user;
};

// Never redirects: returns null instead.
export async function currentUser() {
  const session = await getSession();
  return session?.user ?? null;
}
