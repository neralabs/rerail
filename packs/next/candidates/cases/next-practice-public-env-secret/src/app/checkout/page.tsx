"use client";

import { loadStripe } from "@stripe/stripe-js";

const stripePromise = loadStripe(process.env.NEXT_PUBLIC_STRIPE_PUBLISHABLE_KEY!);
const passwordLoginEnabled = process.env.NEXT_PUBLIC_ENABLE_PASSWORD === "true";
const demoPassword = process.env.NEXT_PUBLIC_DEMO_PASSWORD;
const sharing = process.env.NEXT_PUBLIC_SECRET_SHARING_ENABLED === "true";

export default function CheckoutPage() {
  return (
    <main>
      <p>{passwordLoginEnabled ? `Demo password: ${demoPassword}` : null}</p>
      <p>{sharing ? "Sharing on" : "Sharing off"}</p>
      <button onClick={() => stripePromise}>Pay</button>
    </main>
  );
}
