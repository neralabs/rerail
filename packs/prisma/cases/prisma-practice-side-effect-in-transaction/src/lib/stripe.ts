import Stripe from "stripe";

const globalForStripe = globalThis as unknown as { stripe?: Stripe };

export const stripe = globalForStripe.stripe ?? new Stripe(process.env.STRIPE_SECRET_KEY!);
