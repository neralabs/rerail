import { PrismaClient } from "@prisma/client";

const prisma = new PrismaClient();

async function main() {
  for (const name of ["Alice", "Bob", "Carol"]) {
    await prisma.user.create({ data: { name, email: `${name.toLowerCase()}@example.com` } });
  }
  const users = await prisma.user.findMany();
  for (const u of users) await prisma.profile.create({ data: { userId: u.id } });
}

main();
