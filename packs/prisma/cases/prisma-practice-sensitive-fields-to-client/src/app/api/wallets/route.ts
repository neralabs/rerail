import { prisma } from "@/lib/db";

// `token` defaults to a currency name and `displaySecretKey` is a masked copy: neither is a credential.
export async function GET() {
  const wallets = await prisma.wallet.findMany();
  return Response.json(wallets);
}

// Usage has secret columns only in the schema under examples/, which is not read.
export async function POST() {
  return Response.json(await prisma.usage.findMany());
}
