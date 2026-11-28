import { db } from "~/server/db";

// Exports nothing, but calls no function of its own at the top level: a module run for its side effect.
const queue = createQueue("nightly");
queue.process(async (accountIds: string[]) => {
  for (const id of accountIds) {
    await db.account.update({ where: { id }, data: { checkedAt: new Date() } }); // caught
  }
});

declare function createQueue(name: string): { process(fn: (ids: string[]) => Promise<void>): void };
