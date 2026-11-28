import { unstable_cache } from "next/cache";

import { requireUser } from "@/lib/auth";
import { prisma } from "@/lib/db";

const TAKE = 20;

export default async function DashboardPage() {
  const { userId, orgId } = await requireUser();
  const getInvoices = unstable_cache( // caught
    async () => prisma.invoice.findMany({ where: { userId }, take: TAKE }),
    ["dashboard-invoices"],
    { revalidate: 60 },
  );
  const getOrgName = unstable_cache(
    async (id: string) => (await prisma.org.findUniqueOrThrow({ where: { id } })).name,
    ["org-name"],
  );
  const invoices = await getInvoices();
  return (
    <main>
      <h1>{await getOrgName(orgId)}</h1>
      <ul>{invoices.map((i) => <li key={i.id}>{i.number}</li>)}</ul>
    </main>
  );
}
