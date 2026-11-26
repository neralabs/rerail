import { z } from "zod";
import { createTRPCRouter, protectedProcedure } from "@/server/api/trpc";

export const postRouter = createTRPCRouter({
  publish: protectedProcedure.input(z.object({ id: z.string() })).mutation(async ({ ctx, input }) => {
    return ctx.db.$transaction(async (tx) => {
      const post = await tx.post.update({ where: { id: input.id }, data: { published: true } });
      await ctx.db.feedItem.create({ data: { postId: post.id } }); // caught
      return post;
    });
  }),

  remove: protectedProcedure.input(z.object({ id: z.string() })).mutation(async ({ ctx, input }) => {
    await ctx.db.$transaction(async (tx) => {
      await tx.feedItem.deleteMany({ where: { postId: input.id } });
      await tx.post.delete({ where: { id: input.id } });
    });
  }),
});
