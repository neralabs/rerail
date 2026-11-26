import { auth as clerkAuth, currentUser } from "@clerk/nextjs/server";
import { getServerSession } from "next-auth";

import { auth } from "@/lib/next-auth";
import { prisma } from "./db";

// Auth libraries read the session cookie inside node_modules: the same throw as cookies().
export async function getProfile() {
  "use cache";
  const session = await auth(); // caught
  return prisma.profile.findFirst({ where: { userId: session?.user?.id } });
}

export async function getLegacyProfile() {
  "use cache";
  const session = await getServerSession(); // caught
  return prisma.profile.findFirst({ where: { email: session?.user?.email ?? "" } });
}

export async function getClerkProfile() {
  "use cache";
  const { userId } = await clerkAuth(); // caught
  const user = await currentUser(); // caught
  return prisma.profile.findFirst({ where: { userId: userId ?? user?.id ?? "" } });
}

// The documented fix: read the session outside the cache scope and pass the id in.
export async function getProfileFor(userId: string) {
  "use cache";
  return prisma.profile.findFirst({ where: { userId } });
}

export async function getMyProfile() {
  const session = await auth();
  return getProfileFor(session?.user?.id ?? "");
}
