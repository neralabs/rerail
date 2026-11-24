import { prisma } from "../src/lib/db";

const sub = await prisma.subscription.findFirst({ where: { status: "ACTIVE" } });
console.log(sub);
