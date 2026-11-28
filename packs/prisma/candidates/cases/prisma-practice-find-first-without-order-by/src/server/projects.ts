import { prisma } from "@/lib/db";

export async function searchUser(q: string) {
  // email is unique, but contains matches many users
  return prisma.user.findFirst({ where: { email: { contains: q } }, select: { id: true } }); // caught
}

export async function otherProjectBySlug(id: string, slug: string) {
  return prisma.project.findFirst({ // caught
    where: { id: { not: id }, slug },
  });
}

export async function projectBySlug(teamId: string, slug: string) {
  return prisma.project.findFirst({ where: { teamId, slug } });
}

export async function assertSlugFree(teamId: string, slug: string, id: string) {
  const clash = await prisma.project.findFirst({ where: { slug, NOT: { id } } });
  if (clash) throw new Error("slug taken");
}

export async function hasProjects(teamId: string) {
  if (await prisma.project.findFirst({ where: { teamId } })) return true;
  const other = await prisma.project.findFirst({ where: { name: teamId } });
  return !!other || other !== null;
}

export async function anyProject(args: { where: { teamId: string } }) {
  const a = await prisma.project.findFirst(args);
  const b = await prisma.project.findFirst();
  const c = await prisma.project.findFirst({ ...args });
  return [a, b, c];
}
