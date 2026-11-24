import { PrismaClient } from "@prisma/client";

async function run() {
  const prisma = new PrismaClient();
  await prisma.session.deleteMany({ where: { expiresAt: { lt: new Date() } } });
  await prisma.$disconnect();
}

run().catch((error) => {
  console.error(error);
  process.exit(1);
});
