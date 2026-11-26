import { auth } from "@/auth";

// next-safe-action middleware that turns signed-in visitors away: it reads the session.
export const throwIfSignedIn = async ({ next }: { next: () => Promise<unknown> }) => {
  const session = await auth();
  if (session) throw new Error("Already signed in");
  return next();
};
