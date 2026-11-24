"use server";

import { headers } from "next/headers";
import { api, requireRole } from "@acme/auth-client";
import { prisma } from "@/lib/db";
import { createWorkoutSession } from "@/lib/workout";

// A function named after a "session" model is no session check.
export async function startWorkout(userId: string) { // caught
  await createWorkoutSession(userId);
}

// "Public" at the end of the name says what the action does, not who may call it.
export async function makeWorkoutPublic(id: string) { // caught
  await prisma.workout.update({ where: { id }, data: { visibility: "PUBLIC" } });
}

// headers() read for the client's IP is no auth check.
export async function deleteWorkout(id: string) { // caught
  const ip = (await headers()).get("x-forwarded-for");
  console.log("delete from", ip);
  await prisma.workout.delete({ where: { id } });
}

// The write itself is named after permissions; it checks nothing.
export async function grantCoachRole(userId: string) { // caught
  await updatePermissions(userId);
}

async function updatePermissions(userId: string) {
  await prisma.member.update({ where: { userId }, data: { role: "COACH" } });
}

// getAuthor looks up a note's author, not the caller.
export async function editWorkoutNote(id: string) { // caught
  const author = await getAuthor(id);
  await prisma.note.update({ where: { id }, data: { authorId: author } });
}

async function getAuthor(id: string) {
  return id;
}

// Near misses below: each checks the caller.

// headers() handed to a session lookup, whatever the lookup is called.
export async function renameWorkout(id: string, name: string) {
  const caller = await api.lookup({ headers: await headers() });
  if (!caller) throw new Error("Unauthorized");
  await prisma.workout.update({ where: { id, userId: caller.id }, data: { name } });
}

// The authorization header is read and compared.
export async function archiveWorkout(id: string) {
  const token = (await headers()).get("authorization");
  if (token !== `Bearer ${process.env.CRON_SECRET}`) throw new Error("Unauthorized");
  await prisma.workout.update({ where: { id }, data: { archived: true } });
}

// A role check from a package.
export async function pinWorkout(id: string) {
  await requireRole("COACH");
  await prisma.workout.update({ where: { id }, data: { pinned: true } });
}
