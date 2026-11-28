import { prisma } from "./db";

// An ordinary helper module, not the Next.js middleware file.
export async function auditLog(path: string) {
  await prisma.auditLog.create({ data: { path } });
}
