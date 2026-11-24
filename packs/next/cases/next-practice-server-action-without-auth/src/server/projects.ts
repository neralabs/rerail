import { prisma } from "../lib/db";

// The data access layer: plain server code, not an endpoint itself.
export async function archiveProject(id: string) {
  await prisma.project.update({ where: { id }, data: { archived: true } });
}

export async function listProjects() {
  return prisma.project.findMany();
}

export async function removeProject(id: string) {
  await prisma.project.delete({ where: { id } });
}
