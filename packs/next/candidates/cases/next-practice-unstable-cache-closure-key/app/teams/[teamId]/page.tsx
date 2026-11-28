import * as nextCache from "next/cache";
import { unstable_cache as cache } from "next/cache";

import { prisma } from "@/lib/db";

type Props = { params: Promise<{ teamId: string }>; searchParams: Promise<{ page?: string }> };

export default async function TeamPage({ params, searchParams }: Props) {
  const { teamId } = await params;
  const team = await nextCache.unstable_cache(async () => prisma.team.findUnique({ where: { id: teamId } }))(); // caught
  const { page } = await searchParams;
  const skip = (Number(page ?? "1") - 1) * 20;
  const members = await cache(
    async () => prisma.member.findMany({ where: { teamId }, skip, take: 20 }),
    ["team-members", teamId, String(skip)],
  )();
  const loadProjects = async () => prisma.project.findMany({ where: { teamId, archived: false } });
  const projects = await cache(loadProjects, ["team-projects", teamId])();

  const loadAudit = async () => prisma.auditLog.findMany({ where: { teamId }, take: 50 });
  const audit = await cache(loadAudit, ["team-audit"])(); // caught

  return (
    <section>
      <h1>{team?.name}</h1>
      <p>
        {members.length} members, {projects.length} projects, {audit.length} recent events
      </p>
    </section>
  );
}
