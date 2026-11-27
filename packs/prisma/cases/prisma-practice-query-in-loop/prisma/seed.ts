import { PrismaClient } from "@prisma/client";
import { backfillSlugs } from "./backfill";
import { recountAll } from "@/server/jobs/recount";

const prisma = new PrismaClient();

async function main() {
  for (const name of ["Alice", "Bob", "Carol"]) {
    await prisma.user.create({ data: { name, email: `${name.toLowerCase()}@example.com` } });
  }
  const users = await prisma.user.findMany();
  for (const u of users) await prisma.profile.create({ data: { userId: u.id } });
  await backfillSlugs(prisma);
  await recountAll();
}

main();
