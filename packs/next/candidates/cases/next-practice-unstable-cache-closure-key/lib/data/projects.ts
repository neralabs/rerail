import { unstable_cache } from "next/cache";

import { requireUser } from "@/lib/auth";
import { cache } from "@/lib/cache";
import { prisma } from "@/lib/db";

export async function getRecentProjects() {
  const user = await requireUser();
  const take = 10;
  const orderBy = { updatedAt: "desc" } as const;
  function visible(p: { archived: boolean }) {
    return !p.archived;
  }
  return unstable_cache(
    async () => (await prisma.project.findMany({ where: { orgId: user.orgId }, take, orderBy })).filter(visible),
    ["recent-projects", user.orgId],
  )();
}

export const getProjectCount = cache(
  async (orgId: string) => prisma.project.count({ where: { orgId } }),
  ["project-count"],
);

export async function getArchivedProjects(orgId: string) {
  "use cache";
  return prisma.project.findMany({ where: { orgId, archived: true } });
}
