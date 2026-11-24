import { PrismaClient } from "@prisma/client";

const prisma = new PrismaClient();

export async function seedUsers() {
  const local = new PrismaClient();
  await local.user.create({ data: { email: "admin@example.com" } });
}

await prisma.user.create({ data: { email: "owner@example.com" } });
