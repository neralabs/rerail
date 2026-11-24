import { z } from "zod";
import { protectedProcedure, router } from "../trpc";

export const teamRouter = router({
  removeMembers: protectedProcedure
    .input(z.object({ teamId: z.string(), userIds: z.array(z.string()) }))
    .mutation(async ({ ctx, input }) => {
      await ctx.db.$transaction(async (tx) => {
        for (const userId of input.userIds) {
          await tx.membership.delete({ where: { teamId_userId: { teamId: input.teamId, userId } } }); // caught
        }
      });
      input.userIds.forEach(async (userId) => {
        await ctx.db.notification.deleteMany({ where: { userId, teamId: input.teamId } }); // caught
      });
    }),
  rename: protectedProcedure
    .input(z.object({ teamId: z.string(), names: z.array(z.string()) }))
    .mutation(async ({ ctx, input }) => {
      // one batch: the writes go as one transaction, built by the map
      return ctx.db.$transaction(
        input.names.map((name, i) => ctx.db.alias.upsert({ where: { teamId_pos: { teamId: input.teamId, pos: i } }, create: { teamId: input.teamId, pos: i, name }, update: { name } })),
      );
    }),
});
