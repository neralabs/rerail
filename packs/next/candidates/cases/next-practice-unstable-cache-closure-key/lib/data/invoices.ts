import "server-only";
import { unstable_cache } from "next/cache";

import { prisma } from "@/lib/db";

export async function getOrgInvoices(orgId: string) {
  return unstable_cache(async () => prisma.invoice.findMany({ where: { orgId } }), ["org-invoices"])(); // caught
}

export async function getOrgInvoiceTotal(orgId: string, currency: string) {
  const where = { orgId, currency };
  const key = `org-invoice-total-${orgId}`;
  // keyed on the org only: every currency gets the first one's total
  return unstable_cache( // caught
    async () => prisma.invoice.aggregate({ where, _sum: { amount: true } }),
    [key],
  )();
}

export async function getOrgInvoice(orgId: string, invoiceId: string) {
  const where = { orgId, id: invoiceId };
  return unstable_cache(
    async () => prisma.invoice.findFirst({ where }),
    ["org-invoice", orgId, invoiceId],
  )();
}

export async function getUserInvoices(userId: string) {
  const cacheKey = `user-invoices-${userId}`;
  return unstable_cache(async () => prisma.invoice.findMany({ where: { userId } }), [cacheKey])();
}

export const getInvoice = unstable_cache(
  async (id: string) => prisma.invoice.findUnique({ where: { id } }),
  ["invoice"],
);
