import { PrismaClient } from "@prisma/client";

const prisma = new PrismaClient();

async function main() {
  await prisma.draft.deleteMany();
  await prisma.user.deleteMany({});
  await prisma.user.create({ data: { email: "demo@example.com" } });
}

main().finally(() => prisma.$disconnect());
