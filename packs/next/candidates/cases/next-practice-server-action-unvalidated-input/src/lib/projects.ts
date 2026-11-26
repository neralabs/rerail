import { prisma } from "@/lib/db";

// Reached from an action with the action's argument: the filter is the client's.
export async function touchProject(projectId: string) {
  await prisma.project.updateMany({ where: { id: projectId }, data: { touchedAt: new Date() } });
}

// Called only with values the server made (see page.tsx): no Server Function passes it client input.
export async function listForOwner(ownerId: string) {
  return prisma.project.findMany({ where: { ownerId } });
}
