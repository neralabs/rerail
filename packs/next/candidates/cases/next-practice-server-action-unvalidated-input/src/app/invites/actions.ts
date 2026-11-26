"use server";

import { z } from "zod";
import type { Prisma } from "@prisma/client";
import { prisma } from "@/lib/db";
import { auth } from "@/lib/auth";
import { findOpenInvite, findOpenInviteChecked, countInvitesFor, assertCanEditProject } from "@/lib/invites";
import { touchProject } from "../../lib/projects";
import { archiveAll } from "@/lib/archive";
import { revokeTokens } from "@/lib/tokens";

export async function acceptInvite(token: string) {
  const session = await auth();
  const invite = await prisma.invite.findFirst({
    where: { token, usedAt: null }, // caught
  });
  if (!invite) return { error: "Invite not found" };
  await prisma.invite.update({ where: { id: invite.id }, data: { usedAt: new Date(), userId: session.user.id } });
}

export const deleteComments = async (data: { taskId: string }) => {
  const session = await auth();
  const { taskId } = data;
  if (!taskId) return { error: "Missing task" };
  await prisma.comment.deleteMany({ where: { taskId, authorId: session.user.id } }); // caught
};

export async function listProjects(filters: Prisma.ProjectWhereInput) {
  return prisma.project.findMany({ where: filters }); // caught
}

export async function searchProjects(input: { orgId: string; status: string }) {
  return prisma.project.findMany({ where: { ...input, archived: false } }); // caught
}

export async function renameProject(input: { id: string; name: string }) {
  const session = await auth();
  return prisma.project.updateMany({
    where: { id: input.id as string, ownerId: session.user.id }, // caught
    data: { name: String(input.name) },
  });
}

export async function redeemInvite(token: string) {
  const invite = await findOpenInvite(token); // caught
  return invite?.id ?? null;
}

export async function touch(projectId: string) {
  await touchProject(projectId); // caught
}

export async function archiveProjects(ids: string) {
  await archiveAll(ids); // caught
}

// ---- near misses ------------------------------------------------------------------------------

// A unique lookup: Prisma rejects an object where the unique id goes.
export async function getProject(id: string) {
  return prisma.project.findUnique({ where: { id } });
}

const ArchiveInput = z.object({ id: z.string().cuid() });

// The arguments go through a schema first.
export async function archiveProject(input: unknown) {
  const { id } = ArchiveInput.parse(input);
  await prisma.project.updateMany({ where: { id }, data: { archived: true } });
}

// The schema checks the raw argument; using it afterwards is safe.
export async function restoreProject(input: { id: string }) {
  ArchiveInput.parse(input);
  await prisma.project.updateMany({ where: { id: input.id }, data: { archived: false } });
}

// A typeof check.
export async function removeProject(id: string) {
  if (typeof id !== "string") throw new Error("invalid id");
  await prisma.project.deleteMany({ where: { id } });
}

// A check by a helper whose name says it checks.
export async function closeProject(id: string) {
  const session = await auth();
  await assertCanEditProject(session.user.id, id);
  await prisma.project.updateMany({ where: { id }, data: { closed: true } });
}

// The value is an operator's operand: Prisma types it as a string and rejects an object.
export async function findByTitle(q: string) {
  return prisma.project.findMany({ where: { title: { contains: q, mode: "insensitive" } } });
}

// Coerced where it is used.
export async function countByOwner(ownerId: string) {
  return prisma.project.count({ where: { ownerId: String(ownerId) } });
}

// FormData.get gives a string or a File, never an object Prisma reads as a filter.
export async function updateFromForm(formData: FormData) {
  const id = formData.get("id") as string;
  await prisma.project.updateMany({ where: { id }, data: { name: "x" } });
}

// The helper checks its own argument.
export async function redeemChecked(token: string) {
  return findOpenInviteChecked(token);
}

// The helper coerces its argument.
export async function invitesFor(email: string) {
  return countInvitesFor(email);
}

// Not exported: no client can call it directly.
async function purgeProject(id: string) {
  await prisma.project.deleteMany({ where: { id } });
}

// The filter is built from the session, not from the arguments.
export async function myProjects(page: number) {
  const session = await auth();
  return prisma.project.findMany({ where: { ownerId: session.user.id }, skip: page * 20 });
}

// A unique lookup by id comes first: Prisma rejects an object there, so the filters after it see a scalar.
export async function deactivateUser(userId: string) {
  await prisma.user.update({ where: { id: userId }, data: { active: false } });
  await prisma.session.deleteMany({ where: { userId } });
  await revokeTokens(userId);
}

// The same, in a helper called first.
export async function deletePriceList(id: string) {
  await loadListForWrite(id);
  const used = await prisma.account.count({ where: { priceListId: id } });
  if (used === 0) await prisma.priceList.delete({ where: { id } });
}

async function loadListForWrite(listId: string) {
  const list = await prisma.priceList.findUnique({ where: { id: listId } });
  if (!list) throw new Error("not found");
  return list;
}

// The unique lookup comes after the filter: the ownership check already ran with the client's object.
export async function deleteAccount(id: string) {
  const session = await auth();
  const account = await prisma.account.findFirst({ where: { id, ownerId: session.user.id } }); // caught
  if (!account) throw new Error("Not found");
  await prisma.account.delete({ where: { id } });
}

// One finding per query and value: the slug is compared in several places of one filter.
export async function getProjectBySlug(slug: string) {
  return prisma.project.findFirst({
    where: {
      OR: [
        { slug }, // caught
        { slugEn: slug },
        { slugEs: slug },
      ],
    },
  });
}
