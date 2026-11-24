"use server";

import { revalidatePath } from "next/cache";
import { prisma } from "@/lib/db";
import { archiveProject, listProjects } from "@/server/projects";
import * as projects from "@/server";
import { recordAudit } from "@/server";

// The admin page checks the session; these actions do not, and anyone can POST to them.
export async function deleteProject(id: string) { // caught
  await prisma.project.delete({ where: { id } });
  revalidatePath("/admin");
}

export const renameProject = async (id: string, name: string) => { // caught
  await prisma.$transaction(async (tx) => {
    await tx.project.update({ where: { id }, data: { name } });
  });
};

export async function archive(id: string) { // caught
  await archiveProject(id);
}

export async function purge(id: string) { // caught
  await projects.removeProject(id);
}

async function logAction(name: string) { // caught
  await recordAudit(name);
}

export { logAction };

// Reads only: no write, so nothing to report.
export async function refreshProjects() {
  revalidatePath("/admin");
  return listProjects();
}

// Not exported: not an endpoint.
async function helper(id: string) {
  await prisma.project.delete({ where: { id } });
}
