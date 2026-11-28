import { prisma } from "@acme/data";

// A framework-agnostic request guard for the Node.js API server, not a Next.js middleware file.
export async function requireApiKey(key: string | null) {
  return key ? prisma.apiKey.findUnique({ where: { key } }) : null;
}
