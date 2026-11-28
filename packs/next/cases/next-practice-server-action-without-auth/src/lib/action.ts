import type { ZodType } from "zod";

// A home-made wrapper: it validates the input, and checks no session.
export function action<T, R>(schema: ZodType<T>, fn: (input: T) => Promise<R>) {
  return async (input: unknown) => fn(schema.parse(input));
}

// For the few actions that must work signed out.
export function publicAction<T, R>(schema: ZodType<T>, fn: (input: T) => Promise<R>) {
  return async (input: unknown) => fn(schema.parse(input));
}
