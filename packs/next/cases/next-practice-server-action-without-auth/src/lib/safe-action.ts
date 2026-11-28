import { createSafeActionClient } from "next-safe-action";
import { auth } from "@/auth";

export const actionClient = createSafeActionClient();

// Named for what it is, not for auth: its middleware checks the session.
export const memberClient = actionClient.use(async ({ next }) => {
  const session = await auth();
  if (!session) throw new Error("Unauthorized");
  return next({ ctx: { user: session.user } });
});

export const authedClient = actionClient.use(async ({ next }) => next({ ctx: {} }));
