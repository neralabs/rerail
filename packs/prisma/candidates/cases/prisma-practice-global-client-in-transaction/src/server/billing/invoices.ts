import db from "@/lib/db";

export async function closeInvoice(invoiceId: string) {
  await db.$transaction(async () => {
    await db.invoice.update({ where: { id: invoiceId }, data: { closed: true } }); // caught
    await db.$executeRaw`UPDATE "Balance" SET "open" = "open" - 1 WHERE "invoiceId" = ${invoiceId}`; // caught
  });
}

async function bumpCounter(name: string) {
  await db.counter.update({ where: { name }, data: { value: { increment: 1 } } });
}

export async function issueInvoice(invoiceId: string) {
  await db.$transaction(async (tx) => {
    await tx.invoice.create({ data: { id: invoiceId } });
    await bumpCounter("invoices"); // caught
  });
  await bumpCounter("issued");
}
