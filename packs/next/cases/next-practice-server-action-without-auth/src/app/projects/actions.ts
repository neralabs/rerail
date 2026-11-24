"use server";

import { auth } from "@/auth";
import { prisma } from "@/lib/db";
import { currentOwnerId, requireUser } from "@/lib/session";
import { archiveProject } from "@/server/projects";

export async function deleteProject(id: string) {
  const session = await auth();
  if (!session?.user) throw new Error("Unauthorized");
  await prisma.project.delete({ where: { id, ownerId: session.user.id } });
}

export async function archive(id: string) {
  await requireUser();
  await archiveProject(id);
}

export async function createProject(name: string) {
  const ownerId = await currentOwnerId();
  await prisma.project.create({ data: { name, ownerId } });
}

export async function clearFilter(url: URL) {
  url.searchParams.delete("q");
  return url.toString();
}
