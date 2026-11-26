import type { NextConfig } from "next";
import pkg from "./package.json";

const nextConfig: NextConfig = {
  env: {
    APP_VERSION: pkg.version,
    STRIPE_PUBLISHABLE_KEY: process.env.STRIPE_PUBLISHABLE_KEY,
    JWT_SECRET: process.env.JWT_SECRET, // caught
    "SMTP_PASSWORD": process.env.SMTP_PASSWORD, // caught
    SENTRY_AUTH_TOKEN: process.env.SENTRY_AUTH_TOKEN, // caught
  },
  images: { remotePatterns: [{ hostname: "xyz.supabase.co" }] },
};

export default nextConfig;
