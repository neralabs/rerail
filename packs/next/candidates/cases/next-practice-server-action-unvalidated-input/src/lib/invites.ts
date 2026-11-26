import { prisma } from "@/lib/db";

// Data access: called from the actions with whatever they were given.
export async function findOpenInvite(token: string) {
  return prisma.invite.findFirst({ where: { token, usedAt: null } });
}

// Checks its own argument: a non-string never reaches the filter.
export async function findOpenInviteChecked(token: unknown) {
  if (typeof token !== "string") throw new Error("invalid token");
  return prisma.invite.findFirst({ where: { token, usedAt: null } });
}

// Turns the argument into a string before filtering.
export async function countInvitesFor(email: string) {
  return prisma.invite.count({ where: { email: String(email).toLowerCase() } });
}

export async function assertCanEditProject(userId: string, projectId: unknown) {
  if (typeof projectId !== "string") throw new Error("forbidden");
  const project = await prisma.project.findFirst({ where: { id: projectId, ownerId: userId } });
  if (!project) throw new Error("forbidden");
}
