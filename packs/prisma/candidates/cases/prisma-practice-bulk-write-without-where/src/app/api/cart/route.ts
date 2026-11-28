import type { KeystoneContext } from "@keystone-6/core/types";

// KeystoneJS's list API: each item's filter sits inside the `data` array. Keystone generates a schema.prisma, so
// `Session` is a model there too.
export async function cancel(context: KeystoneContext, ids: string[]) {
  await context.query.Session.updateMany({ data: ids.map((id) => ({ where: { id }, data: { active: false } })) });
  await context.db.Session.deleteMany({ where: ids.map((id) => ({ id })) });
}
