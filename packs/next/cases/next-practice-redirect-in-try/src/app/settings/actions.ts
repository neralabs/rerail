"use server";

import * as nav from "next/navigation";
import { requireOwner, isOwner } from "../../lib/guards";
import { prisma } from "../../lib/db";

export async function renameTeam(teamId: string, name: string) {
  try {
    const user = await requireOwner(teamId); // caught
    await prisma.team.update({ where: { id: teamId }, data: { name, updatedBy: user.id } });
    nav.redirect(`/settings/${teamId}`); // caught
  } catch (error) {
    console.error("rename failed", error);
    return { ok: false };
  }
}

export async function leaveTeam(teamId: string, user: { id: string; role: string }) {
  try {
    if (isOwner(user)) return { ok: false, reason: "owner" };
    await prisma.membership.delete({ where: { teamId_userId: { teamId, userId: user.id } } });
  } catch (error) {
    return { ok: false };
  }
  nav.redirect("/teams");
}
