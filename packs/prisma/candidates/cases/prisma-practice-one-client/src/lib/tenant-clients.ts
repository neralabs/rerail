import { PrismaClient } from "@prisma/client";

const clients = new Map<string, PrismaClient>();
let fallback: PrismaClient | undefined;

export function clientFor(tenant: string, url: string) {
  if (!clients.has(tenant)) clients.set(tenant, new PrismaClient({ datasourceUrl: url }));
  return clients.get(tenant)!;
}

export function fallbackClient() {
  if (!fallback) fallback = new PrismaClient();
  return fallback;
}

export function createClient(url: string) {
  return new PrismaClient({ datasourceUrl: url });
}
