"use server";

import { prisma } from "@/lib/db";

// A public short-link lookup that counts the visit: its only write bumps a counter and a timestamp.
export async function resolveLink(slug: string) {
  const link = await prisma.link.findUnique({ where: { slug } });
  if (!link) return null;
  await prisma.link.update({ where: { id: link.id }, data: { clicks: { increment: 1 }, lastClicked: new Date() } });
  return link.url;
}

// The same lookup that also rewrites where the link goes.
export async function retargetLink(slug: string, url: string) { // caught
  const link = await prisma.link.findUnique({ where: { slug } });
  if (!link) return null;
  await prisma.link.update({ where: { id: link.id }, data: { clicks: { increment: 1 }, url } });
}
