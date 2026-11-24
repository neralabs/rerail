import { initTRPC } from "@trpc/server";
import { z } from "zod";
import { db } from "../lib/db";

const t = initTRPC.create();

export const postRouter = t.router({
  create: t.procedure
    .input(z.object({ title: z.string(), body: z.string(), authorId: z.string() }))
    .mutation(({ input }) => db.post.create({ data: input })),
});
