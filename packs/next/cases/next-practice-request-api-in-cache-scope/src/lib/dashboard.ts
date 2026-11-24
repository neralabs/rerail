import { currentViewer, getTenant } from "@/lib/auth";
import * as auth from "./auth";
import { prisma } from "./db";

export async function getDashboard() {
  "use cache: remote";
  const viewer = await currentViewer(); // caught
  return prisma.dashboard.findFirst({ where: { viewer: viewer ?? "" } });
}

export async function getTenantStats() {
  "use cache";
  const stats = await Promise.all(
    ["a", "b"].map(async (k) => prisma.stat.count({ where: { k, tenant: await getTenant() } })), // caught
  );
  return stats;
}

export async function getTenantName() {
  "use cache";
  return auth.getTenant(); // caught
}

// The fix: read the tenant in the caller and pass it in.
export async function getTenantNameFor(tenant: string) {
  "use cache";
  return prisma.tenant.findUnique({ where: { slug: tenant } });
}
