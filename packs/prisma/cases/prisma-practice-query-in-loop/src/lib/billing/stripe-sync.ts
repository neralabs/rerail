import Stripe from "stripe";
import { prisma } from "@/lib/db";

const stripe = new Stripe(process.env.STRIPE_SECRET_KEY!);

export async function pushCustomers(userIds: string[]) {
  const users = await prisma.user.findMany({ where: { id: { in: userIds } } });
  for (const u of users) {
    // not a Prisma client: Stripe's API has the same `x.y.create` shape
    await stripe.customers.create({ email: u.email, metadata: { userId: u.id } });
  }
}
