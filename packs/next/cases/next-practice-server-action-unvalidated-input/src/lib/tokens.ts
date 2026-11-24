import { prisma } from "./db";

export async function revokeTokens(userId: string) {
  const { count } = await prisma.apiToken.updateMany({ where: { userId, revokedAt: null }, data: { revokedAt: new Date() } });
  return count;
}
