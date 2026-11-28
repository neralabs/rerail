import { unstable_cache } from "next/cache";
import { PrismaClient } from "@prisma/client";

const prisma = new PrismaClient();

// The same shape in a React Router app that has no next dependency: not this rule's.
export async function loader({ params }: { params: { userId: string } }) {
  const userId = params.userId;
  const getInvoices = unstable_cache(async () => prisma.invoice.findMany({ where: { userId } }), ["invoices"]);
  return getInvoices();
}
