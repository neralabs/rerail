"use server";

import { createSafeActionClient } from "next-safe-action";
import { z } from "zod";
import { prisma } from "@/lib/db";
import { action, publicAction } from "@/lib/action";
import { actionClient, authedClient, memberClient } from "@/lib/safe-action";

const idSchema = z.object({ id: z.string() });

export const leaveTeam = actionClient.schema(idSchema).action(async ({ parsedInput }) => { // caught
  await prisma.membership.deleteMany({ where: { teamId: parsedInput.id } });
});

export const renameTeam = action(z.object({ id: z.string(), name: z.string() }), async ({ id, name }) => { // caught
  await prisma.team.update({ where: { id }, data: { name } });
});

// The client's middleware checks the session.
export const deleteTeam = memberClient.schema(idSchema).action(async ({ parsedInput }) => {
  await prisma.team.delete({ where: { id: parsedInput.id } });
});

// The name says the client authenticates; the rule takes its word.
export const archiveTeam = authedClient.schema(idSchema).action(async ({ parsedInput }) => {
  await prisma.team.update({ where: { id: parsedInput.id }, data: { archived: true } });
});

// The action reads ctx, which a client's middleware fills with the user.
export const transferTeam = actionClient.action(async ({ ctx }: { ctx: { userId?: string } }) => {
  await prisma.team.update({ where: { id: "t" }, data: { ownerId: ctx.userId } });
});

// The wrapper comes straight from a package: the rule cannot read what it runs.
export const pinTeam = createSafeActionClient().action(async () => {
  await prisma.team.update({ where: { id: "t" }, data: { pinned: true } });
});

// Built with a wrapper whose name says it is for signed-out visitors.
export const requestAccess = publicAction(z.object({ email: z.string() }), async ({ email }) => {
  await prisma.accessRequest.create({ data: { email } });
});
