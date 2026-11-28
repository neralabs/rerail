import { Provider } from "@prisma/client";
import { db } from "~/server/db";

const REGIONS = ["eu", "us", "apac"] as const;

export async function stockByRegion(sku: string) {
  return Promise.all(REGIONS.map((region) => db.stock.findMany({ where: { sku, region } })));
}

export async function keyStatus(userId: string) {
  const providers = Object.values(Provider) as Provider[];
  return Promise.all(providers.map((provider) => db.apiKey.findFirst({ where: { userId, provider } })));
}

export async function topPerCategory(categoryIds: string[]) {
  return Promise.all(
    categoryIds.map((categoryId) => db.product.findMany({ where: { categoryId }, take: 5, orderBy: { sold: "desc" } })),
  );
}

export async function latestPricePerSku(skus: string[]) {
  return Promise.all(skus.map((sku) => db.price.findFirst({ where: { sku }, orderBy: { validFrom: "desc" } })));
}

export async function firstContactEmail(links: { id: string }[]) {
  for (const link of links) {
    const row = await db.contact.findFirst({ where: { id: link.id, email: { not: null } } });
    if (row) return row.email;
  }
  return null;
}

export async function firstFree(slots: string[]) {
  let found: string | null = null;
  for (const slot of slots) {
    const taken = await db.booking.count({ where: { slot } });
    if (taken === 0) {
      found = slot;
      break;
    }
  }
  return found;
}

export async function retryEach(jobIds: string[]) {
  for (const id of jobIds) {
    try {
      await db.job.update({ where: { id }, data: { status: "QUEUED" } });
    } catch (e) {
      console.error("could not requeue", id, e);
    }
  }
}

export async function syncRates(rates: Record<string, number>) {
  for (const [currency, rate] of Object.entries(rates)) {
    await db.exchangeRate.upsert({ where: { currency }, create: { currency, rate }, update: { rate } }); // caught
  }
}

export async function touchAll(a: string[], b: string[]) {
  for (const id of [...a, ...b]) {
    await db.user.update({ where: { id }, data: { seenAt: new Date() } }); // caught
  }
  for (const id of a) {
    for (const other of b) {
      if (id === other) break;
      await db.follow.deleteMany({ where: { followerId: id, followeeId: other } }); // caught
    }
  }
}

export async function markSent(messages: { id: string; ok: boolean }[]) {
  const sent = [];
  for (const m of messages) if (m.ok) sent.push(m);
  await Promise.all(sent.map((m) => db.message.update({ where: { id: m.id }, data: { status: "SENT" } }))); // caught
}

export async function upgradeEach(pluginIds: string[]) {
  for (const id of pluginIds) {
    try {
      await runUpgrade(id);
    } catch (e) {
      await db.plugin.update({ where: { id }, data: { status: "DISABLED" } });
    }
  }
}

declare function runUpgrade(id: string): Promise<void>;

export async function applyEdits(edits: { id: string; body: string }[]) {
  const ops = [];
  for (const e of edits) ops.push(db.note.update({ where: { id: e.id }, data: { body: e.body } }));
  await db.$transaction(ops);
}
