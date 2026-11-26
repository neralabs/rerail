import { z } from "zod";
import { createTRPCRouter, protectedProcedure } from "@/server/trpc";

const UserOut = z.object({ id: z.string(), email: z.string() });

export const userRouter = createTRPCRouter({
  list: protectedProcedure.query(({ ctx }) => ctx.db.user.findMany()), // caught
  byId: protectedProcedure.input(z.string()).query(async ({ ctx, input }) => {
    return ctx.db.user.findUnique({ where: { id: input }, select: { id: true, name: true } });
  }),
  // tRPC parses the result with UserOut, which strips passwordHash and resetToken.
  me: protectedProcedure
    .output(UserOut)
    .query(async ({ ctx }) => {
      const user = await ctx.db.user.findFirst();
      return user;
    }),
});
