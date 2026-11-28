import { prisma } from "@/lib/db";

// The plan shown and charged comes from "the" active subscription.
export async function activePlan(userId: string) {
  const sub = await prisma.subscription.findFirst({ where: { userId, status: "ACTIVE" } }); // caught
  return sub?.status ?? "free";
}

export async function lastInvoiceNumber(teamId: string) {
  const inv = await prisma.$transaction((tx) => tx.invoice.findFirstOrThrow({ where: { teamId }, select: { number: true } })); // caught
  return inv.number;
}

export async function latestInvoice(teamId: string) {
  return prisma.invoice.findFirst({ where: { teamId }, orderBy: [{ createdAt: "desc" }, { id: "desc" }] });
}

export async function someInvoice(teamId: string) {
  return prisma.invoice.findFirst({ where: { teamId }, take: 1 });
}

export async function invoicePage(teamId: string, page: number) {
  return prisma.invoice.findMany({ where: { teamId }, take: 20, skip: 20 * page });
}

export async function ownInvoice(id: string, teamId: string) {
  return prisma.invoice.findFirst({ where: { id, teamId } });
}

export async function userByEmail(email: string) {
  return prisma.user.findFirst({ where: { email: { equals: email, mode: "insensitive" } } });
}
