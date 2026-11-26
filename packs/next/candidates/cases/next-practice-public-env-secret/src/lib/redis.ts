import { Redis } from "@upstash/redis";

export const redis = new Redis({
  url: process.env.NEXT_PUBLIC_UPSTASH_REDIS_REST_URL!,
  token: process.env.NEXT_PUBLIC_UPSTASH_REDIS_REST_TOKEN!, // caught
});

export const githubToken = process.env.NEXT_PUBLIC_GITHUB_ACCESS_TOKEN; // caught

// Meant for browsers: Upstash's read-only token, Mapbox's public access token, a bare TOKEN.
export const readOnly = process.env.NEXT_PUBLIC_UPSTASH_REDIS_REST_READ_ONLY_TOKEN;
export const mapboxToken = process.env.NEXT_PUBLIC_MAPBOX_ACCESS_TOKEN;
export const widgetToken = process.env.NEXT_PUBLIC_WIDGET_TOKEN;
