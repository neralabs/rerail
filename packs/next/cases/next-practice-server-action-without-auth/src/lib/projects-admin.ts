import { prisma } from "./db";

// No 'use server': ordinary server code, only reachable through callers that check.
export async function wipeProjects() {
  await prisma.project.deleteMany({ where: {} });
}
