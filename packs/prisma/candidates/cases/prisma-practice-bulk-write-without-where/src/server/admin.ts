import { prisma } from "@/lib/db";

// Replace every row: the clear is followed by a create of the same model.
export async function replaceDrafts(userId: string, bodies: string[]) {
  await prisma.$transaction([
    prisma.draft.deleteMany(),
    prisma.draft.createMany({ data: bodies.map((body) => ({ userId, body })) }),
  ]);
}

// Move a flag to one row: every row loses it, then one row gets it.
export async function setDefaultNotificationKind(id: string) {
  await prisma.$transaction(async (tx) => {
    await tx.notification.updateMany({ data: { kind: "standard" } });
    await tx.notification.update({ where: { id }, data: { kind: "default" } });
  });
}

// "Delete all my data": a reset that empties every table.
export async function resetEverything() {
  await prisma.$transaction(async (tx) => {
    await tx.notification.deleteMany();
    await tx.session.deleteMany();
    await tx.draft.deleteMany();
    await tx.user.deleteMany();
  });
}

// A later create of another model is no refill: this still empties Session.
export async function rotateSessions(userId: string) {
  await prisma.session.deleteMany(); // caught
  await prisma.notification.create({ data: { userId, kind: "sessions-rotated" } });
}

// A create of the same model before the clear is no refill either.
export async function addThenClear(userId: string) {
  await prisma.draft.create({ data: { userId, body: "" } });
  await prisma.draft.deleteMany({}); // caught
}

// A later filtered update that sets another field moves no flag: this still marks every notification read.
export async function readAllThenTag(id: string) {
  await prisma.notification.updateMany({ data: { read: true } }); // caught
  await prisma.notification.update({ where: { id }, data: { kind: "tagged" } });
}

// Two tables emptied together: a reset of the user's inbox, not a forgotten filter.
export async function clearInbox() {
  await prisma.notification.deleteMany();
  await prisma.draft.deleteMany();
}

// The same table cleared twice is still one table: each call empties Session.
export async function clearSessionsTwice() {
  await prisma.session.updateMany({ data: { active: false } }); // caught
  await prisma.session.deleteMany(); // caught
}
