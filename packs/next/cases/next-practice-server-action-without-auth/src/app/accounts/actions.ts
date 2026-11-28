"use server";

import { z } from "zod";
import { prisma } from "@/lib/db";
import { throwIfSignedIn } from "@/lib/guards";
import { actionClient } from "@/lib/safe-action";

// The middleware passed by name reads the session: followed like a call.
export const createAccount = actionClient
  .schema(z.object({ email: z.string() }))
  .use(throwIfSignedIn)
  .action(async ({ parsedInput }) => {
    await prisma.user.create({ data: { email: parsedInput.email } });
  });
