"use server";

import { prisma } from "../db";

export async function seedProject() {
  await prisma.project.create({ data: { name: "seed" } });
}
