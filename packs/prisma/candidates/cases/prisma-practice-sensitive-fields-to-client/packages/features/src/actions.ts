"use server";

import { prisma } from "@acme/db";

// A 'use server' module in a package whose package.json does not list next: no Next.js Server Action is claimed.
export async function listSessions() {
  return prisma.session.findMany();
}
