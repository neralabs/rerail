import { PrismaClient } from "@prisma/client";

const prisma = new PrismaClient();

// A one-off script: exports nothing, runs main() when started on its own.
async function main() {
  const users = await prisma.user.findMany({ where: { planId: null } });
  for (const u of users) {
    await prisma.user.update({ where: { id: u.id }, data: { planId: "free" } });
  }
}

main()
  .catch((e) => {
    console.error(e);
    process.exit(1);
  })
  .finally(() => prisma.$disconnect());
