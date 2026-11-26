import { createEnv } from "@t3-oss/env-nextjs";
import { z } from "zod";

export const env = createEnv({
  server: {
    DATABASE_URL: z.string().url(),
    STRIPE_SECRET_KEY: z.string(),
  },
  client: {
    NEXT_PUBLIC_POSTHOG_KEY: z.string(),
    NEXT_PUBLIC_STRIPE_SECRET_KEY: z.string(), // caught
  },
  experimental__runtimeEnv: {
    NEXT_PUBLIC_POSTHOG_KEY: process.env.NEXT_PUBLIC_POSTHOG_KEY,
  },
});
